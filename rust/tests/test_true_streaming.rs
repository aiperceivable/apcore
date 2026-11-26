//! End-to-end tests for true incremental streaming on `Executor::stream`.
//!
//! These tests assert that:
//!   1. Chunks arrive at the caller *before* the inner stream finishes
//!      (i.e. no Vec buffering — true streaming).
//!   2. Chunks arrive in order and none are dropped.
//!   3. Modules that do not implement `stream()` return an error chunk.
//!   4. Schema validation failures in Phase 3 are surfaced as the final
//!      `Err` item of the output stream.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apcore::context::Context;
use apcore::errors::ErrorCode;
use apcore::module::{Module, ModuleAnnotations};
use apcore::{
    build_strategy_from_config, APCore, ChunkStream, Config, Executor, Middleware, ModuleError,
    PipelineContext, PipelineState, Registry, Step, StepMiddleware, StepResult, StreamingModule,
};
use async_stream::stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

mod conformance_env;

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
struct NonStreamingModule(Value);

const OVERWRITING_CHUNKS: u64 = 96;

struct OverwritingStream;

#[async_trait]
impl Module for OverwritingStream {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Stream successive versions of a fixed-size result"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        let invalid_tail = inputs["invalid_tail"] == true;
        Some(Box::pin(stream! {
            let payload = "x".repeat(384 * 1024);
            for revision in 0..OVERWRITING_CHUNKS {
                yield Ok(json!({"payload": payload, "revision": revision}));
            }
            if invalid_tail {
                yield Ok(Value::Null);
            }
        }))
    }
}

#[tokio::test]
async fn streaming_overwrites_repeated_values_and_keeps_the_chunk_index() {
    let client = APCore::new();
    let module_id = "test.overwriting_stream";
    client
        .register(module_id, Box::new(OverwritingStream))
        .unwrap();

    let (mut chunks, completion) = client.stream_with_completion(module_id, None, None, None);
    for revision in 0..OVERWRITING_CHUNKS {
        assert_eq!(chunks.next().await.unwrap().unwrap()["revision"], revision);
    }
    assert!(chunks.next().await.is_none());
    let output = completion.await.output.unwrap();
    assert_eq!(output["revision"], OVERWRITING_CHUNKS - 1);
    assert_eq!(output["payload"].as_str().unwrap().len(), 384 * 1024);

    let mut chunks = client.stream(module_id, Some(json!({"invalid_tail": true})), None, None);
    for _ in 0..OVERWRITING_CHUNKS {
        assert!(chunks.next().await.unwrap().is_ok());
    }
    let error = chunks.next().await.unwrap().unwrap_err();
    assert_eq!(error.details["code"], "STREAM_CHUNK_NOT_OBJECT");
    assert_eq!(error.details["chunk_index"], OVERWRITING_CHUNKS);
    assert!(chunks.next().await.is_none());
}

struct GatedStreamingModule {
    next_chunk: std::sync::Arc<tokio::sync::Notify>,
    opens: Arc<AtomicUsize>,
}

#[async_trait]
impl Module for GatedStreamingModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "streaming module with an externally scheduled next chunk"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let next_chunk = self.next_chunk.clone();
        Some(Box::pin(stream! {
            yield Ok(json!({"chunk": 1}));
            next_chunk.notified().await;
            yield Ok(json!({"chunk": 2}));
        }))
    }
}

struct ReplacementExecute(Arc<AtomicUsize>);

#[async_trait]
impl Step for ReplacementExecute {
    fn name(&self) -> &'static str {
        "execute"
    }

    fn description(&self) -> &'static str {
        "Return a replacement result"
    }

    fn removable(&self) -> bool {
        false
    }

    fn replaceable(&self) -> bool {
        true
    }

    fn provides(&self) -> &[&str] {
        &["output"]
    }

    async fn execute(&self, context: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        context.output = Some(json!({"replacement": true}));
        Ok(StepResult::continue_step())
    }
}

