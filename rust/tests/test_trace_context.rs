//! Tests for TraceParent and TraceContext.

use apcore::context::{Context, Identity};
use apcore::trace_context::{TraceContext, TraceParent};
use std::collections::HashMap;

fn make_traceparent(trace_id: &str, parent_id: &str) -> TraceParent {
    TraceParent {
        version: 0,
        trace_id: trace_id.to_string(),
        parent_id: parent_id.to_string(),
        trace_flags: 1,
        tracestate: vec![],
    }
}

// ---------------------------------------------------------------------------
// TraceParent
// ---------------------------------------------------------------------------

#[test]
fn test_traceparent_to_header() {
    let tp = make_traceparent("4bf92f3577b34da6a3ce929d0e0e4736", "00f067aa0ba902b7");
    let header = tp.to_header();
    assert_eq!(
        header,
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    );
}

#[test]
fn test_traceparent_version_zero_formatted_as_two_hex_digits() {
    let tp = make_traceparent("abc", "def");
    let header = tp.to_header();
    assert!(header.starts_with("00-"));
}

#[test]
fn test_traceparent_flags_formatted_as_two_hex_digits() {
    let tp = TraceParent {
        version: 0,
        trace_id: "aaa".to_string(),
        parent_id: "bbb".to_string(),
        trace_flags: 0,
        tracestate: vec![],
    };
    let header = tp.to_header();
    assert!(header.ends_with("-00"));
}

#[test]
fn test_traceparent_serialization() {
    let tp = make_traceparent("trace-abc", "span-xyz");
    let json = serde_json::to_string(&tp).unwrap();
    let restored: TraceParent = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.trace_id, tp.trace_id);
    assert_eq!(restored.parent_id, tp.parent_id);
    assert_eq!(restored.version, tp.version);
    assert_eq!(restored.trace_flags, tp.trace_flags);
}

// ---------------------------------------------------------------------------
// TraceContext
// ---------------------------------------------------------------------------

#[test]
fn test_trace_context_new() {
    let tp = make_traceparent("trace-1", "parent-1");
    let ctx = TraceContext::new(tp);
    assert_eq!(ctx.traceparent.trace_id, "trace-1");
    assert!(ctx.tracestate.is_empty());
    assert!(ctx.baggage.is_empty());
}

#[test]
fn test_trace_context_baggage() {
    let tp = make_traceparent("t", "p");
    let mut ctx = TraceContext::new(tp);
    ctx.baggage
        .insert("user_id".to_string(), "alice".to_string());
    assert_eq!(ctx.baggage["user_id"], "alice");
}

#[test]
fn test_trace_context_tracestate() {
    let tp = make_traceparent("t", "p");
    let mut ctx = TraceContext::new(tp);
    ctx.tracestate
        .push(("vendor1".to_string(), "abc123".to_string()));
    assert_eq!(ctx.tracestate.len(), 1);
    assert_eq!(ctx.tracestate[0].0, "vendor1");
}

#[test]
fn test_trace_context_serialization() {
    let tp = make_traceparent("trace-abc", "span-xyz");
    let ctx = TraceContext::new(tp);
    let json = serde_json::to_string(&ctx).unwrap();
    let restored: TraceContext = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.traceparent.trace_id, "trace-abc");
}

// ---------------------------------------------------------------------------
// W3C tracestate end-to-end (issue #35 — item 1)
// ---------------------------------------------------------------------------

fn make_apcore_context() -> Context<serde_json::Value> {
    Context::<serde_json::Value>::new(Identity::new(
        "caller".to_string(),
        "user".to_string(),
        vec![],
        HashMap::default(),
    ))
}

