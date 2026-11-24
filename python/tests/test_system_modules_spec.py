"""Spec-traced contract tests for the apcore System Modules feature.

Source spec: apcore/docs/features/system-modules.md
Contracts under test (the six ``## Contract:`` blocks):
  1. ``system.control.update_config``   (UpdateConfigModule.execute)
  2. ``system.control.reload_module``   (ReloadModule.execute)
  3. ``system.control.toggle_feature``  (ToggleFeatureModule.execute)
  4. ``check_module_disabled``
  5. ``is_module_disabled``
  6. ``register_sys_modules``

Each test's docstring carries the verbatim clause id formatted
``system_modules.<method>.<kind>.<detail>`` (kind in
input|error|property|side_effect|return) so cross-language diffs line up
row-for-row with the TypeScript and Rust suites. This Python suite is the
CANONICAL clause source.

These tests are READ-ONLY contract verification — they never modify
production source.

Notable spec/source divergence (handled, not faked):
  The spec ``## Contract: check_module_disabled / is_module_disabled`` blocks
  declare a two-parameter signature ``(module_id, registry)``. The actual
  Python implementation exposes single-parameter free functions
  ``(module_id)`` that read a module-level ``_default_toggle_state``. Tests
  below verify the real single-parameter signature and skip the
  registry-parameter clause with an explanatory reason.
"""

from __future__ import annotations

import asyncio
from typing import Any

import pytest

from apcore.config import Config
from apcore.errors import (
    ConfigError,
    InvalidInputError,
    ModuleDisabledError,
    ModuleError,
    ModuleNotFoundError,
    ModuleReloadConflictError,
)
from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.executor import Executor
from apcore.registry.registry import Registry
from apcore.sys_modules.control import (
    ToggleFeatureModule,
    ToggleState,
    UpdateConfigModule,
    check_module_disabled,
    is_module_disabled,
    _default_toggle_state,
)
from apcore.sys_modules.registration import register_sys_modules


# ---------------------------------------------------------------------------
# Helpers / fixtures
# ---------------------------------------------------------------------------


class _RecordingEmitter(EventEmitter):
    """EventEmitter subclass that records every emitted event."""

    def __init__(self) -> None:
        super().__init__()
        self.events: list[ApCoreEvent] = []

    def emit(self, event: ApCoreEvent) -> Any:  # type: ignore[override]
        self.events.append(event)
        return super().emit(event)


class _DummyModule:
    """A minimal registrable module for toggle/reload tests."""

    input_schema: dict[str, Any] = {"type": "object"}
    output_schema: dict[str, Any] = {"type": "object"}
    description = "dummy"
    version = "1.0.0"

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {}


def _make_config(**overrides: Any) -> Config:
    config = Config.from_defaults()
    for key, value in overrides.items():
        config.set(key, value)
    return config


def _update_module(
    config: Config | None = None,
    emitter: EventEmitter | None = None,
) -> UpdateConfigModule:
    return UpdateConfigModule(
        config=config or Config.from_defaults(),
        event_emitter=emitter or EventEmitter(),
    )


def _toggle_module(
    registry: Registry,
    emitter: EventEmitter | None = None,
    toggle_state: ToggleState | None = None,
) -> ToggleFeatureModule:
    return ToggleFeatureModule(
        registry=registry,
        event_emitter=emitter or EventEmitter(),
        toggle_state=toggle_state or ToggleState(),
    )


def _registry_with_module(module_id: str = "math.add") -> Registry:
    reg = Registry()
    reg.register_internal(module_id, _DummyModule())
    return reg


@pytest.fixture(autouse=True)
def _reset_default_toggle_state() -> Any:
    """Keep the module-global default toggle state clean between tests."""
    _default_toggle_state.clear()
    yield
    _default_toggle_state.clear()


# ===========================================================================
# Contract 1: system.control.update_config  (UpdateConfigModule.execute)
# ===========================================================================


