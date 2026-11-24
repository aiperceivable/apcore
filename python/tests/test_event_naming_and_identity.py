"""Tests for Issues #36 (event-name canonicalization, public SubscriberFactory)
and #45.2 (contextual auditing — auto-extract caller_id) in apcore-python.

The four legacy event names in scope:

    module_registered            → apcore.registry.module_registered
    module_unregistered          → apcore.registry.module_unregistered
    error_threshold_exceeded     → apcore.health.error_threshold_exceeded
    latency_threshold_exceeded   → apcore.health.latency_threshold_exceeded

For backwards compatibility, both the canonical and legacy event names are
emitted during the deprecation window. The legacy emission carries
``deprecated: true`` in its ``data`` payload so subscribers can spot it.

Audit identity: control modules (update_config / toggle_feature /
reload_module) extract ``caller_id`` (and optionally ``identity``) from the
execution context and embed them in the emitted audit event. When neither
is present, ``caller_id`` defaults to the spec sentinel ``@external``.
"""

from __future__ import annotations

from typing import Any
from unittest.mock import MagicMock

import pytest

from apcore.config import Config
from apcore.context import Context, Identity
from apcore.events.emitter import ApCoreEvent, EventEmitter, EventSubscriber
from apcore.events.subscribers import FilterSubscriber, StdoutSubscriber
from apcore.middleware.platform_notify import PlatformNotifyMiddleware
from apcore.observability.metrics import MetricsCollector
from apcore.sys_modules.control import (
    ReloadModule,
    ToggleFeatureModule,
    ToggleState,
    UpdateConfigModule,
)


# ---------------------------------------------------------------------------
# Helpers shared with test_platform_notify_middleware.py
# ---------------------------------------------------------------------------


def _metrics_with_error_rate(module_id: str, total_calls: int, error_calls: int) -> MetricsCollector:
    mc = MetricsCollector()
    labels_ok = {"module_id": module_id, "status": "success"}
    labels_err = {"module_id": module_id, "status": "error"}
    error_labels = {"module_id": module_id, "error_code": "E"}
    success_count = total_calls - error_calls
    for _ in range(success_count):
        mc.increment("apcore_module_calls_total", labels_ok)
    for _ in range(error_calls):
        mc.increment("apcore_module_calls_total", labels_err)
        mc.increment("apcore_module_errors_total", error_labels)
    return mc


def _add_latency(mc: MetricsCollector, module_id: str, values: list[float]) -> None:
    for v in values:
        mc.observe("apcore_module_duration_seconds", {"module_id": module_id}, v)


# ---------------------------------------------------------------------------
# Part A — Event naming standardization (Issue #36)
# ---------------------------------------------------------------------------


class TestRegistryEventCanonicalization:
    def test_module_registered_emits_canonical_only(self) -> None:
        # v0.22.0 spec MUST: canonical-only, no legacy dual-emission (apcore#78).
        from apcore.registry.registry import Registry
        from apcore.sys_modules.registration import _bridge_registry_events

        emitter = MagicMock(spec=EventEmitter)
        registry = Registry()
        _bridge_registry_events(registry, emitter)

        registry.register_internal("test.demo.module", MagicMock())

        emitted_types = [c[0][0].event_type for c in emitter.emit.call_args_list]
        assert "apcore.registry.module_registered" in emitted_types
        assert "module_registered" not in emitted_types

    def test_module_unregistered_emits_canonical_only(self) -> None:
        # v0.22.0 spec MUST: canonical-only, no legacy dual-emission (apcore#78).
        from apcore.registry.registry import Registry
        from apcore.sys_modules.registration import _bridge_registry_events

        emitter = MagicMock(spec=EventEmitter)
        registry = Registry()
        registry.register_internal("test.demo.module", MagicMock())
        _bridge_registry_events(registry, emitter)
        emitter.reset_mock()

        registry.unregister("test.demo.module")

        emitted_types = [c[0][0].event_type for c in emitter.emit.call_args_list]
        assert "apcore.registry.module_unregistered" in emitted_types
        assert "module_unregistered" not in emitted_types

    def test_glob_apcore_registry_matches_register_and_unregister(self) -> None:
        """A FilterSubscriber subscribed to apcore.registry.* receives BOTH events."""
        received: list[ApCoreEvent] = []

        class Capture:
            async def on_event(self, event: ApCoreEvent) -> None:
                received.append(event)

        delegate = Capture()
        sub = FilterSubscriber(delegate=delegate, include_events=["apcore.registry.*"])

        from datetime import datetime, timezone

        events = [
            ApCoreEvent(
                event_type="apcore.registry.module_registered",
                module_id="m.a",
                timestamp=datetime.now(timezone.utc).isoformat(),
                severity="info",
                data={},
            ),
            ApCoreEvent(
                event_type="apcore.registry.module_unregistered",
                module_id="m.a",
                timestamp=datetime.now(timezone.utc).isoformat(),
                severity="info",
                data={},
            ),
            ApCoreEvent(
                event_type="apcore.config.updated",
                module_id="x",
                timestamp=datetime.now(timezone.utc).isoformat(),
                severity="info",
                data={},
            ),
        ]

        import asyncio

        async def run() -> None:
            for e in events:
                await sub.on_event(e)

        asyncio.run(run())

        types = [e.event_type for e in received]
        assert "apcore.registry.module_registered" in types
        assert "apcore.registry.module_unregistered" in types
        assert "apcore.config.updated" not in types


