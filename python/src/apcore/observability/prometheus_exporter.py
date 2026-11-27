"""Render application metrics in Prometheus text format."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from apcore.observability.metrics import MetricsCollector
    from apcore.observability.usage import UsageCollector

__all__ = ["render_prometheus_metrics"]


def render_prometheus_metrics(collector: MetricsCollector, usage_collector: UsageCollector | None = None) -> str:
    """Render live metrics and optional usage as Prometheus text."""
    output = collector.export_prometheus()
    if usage_collector is not None:
        output += usage_collector.export_prometheus()
    return output
