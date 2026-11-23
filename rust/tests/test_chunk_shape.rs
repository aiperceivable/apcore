// D-58 alignment: streaming chunks that are not JSON objects MUST cause the
// merge-on-accumulate path to raise InvalidInput with code STREAM_CHUNK_NOT_OBJECT.
// The previous Rust impl silently replaced or merged non-object chunks, which
// diverged from Python (raises AttributeError on .items()) and TS (TypeError).
// We canonicalize the error so cross-language consumers can match on it.

use apcore::executor::deep_merge_chunks_checked;
use apcore::ErrorCode;
use serde_json::json;

#[test]
fn merge_rejects_non_object_chunk() {
    let chunks = vec![json!({"a": 1}), json!("not an object")];
    let err = deep_merge_chunks_checked(&chunks).expect_err("must reject non-object chunk");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    let code_str = serde_json::to_value(err.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    assert_eq!(code_str, "GENERAL_INVALID_INPUT");
    // The canonical reason must be embedded so cross-language consumers can match.
    let detail_code = err
        .details
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert_eq!(
        detail_code, "STREAM_CHUNK_NOT_OBJECT",
        "details.code must be STREAM_CHUNK_NOT_OBJECT (got {detail_code})"
    );
}

#[test]
fn merge_rejects_array_chunk() {
    let chunks = vec![json!({"a": 1}), json!([1, 2, 3])];
    let err = deep_merge_chunks_checked(&chunks).expect_err("must reject array chunk");
    let detail_code = err
        .details
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert_eq!(detail_code, "STREAM_CHUNK_NOT_OBJECT");
}

#[test]
fn merge_accepts_object_chunks() {
    let chunks = vec![json!({"a": 1}), json!({"b": 2})];
    let merged = deep_merge_chunks_checked(&chunks).expect("object chunks ok");
    assert_eq!(merged, json!({"a": 1, "b": 2}));
}

#[test]
fn merge_accepts_empty_chunks() {
    let merged = deep_merge_chunks_checked(&[]).expect("empty ok");
    assert_eq!(merged, json!({}));
}
