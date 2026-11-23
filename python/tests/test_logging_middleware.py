"""Tests for LoggingMiddleware."""

from __future__ import annotations

import logging
import time
from typing import Any
from unittest.mock import MagicMock


from apcore.middleware.logging import LoggingMiddleware


def _make_context(
    trace_id: str = "trace-abc-123",
    caller_id: str | None = "some.caller",
    redacted_inputs: dict[str, Any] | None = None,
) -> MagicMock:
    """Create a mock Context with required attributes."""
    ctx = MagicMock()
    ctx.trace_id = trace_id
    ctx.caller_id = caller_id
    ctx.redacted_inputs = redacted_inputs
    ctx.data = {}
    return ctx


# === before() Tests ===


class TestBefore:
    """Tests for LoggingMiddleware.before()."""

    def test_stores_start_time_in_context_data(self) -> None:
        """before() writes a float timestamp to context.data['_apcore.mw.logging.start_time']."""
        mw = LoggingMiddleware(logger=MagicMock())
        ctx = _make_context()
        now = time.time()
        mw.before("test.module", {"key": "val"}, ctx)
        assert "_apcore.mw.logging.start_time" in ctx.data
        assert isinstance(ctx.data["_apcore.mw.logging.start_time"], float)
        assert abs(ctx.data["_apcore.mw.logging.start_time"] - now) < 1.0

    def test_logs_info_with_trace_id_module_id_caller_id(self) -> None:
        """before() logs INFO with trace_id, module_id, caller_id."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger)
        ctx = _make_context(
            trace_id="abc-123",
            caller_id="some.caller",
            redacted_inputs={"name": "test"},
        )
        mw.before("test.module", {"name": "test"}, ctx)

        mock_logger.info.assert_called_once()
        call_args = mock_logger.info.call_args
        msg = call_args[0][0]
        assert "abc-123" in msg
        assert "test.module" in msg

        extra = call_args[1]["extra"]
        assert extra["trace_id"] == "abc-123"
        assert extra["module_id"] == "test.module"
        assert extra["caller_id"] == "some.caller"
        assert extra["inputs"] == {"name": "test"}

    def test_logs_redacted_inputs_not_raw(self) -> None:
        """before() logs context.redacted_inputs, NOT raw inputs."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger)
        ctx = _make_context(redacted_inputs={"password": "***REDACTED***"})
        mw.before("test.module", {"password": "secret123"}, ctx)

        extra = mock_logger.info.call_args[1]["extra"]
        assert extra["inputs"] == {"password": "***REDACTED***"}

    def test_log_inputs_false_skips_logging(self) -> None:
        """before() with log_inputs=False skips the info log."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger, log_inputs=False)
        ctx = _make_context()
        mw.before("test.module", {}, ctx)

        mock_logger.info.assert_not_called()

    def test_returns_none(self) -> None:
        """before() always returns None."""
        mw = LoggingMiddleware(logger=MagicMock())
        result = mw.before("test.module", {}, _make_context())
        assert result is None


# === after() Tests ===


class TestAfter:
    """Tests for LoggingMiddleware.after()."""

    def test_calculates_duration_from_start_time(self) -> None:
        """after() computes duration_ms from context.data start time."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger)
        ctx = _make_context()
        ctx.data["_apcore.mw.logging.start_time"] = time.time() - 0.150  # 150ms ago
        mw.after("test.module", {}, {"result": "ok"}, ctx)

        extra = mock_logger.info.call_args[1]["extra"]
        assert abs(extra["duration_ms"] - 150.0) < 50.0

    def test_logs_info_with_trace_id_module_id_duration_output(self) -> None:
        """after() logs INFO with trace_id, module_id, duration_ms, output."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger)
        ctx = _make_context(trace_id="abc-123")
        ctx.data["_apcore.mw.logging.start_time"] = time.time()
        output = {"result": "ok"}
        ctx.redacted_output = (
            output  # D-131: the middleware logs the captured value, as the pipeline's capture point sets it
        )
        mw.after("test.module", {}, output, ctx)

        mock_logger.info.assert_called_once()
        msg = mock_logger.info.call_args[0][0]
        assert "abc-123" in msg
        assert "test.module" in msg

        extra = mock_logger.info.call_args[1]["extra"]
        assert "trace_id" in extra
        assert "module_id" in extra
        assert "duration_ms" in extra
        assert extra["output"] == output

    def test_log_outputs_false_skips_logging(self) -> None:
        """after() with log_outputs=False skips the info log."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger, log_outputs=False)
        ctx = _make_context()
        ctx.data["_apcore.mw.logging.start_time"] = time.time()
        mw.after("test.module", {}, {"result": "ok"}, ctx)

        mock_logger.info.assert_not_called()

    def test_missing_start_time_fallback(self) -> None:
        """after() with no start time falls back to ~0 duration."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger)
        ctx = _make_context()
        # No _apcore.mw.logging.start_time in ctx.data
        mw.after("test.module", {}, {"result": "ok"}, ctx)

        extra = mock_logger.info.call_args[1]["extra"]
        assert extra["duration_ms"] < 100.0  # Should be near 0

    def test_returns_none(self) -> None:
        """after() always returns None."""
        mw = LoggingMiddleware(logger=MagicMock())
        ctx = _make_context()
        ctx.data["_apcore.mw.logging.start_time"] = time.time()
        result = mw.after("test.module", {}, {"result": "ok"}, ctx)
        assert result is None


# === on_error() Tests ===


class TestOnError:
    """Tests for LoggingMiddleware.on_error()."""

    def test_logs_error_with_trace_id_module_id_error(self) -> None:
        """on_error() logs ERROR with trace_id, module_id, error."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger)
        ctx = _make_context(
            trace_id="abc-123",
            redacted_inputs={"name": "test"},
        )
        err = ValueError("something broke")
        mw.on_error("test.module", {"name": "test"}, err, ctx)

        mock_logger.error.assert_called_once()
        msg = mock_logger.error.call_args[0][0]
        assert "abc-123" in msg
        assert "test.module" in msg
        assert "something broke" in msg

        extra = mock_logger.error.call_args[1]["extra"]
        assert extra["trace_id"] == "abc-123"
        assert extra["module_id"] == "test.module"
        assert extra["error"] == "something broke"
        assert extra["inputs"] == {"name": "test"}

    def test_uses_redacted_inputs(self) -> None:
        """on_error() uses context.redacted_inputs, NOT raw inputs."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger)
        ctx = _make_context(redacted_inputs={"password": "***REDACTED***"})
        mw.on_error("test.module", {"password": "secret"}, ValueError("err"), ctx)

        extra = mock_logger.error.call_args[1]["extra"]
        assert extra["inputs"] == {"password": "***REDACTED***"}

    def test_includes_exc_info(self) -> None:
        """on_error() passes exc_info=True for traceback."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger)
        mw.on_error("test.module", {}, ValueError("err"), _make_context())

        assert mock_logger.error.call_args[1]["exc_info"] is True

    def test_log_errors_false_skips_logging(self) -> None:
        """on_error() with log_errors=False skips the error log."""
        mock_logger = MagicMock()
        mw = LoggingMiddleware(logger=mock_logger, log_errors=False)
        mw.on_error("test.module", {}, ValueError("err"), _make_context())

        mock_logger.error.assert_not_called()

    def test_returns_none(self) -> None:
        """on_error() always returns None."""
        mw = LoggingMiddleware(logger=MagicMock())
        result = mw.on_error("test.module", {}, ValueError("err"), _make_context())
        assert result is None


