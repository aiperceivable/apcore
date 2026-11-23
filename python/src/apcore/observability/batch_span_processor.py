"""Span processors for non-blocking OTEL export (Issue #43 §1.2).

This module hosts the canonical Python implementations of
:class:`SimpleSpanProcessor` and :class:`BatchSpanProcessor`.  The classes
were originally defined in :mod:`apcore.observability.tracing`; moving them
into a dedicated module mirrors the layout of the TypeScript and Rust SDKs
(``observability/batch-span-processor.ts`` and
``observability/processor.rs`` respectively) and makes the public surface
easier to discover.

The semantics match the cross-language spec in ``docs/features/observability.md``:

* ``BatchSpanProcessor`` buffers spans in a bounded queue and exports them
  in background batches; queue-full enqueues are dropped (non-blocking) and
  ``spans_dropped`` is incremented.
* ``shutdown()`` flushes remaining spans within ``export_timeout_ms``.
* ``force_flush(timeout_ms)`` synchronously drains the queue while the
  processor remains alive, mirroring the TS/Rust ``forceFlush`` ergonomic.
"""

from __future__ import annotations

import logging
import queue
import threading
import time
from typing import Protocol, runtime_checkable

from apcore.observability.span import Span

_logger = logging.getLogger(__name__)


@runtime_checkable
class SpanExporter(Protocol):
    """Protocol for span export destinations.

    Implementations must accept a single :class:`Span`.  ``shutdown`` is
    optional; when present it is called by processors during their own
    shutdown to release downstream resources.
    """

    def export(self, span: Span) -> None: ...

    def shutdown(self) -> None: ...  # pragma: no cover - protocol


@runtime_checkable
class SpanProcessor(Protocol):
    """Protocol for span processor implementations.

    Both :class:`SimpleSpanProcessor` and :class:`BatchSpanProcessor`
    satisfy this protocol, as do any user-defined processors.
    """

    def on_span_end(self, span: Span) -> None: ...

    def shutdown(self) -> None: ...


class SimpleSpanProcessor:
    """Synchronous span processor: exports each span immediately on the calling thread.

    Use in development and testing environments where blocking is acceptable.
    """

    def __init__(self, exporter: SpanExporter) -> None:
        self._exporter = exporter

    def on_span_end(self, span: Span) -> None:
        """Export the span synchronously."""
        self._exporter.export(span)

    def shutdown(self) -> None:
        """No-op for the simple processor."""


class BatchSpanProcessor:
    """Non-blocking span processor that buffers spans and exports in background batches.

    Spans are enqueued immediately without blocking the caller. A background
    thread periodically drains the queue up to ``max_export_batch_size``
    spans per flush. When the queue is full, new spans are dropped and
    ``spans_dropped`` is incremented.

    The class also accepts the keyword-argument alias ``max_export_batch``
    (matching the parameter name used by some apcore SDK ports) — internally
    both names map to ``max_export_batch_size``.

    Args:
        exporter: The :class:`SpanExporter` to deliver batches to.
        max_queue_size: Maximum buffer capacity before drops occur (default 2048).
        schedule_delay_ms: Milliseconds between successive flush attempts (default 5000).
        max_export_batch_size: Maximum spans per flush call (default 512).
        export_timeout_ms: Shutdown flush deadline in milliseconds (default 30000).
    """

    def __init__(
        self,
        exporter: SpanExporter,
        max_queue_size: int = 2048,
        schedule_delay_ms: int = 5000,
        max_export_batch_size: int = 512,
        export_timeout_ms: int = 30000,
        *,
        max_export_batch: int | None = None,
    ) -> None:
        self._exporter = exporter
        self._max_queue_size = max_queue_size
        self._schedule_delay_ms = schedule_delay_ms
        # Accept the spec-style alias.
        self._max_export_batch_size = max_export_batch if max_export_batch is not None else max_export_batch_size
        self._export_timeout_ms = export_timeout_ms
        self._queue: queue.Queue[Span] = queue.Queue(maxsize=max_queue_size)
        self._spans_dropped = 0
        self._dropped_lock = threading.Lock()
        self._shutdown_event = threading.Event()
        self._shutdown_called = False
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    @property
    def spans_dropped(self) -> int:
        with self._dropped_lock:
            return self._spans_dropped

    @property
    def queue_size(self) -> int:
        return self._queue.qsize()

    def on_span_end(self, span: Span) -> None:
        """Enqueue span or drop (non-blocking) if the queue is at capacity.

        Logs a warning at most once per consecutive drop streak so the
        operator is alerted without flooding the log.
        """
        try:
            self._queue.put_nowait(span)
        except queue.Full:
            with self._dropped_lock:
                self._spans_dropped += 1
                if self._spans_dropped == 1 or self._spans_dropped % 1000 == 0:
                    _logger.warning(
                        "BatchSpanProcessor: queue full (size=%d); span dropped (total dropped=%d)",
                        self._max_queue_size,
                        self._spans_dropped,
                    )

    def _flush(self) -> None:
        """Drain up to max_export_batch_size spans and export them."""
        batch: list[Span] = []
        try:
            while len(batch) < self._max_export_batch_size:
                batch.append(self._queue.get_nowait())
        except queue.Empty:
            pass
        for span in batch:
            try:
                self._exporter.export(span)
            except Exception:  # noqa: BLE001
                _logger.exception("BatchSpanProcessor: export failed")

    def force_flush(self, timeout_ms: int = 30000) -> bool:
        """Synchronously drain the queue, exporting all currently buffered spans.

        Unlike :meth:`shutdown`, the processor stays alive after this call;
        new spans can continue to be enqueued.  Returns ``True`` once the
        queue is empty, or ``False`` if the deadline expires first.

        Repeatedly drains in a tight loop because spans can arrive between
        successive ``_flush()`` calls.
        """
        deadline = time.monotonic() + (timeout_ms / 1000.0)
        while time.monotonic() < deadline:
            self._flush()
            if self._queue.empty():
                return True
            # Yield briefly to producer threads.
            time.sleep(0.001)
        return self._queue.empty()

    def _run(self) -> None:
        """Background worker: flush on each schedule_delay_ms tick, then on shutdown."""
        delay = self._schedule_delay_ms / 1000.0
        while True:
            shutdown_signaled = self._shutdown_event.wait(timeout=delay)
            self._flush()
            if shutdown_signaled:
                break

    def shutdown(self) -> None:
        """Signal shutdown; flush remaining spans within export_timeout_ms deadline.

        Idempotent — calling ``shutdown`` more than once is safe and a no-op
        on subsequent invocations.
        """
        if self._shutdown_called:
            return
        self._shutdown_called = True
        self._shutdown_event.set()
        timeout = self._export_timeout_ms / 1000.0
        self._thread.join(timeout=timeout)
        # Final drain for any spans that arrived after the last flush
        self._flush()
        # Best-effort downstream shutdown.
        try:
            shutdown_fn = getattr(self._exporter, "shutdown", None)
            if callable(shutdown_fn):
                shutdown_fn()
        except Exception:  # noqa: BLE001
            _logger.exception("BatchSpanProcessor: exporter.shutdown() failed")


__all__ = [
    "BatchSpanProcessor",
    "SimpleSpanProcessor",
    "SpanExporter",
    "SpanProcessor",
]
