"""Tests for the push-style UsageExporter interface (#45 §3)."""

from __future__ import annotations

import asyncio
from typing import Any

import pytest

from apcore.observability import (
    NoopUsageExporter,
    PeriodicUsageExporter,
    UsageCollector,
    UsageExporter,
)


class _RecordingExporter:
    """Test double that records every export and shutdown call."""

    def __init__(self) -> None:
        self.exports: list[dict[str, Any]] = []
        self.shutdown_count: int = 0

    def export(self, summary: dict[str, Any]) -> None:
        self.exports.append(summary)

    def shutdown(self) -> None:
        self.shutdown_count += 1


def test_recording_exporter_satisfies_protocol() -> None:
    """A duck-typed object exposing export/shutdown satisfies the Protocol."""
    exp = _RecordingExporter()
    assert isinstance(exp, UsageExporter)


def test_noop_exporter_satisfies_protocol() -> None:
    """The bundled NoopUsageExporter satisfies the protocol."""
    assert isinstance(NoopUsageExporter(), UsageExporter)


def test_noop_exporter_drops_summaries() -> None:
    """NoopUsageExporter.export must not raise and must return None."""
    exp = NoopUsageExporter()
    assert exp.export({"foo": "bar"}) is None
    assert exp.shutdown() is None


def test_top_level_exports() -> None:
    """The new symbols are re-exported from the apcore package."""
    import apcore

    assert hasattr(apcore, "UsageExporter")
    assert hasattr(apcore, "NoopUsageExporter")
    assert hasattr(apcore, "PeriodicUsageExporter")


@pytest.mark.asyncio
async def test_periodic_exporter_pushes_summaries_at_interval() -> None:
    """PeriodicUsageExporter must call exporter.export(summary) at each tick."""
    collector = UsageCollector()
    collector.record("executor.test.echo", "caller-a", 12.5, success=True)
    exporter = _RecordingExporter()

    periodic = PeriodicUsageExporter(
        collector=collector,
        exporter=exporter,
        interval_seconds=0.05,
    )

    await periodic.start()
    # Allow at least two ticks.
    await asyncio.sleep(0.18)
    await periodic.stop()

    assert len(exporter.exports) >= 2
    first = exporter.exports[0]
    # The pushed payload should be a dict carrying the collector summary.
    assert isinstance(first, dict)


@pytest.mark.asyncio
async def test_periodic_exporter_stop_cancels_loop() -> None:
    """After stop(), no further exports occur."""
    collector = UsageCollector()
    exporter = _RecordingExporter()

    periodic = PeriodicUsageExporter(
        collector=collector,
        exporter=exporter,
        interval_seconds=0.02,
    )
    await periodic.start()
    await asyncio.sleep(0.06)
    await periodic.stop()
    captured = len(exporter.exports)

    # Wait long enough for several more ticks if the loop were still running.
    await asyncio.sleep(0.1)
    assert len(exporter.exports) == captured


@pytest.mark.asyncio
async def test_periodic_exporter_double_start_is_idempotent() -> None:
    """Calling start() twice should not spawn duplicate tasks."""
    collector = UsageCollector()
    exporter = _RecordingExporter()

    periodic = PeriodicUsageExporter(
        collector=collector,
        exporter=exporter,
        interval_seconds=0.02,
    )
    await periodic.start()
    await periodic.start()  # second call must be a no-op
    await asyncio.sleep(0.05)
    await periodic.stop()


@pytest.mark.asyncio
async def test_periodic_exporter_stop_without_start_is_safe() -> None:
    """stop() before start() should be a no-op."""
    collector = UsageCollector()
    exporter = _RecordingExporter()

    periodic = PeriodicUsageExporter(
        collector=collector,
        exporter=exporter,
        interval_seconds=0.05,
    )
    # Must not raise.
    await periodic.stop()
