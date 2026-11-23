//! Spec-traced contract tests for the apcore Error System (Rust SDK).
//!
//! Source spec: apcore/docs/features/error-system.md
//! Contract under test: `ModuleError::to_dict`
//!
//! This file MIRRORS the canonical Python suite
//! (`apcore-python/tests/test_error_system_spec.py`). Each test carries the
//! verbatim clause id (`error_system.<method>.<kind>.<detail>`) in a leading
//! `// clause:` comment so cross-language diffs line up row-for-row.
//!
//! These tests are READ-ONLY contract verification — they never modify
//! production source.
//!
//! Cross-language note: the Rust `ModuleError.code` is a typed `ErrorCode`
//! enum (not a free-form string as in Python). `to_dict()` returns a sparse
//! `serde_json::Value` object — `None`/empty fields are omitted. The clause
//! intents are mirrored by asserting the equivalent Rust behavior.

use std::collections::HashMap;

use apcore::errors::{ErrorCode, ModuleError};

// ---------------------------------------------------------------------------
// Returns contract: guaranteed keys
// ---------------------------------------------------------------------------

// clause: error_system.to_dict.returns.code_key
#[test]
fn error_system_to_dict_returns_code_key() {
    // The serialized object MUST always carry a `code` (string) key.
    let err = ModuleError::new(ErrorCode::ModuleNotFound, "something failed");
    let result = err.to_dict();
    let obj = result.as_object().expect("to_dict returns a JSON object");
    assert!(obj.contains_key("code"), "code key must be present");
    // Rust emits the SCREAMING_SNAKE wire string for the ErrorCode variant.
    assert_eq!(result["code"], serde_json::json!("MODULE_NOT_FOUND"));
    assert!(
        result["code"].is_string(),
        "code must serialize as a string"
    );
}

// clause: error_system.to_dict.returns.message_key
#[test]
fn error_system_to_dict_returns_message_key() {
    // The serialized object MUST always carry a `message` (string) key.
    let err = ModuleError::new(ErrorCode::ModuleNotFound, "something failed");
    let result = err.to_dict();
    let obj = result.as_object().expect("to_dict returns a JSON object");
    assert!(obj.contains_key("message"), "message key must be present");
    assert_eq!(result["message"], serde_json::json!("something failed"));
    assert!(result["message"].is_string(), "message must be a string");
}

// clause: error_system.to_dict.returns.ai_guidance_key
#[test]
fn error_system_to_dict_returns_ai_guidance_key() {
    // When the error carries `ai_guidance`, it MUST appear (as a string) in
    // the serialized object. Python's ModuleNotFoundError sets a non-empty
    // default; the Rust idiomatic equivalent is the `invalid_input` builder,
    // which supplies a non-empty default `ai_guidance`.
    let err = ModuleError::invalid_input("missing required field");
    let result = err.to_dict();
    let obj = result.as_object().expect("to_dict returns a JSON object");
    assert!(
        obj.contains_key("ai_guidance"),
        "ai_guidance key must be present when set"
    );
    let guidance = result["ai_guidance"]
        .as_str()
        .expect("ai_guidance must be a string");
    assert!(!guidance.is_empty(), "ai_guidance must be non-empty");
}

// clause: error_system.to_dict.returns.timestamp_key
#[test]
fn error_system_to_dict_returns_timestamp_key() {
    // `timestamp` is an optional-but-emitted key (ISO 8601 UTC string).
    let err = ModuleError::new(ErrorCode::ModuleNotFound, "boom");
    let result = err.to_dict();
    let obj = result.as_object().expect("to_dict returns a JSON object");
    assert!(
        obj.contains_key("timestamp"),
        "timestamp key must be present"
    );
    let ts = result["timestamp"]
        .as_str()
        .expect("timestamp must be a string");
    // ISO 8601 marker present.
    assert!(
        ts.contains('T'),
        "timestamp must be ISO 8601 (contains 'T')"
    );
}

