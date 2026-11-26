"""Built-in context key constants for apcore framework middleware."""

from typing import Any

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


def _middleware_data(context: Any) -> dict[str, Any]:
    """Return per-invocation middleware storage, or legacy data for direct hooks."""
    invocation_data = getattr(context, "_invocation_data", None)
    return invocation_data if isinstance(invocation_data, dict) else context.data


def _current_trace_span(context: Any, *, include_parent: bool = False) -> Any:
    """Find the invocation span without mistaking a sibling for its parent."""
    data = _middleware_data(context)
    spans = data.get(TRACING_SPANS.name)
    if isinstance(spans, list) and spans:
        return spans[-1]
    if include_parent and (parent := getattr(context, "_local_parent_span", None)) is not None:
        return parent
    # Adapters can still supply an explicit legacy span in Context.data.
    if data is not context.data:
        spans = TRACING_SPANS.get(context)
        if isinstance(spans, list) and spans:
            return spans[-1]
    return None
