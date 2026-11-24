"""Auto-registration of sys.* modules and middleware from config (PRD F1-F9)."""

from __future__ import annotations

import logging
from datetime import datetime, timezone
from typing import Any, Callable, TypedDict


from apcore.config import Config
from apcore.errors import SysModuleRegistrationError
from apcore.events.circuit_breaker import CircuitBreakerWrapper
from apcore.events.emitter import ApCoreEvent, EventEmitter, EventSubscriber
from apcore.events.retry import EventRetryConfig
from apcore.events.subscribers import (
    A2ASubscriber,
    FileSubscriber,
    FilterSubscriber,
    StdoutSubscriber,
    WebhookSubscriber,
)
from apcore.executor import Executor
from apcore.middleware.error_history_middleware import ErrorHistoryMiddleware
from apcore.middleware.platform_notify import PlatformNotifyMiddleware
from apcore.observability.error_history import ErrorHistory
from apcore.observability.metrics import MetricsCollector
from apcore.registry.registry import Registry
from apcore.sys_modules.audit import AuditStore
from apcore.sys_modules.control import (
    ReloadModule,
    ToggleFeatureModule,
    ToggleState,
    UpdateConfigModule,
    _default_toggle_state,
)
from apcore.sys_modules.health import HealthModule, HealthSummaryModule
from apcore.sys_modules.overrides import FileOverridesStore, OverridesStore
from apcore.sys_modules.manifest import ManifestFullModule, ManifestModule
from apcore.sys_modules.usage import UsageModule, UsageSummaryModule
from apcore.observability.usage import UsageCollector, UsageMiddleware

logger = logging.getLogger(__name__)

__all__ = [
    "register_sys_modules",
    "register_subscriber_type",
    "unregister_subscriber_type",
    "reset_subscriber_registry",
    "register_subscriber_factory",
    "create_subscriber_from_config",
    "SysModulesContext",
]


class SysModulesContext(TypedDict, total=False):
    """Type of the dict returned by :func:`register_sys_modules`.

    Exported at the package root (``from apcore import SysModulesContext``) for
    cross-language parity with the TypeScript and Rust SDKs.

    All keys are optional because they are only present when the corresponding
    feature is enabled in the ``sys_modules`` config section.
    """

    error_history: Any
    event_emitter: Any
    metrics_collector: Any
    usage_collector: Any
    audit_store: Any


# ---------------------------------------------------------------------------
# Subscriber type registry — extensible factory for EventSubscriber types
# ---------------------------------------------------------------------------

#: Maps subscriber type name → factory function(config_dict) → EventSubscriber
_subscriber_factories: dict[str, Callable[[dict[str, Any]], EventSubscriber]] = {}


def _parse_retry_config(cfg: dict[str, Any]) -> EventRetryConfig | None:
    """Build an :class:`EventRetryConfig` from a subscriber's nested ``retry:`` block.

    Implements the per-subscriber retry policy documented in
    ``docs/features/event-system.md`` §"Per-Subscriber Retry Policy" (apcore#85).
    The block is optional and may be partial — omitted keys fall back to the
    spec defaults (``max_attempts=3``, ``initial_backoff_ms=100``,
    ``max_backoff_ms=30000``, ``backoff_multiplier=2.0``).

    Args:
        cfg: Raw subscriber config dict from ``sys_modules.events.subscribers``.

    Returns:
        The parsed policy, or ``None`` when no ``retry:`` block is present, so
        the subscriber falls back to its own default.
    """
    nested = cfg.get("retry")
    if not isinstance(nested, dict):
        return None
    default = EventRetryConfig()
    return EventRetryConfig(
        max_attempts=int(nested.get("max_attempts", default.max_attempts)),
        initial_backoff_ms=int(nested.get("initial_backoff_ms", default.initial_backoff_ms)),
        max_backoff_ms=int(nested.get("max_backoff_ms", default.max_backoff_ms)),
        backoff_multiplier=float(nested.get("backoff_multiplier", default.backoff_multiplier)),
    )