# === Concurrency and Configuration Tests ===


class TestConcurrencyAndConfig:
    """Tests for thread safety and configuration."""

    def test_concurrent_calls_independent_contexts(self) -> None:
        """Separate contexts do not interfere with each other."""
        mw = LoggingMiddleware(logger=MagicMock())
        ctx_a = _make_context(trace_id="trace-a")
        ctx_b = _make_context(trace_id="trace-b")

        mw.before("mod.a", {}, ctx_a)
        time.sleep(0.01)  # Small delay
        mw.before("mod.b", {}, ctx_b)

        # Start times should be independent
        assert ctx_a.data["_apcore.mw.logging.start_time"] != ctx_b.data["_apcore.mw.logging.start_time"]
        assert ctx_a.data["_apcore.mw.logging.start_time"] < ctx_b.data["_apcore.mw.logging.start_time"]

    def test_default_logger_name(self) -> None:
        """Default logger uses 'apcore.middleware.logging' name."""
        mw = LoggingMiddleware()
        assert mw._logger.name == "apcore.middleware.logging"

    def test_custom_logger(self) -> None:
        """Custom logger is used when provided."""
        custom = logging.getLogger("custom.test")
        mw = LoggingMiddleware(logger=custom)
        assert mw._logger is custom


def test_after_logs_redacted_output_not_raw() -> None:
    """after() logs context.redacted_output, NOT the raw output (§10.6.1 requirement 5, D-131)."""
    mock_logger = MagicMock()
    mw = LoggingMiddleware(logger=mock_logger)
    ctx = _make_context()
    ctx.redacted_output = {"password": "***REDACTED***"}
    mw.after("test.module", {}, {"password": "hunter2"}, ctx)

    assert mock_logger.info.call_args[1]["extra"]["output"] == {"password": "***REDACTED***"}
