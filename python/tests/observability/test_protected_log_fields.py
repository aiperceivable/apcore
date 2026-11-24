"""Regression test for OBS-009: protected correlation-ID log fields.

User-supplied :class:`RedactionConfig` patterns must not redact the
correlation identifiers ``trace_id``, ``caller_id`` or ``module_id`` —
losing those values breaks log/trace stitching.  The
``PROTECTED_LOG_FIELDS`` constant pins these names so a pattern such as
``*_id`` cannot accidentally scramble them.  Mirrors the TS
``PROTECTED_LOG_FIELDS`` and Rust ``NEVER_REDACT_FIELDS`` sets.
"""

from __future__ import annotations

import io
import json

from apcore.context import Context
from apcore.observability.context_logger import (
    PROTECTED_LOG_FIELDS,
    ContextLogger,
    ObsLoggingMiddleware,
    RedactionConfig,
)


class TestProtectedLogFields:
    """trace_id / caller_id / module_id are never user-redactable."""

    def test_protected_log_fields_constant_present(self) -> None:
        assert "trace_id" in PROTECTED_LOG_FIELDS
        assert "span_id" in PROTECTED_LOG_FIELDS
        assert "caller_id" in PROTECTED_LOG_FIELDS
        assert "module_id" in PROTECTED_LOG_FIELDS
        assert "target_id" in PROTECTED_LOG_FIELDS

    def test_user_pattern_does_not_redact_span_or_target_id(self) -> None:
        """A field_pattern of ``*id*`` leaves span_id/target_id intact."""
        config = RedactionConfig(
            field_patterns=["*id*"],
            value_patterns=[],
            replacement="***REDACTED***",
        )
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_inputs=True, redaction_config=config)
        ctx = Context.create()
        ctx.call_chain.append("executor.x")
        ctx.redacted_inputs = {
            "span_id": "s1",
            "target_id": "t1",
            "user_id": "xyz",
        }  # D-131: the middleware logs the captured value, as the pipeline's capture point sets it
        mw.before("executor.x", dict(ctx.redacted_inputs), ctx)
        entry = json.loads(buf.getvalue().strip())
        inputs = entry["extra"]["inputs"]
        # non-protected id field IS redacted
        assert inputs["user_id"] == "***REDACTED***"
        # span_id / target_id are protected and stay intact
        assert inputs["span_id"] == "s1"
        assert inputs["target_id"] == "t1"

    def test_user_pattern_does_not_redact_trace_id(self) -> None:
        """A field_pattern of ``*_id`` redacts user_id but NOT trace_id."""
        config = RedactionConfig(
            field_patterns=["*_id"],
            value_patterns=[],
            replacement="***REDACTED***",
        )
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_inputs=True, redaction_config=config)
        ctx = Context.create()
        ctx.call_chain.append("executor.x")
        ctx.redacted_inputs = {
            "trace_id": "abc",
            "user_id": "xyz",
            "caller_id": "c",
            "module_id": "m",
        }  # D-131: the middleware logs the captured value, as the pipeline's capture point sets it
        mw.before("executor.x", dict(ctx.redacted_inputs), ctx)
        entry = json.loads(buf.getvalue().strip())
        inputs = entry["extra"]["inputs"]
        # user_id IS redacted (matches *_id and not protected)
        assert inputs["user_id"] == "***REDACTED***"
        # protected fields stay intact
        assert inputs["trace_id"] == "abc"
        assert inputs["caller_id"] == "c"
        assert inputs["module_id"] == "m"

    def test_protected_fields_intact_with_explicit_match(self) -> None:
        """Even an exact-name pattern should not redact a protected field."""
        config = RedactionConfig(
            field_patterns=["trace_id"],
            value_patterns=[],
        )
        buf = io.StringIO()
        logger = ContextLogger(name="t", output=buf)
        mw = ObsLoggingMiddleware(logger=logger, log_inputs=True, redaction_config=config)
        ctx = Context.create()
        ctx.redacted_inputs = {
            "trace_id": "kept",
            "other": "v",
        }  # D-131: the middleware logs the captured value, as the pipeline's capture point sets it
        mw.before("mod.a", dict(ctx.redacted_inputs), ctx)
        entry = json.loads(buf.getvalue().strip())
        inputs = entry["extra"]["inputs"]
        assert inputs["trace_id"] == "kept"
