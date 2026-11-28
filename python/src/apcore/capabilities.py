"""ACL-aware projections of the live module catalogue."""

from __future__ import annotations

from collections import OrderedDict
from collections.abc import Iterable
from copy import copy, deepcopy
from dataclasses import dataclass
from secrets import token_urlsafe
from threading import Lock
from time import monotonic
from typing import TYPE_CHECKING, Any, Literal, Protocol, runtime_checkable

from apcore._post_validation_policy import _argument_digest
from apcore.acl import ACL, VisibilityDecision
from apcore.builtin_steps import BuiltinACLCheck, BuiltinApprovalGate, BuiltinContextCreation, BuiltinModuleLookup
from apcore.context import Context, Identity
from apcore.errors import InvalidInputError, PolicyEvaluationError
from apcore.pipeline import _any_match
from apcore.policy import ExecutionPolicy
from apcore.schema.annotations import governance_union

if TYPE_CHECKING:
    from apcore.executor import Executor
    from apcore.registry.registry import _ModuleRegistration
    from apcore.registry.types import ModuleDescriptor

__all__ = ["AccessHint", "CallableDefinition", "CapabilityPage", "CapabilitySearchProvider", "ModuleSummary"]

AccessHint = Literal["allowed", "requires_approval", "conditional"]
_CURSOR_TTL_SECONDS = 300
_MAX_CURSORS = 128


@dataclass(frozen=True)
class ModuleSummary:
    """Lightweight discovery information for one visible module."""

    module_id: str
    version: str
    description: str
    tags: tuple[str, ...]
    readonly: bool
    access_hint: AccessHint


@dataclass(frozen=True)
class CapabilityPage:
    """A page of summaries and an opaque continuation for the requesting frame."""

    items: tuple[ModuleSummary, ...]
    next_cursor: str | None = None


@dataclass(frozen=True)
class CallableDefinition:
    """Detached schemas for a single authorized module registration."""

    module_id: str
    version: str
    input_schema: dict[str, Any]
    output_schema: dict[str, Any]


@runtime_checkable
class CapabilitySearchProvider(Protocol):
    """Rank authorized candidate IDs for the normalized query."""

    def search(self, query: str | None, candidates: tuple[str, ...]) -> Iterable[str]: ...


@dataclass(frozen=True)
class _Filters:
    query: str | None
    prefix: str | None
    tags: tuple[str, ...]
    readonly: bool | None
    limit: int
    include_approval_required: bool

    @classmethod
    def create(
        cls,
        query: str | None,
        prefix: str | None,
        tags: Iterable[str] | None,
        readonly: bool | None,
        limit: int,
        include_approval_required: bool,
    ) -> _Filters:
        if query is not None and not isinstance(query, str) or prefix is not None and not isinstance(prefix, str):
            raise InvalidInputError("Discovery query and prefix must be strings")
        if isinstance(tags, (str, bytes)):
            raise InvalidInputError("Discovery tags must be a collection of strings")
        try:
            tag_items = tuple(() if tags is None else tags)
        except TypeError as exc:
            raise InvalidInputError("Discovery tags must be a collection of strings") from exc
        if any(not isinstance(tag, str) for tag in tag_items):
            raise InvalidInputError("Discovery tags must be a collection of strings")
        if readonly is not None and not isinstance(readonly, bool) or not isinstance(include_approval_required, bool):
            raise InvalidInputError("Discovery flags must be booleans")
        if isinstance(limit, bool) or not isinstance(limit, int) or not 1 <= limit <= 100:
            raise InvalidInputError("Discovery limit must be an integer from 1 through 100")
        normalized_query = query.strip().lower() if query is not None else ""
        return cls(
            normalized_query or None, prefix, tuple(sorted(set(tag_items))), readonly, limit, include_approval_required
        )

    def matches(self, item: ModuleSummary) -> bool:
        return (
            (self.readonly is None or self.readonly == item.readonly)
            and all(tag in item.tags for tag in self.tags)
            and (self.include_approval_required or item.access_hint != "requires_approval")
        )


@dataclass(frozen=True)
class _Caller:
    identity: Identity | None
    caller_id: str | None
    call_chain: tuple[str, ...]

    @classmethod
    def capture(cls, context: Context) -> _Caller:
        return cls(deepcopy(context.identity), context.caller_id, tuple(context.call_chain))

    def snapshot_context(self, context: Context) -> Context:
        snapshot = copy(context)
        snapshot.identity = self.identity
        snapshot.caller_id = self.caller_id
        snapshot.call_chain = list(self.call_chain)
        return snapshot

    def digest(self) -> str:
        identity = self.identity
        scope = {
            "caller_id": self.caller_id,
            "call_chain": list(self.call_chain),
            "identity": identity._authority_projection() if identity is not None else None,
        }
        try:
            return _argument_digest(scope)
        except PolicyEvaluationError:
            raise InvalidInputError("Invalid discovery cursor") from None


