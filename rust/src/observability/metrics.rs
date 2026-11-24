// APCore Protocol — Metrics collection
// Spec reference: Execution metrics and metrics middleware

use async_trait::async_trait;
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;

use crate::context::Context;
use crate::errors::ModuleError;
use crate::middleware::base::Middleware;
use crate::observability::storage::StorageBackend;
use crate::observability::store::{InMemoryObservabilityStore, MetricPoint, ObservabilityStore};

/// Metric name for total module call count.
pub const METRIC_CALLS_TOTAL: &str = "apcore_module_calls_total";
/// Metric name for total module error count.
pub const METRIC_ERRORS_TOTAL: &str = "apcore_module_errors_total";
/// Metric name for module execution duration in seconds.
pub const METRIC_DURATION_SECONDS: &str = "apcore_module_duration_seconds";

/// Default histogram bucket boundaries matching Python reference.
pub(crate) const DEFAULT_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// Composite key for metric identification: (name, sorted labels).
type MetricKey = (String, BTreeMap<String, String>);

/// Internal histogram data.
#[derive(Debug, Clone)]
struct HistogramData {
    sum: f64,
    count: u64,
    buckets: Vec<(f64, u64)>, // (upper_bound, cumulative_count)
}

impl HistogramData {
    fn new() -> Self {
        let buckets = DEFAULT_BUCKETS.iter().map(|&b| (b, 0u64)).collect();
        Self {
            sum: 0.0,
            count: 0,
            buckets,
        }
    }

    fn observe(&mut self, value: f64) {
        self.sum += value;
        self.count += 1;
        for bucket in &mut self.buckets {
            if value <= bucket.0 {
                bucket.1 += 1;
            }
        }
    }
}

/// Collects and stores metrics counters and histogram observations.
///
/// Construction injects a `Arc<dyn ObservabilityStore>`; the default store is
/// `InMemoryObservabilityStore`. The store MUST NOT be replaced after
/// construction (observability.md §1.1). Every `increment`/`observe` call
/// also forwards a `MetricPoint` to the store, mirroring Python's
/// `MetricsCollector` reference implementation.
#[derive(Debug, Clone)]
pub struct MetricsCollector {
    counters: Arc<Mutex<HashMap<MetricKey, f64>>>,
    histograms: Arc<Mutex<HashMap<MetricKey, HistogramData>>>,
    store: Arc<dyn ObservabilityStore>,
    /// Issue #43 §1: optional `StorageBackend` for cross-process persistence.
    /// When set, every counter/histogram observation is also persisted under
    /// namespace `"metrics"` with a key derived from `(name, labels, ts)`.
    storage_backend: Option<Arc<dyn StorageBackend>>,
}

impl MetricsCollector {
    /// Create a new metrics collector with the default in-memory store.
    #[must_use]
    pub fn new() -> Self {
        Self::with_store(Arc::new(InMemoryObservabilityStore::new()))
    }

    /// Create a new metrics collector backed by the given observability store.
    #[must_use]
    pub fn with_store(store: Arc<dyn ObservabilityStore>) -> Self {
        Self {
            counters: Arc::new(Mutex::new(HashMap::new())),
            histograms: Arc::new(Mutex::new(HashMap::new())),
            store,
            // D-113: an omitted backend is the in-memory one, on every
            // constructor that does not take one.
            storage_backend: Some(crate::observability::storage::default_storage_backend()),
        }
    }

    /// Create a new metrics collector with an optional `StorageBackend`
    /// (Issue #43 §1). The internal `ObservabilityStore` is the default
    /// in-memory one; the storage backend is purely additive.
    #[must_use]
    pub fn with_storage_backend(storage_backend: Option<Arc<dyn StorageBackend>>) -> Self {
        // D-113: an OMITTED backend means the bundled in-memory one, not "no
        // storage". Only apcore-typescript honoured that, so the same omission
        // produced a working store there and a silent no-op here — and a caller
        // reading records back got an empty list rather than an error. The
        // parameter stays `Option` so existing callers compile unchanged;
        // `None` now resolves to the default rather than disabling persistence.
        let storage_backend = Some(
            storage_backend.unwrap_or_else(crate::observability::storage::default_storage_backend),
        );
        Self {
            counters: Arc::new(Mutex::new(HashMap::new())),
            histograms: Arc::new(Mutex::new(HashMap::new())),
            store: Arc::new(InMemoryObservabilityStore::new()),
            storage_backend,
        }
    }