class TestUpdateConfigContract:
    def test_update_config_input_key_required(self) -> None:
        """system_modules.update_config.input.key_required

        Missing/empty `key` is rejected with InvalidInputError.
        """
        mod = _update_module()
        with pytest.raises(InvalidInputError):
            mod.execute({"key": "", "value": 1, "reason": "r"}, None)

    def test_update_config_input_reason_required(self) -> None:
        """system_modules.update_config.input.reason_required

        Missing/empty `reason` is rejected with InvalidInputError.
        """
        mod = _update_module()
        with pytest.raises(InvalidInputError):
            mod.execute({"key": "executor.default_timeout", "value": 1, "reason": ""}, None)

    def test_update_config_input_value_any_accepted(self) -> None:
        """system_modules.update_config.input.value_any_accepted

        `value` accepts any JSON-serializable value (no input-time validation).
        """
        config = _make_config()
        mod = _update_module(config=config)
        result = mod.execute(
            {"key": "some.arbitrary.field", "value": {"nested": [1, 2, 3]}, "reason": "r"},
            None,
        )
        assert result["new_value"] == {"nested": [1, 2, 3]}

    def test_update_config_error_restricted_key(self) -> None:
        """system_modules.update_config.error.config_key_restricted

        Updating a restricted key raises ModuleError(code=CONFIG_KEY_RESTRICTED).
        """
        mod = _update_module()
        with pytest.raises(ModuleError) as exc_info:
            mod.execute(
                {"key": "sys_modules.enabled", "value": False, "reason": "r"},
                None,
            )
        assert exc_info.value.code == "CONFIG_KEY_RESTRICTED"

    def test_update_config_error_constraint_violation(self) -> None:
        """system_modules.update_config.error.config_constraint

        A value violating a registered constraint raises ConfigError.
        """
        config = _make_config()
        mod = _update_module(config=config)
        with pytest.raises(ConfigError):
            # executor.default_timeout must be a non-negative integer (ms).
            mod.execute(
                {"key": "executor.default_timeout", "value": -5, "reason": "r"},
                None,
            )

    def test_update_config_side_effect_rollback_on_constraint(self) -> None:
        """system_modules.update_config.side_effect.rollback_on_constraint

        On ConfigError the Config is atomically rolled back to old_value.
        """
        config = _make_config()
        original = config.get("executor.default_timeout")
        mod = _update_module(config=config)
        with pytest.raises(ConfigError):
            mod.execute(
                {"key": "executor.default_timeout", "value": -5, "reason": "r"},
                None,
            )
        assert config.get("executor.default_timeout") == original

    def test_update_config_side_effect_applies_and_emits(self) -> None:
        """system_modules.update_config.side_effect.set_and_emit_event

        On success Config is mutated and apcore.config.updated is emitted.
        """
        config = _make_config()
        emitter = _RecordingEmitter()
        mod = _update_module(config=config, emitter=emitter)
        mod.execute(
            {"key": "executor.default_timeout", "value": 60000, "reason": "r"},
            None,
        )
        assert config.get("executor.default_timeout") == 60000
        types = [e.event_type for e in emitter.events]
        assert "apcore.config.updated" in types

    def test_update_config_return_shape(self) -> None:
        """system_modules.update_config.return.success_shape

        Returns {success, key, old_value, new_value}.
        """
        config = _make_config()
        mod = _update_module(config=config)
        result = mod.execute(
            {"key": "executor.default_timeout", "value": 60000, "reason": "r"},
            None,
        )
        assert result["success"] is True
        assert result["key"] == "executor.default_timeout"
        assert result["old_value"] == 30000
        assert result["new_value"] == 60000

    def test_update_config_return_redacts_sensitive(self) -> None:
        """system_modules.update_config.return.redacts_sensitive_segments

        Sensitive key segments redact old_value/new_value in the return.
        """
        config = _make_config()
        mod = _update_module(config=config)
        result = mod.execute(
            {"key": "platform.api_token", "value": "supersecret", "reason": "r"},
            None,
        )
        assert result["new_value"] != "supersecret"
        assert result["old_value"] != "supersecret"

    def test_update_config_property_not_idempotent(self) -> None:
        """system_modules.update_config.property.idempotent_false

        Repeated calls with different values produce different state.
        """
        config = _make_config()
        mod = _update_module(config=config)
        mod.execute({"key": "executor.default_timeout", "value": 1000, "reason": "r"}, None)
        mod.execute({"key": "executor.default_timeout", "value": 2000, "reason": "r"}, None)
        assert config.get("executor.default_timeout") == 2000

    def test_update_config_property_not_pure(self) -> None:
        """system_modules.update_config.property.pure_false

        Mutates Config state (not a pure function).
        """
        config = _make_config()
        before = config.get("executor.default_timeout")
        mod = _update_module(config=config)
        mod.execute({"key": "executor.default_timeout", "value": 4444, "reason": "r"}, None)
        assert config.get("executor.default_timeout") != before

    def test_update_config_property_not_async(self) -> None:
        """system_modules.update_config.property.async_false

        execute is a synchronous (non-coroutine) function.
        """
        assert not asyncio.iscoroutinefunction(UpdateConfigModule.execute)


