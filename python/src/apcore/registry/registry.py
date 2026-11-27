"""Central module registry for discovering, registering, and querying modules."""

from __future__ import annotations

import asyncio
import inspect
import json
import logging
import re
import threading
import time
from collections.abc import Iterable
from contextlib import contextmanager
from contextvars import ContextVar
from copy import deepcopy
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING, Any, Callable, Iterator, Literal, Protocol, runtime_checkable

from apcore.config import Config
from apcore.cancel import CancelToken
from apcore.errors import (
    DependencyVersionMismatchError,
    ErrorCodes,
    InvalidInputError,
    ModuleError,
    ModuleLoadError,
    ModuleNotFoundError,
    StreamingInterfaceError,
)
from apcore.lifecycle import (
    DrainAbortedContext,
    DrainContext,
    LifecycleError,
    LifecycleOperation,
    ReloadOptions,
    UnregisterOptions,
    _AbortReason,
    _HostDrainAction,
    _ReloadDrainAction,
    _lifecycle_callback,
    _require_lifecycle_loop,
    _validate_drain_options,
    _validate_duration,
)
from apcore.registry.conflicts import ConflictSeverity, detect_id_conflicts
from apcore.registry.dependencies import resolve_dependencies
from apcore.registry.entry_point import resolve_entry_point
from apcore.schema.annotations import governance_union
from apcore.registry.metadata import (
    load_id_map,
    load_metadata,
    merge_module_metadata,
    parse_dependencies,
)
from apcore.registry.scanner import scan_extensions, scan_multi_root, scan_watched_source
from apcore.registry.types import DependencyInfo, DiscoveredModule, ModuleDescriptor
from apcore.registry.validation import validate_module
from apcore.registry.version import VersionedStore, matches_version_hint

if TYPE_CHECKING:
    from apcore.config import Config

logger = logging.getLogger(__name__)
_HookLoopFor = Callable[[str], asyncio.AbstractEventLoop | None]


def _has_running_loop() -> bool:
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return False
    return True


@dataclass(frozen=True)
class _ModuleRegistration:
    module: Any
    metadata: dict[str, Any]
    descriptor_metadata: dict[str, Any]
    hook_loop: asyncio.AbstractEventLoop | None = None


@dataclass(frozen=True)
class _RegistrationSnapshot:
    module: Any
    metadata: dict[str, Any]
    versions: dict[str, _ModuleRegistration]
    source_path: Path | None

    def instances(self) -> Iterator[Any]:
        """Yield the latest instance first, then each distinct older instance."""
        seen = {id(self.module)}
        yield self.module
        for registration in reversed(tuple(self.versions.values())):
            if id(registration.module) not in seen:
                seen.add(id(registration.module))
                yield registration.module

    def hook_loop(self, module: Any) -> asyncio.AbstractEventLoop | None:
        for registration in reversed(tuple(self.versions.values())):
            if registration.module is module:
                return registration.hook_loop
        return None


@dataclass
class _WatchedReplacement:
    generation: object
    registration: _RegistrationSnapshot
    source: Path
    revision: tuple[int, int, int, int] | None
    refresh: bool = False


@dataclass
class _PendingUnload:
    generation: object
    module_id: str
    registration: _RegistrationSnapshot
    context: Any
    remaining_leases: int
    finished: threading.Event = field(default_factory=threading.Event)
    active_hook: bool = False
    active_at_unload: int | None = None
    drain_timed_out: bool = False
    residuals: threading.Event | None = None
    unload_finished: bool = False
    error: LifecycleError | None = None


@dataclass
class _DrainProgress:
    finished: bool = False
    error: LifecycleError | None = None


@dataclass(frozen=True)
class _DiscoveryCandidate:
    source: DiscoveredModule
    entry_point: type
    metadata: dict[str, Any]


def _canonical_deprecation(deprecation: dict[str, Any]) -> str:
    """Canonical rendering of an ``x-deprecation`` block, for the D-89 key.

    Part of the deprecation-warning dedupe key (spec v1.59.0), so that two
    structurally equal notices dedupe regardless of how they were built and a
    CHANGED notice on a re-registered module warns again. Falls back to ``repr``
    for a value JSON cannot render, which is stable enough for an equality key
    and never raises on the read path.
    """
    try:
        return json.dumps(deprecation, sort_keys=True, separators=(",", ":"), default=repr)
    except (TypeError, ValueError):
        return repr(sorted(deprecation.items(), key=lambda kv: kv[0]))


class _DictSchemaAdapter:
    """Adapts a plain JSON Schema dict to the Pydantic model class interface.

    Allows modules that define ``input_schema`` / ``output_schema`` as raw
    dicts to work transparently with the executor, schema exporter, and any
    other code that calls ``model_validate``, ``model_json_schema``, or
    ``model_rebuild`` on a schema object.

    ``model_validate`` validates *data* against the declared schema, so a dict
    schema is the same contract a Pydantic model would be. It used to be a
    pass-through, justified in a comment by ``jsonschema`` "not currently
    declared" -- stale since it became a hard dependency. The effect was that
    every constraint a dict schema declared was inert in apcore-python:
    ``required``, ``enum``, ``minimum`` and ``type`` alike, on user modules and
    on all nine ``system.*`` modules. apcore-typescript (``validateSchema``) and
    apcore-rust (``validate_against_schema``) both enforced them, so the same
    module contract meant two different things depending on the SDK.
    """

    def __init__(self, schema: dict[str, Any]) -> None:
        self._schema = schema

    def model_json_schema(self) -> dict[str, Any]:
        return self._schema

    def model_validate(self, data: Any, *, strict: bool | None = None) -> Any:
        """Validate *data* against the declared dict schema.

        ``strict`` is accepted for signature compatibility with
        ``BaseModel.model_validate`` -- the module-invocation boundary passes
        ``strict=True`` (TYPE_MAPPING §17.3) -- and is not a knob: this path
        never coerces, so it is already strict.

        Raises:
            SchemaValidationError: with wire code ``SCHEMA_VALIDATION_ERROR``,
                the same code the Pydantic path produces, so a caller cannot
                tell which way a module declared its schema.
        """
        from apcore.schema.hardening import validate_schema_dict

        result = validate_schema_dict(data, self._schema)
        if not result.valid:
            raise result.to_error()
        return data

    def model_rebuild(self) -> None:
        pass


def _ensure_schema_adapter(module: Any, *, normalize_legacy_none: bool = False) -> None:
    """Adapt raw JSON Schema and explicit legacy permissive declarations."""
    for attr in ("input_schema", "output_schema"):
        value = getattr(module, attr, None)
        if value is None and normalize_legacy_none:
            value = {"type": "object"}
        if isinstance(value, dict):
            setattr(module, attr, _DictSchemaAdapter(value))


REGISTRY_EVENTS: dict[str, str] = {
    "REGISTER": "register",
    "UNREGISTER": "unregister",
}

