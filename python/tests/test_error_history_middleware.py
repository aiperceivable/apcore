"""Tests for ErrorHistoryMiddleware."""

from __future__ import annotations

from unittest.mock import MagicMock


from apcore.errors import ModuleError, ModuleNotFoundError
from apcore.middleware.base import Middleware
from apcore.middleware.error_history_middleware import ErrorHistoryMiddleware
from apcore.observability.error_history import ErrorHistory


class TestErrorHistoryMiddleware:
    def test_middleware_inherits_base(self) -> None:
        history = ErrorHistory()
        mw = ErrorHistoryMiddleware(history)
        assert isinstance(mw, Middleware)

    def test_on_error_records_module_error(self) -> None:
        history = ErrorHistory()
        mw = ErrorHistoryMiddleware(history)
        error = ModuleError(code="TIMEOUT", message="timed out", ai_guidance="retry")
        mw.on_error("payment.charge", {}, error, MagicMock())
        entries = history.get("payment.charge")
        assert len(entries) == 1
        assert entries[0].code == "TIMEOUT"
        assert entries[0].ai_guidance == "retry"

    def test_on_error_ignores_generic_exception(self) -> None:
        history = ErrorHistory()
        mw = ErrorHistoryMiddleware(history)
        mw.on_error("payment.charge", {}, ValueError("bad"), MagicMock())
        entries = history.get("payment.charge")
        assert len(entries) == 0

    def test_on_error_records_module_error_subclass(self) -> None:
        history = ErrorHistory()
        mw = ErrorHistoryMiddleware(history)
        error = ModuleNotFoundError(module_id="x")
        mw.on_error("x", {}, error, MagicMock())
        entries = history.get("x")
        assert len(entries) == 1

    def test_on_error_returns_none(self) -> None:
        history = ErrorHistory()
        mw = ErrorHistoryMiddleware(history)
        result = mw.on_error("m", {}, ModuleError(code="E", message="e"), MagicMock())
        assert result is None

    def test_on_error_returns_none_for_generic(self) -> None:
        history = ErrorHistory()
        mw = ErrorHistoryMiddleware(history)
        result = mw.on_error("m", {}, ValueError("x"), MagicMock())
        assert result is None

    def test_before_returns_none(self) -> None:
        history = ErrorHistory()
        mw = ErrorHistoryMiddleware(history)
        assert mw.before("m", {}, MagicMock()) is None

    def test_after_returns_none(self) -> None:
        history = ErrorHistory()
        mw = ErrorHistoryMiddleware(history)
        assert mw.after("m", {}, {}, MagicMock()) is None
