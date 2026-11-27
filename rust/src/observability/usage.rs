// APCore Protocol — Usage tracking
// Spec reference: Module usage statistics and middleware
// Sync findings:
//   * D-27 — `record()` honors an optional explicit timestamp;
//            trend is computed from samples (current vs previous period
//            counts); summary accepts a period filter.
//   * Issue #43 §1 — optional `StorageBackend` for cross-process persistence.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::context::Context;
use crate::errors::ModuleError;
use crate::middleware::base::Middleware;
use crate::observability::metrics::estimate_p99_from_sorted;
use crate::observability::storage::{InMemoryStorageBackend, StorageBackend};

/// A single usage record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UsageRecord {
    pub timestamp: DateTime<Utc>,
    pub caller_id: Option<String>,
    pub latency_ms: f64,
    pub success: bool,
    #[serde(skip)]
    storage_key: String,
}

/// Usage summary for a single module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub module_id: String,
    pub call_count: u64,
    pub error_count: u64,
    pub avg_latency_ms: f64,
    pub unique_callers: usize,
    /// D-27: derived from sample counts (current vs previous period).
    /// Possible values: `"stable"`, `"rising"`, `"declining"`, `"new"`,
    /// `"inactive"` — matches Python/TS exactly.
    pub trend: String,
}

/// Maximum records per hourly bucket before oldest are evicted.
const MAX_RECORDS_PER_BUCKET: usize = 10_000;
/// Maximum occupied buckets per module when a retention window is unset.
const MAX_BUCKETS_PER_MODULE: usize = 168;

/// Internal module data for aggregation.
#[derive(Debug, Clone)]
struct ModuleData {
    records: HashMap<String, Vec<UsageRecord>>, // bucket_key -> records
}

impl ModuleData {
    fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// Evict buckets exceeding the retention limit (keeps newest).
    fn evict_old_buckets(&mut self) -> Vec<UsageRecord> {
        if self.records.len() <= MAX_BUCKETS_PER_MODULE {
            return Vec::new();
        }
        let mut keys: Vec<String> = self.records.keys().cloned().collect();
        keys.sort();
        let to_remove = keys.len() - MAX_BUCKETS_PER_MODULE;
        keys.into_iter()
            .take(to_remove)
            .filter_map(|key| self.records.remove(&key))
            .flatten()
            .collect()
    }

    fn expire_before(&mut self, cutoff_bucket: &str) -> Vec<UsageRecord> {
        let mut expired = Vec::new();
        self.records.retain(|bucket, records| {
            if bucket.as_str() < cutoff_bucket {
                expired.append(records);
                false
            } else {
                true
            }
        });
        expired
    }
}

/// Collects usage statistics across module executions.
#[derive(Debug, Clone)]
pub struct UsageCollector {
    data: Arc<Mutex<HashMap<String, ModuleData>>>,
    retention: Option<Duration>,
    /// Issue #43 §1: optional `StorageBackend` for persistence beyond process
    /// lifetime. Each `record()` writes a serialized `UsageRecord` to the
    /// backend under namespace `"usage"`; the in-memory aggregation still
    /// drives the synchronous accessors so reads remain fast.
    storage_backend: Option<Arc<dyn StorageBackend>>,
    default_storage: Option<Arc<InMemoryStorageBackend>>,
}

impl UsageCollector {
    /// Create a new usage collector.
    #[must_use]
    pub fn new() -> Self {
        Self::with_storage_backend(None)
    }

    /// The storage backend records are persisted to (D-113).
    #[must_use]
    pub fn storage_backend(&self) -> Option<Arc<dyn StorageBackend>> {
        self.storage_backend.clone().or_else(|| {
            self.default_storage
                .clone()
                .map(|storage| storage as Arc<dyn StorageBackend>)
        })
    }