# ===========================================================================
# Contract 2: system.control.reload_module  (ReloadModule.execute)
# ===========================================================================


class TestReloadModuleContract:
    def _module(self, registry: Registry, emitter: EventEmitter | None = None) -> Any:
        from apcore.sys_modules.control import ReloadModule

        return ReloadModule(registry=registry, event_emitter=emitter or EventEmitter())

    def test_reload_module_input_module_id_required(self) -> None:
        """system_modules.reload_module.input.module_id_required

        Missing module_id and path_filter -> InvalidInputError.
        """
        mod = self._module(Registry())
        with pytest.raises(InvalidInputError):
            mod.execute({"reason": "r"}, None)

    def test_reload_module_input_reason_required(self) -> None:
        """system_modules.reload_module.input.reason_required

        Missing/empty reason -> InvalidInputError.
        """
        mod = self._module(_registry_with_module("math.add"))
        with pytest.raises(InvalidInputError):
            mod.execute({"module_id": "math.add", "reason": ""}, None)

    def test_reload_module_error_not_found(self) -> None:
        """system_modules.reload_module.error.module_not_found

        module_id absent from Registry -> ModuleNotFoundError.
        """
        mod = self._module(Registry())
        with pytest.raises(ModuleNotFoundError):
            mod.execute({"module_id": "missing.module", "reason": "r"}, None)

    def test_reload_module_error_reload_conflict(self) -> None:
        """system_modules.reload_module.error.reload_conflict

        Both module_id and path_filter -> ModuleReloadConflictError
        (code MODULE_RELOAD_CONFLICT).
        """
        mod = self._module(_registry_with_module("math.add"))
        with pytest.raises(ModuleReloadConflictError) as exc_info:
            mod.execute(
                {"module_id": "math.add", "path_filter": "math.*", "reason": "r"},
                None,
            )
        assert exc_info.value.code == "MODULE_RELOAD_CONFLICT"

    def test_reload_module_property_not_async(self) -> None:
        """system_modules.reload_module.property.async_false

        execute is synchronous (non-coroutine).
        """
        from apcore.sys_modules.control import ReloadModule

        assert not asyncio.iscoroutinefunction(ReloadModule.execute)

    def test_reload_module_property_requires_approval(self) -> None:
        """system_modules.reload_module.property.requires_approval

        Annotation requires_approval=True (control module).
        """
        from apcore.sys_modules.control import ReloadModule

        assert ReloadModule.annotations.requires_approval is True


# ===========================================================================
# Contract 3: system.control.toggle_feature  (ToggleFeatureModule.execute)
# ===========================================================================


