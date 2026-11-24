"""Tests for Algorithm A11 propagate_error()."""

from __future__ import annotations

from typing import Any

import pytest

from apcore.context import Context
from apcore.errors import ModuleError, ModuleExecuteError, SchemaValidationError
from apcore.executor import Executor
from apcore.registry import Registry
from apcore.utils.error_propagation import propagate_error


def _make_context() -> Context:
    """Create a simple context with a child module in the call chain."""
    ctx = Context.create()
    return ctx.child("test.module")


class TestPropagateErrorRawException:
    """Raw Python exceptions are wrapped as ModuleExecuteError."""

    def test_wraps_value_error_as_module_execute_error(self) -> None:
        ctx = _make_context()
        raw = ValueError("bad value")
        result = propagate_error(raw, "test.module", ctx)

        assert isinstance(result, ModuleExecuteError)
        assert result.code == "MODULE_EXECUTE_ERROR"

    def test_wrapped_error_has_trace_id(self) -> None:
        ctx = _make_context()
        raw = ValueError("bad value")
        result = propagate_error(raw, "test.module", ctx)

        assert result.trace_id == ctx.trace_id

    def test_wrapped_error_has_module_id_in_details(self) -> None:
        ctx = _make_context()
        raw = ValueError("bad value")
        result = propagate_error(raw, "test.module", ctx)

        assert result.details["module_id"] == "test.module"

    def test_wrapped_error_has_call_chain_in_details(self) -> None:
        ctx = _make_context()
        raw = ValueError("bad value")
        result = propagate_error(raw, "test.module", ctx)

        assert result.details["call_chain"] == ["test.module"]

    def test_wrapped_error_has_cause(self) -> None:
        ctx = _make_context()
        raw = ValueError("bad value")
        result = propagate_error(raw, "test.module", ctx)

        assert result.cause is raw

    def test_message_includes_original_error_type_and_message(self) -> None:
        ctx = _make_context()
        raw = ValueError("bad value")
        result = propagate_error(raw, "test.module", ctx)

        assert "ValueError" in result.message
        assert "bad value" in result.message
        assert "test.module" in result.message


class TestPropagateErrorModuleError:
    """ModuleError instances are enriched, not re-wrapped."""

    def test_enriches_schema_validation_error_with_trace_id(self) -> None:
        ctx = _make_context()
        original = SchemaValidationError(message="bad schema")
        assert original.trace_id is None

        result = propagate_error(original, "test.module", ctx)

        assert result is original
        assert result.trace_id == ctx.trace_id
        assert result.code == "SCHEMA_VALIDATION_ERROR"

    def test_enriches_with_module_id(self) -> None:
        ctx = _make_context()
        original = SchemaValidationError(message="bad schema")

        result = propagate_error(original, "test.module", ctx)

        assert result.details["module_id"] == "test.module"

    def test_enriches_with_call_chain(self) -> None:
        ctx = _make_context()
        original = SchemaValidationError(message="bad schema")

        result = propagate_error(original, "test.module", ctx)

        assert result.details["call_chain"] == ["test.module"]

    def test_does_not_overwrite_existing_trace_id(self) -> None:
        ctx = _make_context()
        original = SchemaValidationError(message="bad schema", trace_id="existing-trace")

        result = propagate_error(original, "test.module", ctx)

        assert result.trace_id == "existing-trace"

    def test_does_not_overwrite_existing_module_id(self) -> None:
        ctx = _make_context()
        original = ModuleError(code="CUSTOM", message="custom error", details={"module_id": "original.module"})

        result = propagate_error(original, "test.module", ctx)

        assert result.details["module_id"] == "original.module"

    def test_does_not_overwrite_existing_call_chain(self) -> None:
        ctx = _make_context()
        original = ModuleError(
            code="CUSTOM",
            message="custom error",
            details={"call_chain": ["a", "b"]},
        )

        result = propagate_error(original, "test.module", ctx)

        assert result.details["call_chain"] == ["a", "b"]


class TestPropagateErrorCallChainCopy:
    """call_chain in details is a copy, not a reference."""

    def test_call_chain_is_a_copy(self) -> None:
        ctx = _make_context()
        raw = ValueError("test")
        result = propagate_error(raw, "test.module", ctx)

        ctx.call_chain.append("extra.module")

        assert "extra.module" not in result.details["call_chain"]

    def test_module_error_call_chain_is_a_copy(self) -> None:
        ctx = _make_context()
        original = SchemaValidationError(message="bad")
        propagate_error(original, "test.module", ctx)

        ctx.call_chain.append("extra.module")

        assert "extra.module" not in original.details["call_chain"]


class TestPropagateErrorExecutorIntegration:
    """Integration: Executor.call() wraps raw exceptions via propagate_error."""

    def test_executor_wraps_value_error(self) -> None:
        class RaisingModule:
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                raise ValueError("something went wrong")

        registry = Registry()
        registry.register("test.raising", RaisingModule())
        executor = Executor(registry=registry)

        with pytest.raises(ModuleExecuteError) as exc_info:
            executor.call("test.raising", {})

        err = exc_info.value
        assert err.code == "MODULE_EXECUTE_ERROR"
        assert err.trace_id is not None
        assert err.details["module_id"] == "test.raising"
        assert "call_chain" in err.details
        assert "ValueError" in err.message
        assert err.cause is not None

    def test_executor_preserves_module_error_type(self) -> None:
        class SchemaFailModule:
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                raise SchemaValidationError(message="custom schema error")

        registry = Registry()
        registry.register("test.schema_fail", SchemaFailModule())
        executor = Executor(registry=registry)

        with pytest.raises(SchemaValidationError) as exc_info:
            executor.call("test.schema_fail", {})

        err = exc_info.value
        assert err.code == "SCHEMA_VALIDATION_ERROR"
        assert err.trace_id is not None
        assert err.details["module_id"] == "test.schema_fail"