    /// Create with an optional `StorageBackend` (Issue #43 §1).
    #[must_use]
    pub fn with_storage_backend(storage_backend: Option<Arc<dyn StorageBackend>>) -> Self {
        // D-113: an OMITTED backend means the bundled in-memory one, not "no
        // storage". Only apcore-typescript honoured that, so the same omission
        // produced a working store there and a silent no-op here — and a caller
        // reading records back got an empty list rather than an error. The
        // parameter stays `Option` so existing callers compile unchanged;
        // `None` now resolves to the default rather than disabling persistence.
        let default_storage = storage_backend
            .is_none()
            .then(|| Arc::new(InMemoryStorageBackend::new()));
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
            retention: None,
            storage_backend,
            default_storage,
        }
    }

    #[cfg(feature = "sys-modules")]
    pub(crate) fn with_retention(retention: Duration) -> Self {
        Self {
            retention: Some(retention),
            ..Self::new()
        }
    }

    /// Generate hourly bucket key from a timestamp: "YYYY-MM-DDTHH".
    fn bucket_key(ts: DateTime<Utc>) -> String {
        ts.format("%Y-%m-%dT%H").to_string()
    }

    /// Record a module execution at the current time.
    pub fn record(&self, module_id: &str, caller_id: Option<&str>, latency_ms: f64, success: bool) {
        self.record_at(module_id, caller_id, latency_ms, success, Utc::now());
    }

    /// Record a module execution at an explicit timestamp (D-27).
    ///
    /// Used by tests to drive trend computation deterministically and by
    /// integrations that replay historical events. The timestamp is also the
    /// hourly bucket key, so back-dated records sort correctly into past
    /// buckets.
    pub fn record_at(
        &self,
        module_id: &str,
        caller_id: Option<&str>,
        latency_ms: f64,
        success: bool,
        at: DateTime<Utc>,
    ) {
        let record = UsageRecord {
            timestamp: at,
            caller_id: caller_id.map(std::string::ToString::to_string),
            latency_ms,
            success,
            storage_key: format!(
                "{module_id}:{}:{}",
                at.timestamp_nanos_opt().unwrap_or(0),
                uuid::Uuid::new_v4()
            ),
        };

        let bucket = Self::bucket_key(at);
        {
            let mut data = self.data.lock();
            let module = data
                .entry(module_id.to_string())
                .or_insert_with(ModuleData::new);
            let bucket_records = module.records.entry(bucket.clone()).or_default();
            bucket_records.push(record.clone());
            let excess = bucket_records.len().saturating_sub(MAX_RECORDS_PER_BUCKET);
            for expired in bucket_records.drain(..excess) {
                self.remove_default_record(&expired);
            }
            let expired = if let Some(retention) = self.retention {
                Utc::now()
                    .checked_sub_signed(retention)
                    .map_or_else(Vec::new, |cutoff| {
                        module.expire_before(&Self::bucket_key(cutoff))
                    })
            } else {
                module.evict_old_buckets()
            };
            for expired in expired {
                self.remove_default_record(&expired);
            }
            if let Some(storage) = &self.default_storage {
                if module.records.contains_key(&bucket) {
                    if let Ok(value) = serde_json::to_value(&record) {
                        storage.put("usage", &record.storage_key, value);
                    }
                }
            }
        }

        // Forward to the optional storage backend (Issue #43 §1). Best-effort:
        // when no tokio runtime is active or serialization fails the call is
        // dropped silently. We deliberately do NOT block the in-memory path
        // on backend latency.
        if let Some(backend) = self.storage_backend.clone() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Ok(value) = serde_json::to_value(&record) {
                        let _ = backend.save("usage", &record.storage_key, value).await;
                    }
                });
            }
        }
    }

    fn remove_default_record(&self, record: &UsageRecord) {
        if let Some(storage) = &self.default_storage {
            storage.remove("usage", &record.storage_key);
        }
    }

    /// Aggregate a slice of records into a `UsageStats` (helper, no lock).
    fn aggregate_records(module_id: &str, records: &[UsageRecord], trend: String) -> UsageStats {
        let mut call_count: u64 = 0;
        let mut error_count: u64 = 0;
        let mut total_latency: f64 = 0.0;
        let mut unique_callers: HashSet<String> = HashSet::new();
        for record in records {
            call_count += 1;
            if !record.success {
                error_count += 1;
            }
            total_latency += record.latency_ms;
            if let Some(ref cid) = record.caller_id {
                unique_callers.insert(cid.clone());
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let avg_latency_ms = if call_count > 0 {
            total_latency / call_count as f64
        } else {
            0.0
        };
        UsageStats {
            module_id: module_id.to_string(),
            call_count,
            error_count,
            avg_latency_ms,
            unique_callers: unique_callers.len(),
            trend,
        }
    }

    /// Collect records for a module within `[start, end]`. Caller holds the lock.
    fn collect_records_in_window(
        module_data: &ModuleData,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<UsageRecord> {
        let start_key = Self::bucket_key(start);
        let end_key = Self::bucket_key(end);
        let mut out: Vec<UsageRecord> = Vec::new();
        for (bk, recs) in &module_data.records {
            // Hourly bucket keys are lexicographically ordered, so we can prune
            // buckets outside the window without parsing them.
            if bk.as_str() < start_key.as_str() || bk.as_str() > end_key.as_str() {
                continue;
            }
            for r in recs {
                if r.timestamp >= start && r.timestamp <= end {
                    out.push(r.clone());
                }
            }
        }
        out
    }

    /// Compute the trend label by comparing sample counts (D-27).
    ///
    /// Mirrors Python `_compute_trend` exactly:
    ///   - both 0 → `"stable"`
    ///   - current 0 (with previous > 0) → `"inactive"`
    ///   - previous 0 (with current > 0) → `"new"`
    ///   - ratio > 1.2 → `"rising"`
    ///   - ratio < 0.8 → `"declining"`
    ///   - else → `"stable"`
    #[must_use]
    fn compute_trend(current: usize, previous: usize) -> &'static str {
        if current == 0 && previous == 0 {
            return "stable";
        }
        if current == 0 {
            return "inactive";
        }
        if previous == 0 {
            return "new";
        }
        #[allow(clippy::cast_precision_loss)]
        let ratio = current as f64 / previous as f64;
        if ratio > 1.2 {
            "rising"
        } else if ratio < 0.8 {
            "declining"
        } else {
            "stable"
        }
    }

    /// Get usage summary for a specific module (all recorded data, no period filter).
    #[must_use]
    pub fn get_module_summary(&self, module_id: &str) -> Option<UsageStats> {
        let data = self.data.lock();
        let md = data.get(module_id)?;
        let records: Vec<UsageRecord> = md.records.values().flatten().cloned().collect();
        // No period → trend is "stable" (no comparison window).
        Some(Self::aggregate_records(
            module_id,
            &records,
            "stable".to_string(),
        ))
    }

    /// Get all usage summaries (all recorded data, no period filter).
    #[must_use]
    pub fn get_all_summaries(&self) -> Vec<UsageStats> {
        let data = self.data.lock();
        data.iter()
            .map(|(mid, md)| {
                let records: Vec<UsageRecord> = md.records.values().flatten().cloned().collect();
                Self::aggregate_records(mid, &records, "stable".to_string())
            })
            .collect()
    }

    /// Get summaries for all modules, optionally filtered to a recent
    /// period (D-27). When `period` is `None`, the full history is summarised
    /// and trend is `"stable"` (no window to compare against).
    #[must_use]
    pub fn get_summary_for_period(&self, period: Option<Duration>) -> Vec<UsageStats> {
        let now = Utc::now();
        let data = self.data.lock();
        match period {
            None => data
                .iter()
                .map(|(mid, md)| {
                    let records: Vec<UsageRecord> =
                        md.records.values().flatten().cloned().collect();
                    Self::aggregate_records(mid, &records, "stable".to_string())
                })
                .collect(),
            Some(delta) => {
                let cutoff = now - delta;
                let prev_cutoff = cutoff - delta;
                data.iter()
                    .map(|(mid, md)| {
                        let current = Self::collect_records_in_window(md, cutoff, now);
                        let previous = Self::collect_records_in_window(md, prev_cutoff, cutoff);
                        let trend = Self::compute_trend(current.len(), previous.len()).to_string();
                        Self::aggregate_records(mid, &current, trend)
                    })
                    .collect()
            }
        }
    }

    /// Summary for one module over `period` (PROTOCOL_SPEC 6.7.1.1).
    ///
    /// `None` summarises the full retained history with trend `"stable"`,
    /// matching [`Self::get_summary_for_period`].
    #[must_use]
    pub fn get_module_summary_for_period(
        &self,
        module_id: &str,
        period: Option<Duration>,
    ) -> Option<UsageStats> {
        let now = Utc::now();
        let data = self.data.lock();
        let module_data = data.get(module_id)?;
        Some(match period {
            None => {
                let records: Vec<UsageRecord> =
                    module_data.records.values().flatten().cloned().collect();
                Self::aggregate_records(module_id, &records, "stable".to_string())
            }
            Some(delta) => {
                let cutoff = now - delta;
                let prev_cutoff = cutoff - delta;
                let current = Self::collect_records_in_window(module_data, cutoff, now);
                let previous = Self::collect_records_in_window(module_data, prev_cutoff, cutoff);
                let trend = Self::compute_trend(current.len(), previous.len()).to_string();
                Self::aggregate_records(module_id, &current, trend)
            }
        })
    }

    /// Per-caller breakdown over `period` (PROTOCOL_SPEC 6.7.1.1).
    ///
    /// A record with no caller identity is attributed to the literal
    /// `"unknown"` (6.7.1.4).
    #[must_use]
    #[cfg(feature = "sys-modules")]
    pub(crate) fn get_caller_breakdown_for_period(
        &self,
        module_id: &str,
        period: Option<Duration>,
    ) -> Vec<CallerStats> {
        let now = Utc::now();
        let data = self.data.lock();
        let Some(module_data) = data.get(module_id) else {
            return Vec::new();
        };
        let records: Vec<UsageRecord> = match period {
            None => module_data.records.values().flatten().cloned().collect(),
            Some(delta) => Self::collect_records_in_window(module_data, now - delta, now),
        };
        let mut callers: HashMap<String, (u64, u64, f64)> = HashMap::new();
        for rec in &records {
            let cid = rec.caller_id.as_deref().unwrap_or("unknown").to_string();
            let entry = callers.entry(cid).or_insert((0, 0, 0.0));
            entry.0 += 1;
            if !rec.success {
                entry.1 += 1;
            }
            entry.2 += rec.latency_ms;
        }
        #[allow(clippy::cast_precision_loss)] // call counts comfortably fit in f64
        let mut out: Vec<CallerStats> = callers
            .into_iter()
            .map(
                |(caller_id, (call_count, error_count, total_latency))| CallerStats {
                    caller_id,
                    call_count,
                    error_count,
                    avg_latency_ms: if call_count > 0 {
                        total_latency / call_count as f64
                    } else {
                        0.0
                    },
                },
            )
            .collect();
        out.sort_by(|a, b| {
            b.call_count
                .cmp(&a.call_count)
                .then_with(|| a.caller_id.cmp(&b.caller_id))
        });
        out
    }

    /// Hourly distribution over `period`, ascending by hour key.
    ///
    /// `period` filters the counts inside each bucket; the 24-entry span the
    /// sys-module pads to is fixed and is not widened by it (6.7.1.2).
    #[must_use]
    #[cfg(feature = "sys-modules")]
    pub(crate) fn get_hourly_distribution_for_period(
        &self,
        module_id: &str,
        period: Option<Duration>,
    ) -> Vec<HourlyBucket> {
        let now = Utc::now();
        let data = self.data.lock();
        let Some(module_data) = data.get(module_id) else {
            return Vec::new();
        };
        let cutoff = period.map(|delta| now - delta);
        let mut buckets: Vec<HourlyBucket> = module_data
            .records
            .iter()
            .map(|(hour, records)| {
                let in_window = |r: &&UsageRecord| {
                    cutoff.is_none_or(|c| r.timestamp >= c && r.timestamp <= now)
                };
                HourlyBucket {
                    hour: hour.clone(),
                    call_count: records.iter().filter(in_window).count() as u64,
                    error_count: records
                        .iter()
                        .filter(in_window)
                        .filter(|r| !r.success)
                        .count() as u64,
                }
            })
            .collect();
        buckets.sort_by(|a, b| a.hour.cmp(&b.hour));
        buckets
    }

    /// Nearest-rank p99 latency (ms) over `period` (PROTOCOL_SPEC 6.7.1.3).
    #[must_use]
    pub fn get_p99_latency_ms_for_period(&self, module_id: &str, period: Option<Duration>) -> f64 {
        let now = Utc::now();
        let data = self.data.lock();
        let Some(module_data) = data.get(module_id) else {
            return 0.0;
        };
        let mut latencies: Vec<f64> = match period {
            None => module_data
                .records
                .values()
                .flat_map(|recs| recs.iter().map(|r| r.latency_ms))
                .collect(),
            Some(delta) => Self::collect_records_in_window(module_data, now - delta, now)
                .iter()
                .map(|r| r.latency_ms)
                .collect(),
        };
        if latencies.is_empty() {
            return 0.0;
        }
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        estimate_p99_from_sorted(&latencies)
    }

    /// Compute p99 latency (ms) for a module from stored records.
    #[must_use]
    pub fn get_p99_latency_ms(&self, module_id: &str) -> f64 {
        let data = self.data.lock();
        let Some(module_data) = data.get(module_id) else {
            return 0.0;
        };
        let mut latencies: Vec<f64> = module_data
            .records
            .values()
            .flat_map(|recs| recs.iter().map(|r| r.latency_ms))
            .collect();
        if latencies.is_empty() {
            return 0.0;
        }
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        estimate_p99_from_sorted(&latencies)
    }

    /// Reset all stats.
    pub fn reset(&self) {
        let mut data = self.data.lock();
        if let Some(storage) = &self.default_storage {
            for record in data
                .values()
                .flat_map(|module| module.records.values())
                .flatten()
            {
                storage.remove("usage", &record.storage_key);
            }
        }
        data.clear();
    }

    /// Compute a percentile (0.0..=1.0) from a sorted latency slice.
    /// Returns 0.0 for empty input. Used by `export_prometheus`.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn percentile_from_sorted(sorted: &[f64], q: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let rank = (sorted.len() as f64 * q).ceil() as usize;
        sorted[rank.min(sorted.len()).saturating_sub(1)]
    }

    // Snapshot the rolling 24h window under lock, then compute and render the gauges
    // after unlocking so observations do not wait on percentile sorting or formatting.
    fn prometheus_snapshot(&self) -> BTreeMap<String, (usize, Vec<f64>)> {
        let now = Utc::now();
        let cutoff = now - Duration::hours(24);
        self.data
            .lock()
            .iter()
            .map(|(module_id, module)| {
                let mut latencies = Vec::new();
                let mut errors = 0;
                for record in Self::collect_records_in_window(module, cutoff, now) {
                    if !record.success {
                        errors += 1;
                    }
                    latencies.push(record.latency_ms);
                }
                (module_id.clone(), (errors, latencies))
            })
            .collect()
    }

    /// Render rolling 24-hour usage as grouped Prometheus gauge families.
    ///
    /// `apcore_usage_calls_total` is a deprecated alias of `apcore_usage_calls_24h`.
    /// HELP and TYPE lines are emitted even when the collector is empty.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // each module's retained sample limit fits exactly in f64
    pub fn export_prometheus(&self) -> String {
        use std::fmt::Write as _;

        let mut families = [
            (
                "apcore_usage_calls_24h",
                "Module calls in the last 24 hours by status",
            ),
            (
                "apcore_usage_calls_total",
                "Deprecated alias of apcore_usage_calls_24h",
            ),
            ("apcore_usage_error_rate", "Module error rate (0.0–1.0)"),
            (
                "apcore_usage_p50_latency_ms",
                "Module p50 latency in milliseconds",
            ),
            (
                "apcore_usage_p95_latency_ms",
                "Module p95 latency in milliseconds",
            ),
            (
                "apcore_usage_p99_latency_ms",
                "Module p99 latency in milliseconds",
            ),
        ]
        .map(|(name, help)| (name, format!("# HELP {name} {help}\n# TYPE {name} gauge\n")));

        for (module_id, (errors, mut latencies)) in self.prometheus_snapshot() {
            let [calls, legacy_calls, error_rate, p50, p95, p99] = &mut families;
            let escaped = escape_label_value(&module_id);
            let total = latencies.len();
            let success = total - errors;
            for (name, out) in [calls, legacy_calls] {
                let _ = writeln!(
                    out,
                    "{name}{{module_id=\"{escaped}\",status=\"success\"}} {success}"
                );
                let _ = writeln!(
                    out,
                    "{name}{{module_id=\"{escaped}\",status=\"error\"}} {errors}"
                );
            }

            let rate = if total == 0 {
                0.0
            } else {
                errors as f64 / total as f64
            };
            let (name, out) = error_rate;
            let _ = writeln!(out, "{name}{{module_id=\"{escaped}\"}} {rate}");

            latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for ((name, out), percentile) in [(p50, 0.50), (p95, 0.95), (p99, 0.99)] {
                let value = Self::percentile_from_sorted(&latencies, percentile);
                let _ = writeln!(out, "{name}{{module_id=\"{escaped}\"}} {value}");
            }
        }

        families.into_iter().map(|(_, body)| body).collect()
    }
}

