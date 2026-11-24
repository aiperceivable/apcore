// Tests for built-in context key constants.

use apcore::context_key::ContextKey;
use apcore::context_keys::{
    LOGGING_START, METRICS_STARTS, REDACTED_OUTPUT, RETRY_COUNT_BASE, TRACING_SAMPLED,
    TRACING_SPANS,
};

#[test]
fn test_tracing_spans_name() {
    assert_eq!(TRACING_SPANS.name.as_ref(), "_apcore.mw.tracing.spans");
}

#[test]
fn test_tracing_sampled_name() {
    assert_eq!(TRACING_SAMPLED.name.as_ref(), "_apcore.mw.tracing.sampled");
}

#[test]
fn test_metrics_starts_name() {
    assert_eq!(METRICS_STARTS.name.as_ref(), "_apcore.mw.metrics.starts");
}

#[test]
fn test_logging_start_name() {
    assert_eq!(LOGGING_START.name.as_ref(), "_apcore.mw.logging.start_time");
}

#[test]
fn test_redacted_output_name() {
    assert_eq!(
        REDACTED_OUTPUT.name.as_ref(),
        "_apcore.executor.redacted_output"
    );
}

#[test]
fn test_retry_count_base_name() {
    assert_eq!(RETRY_COUNT_BASE.name.as_ref(), "_apcore.mw.retry.count");
}

#[test]
fn test_retry_count_base_scoped() {
    let scoped = RETRY_COUNT_BASE.scoped("my_module");
    assert_eq!(scoped.name.as_ref(), "_apcore.mw.retry.count.my_module");
}

#[test]
fn test_all_keys_are_context_key_instances() {
    // Compile-time verification: all constants are ContextKey<_> instances.
    // If any were not, this function would fail to compile.
    fn assert_is_context_key<T>(_key: &ContextKey<T>) {}
    assert_is_context_key(&TRACING_SPANS);
    assert_is_context_key(&TRACING_SAMPLED);
    assert_is_context_key(&METRICS_STARTS);
    assert_is_context_key(&LOGGING_START);
    assert_is_context_key(&REDACTED_OUTPUT);
    assert_is_context_key(&RETRY_COUNT_BASE);
}