#[test]
fn test_extract_context_populates_tracestate() {
    let mut headers = HashMap::new();
    headers.insert(
        "traceparent".to_string(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
    );
    headers.insert(
        "tracestate".to_string(),
        "vendor1=abc123,vendor2=def456".to_string(),
    );
    let tc = TraceContext::extract_context(&headers).expect("must extract");
    assert_eq!(tc.tracestate.len(), 2);
    assert_eq!(
        tc.tracestate[0],
        ("vendor1".to_string(), "abc123".to_string())
    );
    assert_eq!(
        tc.tracestate[1],
        ("vendor2".to_string(), "def456".to_string())
    );
}

#[test]
fn test_extract_context_drops_malformed_tracestate_entries() {
    let mut headers = HashMap::new();
    headers.insert(
        "traceparent".to_string(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
    );
    // Second entry has no '=' and must be dropped; whitespace must be trimmed.
    headers.insert(
        "tracestate".to_string(),
        "  vendor1=abc123  ,  bogus  , vendor2=def456 ".to_string(),
    );
    let tc = TraceContext::extract_context(&headers).expect("must extract");
    assert_eq!(tc.tracestate.len(), 2);
    assert_eq!(tc.tracestate[0].0, "vendor1");
    assert_eq!(tc.tracestate[1].0, "vendor2");
}

#[test]
fn test_extract_context_caps_tracestate_at_32_entries() {
    let mut headers = HashMap::new();
    headers.insert(
        "traceparent".to_string(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
    );
    let parts: Vec<String> = (0..40).map(|i| format!("v{i}=x{i}")).collect();
    headers.insert("tracestate".to_string(), parts.join(","));
    let tc = TraceContext::extract_context(&headers).expect("must extract");
    assert_eq!(tc.tracestate.len(), 32);
    assert_eq!(tc.tracestate[0].0, "v0");
    assert_eq!(tc.tracestate[31].0, "v31");
}

#[test]
fn test_inject_emits_tracestate_when_nonempty() {
    let ctx = make_apcore_context();
    let tracestate = vec![
        ("vendor1".to_string(), "abc123".to_string()),
        ("vendor2".to_string(), "def456".to_string()),
    ];
    let headers = TraceContext::inject_with_options(&ctx, None, None, Some(&tracestate));
    assert!(
        headers.contains_key("tracestate"),
        "tracestate header must be emitted"
    );
    let value = &headers["tracestate"];
    assert_eq!(value, "vendor1=abc123,vendor2=def456");
}

#[test]
fn test_inject_omits_tracestate_when_empty() {
    let ctx = make_apcore_context();
    let headers = TraceContext::inject(&ctx);
    assert!(
        !headers.contains_key("tracestate"),
        "tracestate header must be omitted when empty"
    );
}

// ---------------------------------------------------------------------------
// D11-002b: ContextBuilder must carry inbound tracestate forward so
// TraceContext::inject(context) propagates vendor state. Previously only
// trace_flags was seeded into context.data; tracestate was lost across
// the request boundary unless the caller used inject_with_options(...).
// ---------------------------------------------------------------------------

#[test]
fn test_context_builder_seeds_tracestate_into_context_data() {
    use apcore::context::ContextBuilder;
    // Per Issue #66, tracestate is a field on TraceParent rather than a
    // separate builder setter.
    let entries = vec![
        ("vendor1".to_string(), "abc123".to_string()),
        ("vendor2".to_string(), "def456".to_string()),
    ];
    let tp = TraceParent {
        version: 0,
        trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
        parent_id: "00f067aa0ba902b7".to_string(),
        trace_flags: 1,
        tracestate: entries.clone(),
    };
    let ctx: apcore::context::Context<serde_json::Value> =
        ContextBuilder::new().trace_parent(Some(tp)).build();
    let headers = TraceContext::inject(&ctx);
    assert!(
        headers.contains_key("tracestate"),
        "inject() must emit tracestate header from inbound state"
    );
    assert_eq!(headers["tracestate"], "vendor1=abc123,vendor2=def456");
}

#[test]
fn test_context_builder_no_tracestate_means_no_outbound_header() {
    use apcore::context::ContextBuilder;
    let tp = make_traceparent("4bf92f3577b34da6a3ce929d0e0e4736", "00f067aa0ba902b7");
    let ctx: apcore::context::Context<serde_json::Value> =
        ContextBuilder::new().trace_parent(Some(tp)).build();
    let headers = TraceContext::inject(&ctx);
    assert!(
        !headers.contains_key("tracestate"),
        "no inbound tracestate means inject() must not emit header"
    );
}

// ---------------------------------------------------------------------------
// Case-insensitive header KEY lookup (issue #35 — item 2)
// ---------------------------------------------------------------------------

#[test]
fn test_extract_case_insensitive_traceparent_key() {
    let mut headers = HashMap::new();
    headers.insert(
        "TraceParent".to_string(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
    );
    let tp = TraceContext::extract(&headers).expect("must look up regardless of case");
    assert_eq!(tp.trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
}

#[test]
fn test_extract_case_insensitive_uppercase_key() {
    let mut headers = HashMap::new();
    headers.insert(
        "TRACEPARENT".to_string(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
    );
    let tp = TraceContext::extract(&headers).expect("must look up regardless of case");
    assert_eq!(tp.parent_id, "00f067aa0ba902b7");
}

#[test]
fn test_extract_context_case_insensitive_tracestate_key() {
    let mut headers = HashMap::new();
    headers.insert(
        "Traceparent".to_string(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
    );
    headers.insert("TraceState".to_string(), "vendor1=abc123".to_string());
    let tc = TraceContext::extract_context(&headers).expect("must extract");
    assert_eq!(tc.tracestate.len(), 1);
    assert_eq!(tc.tracestate[0].0, "vendor1");
}

// ---------------------------------------------------------------------------
// Honor incoming trace_flags on inject (issue #35 — item 3)
// ---------------------------------------------------------------------------

#[test]
fn test_inject_with_options_honors_trace_flags_unsampled() {
    let ctx = make_apcore_context();
    // Flag 0x00 = unsampled — must propagate, not be hardcoded to 01.
    let headers = TraceContext::inject_with_options(&ctx, None, Some(0u8), None);
    let tp = headers.get("traceparent").expect("traceparent set");
    assert!(tp.ends_with("-00"), "expected -00 flags, got: {tp}");
}

#[test]
fn test_inject_with_options_honors_trace_flags_sampled() {
    let ctx = make_apcore_context();
    let headers = TraceContext::inject_with_options(&ctx, None, Some(1u8), None);
    let tp = headers.get("traceparent").expect("traceparent set");
    assert!(tp.ends_with("-01"));
}

#[test]
fn test_inject_default_flags_are_sampled() {
    let ctx = make_apcore_context();
    let headers = TraceContext::inject(&ctx);
    let tp = headers.get("traceparent").expect("traceparent set");
    assert!(tp.ends_with("-01"), "default new-root flags = 01");
}

// ---------------------------------------------------------------------------
// Optional parent_id override (issue #35 — item 4)
// ---------------------------------------------------------------------------

#[test]
fn test_inject_with_options_uses_explicit_parent_id() {
    let ctx = make_apcore_context();
    let parent_id = "00f067aa0ba902b7";
    let headers = TraceContext::inject_with_options(&ctx, Some(parent_id), None, None);
    let tp = headers.get("traceparent").expect("traceparent set");
    let parts: Vec<&str> = tp.split('-').collect();
    assert_eq!(parts[2], parent_id);
}

#[test]
fn test_inject_with_options_rejects_invalid_parent_id_falls_back_to_random() {
    let ctx = make_apcore_context();
    // Not 16 lowercase hex
    let headers = TraceContext::inject_with_options(&ctx, Some("ZZZ"), None, None);
    let tp = headers.get("traceparent").expect("traceparent set");
    let parts: Vec<&str> = tp.split('-').collect();
    assert_eq!(parts[2].len(), 16);
    // The invalid value must not appear verbatim
    assert_ne!(parts[2], "ZZZ");
    // And must be valid hex
    assert!(parts[2]
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn test_inject_simple_signature_still_works() {
    // The original public signature must keep behaving identically (backwards-compat shim).
    let ctx = make_apcore_context();
    let headers = TraceContext::inject(&ctx);
    assert!(headers.contains_key("traceparent"));
    let tp = &headers["traceparent"];
    assert!(tp.starts_with("00-") && tp.ends_with("-01"));
}
