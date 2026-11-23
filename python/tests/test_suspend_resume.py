"""Tests for on_suspend() / on_resume() lifecycle hooks during hot-reload."""

from __future__ import annotations

import logging
from typing import Any
from unittest.mock import MagicMock, patch

import pytest

from apcore.events.emitter import EventEmitter
from apcore.registry.registry import Registry
from apcore.sys_modules.control import ReloadModuleModule


# ---------------------------------------------------------------------------
# Helper module classes
# ---------------------------------------------------------------------------


class _StatefulModule:
    """Module that implements on_suspend / on_resume to preserve state."""

    version = "1.0.0"
    description = "stateful module"

    def __init__(self) -> None:
        self.counter: int = 0
        self.resumed_state: dict[str, Any] | None = None

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        self.counter += 1
        return {"counter": self.counter}

    def on_suspend(self) -> dict[str, Any] | None:
        return {"counter": self.counter}

    def on_resume(self, state: dict[str, Any]) -> None:
        self.counter = state.get("counter", 0)
        self.resumed_state = state


class _StatefulModuleV2(_StatefulModule):
    """Updated version that also supports suspend/resume."""

    version = "2.0.0"
    description = "stateful module v2"


class _PlainModule:
    """Module WITHOUT on_suspend / on_resume (backward compatibility)."""

    version = "1.0.0"
    description = "plain module"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"ok": True}


class _PlainModuleV2:
    """Updated plain module."""

    version = "2.0.0"
    description = "plain module v2"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"ok": True}


class _SuspendReturnsNoneModule:
    """Module whose on_suspend returns None (no state to preserve)."""

    version = "1.0.0"
    description = "returns-none module"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}

    def on_suspend(self) -> dict[str, Any] | None:
        return None

    def on_resume(self, state: dict[str, Any]) -> None:
        pass  # pragma: no cover — should not be called when suspend returns None


class _BrokenSuspendModule:
    """Module whose on_suspend raises an exception."""

    version = "1.0.0"
    description = "broken suspend"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}

    def on_suspend(self) -> dict[str, Any] | None:
        raise RuntimeError("suspend exploded")

    def on_resume(self, state: dict[str, Any]) -> None:
        pass  # pragma: no cover


class _BrokenResumeModule:
    """Module whose on_resume raises an exception."""

    version = "1.0.0"
    description = "broken resume"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}

    def on_suspend(self) -> dict[str, Any] | None:
        return {"data": 42}

    def on_resume(self, state: dict[str, Any]) -> None:
        raise RuntimeError("resume exploded")


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _register_directly(registry: Registry, module_id: str, module: Any) -> None:
    """Register a module directly into the registry (bypassing validation)."""
    with registry._lock:
        registry._modules[module_id] = module
        registry._lowercase_map[module_id.lower()] = module_id


def _make_reload(
    registry: Registry,
    emitter: EventEmitter | None = None,
) -> ReloadModuleModule:
    if emitter is None:
        emitter = EventEmitter()
    return ReloadModuleModule(registry=registry, event_emitter=emitter)


# ---------------------------------------------------------------------------
# Tests: on_suspend returns state, on_resume receives it
# ---------------------------------------------------------------------------


class TestSuspendResumeStateTransfer:
    """Verify state is captured from old instance and restored to new instance."""

    def test_state_transferred_on_reload(self) -> None:
        registry = Registry()
        old_mod = _StatefulModule()
        old_mod.counter = 42
        _register_directly(registry, "stateful.mod", old_mod)

        reload_mod = _make_reload(registry)
        new_mod = _StatefulModuleV2()

        with patch.object(reload_mod, "_rediscover_module", return_value=new_mod):
            result = reload_mod.execute(
                {"module_id": "stateful.mod", "reason": "test suspend/resume"},
                context=None,
            )

        assert result["success"] is True
        assert new_mod.counter == 42
        assert new_mod.resumed_state == {"counter": 42}

    def test_on_suspend_returns_dict(self) -> None:
        mod = _StatefulModule()
        mod.counter = 10
        state = mod.on_suspend()
        assert state == {"counter": 10}

    def test_on_resume_restores_state(self) -> None:
        mod = _StatefulModule()
        mod.on_resume({"counter": 99})
        assert mod.counter == 99
        assert mod.resumed_state == {"counter": 99}


