// APCore Protocol — System health modules
// Spec reference: system.health.summary, system.health.module

use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::context::Context;
use crate::errors::ModuleError;
use crate::module::Module;
use crate::observability::error_history::ErrorHistory;
use crate::observability::metrics::{
    extract_module_call_counts, extract_module_latency_stats, MetricsCollector, ModuleCallCounts,
    ModuleLatencyStats,
};
use crate::registry::registry::Registry;

// NOTE: `registry` is now a plain `Arc<Registry>` — interior mutability via
// `parking_lot::RwLock` means no external lock is needed.

fn classify_health(error_rate: f64, total_calls: u64, threshold: f64) -> &'static str {
    if total_calls == 0 {
        return "unknown";
    }
    if error_rate < threshold {
        "healthy"
    } else if error_rate < 0.10 {
        "degraded"
    } else {
        "error"
    }
}

/// The module's most frequently recorded error, or `None` when it has none.
///
/// SYS-4: `top_error` names a frequency, not a recency. `ErrorHistory::get`
/// returns entries sorted by `last_occurred` descending, so taking the head
/// reported the most RECENT error — a single one-off failure hid the recurring
/// one. apcore-python resolves it with `max(entries, key=lambda e: e.count)`.
///
/// Ties resolve to the more recently seen entry: the scan keeps the first
/// strict maximum of the `last_occurred`-descending list, which is what
/// Python's `max` returns over the same ordering.
fn most_frequent_error(
    history: &ErrorHistory,
    module_id: &str,
) -> Option<crate::observability::error_history::ErrorEntry> {
    let mut best: Option<crate::observability::error_history::ErrorEntry> = None;
    for entry in history.get(module_id, None) {
        if best.as_ref().is_none_or(|b| entry.count > b.count) {
            best = Some(entry);
        }
    }
    best
}

/// system.health.summary — Aggregated health overview of all registered modules.
pub struct HealthSummaryModule {
    registry: Arc<Registry>,
    metrics: Option<MetricsCollector>,
    error_history: ErrorHistory,
    config: Arc<Mutex<Config>>,
}

impl HealthSummaryModule {
    pub fn new(
        registry: Arc<Registry>,
        metrics: Option<MetricsCollector>,
        error_history: ErrorHistory,
        config: Arc<Mutex<Config>>,
    ) -> Self {
        Self {
            registry,
            metrics,
            error_history,
            config,
        }
    }
}

