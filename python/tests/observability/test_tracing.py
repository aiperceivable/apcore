"""Tests for Span dataclass, SpanExporter implementations, and TracingMiddleware."""

from __future__ import annotations

import json
import logging
import sys
import threading
import time
from types import SimpleNamespace
from unittest.mock import MagicMock, patch

import pytest

from apcore.context import Context
from apcore.observability.tracing import (
    InMemoryExporter,
    OTLPExporter,
    Span,
    SpanExporter,
    StdoutExporter,
    TracingMiddleware,
)


class TestSpan:
    """Tests for the Span dataclass."""

    def test_span_created_with_required_fields(self):
        """Span can be created with trace_id, name, and start_time."""
        span = Span(trace_id="abc-123", name="apcore.module.execute", start_time=time.time())
        assert span.trace_id == "abc-123"
        assert span.name == "apcore.module.execute"
        assert isinstance(span.start_time, float)

    def test_span_id_is_16_char_hex(self):
        """span_id should be a 16-character hexadecimal string."""
        span = Span(trace_id="abc-123", name="test", start_time=time.time())
        assert len(span.span_id) == 16
        assert all(c in "0123456789abcdef" for c in span.span_id)

    def test_end_time_defaults_to_none(self):
        """end_time should default to None when not explicitly set."""
        span = Span(trace_id="abc-123", name="test", start_time=time.time())
        assert span.end_time is None

    def test_status_defaults_to_ok(self):
        """status should default to 'ok'."""
        span = Span(trace_id="abc-123", name="test", start_time=time.time())
        assert span.status == "ok"

    def test_attributes_defaults_to_empty_dict(self):
        """attributes should default to an empty dict."""
        span = Span(trace_id="abc-123", name="test", start_time=time.time())
        assert span.attributes == {}
        assert isinstance(span.attributes, dict)

    def test_events_defaults_to_empty_list(self):
        """events should default to an empty list."""
        span = Span(trace_id="abc-123", name="test", start_time=time.time())
        assert span.events == []
        assert isinstance(span.events, list)

    def test_parent_span_id_defaults_to_none(self):
        """parent_span_id should default to None when not explicitly set."""
        span = Span(trace_id="abc-123", name="test", start_time=time.time())
        assert span.parent_span_id is None

    def test_span_is_mutable(self):
        """Span is NOT frozen -- end_time and status can be set after creation."""
        span = Span(trace_id="abc-123", name="test", start_time=time.time())
        span.end_time = time.time()
        span.status = "error"
        assert span.end_time is not None
        assert span.status == "error"


class TestStdoutExporter:
    """Tests for StdoutExporter."""

    def test_export_writes_json_line_to_stdout(self, capsys):
        """StdoutExporter.export() writes a single JSON line to stdout."""
        exporter = StdoutExporter()
        span = Span(trace_id="trace-1", name="test.span", start_time=1000.0)
        span.end_time = 1001.0
        exporter.export(span)
        captured = capsys.readouterr()
        data = json.loads(captured.out.strip())
        assert isinstance(data, dict)

    def test_export_json_includes_required_fields(self, capsys):
        """Exported JSON includes trace_id, span_id, name, attributes, and timing info."""
        exporter = StdoutExporter()
        span = Span(
            trace_id="trace-1",
            name="test.span",
            start_time=1000.0,
            attributes={"module_id": "greet"},
        )
        span.end_time = 1001.0
        exporter.export(span)
        captured = capsys.readouterr()
        data = json.loads(captured.out.strip())
        assert data["trace_id"] == "trace-1"
        assert "span_id" in data
        assert data["name"] == "test.span"
        assert data["attributes"] == {"module_id": "greet"}
        assert data["start_time"] == 1000.0
        assert data["end_time"] == 1001.0


