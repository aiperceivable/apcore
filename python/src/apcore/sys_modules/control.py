"""system.control sys modules -- runtime config updates (F11), hot-reload (F10), toggle (F19)."""

from __future__ import annotations

import logging
import threading
import time
import uuid
from datetime import datetime, timezone
from typing import Any

from apcore.config import Config, _CONSTRAINTS
from apcore.utils.pattern import match_glob
from apcore.utils.redaction import REDACTED_VALUE
from apcore.errors import (
    ConfigError,
    InvalidInputError,
    ModuleDisabledError,
    ModuleError,
    ModuleNotFoundError,
    ModuleReloadConflictError,
    ReloadFailedError,
)
from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.module import ModuleAnnotations
from apcore.registry.registry import Registry, _RegistrationSnapshot
from apcore.sys_modules.audit import AuditEntry, AuditStore
from apcore.sys_modules.identity import _audit_payload_extras
from apcore.sys_modules.overrides import FileOverridesStore, OverridesStore, _persist_override

__all__ = [
    "UpdateConfigModule",
    "ReloadModule",
    "ReloadModuleModule",  # backward-compat alias
    "ToggleFeatureModule",
    "ToggleState",
]

# ---------------------------------------------------------------------------
# Toggle feature state — shared across ToggleFeatureModule instances
# ---------------------------------------------------------------------------


class ToggleState:
    """Thread-safe toggle state container.

    Can be shared across ToggleFeatureModule instances or isolated per-Registry.
    State survives module reload since it lives outside the Registry.
    """

    def __init__(self) -> None:
        self._disabled: set[str] = set()
        self._lock = threading.Lock()

    def is_disabled(self, module_id: str) -> bool:
        """Check if a module is disabled."""
        with self._lock:
            return module_id in self._disabled

    def disable(self, module_id: str) -> None:
        """Mark a module as disabled."""
        with self._lock:
            self._disabled.add(module_id)

    def enable(self, module_id: str) -> None:
        """Mark a module as enabled."""
        with self._lock:
            self._disabled.discard(module_id)

    def clear(self) -> None:
        """Clear all toggle state (useful for testing)."""
        with self._lock:
            self._disabled.clear()


# Default global instance for backward compatibility
_default_toggle_state = ToggleState()


def is_module_disabled(module_id: str) -> bool:
    """Check if a module is disabled using the default toggle state. Thread-safe."""
    return _default_toggle_state.is_disabled(module_id)


def check_module_disabled(module_id: str) -> None:
    """Raise ModuleDisabledError if the module is disabled."""
    if is_module_disabled(module_id):
        raise ModuleDisabledError(module_id=module_id)


logger = logging.getLogger(__name__)

#: Keys that cannot be changed at runtime.
_RESTRICTED_KEYS: frozenset[str] = frozenset({"sys_modules.enabled"})


def _build_audit_entry(
    action: str,
    target_module_id: str,
    context: Any,
    change: dict[str, Any],
    correlation_id: str = "",
) -> AuditEntry:
    """Build an AuditEntry from context identity.

    ``correlation_id`` groups the entries one multi-module operation produced
    (D-111); it is empty for single-target operations, which need no grouping.
    """
    identity = getattr(context, "identity", None) if context is not None else None
    actor_id = getattr(identity, "id", "unknown") if identity is not None else "unknown"
    actor_type = getattr(identity, "type", "unknown") if identity is not None else "unknown"
    trace_id = getattr(context, "trace_id", "") if context is not None else ""
    return AuditEntry(
        timestamp=datetime.now(timezone.utc).isoformat(),
        action=action,
        target_module_id=target_module_id,
        actor_id=actor_id,
        actor_type=actor_type,
        trace_id=trace_id or "",
        change=change,
        correlation_id=correlation_id,
    )


