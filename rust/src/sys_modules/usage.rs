// APCore Protocol — System usage modules
// Spec reference: system.usage.summary, system.usage.module

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};
use crate::module::Module;
use crate::observability::usage::UsageCollector;
use crate::registry::registry::Registry;

/// Hour-key format, identical to `UsageCollector::bucket_key`
/// (PROTOCOL_SPEC 6.7.1.2).
///
/// This constant used to be `%Y-%m-%dT%H:00:00Z`, documented as "matching
/// `UsageCollector` bucket hours" while `bucket_key` produced `%Y-%m-%dT%H` --
/// so the comment asserted an alignment that did not hold, and the sys-module
/// layer reformatted the collector's own key into a spelling neither
/// apcore-python nor apcore-typescript emits.
const HOUR_KEY_FORMAT: &str = "%Y-%m-%dT%H";

/// Parse a `period` input into a [`Duration`] (PROTOCOL_SPEC 6.7.1.1).
///
/// The grammar `^[1-9][0-9]*[hd]$` is declared in both modules' `input_schema`,
/// so a malformed value is normally rejected at input validation before
/// reaching here. This function is the second line of that contract, not the
/// first — and it has to be, because §6.6.3.2 documents three strategy presets
/// that remove the `input_validation` step, so a direct module call can arrive
/// unvalidated.
///
/// `None` means "not parseable" and callers MUST surface it as an error.
/// Passing it downstream aggregates the full retained history while the
/// response still echoes the requested `period` — the exact shape §6.7.1.1
/// forbids, and what both call sites used to do.
fn parse_period(period: &str) -> Option<Duration> {
    let (digits, unit) = period.split_at(period.len().checked_sub(1)?);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if digits.starts_with('0') {
        return None;
    }
    let n: i64 = digits.parse().ok()?;
    match unit {
        "h" => Some(Duration::hours(n)),
        "d" => Some(Duration::days(n)),
        _ => None,
    }
}

/// Parse `period`, or fail with the same error input validation would raise.
///
/// apcore-python's `_parse_period` raises ValueError and apcore-typescript's
/// `parsePeriod` throws; both surface as an error to the caller. This SDK
/// passed `None` downstream instead, which silently produced full-history
/// numbers under the requested window's label (§6.7.1.1).
fn reject_malformed_period(period: &str) -> Result<Duration, ModuleError> {
    parse_period(period).ok_or_else(|| {
        ModuleError::new(
            ErrorCode::SchemaValidationError,
            format!(
                "Invalid `period` '{period}': expected the grammar ^[1-9][0-9]*[hd]$ \
                 (for example \"24h\" or \"7d\")"
            ),
        )
    })
}