class TestInMemoryExporter:
    """Tests for InMemoryExporter."""

    def test_export_adds_span_to_internal_list(self):
        """InMemoryExporter.export() stores the span internally."""
        exporter = InMemoryExporter()
        span = Span(trace_id="trace-1", name="test", start_time=time.time())
        exporter.export(span)
        assert len(exporter.get_spans()) == 1

    def test_get_spans_returns_all_collected(self):
        """get_spans() returns all spans exported so far."""
        exporter = InMemoryExporter()
        for i in range(3):
            span = Span(trace_id=f"trace-{i}", name="test", start_time=time.time())
            exporter.export(span)
        spans = exporter.get_spans()
        assert len(spans) == 3
        assert [s.trace_id for s in spans] == ["trace-0", "trace-1", "trace-2"]

    def test_clear_removes_all_spans(self):
        """clear() empties the internal span list."""
        exporter = InMemoryExporter()
        exporter.export(Span(trace_id="t1", name="test", start_time=time.time()))
        exporter.export(Span(trace_id="t2", name="test", start_time=time.time()))
        assert len(exporter.get_spans()) == 2
        exporter.clear()
        assert len(exporter.get_spans()) == 0


class TestSpanExporterProtocol:
    """Tests for SpanExporter runtime_checkable protocol."""

    def test_stdout_exporter_is_span_exporter(self):
        """StdoutExporter satisfies the SpanExporter protocol."""
        assert isinstance(StdoutExporter(), SpanExporter)

    def test_in_memory_exporter_is_span_exporter(self):
        """InMemoryExporter satisfies the SpanExporter protocol."""
        assert isinstance(InMemoryExporter(), SpanExporter)


class TestOTLPExporter:
    """Tests for OTLPExporter."""

    def test_raises_import_error_when_opentelemetry_not_installed(self):
        """OTLPExporter should raise ImportError at instantiation when opentelemetry is absent.

        Every submodule the constructor imports is blanked, not just the parent
        package. `from opentelemetry.sdk.trace import ...` resolves straight out
        of ``sys.modules`` when the submodule is already cached there, so
        patching only ``"opentelemetry"`` simulates absence exactly as long as
        nothing else in the suite has ever imported the real thing — and this
        case went red the day a test constructed a real ``OTLPExporter``.
        """
        absent = dict.fromkeys(
            [
                "opentelemetry",
                "opentelemetry.exporter.otlp.proto.http.trace_exporter",
                "opentelemetry.sdk.resources",
                "opentelemetry.sdk.trace",
                "opentelemetry.sdk.trace.export",
                "opentelemetry.trace",
            ]
        )
        with patch.dict(sys.modules, absent):
            with pytest.raises(ImportError, match="opentelemetry"):
                OTLPExporter()

    @staticmethod
    def _capture_exporter(allowlist=None):
        exporter = object.__new__(OTLPExporter)
        exporter._processor = MagicMock()
        exporter._attribute_allowlist = allowlist
        exporter._SpanContext = lambda trace_id, span_id, remote, flags: SimpleNamespace(
            trace_id=trace_id, span_id=span_id, is_remote=remote, flags=flags
        )
        exporter._ReadableSpan = SimpleNamespace
        exporter._Event = lambda name, attributes: SimpleNamespace(name=name, attributes=attributes)
        exporter._Status = lambda code: SimpleNamespace(code=code)
        exporter._StatusCode = SimpleNamespace(ERROR="error", UNSET="unset")
        exporter._sampled_trace_flags = 1
        exporter._resource = object()
        exporter._instrumentation_scope = object()
        return exporter

    def test_export_constructs_span_with_matching_data(self):
        exporter = self._capture_exporter()
        span = Span(
            trace_id="1" * 32,
            span_id="2" * 16,
            parent_span_id="3" * 16,
            name="apcore.module.execute",
            start_time=1000.0,
            end_time=1001.5,
            attributes={"module_id": "greet", "success": True, "complex": ["a", "b"]},
            events=[{"name": "exception", "type": "ValueError", "args": ["bad input"]}],
        )

        exporter.export(span)

        exporter._processor.on_end.assert_called_once()
        converted = exporter._processor.on_end.call_args.args[0]
        assert converted.name == span.name
        assert converted.context.trace_id == int(span.trace_id, 16)
        assert converted.context.span_id == int(span.span_id, 16)
        assert converted.parent.trace_id == converted.context.trace_id
        assert converted.parent.span_id == int(span.parent_span_id, 16)
        assert converted.start_time == 1000_000_000_000
        assert converted.end_time == 1001_500_000_000
        assert converted.status.code == "unset"
        assert converted.attributes == {
            "module_id": "greet",
            "success": True,
            "complex": "['a', 'b']",
            "apcore.trace_id": span.trace_id,
            "apcore.span_id": span.span_id,
            "apcore.parent_span_id": span.parent_span_id,
        }
        assert converted.events == [
            SimpleNamespace(name="exception", attributes={"type": "ValueError", "args": "['bad input']"})
        ]

    def test_export_without_parent_or_end_time(self, monkeypatch):
        exporter = self._capture_exporter()
        monkeypatch.setattr(time, "time_ns", lambda: 1001_000_000_000)

        exporter.export(Span(trace_id="user-defined", name="apcore.module.execute", start_time=1000.0, status="error"))

        converted = exporter._processor.on_end.call_args.args[0]
        assert converted.context.trace_id > 0
        assert converted.attributes["apcore.trace_id"] == "user-defined"
        assert converted.parent is None
        assert "apcore.parent_span_id" not in converted.attributes
        assert converted.end_time == 1001_000_000_000
        assert converted.status.code == "error"

    def test_shutdown_calls_processor_shutdown(self):
        exporter = self._capture_exporter()
        exporter.shutdown()
        exporter._processor.shutdown.assert_called_once()

    def test_attribute_allowlist_drops_non_allowed_keys(self):
        exporter = self._capture_exporter(frozenset({"module_id"}))
        span = Span(
            trace_id="1" * 32,
            span_id="2" * 16,
            name="apcore.module.execute",
            start_time=1.0,
            end_time=2.0,
            attributes={"module_id": "greet", "user_email": "example@example.test"},
        )

        exporter.export(span)

        converted = exporter._processor.on_end.call_args.args[0]
        assert converted.attributes == {
            "module_id": "greet",
            "apcore.trace_id": span.trace_id,
            "apcore.span_id": span.span_id,
        }


