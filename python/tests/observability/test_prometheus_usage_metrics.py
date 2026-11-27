"""Render metrics and usage as one Prometheus exposition."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone

import pytest

from apcore.observability import render_prometheus_metrics
from apcore.observability import usage as usage_module
from apcore.observability.metrics import MetricsCollector
from apcore.observability.usage import UsageCollector


@pytest.mark.parametrize("populated_metrics", [False, True])
@pytest.mark.parametrize("populated_usage", [None, False, True])
def test_render_prometheus_metrics_combines_collectors(populated_metrics: bool, populated_usage: bool | None) -> None:
    collector = MetricsCollector()
    if populated_metrics:
        collector.increment_calls("math.add", "success")
        collector.observe("apcore_module_duration_seconds", {"module_id": "math.add"}, 0.01)
    usage = UsageCollector() if populated_usage is not None else None
    if usage is not None and populated_usage:
        usage.record(module_id="math.add", caller_id="client", latency_ms=10.0, success=True)

    fragments = [collector.export_prometheus(), usage.export_prometheus() if usage is not None else ""]
    expected = "".join(fragments)
    assert render_prometheus_metrics(collector, usage) == expected
    assert render_prometheus_metrics(collector) == collector.export_prometheus()


def test_prometheus_usage_gauges_decrease_after_window_rolls(monkeypatch: pytest.MonkeyPatch) -> None:
    instant = datetime(2026, 2, 1, 10, tzinfo=timezone.utc)

    class Clock(datetime):
        @classmethod
        def now(cls, tz: timezone | None = None) -> datetime:
            return instant.astimezone(tz)

    monkeypatch.setattr(usage_module, "datetime", Clock)
    metrics = MetricsCollector()
    metrics.increment("demo_counter_total", {})
    usage = UsageCollector()
    usage.record("demo.one", "client", 12.0, True)
    before = render_prometheus_metrics(metrics, usage)
    instant += timedelta(hours=25)
    after = render_prometheus_metrics(metrics, usage)

    for family in ("apcore_usage_calls_24h", "apcore_usage_calls_total"):
        sample = f'{family}{{module_id="demo.one",status="success"}}'
        assert f"# TYPE {family} gauge" in before.splitlines()
        assert f"{sample} 1" in before.splitlines()
        assert f"{sample} 0" in after.splitlines()
    assert before.endswith("\n") and after.endswith("\n")
    assert usage.export_prometheus().endswith("\n")


class TestPrometheusUsageMetrics:
    def test_usage_metrics_appended_when_collector_present(self) -> None:
        collector = MetricsCollector()
        collector.increment("apcore_module_calls_total", {"module_id": "m.a", "status": "success"})

        usage = UsageCollector()
        usage.record(module_id="m.a", caller_id="caller", latency_ms=12.0, success=True)

        output = render_prometheus_metrics(collector, usage)

        # MetricsCollector families remain present.
        assert "apcore_module_calls_total" in output
        # Usage families are now appended.
        assert "apcore_usage_calls_total" in output
        assert "apcore_usage_error_rate" in output
        assert "apcore_usage_p50_latency_ms" in output

    def test_no_usage_metrics_without_collector(self) -> None:
        collector = MetricsCollector()
        collector.increment("apcore_module_calls_total", {"module_id": "m.a", "status": "success"})

        output = render_prometheus_metrics(collector)

        assert "apcore_module_calls_total" in output
        assert "apcore_usage_" not in output
