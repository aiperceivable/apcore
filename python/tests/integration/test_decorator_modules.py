"""Integration tests for @module decorated functions through the Executor."""

from __future__ import annotations

import pytest

from apcore.decorator import module
from apcore.errors import SchemaValidationError
from apcore.executor import Executor
from apcore.registry import Registry


class TestDecoratorModules:
    """Decorator module integration tests."""

    def test_decorated_function_callable_via_executor(self, tmp_path):
        registry = Registry(extensions_dir=str(tmp_path))

        @module(id="test.add", registry=registry)
        def add(a: int, b: int) -> dict:
            return {"sum": a + b}

        executor = Executor(registry=registry)
        result = executor.call("test.add", {"a": 3, "b": 4})
        assert result == {"sum": 7}

    def test_auto_generated_schemas_validate_inputs(self, tmp_path):
        registry = Registry(extensions_dir=str(tmp_path))

        @module(id="test.typed", registry=registry)
        def typed_func(x: int) -> dict:
            return {"result": x}

        executor = Executor(registry=registry)
        result = executor.call("test.typed", {"x": 42})
        assert result == {"result": 42}

    def test_auto_generated_schemas_reject_invalid_inputs(self, tmp_path):
        registry = Registry(extensions_dir=str(tmp_path))

        @module(id="test.strict", registry=registry)
        def strict_func(x: int) -> dict:
            return {"result": x}

        executor = Executor(registry=registry)
        with pytest.raises(SchemaValidationError):
            executor.call("test.strict", {"x": "not_an_int"})

    @pytest.mark.asyncio
    async def test_async_decorated_function_via_executor(self, tmp_path):
        registry = Registry(extensions_dir=str(tmp_path))

        @module(id="test.async_double", registry=registry)
        async def async_double(n: int) -> dict:
            return {"result": n * 2}

        executor = Executor(registry=registry)
        result = await executor.call_async("test.async_double", {"n": 5})
        assert result == {"result": 10}