def _default_webhook_factory(cfg: dict[str, Any]) -> EventSubscriber:
    retry_cfg = _parse_retry_config(cfg)
    # Deprecated: flat retry_count is the legacy webhook-only shorthand. It maps
    # to EventRetryConfig.max_attempts + 1 (retry_count counted retries *after*
    # the first attempt; max_attempts counts total attempts). The nested retry:
    # block wins when both are present (apcore#85).
    if retry_cfg is None and "retry_count" in cfg:
        retry_cfg = EventRetryConfig(max_attempts=int(cfg["retry_count"]) + 1)
    return WebhookSubscriber(
        url=cfg["url"],
        headers=cfg.get("headers"),
        timeout_ms=cfg.get("timeout_ms", 5000),
        retry=retry_cfg,
    )


def _default_a2a_factory(cfg: dict[str, Any]) -> EventSubscriber:
    return A2ASubscriber(
        platform_url=cfg["platform_url"],
        auth=cfg.get("auth"),
        timeout_ms=cfg.get("timeout_ms", 5000),
        retry=_parse_retry_config(cfg),
    )


def _default_file_factory(cfg: dict[str, Any]) -> EventSubscriber:
    return FileSubscriber(
        path=cfg["path"],
        append=cfg.get("append", True),
        output_format=cfg.get("format", "json"),
        rotate_bytes=cfg.get("rotate_bytes"),
        retry=_parse_retry_config(cfg),
    )


def _default_stdout_factory(cfg: dict[str, Any]) -> EventSubscriber:
    return StdoutSubscriber(
        output_format=cfg.get("format", "text"),
        level_filter=cfg.get("level_filter"),
        retry=_parse_retry_config(cfg),
    )


def _default_filter_factory(cfg: dict[str, Any]) -> EventSubscriber:
    delegate_cfg = {**cfg.get("delegate_config", {}), "type": cfg["delegate_type"]}
    delegate = _create_subscriber(delegate_cfg)
    return FilterSubscriber(
        delegate=delegate,
        include_events=cfg.get("include_events"),
        exclude_events=cfg.get("exclude_events"),
        retry=_parse_retry_config(cfg),
    )


# Built-in subscriber type registry
_BUILTIN_FACTORIES: dict[str, Callable[[dict[str, Any]], EventSubscriber]] = {
    "webhook": _default_webhook_factory,
    "a2a": _default_a2a_factory,
    "file": _default_file_factory,
    "stdout": _default_stdout_factory,
    "filter": _default_filter_factory,
}

# Register built-in types
_subscriber_factories.update(_BUILTIN_FACTORIES)


def register_subscriber_type(
    type_name: str,
    factory: Callable[[dict[str, Any]], EventSubscriber],
) -> None:
    """Register a custom subscriber type factory."""
    _subscriber_factories[type_name] = factory


def unregister_subscriber_type(type_name: str) -> None:
    """Remove a previously registered subscriber type."""
    del _subscriber_factories[type_name]


def reset_subscriber_registry() -> None:
    """Reset the subscriber registry to built-in types only."""
    _subscriber_factories.clear()
    _subscriber_factories.update(_BUILTIN_FACTORIES)


# ---------------------------------------------------------------------------
# Public SubscriberFactory API — parity with apcore-typescript
# (``createSubscriberFromConfig`` / ``registerSubscriberType``) and apcore-rust
# (``create_subscriber`` / ``register_subscriber_type``). Issue #36.
# ---------------------------------------------------------------------------


def register_subscriber_factory(
    type_name: str,
    factory: Callable[[dict[str, Any]], EventSubscriber],
) -> None:
    """Register a custom subscriber-type factory.

    Public, cross-language counterpart to TypeScript's
    ``registerSubscriberType`` / Rust's ``register_subscriber_type`` — the
    normative name in event-system.md, which all three SDKs export.

    Args:
        type_name: The ``type`` string used in subscriber config dicts.
        factory: Callable that takes the config dict and returns an
            :class:`EventSubscriber` instance.
    """
    register_subscriber_type(type_name, factory)


