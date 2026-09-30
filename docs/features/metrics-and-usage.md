---
description: "MetricsCollector and MetricsMiddleware, Prometheus export, per-module usage tracking and push export, threshold alerts, and pluggable storage backends."
---

# Metrics and Usage

> **Normative spec:** [protocol-spec §10.3 Metrics](../spec/protocol-spec.md#103-metrics), [§10.4 Usage Tracking](../spec/protocol-spec.md#104-usage-tracking) and [§6.7.1 usage output contract](../spec/protocol-spec.md#671-usage-module-output-contract). Tracing and logging are on the [Observability](./observability.md) page.

apcore has two collectors for call statistics:

- **`MetricsCollector`** — Prometheus-style counters and histograms, fed by `MetricsMiddleware`. Read by the `system.health.*` modules, `PlatformNotifyMiddleware` and `PrometheusExporter`.
- **`UsageCollector`** — per-module call records in hourly buckets, fed by `UsageMiddleware`. Read by the `system.usage.*` modules, `PrometheusExporter` and `PeriodicUsageExporter`.

Neither is configured from `apcore.yaml`: `observability.metrics.enabled` and `observability.metrics.exporter` are inert, and so are `sys_modules.usage.retention_hours` and `sys_modules.usage.bucketing_strategy` (see [Which config keys work](./observability.md#which-config-keys-work)). You construct the collectors and middlewares yourself, or let [system modules](./system-modules.md) create `UsageCollector`, `ErrorHistory` and their middlewares. `MetricsMiddleware` is never installed for you — add it when you want metrics.

## MetricsCollector

The collector holds counters and histograms keyed by metric name and label set, behind a lock. The standard apcore metrics ([§10.3](../spec/protocol-spec.md#103-metrics)):

| Metric | Type | Labels | Recorded by |
|---|---|---|---|
| `apcore_module_calls_total` | counter | `module_id`, `status` (`success` / `error`) | `increment_calls(module_id, status)` |
| `apcore_module_errors_total` | counter | `module_id`, `error_code` | `increment_errors(module_id, error_code)` |
| `apcore_module_duration_seconds` | histogram | `module_id` | `observe_duration(module_id, seconds)` |

| Method (Python / TypeScript) | Rust | Behaviour |
|---|---|---|
| `increment(name, labels, amount=1)` | `increment(name, labels, amount: f64)` | Add to a counter |
| `observe(name, labels, value)` | `observe(name, labels, value)` | Record a histogram observation; every bucket `>= value` and the `+Inf` bucket are incremented |
| `snapshot()` | `snapshot()` → `serde_json::Value` | Counters and histogram sums, counts and buckets |
| `reset()` | `reset()` | Clear all state |
| `export_prometheus()` / `exportPrometheus()` | `export_prometheus()` | Prometheus text exposition with `# HELP` / `# TYPE` lines, `_bucket`, `_sum`, `_count` |

Construction: Python `MetricsCollector(buckets=None, store=None, storage=None)`; TypeScript `new MetricsCollector({ buckets, store, storage })` or `new MetricsCollector([...buckets])`; Rust `MetricsCollector::new()` or `MetricsCollector::with_storage_backend(backend)`. The default histogram buckets (seconds) are `0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60`; custom buckets are Python and TypeScript only. A Rust `MetricsCollector` is a cheap handle over shared state, so clones feed the same counters.

**p99 from a histogram.** The health modules and `PlatformNotifyMiddleware` estimate p99 latency from the duration histogram by nearest rank over the bucket ladder. When every observation lies beyond the largest finite bucket, the estimate is that largest finite bound, never `0` (D-106) — a zero would report the slowest modules as the fastest and silence latency alerts for exactly those modules.

### MetricsMiddleware

`MetricsMiddleware(collector)` records one `apcore_module_calls_total` increment and one duration observation per call, and on failure also increments `apcore_module_errors_total` with the error's code (`ModuleError.code`, or the exception type name for other errors). Start times are kept per call, so nested calls are measured independently.

The [Observability quick start](./observability.md#quick-start) and the [Prometheus example](#prometheus-export) below show it registered on a client.

## Prometheus export

`PrometheusExporter` serves the collectors over HTTP for scraping. It is constructed and started by your application — no configuration key starts it (`observability.prometheus.*` does not exist).

| Endpoint | Response |
|---|---|
| `GET {path}` (default `/metrics`) | Prometheus text: the `MetricsCollector` families, plus the usage families below when a `UsageCollector` is attached |
| `GET /healthz` | `200 OK` — liveness |
| `GET /readyz` | `200 OK` after `mark_ready()` / `markReady()`, `503` before |

Attaching a `UsageCollector` adds `apcore_usage_calls_total` (by `module_id` and `status`), `apcore_usage_error_rate`, and `apcore_usage_p50_latency_ms` / `p95` / `p99`. The Rust exporter always emits the three standard `apcore_module_*` families, even before the first observation.

| | Construct | Start / stop |
|---|---|---|
| Python | `PrometheusExporter(collector, usage_collector=None)` | `start(port=9090, path="/metrics")` (background thread); `stop()` |
| TypeScript | `new PrometheusExporter({ collector, usageCollector })` | `start({ port, path })`; `await stop()` |
| Rust | `PrometheusExporter::new(collector).with_usage_collector(usage)` | `start(port, path).await?` (spawns a task; `io::Result`); `shutdown()`; `local_addr()` |

`export()` returns the same text the endpoint serves. It reads live collector state and does not raise.

=== "Python"
    ```python
    from apcore import APCore
    from apcore.observability import (
        MetricsCollector,
        MetricsMiddleware,
        PrometheusExporter,
        UsageCollector,
        UsageMiddleware,
    )

    metrics = MetricsCollector()
    usage = UsageCollector()

    client = APCore()
    client.use(MetricsMiddleware(metrics))
    client.use(UsageMiddleware(usage))

    exporter = PrometheusExporter(metrics, usage_collector=usage)
    exporter.start(port=9090, path="/metrics")
    exporter.mark_ready()  # /readyz answers 200 from now on

    # ... serve traffic ...

    exporter.stop()
    ```

=== "TypeScript"
    ```typescript
    import {
      APCore,
      MetricsCollector,
      MetricsMiddleware,
      PrometheusExporter,
      UsageCollector,
      UsageMiddleware,
    } from "apcore-js";

    const metrics = new MetricsCollector();
    const usage = new UsageCollector();

    const client = new APCore();
    client.use(new MetricsMiddleware(metrics));
    client.use(new UsageMiddleware(usage));

    const exporter = new PrometheusExporter({ collector: metrics, usageCollector: usage });
    exporter.start({ port: 9090, path: "/metrics" });
    exporter.markReady(); // /readyz answers 200 from now on

    // ... serve traffic ...

    await exporter.stop();
    ```

=== "Rust"
    ```rust
    use apcore::observability::{
        MetricsCollector, MetricsMiddleware, PrometheusExporter, UsageCollector, UsageMiddleware,
    };
    use apcore::APCore;

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let metrics = MetricsCollector::new();
        let usage = UsageCollector::new();

        let client = APCore::new();
        client.use_middleware(Box::new(MetricsMiddleware::new(metrics.clone())))?;
        client.use_middleware(Box::new(UsageMiddleware::new(usage.clone())))?;

        let exporter = PrometheusExporter::new(metrics).with_usage_collector(usage);
        exporter.start(9090, "/metrics").await?;
        exporter.mark_ready(); // /readyz answers 200 from now on

        // ... serve traffic ...

        exporter.shutdown();
        Ok(())
    }
    ```

For Prometheus Operator auto-discovery, annotate the pod:

```yaml
annotations:
  prometheus.io/scrape: "true"
  prometheus.io/port: "9090"
  prometheus.io/path: "/metrics"
```

## Usage collector

`UsageCollector` records one entry per call — caller, latency, success — into UTC hourly buckets, and aggregates them on demand for the `system.usage.summary` and `system.usage.module` modules. `UsageMiddleware(collector)` feeds it: it notes the start time in `before()` and records success or failure with the elapsed latency in `after()` / `on_error()`. A call with no caller identity is recorded under the caller id `"unknown"`.

| Operation | Python | TypeScript | Rust |
|---|---|---|---|
| Construct | `UsageCollector(retention_hours=168, max_records_per_bucket=10000, storage=None)` | `new UsageCollector({ retentionHours, maxRecordsPerBucket, storage })` | `UsageCollector::new()` / `with_storage_backend(backend)`; 168 hourly buckets retained |
| Record | `record(module_id, caller_id, latency_ms, success, timestamp=None)` | `record(moduleId, callerId, latencyMs, success, timestamp?)` | `record(module_id, caller_id: Option<&str>, latency_ms, success)`; `record_at(…, when)` |
| All modules | `get_summary(period="24h")` → `list[ModuleUsageSummary]` | `getSummary(period)` | `get_summary_for_period(Option<Duration>)` → `Vec<UsageStats>` |
| One module | `get_module(module_id, period="24h")` → `ModuleUsageDetail` | `getModule(moduleId, period)` | `get_module_summary_for_period(module_id, Option<Duration>)` |
| Raw latencies / p99 | `get_latencies(module_id, period="24h")` | `getLatencies(moduleId, period)` | `get_p99_latency_ms_for_period(module_id, Option<Duration>)` |

Recording never raises — it runs on every call. The values the usage modules report are pinned by [§6.7.1](../spec/protocol-spec.md#671-usage-module-output-contract):

| Concern | Rule |
|---|---|
| Hourly bucket key | `YYYY-MM-DDTHH` in UTC (e.g. `2026-03-08T14`), emitted verbatim as `hourly_distribution[].hour` |
| `period` | Matches `^[1-9][0-9]*[hd]$`; every accessor filters records to `[now − period, now]` ([§6.7.1.1](../spec/protocol-spec.md#6711-period-is-a-filter-not-an-echo)). An invalid period is rejected; at the module boundary it is `SCHEMA_VALIDATION_ERROR`. |
| p99 | Nearest rank: `sorted[min(ceil(0.99·N), N) − 1]`, no interpolation, `0` for no samples |
| Trend | Current vs previous period: `> 1.2` rising, `< 0.8` declining, `new` / `inactive` for the zero cases, otherwise `stable` |

A module with no recorded calls is not an error: it reports zero counts, a zero-filled `hourly_distribution` and the `new` / `inactive` trend classifications.

## UsageExporter (push-style)

`PrometheusExporter` is scraped; a `UsageExporter` pushes. `PeriodicUsageExporter` polls a `UsageCollector` on an interval and hands the summary to your exporter — an HTTP poster, a Kafka producer, a file writer. apcore ships only `NoopUsageExporter` (drops everything) and the periodic driver; transport-specific exporters are yours to write.

| | Exporter interface | Driver |
|---|---|---|
| Python | protocol with synchronous `export(summary: dict)` and `shutdown()`; `summary` is `{"modules": [...]}` for the last hour | `PeriodicUsageExporter(collector, exporter, interval_seconds=3600.0, period="1h")`; `await start()`, `await stop()` |
| TypeScript | `export(summary)` and `shutdown()`, each may return a `Promise`; `summary` is `{ modules: [...] }` for the last 24 hours | `new PeriodicUsageExporter(collector, exporter, intervalMs = 3_600_000)`; `start()`, `await stop()` |
| Rust | async trait: `export(&Value) -> Result<(), ModuleError>`, `shutdown()`; `summary` is a JSON array of every module's all-time `UsageStats` | `PeriodicUsageExporter::new(Arc<UsageCollector>, Arc<dyn UsageExporter>, Duration)`; `start().await`, `stop().await` |

- The first export happens one interval after `start()`; `start()` on a running driver does nothing.
- An `export()` failure is logged and the loop continues.
- `stop()` ends the loop and calls `exporter.shutdown()` exactly once. It is idempotent, and before `start()` it does nothing.

=== "Python"
    ```python
    import asyncio
    from typing import Any

    from apcore.observability import PeriodicUsageExporter, UsageCollector


    class PrintUsageExporter:
        def export(self, summary: dict[str, Any]) -> None:
            # Send to your sink here. Do not raise: failures should be logged.
            print(f"{len(summary['modules'])} modules")

        def shutdown(self) -> None:
            pass  # close connections, flush buffers


    async def main() -> None:
        collector = UsageCollector()
        periodic = PeriodicUsageExporter(collector, PrintUsageExporter(), interval_seconds=3600)
        await periodic.start()
        # ... application runs ...
        await periodic.stop()  # calls PrintUsageExporter.shutdown()


    asyncio.run(main())
    ```

=== "TypeScript"
    ```typescript
    import { PeriodicUsageExporter, UsageCollector } from "apcore-js";
    import type { UsageExporter } from "apcore-js";

    class PrintUsageExporter implements UsageExporter {
      async export(summary: Record<string, unknown>): Promise<void> {
        // Send to your sink here.
        console.log(summary);
      }

      async shutdown(): Promise<void> {
        // Close connections, flush buffers.
      }
    }

    const collector = new UsageCollector();
    const periodic = new PeriodicUsageExporter(collector, new PrintUsageExporter(), 3_600_000);
    periodic.start();
    // ... application runs ...
    await periodic.stop(); // awaits PrintUsageExporter.shutdown()
    ```

=== "Rust"
    ```rust
    use std::sync::Arc;
    use std::time::Duration;

    use apcore::errors::ModuleError;
    use apcore::observability::{PeriodicUsageExporter, UsageCollector, UsageExporter};
    use async_trait::async_trait;
    use serde_json::Value;

    struct PrintUsageExporter;

    #[async_trait]
    impl UsageExporter for PrintUsageExporter {
        async fn export(&self, summary: &Value) -> Result<(), ModuleError> {
            // Send to your sink here.
            println!("{summary}");
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), ModuleError> {
            // Close connections, flush buffers.
            Ok(())
        }
    }

    #[tokio::main]
    async fn main() {
        let collector = Arc::new(UsageCollector::new());
        let periodic = PeriodicUsageExporter::new(
            collector,
            Arc::new(PrintUsageExporter),
            Duration::from_secs(3600),
        );
        periodic.start().await;
        // ... application runs ...
        periodic.stop().await; // calls PrintUsageExporter::shutdown()
    }
    ```

## Threshold alerts

`PlatformNotifyMiddleware` watches a `MetricsCollector` and emits [events](./event-system.md) when a module crosses a threshold. System modules install it when `sys_modules.events.enabled` is true, with thresholds from `sys_modules.events.thresholds.error_rate` (default `0.1`) and `sys_modules.events.thresholds.latency_p99_ms` (default `5000`).

| Event | Emitted when |
|---|---|
| `apcore.health.error_threshold_exceeded` | the module's error rate reaches `error_rate` |
| `apcore.health.latency_threshold_exceeded` | the module's estimated p99 latency reaches `latency_p99_ms` |
| `apcore.health.recovered` | an alerted module's error rate falls below `error_rate × 0.5` |

An alert fires once per module and does not fire again until the module has recovered, so a module hovering at the threshold does not produce an alert storm. The error rate and p99 come from the `MetricsCollector`, so `MetricsMiddleware` must be installed for alerts to fire.

## Pluggable storage backends

`MetricsCollector`, `UsageCollector` and `ErrorHistory` persist their records through a small key/value interface, `StorageBackend`, so the same collectors can run against the bundled in-process store in tests and an external store in production.

| Method | Behaviour |
|---|---|
| `save(namespace, key, value)` | Create or overwrite a record |
| `get(namespace, key)` | The record, or `None` / `null` |
| `list(namespace, prefix)` | `(key, value)` pairs whose key starts with `prefix` (all when empty) |
| `delete(namespace, key)` | Remove a record; deleting an absent key is a no-op |

- `value` is always a **JSON object** (Python `dict`, TypeScript `Record<string, unknown>`, Rust `serde_json::Value` holding an object). A backend that stores bytes or strings serializes on `save` and parses on `get` / `list`, because the collectors index into what they read back.
- Namespaces isolate records: each collector writes under its own — `metrics`, `usage`, `error_history` (D-113).
- `InMemoryStorageBackend` is the default and is thread-safe. In Python and TypeScript an omitted `storage` argument means a fresh in-memory backend. In Rust, `with_storage_backend(None)` does the same, while the plain `new()` / `with_limits()` constructors attach no backend.
- The backend is fixed at construction.
- apcore ships no Redis, Postgres or S3 backend; implement the interface over your client library. Python's protocol is synchronous; TypeScript methods may return values or promises; Rust's trait is `async` and requires `Send + Sync + Debug`.

`MetricsCollector` and `ErrorHistory` additionally accept a `store` — an `ObservabilityStore` (`record_error`, `get_errors`, `record_metric`, `get_metrics`, `flush`, `clear`; default `InMemoryObservabilityStore`) that is notified of every recorded error and metric point. Use `StorageBackend` for persistence; `ObservabilityStore` is a record-oriented sink.

=== "Python"
    ```python
    import json
    from typing import Any

    from apcore.observability import (
        ErrorHistory,
        InMemoryStorageBackend,
        MetricsCollector,
        UsageCollector,
    )

    # One backend shared by all three collectors; namespaces keep them apart.
    backend = InMemoryStorageBackend()
    history = ErrorHistory(storage=backend)
    usage = UsageCollector(storage=backend)
    metrics = MetricsCollector(storage=backend)


    class RedisStorageBackend:
        """Not bundled: an example over a redis-py client."""

        def __init__(self, client: Any) -> None:
            self._client = client

        def save(self, namespace: str, key: str, value: dict) -> None:
            self._client.hset(namespace, key, json.dumps(value))

        def get(self, namespace: str, key: str) -> dict | None:
            raw = self._client.hget(namespace, key)
            return None if raw is None else json.loads(raw)

        def list(self, namespace: str, prefix: str = "") -> list[tuple[str, dict]]:
            out = []
            for k, v in self._client.hgetall(namespace).items():
                key = k.decode() if isinstance(k, bytes) else k
                if key.startswith(prefix):
                    out.append((key, json.loads(v)))
            return out

        def delete(self, namespace: str, key: str) -> None:
            self._client.hdel(namespace, key)
    ```

=== "TypeScript"
    ```typescript
    import { ErrorHistory, InMemoryStorageBackend, MetricsCollector, UsageCollector } from "apcore-js";
    import type { StorageBackend } from "apcore-js";

    // One backend shared by all three collectors; namespaces keep them apart.
    const backend = new InMemoryStorageBackend();
    const history = new ErrorHistory({ storage: backend });
    const usage = new UsageCollector({ storage: backend });
    const metrics = new MetricsCollector({ storage: backend });

    // Not bundled: an example over any client with hset/hget/hgetall/hdel.
    interface HashClient {
      hset(ns: string, key: string, value: string): Promise<unknown>;
      hget(ns: string, key: string): Promise<string | null>;
      hgetall(ns: string): Promise<Record<string, string>>;
      hdel(ns: string, key: string): Promise<unknown>;
    }

    class RedisStorageBackend implements StorageBackend {
      constructor(private readonly client: HashClient) {}

      async save(namespace: string, key: string, value: Record<string, unknown>): Promise<void> {
        await this.client.hset(namespace, key, JSON.stringify(value));
      }

      async get(namespace: string, key: string): Promise<Record<string, unknown> | null> {
        const raw = await this.client.hget(namespace, key);
        return raw === null ? null : (JSON.parse(raw) as Record<string, unknown>);
      }

      async list(namespace: string, prefix = ""): Promise<Array<[string, Record<string, unknown>]>> {
        const all = await this.client.hgetall(namespace);
        return Object.entries(all)
          .filter(([k]) => k.startsWith(prefix))
          .map(([k, v]): [string, Record<string, unknown>] => [k, JSON.parse(v) as Record<string, unknown>]);
      }

      async delete(namespace: string, key: string): Promise<void> {
        await this.client.hdel(namespace, key);
      }
    }
    ```

=== "Rust"
    ```rust
    use std::collections::HashMap;
    use std::sync::Arc;

    use apcore::observability::{
        ErrorHistory, InMemoryStorageBackend, MetricsCollector, StorageBackend, StorageError,
        UsageCollector,
    };
    use async_trait::async_trait;
    use serde_json::Value;
    use tokio::sync::Mutex;

    /// Not bundled: a sketch of a custom backend. A real one would wrap a
    /// Redis or SQL client and encode `value` to a string at this boundary.
    #[derive(Debug, Default)]
    struct MapStorageBackend {
        rows: Mutex<HashMap<(String, String), Value>>,
    }

    #[async_trait]
    impl StorageBackend for MapStorageBackend {
        async fn save(&self, namespace: &str, key: &str, value: Value) -> Result<(), StorageError> {
            self.rows.lock().await.insert((namespace.into(), key.into()), value);
            Ok(())
        }

        async fn get(&self, namespace: &str, key: &str) -> Result<Option<Value>, StorageError> {
            Ok(self.rows.lock().await.get(&(namespace.into(), key.into())).cloned())
        }

        async fn list(&self, namespace: &str, prefix: &str) -> Result<Vec<(String, Value)>, StorageError> {
            Ok(self
                .rows
                .lock()
                .await
                .iter()
                .filter(|((ns, k), _)| ns == namespace && k.starts_with(prefix))
                .map(|((_, k), v)| (k.clone(), v.clone()))
                .collect())
        }

        async fn delete(&self, namespace: &str, key: &str) -> Result<(), StorageError> {
            self.rows.lock().await.remove(&(namespace.into(), key.into()));
            Ok(())
        }
    }

    fn main() {
        // One backend shared by all three collectors; namespaces keep them apart.
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryStorageBackend::new());
        let history = ErrorHistory::with_storage_backend(50, 1000, Some(backend.clone()));
        let usage = UsageCollector::with_storage_backend(Some(backend.clone()));
        let metrics = MetricsCollector::with_storage_backend(Some(backend));

        let custom: Arc<dyn StorageBackend> = Arc::new(MapStorageBackend::default());
        let custom_history = ErrorHistory::with_storage_backend(50, 1000, Some(custom));
    }
    ```

## See also

- [Observability](./observability.md) — tracing, logging and the config-key table
- [Error History](./error-history.md) — the third collector that uses `StorageBackend`
- [System Modules](./system-modules.md) — `system.health.*` and `system.usage.*`
- [Event System](./event-system.md) — subscribing to `apcore.health.*` events
