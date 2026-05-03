# Observability System

## Overview

Comprehensive observability with distributed tracing, metrics collection, and structured context logging. The system is implemented as a set of middleware components that plug into the apcore middleware pipeline, providing automatic per-module instrumentation. It includes an OpenTelemetry bridge for production tracing, Prometheus-format metrics export, and a standalone structured logger with trace context injection and sensitive field redaction.

## Requirements

### Tracing
- Provide a `Span` dataclass capturing trace ID, span ID, parent span ID, name, timing, status, attributes, and events.
- Implement `TracingMiddleware` using stack-based span management in `context.data` to correctly handle nested module-to-module calls.
- Support four sampling strategies: `full` (always export), `proportional` (random sampling at configurable rate), `error_first` (always export errors, proportional for successes), and `off` (never export).
- Inherit sampling decisions from parent spans in nested calls.
- Define a `SpanExporter` protocol with three implementations: `StdoutExporter` (JSON lines to stdout), `InMemoryExporter` (bounded in-memory collection for testing), and `OTLPExporter` (OpenTelemetry bridge).
- `InMemoryExporter` MUST be bounded (deque with configurable maxlen, default 10,000) to prevent unbounded memory growth.

### Metrics
- Implement a `MetricsCollector` with thread-safe counters and histograms (with configurable bucket boundaries).
- Provide convenience methods for standard apcore metrics: `increment_calls()` → `apcore_module_calls_total`, `increment_errors()` → `apcore_module_errors_total`, `observe_duration()` → `apcore_module_duration_seconds`.
- Support Prometheus text exposition format export via `export_prometheus()`.
- Implement `MetricsMiddleware` that automatically records call counts (success/error), error codes, and execution duration for each module call.
- Use stack-based timing in `context.data` for correct nested call support.

### Structured Logging
- Implement `ContextLogger` as a standalone structured logger with JSON and text output formats.
- Support log levels: trace, debug, info, warn, error, fatal.
- Inject trace context (trace_id, module_id, caller_id) into every log entry.
- Automatically redact sensitive data using two mechanisms:
  1. `x-sensitive` schema annotation: used by the Executor to redact annotated fields from input/output before logging.
  2. `_secret_` key prefix: used by `ContextLogger` to redact matching keys in log extras when `redact_sensitive=True`.
- Provide `ContextLogger.from_context()` factory for automatic context extraction.
- Implement `ObsLoggingMiddleware` using `ContextLogger` with stack-based timing and configurable input/output logging.

## Technical Design

### Tracing Architecture

The tracing system uses a stack-based approach stored in `context.data["_apcore.mw.tracing.spans"]`. This correctly handles nested module calls within the same trace:

```
TracingMiddleware.before("mod.a"):
  Stack: [Span(mod.a)]

  TracingMiddleware.before("mod.b"):
    Stack: [Span(mod.a), Span(mod.b)]
    Span(mod.b).parent_span_id = Span(mod.a).span_id

  TracingMiddleware.after("mod.b"):
    Pop Span(mod.b), export if sampled
    Stack: [Span(mod.a)]

TracingMiddleware.after("mod.a"):
  Pop Span(mod.a), export if sampled
  Stack: []
```

#### Sampling Decision Flow

```
_should_sample(context):
  1. Check context.data["_apcore.mw.tracing.sampled"] -- if exists, inherit decision
  2. If "full" strategy -> always True
  3. If "off" strategy -> always False
  4. If "proportional" or "error_first" -> random.random() < sampling_rate
  5. Store decision in context.data for child spans to inherit
```

For `error_first`, the sampling decision only affects success spans. Error spans in `on_error()` are always exported regardless of the stored decision.

#### Span Exporters

- **`StdoutExporter`**: Serializes the span to a dictionary and writes it as a single JSON line to stdout.
- **`InMemoryExporter`**: Thread-safe ring buffer with configurable maximum capacity (`max_spans`) and lock protection. Provides `get_spans()`, `clear()` methods.
- **`OTLPExporter`**: Bridges apcore spans to OpenTelemetry. Creates an OTel `TracerProvider` with an OTLP HTTP exporter, converts apcore span attributes (including `apcore.trace_id`, `apcore.span_id`, `apcore.parent_span_id` for correlation), replays events, and maps status codes. Non-primitive attributes are stringified for OTel compatibility.

### Metrics Architecture

`MetricsCollector` maintains three internal dictionaries protected by a single lock:
- `_counters`: Maps `(name, labels_tuple)` to integer counts.
- `_histogram_sums`/`_histogram_counts`: Maps `(name, labels_tuple)` to sum/count values.
- `_histogram_buckets`: Maps `(name, labels_tuple, bucket_boundary)` to bucket counts, including a `+Inf` bucket that is always incremented.

`MetricsMiddleware` uses a stack (`context.data["_apcore.mw.metrics.starts"]`) to track start times for nested calls. In `after()`, it pops the start time, computes duration, and records success metrics. In `on_error()`, it additionally extracts the error code (from `ModuleError.code` or `type(error).__name__`).

### Logging Architecture

`ContextLogger` supports two output formats:
- **JSON**: Emits a single JSON object per line with fields: `timestamp`, `level`, `message`, `trace_id`, `module_id`, `caller_id`, `logger`, `extra`.
- **Text**: Emits formatted lines: `{timestamp} [{LEVEL}] [trace={trace_id}] [module={module_id}] {message} {extras}`.

Redaction applies to any key in `extra` that starts with `_secret_`, replacing the value with `***REDACTED***`.

`ObsLoggingMiddleware` wraps `ContextLogger` and uses the same stack-based timing pattern as `MetricsMiddleware` via `context.data["_apcore.mw.logging.starts"]`.

### Recommended Registration Order

As documented in the package `__init__.py`:
1. `TracingMiddleware` -- Captures total wall-clock time (outermost).
2. `MetricsMiddleware` -- Captures execution timing.
3. `ObsLoggingMiddleware` -- Logs with timing already set up (innermost).

### W3C Trace Context

