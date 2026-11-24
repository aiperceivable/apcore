"""Unit tests for APCore client class."""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel

from apcore.acl import ACL
from apcore.approval import AutoApproveHandler
from apcore.client import APCore
from apcore.config import Config
from apcore.context import Context
from apcore.decorator import FunctionModule
from apcore.errors import (
    ACLDeniedError,
    ModuleNotFoundError,
    ModuleTimeoutError,
    SysModuleRegistrationError,
    SysModulesDisabledError,
)
from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.policy import ExecutionPolicy, PolicyRule
from apcore.registry import Registry


# ---------------------------------------------------------------------------
# T2: Decorator tests
# ---------------------------------------------------------------------------


class TestModuleDecorator:
    @pytest.mark.asyncio
    @pytest.mark.parametrize("entrypoint", ["client", "global", "explicit_registry"])
    async def test_public_decorators_apply_module_timeout(self, entrypoint, monkeypatch) -> None:
        import apcore

        client = APCore()
        module_id = "test.module_budget"
        if entrypoint == "client":
            decorator = client.module(id=module_id, resources={"timeout": 10})
        elif entrypoint == "global":
            monkeypatch.setattr(apcore, "_get_default_client", lambda: client)
            decorator = apcore.module(id=module_id, resources={"timeout": 10})
        else:
            decorator = apcore.module(id=module_id, resources={"timeout": 10}, registry=client.registry)

        @decorator
        async def slow() -> dict:
            await asyncio.sleep(0.1)
            return {}

        with pytest.raises(ModuleTimeoutError) as exc:
            await client.call_async(module_id, {})
        assert exc.value.details["timeout_ms"] == 10

    def test_returns_original_function(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        assert callable(add)
        assert add(1, 2) == 3

    def test_not_function_module(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        assert not isinstance(add, FunctionModule)

    def test_registers_in_registry(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        assert client.registry.has("math.add")

    def test_attaches_apcore_module(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        assert hasattr(add, "apcore_module")
        assert isinstance(add.apcore_module, FunctionModule)  # type: ignore[attr-defined]

    def test_preserves_function_name(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        assert add.__name__ == "add"

    def test_with_description_only(self) -> None:
        """Verify decorator works with explicit id but no description override."""
        client = APCore()

        @client.module(id="util.identity", description="identity function")
        def my_func(x: int) -> int:
            return x

        assert client.registry.count == 1
        assert callable(my_func)
        assert my_func(42) == 42
        assert my_func.apcore_module.description == "identity function"  # type: ignore[attr-defined]

    def test_custom_metadata(self) -> None:
        client = APCore()

        @client.module(
            id="math.add",
            description="Add two numbers",
            tags=["math"],
            version="2.0.0",
        )
        def add(a: int, b: int) -> int:
            return a + b

        fm = add.apcore_module  # type: ignore[attr-defined]
        assert fm.description == "Add two numbers"
        assert fm.tags == ["math"]
        assert fm.version == "2.0.0"

    @pytest.mark.asyncio
    async def test_async_function(self) -> None:
        client = APCore()

        @client.module(id="async.greet")
        async def greet(name: str) -> str:
            return f"Hello, {name}"

        assert callable(greet)
        result = await greet("World")
        assert result == "Hello, World"

    def test_global_module_decorator_returns_function(self) -> None:
        import apcore

        module_id = "global.test.add"

        @apcore.module(id=module_id)
        def add(a: int, b: int) -> int:
            return a + b

        try:
            assert callable(add)
            assert not isinstance(add, FunctionModule)
            assert add(3, 4) == 7
        finally:
            apcore._default_client.registry.unregister(module_id)


# ---------------------------------------------------------------------------
# T3: Construction tests
# ---------------------------------------------------------------------------


class TestAPCoreConstruction:
    def test_default_construction(self) -> None:
        client = APCore()
        assert isinstance(client.registry, Registry)
        assert isinstance(client.executor, Executor)
        assert client.config is None

    def test_custom_registry(self) -> None:
        registry = Registry()
        client = APCore(registry=registry)
        assert client.registry is registry

    def test_custom_executor(self) -> None:
        registry = Registry()
        executor = Executor(registry=registry)
        client = APCore(registry=registry, executor=executor)
        assert client.executor is executor

    def test_config_passed_through(self) -> None:
        from apcore.config import Config

        config = Config({"extensions": {"root": "/tmp"}})
        client = APCore(config=config)
        assert client.config is config

    def test_config_path_construction(self, tmp_path: Any) -> None:
        import yaml

        config_file = tmp_path / "apcore.yaml"
        # `version` and `project.name` have no canonical default, so
        # Config.load()'s default validate=True requires them (§9.1).
        config_data = {
            "version": "1.0.0",
            "project": {"name": "test"},
            "extensions": {"root": "/custom/path"},
        }
        config_file.write_text(yaml.dump(config_data))

        config = Config.load(str(config_file))
        client = APCore(config=config)
        assert client.config is not None
        assert client.config.get("extensions.root") == "/custom/path"


# ---------------------------------------------------------------------------
# T3: Register tests
# ---------------------------------------------------------------------------


class TestRegister:
    def test_register_class_module(self) -> None:
        client = APCore()

        class AddInput(BaseModel):
            a: int
            b: int

        class AddOutput(BaseModel):
            result: int

        class AddModule:
            input_schema = AddInput
            output_schema = AddOutput
            description = "Add two numbers"

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {"result": inputs["a"] + inputs["b"]}

        client.register("math.add", AddModule())
        assert client.registry.has("math.add")

        result = client.call("math.add", {"a": 10, "b": 5})
        assert result == {"result": 15}


# ---------------------------------------------------------------------------
# T3: Call tests
# ---------------------------------------------------------------------------


class TestCall:
    def test_call_sync(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        result = client.call("math.add", {"a": 10, "b": 5})
        assert result == {"result": 15}

    @pytest.mark.asyncio
    async def test_call_async(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        result = await client.call_async("math.add", {"a": 10, "b": 5})
        assert result == {"result": 15}

    def test_call_not_found(self) -> None:
        client = APCore()
        with pytest.raises(ModuleNotFoundError):
            client.call("nonexistent.module", {"a": 1})


# ---------------------------------------------------------------------------
# T3: Use (middleware) tests
# ---------------------------------------------------------------------------


class TestUse:
    def test_use_returns_self(self) -> None:
        client = APCore()
        mw = _TrackingMiddleware()
        result = client.use(mw)
        assert result is client

    def test_use_chaining(self) -> None:
        client = APCore()
        mw1 = _TrackingMiddleware()
        mw2 = _TrackingMiddleware()
        result = client.use(mw1).use(mw2)
        assert result is client

    def test_use_middleware_fires(self) -> None:
        client = APCore()
        mw = _TrackingMiddleware()
        client.use(mw)

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        client.call("math.add", {"a": 1, "b": 2})
        assert mw.before_called
        assert mw.after_called


# ---------------------------------------------------------------------------
# T4: Discover tests
# ---------------------------------------------------------------------------


class TestDiscover:
    def test_discover_delegates_to_registry(self) -> None:
        """Verify discover() proxies to registry.discover()."""
        client = APCore()
        # Calling discover() should delegate to registry.discover().
        # Without a valid extensions dir, this raises ConfigNotFoundError.
        from apcore.errors import ConfigNotFoundError

        with pytest.raises(ConfigNotFoundError):
            client.discover()


# ---------------------------------------------------------------------------
# T4: ListModules tests
# ---------------------------------------------------------------------------


class TestListModules:
    def test_empty(self) -> None:
        client = APCore()
        assert client.list_modules() == []

    def test_with_entries(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        @client.module(id="greet.hello")
        def hello(name: str) -> str:
            return f"Hello, {name}"

        modules = client.list_modules()
        assert modules == ["greet.hello", "math.add"]

    def test_filter_prefix(self) -> None:
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        @client.module(id="greet.hello")
        def hello(name: str) -> str:
            return f"Hello, {name}"

        modules = client.list_modules(prefix="math")
        assert modules == ["math.add"]

    def test_filter_tags(self) -> None:
        client = APCore()

        @client.module(id="math.add", tags=["math", "core"])
        def add(a: int, b: int) -> int:
            return a + b

        @client.module(id="greet.hello", tags=["greet"])
        def hello(name: str) -> str:
            return f"Hello, {name}"

        modules = client.list_modules(tags=["math"])
        assert modules == ["math.add"]


# ---------------------------------------------------------------------------
# T4: Global convenience functions
# ---------------------------------------------------------------------------


class TestGlobalConvenience:
    def test_global_list_modules(self) -> None:
        import apcore

        assert hasattr(apcore, "list_modules")
        assert callable(apcore.list_modules)

    def test_global_discover(self) -> None:
        import apcore

        assert hasattr(apcore, "discover")
        assert callable(apcore.discover)


# ---------------------------------------------------------------------------
# T5: Events convenience tests
# ---------------------------------------------------------------------------


def _sys_config() -> Config:
    """Create a Config with sys_modules and events enabled."""
    return Config(
        {
            "sys_modules": {
                "enabled": True,
                "events": {"enabled": True},
            },
        }
    )


@pytest.mark.parametrize("fail_on_error", [False, True])
def test_client_reports_system_module_registration_failures(
    monkeypatch: pytest.MonkeyPatch, fail_on_error: bool
) -> None:
    registry = Registry()
    register_internal = registry.register_internal

    def register(module_id: str, module: object) -> None:
        if module_id == "system.health.summary":
            raise ValueError("test registration failure")
        register_internal(module_id, module)

    monkeypatch.setattr(registry, "register_internal", register)
    if fail_on_error:
        with pytest.raises(SysModuleRegistrationError) as raised:
            APCore(registry=registry, config=_sys_config(), fail_on_error=True)
        errors = (raised.value,)
    else:
        client = APCore(registry=registry, config=_sys_config())
        errors = client.sys_modules_errors

    assert len(errors) == 1
    assert isinstance(errors[0], SysModuleRegistrationError)
    assert errors[0].code == "SYS_MODULE_REGISTRATION_FAILED"
    assert errors[0].details["module_id"] == "system.health.summary"
    assert registry.has("system.health.module") is (not fail_on_error)


class TestEvents:
    def test_client_receives_executor_governance_events(self) -> None:
        policy = ExecutionPolicy([PolicyRule("test.governed", requires_approval=True)])
        client = APCore(config=_sys_config(), policy=policy)
        client.executor.set_approval_handler(AutoApproveHandler())
        received: list[ApCoreEvent] = []
        for event_type in ("apcore.policy.override", "apcore.approval.decision", "apcore.acl.denied"):
            client.on(event_type, received.append)

        @client.module(id="test.governed")
        def governed() -> dict:
            return {"executed": True}

        assert client.call("test.governed", {}) == {"executed": True}
        client.executor.set_acl(ACL())
        with pytest.raises(ACLDeniedError):
            client.call("test.governed", {})

        assert client.events is not None
        client.events.flush()
        assert sorted((event.event_type, event.module_id) for event in received) == [
            ("apcore.acl.denied", "test.governed"),
            ("apcore.approval.decision", "test.governed"),
            ("apcore.policy.override", "test.governed"),
        ]

    def test_client_preserves_supplied_executor_emitter(self) -> None:
        emitter = EventEmitter()
        executor = Executor(Registry(), acl=ACL(), event_emitter=emitter)
        client = APCore(executor=executor, config=_sys_config())
        received: list[ApCoreEvent] = []

        class Subscriber:
            async def on_event(self, event: ApCoreEvent) -> None:
                received.append(event)

        emitter.subscribe(Subscriber())
        client_events: list[ApCoreEvent] = []
        client.on("apcore.acl.denied", client_events.append)

        @client.module(id="test.denied")
        def denied() -> dict:
            return {}

        with pytest.raises(ACLDeniedError):
            client.call("test.denied", {})

        emitter.flush()
        assert client.events is not None
        client.events.flush()
        assert [event.event_type for event in received] == ["apcore.acl.denied"]
        assert client_events == []

    def test_client_exposes_no_event_bus_when_events_are_disabled(self) -> None:
        client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": False}}}))
        assert client.events is None
        with pytest.raises(SysModulesDisabledError):
            client.on("apcore.acl.denied", lambda _event: None)

    @pytest.mark.parametrize("enabled", [False, True])
    def test_namespace_event_bus_uses_the_persisted_setting(self, tmp_path: Path, enabled: bool) -> None:
        root = tmp_path / "config"
        root.mkdir()
        (root / "overrides.yaml").write_text(f"sys_modules.events.enabled: {str(enabled).lower()}\n", encoding="utf-8")
        path = root / "apcore.yaml"
        path.write_text(
            'apcore:\n  version: "1.0.0"\n  project: {name: client-test}\n'
            "sys_modules:\n  enabled: true\n"
            f"  events: {{enabled: {str(not enabled).lower()}}}\n"
            "  control: {overrides_path: ./overrides.yaml}\n",
            encoding="utf-8",
        )
        client = APCore(config=Config.load(str(path)))
        assert (client.events is not None) is enabled

    def test_events_none_without_config(self) -> None:
        client = APCore()
        assert client.events is None

    def test_events_none_without_sys_modules_enabled(self) -> None:
        config = Config({"sys_modules": {"enabled": False}})
        client = APCore(config=config)
        assert client.events is None

    def test_events_available_with_config(self) -> None:
        client = APCore(config=_sys_config())
        assert client.events is not None

    def test_on_raises_without_events(self) -> None:
        client = APCore()
        with pytest.raises(SysModulesDisabledError, match="Events are not enabled"):
            client.on("test_event", lambda e: None)

    def test_off_raises_without_events(self) -> None:
        client = APCore()
        with pytest.raises(SysModulesDisabledError, match="Events are not enabled"):
            client.off(object())  # type: ignore[arg-type]

    def test_on_subscribes_and_fires(self) -> None:
        client = APCore(config=_sys_config())
        received: list[ApCoreEvent] = []

        sub = client.on("my_event", lambda e: received.append(e))
        assert sub is not None

        event = ApCoreEvent(
            event_type="my_event",
            module_id=None,
            timestamp="2026-01-01T00:00:00Z",
            severity="info",
            data={"key": "value"},
        )
        assert client.events is not None
        client.events.emit(event)
        client.events.flush()

        assert len(received) == 1
        assert received[0].data == {"key": "value"}

    def test_on_filters_by_event_type(self) -> None:
        client = APCore(config=_sys_config())
        received: list[ApCoreEvent] = []

        client.on("target", lambda e: received.append(e))

        assert client.events is not None
        client.events.emit(
            ApCoreEvent(
                event_type="other",
                module_id=None,
                timestamp="t",
                severity="info",
                data={},
            )
        )
        client.events.emit(
            ApCoreEvent(
                event_type="target",
                module_id=None,
                timestamp="t",
                severity="info",
                data={"hit": True},
            )
        )
        client.events.flush()

        assert len(received) == 1
        assert received[0].data == {"hit": True}

    def test_on_with_async_handler(self) -> None:
        client = APCore(config=_sys_config())
        received: list[ApCoreEvent] = []

        async def handler(e: ApCoreEvent) -> None:
            received.append(e)

        client.on("async_ev", handler)

        assert client.events is not None
        client.events.emit(
            ApCoreEvent(
                event_type="async_ev",
                module_id=None,
                timestamp="t",
                severity="info",
                data={"async": True},
            )
        )
        client.events.flush()

        assert len(received) == 1
        assert received[0].data == {"async": True}

    def test_off_unsubscribes(self) -> None:
        client = APCore(config=_sys_config())
        received: list[ApCoreEvent] = []

        sub = client.on("ev", lambda e: received.append(e))
        client.off(sub)

        assert client.events is not None
        client.events.emit(
            ApCoreEvent(
                event_type="ev",
                module_id=None,
                timestamp="t",
                severity="info",
                data={},
            )
        )
        client.events.flush()

        assert len(received) == 0


# ---------------------------------------------------------------------------
# T5: Enable/Disable convenience tests
# ---------------------------------------------------------------------------


class TestEnableDisable:
    def _make_client_with_module(self) -> APCore:
        """Create a client with sys_modules enabled and a test module registered."""
        client = APCore(config=_sys_config())

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        return client

    def test_disable_returns_success(self) -> None:
        client = self._make_client_with_module()
        result = client.disable("math.add")
        assert result["success"] is True
        assert result["enabled"] is False
        assert result["module_id"] == "math.add"

    def test_enable_returns_success(self) -> None:
        client = self._make_client_with_module()
        client.disable("math.add")
        result = client.enable("math.add")
        assert result["success"] is True
        assert result["enabled"] is True

    def test_disable_nonexistent_raises(self) -> None:
        client = APCore(config=_sys_config())
        with pytest.raises(ModuleNotFoundError):
            client.disable("nonexistent.module")

    def test_custom_reason(self) -> None:
        client = self._make_client_with_module()
        result = client.disable("math.add", reason="Maintenance window")
        assert result["success"] is True

    def test_disable_without_sys_modules_raises_sys_modules_disabled(self) -> None:
        """disable() should raise SysModulesDisabledError (typed framework error,
        cross-language equivalent of Rust's SysModulesDisabled), not
        ModuleNotFoundError, when sys_modules not configured."""
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        with pytest.raises(SysModulesDisabledError, match="sys_modules"):
            client.disable("math.add")

    def test_enable_without_sys_modules_raises_sys_modules_disabled(self) -> None:
        """enable() should raise SysModulesDisabledError, not ModuleNotFoundError,
        when sys_modules not configured."""
        client = APCore()

        @client.module(id="math.add")
        def add(a: int, b: int) -> int:
            return a + b

        with pytest.raises(SysModulesDisabledError, match="sys_modules"):
            client.enable("math.add")


# ---------------------------------------------------------------------------
# T5: Global convenience for events/toggle
# ---------------------------------------------------------------------------


class TestGlobalEventsConvenience:
    def test_global_on_exists(self) -> None:
        import apcore

        assert hasattr(apcore, "on")
        assert callable(apcore.on)

    def test_global_off_exists(self) -> None:
        import apcore

        assert hasattr(apcore, "off")
        assert callable(apcore.off)

    def test_global_disable_exists(self) -> None:
        import apcore

        assert hasattr(apcore, "disable")
        assert callable(apcore.disable)

    def test_global_enable_exists(self) -> None:
        import apcore

        assert hasattr(apcore, "enable")
        assert callable(apcore.enable)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


class _TrackingMiddleware(Middleware):
    """Simple middleware that records whether before/after were called."""

    def __init__(self) -> None:
        self.before_called = False
        self.after_called = False

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
        self.before_called = True

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> None:
        self.after_called = True
