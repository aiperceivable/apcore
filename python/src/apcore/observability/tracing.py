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
from typing import Any

from apcore.context_keys import TRACING_SAMPLED, TRACING_SPANS, _current_trace_span, _middleware_data
from apcore.middleware import Middleware
from apcore.observability.span import Span, create_span


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

    Bridges apcore ``Span`` instances to OpenTelemetry with matching IDs,
    timestamps, attributes, and status, then exports them through OTLP HTTP.

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
        """Initialize the OpenTelemetry span processor and HTTP exporter.

        Raises:
            ImportError: If required opentelemetry packages are not installed.
        """
        try:
            from opentelemetry.exporter.otlp.proto.http.trace_exporter import (  # type: ignore[import-not-found]
                OTLPSpanExporter as _OTLPSpanExporter,
            )
            from opentelemetry.sdk.resources import Resource  # type: ignore[import-not-found]
            from opentelemetry.sdk.trace import Event, ReadableSpan  # type: ignore[import-not-found]
            from opentelemetry.sdk.trace.export import SimpleSpanProcessor as _OtelSimpleSpanProcessor  # type: ignore[import-not-found]
            from opentelemetry.sdk.util.instrumentation import InstrumentationScope  # type: ignore[import-not-found]
            from opentelemetry.trace import SpanContext, Status, StatusCode, TraceFlags  # type: ignore[import-not-found]
        except ImportError:
            raise ImportError(
                "opentelemetry packages are required for OTLPExporter. "
                "Install with: pip install opentelemetry-sdk opentelemetry-exporter-otlp-proto-http"
            ) from None

        self._SpanContext = SpanContext
        self._ReadableSpan = ReadableSpan
        self._Event = Event
        self._Status = Status
        self._StatusCode = StatusCode
        self._sampled_trace_flags = TraceFlags(TraceFlags.SAMPLED)
        #: The collector this exporter targets. Retained because
        #: PROTOCOL_SPEC §10.1.1 requirement 3 makes it configurable through
        #: `observability.tracing.otlp_endpoint`, and a value that reaches the
        #: exporter unobservably cannot be told apart from one that does not.
        #: `None` means the OTel SDK's own default.
        self._endpoint = endpoint
        self._attribute_allowlist: frozenset[str] | None = (
            frozenset(attribute_allowlist) if attribute_allowlist is not None else None
        )

        self._resource = Resource.create({"service.name": service_name})
        self._instrumentation_scope = InstrumentationScope("apcore.tracing")

        exporter_kwargs: dict[str, Any] = {}
        if endpoint is not None:
            exporter_kwargs["endpoint"] = endpoint

        self._processor = _OtelSimpleSpanProcessor(_OTLPSpanExporter(**exporter_kwargs))

    def export(self, span: Span) -> None:
        """Export an OpenTelemetry span with the APCore trace and parent IDs."""
        attributes = {
            key: value if isinstance(value, (str, int, float, bool)) else str(value)
            for key, value in span.attributes.items()
            if value is not None and (self._attribute_allowlist is None or key in self._attribute_allowlist)
        }
        attributes["apcore.trace_id"] = span.trace_id
        attributes["apcore.span_id"] = span.span_id
        if span.parent_span_id:
            attributes["apcore.parent_span_id"] = span.parent_span_id

        trace_id = _otel_id(span.trace_id, 32) or _new_otel_id(16)
        span_id = _otel_id(span.span_id, 16) or _new_otel_id(8)
        context = self._SpanContext(trace_id, span_id, False, self._sampled_trace_flags)
        parent_id = _otel_id(span.parent_span_id, 16)
        parent = self._SpanContext(trace_id, parent_id, False, self._sampled_trace_flags) if parent_id else None
        status = self._StatusCode.ERROR if span.status == "error" else self._StatusCode.UNSET

        self._processor.on_end(
            self._ReadableSpan(
                name=span.name,
                context=context,
                parent=parent,
                resource=self._resource,
                attributes=attributes,
                events=[
                    self._Event(event.get("name", "event"), {k: str(v) for k, v in event.items() if k != "name"})
                    for event in span.events
                ],
                instrumentation_scope=self._instrumentation_scope,
                status=self._Status(status),
                start_time=int(span.start_time * 1e9),
                end_time=int(span.end_time * 1e9) if span.end_time is not None else time.time_ns(),
            )
        )

    def shutdown(self) -> None:
        """Shut down the underlying OpenTelemetry span processor."""
        self._processor.shutdown()


def _otel_id(value: str | None, digits: int) -> int | None:
    if value and len(value) == digits and all(digit in "0123456789abcdef" for digit in value):
        return int(value, 16) or None
    return None


def _new_otel_id(size: int) -> int:
    """Retain support for manually exported spans without canonical APCore IDs."""
    return int.from_bytes(os.urandom(size), "big") or 1


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
    SpanExporter,
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

    Executor calls keep spans and sampling decisions on the invocation
    context, with the parent's span captured when a child call starts.
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
        data = _middleware_data(context)
        inherited = getattr(context, "_parent_sampled", None)
        existing = data.get(TRACING_SAMPLED.name, inherited)
        if existing is None:
            existing = TRACING_SAMPLED.get(context)
        if isinstance(existing, bool):
            data[TRACING_SAMPLED.name] = existing
            return existing

        if self._sampling_strategy == "full":
            decision = True
        elif self._sampling_strategy == "off":
            decision = False
        else:  # proportional or error_first
            decision = random.random() < self._sampling_rate

        data[TRACING_SAMPLED.name] = decision
        return decision

    def before(self, module_id: str, inputs: dict[str, Any], context: Any) -> dict[str, Any] | None:
        """Create a span, push to stack, make/inherit sampling decision."""
        self._should_sample(context)

        parent = _current_trace_span(context, include_parent=True)
        parent_span_id = parent.span_id if parent is not None else None
        remote_parent = getattr(context, "_inbound_trace_parent", None)
        if parent_span_id is None and remote_parent is not None and remote_parent.trace_id == context.trace_id:
            parent_span_id = remote_parent.parent_id

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
        _middleware_data(context).setdefault(TRACING_SPANS.name, []).append(span)
        return None

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Any,
    ) -> dict[str, Any] | None:
        """Pop span, finalize with success status, export if sampled."""
        data = _middleware_data(context)
        spans_stack = data.get(TRACING_SPANS.name, [])
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

        if data.get(TRACING_SAMPLED.name):
            self._processor.on_span_end(span)
        return None

    def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Any) -> dict[str, Any] | None:
        """Pop span, finalize with error status, always export for error_first. Return None."""
        data = _middleware_data(context)
        spans_stack = data.get(TRACING_SPANS.name, [])
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

        should_export = self._sampling_strategy == "error_first" or data.get(TRACING_SAMPLED.name)
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