/// Escape a Prometheus label value per the text exposition spec:
/// backslash, double-quote, and newline are the only characters that need
/// escaping. Module IDs in apcore are restricted to `[a-z0-9._-]`, so this
/// is defense-in-depth — but cheap and worth keeping.
fn escape_label_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// Per-caller usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg(feature = "sys-modules")]
pub(crate) struct CallerStats {
    pub caller_id: String,
    pub call_count: u64,
    pub error_count: u64,
    pub avg_latency_ms: f64,
}

/// Hourly usage bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg(feature = "sys-modules")]
pub(crate) struct HourlyBucket {
    pub hour: String,
    pub call_count: u64,
    pub error_count: u64,
}

impl Default for UsageCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Middleware that tracks usage statistics.
#[derive(Debug)]
pub struct UsageMiddleware {
    collector: UsageCollector,
    timer: crate::middleware::CallTimer,
}

impl UsageMiddleware {
    /// Create a new usage middleware.
    #[must_use]
    pub fn new(collector: UsageCollector) -> Self {
        Self {
            collector,
            timer: crate::middleware::CallTimer::default(),
        }
    }

    /// Get a reference to the underlying collector.
    pub fn collector(&self) -> &UsageCollector {
        &self.collector
    }
}

#[async_trait]
impl Middleware for UsageMiddleware {
    fn name(&self) -> &'static str {
        "usage"
    }

    async fn before(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        self.timer.start(module_id, ctx);
        Ok(None)
    }

    async fn after(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        _output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        let Some(duration) = self.timer.finish(module_id, ctx) else {
            return Ok(None);
        };
        let latency_ms = duration.as_secs_f64() * 1000.0;

        self.collector
            .record(module_id, ctx.caller_id.as_deref(), latency_ms, true);

        Ok(None)
    }

    async fn on_error(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        _error: &ModuleError,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        let Some(duration) = self.timer.finish(module_id, ctx) else {
            return Ok(None);
        };
        let latency_ms = duration.as_secs_f64() * 1000.0;

        self.collector
            .record(module_id, ctx.caller_id.as_deref(), latency_ms, false);

        Ok(None)
    }
}
