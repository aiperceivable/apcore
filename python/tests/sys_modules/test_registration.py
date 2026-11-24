"""Tests for auto-registration of sys.* modules and middleware from config."""

from __future__ import annotations

import logging
from typing import Any
from unittest.mock import MagicMock

import pytest

from apcore.config import Config
from apcore.events.emitter import EventEmitter
from apcore.executor import Executor
from apcore.middleware.error_history_middleware import ErrorHistoryMiddleware
from apcore.middleware.platform_notify import PlatformNotifyMiddleware
from apcore.observability.error_history import ErrorHistory
from apcore.observability.metrics import MetricsCollector
from apcore.registry.registry import Registry
from apcore.events.emitter import ApCoreEvent
from apcore.sys_modules.registration import (
    _subscriber_factories,
    register_subscriber_type,
    register_sys_modules,
    reset_subscriber_registry,
    unregister_subscriber_type,
)


def _make_config(overrides: dict[str, Any] | None = None) -> Config:
    """Create a Config with sys_modules defaults and optional overrides."""
    data: dict[str, Any] = {
        "version": "0.8.0",
        "extensions": {"root": "./extensions"},
        "schema": {"root": "./schemas"},
        "acl": {"root": "./acl", "default_effect": "deny"},
        "project": {"name": "test-project"},
        "sys_modules": {
            "enabled": True,
            "error_history": {
                "max_entries_per_module": 50,
                "max_total_entries": 1000,
            },
            "events": {
                "enabled": False,
                "thresholds": {
                    "error_rate": 0.1,
                    "latency_p99_ms": 5000.0,
                },
                "subscribers": [],
            },
        },
    }
    if overrides:
        _deep_set(data, overrides)
    return Config(data=data)


def _deep_set(target: dict, overrides: dict) -> None:
    """Recursively merge overrides into target."""
    for key, value in overrides.items():
        if isinstance(value, dict) and isinstance(target.get(key), dict):
            _deep_set(target[key], value)
        else:
            target[key] = value


def _make_deps() -> tuple[Registry, Executor, MetricsCollector]:
    """Create common dependencies for registration tests."""
    registry = Registry()
    metrics = MetricsCollector()
    executor = Executor(registry=registry)
    return registry, executor, metrics


class TestRegisterSysModulesDisabled:
    def test_register_sys_modules_disabled_noop(self) -> None:
        """Config with sys_modules.enabled=False returns empty dict, no modules registered."""
        config = _make_config({"sys_modules": {"enabled": False}})
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        assert result == {}
        assert not registry.has("system.health.summary")
        assert not registry.has("system.health.module")
        assert not registry.has("system.manifest.module")


class TestErrorHistory:
    def test_register_sys_modules_creates_error_history(self) -> None:
        """ErrorHistory created with config values for max_entries_per_module and max_total_entries."""
        config = _make_config(
            {
                "sys_modules": {
                    "error_history": {
                        "max_entries_per_module": 25,
                        "max_total_entries": 500,
                    },
                },
            }
        )
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        eh = result["error_history"]
        assert isinstance(eh, ErrorHistory)
        assert eh._max_entries_per_module == 25
        assert eh._max_total_entries == 500

    def test_register_sys_modules_registers_error_history_middleware(self) -> None:
        """ErrorHistoryMiddleware is added to executor."""
        config = _make_config()
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        eh_mw = result["error_history_middleware"]
        assert isinstance(eh_mw, ErrorHistoryMiddleware)
        # Verify it was added to executor's middleware manager
        assert eh_mw in executor._middleware_manager._middlewares


class TestSysModuleRegistration:
    def test_register_sys_modules_registers_health_summary(self) -> None:
        """system.health.summary is registered in registry."""
        config = _make_config()
        registry, executor, metrics = _make_deps()

        register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        assert registry.has("system.health.summary")

    def test_register_sys_modules_registers_health_module(self) -> None:
        """system.health.module is registered in registry."""
        config = _make_config()
        registry, executor, metrics = _make_deps()

        register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        assert registry.has("system.health.module")

    def test_register_sys_modules_registers_manifest_module(self) -> None:
        """system.manifest.module is registered in registry."""
        config = _make_config()
        registry, executor, metrics = _make_deps()

        register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        assert registry.has("system.manifest.module")


class TestEventsDisabled:
    def test_register_sys_modules_events_disabled_no_emitter(self) -> None:
        """Events disabled: no EventEmitter created, no PlatformNotifyMiddleware."""
        config = _make_config({"sys_modules": {"events": {"enabled": False}}})
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        assert "event_emitter" not in result
        assert "platform_notify_middleware" not in result


