"""Tracing system: Span dataclass, SpanExporter, SpanProcessor implementations, and TracingMiddleware."""

from __future__ import annotations

import collections
import dataclasses
import json
import logging
import os
import random
import sys
import threading
import time
from typing import Any, Protocol, runtime_checkable

from apcore.middleware import Middleware
from apcore.observability.span import Span, create_span


@runtime_checkable
class SpanExporter(Protocol):
    """Protocol for span export destinations."""

    def export(self, span: Span) -> None:
        """Export a completed span."""
        ...


class StdoutExporter:
    """Exports spans as JSON lines to stdout."""

    def export(self, span: Span) -> None:
        """Write span as a single JSON line to stdout."""
        data = dataclasses.asdict(span)
        sys.stdout.write(json.dumps(data, default=str) + "\n")


_tracing_logger = logging.getLogger(__name__)


class InMemoryExporter:
    """Collects spans in memory for testing.

    Thread-safe and bounded: uses a deque with a configurable max size
    to prevent unbounded memory growth.
    """

    def __init__(self, max_spans: int = 10_000) -> None:
        self._spans: collections.deque[Span] = collections.deque(maxlen=max_spans)
        self._lock = threading.Lock()

    def export(self, span: Span) -> None:
        """Add span to internal collection (thread-safe, bounded)."""
        with self._lock:
            self._spans.append(span)

    def get_spans(self) -> list[Span]:
        """Return all collected spans."""
        with self._lock:
            return list(self._spans)

    def clear(self) -> None:
        """Remove all collected spans."""
        with self._lock:
            self._spans.clear()


class OTLPExporter:
    """Exports spans via OpenTelemetry Protocol (requires opentelemetry SDK).

    Bridges apcore ``Span`` instances to OpenTelemetry by creating real OTel
    spans with matching timestamps, attributes, and status, then exporting them
    through the OTLP HTTP protocol to any compatible collector.

    Args:
        endpoint: OTLP collector endpoint URL. Defaults to OTel SDK default
            (``http://localhost:4318/v1/traces`` for HTTP).
        service_name: ``service.name`` resource attribute. Defaults to ``"apcore"``.
        attribute_allowlist: When provided, only span attributes whose key is
            in this set are exported. Apcore-owned keys (``apcore.trace_id``,
            ``apcore.span_id``, ``apcore.parent_span_id``) are always exported.
            Use this to prevent unvetted upstream attributes from leaking PII
            into the collector backend.
    """

    def __init__(
        self,
        endpoint: str | None = None,
        service_name: str = "apcore",
        attribute_allowlist: set[str] | None = None,
    ) -> None:
        """Initialize OTLPExporter with an OTel TracerProvider and OTLP exporter.

        Raises:
            ImportError: If required opentelemetry packages are not installed.
        """
        try:
            from opentelemetry.exporter.otlp.proto.http.trace_exporter import (  # type: ignore[import-not-found]
                OTLPSpanExporter as _OTLPSpanExporter,
            )
            from opentelemetry.sdk.resources import Resource  # type: ignore[import-not-found]
            from opentelemetry.sdk.trace import TracerProvider  # type: ignore[import-not-found]
            from opentelemetry.sdk.trace.export import SimpleSpanProcessor as _OtelSimpleSpanProcessor  # type: ignore[import-not-found]
            from opentelemetry.trace import StatusCode  # type: ignore[import-not-found]
        except ImportError:
            raise ImportError(
                "opentelemetry packages are required for OTLPExporter. "
                "Install with: pip install opentelemetry-sdk opentelemetry-exporter-otlp-proto-http"
            ) from None

        self._StatusCode = StatusCode
        #: The collector this exporter targets. Retained because
        #: PROTOCOL_SPEC §10.1.1 requirement 3 makes it configurable through
        #: `observability.tracing.otlp_endpoint`, and a value that reaches the
        #: exporter unobservably cannot be told apart from one that does not.
        #: `None` means the OTel SDK's own default.
        self._endpoint = endpoint
        self._attribute_allowlist: frozenset[str] | None = (
            frozenset(attribute_allowlist) if attribute_allowlist is not None else None
        )

        resource = Resource.create({"service.name": service_name})
        self._provider = TracerProvider(resource=resource)

        exporter_kwargs: dict[str, Any] = {}
        if endpoint is not None:
            exporter_kwargs["endpoint"] = endpoint

        otlp_exporter = _OTLPSpanExporter(**exporter_kwargs)
        self._provider.add_span_processor(_OtelSimpleSpanProcessor(otlp_exporter))
        self._tracer = self._provider.get_tracer("apcore.tracing")

    def export(self, span: Span) -> None:
        """Convert an apcore Span to an OpenTelemetry span and export via OTLP."""
        start_ns = int(span.start_time * 1e9)

        otel_span = self._tracer.start_span(name=span.name, start_time=start_ns)

        otel_span.set_attribute("apcore.trace_id", span.trace_id)
        otel_span.set_attribute("apcore.span_id", span.span_id)
        if span.parent_span_id:
            otel_span.set_attribute("apcore.parent_span_id", span.parent_span_id)

        for key, value in span.attributes.items():
            if value is None:
                continue
            if self._attribute_allowlist is not None and key not in self._attribute_allowlist:
                continue
            if isinstance(value, (str, int, float, bool)):
                otel_span.set_attribute(key, value)
            else:
                otel_span.set_attribute(key, str(value))

        if span.status == "error":
            otel_span.set_status(self._StatusCode.ERROR)

        for event in span.events:
            event_name = event.get("name", "event")
            event_attrs = {k: str(v) for k, v in event.items() if k != "name"}
            otel_span.add_event(event_name, attributes=event_attrs)

        end_ns = int(span.end_time * 1e9) if span.end_time else None
        otel_span.end(end_time=end_ns)

    def shutdown(self) -> None:
        """Flush pending spans and shut down the underlying TracerProvider."""
        self._provider.shutdown()