@dataclass(frozen=True)
class _Cursor:
    expires_at: float
    caller_digest: str
    filters: _Filters
    after: str
    provider: CapabilitySearchProvider | None
    remaining_ranked_ids: tuple[str, ...] | None


_Registration = tuple["_ModuleRegistration", object | None]


@dataclass(frozen=True)
class _SummaryCandidate:
    index: int
    summary: ModuleSummary
    registration: _Registration
    access: VisibilityDecision


class _CapabilityDiscovery:
    def __init__(self) -> None:
        self.provider: CapabilitySearchProvider | None = None
        self._cursors: OrderedDict[str, _Cursor] = OrderedDict()
        self._cursor_lock = Lock()

    def _cursor(
        self, token: str | None, caller: _Caller, filters: _Filters, provider: CapabilitySearchProvider | None
    ) -> _Cursor | None:
        if token is None:
            return None
        if not isinstance(token, str) or len(token) != 32:
            raise InvalidInputError("Invalid discovery cursor")
        with self._cursor_lock:
            entry = self._cursors.get(token)
        if (
            entry is None
            or entry.expires_at <= monotonic()
            or entry.caller_digest != caller.digest()
            or entry.filters != filters
            or entry.provider is not provider
        ):
            raise InvalidInputError("Invalid discovery cursor")
        return entry

    def _next_cursor(self, entry: _Cursor) -> str:
        token = token_urlsafe(24)
        with self._cursor_lock:
            self._cursors[token] = entry
            if len(self._cursors) > _MAX_CURSORS:
                self._cursors.popitem(last=False)
        return token

    def list(self, executor: Executor, context: Context, filters: _Filters, cursor: str | None) -> CapabilityPage:
        caller = _Caller.capture(context)
        context = caller.snapshot_context(context)
        provider = self.provider
        entry = self._cursor(cursor, caller, filters, provider)
        if entry is not None and entry.remaining_ranked_ids is not None:
            candidates = entry.remaining_ranked_ids
        else:
            candidates = tuple(executor.registry.list(prefix=filters.prefix))
            if entry is not None:
                candidates = tuple(module_id for module_id in candidates if module_id > entry.after)
            elif provider is not None:
                eligible = tuple(self._summaries(executor, context, filters, candidates, lexical=False))
                allowed_ids = tuple(
                    candidate.summary.module_id
                    for candidate in eligible
                    if self._summary_is_current(executor, context, candidate)
                )
                allowed = set(allowed_ids)
                candidates = tuple(
                    dict.fromkeys(
                        candidate
                        for candidate in provider.search(filters.query, allowed_ids)
                        if isinstance(candidate, str) and candidate in allowed
                    )
                )

        items: list[_SummaryCandidate] = []
        next_cursor = None
        for candidate in self._summaries(executor, context, filters, candidates, lexical=provider is None):
            if len(items) == filters.limit:
                next_cursor = self._next_cursor(
                    _Cursor(
                        monotonic() + _CURSOR_TTL_SECONDS,
                        caller.digest(),
                        filters,
                        items[-1].summary.module_id,
                        provider,
                        candidates[candidate.index :] if provider is not None else None,
                    )
                )
                break
            items.append(candidate)
        emitted = tuple(item.summary for item in items if self._summary_is_current(executor, context, item))
        return CapabilityPage(emitted, next_cursor)

    def _summary_is_current(self, executor: Executor, context: Context, item: _SummaryCandidate) -> bool:
        current = self._visible(executor, item.summary.module_id, context, None)
        return current is not None and _same_registration(item.registration, current[0]) and item.access == current[1]

    def _summaries(
        self, executor: Executor, context: Context, filters: _Filters, candidates: tuple[str, ...], *, lexical: bool
    ) -> Iterable[_SummaryCandidate]:
        for index, module_id in enumerate(candidates):
            selected = self._visible(executor, module_id, context, None)
            if selected is None:
                continue
            (registration, _), access = selected
            metadata = registration.metadata
            declared = metadata.get("annotations")
            annotations = governance_union(getattr(registration.module, "annotations", None), declared)
            hint = _access_hint(executor, module_id, annotations, access)
            readonly = annotations.readonly if annotations is not None else False
            if declared is not None:
                readonly = declared.get("readonly", readonly) if isinstance(declared, dict) else declared.readonly
            description = metadata.get("description") or ""
            tags = tuple(metadata.get("tags") or ())
            if (
                lexical
                and filters.query
                and not any(filters.query in value.lower() for value in (module_id, description, *tags))
            ):
                continue
            item = ModuleSummary(
                module_id,
                executor.registry._effective_version(registration.module, metadata),
                description,
                tags,
                readonly,
                hint,
            )
            if filters.matches(item) and _still_current(executor, module_id, None, selected[0]):
                yield _SummaryCandidate(index, item, selected[0], access)

    def descriptor(
        self, executor: Executor, module_id: str, context: Context, version_hint: str | None
    ) -> ModuleDescriptor | None:
        if not isinstance(module_id, str) or not module_id:
            return None
        context = _Caller.capture(context).snapshot_context(context)
        selected = self._visible(executor, module_id, context, version_hint)
        if selected is None:
            return None
        descriptor = deepcopy(
            executor.registry._describe_registration(module_id, selected[0][0], warn_deprecation=False)
        )
        current = self._visible(executor, module_id, context, version_hint)
        if current is None or not _same_registration(selected[0], current[0]):
            return None
        return descriptor

    @staticmethod
    def _visible(
        executor: Executor, module_id: str, context: Context, version_hint: str | None
    ) -> tuple[_Registration, VisibilityDecision] | None:
        context_ready = False
        lookup = None
        gate = None
        for step in executor.current_strategy.steps:
            patterns = getattr(step, "match_modules", None)
            if patterns is not None and not _any_match(patterns, module_id):
                continue
            if type(step) is BuiltinContextCreation:
                context_ready = True
            elif type(step) is BuiltinModuleLookup:
                lookup = step
            elif type(step) is BuiltinACLCheck:
                gate = step
                break
        if not context_ready or gate is None:
            return None
        if lookup is None or lookup._registry is not executor.registry or lookup.toggle_state.is_disabled(module_id):
            return None
        selected = executor.registry._discovery_registration(module_id, version_hint)
        if selected is None:
            return None
        access = _visibility_decision(gate.acl, module_id, context)
        if access.visible and _still_current(executor, module_id, version_hint, selected):
            return selected, access
        return None