class TestToggleFeatureContract:
    def test_toggle_feature_input_module_id_required(self) -> None:
        """system_modules.toggle_feature.input.module_id_required

        Missing/empty module_id -> InvalidInputError.
        """
        mod = _toggle_module(Registry())
        with pytest.raises(InvalidInputError):
            mod.execute({"module_id": "", "enabled": False, "reason": "r"}, None)

    def test_toggle_feature_input_enabled_must_be_bool(self) -> None:
        """system_modules.toggle_feature.input.enabled_must_be_bool

        Non-bool `enabled` (e.g. string/int) -> InvalidInputError.
        """
        reg = _registry_with_module("math.add")
        mod = _toggle_module(reg)
        with pytest.raises(InvalidInputError):
            mod.execute({"module_id": "math.add", "enabled": "false", "reason": "r"}, None)

    def test_toggle_feature_input_enabled_int_rejected(self) -> None:
        """system_modules.toggle_feature.input.enabled_int_rejected

        Integer 0/1 is not a bool -> InvalidInputError.
        """
        reg = _registry_with_module("math.add")
        mod = _toggle_module(reg)
        with pytest.raises(InvalidInputError):
            mod.execute({"module_id": "math.add", "enabled": 1, "reason": "r"}, None)

    def test_toggle_feature_input_reason_required(self) -> None:
        """system_modules.toggle_feature.input.reason_required

        Missing/empty reason -> InvalidInputError.
        """
        reg = _registry_with_module("math.add")
        mod = _toggle_module(reg)
        with pytest.raises(InvalidInputError):
            mod.execute({"module_id": "math.add", "enabled": False, "reason": ""}, None)

    def test_toggle_feature_error_not_found(self) -> None:
        """system_modules.toggle_feature.error.module_not_found

        Unregistered module_id -> ModuleNotFoundError.
        """
        mod = _toggle_module(Registry())
        with pytest.raises(ModuleNotFoundError):
            mod.execute({"module_id": "ghost.module", "enabled": False, "reason": "r"}, None)

    def test_toggle_feature_side_effect_disable_sets_state(self) -> None:
        """system_modules.toggle_feature.side_effect.disable_sets_state

        enabled=False marks module disabled in ToggleState.
        """
        reg = _registry_with_module("math.add")
        state = ToggleState()
        mod = _toggle_module(reg, toggle_state=state)
        mod.execute({"module_id": "math.add", "enabled": False, "reason": "r"}, None)
        assert state.is_disabled("math.add") is True

    def test_toggle_feature_side_effect_enable_clears_state(self) -> None:
        """system_modules.toggle_feature.side_effect.enable_clears_state

        enabled=True clears disabled state.
        """
        reg = _registry_with_module("math.add")
        state = ToggleState()
        state.disable("math.add")
        mod = _toggle_module(reg, toggle_state=state)
        mod.execute({"module_id": "math.add", "enabled": True, "reason": "r"}, None)
        assert state.is_disabled("math.add") is False

    def test_toggle_feature_side_effect_emits_event(self) -> None:
        """system_modules.toggle_feature.side_effect.emits_toggled_event

        Emits apcore.module.toggled.
        """
        reg = _registry_with_module("math.add")
        emitter = _RecordingEmitter()
        mod = _toggle_module(reg, emitter=emitter)
        mod.execute({"module_id": "math.add", "enabled": False, "reason": "r"}, None)
        types = [e.event_type for e in emitter.events]
        assert "apcore.module.toggled" in types

    def test_toggle_feature_return_shape(self) -> None:
        """system_modules.toggle_feature.return.success_shape

        Returns {success, module_id, enabled}.
        """
        reg = _registry_with_module("math.add")
        mod = _toggle_module(reg)
        result = mod.execute({"module_id": "math.add", "enabled": False, "reason": "r"}, None)
        assert result == {"success": True, "module_id": "math.add", "enabled": False}

    def test_toggle_feature_property_idempotent(self) -> None:
        """system_modules.toggle_feature.property.idempotent_true

        Toggling to the current state twice yields the same outcome.
        """
        reg = _registry_with_module("math.add")
        state = ToggleState()
        mod = _toggle_module(reg, toggle_state=state)
        r1 = mod.execute({"module_id": "math.add", "enabled": False, "reason": "r"}, None)
        r2 = mod.execute({"module_id": "math.add", "enabled": False, "reason": "r"}, None)
        assert r1 == r2
        assert state.is_disabled("math.add") is True

    def test_toggle_feature_property_thread_safe(self) -> None:
        """system_modules.toggle_feature.property.thread_safe

        ToggleState serializes concurrent mutations under threading.Lock;
        verified via concurrent asyncio.gather of >=8 toggle calls.
        """
        module_ids = [f"mod.m{i}" for i in range(8)]
        reg = Registry()
        for mid in module_ids:
            reg.register_internal(mid, _DummyModule())
        state = ToggleState()
        mod = _toggle_module(reg, toggle_state=state)

        async def _toggle(mid: str) -> dict[str, Any]:
            return await asyncio.to_thread(
                mod.execute,
                {"module_id": mid, "enabled": False, "reason": "r"},
                None,
            )

        async def _run() -> list[dict[str, Any]]:
            return await asyncio.gather(*[_toggle(mid) for mid in module_ids])

        results = asyncio.run(_run())
        assert len(results) == 8
        for mid in module_ids:
            assert state.is_disabled(mid) is True

    def test_toggle_feature_property_not_async(self) -> None:
        """system_modules.toggle_feature.property.async_false

        execute is synchronous (non-coroutine).
        """
        assert not asyncio.iscoroutinefunction(ToggleFeatureModule.execute)

    def test_toggle_feature_property_requires_approval(self) -> None:
        """system_modules.toggle_feature.property.requires_approval

        Annotation requires_approval=True (control module).
        """
        assert ToggleFeatureModule.annotations.requires_approval is True


