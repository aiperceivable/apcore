"""Execution context, identity, and context logger."""

from __future__ import annotations

import logging
import time
import uuid
from collections.abc import Awaitable, Callable, Sequence
from copy import copy, deepcopy
from dataclasses import dataclass, field, replace
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, Generic, Literal, Mapping, Protocol, TypeVar, runtime_checkable

from apcore.call_budget import CallBudget, _CallReservation
from apcore.cancel import CancelToken, _AwaitDeadlineExpired, _await_with_cancellation
from apcore.context_keys import TRACING_SAMPLED, _current_trace_span, _middleware_data
from apcore.errors import InvalidInputError, ModuleTimeoutError
from apcore.trace_context import _valid_inbound_parent_id

if TYPE_CHECKING:
    from contextlib import AsyncExitStack

    from apcore.trace_context import TraceParent


__all__ = [
    "Context",
    "Identity",
    "ContextFactory",
    "GovernanceProjection",
    "ProgressReporter",
    "ElicitationHandler",
    "ElicitationResult",
]

T = TypeVar("T")
R = TypeVar("R")
_MAX_DELEGATION_DEPTH = 8


def _valid_scope_literal(scope: Any) -> bool:
    return (
        isinstance(scope, str)
        and bool(scope)
        and not any(char.isspace() and char not in "\x1c\x1d\x1e\x1f" for char in scope)
    )


def _normalize_scopes(scopes: Sequence[str]) -> tuple[str, ...]:
    if isinstance(scopes, (str, bytes)) or not isinstance(scopes, (list, tuple)):
        raise InvalidInputError("Identity.scopes must be an array of scope strings")
    if not all(_valid_scope_literal(scope) for scope in scopes):
        raise InvalidInputError("Identity.scopes must contain nonempty strings without whitespace")
    return tuple(sorted(set(scopes)))


def _identity_audit_fields(context: Context) -> dict[str, Any]:
    identity = getattr(context, "identity", None)
    if not isinstance(identity, Identity) or identity.scopes is None:
        return {}
    return {
        "actor_id": identity.id,
        "subject_id": identity.root_subject.id,
        "delegation_depth": identity.delegation_depth,
    }


