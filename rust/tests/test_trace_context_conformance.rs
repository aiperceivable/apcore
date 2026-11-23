//! Drive `trace_context.json` — W3C TraceContext alignment (Issue #35,
//! docs/features/observability.md §W3C Alignment Rules).
//!
//! apcore-typescript drives this fixture from `tests/conformance.test.ts`; this
//! is the Rust counterpart. `tests/test_trace_context.rs` covers adjacent
//! ground by hand but never loads the fixture.
//!
//! Context wiring: the TS driver stashes the inbound TraceParent on
//! `ctx.data['_apcore.trace.inbound']`. The Rust equivalents are the two
//! well-known keys `TRACE_FLAGS_KEY` (`_apcore.trace.flags`) and
//! `TRACE_STATE_KEY` (`_apcore.trace.state`), which `TraceContext::inject`
//! reads to propagate the inbound sampling decision and vendor state.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::context::{Context, Identity};
use apcore::trace_context::{TraceContext, TraceParent, TRACE_FLAGS_KEY, TRACE_STATE_KEY};
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("trace_context.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("trace_context.json parses")
}

fn case_by_id(fx: &Value, id: &str) -> Value {
    fx["test_cases"]
        .as_array()
        .expect("test_cases is an array")
        .iter()
        .find(|tc| tc["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("trace_context.json no longer carries case `{id}`"))
        .clone()
}

/// Cases lifted out of the bulk loop into a test of their own, so a failure
/// names the case instead of the loop. These are NOT skipped — every id here
/// runs, and the `#[test]` below it is what runs it. `case_by_id` is still
/// called for each in the bulk test, so removing a case from the fixture
/// without removing it here fails loudly.
const QUARANTINED: &[&str] = &["parent_id_override_rejected_malformed"];

/// Build the header map the case describes. `tracestate_entry_count: N` asks
/// the harness to synthesise N `vendorNN=opaqueNN` entries — the same
/// convention apcore-typescript's driver uses, and the one the fixture's
/// `tracestate_first_key` / `tracestate_last_key` expectations encode.
fn headers_of(tc: &Value) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for (key, value) in tc["input"]["headers"]
        .as_object()
        .expect("input.headers is an object")
    {
        if key == "tracestate_entry_count" {
            let n = value.as_u64().expect("tracestate_entry_count is a number");
            let entries: Vec<String> = (0..n)
                .map(|i| format!("vendor{i:02}=opaque{i:02}"))
                .collect();
            headers.insert("tracestate".to_string(), entries.join(","));
        } else {
            headers.insert(
                key.clone(),
                value
                    .as_str()
                    .expect("header value is a string")
                    .to_string(),
            );
        }
    }
    headers
}

/// A Context carrying the inbound trace so `inject` round-trips flags and
/// tracestate rather than re-deriving them.
fn context_from(tp: &TraceParent) -> Context<Value> {
    let mut data: HashMap<String, Value> = HashMap::new();
    data.insert(
        TRACE_FLAGS_KEY.to_string(),
        json!(format!("{:02x}", tp.trace_flags)),
    );
    if !tp.tracestate.is_empty() {
        data.insert(
            TRACE_STATE_KEY.to_string(),
            Value::Array(
                tp.tracestate
                    .iter()
                    .map(|(k, v)| json!([k, v]))
                    .collect::<Vec<_>>(),
            ),
        );
    }
    Context {
        trace_id: tp.trace_id.clone(),
        identity: Some(Identity::new(
            "@conformance".to_string(),
            "conformance".to_string(),
            vec![],
            HashMap::new(),
        )),
        services: Value::Null,
        caller_id: None,
        data: Arc::new(parking_lot::RwLock::new(data)),
        call_chain: vec![],
        redacted_inputs: None,
        redacted_output: None,
        cancel_token: None,
        global_deadline: None,
        executor: None,
    }
}

fn entries_as_json(entries: &[(String, String)]) -> Value {
    Value::Array(entries.iter().map(|(k, v)| json!([k, v])).collect())
}

fn run_case(tc: &Value) {
    let id = tc["id"].as_str().expect("every case needs an id");
    let headers = headers_of(tc);
    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

    // The malformed-override case never gets a valid extraction path; it is
    // handled by the dedicated test below.
    if let Some(error) = expected.get("error") {
        let extracted = TraceContext::extract_context(&headers).unwrap_or_else(|| {
            panic!("[{id}] traceparent must extract before inject is exercised")
        });
        let ctx = context_from(&extracted.traceparent);
        let parent_id = tc["input"]["inject_parent_id"]
            .as_str()
            .unwrap_or_else(|| panic!("[{id}] error case needs input.inject_parent_id"));
        let err = TraceContext::inject_checked(&ctx, Some(parent_id), None, None)
            .expect_err("malformed parent_id must be rejected");
        let want_code = error["code"].as_str().expect("expected.error.code");
        let actual_code = serde_json::to_value(err.code).expect("ErrorCode serializes");
        assert_eq!(
            actual_code.as_str(),
            Some(want_code),
            "[{id}] error code (message: {})",
            err.message
        );
        return;
    }

    let extracted = TraceContext::extract_context(&headers);

    // Everything below needs a successful extraction; a `None` here fails the
    // case rather than skipping it.
    let tc_extracted = extracted
        .unwrap_or_else(|| panic!("[{id}] extract_context returned None for headers {headers:?}"));
    let tp = &tc_extracted.traceparent;
    let flags_hex = format!("{:02x}", tp.trace_flags);

    // The declared entry count, used to derive the dropped count exactly as
    // the TypeScript driver does.
    let declared = headers.get("tracestate").map_or(0, |raw| {
        if raw.is_empty() {
            0
        } else {
            raw.split(',').count()
        }
    });

    let mut injected: Option<HashMap<String, String>> = None;
    // No override: the plain `inject` entry point, which reads both the inbound
    // flags and the inbound tracestate off the Context. With an override: the
    // validating variant, which is the only way to pass a parent_id.
    let inject = |parent_id: Option<&str>| -> HashMap<String, String> {
        let ctx = context_from(tp);
        match parent_id {
            None => TraceContext::inject(&ctx),
            Some(pid) => TraceContext::inject_checked(&ctx, Some(pid), None, None)
                .expect("inject_checked ok"),
        }
    };

    for (field, want) in expected {
        match field.as_str() {
            "extract_succeeded" => assert!(
                want.as_bool().expect("extract_succeeded is a bool"),
                "[{id}] fixture must not expect a failed extraction here"
            ),
            "trace_id" => assert_eq!(json!(tp.trace_id), *want, "[{id}] trace_id"),
            "parent_id" => assert_eq!(json!(tp.parent_id), *want, "[{id}] parent_id"),
            "trace_flags" | "extracted_trace_flags" => {
                assert_eq!(json!(flags_hex), *want, "[{id}] {field}");
            }
            "tracestate_entries" => {
                assert_eq!(
                    entries_as_json(&tc_extracted.tracestate),
                    *want,
                    "[{id}] tracestate_entries"
                );
            }
            "tracestate_retained_count" => assert_eq!(
                tc_extracted.tracestate.len() as u64,
                want.as_u64().expect("count is a number"),
                "[{id}] tracestate_retained_count"
            ),
            "tracestate_dropped_count" => assert_eq!(
                (declared - tc_extracted.tracestate.len()) as u64,
                want.as_u64().expect("count is a number"),
                "[{id}] tracestate_dropped_count (declared={declared})"
            ),
            "tracestate_first_key" => assert_eq!(
                json!(tc_extracted.tracestate.first().expect("non-empty").0),
                *want,
                "[{id}] tracestate_first_key"
            ),
            "tracestate_last_key" => assert_eq!(
                json!(tc_extracted.tracestate.last().expect("non-empty").0),
                *want,
                "[{id}] tracestate_last_key"
            ),
            "reinjected_tracestate" => {
                let out = injected
                    .get_or_insert_with(|| inject(tc["input"]["inject_parent_id"].as_str()));
                assert_eq!(
                    json!(out.get("tracestate").cloned().unwrap_or_default()),
                    *want,
                    "[{id}] reinjected_tracestate"
                );
            }
            "injected_trace_flags" => {
                let out = injected
                    .get_or_insert_with(|| inject(tc["input"]["inject_parent_id"].as_str()));
                let parts: Vec<&str> = out["traceparent"].split('-').collect();
                assert_eq!(json!(parts[3]), *want, "[{id}] injected_trace_flags");
            }
            "injected_traceparent" => {
                let out = injected
                    .get_or_insert_with(|| inject(tc["input"]["inject_parent_id"].as_str()));
                assert_eq!(
                    json!(out["traceparent"]),
                    *want,
                    "[{id}] injected_traceparent"
                );
            }
            "parent_id_in_output" => {
                let out = injected
                    .get_or_insert_with(|| inject(tc["input"]["inject_parent_id"].as_str()));
                let parts: Vec<&str> = out["traceparent"].split('-').collect();
                assert_eq!(json!(parts[2]), *want, "[{id}] parent_id_in_output");
            }
            other => panic!(
                "[{id}] trace_context.json grew expectation `{other}` that this driver \
                 does not check — teach the driver, do not skip it"
            ),
        }
    }
}

#[test]
fn conformance_trace_context() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");

    for id in QUARANTINED {
        let _ = case_by_id(&fx, id);
    }

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        if QUARANTINED.contains(&id) {
            continue; // run by conformance_parent_id_override_rejected_malformed below
        }
        run_case(tc);
    }
}

/// Case `parent_id_override_rejected_malformed`.
///
/// `TraceContext::inject_checked` rejects a `parent_id` that does not match
/// `^[0-9a-f]{16}$` with `ErrorCode::InvalidParentId`, serialising to
/// `INVALID_PARENT_ID` as required by the fixture,
/// `docs/features/observability.md` §"Optional `parent_id` Override on
/// `inject()`" and decision D-51. apcore-typescript sets the same code
/// (`src/trace-context.ts:71`) and apcore-python raises `ValueError`.
#[test]
fn conformance_parent_id_override_rejected_malformed() {
    let fx = fixture();
    run_case(&case_by_id(&fx, "parent_id_override_rejected_malformed"));
}