/// Pad an hourly distribution to exactly 24 entries, zero-filling gaps.
///
/// Generates the 24 hourly keys covering the last 24 hours (`now-23h .. now`),
/// merges in any actual data buckets, sorts and dedups, then keeps the latest
/// 24. Missing hours are filled with zero counts. Spec MUST: `system.usage.module`
/// always returns 24 hourly entries. Mirrors apcore-python
/// `_pad_hourly_distribution` (usage.py:172) — sync finding A-D-13.
fn pad_hourly_distribution(hourly: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let now = Utc::now();

    // Map existing hour-key -> (call_count, error_count).
    let mut existing: std::collections::HashMap<String, (u64, u64)> =
        std::collections::HashMap::new();
    for h in hourly {
        if let Some(hour) = h.get("hour").and_then(serde_json::Value::as_str) {
            let calls = h
                .get("call_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let errors = h
                .get("error_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            existing.insert(hour.to_string(), (calls, errors));
        }
    }

    // Generate the 24 hourly keys for now-23h .. now, then union with any
    // data keys not already present, sort+dedup, and take the latest 24.
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for i in 0..24i64 {
        let hour_dt = now - Duration::hours(23 - i);
        keys.insert(hour_dt.format(HOUR_KEY_FORMAT).to_string());
    }
    for k in existing.keys() {
        keys.insert(k.clone());
    }
    let latest_24: Vec<String> = {
        let all: Vec<String> = keys.into_iter().collect();
        let start = all.len().saturating_sub(24);
        all[start..].to_vec()
    };

    latest_24
        .into_iter()
        .map(|key| {
            let (call_count, error_count) = existing.get(&key).copied().unwrap_or((0, 0));
            json!({
                "hour": key,
                "call_count": call_count,
                "error_count": error_count,
            })
        })
        .collect()
}

/// system.usage.summary — Usage overview with trend detection across all modules.
pub struct UsageSummaryModule {
    collector: UsageCollector,
}

impl UsageSummaryModule {
    #[must_use]
    pub fn new(collector: UsageCollector) -> Self {
        Self { collector }
    }
}

#[async_trait]
impl Module for UsageSummaryModule {
    fn description(&self) -> &'static str {
        "Usage overview with trend detection across all modules"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "period": {
                    "type": "string",
                    "description": "Time window: a positive integer followed by 'h' (hours) or 'd' (days), e.g. 1h, 24h, 7d. Every statistic in the output is computed over [now - period, now].",
                    "default": "24h",
                    "pattern": "^[1-9][0-9]*[hd]$"
                }
            }
        })
    }

    // PROTOCOL_SPEC 6.7.1.6: output_schema MUST declare the full field
    // contract. This returned a bare {"type": "object"} for both usage modules,
    // which satisfies 6.7's "equivalent output schemas" only in the sense that
    // any two such declarations are equivalent to each other. Canonical shape:
    // apcore/schemas/sys-usage-summary.schema.json.
    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["period", "total_calls", "total_errors", "modules"],
            "properties": {
                "period": {"type": "string", "description": "Requested time window, echoed back"},
                "total_calls": {"type": "integer", "description": "Total calls across all modules in the period"},
                "total_errors": {"type": "integer", "description": "Total failed calls across all modules in the period"},
                "modules": {
                    "type": "array",
                    "description": "Per-module usage entries, sorted by call_count descending",
                    "items": {
                        "type": "object",
                        "required": ["module_id", "call_count", "error_count", "avg_latency_ms", "unique_callers", "trend"],
                        "properties": {
                            "module_id": {"type": "string", "description": "Canonical module ID"},
                            "call_count": {"type": "integer", "description": "Calls in the period"},
                            "error_count": {"type": "integer", "description": "Failed calls in the period"},
                            "avg_latency_ms": {"type": "number", "description": "Mean latency in ms over the period"},
                            "unique_callers": {"type": "integer", "description": "Distinct caller_id values in the period"},
                            "trend": {"type": "string", "enum": ["stable", "inactive", "new", "rising", "declining"], "description": "Direction of change against the preceding window"}
                        }
                    }
                }
            }
        })
    }

    async fn execute(
        &self,
        inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        let period = inputs
            .get("period")
            .and_then(|v| v.as_str())
            .unwrap_or("24h");

        // PROTOCOL_SPEC 6.7.1.1: `period` is a filter, not an echo. This used
        // to call get_all_summaries(), so every statistic covered the full
        // retained history while the response named a window it had not
        // applied -- silent by construction.
        let window = Some(reject_malformed_period(period)?);
        let mut summaries = self.collector.get_summary_for_period(window);
        // Sort by call_count descending per spec.
        summaries.sort_by_key(|b| std::cmp::Reverse(b.call_count));

        let total_calls: u64 = summaries.iter().map(|s| s.call_count).sum();
        let total_errors: u64 = summaries.iter().map(|s| s.error_count).sum();

        let modules: Vec<serde_json::Value> = summaries
            .into_iter()
            .map(|s| {
                json!({
                    "module_id": s.module_id,
                    "call_count": s.call_count,
                    "error_count": s.error_count,
                    "avg_latency_ms": s.avg_latency_ms,
                    "unique_callers": s.unique_callers,
                    "trend": s.trend,
                })
            })
            .collect();

        Ok(json!({
            "period": period,
            "total_calls": total_calls,
            "total_errors": total_errors,
            "modules": modules,
        }))
    }
}

/// system.usage.module — Detailed usage for a single module.
pub struct UsageModule {
    registry: Arc<Registry>,
    collector: UsageCollector,
}

impl UsageModule {
    pub fn new(registry: Arc<Registry>, collector: UsageCollector) -> Self {
        Self {
            registry,
            collector,
        }
    }
}