def _same_registration(selected: _Registration, current: _Registration | None) -> bool:
    return (
        current is not None
        and selected[1] is current[1]
        and selected[0].module is current[0].module
        and selected[0].metadata is current[0].metadata
        and selected[0].descriptor_metadata == current[0].descriptor_metadata
    )


def _still_current(executor: Executor, module_id: str, version_hint: str | None, selected: _Registration) -> bool:
    current = executor.registry._discovery_registration(module_id, version_hint)
    return _same_registration(selected, current) and not executor.toggle_state.is_disabled(module_id)


def _visibility_decision(acl: ACL | None, module_id: str, context: Context) -> VisibilityDecision:
    if context.identity is not None and context.identity.scopes is not None and type(acl) is not ACL:
        return VisibilityDecision(False)
    if acl is None:
        return VisibilityDecision(True)
    evaluator = getattr(acl, "evaluate_visibility", None)
    if not callable(evaluator) or (
        type(acl) is not ACL and getattr(type(acl), "evaluate_visibility", None) is ACL.evaluate_visibility
    ):
        return VisibilityDecision(False)
    child = context.child(module_id)
    child.identity = deepcopy(child.identity)
    try:
        decision = evaluator(child.caller_id, module_id, child)
    except Exception:
        return VisibilityDecision(False)
    return decision if isinstance(decision, VisibilityDecision) else VisibilityDecision(False)


def _access_hint(executor: Executor, module_id: str, annotations: Any, access: VisibilityDecision) -> AccessHint:
    gate = next((step for step in executor.current_strategy.steps if type(step) is BuiltinApprovalGate), None)
    if gate is not None:
        enforces = gate.policy.strict or gate.handler is not None or executor._policy_evaluator is not None
        policy_requires_approval = (
            type(gate.policy) is ExecutionPolicy and gate.policy.resolve(module_id, annotations).needs_approval
        )
        if enforces and (access.approval_required or policy_requires_approval):
            return "requires_approval"
        if type(gate.policy) is not ExecutionPolicy or enforces and access.approval_conditional:
            return "conditional"
    return "conditional" if executor._policy_evaluator is not None else "allowed"
