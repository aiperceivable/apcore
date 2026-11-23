//! Audit D10-001: the streaming pipeline MUST reject a non-object chunk
//! *before* delivering it to the consumer.
//!
//! Canonical contract (user decision): "reject before deliver + raise". The
//! previous Rust behavior delivered every chunk as-is in Phase 2 and only
//! checked shape in Phase 3 via `deep_merge_chunks_checked`, where the error
//! was swallowed with `tracing::warn`. Now the check moves into the Phase 2
//! loop: a chunk is valid iff it is a JSON object; the first invalid chunk
//! surfaces a `GeneralInvalidInput` error with
//! `details["code"] == "STREAM_CHUNK_NOT_OBJECT"` and is NOT yielded.

use apcore::context::Context;
use apcore::module::Module;
use apcore::{APCore, ChunkStream, ErrorCode, ModuleError};
use async_stream::stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

/// Yields one valid object chunk, then a non-object (string) chunk. The string
/// chunk MUST be rejected before delivery.
struct ObjectThenStringModule;

#[async_trait]
impl Module for ObjectThenStringModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "yields an object chunk then a non-object string chunk"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(json!({"a": 1}));
            yield Ok(json!("not an object"));
            // A third chunk that must never be reached, proving the stream
            // short-circuits on the first invalid chunk.
            yield Ok(json!({"c": 3}));
        }))
    }
}

/// All-object stream: every chunk is a JSON object and must be delivered.
struct AllObjectModule;

#[async_trait]
impl Module for AllObjectModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "yields three object chunks"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(json!({"a": 1}));
            yield Ok(json!({"b": 2}));
            yield Ok(json!({"c": 3}));
        }))
    }
}

#[tokio::test]
async fn non_object_chunk_is_rejected_before_delivery() {
    let apcore = APCore::new();
    apcore
        .register("reject.stream", Box::new(ObjectThenStringModule))
        .unwrap();

    let mut s = apcore
        .executor()
        .stream("reject.stream", json!({}), None, None);

    let mut delivered: Vec<Value> = Vec::new();
    let mut surfaced_err: Option<ModuleError> = None;
    while let Some(item) = s.next().await {
        match item {
            Ok(v) => delivered.push(v),
            Err(e) => {
                surfaced_err = Some(e);
                break;
            }
        }
    }

    // (a) The non-object chunk must NOT be delivered. Only the first valid
    // object chunk should have reached the consumer.
    assert_eq!(
        delivered,
        vec![json!({"a": 1})],
        "only the leading valid object chunk should be delivered; the non-object \
         chunk (and anything after) must be rejected before delivery"
    );
    for v in &delivered {
        assert!(
            v.is_object(),
            "a non-object chunk leaked to the consumer: {v:?}"
        );
    }

    // (b) The stream must surface a structured error for the bad chunk.
    let err = surfaced_err.expect("stream must surface an error for the non-object chunk");
    assert_eq!(
        err.code,
        ErrorCode::GeneralInvalidInput,
        "rejected-chunk error code must be GeneralInvalidInput"
    );
    let detail_code = err
        .details
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert_eq!(
        detail_code, "STREAM_CHUNK_NOT_OBJECT",
        "details.code must be STREAM_CHUNK_NOT_OBJECT (got {detail_code})"
    );
    // The bad chunk is at index 1 (0-based) and is a string.
    assert_eq!(
        err.details.get("chunk_index").and_then(Value::as_u64),
        Some(1),
        "chunk_index must be the 0-based index of the rejected chunk"
    );
    assert_eq!(
        err.details.get("actual_type").and_then(Value::as_str),
        Some("string"),
        "actual_type must name the JSON type of the rejected chunk"
    );
}

#[tokio::test]
async fn all_object_stream_delivers_every_chunk() {
    let apcore = APCore::new();
    apcore
        .register("ok.stream", Box::new(AllObjectModule))
        .unwrap();

    let mut s = apcore.executor().stream("ok.stream", json!({}), None, None);
    let mut delivered: Vec<Value> = Vec::new();
    while let Some(item) = s.next().await {
        delivered.push(item.expect("all-object stream must not surface any error"));
    }
    assert_eq!(
        delivered,
        vec![json!({"a": 1}), json!({"b": 2}), json!({"c": 3})],
        "an all-object stream must deliver every chunk unchanged and in order"
    );
}