#[async_trait]
impl Module for UsageModule {
    fn description(&self) -> &'static str {
        "Detailed usage for a single module with caller breakdown"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["module_id"],
            "properties": {
                "module_id": {"type": "string", "description": "ID of the module to inspect"},
                "period": {
                    "type": "string",
                    "description": "Time window: a positive integer followed by 'h' (hours) or 'd' (days), e.g. 1h, 24h, 7d. Every statistic in the output is computed over [now - period, now].",
                    "default": "24h",
                    "pattern": "^[1-9][0-9]*[hd]$"
                }
            }
        })
    }

    // PROTOCOL_SPEC 6.7.1.6. Canonical shape:
    // apcore/schemas/sys-usage-module.schema.json.
    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["module_id", "period", "call_count", "error_count", "avg_latency_ms", "p99_latency_ms", "trend", "callers", "hourly_distribution"],
            "properties": {
                "module_id": {"type": "string", "description": "Canonical ID of the module this report covers"},
                "period": {"type": "string", "description": "Requested time window, echoed back"},
                "call_count": {"type": "integer", "description": "Calls in the period"},
                "error_count": {"type": "integer", "description": "Failed calls in the period"},
                "avg_latency_ms": {"type": "number", "description": "Mean latency in ms over the period"},
                "p99_latency_ms": {"type": "number", "description": "Nearest-rank 99th percentile latency in ms over the period"},
                "trend": {"type": "string", "enum": ["stable", "inactive", "new", "rising", "declining"], "description": "Direction of change against the preceding window"},
                "callers": {
                    "type": "array",
                    "description": "Per-caller breakdown of the calls in the period",
                    "items": {
                        "type": "object",
                        "required": ["caller_id", "call_count", "error_count", "avg_latency_ms"],
                        "properties": {
                            "caller_id": {"type": "string", "description": "Caller identity; the literal 'unknown' for an unattributed call"},
                            "call_count": {"type": "integer", "description": "Calls from this caller in the period"},
                            "error_count": {"type": "integer", "description": "Failed calls from this caller in the period"},
                            "avg_latency_ms": {"type": "number", "description": "Mean latency in ms for this caller"}
                        }
                    }
                },
                "hourly_distribution": {
                    "type": "array",
                    "description": "The 24 hourly buckets ending at the current hour, zero-filled and ascending",
                    "minItems": 24,
                    "maxItems": 24,
                    "items": {
                        "type": "object",
                        "required": ["hour", "call_count", "error_count"],
                        "properties": {
                            "hour": {"type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}$", "description": "UTC hourly bucket key, YYYY-MM-DDTHH"},
                            "call_count": {"type": "integer", "description": "Calls in this bucket"},
                            "error_count": {"type": "integer", "description": "Failed calls in this bucket"}
                        }
                    }
                }
            }
        })
    }

    async fn execute(
        &self,
        inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        // Reject an empty module_id with InvalidInput (GENERAL_INVALID_INPUT)
        // rather than letting it fall through to ModuleNotFound, matching
        // apcore-python / apcore-typescript.
        let module_id = super::require_string(&inputs, "module_id")?;
        let module_id = module_id.as_str();
        let period = inputs
            .get("period")
            .and_then(|v| v.as_str())
            .unwrap_or("24h");

        if !self.registry.has(module_id) {
            return Err(ModuleError::new(
                ErrorCode::ModuleNotFound,
                format!("Module '{module_id}' not found"),
            ));
        }

        // PROTOCOL_SPEC 6.7.1.1: every statistic below is computed over the
        // requested window. These four accessors previously had no period-aware
        // form at all, so `period` was echoed back over full-history numbers.
        let window = Some(reject_malformed_period(period)?);
        let stats = self
            .collector
            .get_module_summary_for_period(module_id, window);
        let p99 = self
            .collector
            .get_p99_latency_ms_for_period(module_id, window);
        let callers: Vec<serde_json::Value> = self
            .collector
            .get_caller_breakdown_for_period(module_id, window)
            .into_iter()
            .map(|c| {
                json!({
                    "caller_id": c.caller_id,
                    "call_count": c.call_count,
                    "error_count": c.error_count,
                    "avg_latency_ms": c.avg_latency_ms,
                })
            })
            .collect();
        let hourly: Vec<serde_json::Value> = self
            .collector
            .get_hourly_distribution_for_period(module_id, window)
            .into_iter()
            .map(|h| {
                json!({
                    "hour": h.hour,
                    "call_count": h.call_count,
                    "error_count": h.error_count,
                })
            })
            .collect();

        // Spec MUST: always return exactly 24 hourly entries, zero-filling
        // gaps (sync finding A-D-13).
        let hourly = pad_hourly_distribution(&hourly);

        match stats {
            Some(s) => Ok(json!({
                "module_id": module_id,
                "period": period,
                "call_count": s.call_count,
                "error_count": s.error_count,
                "avg_latency_ms": s.avg_latency_ms,
                "p99_latency_ms": p99,
                "trend": s.trend,
                "callers": callers,
                "hourly_distribution": hourly,
            })),
            None => Ok(json!({
                "module_id": module_id,
                "period": period,
                "call_count": 0,
                "error_count": 0,
                "avg_latency_ms": 0.0,
                "p99_latency_ms": 0.0,
                // A module with no records at all is `current == 0` AND
                // `previous == 0`, which §6.7.1.5's table decides as "stable" —
                // the zero-zero row is ordered FIRST precisely so it wins over
                // the `current == 0` row. `inactive` means "had traffic, now has
                // none" and is wrong for a module that never ran.
                //
                // `UsageCollector::compute_trend` gets this right; this arm
                // bypassed it because `get_module_summary_for_period` returns
                // None for an unknown module. apcore-python and
                // apcore-typescript route through `_build_detail` /
                // `_buildDetail`, which run the table, and answer "stable".
                "trend": "stable",
                "callers": [],
                "hourly_distribution": hourly,
            })),
        }
    }
}