class UpdateConfigModule:
    """Update a runtime configuration value by dot-path key."""

    description = "Update a runtime configuration value by dot-path key"
    # SYS-19: `idempotent` is declared per module by system-modules.md, and all
    # three `system.control.*` modules took the `ModuleAnnotations` default
    # (False) instead. Two happened to match; `toggle_feature` did not. Stated
    # explicitly so the value is the spec's rather than the dataclass's.
    # "repeated calls with different values produce different state".
    # D-119: written out rather than inherited — the ModuleAnnotations default is
    # `open_world=True`, which means the opposite of the intended value, and relying
    # on it is how the divergence arose. No system module reaches an external system.
    annotations = ModuleAnnotations(requires_approval=True, destructive=False, idempotent=False, open_world=False)
    input_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "key": {"type": "string", "description": "Dot-path config key"},
            "value": {"description": "New value"},
            "reason": {"type": "string", "description": "Audit reason"},
        },
        "required": ["key", "value", "reason"],
    }
    output_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "success": {
                "type": "boolean",
                "description": "Whether the update succeeded",
            },
            "key": {"type": "string", "description": "Updated config key"},
            "old_value": {"description": "Previous value (redacted for sensitive keys)"},
            "new_value": {"description": "New value (redacted for sensitive keys)"},
        },
        "required": ["success", "key", "old_value", "new_value"],
    }

    def __init__(
        self,
        config: Config,
        event_emitter: EventEmitter,
        overrides_path: str | None = None,
        audit_store: AuditStore | None = None,
        overrides_store: OverridesStore | None = None,
    ) -> None:
        self._config = config
        self._emitter = event_emitter
        self._overrides_path = overrides_path
        self._audit_store = audit_store
        self._overrides_store = overrides_store

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        """Update a config value, emit event, and optionally persist + audit."""
        key, value, reason = self._validate_inputs(inputs)
        constraint_key = key.removeprefix("apcore.")
        self._check_restricted(constraint_key)
        self._validate_value(constraint_key, value)

        old_value = self._config.get(key)
        self._config.set(key, value)

        self._emit_event(key, old_value, value, context)
        self._log_change(key, old_value, value, reason)

        if self._overrides_store is not None:
            _persist_override(self._overrides_store, key, value)
        elif self._overrides_path:
            _persist_override(FileOverridesStore(self._overrides_path), key, value)

        redacted = self._is_sensitive_key(key)
        if self._audit_store is not None:
            entry = _build_audit_entry(
                action="update_config",
                target_module_id="system.control.update_config",
                context=context,
                change={
                    "before": REDACTED_VALUE if redacted else old_value,
                    "after": REDACTED_VALUE if redacted else value,
                },
            )
            self._audit_store.append(entry)
        return {
            "success": True,
            "key": key,
            "old_value": REDACTED_VALUE if redacted else old_value,
            "new_value": REDACTED_VALUE if redacted else value,
        }

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _validate_inputs(inputs: dict[str, Any]) -> tuple[str, Any, str]:
        """Validate and extract key, value, reason from inputs."""
        key: str = inputs.get("key", "")
        if not key:
            raise InvalidInputError(message="'key' is required and must not be empty")

        reason: str = inputs.get("reason", "")
        if not reason:
            raise InvalidInputError(message="'reason' is required and must not be empty")

        value: Any = inputs.get("value")
        return key, value, reason

    @staticmethod
    def _check_restricted(key: str) -> None:
        """Raise if key is in the restricted keys list."""
        if key in _RESTRICTED_KEYS:
            raise ModuleError(
                code="CONFIG_KEY_RESTRICTED",
                message=f"Configuration key '{key}' cannot be changed at runtime",
                details={"key": key},
            )

    def _validate_value(self, key: str, value: Any) -> None:
        """Validate the candidate before publishing it to readers."""
        if key not in _CONSTRAINTS:
            return

        check_fn, err_msg = _CONSTRAINTS[key]
        if not check_fn(value):
            raise ConfigError(
                message=f"Invalid value for '{key}': {err_msg} (got {value!r})",
                details={"key": key, "value": value},
            )

    def _emit_event(self, key: str, old_value: Any, new_value: Any, context: Any = None) -> None:
        """Emit ``apcore.config.updated`` (canonical event) with caller identity."""
        redacted = self._is_sensitive_key(key)
        data: dict[str, Any] = {
            "key": key,
            "old_value": REDACTED_VALUE if redacted else old_value,
            "new_value": REDACTED_VALUE if redacted else new_value,
        }
        # Issue #45.2 — attach requester identity to the audit event payload.
        data.update(_audit_payload_extras(context))
        self._emitter.emit(
            ApCoreEvent(
                event_type="apcore.config.updated",
                module_id="system.control.update_config",
                timestamp=datetime.now(timezone.utc).isoformat(),
                severity="info",
                data=data,
            )
        )

    _SENSITIVE_SEGMENTS = ("token", "secret", "key", "password", "auth", "credential")

    @classmethod
    def _is_sensitive_key(cls, key: str) -> bool:
        """Check if a config key path contains sensitive-sounding segments."""
        return any(
            seg == s or seg.endswith(f"_{s}") or seg.startswith(f"{s}_")
            for seg in key.lower().split(".")
            for s in cls._SENSITIVE_SEGMENTS
        )

    @classmethod
    def _log_change(cls, key: str, old_value: Any, new_value: Any, reason: str) -> None:
        """Log the configuration change at INFO level for audit."""
        if cls._is_sensitive_key(key):
            logger.info(
                "Config updated: key=%s old_value=*** new_value=*** reason=%s",
                key,
                reason,
            )
        else:
            logger.info(
                "Config updated: key=%s old_value=%s new_value=%s reason=%s",
                key,
                old_value,
                new_value,
                reason,
            )