    /// Attach an optional `StorageBackend` after construction.
    #[must_use]
    pub fn with_storage(mut self, storage_backend: Option<Arc<dyn StorageBackend>>) -> Self {
        self.storage_backend = storage_backend;
        self
    }

    /// Get a clone of the underlying store handle.
    #[must_use]
    pub fn store(&self) -> Arc<dyn ObservabilityStore> {
        self.store.clone()
    }

    /// The storage backend observations are persisted to (D-113). `Some`
    /// unless detached with `with_storage(None)`.
    #[must_use]
    pub fn storage_backend(&self) -> Option<Arc<dyn StorageBackend>> {
        self.storage_backend.clone()
    }

    /// Format labels into a composite key.
    fn make_key(name: &str, labels: &HashMap<String, String>) -> MetricKey {
        let sorted: BTreeMap<String, String> =
            labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        (name.to_string(), sorted)
    }

    /// Increment a counter metric by `amount`.
    #[allow(clippy::needless_pass_by_value)] // public API: HashMap passed by value is idiomatic for fire-and-forget metrics
    pub fn increment(&self, name: &str, labels: HashMap<String, String>, amount: f64) {
        let key = Self::make_key(name, &labels);
        {
            let mut counters = self.counters.lock();
            let entry = counters.entry(key).or_insert(0.0);
            *entry += amount;
        }
        self.notify_store(name, &labels, amount);
    }

    /// Observe a value for a histogram metric.
    #[allow(clippy::needless_pass_by_value)] // public API: HashMap passed by value is idiomatic for fire-and-forget metrics
    pub fn observe(&self, name: &str, labels: HashMap<String, String>, value: f64) {
        let key = Self::make_key(name, &labels);
        {
            let mut histograms = self.histograms.lock();
            let entry = histograms.entry(key).or_insert_with(HistogramData::new);
            entry.observe(value);
        }
        self.notify_store(name, &labels, value);
    }

