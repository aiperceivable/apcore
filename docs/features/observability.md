---
description: "Observability overview: config-driven and programmatic tracing, sampling, span exporters and processors, W3C trace context propagation, and the ContextLogger."
---

# Observability

> **Normative spec:** [protocol-spec §10 Observability](../spec/protocol-spec.md#10-observability-specification). This page describes how the three SDKs implement it.

apcore instruments module calls through ordinary middleware, so observability plugs into the same [middleware pipeline](./middleware-system.md) as everything else. Each concern has one page:

| Concern | Main types | Page |
|---|---|---|
| Tracing — spans, sampling, exporters, processors, W3C propagation | `TracingMiddleware`, `BatchSpanProcessor`, `TraceContext` | this page |
| Structured logging | `ContextLogger`, `ObsLoggingMiddleware` | this page |
| Metrics, usage analytics, Prometheus, storage backends | `MetricsCollector`, `UsageCollector`, `PrometheusExporter`, `StorageBackend` | [Metrics and Usage](./metrics-and-usage.md) |
| Recent-error tracking and fingerprints | `ErrorHistory`, `ErrorHistoryMiddleware` | [Error History](./error-history.md) |
| Secret redaction in logs and captured I/O | `x-sensitive`, `obs.redaction.*`, `RedactionConfig` | [Redaction](./redaction.md) |

## Which config keys work

Only the keys below touch observability. The authoritative per-key status is `conformance/config_key_consumers.json`; every other spelling is undeclared and is rejected under `_config.strict`.

| Key | Status | Effect |
|---|---|---|
| `observability.tracing.enabled` | live | `true` installs a `TracingMiddleware` at client construction ([below](#tracing-from-configuration)) |
| `observability.tracing.strategy` | live | `full` (default) · `proportional` · `error_first` · `off` |
| `observability.tracing.sampling_rate` | live | Probability used by `proportional` and `error_first` (default `1.0`) |
| `observability.tracing.exporter` | live | `stdout` (default) · `otlp` · `jaeger` |
| `observability.tracing.otlp_endpoint` | live | OTLP target; only valid with `exporter: otlp` |
| `obs.redaction.sensitive_keys` · `regex_patterns` · `replacement` | live | See [Redaction](./redaction.md#configured-rules-obsredaction) |
| `sys_modules.error_history.max_entries_per_module` · `max_total_entries` | live | Limits of the auto-created `ErrorHistory` ([Error History](./error-history.md#configuration)) |
| `sys_modules.events.thresholds.error_rate` · `latency_p99_ms` | live | Alert thresholds of `PlatformNotifyMiddleware` ([Metrics and Usage](./metrics-and-usage.md#threshold-alerts)) |
| `observability.metrics.enabled` · `observability.metrics.exporter` | **inert** | Read by nothing; deprecated ([§9.2.4](../spec/protocol-spec.md#924-deprecated-configuration-keys)). Metrics come from a `MetricsCollector` you construct. |
| `logging.level` · `logging.format` | **inert** | Withdrawn ([§9.2.4](../spec/protocol-spec.md#924-deprecated-configuration-keys)). Pass level and format to `ContextLogger` directly. |
| `sys_modules.usage.retention_hours` · `sys_modules.usage.bucketing_strategy` | **inert** | Read by nothing. Retention is a `UsageCollector` constructor argument. |

Keys that do not exist and are often guessed: a top-level `tracing:` block, `observability.prometheus.*`, `observability.health.*`, `observability.redaction.*`, `obs.otel.*`.

## Quick start

Turn on tracing without writing code:

```yaml
# apcore.yaml
apcore:
  version: "1.0.0"

observability:
  tracing:
    enabled: true
    strategy: proportional
    sampling_rate: 0.1
    exporter: otlp
    otlp_endpoint: "http://otel-collector:4318/v1/traces"
```

Pass the loaded file to the client — Python `APCore(config=Config.load("apcore.yaml"))`, TypeScript `new APCore({ config: Config.load("apcore.yaml") })`, Rust `APCore::from_path("apcore.yaml")?`.

Or build the stack yourself. Register the middlewares outermost first — tracing, then metrics, then logging — so each one wraps the next:

=== "Python"
    ```python
    from apcore import APCore
    from apcore.observability import (
        InMemoryExporter,
        MetricsCollector,
        MetricsMiddleware,
        ObsLoggingMiddleware,
        TracingMiddleware,
    )

    exporter = InMemoryExporter()
    metrics = MetricsCollector()

    client = APCore()
    client.use(TracingMiddleware(exporter=exporter, sampling_rate=1.0, sampling_strategy="full"))
    client.use(MetricsMiddleware(metrics))
    client.use(ObsLoggingMiddleware(log_inputs=True, log_outputs=True))


    @client.module(id="math.add", description="Add two numbers")
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}


    client.call("math.add", {"a": 3, "b": 4})

    print(exporter.get_spans()[0].name)  # apcore.module.execute
    print(metrics.export_prometheus())
    ```

=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import {
      APCore,
      InMemoryExporter,
      MetricsCollector,
      MetricsMiddleware,
      ObsLoggingMiddleware,
      TracingMiddleware,
    } from "apcore-js";

    const exporter = new InMemoryExporter();
    const metrics = new MetricsCollector();

    const client = new APCore();
    client.use(new TracingMiddleware(exporter, 1.0, "full"));
    client.use(new MetricsMiddleware(metrics));
    client.use(new ObsLoggingMiddleware({ logInputs: true, logOutputs: true }));

    client.module({
      id: "math.add",
      description: "Add two numbers",
      inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
      outputSchema: Type.Object({ sum: Type.Number() }),
      execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
    });

    await client.call("math.add", { a: 3, b: 4 });

    console.log(exporter.getSpans()[0].name); // apcore.module.execute
    console.log(metrics.exportPrometheus());
    ```

=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use apcore::observability::{
        ContextLogger, InMemoryExporter, MetricsCollector, MetricsMiddleware,
        ObsLoggingMiddleware, SamplingStrategy, TracingMiddleware,
    };
    use apcore::APCore;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct AddModule;

    #[async_trait]
    impl Module for AddModule {
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "a": { "type": "integer" }, "b": { "type": "integer" } },
                "required": ["a", "b"]
            })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "sum": { "type": "integer" } } })
        }
        fn description(&self) -> &str {
            "Add two numbers"
        }
        async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let a = input["a"].as_i64().unwrap_or(0);
            let b = input["b"].as_i64().unwrap_or(0);
            Ok(json!({ "sum": a + b }))
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let exporter = InMemoryExporter::new();
        let metrics = MetricsCollector::new();

        let client = APCore::new();
        // InMemoryExporter and MetricsCollector are cheap handles over shared
        // state, so the clones handed to the middlewares feed the originals.
        client.use_middleware(Box::new(TracingMiddleware::with_sampling(
            Box::new(exporter.clone()),
            SamplingStrategy::Always,
            1.0,
        )))?;
        client.use_middleware(Box::new(MetricsMiddleware::new(metrics.clone())))?;
        client.use_middleware(Box::new(ObsLoggingMiddleware::with_options(
            ContextLogger::new("apcore.obs_logging"),
            true,
            true,
        )))?;
        client.register("math.add", Box::new(AddModule))?;

        client.call("math.add", json!({ "a": 3, "b": 4 }), None, None).await?;

        println!("{}", exporter.get_spans()[0].name); // apcore.module.execute
        println!("{}", metrics.export_prometheus());
        Ok(())
    }
    ```

## Tracing

### Tracing from configuration

When the loaded configuration sets `observability.tracing.enabled: true`, the client builds a `TracingMiddleware` from the five `observability.tracing.*` keys at construction and installs it. With the key absent or `false` (the default) nothing is installed. The full rules are [protocol-spec §10.1.1](../spec/protocol-spec.md#1011-tracing-from-configuration-observabilitytracing); in practice:

| `exporter` | What gets installed |
|---|---|
| `stdout` (default) | `StdoutExporter` — one JSON line per span on standard output |
| `otlp` | `OTLPExporter` sending to `otlp_endpoint`, or `http://localhost:4318/v1/traces` when that is `null` |
| `jaeger` | Nothing. A warning names the value and suggests `otlp` pointed at Jaeger's OTLP port. |

- `otlp_endpoint` set with any exporter other than `otlp` is rejected when the configuration loads, with `CONFIG_INVALID` naming both keys.
- If the named exporter cannot be built in this installation — `otlp` in Python without the OpenTelemetry packages, or in Rust without the `events` feature — the client logs a warning and installs **no** middleware rather than substituting another exporter.
- `strategy` and `sampling_rate` reach the sampling decision exactly as in [Sampling strategies](#sampling-strategies).
- Code wins over configuration. Configuration-driven tracing is skipped when you pass your own `Executor`, and a `TracingMiddleware` you add with `use()` / `use_middleware()` is kept. A `span_exporter` extension reconfigures the installed middleware's exporter instead of adding a second middleware.
- The in-memory exporter cannot be selected by name; it is for tests that construct it directly.

### Tracing architecture

`TracingMiddleware` opens a span in `before()` and closes it in `after()` or `on_error()`. Spans for nested module-to-module calls are kept on a per-trace stack, so each child span records its parent:

```text
TracingMiddleware.before("mod.a")        stack: [a]
  TracingMiddleware.before("mod.b")      stack: [a, b]    b.parent_span_id = a.span_id
  TracingMiddleware.after("mod.b")       stack: [a]       b ended, exported if sampled
TracingMiddleware.after("mod.a")         stack: []        a ended, exported if sampled
```

Python and TypeScript keep the stack in `context.data["_apcore.mw.tracing.spans"]` and the sampling decision in `context.data["_apcore.mw.tracing.sampled"]`; Rust keeps both inside the middleware, keyed by `trace_id`, and honours a decision already present under that `context.data` key. Either way every child call inherits the root's decision.

Every span follows the [§10.8 naming convention](../spec/protocol-spec.md#108-span-naming-convention):

| Field | Value |
|---|---|
| `name` | `apcore.module.execute` |
| `trace_id` | The context's 32-char lowercase hex trace id ([§10.5](../spec/protocol-spec.md#105-trace-id-format)) |
| `span_id` / `parent_span_id` | 16-char lowercase hex |
| `status` | `ok` or `error` |
| attributes | `module_id`, `method` (`"execute"`), `caller_id`, `duration_ms`, `success`, and `error_code` on failure |

### Sampling strategies

| `strategy` | Decision | `sampling_rate` |
|---|---|---|
| `full` (default) | every call chain is recorded | not consulted |
| `proportional` | recorded with probability `sampling_rate` | consulted |
| `error_first` | failed spans always exported; successful ones with probability `sampling_rate` | consulted |
| `off` | nothing is recorded | not consulted |

The decision is made once at the root of a call chain; child calls inherit it ([§10.7](../spec/protocol-spec.md#107-sampling-strategy)). Rust names the strategies with the `SamplingStrategy` enum — `Always` (`full`), `Probabilistic` (`proportional`), `ErrorFirst` (`error_first`), `Never` (`off`) — and serializes them to the configuration spellings.

### TracingMiddleware

| | Constructor |
|---|---|
| Python | `TracingMiddleware(exporter=None, sampling_rate=1.0, sampling_strategy="full", *, processor=None, priority=100)` — pass `exporter` (wrapped in a `SimpleSpanProcessor`) or `processor`; `sampling_rate` outside `[0, 1]` or an unknown strategy raises `ValueError` |
| TypeScript | `new TracingMiddleware(exporter, samplingRate = 1.0, samplingStrategy = "full")` — throws on an out-of-range rate or unknown strategy |
| Rust | `TracingMiddleware::new(exporter)` (`Always`) or `TracingMiddleware::with_sampling(exporter, strategy, rate)`; `exporter` is a `Box<dyn SpanExporter>` and `rate` is clamped to `[0, 1]` |

`set_exporter()` / `setExporter()` swaps the exporter of an already-registered middleware; the `span_exporter` extension point uses it.

### Span exporters

A span exporter receives finished spans. The SDKs ship:

| Exporter | Behaviour |
|---|---|
| `StdoutExporter` | Writes each span as one JSON line (Python/Rust: stdout; TypeScript: `console.info`). |
| `InMemoryExporter` | Bounded buffer for tests; `get_spans()` / `getSpans()` and `clear()`. Default capacity 10,000 in Python (`InMemoryExporter(max_spans=…)`) and TypeScript (`new InMemoryExporter(maxSpans)`), 1,000 in Rust (`InMemoryExporter::with_max_spans(n)`). |
| `OTLPExporter` | OTLP/HTTP. **Python** bridges to the OpenTelemetry SDK — `OTLPExporter(endpoint=None, service_name="apcore", attribute_allowlist=None)`, requires `opentelemetry-sdk` and `opentelemetry-exporter-otlp-proto-http`, and takes the full traces URL. **TypeScript** posts JSON with `fetch` — `new OTLPExporter({ endpoint, serviceName, headers, timeoutMs })`, full traces URL, default `http://localhost:4318/v1/traces`. **Rust** — `OTLPExporter::new(base_url)` appends `/v1/traces` itself and needs the `events` feature; without it every span is discarded with a warning. |
| `CompositeExporter` | Rust only: fans each span out to several exporters with per-exporter error isolation. |

A custom exporter implements `export(span)` (Python protocol `SpanExporter`, TypeScript interface `SpanExporter`, Rust async trait `SpanExporter` with `export` and `shutdown`).

### Span processors

A processor sits between the middleware and the exporter and decides when spans are exported.

| | `SimpleSpanProcessor` | `BatchSpanProcessor` |
|---|---|---|
| Use | development, tests | production |
| Export | synchronously, on the calling thread/task | in background batches |
| Memory | no buffer | up to `max_queue_size` spans |
| Queue full | — | span dropped, `spans_dropped` incremented; never blocks |

`BatchSpanProcessor` tunables and defaults (identical in all three SDKs):

| Parameter | Default | Meaning |
|---|---|---|
| `max_queue_size` | `2048` | Spans buffered before new ones are dropped |
| `schedule_delay_ms` | `5000` | Interval between background flushes |
| `max_export_batch_size` | `512` | Spans exported per flush |
| `export_timeout_ms` | `30000` | Deadline for the final flush in `shutdown()` |

Lifecycle methods:

| Method | Behaviour |
|---|---|
| `on_span_end(span)` (TypeScript `onSpan`) | Enqueue and return immediately; drop and count when the queue is full. Never raises. |
| `force_flush(timeout_ms = 30000)` (TypeScript `forceFlush`) | Drain the queue now; returns `true` if it emptied before the deadline, `false` otherwise. The processor keeps running. Python is synchronous; TypeScript and Rust are awaited. |
| `shutdown()` | Stop the background worker and flush what is left within `export_timeout_ms`. Python and Rust also call the exporter's `shutdown()`. |

Python passes a processor straight to `TracingMiddleware(processor=…)`. In Rust both processors implement `SpanExporter`, so they go where an exporter goes. TypeScript's `TracingMiddleware` takes an exporter, so wrap the processor in a one-line adapter:

=== "Python"
    ```python
    from apcore.observability import BatchSpanProcessor, OTLPExporter, TracingMiddleware

    processor = BatchSpanProcessor(
        exporter=OTLPExporter(endpoint="http://otel-collector:4318/v1/traces"),
        max_queue_size=2048,
        schedule_delay_ms=5000,
        max_export_batch_size=512,
        export_timeout_ms=30000,
    )
    tracing = TracingMiddleware(processor=processor, sampling_rate=0.1, sampling_strategy="proportional")

    # ... register `tracing` with client.use(tracing) and serve traffic ...

    processor.force_flush()  # drain now, e.g. before a test assertion
    processor.shutdown()  # final flush within export_timeout_ms
    ```

=== "TypeScript"
    ```typescript
    import { BatchSpanProcessor, OTLPExporter, TracingMiddleware } from "apcore-js";

    const processor = new BatchSpanProcessor({
      exporter: new OTLPExporter({ endpoint: "http://otel-collector:4318/v1/traces" }),
      maxQueueSize: 2048,
      scheduleDelayMs: 5000,
      maxExportBatchSize: 512,
      exportTimeoutMs: 30000,
    });
    const tracing = new TracingMiddleware(
      { export: (span) => processor.onSpan(span) },
      0.1,
      "proportional",
    );

    // ... register `tracing` with client.use(tracing) and serve traffic ...

    await processor.forceFlush(); // drain now, e.g. before a test assertion
    await processor.shutdown(); // final flush within exportTimeoutMs
    ```

=== "Rust"
    ```rust
    use std::sync::Arc;

    use apcore::errors::ModuleError;
    use apcore::observability::{
        BatchSpanProcessor, OTLPExporter, SamplingStrategy, SpanProcessor, TracingMiddleware,
    };

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let processor = BatchSpanProcessor::builder(Arc::new(OTLPExporter::new("http://otel-collector:4318")))
            .max_queue_size(2048)
            .schedule_delay_ms(5000)
            .max_export_batch_size(512)
            .export_timeout_ms(30000)
            .build();
        let tracing = TracingMiddleware::with_sampling(
            Box::new(processor.clone()),
            SamplingStrategy::Probabilistic,
            0.1,
        );

        // ... register `tracing` with client.use_middleware(Box::new(tracing))? ...
        drop(tracing);

        processor.force_flush(30_000).await; // drain now, e.g. before a test assertion
        processor.shutdown().await?; // final flush within export_timeout_ms
        Ok(())
    }
    ```

!!! warning "Rust: keep the `BatchSpanProcessor` handle alive"
    Clones share one queue and one worker, and dropping **any** clone signals the worker to stop. Keep the handle you created for as long as the middleware runs, then call `shutdown()`.

### W3C trace context propagation

`TraceContext` carries a trace across process boundaries with the W3C `traceparent` / `tracestate` headers (`traceparent: 00-{trace_id}-{parent_id}-{trace_flags}`). Pass an extracted `TraceParent` to `Context.create` so the call joins the upstream trace; inject headers into outgoing requests. Behaviour, pinned by `conformance/fixtures/trace_context.json`:

- **Header names are case-insensitive** on extraction (`traceparent`, `Traceparent`, `TRACEPARENT` …); injected headers always use the lowercase names.
- **`trace_flags` is preserved**: the flags byte extracted from an inbound `traceparent` is what `inject()` emits for that trace. A root context emits `01`.
- **`tracestate` is kept in order**, capped at 32 entries; malformed entries are dropped without affecting their neighbours; an extract → inject round trip is lossless.
- **`parent_id` override**: `inject()` accepts an explicit 16-lowercase-hex parent id for callers that manage their own spans. A malformed override fails with `INVALID_PARENT_ID`. Without an override, Python and TypeScript use the span on top of the tracing stack (or a fresh id); Rust generates a fresh id.
- A missing or malformed inbound `traceparent` — bad syntax, version `ff`, all-zero trace or parent id — extracts as `None` / `null`; `Context.create` then generates a new trace id ([§10.5](../spec/protocol-spec.md#105-trace-id-format)).

=== "Python"
    ```python
    from apcore import Context, InvalidParentIdError, TraceContext

    incoming = {
        "Traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
        "TRACESTATE": "vendor1=opaque1,vendor2=opaque2",
    }
    trace_parent = TraceContext.extract(incoming)
    assert trace_parent is not None and trace_parent.trace_flags == "00"

    context = Context.create(trace_parent=trace_parent)

    headers = TraceContext.inject(context)
    # {"traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-<new parent>-00",
    #  "tracestate": "vendor1=opaque1,vendor2=opaque2"}

    headers = TraceContext.inject(context, parent_id="aaaaaaaaaaaaaaaa")
    assert headers["traceparent"].split("-")[2] == "aaaaaaaaaaaaaaaa"

    try:
        TraceContext.inject(context, parent_id="ZZZZ")
    except InvalidParentIdError as exc:
        assert exc.code == "INVALID_PARENT_ID"
    ```

=== "TypeScript"
    ```typescript
    import { Context, TraceContext } from "apcore-js";

    const incoming: Record<string, string> = {
      Traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
      TRACESTATE: "vendor1=opaque1,vendor2=opaque2",
    };
    const traceParent = TraceContext.extract(incoming);
    if (traceParent === null || traceParent.traceFlags !== "00") throw new Error("unexpected");

    const context = Context.create(null, traceParent);

    let headers = TraceContext.inject(context);
    // { traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-<new parent>-00",
    //   tracestate: "vendor1=opaque1,vendor2=opaque2" }

    headers = TraceContext.inject(context, "aaaaaaaaaaaaaaaa");
    console.assert(headers.traceparent.split("-")[2] === "aaaaaaaaaaaaaaaa");

    try {
      TraceContext.inject(context, "ZZZZ");
    } catch (err) {
      console.assert((err as { code?: string }).code === "INVALID_PARENT_ID");
    }
    ```

=== "Rust"
    ```rust
    use std::collections::HashMap;

    use apcore::context::Context;
    use apcore::errors::ErrorCode;
    use apcore::TraceContext;
    use serde_json::Value;

    fn main() {
        let mut incoming: HashMap<String, String> = HashMap::new();
        incoming.insert(
            "Traceparent".into(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00".into(),
        );
        incoming.insert("TRACESTATE".into(), "vendor1=opaque1,vendor2=opaque2".into());

        // `extract_context` parses traceparent AND tracestate; `extract` parses traceparent only.
        let trace_parent = TraceContext::extract_context(&incoming).expect("valid header").traceparent;
        assert_eq!(trace_parent.trace_flags, 0x00);

        let context: Context<Value> = Context::create(None, Some(trace_parent), None, None, Value::Null, None);

        let headers = TraceContext::inject(&context);
        assert_eq!(headers["tracestate"], "vendor1=opaque1,vendor2=opaque2");

        let headers = TraceContext::inject_checked(&context, Some("aaaaaaaaaaaaaaaa"), None, None).unwrap();
        assert_eq!(headers["traceparent"].split('-').nth(2), Some("aaaaaaaaaaaaaaaa"));

        let err = TraceContext::inject_checked(&context, Some("ZZZZ"), None, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParentId);
    }
    ```

#### TraceContext API

| Operation | Python | TypeScript | Rust | Result / errors |
|---|---|---|---|---|
| Parse headers | `TraceContext.extract(headers)` | `TraceContext.extract(headers)` (plain object, `Headers` or `Map`) | `TraceContext::extract(&headers)`; `extract_context(&headers)` also fills `tracestate` | `TraceParent` or `None`/`null`; never raises |
| Build headers | `TraceContext.inject(context, parent_id=None)` | `TraceContext.inject(context, parentId?)` | `inject(&ctx)`; `inject_checked(&ctx, parent_id, trace_flags, tracestate)` | `traceparent` (+ `tracestate` when the context carries one). Bad override: Python `InvalidParentIdError` (a `ValueError`), TypeScript `Error` with `code = "INVALID_PARENT_ID"`, Rust `inject_checked` → `Err` with `ErrorCode::InvalidParentId` |
| Strict parse of one `traceparent` value | `TraceContext.from_traceparent(value)` | `TraceContext.fromTraceparent(value)` | `TraceParent::parse(value)` | `TraceParent` without `tracestate`; malformed input raises a codeless `ValueError` / `Error` (Rust: `Err` with `GENERAL_INVALID_INPUT`) |

`TraceParent` carries `version`, `trace_id`, `parent_id`, `trace_flags` and the ordered `tracestate` pairs; Rust stores `version` and `trace_flags` as `u8`, the other SDKs as two-character hex strings. Rust also has `inject_with_options`, which takes the same overrides as `inject_checked` but silently replaces a malformed `parent_id` with a fresh one — prefer `inject_checked`. All of these are synchronous and thread-safe.

## Logging

### ContextLogger

`ContextLogger` is a standalone structured logger that stamps every record with the call's correlation fields ([§10.2](../spec/protocol-spec.md#102-logging)).

- **Levels:** `trace`, `debug`, `info`, `warn`, `error`, `fatal` (default threshold `info`).
- **JSON format** (default): one object per line with `timestamp`, `level`, `message`, `trace_id`, `module_id`, `caller_id`, `logger` and an `extra` object.
- **Text format:** `{timestamp} [{LEVEL}] [trace={trace_id}] [module={module_id}] {message} {extras}` in Python and TypeScript; Rust writes `{timestamp} {LEVEL} {logger} [trace={trace_id} module={module_id}] {message}` without extras.
- **Output:** standard error by default (TypeScript: `console.error`); replaceable for tests.
- **`from_context(context, name)`** copies `trace_id`, `caller_id` and the last entry of `call_chain` (as `module_id`) from a `Context`.
- **Redaction:** in Python and TypeScript, `extra` is redacted recursively with a [`RedactionConfig`](./redaction.md#redactionconfig-api) — the default `sensitive_keys` list unless you pass one (`redaction_config=` / `redaction:`); `redact_sensitive=False` turns it off. Rust's `ContextLogger` redacts only top-level `extra` keys starting with `_secret_`; attach a `RedactionConfig` to `ObsLoggingMiddleware` for full rules.

| | Construction |
|---|---|
| Python | `ContextLogger(name="apcore", *, output_format="json", level="info", redact_sensitive=True, output=None, redaction_config=None)`; `ContextLogger.from_context(context, name, **kwargs)` |
| TypeScript | `new ContextLogger({ name, format, level, redactSensitive, redaction, output })`; `ContextLogger.fromContext(context, name, options)` |
| Rust | `ContextLogger::new(name)` then `set_level`, `set_format(LogFormat::Text)`, `set_writer`; `ContextLogger::from_context(&ctx, name)`; `emit(level, message, Some(&extra))` logs with extra fields |

`logging.level` and `logging.format` in `apcore.yaml` do not configure this logger — they are inert.

### ObsLoggingMiddleware

`ObsLoggingMiddleware` writes one record when a call starts, one when it completes (with `duration_ms`) and one when it fails (with the error), through a `ContextLogger`, optionally including inputs and outputs. Timings are tracked per call, so nested calls are measured independently.

What it logs as inputs and outputs:

- Python logs `context.redacted_inputs` (the executor's capture, see [Redaction](./redaction.md#where-redaction-applies)) and the raw output; TypeScript logs `context.redactedInputs` and `context.redactedOutput`. The logger's own pass then applies its `RedactionConfig`.
- Rust logs the raw inputs and output, redacted only by a `RedactionConfig` attached with `with_redaction_config` — attach one in production.
- An attached `RedactionConfig` is applied to the logged inputs/outputs and, in Python and TypeScript, also replaces the logger's own rules so the whole record uses one rule set.

| | Construction |
|---|---|
| Python | `ObsLoggingMiddleware(logger=None, log_inputs=True, log_outputs=True, redaction_config=None)` |
| TypeScript | `new ObsLoggingMiddleware({ logger, logInputs, logOutputs, redactionConfig })` |
| Rust | `ObsLoggingMiddleware::new(logger)` or `::with_options(logger, log_inputs, log_outputs)`, then `.with_redaction_config(config)` |

### Recommended middleware order

Register outermost first; with equal priorities the registration order is the execution order:

1. `TracingMiddleware` — covers total wall-clock time.
2. `MetricsMiddleware` — records counts and duration.
3. `ObsLoggingMiddleware` — logs with timing already set up.

`ErrorHistoryMiddleware`, `UsageMiddleware` and `PlatformNotifyMiddleware` are registered for you when system modules are enabled; see [System Modules](./system-modules.md).

## SDK notes

- Python needs no extra packages except for `OTLPExporter`, which imports `opentelemetry-sdk` and `opentelemetry-exporter-otlp-proto-http` at construction and raises `ImportError` with an install hint when they are missing.
- TypeScript's `OTLPExporter` uses the platform `fetch`; exports are fire-and-forget with a timeout (`timeoutMs`, default 5000).
- Rust's OTLP export requires the crate's `events` feature. Middleware hooks are `async`; `use_middleware` returns a `Result`.

## See also

- [Metrics and Usage](./metrics-and-usage.md) · [Error History](./error-history.md) · [Redaction](./redaction.md)
- [Middleware System](./middleware-system.md) — hook semantics and priorities
- [Context Object](./context-object.md) — `trace_id`, `call_chain`, `data`
- [Event System](./event-system.md) — `apcore.health.*` alert events
- [Observability cookbook](../guides/cookbook-observability.md)
