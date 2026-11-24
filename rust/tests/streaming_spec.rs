// Spec-traced contract tests for the apcore-rust streaming feature.
//
// Source spec: apcore/docs/features/streaming.md — "## Contract: Module.stream".
// Canonical clause list mirrored from:
//   apcore-python/tests/test_streaming_spec.py
//
// Each test maps to exactly one clause in the feature spec's Contract block.
// The verbatim cross-language clause id appears in a leading
// `// clause: <clause_id>` comment on the line above each test fn so a
// cross-language diff tool can line up the Python / TypeScript / Rust rows by
// that exact string. The fn name is the clause id flattened to snake_case.
//
// Cross-language divergence (architectural, applies to the whole file):
//   In Python/TS the streaming contract is exercised through `Executor.stream`
//   which *raises* on bad input / D-58 violations / mid-stream errors. The Rust
//   `Executor::stream` instead returns a `Stream<Item = Result<Value,
//   ModuleError>>` and surfaces every error as an `Err` *item* in the stream
//   (Phase-1 failures, D-58 rejects, and mid-stream module errors all appear as
//   stream items, never as a panic / Result from a single call). These tests
//   therefore drain the stream and assert on the yielded `Err` item — the real
//   Rust surface that implements the normative rules.
//
// Conventions copied from tests/core_executor_spec.rs and tests/test_true_streaming.rs.

use std::sync::{Arc, Mutex};

use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::executor::Executor;
use apcore::module::{ChunkStream, Module};
use apcore::registry::registry::Registry;
use apcore::{Config, Middleware};
use async_stream::stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Streaming test modules (mirror the Python fixtures)
// ---------------------------------------------------------------------------

/// Minimal streaming module: yields `{"value": i}` for i in 1..=count.
/// Mirrors Python `StreamingCounter`.
struct StreamingCounter;

#[async_trait]
impl Module for StreamingCounter {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["count"],
            "properties": { "count": { "type": "integer" } }
        })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "streaming counter"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "value": inputs["count"] }))
    }
    fn stream(&self, inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        let count = inputs["count"].as_i64().unwrap_or(0);
        Some(Box::pin(stream! {
            for i in 1..=count {
                yield Ok(json!({ "value": i }));
            }
        }))
    }
}

/// Non-streaming module — only `execute()`. Mirrors Python `PlainModule`.
struct PlainModule;

#[async_trait]
impl Module for PlainModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "plain module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "result": "done" }))
    }
}

/// Yields overlapping nested dicts to exercise the deep-merge accumulator.
/// Mirrors Python `NestedMergeModule`.
struct NestedMergeModule;

#[async_trait]
impl Module for NestedMergeModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "nested merge module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(json!({ "content": "Hello", "metadata": { "tokens": 1 } }));
            yield Ok(json!({ "content": " world", "metadata": { "tokens": 1, "model": "gpt-4" } }));
        }))
    }
}

/// Yields one valid object chunk then a single caller-supplied bad chunk.
/// Mirrors Python `BadChunkModule`.
struct BadChunkModule {
    bad_chunk: Value,
}

#[async_trait]
impl Module for BadChunkModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "bad chunk module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        let bad = self.bad_chunk.clone();
        Some(Box::pin(stream! {
            yield Ok(json!({ "a": 1 }));
            yield Ok(bad);
        }))
    }
}

/// Yields one valid chunk then fails mid-stream. Mirrors Python `MidFailModule`.
struct MidFailModule;

#[async_trait]
impl Module for MidFailModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "mid-fail module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(json!({ "ok": 1 }));
            yield Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "boom mid-stream",
            ));
        }))
    }
}

/// Stateful streaming module: yields `{"call": n}` where n increments per
/// invocation. Mirrors Python `CounterStateModule` (non-idempotent).
struct CounterStateModule {
    calls: Arc<Mutex<i64>>,
}

#[async_trait]
impl Module for CounterStateModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "counter state module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        let calls = Arc::clone(&self.calls);
        Some(Box::pin(stream! {
            let n = {
                let mut g = calls.lock().unwrap();
                *g += 1;
                *g
            };
            yield Ok(json!({ "call": n }));
        }))
    }
}

