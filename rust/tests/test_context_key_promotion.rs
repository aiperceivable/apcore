// Issue #63 — ContextKey<T> export verification
// Verifies that ContextKey and the 6 built-in constants are part of the public
// API and that their names match the spec identifiers.

use apcore::context::Context;
use apcore::context_keys::{
    LOGGING_START, METRICS_STARTS, REDACTED_OUTPUT, RETRY_COUNT_BASE, TRACING_SAMPLED,
    TRACING_SPANS,
};
use apcore::ContextKey;
use serde_json::Value;

fn make_test_context() -> Context<Value> {
    Context::anonymous()
}

#[test]
fn builtin_identifiers_match_spec() {
    assert_eq!(TRACING_SPANS.name.as_ref(), "_apcore.mw.tracing.spans");
    assert_eq!(TRACING_SAMPLED.name.as_ref(), "_apcore.mw.tracing.sampled");
    assert_eq!(METRICS_STARTS.name.as_ref(), "_apcore.mw.metrics.starts");
    assert_eq!(LOGGING_START.name.as_ref(), "_apcore.mw.logging.start_time");
    assert_eq!(
        REDACTED_OUTPUT.name.as_ref(),
        "_apcore.executor.redacted_output"
    );
    assert_eq!(RETRY_COUNT_BASE.name.as_ref(), "_apcore.mw.retry.count");
}

#[test]
fn key_anchored_api_roundtrip() {
    static KEY: ContextKey<u32> = ContextKey::new("ext.test.retry.count");
    let ctx = make_test_context();
    KEY.set(&ctx, 3u32);
    assert_eq!(KEY.get(&ctx), Some(3u32));
}

#[test]
fn key_delete_removes_value() {
    static KEY: ContextKey<String> = ContextKey::new("ext.test.delete_me");
    let ctx = make_test_context();
    KEY.set(&ctx, "hello".to_string());
    assert!(KEY.exists(&ctx));
    KEY.delete(&ctx);
    assert!(!KEY.exists(&ctx));
    assert_eq!(KEY.get(&ctx), None);
}

#[test]
fn key_scoped_produces_correctly_named_subkey() {
    static BASE: ContextKey<i64> = ContextKey::new("_apcore.mw.retry.count");
    let scoped = BASE.scoped("module.foo");
    assert_eq!(scoped.name.as_ref(), "_apcore.mw.retry.count.module.foo");
}

#[test]
fn ext_key_works_equivalently_to_builtin() {
    static EXT_KEY: ContextKey<serde_json::Value> = ContextKey::new("ext.plugin.my_data");
    let ctx = make_test_context();
    let val = serde_json::json!({"foo": 1});
    EXT_KEY.set(&ctx, val.clone());
    assert_eq!(EXT_KEY.get(&ctx), Some(val));
}

#[test]
fn builtin_key_roundtrip_tracing_sampled() {
    let ctx = make_test_context();
    TRACING_SAMPLED.set(&ctx, true);
    assert_eq!(TRACING_SAMPLED.get(&ctx), Some(true));
}

#[test]
fn builtin_key_roundtrip_retry_count_scoped() {
    let ctx = make_test_context();
    let scoped = RETRY_COUNT_BASE.scoped("math.add");
    scoped.set(&ctx, 2i64);
    assert_eq!(scoped.get(&ctx), Some(2i64));
    // Base key is unaffected by writing to the scoped key.
    assert_eq!(RETRY_COUNT_BASE.get(&ctx), None);
}