def create_subscriber_from_config(config: dict[str, Any]) -> EventSubscriber:
    """Instantiate an :class:`EventSubscriber` from a config dict.

    Public, cross-language counterpart to TypeScript's
    ``createSubscriberFromConfig`` / Rust's ``create_subscriber``.

    Looks up the factory registered for ``config['type']`` and invokes it
    with the full config. Built-in types (``webhook``, ``a2a``, ``file``,
    ``stdout``, ``filter``) are auto-registered on import.

    Raises:
        ValueError: If ``config['type']`` is not in the factory registry.
    """
    return _create_subscriber(config)


# ---------------------------------------------------------------------------
# Namespace-mode helpers — §9.15.3
# ---------------------------------------------------------------------------

_MISSING = object()


def _nested_dict_get(data: dict[str, Any], dotted_key: str, default: Any) -> Any:
    """Get a value from a nested dict using a dot-separated key path."""
    current: Any = data
    for key in dotted_key.split("."):
        if not isinstance(current, dict):
            return default
        v = current.get(key, _MISSING)
        if v is _MISSING:
            return default
        current = v
    return current


def _resolve_sys_cfg(config: Config) -> dict[str, Any] | None:
    """Return the sys_modules namespace dict in namespace mode (§9.15.3)."""
    if getattr(config, "_mode", "legacy") == "namespace":
        return config.namespace("sys_modules") or {}
    return None


def _cfg_get(sys_cfg: dict[str, Any] | None, config: Config, sub_key: str, default: Any) -> Any:
    """Get a sys_modules value — namespace dict in namespace mode, dot-path in legacy."""
    if sys_cfg is not None:
        return _nested_dict_get(sys_cfg, sub_key, default)
    return config.get(f"sys_modules.{sub_key}", default)


def _apply_overrides(
    config: Config,
    overrides: dict[str, Any],
    toggle_state: Any | None = None,
    source: str = "<store>",
) -> None:
    """Apply an overrides mapping to ``config`` (and optionally ``toggle_state``).

    Keys prefixed with ``toggle.`` are applied to the toggle_state (if provided)
    rather than the config. All other keys are set as config dot-paths.
    Keys starting with ``_`` are reserved/ignored.
    """
    config_count = 0
    toggle_count = 0
    for key, value in overrides.items():
        if not isinstance(key, str):
            continue
        if key.startswith("toggle.") and toggle_state is not None:
            module_id = key[len("toggle.") :]
            if isinstance(value, bool):
                if value:
                    toggle_state.enable(module_id)
                else:
                    toggle_state.disable(module_id)
                toggle_count += 1
        elif not key.startswith("_"):
            try:
                config.set(key, value)
                config_count += 1
            except Exception as exc:
                logger.warning(
                    "Skipping override %s=%r from %s: %s",
                    key,
                    value,
                    source,
                    exc,
                )
    if config_count or toggle_count:
        logger.info(
            "Applied %d config overrides and %d toggle overrides from %s",
            config_count,
            toggle_count,
            source,
        )


def _load_overrides_from_store(
    config: Config,
    store: OverridesStore,
    toggle_state: Any | None = None,
) -> None:
    """Load overrides from a pluggable :class:`OverridesStore` and apply them."""
    try:
        overrides: Any = store.load()
    except Exception as exc:
        logger.error("Failed to load overrides from store %r: %s", store, exc)
        return
    if not isinstance(overrides, dict):
        logger.warning("Overrides store %r did not return a mapping; skipping", store)
        return
    _apply_overrides(config, overrides, toggle_state=toggle_state, source=repr(store))