/// Mutates external state while streaming. Mirrors Python `ImpureModule`.
struct ImpureModule {
    side_effects: Arc<Mutex<Vec<i64>>>,
}

#[async_trait]
impl Module for ImpureModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "impure module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        let sink = Arc::clone(&self.side_effects);
        Some(Box::pin(stream! {
            for i in 0..3i64 {
                sink.lock().unwrap().push(i);
                yield Ok(json!({ "i": i }));
            }
        }))
    }
}

/// Yields two chunks nested past the 32-level merge cap with a divergent leaf.
/// Mirrors Python `DeepModule`.
struct DeepModule;

fn nested(depth: usize, leaf: &str) -> Value {
    let mut node = json!({ "leaf": leaf });
    for _ in 0..depth {
        node = json!({ "n": node });
    }
    node
}

#[async_trait]
impl Module for DeepModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "deep nesting module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(nested(40, "a"));
            yield Ok(nested(40, "b"));
        }))
    }
}

// ---------------------------------------------------------------------------
// Middleware: captures the Phase-3 merged output and before/after ordering.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CaptureMiddleware {
    order: Arc<Mutex<Vec<String>>>,
    captured: Arc<Mutex<Option<Value>>>,
}

#[async_trait]
impl Middleware for CaptureMiddleware {
    fn name(&self) -> &'static str {
        "Capture"
    }
    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.order.lock().unwrap().push("before".to_string());
        Ok(None)
    }
    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.order.lock().unwrap().push("after".to_string());
        *self.captured.lock().unwrap() = Some(output);
        Ok(None)
    }
    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an executor with a single registered module.
fn make_executor(module_id: &str, module: Box<dyn Module>) -> Executor {
    let reg = Registry::new();
    reg.register_module(module_id, module)
        .expect("register module");
    Executor::new(reg, Config::default())
}

/// A context whose `caller_id` is set so `BuiltinContextCreation` does not
/// clobber it (matching the established convention in test_true_streaming.rs).
fn external_context() -> Context<Value> {
    let mut ctx = Context::<Value>::new(Identity::new(
        "@external".to_string(),
        "external".to_string(),
        vec![],
        std::collections::HashMap::new(),
    ));
    ctx.caller_id = Some("@external".to_string());
    ctx
}

/// Drain the stream, collecting Ok chunks; returns the first Err encountered.
async fn drain(
    mut s: std::pin::Pin<
        Box<dyn futures_core::Stream<Item = Result<Value, ModuleError>> + Send + '_>,
    >,
) -> (Vec<Value>, Option<ModuleError>) {
    let mut chunks = Vec::new();
    let mut err = None;
    while let Some(item) = s.next().await {
        match item {
            Ok(v) => chunks.push(v),
            Err(e) => {
                err = Some(e);
                break;
            }
        }
    }
    (chunks, err)
}

/// The string code carried by a `ModuleError` (SCREAMING_SNAKE_CASE) via
/// `to_dict()["code"]` — the canonical serialized wire form.
fn code_str(err: &ModuleError) -> String {
    err.to_dict()
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

// ===========================================================================
// INPUT
// ===========================================================================

// clause: streaming.stream.input.inputs_validated_against_schema
#[tokio::test]
async fn stream_input_inputs_validated_against_schema() {
    // A schema-conformant input streams normally.
    let ex = make_executor("counter", Box::new(StreamingCounter));
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("counter", json!({ "count": 2 }), Some(&ctx), None)).await;
    assert!(err.is_none(), "valid input must not surface an error item");
    assert_eq!(chunks, vec![json!({ "value": 1 }), json!({ "value": 2 })]);
}

// clause: streaming.stream.input.context_accepted
#[tokio::test]
async fn stream_input_context_accepted() {
    // An explicitly-supplied Context is accepted and the stream runs under it.
    let ex = make_executor("counter", Box::new(StreamingCounter));
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("counter", json!({ "count": 1 }), Some(&ctx), None)).await;
    assert!(err.is_none());
    assert_eq!(chunks, vec![json!({ "value": 1 })]);
}

