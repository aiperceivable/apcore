"""Push-style ``UsageExporter`` interface (RFC #45 §3).

The original observability RFC (#45 §3) committed to a push-style exporter
that delivers hourly :class:`UsageCollector` summaries to external sinks
(HTTP, Kafka, custom backends). The pull-style :class:`PrometheusExporter`
covers scrape-based collection; this module covers the complementary push
path so operators can stream usage data into log pipelines or analytics
systems without standing up a Prometheus scrape target.

The shape mirrors the OpenTelemetry ``BatchSpanProcessor`` /
``OTLPExporter`` split:

* :class:`UsageExporter` — a duck-typed Protocol implemented by user
  code (HTTP, Kafka, file, etc.).
* :class:`NoopUsageExporter` — the default exporter that drops payloads.
* :class:`PeriodicUsageExporter` — orchestrator that periodically polls a
  :class:`apcore.observability.UsageCollector` and forwards the summary
  to the configured ``UsageExporter``. Decoupled from ``UsageCollector``
  so applications can compose multiple exporters or swap backends without
  touching the collector.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import asdict, is_dataclass
from typing import Any, Protocol, runtime_checkable

from apcore.observability.usage import UsageCollector

__all__ = [
    "NoopUsageExporter",
    "PeriodicUsageExporter",
    "UsageExporter",
]

logger = logging.getLogger(__name__)


@runtime_checkable
class UsageExporter(Protocol):
    """Push-style exporter for hourly :class:`UsageCollector` summaries.

    Mirrors the ``BatchSpanProcessor`` / ``OTLPExporter`` shape but for
    usage data. Implementations push summaries to external sinks (HTTP,
    Kafka, custom). Implementations MUST NOT raise from ``export``;
    failures should be swallowed and logged so a single bad sink cannot
    take down the periodic loop.
    """

    def export(self, summary: dict[str, Any]) -> None:
        """Forward a usage summary payload to the underlying sink."""
        ...

    def shutdown(self) -> None:
        """Release any resources held by the exporter (sockets, threads)."""
        ...


class NoopUsageExporter:
    """Default exporter that drops every summary.

    Used when no exporter is configured, mirroring the
    ``DefaultMetricsRecorder`` no-op pattern elsewhere in the SDK.
    """

    def export(self, summary: dict[str, Any]) -> None:  # noqa: D401 - protocol impl
        """Drop the summary on the floor."""
        return None

    def shutdown(self) -> None:  # noqa: D401 - protocol impl
        """No-op."""
        return None


def _summary_to_dict(summary: Any) -> dict[str, Any]:
    """Convert a UsageCollector summary (dataclass list or dict) to a JSON-friendly dict."""
    if isinstance(summary, dict):
        return summary
    if isinstance(summary, list):
        modules = [asdict(m) if is_dataclass(m) else m for m in summary]
        return {"modules": modules}
    if is_dataclass(summary):
        return asdict(summary)
    return {"value": summary}


class PeriodicUsageExporter:
    """Periodically poll a :class:`UsageCollector` and push to an exporter.

    Lifecycle is managed by :meth:`start` / :meth:`stop`; the loop runs as
    an :func:`asyncio.create_task` and is cancellable. The default
    ``interval_seconds`` of 3600 matches the hourly cadence specified in
    RFC #45 §3.

    Args:
        collector: The :class:`UsageCollector` to poll.
        exporter: Any object satisfying the :class:`UsageExporter`
            protocol (HTTP poster, Kafka producer, ...).
        interval_seconds: Seconds between poll-and-export ticks. Must be
            greater than zero.
        period: ``UsageCollector.get_summary`` period to request on each
            tick (default ``"1h"`` -- the most recent hour).
    """

    def __init__(
        self,
        collector: UsageCollector,
        exporter: UsageExporter,
        interval_seconds: float = 3600.0,
        period: str = "1h",
    ) -> None:
        if interval_seconds <= 0:
            raise ValueError("interval_seconds must be > 0")
        self._collector = collector
        self._exporter = exporter
        self._interval_seconds = float(interval_seconds)
        self._period = period
        self._task: asyncio.Task[None] | None = None
        self._stop_event: asyncio.Event | None = None

    async def start(self) -> None:
        """Begin the periodic export loop. Idempotent."""
        if self._task is not None and not self._task.done():
            return
        self._stop_event = asyncio.Event()
        self._task = asyncio.create_task(self._run(), name="apcore-periodic-usage-exporter")

    async def stop(self) -> None:
        """Stop the loop and call ``exporter.shutdown()``. Safe before start."""
        task = self._task
        stop_event = self._stop_event
        if task is None:
            # Still call shutdown for symmetry with started lifecycle? No -- only
            # shutdown the exporter if it was actually started, mirroring
            # BatchSpanProcessor semantics.
            return
        self._task = None
        self._stop_event = None
        if stop_event is not None:
            stop_event.set()
        task.cancel()
        try:
            await task
        except (asyncio.CancelledError, Exception):  # pragma: no cover - defensive
            pass
        try:
            self._exporter.shutdown()
        except Exception:  # pragma: no cover - defensive
            logger.exception("UsageExporter.shutdown() raised; ignoring")

    async def _run(self) -> None:
        """Internal loop: sleep, poll, push, repeat until cancelled."""
        assert self._stop_event is not None
        try:
            while not self._stop_event.is_set():
                try:
                    await asyncio.wait_for(
                        self._stop_event.wait(),
                        timeout=self._interval_seconds,
                    )
                    # If we get here, stop_event was set -> exit loop.
                    break
                except asyncio.TimeoutError:
                    pass
                self._tick()
        except asyncio.CancelledError:
            raise

    def _tick(self) -> None:
        """Single poll-and-push iteration; never propagates exceptions."""
        try:
            summary = self._collector.get_summary(self._period)
            payload = _summary_to_dict(summary)
        except Exception:
            logger.exception("UsageCollector.get_summary failed; skipping tick")
            return
        try:
            self._exporter.export(payload)
        except Exception:
            logger.exception("UsageExporter.export raised; continuing loop")