class ReloadModule:
    """Hot-reload a module via safe unregister + re-discover (PRD F10).

    Supports both single-module reload (via module_id) and bulk reload
    (via path_filter glob pattern). The two modes are mutually exclusive.
    """

    description = "Hot-reload a module by safe unregister and re-discover"
    # SYS-19: "each call unregisters and re-registers; invoking twice reloads
    # twice" (system-modules.md).
    # D-119: written out rather than inherited — the ModuleAnnotations default is
    # `open_world=True`, which means the opposite of the intended value, and relying
    # on it is how the divergence arose. No system module reaches an external system.
    annotations = ModuleAnnotations(requires_approval=True, destructive=False, idempotent=False, open_world=False)
    input_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "module_id": {
                "type": "string",
                "description": "ID of the module to reload (mutually exclusive with path_filter)",
            },
            "path_filter": {
                "type": "string",
                "description": "Glob pattern for bulk reload (mutually exclusive with module_id)",
            },
            "reload_dependents": {
                "type": "boolean",
                "description": (
                    "DEPRECATED (spec v1.51.0, D-121) — ignored, and removed at 2.0. "
                    "Declared in all three SDKs and implemented by none, so no caller "
                    "could ever rely on it. Use an explicit `path_filter` that also "
                    "matches the dependents. At 2.0 this field is REMOVED and, because "
                    "this schema sets additionalProperties: false, passing it becomes a "
                    "validation error rather than a silent no-op."
                ),
                "default": False,
                "deprecated": True,
            },
            "reason": {"type": "string", "description": "Audit reason for the reload"},
        },
        "required": ["reason"],
    }
    output_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "success": {
                "type": "boolean",
                "description": "Whether the reload succeeded",
            },
            "module_id": {
                "type": ["string", "null"],
                "description": "ID of the reloaded module (null for bulk reload via path_filter)",
            },
            "previous_version": {
                "type": "string",
                "description": "Version before reload",
            },
            "new_version": {"type": "string", "description": "Version after reload"},
            "reload_duration_ms": {
                "type": "number",
                "description": "Reload duration in milliseconds",
            },
        },
        "required": ["success", "module_id"],
    }

    def __init__(
        self,
        registry: Registry,
        event_emitter: EventEmitter,
        audit_store: AuditStore | None = None,
    ) -> None:
        self._registry = registry
        self._emitter = event_emitter
        self._audit_store = audit_store
        self._reload_dependents_warned = False

    # Once per INSTANCE, not per process. `reload` is called by hot-reload loops
    # and watchers, so an advisory whose volume is proportional to traffic is one
    # operators learn to filter out — the cadence reasoning D-89 settled. Per
    # instance rather than per process for the reason D-90's notice is per token:
    # a process-wide one-shot tells the first caller and leaves every later one
    # to discover it in production.
    def _warn_reload_dependents(self, inputs: dict[str, Any]) -> None:
        """Warn once that ``reload_dependents`` is ignored and will be removed (D-121).

        The field was declared in all three SDKs' input schemas and read by
        none — a spec MUST that no implementation satisfied, which is the
        §9.1.3 "declared surface reaches no mechanism" shape applied to a module
        input field. Three independent implementations skipping it is the
        evidence the maintainer decision rests on.

        Deprecated rather than removed now, because the input schema sets
        ``additionalProperties: false``: at 2.0 the same call stops being a
        silent no-op and becomes a validation error, so a caller passing it
        today needs a release in which they are told.
        """
        if not inputs.get("reload_dependents"):
            return
        if self._reload_dependents_warned:
            return
        self._reload_dependents_warned = True
        logger.warning(
            "system.control.reload: 'reload_dependents' is ignored and will be removed at 2.0 "
            "(spec D-121). No SDK has ever implemented it. Use a 'path_filter' that also matches "
            "the dependents; after removal this field becomes a validation error, not a no-op."
        )

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        """Reload one or more modules.

        Raises ModuleReloadConflictError if both module_id and path_filter are given.
        ``reload_dependents`` is accepted and ignored — see :meth:`_warn_reload_dependents`.
        """
        module_id: Any = inputs.get("module_id")
        path_filter: Any = inputs.get("path_filter")
        self._warn_reload_dependents(inputs)
        reason = self._validate_reason(inputs)

        if module_id is not None and path_filter is not None:
            raise ModuleReloadConflictError()

        if path_filter is not None:
            return self._execute_bulk(path_filter, reason, context)

        return self._execute_single(module_id, reason, context)

    # ------------------------------------------------------------------
    # Single-module reload
    # ------------------------------------------------------------------

    def _execute_single(self, module_id: Any, reason: str, context: Any) -> dict[str, Any]:
        """Reload a single module by ID."""
        if module_id is None or not isinstance(module_id, str):
            raise InvalidInputError(message="'module_id' is required and must be a string")
        if not module_id:
            raise InvalidInputError(message="'module_id' must not be empty")

        previous = self._registry._snapshot_registration(module_id)
        if previous is None:
            raise ModuleNotFoundError(module_id=module_id)
        previous_version = str(previous.metadata.get("version") or "1.0.0")
        start = time.monotonic()
        suspended_state = self._try_suspend(module_id, previous.module)

        self._registry.safe_unregister(module_id)

        try:
            new_module = self._rediscover_module(module_id)
            self._reregister_module(module_id, new_module)
        except Exception as exc:
            # D-112: put the previous instance back rather than leaving a
            # working module gone. Both steps are inside the guard because a
            # re-registration that fails leaves exactly the same hole as a
            # re-discovery that fails.
            self._restore_after_failed_reload(module_id, previous, suspended_state)
            if isinstance(exc, ReloadFailedError):
                raise
            raise ReloadFailedError(module_id=module_id, reason=str(exc)) from exc

        if suspended_state is not None:
            self._try_resume(module_id, new_module, suspended_state)

        elapsed_ms = (time.monotonic() - start) * 1000.0
        new_version = getattr(new_module, "version", "1.0.0")

        self._emit_module_reloaded(module_id, previous_version, new_version, context)
        self._log_reload(module_id, previous_version, new_version, reason)

        if self._audit_store is not None:
            entry = _build_audit_entry(
                action="reload_module",
                target_module_id=module_id,
                context=context,
                change={"before": previous_version, "after": new_version},
            )
            self._audit_store.append(entry)

        return {
            "success": True,
            "module_id": module_id,
            "previous_version": previous_version,
            "new_version": new_version,
            "reload_duration_ms": elapsed_ms,
        }

    # ------------------------------------------------------------------
    # Bulk reload via path_filter
    # ------------------------------------------------------------------

    def _execute_bulk(self, path_filter: Any, reason: str, context: Any) -> dict[str, Any]:
        """Reload all modules matching the pattern in topological order.

        PROTOCOL_SPEC 6.7 clause 4: `path_filter` is a glob-dialect pattern
        matched with Algorithm A25 (9.2.3) against each registered module ID.
        Not fnmatch: `[em]` is a literal here, not a character class.
        """
        if not isinstance(path_filter, str) or not path_filter:
            raise InvalidInputError(message="'path_filter' must be a non-empty string glob pattern")

        all_ids = self._registry.module_ids
        matched = sorted(mid for mid in all_ids if match_glob(path_filter, mid))

        topo_order = self._topo_sort_modules(matched)
        start = time.monotonic()

        reloaded: list[str] = []
        for mid in topo_order:
            try:
                self._reload_one(mid, context)
                reloaded.append(mid)
            except Exception as exc:
                # SYS-17 — fatal, not swallowed.
                #
                # `_reload_one` unregisters the module BEFORE it re-discovers
                # it, so a re-discovery failure leaves the module gone. Logging
                # and continuing meant the method returned
                # `{"success": true, "reloaded_modules": []}` for a bulk reload
                # that unregistered every matched module and restored none —
                # the worst possible pairing of an outcome with a report,
                # because the caller has no reason to look further. Both
                # apcore-typescript and apcore-rust make it fatal.
                #
                # Already-reloaded modules stay reloaded; the error names the
                # one that stopped the run, and the partial list is not
                # reported as a success.
                logger.error("Bulk reload: failed to reload '%s': %s", mid, exc)
                if isinstance(exc, ReloadFailedError):
                    raise
                raise ReloadFailedError(module_id=mid, reason=str(exc)) from exc

        elapsed_ms = (time.monotonic() - start) * 1000.0
        logger.info(
            "Bulk reload: reloaded %d modules via path_filter=%r reason=%s",
            len(reloaded),
            path_filter,
            reason,
        )

        if self._audit_store is not None:
            # D-111: one entry PER MODULE, not one keyed on the glob.
            #
            # `AuditStore.query(module_id=...)` filters on a concrete id, so an
            # entry whose `target_module_id` was `executor.*` is unfindable by
            # the accessor the store exists for — the operation was audited and
            # could not be looked up.
            #
            # Per-module entries alone lose the fact that they were ONE deploy,
            # so they share a correlation id and "what did this deploy touch"
            # stays a single query. Generated once, here, rather than per entry.
            correlation_id = str(uuid.uuid4())
            for mid in reloaded:
                self._audit_store.append(
                    _build_audit_entry(
                        action="reload_module",
                        target_module_id=mid,
                        context=context,
                        change={"before": None, "after": mid},
                        correlation_id=correlation_id,
                    )
                )

        return {
            "success": True,
            "module_id": None,
            "reloaded_modules": reloaded,
            "reload_duration_ms": elapsed_ms,
        }

    def _topo_sort_modules(self, module_ids: list[str]) -> list[str]:
        """Return module_ids in dependency topological order (leaves first)."""
        from apcore.registry.dependencies import resolve_dependencies
        from apcore.registry.metadata import parse_dependencies

        matched_set = set(module_ids)
        entries: list[tuple[str, list[Any]]] = []
        for mid in module_ids:
            meta = self._registry.get_module_metadata(mid)
            deps_raw = meta.get("dependencies", [])
            deps = parse_dependencies(deps_raw) if deps_raw else []
            # Only include deps that are also in the matched set
            filtered_deps = [d for d in deps if d.module_id in matched_set]
            entries.append((mid, filtered_deps))

        try:
            return resolve_dependencies(entries, known_ids=matched_set)
        except Exception as exc:
            logger.warning(
                "Topo sort failed for path_filter reload; falling back to alphabetical: %s",
                exc,
            )
            return sorted(module_ids)

    def _restore_after_failed_reload(
        self, module_id: str, previous: _RegistrationSnapshot, suspended_state: dict[str, Any] | None
    ) -> None:
        """Restore all previous versions after reinitializing the unloaded module (D-112)."""
        if not self._registry._restore_registration(module_id, previous):
            logger.error("system.control.reload: could not restore '%s' after a failed reload", module_id)
            return
        if suspended_state is not None:
            self._try_resume(module_id, previous.module, suspended_state)

    def _reload_one(self, module_id: str, context: Any = None) -> None:
        """Reload a single module as part of a bulk (path_filter) reload.

        ``context`` is threaded through so the emitted ``apcore.module.reloaded``
        carries the real caller identity. It used to be omitted at the
        ``_emit_module_reloaded`` call below, which made
        ``_audit_payload_extras(None)`` fall back to ``caller_id="@external"`` —
        so an authenticated bulk reload was attributed to an anonymous caller on
        the event bus while the AuditStore entry (built in ``_execute_bulk``,
        which does pass ``context``) recorded the real one. Two records of the
        same action disagreeing about who performed it (sync finding A-D-017).
        """
        previous = self._registry._snapshot_registration(module_id)
        if previous is None:
            return

        suspended_state = self._try_suspend(module_id, previous.module)
        self._registry.safe_unregister(module_id)

        try:
            new_module = self._rediscover_module(module_id)
            self._reregister_module(module_id, new_module)
        except Exception as exc:
            # D-112 rule 4: restoration is PER MODULE. The bulk operation as a
            # whole still fails (D-17), and modules that already reloaded
            # successfully are NOT rolled back — cross-module transactionality
            # is not a primitive the registry has.
            self._restore_after_failed_reload(module_id, previous, suspended_state)
            if isinstance(exc, ReloadFailedError):
                raise
            raise ReloadFailedError(module_id=module_id, reason=str(exc)) from exc

        if suspended_state is not None:
            self._try_resume(module_id, new_module, suspended_state)

        new_version = getattr(new_module, "version", "1.0.0")
        prev_version = previous.metadata.get("version") or "1.0.0"
        self._emit_module_reloaded(module_id, str(prev_version), str(new_version), context)

    # ------------------------------------------------------------------
    # Shared helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _validate_reason(inputs: dict[str, Any]) -> str:
        """Validate and return reason."""
        reason: Any = inputs.get("reason")
        if reason is None or not isinstance(reason, str) or not reason:
            raise InvalidInputError(message="'reason' is required and must be a non-empty string")
        return reason

    def _rediscover_module(self, module_id: str) -> Any:
        """Re-discover the module source and return a new instance."""
        self._registry.discover()
        module = self._registry.get(module_id)
        if module is None:
            raise RuntimeError(f"Module '{module_id}' was not found after re-discovery")
        return module

    def _reregister_module(self, module_id: str, module: Any) -> None:
        """Re-register a module instance after reload — unless it is already registered.

        Issue #33 — ``_rediscover_module`` calls ``Registry.discover()``, and
        discovery *registers* what it finds. On the real filesystem path
        ``module`` is therefore already published under ``module_id`` by the
        time it is handed here, and an unconditional ``register_internal``
        raised ``InvalidInputError(DUPLICATE_MODULE_ID)`` on *every* reload:
        loudly from ``_execute_single``, silently from ``_reload_one``, whose
        caller ``_execute_bulk`` logs the failure and still returns
        ``success: True`` with an empty ``reloaded_modules``.

        Publishing is therefore conditional on the registry not already holding
        this exact instance:

        * Same object already registered -> discovery published it; the reload
          is complete. Its entry is also the higher-fidelity one (version-
          tracked in ``_versioned_modules`` with merged metadata), which
          ``register_internal`` would have downgraded to a sys/internal entry
          that ``get(id, version_hint=...)`` cannot resolve (D11-001).
        * Nothing registered (programmatic reload, or a test that stubs
          discovery) -> ``register_internal`` publishes as before.
        * A *different* object registered under the id -> a genuine duplicate;
          ``register_internal`` still raises.
        """
        if self._registry.get(module_id) is module:
            return
        self._registry.register_internal(module_id, module)

    def _emit_module_reloaded(
        self,
        module_id: str,
        previous_version: str,
        new_version: str,
        context: Any = None,
    ) -> None:
        """Emit ``apcore.module.reloaded`` (canonical event) with caller identity."""
        data: dict[str, Any] = {
            "module_id": module_id,
            "previous_version": previous_version,
            "new_version": new_version,
        }
        # Issue #45.2 — attach requester identity to the audit event payload.
        data.update(_audit_payload_extras(context))
        self._emitter.emit(
            ApCoreEvent(
                event_type="apcore.module.reloaded",
                module_id=module_id,
                timestamp=datetime.now(timezone.utc).isoformat(),
                severity="info",
                data=data,
            )
        )

    @staticmethod
    def _try_suspend(module_id: str, module: Any) -> dict[str, Any] | None:
        """Call on_suspend() on the module if available. Returns state or None."""
        if not hasattr(module, "on_suspend") or not callable(module.on_suspend):
            return None
        try:
            state = module.on_suspend()
            if state is not None and not isinstance(state, dict):
                logger.warning(
                    "on_suspend() for module '%s' returned non-dict (%s); ignoring",
                    module_id,
                    type(state).__name__,
                )
                return None
            return state
        except Exception as exc:
            logger.error("on_suspend() failed for module '%s': %s", module_id, exc)
            return None

    @staticmethod
    def _try_resume(module_id: str, module: Any, state: dict[str, Any]) -> None:
        """Call on_resume() on the module if available."""
        if not hasattr(module, "on_resume") or not callable(module.on_resume):
            return
        try:
            module.on_resume(state)
        except Exception as exc:
            logger.error("on_resume() failed for module '%s': %s", module_id, exc)

    @staticmethod
    def _log_reload(module_id: str, previous_version: str, new_version: str, reason: str) -> None:
        """Log the reload at INFO level for audit."""
        logger.info(
            "Module reloaded: module_id=%s previous_version=%s new_version=%s reason=%s",
            module_id,
            previous_version,
            new_version,
            reason,
        )


