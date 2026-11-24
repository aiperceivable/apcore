"""Shared test fixtures for the executor/middleware test suite."""

from __future__ import annotations

import time
from typing import Any

import pytest
from pydantic import BaseModel, ConfigDict

from apcore.context import Context
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.registry import Registry


# === Schemas ===


class SimpleInput(BaseModel):
    name: str


class SimpleOutput(BaseModel):
    greeting: str


class PermissiveInput(BaseModel):
    model_config = ConfigDict(extra="allow")


class PermissiveOutput(BaseModel):
    model_config = ConfigDict(extra="allow")


# === Module Fixtures ===


class SyncModuleImpl:
    """Minimal synchronous module."""

    input_schema = SimpleInput
    output_schema = SimpleOutput

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"greeting": f"Hello, {inputs['name']}!"}


class AsyncModuleImpl:
    """Minimal async module."""

    input_schema = SimpleInput
    output_schema = SimpleOutput

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"greeting": f"Hello, {inputs['name']}!"}


class FailingModuleImpl:
    """Module that always raises."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        raise RuntimeError("module execution failed")


class SlowModuleImpl:
    """Module that sleeps to trigger timeouts."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        time.sleep(5)
        return {"result": "slow"}


class RecordingMiddleware(Middleware):
    """Middleware that records all calls for test assertions."""

    def __init__(self) -> None:
        self.before_calls: list[tuple[str, dict[str, Any]]] = []
        self.after_calls: list[tuple[str, dict[str, Any], dict[str, Any]]] = []
        self.error_calls: list[tuple[str, dict[str, Any], Exception]] = []

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
        self.before_calls.append((module_id, inputs))
        return None

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> None:
        self.after_calls.append((module_id, inputs, output))
        return None

    def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context) -> None:
        self.error_calls.append((module_id, inputs, error))
        return None


# === Fixtures ===


@pytest.fixture
def sync_module() -> SyncModuleImpl:
    """A minimal synchronous module with defined input/output schemas."""
    return SyncModuleImpl()


@pytest.fixture
def async_module() -> AsyncModuleImpl:
    """A minimal async module with defined input/output schemas."""
    return AsyncModuleImpl()


@pytest.fixture
def failing_module() -> FailingModuleImpl:
    """A module whose execute() always raises RuntimeError."""
    return FailingModuleImpl()


@pytest.fixture
def slow_module() -> SlowModuleImpl:
    """A module whose execute() sleeps to trigger timeouts."""
    return SlowModuleImpl()


@pytest.fixture
def mock_registry(
    sync_module: SyncModuleImpl,
    async_module: AsyncModuleImpl,
    failing_module: FailingModuleImpl,
    slow_module: SlowModuleImpl,
) -> Registry:
    """Registry with pre-registered sync, async, failing, and slow modules."""
    reg = Registry()
    reg.register("test.sync_module", sync_module)
    reg.register("test.async_module", async_module)
    reg.register("test.failing_module", failing_module)
    reg.register("test.slow_module", slow_module)
    return reg


@pytest.fixture
def executor(mock_registry: Registry) -> Executor:
    """Executor with mock registry and default settings."""
    return Executor(registry=mock_registry)


@pytest.fixture
def acl_yaml(tmp_path: Any) -> str:
    """Write a sample ACL YAML file and return its path."""
    content = """
rules:
  - callers: ["test.*"]
    targets: ["test.*"]
    effect: allow
  - callers: ["@external"]
    targets: ["internal.*"]
    effect: deny
default_effect: deny
"""
    yaml_file = tmp_path / "acl.yaml"
    yaml_file.write_text(content)
    return str(yaml_file)


@pytest.fixture
def sample_middleware() -> RecordingMiddleware:
    """A recording middleware that tracks all calls to before/after/on_error."""
    return RecordingMiddleware()


@pytest.fixture
def chain_module_factory(mock_registry: Registry) -> Any:
    """Factory to create modules that chain-call other modules via context.executor."""

    def factory(module_id: str, calls: str | None = None) -> Any:
        class ChainModule:
            input_schema = SimpleInput
            output_schema = SimpleOutput

            def __init__(self, target: str | None) -> None:
                self._target = target

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                if self._target:
                    return context.executor.call(self._target, inputs, context)
                return {"greeting": f"Hello, {inputs['name']}!"}

        mod = ChainModule(target=calls)
        mock_registry.register(module_id, mod)
        return mod

    return factory