class TestHealthEventCanonicalization:
    def test_error_threshold_emits_canonical_only(self) -> None:
        # v0.22.0 spec MUST: canonical-only, no legacy dual-emission (apcore#78).
        emitter = MagicMock(spec=EventEmitter)
        mc = _metrics_with_error_rate("mod.a", total_calls=100, error_calls=15)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            error_rate_threshold=0.1,
        )

        mw.on_error("mod.a", {}, RuntimeError("boom"), MagicMock())

        emitted_types = [c[0][0].event_type for c in emitter.emit.call_args_list]
        assert "apcore.health.error_threshold_exceeded" in emitted_types
        assert "error_threshold_exceeded" not in emitted_types

    def test_latency_threshold_emits_canonical_only(self) -> None:
        # v0.22.0 spec MUST: canonical-only, no legacy dual-emission (apcore#78).
        emitter = MagicMock(spec=EventEmitter)
        mc = MetricsCollector()
        _add_latency(mc, "mod.b", [0.1] * 95 + [6.0] * 5)
        mw = PlatformNotifyMiddleware(
            event_emitter=emitter,
            metrics_collector=mc,
            latency_p99_threshold_ms=5000.0,
        )

        mw.after("mod.b", {}, {}, MagicMock())

        emitted_types = [c[0][0].event_type for c in emitter.emit.call_args_list]
        assert "apcore.health.latency_threshold_exceeded" in emitted_types
        assert "latency_threshold_exceeded" not in emitted_types


# ---------------------------------------------------------------------------
# Part B — Public SubscriberFactory API (Issue #36)
# ---------------------------------------------------------------------------


class TestPublicSubscriberFactoryAPI:
    def test_register_subscriber_factory_and_create_from_config(self) -> None:
        from apcore.events import (
            create_subscriber_from_config,
            register_subscriber_factory,
        )

        class CustomSub:
            def __init__(self, value: str) -> None:
                self.value = value

            async def on_event(self, event: ApCoreEvent) -> None:
                pass

        def factory(cfg: dict) -> EventSubscriber:
            return CustomSub(value=cfg["value"])

        register_subscriber_factory("__test_custom__", factory)
        try:
            sub = create_subscriber_from_config({"type": "__test_custom__", "value": "hello"})
            assert isinstance(sub, CustomSub)
            assert sub.value == "hello"
        finally:
            # Clean up the custom factory so other tests are not affected
            from apcore.sys_modules.registration import unregister_subscriber_type

            unregister_subscriber_type("__test_custom__")

    def test_create_subscriber_from_config_supports_builtin_types(self) -> None:
        from apcore.events import create_subscriber_from_config

        sub = create_subscriber_from_config({"type": "stdout", "format": "text"})
        assert isinstance(sub, StdoutSubscriber)

    def test_create_subscriber_from_config_unknown_type_raises(self) -> None:
        from apcore.events import create_subscriber_from_config

        with pytest.raises(ValueError) as excinfo:
            create_subscriber_from_config({"type": "this-type-does-not-exist"})
        assert "Unknown subscriber type" in str(excinfo.value)

    def test_public_factory_api_exported_from_apcore_events(self) -> None:
        import apcore.events as events_pkg

        assert hasattr(events_pkg, "register_subscriber_factory")
        assert hasattr(events_pkg, "create_subscriber_from_config")


# ---------------------------------------------------------------------------
# Part C — Contextual auditing (Issue #45.2)
# ---------------------------------------------------------------------------


def _make_context(caller_id: str | None, identity: Identity | None = None) -> Context[Any]:
    return Context(
        trace_id="trace-test",
        caller_id=caller_id,
        identity=identity,
    )