class TestEventsEnabled:
    def test_register_sys_modules_events_enabled_creates_emitter(self) -> None:
        """Events enabled: EventEmitter created."""
        config = _make_config({"sys_modules": {"events": {"enabled": True}}})
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        assert isinstance(result["event_emitter"], EventEmitter)

    def test_register_sys_modules_events_registers_platform_notify_middleware(
        self,
    ) -> None:
        """Events enabled: PlatformNotifyMiddleware added to executor with correct thresholds."""
        config = _make_config(
            {
                "sys_modules": {
                    "events": {
                        "enabled": True,
                        "thresholds": {
                            "error_rate": 0.2,
                            "latency_p99_ms": 3000.0,
                        },
                    },
                },
            }
        )
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        pn_mw = result["platform_notify_middleware"]
        assert isinstance(pn_mw, PlatformNotifyMiddleware)
        assert pn_mw._error_rate_threshold == 0.2
        assert pn_mw._latency_p99_threshold_ms == 3000.0
        assert pn_mw in executor._middleware_manager._middlewares

    def test_register_sys_modules_events_instantiates_subscribers(self) -> None:
        """Webhook subscriber created and subscribed to emitter from config."""
        config = _make_config(
            {
                "sys_modules": {
                    "events": {
                        "enabled": True,
                        "subscribers": [
                            {
                                "type": "webhook",
                                "url": "https://example.com/hook",
                                "headers": {"X-Custom": "value"},
                                "retry_count": 2,
                                "timeout_ms": 3000,
                            },
                        ],
                    },
                },
            }
        )
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        emitter = result["event_emitter"]
        assert len(emitter._subscribers) == 1
        from apcore.events.circuit_breaker import CircuitBreakerWrapper

        wrapper = emitter._subscribers[0]
        assert isinstance(wrapper, CircuitBreakerWrapper)
        from apcore.events.subscribers import WebhookSubscriber

        sub = wrapper._subscriber
        assert isinstance(sub, WebhookSubscriber)
        assert sub._url == "https://example.com/hook"
        assert sub._headers == {"X-Custom": "value"}
        # retry_count=2 in config → max_attempts=3 (retry_count was retries after first attempt)
        assert sub.retry.max_attempts == 3
        assert sub._timeout_ms == 3000

    def test_register_sys_modules_subscriber_failure_logged(self, caplog: pytest.LogCaptureFixture) -> None:
        """Invalid subscriber config logged but registration continues."""
        config = _make_config(
            {
                "sys_modules": {
                    "events": {
                        "enabled": True,
                        "subscribers": [
                            {"type": "unknown_type"},
                        ],
                    },
                },
            }
        )
        registry, executor, metrics = _make_deps()

        with caplog.at_level(logging.WARNING):
            result = register_sys_modules(
                registry=registry,
                executor=executor,
                config=config,
                metrics_collector=metrics,
            )

        # Should still have emitter and PlatformNotifyMiddleware
        assert "event_emitter" in result
        assert any("Failed to instantiate subscriber" in msg for msg in caplog.messages)

    def test_register_sys_modules_bridges_registry_events(self) -> None:
        """Events enabled: registry on('register') and on('unregister') bridge to EventEmitter."""
        config = _make_config({"sys_modules": {"events": {"enabled": True}}})
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        emitter = result["event_emitter"]
        # Subscribe a mock to capture emitted events
        events_received: list[Any] = []

        class MockSubscriber:
            async def on_event(self, event: Any) -> None:
                events_received.append(event)

        emitter.subscribe(MockSubscriber())

        # Register a new module -- should bridge to emitter
        dummy_module = MagicMock()
        dummy_module.version = None  # ensure default version is used
        dummy_module.on_load = None  # prevent on_load call
        del dummy_module.on_load
        registry.register("test.bridged", dummy_module)

        # Wait for background delivery to complete
        emitter.flush()

        # Canonical-only per the v0.22.0 spec MUST — no legacy alias (apcore#78).
        assert events_received[0].event_type == "apcore.registry.module_registered"
        assert events_received[0].module_id == "test.bridged"
        legacy = [e for e in events_received if e.event_type == "module_registered"]
        assert legacy == []


class TestReturnContext:
    def test_register_sys_modules_returns_context(self) -> None:
        """Function returns a dict with references to created components."""
        config = _make_config({"sys_modules": {"events": {"enabled": True}}})
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        assert "error_history" in result
        assert "error_history_middleware" in result
        assert "event_emitter" in result
        assert "platform_notify_middleware" in result


class TestAPCoreClientIntegration:
    def test_apcore_client_calls_register_sys_modules(self) -> None:
        """APCore with config having sys_modules.enabled=True registers sys modules."""
        from apcore.client import APCore

        config = _make_config()

        client = APCore(config=config)

        assert client.registry.has("system.health.summary")
        assert client.registry.has("system.health.module")
        assert client.registry.has("system.manifest.module")
        assert hasattr(client, "_sys_modules_context")
        assert isinstance(client._sys_modules_context, dict)


class TestA2ASubscriber:
    def test_register_sys_modules_a2a_subscriber(self) -> None:
        """A2A subscriber created from config."""
        config = _make_config(
            {
                "sys_modules": {
                    "events": {
                        "enabled": True,
                        "subscribers": [
                            {
                                "type": "a2a",
                                "platform_url": "https://platform.example.com/a2a",
                                "auth": "my-token",
                            },
                        ],
                    },
                },
            }
        )
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        emitter = result["event_emitter"]
        assert len(emitter._subscribers) == 1
        from apcore.events.circuit_breaker import CircuitBreakerWrapper

        wrapper = emitter._subscribers[0]
        assert isinstance(wrapper, CircuitBreakerWrapper)
        from apcore.events.subscribers import A2ASubscriber

        sub = wrapper._subscriber
        assert isinstance(sub, A2ASubscriber)
        assert sub._platform_url == "https://platform.example.com/a2a"
        assert sub._auth == "my-token"