#[tokio::test]
async fn test_stream_uses_the_configured_execute_step() {
    let replacements = Arc::new(AtomicUsize::new(0));
    let opens = Arc::new(AtomicUsize::new(0));
    let mut strategy = build_strategy_from_config(&json!({})).unwrap();
    strategy
        .replace(
            "execute",
            Box::new(ReplacementExecute(replacements.clone())),
        )
        .unwrap();
    let executor = Executor::with_strategy(Registry::new(), Config::default(), strategy);
    let client = APCore::with_options(None, Some(executor), None, None);
    client
        .register(
            "test.strategy_stream",
            Box::new(GatedStreamingModule {
                next_chunk: Default::default(),
                opens: opens.clone(),
            }),
        )
        .unwrap();

    let mut chunks = client.stream("test.strategy_stream", None, None, None);
    assert_eq!(
        chunks.next().await.unwrap().unwrap(),
        json!({"replacement": true})
    );
    assert!(chunks.next().await.is_none());
    let (mut chunks, completion) =
        client.stream_with_completion("test.strategy_stream", None, None, None);
    assert_eq!(
        chunks.next().await.unwrap().unwrap(),
        json!({"replacement": true})
    );
    assert!(chunks.next().await.is_none());
    assert_eq!(completion.await.output, Some(json!({"replacement": true})));
    assert_eq!(opens.load(Ordering::SeqCst), 0);
    assert_eq!(replacements.load(Ordering::SeqCst), 2);
}

struct TypedOnlyStream(Arc<AtomicUsize>);

#[async_trait]
impl Module for TypedOnlyStream {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Module with only a typed stream implementation"
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            streaming: true,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"unary": true}))
    }
    fn as_streaming(&self) -> Option<&dyn StreamingModule> {
        Some(self)
    }
}

impl StreamingModule for TypedOnlyStream {
    fn stream_typed(&self, _inputs: Value, _ctx: &Context<Value>) -> ChunkStream {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(futures_util::stream::iter([Ok(json!({"typed": true}))]))
    }
}