// ===========================================================================
// ERROR
// ===========================================================================

// clause: streaming.stream.error.schema_validation_on_bad_inputs
#[tokio::test]
async fn stream_error_schema_validation_on_bad_inputs() {
    // count must be int; a string fails input_schema validation in Phase 1.
    // Cross-language note: Python raises SchemaValidationError; Rust surfaces it
    // as the (first) Err item of the stream with code SCHEMA_VALIDATION_ERROR.
    let ex = make_executor("counter", Box::new(StreamingCounter));
    let ctx = external_context();
    let (_chunks, err) = drain(ex.stream(
        "counter",
        json!({ "count": "not-an-int" }),
        Some(&ctx),
        None,
    ))
    .await;
    let err = err.expect("bad input must surface an Err item");
    assert_eq!(err.code, ErrorCode::SchemaValidationError);
    assert_eq!(code_str(&err), "SCHEMA_VALIDATION_ERROR");
}

// clause: streaming.stream.error.mid_stream_error_surfaced
#[tokio::test]
async fn stream_error_mid_stream_error_surfaced() {
    // A chunk emitted before the error is delivered; the error then surfaces and
    // the stream terminates. Cross-language note: Python wraps mid-stream errors
    // in ModuleError; Rust forwards the module's Err item directly (here the
    // module yields a ModuleExecutionError).
    let ex = make_executor("midfail", Box::new(MidFailModule));
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("midfail", json!({}), Some(&ctx), None)).await;
    let err = err.expect("mid-stream error must surface as an Err item");
    assert_eq!(chunks, vec![json!({ "ok": 1 })]);
    assert_eq!(err.code, ErrorCode::ModuleExecuteError);
}

// ===========================================================================
// RETURN  (D-58 chunk-shape rule + lazy async stream)
// ===========================================================================

// clause: streaming.stream.return.async_iterator_of_objects
#[tokio::test]
async fn stream_return_async_iterator_of_objects() {
    // On success, a lazy async Stream of objects. Every yielded chunk is an
    // object.
    let ex = make_executor("counter", Box::new(StreamingCounter));
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("counter", json!({ "count": 3 }), Some(&ctx), None)).await;
    assert!(err.is_none());
    assert_eq!(
        chunks,
        vec![
            json!({ "value": 1 }),
            json!({ "value": 2 }),
            json!({ "value": 3 })
        ]
    );
    assert!(chunks.iter().all(Value::is_object));
}

// clause: streaming.stream.return.d58_reject_non_object_string
#[tokio::test]
async fn stream_return_d58_reject_non_object_string() {
    // D-58: a string chunk MUST be rejected with code GENERAL_INVALID_INPUT,
    // details.code=STREAM_CHUNK_NOT_OBJECT, actual_type='string', and MUST NOT
    // be delivered.
    let module = BadChunkModule {
        bad_chunk: json!("nope"),
    };
    let ex = make_executor("bad", Box::new(module));
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("bad", json!({}), Some(&ctx), None)).await;
    let err = err.expect("non-object chunk must surface an Err item");
    assert_eq!(
        chunks,
        vec![json!({ "a": 1 })],
        "invalid chunk never delivered"
    );
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
    assert_eq!(code_str(&err), "GENERAL_INVALID_INPUT");
    assert_eq!(
        err.details.get("code").and_then(Value::as_str),
        Some("STREAM_CHUNK_NOT_OBJECT")
    );
    assert_eq!(
        err.details.get("actual_type").and_then(Value::as_str),
        Some("string")
    );
    assert_eq!(
        err.details.get("chunk_index").and_then(Value::as_i64),
        Some(1)
    );
}