class TestUpdateConfigCallerId:
    def test_update_config_event_includes_caller_id_from_context(self) -> None:
        config = Config.from_defaults()
        emitter = EventEmitter()
        emitter.emit = MagicMock()  # type: ignore[method-assign]
        mod = UpdateConfigModule(config=config, event_emitter=emitter)

        ctx = _make_context(caller_id="user.alice")
        mod.execute(
            {"key": "executor.default_timeout", "value": 5000, "reason": "test"},
            ctx,
        )

        canonical = [c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.config.updated"]
        assert len(canonical) == 1
        event: ApCoreEvent = canonical[0][0][0]
        assert event.data.get("caller_id") == "user.alice"

    def test_update_config_event_defaults_to_external(self) -> None:
        config = Config.from_defaults()
        emitter = EventEmitter()
        emitter.emit = MagicMock()  # type: ignore[method-assign]
        mod = UpdateConfigModule(config=config, event_emitter=emitter)

        # context is None — no caller_id available
        mod.execute(
            {"key": "executor.default_timeout", "value": 5000, "reason": "test"},
            None,
        )

        canonical = [c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.config.updated"]
        assert len(canonical) == 1
        assert canonical[0][0][0].data.get("caller_id") == "@external"

    def test_update_config_event_includes_identity_when_present(self) -> None:
        config = Config.from_defaults()
        emitter = EventEmitter()
        emitter.emit = MagicMock()  # type: ignore[method-assign]
        mod = UpdateConfigModule(config=config, event_emitter=emitter)

        identity = Identity(id="alice@example.com", type="user", roles=("admin",))
        ctx = _make_context(caller_id="user.alice", identity=identity)
        mod.execute(
            {"key": "executor.default_timeout", "value": 5000, "reason": "test"},
            ctx,
        )

        canonical = [c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.config.updated"]
        assert len(canonical) == 1
        ident_payload = canonical[0][0][0].data.get("identity")
        assert isinstance(ident_payload, dict)
        assert ident_payload.get("id") == "alice@example.com"
        assert ident_payload.get("type") == "user"


class TestToggleFeatureCallerId:
    def test_toggle_event_includes_caller_id(self) -> None:
        from apcore.registry.registry import Registry

        registry = Registry()
        registry.register_internal("test.dummy", MagicMock())

        emitter = EventEmitter()
        emitter.emit = MagicMock()  # type: ignore[method-assign]

        mod = ToggleFeatureModule(
            registry=registry,
            event_emitter=emitter,
            toggle_state=ToggleState(),
        )

        ctx = _make_context(caller_id="user.bob")
        mod.execute(
            {"module_id": "test.dummy", "enabled": False, "reason": "audit test"},
            ctx,
        )

        canonical = [c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.module.toggled"]
        assert len(canonical) == 1
        assert canonical[0][0][0].data.get("caller_id") == "user.bob"

    def test_toggle_event_defaults_to_external(self) -> None:
        from apcore.registry.registry import Registry

        registry = Registry()
        registry.register_internal("test.dummy2", MagicMock())

        emitter = EventEmitter()
        emitter.emit = MagicMock()  # type: ignore[method-assign]

        mod = ToggleFeatureModule(
            registry=registry,
            event_emitter=emitter,
            toggle_state=ToggleState(),
        )

        mod.execute(
            {"module_id": "test.dummy2", "enabled": False, "reason": "audit test"},
            None,
        )

        canonical = [c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.module.toggled"]
        assert len(canonical) == 1
        assert canonical[0][0][0].data.get("caller_id") == "@external"


class TestReloadModuleCallerId:
    def test_reload_event_includes_caller_id(self) -> None:
        """ReloadModule emits apcore.module.reloaded with caller_id in payload."""
        from apcore.registry.registry import Registry

        registry = Registry()

        # Stand up a dummy module with a version attribute
        dummy = MagicMock()
        dummy.version = "1.0.0"
        del dummy.on_load
        del dummy.on_suspend
        del dummy.on_resume
        registry.register_internal("test.reloadable", dummy)

        emitter = EventEmitter()
        emitter.emit = MagicMock()  # type: ignore[method-assign]

        mod = ReloadModule(registry=registry, event_emitter=emitter)

        # Stub _rediscover_module to return a fresh module without touching disk
        new_module = MagicMock()
        new_module.version = "1.1.0"
        del new_module.on_resume
        del new_module.on_load
        mod._rediscover_module = lambda module_id: new_module  # type: ignore[assignment]

        ctx = _make_context(caller_id="user.carol")
        mod.execute(
            {"module_id": "test.reloadable", "reason": "audit test"},
            ctx,
        )

        canonical = [c for c in emitter.emit.call_args_list if c[0][0].event_type == "apcore.module.reloaded"]
        assert len(canonical) == 1
        assert canonical[0][0][0].data.get("caller_id") == "user.carol"
