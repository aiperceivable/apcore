//! Conformance driver for `executor_trace_cancellation.json` (sync finding
//! A-D-001).
//!
//! When the pipeline raises `ExecutionCancelledError` mid-execution, the trace
//! variant (`call_with_trace`) MUST propagate it directly (final error code
//! `EXECUTION_CANCELLED`) and MUST NOT route it through the `on_error`
//! middleware chain — a recovering `on_error` middleware MUST NOT be able to
//! suppress a cancellation. Mirrors `call()`/`call_async` (D-19 trace parity,
//! D-20 cancellation short-circuit).
//!
//! Driver contract (from the fixture): register a module whose `execute`
//! raises `ExecutionCancelledError`; install an `on_error` middleware that
//! records whether it was invoked and would otherwise recover; call
//! `call_with_trace`; assert it returns Err with code `EXECUTION_CANCELLED`
//! AND the recording `on_error` middleware was NOT invoked. Both `call()` and
//! `call_with_trace()` must behave identically.
#![allow(clippy::pedantic)] // fixture-driven test file: casts/layout follow the fixture schema

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{ExecutionCancelledError, Executor, Middleware, OnErrorOutcome};
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture() -> Value {
    let path = find_fixtures_root().join("executor_trace_cancellation.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON: {e}"))
}

fn fixture_case<'a>(fixture: &'a Value, id: &str) -> &'a Value {
    fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array")
        .iter()
        .find(|c| c["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("fixture missing test case {id}"))
}

// ---------------------------------------------------------------------------
// Test module + middleware
// ---------------------------------------------------------------------------

const MODULE_ID: &str = "test.cancelling_module";

/// A module whose `execute` raises `ExecutionCancelledError` (→ ModuleError
/// with code EXECUTION_CANCELLED).
#[derive(Debug)]
struct CancellingModule;

#[async_trait]
impl Module for CancellingModule {
    fn description(&self) -> &'static str {
        "module that cancels its own execution"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Err(ExecutionCancelledError::new(MODULE_ID, "Execution was cancelled").to_module_error())
    }
}

/// Middleware whose `before` is a pass-through (so it is recorded in
/// `executed_middlewares`) and whose `on_error` would recover for ANY error —
/// while recording that it was invoked. For a cancellation it MUST NOT be
/// invoked (D-20 short-circuit).
#[derive(Debug)]
struct RecordingRecoveryMiddleware {
    on_error_invoked: Arc<AtomicBool>,
}

#[async_trait]
impl Middleware for RecordingRecoveryMiddleware {
    fn name(&self) -> &'static str {
        "RecordingRecovery"
    }

    async fn before(
        &self,
        _module_id: &str,
        inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Pass inputs through so this middleware lands in executed_middlewares.
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
        self.on_error_invoked.store(true, Ordering::SeqCst);
        Ok(Some(json!({ "recovered": true })))
    }

    async fn on_error_outcome(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<OnErrorOutcome>, ModuleError> {
        self.on_error_invoked.store(true, Ordering::SeqCst);
        Ok(Some(OnErrorOutcome::Recovery(json!({ "recovered": true }))))
    }
}

fn cancelling_descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: MODULE_ID.to_string(),
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

fn build_executor(on_error_invoked: Arc<AtomicBool>) -> Executor {
    let registry = Arc::new(Registry::new());
    registry
        .register(
            MODULE_ID,
            Box::new(CancellingModule),
            cancelling_descriptor(),
        )
        .unwrap();
    let executor = Executor::new(registry, Arc::new(apcore::Config::from_defaults()));
    executor
        .use_middleware(Box::new(RecordingRecoveryMiddleware { on_error_invoked }))
        .unwrap();
    executor
}

#[tokio::test]
async fn trace_cancellation_propagates_bypassing_on_error() {
    let fixture = load_fixture();
    let tc = fixture_case(&fixture, "trace_cancellation_propagates_bypassing_on_error");
    let expected_error = tc["expected_error"].as_str().expect("expected_error");
    assert_eq!(
        expected_error, "EXECUTION_CANCELLED",
        "fixture contract drift: expected_error must be EXECUTION_CANCELLED"
    );
    let expected_on_error_invoked = tc["expected_on_error_invoked"]
        .as_bool()
        .expect("expected_on_error_invoked");

    // --- call_with_trace (the variant this fixture locks) ---
    {
        let on_error_invoked = Arc::new(AtomicBool::new(false));
        let executor = build_executor(Arc::clone(&on_error_invoked));
        let ctx = Context::<Value>::new(dummy_identity());

        let result = executor
            .call_with_trace(MODULE_ID, json!({}), Some(&ctx), None, None)
            .await;

        let err = result.err().unwrap_or_else(|| {
            panic!("call_with_trace must return Err for a cancellation, not a recovered success")
        });
        assert_eq!(
            err.code,
            ErrorCode::ExecutionCancelled,
            "call_with_trace must surface EXECUTION_CANCELLED, got {:?}",
            err.code
        );
        assert_eq!(
            on_error_invoked.load(Ordering::SeqCst),
            expected_on_error_invoked,
            "call_with_trace: the on_error middleware must NOT be invoked for a cancellation \
             (D-20 short-circuit)"
        );
    }

    // --- call (must behave identically per the fixture note) ---
    {
        let on_error_invoked = Arc::new(AtomicBool::new(false));
        let executor = build_executor(Arc::clone(&on_error_invoked));
        let ctx = Context::<Value>::new(dummy_identity());

        let result = executor.call(MODULE_ID, json!({}), Some(&ctx), None).await;

        let err = result
            .err()
            .unwrap_or_else(|| panic!("call must return Err for a cancellation"));
        assert_eq!(
            err.code,
            ErrorCode::ExecutionCancelled,
            "call must surface EXECUTION_CANCELLED, got {:?}",
            err.code
        );
        assert_eq!(
            on_error_invoked.load(Ordering::SeqCst),
            expected_on_error_invoked,
            "call: the on_error middleware must NOT be invoked for a cancellation"
        );
    }
}