    /// Forward a metric observation to the pluggable store. Best-effort:
    /// when no tokio runtime is active the call is dropped (with a debug log).
    fn notify_store(&self, name: &str, labels: &HashMap<String, String>, value: f64) {
        let module_id = labels.get("module_id").cloned();
        let mut metric = MetricPoint::new(name, value).with_labels(labels.clone());
        if let Some(id) = module_id {
            metric = metric.with_module_id(id);
        }
        let store = self.store.clone();
        let backend = self.storage_backend.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let metric_for_backend = metric.clone();
            handle.spawn(async move {
                store.record_metric(metric).await;
            });
            if let Some(backend) = backend {
                let key = format!(
                    "{}:{}",
                    metric_for_backend.name,
                    metric_for_backend
                        .timestamp
                        .timestamp_nanos_opt()
                        .unwrap_or(0)
                );
                handle.spawn(async move {
                    if let Ok(value) = serde_json::to_value(&metric_for_backend) {
                        let _ = backend.save("metrics", &key, value).await;
                    }
                });
            }
        } else {
            tracing::debug!(
                metric = %name,
                "MetricsCollector observation outside a tokio runtime; \
                 store notification skipped"
            );
        }
    }

    /// Return a snapshot of all current metric values as JSON.
    #[must_use]
    pub fn snapshot(&self) -> serde_json::Value {
        let counters = self.counters.lock();
        let histograms = self.histograms.lock();

        let mut counters_map = serde_json::Map::new();
        for ((name, labels), value) in counters.iter() {
            let label_str = if labels.is_empty() {
                name.clone()
            } else {
                let label_parts: Vec<String> =
                    labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("{}|{}", name, label_parts.join(","))
            };
            counters_map.insert(label_str, serde_json::json!(*value));
        }

        let mut histograms_map = serde_json::Map::new();
        for ((name, labels), data) in histograms.iter() {
            let label_str = if labels.is_empty() {
                name.clone()
            } else {
                let label_parts: Vec<String> =
                    labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("{}|{}", name, label_parts.join(","))
            };
            // D-106: the `+Inf` bucket is emitted alongside the finite ones (as
            // `export_prometheus` already does) so a consumer can tell "no
            // data" from "all overflow". Its bound is the string `"+Inf"`,
            // because JSON has no infinity literal and `serde_json` renders
            // `f64::INFINITY` as `null`.
            let mut buckets: Vec<serde_json::Value> = data
                .buckets
                .iter()
                .map(|(b, c)| serde_json::json!({"le": b, "count": c}))
                .collect();
            buckets.push(serde_json::json!({"le": "+Inf", "count": data.count}));
            histograms_map.insert(
                label_str,
                serde_json::json!({
                    "sum": data.sum,
                    "count": data.count,
                    "buckets": buckets,
                }),
            );
        }

        serde_json::json!({
            "counters": counters_map,
            "histograms": histograms_map,
        })
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        self.counters.lock().clear();
        self.histograms.lock().clear();
    }

    /// Export metrics in Prometheus text format.
    #[must_use]
    pub fn export_prometheus(&self) -> String {
        let mut output = String::new();
        let counters = self.counters.lock();
        let histograms = self.histograms.lock();

        // Export counters
        let mut seen_counter_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for ((name, labels), value) in counters.iter() {
            if seen_counter_names.insert(name.clone()) {
                let _ = writeln!(output, "# HELP {name} {}", metric_help_text(name));
                let _ = writeln!(output, "# TYPE {name} counter");
            }
            let label_str = format_prometheus_labels(labels);
            let _ = writeln!(output, "{name}{label_str} {value}");
        }

        // Export histograms
        let mut seen_hist_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for ((name, labels), data) in histograms.iter() {
            if seen_hist_names.insert(name.clone()) {
                let _ = writeln!(output, "# HELP {name} {}", metric_help_text(name));
                let _ = writeln!(output, "# TYPE {name} histogram");
            }
            let base_labels = format_prometheus_labels(labels);
            for (bound, count) in &data.buckets {
                let le_label = if labels.is_empty() {
                    format!("{{le=\"{bound}\"}}")
                } else {
                    // Insert le into existing labels
                    let inner = &base_labels[1..base_labels.len() - 1]; // strip { }
                    format!("{{{inner},le=\"{bound}\"}}")
                };
                let _ = writeln!(output, "{name}_bucket{le_label} {count}");
            }
            // +Inf bucket
            let inf_label = if labels.is_empty() {
                "{le=\"+Inf\"}".to_string()
            } else {
                let inner = &base_labels[1..base_labels.len() - 1];
                format!("{{{inner},le=\"+Inf\"}}")
            };
            let _ = writeln!(output, "{name}_bucket{inf_label} {}", data.count);
            let _ = writeln!(output, "{name}_sum{base_labels} {}", data.sum);
            let _ = writeln!(output, "{name}_count{base_labels} {}", data.count);
        }

        output
    }

    /// Convenience: increment call counter.
    pub fn increment_calls(&self, module_id: &str, status: &str) {
        let mut labels = HashMap::new();
        labels.insert("module_id".to_string(), module_id.to_string());
        labels.insert("status".to_string(), status.to_string());
        self.increment(METRIC_CALLS_TOTAL, labels, 1.0);
    }

    /// Convenience: increment error counter.
    pub fn increment_errors(&self, module_id: &str, error_code: &str) {
        let mut labels = HashMap::new();
        labels.insert("module_id".to_string(), module_id.to_string());
        labels.insert("error_code".to_string(), error_code.to_string());
        self.increment(METRIC_ERRORS_TOTAL, labels, 1.0);
    }

    /// Convenience: observe call duration.
    pub fn observe_duration(&self, module_id: &str, duration_secs: f64) {
        let mut labels = HashMap::new();
        labels.insert("module_id".to_string(), module_id.to_string());
        self.observe(METRIC_DURATION_SECONDS, labels, duration_secs);
    }
}

/// Per-metric-name HELP description for Prometheus export.
///
/// Mirrors the description table used by apcore-python / apcore-typescript so
/// the exported `# HELP` lines are identical across SDKs. Unknown metric names
/// fall back to the metric name itself (rather than a generic
/// "Counter metric" / "Histogram metric").
fn metric_help_text(name: &str) -> &str {
    match name {
        "apcore_module_calls_total" => "Total module calls",
        "apcore_module_errors_total" => "Total module errors",
        "apcore_module_duration_seconds" => "Module execution duration",
        other => other,
    }
}