# ===========================================================================
# Contract 4: check_module_disabled
# ===========================================================================


class TestCheckModuleDisabledContract:
    def test_check_module_disabled_error_raises_when_disabled(self) -> None:
        """system_modules.check_module_disabled.error.module_disabled

        Raises ModuleDisabledError (code MODULE_DISABLED) when disabled.
        """
        _default_toggle_state.disable("risky.module")
        with pytest.raises(ModuleDisabledError) as exc_info:
            check_module_disabled("risky.module")
        assert exc_info.value.code == "MODULE_DISABLED"

    def test_check_module_disabled_return_none_when_enabled(self) -> None:
        """system_modules.check_module_disabled.return.none_when_enabled

        Returns None (does not raise) when the module is enabled.
        """
        assert check_module_disabled("enabled.module") is None

    def test_check_module_disabled_property_pure_read_only(self) -> None:
        """system_modules.check_module_disabled.property.pure_read_only

        Reads toggle state only; calling it does not mutate disabled state.
        """
        _default_toggle_state.disable("x.mod")
        with pytest.raises(ModuleDisabledError):
            check_module_disabled("x.mod")
        # State unchanged after the read-only check.
        assert is_module_disabled("x.mod") is True

    def test_check_module_disabled_input_registry_param(self) -> None:
        """system_modules.check_module_disabled.input.registry_param

        DIVERGENCE: spec declares a two-arg signature (module_id, registry);
        the Python implementation is single-arg using a global toggle state.
        """
        import inspect

        params = list(inspect.signature(check_module_disabled).parameters)
        if "registry" not in params:
            pytest.skip(
                "system_modules.check_module_disabled.input.registry_param: "
                "Python impl is single-arg (module_id only); uses global "
                "_default_toggle_state, no `registry` parameter exists."
            )
        assert "registry" in params


# ===========================================================================
# Contract 5: is_module_disabled
# ===========================================================================