#[tokio::test]
async fn test_stream_uses_the_typed_hook_without_a_legacy_override() {
    let opens = Arc::new(AtomicUsize::new(0));
    let client = APCore::new();
    client
        .register(
            "test.typed_stream",
            Box::new(TypedOnlyStream(opens.clone())),
        )
        .unwrap();
    let (mut chunks, completion) =
        client.stream_with_completion("test.typed_stream", None, None, None);
    assert_eq!(
        chunks.next().await.unwrap().unwrap(),
        json!({"typed": true})
    );
    assert!(chunks.next().await.is_none());
    assert_eq!(completion.await.output, Some(json!({"typed": true})));
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_unmatched_execute_step_does_not_open_a_stream() {
    let opens = Arc::new(AtomicUsize::new(0));
    let strategy = build_strategy_from_config(&json!({
        "configure": {"execute": {"match_modules": ["other.*"]}}
    }))
    .unwrap();
    let executor = Executor::with_strategy(Registry::new(), Config::default(), strategy);
    let client = APCore::with_options(None, Some(executor), None, None);
    client
        .register(
            "test.unmatched_stream",
            Box::new(TypedOnlyStream(opens.clone())),
        )
        .unwrap();

    let (mut chunks, completion) =
        client.stream_with_completion("test.unmatched_stream", None, None, None);
    let unary = client
        .executor()
        .call("test.unmatched_stream", json!({}), None, None)
        .await
        .unwrap();
    assert_eq!(chunks.next().await.unwrap().unwrap(), unary);
    assert!(chunks.next().await.is_none());
    assert_eq!(completion.await.output, None);
    assert_eq!(opens.load(Ordering::SeqCst), 0);
}

struct RecordStreamSteps(Arc<Mutex<Vec<&'static str>>>);

#[async_trait]
impl StepMiddleware for RecordStreamSteps {
    async fn before_step(&self, name: &str, state: &PipelineState<'_>) -> Result<(), ModuleError> {
        match name {
            "execute" => {
                assert!(state.outputs.contains_key("input_validation"));
                self.0.lock().unwrap().push("before execute");
            }
            "output_validation" => {
                assert!(state.outputs.contains_key("execute"));
                self.0.lock().unwrap().push("before output validation");
            }
            _ => {}
        }
        Ok(())
    }

    async fn after_step(
        &self,
        name: &str,
        _state: &PipelineState<'_>,
        _result: &Value,
    ) -> Result<(), ModuleError> {
        if name == "execute" {
            self.0.lock().unwrap().push("after execute");
        }
        Ok(())
    }
}

#[tokio::test]
async fn test_stream_runs_execute_hooks_once_and_preserves_prior_step_outputs() {
    let steps = Arc::new(Mutex::new(Vec::new()));
    let opens = Arc::new(AtomicUsize::new(0));
    let mut strategy = build_strategy_from_config(&json!({})).unwrap();
    strategy.add_step_middleware(Arc::new(RecordStreamSteps(steps.clone())));
    let executor = Executor::with_strategy(Registry::new(), Config::default(), strategy);
    let client = APCore::with_options(None, Some(executor), None, None);
    client
        .register(
            "test.hooked_stream",
            Box::new(TypedOnlyStream(opens.clone())),
        )
        .unwrap();

    let mut chunks = client.stream("test.hooked_stream", None, None, None);
    assert_eq!(
        chunks.next().await.unwrap().unwrap(),
        json!({"typed": true})
    );
    assert_eq!(*steps.lock().unwrap(), ["before execute", "after execute"]);
    assert!(chunks.next().await.is_none());
    assert_eq!(
        *steps.lock().unwrap(),
        [
            "before execute",
            "after execute",
            "before output validation"
        ]
    );
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_stream_deadline_while_awaiting_next_chunk() {
    let apcore = APCore::new();
    apcore
        .register(
            "gated.stream",
            Box::new(GatedStreamingModule {
                next_chunk: Default::default(),
                opens: Default::default(),
            }),
        )
        .unwrap();
    let mut ctx = Context::<Value>::anonymous();
    ctx.global_deadline = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            + 0.100,
    );
    let mut chunks = apcore
        .executor()
        .stream("gated.stream", json!({}), Some(&ctx), None);
    assert_eq!(chunks.next().await.unwrap().unwrap(), json!({"chunk": 1}));
    let error = timeout(Duration::from_millis(500), chunks.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, apcore::ErrorCode::ModuleTimeout);
    assert!(chunks.next().await.is_none());
}

#[tokio::test]
async fn test_expired_deadline_leaves_stream_producer_unopened() {
    let client = APCore::new();
    let opens = Arc::new(AtomicUsize::new(0));
    client
        .register(
            "test.expired_stream",
            Box::new(GatedStreamingModule {
                next_chunk: Default::default(),
                opens: opens.clone(),
            }),
        )
        .unwrap();
    let mut context = Context::anonymous();
    context.global_deadline = Some(0.0);
    let mut chunks =
        client
            .executor()
            .stream("test.expired_stream", json!({}), Some(&context), None);
    assert_eq!(
        chunks.next().await.unwrap().unwrap_err().code,
        ErrorCode::ModuleTimeout
    );
    assert!(chunks.next().await.is_none());
    assert_eq!(opens.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_stream_wait_observes_cancellation_from_its_parent() {
    let apcore = APCore::new();
    apcore
        .register(
            "gated.stream",
            Box::new(GatedStreamingModule {
                next_chunk: Default::default(),
                opens: Default::default(),
            }),
        )
        .unwrap();
    let mut ctx = Context::<Value>::anonymous();
    let token = apcore::CancelToken::new();
    ctx.cancel_token = Some(token.clone());
    let mut chunks = apcore
        .executor()
        .stream("gated.stream", json!({}), Some(&ctx), None);
    assert_eq!(chunks.next().await.unwrap().unwrap(), json!({"chunk": 1}));
    tokio::spawn(async move {
        sleep(Duration::from_millis(20)).await;
        token.cancel();
    });
    let error = timeout(Duration::from_millis(500), chunks.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, apcore::ErrorCode::ExecutionCancelled);
    assert!(chunks.next().await.is_none());
}

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
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn test_null_output_fails_unary_call_and_stream_fallback() {
    let client = APCore::new();
    let module_id = "test.null_output";
    client
        .register(module_id, Box::new(NonStreamingModule(Value::Null)))
        .unwrap();
    let executor = client.executor();
    let unary = executor
        .call(module_id, json!({}), None, None)
        .await
        .unwrap_err();
    let mut chunks = executor.stream(module_id, json!({}), None, None);
    let streaming = chunks.next().await.unwrap().unwrap_err();
    assert!(chunks.next().await.is_none());
    for error in [unary, streaming] {
        assert_eq!(error.code, ErrorCode::ModuleExecuteError);
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
        .register(
            "plain.mod",
            Box::new(NonStreamingModule(json!({"ok": true}))),
        )
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

struct FixtureStream(Vec<Value>);

#[async_trait]
impl Module for FixtureStream {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "stream post-processing test module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(futures_util::stream::iter(
            self.0.clone().into_iter().map(Ok),
        )))
    }
}

#[derive(Debug)]
struct FailAfter {
    code: ErrorCode,
    on_error_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl Middleware for FailAfter {
    fn name(&self) -> &'static str {
        "FailAfter"
    }
    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        _output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Err(ModuleError::new(self.code.clone(), "after failed"))
    }
    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.on_error_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(None)
    }
}