class TestCustomSubscriberRegistry:
    """Tests for the extensible subscriber type registry."""

    @pytest.fixture(autouse=True)
    def _isolate_subscriber_registry(self) -> Any:
        """Reset subscriber registry to built-in defaults after each test."""
        yield
        reset_subscriber_registry()

    def test_register_custom_subscriber_type(self) -> None:
        """Register a custom subscriber type and use it from config."""

        class SlackSubscriber:
            def __init__(self, channel: str) -> None:
                self.channel = channel

            async def on_event(self, event: ApCoreEvent) -> None:
                pass

        register_subscriber_type("slack", lambda cfg: SlackSubscriber(channel=cfg["channel"]))
        config = _make_config(
            {
                "sys_modules": {
                    "events": {
                        "enabled": True,
                        "subscribers": [
                            {"type": "slack", "channel": "#alerts"},
                        ],
                    },
                },
            }
        )
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        emitter = result["event_emitter"]
        assert len(emitter._subscribers) == 1
        from apcore.events.circuit_breaker import CircuitBreakerWrapper

        wrapper = emitter._subscribers[0]
        assert isinstance(wrapper, CircuitBreakerWrapper)
        sub = wrapper._subscriber
        assert isinstance(sub, SlackSubscriber)
        assert sub.channel == "#alerts"

    def test_unregister_subscriber_type(self) -> None:
        """Unregister a subscriber type; verify it is no longer available."""
        register_subscriber_type("temp", lambda cfg: MagicMock())
        unregister_subscriber_type("temp")
        assert "temp" not in _subscriber_factories

    def test_unregister_nonexistent_type_raises(self) -> None:
        """Unregistering a non-existent type raises KeyError."""
        with pytest.raises(KeyError):
            unregister_subscriber_type("nonexistent_type_xyz")

    def test_override_builtin_subscriber_type(self) -> None:
        """Override the built-in 'webhook' type with a custom factory."""

        class CustomWebhook:
            def __init__(self, url: str) -> None:
                self.url = url
                self.custom = True

            async def on_event(self, event: ApCoreEvent) -> None:
                pass

        register_subscriber_type("webhook", lambda cfg: CustomWebhook(url=cfg["url"]))
        config = _make_config(
            {
                "sys_modules": {
                    "events": {
                        "enabled": True,
                        "subscribers": [
                            {"type": "webhook", "url": "https://example.com/hook"},
                        ],
                    },
                },
            }
        )
        registry, executor, metrics = _make_deps()

        result = register_sys_modules(
            registry=registry,
            executor=executor,
            config=config,
            metrics_collector=metrics,
        )

        from apcore.events.circuit_breaker import CircuitBreakerWrapper

        wrapper = result["event_emitter"]._subscribers[0]
        assert isinstance(wrapper, CircuitBreakerWrapper)
        sub = wrapper._subscriber
        assert isinstance(sub, CustomWebhook)
        assert sub.custom is True

    def test_unknown_type_error_lists_registered_types(self, caplog: pytest.LogCaptureFixture) -> None:
        """Unknown subscriber type error message lists available registered types."""
        config = _make_config(
            {
                "sys_modules": {
                    "events": {
                        "enabled": True,
                        "subscribers": [
                            {"type": "nonexistent"},
                        ],
                    },
                },
            }
        )
        registry, executor, metrics = _make_deps()

        with caplog.at_level(logging.WARNING):
            register_sys_modules(
                registry=registry,
                executor=executor,
                config=config,
                metrics_collector=metrics,
            )

        assert any("Failed to instantiate subscriber" in msg for msg in caplog.messages)

    def test_builtin_types_registered_by_default(self) -> None:
        """Verify 'webhook' and 'a2a' are registered by default."""
        assert "webhook" in _subscriber_factories
        assert "a2a" in _subscriber_factories

    def test_reset_subscriber_registry(self) -> None:
        """reset_subscriber_registry restores only built-in types."""
        register_subscriber_type("custom", lambda cfg: MagicMock())
        assert "custom" in _subscriber_factories

        reset_subscriber_registry()

        assert "custom" not in _subscriber_factories
        assert "webhook" in _subscriber_factories
        assert "a2a" in _subscriber_factories


class TestMetricsCollectorNone:
    def test_register_sys_modules_with_metrics_collector_none(self) -> None:
        """register_sys_modules works when metrics_collector is None."""
        config = _make_config()
        registry = Registry()
        executor = Executor(registry=registry)

        result = register_sys_modules(registry=registry, executor=executor, config=config, metrics_collector=None)

        assert registry.has("system.health.summary")
        assert registry.has("system.health.module")
        assert registry.has("system.manifest.module")
        assert "error_history" in result