def register_sys_modules(
    registry: Registry,
    executor: Executor,
    config: Config,
    metrics_collector: MetricsCollector | None = None,
    fail_on_error: bool = False,
    audit_store: AuditStore | None = None,
    overrides_store: OverridesStore | None = None,
    toggle_state: ToggleState | None = None,
) -> dict[str, Any]:
    """Auto-register all sys.* modules and middleware based on config.

    Args:
        registry: Module registry.
        executor: Executor for middleware registration.
        config: Configuration with sys_modules section.
        metrics_collector: Optional MetricsCollector instance.
        fail_on_error: When True, any registration failure raises
            SysModuleRegistrationError immediately. When False (default),
            failures are logged at ERROR level and execution continues.
        audit_store: Optional audit store for control module audit trails.
        toggle_state: Optional per-instance :class:`ToggleState` (#71). When
            provided, ``ToggleFeatureModule`` writes to this instance (the
            same one the owning ``APCore``'s Executor read path observes),
            isolating toggles between instances. When ``None``, the
            process-global ``_default_toggle_state`` is used for back-compat.
        overrides_store: Optional :class:`OverridesStore` for runtime
            config + toggle override persistence. When provided, overrides
            are loaded at startup and persisted after every successful
            ``update_config`` / ``toggle_feature`` call. Cross-language
            counterpart to TypeScript ``OverridesStore`` and Rust
            ``OverridesStore`` traits. When ``None``, the legacy
            ``sys_modules.control.overrides_path`` config key is honored
            via an auto-constructed :class:`FileOverridesStore` for
            backwards compatibility.

    Returns:
        Dict with references to created components for testing/inspection.
    """
    result: dict[str, Any] = {}

    # §9.15.3: prefer config.namespace("sys_modules") in namespace mode
    sys_cfg = _resolve_sys_cfg(config)

    if not _cfg_get(sys_cfg, config, "enabled", False):
        return result

    # Resolve overrides store: explicit kwarg wins; else fall back to the
    # legacy ``control.overrides_path`` config key by constructing a
    # FileOverridesStore. ``overrides_path`` is also retained for
    # downstream callers that still want the bare path (logging, audit).
    overrides_path: str | None = _cfg_get(sys_cfg, config, "control.overrides_path", None)
    effective_store: OverridesStore | None = overrides_store
    if effective_store is None and overrides_path:
        effective_store = FileOverridesStore(overrides_path)
    result["overrides_store"] = effective_store

    # Apply config-level overrides BEFORE module registration so that any
    # config-driven decisions made during registration see the override
    # values. ``toggle.*`` keys are deferred — they require the
    # ``ToggleState`` instance owned by ``toggle_feature``, which is only
    # created in ``_setup_events`` (and only when events are enabled).
    if effective_store is not None:
        _load_overrides_from_store(config, effective_store, toggle_state=None)

    error_history = _create_error_history(config, sys_cfg)
    result["error_history"] = error_history

    eh_middleware = ErrorHistoryMiddleware(error_history)
    executor.use(eh_middleware)
    result["error_history_middleware"] = eh_middleware

    # Usage tracking
    usage_collector = UsageCollector()
    result["usage_collector"] = usage_collector
    usage_middleware = UsageMiddleware(usage_collector)
    executor.use(usage_middleware)
    result["usage_middleware"] = usage_middleware

    _register_sys_modules(
        registry,
        config,
        sys_cfg,
        metrics_collector,
        error_history,
        usage_collector,
        fail_on_error=fail_on_error,
    )

    if _cfg_get(sys_cfg, config, "events.enabled", False):
        _setup_events(
            registry,
            executor,
            config,
            sys_cfg,
            metrics_collector,
            error_history,
            usage_collector,
            result,
            overrides_path=overrides_path,
            overrides_store=effective_store,
            audit_store=audit_store,
            fail_on_error=fail_on_error,
            toggle_state=toggle_state,
        )

    return result


def _create_error_history(config: Config, sys_cfg: dict[str, Any] | None) -> ErrorHistory:
    """Create an ErrorHistory instance from config values."""
    max_per_module = _cfg_get(sys_cfg, config, "error_history.max_entries_per_module", 50)
    max_total = _cfg_get(sys_cfg, config, "error_history.max_total_entries", 1000)
    return ErrorHistory(
        max_entries_per_module=max_per_module,
        max_total_entries=max_total,
    )