# --- Section 02: TracingMiddleware Tests ---


class TestSampling:
    """Tests for TracingMiddleware sampling strategies."""

    def test_sampling_rate_1_always_samples(self):
        """sampling_rate=1.0 always samples."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter, sampling_rate=1.0, sampling_strategy="proportional")
        ctx = Context.create()
        mw.before("mod.a", {"x": 1}, ctx)
        mw.after("mod.a", {"x": 1}, {"result": "ok"}, ctx)
        assert len(exporter.get_spans()) == 1

    def test_sampling_rate_0_never_samples_but_creates_span(self):
        """sampling_rate=0.0 never samples (but still creates span)."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter, sampling_rate=0.0, sampling_strategy="proportional")
        ctx = Context.create()
        mw.before("mod.a", {"x": 1}, ctx)
        # Span was created on the stack
        assert len(ctx.data["_apcore.mw.tracing.spans"]) == 1
        mw.after("mod.a", {"x": 1}, {"result": "ok"}, ctx)
        # But not exported
        assert len(exporter.get_spans()) == 0

    def test_sampling_rate_rejects_negative(self):
        """sampling_rate validation rejects negative values."""
        with pytest.raises(ValueError):
            TracingMiddleware(exporter=InMemoryExporter(), sampling_rate=-0.1)

    def test_sampling_rate_rejects_above_1(self):
        """sampling_rate validation rejects values > 1.0."""
        with pytest.raises(ValueError):
            TracingMiddleware(exporter=InMemoryExporter(), sampling_rate=1.5)

    def test_invalid_strategy_rejects(self):
        """Invalid sampling_strategy value raises ValueError."""
        with pytest.raises(ValueError):
            TracingMiddleware(exporter=InMemoryExporter(), sampling_strategy="bogus")

    def test_full_strategy_always_exports(self):
        """'full' strategy always exports."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter, sampling_rate=0.0, sampling_strategy="full")
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.after("mod.a", {}, {"r": 1}, ctx)
        assert len(exporter.get_spans()) == 1

    def test_off_strategy_never_exports(self):
        """'off' strategy never exports."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter, sampling_rate=1.0, sampling_strategy="off")
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.after("mod.a", {}, {"r": 1}, ctx)
        assert len(exporter.get_spans()) == 0

    def test_proportional_strategy_exports_proportionally(self):
        """'proportional' strategy exports proportionally (statistical test)."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter, sampling_rate=0.5, sampling_strategy="proportional")
        for _ in range(1000):
            ctx = Context.create()
            mw.before("mod.a", {}, ctx)
            mw.after("mod.a", {}, {"r": 1}, ctx)
        count = len(exporter.get_spans())
        assert 350 <= count <= 650, f"Expected ~500, got {count}"

    def test_error_first_always_exports_errors(self):
        """'error_first' strategy always exports errors regardless of rate."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter, sampling_rate=0.0, sampling_strategy="error_first")
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.on_error("mod.a", {}, RuntimeError("fail"), ctx)
        assert len(exporter.get_spans()) == 1

    def test_error_first_uses_proportional_for_successes(self):
        """'error_first' strategy uses proportional for successes."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter, sampling_rate=0.0, sampling_strategy="error_first")
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.after("mod.a", {}, {"r": 1}, ctx)
        assert len(exporter.get_spans()) == 0

    def test_sampling_decision_inherited_from_parent(self):
        """Sampling decision inherited from parent context (nested calls)."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter, sampling_rate=0.0, sampling_strategy="proportional")
        ctx = Context.create()
        # Pre-set the sampling decision as if parent decided to sample
        ctx.data["_apcore.mw.tracing.sampled"] = True
        mw.before("mod.a", {}, ctx)
        mw.after("mod.a", {}, {"r": 1}, ctx)
        # Should export because parent decided to sample
        assert len(exporter.get_spans()) == 1


