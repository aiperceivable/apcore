"""Module protocol and related data types."""

from __future__ import annotations

from dataclasses import dataclass, field

import logging
from typing import TYPE_CHECKING, Any, Protocol, runtime_checkable

from pydantic import BaseModel as _PydBaseModel, ConfigDict, model_validator

if TYPE_CHECKING:
    from pydantic import BaseModel

    from apcore.context import Context

__all__ = [
    "Change",
    "DEFAULT_ANNOTATIONS",
    "Module",
    "ModuleAnnotations",
    "ModuleExample",
    "PreflightCheckResult",
    "PreflightResult",
    "PreviewResult",
    "ValidationResult",
]

_logger = logging.getLogger(__name__)


@runtime_checkable
class Module(Protocol):
    """Protocol for apcore modules.

    Any class with ``input_schema``, ``output_schema``, ``description``,
    and an ``execute(inputs, context)`` method satisfies this protocol.
    Inheriting from ``Module`` is optional but provides IDE autocompletion.

    At runtime, ``@runtime_checkable`` only checks attribute existence.
    Static type checkers (pyright) will verify the full signature.

    Optional lifecycle hooks (detected via hasattr by the Registry/Executor):
    - ``on_load()`` — called after module is registered; raise to abort.
    - ``on_unload()`` — called after module is unregistered.
    - ``on_suspend()`` — called before hot-reload; returns optional state dict.
    - ``on_resume(state)`` — called after hot-reload; receives suspended state.
    - ``preflight(inputs, context)`` — custom validation before execution;
      return a ``PreflightResult``-compatible object.
    - ``preview(inputs, context)`` — structured prediction of state changes the
      call would produce. Return a ``PreviewResult`` (or ``None`` when prediction
      is unavailable). MUST NOT have side effects. Mirrors ``preflight()``
      exception semantics: a raised exception is folded into a warning on the
      ``module_preview`` advisory check rather than failing validation.
      Per protocol-spec §5.6.
    - ``stream(inputs, context)`` — async generator variant (streaming mode).

    Cross-language parity with apcore-typescript Module interface and
    apcore-rust Module trait (sync finding A-017).
    """

    input_schema: type[BaseModel]
    output_schema: type[BaseModel]
    description: str

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]: ...


_CANONICAL_FIELDS = {
    "readonly",
    "destructive",
    "idempotent",
    "requires_approval",
    "open_world",
    "streaming",
    "cacheable",
    "cache_ttl",
    "cache_key_fields",
    "paginated",
    "pagination_style",
    "discoverable",
    "extra",
}