/// Escape a Prometheus exposition-format label value.
///
/// Per the exposition format, label values are wrapped in double quotes and the
/// characters that MUST be escaped are backslash (`\`), double quote (`"`) and
/// line feed (`\n`). Backslash is escaped first so the escapes added after it
/// are not themselves escaped.
///
/// `MetricsCollector::increment` / `observe` take a caller-supplied
/// `HashMap<String, String>` with no documented constraint on the value, so an
/// unescaped `"` emitted a malformed line — and Prometheus rejects the ENTIRE
/// scrape on a parse error, dropping every other metric with it. apcore-python
/// (`_escape_label_value`) and apcore-typescript (`escapeLabelValue`) both do
/// this, each with a comment naming the same failure.
fn escape_prometheus_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Format labels as Prometheus label string: {key="value",...}
fn format_prometheus_labels(labels: &BTreeMap<String, String>) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = labels
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", escape_prometheus_label_value(v)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// Compute the minimum number of observations that must be accumulated to
/// reach the 99th-percentile threshold for a population of `total` items.
///
/// This is the shared core used by both [`estimate_p99_from_histogram`] and
/// [`estimate_p99_from_sorted`].
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
// intentional: realistic metric counts fit in f64; result is non-negative
fn p99_target_count(total: u64) -> u64 {
    (total as f64 * 0.99).ceil() as u64
}

/// Compute the 0-based index into a sorted slice that corresponds to the
/// 99th-percentile position for a slice of `len` items.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
// intentional: realistic slice lengths fit in f64; result is non-negative
fn p99_sorted_index(len: usize) -> usize {
    // ceil(len * 0.99) gives us the 1-based rank; clamp then convert to 0-based
    let rank = (len as f64 * 0.99).ceil() as usize;
    rank.min(len).saturating_sub(1)
}

/// Estimate p99 latency from histogram buckets in a metrics snapshot.
///
/// Expects `buckets` to be a JSON array of `{"le": <f64>, "count": <u64>}` objects
/// with cumulative counts, and `total_count` to be the total number of observations.
///
/// Returns the upper bound (`le`) of the first bucket whose cumulative count
/// reaches or exceeds the 99th-percentile threshold, converted to milliseconds.
/// Returns 0.0 if `total_count` is 0 or `buckets` is empty/missing.
///
/// D-106: when the nearest-rank target falls beyond the largest FINITE bucket —
/// every observation overflowed it — the estimate is that largest finite bound,
/// not `0.0`. Returning zero reported the fastest possible latency for the
/// slowest modules, and since `apcore.health.latency_threshold_exceeded`
/// compares the estimate against a threshold, it disabled the alert precisely
/// for the modules that should fire it. Mirrors apcore-python
/// (`metrics.py`: "Fall back to last finite bucket or 0") and apcore-typescript
/// (`metrics-utils.ts`: "All observations exceed the largest bucket").
///
/// The `+Inf` bucket the snapshot now carries (`le: "+Inf"`) is deliberately
/// skipped here: it is not a finite bound and cannot be reported as a latency.
pub(crate) fn estimate_p99_from_histogram(buckets: &[serde_json::Value], total_count: u64) -> f64 {
    if total_count == 0 || buckets.is_empty() {
        return 0.0;
    }
    let target = p99_target_count(total_count);
    let mut largest_finite: Option<f64> = None;
    for bucket in buckets {
        let Some(le) = bucket.get("le").and_then(serde_json::Value::as_f64) else {
            // `"+Inf"` (or any non-numeric bound) — not a reportable latency.
            continue;
        };
        if !le.is_finite() {
            continue;
        }
        largest_finite = Some(largest_finite.map_or(le, |prev: f64| prev.max(le)));
        let cnt = bucket
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if cnt >= target {
            return le * 1000.0; // seconds -> ms
        }
    }
    largest_finite.map_or(0.0, |le| le * 1000.0)
}

/// Label key carrying the module ID on every apcore metric.
///
/// `MetricsCollector::increment_calls` / `increment_errors` / `observe_duration`
/// are the only writers in this crate and all three emit `module_id`. Readers
/// MUST build their snapshot keys from this constant: a reader that spelled the
/// label `module=` instead looked up a key that is never written, and silently
/// reported zero for every module (see `PlatformNotifyMiddleware`, whose error
/// rate was therefore pinned at 0.0 and whose
/// `apcore.health.error_threshold_exceeded` event could never fire).
pub(crate) const LABEL_MODULE_ID: &str = "module_id";

/// Per-module call counts read out of a [`MetricsCollector::snapshot`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModuleCallCounts {
    /// Successful plus failed calls.
    pub total: u64,
    /// Failed calls only.
    pub errors: u64,
}

