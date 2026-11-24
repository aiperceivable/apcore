//! Sync alignment: `TraceContext::inject` MUST propagate inbound trace_flags
//! from `context.data["_apcore.trace.flags"]` (matching the Python SDK's
//! `_TRACE_FLAGS_KEY`). When no inbound flags are present, the default is
//! `0x01` (sampled). This mirrors apcore-python's behaviour and keeps the W3C
//! sampling decision consistent across hops.

use std::collections::HashMap;

use apcore::context::{Context, ContextBuilder, Identity};
use apcore::trace_context::{TraceContext, TraceParent, TRACE_FLAGS_KEY};
use serde_json::json;

fn make_context() -> Context<serde_json::Value> {
    Context::<serde_json::Value>::new(Identity::new(
        "caller".to_string(),
        "user".to_string(),
        vec![],
        HashMap::default(),
    ))
}

#[test]
fn inject_defaults_to_01_when_no_inbound_flags() {
    let ctx = make_context();
    let headers = TraceContext::inject(&ctx);
    let tp = headers
        .get("traceparent")
        .expect("traceparent must be present");
    assert!(tp.ends_with("-01"), "expected default flags 01, got {tp}");
}

#[test]
fn inject_propagates_inbound_flags_zero_when_present() {
    // Stash inbound flag in context.data under the canonical key.
    let ctx = make_context();
    ctx.data
        .write()
        .insert(TRACE_FLAGS_KEY.to_string(), json!("00"));
    let headers = TraceContext::inject(&ctx);
    let tp = headers
        .get("traceparent")
        .expect("traceparent must be present");
    assert!(
        tp.ends_with("-00"),
        "expected propagated flags 00, got {tp}"
    );
}

#[test]
fn inject_propagates_inbound_flags_one_when_present() {
    let ctx = make_context();
    ctx.data
        .write()
        .insert(TRACE_FLAGS_KEY.to_string(), json!("01"));
    let headers = TraceContext::inject(&ctx);
    let tp = headers
        .get("traceparent")
        .expect("traceparent must be present");
    assert!(tp.ends_with("-01"));
}

#[test]
fn context_builder_seeds_trace_flags_from_trace_parent() {
    // When a Context is built from an inbound TraceParent, its trace_flags
    // MUST be stored under TRACE_FLAGS_KEY so subsequent inject() propagates.
    let parent = TraceParent {
        version: 0,
        trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
        parent_id: "00f067aa0ba902b7".to_string(),
        trace_flags: 0, // unsampled
        tracestate: vec![],
    };
    let ctx: Context<serde_json::Value> = ContextBuilder::<serde_json::Value>::new()
        .trace_parent(Some(parent))
        .build();

    let headers = TraceContext::inject(&ctx);
    let tp = headers.get("traceparent").unwrap();
    assert!(
        tp.ends_with("-00"),
        "ContextBuilder must seed inbound flags 00, got {tp}"
    );
}