@dataclass(frozen=True)
class ModuleAnnotations:
    """Behavioral annotations for a module.

    Attributes:
        readonly: Whether the module only reads data (no side effects).
        destructive: Whether the module may irreversibly modify data.
        idempotent: Whether repeated calls produce the same result.
        requires_approval: Whether THIS MODULE requires human approval before
            execution. A module-level declaration, NOT the governance-effective
            value: ``False`` means this module asks for none, which does not mean
            none will be required. An ACL rule (PROTOCOL_SPEC §6.1.6), an
            ``ExecutionPolicy`` override or ``gate_destructive`` may require one
            for a particular call (§6.9 rows 3-5). The effective value is call-site
            dependent — read it from :meth:`Executor.validate` (§7.9.5).
        open_world: Whether the module interacts with external systems.
        streaming: Whether the module supports streaming execution.
        cacheable: Whether the module's results can be cached.
        cache_ttl: Cache time-to-live in seconds (0 means no expiry).
        cache_key_fields: Input fields used to compute the cache key (None = all).
        paginated: Whether the module supports paginated results.
        pagination_style: Pagination strategy (default "cursor"). Accepts any string.
        discoverable: Whether the module appears in enumeration surfaces
            (``Registry.list``, manifest export, etc.). Default ``True``.
            ``ephemeral.*`` modules SHOULD set this to ``False`` per
            protocol-spec §2.5.1. Hidden modules remain callable
            through ``Registry.get`` / ``Executor.execute`` when the caller
            already knows the module ID.
        extra: Extension dictionary for ecosystem package metadata.
    """

    readonly: bool = False
    destructive: bool = False
    idempotent: bool = False
    requires_approval: bool = False
    open_world: bool = True
    streaming: bool = False
    cacheable: bool = False
    cache_ttl: int = 0
    cache_key_fields: tuple[str, ...] | None = None
    paginated: bool = False
    pagination_style: str = "cursor"
    discoverable: bool = True
    extra: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        """Enforce immutability constraints on frozen dataclass."""
        # Convert list to tuple for cache_key_fields
        if isinstance(self.cache_key_fields, list):
            object.__setattr__(self, "cache_key_fields", tuple(self.cache_key_fields))
        # D-115: a malformed value is TOLERATED and dropped, the rest survives,
        # and it warns. `extra` is declared an object; a string there is neither
        # an object nor a reason to discard the module.
        #
        # `from_dict` already coerced a non-dict `extra` to `{}` — and
        # `merge_annotations` builds this dataclass directly, where
        # `dict("oops")` raised a bare ValueError that escapes every
        # `except ModuleError` handler and took the whole registration with it.
        # The tolerance belongs HERE, the one point every construction path goes
        # through, rather than in one door of two.
        if not isinstance(self.extra, dict):
            _logger.warning(
                "ModuleAnnotations.extra must be an object, got %s; dropping it (D-115)",
                type(self.extra).__name__,
            )
            object.__setattr__(self, "extra", {})
        else:
            # Shallow copy to detach from the caller's mutable dict.
            object.__setattr__(self, "extra", dict(self.extra))
        # Validate cache_ttl >= 0
        if not isinstance(self.cache_ttl, int) or isinstance(self.cache_ttl, bool):
            _logger.warning(
                "cache_ttl must be an integer, got %s; dropping it (D-115)",
                type(self.cache_ttl).__name__,
            )
            object.__setattr__(self, "cache_ttl", 0)
        elif self.cache_ttl < 0:
            _logger.warning("cache_ttl %d is negative, clamping to 0", self.cache_ttl)
            object.__setattr__(self, "cache_ttl", 0)

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> ModuleAnnotations:
        """Deserialize from dict per PROTOCOL_SPEC §4.4.1 wire format.

        - The canonical form carries extension data under a nested ``extra`` object.
        - Legacy top-level overflow keys (unknown keys at the annotations root) are
          tolerated for backward compatibility and merged into ``extra``.
        - When the same key appears in BOTH the nested ``extra`` AND as a top-level
          overflow key, the nested value wins (§4.4.1 rule 7).
        """
        known = {k: v for k, v in data.items() if k in _CANONICAL_FIELDS}
        overflow = {k: v for k, v in data.items() if k not in _CANONICAL_FIELDS}
        explicit_extra = known.pop("extra", {})
        if not isinstance(explicit_extra, dict):
            explicit_extra = {}
        # §4.4.1 rule 7: nested explicit `extra` wins over legacy top-level overflow.
        extra: dict[str, Any] = {**overflow, **explicit_extra}
        return cls(**known, extra=extra)


DEFAULT_ANNOTATIONS = ModuleAnnotations()


@dataclass
class ModuleExample:
    """An example invocation of a module.

    Attributes:
        title: Short title for the example.
        inputs: Example input data dict.
        output: Expected output data dict.
        description: Optional description of the example.
    """

    title: str
    inputs: dict[str, Any] = field(default_factory=dict)
    output: dict[str, Any] = field(default_factory=dict)
    description: str | None = None


@dataclass
class ValidationResult:
    """Result of input validation.

    Attributes:
        valid: Whether validation passed.
        errors: List of error dicts, each with 'field', 'code', 'message' keys.
    """

    valid: bool
    errors: list[dict[str, str]] = field(default_factory=list)