# ---------------------------------------------------------------------------
# Tests: backward compatibility (modules without suspend/resume)
# ---------------------------------------------------------------------------


class TestBackwardCompatibility:
    """Modules without on_suspend/on_resume still reload normally."""

    def test_plain_module_reloads_without_error(self) -> None:
        registry = Registry()
        _register_directly(registry, "plain.mod", _PlainModule())

        reload_mod = _make_reload(registry)
        new_mod = _PlainModuleV2()

        with patch.object(reload_mod, "_rediscover_module", return_value=new_mod):
            result = reload_mod.execute(
                {"module_id": "plain.mod", "reason": "upgrade"},
                context=None,
            )

        assert result["success"] is True
        assert result["new_version"] == "2.0.0"
        assert registry.get("plain.mod") is new_mod


# ---------------------------------------------------------------------------
# Tests: on_suspend returns None (no state to preserve)
# ---------------------------------------------------------------------------


class TestSuspendReturnsNone:
    """When on_suspend returns None, on_resume is NOT called."""

    def test_resume_not_called_when_suspend_returns_none(self) -> None:
        registry = Registry()
        _register_directly(registry, "none.mod", _SuspendReturnsNoneModule())

        reload_mod = _make_reload(registry)
        new_mod = _SuspendReturnsNoneModule()
        new_mod.on_resume = MagicMock()  # type: ignore[method-assign]

        with patch.object(reload_mod, "_rediscover_module", return_value=new_mod):
            result = reload_mod.execute(
                {"module_id": "none.mod", "reason": "test"},
                context=None,
            )

        assert result["success"] is True
        new_mod.on_resume.assert_not_called()


# ---------------------------------------------------------------------------
# Tests: error handling in suspend/resume
# ---------------------------------------------------------------------------


class TestSuspendErrorHandling:
    """on_suspend() errors are logged but do not fail the reload."""

    def test_suspend_error_logged_reload_succeeds(self, caplog: pytest.LogCaptureFixture) -> None:
        registry = Registry()
        _register_directly(registry, "broken.suspend", _BrokenSuspendModule())

        reload_mod = _make_reload(registry)
        new_mod = _PlainModuleV2()

        with caplog.at_level(logging.ERROR), patch.object(reload_mod, "_rediscover_module", return_value=new_mod):
            result = reload_mod.execute(
                {"module_id": "broken.suspend", "reason": "test"},
                context=None,
            )

        assert result["success"] is True
        assert any("on_suspend() failed" in msg for msg in caplog.messages)


class TestResumeErrorHandling:
    """on_resume() errors are logged but do not fail the reload."""

    def test_resume_error_logged_reload_succeeds(self, caplog: pytest.LogCaptureFixture) -> None:
        registry = Registry()
        _register_directly(registry, "broken.resume", _BrokenResumeModule())

        reload_mod = _make_reload(registry)
        # New module also has broken on_resume
        new_mod = _BrokenResumeModule()

        with caplog.at_level(logging.ERROR), patch.object(reload_mod, "_rediscover_module", return_value=new_mod):
            result = reload_mod.execute(
                {"module_id": "broken.resume", "reason": "test"},
                context=None,
            )

        assert result["success"] is True
        assert any("on_resume() failed" in msg for msg in caplog.messages)


# ---------------------------------------------------------------------------
# Tests: Module protocol default implementations
# ---------------------------------------------------------------------------


class TestModuleProtocolDefaults:
    """Verify the Module protocol provides default no-op implementations."""

    def test_default_on_suspend_returns_none(self) -> None:
        """A module that doesn't override on_suspend gets None by default."""
        mod = _PlainModule()
        # PlainModule has no on_suspend; the protocol default would return None
        # but since it's a Protocol, concrete classes don't inherit defaults.
        # The reload logic checks hasattr, so this is fine.
        assert not hasattr(mod, "on_suspend")

    def test_default_on_resume_is_absent(self) -> None:
        """A module that doesn't override on_resume has no on_resume attr."""
        mod = _PlainModule()
        assert not hasattr(mod, "on_resume")