// clause: streaming.stream.return.d58_reject_non_object_array
#[tokio::test]
async fn stream_return_d58_reject_non_object_array() {
    // D-58: an array chunk is non-object and MUST be rejected (actual_type='array')
    // without being delivered.
    let module = BadChunkModule {
        bad_chunk: json!([1, 2]),
    };
    let ex = make_executor("bad", Box::new(module));
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("bad", json!({}), Some(&ctx), None)).await;
    let err = err.expect("array chunk must surface an Err item");
    assert_eq!(chunks, vec![json!({ "a": 1 })]);
    assert_eq!(
        err.details.get("actual_type").and_then(Value::as_str),
        Some("array")
    );
}

// clause: streaming.stream.return.d58_reject_non_object_scalars
#[tokio::test]
async fn stream_return_d58_reject_non_object_scalars() {
    // D-58: number, bool, and null chunks are all non-object and MUST be
    // rejected with the correct JSON type name in details.actual_type, without
    // being delivered. (Parametrized in Python; unrolled here.)
    let cases: &[(Value, &str)] = &[
        (json!(3), "number"),
        (json!(true), "bool"),
        (json!(null), "null"),
    ];
    for (bad, expected_type) in cases {
        let module = BadChunkModule {
            bad_chunk: bad.clone(),
        };
        let ex = make_executor("bad", Box::new(module));
        let ctx = external_context();
        let (chunks, err) = drain(ex.stream("bad", json!({}), Some(&ctx), None)).await;
        let err = err.expect("scalar chunk must surface an Err item");
        assert_eq!(chunks, vec![json!({ "a": 1 })], "case {expected_type}");
        assert_eq!(
            err.details.get("actual_type").and_then(Value::as_str),
            Some(*expected_type),
            "case {expected_type}"
        );
        assert_eq!(
            err.details.get("chunk_index").and_then(Value::as_i64),
            Some(1),
            "case {expected_type}"
        );
    }
}

// ===========================================================================
// PROPERTY
// ===========================================================================

// clause: streaming.stream.property.async
#[tokio::test]
async fn stream_property_async() {
    // stream() returns an async Stream; awaiting the first item resolves to the
    // first chunk. (Rust's stream() is the async surface — driven via .next().await.)
    let ex = make_executor("counter", Box::new(StreamingCounter));
    let ctx = external_context();
    let mut s = ex.stream("counter", json!({ "count": 1 }), Some(&ctx), None);
    let first = s
        .next()
        .await
        .expect("stream yields a first item")
        .expect("first item is Ok");
    assert_eq!(first, json!({ "value": 1 }));
}

// clause: streaming.stream.property.thread_safe_false
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stream_property_thread_safe_false() {
    // thread_safe = false: a single stream instance MUST NOT be shared across
    // consumers. We assert the contract operationally: >=8 *independent* stream
    // instances run concurrently (one per spawned task), each producing its own
    // correct, isolated sequence (no cross-talk).
    let ex = Arc::new(make_executor("counter", Box::new(StreamingCounter)));
    let mut handles = Vec::new();
    for n in 1..=8i64 {
        let ex = Arc::clone(&ex);
        handles.push(tokio::spawn(async move {
            let ctx = external_context();
            let (chunks, err) =
                drain(ex.stream("counter", json!({ "count": n }), Some(&ctx), None)).await;
            assert!(err.is_none(), "concurrent stream {n} must not error");
            (n, chunks)
        }));
    }
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.expect("task join"));
    }
    results.sort_by_key(|(n, _)| *n);
    assert_eq!(results.len(), 8);
    for (n, chunks) in results {
        let expected: Vec<Value> = (1..=n).map(|i| json!({ "value": i })).collect();
        assert_eq!(chunks, expected, "isolated sequence for n={n}");
    }
}

// clause: streaming.stream.property.idempotent_false
#[tokio::test]
async fn stream_property_idempotent_false() {
    // idempotent = false: a stateful streaming module yields different sequences
    // across repeated invocations.
    let calls = Arc::new(Mutex::new(0i64));
    let module = CounterStateModule {
        calls: Arc::clone(&calls),
    };
    let ex = make_executor("stateful", Box::new(module));
    let ctx = external_context();
    let (first, e1) = drain(ex.stream("stateful", json!({}), Some(&ctx), None)).await;
    let (second, e2) = drain(ex.stream("stateful", json!({}), Some(&ctx), None)).await;
    assert!(e1.is_none() && e2.is_none());
    assert_eq!(first, vec![json!({ "call": 1 })]);
    assert_eq!(second, vec![json!({ "call": 2 })]);
    assert_ne!(first, second, "not idempotent");
}

