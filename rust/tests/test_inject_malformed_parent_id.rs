//! Sync alignment (W-6): `TraceContext::inject_with_options` MUST reject a
//! malformed `parent_id` override rather than silently falling back to a
//! random value. PY/TS raise / throw on this; Rust returns a `ModuleError`.
//!
//! The code is `INVALID_PARENT_ID` (decision D-51), not the generic
//! `GENERAL_INVALID_INPUT` these tests used to assert. apcore-typescript
//! already threw `INVALID_PARENT_ID`, so the generic code made Rust the
//! outlier: a caller matching on the specific code got nothing to match.

use std::collections::HashMap;

use apcore::context::{Context, Identity};
use apcore::errors::ErrorCode;
use apcore::trace_context::TraceContext;

fn make_context() -> Context<serde_json::Value> {
    Context::<serde_json::Value>::new(Identity::new(
        "caller".to_string(),
        "user".to_string(),
        vec![],
        HashMap::default(),
    ))
}

#[test]
fn inject_checked_returns_error_on_malformed_parent_id() {
    let ctx = make_context();
    let err = TraceContext::inject_checked(&ctx, Some("not-hex"), None, None)
        .expect_err("malformed parent_id must error");
    assert_eq!(err.code, ErrorCode::InvalidParentId);
    assert!(
        err.message.to_lowercase().contains("parent_id"),
        "error message must mention parent_id, got: {}",
        err.message
    );
}

#[test]
fn inject_checked_returns_error_on_short_parent_id() {
    let ctx = make_context();
    // 15 hex chars instead of 16
    let err = TraceContext::inject_checked(&ctx, Some("00f067aa0ba902b"), None, None)
        .expect_err("short parent_id must error");
    assert_eq!(err.code, ErrorCode::InvalidParentId);
}

#[test]
fn inject_checked_accepts_valid_parent_id() {
    let ctx = make_context();
    let headers = TraceContext::inject_checked(&ctx, Some("00f067aa0ba902b7"), None, None)
        .expect("valid parent_id must succeed");
    let tp = headers.get("traceparent").unwrap();
    assert!(tp.contains("00f067aa0ba902b7"), "got: {tp}");
}

#[test]
fn inject_checked_accepts_none_parent_id() {
    let ctx = make_context();
    let headers =
        TraceContext::inject_checked(&ctx, None, None, None).expect("None parent_id must succeed");
    assert!(headers.contains_key("traceparent"));
}