// clause: error_system.to_dict.returns.details_key
#[test]
fn error_system_to_dict_returns_details_key() {
    // The optional `details` key is included only when populated, and round
    // trips the supplied mapping.
    let mut details = HashMap::new();
    details.insert("field".to_string(), serde_json::json!("email"));
    let err = ModuleError::new(ErrorCode::ModuleNotFound, "boom").with_details(details);
    let result = err.to_dict();
    assert_eq!(result["details"], serde_json::json!({"field": "email"}));
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

// clause: error_system.to_dict.property.async
#[tokio::test(flavor = "multi_thread")]
async fn error_system_to_dict_property_async() {
    // to_dict is declared async: false — it MUST be a plain synchronous call
    // returning a concrete value (not a future). In Rust this is enforced by
    // the type system: `to_dict(&self) -> serde_json::Value` is not async.
    // We call it inside an async context and assert it produces a concrete
    // object without `.await`.
    let err = ModuleError::new(ErrorCode::ModuleNotFound, "boom");
    let result = err.to_dict();
    assert!(
        result.is_object(),
        "to_dict must return a concrete JSON object synchronously"
    );
}

// clause: error_system.to_dict.property.pure
#[test]
fn error_system_to_dict_property_pure() {
    // to_dict is declared pure — calling it MUST NOT mutate any observable
    // state on the error instance.
    let mut details = HashMap::new();
    details.insert("field".to_string(), serde_json::json!("email"));
    let err = ModuleError::new(ErrorCode::ModuleNotFound, "boom")
        .with_details(details)
        .with_retryable(false)
        .with_ai_guidance("fix the input")
        .with_user_fixable(true)
        .with_suggestion("correct the field");

    let before = (
        err.code,
        err.message.clone(),
        err.details.clone(),
        err.cause.clone(),
        err.trace_id.clone(),
        err.timestamp,
        err.retryable,
        err.ai_guidance.clone(),
        err.user_fixable,
        err.suggestion.clone(),
    );
    let _ = err.to_dict();
    let after = (
        err.code,
        err.message.clone(),
        err.details.clone(),
        err.cause.clone(),
        err.trace_id.clone(),
        err.timestamp,
        err.retryable,
        err.ai_guidance.clone(),
        err.user_fixable,
        err.suggestion.clone(),
    );
    assert_eq!(before, after, "to_dict must not mutate the error instance");
}

// clause: error_system.to_dict.property.pure.fresh_top_level
#[test]
fn error_system_to_dict_property_pure_fresh_top_level() {
    // Purity requires that mutating the TOP-LEVEL of the returned value does
    // not feed back into the error instance, and that a fresh serialization
    // still reflects the original (unmutated) state. Rust returns an owned
    // `serde_json::Value` by value, so the returned object is always fresh.
    let mut details = HashMap::new();
    details.insert("field".to_string(), serde_json::json!("email"));
    let err = ModuleError::new(ErrorCode::ModuleNotFound, "boom").with_details(details);

    let mut result = err.to_dict();
    result["code"] = serde_json::json!("TAMPERED");
    result["message"] = serde_json::json!("tampered");

    // Top-level mutation of the returned value does not touch the instance.
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
    assert_eq!(err.message, "boom");

    // A fresh serialization still reflects the original state.
    let fresh = err.to_dict();
    assert_eq!(fresh["code"], serde_json::json!("MODULE_NOT_FOUND"));
    assert_eq!(fresh["message"], serde_json::json!("boom"));
}

// clause: error_system.to_dict.property.idempotent
#[test]
fn error_system_to_dict_property_idempotent() {
    // Two successive calls with identical (unchanged) state MUST produce equal
    // output and leave observable state identical.
    let mut details = HashMap::new();
    details.insert(
        "errors".to_string(),
        serde_json::json!([{"path": "email", "msg": "invalid"}]),
    );
    let err = ModuleError::new(ErrorCode::SchemaValidationError, "validation failed")
        .with_details(details);

    let first = err.to_dict();
    let state_after_first = (
        err.code,
        err.message.clone(),
        err.details.clone(),
        err.timestamp,
    );
    let second = err.to_dict();
    let state_after_second = (
        err.code,
        err.message.clone(),
        err.details.clone(),
        err.timestamp,
    );
    assert_eq!(first, second, "two calls must produce equal output");
    assert_eq!(
        state_after_first, state_after_second,
        "observable state must be identical between calls"
    );
}

// clause: error_system.to_dict.property.thread_safe
#[tokio::test(flavor = "multi_thread")]
async fn error_system_to_dict_property_thread_safe() {
    // Declared thread_safe: true. Launch N (>=8) concurrent serializations of
    // distinct error instances via tokio::spawn; assert no task panics and
    // every result is consistent with its source error.
    let mut handles = Vec::new();
    for i in 0..12u32 {
        handles.push(tokio::spawn(async move {
            let mut details = HashMap::new();
            details.insert("i".to_string(), serde_json::json!(format!("CODE_{i}")));
            let err = ModuleError::new(ErrorCode::ModuleNotFound, format!("message {i}"))
                .with_details(details);
            // Yield so the tasks genuinely interleave.
            tokio::task::yield_now().await;
            (i, err.to_dict())
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        // `await` resolving without an error confirms no task panicked.
        let (i, value) = handle.await.expect("spawned serialization must not panic");
        results.push((i, value));
    }

    assert_eq!(results.len(), 12, "all 12 serializations must complete");
    for (i, value) in results {
        assert_eq!(value["code"], serde_json::json!("MODULE_NOT_FOUND"));
        assert_eq!(value["message"], serde_json::json!(format!("message {i}")));
        assert_eq!(
            value["details"],
            serde_json::json!({"i": format!("CODE_{i}")})
        );
    }
}
