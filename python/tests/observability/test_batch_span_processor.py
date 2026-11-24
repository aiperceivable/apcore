"""Tests for the BatchSpanProcessor module (Issue #43)."""

from __future__ import annotations

import time


from apcore.observability import BatchSpanProcessor
from apcore.observability.batch_span_processor import BatchSpanProcessor as DirectImport
from apcore.observability.tracing import InMemoryExporter, Span


def _span(name: str = "s") -> Span:
    return Span(trace_id="t1", name=name, start_time=time.time())


# ---------------------------------------------------------------------------
# Module surface
# ---------------------------------------------------------------------------


class TestBatchSpanProcessorModule:
    def test_module_exists(self) -> None:
        # The new dedicated module is importable
        import apcore.observability.batch_span_processor as mod

        assert mod is not None

    def test_re_exported_from_observability_package(self) -> None:
        # Same class accessible via top-level package
        assert BatchSpanProcessor is DirectImport


# ---------------------------------------------------------------------------
# Enqueue + flush
# ---------------------------------------------------------------------------


class TestBatchSpanProcessorEnqueue:
    def test_on_end_enqueues_spans(self) -> None:
        exporter = InMemoryExporter()
        proc = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=10,
            schedule_delay_ms=60_000,
        )
        try:
            proc.on_span_end(_span())
            proc.on_span_end(_span())
            assert proc.queue_size == 2
            assert exporter.get_spans() == []
        finally:
            proc.shutdown()

    def test_force_flush_drains_queue_synchronously(self) -> None:
        exporter = InMemoryExporter()
        proc = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=10,
            schedule_delay_ms=60_000,
        )
        try:
            for _ in range(3):
                proc.on_span_end(_span())
            assert proc.queue_size == 3
            proc.force_flush(timeout_ms=2000)
            assert proc.queue_size == 0
            assert len(exporter.get_spans()) == 3
        finally:
            proc.shutdown()


# ---------------------------------------------------------------------------
# Queue full backpressure
# ---------------------------------------------------------------------------


class TestBatchSpanProcessorQueueFull:
    def test_drops_when_queue_full_and_increments_counter(self) -> None:
        exporter = InMemoryExporter()
        proc = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=2,
            schedule_delay_ms=60_000,
        )
        try:
            proc.on_span_end(_span())
            proc.on_span_end(_span())
            assert proc.spans_dropped == 0

            proc.on_span_end(_span())  # third — must drop
            proc.on_span_end(_span())  # fourth — must drop
            assert proc.queue_size == 2
            assert proc.spans_dropped == 2
        finally:
            proc.shutdown()


# ---------------------------------------------------------------------------
# Background flush + shutdown
# ---------------------------------------------------------------------------


class TestBatchSpanProcessorShutdown:
    def test_background_flush_exports_within_delay(self) -> None:
        exporter = InMemoryExporter()
        proc = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=10,
            schedule_delay_ms=50,
        )
        try:
            for _ in range(3):
                proc.on_span_end(_span())
            deadline = time.time() + 2.0
            while len(exporter.get_spans()) < 3 and time.time() < deadline:
                time.sleep(0.01)
            assert len(exporter.get_spans()) == 3
        finally:
            proc.shutdown()

    def test_shutdown_flushes_remaining_spans(self) -> None:
        exporter = InMemoryExporter()
        proc = BatchSpanProcessor(
            exporter=exporter,
            max_queue_size=10,
            schedule_delay_ms=60_000,
        )
        proc.on_span_end(_span())
        proc.on_span_end(_span())
        proc.shutdown()
        assert len(exporter.get_spans()) == 2

    def test_shutdown_idempotent(self) -> None:
        exporter = InMemoryExporter()
        proc = BatchSpanProcessor(exporter=exporter, schedule_delay_ms=60_000)
        proc.shutdown()
        proc.shutdown()  # should not raise
