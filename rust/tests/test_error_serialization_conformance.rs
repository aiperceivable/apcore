//! Conformance driver for `error_serialization.json` (sync finding A-D-008).
//!
//! Locks the `ModuleError` wire format: the serialized form (`to_dict` /
//! serde) MUST use snake_case keys (`trace_id`, `ai_guidance`, `user_fixable`)
//! and MUST snake_case keys inside `details`, matching the Python `to_dict`,
//! TypeScript `toJSON`, and the serialization example in
//! `docs/features/error-system.md`. Null/None optional fields are omitted
//! (sparse output).
//!
//! Driver contract (from the fixture `description`): build a `ModuleError`
//! from `input`, serialize via the canonical serializer (`to_dict`), then
//! assert each key in `expected_keys_present` is present, each in
//! `expected_keys_absent` is absent, and likewise for the nested `details`
//! object.
#![allow(clippy::pedantic)] // fixture-driven test file: casts/layout follow the fixture schema

use std::collections::HashMap;

use apcore::errors::{ErrorCode, ModuleError};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Fixture loading (resolves apcore/conformance/fixtures, like the other
// conformance drivers in this crate).
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture() -> Value {
    let path = find_fixtures_root().join("error_serialization.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON: {e}"))
}

/// Map a SCREAMING_SNAKE_CASE code string to the typed `ErrorCode` via serde
/// (the enum is `#[serde(rename_all = "SCREAMING_SNAKE_CASE")]`).
fn error_code_from_str(code: &str) -> ErrorCode {
    serde_json::from_value(Value::String(code.to_string()))
        .unwrap_or_else(|e| panic!("unknown error code {code:?}: {e}"))
}

/// Build a `ModuleError` from a fixture `input` object, applying every
/// optional builder method that is present.
fn build_error(input: &Value) -> ModuleError {
    let code = error_code_from_str(input["code"].as_str().expect("input.code must be a string"));
    let message = input["message"]
        .as_str()
        .expect("input.message must be a string");

    let mut err = ModuleError::new(code, message);

    if let Some(trace_id) = input.get("trace_id").and_then(Value::as_str) {
        err = err.with_trace_id(trace_id);
    }
    if let Some(ai_guidance) = input.get("ai_guidance").and_then(Value::as_str) {
        err = err.with_ai_guidance(ai_guidance);
    }
    if let Some(retryable) = input.get("retryable").and_then(Value::as_bool) {
        err = err.with_retryable(retryable);
    }
    // `user_fixable` has no dedicated builder; set the public field directly.
    if let Some(user_fixable) = input.get("user_fixable").and_then(Value::as_bool) {
        err.user_fixable = Some(user_fixable);
    }
    if let Some(details_obj) = input.get("details").and_then(Value::as_object) {
        let details: HashMap<String, Value> = details_obj
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        err = err.with_details(details);
    }

    err
}

fn as_string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().expect("expected a string").to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn error_serialization_conformance() {
    let fixture = load_fixture();
    let cases = fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array");

    for tc in cases {
        let id = tc["id"].as_str().expect("each case needs an id");

        let serialized = build_error(&tc["input"]).to_dict();
        let obj = serialized.as_object().unwrap_or_else(|| {
            panic!("case {id}: to_dict() must serialize to a JSON object, got {serialized}")
        });

        // Top-level keys present.
        for key in as_string_list(&tc["expected_keys_present"]) {
            assert!(
                obj.contains_key(&key),
                "case {id}: expected top-level key {key:?} to be present in {serialized}"
            );
        }
        // Top-level keys absent.
        for key in as_string_list(&tc["expected_keys_absent"]) {
            assert!(
                !obj.contains_key(&key),
                "case {id}: expected top-level key {key:?} to be ABSENT in {serialized}"
            );
        }

        // Nested `details` keys. When the fixture lists no detail expectations
        // (and supplies no details), `details` is omitted entirely.
        let detail_present = as_string_list(&tc["expected_detail_keys_present"]);
        let detail_absent = as_string_list(&tc["expected_detail_keys_absent"]);
        if !detail_present.is_empty() || !detail_absent.is_empty() {
            let details = obj
                .get("details")
                .and_then(Value::as_object)
                .unwrap_or_else(|| {
                    panic!("case {id}: expected a `details` object in {serialized}")
                });
            for key in detail_present {
                assert!(
                    details.contains_key(&key),
                    "case {id}: expected detail key {key:?} to be present in {serialized}"
                );
            }
            for key in detail_absent {
                assert!(
                    !details.contains_key(&key),
                    "case {id}: expected detail key {key:?} to be ABSENT in {serialized}"
                );
            }
        }
    }
}