class TestTracingMiddleware:
    """Tests for TracingMiddleware before/after/on_error lifecycle."""

    def test_before_creates_span_and_pushes_to_stack(self):
        """before() creates span and pushes to context.data stack."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        mw.before("mod.a", {"x": 1}, ctx)
        spans = ctx.data["_apcore.mw.tracing.spans"]
        assert len(spans) == 1
        assert spans[0].trace_id == ctx.trace_id
        assert spans[0].name == "apcore.module.execute"

    def test_after_pops_span_sets_ok_and_exports(self):
        """after() pops span, sets end_time and status='ok', exports if sampled."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.after("mod.a", {}, {"result": "ok"}, ctx)
        # Stack should be empty
        assert len(ctx.data["_apcore.mw.tracing.spans"]) == 0
        # Exported span
        spans = exporter.get_spans()
        assert len(spans) == 1
        assert spans[0].status == "ok"
        assert spans[0].end_time is not None
        assert spans[0].attributes["success"] is True

    def test_on_error_pops_span_sets_error_returns_none(self):
        """on_error() pops span, sets status='error' and error_code, returns None."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        result = mw.on_error("mod.a", {}, RuntimeError("fail"), ctx)
        assert result is None
        assert len(ctx.data["_apcore.mw.tracing.spans"]) == 0
        spans = exporter.get_spans()
        assert len(spans) == 1
        assert spans[0].status == "error"
        assert spans[0].attributes["error_code"] == "RuntimeError"

    def test_stack_based_storage_nested_calls(self):
        """Middleware uses stack-based storage (nested calls don't overwrite)."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.before("mod.b", {}, ctx)
        assert len(ctx.data["_apcore.mw.tracing.spans"]) == 2
        mw.after("mod.b", {}, {"r": 1}, ctx)
        assert len(ctx.data["_apcore.mw.tracing.spans"]) == 1
        mw.after("mod.a", {}, {"r": 2}, ctx)
        assert len(ctx.data["_apcore.mw.tracing.spans"]) == 0
        assert len(exporter.get_spans()) == 2

    def test_span_name_convention(self):
        """Span name follows 'apcore.module.execute' convention."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        span = ctx.data["_apcore.mw.tracing.spans"][0]
        assert span.name == "apcore.module.execute"

    def test_span_attributes_include_module_id_method_caller_id(self):
        """Span attributes include module_id, method, caller_id."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        ctx.caller_id = "test.caller"
        mw.before("mod.a", {}, ctx)
        span = ctx.data["_apcore.mw.tracing.spans"][0]
        assert span.attributes["module_id"] == "mod.a"
        assert span.attributes["method"] == "execute"
        assert span.attributes["caller_id"] == "test.caller"

    def test_duration_ms_computed(self):
        """duration_ms computed correctly."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        time.sleep(0.01)  # Sleep 10ms
        mw.after("mod.a", {}, {"r": 1}, ctx)
        span = exporter.get_spans()[0]
        assert span.attributes["duration_ms"] > 0

    def test_parent_span_id_set_in_nested_calls(self):
        """parent_span_id set from parent span in nested calls."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        parent_span_id = ctx.data["_apcore.mw.tracing.spans"][0].span_id
        mw.before("mod.b", {}, ctx)
        child_span = ctx.data["_apcore.mw.tracing.spans"][1]
        assert child_span.parent_span_id == parent_span_id

    def test_end_to_end_with_in_memory_exporter(self):
        """Works with InMemoryExporter (end-to-end)."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        ctx.caller_id = "system"
        mw.before("greet", {"name": "Alice"}, ctx)
        mw.after("greet", {"name": "Alice"}, {"message": "Hello"}, ctx)
        spans = exporter.get_spans()
        assert len(spans) == 1
        span = spans[0]
        assert span.trace_id == ctx.trace_id
        assert span.name == "apcore.module.execute"
        assert span.status == "ok"
        assert span.end_time is not None
        assert span.attributes["module_id"] == "greet"
        assert span.attributes["duration_ms"] >= 0


class TestInMemoryExporterThreadSafety:
    """Tests for InMemoryExporter thread safety and bounded size."""

    def test_concurrent_export_no_error(self) -> None:
        """Concurrent export() calls should not raise."""
        exporter = InMemoryExporter()
        errors: list[Exception] = []

        def exporter_worker() -> None:
            try:
                for i in range(100):
                    span = Span(trace_id=f"t-{i}", name="test", start_time=time.time())
                    exporter.export(span)
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=exporter_worker) for _ in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == []
        assert len(exporter.get_spans()) == 1000

    def test_bounded_max_spans(self) -> None:
        """InMemoryExporter with max_spans drops oldest spans when full."""
        exporter = InMemoryExporter(max_spans=5)
        for i in range(10):
            exporter.export(Span(trace_id=f"t-{i}", name="test", start_time=float(i)))
        spans = exporter.get_spans()
        assert len(spans) == 5
        # Oldest should be dropped; newest remain
        assert spans[0].trace_id == "t-5"
        assert spans[-1].trace_id == "t-9"

    def test_default_max_spans(self) -> None:
        """Default max_spans is 10_000."""
        exporter = InMemoryExporter()
        # Check the internal deque maxlen
        assert exporter._spans.maxlen == 10_000


class TestTracingMiddlewareEmptyStackGuard:
    """Tests for empty stack guard in after() and on_error()."""

    def test_after_with_empty_stack_logs_warning(self, caplog: pytest.LogCaptureFixture) -> None:
        """after() with empty span stack should log warning and return None."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        # Don't call before(), so stack is empty
        with caplog.at_level(logging.WARNING):
            result = mw.after("mod.a", {}, {"r": 1}, ctx)
        assert result is None
        assert "empty span stack" in caplog.text
        assert len(exporter.get_spans()) == 0

    def test_on_error_with_empty_stack_logs_warning(self, caplog: pytest.LogCaptureFixture) -> None:
        """on_error() with empty span stack should log warning and return None."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        # Don't call before(), so stack is empty
        with caplog.at_level(logging.WARNING):
            result = mw.on_error("mod.a", {}, RuntimeError("fail"), ctx)
        assert result is None
        assert "empty span stack" in caplog.text
        assert len(exporter.get_spans()) == 0

    def test_after_with_no_tracing_spans_key(self, caplog: pytest.LogCaptureFixture) -> None:
        """after() with no _apcore.mw.tracing.spans key should log warning and return None."""
        exporter = InMemoryExporter()
        mw = TracingMiddleware(exporter=exporter)
        ctx = Context.create()
        # Ensure no _apcore.mw.tracing.spans key exists
        assert "_apcore.mw.tracing.spans" not in ctx.data
        with caplog.at_level(logging.WARNING):
            result = mw.after("mod.a", {}, {"r": 1}, ctx)
        assert result is None