impl ModuleCallCounts {
    /// Errors as a fraction of total calls; 0.0 when no calls were recorded.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // counter magnitudes are far below 2^53
    pub fn error_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.errors as f64 / self.total as f64
    }
}

/// Per-module latency statistics read out of a [`MetricsCollector::snapshot`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModuleLatencyStats {
    /// Mean duration in milliseconds; 0.0 when no observations were recorded.
    pub avg_ms: f64,
    /// 99th-percentile duration in milliseconds, estimated from the histogram
    /// buckets; 0.0 when no observations were recorded.
    pub p99_ms: f64,
    /// Number of recorded observations.
    pub count: u64,
}

/// Snapshot key for a module-scoped counter: `name|module_id=<id>,status=<s>`.
///
/// Label order follows `snapshot()`, which renders the labels from a
/// `BTreeMap` and therefore sorts them (`module_id` before `status`).
fn counter_key(name: &str, module_id: &str, status: &str) -> String {
    format!("{name}|{LABEL_MODULE_ID}={module_id},status={status}")
}

/// Snapshot key for a module-scoped histogram: `name|module_id=<id>`.
fn histogram_key(name: &str, module_id: &str) -> String {
    format!("{name}|{LABEL_MODULE_ID}={module_id}")
}

/// Extract a module's call counts from a [`MetricsCollector::snapshot`].
///
/// The single shared reader for `apcore_module_calls_total`. Both the health
/// system module and `PlatformNotifyMiddleware` go through it so the label
/// spelling cannot diverge from what `increment_calls` writes again. Mirrors
/// apcore-typescript `computeModuleErrorRate` in
/// `src/observability/metrics-utils.ts`.
#[must_use]
pub fn extract_module_call_counts(
    snapshot: &serde_json::Value,
    module_id: &str,
) -> ModuleCallCounts {
    let Some(counters) = snapshot.get("counters").and_then(|c| c.as_object()) else {
        return ModuleCallCounts::default();
    };
    // Counters are `f64` in the collector and `snapshot()` renders them as
    // JSON floats (`3.0`), so `Value::as_u64` answers `None` for every one of
    // them — the health module's own extractor read them that way and therefore
    // reported zero calls for every module, whatever the collector held.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let read = |status: &str| -> u64 {
        counters
            .get(&counter_key(METRIC_CALLS_TOTAL, module_id, status))
            .and_then(serde_json::Value::as_f64)
            .map_or(0, |value| if value > 0.0 { value as u64 } else { 0 })
    };
    let errors = read("error");
    ModuleCallCounts {
        total: read("success") + errors,
        errors,
    }
}

/// Extract a module's latency statistics from a [`MetricsCollector::snapshot`].
///
/// The single shared reader for `apcore_module_duration_seconds`; see
/// [`extract_module_call_counts`] for why it is shared. Mirrors apcore-typescript
/// `estimateP99FromHistogram` in `src/observability/metrics-utils.ts`.
#[must_use]
#[allow(clippy::cast_precision_loss)] // latency avg: precision loss acceptable
pub fn extract_module_latency_stats(
    snapshot: &serde_json::Value,
    module_id: &str,
) -> ModuleLatencyStats {
    let Some(data) = snapshot
        .get("histograms")
        .and_then(|h| h.as_object())
        .and_then(|h| h.get(&histogram_key(METRIC_DURATION_SECONDS, module_id)))
    else {
        return ModuleLatencyStats::default();
    };
    let sum = data
        .get("sum")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let count = data
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let avg_ms = if count > 0 {
        (sum / count as f64) * 1000.0
    } else {
        0.0
    };
    let p99_ms = data
        .get("buckets")
        .and_then(|b| b.as_array())
        .map_or(0.0, |buckets| estimate_p99_from_histogram(buckets, count));
    ModuleLatencyStats {
        avg_ms,
        p99_ms,
        count,
    }
}