@dataclass(frozen=True)
class Identity:
    """Caller identity (human/service/AI generic)."""

    id: str
    type: str = "user"
    roles: tuple[str, ...] = field(default_factory=tuple)
    attrs: dict[str, Any] = field(default_factory=dict)
    scopes: Sequence[str] | None = field(default=None, kw_only=True)
    on_behalf_of: Identity | None = field(default=None, kw_only=True)
    issuer: str | None = field(default=None, kw_only=True)

    def __post_init__(self) -> None:
        scoped = self.scopes is not None or self.on_behalf_of is not None
        if scoped and (
            not isinstance(self.id, str)
            or not self.id
            or not isinstance(self.type, str)
            or not self.type
            or not isinstance(self.roles, (list, tuple))
            or any(not isinstance(role, str) or not role for role in self.roles)
        ):
            raise InvalidInputError("Scoped identities require a nonempty ID and type and valid roles")

        # Ensure attrs is a dict if None was passed, and protect against mutation
        # of the original dict passed to the constructor.
        if not isinstance(self.attrs, dict):
            object.__setattr__(self, "attrs", {})
        else:
            object.__setattr__(self, "attrs", dict(self.attrs))

        # IDN-2. identity-system.md declares `roles` an IMMUTABLE sequence, and
        # the annotation says `tuple[str, ...]`, but the dataclass stored
        # whatever it was handed: `roles = ["admin"]; Identity(id="u",
        # roles=roles); roles.append("root")` left the identity holding
        # `['admin', 'root']` — a list, not a tuple — so a later ACL `roles`
        # condition granted a role nobody assigned. Copying beside `attrs`,
        # which has been copied for the same reason since the field existed.
        if self.roles is None:
            object.__setattr__(self, "roles", ())
        elif not isinstance(self.roles, tuple):
            object.__setattr__(self, "roles", tuple(self.roles))

        if self.scopes is not None:
            object.__setattr__(self, "scopes", _normalize_scopes(self.scopes))
        if self.issuer is not None and (not isinstance(self.issuer, str) or not self.issuer):
            raise InvalidInputError("Identity.issuer must be a nonempty string")
        self._lineage()

    def _lineage(self) -> tuple[Identity, ...]:
        chain: list[Identity] = [self]
        seen = {id(self)}
        current = self
        while current.on_behalf_of is not None:
            parent = current.on_behalf_of
            if not isinstance(parent, Identity) or id(parent) in seen or len(chain) > _MAX_DELEGATION_DEPTH:
                raise InvalidInputError("Identity delegation must be acyclic and at most eight edges deep")
            if current.scopes is None or parent.scopes is None or not set(current.scopes).issubset(parent.scopes):
                raise InvalidInputError("Identity delegation requires explicit scopes within the parent ceiling")
            chain.append(parent)
            seen.add(id(parent))
            current = parent
        return tuple(chain)

    @property
    def root_subject(self) -> Identity:
        """Return the root subject of the active authority chain."""
        return self._lineage()[-1]

    @property
    def delegation_depth(self) -> int:
        """Return the number of active delegation edges."""
        return len(self._lineage()) - 1

    def delegate(self, actor: Identity, scopes: Sequence[str]) -> Identity:
        """Create a new actor identity with attenuated local authority."""
        if not isinstance(actor, Identity) or actor.on_behalf_of is not None:
            raise InvalidInputError("Identity delegation requires a direct actor")
        requested = _normalize_scopes(scopes)
        if actor.scopes is not None and not set(requested).issubset(actor.scopes):
            raise InvalidInputError("Requested scopes exceed the actor's own scope ceiling")
        return replace(actor, scopes=requested, on_behalf_of=self)

    def authority_key(self) -> str | None:
        """Return the native authority binding for a scope-enabled identity."""
        if self.scopes is None:
            return None
        from apcore._post_validation_policy import _argument_digest

        return _argument_digest(self._authority_projection())

    def owner_key(self) -> str | None:
        """Return the actor and root-subject binding independently of scopes."""
        if self.scopes is None:
            return None
        from apcore._post_validation_policy import _argument_digest

        subject = self.root_subject
        return _argument_digest(
            {
                "actor": [self.issuer, self.type, self.id],
                "subject": [subject.issuer, subject.type, subject.id],
            }
        )

    def _authority_projection(self) -> dict[str, Any]:
        if self.scopes is None:
            return self._to_wire(detach_attrs=True)
        subject: dict[str, Any] | None = None
        for identity in reversed(self._lineage()):
            node: dict[str, Any] = {
                "id": identity.id,
                "type": identity.type,
                "roles": sorted(set(identity.roles)),
                "scopes": list(identity.scopes) if identity.scopes is not None else None,
            }
            if identity.issuer is not None:
                node["issuer"] = identity.issuer
            if subject is not None:
                node["on_behalf_of"] = subject
            subject = node
        assert subject is not None
        return subject

    def _to_wire(self, *, detach_attrs: bool = False) -> dict[str, Any]:
        self._lineage()
        return self._encode_wire(detach_attrs)

    def _encode_wire(self, detach_attrs: bool) -> dict[str, Any]:
        result = {
            "id": self.id,
            "type": self.type,
            "roles": list(self.roles),
            "attrs": deepcopy(self.attrs) if detach_attrs else dict(self.attrs),
        }
        if self.scopes is not None:
            result["scopes"] = list(self.scopes)
        if self.on_behalf_of is not None:
            result["on_behalf_of"] = self.on_behalf_of._encode_wire(detach_attrs)
        if self.issuer is not None:
            result["issuer"] = self.issuer
        return result

    @classmethod
    def _from_wire(cls, value: Any, depth: int = 0) -> Identity:
        prefix = "Context.deserialize"
        if not isinstance(value, dict):
            raise InvalidInputError(f"{prefix}: 'identity' must be an object, got {type(value).__name__}")
        if depth > _MAX_DELEGATION_DEPTH:
            raise InvalidInputError("Identity delegation must be acyclic and at most eight edges deep")
        if not isinstance(value.get("id"), str) or not value["id"]:
            raise InvalidInputError(f"{prefix}: 'identity.id' is required and must be a non-empty string")
        roles = value.get("roles", ()) if "scopes" in value or "on_behalf_of" in value else value.get("roles") or ()
        if not isinstance(roles, (list, tuple)):
            raise InvalidInputError(f"{prefix}: 'identity.roles' must be a list of strings")
        attrs = value.get("attrs") or {}
        if not isinstance(attrs, dict):
            raise InvalidInputError(f"{prefix}: 'identity.attrs' must be an object")
        if "issuer" in value and value["issuer"] is None:
            raise InvalidInputError("Identity.issuer must be a nonempty string")
        return cls(
            value["id"],
            value.get("type", "user"),
            tuple(roles),
            attrs,
            scopes=_normalize_scopes(value["scopes"]) if "scopes" in value else None,
            on_behalf_of=cls._from_wire(value["on_behalf_of"], depth + 1) if "on_behalf_of" in value else None,
            issuer=value.get("issuer"),
        )

    def get_attr(self, key: str, default: Any = None) -> Any:
        """Get an attribute value by key.

        Aligned with apcore D-03. May trigger a fetch if the identity
        is lazy-loaded (future extension).
        """
        return self.attrs.get(key, default)


