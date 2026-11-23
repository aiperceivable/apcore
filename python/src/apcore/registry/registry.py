"""Central module registry for discovering, registering, and querying modules."""

from __future__ import annotations

import json
import logging
import os
import re
import threading
import time
from contextlib import contextmanager
from pathlib import Path
from typing import TYPE_CHECKING, Any, Callable, Iterator, Protocol, runtime_checkable

import inspect

from apcore.config import Config
from apcore.errors import (
    ErrorCodes,
    InvalidInputError,
    ModuleError,
    ModuleNotFoundError,
    StreamingInterfaceError,
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
from apcore.registry.scanner import scan_extensions, scan_multi_root
from apcore.registry.types import DependencyInfo, ModuleDescriptor
from apcore.registry.validation import validate_module
from apcore.registry.version import VersionedStore

if TYPE_CHECKING:
    from apcore.config import Config

logger = logging.getLogger(__name__)


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
        from apcore.errors import SchemaValidationError
        from apcore.schema.hardening import validate_schema_dict

        result = validate_schema_dict(data, self._schema)
        if not result.valid:
            raise SchemaValidationError(
                message=f"Input validation failed: {result.errors}",
                errors=result.to_error().details["errors"],
            )
        return data

    def model_rebuild(self) -> None:
        pass


def _ensure_schema_adapter(module: Any) -> None:
    """Wrap raw dict schemas on *module* with ``_DictSchemaAdapter`` in-place."""
    for attr in ("input_schema", "output_schema"):
        value = getattr(module, attr, None)
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


def _validate_streaming_annotation(module_id: str, module: Any) -> None:
    """Enforce Issue #62 when ``annotations.streaming`` is explicitly True.

    Shared by ``register`` and ``register_internal`` so a sys module cannot
    advertise streaming it does not implement. Previously only ``register``
    performed the check, contradicting ``register_internal``'s own docstring
    ("bypasses **only** the reserved word check"); apcore-rust routes both entry
    points through ``register_core``, which performs it.

    Accepts both annotation carriers: a ``ModuleAnnotations`` dataclass and a
    raw dict.
    """
    annotations = getattr(module, "annotations", None)
    if isinstance(annotations, dict):
        streaming_declared = annotations.get("streaming") is True
    elif annotations is not None:
        streaming_declared = getattr(annotations, "streaming", None) is True
    else:
        streaming_declared = False
    if streaming_declared:
        _validate_streaming_signature(module_id, module)


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
    if not module_id:
        raise InvalidInputError(
            message="module_id must be a non-empty string",
            code=ErrorCodes.INVALID_MODULE_ID,
        )

    # 2. EBNF pattern check
    if not MODULE_ID_PATTERN.match(module_id):
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


class _ModuleChangeHandler:
    """Watchdog event handler for file-system events on extension roots.

    Hoisted to module scope (from an inline class inside ``Registry.watch``)
    so it can be unit-tested directly and so the debounce dict stays on a
    long-lived object with bounded memory. Registry is held via a weakref to
    avoid extending its lifetime beyond the caller's expectation.
    """

    _DEBOUNCE_WINDOW_SEC: float = 0.3
    _MAX_DEBOUNCE_ENTRIES: int = 1024
    _DEBOUNCE_PRUNE_KEEP: int = 256

    def __init__(self, registry: "Registry") -> None:
        import weakref

        self._registry_ref: Any = weakref.ref(registry)
        self._debounce_timer: dict[str, float] = {}

    def _registry(self) -> "Registry | None":
        return self._registry_ref()

    def _should_process(self, path: str) -> bool:
        if not path.endswith(".py"):
            return False
        now = time.time()
        last = self._debounce_timer.get(path, 0.0)
        if now - last < self._DEBOUNCE_WINDOW_SEC:
            return False
        self._debounce_timer[path] = now
        # Bounded memory: prune oldest entries when the dict grows too large.
        if len(self._debounce_timer) > self._MAX_DEBOUNCE_ENTRIES:
            sorted_by_age = sorted(self._debounce_timer.items(), key=lambda kv: kv[1])
            self._debounce_timer = dict(sorted_by_age[-self._DEBOUNCE_PRUNE_KEEP :])
        return True

    def on_modified(self, event: Any) -> None:
        if event.is_directory:
            return
        path = str(event.src_path)
        if not self._should_process(path):
            return
        reg = self._registry()
        if reg is not None:
            reg._handle_file_change(path)

    def on_created(self, event: Any) -> None:
        if event.is_directory:
            return
        path = str(event.src_path)
        if not self._should_process(path):
            return
        reg = self._registry()
        if reg is not None:
            reg._handle_file_change(path)

    def on_deleted(self, event: Any) -> None:
        if event.is_directory:
            return
        path = str(event.src_path)
        if not path.endswith(".py"):
            return
        reg = self._registry()
        if reg is not None:
            reg._handle_file_deletion(path)


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
        self._lowercase_map: dict[str, str] = {}
        # Versioned storage for multi-version module support (F18)
        self._versioned_modules: VersionedStore[Any] = VersionedStore()
        self._versioned_meta: VersionedStore[dict[str, Any]] = VersionedStore()
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
        self._draining: set[str] = set()
        self._drain_events: dict[str, threading.Event] = {}

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
        # A relative value is used AS DECLARED — resolved against the process
        # working directory, exactly as `extensions.root` two blocks up is.
        # PROTOCOL_SPEC §9.2.1 states that the resolution base for path-typed
        # keys is deliberately unspecified and tracked in #113: `acl.root`
        # resolves against the config file's directory, `schema.root` against
        # the CWD. This key is not the place to settle that — but it does have
        # to pick, and it picks the base its SIBLING uses. `id_map.overrides`
        # and `extensions.root` are two halves of one discovery configuration
        # and are always read together, so a split base between them would be
        # worse than either base. Recorded as an input to #113, not an answer.
        resolved_id_map = id_map_path
        if resolved_id_map is None and config is not None:
            from_config = config.get("id_map.overrides")
            if isinstance(from_config, str) and from_config.strip():
                resolved_id_map = from_config
        if resolved_id_map is not None:
            self._id_map = load_id_map(Path(resolved_id_map))

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
                    entries.append({"root": item, "namespace": Path(item).name})
                elif isinstance(item, dict) and item.get("root"):
                    root_value = item["root"]
                    entries.append(
                        {
                            "root": root_value,
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

        ext_root = config.get("extensions.root")
        if ext_root:
            return [{"root": ext_root}]
        return [{"root": Config.get_default("extensions.root")}]

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
        if self._custom_discoverer is not None:
            return self._discover_custom()
        return self._discover_default(path_filter=path_filter)

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

    def _discover_custom(self) -> int:
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
                self.register(mod_id, mod, _skip_custom_validator=True)
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
    ) -> int:
        """Run discovery using the default file-system scanning logic.

        Orchestrates 8 named stages; per-stage logic lives in the dedicated
        helpers below:

        1. ``_scan_roots`` -- walk extension root(s) for candidate files.
        2. ``_apply_path_filter`` -- narrow the candidate set (optional).
        3. ``_apply_id_map_overrides`` -- apply explicit ID overrides.
        4. ``_load_all_metadata`` -- read each module's metadata block.
        5. ``_resolve_all_entry_points`` -- import the entry-point classes.
        6. ``_validate_all`` -- structural / schema validation.
        7. ``_resolve_load_order`` -- dependency-aware ordering.
        8. ``_filter_id_conflicts`` then ``_register_in_order`` --
           deduplicate and register in order.

        Mirrors the structure of
        ``apcore-typescript/src/registry/registry.ts:_discoverDefault``.
        """
        max_depth, follow_symlinks, ignore_patterns = self._scan_params()
        discovered = self._scan_roots(max_depth, follow_symlinks, ignore_patterns)
        if path_filter is not None:
            discovered = self._apply_path_filter(discovered, path_filter)
        self._apply_id_map_overrides(discovered)
        raw_metadata = self._load_all_metadata(discovered)
        resolved_classes = self._resolve_all_entry_points(discovered, raw_metadata)
        valid_classes = self._validate_all(resolved_classes)
        load_order = self._resolve_load_order(valid_classes, raw_metadata)
        valid_classes = self._filter_id_conflicts(load_order, valid_classes)
        registered_count = self._register_in_order(load_order, valid_classes, raw_metadata)

        if registered_count == 0 and discovered:
            logger.warning(
                "No modules successfully registered from %d discovered files",
                len(discovered),
            )
        elif registered_count == 0:
            logger.warning("No modules discovered")

        return registered_count

    def _scan_params(self) -> tuple[int, bool]:
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

    def _scan_roots(self, max_depth: int, follow_symlinks: bool, ignore_patterns: list[str] | None = None) -> list[Any]:
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
            root_path = Path(self._extension_roots[0]["root"])
            discovered = scan_extensions(
                root=root_path,
                max_depth=max_depth,
                follow_symlinks=follow_symlinks,
                ignore_patterns=ignore_patterns,
            )
        self._reject_ephemeral_discoveries(discovered)
        return discovered

    @staticmethod
    def _reject_ephemeral_discoveries(discovered: list[Any]) -> None:
        """Reject filesystem-derived IDs that fall in the reserved ``ephemeral.*`` namespace.

        Per protocol-spec §2.5.1, ``ephemeral.*`` is reserved
        for programmatically-registered modules synthesized at runtime. Any
        filesystem layout that produces such an ID is a configuration error —
        either the directory is misnamed or the namespace prefix is being
        misused.
        """
        offenders = [dm for dm in discovered if _is_ephemeral(getattr(dm, "canonical_id", ""))]
        if not offenders:
            return
        ids = sorted({dm.canonical_id for dm in offenders})
        raise InvalidInputError(
            message=(
                "Filesystem discovery produced module ID(s) in the reserved "
                f"'ephemeral.*' namespace: {ids}. The ephemeral.* namespace is "
                "reserved for programmatically-registered modules and may only "
                "be used via Registry.register(). Rename the offending "
                "directory or extension namespace."
            ),
            code=ErrorCodes.INVALID_MODULE_ID,
        )

    def _apply_path_filter(
        self,
        discovered: list[Any],
        path_filter: str | list[str],
    ) -> list[Any]:
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

    def _apply_id_map_overrides(self, discovered: list[Any]) -> None:
        """Stage 2 — rewrite ``canonical_id`` for files listed in the ID map."""
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

    def _load_all_metadata(self, discovered: list[Any]) -> dict[str, dict[str, Any]]:
        """Stage 3 — read each module's optional companion ``*_meta.yaml``."""
        raw_metadata: dict[str, dict[str, Any]] = {}
        for dm in discovered:
            raw_metadata[dm.canonical_id] = load_metadata(dm.meta_path) if dm.meta_path else {}
        return raw_metadata

    def _resolve_all_entry_points(
        self,
        discovered: list[Any],
        raw_metadata: dict[str, dict[str, Any]],
    ) -> dict[str, type]:
        """Stage 4 — resolve each discovered file to its module class."""
        resolved: dict[str, type] = {}
        for dm in discovered:
            meta = raw_metadata.get(dm.canonical_id, {})
            try:
                resolved[dm.canonical_id] = resolve_entry_point(
                    dm.file_path,
                    meta=meta,
                    pre_approval_hook=self._pre_approval_hook,
                )
            except Exception as e:
                logger.warning("Failed to resolve entry point for '%s': %s", dm.canonical_id, e)
        return resolved

    def _validate_all(self, resolved_classes: dict[str, type]) -> dict[str, type]:
        """Stage 5 — run the (custom or built-in) validator over each class."""
        valid: dict[str, type] = {}
        for mod_id, cls in resolved_classes.items():
            if self._custom_validator is not None:
                errors = self._custom_validator.validate(cls)
            else:
                errors = validate_module(cls)
            if errors:
                logger.warning("Module '%s' failed validation: %s", mod_id, "; ".join(errors))
                continue
            valid[mod_id] = cls
        return valid

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
        """Stage 7.5 — drop classes whose IDs are invalid or collide.

        Two checks, in the order ``register()`` applies them:

        1. **PROTOCOL_SPEC §2.7 grammar** (empty → pattern → length → reserved).
           ``_validate_module_id`` used to be reachable only from ``register()``
           and ``register_internal()``, so nothing on the discovery path enforced
           the ID grammar at all. ``_apply_id_map_overrides`` is the concrete
           vector: it rewrites ``canonical_id`` from the YAML ID map *after*
           scanning and never re-validates, so ``{'id': 'Foo-Bar'}`` or a
           200-character ID reached ``_modules`` intact. Hyphens in particular
           are banned to keep MCP / OpenAI tool-name normalisation bijective.
        2. **A03 batch conflict detection** (duplicate / reserved / case).

        Both are non-fatal: the offending module is dropped with a warning and
        the rest of the discovery run proceeds. Raising would let one bad ID-map
        entry take down every module under the same root. Parity with
        apcore-typescript ``_filterIdConflicts`` (validate-then-detect, skip on
        failure) and apcore-rust, which validates each discovered entry.
        """
        filtered = dict(valid_classes)
        batch_ids: set[str] = set()
        for mod_id in load_order:
            try:
                _validate_module_id(mod_id, allow_reserved=False)
            except InvalidInputError as exc:
                logger.warning("Skipping discovered module with invalid ID '%s': %s", mod_id, exc)
                filtered.pop(mod_id, None)
                continue

            conflict = detect_id_conflicts(
                new_id=mod_id,
                existing_ids=batch_ids | set(self._modules.keys()),
                reserved_words=RESERVED_WORDS,
                lowercase_map=self._lowercase_map,
            )
            if conflict is not None:
                if conflict.severity == "error":
                    logger.warning("Skipping module '%s': %s", mod_id, conflict.message)
                    filtered.pop(mod_id, None)
                else:
                    logger.warning("ID conflict: %s", conflict.message)
            batch_ids.add(mod_id)
        return filtered

    def _register_in_order(
        self,
        load_order: list[str],
        valid_classes: dict[str, type],
        raw_metadata: dict[str, dict[str, Any]],
    ) -> int:
        """Stage 8 — instantiate, register, and run on_load() for each module.

        Populates both the primary ``_modules`` map AND the multi-version
        ``_versioned_modules`` / ``_versioned_meta`` stores so that
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
            # Pass the instance (not the class) so __init__-set attributes
            # are picked up; getattr falls through to class attrs anyway.
            # Aligned with manual register() / register_internal().
            merged_meta = merge_module_metadata(module, meta)

            # Phase 1: reserve in-flight slot. Skip if already in-flight or
            # visible — concurrent discover invocations must not double-load.
            with self._lock:
                if mod_id in self._in_flight or mod_id in self._modules:
                    logger.warning("Discover skipping '%s' — already registered or in-flight", mod_id)
                    continue
                self._in_flight.add(mod_id)

            # Phase 2: run on_load OUTSIDE the lock. Module is NOT visible.
            if not self._invoke_on_load(mod_id, module, effective_version):
                # _invoke_on_load already discarded the in-flight slot and
                # emitted apcore.registry.module_load_failed. Move on.
                continue

            # Phase 3: atomic publish.
            with self._lock:
                self._in_flight.discard(mod_id)
                self._versioned_modules.add(mod_id, effective_version, module)
                if meta:
                    self._versioned_meta.add(mod_id, effective_version, meta)
                self._modules[mod_id] = module
                self._module_meta[mod_id] = merged_meta
                self._lowercase_map[mod_id.lower()] = mod_id

            self._trigger_event("register", mod_id, module)
            registered_count += 1
        return registered_count

    @staticmethod
    def _effective_version(module: Any, meta: dict[str, Any]) -> str:
        """Resolve the effective version for a module from YAML > instance > default."""
        yaml_version = meta.get("version") if meta else None
        code_version = getattr(module, "version", None)
        resolved = yaml_version or code_version or DEFAULT_MODULE_VERSION
        if not isinstance(resolved, str):
            resolved = str(resolved)
        return resolved

    def _invoke_on_load(self, mod_id: str, module: Any, effective_version: str) -> bool:
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
        if not (hasattr(module, "on_load") and callable(module.on_load)):
            return True
        try:
            _run_on_load(module, mod_id)
        except Exception as e:
            logger.error("on_load() failed for module '%s': %s", mod_id, e)
            with self._lock:
                self._in_flight.discard(mod_id)
            # A-D-REG-005: align with the public register() path — every
            # registration site that observes an on_load failure must
            # emit the canonical module_load_failed event so subscribers
            # have a single hook for partial-init detection.
            self._emit_module_load_failed(mod_id, e)
            return False
        return True

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
        _validate_module_id(module_id, allow_reserved=False)
        # ``ephemeral.*`` registrations only land via this programmatic
        # entry point — the filesystem discoverer rejects matching IDs
        # earlier (see ``_reject_ephemeral_discoveries``). When invoked
        # here protocol-spec §2.5.1 recommends ``requires_approval=true`` so a
        # human gates execution of agent-synthesized code.
        ephemeral = _is_ephemeral(module_id)
        if ephemeral:
            self._warn_if_missing_approval(module_id, module, metadata)

        _ensure_schema_adapter(module)

        # D-86: structure/streaming BEFORE the custom validator, and both
        # before the duplicate check further down. Intrinsic-then-extrinsic —
        # what is wrong with the module itself is reported before what is
        # wrong about where it is being put. Matches apcore-rust and
        # apcore-typescript; Python previously ran the validator first, so a
        # module that was both streaming-malformed and validator-rejected
        # reported a different error here than in the other two SDKs.
        _validate_streaming_annotation(module_id, module)

        if not _skip_custom_validator and self._custom_validator is not None:
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
            # Check the in-flight set FIRST so concurrent same-ID registrations are
            # rejected even when on_load is running (module not yet in _modules).
            if not is_versioned and module_id in self._in_flight:
                raise InvalidInputError(
                    message=f"Module '{module_id}' is already being registered (in-flight)",
                    code=ErrorCodes.DUPLICATE_MODULE_ID,
                )

            # For explicit versioned registration, skip conflict check when
            # the module_id already exists (we allow multiple versions).
            # For non-versioned registration, preserve original conflict detection.
            if is_versioned and self._versioned_modules.has(module_id):
                # Multi-version: allow adding another version
                pass
            else:
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
            if not is_versioned:
                self._in_flight.add(module_id)

        # Phase 2: call on_load() OUTSIDE the lock.
        # The module is not visible during this phase. Concurrent same-ID
        # registrations will see module_id in _in_flight and raise DUPLICATE_MODULE_ID.
        if hasattr(module, "on_load") and callable(module.on_load):
            try:
                _run_on_load(module, module_id)
            except Exception as exc:
                with self._lock:
                    self._in_flight.discard(module_id)
                self._emit_module_load_failed(module_id, exc)
                raise

        # Phase 3: atomically insert into visible store.
        with self._lock:
            self._in_flight.discard(module_id)

            # Store in versioned store
            self._versioned_modules.add(module_id, effective_version, module)
            if metadata:
                self._versioned_meta.add(module_id, effective_version, metadata)

            # Always point the primary map to the latest version
            latest = self._versioned_modules.get_latest(module_id)
            if latest is not None:
                self._modules[module_id] = latest
            self._module_meta[module_id] = merged_meta
            self._lowercase_map[module_id.lower()] = module_id

        self._trigger_event("register", module_id, module)
        if ephemeral:
            self._emit_ephemeral_audit("apcore.registry.module_registered", module_id, context)

    def unregister(self, module_id: str, *, context: Any = None) -> bool:
        """Remove a module from the registry.

        Args:
            module_id: ID of the module to remove.
            context: Optional execution context. Used solely to enrich the
                audit event emitted for ``ephemeral.*`` unregistrations
                (RFC pilot). Ignored for non-ephemeral modules.

        Returns False if module was not registered.
        """
        with self._lock:
            if module_id not in self._modules:
                return False
            module = self._modules.pop(module_id)
            self._module_meta.pop(module_id, None)
            self._schema_cache.pop(module_id, None)
            self._lowercase_map.pop(module_id.lower(), None)
            self._versioned_modules.remove_all(module_id)
            self._versioned_meta.remove_all(module_id)
            # Clear hot-reload tracking state to avoid stale entries if
            # unregister() is called directly (not via safe_unregister()).
            self._draining.discard(module_id)
            self._drain_events.pop(module_id, None)
            self._ref_counts.pop(module_id, None)

        # Call on_unload if available
        if hasattr(module, "on_unload") and callable(module.on_unload):
            try:
                module.on_unload()
            except Exception as e:
                logger.error("on_unload() failed for module '%s': %s", module_id, e)

        self._trigger_event("unregister", module_id, module)
        if _is_ephemeral(module_id):
            self._emit_ephemeral_audit("apcore.registry.module_unregistered", module_id, context)
        return True

    # ----- Query Methods -----

    def get(self, module_id: str, version_hint: str | None = None) -> Any:
        """Look up a module by ID and optional version hint. Returns None if not found.

        If version_hint is provided, resolves to the best matching version.
        If no hint, returns the latest version.

        Raises:
            ModuleNotFoundError: If module_id is empty string, or if a
                ``version_hint`` is supplied but ``module_id`` was registered
                via :meth:`register_internal` (which deliberately skips
                ``_versioned_modules`` / ``_versioned_meta`` — D11-001).
                Sys/internal modules are not version-tracked; callers asking
                for version resolution on one get a clear error rather than
                a silent ``None``.
        """
        if module_id == "":
            raise ModuleNotFoundError(module_id="")
        with self._lock:
            if version_hint is not None:
                resolved = self._versioned_modules.resolve(module_id, version_hint)
                if resolved is not None:
                    return resolved
                # Distinguish two "no match in versioned store" cases:
                # 1. Module was registered via the public register() path —
                #    appears in BOTH _modules AND _versioned_modules — the
                #    hint just didn't match any registered version. Return
                #    None to preserve "no-match returns None" semantics
                #    expected by version-negotiation callers.
                # 2. Module was registered via register_internal() — appears
                #    in _modules but NOT in _versioned_modules. Sys/internal
                #    modules deliberately opt out of version tracking
                #    (D11-001). Raising a clear error beats silently
                #    returning None which masks the asymmetry.
                if module_id in self._modules and not self._versioned_modules.has(module_id):
                    raise ModuleNotFoundError(
                        module_id=module_id,
                        message=(
                            f"Module '{module_id}' is registered as a sys/internal "
                            "module (via register_internal) and is not version-tracked; "
                            "drop the version_hint argument to look it up by ID alone."
                        ),
                    )
                return None
            # No hint: return latest from versioned store if available,
            # otherwise fall back to primary map (covers register_internal).
            latest = self._versioned_modules.get_latest(module_id)
            if latest is not None:
                return latest
            return self._modules.get(module_id)

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
        include_hidden: bool | None = None,
    ) -> list[str]:
        """Return sorted list of unique registered module IDs, optionally filtered.

        Args:
            tags: When supplied, only modules carrying *all* of the given tags
                are returned.
            prefix: When supplied, only IDs starting with the prefix are returned.
            visibility: Filter by module visibility. Supported: ``["public", "hidden"]``.
                Defaults to ``["public"]``. Pass ``["public", "hidden"]`` to see
                all modules (protocol-spec §4.4 ``discoverable``).
            include_hidden: Deprecated. Use ``visibility=["public", "hidden"]`` instead.
        """
        # protocol-spec §4.4 discoverable: the visibility list takes precedence over the legacy include_hidden bool.
        if visibility is not None:
            vis = set(visibility)
            show_public = "public" in vis
            show_hidden = "hidden" in vis
        elif include_hidden is True:
            show_public = True
            show_hidden = True
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
        are filtered out. Use :meth:`list` with ``include_hidden=True`` when
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
        meta = module_meta.get(module_id)
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
        module = modules.get(module_id)
        if module is None:
            return True
        ann = getattr(module, "annotations", None)
        if ann is None:
            return True
        if isinstance(ann, dict):
            return ann.get("discoverable", True) is not False
        return getattr(ann, "discoverable", True) is not False

    def get_definition(self, module_id: str, version_hint: str | None = None) -> ModuleDescriptor | None:
        """Get a ModuleDescriptor for a registered module. Returns None if not found.

        Args:
            module_id: The module ID.
            version_hint: Optional version hint for selecting a specific version.
        """
        module = self.get(module_id, version_hint=version_hint)
        if module is None:
            return None

        with self._lock:
            meta = dict(self._module_meta.get(module_id, {}))
            # Resolve versioned metadata under the version the module was
            # REGISTERED with, which `_module_meta` now carries. Keying off
            # `getattr(module, "version")` missed the versioned store whenever
            # `register(..., version=...)` supplied a version the module object
            # does not carry itself — so the caller's metadata silently vanished
            # from the descriptor alongside the version (sync finding A-D-002).
            version_str = meta.get("version") or getattr(module, "version", None) or DEFAULT_MODULE_VERSION
            versioned_meta = self._versioned_meta.get(module_id, version_str)
            if versioned_meta:
                meta["metadata"] = {**meta.get("metadata", {}), **versioned_meta}

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

        effective_metadata = meta.get("metadata", {})

        # Log deprecation warning if x-deprecation is present
        deprecation = effective_metadata.get("x-deprecation")
        if deprecation:
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
        # metadata payload. Aligned with apcore-typescript Registry.getDefinition.
        return ModuleDescriptor(
            module_id=module_id,
            name=meta.get("name"),
            description=meta.get("description") or "",
            documentation=meta.get("documentation"),
            input_schema=input_json,
            output_schema=output_json,
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
            import copy

            from apcore.schema.strict import to_strict_schema

            input_schema = to_strict_schema(copy.deepcopy(input_schema))
            output_schema = to_strict_schema(copy.deepcopy(output_schema))

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
        """Context manager to track in-flight executions for safe hot-reload.

        Increments the reference count for the module on entry and decrements
        it on exit.  If the module is draining (marked for unload), raises
        ``ModuleNotFoundError`` to prevent new executions.

        Yields:
            The module instance.

        Raises:
            ModuleNotFoundError: If the module is currently draining.
        """
        with self._lock:
            if module_id in self._draining:
                raise ModuleNotFoundError(module_id=module_id)
            if module_id not in self._modules:
                raise ModuleNotFoundError(module_id=module_id)
            self._ref_counts[module_id] = self._ref_counts.get(module_id, 0) + 1
            module = self._modules[module_id]
        try:
            yield module
        finally:
            with self._lock:
                count = self._ref_counts.get(module_id, 1) - 1
                self._ref_counts[module_id] = count
                if count <= 0:
                    self._ref_counts.pop(module_id, None)
                    event = self._drain_events.get(module_id)
                    if event:
                        event.set()

    def release(self, module_id: str) -> None:
        """Decrement the reference count for a module.

        Standalone counterpart to the ``acquire()`` context manager for
        cases where the caller cannot use a ``with`` block (e.g. async
        streams that span multiple await points).

        If the ref count reaches zero and the module is draining, the
        drain event is signalled.
        """
        with self._lock:
            count = self._ref_counts.get(module_id, 1) - 1
            self._ref_counts[module_id] = count
            if count <= 0:
                self._ref_counts.pop(module_id, None)
                event = self._drain_events.get(module_id)
                if event:
                    event.set()

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
        with self._lock:
            if module_id not in self._modules:
                return False
            self._draining.add(module_id)
            ref_count = self._ref_counts.get(module_id, 0)
            if ref_count > 0:
                event = threading.Event()
                self._drain_events[module_id] = event
            else:
                event = None

        clean = True
        if event is not None:
            if not event.wait(timeout=timeout_ms / 1000.0):
                logger.warning(
                    "Force-unloading module %s after %dms timeout (%d in-flight executions)",
                    module_id,
                    timeout_ms,
                    self._ref_counts.get(module_id, 0),
                )
                clean = False

        # Perform actual unregistration
        with self._lock:
            self._draining.discard(module_id)
            self._drain_events.pop(module_id, None)
            self._ref_counts.pop(module_id, None)

        self.unregister(module_id)
        return clean

    # ----- Hot Reload -----

    def watch(self) -> None:
        """Start watching extension directories for file changes.

        Requires the optional ``watchdog`` dependency.
        Raises ImportError if watchdog is not installed.
        """
        try:
            from watchdog.observers import Observer  # type: ignore[import-not-found]
        except ImportError:
            raise ImportError("watchdog is required for hot reload. Install it with: pip install watchdog")

        if hasattr(self, "_observer") and self._observer is not None:
            return  # Already watching

        self._observer = Observer()
        handler = _ModuleChangeHandler(self)

        for root_config in self._extension_roots:
            root_path = root_config.get("root", "")
            if root_path and os.path.isdir(root_path):
                self._observer.schedule(handler, root_path, recursive=True)

        self._observer.start()

    def unwatch(self) -> None:
        """Stop watching extension directories for file changes."""
        if hasattr(self, "_observer") and self._observer is not None:
            self._observer.stop()
            self._observer.join()
            self._observer = None

    def _inline_unregister(self, mid: str) -> Any:
        """Remove a module from internal maps (caller must hold self._lock).

        Returns the removed module instance, or None if not found.
        """
        module = self._modules.pop(mid, None)
        self._module_meta.pop(mid, None)
        self._schema_cache.pop(mid, None)
        self._lowercase_map.pop(mid.lower(), None)
        return module

    def _handle_file_change(self, path: str) -> None:
        """Handle a file modification or creation event.

        Performs arbitrary-Python import (``resolve_entry_point``) and user
        hooks (``on_suspend``/``on_unload``/``on_resume``) OUTSIDE
        ``self._lock`` so a slow or re-entrant module cannot block unrelated
        registry queries or deadlock a callback that calls back into the
        registry. Lock is acquired only for the two short atomic sections:
        capturing the old module's snapshot + state maps, and inserting the
        new instance into all internal maps (``_modules``, ``_module_meta``,
        ``_lowercase_map``, ``_versioned_modules``, ``_versioned_meta``).
        """
        # Phase 1 (outside lock): compile + instantiate + validate new module.
        try:
            cls = resolve_entry_point(Path(path), pre_approval_hook=self._pre_approval_hook)
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
        if self._custom_validator is not None:
            try:
                self._custom_validator.validate(instance)
            except Exception as e:
                logger.warning(
                    "Hot reload rejected module from %s: custom validator raised: %s",
                    path,
                    e,
                )
                return

        # Phase 2 (under lock): snapshot old module + inline unregister from
        # all stores. Do NOT call user hooks or fire events while holding the lock.
        with self._lock:
            new_id = (
                self._canonical_id_for_path(path)
                or self._path_to_module_id(path)
                or os.path.splitext(os.path.basename(path))[0]
            )
            old_module = self._modules.get(new_id)
            self._inline_unregister(new_id)
            self._versioned_modules.remove_all(new_id)
            self._versioned_meta.remove_all(new_id)

        # Phase 3 (outside lock): user hooks on the old module + unregister event.
        suspended_state: dict[str, Any] | None = None
        if old_module is not None:
            suspended_state = self._call_on_suspend(new_id, old_module)
            self._call_on_unload(new_id, old_module)
            self._trigger_event("unregister", new_id, old_module)

        # Phase 4 (under lock): insert the new instance into every store.
        effective_version = getattr(instance, "version", None) or "1.0.0"
        # `_module_meta` is rebuilt from the new instance here because Phase 2's
        # `_inline_unregister` popped the old entry. Omitting it left every
        # reloaded module with an empty descriptor: `get_definition()` fell back
        # to description="" / version="1.0.0" / tags=[] and
        # `get_module_metadata()` returned {} — which also silently degraded
        # `ReloadModule._topo_sort_modules` (it reads the stored
        # `dependencies`) to alphabetical order.
        merged_meta = merge_module_metadata(instance, {})

        # Phase 4a (outside lock): D-123 — the new instance MUST run `on_load()`
        # before it becomes visible, the same precondition `Contract:
        # Registry.register` Side Effects step 8 places on every other
        # registration. That step is scoped to `register`, and this path writes
        # the internal maps directly, so it violated no stated rule while
        # publishing a module that was visible but never initialised — harder to
        # diagnose than one that is simply absent.
        if not self._invoke_on_load(new_id, instance, effective_version):
            # D-112 rules 2-4, applied here via D-123: restore the previous
            # instance rather than leaving the module gone. A failed hot reload
            # must not make a working module disappear.
            self._restore_after_failed_hot_reload(new_id, old_module, suspended_state)
            return

        # Phase 4b (under lock): insert the new instance into every store.
        with self._lock:
            self._modules[new_id] = instance
            self._module_meta[new_id] = merged_meta
            self._lowercase_map[new_id.lower()] = new_id
            self._versioned_modules.add(new_id, effective_version, instance)
            self._versioned_meta.add(new_id, effective_version, {"version": effective_version})

        # Phase 5 (outside lock): register event + on_resume hook on new module.
        self._trigger_event("register", new_id, instance)
        if suspended_state is not None and hasattr(instance, "on_resume") and callable(instance.on_resume):
            try:
                instance.on_resume(suspended_state)
            except Exception as e:
                logger.error(
                    "on_resume() failed for module '%s' during hot reload: %s",
                    new_id,
                    e,
                )

    def _restore_after_failed_hot_reload(
        self,
        module_id: str,
        old_module: Any | None,
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
        if old_module is None:
            logger.error(
                "Hot reload of '%s' failed its on_load() and there was no previous "
                "instance to restore; the module is not registered.",
                module_id,
            )
            return

        old_version = getattr(old_module, "version", None) or "1.0.0"
        if not self._invoke_on_load(module_id, old_module, old_version):
            logger.error(
                "Hot reload of '%s' failed its on_load(), and re-loading the previous "
                "instance failed as well; the module stays unavailable (D-112 rule 3).",
                module_id,
            )
            return

        merged_meta = merge_module_metadata(old_module, {})
        with self._lock:
            self._modules[module_id] = old_module
            self._module_meta[module_id] = merged_meta
            self._lowercase_map[module_id.lower()] = module_id
            self._versioned_modules.add(module_id, old_version, old_module)
            self._versioned_meta.add(module_id, old_version, {"version": old_version})

        self._trigger_event("register", module_id, old_module)
        if suspended_state is not None and hasattr(old_module, "on_resume") and callable(old_module.on_resume):
            try:
                old_module.on_resume(suspended_state)
            except Exception as e:
                logger.error(
                    "on_resume() failed for restored module '%s' after a failed hot reload: %s",
                    module_id,
                    e,
                )
        logger.warning(
            "Hot reload of '%s' failed its on_load(); the previous instance was restored.",
            module_id,
        )

    def _call_on_suspend(self, module_id: str, module: Any) -> dict[str, Any] | None:
        """Call on_suspend on ``module`` outside the registry lock; return state dict or None."""
        if not (hasattr(module, "on_suspend") and callable(module.on_suspend)):
            return None
        try:
            raw_state: Any = module.on_suspend()
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
            return raw_state
        logger.warning("on_suspend() for module '%s' returned non-dict; ignoring", module_id)
        return None

    def _call_on_unload(self, module_id: str, module: Any) -> None:
        """Call on_unload on ``module`` outside the registry lock."""
        if not hasattr(module, "on_unload"):
            return
        try:
            module.on_unload()
        except Exception as e:
            logger.warning(
                "on_unload() failed for module '%s' during hot reload: %s",
                module_id,
                e,
            )

    def _handle_file_deletion(self, path: str) -> None:
        """Handle a file deletion event.

        Snapshots the module under the lock, removes it from every internal
        store (including versioned), then calls ``on_unload`` and fires the
        unregister event OUTSIDE the lock.
        """
        with self._lock:
            module_id = self._canonical_id_for_path(path)
            if module_id not in self._modules:
                module_id = self._path_to_module_id(path)
            if not module_id or module_id not in self._modules:
                return
            module = self._modules.get(module_id)
            self._inline_unregister(module_id)
            self._versioned_modules.remove_all(module_id)
            self._versioned_meta.remove_all(module_id)

        if module is not None:
            self._call_on_unload(module_id, module)
        self._trigger_event("unregister", module_id, module)

    def _canonical_id_for_path(self, path: str) -> str | None:
        """Derive the module ID ``discover()`` would give the file at ``path``.

        Applies the same rules as discovery — path relative to its extension
        root, the root's namespace prefix when discovery would add one, and the
        ID-map override — so a file first seen by the watcher gets the ID it
        would have had if it had existed at startup. Returns None when ``path``
        lies under no configured extension root.
        """
        real = Path(path).resolve()
        prefixed = len(self._extension_roots) > 1 or any("namespace" in r for r in self._extension_roots)
        for root_config in self._extension_roots:
            root = Path(root_config.get("root", "")).resolve()
            try:
                rel = real.relative_to(root)
            except ValueError:
                continue
            if self._id_map and str(rel) in self._id_map:
                return str(self._id_map[str(rel)]["id"])
            canonical_id = str(rel.with_suffix("")).replace(os.sep, ".")
            if prefixed:
                namespace = root_config.get("namespace") or Path(root_config.get("root", "")).name
                canonical_id = f"{namespace}.{canonical_id}"
            return canonical_id
        return None

    def _path_to_module_id(self, path: str) -> str | None:
        """Map a file path to a registered module ID by basename, for paths under no extension root."""
        basename = os.path.splitext(os.path.basename(path))[0]
        # Check if any registered module ID ends with this basename
        for mid in self.module_ids:
            if mid.endswith(basename) or mid == basename:
                return mid
        return None

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
        _validate_streaming_annotation(module_id, module)

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
        if hasattr(module, "on_load") and callable(module.on_load):
            try:
                _run_on_load(module, module_id)
            except Exception as exc:
                with self._lock:
                    self._in_flight.discard(module_id)
                self._emit_module_load_failed(module_id, exc)
                raise

        # Phase 3: atomically publish into the visible store.
        with self._lock:
            self._in_flight.discard(module_id)
            self._modules[module_id] = module
            self._module_meta[module_id] = merged_meta
            self._lowercase_map[module_id.lower()] = module_id

        self._trigger_event("register", module_id, module)

    # ----- Cache -----

    def clear_cache(self) -> None:
        """Clear the schema cache."""
        with self._lock:
            self._schema_cache.clear()