/// Estimate p99 latency from a sorted slice of raw latency values (in ms).
///
/// Returns the value at the 99th-percentile index. Returns 0.0 if the slice is empty.
pub(crate) fn estimate_p99_from_sorted(sorted_latencies: &[f64]) -> f64 {
    if sorted_latencies.is_empty() {
        return 0.0;
    }
    let idx = p99_sorted_index(sorted_latencies.len());
    sorted_latencies[idx]
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Middleware that records execution metrics.
///
/// Per-trace start times are kept as a **stack** (`Vec<Instant>`) so a nested
/// call sharing the same `trace_id` pushes its own start without clobbering the
/// outer call's. `after`/`on_error` pop the innermost start (LIFO), so each
/// frame records its own duration. Mirrors the per-context stack used by
/// apcore-python / apcore-typescript (sync finding [obs-nested-timing]); before
/// this fix a single-slot map made the parent record a 0ms duration.
#[derive(Debug)]
pub struct MetricsMiddleware {
    collector: MetricsCollector,
    starts: Mutex<HashMap<String, Vec<std::time::Instant>>>,
}

impl MetricsMiddleware {
    /// Create a new metrics middleware.
    #[must_use]
    pub fn new(collector: MetricsCollector) -> Self {
        Self {
            collector,
            starts: Mutex::new(HashMap::new()),
        }
    }

    /// Get a reference to the underlying collector.
    pub fn collector(&self) -> &MetricsCollector {
        &self.collector
    }

    /// Pop the innermost (LIFO) start time for `trace_id` and return its elapsed
    /// duration in seconds, or 0.0 if no matching start was recorded. Empty
    /// stacks are removed to avoid unbounded growth of the map.
    fn pop_duration_secs(&self, trace_id: &str) -> f64 {
        let mut starts = self.starts.lock();
        let Some(stack) = starts.get_mut(trace_id) else {
            return 0.0;
        };
        let duration = stack.pop().map_or(0.0, |s| s.elapsed().as_secs_f64());
        if stack.is_empty() {
            starts.remove(trace_id);
        }
        duration
    }
}

#[async_trait]
impl Middleware for MetricsMiddleware {
    fn name(&self) -> &'static str {
        "metrics"
    }

    async fn before(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        let mut starts = self.starts.lock();
        starts
            .entry(_ctx.trace_id.clone())
            .or_default()
            .push(std::time::Instant::now());
        Ok(None)
    }

    async fn after(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        _output: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        let duration_secs = self.pop_duration_secs(&_ctx.trace_id);

        self.collector.increment_calls(module_id, "success");
        self.collector.observe_duration(module_id, duration_secs);

        Ok(None)
    }

    async fn on_error(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        _error: &ModuleError,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        let duration_secs = self.pop_duration_secs(&_ctx.trace_id);

        // Use the canonical wire code (SCREAMING_SNAKE_CASE) for the metric
        // label, not Debug (PascalCase), for cross-language parity (sync
        // finding A-D-14).
        let error_code = _error.code.wire_str();
        self.collector.increment_calls(module_id, "error");
        self.collector.increment_errors(module_id, &error_code);
        self.collector.observe_duration(module_id, duration_secs);

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Context, Identity};
    use crate::errors::ErrorCode;

    // The extractors MUST read what the collector actually writes. Reader and
    // writer used to spell the module label differently (`module=` vs
    // `module_id=`), so every lookup missed and the middleware's error rate was
    // pinned at 0.0. These tests drive the real writers, never a hand-built
    // label map, so the two halves cannot diverge again unnoticed.
    #[test]
    fn extract_module_call_counts_reads_what_increment_calls_writes() {
        let collector = MetricsCollector::new();
        for _ in 0..7 {
            collector.increment_calls("mod.a", "success");
        }
        for _ in 0..3 {
            collector.increment_calls("mod.a", "error");
        }
        collector.increment_calls("mod.other", "error");

        let counts = extract_module_call_counts(&collector.snapshot(), "mod.a");
        assert_eq!(counts.total, 10);
        assert_eq!(counts.errors, 3);
        assert!((counts.error_rate() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn extract_module_call_counts_is_zero_for_an_unseen_module() {
        let collector = MetricsCollector::new();
        collector.increment_calls("mod.a", "error");
        let counts = extract_module_call_counts(&collector.snapshot(), "mod.missing");
        assert_eq!(counts, ModuleCallCounts::default());
        assert!((counts.error_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn extract_module_latency_stats_reads_what_observe_duration_writes() {
        let collector = MetricsCollector::new();
        collector.observe_duration("mod.a", 0.2);
        collector.observe_duration("mod.a", 0.2);

        let stats = extract_module_latency_stats(&collector.snapshot(), "mod.a");
        assert_eq!(stats.count, 2);
        assert!((stats.avg_ms - 200.0).abs() < 1e-6, "{}", stats.avg_ms);
        assert!(stats.p99_ms > 0.0);
    }

    // A-D-14: the error metric label MUST be the canonical wire code
    // (SCREAMING_SNAKE_CASE), not Debug formatting (PascalCase).
    #[tokio::test]
    async fn on_error_uses_canonical_wire_code_label() {
        let collector = MetricsCollector::new();
        let mw = MetricsMiddleware::new(collector);
        let ctx = Context::<serde_json::Value>::new(Identity::new(
            "@test".to_string(),
            "test".to_string(),
            vec![],
            HashMap::new(),
        ));
        let error = ModuleError::new(ErrorCode::ModuleExecuteError, "boom");

        mw.on_error("demo.module", serde_json::json!({}), &error, &ctx)
            .await
            .expect("on_error must not fail");

        let exported = mw.collector().export_prometheus();
        assert!(
            exported.contains("error_code=\"MODULE_EXECUTE_ERROR\""),
            "metric label must use canonical wire code; got:\n{exported}"
        );
        assert!(
            !exported.contains("ModuleExecuteError"),
            "metric label must NOT use Debug (PascalCase) formatting; got:\n{exported}"
        );
    }

    // [obs-nested-timing] A nested call sharing the same trace_id must not
    // clobber the outer call's start time: the outer call must record a
    // non-zero duration.
    #[tokio::test]
    async fn nested_same_trace_call_records_outer_duration() {
        let mw = MetricsMiddleware::new(MetricsCollector::new());
        let mut ctx = Context::<serde_json::Value>::new(Identity::new(
            "@test".to_string(),
            "test".to_string(),
            vec![],
            HashMap::new(),
        ));
        // Force both frames onto the same trace.
        ctx.trace_id = "shared-trace".to_string();

        // Outer begins.
        mw.before("outer", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        // Let some wall-clock time pass for the outer frame.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        // Nested call on the same trace: before + after.
        mw.before("inner", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        mw.after("inner", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();
        // Outer completes; its duration must still be > 0.
        mw.after("outer", serde_json::json!({}), serde_json::json!({}), &ctx)
            .await
            .unwrap();

        let snap = mw.collector().snapshot();
        let outer_sum = snap["histograms"]["apcore_module_duration_seconds|module_id=outer"]["sum"]
            .as_f64()
            .expect("outer duration histogram present");
        assert!(
            outer_sum > 0.0,
            "nested same-trace call must not zero out the outer duration; got {outer_sum}"
        );
    }

    #[test]
    fn error_code_wire_str_is_screaming_snake_case() {
        assert_eq!(
            ErrorCode::ModuleExecuteError.wire_str(),
            "MODULE_EXECUTE_ERROR"
        );
    }

    // -------------------------------------------------------------------------
    // p99 helper — correctness regression tests for Issue 23 refactor
    // -------------------------------------------------------------------------

    #[test]
    fn estimate_p99_from_sorted_empty_returns_zero() {
        assert!((estimate_p99_from_sorted(&[]) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_p99_from_sorted_single_element() {
        assert!((estimate_p99_from_sorted(&[42.0]) - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_p99_from_sorted_100_elements() {
        // 100 elements [1.0, 2.0, ..., 100.0]
        let data: Vec<f64> = (1..=100).map(f64::from).collect();
        let p99 = estimate_p99_from_sorted(&data);
        // ceil(100 * 0.99) = ceil(99) = 99 → index 98 (0-based) → value 99.0
        assert!((p99 - 99.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_p99_from_sorted_two_elements() {
        // ceil(2 * 0.99) = ceil(1.98) = 2 → index 1 → second element
        let data = vec![10.0, 200.0];
        let p99 = estimate_p99_from_sorted(&data);
        assert!((p99 - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_p99_from_histogram_empty_buckets_returns_zero() {
        assert!((estimate_p99_from_histogram(&[], 100) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_p99_from_histogram_zero_count_returns_zero() {
        let buckets = vec![serde_json::json!({"le": 0.1, "count": 50u64})];
        assert!((estimate_p99_from_histogram(&buckets, 0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_p99_from_histogram_finds_correct_bucket() {
        // 100 total, p99 threshold = ceil(99) = 99.
        // Bucket le=0.1 has cumulative count 90 (not enough).
        // Bucket le=0.5 has cumulative count 99 (exactly meets threshold).
        let buckets = vec![
            serde_json::json!({"le": 0.1, "count": 90u64}),
            serde_json::json!({"le": 0.5, "count": 99u64}),
            serde_json::json!({"le": 1.0, "count": 100u64}),
        ];
        // le=0.5 seconds → 500ms
        assert!((estimate_p99_from_histogram(&buckets, 100) - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_p99_from_histogram_beyond_top_bucket_returns_largest_finite_bound() {
        // D-106. No bucket reaches the nearest-rank target, which means every
        // remaining observation overflowed the largest finite bound. The
        // estimate is that bound — NOT 0.0, which reported the fastest possible
        // latency for the slowest modules and disabled latency alerting for
        // exactly the modules that should fire it.
        //
        // This test previously asserted 0.0 and so pinned the defect: a green
        // suite is why it survived. apcore-python (`metrics.py`) and
        // apcore-typescript (`metrics-utils.ts`) both return the last finite
        // bound here.
        let buckets = vec![
            serde_json::json!({"le": 0.1, "count": 50u64}),
            serde_json::json!({"le": 0.5, "count": 50u64}),
            serde_json::json!({"le": "+Inf", "count": 100u64}),
        ];
        // total=100, target=99; the top finite bucket holds only 50.
        assert!((estimate_p99_from_histogram(&buckets, 100) - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimate_p99_from_histogram_ignores_the_inf_bucket_as_a_bound() {
        // `+Inf` is not a reportable latency: it must never be returned as the
        // estimate, even though it is the bucket that holds the overflow.
        let buckets = vec![
            serde_json::json!({"le": 1.0, "count": 1u64}),
            serde_json::json!({"le": "+Inf", "count": 10u64}),
        ];
        let p99 = estimate_p99_from_histogram(&buckets, 10);
        assert!(
            p99.is_finite() && (p99 - 1000.0).abs() < f64::EPSILON,
            "got {p99}"
        );
    }

    // -------------------------------------------------------------------------
    // MetricsCollector — basic construction and increment
    // -------------------------------------------------------------------------

    #[test]
    fn metrics_collector_new_produces_empty_snapshot() {
        let collector = MetricsCollector::new();
        let snapshot = collector.snapshot();
        let counters = snapshot.get("counters").unwrap();
        assert!(counters.as_object().unwrap().is_empty());
    }

    #[test]
    fn metrics_collector_records_calls_and_snapshot_contains_counter() {
        let collector = MetricsCollector::new();
        collector.increment_calls("math.add", "success");
        collector.increment_calls("math.add", "success");
        let snapshot = collector.snapshot();
        let counters = snapshot.get("counters").unwrap().as_object().unwrap();
        // At least one key should contain "math.add"
        let found = counters.keys().any(|k| k.contains("math.add"));
        assert!(found, "snapshot should contain a counter for math.add");
    }

    #[test]
    fn metrics_collector_increment_by_known_amount() {
        let collector = MetricsCollector::new();
        let mut labels = HashMap::new();
        labels.insert("module".to_string(), "test".to_string());
        collector.increment("my_counter", labels.clone(), 3.0);
        collector.increment("my_counter", labels, 7.0);
        let snapshot = collector.snapshot();
        let counters = snapshot.get("counters").unwrap().as_object().unwrap();
        // Find the counter
        let val = counters
            .iter()
            .find(|(k, _)| k.contains("my_counter"))
            .map(|(_, v)| v.as_f64().unwrap())
            .expect("counter should exist");
        assert!(
            (val - 10.0).abs() < f64::EPSILON,
            "counter should be 10.0, got {val}"
        );
    }

    #[test]
    fn metrics_collector_reset_clears_all_metrics() {
        let collector = MetricsCollector::new();
        collector.increment_calls("m", "success");
        collector.reset();
        let snapshot = collector.snapshot();
        assert!(snapshot
            .get("counters")
            .unwrap()
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn metrics_collector_observe_duration_populates_histogram() {
        let collector = MetricsCollector::new();
        collector.observe_duration("m", 0.05); // 50ms — falls in 0.05s bucket
        collector.observe_duration("m", 0.2); // 200ms
        let snapshot = collector.snapshot();
        let histograms = snapshot.get("histograms").unwrap().as_object().unwrap();
        let found = histograms.keys().any(|k| k.contains("duration"));
        assert!(found, "duration histogram should be present");
    }
}