def _register_sys_module(
    registry: Registry,
    module_id: str,
    module: Any,
    fail_on_error: bool = False,
) -> None:
    """Register a sys module, bypassing reserved word checks.

    When fail_on_error is True, raises SysModuleRegistrationError on failure.
    When fail_on_error is False, logs at ERROR level and continues.
    """
    try:
        registry.register_internal(module_id, module)
    except Exception as exc:
        if fail_on_error:
            raise SysModuleRegistrationError(module_id=module_id, reason=str(exc)) from exc
        logger.error("System module '%s' failed to register: %s. Continuing.", module_id, exc)


def _register_sys_modules(
    registry: Registry,
    config: Config,
    sys_cfg: dict[str, Any] | None,
    metrics_collector: MetricsCollector | None,
    error_history: ErrorHistory,
    usage_collector: UsageCollector,
    fail_on_error: bool = False,
) -> None:
    """Register the sys.* module groups the configuration selects.

    ``sys_modules.enabled`` is the master switch and is checked by the caller.
    The three per-group flags below select *which* modules register once
    activation has happened — the split the v1.17.0 revision of §9.15.3 states
    and the one ``schemas/sys-modules.schema.json`` declares, key by key:

    - ``health.enabled``   — "Whether system.health.summary and system.health.module are registered"
    - ``manifest.enabled`` — "Whether system.manifest.module and system.manifest.full are registered"
    - ``usage.enabled``    — "Whether system.usage.summary and system.usage.module are registered"

    All three default to ``True``, so the flags narrow and never widen.

    They were read by no code path until now (apcore#118): every flag false
    registered the same six modules as every flag true, while the master switch
    worked — a section that is half alive, so a smoke test of it passes.
    """
    effective_metrics = metrics_collector if metrics_collector is not None else MetricsCollector()

    if _cfg_get(sys_cfg, config, "health.enabled", True):
        _register_sys_module(
            registry,
            "system.health.summary",
            HealthSummaryModule(
                registry=registry,
                metrics_collector=effective_metrics,
                error_history=error_history,
                config=config,
            ),
            fail_on_error=fail_on_error,
        )

        _register_sys_module(
            registry,
            "system.health.module",
            HealthModule(
                registry=registry,
                metrics_collector=effective_metrics,
                error_history=error_history,
            ),
            fail_on_error=fail_on_error,
        )

    if _cfg_get(sys_cfg, config, "manifest.enabled", True):
        _register_sys_module(
            registry,
            "system.manifest.module",
            ManifestModule(registry=registry, config=config),
            fail_on_error=fail_on_error,
        )

        _register_sys_module(
            registry,
            "system.manifest.full",
            ManifestFullModule(registry=registry, config=config),
            fail_on_error=fail_on_error,
        )

    if _cfg_get(sys_cfg, config, "usage.enabled", True):
        _register_sys_module(
            registry,
            "system.usage.summary",
            UsageSummaryModule(collector=usage_collector),
            fail_on_error=fail_on_error,
        )

        _register_sys_module(
            registry,
            "system.usage.module",
            UsageModule(registry=registry, usage_collector=usage_collector),
            fail_on_error=fail_on_error,
        )


