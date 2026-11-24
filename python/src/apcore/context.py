"""Execution context, identity, and context logger."""

from __future__ import annotations

import logging
import uuid
from dataclasses import dataclass, field
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, Generic, Mapping, Protocol, TypeVar, runtime_checkable

from apcore.cancel import CancelToken

if TYPE_CHECKING:
    from apcore.observability.context_logger import ContextLogger
    from apcore.trace_context import TraceParent


__all__ = ["Context", "Identity", "ContextFactory", "GovernanceProjection"]

T = TypeVar("T")


@dataclass(frozen=True)
class Identity:
    """Caller identity (human/service/AI generic)."""

    id: str
    type: str = "user"
    roles: tuple[str, ...] = field(default_factory=tuple)
    attrs: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
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
        it is exactly 32 lowercase hex characters and not the W3C-reserved
        all-zero or all-f value. Otherwise a fresh 32-hex trace_id is
        generated and a WARN-level log is emitted. No normalization (dashed
        UUID stripping, case folding) is performed here; such normalization
        is the responsibility of the TraceParent header parser or the
        caller's ContextFactory.
        """
        if trace_parent is not None:
            hex_id = trace_parent.trace_id
            if (
                len(hex_id) == 32
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
        return cls(
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

    def _bind_executor(self, executor: Any) -> None:
        """Deprecated alias for :meth:`bind_executor`. Will be removed in a future major release."""
        import warnings

        warnings.warn(
            "Context._bind_executor is deprecated; use Context.bind_executor.",
            DeprecationWarning,
            stacklevel=2,
        )
        self.bind_executor(executor)

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
        if self.identity is not None:
            result["identity"] = {
                "id": self.identity.id,
                "type": self.identity.type,
                "roles": list(self.identity.roles),
                "attrs": dict(self.identity.attrs),
            }
        else:
            result["identity"] = None
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

        identity = None
        if data.get("identity") is not None:
            # CTX-1. This used to index `id_data["id"]` unguarded, so a
            # malformed wire identity left a bare `KeyError: 'id'` or
            # `TypeError: string indices must be integers` escaping a method
            # documented to "best-effort proceed" — neither is an apcore error,
            # neither carries a registry code, and neither is what a caller
            # catching ModuleError around a deserialize is prepared for.
            # apcore-typescript coerces every field and apcore-rust drops the
            # identity; both report rather than crash on a builtin.
            from apcore.errors import InvalidInputError

            id_data = data["identity"]
            if not isinstance(id_data, dict):
                raise InvalidInputError(
                    message=f"Context.deserialize: 'identity' must be an object, got {type(id_data).__name__}",
                )
            if not isinstance(id_data.get("id"), str) or not id_data["id"]:
                raise InvalidInputError(
                    message="Context.deserialize: 'identity.id' is required and must be a non-empty string",
                )
            roles = id_data.get("roles") or ()
            if isinstance(roles, (str, bytes)) or not isinstance(roles, (list, tuple)):
                raise InvalidInputError(
                    message="Context.deserialize: 'identity.roles' must be a list of strings",
                )
            attrs = id_data.get("attrs") or {}
            if not isinstance(attrs, dict):
                raise InvalidInputError(
                    message="Context.deserialize: 'identity.attrs' must be an object",
                )
            identity = Identity(
                id=id_data["id"],
                type=id_data.get("type", "user"),
                roles=tuple(roles),
                attrs=attrs,
            )

        return cls(
            trace_id=data.get("trace_id", ""),
            caller_id=data.get("caller_id"),
            call_chain=list(data.get("call_chain", [])),
            executor=None,
            identity=identity,
            redacted_inputs=data.get("redacted_inputs"),
            redacted_output=data.get("redacted_output"),
            data=dict(data.get("data", {})),
            services=None,  # type: ignore[arg-type]
            cancel_token=None,
        )

    @property
    def logger(self) -> ContextLogger:
        """DEPRECATED — use the host application's logger (apcore#121).

        Application and module code SHOULD log through the logger the host
        application has already configured. To emit automatic apcore execution
        events instead, install ``ObsLoggingMiddleware`` explicitly — that is a
        different facility, not a drop-in replacement for ad-hoc logging.

        This property has no configuration door and cannot acquire one without
        paying for it somewhere the framework should not: its logger is built
        per access from a ``Context``, which carries no ``Config`` in any SDK,
        and the only route that avoids threading one through ``Context``'s
        pinned six-parameter contract is a process-global logger — which would
        end the multi-instance isolation apcore currently gets for free. So the
        output is fixed at stderr / ``info`` / JSON, bypassing whatever the host
        has configured. Per PROTOCOL_SPEC §9.2.4's D-67 boundary, apcore does
        not own the host's logging policy.

        Removed at v2.0. Returns a logger carrying this context's ``trace_id``,
        ``module_id`` and ``caller_id`` until then.
        """
        import warnings

        warnings.warn(
            "Context.logger is deprecated and will be removed in version 2.0. "
            "Application and module code SHOULD use the host application's logger; "
            "its output is under the host's control, and this one's is not "
            "(always stderr, info, JSON). To emit automatic apcore execution events, "
            "install ObsLoggingMiddleware explicitly. See apcore#121.",
            DeprecationWarning,
            stacklevel=2,
        )
        from apcore.observability.context_logger import ContextLogger

        return ContextLogger.from_context(self, name=self.caller_id or "unknown")

    def child(self, target_module_id: str) -> Context[T]:
        """Create a child Context for calling a target module.

        The ``data`` dict is intentionally shared (not copied) between parent
        and child contexts.  Middleware such as TracingMiddleware and
        MetricsMiddleware rely on this shared reference to maintain span and
        timing stacks across nested module-to-module calls.
        """
        return Context(
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
