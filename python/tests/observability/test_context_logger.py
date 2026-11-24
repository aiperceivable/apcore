"""Tests for ContextLogger and ObsLoggingMiddleware."""

from __future__ import annotations

import io
import json


from apcore.context import Context
from apcore.middleware import Middleware
from apcore.observability.context_logger import ContextLogger, ObsLoggingMiddleware


# --- ContextLogger Creation Tests ---


class TestContextLoggerCreation:
    """Test ContextLogger instantiation and defaults."""

    def test_created_with_name_and_defaults(self):
        """ContextLogger created with name and default settings."""
        logger = ContextLogger(name="test.logger")
        assert logger._name == "test.logger"
        assert logger._output_format == "json"
        assert logger._trace_id is None
        assert logger._module_id is None
        assert logger._caller_id is None

    def test_from_context_extracts_context_fields(self):
        """from_context extracts trace_id, module_id, caller_id."""
        ctx = Context.create()
        # Simulate call chain: system -> parent.module -> greet.module
        ctx.call_chain.append("parent.module")
        child = ctx.child("greet.module")
        # child.caller_id is "parent.module" (last in parent's call_chain)
        # child.call_chain is ["parent.module", "greet.module"]
        logger = ContextLogger.from_context(child, name="test")
        assert logger._trace_id == ctx.trace_id
        assert logger._module_id == "greet.module"
        assert logger._caller_id == "parent.module"

    def test_from_context_empty_call_chain(self):
        """from_context with empty call_chain sets module_id to None."""
        ctx = Context.create()
        logger = ContextLogger.from_context(ctx, name="test")
        assert logger._module_id is None


# --- Level Filtering Tests ---


class TestContextLoggerLevels:
    """Test log level filtering."""

    def test_info_emits_entry(self):
        """info() emits log entry with correct level."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", level="info", output=buf)
        logger.info("hello")
        output = buf.getvalue()
        assert output.strip()
        data = json.loads(output)
        assert data["level"] == "info"

    def test_debug_suppressed_at_info_level(self):
        """debug() not emitted when level='info'."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", level="info", output=buf)
        logger.debug("should not appear")
        assert buf.getvalue() == ""

    def test_error_emitted_at_info_level(self):
        """error() always emitted when level='info'."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", level="info", output=buf)
        logger.error("bad thing")
        output = buf.getvalue()
        assert output.strip()
        data = json.loads(output)
        assert data["level"] == "error"

    def test_all_level_filtering(self):
        """Log level filtering works for all levels."""
        levels = ["trace", "debug", "info", "warn", "error", "fatal"]
        for i, threshold in enumerate(levels):
            buf = io.StringIO()
            logger = ContextLogger(name="test", level=threshold, output=buf)
            for emit_level in levels:
                getattr(logger, emit_level)(f"msg at {emit_level}")
            lines = [line for line in buf.getvalue().strip().split("\n") if line]
            # Should have emitted levels from threshold onward
            assert len(lines) == len(levels) - i


# --- JSON Format Tests ---


class TestContextLoggerJsonFormat:
    """Test JSON format output."""

    def test_json_output_valid(self):
        """JSON format output is valid JSON with required fields."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", output_format="json", output=buf)
        logger.info("hello")
        data = json.loads(buf.getvalue())
        assert isinstance(data, dict)

    def test_json_includes_all_fields(self):
        """JSON includes timestamp, level, message, trace_id, module_id, caller_id, logger, extra."""
        buf = io.StringIO()
        ctx = Context.create()
        child = ctx.child("mod.a")
        logger = ContextLogger.from_context(child, name="mylog", output=buf)
        logger.info("test msg", extra={"key": "val"})
        data = json.loads(buf.getvalue())
        assert "timestamp" in data
        assert data["level"] == "info"
        assert data["message"] == "test msg"
        assert data["trace_id"] == ctx.trace_id
        assert data["module_id"] == "mod.a"
        assert data["logger"] == "mylog"
        assert data["extra"] == {"key": "val"}

    def test_non_serializable_extras(self):
        """Non-serializable extras handled gracefully (no crash)."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", output=buf)

        class Custom:
            def __str__(self):
                return "custom-obj"

        logger.info("test", extra={"obj": Custom()})
        data = json.loads(buf.getvalue())
        assert data["extra"]["obj"] == "custom-obj"


# --- Text Format Tests ---


class TestContextLoggerTextFormat:
    """Test text format output."""

    def test_text_format_pattern(self):
        """Text format output matches expected pattern."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", output_format="text", output=buf)
        logger._trace_id = "trace-abc"
        logger._module_id = "mod.a"
        logger.info("hello world", extra={"key": "val"})
        line = buf.getvalue().strip()
        assert "[INFO]" in line
        assert "[trace=trace-abc]" in line
        assert "[module=mod.a]" in line
        assert "hello world" in line
        assert "key=val" in line


# --- Redaction Tests ---


class TestContextLoggerRedaction:
    """Test sensitive field redaction."""

    def test_redact_secret_prefix_keys(self):
        """redact_sensitive=True redacts _secret_ prefixed keys in extra."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", redact_sensitive=True, output=buf)
        logger.info("test", extra={"_secret_key": "my-secret", "normal": "visible"})
        data = json.loads(buf.getvalue())
        assert data["extra"]["_secret_key"] == "***REDACTED***"
        assert data["extra"]["normal"] == "visible"

    def test_no_redaction_when_disabled(self):
        """redact_sensitive=False does not redact."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", redact_sensitive=False, output=buf)
        logger.info("test", extra={"_secret_key": "my-secret"})
        data = json.loads(buf.getvalue())
        assert data["extra"]["_secret_key"] == "my-secret"