#: The framework-owned approval token (PROTOCOL_SPEC §7.4). Excluded from the
#: governance projection: it is a protocol-level key, not caller input, and its
#: presence is the one difference between a call and its ``_approval_token``
#: resume. §7.4's resume semantics re-enter the pipeline from Step 1, so a
#: projection that carried the token would let the ACL reach a *different*
#: Step 4 verdict on the resume than on the call a human just approved.
_APPROVAL_TOKEN_KEY = "_approval_token"


def _json_type(value: Any) -> str:
    """Name *value*'s JSON Schema type, without reading the value itself.

    ``bool`` is tested before ``int`` because Python's ``bool`` is a subclass of
    ``int``; the other way round every flag would be reported as an integer.
    A value with no JSON counterpart is ``"unknown"`` rather than a plausible
    guess — the arguments reaching Step 4 have not been schema-validated
    (§6.1.7), so they may hold anything at all.
    """
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, (list, tuple)):
        return "array"
    if isinstance(value, Mapping):
        return "object"
    return "unknown"


@dataclass(frozen=True)
class GovernanceProjection:
    """The argument view a governance decision is allowed to see (§6.1.8).

    Computed during module lookup (Step 3) and carried to the ACL check
    (Step 4), where the built-in ``arguments`` condition (§6.1.7) reads it.
    It carries the argument **key set** and each key's JSON type, and it
    **cannot** carry a value: there is no field for one. A projection that
    structurally cannot hold a value cannot leak one, whatever a future
    predicate does with it.

    It is deliberately **not** :attr:`Context.redacted_inputs`, which §6.1.8
    rule 3 forbids substituting. ``redacted_inputs`` is documented as safe
    *logging*, and it is a raw copy of the inputs when the module declares no
    input schema — one field serving both "safe to log" and "input to a
    security decision" will eventually break one of them in a change made for
    the other.

    Attributes:
        keys: The argument keys present on this call.
        types: Each key's JSON Schema type name — ``"string"``, ``"integer"``,
            ``"number"``, ``"boolean"``, ``"array"``, ``"object"``, ``"null"``,
            or ``"unknown"`` for a value with no JSON counterpart.
    """

    keys: frozenset[str]
    types: Mapping[str, str]

    @classmethod
    def of(cls, arguments: Mapping[str, Any] | None) -> GovernanceProjection:
        """Project *arguments* down to keys and types, discarding every value.

        ``_approval_token`` is dropped: see :data:`_APPROVAL_TOKEN_KEY`. A
        non-string key is dropped too — ACL predicates name keys as strings, so
        a key that cannot be named is a key no rule can be written about.
        """
        if not isinstance(arguments, Mapping):
            return cls(keys=frozenset(), types=MappingProxyType({}))
        types = {
            key: _json_type(value)
            for key, value in arguments.items()
            if isinstance(key, str) and key != _APPROVAL_TOKEN_KEY
        }
        return cls(keys=frozenset(types), types=MappingProxyType(types))

    def has(self, key: str) -> bool:
        """Whether *key* was present in the call's arguments."""
        return key in self.keys

    def type_of(self, key: str) -> str | None:
        """The JSON type of *key*, or None when the key was absent."""
        return self.types.get(key)


