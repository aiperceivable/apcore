"""Tests for MetricsCollector and MetricsMiddleware."""

from __future__ import annotations

import threading


from apcore.context import Context
from apcore.errors import ModuleError
from apcore.observability.metrics import MetricsCollector, MetricsMiddleware


# --- MetricsCollector Increment Tests ---


class TestMetricsCollectorIncrement:
    """Tests for MetricsCollector.increment()."""

    def test_increment_creates_counter(self):
        """Calling increment() on a new metric name creates a counter."""
        c = MetricsCollector()
        c.increment("my_counter", {"env": "test"})
        snap = c.snapshot()
        key = ("my_counter", (("env", "test"),))
        assert snap["counters"][key] == 1

    def test_increment_same_labels_accumulates(self):
        """Multiple increment() calls with identical name+labels add up."""
        c = MetricsCollector()
        c.increment("my_counter", {"env": "test"})
        c.increment("my_counter", {"env": "test"})
        c.increment("my_counter", {"env": "test"}, amount=3)
        snap = c.snapshot()
        key = ("my_counter", (("env", "test"),))
        assert snap["counters"][key] == 5

    def test_increment_different_labels_separate(self):
        """Different label sets produce independent counters."""
        c = MetricsCollector()
        c.increment("calls", {"module": "a"})
        c.increment("calls", {"module": "b"})
        c.increment("calls", {"module": "a"})
        snap = c.snapshot()
        assert snap["counters"][("calls", (("module", "a"),))] == 2
        assert snap["counters"][("calls", (("module", "b"),))] == 1


# --- MetricsCollector Observe Tests ---


class TestMetricsCollectorObserve:
    """Tests for MetricsCollector.observe() histogram recording."""

    def test_observe_records_value(self):
        """observe() increments _sum, _count, and appropriate bucket entries."""
        c = MetricsCollector(buckets=[1.0, 5.0, 10.0])
        c.observe("duration", {"mod": "a"}, 3.0)
        snap = c.snapshot()
        key = ("duration", (("mod", "a"),))
        assert snap["histograms"]["sums"][key] == 3.0
        assert snap["histograms"]["counts"][key] == 1

    def test_observe_increments_correct_buckets(self):
        """Only buckets where le >= observed value are incremented."""
        c = MetricsCollector(buckets=[1.0, 5.0, 10.0])
        c.observe("duration", {"mod": "a"}, 3.0)
        snap = c.snapshot()
        lk = (("mod", "a"),)
        # 3.0 > 1.0, so bucket 1.0 NOT incremented
        assert snap["histograms"]["buckets"].get(("duration", lk, 1.0), 0) == 0
        # 3.0 <= 5.0, so bucket 5.0 incremented
        assert snap["histograms"]["buckets"][("duration", lk, 5.0)] == 1
        # 3.0 <= 10.0, so bucket 10.0 incremented
        assert snap["histograms"]["buckets"][("duration", lk, 10.0)] == 1

    def test_observe_always_increments_inf_bucket(self):
        """Every observation increments the +Inf bucket."""
        c = MetricsCollector(buckets=[1.0, 5.0])
        c.observe("duration", {"mod": "a"}, 100.0)
        snap = c.snapshot()
        lk = (("mod", "a"),)
        assert snap["histograms"]["buckets"][("duration", lk, float("inf"))] == 1


# --- Snapshot and Reset ---


class TestMetricsCollectorSnapshotAndReset:
    """Tests for snapshot() and reset()."""

    def test_snapshot_returns_dict(self):
        """snapshot() returns a plain dict reflecting current metric state."""
        c = MetricsCollector()
        c.increment("counter_a", {"x": "1"})
        snap = c.snapshot()
        assert isinstance(snap, dict)
        assert "counters" in snap
        assert "histograms" in snap

    def test_reset_clears_all(self):
        """After reset(), snapshot() returns empty state."""
        c = MetricsCollector()
        c.increment("counter_a", {"x": "1"})
        c.observe("hist_a", {"x": "1"}, 1.0)
        c.reset()
        snap = c.snapshot()
        assert snap["counters"] == {}
        assert snap["histograms"]["sums"] == {}


# --- Prometheus Export ---


