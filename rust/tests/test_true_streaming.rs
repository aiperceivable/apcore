//! End-to-end tests for true incremental streaming on `Executor::stream`.
//!
//! These tests assert that:
//!   1. Chunks arrive at the caller *before* the inner stream finishes
//!      (i.e. no Vec buffering — true streaming).
//!   2. Chunks arrive in order and none are dropped.
//!   3. Modules that do not implement `stream()` return an error chunk.
//!   4. Schema validation failures in Phase 3 are surfaced as the final
//!      `Err` item of the output stream.

use std::time::{Duration, Instant};

use apcore::context::Context;
use apcore::module::{Module, ModuleAnnotations};
use apcore::{APCore, ChunkStream, ModuleError};
use async_stream::stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

// ---------------------------------------------------------------------------
// Streaming test modules
// ---------------------------------------------------------------------------

/// Yields 5 chunks, sleeping 100ms between each. Total runtime ~500ms.
/// Used to prove that the first chunk arrives well before the stream ends.
#[derive(Default)]
struct SlowStreamingModule {
    idempotent: bool,
}

#[async_trait]
impl Module for SlowStreamingModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "slow streaming test module"
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            idempotent: self.idempotent,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            for i in 0..5u32 {
                sleep(Duration::from_millis(100)).await;
                yield Ok(json!({ "chunk": i }));
            }
        }))
    }
}

/// Non-streaming module: only implements `execute()`. Used to check that
/// `stream()` on such a module yields the "not supported" error.
struct NonStreamingModule;

#[async_trait]
impl Module for NonStreamingModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "non-streaming module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"ok": true}))
    }
}

/// Yields chunks whose merged output violates the output schema. Used to
/// verify that Phase 3 validation errors appear as the final `Err` item.
struct BadSchemaStreamingModule;

#[async_trait]
impl Module for BadSchemaStreamingModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        // Require a "result" field of type integer — but the merged chunks
        // below won't contain it, so validation MUST fail in Phase 3.
        json!({
            "type": "object",
            "properties": {
                "result": { "type": "integer" }
            },
            "required": ["result"]
        })
    }
    fn description(&self) -> &'static str {
        "streaming module with bad schema"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(json!({"partial": "a"}));
            yield Ok(json!({"partial": "b"}));
        }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn first_chunk_arrives_before_stream_completes() {
    let apcore = APCore::new();
    apcore
        .register("slow.stream", Box::<SlowStreamingModule>::default())
        .unwrap();

    let start = Instant::now();
    let mut s = apcore
        .executor()
        .stream("slow.stream", json!({}), None, None);

    // The inner module takes ~500ms total, but the first chunk should arrive
    // within roughly 100ms (plus scheduling slack). We allow up to 300ms — far
    // less than the full 500ms, which proves chunks are delivered incrementally.
    let first = timeout(Duration::from_millis(300), s.next())
        .await
        .expect("first chunk should arrive before Vec-collected stream could finish");
    let first_elapsed = start.elapsed();
    let chunk = first
        .expect("stream yielded None before first chunk")
        .expect("first chunk should be Ok");
    assert_eq!(chunk["chunk"], 0);
    assert!(
        first_elapsed < Duration::from_millis(300),
        "first chunk arrived too late: {first_elapsed:?}"
    );
}

#[tokio::test]
async fn all_chunks_arrive_in_order() {
    let apcore = APCore::new();
    apcore
        .register("slow.stream", Box::<SlowStreamingModule>::default())
        .unwrap();

    let mut s = apcore
        .executor()
        .stream("slow.stream", json!({}), None, None);
    let mut chunks = Vec::new();
    while let Some(item) = s.next().await {
        chunks.push(item.expect("no chunks should be errors"));
    }
    assert_eq!(chunks.len(), 5);
    for (i, c) in chunks.iter().enumerate() {
        assert_eq!(c["chunk"], i as u64);
    }
}

/// Sync STREAM-002: a module that does NOT override `stream()` must fall back
/// to `execute()` and yield its result as a single chunk. Mirrors apcore-python
/// (executor.py:862-865) and apcore-typescript (executor.ts:519-522).
#[tokio::test]
async fn streaming_falls_back_to_execute_when_module_does_not_support_streaming() {
    let apcore = APCore::new();
    apcore
        .register("plain.mod", Box::new(NonStreamingModule))
        .unwrap();

    let mut s = apcore.executor().stream("plain.mod", json!({}), None, None);
    let first = s
        .next()
        .await
        .expect("stream should yield exactly one chunk equal to execute()'s output");
    let chunk = first.expect("fallback path must succeed");
    assert_eq!(chunk, json!({"ok": true}));
    // No further items should be produced after the single execute() chunk.
    assert!(s.next().await.is_none());
}

/// Sync STREAM-003: streaming must enforce `Context::global_deadline` between
/// chunks. A slow stream that exceeds the deadline yields a `ModuleTimeout`
/// error and stops yielding further chunks.
#[tokio::test]
async fn streaming_global_deadline_aborts_between_chunks() {
    assert_streaming_deadline(false).await;
    assert_streaming_deadline(true).await;
}