apcore supports [W3C Trace Context](https://www.w3.org/TR/trace-context/) for distributed tracing interoperability. The `TraceContext` class provides methods to inject and extract `traceparent` headers, enabling trace propagation across service boundaries.

| Method | Description |
|--------|-------------|
| `TraceContext.inject(context)` | Convert an apcore `Context` into a headers dict (`dict[str, str]` / `Record<string, string>`) containing a `traceparent` key |
| `TraceContext.extract(headers)` | Parse a `traceparent` header from an incoming request headers dict and return a `TraceParent` |
| `TraceContext.from_traceparent(str)` | Strict parsing of a `traceparent` string into a `TraceParent` object |

Integration with `Context.create()`:

=== "Python"
    ```python
    from apcore import Context
    from apcore.observability import TraceContext

    # Extract trace parent from incoming request
    trace_parent = TraceContext.extract(request.headers)

    # Create context with propagated trace
    context = Context.create(trace_parent=trace_parent)

    # Inject trace parent into outgoing request headers
    outgoing_headers = TraceContext.inject(context)
    # outgoing_headers = {"traceparent": "00-<trace_id>-<span_id>-01"}
    ```
=== "TypeScript"
    ```typescript
    import { Context } from "apcore-js/context";
    import { TraceContext } from "apcore-js/observability";

    // Extract trace parent from incoming request
    const traceParent = TraceContext.extract(request.headers);

    // Create context with propagated trace
    const context = Context.create({ traceParent });

    // Inject trace parent into outgoing request headers
    const outgoingHeaders = TraceContext.inject(context);
    // outgoingHeaders = { traceparent: "00-<trace_id>-<span_id>-01" }
    ```
=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::observability::TraceContext;

    // Extract trace parent from incoming request
    let trace_parent = TraceContext::extract(&request.headers)?;

    // Create context with propagated trace
    let context = Context::create(None, Some(trace_parent));

    // Inject trace parent into outgoing request headers
    let outgoing_headers = TraceContext::inject(&context);
    // outgoing_headers = {"traceparent": "00-<trace_id>-<span_id>-01"}
    ```

The `traceparent` header follows the W3C format: `{version}-{trace_id}-{parent_id}-{trace_flags}`.

#### W3C Alignment Rules (Issue #35)

The following rules harden `TraceContext.extract` / `TraceContext.inject` for production
W3C interoperability across Python, TypeScript, and Rust SDKs.

##### Tracestate Propagation

- Implementations MUST parse the `tracestate` header alongside `traceparent` during
  `extract()`. Each entry is a `key=value` pair separated by `,`. Whitespace around the
  separators MUST be tolerated (per W3C Trace Context §3.3.1.1).
- The parsed result MUST be an ordered list of `(key, value)` pairs preserving the
  on-the-wire order of the originating header. Order is significant — the W3C spec uses
  position to identify the most recently mutating vendor.
- Implementations MUST cap retained entries at **32**; entries beyond the 32nd MUST be
  dropped from the head of the list per the W3C `tracestate` size limit.
- Malformed entries (missing `=`, empty key, or non-printable characters) MUST be dropped
  silently while leaving valid neighboring entries intact.
- `inject()` MUST serialize the stored list back to a `tracestate` header preserving order
  and entry count, so that an `extract → inject` round-trip is lossless for any input that
  already satisfies the rules above.

##### Case-Insensitive Header Lookup

- `extract()` MUST treat header keys case-insensitively. Callers may pass any of
  `traceparent`, `Traceparent`, `TRACEPARENT`, `tracestate`, `Tracestate`, or `TRACESTATE`
  and the lookup MUST succeed. This matches RFC 7230 §3.2 (HTTP header field names are
  case-insensitive) and the practical reality of WSGI/ASGI/Node/Actix header maps.
- The injected output of `inject()` MUST always use the canonical lowercase header keys
  `traceparent` and (when present) `tracestate`.

##### Dynamic Sampling Flag Honoring

- `extract()` MUST preserve the `trace_flags` byte from the incoming `traceparent` (`01`
  sampled, `00` unsampled). Implementations MUST NOT hardcode a sampling flag.
- `inject()` MUST emit the same `trace_flags` byte that was extracted, so that the upstream
  caller's sampling decision propagates downstream untouched.
- When no incoming `traceparent` exists and `inject()` is called against a fresh context,
  the implementation MAY choose its own flag according to the local sampling strategy
  (`full` → `01`, `off` → `00`, `proportional` / `error_first` → flag derived from the
  middleware's sampling decision for the current span).

##### Optional `parent_id` Override on `inject()`

- `inject()` MUST accept an optional `parent_id` argument. When provided, the value MUST
  match the regex `^[0-9a-f]{16}$` (16 lowercase hex characters). Non-matching values MUST
  raise an error with code `INVALID_PARENT_ID` (Python `ValueError`, TypeScript `Error`,
  Rust `Err(TraceContextError::InvalidParentId)`).
- When `parent_id` is omitted, implementations MUST derive the parent id automatically
  from the top-of-stack span (`context.data["_apcore.mw.tracing.spans"][-1].span_id` if
  present, otherwise a freshly generated 16-hex value).
- The override is intended for callers that wrap apcore with their own span manager and
  need to anchor the outgoing `traceparent` to a span id apcore did not create.

=== "Python"
    ```python
    from apcore import Context
    from apcore.trace_context import TraceContext

    # Case-insensitive extract preserves tracestate order and flags
    incoming = {
        "Traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
        "TRACESTATE": "vendor1=opaque1,vendor2=opaque2",
    }
    trace_parent = TraceContext.extract(incoming)
    assert trace_parent is not None
    assert trace_parent.trace_flags == "00"  # honored, not hardcoded

    # Build a context that carries the propagated trace + state
    context = Context.create(trace_parent=trace_parent)

    # Inject with default (auto-derived) parent_id
    headers = TraceContext.inject(context)
    # headers == {
    #   "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-<auto>-00",
    #   "tracestate": "vendor1=opaque1,vendor2=opaque2",
    # }

    # Optional override: pin the outgoing parent_id to a caller-managed span
    headers = TraceContext.inject(context, parent_id="aaaaaaaaaaaaaaaa")
    assert headers["traceparent"].split("-")[2] == "aaaaaaaaaaaaaaaa"

    # Malformed override raises immediately
    try:
        TraceContext.inject(context, parent_id="ZZZZ")
    except ValueError as exc:
        assert "INVALID_PARENT_ID" in str(exc)
    ```
=== "TypeScript"
    ```typescript
    import { Context } from "apcore-js/context";
    import { TraceContext } from "apcore-js/trace-context";

    // Case-insensitive extract preserves tracestate order and flags
    const incoming: Record<string, string> = {
        Traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
        TRACESTATE: "vendor1=opaque1,vendor2=opaque2",
    };
    const traceParent = TraceContext.extract(incoming);
    if (traceParent === null) throw new Error("expected parent");
    console.assert(traceParent.traceFlags === "00"); // honored, not hardcoded

    // Build a context that carries the propagated trace + state
    const context = Context.create({ traceParent });

    // Inject with default (auto-derived) parent_id
    let headers = TraceContext.inject(context);
    // headers == {
    //   traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-<auto>-00",
    //   tracestate: "vendor1=opaque1,vendor2=opaque2",
    // }

    // Optional override: pin the outgoing parent_id
    headers = TraceContext.inject(context, { parentId: "aaaaaaaaaaaaaaaa" });
    console.assert(headers.traceparent.split("-")[2] === "aaaaaaaaaaaaaaaa");

    // Malformed override raises immediately
    try {
        TraceContext.inject(context, { parentId: "ZZZZ" });
    } catch (err) {
        console.assert((err as Error).message.includes("INVALID_PARENT_ID"));
    }
    ```
=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::trace_context::{TraceContext, TraceContextError};
    use std::collections::HashMap;

    // Case-insensitive extract preserves tracestate order and flags
    let mut incoming: HashMap<String, String> = HashMap::new();
    incoming.insert(
        "Traceparent".to_string(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00".to_string(),
    );
    incoming.insert(
        "TRACESTATE".to_string(),
        "vendor1=opaque1,vendor2=opaque2".to_string(),
    );

    let trace_parent = TraceContext::extract(&incoming).expect("present");
    assert_eq!(trace_parent.trace_flags, "00"); // honored, not hardcoded

    // Build a context that carries the propagated trace + state
    let context = Context::create(None, Some(trace_parent));

    // Inject with default (auto-derived) parent_id
    let headers = TraceContext::inject(&context, None);
    // headers["traceparent"] -> "00-4bf92f3577b34da6a3ce929d0e0e4736-<auto>-00"
    // headers["tracestate"]  -> "vendor1=opaque1,vendor2=opaque2"

    // Optional override: pin the outgoing parent_id
    let headers = TraceContext::inject(&context, Some("aaaaaaaaaaaaaaaa"));
    assert_eq!(headers["traceparent"].split('-').nth(2).unwrap(), "aaaaaaaaaaaaaaaa");

    // Malformed override returns an error
    match TraceContext::try_inject(&context, Some("ZZZZ")) {
        Err(TraceContextError::InvalidParentId) => {}
        _ => panic!("expected INVALID_PARENT_ID"),
    }
    ```

### Error History

`ErrorHistory` is a ring-buffer tracker for recent module errors, providing deduplication and per-module querying. It is automatically created and wired by `register_sys_modules()` when system modules are enabled.

**Architecture:**
- Uses a ring-buffer data structure with configurable per-module capacity (`max_entries_per_module`, default 50) and total capacity (`max_total_entries`, default 1000).
- Deduplication by `(code, message)` tuple — repeated errors increment `count` and update `last_occurred` instead of creating new entries.
- Thread-safe via locking.

**API:**

| Method | Description |
|--------|-------------|
| `record(error: ModuleError)` | Record an error instance with deduplication |
| `get(module_id) → list[ErrorEntry]` | Return entries for a module (newest first) |
| `get_all() → list[ErrorEntry]` | Return all entries sorted by `last_occurred` |

**ErrorEntry dataclass:**

| Field | Type | Description |
|-------|------|-------------|
| `module_id` | str | Source module |
| `code` | str | Error code |
| `message` | str | Error message |
| `ai_guidance` | str \| None | AI guidance from the error |
| `count` | int | Number of occurrences (deduplicated) |
| `first_occurred` | str | ISO timestamp of first occurrence |
| `last_occurred` | str | ISO timestamp of most recent occurrence |

**ErrorHistoryMiddleware** records `ModuleError` instances into `ErrorHistory` on every `on_error()` call. Generic exceptions (non-`ModuleError`) are ignored. The middleware never recovers from errors (always returns `None`).

### Usage Collector

`UsageCollector` is a thread-safe in-memory tracker for per-module call counting, latency measurement, and hourly trend data. It is automatically created and wired by `register_sys_modules()`.

**Architecture:**
- Hourly bucketed storage with configurable retention (`retention_hours`, default 168 = 7 days).
- Trend computation compares current period vs previous period: `stable`, `rising`, `declining`, `new`, `inactive`.
- Thread-safe via locking.

**API:**

| Method | Description |
|--------|-------------|
| `record(module_id, caller_id, latency_ms, success)` | Record a usage event |
| `get_summary(period="24h") → list[ModuleUsageSummary]` | Aggregated summary for all modules |
| `UsageCollector.get_module(module_id, period="24h") → ModuleUsageDetail` | Detailed usage with caller breakdown and hourly distribution |
| `get_latencies(module_id) → list[float]` | Raw latency values for p99 computation |

**UsageMiddleware** records usage in `before()` (start timestamp), `after()` (success + latency), and `on_error()` (failure + latency) hooks.

### Platform Notify Middleware

`PlatformNotifyMiddleware` is a threshold-based sensor that emits events when module error rates or latency exceed configured thresholds.

**Configuration:**

| Parameter | Default | Description |
|-----------|---------|-------------|
| `error_rate_threshold` | 0.1 (10%) | Error rate that triggers alert |
| `latency_p99_threshold_ms` | 5000.0 | p99 latency that triggers alert |

**Events emitted:**

| Event | Trigger |
|-------|---------|
| `apcore.error.threshold_exceeded` | Error rate >= threshold |
| `apcore.latency.threshold_exceeded` | p99 latency >= threshold |
| `apcore.health.recovered` | Recovery: error rate < threshold × 0.5 |

**Hysteresis:** Once an alert fires for a module, it will not re-fire until the module recovers below `threshold × 0.5`, then crosses the threshold again. This prevents alert storms.

## Usage

=== "Python"
    ```python
    from apcore import APCore
    from apcore.observability import (
        TracingMiddleware,
        MetricsMiddleware,
        ObsLoggingMiddleware,
        InMemoryExporter,
        MetricsCollector,
    )

    # Build observability stack
    exporter = InMemoryExporter()
    tracing = TracingMiddleware(exporter=exporter, strategy="proportional", sampling_rate=0.1)
    metrics = MetricsCollector()
    metrics_mw = MetricsMiddleware(collector=metrics)
    logging_mw = ObsLoggingMiddleware(log_inputs=True, log_outputs=True)

    # Register in recommended order (outermost first)
    client = APCore()
    client.use(tracing)
    client.use(metrics_mw)
    client.use(logging_mw)

    @client.module(id="math.add", description="Add two numbers")
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}

    client.call("math.add", {"a": 3, "b": 4})

    # Inspect collected spans and metrics
    spans = exporter.get_spans()
    prometheus_text = metrics.export_prometheus()
    print(prometheus_text)
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";
    import {
        TracingMiddleware,
        MetricsMiddleware,
        ObsLoggingMiddleware,
        InMemoryExporter,
        MetricsCollector,
    } from "apcore-js/observability";

    // Build observability stack
    const exporter = new InMemoryExporter();
    const tracing = new TracingMiddleware({ exporter, strategy: "proportional", samplingRate: 0.1 });
    const collector = new MetricsCollector();
    const metricsMw = new MetricsMiddleware({ collector });
    const loggingMw = new ObsLoggingMiddleware({ logInputs: true, logOutputs: true });

    // Register in recommended order (outermost first)
    const client = new APCore();
    client.use(tracing);
    client.use(metricsMw);
    client.use(loggingMw);

    client.module({
        id: "math.add",
        description: "Add two numbers",
        inputSchema: { type: "object", properties: { a: { type: "number" }, b: { type: "number" } } },
        outputSchema: { type: "object", properties: { sum: { type: "number" } } },
        execute: ({ a, b }: { a: number; b: number }) => ({ sum: a + b }),
    });

    await client.call("math.add", { a: 3, b: 4 });

    // Inspect collected spans and metrics
    const spans = exporter.getSpans();
    const prometheusText = collector.exportPrometheus();
    console.log(prometheusText);
    ```
=== "Rust"
    ```rust
    use apcore::APCore;
    use apcore::observability::{
        TracingMiddleware, MetricsMiddleware, ObsLoggingMiddleware,
        InMemoryExporter, MetricsCollector, SamplingStrategy,
    };
    use std::sync::Arc;

    // Build observability stack
    let exporter = Arc::new(InMemoryExporter::new(10_000));
    let tracing = TracingMiddleware::new(exporter.clone(), SamplingStrategy::Proportional(0.1));
    let collector = Arc::new(MetricsCollector::new());
    let metrics_mw = MetricsMiddleware::new(collector.clone());
    let logging_mw = ObsLoggingMiddleware::new(true, true);

    // Register in recommended order (outermost first)
    let mut client = APCore::new();
    client.use_middleware(Box::new(tracing));
    client.use_middleware(Box::new(metrics_mw));
    client.use_middleware(Box::new(logging_mw));

    // After calling modules, inspect results
    let spans = exporter.get_spans();
    let prometheus_text = collector.export_prometheus();
    println!("{}", prometheus_text);
    ```

## Dependencies

- `apcore.middleware.Middleware` -- Base class for all three observability middlewares.
- `apcore.context.Context` -- Provides `trace_id`, `caller_id`, `call_chain`, and `data` dict for per-call state.
- `apcore.errors.ModuleError` -- Used by `MetricsMiddleware` to extract structured error codes.
- An OpenTelemetry SDK is required only when the `OTLPExporter` is used; SDKs SHOULD lazy-load it and fail with a clear error if missing.

??? info "Python SDK reference"
    The following tables are **not protocol requirements** — they document the Python SDK's source layout and runtime dependencies for implementers/users of `apcore-python`.

    **Source files:**

    | File | Lines | Purpose |
    |------|-------|---------|
    | `src/apcore/observability/__init__.py` | 37 | Package re-exports and recommended middleware ordering |
    | `src/apcore/observability/tracing.py` | 293 | `Span`, `SpanExporter`, `StdoutExporter`, `InMemoryExporter`, `OTLPExporter`, `TracingMiddleware` |
    | `src/apcore/observability/metrics.py` | 195 | `MetricsCollector`, `MetricsMiddleware`, Prometheus export |
    | `src/apcore/observability/context_logger.py` | 170 | `ContextLogger`, `ObsLoggingMiddleware` |
    | `src/apcore/observability/error_history.py` | — | `ErrorHistory`, `ErrorEntry` |
    | `src/apcore/observability/usage.py` | — | `UsageCollector`, `UsageMiddleware`, `ModuleUsageSummary`, `ModuleUsageDetail` |
    | `src/apcore/middleware/error_history.py` | — | `ErrorHistoryMiddleware` |
    | `src/apcore/middleware/platform_notify.py` | — | `PlatformNotifyMiddleware` |

    **External dependencies:**

    - `collections` (stdlib) -- `deque` for bounded `InMemoryExporter`.
    - `dataclasses` (stdlib) -- `asdict()` for span serialization in `StdoutExporter`.
    - `threading` (stdlib) -- Locks for thread-safe `InMemoryExporter` and `MetricsCollector`.
    - `time` (stdlib) -- Wall-clock timing for span and middleware duration measurements.
    - `json` (stdlib) -- JSON serialization for `StdoutExporter` and `ContextLogger`.
    - `random` (stdlib) -- Proportional sampling decision in `TracingMiddleware`.
    - `opentelemetry-sdk` / `opentelemetry-exporter-otlp-proto-http` (optional) -- Required only for `OTLPExporter`. Lazy-imported at instantiation time with a clear `ImportError` message.

## Testing Strategy

### Tracing Tests (`tests/observability/test_tracing.py`)

- **Span dataclass**: Required field creation, 16-char hex span_id generation, defaults (end_time=None, status="ok", empty attributes/events/parent_span_id), and mutability of end_time/status.
- **StdoutExporter**: Validates JSON line output and presence of all required fields (trace_id, span_id, name, attributes, timing).
- **InMemoryExporter**: Tests export/get_spans/clear lifecycle, thread-safe concurrent export (10 threads x 100 spans), bounded deque behavior (oldest spans dropped at capacity), and default maxlen of 10,000.
- **SpanExporter protocol**: Verifies that both `StdoutExporter` and `InMemoryExporter` satisfy the `runtime_checkable` `SpanExporter` protocol.
- **OTLPExporter**: Tests `ImportError` when OpenTelemetry packages are missing, span-to-OTel conversion (timestamps, attributes, correlation IDs), error status mapping, event replay, None end_time handling, non-primitive attribute stringification, None parent_span_id skipping, and `shutdown()` delegation.
- **Sampling strategies**: Full (always), off (never), proportional (statistical test over 1000 iterations), error_first (always exports errors, proportional for successes), and sampling decision inheritance from parent context.
- **TracingMiddleware lifecycle**: before() creates span and pushes to stack, after() pops/finalizes/exports, on_error() pops/sets error status/exports, stack-based nested calls with parent-child relationships, span name convention, attribute inclusion, duration computation, and empty-stack guard (logs warning, returns None).

### Metrics Tests (`tests/observability/test_metrics.py`)

- **MetricsCollector.increment()**: Counter creation, same-label accumulation, different-label separation.
- **MetricsCollector.observe()**: Histogram recording (_sum, _count), correct bucket increments (only buckets >= value), always-increment +Inf bucket.
- **Snapshot and reset**: Snapshot returns dict with counters and histograms, reset clears all state.
- **Prometheus export**: Text format conventions, HELP/TYPE comment lines, +Inf bucket, _sum/_count suffixed lines.
- **Configuration**: Custom bucket boundaries respected.
- **Thread safety**: 100 threads x 100 increments producing correct total of 10,000.
- **Convenience methods**: `increment_calls()`, `increment_errors()`, `observe_duration()` map to correct metric names and labels.
- **MetricsMiddleware**: before() pushes start time, after() records success and duration, on_error() records error calls and error counts (with `ModuleError.code` and generic `type(error).__name__`), returns None from on_error(), and nested calls produce isolated independent metrics.

### Context Logger Tests (`tests/observability/test_context_logger.py`)

- **Creation**: Default settings, `from_context()` extraction (trace_id, module_id from call_chain[-1], caller_id), empty call_chain handling.
- **Level filtering**: Each level emits correctly, lower levels suppressed, full matrix test across all 6 levels.
- **JSON format**: Valid JSON output, all fields present (timestamp, level, message, trace_id, module_id, caller_id, logger, extra), non-serializable extras handled via `default=str`.
- **Text format**: Pattern matching for [LEVEL], [trace=...], [module=...], message, and key=val extras.
- **Redaction**: `_secret_` prefix keys redacted to `***REDACTED***`, no redaction when disabled.
- **Custom output**: Writing to custom `io.StringIO` target.
- **ObsLoggingMiddleware**: Is Middleware subclass, before() pushes start and logs, after() pops and logs completion with duration, on_error() pops and logs failure with error type, input/output logging toggles, stack-based nested calls (4 log entries for 2 nested calls), and auto-creates ContextLogger when None.

## Contract: Tracer.start_span

### Inputs
- `name` (str/string/&str, required) — span name; MUST NOT be empty
- `parent` (Span/SpanContext, optional) — parent span for distributed tracing; creates a root span when absent

### Errors
- No errors raised (span creation failures are silently swallowed and return a no-op span)

### Returns
- On success: `Span` — active span; MUST be ended with `.end()` or used as a context manager

### Properties
- async: false
- thread_safe: true
- pure: false (registers span in the active trace context)

## Contract: MetricsEmitter.record

### Inputs
- `metric_name` (str/string/&str, required) — metric key; MUST be a registered metric constant
- `value` (float/number/f64, required) — numeric measurement
- `labels` (dict/object/HashMap, optional) — dimensional labels for the metric

### Errors
- No errors raised (metric emission failures are silently swallowed)

### Returns
- On success: void/None/()

### Properties
- async: false
- thread_safe: true
- pure: false (side-effect: metric emitted to configured backend)

---

## Observability Hardening (Issue #43)

### 1.1 Pluggable Observability Storage

`ErrorHistory` and `MetricsCollector` currently use in-memory storage only. Production deployments need persistence through pluggable storage backends.

#### Normative Rules

- Implementations MUST define an `ObservabilityStore` interface/protocol/trait with the following methods: `record_error(entry)`, `get_errors(module_id?, limit?) → List[ErrorEntry]`, `record_metric(metric)`, `get_metrics(module_id?, metric_name?) → List[MetricPoint]`, `flush()`, `clear()`.
- Implementations MUST provide `InMemoryObservabilityStore` as the default backend.
- Implementations SHOULD provide `RedisObservabilityStore` and `SqlObservabilityStore` as optional backends.
- The store MUST be injected into `ErrorHistory` and `MetricsCollector` at construction time: `ErrorHistory(store=InMemoryObservabilityStore())`. It MUST NOT be set after construction.

=== "Python"
    ```python
    from apcore.observability import (
        ErrorHistory,
        MetricsCollector,
        InMemoryObservabilityStore,
        RedisObservabilityStore,
    )

    # Default: in-memory store
    history = ErrorHistory(store=InMemoryObservabilityStore())
    collector = MetricsCollector(store=InMemoryObservabilityStore())

    # Production: Redis-backed store
    redis_store = RedisObservabilityStore(
        host="redis.internal",
        port=6379,
        key_prefix="apcore:obs:",
        ttl_seconds=86400,
    )
    history = ErrorHistory(store=redis_store)
    collector = MetricsCollector(store=redis_store)
    ```
=== "TypeScript"
    ```typescript
    import {
        ErrorHistory,
        MetricsCollector,
        InMemoryObservabilityStore,
        RedisObservabilityStore,
    } from "apcore-js/observability";

    // Default: in-memory store
    const history = new ErrorHistory({ store: new InMemoryObservabilityStore() });
    const collector = new MetricsCollector({ store: new InMemoryObservabilityStore() });

    // Production: Redis-backed store
    const redisStore = new RedisObservabilityStore({
        host: "redis.internal",
        port: 6379,
        keyPrefix: "apcore:obs:",
        ttlSeconds: 86400,
    });
    const historyProd = new ErrorHistory({ store: redisStore });
    const collectorProd = new MetricsCollector({ store: redisStore });
    ```
=== "Rust"
    ```rust
    use apcore::observability::{
        ErrorHistory, MetricsCollector,
        InMemoryObservabilityStore, RedisObservabilityStore,
    };
    use std::sync::Arc;

    // Default: in-memory store
    let store = Arc::new(InMemoryObservabilityStore::new());
    let history = ErrorHistory::new(store.clone());
    let collector = MetricsCollector::new(store.clone());

    // Production: Redis-backed store
    let redis_store = Arc::new(
        RedisObservabilityStore::new("redis://redis.internal:6379")
            .with_key_prefix("apcore:obs:")
            .with_ttl_seconds(86400),
    );
    let history_prod = ErrorHistory::new(redis_store.clone());
    let collector_prod = MetricsCollector::new(redis_store.clone());
    ```

---

### 1.2 BatchSpanProcessor for Non-Blocking OTEL Export

The current span exporter is synchronous — blocking the calling thread during each export. `BatchSpanProcessor` moves export to a background thread/task, keeping the hot path non-blocking.

#### Normative Rules

- Implementations MUST support a `BatchSpanProcessor` that buffers spans in an internal queue and exports them asynchronously in background batches.
- `BatchSpanProcessor` MUST have the following configurable parameters:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `max_queue_size` | 2048 | Maximum number of spans held in the buffer |
| `schedule_delay_ms` | 5000 | Delay between successive export attempts (milliseconds) |
| `max_export_batch_size` | 512 | Maximum spans per single export call |
| `export_timeout_ms` | 30000 | Deadline for the final flush on shutdown |

- When the queue is full, new spans MUST be dropped (not block) and a counter `spans_dropped` MUST be incremented.
- `BatchSpanProcessor` MUST flush all remaining buffered spans on shutdown, within the `export_timeout_ms` deadline. Spans not flushed within the deadline MUST be discarded.
- `SimpleSpanProcessor` (synchronous, immediate export) MUST remain available as an alternative for development and testing environments.

#### Processor Comparison

| Property | SimpleSpanProcessor | BatchSpanProcessor |
|----------|--------------------|--------------------|
| Use case | Development / testing | Production |
| Blocking | Yes — blocks caller per span | No — enqueues and returns immediately |
| Memory | O(1) — no buffer | O(max_queue_size) |
| Reliability | Guaranteed delivery (synchronous) | Best-effort (drops on full queue) |

#### Configuration (YAML)

```yaml
tracing:
  processor: "batch"
  batch:
    max_queue_size: 2048
    schedule_delay_ms: 5000
    max_export_batch_size: 512
    export_timeout_ms: 30000
```

=== "Python"
    ```python
    from apcore.observability import (
        BatchSpanProcessor,
        SimpleSpanProcessor,
        OTLPExporter,
        TracingMiddleware,
    )

    # Production: non-blocking batch export to OTLP endpoint
    exporter = OTLPExporter(endpoint="http://otel-collector:4318")
    processor = BatchSpanProcessor(
        exporter=exporter,
        max_queue_size=2048,
        schedule_delay_ms=5000,
        max_export_batch_size=512,
        export_timeout_ms=30000,
    )
    tracing = TracingMiddleware(processor=processor, strategy="proportional", sampling_rate=0.1)

    # Development: synchronous simple processor
    dev_processor = SimpleSpanProcessor(exporter=OTLPExporter(endpoint="http://localhost:4318"))
    dev_tracing = TracingMiddleware(processor=dev_processor, strategy="full")
    ```
=== "TypeScript"
    ```typescript
    import {
        BatchSpanProcessor,
        SimpleSpanProcessor,
        OTLPExporter,
        TracingMiddleware,
    } from "apcore-js/observability";

    // Production: non-blocking batch export to OTLP endpoint
    const exporter = new OTLPExporter({ endpoint: "http://otel-collector:4318" });
    const processor = new BatchSpanProcessor({
        exporter,
        maxQueueSize: 2048,
        scheduleDelayMs: 5000,
        maxExportBatchSize: 512,
        exportTimeoutMs: 30000,
    });
    const tracing = new TracingMiddleware({ processor, strategy: "proportional", samplingRate: 0.1 });

    // Development: synchronous simple processor
    const devProcessor = new SimpleSpanProcessor({
        exporter: new OTLPExporter({ endpoint: "http://localhost:4318" }),
    });
    const devTracing = new TracingMiddleware({ processor: devProcessor, strategy: "full" });
    ```
=== "Rust"
    ```rust
    use apcore::observability::{
        BatchSpanProcessor, SimpleSpanProcessor,
        OTLPExporter, TracingMiddleware, SamplingStrategy,
    };
    use std::sync::Arc;

    // Production: non-blocking batch export to OTLP endpoint
    let exporter = Arc::new(OTLPExporter::new("http://otel-collector:4318"));
    let processor = BatchSpanProcessor::builder(exporter.clone())
        .max_queue_size(2048)
        .schedule_delay_ms(5000)
        .max_export_batch_size(512)
        .export_timeout_ms(30000)
        .build();
    let tracing = TracingMiddleware::new(
        Box::new(processor),
        SamplingStrategy::Proportional(0.1),
    );

    // Development: synchronous simple processor
    let dev_processor = SimpleSpanProcessor::new(exporter.clone());
    let dev_tracing = TracingMiddleware::new(
        Box::new(dev_processor),
        SamplingStrategy::Full,
    );
    ```

---

### 1.3 O(log N) ErrorHistory Eviction with Min-Heap

The current ring-buffer eviction is O(M) where M = `max_total_entries`. At scale (millions of calls per day) this causes measurable latency spikes. Replacing the ring buffer with a min-heap keyed on `last_seen_at` reduces eviction cost to O(log N).

#### Normative Rules

- Implementations MUST maintain a min-heap of `ErrorEntry` objects keyed on `last_seen_at` timestamp.
- When the total entry count exceeds `max_total_entries`, the entry with the OLDEST `last_seen_at` MUST be evicted (min-heap pop). This is a normative data structure requirement, not a recommendation, because O(M) eviction causes measurable latency at production scale.
- Heap operations MUST be protected by a lock in multi-threaded environments.
- The public API (`record`, `get_errors`, `count`) MUST remain unchanged from the existing spec.

#### Data Structure

```
ErrorHistory:
  heap: min-heap[ErrorEntry] keyed on last_seen_at
  index: dict[module_id → list[ErrorEntry ref]]  # O(1) module lookup
```

The `index` provides O(1) lookup by `module_id` for `get_errors(module_id)` without requiring a heap scan. Both the heap and the index reference the same `ErrorEntry` objects; eviction removes from both structures atomically under the lock.

!!! note "Why this is a MUST, not a SHOULD"
    At 1M calls/day with `max_total_entries=1000`, eviction fires ~1000 times/day. O(1000) per eviction with a naive ring buffer amounts to 1M comparisons/day in the eviction path alone. The min-heap reduces this to ~10 comparisons per eviction. This difference is measurable in profiling at sustained high throughput.

---

### 1.4 Error Fingerprinting for Deduplication

Current deduplication is keyed on `(code, message)` tuple, which fails to deduplicate errors whose messages contain ephemeral values (UUIDs, timestamps, numeric IDs). Content-addressable fingerprinting normalizes these values before hashing.

#### Normative Rules

- Implementations MUST compute an error fingerprint as: `SHA-256(error_code + ":" + module_id + ":" + normalized_message)`, encoded as a 64-character lowercase hex string.
- `normalized_message` MUST be produced by the normalization algorithm below.
- When recording an error, if an entry with the same fingerprint already exists, implementations MUST increment its `count` and update its `last_seen_at`. Implementations MUST NOT create a duplicate entry.
- The fingerprint MUST be stored in `ErrorEntry` as a `fingerprint` field (64-char hex string).

#### Normalization Algorithm

```
normalize_message(msg):
  1. Replace UUID patterns (8-4-4-4-12 hex, optionally hyphenated) with <UUID>
  2. Replace integers > 3 digits with <ID>
  3. Replace ISO 8601 timestamps (date, datetime, datetime+timezone) with <TIMESTAMP>
  4. Strip leading/trailing whitespace
  5. Lowercase entire string
  Return normalized string
```

=== "Python"
    ```python
    import hashlib
    import re

    def normalize_message(msg: str) -> str:
        # Step 1: UUID patterns (with or without hyphens)
        msg = re.sub(
            r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
            "<UUID>", msg,
        )
        # Step 2: integers > 3 digits
        msg = re.sub(r"\b\d{4,}\b", "<ID>", msg)
        # Step 3: ISO 8601 timestamps
        msg = re.sub(
            r"\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?",
            "<TIMESTAMP>", msg,
        )
        return msg.strip().lower()

    def compute_fingerprint(error_code: str, module_id: str, message: str) -> str:
        normalized = normalize_message(message)
        raw = f"{error_code}:{module_id}:{normalized}"
        return hashlib.sha256(raw.encode("utf-8")).hexdigest()

    # Example usage
    fp = compute_fingerprint(
        "DB_TIMEOUT",
        "executor.db.query",
        "Connection to host 192.168.1.100 timed out after 30000ms (request-id: a1b2c3d4-e5f6-7890-abcd-ef1234567890)",
    )
    # normalized: "connection to host <id>.<id>.<id>.<id> timed out after <id>ms (request-id: <uuid>)"
    print(fp)  # 64-char hex string
    ```
=== "TypeScript"
    ```typescript
    import { createHash } from "crypto";

    function normalizeMessage(msg: string): string {
        // Step 1: UUID patterns
        msg = msg.replace(
            /[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/g,
            "<UUID>",
        );
        // Step 2: integers > 3 digits
        msg = msg.replace(/\b\d{4,}\b/g, "<ID>");
        // Step 3: ISO 8601 timestamps
        msg = msg.replace(
            /\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?/g,
            "<TIMESTAMP>",
        );
        return msg.trim().toLowerCase();
    }

    function computeFingerprint(errorCode: string, moduleId: string, message: string): string {
        const normalized = normalizeMessage(message);
        const raw = `${errorCode}:${moduleId}:${normalized}`;
        return createHash("sha256").update(raw, "utf8").digest("hex");
    }

    // Example usage
    const fp = computeFingerprint(
        "DB_TIMEOUT",
        "executor.db.query",
        "Connection to host 192.168.1.100 timed out after 30000ms (request-id: a1b2c3d4-e5f6-7890-abcd-ef1234567890)",
    );
    console.log(fp); // 64-char hex string
    ```
=== "Rust"
    ```rust
    use sha2::{Sha256, Digest};
    use regex::Regex;

    fn normalize_message(msg: &str) -> String {
        let uuid_re = Regex::new(
            r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
        ).unwrap();
        let id_re = Regex::new(r"\b\d{4,}\b").unwrap();
        let ts_re = Regex::new(
            r"\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?"
        ).unwrap();

        let msg = uuid_re.replace_all(msg, "<UUID>");
        let msg = id_re.replace_all(&msg, "<ID>");
        let msg = ts_re.replace_all(&msg, "<TIMESTAMP>");
        msg.trim().to_lowercase()
    }

    fn compute_fingerprint(error_code: &str, module_id: &str, message: &str) -> String {
        let normalized = normalize_message(message);
        let raw = format!("{}:{}:{}", error_code, module_id, normalized);
        let mut hasher = Sha256::new();
        hasher.update(raw.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    // Example usage
    let fp = compute_fingerprint(
        "DB_TIMEOUT",
        "executor.db.query",
        "Connection to host 192.168.1.100 timed out after 30000ms",
    );
    println!("{}", fp); // 64-char hex string
    ```

---

### 1.5 Configurable Redaction Rules

Currently redaction is driven solely by `x-sensitive: true` schema annotations. Runtime-configurable rules extend this to cover field name patterns and value patterns without requiring schema changes.

#### Normative Rules

- Implementations MUST support a `RedactionConfig` with three fields:
  - `field_patterns` — list of glob patterns matching field names to redact (e.g., `"*password*"`)
  - `value_patterns` — list of regex patterns matching field values to redact (e.g., `"^Bearer .*"`)
  - `replacement` — string substituted for redacted values; default `"***REDACTED***"`
- When logging inputs/outputs, the redaction engine MUST apply both schema-level (`x-sensitive`) and config-level (`RedactionConfig`) rules. The union of all matched fields and values is redacted.
- Implementations MUST NOT redact `trace_id`, `caller_id`, or `module_id` — these fields are required for observability correlation and MUST always appear in logs unmodified.

#### Configuration (YAML)

```yaml
observability:
  redaction:
    field_patterns:
      - "*password*"
      - "*token*"
      - "*secret*"
      - "*api_key*"
    value_patterns:
      - "^Bearer .*"
      - "^sk-[A-Za-z0-9]+"
    replacement: "***REDACTED***"
```

=== "Python"
    ```python
    from apcore.observability import RedactionConfig, ObsLoggingMiddleware

    redaction = RedactionConfig(
        field_patterns=["*password*", "*token*", "*secret*", "*api_key*"],
        value_patterns=[r"^Bearer .*", r"^sk-[A-Za-z0-9]+"],
        replacement="***REDACTED***",
    )
    logging_mw = ObsLoggingMiddleware(
        log_inputs=True,
        log_outputs=True,
        redaction_config=redaction,
    )
    ```
=== "TypeScript"
    ```typescript
    import { RedactionConfig, ObsLoggingMiddleware } from "apcore-js/observability";

    const redaction = new RedactionConfig({
        fieldPatterns: ["*password*", "*token*", "*secret*", "*api_key*"],
        valuePatterns: [/^Bearer .*/, /^sk-[A-Za-z0-9]+/],
        replacement: "***REDACTED***",
    });
    const loggingMw = new ObsLoggingMiddleware({
        logInputs: true,
        logOutputs: true,
        redactionConfig: redaction,
    });
    ```
=== "Rust"
    ```rust
    use apcore::observability::{RedactionConfig, ObsLoggingMiddleware};

    let redaction = RedactionConfig::builder()
        .field_patterns(vec!["*password*", "*token*", "*secret*", "*api_key*"])
        .value_patterns(vec![r"^Bearer .*", r"^sk-[A-Za-z0-9]+"])
        .replacement("***REDACTED***")
        .build();
    let logging_mw = ObsLoggingMiddleware::new(true, true)
        .with_redaction_config(redaction);
    ```

---

### 1.6 K8s/Prometheus Integration Hooks

#### Normative Rules

- Implementations MUST expose a `/metrics` HTTP endpoint returning Prometheus text format when `observability.prometheus.enabled: true` is configured.
- Implementations SHOULD expose a `/healthz` liveness endpoint and a `/readyz` readiness endpoint.
- The Prometheus `/metrics` endpoint MUST include the following standard apcore metrics: `apcore_module_calls_total`, `apcore_module_errors_total`, `apcore_module_duration_seconds` (histogram).
- Implementations SHOULD document the required K8s ServiceMonitor annotation `prometheus.io/scrape: "true"` so that Prometheus Operator can auto-discover the endpoint.

#### Configuration (YAML)

```yaml
observability:
  prometheus:
    enabled: true
    port: 9090
    path: "/metrics"
  health:
    liveness_path: "/healthz"
    readiness_path: "/readyz"
```

#### K8s ServiceMonitor annotations

```yaml
# Kubernetes Pod/Deployment annotation for Prometheus auto-discovery
annotations:
  prometheus.io/scrape: "true"
  prometheus.io/port: "9090"
  prometheus.io/path: "/metrics"
```

!!! tip "Wire the UsageCollector for full /metrics coverage"
    Attaching a `UsageCollector` to the `PrometheusExporter` is what makes
    the `apcore_usage_calls_total`, `apcore_usage_error_rate`, and
    `apcore_usage_p{50,95,99}_latency_ms` series appear on `/metrics`.
    Omitting it limits `/metrics` to the module-level metrics from the
    `MetricsCollector` only.

=== "Python"
    ```python
    from apcore import APCore
    from apcore.observability import MetricsCollector, PrometheusExporter, UsageCollector

    collector = MetricsCollector()
    usage_collector = UsageCollector()
    exporter = PrometheusExporter(collector=collector, usage_collector=usage_collector)

    # Start the metrics HTTP server (non-blocking, runs in background thread)
    exporter.start(port=9090, path="/metrics")

    # Health endpoints are served on the same port
    # GET /healthz → 200 OK  (liveness)
    # GET /readyz  → 200 OK  (readiness, after APCore finishes loading modules)

    client = APCore()
    client.configure_observability(prometheus_exporter=exporter)
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";
    import { MetricsCollector, PrometheusExporter, UsageCollector } from "apcore-js/observability";

    const collector = new MetricsCollector();
    const usageCollector = new UsageCollector();
    const exporter = new PrometheusExporter({ collector, usageCollector });

    // Start the metrics HTTP server (non-blocking)
    await exporter.start({ port: 9090, path: "/metrics" });

    // Health endpoints served on same port:
    // GET /healthz → 200 OK  (liveness)
    // GET /readyz  → 200 OK  (readiness)

    const client = new APCore();
    client.configureObservability({ prometheusExporter: exporter });
    ```
=== "Rust"
    ```rust
    use apcore::APCore;
    use apcore::observability::{MetricsCollector, PrometheusExporter, UsageCollector};
    use std::sync::Arc;

    let collector = Arc::new(MetricsCollector::new());
    let usage_collector = Arc::new(UsageCollector::new());
    let exporter = PrometheusExporter::new(collector.clone())
        .with_usage_collector(usage_collector.clone());

    // Start the metrics HTTP server (non-blocking, spawns background task)
    exporter.start(9090, "/metrics").await?;

    // Health endpoints served on same port:
    // GET /healthz → 200 OK  (liveness)
    // GET /readyz  → 200 OK  (readiness)

    let mut client = APCore::new();
    client.configure_observability(exporter);
    ```

## Contract: PrometheusExporter.export

### Inputs
- `collector` (MetricsCollector, required) — source of metrics data

### Errors
- None — export errors MUST be logged and MUST NOT propagate to callers

### Returns
- On success: str/string/String — Prometheus text exposition format, UTF-8

### Properties
- async: false
- thread_safe: true
- pure: false (reads from live collector state)
- idempotent: true

---

## Batch span processing

`BatchSpanProcessor` is the production-grade non-blocking span exporter. The deeper normative spec for queue/drop behavior, configuration, and lifecycle lives in [§1.2 BatchSpanProcessor for Non-Blocking OTEL Export](#12-batchspanprocessor-for-non-blocking-otel-export). This section restates the **cross-SDK parity contract** so SDK maintainers can verify their implementation at a glance.

### Cross-SDK parity

- All three SDKs (Python, TypeScript, Rust) MUST ship a non-blocking `BatchSpanProcessor` as part of the observability surface. Prior to Issue #43 only TypeScript and Rust shipped one; Python now reaches parity.
- The TypeScript and Rust BatchSpanProcessors are wired to the OpenTelemetry-API conventions for batch export. The Python BatchSpanProcessor MUST behave identically as a black box (same default tunables, same drop semantics, same flush/shutdown ordering) so cross-language conformance fixtures pass without per-SDK conditionals.
- `SimpleSpanProcessor` MUST also remain available in all three SDKs as the synchronous fallback for development and testing.

### Default tunables

Every SDK MUST default the four tunables to the values below. These match the upstream OpenTelemetry SDK defaults and are the values verified by the `observability_hardening.json` conformance fixture.

| Parameter | Default | Notes |
|-----------|---------|-------|
| `max_queue_size` | `2048` | Maximum spans buffered. New spans dropped when full. |
| `max_export_batch_size` | `512` | MUST be `<= max_queue_size`. Maximum spans per single export call. |
| `schedule_delay_ms` | `5000` | Worker idle delay between export attempts. |
| `export_timeout_ms` | `30000` | Final flush deadline on shutdown. |

### Lifecycle

The processor exposes three normative methods:

| Method | Behavior |
|--------|----------|
| `on_end(span)` | Called by `TracingMiddleware` when a span ends. MUST enqueue the span and return immediately (non-blocking). When the queue is at `max_queue_size`, the span MUST be dropped and `spans_dropped` MUST be incremented. |
| `force_flush(timeout_ms?)` | Drains the queue synchronously up to the optional timeout. MUST return `true` if the queue was fully drained within the deadline, `false` otherwise. Idempotent. |
| `shutdown(timeout_ms?)` | Calls `force_flush` with `export_timeout_ms` (or the supplied timeout), then stops the worker. After `shutdown` returns, `on_end` MUST treat further spans as dropped without enqueuing. |

### Cross-language usage

=== "Python"
    ```python
    from apcore.observability import (
        BatchSpanProcessor,
        OTLPExporter,
        TracingMiddleware,
    )

    exporter = OTLPExporter(endpoint="http://otel-collector:4318")
    processor = BatchSpanProcessor(
        exporter=exporter,
        max_queue_size=2048,
        max_export_batch_size=512,
        schedule_delay_ms=5000,
        export_timeout_ms=30000,
    )
    tracing = TracingMiddleware(processor=processor, strategy="proportional", sampling_rate=0.1)

    # Lifecycle:
    #   on_end(span) is called automatically by TracingMiddleware.after()
    #   force_flush() drains the queue (e.g. before a synchronous test assertion)
    #   shutdown() flushes within export_timeout_ms then stops the worker
    processor.force_flush()
    processor.shutdown()
    ```
=== "TypeScript"
    ```typescript
    import {
        BatchSpanProcessor,
        OTLPExporter,
        TracingMiddleware,
    } from "apcore-js/observability";

    const exporter = new OTLPExporter({ endpoint: "http://otel-collector:4318" });
    const processor = new BatchSpanProcessor({
        exporter,
        maxQueueSize: 2048,
        maxExportBatchSize: 512,
        scheduleDelayMs: 5000,
        exportTimeoutMs: 30000,
    });
    const tracing = new TracingMiddleware({ processor, strategy: "proportional", samplingRate: 0.1 });

    // Lifecycle:
    //   onEnd(span) is called automatically by TracingMiddleware.after()
    //   forceFlush() drains the queue before assertions in tests
    //   shutdown() flushes within exportTimeoutMs and stops the worker
    await processor.forceFlush();
    await processor.shutdown();
    ```
=== "Rust"
    ```rust
    use apcore::observability::{
        BatchSpanProcessor, OTLPExporter, TracingMiddleware, SamplingStrategy,
    };
    use std::sync::Arc;

    let exporter = Arc::new(OTLPExporter::new("http://otel-collector:4318"));
    let processor = BatchSpanProcessor::builder(exporter.clone())
        .max_queue_size(2048)
        .max_export_batch_size(512)
        .schedule_delay_ms(5000)
        .export_timeout_ms(30000)
        .build();
    let tracing = TracingMiddleware::new(
        Box::new(processor.clone()),
        SamplingStrategy::Proportional(0.1),
    );

    // Lifecycle:
    //   on_end(span) is called automatically by TracingMiddleware.after()
    //   force_flush() drains the queue before test assertions
    //   shutdown() flushes within export_timeout_ms then stops the worker task
    processor.force_flush(None).await;
    processor.shutdown(None).await;
    ```
