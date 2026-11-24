//! Built-in context key constants for apcore framework middleware.

use crate::context_key::ContextKey;

// Direct keys -- used as-is by middleware
pub const TRACING_SPANS: ContextKey<Vec<serde_json::Value>> =
    ContextKey::new("_apcore.mw.tracing.spans");
pub const TRACING_SAMPLED: ContextKey<bool> = ContextKey::new("_apcore.mw.tracing.sampled");
pub const METRICS_STARTS: ContextKey<Vec<f64>> = ContextKey::new("_apcore.mw.metrics.starts");
pub const LOGGING_START: ContextKey<f64> = ContextKey::new("_apcore.mw.logging.start_time");
pub const REDACTED_OUTPUT: ContextKey<serde_json::Value> =
    ContextKey::new("_apcore.executor.redacted_output");

// Base keys -- always use .scoped(module_id) for per-module sub-keys
pub const RETRY_COUNT_BASE: ContextKey<i64> = ContextKey::new("_apcore.mw.retry.count");