# --- Custom Output Tests ---


class TestContextLoggerOutput:
    """Test custom output targets."""

    def test_custom_output_target(self):
        """Custom output target receives log entries."""
        buf = io.StringIO()
        logger = ContextLogger(name="test", output=buf)
        logger.info("hello")
        assert buf.getvalue().strip()


# --- ObsLoggingMiddleware Tests ---


class TestObsLoggingMiddleware:
    """Test ObsLoggingMiddleware lifecycle and stack-based timing."""

    def test_is_middleware_subclass(self):
        """ObsLoggingMiddleware is a Middleware subclass."""
        assert issubclass(ObsLoggingMiddleware, Middleware)

    def test_before_pushes_start_and_logs(self):
        """before() pushes start time to stack and logs 'Module call started'."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger)
        ctx = Context.create()
        mw.before("mod.a", {"x": 1}, ctx)
        assert len(ctx.data["_apcore.mw.logging.starts"]) == 1
        data = json.loads(buf.getvalue())
        assert "Module call started" in data["message"]

    def test_after_pops_and_logs_completion(self):
        """after() pops start time, logs 'Module call completed' with duration."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        buf.truncate(0)
        buf.seek(0)
        mw.after("mod.a", {}, {"result": "ok"}, ctx)
        assert len(ctx.data["_apcore.mw.logging.starts"]) == 0
        data = json.loads(buf.getvalue())
        assert "Module call completed" in data["message"]
        assert "duration_ms" in data["extra"]

    def test_on_error_pops_and_logs_failure(self):
        """on_error() pops start time, logs 'Module call failed', returns None."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        buf.truncate(0)
        buf.seek(0)
        result = mw.on_error("mod.a", {}, RuntimeError("fail"), ctx)
        assert result is None
        assert len(ctx.data["_apcore.mw.logging.starts"]) == 0
        data = json.loads(buf.getvalue())
        assert "Module call failed" in data["message"]
        assert data["extra"]["error_type"] == "RuntimeError"

    def test_log_inputs_true(self):
        """log_inputs=True includes inputs in before log."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_inputs=True)
        ctx = Context.create()
        ctx.redacted_inputs = {
            "name": "Alice"
        }  # D-131: the middleware logs the captured value, as the pipeline's capture point sets it
        mw.before("mod.a", {"name": "Alice"}, ctx)
        data = json.loads(buf.getvalue())
        assert "inputs" in data["extra"]

    def test_log_inputs_false(self):
        """log_inputs=False omits inputs."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_inputs=False)
        ctx = Context.create()
        mw.before("mod.a", {"name": "Alice"}, ctx)
        data = json.loads(buf.getvalue())
        assert "inputs" not in data["extra"]

    def test_log_outputs_true(self):
        """log_outputs=True includes output in after log."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_outputs=True)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        buf.truncate(0)
        buf.seek(0)
        ctx.redacted_output = {
            "message": "hello"
        }  # D-131: the middleware logs the captured value, as the pipeline's capture point sets it
        mw.after("mod.a", {}, {"message": "hello"}, ctx)
        data = json.loads(buf.getvalue())
        assert "output" in data["extra"]

    def test_log_outputs_false(self):
        """log_outputs=False omits output."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_outputs=False)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        buf.truncate(0)
        buf.seek(0)
        mw.after("mod.a", {}, {"message": "hello"}, ctx)
        data = json.loads(buf.getvalue())
        assert "output" not in data["extra"]

    def test_stack_based_nested_calls(self):
        """Stack-based timing works for nested calls."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.before("mod.b", {}, ctx)
        assert len(ctx.data["_apcore.mw.logging.starts"]) == 2
        mw.after("mod.b", {}, {"r": 1}, ctx)
        assert len(ctx.data["_apcore.mw.logging.starts"]) == 1
        mw.after("mod.a", {}, {"r": 2}, ctx)
        assert len(ctx.data["_apcore.mw.logging.starts"]) == 0
        # Should have 4 log entries (2 before + 2 after)
        lines = [line for line in buf.getvalue().strip().split("\n") if line]
        assert len(lines) == 4

    def test_auto_creates_logger(self):
        """Auto-creates ContextLogger when logger=None."""
        mw = ObsLoggingMiddleware(logger=None)
        assert mw._logger is not None
        assert isinstance(mw._logger, ContextLogger)

    def test_after_without_before_returns_none(self):
        """after() returns None without crashing when before() was never called."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger)
        ctx = Context.create()
        result = mw.after("mod.a", {}, {"r": 1}, ctx)
        assert result is None

    def test_on_error_without_before_returns_none(self):
        """on_error() returns None without crashing when before() was never called."""
        buf = io.StringIO()
        logger = ContextLogger(name="obs", output=buf)
        mw = ObsLoggingMiddleware(logger=logger)
        ctx = Context.create()
        result = mw.on_error("mod.a", {}, RuntimeError("fail"), ctx)
        assert result is None