#[tokio::test]
async fn test_stream_post_processing_failure_contract() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let path = conformance_env::find_fixtures_root().join("stream_post_processing.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for case in fixture["test_cases"].as_array().unwrap() {
        let input = &case["input"];
        let expected = &case["expected"];
        let mut config = Config::default();
        config.set("sys_modules.enabled", json!(true));
        config.set("sys_modules.events.enabled", json!(true));
        let mut client = APCore::with_options(None, None, Some(config), None);
        client
            .register(
                "test.stream",
                Box::new(FixtureStream(input["chunks"].as_array().unwrap().clone())),
            )
            .unwrap();
        let on_error_calls = Arc::new(AtomicUsize::new(0));
        client
            .use_middleware(Box::new(FailAfter {
                code: match input["after_error_code"].as_str().unwrap() {
                    "EXECUTION_CANCELLED" => ErrorCode::ExecutionCancelled,
                    "MODULE_EXECUTE_ERROR" => ErrorCode::ModuleExecuteError,
                    code => panic!("unexpected fixture error code: {code}"),
                },
                on_error_calls: Arc::clone(&on_error_calls),
            }))
            .unwrap();
        let event_count = Arc::new(AtomicUsize::new(0));
        let captured = Arc::clone(&event_count);
        client
            .on("apcore.stream.post_validation_failed", move |_| {
                captured.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
        let mut stream = client.stream("test.stream", json!({}), None, None);
        let mut chunks = Vec::new();
        let mut error_code = Value::Null;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => chunks.push(chunk),
                Err(error) => error_code = json!(error.code.wire_str()),
            }
        }
        client.events().unwrap().flush_default().await.unwrap();
        assert_eq!(json!(chunks), expected["chunks"], "{}", case["id"]);
        assert_eq!(error_code, expected["error_code"], "{}", case["id"]);
        assert_eq!(
            json!(on_error_calls.load(Ordering::Relaxed)),
            expected["on_error_calls"]
        );
        assert_eq!(
            json!(event_count.load(Ordering::Relaxed)),
            expected["post_failure_events"]
        );
    }
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