def _setup_events(
    registry: Registry,
    executor: Executor,
    config: Config,
    sys_cfg: dict[str, Any] | None,
    metrics_collector: MetricsCollector | None,
    error_history: ErrorHistory,
    usage_collector: UsageCollector,
    result: dict[str, Any],
    overrides_path: str | None = None,
    overrides_store: OverridesStore | None = None,
    audit_store: AuditStore | None = None,
    fail_on_error: bool = False,
    toggle_state: ToggleState | None = None,
) -> None:
    """Set up event emitter, subscribers, PlatformNotifyMiddleware, control modules, and registry bridge."""
    event_emitter = EventEmitter()
    result["event_emitter"] = event_emitter

    error_rate_threshold = _cfg_get(sys_cfg, config, "events.thresholds.error_rate", 0.1)
    latency_p99_threshold = _cfg_get(sys_cfg, config, "events.thresholds.latency_p99_ms", 5000.0)
    pn_middleware = PlatformNotifyMiddleware(
        event_emitter=event_emitter,
        metrics_collector=metrics_collector,
        error_rate_threshold=error_rate_threshold,
        latency_p99_threshold_ms=latency_p99_threshold,
    )
    executor.use(pn_middleware)
    result["platform_notify_middleware"] = pn_middleware

    # `control.enabled` selects whether the Level 2 write plane registers once
    # `events.enabled` has activated it — schemas/sys-modules.schema.json:
    # "Whether the system.control.* modules are registered". Default True, so
    # the flag narrows and never widens. It was read by nothing until now
    # (apcore#118), which made it the most consequential of the four inert
    # sub-flags: an operator writing `control.enabled: false` to keep the
    # approval-gated write surface off a deployment got all three modules.
    #
    # When the flag is off, no ToggleState is created here: `toggle_feature`
    # owns it, and the caller's `toggle.*` overrides have nothing to apply to.
    if _cfg_get(sys_cfg, config, "control.enabled", True):
        effective_toggle_state = _register_control_modules(
            registry,
            config,
            event_emitter,
            overrides_path=overrides_path,
            overrides_store=overrides_store,
            audit_store=audit_store,
            fail_on_error=fail_on_error,
            toggle_state=toggle_state,
        )
    else:
        effective_toggle_state = toggle_state if toggle_state is not None else _default_toggle_state
    result["toggle_state"] = effective_toggle_state

    # Apply persisted overrides AFTER control modules are registered so the
    # toggle_feature module's ToggleState is available for ``toggle.*`` keys.
    if overrides_store is not None:
        _load_overrides_from_store(config, overrides_store, toggle_state=effective_toggle_state)

    _instantiate_subscribers(config, sys_cfg, event_emitter)
    registry.set_event_emitter(event_emitter)
    _bridge_registry_events(registry, event_emitter)


def _register_control_modules(
    registry: Registry,
    config: Config,
    event_emitter: EventEmitter,
    overrides_path: str | None = None,
    overrides_store: OverridesStore | None = None,
    audit_store: AuditStore | None = None,
    fail_on_error: bool = False,
    toggle_state: ToggleState | None = None,
) -> ToggleState:
    """Register control sys modules that require an EventEmitter.

    When *toggle_state* is provided (the per-instance path, #71), toggles
    written by ``ToggleFeatureModule`` go to that instance, which the owning
    ``APCore``'s pipeline ``BuiltinModuleLookup`` step also reads from — so
    toggles are isolated per instance and survive registry reloads of that
    instance. When *toggle_state* is ``None``, the process-global
    ``_default_toggle_state`` is used for back-compat (cross-language parity
    with apcore-typescript's ``DEFAULT_TOGGLE_STATE`` and apcore-rust's
    process-global ``ToggleState``, sync finding A-D-12).

    Returns the ``ToggleState`` instance owned by ``toggle_feature`` so the
    caller can apply persisted ``toggle.*`` overrides to it.
    """
    effective_toggle_state = toggle_state if toggle_state is not None else _default_toggle_state

    _register_sys_module(
        registry,
        "system.control.update_config",
        UpdateConfigModule(
            config=config,
            event_emitter=event_emitter,
            overrides_path=overrides_path,
            audit_store=audit_store,
            overrides_store=overrides_store,
        ),
        fail_on_error=fail_on_error,
    )

    _register_sys_module(
        registry,
        "system.control.reload_module",
        ReloadModule(
            registry=registry,
            event_emitter=event_emitter,
            audit_store=audit_store,
        ),
        fail_on_error=fail_on_error,
    )

    _register_sys_module(
        registry,
        "system.control.toggle_feature",
        ToggleFeatureModule(
            registry=registry,
            event_emitter=event_emitter,
            toggle_state=effective_toggle_state,
            overrides_path=overrides_path,
            audit_store=audit_store,
            overrides_store=overrides_store,
        ),
        fail_on_error=fail_on_error,
    )

    return effective_toggle_state