async fn assert_streaming_deadline(idempotent: bool) {
    use apcore::context::Identity;

    let apcore = APCore::new();
    apcore
        .register("slow.stream", Box::new(SlowStreamingModule { idempotent }))
        .unwrap();

    // Set a global_deadline 150ms from now. SlowStreamingModule yields one
    // chunk every 100ms, so after the second chunk the deadline is past.
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let mut ctx = Context::<Value>::new(Identity::new(
        "@external".to_string(),
        "external".to_string(),
        vec![],
        std::collections::HashMap::new(),
    ));
    // Set caller_id so BuiltinContextCreation does not replace this context
    // (it clobbers user-supplied contexts whose caller_id is None).
    ctx.caller_id = Some("@external".to_string());
    ctx.global_deadline = Some(now_secs + 0.15);

    let mut s = apcore
        .executor()
        .stream("slow.stream", json!({}), Some(&ctx), None);

    let mut ok_count = 0usize;
    let mut got_timeout = false;
    while let Some(item) = s.next().await {
        match item {
            Ok(_) => ok_count += 1,
            Err(e) => {
                assert_eq!(
                    e.code,
                    apcore::errors::ErrorCode::ModuleTimeout,
                    "deadline-exceeded error must be ModuleTimeout"
                );
                assert_eq!(e.retryable, Some(idempotent));
                got_timeout = true;
                break;
            }
        }
    }
    assert!(
        got_timeout,
        "stream must surface a ModuleTimeout once the global deadline elapses"
    );
    assert!(
        (1..5).contains(&ok_count),
        "stream must yield at least one chunk before aborting (saw {ok_count})"
    );
}

#[tokio::test]
async fn phase3_validation_failure_is_swallowed_chunks_still_delivered() {
    // Per spec (sync finding A-D-012): chunks are already delivered when
    // Phase-3 validation runs, so failures MUST NOT bubble out as a final
    // stream Err item. They are logged via tracing::warn and the stream
    // ends cleanly. Matches apcore-python's `apcore.stream.post_validation_failed`
    // event-emit-and-swallow and apcore-typescript's console.warn-and-swallow.
    let apcore = APCore::new();
    apcore
        .register("bad.stream", Box::new(BadSchemaStreamingModule))
        .unwrap();

    let mut s = apcore
        .executor()
        .stream("bad.stream", json!({}), None, None);
    let mut ok_count = 0usize;
    let mut err_count = 0usize;
    while let Some(item) = s.next().await {
        match item {
            Ok(_) => ok_count += 1,
            Err(_) => err_count += 1,
        }
    }
    assert_eq!(ok_count, 2, "both data chunks should reach the caller");
    assert_eq!(
        err_count, 0,
        "Phase-3 validation failure must be swallowed (logged) — chunks already delivered"
    );
}

#[tokio::test]
async fn phase3_validation_failure_emits_post_validation_event() {
    // apcore#78: the swallowed Phase-3 failure must still surface an
    // observability event on the bus — `apcore.stream.post_validation_failed`
    // — matching apcore-python / apcore-typescript. Previously Rust only
    // tracing::warn-logged it (comment-only stub).
    use std::sync::Arc;

    use apcore::config::Config;
    use parking_lot::Mutex;

    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let mut apcore = APCore::with_options(None, None, Some(config), None);
    apcore
        .register("bad.stream", Box::new(BadSchemaStreamingModule))
        .unwrap();

    let captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    apcore
        .on("apcore.stream.post_validation_failed", move |event| {
            sink.lock().push(event.data.clone());
        })
        .expect("events enabled");

    let mut s = apcore.stream("bad.stream", json!({}), None, None);
    let mut chunks = 0;
    while let Some(chunk) = s.next().await {
        chunk.expect("the chunk should still reach the caller");
        chunks += 1;
    }
    assert_eq!(chunks, 2);

    apcore.events().unwrap().flush_default().await.unwrap();

    let events = captured.lock();
    assert_eq!(
        events.len(),
        1,
        "exactly one apcore.stream.post_validation_failed event expected, got {events:?}"
    );
    assert!(
        events[0].get("message").is_some(),
        "event payload must carry a message field: {:?}",
        events[0]
    );
    assert!(
        events[0].get("error_type").is_some(),
        "event payload must carry an error_type field: {:?}",
        events[0]
    );
}

/// A-D-002: the streaming path must honor the two-point cancellation invariant.
/// A token cancelled before the stream begins must abort BEFORE any chunk is
/// yielded — mirroring the unary Step-8 check in `BuiltinExecute`
/// (apcore-python builtin_steps.py:607, apcore-typescript builtin-steps.ts:502).
#[tokio::test]
async fn streaming_pre_cancelled_token_aborts_before_first_chunk() {
    use apcore::cancel::CancelToken;
    use apcore::context::Identity;

    let apcore = APCore::new();
    apcore
        .register("slow.stream", Box::<SlowStreamingModule>::default())
        .unwrap();

    let token = CancelToken::new();
    token.cancel(); // pre-cancelled before streaming starts

    let mut ctx = Context::<Value>::new(Identity::new(
        "@external".to_string(),
        "external".to_string(),
        vec![],
        std::collections::HashMap::new(),
    ));
    ctx.caller_id = Some("@external".to_string());
    ctx.cancel_token = Some(token);

    let mut s = apcore
        .executor()
        .stream("slow.stream", json!({}), Some(&ctx), None);

    let first = s.next().await.expect("stream must yield at least one item");
    match first {
        Ok(chunk) => panic!("pre-cancelled stream must NOT yield a data chunk, got {chunk:?}"),
        Err(e) => assert_eq!(
            e.code,
            apcore::errors::ErrorCode::ExecutionCancelled,
            "pre-cancelled stream must surface ExecutionCancelled before any chunk"
        ),
    }
}