#[async_trait]
impl Module for HealthSummaryModule {
    fn description(&self) -> &'static str {
        "Aggregated health overview of all registered modules"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "error_rate_threshold": {"type": "number", "default": 0.01},
                "include_healthy": {"type": "boolean", "default": true}
            }
        })
    }

    // PROTOCOL_SPEC §6.7.1.6 (SYS-24): the full field contract, transcribed
    // from apcore/schemas/sys-health-summary.schema.json the way `usage.rs`
    // already does. A bare `{"type": "object"}` tells a caller nothing and
    // tells a schema-driven adapter less.
    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["project", "summary", "modules"],
            "properties": {
                "project": {
                    "type": "object",
                    "description": "Project metadata",
                    "properties": {
                        "name": {"type": "string", "description": "Project name"},
                        "version": {"type": ["string", "null"], "description": "Project version"}
                    }
                },
                "summary": {
                    "type": "object",
                    "description": "Aggregate health statistics",
                    "properties": {
                        "total_modules": {"type": "integer", "description": "Total number of registered modules"},
                        "healthy": {"type": "integer", "description": "Number of healthy modules"},
                        "degraded": {"type": "integer", "description": "Number of degraded modules"},
                        "error": {"type": "integer", "description": "Number of modules in the `error` tier"},
                        "unknown": {"type": "integer", "description": "Number of modules that have recorded no calls yet"}
                    }
                },
                "modules": {
                    "type": "array",
                    "description": "Per-module health status entries",
                    "items": {
                        "type": "object",
                        "properties": {
                            "module_id": {"type": "string", "description": "Canonical module ID"},
                            "status": {"type": "string", "enum": ["healthy", "degraded", "error", "unknown"], "description": "Health status; `unknown` means no calls recorded yet"},
                            "error_rate": {"type": "number", "minimum": 0.0, "maximum": 1.0, "description": "Failed calls over total calls, 0.0-1.0"},
                            "top_error": {
                                "type": ["object", "null"],
                                "description": "The module's most frequent recorded error, or null when it has none",
                                "required": ["code", "message", "count"],
                                "properties": {
                                    "code": {"type": "string", "description": "Canonical error code"},
                                    "message": {"type": "string", "description": "Error message"},
                                    "ai_guidance": {"type": ["string", "null"], "description": "Remediation guidance for an AI caller, when the error carries one"},
                                    "count": {"type": "integer", "minimum": 1, "description": "How many times this error was recorded"}
                                }
                            }
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
        let threshold = inputs
            .get("error_rate_threshold")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.01);
        let include_healthy = inputs
            .get("include_healthy")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        let module_ids = self.registry.list(None, None, None);

        let project_name = {
            let cfg = self.config.lock().await;
            cfg.get("project.name")
                .and_then(|v| v.as_str().map(std::string::ToString::to_string))
                .unwrap_or_else(|| "apcore".to_string())
        };

        let snapshot = self
            .metrics
            .as_ref()
            .map(super::super::observability::metrics::MetricsCollector::snapshot);

        let mut modules = Vec::new();
        let (mut healthy, mut degraded, mut error_count, mut unknown) = (0u32, 0u32, 0u32, 0u32);

        for mid in &module_ids {
            let counts = snapshot
                .as_ref()
                .map_or_else(ModuleCallCounts::default, |s| {
                    extract_module_call_counts(s, mid.as_str())
                });
            let total_calls = counts.total;
            let error_rate = counts.error_rate();
            let status = classify_health(error_rate, total_calls, threshold);

            match status {
                "healthy" => healthy += 1,
                "degraded" => degraded += 1,
                "error" => error_count += 1,
                _ => unknown += 1,
            }

            if !include_healthy && status == "healthy" {
                continue;
            }

            // SYS-4: the most FREQUENT error, not the most recent — the field
            // is named `top_error`, and apcore-python resolves it with
            // `max(entries, key=lambda e: e.count)`. Reading `get(.., Some(1))`
            // took the head of a `last_occurred`-descending list, so a one-off
            // failure displaced the recurring one the operator needs to see.
            // Ties keep the more recent entry, matching Python's `max`, which
            // returns the first maximal element of that same ordering.
            let top_error = most_frequent_error(&self.error_history, mid.as_str()).map(|e| {
                json!({
                    "code": e.error_code,
                    "message": e.message,
                    "ai_guidance": e.ai_guidance,
                    "count": e.count,
                })
            });

            modules.push(json!({
                "module_id": mid,
                "status": status,
                "error_rate": error_rate,
                "top_error": top_error,
            }));
        }

        Ok(json!({
            "project": { "name": project_name },
            "summary": {
                "total_modules": module_ids.len(),
                "healthy": healthy,
                "degraded": degraded,
                "error": error_count,
                "unknown": unknown,
            },
            "modules": modules,
        }))
    }
}

/// system.health.module — Detailed health for a single module.
pub struct HealthModule {
    registry: Arc<Registry>,
    metrics: Option<MetricsCollector>,
    error_history: ErrorHistory,
}

impl HealthModule {
    pub fn new(
        registry: Arc<Registry>,
        metrics: Option<MetricsCollector>,
        error_history: ErrorHistory,
    ) -> Self {
        Self {
            registry,
            metrics,
            error_history,
        }
    }
}

#[async_trait]
impl Module for HealthModule {
    fn description(&self) -> &'static str {
        "Detailed health information for a single module"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["module_id"],
            "properties": {
                "module_id": {"type": "string"},
                "error_limit": {"type": "integer", "default": 10}
            }
        })
    }

    // PROTOCOL_SPEC §6.7.1.6 (SYS-24). Canonical shape:
    // apcore/schemas/sys-health-module.schema.json.
    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["module_id", "status", "total_calls", "error_count", "error_rate"],
            "properties": {
                "module_id": {"type": "string", "description": "Canonical module ID"},
                "status": {"type": "string", "enum": ["healthy", "degraded", "error", "unknown"], "description": "Health status; `unknown` means no calls recorded yet"},
                "total_calls": {"type": "integer", "description": "Total number of calls to this module"},
                "error_count": {"type": "integer", "description": "Total number of errors from this module"},
                "error_rate": {"type": "number", "description": "Error rate as a decimal (0.0 to 1.0)"},
                "avg_latency_ms": {"type": "number", "description": "Average execution latency in milliseconds"},
                "p99_latency_ms": {"type": "number", "description": "99th percentile execution latency in milliseconds"},
                "recent_errors": {
                    "type": "array",
                    "description": "Recent error entries, most recent first",
                    "items": {
                        "type": "object",
                        "properties": {
                            "code": {"type": "string", "description": "Canonical error code"},
                            "message": {"type": "string", "description": "Error message"},
                            "ai_guidance": {"type": ["string", "null"], "description": "Remediation guidance for an AI caller, when the error carries one"},
                            "count": {"type": "integer", "description": "Number of occurrences"},
                            "first_occurred": {"type": "string", "description": "RFC 3339 timestamp of the first occurrence"},
                            "last_occurred": {"type": "string", "description": "RFC 3339 timestamp of the most recent occurrence"}
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
        #[allow(clippy::cast_possible_truncation)]
        // config value won't exceed platform usize limits
        let error_limit = inputs
            .get("error_limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(10) as usize;

        if !self.registry.has(module_id) {
            return Err(ModuleError::module_not_found(module_id));
        }

        let snapshot = self
            .metrics
            .as_ref()
            .map(super::super::observability::metrics::MetricsCollector::snapshot);
        let counts = snapshot
            .as_ref()
            .map_or_else(ModuleCallCounts::default, |s| {
                extract_module_call_counts(s, module_id)
            });
        let (total_calls, errors) = (counts.total, counts.errors);
        let error_rate = counts.error_rate();
        let status = classify_health(error_rate, total_calls, 0.01);

        let recent_errors: Vec<serde_json::Value> = self
            .error_history
            .get(module_id, Some(error_limit))
            .into_iter()
            .map(|e| {
                json!({
                    "code": e.error_code,
                    "message": e.message,
                    "ai_guidance": e.ai_guidance,
                    "count": e.count,
                    // D-120: the same `…123Z` form the record itself serializes.
                    // `to_rfc3339()` gives `+00:00` and the instant's own
                    // precision, so this summary used to disagree with the
                    // stored record it was summarizing.
                    "first_occurred": crate::observability::error_history::format_millis_z(e.first_occurred),
                    "last_occurred": crate::observability::error_history::format_millis_z(e.last_occurred),
                })
            })
            .collect();

        let ModuleLatencyStats {
            avg_ms: avg_latency_ms,
            p99_ms: p99_latency_ms,
            ..
        } = snapshot
            .as_ref()
            .map_or_else(ModuleLatencyStats::default, |s| {
                extract_module_latency_stats(s, module_id)
            });

        Ok(json!({
            "module_id": module_id,
            "status": status,
            "total_calls": total_calls,
            "error_count": errors,
            "error_rate": error_rate,
            "avg_latency_ms": avg_latency_ms,
            "p99_latency_ms": p99_latency_ms,
            "recent_errors": recent_errors,
        }))
    }
}

// Snapshot extraction lives in `observability::metrics`
// (`extract_module_call_counts` / `extract_module_latency_stats`) so this module
// and `PlatformNotifyMiddleware` cannot drift apart on the label spelling again.
