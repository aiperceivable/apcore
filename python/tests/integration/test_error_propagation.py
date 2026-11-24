"""Integration tests for error propagation through the Executor pipeline."""

from __future__ import annotations

import pytest

from apcore.errors import (
    ModuleExecuteError,
    ModuleNotFoundError,
    SchemaValidationError,
)
from apcore.executor import Executor
from apcore.middleware import Middleware


class TestErrorPropagation:
    """Error propagation integration tests."""

    def test_module_not_found_error_code(self, int_registry):
        executor = Executor(registry=int_registry)
        with pytest.raises(ModuleNotFoundError) as exc_info:
            executor.call("nonexistent.module", {})
        assert exc_info.value.code == "MODULE_NOT_FOUND"

    def test_module_not_found_error_for_nonexistent_module(self, int_executor):
        with pytest.raises(ModuleNotFoundError) as exc_info:
            int_executor.call("does.not.exist", {})
        assert "does.not.exist" in str(exc_info.value)

    def test_on_error_cascade_calls_middlewares_in_reverse(self, int_registry):
        log = []

        class TrackingMiddleware(Middleware):
            def __init__(self, name):
                self.name = name

            def on_error(self, module_id, inputs, error, context):
                log.append(self.name)
                return None

        executor = Executor(
            registry=int_registry,
            middlewares=[TrackingMiddleware("A"), TrackingMiddleware("B")],
        )
        with pytest.raises(ModuleExecuteError):
            executor.call("failing", {})
        assert log == ["B", "A"]

    def test_schema_validation_error_includes_field_details(self, int_registry):
        executor = Executor(registry=int_registry)
        with pytest.raises(SchemaValidationError) as exc_info:
            executor.call("greet", {"name": 12345})
        err = exc_info.value
        assert err.code == "SCHEMA_VALIDATION_ERROR"
        assert len(err.details["errors"]) > 0
        error_entry = err.details["errors"][0]
        assert set(error_entry) == {"path", "keyword", "message"}
        assert error_entry["path"] == "/name"
        assert error_entry["keyword"] == "type"
        assert error_entry["message"]