# ---------------------------------------------------------------------------
# Span Processors
# ---------------------------------------------------------------------------


# Span processor implementations live in their own module (Issue #43 §1.2)
# to mirror the layout of the TypeScript and Rust SDKs.  They are re-exported
# here for backward compatibility with code that imports them from this
# module.
from apcore.observability.batch_span_processor import (  # noqa: E402
    BatchSpanProcessor,
    SimpleSpanProcessor,
    SpanProcessor,
)


# ---------------------------------------------------------------------------
# TracingMiddleware
# ---------------------------------------------------------------------------

_VALID_STRATEGIES = {"full", "proportional", "error_first", "off"}


class TracingMiddleware(Middleware):
    """Middleware that creates and manages trace spans for module calls.

    Accepts either an ``exporter`` (wrapped in ``SimpleSpanProcessor``) or a
    ``processor`` directly.  When ``exporter`` is supplied, backward-compatible
    behavior is preserved.

    Uses stack-based context.data storage to correctly handle nested
    module-to-module call chains.
    """

    def __init__(
        self,
        exporter: SpanExporter | None = None,
        sampling_rate: float = 1.0,
        sampling_strategy: str = "full",
        *,
        processor: SpanProcessor | None = None,
        priority: int = 100,
    ) -> None:
        super().__init__(priority=priority)
        if not (0.0 <= sampling_rate <= 1.0):
            raise ValueError(f"sampling_rate must be between 0.0 and 1.0, got {sampling_rate}")
        if sampling_strategy not in _VALID_STRATEGIES:
            raise ValueError(f"sampling_strategy must be one of {_VALID_STRATEGIES}, got {sampling_strategy!r}")
        if exporter is None and processor is None:
            raise ValueError("Either 'exporter' or 'processor' must be provided.")
        self._sampling_rate = sampling_rate
        self._sampling_strategy = sampling_strategy
        if processor is not None:
            self._processor: SpanProcessor = processor
        else:
            assert exporter is not None
            self._processor = SimpleSpanProcessor(exporter)

    @property
    def _exporter(self) -> SpanExporter:
        """Backward-compatible accessor: return the exporter from the underlying processor."""
        if isinstance(self._processor, SimpleSpanProcessor):
            return self._processor._exporter
        raise AttributeError("TracingMiddleware uses a BatchSpanProcessor; access _processor._exporter instead.")

    def set_exporter(self, exporter: SpanExporter) -> None:
        """Replace the underlying exporter (wraps it in a SimpleSpanProcessor).

        Calls ``shutdown()`` on the previous processor to prevent thread leaks
        when replacing a ``BatchSpanProcessor``.
        """
        old = self._processor
        self._processor = SimpleSpanProcessor(exporter)
        old.shutdown()

    def _should_sample(self, context: Any) -> bool:
        """Make or inherit sampling decision."""
        existing = context.data.get("_apcore.mw.tracing.sampled")
        if isinstance(existing, bool):
            return existing

        if self._sampling_strategy == "full":
            decision = True
        elif self._sampling_strategy == "off":
            decision = False
        else:  # proportional or error_first
            decision = random.random() < self._sampling_rate

        context.data["_apcore.mw.tracing.sampled"] = decision
        return decision

    def before(self, module_id: str, inputs: dict[str, Any], context: Any) -> dict[str, Any] | None:
        """Create a span, push to stack, make/inherit sampling decision."""
        self._should_sample(context)

        spans_stack = context.data.setdefault("_apcore.mw.tracing.spans", [])
        parent_span_id = spans_stack[-1].span_id if spans_stack else None

        span = Span(
            trace_id=context.trace_id,
            span_id=os.urandom(8).hex(),
            parent_span_id=parent_span_id,
            name="apcore.module.execute",
            start_time=time.time(),
            attributes={
                "module_id": module_id,
                "method": "execute",
                "caller_id": context.caller_id,
            },
        )
        spans_stack.append(span)
        return None

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Any,
    ) -> dict[str, Any] | None:
        """Pop span, finalize with success status, export if sampled."""
        spans_stack = context.data.get("_apcore.mw.tracing.spans", [])
        if not spans_stack:
            _tracing_logger.warning(
                "TracingMiddleware.after() called with empty span stack for %s",
                module_id,
            )
            return None
        span = spans_stack.pop()
        span.end_time = time.time()
        span.status = "ok"
        span.attributes["duration_ms"] = (span.end_time - span.start_time) * 1000
        span.attributes["success"] = True

        if context.data.get("_apcore.mw.tracing.sampled"):
            self._processor.on_span_end(span)
        return None

    def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Any) -> dict[str, Any] | None:
        """Pop span, finalize with error status, always export for error_first. Return None."""
        spans_stack = context.data.get("_apcore.mw.tracing.spans", [])
        if not spans_stack:
            _tracing_logger.warning(
                "TracingMiddleware.on_error() called with empty span stack for %s",
                module_id,
            )
            return None
        span = spans_stack.pop()
        span.end_time = time.time()
        span.status = "error"
        span.attributes["duration_ms"] = (span.end_time - span.start_time) * 1000
        span.attributes["success"] = False
        span.attributes["error_code"] = getattr(error, "code", type(error).__name__)

        should_export = self._sampling_strategy == "error_first" or context.data.get("_apcore.mw.tracing.sampled")
        if should_export:
            self._processor.on_span_end(span)
        return None


__all__ = [
    "Span",
    "create_span",
    "SpanExporter",
    "SpanProcessor",
    "StdoutExporter",
    "InMemoryExporter",
    "OTLPExporter",
    "SimpleSpanProcessor",
    "BatchSpanProcessor",
    "TracingMiddleware",
]