@dataclass(frozen=True)
class ElicitationResult:
    """A transport-neutral reply to a request for input."""

    action: Literal["accept", "decline", "cancel"]
    content: dict[str, Any] | None = None

    def __post_init__(self) -> None:
        if self.action == "accept":
            if not isinstance(self.content, dict) or not all(isinstance(key, str) for key in self.content):
                raise InvalidInputError("Accepted elicitation content must be an object")
        elif self.action not in ("decline", "cancel") or self.content is not None:
            raise InvalidInputError("Declined or cancelled elicitation must have no content")


ProgressReporter = Callable[[float, float | None, str | None], Awaitable[None]]
ElicitationHandler = Callable[[str, dict[str, Any] | None], Awaitable[ElicitationResult]]


@dataclass
class Context(Generic[T]):
    """Module execution context."""

    trace_id: str
    caller_id: str | None = None
    call_chain: list[str] = field(default_factory=list)
    executor: Any = None
    identity: Identity | None = None
    redacted_inputs: dict[str, Any] | None = None
    redacted_output: dict[str, Any] | None = None
    data: dict[str, Any] = field(default_factory=dict)
    services: T = None  # type: ignore[assignment]
    cancel_token: CancelToken | None = None
    global_deadline: float | None = field(default=None, repr=False)
    # PROTOCOL_SPEC §6.1.8. Set by pipeline Step 3 (module lookup) and read by
    # the ACL check at Step 4; None outside a pipeline run, which makes the
    # `arguments` condition unevaluable rather than vacuously true (§6.1.1).
    # Transient like `executor` and `services`: not serialized.
    governance_projection: GovernanceProjection | None = field(default=None, repr=False)
    _execution_scope: AsyncExitStack | None = field(default=None, init=False, repr=False, compare=False)
    _invocation_data: dict[str, Any] | None = field(default=None, init=False, repr=False, compare=False)
    _local_parent_span: Any = field(default=None, init=False, repr=False, compare=False)
    _parent_sampled: bool | None = field(default=None, init=False, repr=False, compare=False)
    _inbound_trace_parent: TraceParent | None = field(default=None, init=False, repr=False, compare=False)
    _progress_reporter: ProgressReporter | None = field(default=None, init=False, repr=False, compare=False)
    _elicitation_handler: ElicitationHandler | None = field(default=None, init=False, repr=False, compare=False)
    _call_budget: CallBudget | None = field(default=None, init=False, repr=False, compare=False)
    _budget_reservation: _CallReservation | None = field(default=None, init=False, repr=False, compare=False)

    @classmethod
    def create(
        cls,
        identity: Identity | None = None,
        trace_parent: TraceParent | None = None,
        cancel_token: CancelToken | None = None,
        data: dict[str, Any] | None = None,
        services: T = None,  # type: ignore[assignment]
        global_deadline: float | None = None,
    ) -> Context[T]:
        """Create a new top-level Context with a generated trace_id.

        Unified signature per apcore PROTOCOL_SPEC §"Contract: Context.create"
        (v0.22.0, Issue #66). The accepted caller inputs are exactly:
        ``identity``, ``trace_parent``, ``cancel_token``, ``data``,
        ``services``, ``global_deadline``. ``executor`` and ``caller_id`` are
        intentionally NOT inputs — the executor is bound by the Executor at
        pipeline entry (see ``Context.bind_executor``), and ``caller_id`` is
        managed exclusively by ``Context.child()``.

        When *trace_parent* is provided, its ``trace_id`` is accepted only if
        it is exactly 32 lowercase hex characters and neither all-zero
        nor the APCore-reserved all-f value. Otherwise a fresh 32-hex trace_id is
        generated and a WARN-level log is emitted. No normalization (dashed
        UUID stripping, case folding) is performed here; such normalization
        is the responsibility of the TraceParent header parser or the
        caller's ContextFactory.
        """
        if trace_parent is not None:
            hex_id = trace_parent.trace_id
            if (
                isinstance(hex_id, str)
                and len(hex_id) == 32
                and all(c in "0123456789abcdef" for c in hex_id)
                and hex_id != "0" * 32
                and hex_id != "f" * 32
            ):
                trace_id = hex_id
            else:
                logging.getLogger(__name__).warning(
                    "Invalid trace_id format in trace_parent: %s. Restarting trace.",
                    hex_id,
                )
                trace_id = uuid.uuid4().hex
        else:
            trace_id = uuid.uuid4().hex
        ctx_data: dict[str, Any] = data if data is not None else {}
        # Carry W3C trace_flags and tracestate through the request lifecycle so
        # downstream TraceContext.inject() can propagate the inbound sampling
        # decision and vendor state instead of hardcoding "01".
        if trace_parent is not None:
            flags = getattr(trace_parent, "trace_flags", None)
            if isinstance(flags, str) and len(flags) == 2:
                ctx_data.setdefault("_apcore.trace.flags", flags)
            tracestate = getattr(trace_parent, "tracestate", ())
            if tracestate:
                ctx_data.setdefault("_apcore.trace.state", tuple(tracestate))
        context = cls(
            trace_id=trace_id,
            caller_id=None,
            call_chain=[],
            executor=None,
            identity=identity,
            data=ctx_data,
            services=services,  # type: ignore[arg-type]
            cancel_token=cancel_token,
            global_deadline=global_deadline,
        )
        if (
            trace_parent is not None
            and trace_id == trace_parent.trace_id
            and _valid_inbound_parent_id(trace_parent.parent_id)
        ):
            context._inbound_trace_parent = trace_parent
        return context

    def with_progress_reporter(self, callback: ProgressReporter | None) -> Context[T]:
        """Return a context with its own progress callback binding."""
        context = self._with_local_binding()
        context._progress_reporter = callback
        return context

    def with_elicitation_handler(self, callback: ElicitationHandler | None) -> Context[T]:
        """Return a context with its own request-for-input callback binding."""
        context = self._with_local_binding()
        context._elicitation_handler = callback
        return context

    def with_budget(self, budget: CallBudget | None) -> Context[T]:
        """Return a context that gives future calls and their children this budget."""
        if budget is not None and not isinstance(budget, CallBudget):
            raise InvalidInputError("Expected a CallBudget or None")
        context = self._with_local_binding()
        context._call_budget = budget
        context._budget_reservation = None
        return context

    def record_cost(self, actual: int) -> None:
        """Replace this attempt's estimated cost with its one actual cost report."""
        if self._call_budget is not None:
            reservation = self._budget_reservation
            if reservation is None:
                raise InvalidInputError("Actual cost can only be recorded during an active budgeted call")
            reservation.record_cost(actual)

    def _with_local_binding(self) -> Context[T]:
        context = copy(self)
        context.call_chain = list(self.call_chain)
        return context

    async def report_progress(self, progress: float, total: float | None = None, message: str | None = None) -> None:
        """Report progress to the attached host callback when one is available."""
        callback = self._progress_reporter
        if callback is not None:
            await self._wait_for_interaction(lambda: callback(progress, total, message))

    async def elicit(self, message: str, requested_schema: dict[str, Any] | None = None) -> ElicitationResult | None:
        """Ask the host for optional structured input."""
        callback = self._elicitation_handler
        if callback is None:
            return None
        result = await self._wait_for_interaction(lambda: callback(message, requested_schema))
        if not isinstance(result, ElicitationResult):
            raise InvalidInputError("Expected an ElicitationResult from the handler")
        return result

    async def _wait_for_interaction(self, callback: Callable[[], Awaitable[R]]) -> R:
        remaining = self.global_deadline - time.time() if self.global_deadline is not None else None
        try:
            return await _await_with_cancellation(callback, self.cancel_token, remaining)
        except _AwaitDeadlineExpired:
            module_id = self.call_chain[-1] if self.call_chain else ""
            raise ModuleTimeoutError(module_id, int(max(0, remaining or 0) * 1000)) from None

    def bind_executor(self, executor: Any) -> None:
        """SDK-internal contract member. Bind the Executor to this Context.

        Implements PROTOCOL_SPEC §"Contract: Executor binding to Context":
        - If ``self.executor`` is None, bind it.
        - If ``self.executor`` is already the same Executor instance
          (identity comparison), the rebind is a noop.
        - If ``self.executor`` is a *different* Executor instance, raise
          :class:`apcore.errors.ContextBindingError`.

        Not intended for application code; the Executor invokes this before
        pipeline step 1 on every entry point that accepts a caller-supplied
        Context.
        """
        if self.executor is None:
            self.executor = executor
        elif self.executor is not executor:
            # Imported lazily to avoid a circular import (errors -> context).
            from apcore.errors import ContextBindingError

            raise ContextBindingError("Context already bound to a different Executor instance")
        # else: same executor instance, noop.

    def serialize(self) -> dict[str, Any]:
        """Serialize Context to a JSON-encodable dict.

        Includes ``_context_version: 1`` at top level for forward
        compatibility. Excludes non-serializable / transient fields
        (``executor``, ``services``, ``cancel_token``, ``global_deadline``)
        and filters ``_``-prefixed keys from ``data``.
        """
        result: dict[str, Any] = {
            "_context_version": 1,
            "trace_id": self.trace_id,
            "caller_id": self.caller_id,
            "call_chain": list(self.call_chain),
        }
        result["identity"] = self.identity._to_wire() if self.identity is not None else None
        if self.redacted_inputs is not None:
            result["redacted_inputs"] = dict(self.redacted_inputs)
        if self.redacted_output is not None:
            result["redacted_output"] = dict(self.redacted_output)
        result["data"] = {k: v for k, v in self.data.items() if not k.startswith("_")}
        return result

    @classmethod
    def deserialize(cls, data: dict[str, Any]) -> Context:
        """Reconstruct a Context from its :meth:`serialize` output.

        Non-serializable fields (``executor``, ``services``, ``cancel_token``,
        ``global_deadline``) are set to ``None``; callers that need them
        should re-inject after deserialization. A ``_context_version`` greater
        than 1 logs a warning and best-effort proceeds (forward compatibility).
        """
        version = data.get("_context_version", 1)
        if version > 1:
            logging.getLogger(__name__).warning(
                "Unknown _context_version %d (expected 1). Proceeding with best-effort deserialization.",
                version,
            )

        return cls(
            trace_id=data.get("trace_id", ""),
            caller_id=data.get("caller_id"),
            call_chain=list(data.get("call_chain", [])),
            executor=None,
            identity=Identity._from_wire(data["identity"]) if data.get("identity") is not None else None,
            redacted_inputs=data.get("redacted_inputs"),
            redacted_output=data.get("redacted_output"),
            data=dict(data.get("data", {})),
            services=None,  # type: ignore[arg-type]
            cancel_token=None,
        )

    def child(self, target_module_id: str) -> Context[T]:
        """Create a child Context for calling a target module.

        The caller's ``data`` dictionary stays shared. A child carries its
        parent's current span explicitly, while each invocation owns its
        middleware timing and span data.
        """
        child = Context(
            trace_id=self.trace_id,
            caller_id=self.call_chain[-1] if self.call_chain else None,
            call_chain=[*self.call_chain, target_module_id],
            executor=self.executor,
            identity=self.identity,
            data=self.data,
            services=self.services,
            cancel_token=self.cancel_token,
            global_deadline=self.global_deadline,
        )
        child._invocation_data = {}
        child._local_parent_span = _current_trace_span(self, include_parent=True)
        child._parent_sampled = _middleware_data(self).get(TRACING_SAMPLED.name, self._parent_sampled)
        child._inbound_trace_parent = self._inbound_trace_parent
        child._progress_reporter = self._progress_reporter
        child._elicitation_handler = self._elicitation_handler
        child._call_budget = self._call_budget
        return child


@runtime_checkable
class ContextFactory(Protocol):
    """Protocol for creating Context from runtime-specific requests.

    Web framework integrations should implement this to extract Identity
    from HTTP requests (e.g., Django request.user, JWT tokens, API keys).

    Example:
        class DjangoContextFactory:
            def create_context(self, request: HttpRequest) -> Context:
                identity = Identity(
                    id=str(request.user.id),
                    type="user",
                    roles=list(request.user.groups.values_list("name", flat=True)),
                )
                return Context.create(identity=identity)
    """

    def create_context(self, request: Any) -> Context: ...