class TestMetricsCollectorPrometheus:
    """Tests for export_prometheus() text format."""

    def test_export_prometheus_format(self):
        """Output string follows Prometheus text exposition conventions."""
        c = MetricsCollector()
        c.increment("apcore_module_calls_total", {"module_id": "greet", "status": "success"}, 10)
        output = c.export_prometheus()
        assert 'apcore_module_calls_total{module_id="greet",status="success"} 10' in output

    def test_export_prometheus_type_help(self):
        """Each metric family starts with # HELP and # TYPE lines."""
        c = MetricsCollector()
        c.increment("apcore_module_calls_total", {"module_id": "greet", "status": "success"})
        output = c.export_prometheus()
        assert "# HELP apcore_module_calls_total Total module calls" in output
        assert "# TYPE apcore_module_calls_total counter" in output

    def test_export_prometheus_inf_bucket(self):
        """Histogram output includes a le='+Inf' bucket line."""
        c = MetricsCollector(buckets=[1.0])
        c.observe("apcore_module_duration_seconds", {"module_id": "greet"}, 0.5)
        output = c.export_prometheus()
        assert 'le="+Inf"' in output

    def test_export_prometheus_sum_count(self):
        """Histogram output includes _sum and _count suffixed lines."""
        c = MetricsCollector(buckets=[1.0])
        c.observe("apcore_module_duration_seconds", {"module_id": "greet"}, 0.5)
        output = c.export_prometheus()
        assert "apcore_module_duration_seconds_sum" in output
        assert "apcore_module_duration_seconds_count" in output


# --- Configuration ---


class TestMetricsCollectorConfiguration:
    """Tests for constructor configuration."""

    def test_custom_buckets(self):
        """Passing buckets=[1.0, 5.0, 10.0] uses only those thresholds."""
        c = MetricsCollector(buckets=[1.0, 5.0, 10.0])
        c.observe("test", {"x": "1"}, 3.0)
        snap = c.snapshot()
        lk = (("x", "1"),)
        # Only 3 bucket entries + inf
        bucket_keys = [k for k in snap["histograms"]["buckets"] if k[0] == "test"]
        assert len(bucket_keys) == 3  # 5.0, 10.0, +Inf (not 1.0 since 3.0 > 1.0... wait, keys always exist)
        # Actually the keys only exist if incremented. Let's check values.
        assert snap["histograms"]["buckets"].get(("test", lk, 1.0), 0) == 0
        assert snap["histograms"]["buckets"][("test", lk, 5.0)] == 1
        assert snap["histograms"]["buckets"][("test", lk, 10.0)] == 1
        assert snap["histograms"]["buckets"][("test", lk, float("inf"))] == 1


# --- Thread Safety ---


class TestMetricsCollectorThreadSafety:
    """Tests for thread safety."""

    def test_concurrent_increments(self):
        """Many threads incrementing the same counter yield the correct total."""
        c = MetricsCollector()
        threads = []
        for _ in range(100):
            t = threading.Thread(target=lambda: [c.increment("counter", {"t": "1"}) for _ in range(100)])
            threads.append(t)
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        snap = c.snapshot()
        assert snap["counters"][("counter", (("t", "1"),))] == 10000


# --- Convenience Methods ---


class TestMetricsCollectorConvenienceMethods:
    """Tests for convenience methods."""

    def test_increment_calls(self):
        """increment_calls maps to correct metric name and labels."""
        c = MetricsCollector()
        c.increment_calls("greet", "success")
        snap = c.snapshot()
        key = (
            "apcore_module_calls_total",
            (("module_id", "greet"), ("status", "success")),
        )
        assert snap["counters"][key] == 1

    def test_increment_errors(self):
        """increment_errors maps to correct metric name and labels."""
        c = MetricsCollector()
        c.increment_errors("greet", "RuntimeError")
        snap = c.snapshot()
        key = (
            "apcore_module_errors_total",
            (("error_code", "RuntimeError"), ("module_id", "greet")),
        )
        assert snap["counters"][key] == 1

    def test_observe_duration(self):
        """observe_duration maps to correct metric name and labels."""
        c = MetricsCollector()
        c.observe_duration("greet", 0.5)
        snap = c.snapshot()
        key = ("apcore_module_duration_seconds", (("module_id", "greet"),))
        assert snap["histograms"]["sums"][key] == 0.5
        assert snap["histograms"]["counts"][key] == 1


# --- MetricsMiddleware Tests ---


class TestMetricsMiddlewareBefore:
    """Tests for MetricsMiddleware.before()."""

    def test_before_pushes_start_time(self):
        """before() appends a float timestamp to context.data['_apcore.mw.metrics.starts']."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        assert len(ctx.data["_apcore.mw.metrics.starts"]) == 1
        assert isinstance(ctx.data["_apcore.mw.metrics.starts"][0], float)


class TestMetricsMiddlewareAfter:
    """Tests for MetricsMiddleware.after()."""

    def test_after_records_success(self):
        """after() pops start, calls increment_calls with 'success', and observe_duration."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.after("mod.a", {}, {"result": "ok"}, ctx)
        assert len(ctx.data["_apcore.mw.metrics.starts"]) == 0
        snap = c.snapshot()
        calls_key = (
            "apcore_module_calls_total",
            (("module_id", "mod.a"), ("status", "success")),
        )
        assert snap["counters"][calls_key] == 1
        dur_key = ("apcore_module_duration_seconds", (("module_id", "mod.a"),))
        assert snap["histograms"]["counts"][dur_key] == 1


