"""Tests for ErrorFormatterRegistry (§8.8)."""

from __future__ import annotations

from typing import Any

import pytest

from apcore.error_formatter import ErrorFormatter, ErrorFormatterRegistry
from apcore.errors import ErrorFormatterDuplicateError, InternalError, ModuleError


# ---------------------------------------------------------------------------
# Concrete formatter for testing
# ---------------------------------------------------------------------------


class _JsonRpcFormatter:
    """Minimal ErrorFormatter implementation for tests."""

    def format(self, error: ModuleError, context: object) -> dict[str, Any]:
        return {
            "jsonrpc": "2.0",
            "error": {
                "code": -32000,
                "message": error.message,
                "data": {"apcore_code": error.code, "context": str(context)},
            },
        }


class _HttpFormatter:
    def format(self, error: ModuleError, context: object) -> dict[str, Any]:
        return {"status": 500, "body": error.to_dict()}


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


class TestErrorFormatterRegistry:
    def setup_method(self) -> None:
        ErrorFormatterRegistry._reset()

    def teardown_method(self) -> None:
        ErrorFormatterRegistry._reset()

    # -- Protocol conformance --

    def test_formatter_satisfies_protocol(self) -> None:
        formatter = _JsonRpcFormatter()
        assert isinstance(formatter, ErrorFormatter)

    # -- register --

    def test_register_stores_formatter(self) -> None:
        fmt = _JsonRpcFormatter()
        ErrorFormatterRegistry.register("jsonrpc", fmt)
        assert ErrorFormatterRegistry.get("jsonrpc") is fmt

    def test_register_duplicate_raises(self) -> None:
        ErrorFormatterRegistry.register("jsonrpc", _JsonRpcFormatter())
        with pytest.raises(ErrorFormatterDuplicateError):
            ErrorFormatterRegistry.register("jsonrpc", _JsonRpcFormatter())

    def test_register_different_adapters_ok(self) -> None:
        ErrorFormatterRegistry.register("jsonrpc", _JsonRpcFormatter())
        ErrorFormatterRegistry.register("http", _HttpFormatter())
        assert ErrorFormatterRegistry.get("jsonrpc") is not None
        assert ErrorFormatterRegistry.get("http") is not None

    # -- get --

    def test_get_unknown_returns_none(self) -> None:
        assert ErrorFormatterRegistry.get("unknown") is None

    # -- format --

    def test_format_uses_registered_formatter(self) -> None:
        ErrorFormatterRegistry.register("jsonrpc", _JsonRpcFormatter())
        error = InternalError(message="Oops")
        result = ErrorFormatterRegistry.format("jsonrpc", error, context={"req_id": 1})
        assert "jsonrpc" in result
        assert result["error"]["message"] == "Oops"

    def test_format_falls_back_to_to_dict_when_no_formatter(self) -> None:
        error = InternalError(message="Fallback")
        result = ErrorFormatterRegistry.format("unknown_adapter", error)
        assert result["code"] == "GENERAL_INTERNAL_ERROR"
        assert result["message"] == "Fallback"

    def test_format_context_none_default(self) -> None:
        ErrorFormatterRegistry.register("jsonrpc", _JsonRpcFormatter())
        error = InternalError(message="Ctx test")
        # Should not raise
        result = ErrorFormatterRegistry.format("jsonrpc", error)
        assert result is not None

    def test_format_passes_context_to_formatter(self) -> None:
        ErrorFormatterRegistry.register("jsonrpc", _JsonRpcFormatter())
        error = InternalError(message="Ctx check")
        ctx = {"trace_id": "abc123"}
        result = ErrorFormatterRegistry.format("jsonrpc", error, context=ctx)
        assert "abc123" in result["error"]["data"]["context"]

    # -- _reset --

    def test_reset_clears_all_formatters(self) -> None:
        ErrorFormatterRegistry.register("jsonrpc", _JsonRpcFormatter())
        ErrorFormatterRegistry._reset()
        assert ErrorFormatterRegistry.get("jsonrpc") is None