// clause: streaming.stream.property.pure_false
#[tokio::test]
async fn stream_property_pure_false() {
    // pure = false: a module that mutates external state during streaming proves
    // the call is not pure.
    let side_effects = Arc::new(Mutex::new(Vec::<i64>::new()));
    let module = ImpureModule {
        side_effects: Arc::clone(&side_effects),
    };
    let ex = make_executor("impure", Box::new(module));
    let ctx = external_context();
    let (_chunks, err) = drain(ex.stream("impure", json!({}), Some(&ctx), None)).await;
    assert!(err.is_none());
    assert_eq!(
        *side_effects.lock().unwrap(),
        vec![0, 1, 2],
        "external state was mutated"
    );
}

// ===========================================================================
// SIDE_EFFECT  (ordering, accumulation, fallback, post-validation phase)
// ===========================================================================

// clause: streaming.stream.side_effect.chunk_ordering_preserved
#[tokio::test]
async fn stream_side_effect_chunk_ordering_preserved() {
    // Phase 2: the executor yields each chunk immediately and in source order.
    let ex = make_executor("counter", Box::new(StreamingCounter));
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("counter", json!({ "count": 5 }), Some(&ctx), None)).await;
    assert!(err.is_none());
    let expected: Vec<Value> = (1..=5).map(|i| json!({ "value": i })).collect();
    assert_eq!(chunks, expected);
}

// clause: streaming.stream.side_effect.deep_merge_accumulation
#[tokio::test]
async fn stream_side_effect_deep_merge_accumulation() {
    // Chunks MUST be accumulated via deep merge to produce the final combined
    // output for Phase-3 validation. Nested dicts merge recursively; right value
    // wins for scalars (per the spec's worked example). We observe the merged
    // output via an after-middleware that captures it.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = Arc::new(Mutex::new(None));
    let ex = make_executor("merge", Box::new(NestedMergeModule));
    ex.use_middleware(Box::new(CaptureMiddleware {
        order: Arc::clone(&order),
        captured: Arc::clone(&captured),
    }))
    .expect("add middleware");
    let ctx = external_context();
    let (_chunks, err) = drain(ex.stream("merge", json!({}), Some(&ctx), None)).await;
    assert!(err.is_none());
    let merged = captured
        .lock()
        .unwrap()
        .clone()
        .expect("after captured merged output");
    assert_eq!(
        merged,
        json!({
            "content": " world",
            "metadata": { "tokens": 1, "model": "gpt-4" }
        })
    );
}

// clause: streaming.stream.side_effect.fallback_single_chunk
#[tokio::test]
async fn stream_side_effect_fallback_single_chunk() {
    // If a module does not implement stream(), the executor's stream() MUST fall
    // back to execute() and yield the complete result as a single chunk.
    let ex = make_executor("plain", Box::new(PlainModule));
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("plain", json!({}), Some(&ctx), None)).await;
    assert!(err.is_none());
    assert_eq!(chunks, vec![json!({ "result": "done" })]);
}

// clause: streaming.stream.side_effect.after_middleware_post_accumulation
#[tokio::test]
async fn stream_side_effect_after_middleware_post_accumulation() {
    // Phase 3: output validation + after-middleware MUST run on the accumulated
    // output AFTER all chunks are emitted — before runs first, after runs last,
    // and after receives the merged output.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = Arc::new(Mutex::new(None));
    let ex = make_executor("merge", Box::new(NestedMergeModule));
    ex.use_middleware(Box::new(CaptureMiddleware {
        order: Arc::clone(&order),
        captured: Arc::clone(&captured),
    }))
    .expect("add middleware");
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("merge", json!({}), Some(&ctx), None)).await;
    assert!(err.is_none());
    assert_eq!(chunks.len(), 2);
    assert_eq!(*order.lock().unwrap(), vec!["before", "after"]);
    let merged = captured
        .lock()
        .unwrap()
        .clone()
        .expect("after captured merged output");
    assert_eq!(merged["metadata"], json!({ "tokens": 1, "model": "gpt-4" }));
}