MODULE_ID_PATTERN = re.compile(r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$")


@runtime_checkable
class Discoverer(Protocol):
    """Protocol for custom module discovery."""

    def discover(self, roots: list[str]) -> list[dict[str, Any]]:
        """Discover modules from the given root directories.

        Returns:
            List of dicts with at least 'module_id' and 'module' keys.
        """
        ...


@runtime_checkable
class ModuleValidator(Protocol):
    """Protocol for custom module validation."""

    def validate(self, module: Any) -> list[str]:
        """Validate a module.

        Returns:
            List of error strings. Empty list means valid.
        """
        ...


MAX_MODULE_ID_LENGTH = 192

# Default version applied to modules registered (manually or via discover()) without
# an explicit `version=` argument or `version` class/instance attribute. Aligned
# across `Registry.register`, `_resolve_load_order`, `_register_in_order`, and
# `ModuleDescriptor.version` default so all registration paths produce
# equivalent state. Changing this is a BREAKING change — callers relying on a
# specific unset marker should supply `version="0.0.0"` explicitly.
DEFAULT_MODULE_VERSION = "1.0.0"

RESERVED_WORDS = frozenset({"system", "internal", "core", "apcore", "plugin", "schema", "acl"})

# Namespace reserved for programmatically-registered modules synthesized at
# runtime (see protocol-spec §2.5.1). IDs in this
# namespace MUST be registered through :meth:`Registry.register` only; the
# filesystem discoverer rejects matching IDs because the namespace has no
# directory-rooted source of truth. The trailing dot is required so module IDs
# whose first segment merely *starts with* ``ephemeral`` (e.g. ``ephemerals``)
# are not falsely classified.
EPHEMERAL_NAMESPACE_PREFIX = "ephemeral."

__all__ = [
    "Registry",
    "REGISTRY_EVENTS",
    "MODULE_ID_PATTERN",
    "MAX_MODULE_ID_LENGTH",
    "DEFAULT_MODULE_VERSION",
    "RESERVED_WORDS",
    "EPHEMERAL_NAMESPACE_PREFIX",
    "Discoverer",
    "ModuleValidator",
]


_EXPECTED_STREAM_SIGNATURE = "(self, inputs: dict, context: Context) -> AsyncIterator[dict]"


def _validate_streaming_signature(module_id: str, module: Any) -> None:
    """Raise StreamingInterfaceError when module.stream() doesn't match the Protocol.

    Called only when the module's annotations declare ``streaming = True``.
    """
    stream_fn = getattr(module, "stream", None)
    if stream_fn is None or not callable(stream_fn):
        raise StreamingInterfaceError(
            module_id=module_id,
            expected_signature=_EXPECTED_STREAM_SIGNATURE,
            actual_signature="<missing>",
            mismatch_reason="missing_marker",
        )

    if not inspect.iscoroutinefunction(stream_fn) and not inspect.isasyncgenfunction(stream_fn):
        try:
            sig = str(inspect.signature(stream_fn))
        except (ValueError, TypeError):
            sig = "<unknown>"
        raise StreamingInterfaceError(
            module_id=module_id,
            expected_signature=_EXPECTED_STREAM_SIGNATURE,
            actual_signature=sig,
            mismatch_reason="not_async",
        )

    try:
        params = list(inspect.signature(stream_fn).parameters.keys())
    except (ValueError, TypeError):
        params = []

    # Bound method: self is already consumed by descriptor protocol; inspect
    # returns the unbound params list when inspecting the function via the class,
    # but ``getattr(instance, 'stream')`` gives a bound method where 'self' is
    # absent from the visible parameter list.
    expected_params = {"inputs", "context"}
    if not expected_params.issubset(set(params)):
        try:
            actual_sig = str(inspect.signature(stream_fn))
        except (ValueError, TypeError):
            actual_sig = "<unknown>"
        raise StreamingInterfaceError(
            module_id=module_id,
            expected_signature=_EXPECTED_STREAM_SIGNATURE,
            actual_signature=actual_sig,
            mismatch_reason="wrong_arity",
        )

    # NOTE: the spec mismatch_reason literal also includes "wrong_return_type".
    # Return-type annotation validation is intentionally skipped here: Python
    # annotations are not enforced at runtime, AsyncIterator[dict] cannot be
    # reliably introspected from inspect.signature() for async generators, and
    # the @runtime_checkable Protocol already validates structural presence.
    # The "not_async" and "wrong_arity" checks cover the practically important
    # mismatches; "wrong_return_type" would only fire on annotation-only errors.


def _validate_streaming_annotation(module_id: str, module: Any, annotations: object) -> None:
    """Require a compatible stream method when the module declares streaming."""
    if isinstance(annotations, dict):
        streaming_declared = annotations.get("streaming") is True
    else:
        streaming_declared = getattr(annotations, "streaming", None) is True
    if streaming_declared:
        _validate_streaming_signature(module_id, module)


def _prepare_discovered_module(module_id: str, module: Any, metadata: dict[str, Any]) -> dict[str, Any]:
    merged_metadata = merge_module_metadata(module, metadata)
    _validate_streaming_annotation(module_id, module, merged_metadata["annotations"])
    _ensure_schema_adapter(module)
    return merged_metadata


def _is_ephemeral(module_id: str) -> bool:
    """Return True when ``module_id`` belongs to the reserved ephemeral.* namespace."""
    return module_id == "ephemeral" or module_id.startswith(EPHEMERAL_NAMESPACE_PREFIX)


def _validate_module_id(module_id: str, *, allow_reserved: bool = False) -> None:
    """Validate a module ID against PROTOCOL_SPEC §2.7 in canonical order.

    Order: empty → pattern → length → reserved (first-segment).
    Duplicate detection is the caller's responsibility (it requires registry
    state).

    Args:
        module_id: Candidate module ID to validate.
        allow_reserved: When True, the first-segment reserved word check is
            skipped — used by ``Registry.register_internal`` to allow sys
            modules to use the ``system.*`` prefix. All other validations
            (empty, pattern, length) still apply.

    Raises:
        InvalidInputError: On any validation failure.

    Aligned with ``apcore-typescript._validateModuleId`` and
    ``apcore::registry::registry::validate_module_id``.
    """
    # 1. empty check
    if not isinstance(module_id, str) or not module_id:
        raise InvalidInputError(
            message="module_id must be a non-empty string",
            code=ErrorCodes.INVALID_MODULE_ID,
        )

    # 2. EBNF pattern check
    if not MODULE_ID_PATTERN.fullmatch(module_id):
        raise InvalidInputError(
            f"Invalid module ID: '{module_id}'. Must match pattern: "
            f"{MODULE_ID_PATTERN.pattern} (lowercase, digits, underscores, dots only; no hyphens)",
            code=ErrorCodes.INVALID_MODULE_ID,
        )

    # 3. length check
    if len(module_id) > MAX_MODULE_ID_LENGTH:
        raise InvalidInputError(
            f"Module ID exceeds maximum length of {MAX_MODULE_ID_LENGTH}: {len(module_id)}",
            code=ErrorCodes.INVALID_MODULE_ID,
        )

    # 4. reserved word first-segment check (skipped for register_internal)
    if not allow_reserved:
        first_segment = module_id.split(".")[0]
        if first_segment in RESERVED_WORDS:
            raise InvalidInputError(
                f"Module ID contains reserved word: '{first_segment}'",
                code=ErrorCodes.INVALID_MODULE_ID,
            )


def _run_on_load(module: Any, module_id: str) -> None:
    """Invoke ``module.on_load()``, refusing an awaitable rather than dropping it.

    ``on_load`` is synchronous in this SDK and in apcore-rust, where the trait
    signature enforces it. apcore-typescript accepts an async ``onLoad`` and has
    ``register`` return a promise that resolves once it completes.

    A module author following the TypeScript shape used to get silence here: the
    coroutine was created, never awaited, and discarded, so the module was
    published and callable with none of its initialisation having run. The only
    trace was a ``RuntimeWarning`` on the next garbage collection, attributed to
    whatever code happened to be running then. That is exactly the
    half-initialised module the deferred-publish design exists to prevent, and it
    arrived through the one path that skipped the check (sync finding A-C-002).

    Raising is the smaller change it looks like: an ``async def on_load`` has
    never once run, so nothing can depend on it having done so.
    """
    result = module.on_load()
    if inspect.isawaitable(result):
        # Close it so the refusal does not also produce the "never awaited"
        # warning, attributed to an unrelated call site.
        if inspect.iscoroutine(result):
            result.close()
        raise ModuleError(
            code=ErrorCodes.MODULE_LOAD_ERROR,
            message=(
                f"Module '{module_id}' defines an async on_load(); apcore-python "
                "invokes on_load synchronously, so an awaitable one never runs. "
                "Make it a regular def, or move the async work into execute()."
            ),
            ai_guidance=(
                "Change `async def on_load(self)` to `def on_load(self)`. If the "
                "initialisation genuinely needs an event loop, do it lazily on "
                "first execute() instead — Registry.register is a synchronous API "
                "and cannot await."
            ),
        )


_LIFECYCLE_HOOKS = ("on_load", "on_unload", "on_suspend", "on_resume", "on_drain", "on_drain_aborted")


def _call_lifecycle_hook(
    hook: Callable[..., Any],
    arguments: tuple[Any, ...],
    loop: asyncio.AbstractEventLoop | None,
    module_id: str,
    name: str,
) -> Any:
    result = hook(*arguments)
    if not inspect.isawaitable(result):
        return result
    if loop is None or not loop.is_running():
        if inspect.iscoroutine(result):
            result.close()
        raise ModuleLoadError(module_id, f"Async {name} requires an available lifecycle event loop")
    if _has_running_loop() and asyncio.get_running_loop() is loop:
        if inspect.iscoroutine(result):
            result.close()
        raise ModuleLoadError(module_id, f"Async {name} cannot be joined synchronously from its lifecycle event loop")

    async def await_hook() -> Any:
        return await result

    coroutine = await_hook()
    try:
        future = asyncio.run_coroutine_threadsafe(coroutine, loop)
    except RuntimeError as error:
        coroutine.close()
        if inspect.iscoroutine(result):
            result.close()
        raise ModuleLoadError(module_id, f"Lifecycle event loop is unavailable for {name}") from error
    while True:
        try:
            return future.result(timeout=0.1)
        except TimeoutError:
            if future.done():
                return future.result()
            if not loop.is_running():
                future.cancel()
                raise ModuleLoadError(module_id, f"Lifecycle event loop stopped during {name}") from None


class Registry:
    """Central module registry for discovering, registering, and querying modules."""

    def __init__(
        self,
        config: Config | None = None,
        extensions_dir: str | None = None,
        extensions_dirs: list[str | dict] | None = None,
        id_map_path: str | None = None,
        metrics_collector: Any = None,
        pre_approval_hook: Callable[[Path], None] | None = None,
    ) -> None:
        """Initialize the Registry.

        Args:
            config: Optional Config object for framework-wide settings.
            extensions_dir: Single extensions directory path.
            extensions_dirs: List of extension root configs (mutually exclusive with extensions_dir).
            id_map_path: Path to ID Map YAML file for overriding canonical IDs.
            metrics_collector: Optional MetricsCollector. When provided, the
                registry increments ``apcore_registry_callback_errors_total`` with
                labels ``{event, module_id, error_type}`` each time an event
                callback raises, giving ops a per-event error signal beyond
                the process-local counter exposed by ``get_callback_errors()``.
            pre_approval_hook: Optional callable invoked with the file path
                of every extension Python file BEFORE it is imported. The
                hook may raise to reject the file (wrapped as
                ``ModuleLoadError``). Use for signature verification,
                hash-based allowlists, or audit logging of extension loads.
                Applies to both discovery and hot-reload paths.

        Raises:
            InvalidInputError: If both extensions_dir and extensions_dirs are specified.
        """
        if extensions_dir is not None and extensions_dirs is not None:
            raise InvalidInputError(message="Cannot specify both extensions_dir and extensions_dirs")

        # Determine extension roots: individual params > config > defaults
        if extensions_dir is not None:
            self._extension_roots: list[dict[str, Any]] = [{"root": extensions_dir}]
        elif extensions_dirs is not None:
            self._extension_roots = [{"root": item} if isinstance(item, str) else item for item in extensions_dirs]
        elif config is not None:
            # `extensions.roots` before `extensions.root`: §9.1.1 declares both
            # and `$defs/ExtensionsConfig` puts them in exclusive `oneOf`
            # branches, so a document carrying `roots` is in multi-root mode and
            # `root` is only its single-root sibling.
            #
            # apcore#118 decision D-70. This key was read by apcore-rust alone,
            # so a multi-root project worked on one SDK of three, silently.
            # `scan_multi_root` below has always been able to do the work —
            # including the namespace prefixing `roots` exists for — and nothing
            # extracted the list from a `Config` to reach it.
            self._extension_roots = self._roots_from_config(config)
        else:
            self._extension_roots = [{"root": Config.get_default("extensions.root")}]

        # Internal state
        self._modules: dict[str, Any] = {}
        self._module_meta: dict[str, dict[str, Any]] = {}
        self._module_sources: dict[str, Path] = {}
        self._lowercase_map: dict[str, str] = {}
        # Versioned storage for multi-version module support (F18)
        self._versioned_modules: VersionedStore[_ModuleRegistration] = VersionedStore()
        self._callbacks: dict[str, list[Callable[..., Any]]] = {
            REGISTRY_EVENTS["REGISTER"]: [],
            REGISTRY_EVENTS["UNREGISTER"]: [],
        }
        # Per-event counter of callback exceptions — exposed via
        # `get_callback_errors()` so ops can watch for registry-event-handler
        # health. Callbacks themselves remain fire-and-forget (errors are
        # logged + suppressed) to keep the registry's register/unregister
        # contract crash-free.
        self._callback_errors: dict[str, int] = {
            REGISTRY_EVENTS["REGISTER"]: 0,
            REGISTRY_EVENTS["UNREGISTER"]: 0,
        }
        self._lock = threading.RLock()
        self._id_map: dict[str, dict[str, Any]] = {}
        self._schema_cache: dict[str, dict[str, Any]] = {}
        self._config = config
        self._metrics_collector = metrics_collector
        self._pre_approval_hook = pre_approval_hook
        self._custom_discoverer: Discoverer | None = None
        self._custom_validator: ModuleValidator | None = None

        # D-89: deprecation warnings are emitted at most once per
        # ``(module_id, version)`` per registry instance. ``get_definition``
        # is a read that hosts call in loops, so warning per read produces
        # log spam proportional to traffic — which is how operators learn to
        # filter the advisory out. Mirrors apcore-typescript's
        # ``_deprecationWarned``. Entries for a module are dropped when it
        # leaves the registry (``unregister`` / hot reload) so a genuinely
        # new deprecation block on the replacement is not masked.
        self._deprecation_warned: set[tuple[str, str, str]] = set()

        # Optional EventEmitter wired in by callers that want registry-level
        # audit events for ``ephemeral.*`` registrations (RFC pilot). When
        # unset, ephemeral audit events are emitted to the standard logger
        # instead so they are never silently dropped.
        self._event_emitter: Any = None

        # Safe hot-reload state (F09 / Algorithm A21)
        self._ref_counts: dict[str, int] = {}
        self._lease_generations: dict[str, object] = {}
        self._invocation_tokens: dict[object, set[CancelToken]] = {}
        self._draining: set[str] = set()
        self._drain_events: dict[str, threading.Event] = {}
        self._pending_unloads: dict[object, _PendingUnload] = {}
        self._closed = False
        self._lifecycle_changed = threading.Condition(self._lock)
        self._shutdown_operation: LifecycleOperation | None = None
        self._shutdown_rows: dict[object, int] = {}
        self._shutdown_drain_timeout_ms = 5000
        self._shutdown_legacy_force = False
        self._shutdown_cancel_grace_ms: int | None = None
        self._unavailable: set[str] = set()
        self._watch_lifecycle: ReloadOptions | None = None
        self._watch_hook_loop: asyncio.AbstractEventLoop | None = None
        self._watched_replacements: dict[Path, _WatchedReplacement] = {}
        self._reload_state: ContextVar[tuple[str, dict[str, Any]] | None] = ContextVar("reload_state", default=None)

        # Deferred-publish: module IDs currently executing on_load (not yet visible).
        # Concurrent same-ID registrations see the in-flight set and raise DUPLICATE_MODULE_ID.
        self._in_flight: set[str] = set()

        # Load the ID map: explicit argument > `id_map.overrides` > none.
        #
        # PROTOCOL_SPEC §9.1.1 declares `id_map.overrides` and nothing read it
        # (apcore#118, decision D-71). The MECHANISM was implemented in all
        # three SDKs — this class's `_apply_id_map_overrides` is stage 2 of
        # discovery — and the map arrived only through this constructor's
        # `id_map_path`. Measured before the fix: with `id_map.overrides`
        # pointing at a map that renames `executor/orig/mod.py`, discovery still
        # registered `executor.orig.mod`.
        #
        # The precedence is D-73's, and the same one `extensions.root` follows
        # above: an API argument beats `Config`.
        #
        # Config paths share the project root (§9.2.2).
        resolved_id_map = id_map_path
        if resolved_id_map is None and config is not None:
            from_config = config.get("id_map.overrides")
            if isinstance(from_config, str) and from_config.strip():
                resolved_id_map = config.resolve_path(from_config)
        if resolved_id_map is not None:
            self._id_map = load_id_map(resolved_id_map)

    @staticmethod
    def _roots_from_config(config: Config) -> list[dict[str, Any]]:
        """`extensions.roots` if declared, else `extensions.root`, else the default.

        Both element shapes `$defs/ExtensionsConfig` admits are accepted: a bare
        path string and an object carrying an explicit `namespace`.

        **Every entry gets a namespace here**, derived from the last path
        segment when the entry does not name one — the same value
        `scan_multi_root` would derive. Deriving it at this door rather than
        leaving it to the scanner is what makes a ONE-element `roots` list
        behave like an n-element one: the scan dispatches on "more than one root
        or any namespace", so a lone `roots: ["./beta"]` would otherwise take
        the single-root branch and prefix nothing. Nothing in the schema makes a
        one-element list special, and `roots` versus `root` is the whole
        distinction between namespaced and backward-compatible mode.

        A `roots` list that is present but yields no usable entry falls through
        to the single-root branch rather than scanning nothing: an empty list is
        a configuration that discovers no modules, and silently discovering none
        is the failure this whole audit is about.
        """
        declared = config.get("extensions.roots")
        if isinstance(declared, list):
            entries: list[dict[str, Any]] = []
            for item in declared:
                if isinstance(item, str) and item.strip():
                    entries.append({"root": config.resolve_path(item), "namespace": Path(item).name})
                elif isinstance(item, dict) and item.get("root"):
                    root_value = item["root"]
                    entries.append(
                        {
                            "root": config.resolve_path(str(root_value)),
                            "namespace": item.get("namespace") or Path(str(root_value)).name,
                        }
                    )
            if entries:
                return entries
            if declared:
                logger.warning(
                    "extensions.roots is declared with %d entry/entries and none of them "
                    "names a root, so it was ignored and extensions.root is used instead. "
                    "An entry is either a path string or an object with a 'root' key.",
                    len(declared),
                )

        ext_root = config.get("extensions.root") or Config.get_default("extensions.root")
        return [{"root": config.resolve_path(ext_root)}]

    # ----- Custom Discoverer / Validator -----

    def set_discoverer(self, discoverer: Discoverer) -> None:
        """Set a custom module discoverer."""
        self._custom_discoverer = discoverer

    def set_validator(self, validator: ModuleValidator) -> None:
        """Set a custom module validator."""
        self._custom_validator = validator

    def set_event_emitter(self, emitter: Any) -> None:
        """Wire an :class:`apcore.events.EventEmitter` for registry audit events.

        When set, ``ephemeral.*`` registrations (and their unregistrations)
        emit ``apcore.registry.module_registered`` /
        ``apcore.registry.module_unregistered`` events whose ``data`` payload
        mirrors the D-35 contextual-auditing shape used by
        ``system.control.*`` modules: ``caller_id`` (defaulting to
        ``"@external"`` when no identity is attached) plus a redacted
        identity snapshot when ``context.identity`` is set on the call site.

        Pilot scope (protocol-spec §2.5.1):
        only ``ephemeral.*`` registrations trigger registry-side emits. The
        ``_bridge_registry_events`` helper in ``apcore.sys_modules.registration``
        continues to emit canonical events for all module registrations via
        the callback pathway.
        """
        self._event_emitter = emitter

    # ----- Discovery -----

    def discover(
        self,
        path_filter: str | list[str] | None = None,
    ) -> int:
        """Discover and register modules from configured extension directories.

        If a custom discoverer is set via ``set_discoverer()``, it is used
        instead of the default file-system scanning logic.  If a custom
        validator is set via ``set_validator()``, it replaces the built-in
        ``validate_module()`` check.

        Args:
            path_filter: Optional glob pattern (string) or list of patterns
                applied to each candidate module file's path.  Only files
                whose path matches at least one pattern are walked through
                discovery (Issue #45 §4 — granular reload).  Already-
                registered modules outside the filter are *not* touched.
                Patterns are matched against both the absolute file path
                and its path relative to the extension root, using
                ``pathlib.PurePath.match``.

        Returns:
            Number of modules successfully registered in this discovery pass.

        Raises:
            CircularDependencyError: If circular dependencies detected among modules.
            ConfigNotFoundError: If a configured extension root does not exist.
        """
        return self._discover(path_filter)

    async def discover_async(self, path_filter: str | list[str] | None = None) -> int:
        """Discover and register modules with lifecycle hooks on the current loop."""
        loop = asyncio.get_running_loop()
        return await asyncio.to_thread(self._discover, path_filter, hook_loop_for=lambda _: loop)

    def _discover(
        self, path_filter: str | list[str] | None = None, *, hook_loop_for: _HookLoopFor | None = None
    ) -> int:
        with self._lock:
            self._require_open()
        if self._custom_discoverer is not None:
            return self._discover_custom(hook_loop_for)
        return self._discover_default(path_filter, hook_loop_for)

    def discover_multi_class(
        self,
        file_path: "str | Path",
        extensions_root: str = "extensions",
    ) -> "list[tuple[str, type]]":
        """Discover ``@multi_class`` Module classes in a single file (D-15).

        Method-shaped wrapper around :func:`apcore.registry.multi_class.discover_multi_class`
        so the multi-class discovery surface matches the protocol spec
        Registry contract.  The free function remains importable for
        existing callers; new code SHOULD prefer this method.

        The optional ``pre_approval_hook`` configured on the registry is
        forwarded to the underlying scanner so signature-verification and
        audit policies apply uniformly across discovery paths.

        Args:
            file_path: Path to the Python file to scan.
            extensions_root: Root directory name used by Algorithm A01 to
                derive the base module ID.

        Returns:
            List of ``(module_id, class_ref)`` pairs, one per qualifying
            class.  Single-class files return ``[(base_id, cls)]`` (no
            segment appended).
        """
        # Lazy import to avoid a circular import at module load time.
        from apcore.registry.multi_class import discover_multi_class as _discover_multi_class

        return _discover_multi_class(
            file_path,
            extensions_root=extensions_root,
            pre_approval_hook=self._pre_approval_hook,
        )

    def _discover_custom(self, hook_loop_for: _HookLoopFor | None = None) -> int:
        """Run discovery using the custom discoverer."""
        assert self._custom_discoverer is not None  # noqa: S101

        root_paths = [str(r["root"]) for r in self._extension_roots]
        custom_modules = self._custom_discoverer.discover(root_paths)

        # A-D-014: guard that the discoverer returned a list. A non-list result
        # (e.g. ``None`` or a scalar) would otherwise raise an uncaught
        # ``TypeError`` from the loop below. Fail gracefully with a warning and
        # return 0, matching apcore-typescript's ``!Array.isArray`` guard.
        if not isinstance(custom_modules, list):
            logger.warning(
                "Custom discoverer returned non-list (%s); expected list of {'module_id', 'module'} entries. Ignoring.",
                type(custom_modules).__name__,
            )
            return 0

        registered_count = 0
        for entry in custom_modules:
            try:
                mod_id = entry["module_id"]
                mod = entry["module"]
            except (KeyError, TypeError) as e:
                logger.warning(
                    "Malformed entry from custom discoverer (expected dict with 'module_id' and 'module' keys): %s",
                    e,
                )
                continue

            # A-D-015: the reserved ``ephemeral.*`` namespace may only be
            # populated via the explicit Registry.register() path, never via
            # discovery. The filesystem path enforces this in
            # ``_reject_ephemeral_discoveries``; mirror it here for the
            # custom-discoverer path by skipping such entries with a warning
            # (matching apcore-typescript ``_discoverCustom``).
            if _is_ephemeral(mod_id):
                logger.warning(
                    "Skipping custom-discovered module '%s': the 'ephemeral.*' "
                    "namespace is reserved for programmatic registration via "
                    "Registry.register() and may not be populated by discovery.",
                    mod_id,
                )
                continue

            # Apply custom validator if set
            if self._custom_validator is not None:
                errors = self._custom_validator.validate(mod)
                if errors:
                    logger.warning(
                        "Custom validator rejected module '%s': %s",
                        mod_id,
                        "; ".join(errors),
                    )
                    continue

            try:
                # D11-006: validator already ran inline above; skip the
                # re-run inside register() so stateful validators see one
                # call per module (matching apcore-typescript).
                loop = hook_loop_for(mod_id) if hook_loop_for is not None else None
                if loop is None:
                    self.register(mod_id, mod, _skip_custom_validator=True)
                else:
                    self._register(mod_id, mod, skip_custom_validator=True, hook_loop=loop)
                registered_count += 1
            except Exception as e:
                logger.warning("Failed to register custom-discovered module '%s': %s", mod_id, e)

        if registered_count == 0 and custom_modules:
            logger.warning(
                "No modules successfully registered from %d custom-discovered entries",
                len(custom_modules),
            )
        elif registered_count == 0:
            logger.warning("No modules discovered by custom discoverer")

        return registered_count

    def _discover_default(
        self,
        path_filter: str | list[str] | None = None,
        hook_loop_for: _HookLoopFor | None = None,
    ) -> int:
        """Run discovery using the default file-system scanning logic.

        Orchestrates 8 named stages; per-stage logic lives in the dedicated
        helpers below:

        1. ``_scan_roots`` -- walk extension root(s) for candidate files.
        2. ``_apply_path_filter`` -- narrow the candidate set (optional).
        3. ``_apply_id_map_overrides`` -- apply overrides and validate final IDs.
        4. ``_load_all_metadata`` -- read each module's metadata block.
        5. ``_resolve_all_entry_points`` -- import the entry-point classes.
        6. ``_validate_all`` then ``_index_discovered_modules`` -- validate
           candidate classes and check conflicts among final IDs in the batch.
        7. ``_resolve_load_order`` -- dependency-aware ordering.
        8. ``_filter_id_conflicts`` then ``_register_in_order`` -- check the
           published registry and register in order.

        Mirrors the structure of
        ``apcore-typescript/src/registry/registry.ts:_discoverDefault``.
        """
        max_depth, follow_symlinks, ignore_patterns = self._scan_params()
        discovered = self._scan_roots(max_depth, follow_symlinks, ignore_patterns)
        if path_filter is not None:
            discovered = self._apply_path_filter(discovered, path_filter)
        self._apply_id_map_overrides(discovered)
        valid_ids = self._filter_invalid_discovery_ids(discovered)
        candidates = self._resolve_all_entry_points(self._load_all_metadata(valid_ids))
        selected = self._index_discovered_modules(self._validate_all(candidates))
        valid_classes = {module_id: candidate.entry_point for module_id, candidate in selected.items()}
        raw_metadata = {module_id: candidate.metadata for module_id, candidate in selected.items()}
        load_order = self._resolve_load_order(valid_classes, raw_metadata)
        valid_classes = self._filter_id_conflicts(load_order, valid_classes)
        source_paths = {module_id: candidate.source.file_path for module_id, candidate in selected.items()}
        registered_count = self._register_in_order(load_order, valid_classes, raw_metadata, source_paths, hook_loop_for)

        if registered_count == 0 and discovered:
            logger.warning(
                "No modules successfully registered from %d discovered files",
                len(discovered),
            )
        elif registered_count == 0:
            logger.warning("No modules discovered")

        return registered_count

    def _scan_params(self) -> tuple[int, bool, list[str]]:
        """Resolve scan parameters from config, falling back to spec defaults.

        Logs a WARN on the first call with ``follow_symlinks=True`` so
        operators see the trust-boundary reminder in logs when they opt
        into broader filesystem traversal. The scanner itself already
        refuses to follow symlinks escaping the extension root (see
        ``scan_extensions``); this log is a secondary signal that the
        configuration enables a potentially sensitive feature.
        """
        if self._config is None:
            return 8, False, []
        max_depth = self._config.get("extensions.max_depth", 8)
        follow_symlinks = self._config.get("extensions.follow_symlinks", False)
        ignore_patterns = self._config.get("extensions.ignore_patterns", []) or []
        if follow_symlinks and not getattr(self, "_logged_follow_symlinks_warning", False):
            logger.warning(
                "extensions.follow_symlinks=True — scanner will traverse symlinked "
                "directories (confined to the extension root). Ensure the root is "
                "trusted; see apcore.registry.entry_point for the trust-boundary note."
            )
            self._logged_follow_symlinks_warning = True
        return (max_depth, follow_symlinks, list(ignore_patterns))

    def _scan_roots(
        self, max_depth: int, follow_symlinks: bool, ignore_patterns: list[str] | None = None
    ) -> list[DiscoveredModule]:
        """Stage 1 — walk extension root(s) and return DiscoveredModule entries."""
        has_namespace = any("namespace" in r for r in self._extension_roots)
        if len(self._extension_roots) > 1 or has_namespace:
            discovered = scan_multi_root(
                roots=self._extension_roots,
                max_depth=max_depth,
                follow_symlinks=follow_symlinks,
                ignore_patterns=ignore_patterns,
            )
        else:
            discovered = scan_extensions(
                root=self._extension_roots[0]["root"],
                max_depth=max_depth,
                follow_symlinks=follow_symlinks,
                ignore_patterns=ignore_patterns,
            )
        self._reject_ephemeral_discoveries(discovered)
        return discovered

    @staticmethod
    def _reject_ephemeral_discoveries(discovered: list[DiscoveredModule]) -> None:
        """Reject scanned or mapped IDs in the reserved ``ephemeral.*`` namespace.

        Per protocol-spec §2.5.1, ``ephemeral.*`` is reserved
        for programmatically-registered modules synthesized at runtime. Any
        filesystem layout that produces such an ID is a configuration error —
        either the directory is misnamed or the namespace prefix is being
        misused.
        """
        ids = sorted(
            {
                dm.canonical_id
                for dm in discovered
                if isinstance(dm.canonical_id, str) and _is_ephemeral(dm.canonical_id)
            }
        )
        if not ids:
            return
        raise InvalidInputError(
            message=(
                "Filesystem discovery produced module ID(s) in the reserved "
                f"'ephemeral.*' namespace: {ids}. The ephemeral.* namespace is "
                "reserved for programmatically-registered modules and may only "
                "be used via Registry.register(). Rename the offending "
                "directory or extension namespace, or correct the ID map."
            ),
            code=ErrorCodes.INVALID_MODULE_ID,
        )

    def _apply_path_filter(
        self,
        discovered: list[DiscoveredModule],
        path_filter: str | list[str],
    ) -> list[DiscoveredModule]:
        """Filter ``discovered`` entries to those matching ``path_filter``.

        Each entry's ``file_path`` is tested against the supplied glob
        pattern(s) using :meth:`pathlib.PurePath.match`.  The match is tried
        against both the absolute path and the path relative to each
        configured extension root so that patterns like ``"*alpha*"`` work
        as expected (PurePath.match anchors on the right-most segments).
        """
        from pathlib import PurePath

        patterns: list[str]
        if isinstance(path_filter, str):
            patterns = [path_filter]
        else:
            patterns = list(path_filter)
        if not patterns:
            return discovered

        resolved_roots = [Path(r["root"]).resolve() for r in self._extension_roots]

        def _matches(path: Path) -> bool:
            candidates: list[PurePath] = [PurePath(str(path))]
            for root in resolved_roots:
                try:
                    candidates.append(PurePath(str(path.relative_to(root))))
                except ValueError:
                    continue
            for cand in candidates:
                for pat in patterns:
                    if cand.match(pat):
                        return True
            return False

        return [dm for dm in discovered if _matches(dm.file_path)]

    def _apply_id_map_overrides(self, discovered: list[DiscoveredModule]) -> None:
        """Apply ID-map overrides and check the resulting namespace."""
        if not self._id_map:
            return
        resolved_roots = [Path(r["root"]).resolve() for r in self._extension_roots]
        for dm in discovered:
            rel_path: str | None = None
            for root in resolved_roots:
                try:
                    rel_path = str(dm.file_path.relative_to(root))
                    break
                except ValueError:
                    continue
            if rel_path and rel_path in self._id_map:
                dm.canonical_id = self._id_map[rel_path]["id"]
        self._reject_ephemeral_discoveries(discovered)

    @staticmethod
    def _filter_invalid_discovery_ids(discovered: list[DiscoveredModule]) -> list[DiscoveredModule]:
        """Check the final ID grammar before reading metadata or importing a module."""
        valid: list[DiscoveredModule] = []
        for dm in discovered:
            try:
                _validate_module_id(dm.canonical_id)
            except InvalidInputError as exc:
                logger.warning(
                    "Skipping discovered module with invalid ID '%s' at %s: %s", dm.canonical_id, dm.file_path, exc
                )
                continue
            valid.append(dm)
        return valid

    @staticmethod
    def _load_all_metadata(discovered: list[DiscoveredModule]) -> list[tuple[DiscoveredModule, dict[str, Any]]]:
        """Read each optional companion ``*_meta.yaml`` and retain its candidate."""
        return [(dm, load_metadata(dm.meta_path) if dm.meta_path else {}) for dm in discovered]

    def _resolve_all_entry_points(
        self,
        candidates: list[tuple[DiscoveredModule, dict[str, Any]]],
    ) -> list[_DiscoveryCandidate]:
        """Resolve each discovered file with its own entry-point metadata."""
        resolved: list[_DiscoveryCandidate] = []
        for dm, meta in candidates:
            try:
                entry_point = resolve_entry_point(dm.file_path, meta=meta, pre_approval_hook=self._pre_approval_hook)
            except Exception as e:
                logger.warning("Failed to resolve entry point for '%s': %s", dm.canonical_id, e)
                continue
            resolved.append(_DiscoveryCandidate(dm, entry_point, meta))
        return resolved

    def _validate_all(self, candidates: list[_DiscoveryCandidate]) -> list[_DiscoveryCandidate]:
        """Run the custom or built-in validator over each class."""
        valid: list[_DiscoveryCandidate] = []
        for candidate in candidates:
            if self._custom_validator is not None:
                errors = self._custom_validator.validate(candidate.entry_point)
            else:
                errors = validate_module(candidate.entry_point)
            if errors:
                logger.warning("Module '%s' failed validation: %s", candidate.source.canonical_id, "; ".join(errors))
                continue
            valid.append(candidate)
        return valid

    @staticmethod
    def _index_discovered_modules(candidates: list[_DiscoveryCandidate]) -> dict[str, _DiscoveryCandidate]:
        """Keep the first valid candidate for each final ID in scan order."""
        selected: dict[str, _DiscoveryCandidate] = {}
        for candidate in candidates:
            module_id = candidate.source.canonical_id
            previous = selected.get(module_id)
            if previous is not None:
                logger.warning(
                    "Duplicate module ID '%s' at %s, already found at %s. Skipping.",
                    module_id,
                    candidate.source.file_path,
                    previous.source.file_path,
                )
                continue
            selected[module_id] = candidate
        return selected

    def _resolve_load_order(
        self,
        valid_classes: dict[str, type],
        raw_metadata: dict[str, dict[str, Any]],
    ) -> list[str]:
        """Stage 6 + 7 — gather declared dependencies and topo-sort the load order.

        Raises ``CircularDependencyError`` if a cycle is detected.
        """
        modules_with_deps: list[tuple[str, list[DependencyInfo]]] = []
        for mod_id in valid_classes:
            meta = raw_metadata.get(mod_id, {})
            deps_raw = meta.get("dependencies", [])
            deps = parse_dependencies(deps_raw) if deps_raw else []
            modules_with_deps.append((mod_id, deps))

        module_versions = self._collect_module_versions(valid_classes, raw_metadata)
        known_ids = {mod_id for mod_id, _ in modules_with_deps} | set(self._modules.keys())
        return resolve_dependencies(
            modules_with_deps,
            known_ids=known_ids,
            module_versions=module_versions,
        )

    def _collect_module_versions(
        self,
        valid_classes: dict[str, type],
        raw_metadata: dict[str, dict[str, Any]],
    ) -> dict[str, str]:
        """Collect batch-local + live-registry version map for constraint enforcement.

        The returned map is the **highest registered version per module_id**,
        matching the semantics of ``VersionedStore.list_versions`` +
        ``select_best_version`` — the map that
        :func:`resolve_dependencies` uses to satisfy inter-batch version
        constraints. Live-registry entries come from
        ``_versioned_modules`` (the authoritative multi-version store), not
        from the latest-only ``_modules`` view, so modules with multiple
        registered versions are correctly visible to the check.

        Non-string versions in YAML or on class attributes are coerced via
        ``str()`` with a warning — silent drops are what the previous
        revision did and hid misconfigurations.
        """
        module_versions: dict[str, str] = {}
        for mod_id, cls in valid_classes.items():
            meta = raw_metadata.get(mod_id, {})
            yaml_version = meta.get("version")
            code_version = getattr(cls, "version", None)
            resolved_version = yaml_version or code_version or DEFAULT_MODULE_VERSION
            if not isinstance(resolved_version, str):
                logger.warning(
                    "Module '%s' has non-string version %r (%s); coercing to str. "
                    "Fix by quoting the version in YAML or setting a str class attr.",
                    mod_id,
                    resolved_version,
                    type(resolved_version).__name__,
                )
                resolved_version = str(resolved_version)
            module_versions[mod_id] = resolved_version

        # Include already-registered modules from the versioned store so
        # inter-batch constraints resolve against the live registry's
        # highest registered version per module_id.
        with self._lock:
            for existing_id in self._versioned_modules.list_ids():
                if existing_id in module_versions:
                    continue
                versions = self._versioned_modules.list_versions(existing_id)
                if versions:
                    module_versions[existing_id] = versions[-1]
                    continue
                # Fallback: module in _modules but not _versioned_modules
                # (legacy path — should not happen after _register_in_order
                # fix, but guard anyway).
                existing_mod = self._modules.get(existing_id)
                if existing_mod is not None:
                    existing_version = getattr(existing_mod, "version", None)
                    if isinstance(existing_version, str):
                        module_versions[existing_id] = existing_version
            for existing_id, existing_mod in self._modules.items():
                if existing_id in module_versions:
                    continue
                existing_version = getattr(existing_mod, "version", None)
                if isinstance(existing_version, str):
                    module_versions[existing_id] = existing_version
        return module_versions

    def _filter_id_conflicts(
        self,
        load_order: list[str],
        valid_classes: dict[str, type],
    ) -> dict[str, type]:
        """Drop discovery candidates that conflict with the published registry."""
        filtered = dict(valid_classes)
        with self._lock:
            existing_ids = set(self._modules)
            lowercase_map = self._lowercase_map.copy()
        for mod_id in load_order:
            conflict = detect_id_conflicts(
                new_id=mod_id,
                existing_ids=existing_ids,
                reserved_words=RESERVED_WORDS,
                lowercase_map=lowercase_map,
            )
            if conflict is not None:
                if conflict.severity == "error":
                    logger.warning("Skipping module '%s': %s", mod_id, conflict.message)
                    filtered.pop(mod_id, None)
                else:
                    logger.warning("ID conflict: %s", conflict.message)
        return filtered

    def _register_in_order(
        self,
        load_order: list[str],
        valid_classes: dict[str, type],
        raw_metadata: dict[str, dict[str, Any]],
        source_paths: dict[str, Path] | None = None,
        hook_loop_for: _HookLoopFor | None = None,
    ) -> int:
        """Stage 8 — instantiate, register, and run on_load() for each module.

        Populates both the primary ``_modules`` map AND the multi-version
        ``_versioned_modules`` store so that
        ``Registry.get(id, version_hint=...)`` resolves discovered modules
        identically to manually-registered ones. Prior revisions only wrote
        to ``_modules``, leaving version-hint queries unable to see
        auto-discovered modules.

        A-D-REG-003 / Issue #65 — discover paths now apply the same
        deferred-publish protocol as the public ``register()`` API:
        reserve an in-flight slot → run ``on_load()`` outside the lock →
        atomically publish on success. The earlier implementation
        published into ``_modules`` *before* invoking ``on_load`` and
        relied on rollback, leaving a window in which ``registry.get()``
        callers could observe a module whose ``on_load``-installed state
        (warmed pools, primed caches) was incomplete.

        Returns the number of modules that successfully completed registration
        (including a successful ``on_load`` call when defined).
        """
        registered_count = 0
        for mod_id in load_order:
            cls = valid_classes.get(mod_id)
            if cls is None:
                continue
            meta = raw_metadata.get(mod_id, {})
            try:
                module = cls()
            except Exception as e:
                logger.error("Failed to instantiate module '%s': %s", mod_id, e)
                continue

            effective_version = self._effective_version(module, meta)
            try:
                merged_meta = _prepare_discovered_module(mod_id, module, meta)
            except Exception as error:
                logger.warning("Discovery rejected module '%s': %s", mod_id, error)
                continue

            dependencies = parse_dependencies(meta.get("dependencies"))
            # Phase 1: reserve in-flight slot. Skip if already in-flight or
            # visible — concurrent discover invocations must not double-load.
            with self._lock:
                self._require_open()
                if mod_id in self._in_flight or mod_id in self._modules:
                    logger.warning("Discover skipping '%s' — already registered or in-flight", mod_id)
                    continue
                error = self._discovery_dependency_error(mod_id, dependencies)
                if error is None:
                    self._in_flight.add(mod_id)
                failed_optional = [
                    dep.module_id
                    for dep in dependencies
                    if dep.optional and dep.module_id in valid_classes and dep.module_id not in self._modules
                ]
            if error is not None:
                self._emit_module_load_failed(mod_id, error)
                continue
            for dependency_id in failed_optional:
                logger.warning("Module '%s' is loading without optional dependency '%s'", mod_id, dependency_id)

            # Phase 2: run on_load OUTSIDE the lock. Module is NOT visible.
            loop = hook_loop_for(mod_id) if hook_loop_for is not None else None
            if not self._invoke_on_load(mod_id, module, hook_loop=loop):
                # _invoke_on_load already discarded the in-flight slot and
                # emitted apcore.registry.module_load_failed. Move on.
                continue

            # Phase 3: atomic publish.
            with self._ready_to_publish(mod_id, module, hook_loop=loop) as admitted:
                if not admitted:
                    continue
                error = self._discovery_dependency_error(mod_id, dependencies)
                if error is None:
                    self._versioned_modules.add(
                        mod_id, effective_version, _ModuleRegistration(module, merged_meta, meta, loop)
                    )
                    self._modules[mod_id] = module
                    self._module_meta[mod_id] = merged_meta
                    self._lowercase_map[mod_id.lower()] = mod_id
                    if source_paths is not None and mod_id in source_paths:
                        self._module_sources[mod_id] = source_paths[mod_id]
            if error is not None:
                with _lifecycle_callback(self):
                    self._call_on_unload(mod_id, module, hook_loop=loop)
                self._emit_module_load_failed(mod_id, error)
                continue

            self._trigger_event("register", mod_id, module)
            registered_count += 1
        return registered_count

    def _discovery_dependency_error(self, module_id: str, dependencies: list[DependencyInfo]) -> ModuleError | None:
        """Check published dependencies while the caller holds the registry lock."""
        for dependency in dependencies:
            if dependency.optional:
                continue
            provider = self._modules.get(dependency.module_id)
            if provider is None:
                return ModuleLoadError(module_id, f"Required dependency '{dependency.module_id}' did not load")
            if dependency.version:
                versions = self._versioned_modules.list_versions(dependency.module_id)
                actual = (
                    versions[-1]
                    if versions
                    else self._effective_version(provider, self._module_meta.get(dependency.module_id, {}))
                )
                if not matches_version_hint(actual, dependency.version):
                    return DependencyVersionMismatchError(module_id, dependency.module_id, dependency.version, actual)
        return None

    @staticmethod
    def _effective_version(module: Any, meta: dict[str, Any]) -> str:
        """Resolve the effective version for a module from YAML > instance > default."""
        yaml_version = meta.get("version") if meta else None
        code_version = getattr(module, "version", None)
        resolved = yaml_version or code_version or DEFAULT_MODULE_VERSION
        if not isinstance(resolved, str):
            resolved = str(resolved)
        return resolved

    def _invoke_on_load(
        self,
        mod_id: str,
        module: Any,
        *,
        clear_reservation_on_error: bool = True,
        hook_loop: asyncio.AbstractEventLoop | None = None,
    ) -> bool:
        """Call ``module.on_load()`` if defined; clean up in-flight slot on failure.

        Returns True when ``on_load`` succeeded (or no callback was defined)
        and the caller may proceed to publish. Returns False when the
        callback raised, in which case this helper has already:

          * removed ``mod_id`` from the in-flight set,
          * emitted ``apcore.registry.module_load_failed`` (A-D-REG-005),
            mirroring the public ``register()`` path so observers see a
            uniform DLQ-style signal regardless of which registration
            path failed,
          * logged the failure at ERROR.

        Because the deferred-publish refactor (A-D-REG-003) calls this
        BEFORE the module is inserted into the visible store, there is no
        rollback of ``_modules`` / ``_versioned_modules`` to do — the
        invariant is that an on_load failure leaves the registry exactly
        as it was before the registration attempt began.
        """
        try:
            self._initialize_module(
                mod_id, module, clear_reservation_on_error=clear_reservation_on_error, hook_loop=hook_loop
            )
        except Exception as e:
            logger.error("on_load() failed for module '%s': %s", mod_id, e)
            return False
        return True

    def _initialize_module(
        self,
        module_id: str,
        module: Any,
        *,
        clear_reservation_on_error: bool = True,
        hook_loop: asyncio.AbstractEventLoop | None = None,
    ) -> None:
        """Run load and any reload state transfer before making a module visible."""
        try:
            with _lifecycle_callback(self):
                self._validate_hook_adapter(module_id, module, hook_loop)
                if hasattr(module, "on_load") and callable(module.on_load):
                    if hook_loop is None:
                        _run_on_load(module, module_id)
                    else:
                        _call_lifecycle_hook(module.on_load, (), hook_loop, module_id, "on_load")
                reload_state = self._reload_state.get()
                if reload_state is not None and reload_state[0] == module_id:
                    self._call_on_resume(module_id, module, reload_state[1], hook_loop)
        except BaseException as exc:
            if clear_reservation_on_error:
                with self._lock:
                    self._clear_loading(module_id)
            if isinstance(exc, Exception):
                self._emit_module_load_failed(module_id, exc)
            raise

    @contextmanager
    def _ready_to_publish(
        self, module_id: str, module: Any, *, hook_loop: asyncio.AbstractEventLoop | None = None
    ) -> Iterator[bool]:
        """Hold the Registry lock for publication or release an initialized, rejected instance."""
        with self._lock:
            if not self._closed:
                self._clear_loading(module_id)
                yield True
                return
        try:
            with _lifecycle_callback(self):
                self._call_on_unload(module_id, module, hook_loop=hook_loop)
        finally:
            with self._lock:
                self._clear_loading(module_id)
        yield False

    @staticmethod
    def _validate_hook_adapter(module_id: str, module: Any, loop: asyncio.AbstractEventLoop | None) -> None:
        if loop is not None:
            return
        for name in _LIFECYCLE_HOOKS:
            # on_load checks both async def and awaitable results with its
            # established synchronous registration diagnostic.
            if name != "on_load" and inspect.iscoroutinefunction(getattr(module, name, None)):
                raise ModuleLoadError(module_id, f"Module defines async {name}; use register_async or discover_async")

    @contextmanager
    def _resume_on_load(self, module_id: str, state: dict[str, Any] | None) -> Iterator[None]:
        """Share the target's reload state only with its own registration operation."""
        token = self._reload_state.set(None if state is None else (module_id, state))
        try:
            yield
        finally:
            self._reload_state.reset(token)

    # ----- Manual Registration -----

    def register(
        self,
        module_id: str,
        module: Any,
        version: str | None = None,
        metadata: dict[str, Any] | None = None,
        *,
        context: Any = None,
        _skip_custom_validator: bool = False,
    ) -> None:
        """Manually register a module instance.

        Args:
            module_id: Unique identifier for the module.
            module: Module instance to register.
            version: Optional semver version string for versioned registration.
            metadata: Optional metadata dict (may include x-compatible-versions, x-deprecation).
            context: Optional execution context. Used solely to enrich audit
                events emitted for ``ephemeral.*`` registrations (RFC pilot).
                When the context exposes ``caller_id`` / ``identity`` they
                are folded into the audit-event payload (D-35 shape); when
                absent the sentinel ``"@external"`` is used. Ignored for
                non-ephemeral modules.
            _skip_custom_validator: Internal flag — when True, bypass the
                ``_custom_validator.validate`` call. Used by
                ``_discover_custom`` (which already validates inline at line
                442) so stateful validators are invoked exactly once per
                module, matching apcore-typescript ``_registerImpl`` (which
                does not run the validator) called from ``_discoverCustom``
                (which does, once). D11-006.

        Raises:
            InvalidInputError: If module_id is empty, malformed, exceeds the
                length limit, contains a reserved word, or is already
                registered (non-versioned).
            RuntimeError: If module.on_load() fails (propagated).

        Validation order (PROTOCOL_SPEC §2.7, aligned with apcore-typescript
        and apcore-rust): empty → pattern → length → reserved (per-segment)
        → duplicate.

        Step order across validation kinds (registry-system.md "Side Effects
        (ordered)", D-86): module_id → structure/streaming → custom validator
        → duplicate.
        """
        self._register(
            module_id, module, version, metadata, context=context, skip_custom_validator=_skip_custom_validator
        )

    def _register(
        self,
        module_id: str,
        module: Any,
        version: str | None = None,
        metadata: dict[str, Any] | None = None,
        *,
        context: Any = None,
        skip_custom_validator: bool = False,
        hook_loop: asyncio.AbstractEventLoop | None = None,
    ) -> None:
        _validate_module_id(module_id, allow_reserved=False)
        # ``ephemeral.*`` registrations only land via this programmatic
        # entry point — the filesystem discoverer rejects matching IDs
        # earlier (see ``_reject_ephemeral_discoveries``). When invoked
        # here protocol-spec §2.5.1 recommends ``requires_approval=true`` so a
        # human gates execution of agent-synthesized code.
        ephemeral = _is_ephemeral(module_id)
        if ephemeral:
            self._warn_if_missing_approval(module_id, module, metadata)

        errors = validate_module(module, programmatic=True)
        if errors:
            raise InvalidInputError(message=f"Invalid module '{module_id}': {'; '.join(errors)}")
        _ensure_schema_adapter(module, normalize_legacy_none=True)

        # D-86: structure/streaming BEFORE the custom validator, and both
        # before the duplicate check further down. Intrinsic-then-extrinsic —
        # what is wrong with the module itself is reported before what is
        # wrong about where it is being put. Matches apcore-rust and
        # apcore-typescript; Python previously ran the validator first, so a
        # module that was both streaming-malformed and validator-rejected
        # reported a different error here than in the other two SDKs.
        _validate_streaming_annotation(module_id, module, getattr(module, "annotations", None))

        if not skip_custom_validator and self._custom_validator is not None:
            errors = self._custom_validator.validate(module)
            if errors:
                raise InvalidInputError(message=f"Custom validator rejected module '{module_id}': {'; '.join(errors)}")

        effective_version = version or getattr(module, "version", None) or DEFAULT_MODULE_VERSION

        is_versioned = version is not None

        # Pre-compute the merged metadata view OUTSIDE the lock so the
        # critical section stays minimal. merge_module_metadata is pure
        # (no I/O, no shared state). Pass the *instance* so getattr()
        # picks up instance-level attribute overrides (e.g. modules that
        # set self.version in __init__) and still falls back to class
        # attributes via Python's normal lookup chain. This populates
        # _module_meta even for the manual register() path, matching
        # apcore-typescript Registry.register() so get_definition()
        # never has to fall back to the raw module object.
        # The `version` ARGUMENT must reach the merged view, not just the
        # versioned store. `merge_module_metadata` resolves `version` from the
        # meta mapping and falls back to the module attribute, so omitting it
        # here made `get_definition(module_id).version` report "1.0.0" for
        # `register(module_id, module, version="2.0.0")` while apcore-typescript
        # and apcore-rust both reported "2.0.0" (sync finding A-D-002).
        merge_input = dict(metadata or {})
        if version is not None:
            merge_input["version"] = version
        merged_meta = merge_module_metadata(module, merge_input)

        # Phase 1: conflict detection + mark in-flight (DEFERRED-PUBLISH, apcore #65).
        # The module is NOT inserted into _modules/_versioned_modules until after on_load
        # completes, so discovery APIs see it only after full initialisation.
        with self._lock:
            self._require_open()
            self._reject_unloading_instance(module_id, module)
            # Check the in-flight set FIRST so concurrent same-ID registrations are
            # rejected even when on_load is running (module not yet in _modules).
            if module_id in self._in_flight or module_id in self._draining:
                raise InvalidInputError(
                    message=f"Module '{module_id}' is already being registered (in-flight)",
                    code=ErrorCodes.DUPLICATE_MODULE_ID,
                )

            # For explicit versioned registration, skip conflict check when
            # the module_id already exists (we allow multiple versions).
            # For non-versioned registration, preserve original conflict detection.
            if is_versioned and self._versioned_modules.has_version(module_id, effective_version):
                raise InvalidInputError(
                    message=f"Module '{module_id}' version '{effective_version}' is already registered",
                    code=ErrorCodes.DUPLICATE_MODULE_ID,
                )
            if not is_versioned or not self._versioned_modules.has(module_id):
                conflict = detect_id_conflicts(
                    new_id=module_id,
                    existing_ids=set(self._modules.keys()),
                    reserved_words=RESERVED_WORDS,
                    lowercase_map=self._lowercase_map,
                )
                if conflict is not None:
                    if conflict.severity == "error":
                        raise InvalidInputError(
                            message=conflict.message,
                            code=ErrorCodes.DUPLICATE_MODULE_ID,
                        )
                    else:
                        logger.warning("ID conflict: %s", conflict.message)

            # Mark as in-flight — visible store is NOT updated yet.
            self._in_flight.add(module_id)

        # Phase 2: call on_load() OUTSIDE the lock.
        # The module is not visible during this phase. Concurrent same-ID
        # registrations will see module_id in _in_flight and raise DUPLICATE_MODULE_ID.
        self._initialize_module(module_id, module, hook_loop=hook_loop)

        # Phase 3: atomically insert into visible store.
        with self._ready_to_publish(module_id, module, hook_loop=hook_loop) as admitted:
            if not admitted:
                raise InvalidInputError(message="The module registry is shut down")

            # Store in versioned store
            registration = _ModuleRegistration(module, merged_meta, dict(metadata or {}), hook_loop)
            self._versioned_modules.add(module_id, effective_version, registration)

            # Always point the primary map to the latest version
            latest = self._versioned_modules.get_latest(module_id)
            if latest is not None:
                self._modules[module_id] = latest.module
                self._module_meta[module_id] = latest.metadata
                if latest is registration:
                    self._module_sources.pop(module_id, None)
            self._lowercase_map[module_id.lower()] = module_id

        self._trigger_event("register", module_id, module)
        if ephemeral:
            self._emit_ephemeral_audit("apcore.registry.module_registered", module_id, context)

    def register_many(self, entries: Iterable[tuple[str, Any]]) -> None:
        """Register modules in input order. Prior entries remain registered after an error."""
        for module_id, module in entries:
            self.register(module_id, module)

    async def register_async(
        self, module_id: str, module: Any, version: str | None = None, metadata: dict[str, Any] | None = None
    ) -> None:
        """Register a module with async lifecycle hooks bound to this live loop."""
        loop = asyncio.get_running_loop()
        await asyncio.to_thread(self._register, module_id, module, version, metadata, hook_loop=loop)

    async def register_many_async(self, entries: Iterable[tuple[str, Any]]) -> None:
        """Register modules in order with support for async lifecycle hooks."""
        for module_id, module in entries:
            await self.register_async(module_id, module)

    def _require_open(self) -> None:
        if self._closed:
            raise InvalidInputError(message="The module registry is shut down")

    def _reject_unloading_instance(self, module_id: str, module: Any) -> None:
        if any(
            pending.module_id == module_id and any(instance is module for instance in pending.registration.instances())
            for pending in self._pending_unloads.values()
        ):
            raise InvalidInputError(
                message=f"The '{module_id}' module instance is still unloading",
                code=ErrorCodes.DUPLICATE_MODULE_ID,
            )

    def _clear_loading(self, module_id: str) -> None:
        """Release a registration reservation under the registry lock."""
        self._in_flight.discard(module_id)
        self._lifecycle_changed.notify_all()

    def unregister(self, module_id: str, *, context: Any = None) -> bool:
        """Remove a module from the registry.

        Existing calls retain their registration until they finish. The last
        active call runs the detached registration's cleanup.

        Args:
            module_id: ID of the module to remove.
            context: Optional execution context. Used solely to enrich the
                audit event emitted for ``ephemeral.*`` unregistrations
                (RFC pilot). Ignored for non-ephemeral modules.

        Returns False if module was not registered.
        """
        with self._lock:
            if module_id in self._draining:
                return False
            leases = self._ref_counts.get(module_id, 0)
            pending = self._detach_registration(module_id, context=context, remaining_leases=leases)
            if pending is None:
                return False
        if not leases:
            self._finalize_detached(pending)
        return True

    def _notify_unregistered(
        self,
        module_id: str,
        registration: _RegistrationSnapshot,
        context: Any = None,
        hook_progress: Callable[[bool], None] | None = None,
    ) -> LifecycleError | None:
        error = None
        for module in registration.instances():
            current_error = self._call_on_unload(module_id, module, hook_progress, registration.hook_loop(module))
            error = error or current_error
        self._trigger_event("unregister", module_id, registration.module)
        if _is_ephemeral(module_id):
            self._emit_ephemeral_audit("apcore.registry.module_unregistered", module_id, context)
        return error

    # ----- Query Methods -----

    def get(self, module_id: str, version_hint: str | None = None) -> Any:
        """Look up a module by ID and optional version hint. Returns None if not found.

        If version_hint is provided, resolves to the best matching version.
        If no hint, returns the latest version.

        Raises:
            ModuleNotFoundError: If module_id is empty string, or if a
                ``version_hint`` is supplied but ``module_id`` was registered
                via :meth:`register_internal` (which deliberately skips
                ``_versioned_modules`` — D11-001).
                Sys/internal modules are not version-tracked; callers asking
                for version resolution on one get a clear error rather than
                a silent ``None``.
        """
        registration = self._resolve_registration(module_id, version_hint)
        return registration.module if registration else None

    def _resolve_registration(self, module_id: str, version_hint: str | None) -> _ModuleRegistration | None:
        """Select a module and its descriptor from one registry snapshot."""
        if module_id == "":
            raise ModuleNotFoundError(module_id="")
        with self._lock:
            if self._versioned_modules.has(module_id):
                return self._versioned_modules.resolve(module_id, version_hint)
            if module_id not in self._modules:
                return None
            if version_hint is not None:
                raise ModuleNotFoundError(
                    module_id=module_id,
                    message=(
                        f"Module '{module_id}' is registered as a sys/internal "
                        "module (via register_internal) and is not version-tracked; "
                        "drop the version_hint argument to look it up by ID alone."
                    ),
                )
            return _ModuleRegistration(self._modules[module_id], self._module_meta[module_id], {})

    def has(self, module_id: str) -> bool:
        """Check whether a module is registered."""
        with self._lock:
            return module_id in self._modules

    def list(
        self,
        tags: list[str] | None = None,
        prefix: str | None = None,
        *,
        visibility: list[str] | None = None,
    ) -> list[str]:
        """Return sorted list of unique registered module IDs, optionally filtered.

        Args:
            tags: When supplied, only modules carrying *all* of the given tags
                are returned.
            prefix: When supplied, only IDs starting with the prefix are returned.
            visibility: Filter by module visibility. Supported: ``["public", "hidden"]``.
                Defaults to ``["public"]``. Pass ``["public", "hidden"]`` to see
                all modules (protocol-spec §4.4 ``discoverable``).
        """
        if visibility is not None:
            vis = set(visibility)
            show_public = "public" in vis
            show_hidden = "hidden" in vis
        else:
            # Default per protocol-spec §4.4 discoverable: public only.
            show_public = True
            show_hidden = False

        with self._lock:
            snapshot = dict(self._modules)
            meta_snapshot = dict(self._module_meta)

        ids = list(snapshot.keys())

        # Filter by visibility
        filtered_ids: list[str] = []
        for mid in ids:
            is_disc = self._is_discoverable(mid, snapshot, meta_snapshot)
            if (is_disc and show_public) or (not is_disc and show_hidden):
                filtered_ids.append(mid)

        ids = filtered_ids

        if prefix is not None:
            ids = [mid for mid in ids if mid.startswith(prefix)]

        if tags is not None:
            tag_set = set(tags)

            def has_all_tags(mid: str) -> bool:
                mod = snapshot[mid]
                # Check module-level tags attribute
                mod_tags = set(getattr(mod, "tags", []) or [])
                # Also check merged metadata tags
                meta_tags = meta_snapshot.get(mid, {}).get("tags", [])
                if meta_tags:
                    mod_tags.update(meta_tags)
                return tag_set.issubset(mod_tags)

            ids = [mid for mid in ids if has_all_tags(mid)]

        return sorted(ids)

    def iter(self, *, include_hidden: bool = False) -> Iterator[tuple[str, Any]]:
        """Return an iterator of (module_id, module) tuples (snapshot-based).

        Modules annotated ``discoverable=False`` are excluded by default.
        Pass ``include_hidden=True`` to enumerate every registered module.
        """
        with self._lock:
            items = list(self._modules.items())
            meta_snapshot = dict(self._module_meta)
        snapshot = dict(items)
        if not include_hidden:
            items = [(mid, mod) for mid, mod in items if self._is_discoverable(mid, snapshot, meta_snapshot)]
        return iter(items)

    @property
    def count(self) -> int:
        """Number of registered modules (including hidden ones)."""
        with self._lock:
            return len(self._modules)

    @property
    def module_ids(self) -> list[str]:
        """Sorted list of registered module IDs (excludes ``discoverable=False`` modules).

        Modules annotated ``discoverable=False`` (per the apcore RFC pilot)
        are filtered out. Use :meth:`list` with ``visibility=["public", "hidden"]`` when
        the full set is required.
        """
        with self._lock:
            snapshot = dict(self._modules)
            meta_snapshot = dict(self._module_meta)
        return sorted(mid for mid in snapshot if self._is_discoverable(mid, snapshot, meta_snapshot))

    @staticmethod
    def _is_discoverable(
        module_id: str,
        modules: dict[str, Any],
        module_meta: dict[str, dict[str, Any]],
    ) -> bool:
        """Return False when the module's annotations declare ``discoverable=False``.

        Resolution order matches :func:`merge_module_metadata`:
        1. The merged ``annotations`` slot in ``_module_meta`` (which
           already accounts for YAML > code precedence).
        2. The module instance's ``annotations`` attribute (covers paths
           that bypassed the merge, e.g. ``register_internal``).

        Anything other than an explicit ``False`` keeps the module visible —
        the default ``True`` preserves backward compatibility.
        """
        return Registry._registration_is_discoverable(modules.get(module_id), module_meta.get(module_id))

    @staticmethod
    def _registration_is_discoverable(module: Any, meta: dict[str, Any] | None) -> bool:
        if meta is not None:
            ann = meta.get("annotations")
            if ann is not None:
                discoverable = getattr(ann, "discoverable", None)
                if discoverable is None and isinstance(ann, dict):
                    discoverable = ann.get("discoverable")
                if discoverable is False:
                    return False
                if discoverable is not None:
                    return True
        if module is None:
            return True
        ann = getattr(module, "annotations", None)
        if ann is None:
            return True
        if isinstance(ann, dict):
            return ann.get("discoverable", True) is not False
        return getattr(ann, "discoverable", True) is not False

    def _discovery_registration(
        self, module_id: str, version_hint: str | None
    ) -> tuple[_ModuleRegistration, object | None] | None:
        """Snapshot a public registration that can accept new calls, without holding a lease."""
        with self._lock:
            if self._closed or module_id in self._draining or module_id in self._unavailable:
                return None
            try:
                registration = self._resolve_registration(module_id, version_hint)
            except ModuleNotFoundError:
                return None
            if registration is None or not self._registration_is_discoverable(
                registration.module, registration.metadata
            ):
                return None
            return registration, self._lease_generations.get(module_id)

    def get_definition(self, module_id: str, version_hint: str | None = None) -> ModuleDescriptor | None:
        """Get a ModuleDescriptor for a registered module. Returns None if not found.

        Args:
            module_id: The module ID.
            version_hint: Optional version hint for selecting a specific version.
        """
        registration = self._resolve_registration(module_id, version_hint)
        if registration is None:
            return None
        return self._describe_registration(module_id, registration)

    def _describe_registration(
        self, module_id: str, registration: _ModuleRegistration, *, warn_deprecation: bool = True
    ) -> ModuleDescriptor:
        module = registration.module
        meta = registration.metadata
        version_str = meta.get("version") or getattr(module, "version", None) or DEFAULT_MODULE_VERSION

        cls = type(module)

        input_schema_cls = getattr(module, "input_schema", None) or getattr(cls, "input_schema", None)
        output_schema_cls = getattr(module, "output_schema", None) or getattr(cls, "output_schema", None)

        for schema_cls in (input_schema_cls, output_schema_cls):
            if schema_cls is not None and hasattr(schema_cls, "model_rebuild"):
                try:
                    schema_cls.model_rebuild()
                except Exception:
                    pass

        input_json = (
            input_schema_cls
            if isinstance(input_schema_cls, dict)
            else input_schema_cls.model_json_schema()
            if input_schema_cls
            else {}
        )
        output_json = (
            output_schema_cls
            if isinstance(output_schema_cls, dict)
            else output_schema_cls.model_json_schema()
            if output_schema_cls
            else {}
        )

        effective_metadata = {**meta.get("metadata", {}), **registration.descriptor_metadata}

        # Log deprecation warning if x-deprecation is present
        deprecation = effective_metadata.get("x-deprecation")
        if deprecation and warn_deprecation:
            self._log_deprecation_warning(module_id, version_str, deprecation)

        sunset_date: str | None = None
        if deprecation:
            sunset_date = deprecation.get("sunset_date")

        # INVARIANT: every registration site (`register`, `register_internal`,
        # `_register_in_order`) populates `_module_meta` via
        # `merge_module_metadata`, so `meta` always contains the full set of
        # canonical keys including the merged `annotations` slot. Read all
        # merged-meta fields straight from it. Schemas come straight from
        # the module instance because they are not part of the merged
        # metadata payload. Return detached schemas for catalogue consumers.
        return ModuleDescriptor(
            module_id=module_id,
            name=meta.get("name"),
            description=meta.get("description") or "",
            documentation=meta.get("documentation"),
            input_schema=deepcopy(input_json),
            output_schema=deepcopy(output_json),
            version=meta.get("version") or DEFAULT_MODULE_VERSION,
            tags=list(meta.get("tags") or []),
            annotations=meta.get("annotations"),
            examples=list(meta.get("examples") or []),
            metadata=effective_metadata,
            sunset_date=sunset_date,
            # Parsed once here so consumers get typed DependencyInfo rather than
            # re-deriving `{module_id, version?, optional?}` from raw JSON at
            # every call site (PROTOCOL_SPEC §12.2; sync finding A-D-004).
            # `meta` is checked first because `merge_module_metadata` lifts a
            # declared `dependencies` to the top level; the nested
            # `metadata["dependencies"]` form is the manual-register path.
            dependencies=parse_dependencies(meta.get("dependencies") or effective_metadata.get("dependencies") or []),
        )

    def _log_deprecation_warning(self, module_id: str, version: str, deprecation: dict[str, Any]) -> None:
        """Log a deprecation warning for a module version, once per registry.

        At most one warning per ``(module_id, version, x-deprecation block)``
        for the lifetime of this registry instance (D-89, spec v1.59.0).

        The BLOCK is part of the key, and the key is never cleared on
        ``unregister``. Forgetting on unregister used to be how a re-registered
        module carrying a NEW notice still warned — but ``watch()`` re-runs
        discovery as an unregister + re-register, so it also re-warned for every
        deprecated module on every hot reload: the traffic-proportional spam
        D-89 exists to prevent, arriving through the door its wording did not
        close. Keying on the block gives both halves — a re-registration with a
        CHANGED or newly added notice warns, one with the same notice does not.

        Blocks are compared by canonical value (sorted keys, no incidental
        whitespace), so two structurally equal notices dedupe regardless of how
        they were built.
        """
        key = (module_id, version, _canonical_deprecation(deprecation))
        with self._lock:
            if key in self._deprecation_warned:
                return
            self._deprecation_warned.add(key)

        deprecated_since = deprecation.get("deprecated_since", "unknown")
        sunset_version = deprecation.get("sunset_version", "unknown")
        migration_guide = deprecation.get("migration_guide", "")
        msg = f"Module '{module_id}' v{version} is deprecated (since {deprecated_since}, sunset in {sunset_version})."
        if migration_guide:
            msg += f" Migration: {migration_guide}"
        logger.warning(msg)

    def export_schema(self, module_id: str, strict: bool = False) -> dict[str, Any] | None:
        """Export the schema definition for a registered module as a plain dict.

        Returns the module's input and output schemas in the generic export
        format (no platform-specific transformations).  Returns ``None`` if
        the module is not registered.

        Args:
            module_id: The ID of the module whose schema should be exported.
            strict: When True, applies strict JSON Schema constraints
                (removes ``additionalProperties``, marks all required).
                Defaults to False.

        Returns:
            A dict with keys ``module_id``, ``description``, ``input_schema``
            and ``output_schema``; or ``None`` if the module is not found.
            Pinned by ``schemas/module-schema-export.schema.json``.

        Aligned with ``apcore-rust Registry::export_schema``.
        """
        descriptor = self.get_definition(module_id)
        if descriptor is None:
            return None

        input_schema = descriptor.input_schema
        output_schema = descriptor.output_schema
        if strict:
            from apcore.schema.strict import to_strict_schema

            input_schema = to_strict_schema(input_schema)
            output_schema = to_strict_schema(output_schema)

        # Built directly rather than through ``SchemaExporter.export_generic``:
        # that helper also emits ``definitions``, which is a field of a
        # ``SchemaDefinition`` parsed from a YAML schema file. A descriptor has
        # no such field, so on this path it was always ``{}`` — a second,
        # permanently empty place to look for the ``$defs`` that JSON Schema
        # already keeps inside ``input_schema``. See
        # ``schemas/module-schema-export.schema.json``.
        return {
            "module_id": descriptor.module_id,
            "description": descriptor.description,
            "input_schema": input_schema,
            "output_schema": output_schema,
        }

    def describe(self, module_id: str) -> str:
        """Return a human-readable description of a module.

        Args:
            module_id: The ID of the module to describe.

        Returns:
            Markdown-formatted description string.

        Raises:
            ModuleNotFoundError: If module is not registered.
        """
        module = self.get(module_id)
        if module is None:
            raise ModuleNotFoundError(module_id)

        # Module-supplied override (D-77). ``Module.describe()`` is declared to
        # return an introspection MAPPING, so only a string return is the
        # author's deliberate description override and passed through verbatim.
        # Anything else — a mapping, None, or another type — falls through to
        # the generated envelope: ``str(dict)`` is a Python repr, not a
        # description, and this method is declared to return a string.
        if hasattr(module, "describe") and callable(module.describe):
            override = module.describe()
            if isinstance(override, str):
                return override

        # Auto-generate from descriptor
        descriptor = self.get_definition(module_id)
        if descriptor is None:
            return f"Module: {module_id}\n\nNo description available."

        lines = [f"# {descriptor.module_id}"]
        if descriptor.description:
            lines.append(f"\n{descriptor.description}")
        if descriptor.tags:
            lines.append(f"\n**Tags:** {', '.join(descriptor.tags)}")
        if descriptor.input_schema and descriptor.input_schema.get("properties"):
            lines.append("\n**Parameters:**")
            for param, schema in descriptor.input_schema["properties"].items():
                param_type = schema.get("type", "any")
                param_desc = schema.get("description", "")
                required = param in descriptor.input_schema.get("required", [])
                req_marker = " (required)" if required else ""
                lines.append(f"- `{param}` ({param_type}){req_marker}: {param_desc}")
        if descriptor.documentation:
            lines.append(f"\n**Documentation:**\n{descriptor.documentation}")
        return "\n".join(lines)

    # ----- Event System -----

    def on(self, event: str, callback: Callable[..., Any]) -> None:
        """Register an event callback.

        Args:
            event: Event name ('register' or 'unregister').
            callback: Callable(module_id, module) to invoke on the event.

        Raises:
            InvalidInputError: If event name is invalid.
        """
        with self._lock:
            if event not in self._callbacks:
                raise InvalidInputError(message=f"Invalid event: {event}. Must be 'register' or 'unregister'")
            self._callbacks[event].append(callback)

    def off(self, event: str, callback: Callable[..., Any]) -> bool:
        """Unregister an event callback.

        Mirrors ``apcore-typescript.Registry.off`` and
        ``apcore-rust.Registry::off``.

        Args:
            event: Event name ('register' or 'unregister').
            callback: The exact callable previously passed to ``on()``.

        Returns:
            True if the callback was found and removed, False if not found.

        Raises:
            InvalidInputError: If event name is invalid.
        """
        with self._lock:
            if event not in self._callbacks:
                raise InvalidInputError(message=f"Invalid event: {event}. Must be 'register' or 'unregister'")
            callbacks = self._callbacks[event]
            try:
                callbacks.remove(callback)
                return True
            except ValueError:
                return False

    def _trigger_event(self, event: str, module_id: str, module: Any) -> None:
        """Trigger all callbacks for an event.

        Callbacks are invoked outside the registry lock on a per-event
        snapshot of the subscriber list. This is a deliberate divergence
        from a strict "synchronous within the lock" reading of the
        registry-system spec: running callbacks under the RLock would make
        a callback that re-enters the registry (e.g., lists modules,
        triggers another register) susceptible to deadlock on downstream
        locks and would serialize otherwise-independent work. Python's
        registry therefore snapshots callbacks, releases the lock, and
        fires them outside — at the cost of a window where a callback
        observes pre-commit state. Exceptions are logged and counted via
        ``get_callback_errors(event)``; they do NOT propagate into the
        register/unregister caller.
        """
        with self._lock:
            callbacks = list(self._callbacks.get(event, []))
        for cb in callbacks:
            try:
                cb(module_id, module)
            except Exception as e:
                with self._lock:
                    self._callback_errors[event] = self._callback_errors.get(event, 0) + 1
                logger.error(
                    "Callback error for event '%s' on module '%s': %s",
                    event,
                    module_id,
                    e,
                )
                if self._metrics_collector is not None:
                    self._metrics_collector.increment(
                        # OBS-007: underscores, not dots. The Prometheus
                        # exposition format admits `[a-zA-Z_:][a-zA-Z0-9_:]*`
                        # only, and `export_prometheus` writes the name
                        # verbatim — so one dotted sample made the WHOLE scrape
                        # unparseable and every other apcore metric was lost
                        # with it. `_total` per the counter convention. No
                        # other SDK carries this metric, so nothing depends on
                        # the old spelling.
                        "apcore_registry_callback_errors_total",
                        {
                            "event": event,
                            "module_id": module_id,
                            "error_type": type(e).__name__,
                        },
                    )

    # ----- Ephemeral namespace pilot (protocol-spec §2.5.1) -----

    @staticmethod
    def _warn_if_missing_approval(module_id: str, module: Any, metadata: dict[str, Any] | None = None) -> None:
        """Soft-warn when an ephemeral.* module is registered without ``requires_approval=True``.

        Per protocol-spec §2.5.1, agent-synthesized modules SHOULD
        declare ``requires_approval: true`` so a human gates execution. The
        registry only warns; it does not refuse the registration.

        The two sources are unioned (PROTOCOL_SPEC §7.4, D-96) because that is
        what the approval gate reads: a requirement declared in the ``metadata``
        / ``*.binding.yaml`` source gates the module just as a code-level one
        does. Reading the instance alone advised an author to set something they
        had already set, in the one place the registry itself records it — an
        advisory that fires against correct configuration is one operators learn
        to ignore.
        """
        effective = governance_union(
            getattr(module, "annotations", None),
            (metadata or {}).get("annotations"),
        )
        requires_approval = bool(effective is not None and effective.requires_approval)
        if not requires_approval:
            logger.warning(
                "ephemeral.* module '%s' registered without requires_approval=True. "
                "The apcore protocol-spec §2.5.1 recommends "
                "setting ModuleAnnotations(requires_approval=True) so agent-"
                "synthesized code does not run unattended.",
                module_id,
            )

    def _emit_module_load_failed(self, module_id: str, exc: Exception) -> None:
        """Emit apcore.registry.module_load_failed when on_load raises (apcore #65)."""
        emitter = self._event_emitter
        if emitter is None:
            logger.error(
                "apcore.registry.module_load_failed module_id=%s error_type=%s error_message=%s",
                module_id,
                type(exc).__name__,
                str(exc),
            )
            return

        try:
            from datetime import datetime, timezone

            from apcore.events.emitter import ApCoreEvent

            ts = datetime.now(timezone.utc).isoformat()
            emitter.emit(
                ApCoreEvent(
                    event_type="apcore.registry.module_load_failed",
                    module_id=module_id,
                    timestamp=ts,
                    severity="error",
                    data={
                        "module_id": module_id,
                        "callback_name": "on_load",
                        "error_type": type(exc).__name__,
                        "error_message": str(exc),
                        "timestamp": ts,
                    },
                )
            )
        except Exception as e:
            logger.error("Failed to emit module_load_failed event for '%s': %s", module_id, e)

    def _build_ephemeral_audit_payload(self, context: Any) -> dict[str, Any]:
        """Build the ``caller_id`` (+ optional ``identity``) D-35 payload fragment.

        Shares the redaction rules used by ``system.control.*`` audit events
        (see ``apcore.sys_modules.identity._audit_payload_extras``).
        """
        from apcore.sys_modules.identity import _audit_payload_extras

        return _audit_payload_extras(context)

    def _emit_ephemeral_audit(
        self,
        event_type: str,
        module_id: str,
        context: Any,
    ) -> None:
        """Emit an audit event for an ephemeral.* register/unregister.

        When :meth:`set_event_emitter` has wired an emitter, the canonical
        ``apcore.registry.module_registered`` /
        ``apcore.registry.module_unregistered`` event is emitted with the
        D-35 contextual-audit payload. When no emitter is wired the event
        is logged at INFO so it never silently disappears — useful for the
        v1 pilot where most callers will not have an emitter attached.
        """
        try:
            payload = self._build_ephemeral_audit_payload(context)
        except Exception as e:  # defensive: identity extraction must never break registration
            logger.warning(
                "Failed to extract audit payload for ephemeral '%s': %s. Falling back to caller_id='@external'.",
                module_id,
                e,
            )
            payload = {"caller_id": "@external", "identity": None}

        payload.setdefault("identity", None)
        payload["namespace_class"] = "ephemeral"

        emitter = self._event_emitter
        if emitter is None:
            logger.info(
                "ephemeral audit event %s module_id=%s payload=%s "
                "(no EventEmitter wired; call Registry.set_event_emitter to capture)",
                event_type,
                module_id,
                payload,
            )
            return

        # Lazy import — events package is optional at the registry layer.
        try:
            from datetime import datetime, timezone

            from apcore.events.emitter import ApCoreEvent
        except Exception as e:  # pragma: no cover - extremely defensive
            logger.warning("Cannot import ApCoreEvent for ephemeral audit emit: %s", e)
            return

        try:
            emitter.emit(
                ApCoreEvent(
                    event_type=event_type,
                    module_id=module_id,
                    timestamp=datetime.now(timezone.utc).isoformat(),
                    severity="info",
                    data=payload,
                )
            )
        except Exception as e:
            logger.error(
                "EventEmitter.emit failed for ephemeral audit event %s on '%s': %s",
                event_type,
                module_id,
                e,
            )

    def get_callback_errors(self, event: str | None = None) -> dict[str, int] | int:
        """Return callback-exception counts per event.

        Args:
            event: If given, returns the integer count for that event.
                If None, returns a snapshot dict of all per-event counts.
        """
        with self._lock:
            if event is None:
                return dict(self._callback_errors)
            return self._callback_errors.get(event, 0)

    # ----- Safe Hot-Reload (F09 / Algorithm A21) -----

    @contextmanager
    def acquire(self, module_id: str) -> Iterator[Any]:
        """Hold a lease on the latest module while it is in use."""
        with self._acquire_registration(module_id, None) as registration:
            yield registration.module

    @contextmanager
    def _acquire_registration(
        self, module_id: str, version_hint: str | None, cancel_token: CancelToken | None = None
    ) -> Iterator[_ModuleRegistration]:
        with self._lock:
            if self._closed or module_id in self._draining or module_id in self._unavailable:
                raise ModuleNotFoundError(module_id=module_id)
            registration = self._resolve_registration(module_id, version_hint)
            if registration is None:
                raise ModuleNotFoundError(module_id=module_id)
            generation = self._lease_generations.setdefault(module_id, object())
            self._ref_counts[module_id] = self._ref_counts.get(module_id, 0) + 1
            if cancel_token is not None:
                tokens = self._invocation_tokens.get(generation)
                if tokens is None:
                    tokens = self._invocation_tokens[generation] = set()
                tokens.add(cancel_token)
        try:
            yield registration
        finally:
            self._release_generation(module_id, generation, cancel_token)

    def release(self, module_id: str) -> None:
        """Release one reference to the current module and notify a waiting unload."""
        with self._lock:
            generation = self._lease_generations.get(module_id)
        if generation is not None:
            self._release_generation(module_id, generation)

    def _release_generation(self, module_id: str, generation: object, cancel_token: CancelToken | None = None) -> None:
        with self._lock:
            if cancel_token is not None and (tokens := self._invocation_tokens.get(generation)) is not None:
                tokens.discard(cancel_token)
                if not tokens and self._lease_generations.get(module_id) is not generation:
                    self._invocation_tokens.pop(generation, None)
            if self._lease_generations.get(module_id) is generation:
                count = self._ref_counts.get(module_id, 0)
                if count > 1:
                    self._ref_counts[module_id] = count - 1
                elif count == 1:
                    self._ref_counts.pop(module_id, None)
                    event = self._drain_events.get(module_id)
                    if event is not None:
                        event.set()
                        self._lifecycle_changed.notify_all()
                return

            pending = self._pending_unloads.get(generation)
            if pending is None or pending.remaining_leases == 0:
                return
            pending.remaining_leases -= 1
            if pending.remaining_leases:
                return
            if pending.residuals is not None:
                pending.residuals.set()
                return
        self._finalize_detached(pending)

    def _finalize_detached(self, pending: _PendingUnload) -> None:
        if _has_running_loop() and any(
            pending.registration.hook_loop(module) is not None for module in pending.registration.instances()
        ):
            threading.Thread(
                target=self._finish_unload, args=(pending,), name="apcore-module-unload", daemon=True
            ).start()
        else:
            self._finish_unload(pending)

    def _finish_unload(
        self, pending: _PendingUnload, operation: LifecycleOperation | None = None, index: int = 0
    ) -> LifecycleError | None:
        def report_hook(active: bool) -> None:
            hook = "on_unload" if active else None
            with self._lock:
                pending.active_hook = active
                self._report_shutdown_unload(pending, active_hook=hook)
            if operation is not None and operation._kind != "shutdown":
                operation._update_module(index, active_hook=hook)

        error = None
        try:
            with self._lock:
                self._report_shutdown_unload(pending, phase="unloading")
            with _lifecycle_callback(self, operation._id if operation is not None else None):
                error = self._notify_unregistered(pending.module_id, pending.registration, pending.context, report_hook)
                return error
        except BaseException as failure:
            error = LifecycleError._from_exception(failure, "unloading")
            raise
        finally:
            with self._lock:
                pending.unload_finished = True
                pending.error = pending.error or error
                self._report_joined_removal(pending)
                if pending.residuals is None:
                    self._settle_unload(pending)

    def _settle_unload(self, pending: _PendingUnload) -> None:
        if pending.residuals is not None:
            self._report_shutdown_unload(pending, cleanup_pending=False)
        self._pending_unloads.pop(pending.generation, None)
        pending.finished.set()

    def _report_joined_removal(self, pending: _PendingUnload) -> None:
        outcome = None
        if pending.unload_finished:
            outcome = "failed" if pending.error else "forced_removed" if pending.active_at_unload else "removed"
        self._report_shutdown_unload(
            pending,
            phase="done" if pending.unload_finished else "draining" if pending.remaining_leases > 0 else "unloading",
            outcome=outcome,
            active_hook="on_unload" if pending.active_hook else None,
            active_at_unload=pending.active_at_unload,
            drain_timed_out=pending.drain_timed_out or pending.active_at_unload is not None,
            cleanup_pending=pending.unload_finished and pending.remaining_leases > 0,
            error=pending.error,
        )

    def _report_shutdown_unload(self, pending: _PendingUnload, **updates: object) -> None:
        """Mirror a generation's existing removal into the shutdown report under the Registry lock."""
        operation = self._shutdown_operation
        if operation is not None and not operation._finished.is_set():
            index = self._shutdown_rows.get(pending.generation)
            if index is None:
                index = operation._add_module(pending.module_id)
                self._shutdown_rows[pending.generation] = index
            operation._update_module(index, **updates)

    def _capture_state_and_unload(self, pending: _PendingUnload, clean: bool) -> dict[str, Any] | None:
        try:
            if clean:
                with _lifecycle_callback(self):
                    module = pending.registration.module
                    return self._call_on_suspend(pending.module_id, module, pending.registration.hook_loop(module))
            return None
        finally:
            self._finish_unload(pending)

    def is_draining(self, module_id: str) -> bool:
        """Check whether a module is marked for unload (draining).

        Returns:
            True if the module is currently draining, False otherwise.
        """
        with self._lock:
            return module_id in self._draining

    def safe_unregister(self, module_id: str, *, timeout_ms: int = 5000) -> bool:
        """Safely unregister a module with cooperative wait for in-flight executions.

        Marks the module as *draining* so that no new ``acquire()`` calls are
        accepted, then waits up to ``timeout_ms`` milliseconds for in-flight
        executions to finish.  If they do not finish in time the module is
        force-unloaded and a warning is logged.

        Args:
            module_id: The ID of the module to unregister.
            timeout_ms: Maximum time to wait for in-flight executions (milliseconds).

        Returns:
            True if the module was cleanly shut down (no in-flight executions
            remaining), False if it was force-unloaded after timeout.
        """
        if _has_running_loop():
            registration = self._snapshot_registration(module_id)
            if registration is not None and any(
                registration.hook_loop(module) is asyncio.get_running_loop() for module in registration.instances()
            ):
                raise InvalidInputError(message="Use safe_unregister_async() for a module with async lifecycle hooks")
        detached = self._detach_after_drain(module_id, timeout_ms=timeout_ms)
        if detached is None:
            return False
        pending, clean = detached
        self._finish_unload(pending)
        return clean

    async def safe_unregister_async(self, module_id: str, *, timeout_ms: int = 5000) -> bool:
        """Drain and unload without blocking the caller's event loop."""
        return await asyncio.to_thread(self.safe_unregister, module_id, timeout_ms=timeout_ms)

    async def unregister_many(self, module_ids: Iterable[str], *, timeout_ms: int = 5000) -> list[bool]:
        """Drain and unload modules in input order, with one result per ID."""
        return [await self.safe_unregister_async(module_id, timeout_ms=timeout_ms) for module_id in module_ids]

    def start_unregister_many(
        self, module_ids: Iterable[str], *, options: UnregisterOptions | None = None
    ) -> LifecycleOperation:
        """Own an ordered, generation-bound removal independent of its waiters."""
        _require_lifecycle_loop()
        if options is not None and not isinstance(options, UnregisterOptions):
            raise InvalidInputError(message="Expected UnregisterOptions")
        options = UnregisterOptions() if options is None else options
        _validate_drain_options(options)
        if options.cancel_grace_ms is not None and options.on_drain_timeout != "cancel_then_force":
            raise InvalidInputError(message="cancel_grace_ms requires cancel_then_force")
        _validate_duration("operation_timeout_ms", options.operation_timeout_ms)
        if isinstance(module_ids, (str, bytes)):
            raise InvalidInputError(message="module_ids must be an iterable of module ID strings")
        try:
            ids = tuple(module_ids)
        except TypeError as error:
            raise InvalidInputError(message="module_ids must be an iterable of module ID strings") from error
        for module_id in ids:
            _validate_module_id(module_id, allow_reserved=True)

        operation = LifecycleOperation("unregister_many")
        operation._set_modules(iter(ids))
        with self._lock:
            self._require_open()
            registrations = self._snapshot_generations(iter(ids))
        if not ids:
            operation._complete()
            return operation
        operation_deadline = (
            None if options.operation_timeout_ms is None else time.monotonic() + options.operation_timeout_ms / 1000
        )
        drain_timeout_ms = 5000 if options.drain_timeout_ms is None else options.drain_timeout_ms
        grace = None
        if options.on_drain_timeout == "cancel_then_force":
            grace = 1000 if options.cancel_grace_ms is None else options.cancel_grace_ms
        operation._start(
            lambda: self._run_removals(
                operation, registrations, drain_timeout_ms, operation_deadline, cancel_grace_ms=grace
            ),
            "unloading",
        )
        return operation

    def _snapshot_generations(self, module_ids: Iterator[str]) -> list[tuple[str, object | None]]:
        """Capture registration identities under the Registry lock."""
        return [
            (module_id, self._lease_generations.setdefault(module_id, object()) if module_id in self._modules else None)
            for module_id in module_ids
        ]

    def _run_removals(
        self,
        operation: LifecycleOperation,
        registrations: list[tuple[str, object | None]],
        drain_timeout_ms: int,
        operation_deadline: float | None = None,
        *,
        legacy_force: bool = False,
        cancel_grace_ms: int | None = None,
    ) -> None:
        for index, (module_id, generation) in enumerate(registrations):
            with self._lock:
                shutdown = operation._kind == "shutdown"
                current = generation is not None and self._lease_generations.get(module_id) is generation
                now = time.monotonic()
                if operation_deadline is not None and now >= operation_deadline:
                    operation._stop_batch(index)
                    break
                if not current or module_id in self._draining or (self._closed and not shutdown):
                    if not shutdown:
                        owned = current or generation in self._pending_unloads
                        operation._update_module(index, phase="done", outcome="already_removing" if owned else "absent")
                    continue
                if shutdown:
                    index = self._shutdown_rows[generation]
                ready = self._mark_draining(module_id)
                deadline = now + drain_timeout_ms / 1000
                if operation_deadline is not None:
                    deadline = min(deadline, operation_deadline)
                operation._update_module(index, phase="draining")
                registration = self._snapshot_registration(module_id)
            assert registration is not None and generation is not None
            self._remove_drained_generation(
                operation,
                index,
                module_id,
                ready,
                deadline,
                drain_timeout_ms,
                legacy_force,
                cancel_grace_ms,
                generation,
                registration,
            )
        if operation._kind == "unregister_many":
            operation._complete()

    def _remove_drained_generation(
        self,
        operation: LifecycleOperation,
        index: int,
        module_id: str,
        ready: threading.Event,
        deadline: float,
        drain_timeout_ms: int,
        legacy_force: bool,
        cancel_grace_ms: int | None,
        generation: object,
        registration: _RegistrationSnapshot,
    ) -> None:
        drain_error = None
        timed_out = False
        if legacy_force:
            ready.wait(timeout=max(0, deadline - time.monotonic()))
        else:
            _, drain_error, timed_out = self._run_managed_drain(
                operation,
                index,
                module_id,
                generation,
                registration,
                ready,
                deadline,
                drain_timeout_ms,
                "shutdown" if operation._kind == "shutdown" else "unregister",
                cancel_grace_ms=cancel_grace_ms,
            )
        with self._lock:
            active = self._ref_counts.get(module_id, 0)
            if legacy_force and active:
                operation._update_module(index, drain_timed_out=True)
            residual = active if cancel_grace_ms is not None else 0
            pending = self._detach_registration(module_id, remaining_leases=residual)
            if pending is not None and residual:
                pending.residuals = threading.Event()
        assert pending is not None
        forced = active > 0
        pending.drain_timed_out = timed_out
        pending.error = drain_error
        if forced:
            pending.active_at_unload = active
            logger.warning("Force-unloading module %s after drain timeout (%d active calls)", module_id, active)
        operation._update_module(index, phase="unloading", active_at_unload=pending.active_at_unload)
        unload_error = self._finish_unload(pending, operation, index)
        error = drain_error or unload_error
        outcome = "failed" if error is not None else "forced_removed" if forced else "removed"
        if pending.residuals is not None:
            operation._update_module(index, phase="done", outcome=outcome, error=error, cleanup_pending=True)
            pending.residuals.wait()
            with self._lock:
                self._settle_unload(pending)
        operation._update_module(index, phase="done", outcome=outcome, error=error, cleanup_pending=False)

    def _run_managed_drain(
        self,
        operation: LifecycleOperation,
        index: int,
        module_id: str,
        generation: object,
        registration: _RegistrationSnapshot,
        ready: threading.Event,
        deadline: float,
        drain_timeout_ms: int,
        reason: Literal["unregister", "reload", "shutdown"],
        *,
        reload_policy: _ReloadDrainAction = "continue_waiting",
        cancel_grace_ms: int | None = None,
    ) -> tuple[_AbortReason | None, LifecycleError | None, bool]:
        token = CancelToken()
        context = DrainContext(operation._id, module_id, reason, token, deadline)
        self._emit_lifecycle(
            "module_drain_started",
            operation,
            module_id,
            {
                "reason_kind": reason,
                "drain_timeout_ms": drain_timeout_ms,
            },
        )
        progress = self._begin_drain_hooks(operation, index, module_id, registration, context)
        with self._lifecycle_changed:
            self._lifecycle_changed.wait_for(
                lambda: progress.finished and (ready.is_set() or (reason == "reload" and progress.error is not None)),
                timeout=max(0, deadline - time.monotonic()),
            )
            active = self._ref_counts.get(module_id, 0)
            error_won = reason == "reload" and progress.finished and progress.error is not None
            timed_out = not error_won and (not progress.finished or active > 0)
            hook_running = not progress.finished
            scopes = (
                tuple(self._invocation_tokens.get(generation, ())) if timed_out and cancel_grace_ms is not None else ()
            )
        grace_deadline = None
        if timed_out:
            operation._update_module(index, drain_timed_out=True)
            action = "cancel_then_force" if cancel_grace_ms is not None else reload_policy
            self._emit_lifecycle(
                "module_drain_timed_out",
                operation,
                module_id,
                {
                    "reason_kind": reason,
                    "action": action,
                    "active_count": active,
                },
                severity="warn",
            )
            if hook_running:
                self._signal_lifecycle_token(token)
            if cancel_grace_ms is not None:
                grace_deadline = time.monotonic() + cancel_grace_ms / 1000
                for scope in scopes:
                    self._signal_lifecycle_token(scope)
        with self._lifecycle_changed:
            self._lifecycle_changed.wait_for(lambda: progress.finished)
        if reason == "reload":
            if timed_out and reload_policy == "abort_reload":
                return "drain_timeout", LifecycleError(ErrorCodes.RELOAD_FAILED, "draining"), timed_out
            if progress.error is not None:
                return "drain_error", progress.error, timed_out
        ready.wait(timeout=None if grace_deadline is None else max(0, grace_deadline - time.monotonic()))
        return None, progress.error, timed_out

    @staticmethod
    def _signal_lifecycle_token(token: CancelToken) -> None:
        try:
            token.cancel()
        except Exception as error:
            logger.warning("Lifecycle cancellation listener failed: %s", type(error).__name__)

    def _begin_drain_hooks(
        self,
        operation: LifecycleOperation,
        index: int,
        module_id: str,
        registration: _RegistrationSnapshot,
        context: DrainContext,
    ) -> _DrainProgress:
        progress = _DrainProgress()

        def run() -> None:
            try:
                for instance in registration.instances():
                    try:
                        hook = getattr(instance, "on_drain", None)
                        if callable(hook):
                            operation._update_module(index, active_hook="on_drain")
                            with _lifecycle_callback(self, operation._id):
                                _call_lifecycle_hook(
                                    hook, (context,), registration.hook_loop(instance), module_id, "on_drain"
                                )
                    except BaseException as error:
                        progress.error = progress.error or LifecycleError._from_exception(error, "draining")
            finally:
                operation._update_module(index, active_hook=None)
                with self._lifecycle_changed:
                    progress.finished = True
                    self._lifecycle_changed.notify_all()

        try:
            if any(callable(getattr(instance, "on_drain", None)) for instance in registration.instances()):
                threading.Thread(target=run, name="apcore-module-drain", daemon=True).start()
            else:
                progress.finished = True
        except Exception as error:
            progress.error = LifecycleError._from_exception(error, "draining")
            progress.finished = True
        return progress

    def _emit_lifecycle(
        self, event: str, operation: LifecycleOperation, module_id: str, data: dict[str, Any], *, severity: str = "info"
    ) -> None:
        emitter = self._event_emitter
        if emitter is not None:
            from datetime import datetime, timezone
            from apcore.events.emitter import ApCoreEvent

            try:
                emitter.emit(
                    ApCoreEvent(
                        f"apcore.registry.{event}",
                        module_id,
                        datetime.now(timezone.utc).isoformat(),
                        severity,
                        {"operation_id": operation._id, **data},
                    )
                )
            except Exception as error:
                logger.warning("Unable to emit lifecycle event %s: %s", event, type(error).__name__)

    def _start_managed_reload(
        self,
        module_ids: list[str],
        drain_timeout_ms: int,
        policy: _ReloadDrainAction,
        on_reloaded: Callable[[str, str, str, str], None],
        *,
        exact: bool = False,
        prepared: _WatchedReplacement | None = None,
    ) -> LifecycleOperation:
        with self._lock:
            self._require_open()
            plan = [
                (module_id, generation, self._snapshot_registration(module_id))
                for module_id, generation in self._snapshot_generations(iter(module_ids))
            ]
        for module_id, generation, registration in plan:
            if generation is None or registration is None:
                if exact:
                    raise ModuleNotFoundError(module_id=module_id)
                continue
            for module in registration.instances():
                if callable(getattr(module, "on_drain", None)) and not callable(
                    getattr(module, "on_drain_aborted", None)
                ):
                    raise InvalidInputError(
                        message=f"Module '{module_id}' must pair on_drain with on_drain_aborted for managed reload"
                    )
        operation = LifecycleOperation("reload", registry=self)
        operation._set_modules(iter(module_ids))
        if not plan:
            operation._complete()
            return operation

        def run() -> None:
            try:
                for index, (module_id, generation, registration) in enumerate(plan):
                    self._run_managed_reload(
                        operation,
                        index,
                        module_id,
                        generation,
                        registration,
                        drain_timeout_ms,
                        policy,
                        on_reloaded,
                        prepared,
                    )
                operation._complete()
            finally:
                if prepared is not None:
                    self._finish_watched_replacement(prepared)

        operation._start(run, "loading")
        return operation

    def _run_managed_reload(
        self,
        operation: LifecycleOperation,
        index: int,
        module_id: str,
        generation: object | None,
        registration: _RegistrationSnapshot | None,
        timeout_ms: int,
        policy: _ReloadDrainAction,
        on_reloaded: Callable[[str, str, str, str], None],
        prepared: _WatchedReplacement | None,
    ) -> None:
        with self._lock:
            if (
                self._closed
                or generation is None
                or registration is None
                or (prepared is not None and prepared.generation is not generation)
                or self._lease_generations.get(module_id) is not generation
                or module_id in self._draining
                or module_id in self._in_flight
                or module_id in self._unavailable
                or (current := self._snapshot_registration(module_id)) is None
                or current.module is not registration.module
                or current.versions.keys() != registration.versions.keys()
                or any(current.versions[key] is not value for key, value in registration.versions.items())
            ):
                error = LifecycleError(ErrorCodes.MODULE_RELOAD_CONFLICT, "draining")
                operation._update_module(
                    index, phase="done", outcome="failed", error=error, compensation="not_required"
                )
                return
            ready = self._mark_draining(module_id)
            deadline = time.monotonic() + timeout_ms / 1000
            operation._update_module(index, phase="draining")
        abort, error, timed_out = self._run_managed_drain(
            operation,
            index,
            module_id,
            generation,
            registration,
            ready,
            deadline,
            timeout_ms,
            "reload",
            reload_policy=policy,
        )
        if abort is not None:
            self._recover_managed_reload(operation, index, module_id, registration, abort, error)
            return

        with self._lifecycle_changed:
            if self._closed:
                self._draining.discard(module_id)
                self._drain_events.pop(module_id, None)
                self._lifecycle_changed.notify_all()
                error = LifecycleError(ErrorCodes.MODULE_RELOAD_CONFLICT, "draining")
                operation._update_module(
                    index, phase="done", outcome="failed", error=error, compensation="not_required"
                )
                return
            pending = self._detach_registration(module_id)
            self._in_flight.add(module_id)
        assert pending is not None
        pending.drain_timed_out = timed_out
        self._replace_managed_generation(operation, index, module_id, pending, on_reloaded, prepared)

    def _recover_managed_reload(
        self,
        operation: LifecycleOperation,
        index: int,
        module_id: str,
        registration: _RegistrationSnapshot,
        abort: _AbortReason,
        error: LifecycleError | None,
    ) -> None:
        operation._update_module(index, phase="recovering", abort_reason=abort)
        context = DrainAbortedContext(operation._id, module_id, abort, CancelToken())
        recovery_error = None
        for module in registration.instances():
            try:
                hook = getattr(module, "on_drain_aborted", None)
                if callable(getattr(module, "on_drain", None)) and callable(hook):
                    operation._update_module(index, active_hook="on_drain_aborted")
                    with _lifecycle_callback(self, operation._id):
                        _call_lifecycle_hook(
                            hook, (context,), registration.hook_loop(module), module_id, "on_drain_aborted"
                        )
            except BaseException as failure:
                recovery_error = recovery_error or LifecycleError._from_exception(failure, "recovering")
        with self._lifecycle_changed:
            ready = recovery_error is None and not self._closed
            if not ready:
                self._unavailable.add(module_id)
            self._draining.discard(module_id)
            self._drain_events.pop(module_id, None)
            self._lifecycle_changed.notify_all()
        outcome = "retained_ready" if ready else "retained_unavailable"
        operation._update_module(
            index,
            phase="done",
            active_hook=None,
            outcome=outcome,
            error=recovery_error or error,
            compensation="not_required",
        )
        self._emit_lifecycle(
            "module_drain_recovered",
            operation,
            module_id,
            {
                "abort_reason": abort,
                "outcome": outcome,
            },
            severity="info" if ready else "error",
        )

    def _replace_managed_generation(
        self,
        operation: LifecycleOperation,
        index: int,
        module_id: str,
        pending: _PendingUnload,
        on_reloaded: Callable[[str, str, str, str], None],
        prepared: _WatchedReplacement | None,
    ) -> None:
        old = pending.registration
        state = None
        phase = "unloading"
        try:
            operation._update_module(index, phase="unloading")
            try:
                if callable(getattr(old.module, "on_suspend", None)):
                    operation._update_module(index, active_hook="on_suspend")
                    with _lifecycle_callback(self, operation._id):
                        state = self._call_on_suspend(module_id, old.module, old.hook_loop(old.module))
            finally:
                operation._update_module(index, active_hook=None)
                unload_error = self._finish_unload(pending, operation, index)
            if unload_error is not None:
                raise ModuleError(unload_error.code, "Module unload failed")
            phase = "loading"
            operation._update_module(index, phase="loading")
            if prepared is None:
                replacement = self._replacement_snapshot(module_id, old)
            else:
                with self._lock:
                    self._require_current_watched_source(module_id, prepared)
                replacement = prepared.registration
            self._activate_managed_snapshot(operation, index, module_id, replacement, state, watched=prepared)
        except BaseException as failure:
            error = (
                LifecycleError(ErrorCodes.MODULE_LOAD_ERROR, "loading")
                if phase == "loading" and not isinstance(failure, ModuleError)
                else LifecycleError._from_exception(failure, phase)
            )
            if prepared is not None and not prepared.source.is_file():
                # Deletion is terminal for this source. Never republish its old registration.
                compensation = "not_required"
            else:
                restored = self._compensate_managed_reload(operation, index, module_id, old, state, watched=prepared)
                compensation = "restored" if restored else "failed"
            operation._update_module(
                index, phase="done", active_hook=None, outcome="failed", error=error, compensation=compensation
            )
            self._emit_lifecycle(
                "module_reload_failed",
                operation,
                module_id,
                {
                    "phase": error.phase,
                    "error_code": error.code,
                    "compensation": compensation,
                },
                severity="error",
            )
            return
        finally:
            with self._lock:
                self._clear_loading(module_id)

        previous_version = str(old.metadata.get("version") or DEFAULT_MODULE_VERSION)
        new_version = str(replacement.metadata.get("version") or DEFAULT_MODULE_VERSION)
        operation._update_module(index, phase="done", outcome="reloaded", compensation="not_required")
        try:
            on_reloaded(module_id, previous_version, new_version, operation._id)
        except Exception as failure:
            logger.warning("Could not record successful reload of %s: %s", module_id, type(failure).__name__)

    def _compensate_managed_reload(
        self,
        operation: LifecycleOperation,
        index: int,
        module_id: str,
        registration: _RegistrationSnapshot,
        state: dict[str, Any] | None,
        *,
        watched: _WatchedReplacement | None = None,
    ) -> bool:
        try:
            with self._lock:
                self._require_open()
            self._activate_managed_snapshot(operation, index, module_id, registration, state, watched=watched)
            return True
        except BaseException as failure:
            logger.error("Managed reload compensation failed for %s: %s", module_id, type(failure).__name__)
            return False

    def _activate_managed_snapshot(
        self,
        operation: LifecycleOperation,
        index: int,
        module_id: str,
        registration: _RegistrationSnapshot,
        state: dict[str, Any] | None,
        *,
        watched: _WatchedReplacement | None = None,
    ) -> None:
        initialized = []
        try:
            with _lifecycle_callback(self, operation._id):
                for module in registration.instances():
                    loop = registration.hook_loop(module)
                    self._validate_hook_adapter(module_id, module, loop)
                    hook = getattr(module, "on_load", None)
                    if callable(hook):
                        operation._update_module(index, phase="loading", active_hook="on_load")
                        _call_lifecycle_hook(hook, (), loop, module_id, "on_load")
                    initialized.append(module)
                    if (
                        module is registration.module
                        and state is not None
                        and callable(getattr(module, "on_resume", None))
                    ):
                        operation._update_module(index, phase="resuming", active_hook="on_resume")
                        self._call_on_resume(module_id, module, state, loop)
                operation._update_module(index, active_hook=None)
                with self._lock:
                    self._require_open()
                    if watched is not None and registration is watched.registration:
                        self._require_current_watched_source(module_id, watched)
                    elif watched is not None and not watched.source.is_file():
                        raise ModuleLoadError(module_id, "Watched module source was deleted before restoration")
                    self._publish_snapshot(module_id, registration)
                self._trigger_event("register", module_id, registration.module)
        except BaseException:
            with _lifecycle_callback(self, operation._id):
                for module in reversed(initialized):
                    self._call_on_unload(module_id, module, hook_loop=registration.hook_loop(module))
            raise

    def _replacement_snapshot(self, module_id: str, old: _RegistrationSnapshot) -> _RegistrationSnapshot:
        loop = old.hook_loop(old.module)
        source = None
        metadata: dict[str, Any] = {}
        if self._custom_discoverer is not None:
            roots = [str(root["root"]) for root in self._extension_roots]
            modules = self._custom_discoverer.discover(roots)
            match = next(
                (entry for entry in modules if isinstance(entry, dict) and entry.get("module_id") == module_id), None
            )
            if match is None or "module" not in match:
                raise ModuleLoadError(module_id, "The replacement was not discovered")
            module = match["module"]
            errors = validate_module(module, programmatic=True)
            if not errors and self._custom_validator is not None:
                errors = self._custom_validator.validate(module)
            if errors:
                raise ModuleLoadError(module_id, "The replacement is not a valid module")
            _ensure_schema_adapter(module, normalize_legacy_none=True)
        else:
            candidates = (
                [self._watched_candidate(old.source_path)]
                if old.source_path is not None
                else self._reload_source_candidates(module_id)
            )
            target = next((item for item in candidates if item is not None and item.canonical_id == module_id), None)
            if target is None:
                raise ModuleLoadError(module_id, "The replacement source was not discovered")
            selected = self._validate_all(self._resolve_all_entry_points(self._load_all_metadata([target])))
            if not selected:
                raise ModuleLoadError(module_id, "The replacement source was invalid")
            source = target.file_path
            metadata = selected[0].metadata
            module = selected[0].entry_point()
        merged = _prepare_discovered_module(module_id, module, metadata)
        version = self._effective_version(module, metadata)
        deps = parse_dependencies(metadata.get("dependencies"))
        with self._lock:
            failure = self._discovery_dependency_error(module_id, deps)
        if failure is not None:
            raise failure
        registration = _ModuleRegistration(module, merged, metadata, loop)
        return _RegistrationSnapshot(module, merged, {version: registration}, source)

    def _reload_source_candidates(self, module_id: str) -> list[DiscoveredModule]:
        candidates = self._scan_roots(*self._scan_params())
        self._apply_id_map_overrides(candidates)
        return [item for item in self._filter_invalid_discovery_ids(candidates) if item.canonical_id == module_id]

    def _stop_admission(self) -> None:
        with self._lock:
            self._closed = True

    def _start_shutdown(
        self,
        drain_timeout_ms: int | None,
        on_drain_timeout: _HostDrainAction | None,
        cancel_grace_ms: int | None = None,
        *,
        legacy: bool,
    ) -> LifecycleOperation:
        with self._lock:
            existing = self._shutdown_operation
            if existing is not None:
                if drain_timeout_ms is not None and drain_timeout_ms != self._shutdown_drain_timeout_ms:
                    raise InvalidInputError(message="Conflicting shutdown drain_timeout_ms")
                active_policy = "continue_waiting" if self._shutdown_cancel_grace_ms is None else "cancel_then_force"
                if on_drain_timeout is not None and (self._shutdown_legacy_force or on_drain_timeout != active_policy):
                    raise InvalidInputError(message="Conflicting shutdown on_drain_timeout")
                if cancel_grace_ms is not None and cancel_grace_ms != self._shutdown_cancel_grace_ms:
                    raise InvalidInputError(message="Conflicting shutdown cancel_grace_ms")
                if not legacy or existing._failure is None:
                    return existing

            timeout = self._shutdown_drain_timeout_ms if drain_timeout_ms is None else drain_timeout_ms
            force = legacy if existing is None else self._shutdown_legacy_force
            grace = self._shutdown_cancel_grace_ms
            if existing is None:
                if cancel_grace_ms is not None and on_drain_timeout != "cancel_then_force":
                    raise InvalidInputError(message="cancel_grace_ms requires cancel_then_force")
                if on_drain_timeout == "cancel_then_force":
                    grace = 1000 if cancel_grace_ms is None else cancel_grace_ms
            operation = LifecycleOperation("shutdown", registry=self)
            operation._start(lambda: self._run_shutdown(operation, timeout, force, grace), "shutdown")
            self._shutdown_operation = operation
            self._shutdown_drain_timeout_ms = timeout
            self._shutdown_legacy_force = force
            self._shutdown_cancel_grace_ms = grace
            self._closed = True
            self._shutdown_rows.clear()
            self._track_shutdown_registrations(operation, self._snapshot_generations(reversed(self._modules)))
            for pending in self._pending_unloads.values():
                self._report_joined_removal(pending)
            return operation

    def _track_shutdown_registrations(
        self, operation: LifecycleOperation, registrations: list[tuple[str, object | None]]
    ) -> None:
        for module_id, generation in registrations:
            if generation is not None and generation not in self._shutdown_rows:
                index = operation._add_module(module_id)
                self._shutdown_rows[generation] = index
                if module_id in self._draining:
                    operation._update_module(index, phase="draining")

    def _run_shutdown(
        self, operation: LifecycleOperation, drain_timeout_ms: int, legacy_force: bool, cancel_grace_ms: int | None
    ) -> None:
        # Ownership and admission are published under the Registry lock
        # before the worker can call into an injected watcher.
        with self._lock:
            pass
        self.unwatch()
        with self._lifecycle_changed:
            self._lifecycle_changed.wait_for(lambda: not self._in_flight and not self._draining)
            registrations = self._snapshot_generations(reversed(self._modules))
            self._track_shutdown_registrations(operation, registrations)
        self._run_removals(
            operation, registrations, drain_timeout_ms, legacy_force=legacy_force, cancel_grace_ms=cancel_grace_ms
        )
        with self._lifecycle_changed:
            self._lifecycle_changed.wait_for(lambda: not self._draining)
        self._wait_for_unloads(legacy_force=legacy_force)
        operation._complete()

    def _wait_for_unloads(self, *, legacy_force: bool) -> None:
        with self._lock:
            pending = tuple(self._pending_unloads.values())
        deadline = time.monotonic() + 5
        for removal in pending:
            if not legacy_force or removal.residuals is not None:
                removal.finished.wait()
                continue
            if removal.finished.wait(timeout=max(0, deadline - time.monotonic())):
                continue
            with self._lock:
                force = removal.remaining_leases > 0
                if force:
                    removal.active_at_unload = removal.remaining_leases
                    self._report_shutdown_unload(
                        removal, drain_timed_out=True, active_at_unload=removal.active_at_unload
                    )
                removal.remaining_leases = 0
            if force:
                logger.warning("Force-unloading module %s after shutdown timeout", removal.module_id)
                self._finish_unload(removal)
            else:
                removal.finished.wait()

    def _detach_after_drain(
        self,
        module_id: str,
        *,
        timeout_ms: int = 5000,
        reserve_replacement: bool = False,
        source_path: Path | None = None,
    ) -> tuple[_PendingUnload, bool] | None:
        """Detach a module after its active leases have drained, before calling any hooks."""
        with self._lock:
            if module_id not in self._modules or module_id in self._draining:
                return None
            if source_path is not None and self._module_sources.get(module_id) != source_path:
                return None
            if self._ref_counts.get(module_id, 0) and _has_running_loop():
                raise InvalidInputError(message="Use safe_unregister_async() to drain an active module from asyncio")
            event = self._mark_draining(module_id)

        clean = event.wait(timeout=timeout_ms / 1000.0)
        with self._lock:
            if self._drain_events.get(module_id) is not event:
                return None
            ref_count = self._ref_counts.get(module_id, 0)
            pending = self._detach_registration(module_id)
            assert pending is not None
            if not clean:
                pending.active_at_unload = ref_count
                self._report_shutdown_unload(pending, drain_timed_out=True, active_at_unload=ref_count)
            if reserve_replacement:
                self._in_flight.add(module_id)
        if not clean:
            logger.warning(
                "Force-unloading module %s after %dms timeout (%d in-flight executions)",
                module_id,
                timeout_ms,
                ref_count,
            )
        return pending, clean

    def _mark_draining(self, module_id: str) -> threading.Event:
        """Close admission for a generation under the Registry lock."""
        self._draining.add(module_id)
        event = self._drain_events[module_id] = threading.Event()
        if self._ref_counts.get(module_id, 0) == 0:
            event.set()
        return event

    # ----- Hot Reload -----

    def watch(self, *, lifecycle: ReloadOptions | None = None) -> None:
        """Start watching extension directories for file changes.

        Requires the optional ``watchdog`` dependency.
        Raises ImportError if watchdog is not installed.
        """
        hook_loop = None
        if lifecycle is not None:
            if not isinstance(lifecycle, ReloadOptions):
                raise InvalidInputError(message="Expected ReloadOptions for watch lifecycle")
            _validate_duration("drain_timeout_ms", lifecycle.drain_timeout_ms)
            if lifecycle.drain_timeout_ms is None or lifecycle.on_drain_timeout not in (
                "continue_waiting",
                "abort_reload",
            ):
                raise InvalidInputError(message="Invalid watch lifecycle policy")
            if _has_running_loop():
                hook_loop = asyncio.get_running_loop()
        from apcore.registry.watcher import load_observer_factory

        create_observer = load_observer_factory()

        with self._lock:
            self._require_open()
            if hasattr(self, "_observer") and self._observer is not None:
                return
            observer = create_observer(self, self._extension_roots)
            self._watch_lifecycle = lifecycle
            self._watch_hook_loop = hook_loop
            observer.start()
            self._observer = observer

    def unwatch(self) -> None:
        """Stop watching extension directories for file changes."""
        with self._lock:
            observer = getattr(self, "_observer", None)
            self._observer = None
            self._watch_lifecycle = None
            self._watch_hook_loop = None
            self._watched_replacements.clear()
        if observer is not None:
            observer.stop()
            observer.join()

    def _detach_registration(
        self, mid: str, *, context: Any = None, remaining_leases: int = 0
    ) -> _PendingUnload | None:
        """Remove a module from visibility while retaining its cleanup state under the lock."""
        registration = self._snapshot_registration(mid)
        if registration is None:
            return None
        self._modules.pop(mid, None)
        self._module_meta.pop(mid, None)
        self._module_sources.pop(mid, None)
        self._schema_cache.pop(mid, None)
        self._lowercase_map.pop(mid.lower(), None)
        self._versioned_modules.remove_all(mid)
        self._draining.discard(mid)
        self._unavailable.discard(mid)
        event = self._drain_events.pop(mid, None)
        if event is not None:
            event.set()
        self._ref_counts.pop(mid, None)
        generation = self._lease_generations.pop(mid, None) or object()
        if not self._invocation_tokens.get(generation):
            self._invocation_tokens.pop(generation, None)
        pending = _PendingUnload(generation, mid, registration, context, remaining_leases)
        self._pending_unloads[generation] = pending
        self._report_shutdown_unload(pending, phase="draining" if remaining_leases > 0 else "unloading")
        self._lifecycle_changed.notify_all()
        return pending

    def _snapshot_registration(self, module_id: str) -> _RegistrationSnapshot | None:
        with self._lock:
            if module_id not in self._modules:
                return None
            return _RegistrationSnapshot(
                self._modules[module_id],
                self._module_meta[module_id],
                self._versioned_modules.snapshot(module_id),
                self._module_sources.get(module_id),
            )

    def _restore_registration(
        self, module_id: str, registration: _RegistrationSnapshot | None, suspended_state: dict[str, Any] | None = None
    ) -> bool:
        """Restore a detached registration after reinitializing its unloaded instance."""
        if registration is None:
            return False
        with self._lock:
            if self._closed or module_id in self._modules or module_id in self._in_flight:
                return False
            self._in_flight.add(module_id)
        module = registration.module
        loaded = []
        published = False
        try:
            for instance in registration.instances():
                loop = registration.hook_loop(instance)
                state = suspended_state if instance is module else None
                with self._resume_on_load(module_id, state):
                    if not self._invoke_on_load(module_id, instance, clear_reservation_on_error=False, hook_loop=loop):
                        return False
                loaded.append(instance)
            with self._lock:
                if self._closed:
                    return False
                self._publish_snapshot(module_id, registration)
                published = True
            with _lifecycle_callback(self):
                self._trigger_event("register", module_id, module)
            return True
        finally:
            if not published:
                with _lifecycle_callback(self):
                    for instance in reversed(loaded):
                        self._call_on_unload(module_id, instance, hook_loop=registration.hook_loop(instance))
            with self._lock:
                self._clear_loading(module_id)

    def _publish_snapshot(self, module_id: str, registration: _RegistrationSnapshot) -> None:
        """Publish initialized registrations under the Registry lock."""
        self._modules[module_id] = registration.module
        self._module_meta[module_id] = registration.metadata
        self._lowercase_map[module_id.lower()] = module_id
        if registration.source_path is not None:
            self._module_sources[module_id] = registration.source_path
        for version, entry in registration.versions.items():
            self._versioned_modules.add(module_id, version, entry)

    def _watched_candidate(self, path: Path) -> DiscoveredModule | None:
        max_depth, follow_symlinks, ignore_patterns = self._scan_params()
        candidate = scan_watched_source(path, self._extension_roots, max_depth, follow_symlinks, ignore_patterns)
        if candidate is not None:
            self._reject_ephemeral_discoveries([candidate])
            self._apply_id_map_overrides([candidate])
            _validate_module_id(candidate.canonical_id)
        return candidate

    def _handle_file_change(self, path: str) -> None:
        """Handle a file modification or creation event.

        Performs arbitrary-Python import (``resolve_entry_point``) and user
        hooks (``on_suspend``/``on_unload``/``on_resume``) OUTSIDE
        ``self._lock`` so a slow or re-entrant module cannot block unrelated
        registry queries or deadlock a callback that calls back into the
        registry. Lock is acquired only for the two short atomic sections:
        capturing the old module's snapshot + state maps, and inserting the
        new instance into all internal maps (``_modules``, ``_module_meta``,
        ``_lowercase_map``, ``_versioned_modules``).
        """
        if self._defer_watched_change(path):
            return
        # Phase 1 (outside lock): compile + instantiate + validate new module.
        try:
            candidate = self._watched_candidate(Path(path))
            if candidate is None:
                return
            new_id = candidate.canonical_id
            source = candidate.file_path
            with self._lock:
                if new_id in self._modules and self._module_sources.get(new_id) != source:
                    logger.warning("Hot reload skipped '%s' because it is registered from another source", new_id)
                    return
                managed = self._watch_lifecycle is not None
            revision = self._source_revision(source) if managed else None
            metadata = load_metadata(candidate.meta_path) if candidate.meta_path is not None else {}
            cls = resolve_entry_point(source, meta=metadata, pre_approval_hook=self._pre_approval_hook)
        except Exception as e:
            logger.warning("Hot reload failed to resolve entry point for %s: %s", path, e)
            return

        try:
            instance = cls()
        except Exception as e:
            logger.warning("Hot reload failed to instantiate class from %s: %s", path, e)
            return

        validation_errors = validate_module(instance)
        if validation_errors:
            logger.warning(
                "Hot reload rejected module from %s: validation failed (%s)",
                path,
                "; ".join(validation_errors),
            )
            return

        try:
            effective_version = self._effective_version(instance, metadata)
            merged_meta = _prepare_discovered_module(new_id, instance, metadata)
        except Exception as error:
            logger.warning("Hot reload rejected module from %s: %s", path, error)
            return

        if self._custom_validator is not None:
            try:
                errors = self._custom_validator.validate(instance)
            except Exception as e:
                logger.warning(
                    "Hot reload rejected module from %s: custom validator raised: %s",
                    path,
                    e,
                )
                return
            if errors:
                logger.warning("Hot reload rejected module from %s: %s", path, "; ".join(errors))
                return

        # Reuse the registration's adapter on the watcher thread. An opted-in
        # watcher can also introduce async-capable registrations on its loop.
        with self._lock:
            if self._closed or new_id in self._in_flight or new_id in self._draining:
                return
            exists = new_id in self._modules
            current = self._snapshot_registration(new_id)
            hook_loop = current.hook_loop(current.module) if current is not None else self._watch_hook_loop
            options = self._watch_lifecycle
            if exists and options is not None:
                watched_generation = self._lease_generations.setdefault(new_id, object())
            else:
                watched_generation = None
            if not exists:
                self._in_flight.add(new_id)
        if watched_generation is not None and options is not None:
            self._start_watched_replacement(
                new_id,
                instance,
                source,
                effective_version,
                metadata,
                merged_meta,
                hook_loop,
                watched_generation,
                options,
                revision,
            )
            return
        old_registration: _RegistrationSnapshot | None = None
        suspended_state: dict[str, Any] | None = None
        if exists:
            detached = self._detach_after_drain(new_id, reserve_replacement=True, source_path=source)
            if detached is None:
                return
            pending, clean = detached
            old_registration = pending.registration
            # Phase 3: run hooks on the old module outside the registry lock.
            suspended_state = self._capture_state_and_unload(pending, clean)

        with self._resume_on_load(new_id, suspended_state):
            if not self._invoke_on_load(new_id, instance, hook_loop=hook_loop):
                self._restore_after_failed_hot_reload(new_id, old_registration, suspended_state)
                return
            with self._ready_to_publish(new_id, instance, hook_loop=hook_loop) as admitted:
                if not admitted:
                    return
                registration = _ModuleRegistration(instance, merged_meta, metadata, hook_loop)
                self._publish_snapshot(
                    new_id, _RegistrationSnapshot(instance, merged_meta, {effective_version: registration}, source)
                )

        # Phase 5 (outside lock): notify observers of the initialized module.
        self._trigger_event("register", new_id, instance)

    def _start_watched_replacement(
        self,
        module_id: str,
        module: Any,
        source: Path,
        version: str,
        metadata: dict[str, Any],
        merged_meta: dict[str, Any],
        loop: asyncio.AbstractEventLoop | None,
        generation: object,
        options: ReloadOptions,
        revision: tuple[int, int, int, int] | None,
    ) -> None:
        try:
            self._validate_hook_adapter(module_id, module, loop)
            if inspect.iscoroutinefunction(getattr(module, "on_load", None)) and (
                loop is None or not loop.is_running()
            ):
                raise ModuleLoadError(module_id, "Async on_load requires an available lifecycle event loop")
            registration = _ModuleRegistration(module, merged_meta, metadata, loop)
            replacement = _RegistrationSnapshot(module, merged_meta, {version: registration}, source)
            watched = _WatchedReplacement(generation, replacement, source, revision)
            with self._lock:
                if self._closed or self._watch_lifecycle is not options or source in self._watched_replacements:
                    return
                self._watched_replacements[source] = watched
            try:
                self._start_managed_reload(
                    [module_id],
                    options.drain_timeout_ms,
                    options.on_drain_timeout,
                    lambda *_: None,
                    exact=True,
                    prepared=watched,
                )
            except BaseException:
                with self._lock:
                    if self._watched_replacements.get(source) is watched:
                        self._watched_replacements.pop(source)
                raise
        except Exception as error:
            logger.warning("Watcher could not start reload for '%s': %s", module_id, type(error).__name__)

    @staticmethod
    def _source_revision(source: Path) -> tuple[int, int, int, int] | None:
        try:
            details = source.stat()
        except OSError:
            return None
        return (details.st_dev, details.st_ino, details.st_mtime_ns, details.st_size)

    def _defer_watched_change(self, path: str) -> bool:
        """Keep one pending refresh per source while its replacement owns admission."""
        with self._lock:
            if not self._watched_replacements:
                return False
        try:
            source = Path(path).resolve()
        except (OSError, RuntimeError):
            return False
        with self._lock:
            watched = self._watched_replacements.get(source)
            if watched is None:
                return False
            watched.refresh |= self._source_revision(source) != watched.revision
            return True

    def _require_current_watched_source(self, module_id: str, watched: _WatchedReplacement) -> None:
        """Check the candidate revision and watcher ownership under the Registry lock."""
        if self._watched_replacements.get(watched.source) is not watched:
            raise ModuleLoadError(module_id, "Watcher stopped before module replacement")
        watched.refresh |= self._source_revision(watched.source) != watched.revision
        if watched.refresh or watched.revision is None:
            raise ModuleLoadError(module_id, "Watched module source changed during replacement")

    def _finish_watched_replacement(self, watched: _WatchedReplacement) -> None:
        with self._lock:
            if self._watched_replacements.get(watched.source) is not watched:
                return
            self._watched_replacements.pop(watched.source)
            if self._closed or self._watch_lifecycle is None:
                return
            revision = self._source_revision(watched.source)
            if not watched.refresh and revision == watched.revision:
                return
        if revision is None:
            self._handle_file_deletion(str(watched.source))
        else:
            self._handle_file_change(str(watched.source))

    def _restore_after_failed_hot_reload(
        self,
        module_id: str,
        registration: _RegistrationSnapshot | None,
        suspended_state: dict[str, Any] | None,
    ) -> None:
        """Republish the previous instance after a hot reload failed to load.

        D-123, applying D-112 rules 2-4 to the watch-driven path. Phase 2 has
        already unregistered the old instance and Phase 3 has already run its
        ``on_unload``, so restoring it means re-running ``on_load`` — a module
        republished without it would be visible but torn down, which D-112 rule
        2 names as the state this whole decision exists to avoid.

        When the restoring load ALSO fails the module stays unavailable
        (D-112 rule 3): there is no good state left to return to, and publishing
        a module whose load hook failed is the defect the deferred-publish rule
        exists to prevent. Nothing is restored when the reload created a module
        that did not previously exist — there is no previous instance to return
        to, and the file simply failed to load.
        """
        if registration is None:
            logger.error(
                "Hot reload of '%s' failed its on_load() and there was no previous "
                "instance to restore; the module is not registered.",
                module_id,
            )
            return

        if not self._restore_registration(module_id, registration, suspended_state):
            logger.error(
                "Hot reload of '%s' failed its on_load(), and re-loading the previous "
                "instance failed as well; the module stays unavailable (D-112 rule 3).",
                module_id,
            )
            return

        logger.warning(
            "Hot reload of '%s' failed its on_load(); the previous instance was restored.",
            module_id,
        )

    def _call_on_suspend(
        self, module_id: str, module: Any, hook_loop: asyncio.AbstractEventLoop | None = None
    ) -> dict[str, Any] | None:
        """Call on_suspend on ``module`` outside the registry lock; return state dict or None."""
        if not (hasattr(module, "on_suspend") and callable(module.on_suspend)):
            return None
        try:
            raw_state: Any = _call_lifecycle_hook(module.on_suspend, (), hook_loop, module_id, "on_suspend")
        except Exception as e:
            logger.error(
                "on_suspend() failed for module '%s' during hot reload: %s",
                module_id,
                e,
            )
            return None
        if raw_state is None:
            return None
        if isinstance(raw_state, dict):
            try:
                json.dumps(raw_state, allow_nan=False)
            except (TypeError, ValueError):
                logger.warning("on_suspend() for module '%s' returned non-JSON state; ignoring", module_id)
            else:
                return raw_state
        logger.warning("on_suspend() for module '%s' returned non-dict; ignoring", module_id)
        return None

    def _call_on_unload(
        self,
        module_id: str,
        module: Any,
        hook_progress: Callable[[bool], None] | None = None,
        hook_loop: asyncio.AbstractEventLoop | None = None,
    ) -> LifecycleError | None:
        """Call on_unload on ``module`` outside the registry lock."""
        try:
            if hasattr(module, "on_unload"):
                if hook_progress is not None:
                    hook_progress(True)
                try:
                    _call_lifecycle_hook(module.on_unload, (), hook_loop, module_id, "on_unload")
                finally:
                    if hook_progress is not None:
                        hook_progress(False)
        except Exception as e:
            logger.error(
                "on_unload() failed for module '%s': %s",
                module_id,
                e,
            )
            return LifecycleError._from_exception(e, "unloading")
        return None

    def _call_on_resume(
        self, module_id: str, module: Any, state: dict[str, Any], hook_loop: asyncio.AbstractEventLoop | None = None
    ) -> None:
        if hasattr(module, "on_resume") and callable(module.on_resume):
            try:
                _call_lifecycle_hook(module.on_resume, (state,), hook_loop, module_id, "on_resume")
            except Exception as exc:
                logger.error("on_resume() failed for module '%s': %s", module_id, exc)

    def _handle_file_deletion(self, path: str) -> None:
        """Drain the module, then unload it when its source file is deleted."""
        if self._defer_watched_change(path):
            return
        try:
            source = Path(path).resolve()
        except (OSError, RuntimeError):
            logger.warning("Hot reload could not resolve deleted path %s", path)
            return
        if source.exists():
            return
        with self._lock:
            module_ids = [module_id for module_id, file_path in self._module_sources.items() if file_path == source]
        for module_id in module_ids:
            detached = self._detach_after_drain(module_id, source_path=source)
            if detached is not None:
                self._finish_unload(detached[0])

    # ----- Public accessors for internal state -----

    def get_module_metadata(self, module_id: str) -> dict[str, Any]:
        """Return metadata dict for a module, or empty dict if not found."""
        with self._lock:
            return dict(self._module_meta.get(module_id, {}))

    def get_declared_annotations(self, module_id: str) -> Any | None:
        """Return the merged (descriptor) annotations for a module, or ``None``.

        This is the SECOND governance source PROTOCOL_SPEC §7.4 (D-96) unions
        with the live module instance. It is not a duplicate of the instance's
        own ``annotations``: :func:`merge_module_metadata` folds a
        ``*.binding.yaml`` / ``metadata=`` declaration into this slot with
        YAML > code precedence, so an operator can declare governance HERE that
        the instance never carries.

        Reading it is what closes a bypass: before this existed, a metadata
        source declaring ``requires_approval: true`` reached ``get_definition``
        and the manifest, and reached no gate — the module executed ungated with
        an approval handler configured.

        Cheap by design: one dict read under the lock, because the approval
        gate consults it on every call. Callers union it with the instance's
        annotations via :func:`apcore.schema.governance_union`; they MUST NOT
        use it alone, since the merge's YAML > code precedence would let a
        metadata ``false`` cancel a module that asks to be gated.
        """
        with self._lock:
            meta = self._module_meta.get(module_id)
            return meta.get("annotations") if meta is not None else None

    def register_internal(self, module_id: str, module: Any) -> None:
        """Register a sys/internal module that bypasses **only** the reserved
        word check.

        All other PROTOCOL_SPEC §2.7 validations (empty, EBNF pattern, length,
        duplicate) still apply. The intended use case is registering modules
        under reserved prefixes like ``system.health`` or
        ``system.control.toggle_feature`` from ``apcore.sys_modules``.

        Aligned with apcore-typescript ``Registry.registerInternal`` and
        apcore-rust ``Registry::register_internal``.

        Raises:
            InvalidInputError: If module_id is empty, malformed, exceeds the
                length limit, is already registered, or falls under the
                ``ephemeral.*`` namespace (which must use :meth:`register`).
            StreamingInterfaceError: If ``annotations.streaming`` is True but
                the module does not implement a conformant ``stream()``.
            RuntimeError: If module.on_load() fails (propagated).
        """
        # Per protocol-spec §2.5.1: ephemeral.* IDs MUST be rejected
        # here. Namespace → registration-mechanism is a 1:1 mapping; mixing
        # blurs the audit-trail distinction between framework-emitted
        # (system.*) and caller-emitted (ephemeral.*) modules.
        # A-D-REG-002: use the shared _is_ephemeral helper so the bare
        # ``ephemeral`` ID (no dot) is rejected here too — ``startswith``
        # missed it, leaving a one-character carveout that contradicted
        # the canonical _is_ephemeral classifier used everywhere else.
        # Typed, coded error like every other rejection in this method: a bare
        # builtin ``ValueError`` carried no error code, so no conformance
        # fixture could cover the path and no caller could branch on it.
        # apcore-typescript throws ``InvalidInputError``; apcore-rust returns
        # ``ModuleError(GeneralInvalidInput)``.
        if _is_ephemeral(module_id):
            raise InvalidInputError(
                message=(
                    f"ephemeral.* module IDs must be registered via Registry.register(), "
                    f"not register_internal() (got: {module_id!r}). "
                    "See protocol-spec §2.5.1 for rationale."
                ),
                code=ErrorCodes.INVALID_MODULE_ID,
            )
        _validate_module_id(module_id, allow_reserved=True)

        # Issue #62: a module that declares ``streaming=True`` MUST implement a
        # conformant ``stream()``. This method bypasses **only** the
        # reserved-word check — its docstring has always said so — but the
        # streaming check ran on ``register()`` alone, so a sys module could
        # advertise streaming it cannot do. apcore-rust routes both entry points
        # through the shared ``register_core``, which performs the check.
        _validate_streaming_annotation(module_id, module, getattr(module, "annotations", None))

        _ensure_schema_adapter(module)

        # Mirror register(): populate _module_meta via the same merge path
        # so get_definition() reads merged metadata uniformly regardless of
        # which registration entry point was used. Pass the instance so
        # __init__-set attributes are picked up.
        merged_meta = merge_module_metadata(module, {})

        # A-D-REG-004: register_internal now routes through the SAME
        # deferred-publish + module_load_failed-emit helper that
        # register() uses, so the Issue #65 invariant ("modules are
        # invisible until on_load completes") holds uniformly across
        # every registration path, not just the public API.
        with self._lock:
            self._require_open()
            self._reject_unloading_instance(module_id, module)
            if module_id in self._in_flight:
                raise InvalidInputError(
                    message=f"Module '{module_id}' is already being registered (in-flight)",
                    code=ErrorCodes.DUPLICATE_MODULE_ID,
                )
            # Sync finding A-D-002: use detect_id_conflicts with an empty
            # reserved-words set so case-collision detection still runs on
            # the sys/internal path. Apcore-rust's register_core (called by
            # register_internal) does this; previously Python+TS used a bare
            # `module_id in self._modules` check, losing case-collision
            # symmetry. The lowercase-only EBNF pattern enforced by
            # _validate_module_id makes the case mismatch unreachable today,
            # but keeping the contract surface aligned across SDKs preserves
            # the invariant for future relaxations.
            conflict = detect_id_conflicts(
                module_id,
                set(self._modules.keys()),
                reserved_words=frozenset(),
                lowercase_map=self._lowercase_map,
            )
            if conflict is not None and conflict.severity == ConflictSeverity.ERROR:
                raise InvalidInputError(
                    message=conflict.message,
                    code=ErrorCodes.DUPLICATE_MODULE_ID,
                )
            self._in_flight.add(module_id)

        # Phase 2: call on_load() OUTSIDE the lock. Module is not visible
        # yet. On failure: remove from in-flight, emit module_load_failed,
        # re-raise the original exception unchanged (no wrapping).
        self._initialize_module(module_id, module)

        # Phase 3: atomically publish into the visible store.
        with self._ready_to_publish(module_id, module) as admitted:
            if not admitted:
                raise InvalidInputError(message="The module registry is shut down")
            self._modules[module_id] = module
            self._module_meta[module_id] = merged_meta
            self._lowercase_map[module_id.lower()] = module_id

        self._trigger_event("register", module_id, module)

    # ----- Cache -----

    def clear_cache(self) -> None:
        """Clear the schema cache."""
        with self._lock:
            self._schema_cache.clear()