@dataclass(frozen=True)
class PreflightCheckResult:
    """Result of a single preflight check.

    Attributes:
        check: Check name ("module_id", "module_lookup", "call_chain", "acl",
            "approval", "schema", "module_preflight").
        passed: Whether this check passed.
        error: Error details when passed is False; None when passed is True.
        warnings: Non-fatal issues that do not prevent execution.
    """

    check: str
    passed: bool
    error: dict[str, Any] | None = None
    warnings: list[str] = field(default_factory=list)


class Change(_PydBaseModel):
    """Structured prediction of a single state change a module call would produce.

    Per protocol-spec §12.8.5.1:

    - ``action`` is a free-form verb describing the kind of change ("write",
      "delete", "send", ...). Module authors define their own taxonomy.
    - ``target`` is a free-form identifier of what is changed.
    - ``summary`` is the required human-readable single-line description — the
      floor that even modules wrapping opaque external APIs can produce.
    - ``before`` / ``after`` are optional and module-class specific.

    Extension keys MUST follow the ``^x-`` convention (consistent with §4.6
    metadata extensions). Extra keys without the ``x-`` prefix are rejected at
    construction time. This is the Python idiomatic encoding called out in the
    RFC's "Change.x-* extension fields" cross-SDK schema-encoding table:
    ``ConfigDict(extra='allow')`` + a model-validator that asserts the prefix.
    """

    model_config = ConfigDict(extra="allow")

    action: str
    target: str
    summary: str
    before: Any | None = None
    after: Any | None = None

    @model_validator(mode="after")
    def _validate_extension_keys(self) -> Change:
        # Pydantic stashes extras on .model_extra; enforce the ^x- pattern.
        extras = self.model_extra or {}
        for key in extras:
            if not key.startswith("x-"):
                raise ValueError(
                    f"Unknown field {key!r} on Change; non-'x-*' extension keys "
                    "are forbidden (see protocol-spec §4.6 / §12.8.5.1)."
                )
        return self


class PreviewResult(_PydBaseModel):
    """Module's prediction of what would change if a call were executed.

    Returned by the optional ``Module.preview()`` method; folded into
    ``PreflightResult.predicted_changes`` by ``Executor.validate()`` when the
    target module implements the method.
    """

    changes: list[Change]


@dataclass
class PreflightResult:
    """Result of Executor.validate() preflight check.

    Duck-type compatible with ValidationResult: has ``valid`` and ``errors``
    properties so existing consumers continue to work.

    Attributes:
        valid: True only if all checks passed.
        checks: Per-step check results.
        requires_approval: The GOVERNANCE-EFFECTIVE requirement for this call
            (PROTOCOL_SPEC §7.9.5) — the union of the module annotation, an ACL
            rule carrying ``approval``, an ``ExecutionPolicy`` override and
            ``gate_destructive`` (§6.9 rows 3-5). NOT the annotation alone: since
            spec v1.28.0 a module declaring ``requires_approval=False`` still
            reports ``True`` here when the ACL requires a human for the arguments
            this call carries. That union is by construction the same verdict the
            approval gate will enforce.
        predicted_changes: Optional structured prediction of state changes the
            call would produce. Populated when ``Executor.validate()`` runs
            against a module that implements ``preview()`` and the method
            returns a :class:`PreviewResult`. Empty otherwise (module does not
            implement ``preview()``, returned ``None``, or ``preview()`` raised
            — in the last case a warning is emitted on the ``module_preview``
            advisory check, mirroring ``preflight()`` semantics).
            Per PROTOCOL_SPEC §12.8.
    """

    valid: bool
    checks: list[PreflightCheckResult] = field(default_factory=list)
    requires_approval: bool = False
    predicted_changes: list[Change] = field(default_factory=list)

    @property
    def errors(self) -> list[dict[str, Any]]:
        """All failed check errors, compatible with ValidationResult.errors."""
        return [c.error for c in self.checks if not c.passed and c.error is not None]