class ToggleFeatureModule:
    """Disable or enable a module without unloading it from the Registry (PRD F19)."""

    description = "Disable or enable a module without unloading it"
    # SYS-19: "toggling to the current state produces the same outcome"
    # (system-modules.md) — the one of the three whose declared value differs
    # from the dataclass default, and the one that was therefore wrong.
    # D-119: written out rather than inherited — the ModuleAnnotations default is
    # `open_world=True`, which means the opposite of the intended value, and relying
    # on it is how the divergence arose. No system module reaches an external system.
    annotations = ModuleAnnotations(requires_approval=True, destructive=False, idempotent=True, open_world=False)
    input_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "module_id": {
                "type": "string",
                "description": "ID of the module to toggle",
            },
            "enabled": {
                "type": "boolean",
                "description": "True to enable, false to disable",
            },
            "reason": {"type": "string", "description": "Audit reason for the toggle"},
        },
        "required": ["module_id", "enabled", "reason"],
    }
    output_schema: dict[str, Any] = {
        "type": "object",
        "properties": {
            "success": {
                "type": "boolean",
                "description": "Whether the toggle succeeded",
            },
            "module_id": {"type": "string", "description": "ID of the toggled module"},
            "enabled": {"type": "boolean", "description": "Current enabled state"},
        },
        "required": ["success", "module_id", "enabled"],
    }

    def __init__(
        self,
        registry: Registry,
        event_emitter: EventEmitter,
        toggle_state: ToggleState | None = None,
        overrides_path: str | None = None,
        audit_store: AuditStore | None = None,
        overrides_store: OverridesStore | None = None,
    ) -> None:
        self._registry = registry
        self._emitter = event_emitter
        self._toggle_state = toggle_state or _default_toggle_state
        self._overrides_path = overrides_path
        self._audit_store = audit_store
        self._overrides_store = overrides_store

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        """Toggle a module's enabled/disabled state."""
        module_id, enabled, reason = self._validate_inputs(inputs)
        self._check_module_exists(module_id)

        before = not self._toggle_state.is_disabled(module_id)
        self._apply_toggle(module_id, enabled)
        self._emit_event(module_id, enabled, context)
        self._log_toggle(module_id, enabled, reason)

        if self._overrides_store is not None:
            _persist_override(
                self._overrides_store,
                f"toggle.{module_id}",
                enabled,
            )
        elif self._overrides_path:
            _persist_override(
                FileOverridesStore(self._overrides_path),
                f"toggle.{module_id}",
                enabled,
            )

        if self._audit_store is not None:
            entry = _build_audit_entry(
                action="toggle_feature",
                target_module_id=module_id,
                context=context,
                change={"before": before, "after": enabled},
            )
            self._audit_store.append(entry)

        return {
            "success": True,
            "module_id": module_id,
            "enabled": enabled,
        }

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _validate_inputs(inputs: dict[str, Any]) -> tuple[str, bool, str]:
        """Validate and extract module_id, enabled, reason from inputs."""
        module_id: Any = inputs.get("module_id")
        if module_id is None or not isinstance(module_id, str) or not module_id:
            raise InvalidInputError(message="'module_id' is required and must be a non-empty string")

        enabled: Any = inputs.get("enabled")
        if enabled is None or not isinstance(enabled, bool):
            raise InvalidInputError(message="'enabled' is required and must be a boolean")

        reason: Any = inputs.get("reason")
        if reason is None or not isinstance(reason, str) or not reason:
            raise InvalidInputError(message="'reason' is required and must be a non-empty string")

        return module_id, enabled, reason

    def _check_module_exists(self, module_id: str) -> None:
        """Raise ModuleNotFoundError if the module is not in the Registry."""
        if not self._registry.has(module_id):
            raise ModuleNotFoundError(module_id=module_id)

    def _apply_toggle(self, module_id: str, enabled: bool) -> None:
        """Add or remove module_id from the disabled set."""
        if enabled:
            self._toggle_state.enable(module_id)
        else:
            self._toggle_state.disable(module_id)

    def _emit_event(self, module_id: str, enabled: bool, context: Any = None) -> None:
        """Emit ``apcore.module.toggled`` (canonical event) with caller identity."""
        data: dict[str, Any] = {"module_id": module_id, "enabled": enabled}
        # Issue #45.2 — attach requester identity to the audit event payload.
        data.update(_audit_payload_extras(context))
        self._emitter.emit(
            ApCoreEvent(
                event_type="apcore.module.toggled",
                module_id=module_id,
                timestamp=datetime.now(timezone.utc).isoformat(),
                severity="info",
                data=data,
            )
        )

    @staticmethod
    def _log_toggle(module_id: str, enabled: bool, reason: str) -> None:
        """Log the toggle at INFO level for audit."""
        logger.info(
            "Module toggled: module_id=%s enabled=%s reason=%s",
            module_id,
            enabled,
            reason,
        )


# ---------------------------------------------------------------------------
# Backward-compat alias — removes doubled "Module" suffix
# ---------------------------------------------------------------------------

#: Backward-compatible alias for :class:`ReloadModule`.
ReloadModuleModule = ReloadModule
