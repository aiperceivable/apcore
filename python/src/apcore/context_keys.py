"""Built-in context key constants for apcore framework middleware."""

from apcore.context_key import ContextKey

# Direct keys -- used as-is by middleware
TRACING_SPANS: ContextKey[list] = ContextKey("_apcore.mw.tracing.spans")
TRACING_SAMPLED: ContextKey[bool] = ContextKey("_apcore.mw.tracing.sampled")
METRICS_STARTS: ContextKey[list] = ContextKey("_apcore.mw.metrics.starts")
LOGGING_START: ContextKey[float] = ContextKey("_apcore.mw.logging.start_time")
USAGE_STARTS: ContextKey[list] = ContextKey("_apcore.mw.usage.starts")
LOGGING_STARTS: ContextKey[list] = ContextKey("_apcore.mw.logging.starts")
REDACTED_OUTPUT: ContextKey[dict] = ContextKey("_apcore.executor.redacted_output")

# Base keys -- always use .scoped(module_id) for per-module sub-keys
RETRY_COUNT_BASE: ContextKey[int] = ContextKey("_apcore.mw.retry.count")
