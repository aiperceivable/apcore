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
- `InMemoryExporter` must be bounded (deque with configurable maxlen, default 10,000) to prevent unbounded memory growth.

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
