//! Regression test for sync finding A-D-001.
//!
//! `Executor::stream()` Phase 1 (pre-execute pipeline) must run the middleware
//! `on_error` recovery chain over the executed middlewares — mirroring `call()`
//! and the Python/TypeScript SDKs — and yield any recovery value as the
//! stream's chunk before surfacing the error. Previously a Phase-1 failure
//! short-circuited via `?` with NO on_error chain, silently dropping recovery.

use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::{ChunkStream, Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{Executor, Middleware, OnErrorOutcome};
use async_stream::stream;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

/// A streaming module whose `input_schema` requires a field that the test does
/// NOT supply, so the Phase-1 `input_validation` step fails — *after* the
/// `before` middleware has run and been recorded in `executed_middlewares`.
/// If validation ever passed, the stream would yield the chunks below.
#[derive(Debug)]
struct StrictStreamingModule;

#[async_trait]
impl Module for StrictStreamingModule {
    fn description(&self) -> &'static str {
        "streaming module that requires a 'required_field' input"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["required_field"],
            "properties": { "required_field": { "type": "string" } }
        })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "executed": true }))
    }
    fn stream(&self, _inputs: Value, _ctx: &Context<Value>) -> Option<ChunkStream> {
        Some(Box::pin(stream! {
            yield Ok(json!({ "chunk": 0 }));
        }))
    }
}

/// Middleware whose `before` is a no-op (so it lands in `executed_middlewares`)
/// and whose `on_error` returns a recovery value. In `call()` this recovery is
/// returned; in `stream()` it MUST be yielded as the single chunk.
#[derive(Debug)]
struct RecoveryMiddleware;

#[async_trait]
impl Middleware for RecoveryMiddleware {
    fn name(&self) -> &'static str {
        "Recovery"
    }

    async fn before(
        &self,
        _module_id: &str,
        inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Pass inputs through unchanged so this middleware is recorded as
        // executed and input_validation runs next (and fails).
        Ok(Some(inputs))
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        _output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(None)
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(Some(json!({ "recovered": true })))
    }

    async fn on_error_outcome(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<OnErrorOutcome>, ModuleError> {
        Ok(Some(OnErrorOutcome::Recovery(json!({ "recovered": true }))))
    }
}

fn dummy_descriptor(name: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: name.to_string(),
        name: None,
        description: String::new(),
        documentation: None,
        input_schema: json!({ "type": "object" }),
        output_schema: json!({ "type": "object" }),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

fn dummy_identity() -> Identity {
    Identity::new(
        "@external".to_string(),
        "external".to_string(),
        vec![],
        HashMap::new(),
    )
}

#[tokio::test]
async fn stream_phase1_failure_yields_middleware_on_error_recovery() {
    let registry = Arc::new(Registry::new());
    registry
        .register(
            "test.strict_stream",
            Box::new(StrictStreamingModule),
            dummy_descriptor("test.strict_stream"),
        )
        .unwrap();

    let config = Arc::new(apcore::Config::from_defaults());
    let executor = Executor::new(Arc::clone(&registry), config);
    executor
        .use_middleware(Box::new(RecoveryMiddleware))
        .unwrap();

    let ctx = Context::<Value>::new(dummy_identity());

    // Sanity: the non-streaming call() path already recovers — establishes the
    // expected cross-path behavior the stream() path must match.
    let call_result = executor
        .call("test.strict_stream", json!({}), Some(&ctx), None)
        .await
        .expect("call() should recover via middleware on_error");
    assert_eq!(call_result, json!({ "recovered": true }));

    // The streaming path: Phase-1 input_validation fails; the executed
    // RecoveryMiddleware's on_error returns a recovery value, which MUST be
    // yielded as the single chunk (matching Python/TS), not dropped.
    let mut stream = executor.stream("test.strict_stream", json!({}), Some(&ctx), None);

    let first = stream
        .next()
        .await
        .expect("stream must yield a recovery chunk, not end empty");
    let chunk = first.expect("the yielded item must be the recovery value, not an Err");
    assert_eq!(
        chunk,
        json!({ "recovered": true }),
        "Phase-1 on_error recovery must be yielded as the stream chunk"
    );

    // No further items after the recovery chunk.
    assert!(
        stream.next().await.is_none(),
        "stream must end after yielding the single recovery chunk"
    );
}
