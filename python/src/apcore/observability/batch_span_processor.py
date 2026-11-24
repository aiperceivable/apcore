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
import threading
import time
from collections import deque
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
        self._queue: deque[Span] = deque()
        self._spans_dropped = 0
        self._condition = threading.Condition()
        self._accepted = 0
        self._completed = 0
        self._flush_target = 0
        self._shutdown_deadline: float | None = None
        self._stopped = False
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    @property
    def spans_dropped(self) -> int:
        with self._condition:
            return self._spans_dropped

    @property
    def queue_size(self) -> int:
        with self._condition:
            return len(self._queue)

    def on_span_end(self, span: Span) -> None:
        """Enqueue a span without waiting for the exporter."""
        with self._condition:
            if self._shutdown_deadline is not None:
                return
            if self._max_queue_size > 0 and len(self._queue) >= self._max_queue_size:
                self._spans_dropped += 1
                if self._spans_dropped == 1 or self._spans_dropped % 1000 == 0:
                    _logger.warning(
                        "BatchSpanProcessor: queue full (size=%d); span dropped (total dropped=%d)",
                        self._max_queue_size,
                        self._spans_dropped,
                    )
                return
            self._queue.append(span)
            self._accepted += 1

    def force_flush(self, timeout_ms: int = 30000) -> bool:
        """Wait at most ``timeout_ms`` for previously accepted spans to export."""
        with self._condition:
            target = self._accepted
            self._flush_target = max(self._flush_target, target)
            self._condition.notify_all()
            self._condition.wait_for(lambda: self._completed >= target or self._stopped, timeout=timeout_ms / 1000.0)
            return self._completed >= target

    def _run(self) -> None:
        """Own all exports and drain buffered spans when flushing or stopping."""
        delay = max(self._schedule_delay_ms, 1) / 1000.0
        batch_size = max(self._max_export_batch_size, 1)
        try:
            while True:
                with self._condition:
                    self._condition.wait_for(
                        lambda: self._shutdown_deadline is not None or self._completed < self._flush_target,
                        timeout=delay,
                    )
                    deadline = self._shutdown_deadline
                    if deadline is not None and (not self._queue or time.monotonic() >= deadline):
                        self._queue.clear()
                        return
                    batch = [self._queue.popleft() for _ in range(min(batch_size, len(self._queue)))]

                for span in batch:
                    deadline = self._shutdown_deadline
                    if deadline is not None and time.monotonic() >= deadline:
                        break
                    try:
                        self._exporter.export(span)
                    except Exception:  # noqa: BLE001 - exporter failures are best effort
                        _logger.exception("BatchSpanProcessor: export failed")
                    with self._condition:
                        self._completed += 1
                        self._condition.notify_all()
        finally:
            with self._condition:
                self._stopped = True
                self._condition.notify_all()
            try:
                close_exporter = getattr(self._exporter, "shutdown", None)
                if callable(close_exporter):
                    close_exporter()
            except Exception:  # noqa: BLE001 - exporter shutdown is best effort
                _logger.exception("BatchSpanProcessor: exporter.shutdown() failed")

    def shutdown(self) -> None:
        """Stop admission and drain as many queued spans as the deadline allows."""
        with self._condition:
            if self._shutdown_deadline is not None:
                return
            timeout = max(self._export_timeout_ms, 0) / 1000.0
            self._shutdown_deadline = time.monotonic() + timeout
            self._condition.notify_all()
        if threading.current_thread() is not self._thread:
            self._thread.join(timeout=max(self._shutdown_deadline - time.monotonic(), 0))


__all__ = [
    "BatchSpanProcessor",
    "SimpleSpanProcessor",
    "SpanExporter",
    "SpanProcessor",
]