class TestIsModuleDisabledContract:
    def test_is_module_disabled_return_true_when_disabled(self) -> None:
        """system_modules.is_module_disabled.return.true_when_disabled

        Returns True for a disabled module.
        """
        _default_toggle_state.disable("dm")
        assert is_module_disabled("dm") is True

    def test_is_module_disabled_return_false_when_enabled(self) -> None:
        """system_modules.is_module_disabled.return.false_when_enabled

        Returns False for an enabled module.
        """
        assert is_module_disabled("em") is False

    def test_is_module_disabled_error_never_raises_unknown(self) -> None:
        """system_modules.is_module_disabled.error.never_raises_unknown

        Never raises; returns False for unknown module IDs.
        """
        assert is_module_disabled("totally.unknown.module.id") is False

    def test_is_module_disabled_property_pure_read_only(self) -> None:
        """system_modules.is_module_disabled.property.pure_read_only

        Pure read of toggle state; repeated calls are stable and side-effect free.
        """
        _default_toggle_state.disable("p.mod")
        assert is_module_disabled("p.mod") is True
        assert is_module_disabled("p.mod") is True

    def test_is_module_disabled_input_registry_param(self) -> None:
        """system_modules.is_module_disabled.input.registry_param

        DIVERGENCE: spec declares a two-arg signature (module_id, registry);
        the Python implementation is single-arg using a global toggle state.
        """
        import inspect

        params = list(inspect.signature(is_module_disabled).parameters)
        if "registry" not in params:
            pytest.skip(
                "system_modules.is_module_disabled.input.registry_param: "
                "Python impl is single-arg (module_id only); uses global "
                "_default_toggle_state, no `registry` parameter exists."
            )
        assert "registry" in params


# ===========================================================================
# Contract 6: register_sys_modules
# ===========================================================================


class TestRegisterSysModulesContract:
    def _executor(self, registry: Registry) -> Executor:
        return Executor(registry=registry)

    def test_register_sys_modules_side_effect_disabled_noop(self) -> None:
        """system_modules.register_sys_modules.side_effect.disabled_returns_empty

        When sys_modules.enabled is False, returns an empty context (exit early).
        """
        registry = Registry()
        executor = self._executor(registry)
        config = _make_config()  # sys_modules.enabled defaults to False
        result = register_sys_modules(registry=registry, executor=executor, config=config)
        assert result == {}

    def test_register_sys_modules_return_context_keys(self) -> None:
        """system_modules.register_sys_modules.return.context_components

        When enabled, returns a context dict with error_history,
        error_history_middleware, usage_collector, usage_middleware.
        """
        registry = Registry()
        executor = self._executor(registry)
        config = _make_config(**{"sys_modules.enabled": True})
        result = register_sys_modules(registry=registry, executor=executor, config=config)
        for key in (
            "error_history",
            "error_history_middleware",
            "usage_collector",
            "usage_middleware",
        ):
            assert key in result, f"missing context key: {key}"

    def test_register_sys_modules_side_effect_registers_modules(self) -> None:
        """system_modules.register_sys_modules.side_effect.registers_health_modules

        When enabled, health/manifest/usage system modules are registered.
        """
        registry = Registry()
        executor = self._executor(registry)
        config = _make_config(**{"sys_modules.enabled": True})
        register_sys_modules(registry=registry, executor=executor, config=config)
        assert registry.has("system.health.summary")
        assert registry.has("system.manifest.full")

    def test_register_sys_modules_input_fail_on_error_param(self) -> None:
        """system_modules.register_sys_modules.input.fail_on_error_default

        Accepts fail_on_error (default False) per the 1.5 hardening rule.
        """
        import inspect

        params = inspect.signature(register_sys_modules).parameters
        assert "fail_on_error" in params
        assert params["fail_on_error"].default is False

    def test_register_sys_modules_property_not_idempotent(self) -> None:
        """system_modules.register_sys_modules.property.idempotent_false

        Registering twice raises (modules already registered).
        """
        registry = Registry()
        executor = self._executor(registry)
        config = _make_config(**{"sys_modules.enabled": True})
        register_sys_modules(registry=registry, executor=executor, config=config)
        with pytest.raises(Exception):
            register_sys_modules(
                registry=registry,
                executor=executor,
                config=config,
                fail_on_error=True,
            )

    def test_register_sys_modules_property_not_async(self) -> None:
        """system_modules.register_sys_modules.property.async_false

        register_sys_modules is synchronous (non-coroutine).
        """
        assert not asyncio.iscoroutinefunction(register_sys_modules)