class TestMetricsMiddlewareOnError:
    """Tests for MetricsMiddleware.on_error()."""

    def test_on_error_records_error(self):
        """on_error() pops start, records error call and error count."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.on_error("mod.a", {}, RuntimeError("fail"), ctx)
        snap = c.snapshot()
        calls_key = (
            "apcore_module_calls_total",
            (("module_id", "mod.a"), ("status", "error")),
        )
        assert snap["counters"][calls_key] == 1
        errors_key = (
            "apcore_module_errors_total",
            (("error_code", "RuntimeError"), ("module_id", "mod.a")),
        )
        assert snap["counters"][errors_key] == 1

    def test_on_error_module_error_code(self):
        """When error is a ModuleError, uses error.code as the error_code label."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        err = ModuleError(code="ACL_DENIED", message="denied")
        mw.on_error("mod.a", {}, err, ctx)
        snap = c.snapshot()
        errors_key = (
            "apcore_module_errors_total",
            (("error_code", "ACL_DENIED"), ("module_id", "mod.a")),
        )
        assert snap["counters"][errors_key] == 1

    def test_on_error_generic_exception_code(self):
        """When error is not a ModuleError, uses type(error).__name__ as error_code."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.on_error("mod.a", {}, ValueError("bad"), ctx)
        snap = c.snapshot()
        errors_key = (
            "apcore_module_errors_total",
            (("error_code", "ValueError"), ("module_id", "mod.a")),
        )
        assert snap["counters"][errors_key] == 1

    def test_on_error_returns_none(self):
        """on_error() always returns None."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        result = mw.on_error("mod.a", {}, RuntimeError("x"), ctx)
        assert result is None


class TestMetricsMiddlewareNested:
    """Tests for stack-based timing with nested calls."""

    def test_nested_calls_stack_isolation(self):
        """Nested before/after calls produce independent metrics."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        mw.before("mod.a", {}, ctx)
        mw.before("mod.b", {}, ctx)
        assert len(ctx.data["_apcore.mw.metrics.starts"]) == 2
        mw.after("mod.b", {}, {"r": 1}, ctx)
        assert len(ctx.data["_apcore.mw.metrics.starts"]) == 1
        mw.after("mod.a", {}, {"r": 2}, ctx)
        assert len(ctx.data["_apcore.mw.metrics.starts"]) == 0
        snap = c.snapshot()
        # Both modules should have success calls
        key_a = (
            "apcore_module_calls_total",
            (("module_id", "mod.a"), ("status", "success")),
        )
        key_b = (
            "apcore_module_calls_total",
            (("module_id", "mod.b"), ("status", "success")),
        )
        assert snap["counters"][key_a] == 1
        assert snap["counters"][key_b] == 1


class TestMetricsMiddlewareSkippedBefore:
    """Tests for MetricsMiddleware when before() was never called."""

    def test_after_without_before_returns_none(self):
        """after() returns None without crashing when before() was never called."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        result = mw.after("mod.a", {}, {"r": 1}, ctx)
        assert result is None
        assert c.snapshot()["counters"] == {}

    def test_on_error_without_before_returns_none(self):
        """on_error() returns None without crashing when before() was never called."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        result = mw.on_error("mod.a", {}, RuntimeError("fail"), ctx)
        assert result is None
        assert c.snapshot()["counters"] == {}


class TestMetricsMiddlewareIntegration:
    """Integration-style test using real MetricsCollector."""

    def test_end_to_end_with_collector(self):
        """Full before/after cycle; verify snapshot() shows expected data."""
        c = MetricsCollector()
        mw = MetricsMiddleware(c)
        ctx = Context.create()
        mw.before("greet", {"name": "Alice"}, ctx)
        mw.after("greet", {"name": "Alice"}, {"message": "Hello"}, ctx)
        snap = c.snapshot()
        calls_key = (
            "apcore_module_calls_total",
            (("module_id", "greet"), ("status", "success")),
        )
        assert snap["counters"][calls_key] == 1
        dur_key = ("apcore_module_duration_seconds", (("module_id", "greet"),))
        assert snap["histograms"]["sums"][dur_key] >= 0
        assert snap["histograms"]["counts"][dur_key] == 1