def _instantiate_subscribers(config: Config, sys_cfg: dict[str, Any] | None, event_emitter: EventEmitter) -> None:
    """Create and subscribe EventSubscribers from config, each wrapped in a circuit breaker."""
    subscribers_config = _cfg_get(sys_cfg, config, "events.subscribers", [])
    for sub_cfg in subscribers_config:
        try:
            subscriber = _create_subscriber(sub_cfg)
            cb_cfg: dict[str, Any] = sub_cfg.get("circuit_breaker") or {}
            wrapped = CircuitBreakerWrapper(
                subscriber=subscriber,
                emitter=event_emitter,
                timeout_ms=cb_cfg.get("timeout_ms", 5000),
                open_threshold=cb_cfg.get("open_threshold", 5),
                recovery_window_ms=cb_cfg.get("recovery_window_ms", 60000),
            )
            event_emitter.subscribe(wrapped)
        except Exception:
            logger.warning(
                "Failed to instantiate subscriber: %s",
                sub_cfg,
                exc_info=True,
            )


def _create_subscriber(sub_cfg: dict[str, Any]) -> EventSubscriber:
    """Factory for EventSubscriber from a config dict."""
    sub_type = sub_cfg.get("type", "")
    factory = _subscriber_factories.get(sub_type)
    if factory is None:
        registered = ", ".join(sorted(_subscriber_factories.keys()))
        raise ValueError(
            f"Unknown subscriber type: {sub_type!r}. "
            f"Registered types: [{registered}]. "
            f"Use register_subscriber_type() to add custom types."
        )
    return factory(sub_cfg)


def _bridge_registry_events(registry: Registry, emitter: EventEmitter) -> None:
    """Bridge registry register/unregister events to the EventEmitter.

    Emits the canonical ``apcore.registry.module_registered`` /
    ``apcore.registry.module_unregistered`` event types, and ONLY those.

    Correction: this docstring previously described a dual emit of the legacy
    bare names (``module_registered`` / ``module_unregistered``) carrying
    ``deprecated: true``, "scheduled for removal in v0.22.0 (Issue #36)". The
    removal happened and the code below has emitted the canonical name alone
    ever since; only the docstring still described the deprecation window — the
    description a maintainer would trust when deciding whether a legacy
    subscriber still receives anything. The inverse assertion is pinned by the
    ``legacy_names_are_not_emitted`` case of
    ``conformance/fixtures/event_naming.json`` (apcore#78).
    """

    def _now() -> str:
        return datetime.now(timezone.utc).isoformat()

    def on_register(module_id: str, module: Any) -> None:
        # Single-emit rule for ephemeral.* registrations: the registry-side
        # direct emit (Registry.set_event_emitter / _emit_ephemeral_audit)
        # already fires the canonical event with the full D-35 contextual
        # payload. Skipping the empty-payload bridge emit here avoids dual
        # emission for the same event_type. See protocol-spec §2.5.1
        # "Audit events".
        if module_id == "ephemeral" or module_id.startswith("ephemeral."):
            return
        ts = _now()
        emitter.emit(
            ApCoreEvent(
                event_type="apcore.registry.module_registered",
                module_id=module_id,
                timestamp=ts,
                severity="info",
                data={},
            )
        )

    def on_unregister(module_id: str, module: Any) -> None:
        # Mirror the single-emit rule for unregistrations. See protocol-spec
        # §2.5.1 "Audit events".
        if module_id == "ephemeral" or module_id.startswith("ephemeral."):
            return
        ts = _now()
        emitter.emit(
            ApCoreEvent(
                event_type="apcore.registry.module_unregistered",
                module_id=module_id,
                timestamp=ts,
                severity="info",
                data={},
            )
        )

    registry.on("register", on_register)
    registry.on("unregister", on_unregister)
