"""Regression test for A-D-16: PrometheusExporter must append usage metrics.

Cross-language parity: TS prometheus-exporter.ts and Rust prometheus_exporter.rs
both append the ``apcore_usage_*`` metric families to their /metrics output when
a usage collector is present. The Python exporter previously exposed only the
MetricsCollector output and left UsageCollector.export_prometheus() unwired.

Back-compat: when no usage_collector is supplied, the output must be unchanged
(MetricsCollector output only).
"""

from __future__ import annotations

from apcore.observability.metrics import MetricsCollector
from apcore.observability.prometheus_exporter import PrometheusExporter
from apcore.observability.usage import UsageCollector


class TestPrometheusUsageMetrics:
    def test_usage_metrics_appended_when_collector_present(self) -> None:
        collector = MetricsCollector()
        collector.increment("apcore_module_calls_total", {"module_id": "m.a", "status": "success"})

        usage = UsageCollector()
        usage.record(module_id="m.a", caller_id="caller", latency_ms=12.0, success=True)

        exporter = PrometheusExporter(collector, usage_collector=usage)
        output = exporter.export()

        # MetricsCollector families remain present.
        assert "apcore_module_calls_total" in output
        # Usage families are now appended.
        assert "apcore_usage_calls_total" in output
        assert "apcore_usage_error_rate" in output
        assert "apcore_usage_p50_latency_ms" in output

    def test_no_usage_metrics_without_collector(self) -> None:
        collector = MetricsCollector()
        collector.increment("apcore_module_calls_total", {"module_id": "m.a", "status": "success"})

        exporter = PrometheusExporter(collector)
        output = exporter.export()

        assert "apcore_module_calls_total" in output
        assert "apcore_usage_" not in output

    def test_default_usage_collector_is_none(self) -> None:
        """Back-compat: the positional-only signature still works."""
        exporter = PrometheusExporter(MetricsCollector())
        # No exception, no usage output.
        assert "apcore_usage_" not in exporter.export()
