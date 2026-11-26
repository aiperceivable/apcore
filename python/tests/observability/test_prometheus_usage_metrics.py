"""Regression test for A-D-16: PrometheusExporter must append usage metrics.

Cross-language parity: TS prometheus-exporter.ts and Rust prometheus_exporter.rs
both append the ``apcore_usage_*`` metric families to their /metrics output when
a usage collector is present. The Python exporter previously exposed only the
MetricsCollector output and left UsageCollector.export_prometheus() unwired.

Back-compat: when no usage_collector is supplied, the output must be unchanged
(MetricsCollector output only).
"""

from __future__ import annotations

import threading
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from datetime import datetime, timedelta, timezone
from http.client import HTTPConnection

import pytest

from apcore.observability import render_prometheus_metrics
from apcore.observability import usage as usage_module
from apcore.observability.metrics import MetricsCollector
from apcore.observability.prometheus_exporter import PrometheusExporter
from apcore.observability.usage import UsageCollector


def _get(port: int, path: str) -> tuple[int, bytes]:
    with closing(HTTPConnection("127.0.0.1", port, timeout=2)) as connection:
        connection.request("GET", path)
        response = connection.getresponse()
        return response.status, response.read()


def test_prometheus_listener_binds_to_loopback_and_can_restart() -> None:
    exporter = PrometheusExporter(MetricsCollector())
    exporter.start(port=0)
    try:
        assert exporter._server is not None
        host, port = exporter._server.server_address
        assert host == "127.0.0.1"
        assert _get(port, "/readyz")[0] == 503
    finally:
        exporter.stop()

    exporter.start(port, host="127.0.0.1")
    try:
        exporter.mark_ready()
        assert _get(port, "/readyz") == (200, b"OK")
    finally:
        exporter.stop()


def test_health_probe_progresses_while_metrics_are_rendered(monkeypatch: pytest.MonkeyPatch) -> None:
    exporter = PrometheusExporter(MetricsCollector())
    rendering = threading.Event()
    release_render = threading.Event()

    def render() -> str:
        rendering.set()
        assert release_render.wait(timeout=5)
        return "sample_metric 1\n"

    monkeypatch.setattr(exporter, "export", render)
    exporter.start(port=0)
    try:
        assert exporter._server is not None
        _, port = exporter._server.server_address
        with ThreadPoolExecutor(max_workers=1) as pool:
            metrics = pool.submit(_get, port, "/metrics")
            try:
                assert rendering.wait(timeout=2)
                assert _get(port, "/healthz") == (200, b"OK")
            finally:
                release_render.set()
            assert metrics.result(timeout=2) == (200, b"sample_metric 1\n")
    finally:
        exporter.stop()


@pytest.mark.parametrize("populated_metrics", [False, True])
@pytest.mark.parametrize("populated_usage", [None, False, True])
def test_render_prometheus_metrics_matches_legacy_export(populated_metrics: bool, populated_usage: bool | None) -> None:
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
    assert PrometheusExporter(collector, usage).export() == expected
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
    exporter = PrometheusExporter(metrics, usage)
    before = exporter.export()
    instant += timedelta(hours=25)
    after = exporter.export()

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