// clause: streaming.stream.side_effect.deep_merge_depth_capped
#[tokio::test]
async fn stream_side_effect_deep_merge_depth_capped() {
    // Deep merge MUST be depth-capped (default 32): nesting past the cap MUST NOT
    // overflow the stack, and at the cap the right value MUST win.
    //
    // Cross-language note: the Python suite documents this clause as currently
    // FAILING (its impl drops the override at the cap). The Rust impl
    // (src/executor.rs:117-121) replaces with the overlay at the cap, so the
    // surviving leaf is the right chunk's "b" — this test asserts the spec's
    // right-value-wins requirement and is expected to PASS in Rust.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = Arc::new(Mutex::new(None));
    let ex = make_executor("deep", Box::new(DeepModule));
    ex.use_middleware(Box::new(CaptureMiddleware {
        order: Arc::clone(&order),
        captured: Arc::clone(&captured),
    }))
    .expect("add middleware");
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("deep", json!({}), Some(&ctx), None)).await;
    // Must complete without stack overflow despite 40 > 32 levels.
    assert!(err.is_none());
    assert_eq!(chunks.len(), 2);
    let merged = captured
        .lock()
        .unwrap()
        .clone()
        .expect("after captured merged output");
    let mut node = &merged;
    for _ in 0..40 {
        node = &node["n"];
    }
    // Spec mandates right-value-wins at the depth cap.
    assert_eq!(node["leaf"], json!("b"));
}

// ===========================================================================
// A-D-015 — Phase-2 (mid-stream) non-cancel error runs middleware on_error
// recovery and yields the recovery value (parity with Python executor.py:1069
// and TS). Without the fix the raw error propagated as a stream Err item.
// ===========================================================================

/// Yields one Ok chunk, then an Err chunk (non-cancellation error).
#[derive(Debug)]
struct ErrorMidStreamModule;

#[async_trait]
impl Module for ErrorMidStreamModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "yields one chunk then errors mid-stream"
    }
    async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn stream(&self, _i: Value, _c: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(json!({ "partial": "first" }));
            yield Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "mid-stream failure",
            ));
        }))
    }
}

/// Middleware whose on_error returns a recovery value.
#[derive(Debug)]
struct RecoveryMiddleware;

#[async_trait]
impl Middleware for RecoveryMiddleware {
    fn name(&self) -> &'static str {
        "Recovery"
    }
    async fn before(
        &self,
        _m: &str,
        inputs: Value,
        _c: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Must run a before() so the executor records this middleware as
        // "executed" and runs its on_error during Phase-2 recovery.
        Ok(Some(inputs))
    }
    async fn after(
        &self,
        _m: &str,
        _i: Value,
        _o: Value,
        _c: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }
    async fn on_error(
        &self,
        _m: &str,
        _i: Value,
        _e: &ModuleError,
        _c: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(Some(json!({ "recovered": true })))
    }
}

#[tokio::test]
async fn stream_phase2_error_runs_on_error_recovery_and_yields_recovery_value() {
    let ex = make_executor("err.mid", Box::new(ErrorMidStreamModule));
    ex.use_middleware(Box::new(RecoveryMiddleware))
        .expect("add middleware");
    let ctx = external_context();
    let (chunks, err) = drain(ex.stream("err.mid", json!({}), Some(&ctx), None)).await;

    // The recovery value is yielded; no Err item surfaces.
    assert!(
        err.is_none(),
        "Phase-2 error must be recovered, not surfaced as a stream Err: {err:?}"
    );
    assert_eq!(
        chunks,
        vec![json!({ "partial": "first" }), json!({ "recovered": true })],
        "first chunk then the on_error recovery value"
    );
}
