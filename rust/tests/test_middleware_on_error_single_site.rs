//! Regression tests for the single-site on_error / RetrySignal contract.
//!
//! A-D-010: on an after()-failure, the step-level recovery must NOT run in
//! addition to the executor-level recovery, so a middleware's on_error fires
//! exactly ONCE per failure (previously fired twice).
//!
//! A-D-012: step-level recovery used `execute_on_error` which maps a
//! RetrySignal to None (dropping it). With the executor as the sole on_error
//! site, a RetrySignal raised from a before()-failure is honored exactly once.
//!
//! Canonical behavior mirrors apcore-python / apcore-typescript, where
//! on_error recovery happens only at the executor level.

use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{Executor, Middleware, OnErrorOutcome, RetrySignal};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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

/// Module that always succeeds (the after-failure must come from middleware).
#[derive(Debug)]
struct OkModule;

#[async_trait]
impl Module for OkModule {
    fn description(&self) -> &'static str {
        "always ok"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "ok": true }))
    }
}

// ---------------------------------------------------------------------------
// A-D-010: on_error fires exactly once per after()-failure
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct AfterFailsCountingOnError {
    on_error_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Middleware for AfterFailsCountingOnError {
    fn name(&self) -> &'static str {
        "AfterFailsCountingOnError"
    }
    async fn before(
        &self,
        _m: &str,
        inputs: Value,
        _c: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Ok(Some(inputs))
    }
    async fn after(
        &self,
        _m: &str,
        _i: Value,
        _o: Value,
        _c: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Fail in the after-chain.
        Err(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "after() always fails",
        ))
    }
    async fn on_error(
        &self,
        _m: &str,
        _i: Value,
        _e: &ModuleError,
        _c: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Count every invocation; return None (no recovery) so the error
        // propagates. The point is to assert it is invoked exactly ONCE.
        self.on_error_calls.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
}

#[tokio::test]
async fn on_error_fires_exactly_once_on_after_failure() {
    let on_error_calls = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(Registry::new());
    registry
        .register("test.ok", Box::new(OkModule), dummy_descriptor("test.ok"))
        .unwrap();

    let executor = Executor::new(
        Arc::clone(&registry),
        Arc::new(apcore::Config::from_defaults()),
    );
    executor
        .use_middleware(Box::new(AfterFailsCountingOnError {
            on_error_calls: Arc::clone(&on_error_calls),
        }))
        .unwrap();

    let ctx = Context::<Value>::new(dummy_identity());
    let result = executor.call("test.ok", json!({}), Some(&ctx), None).await;

    assert!(
        result.is_err(),
        "after()-failure with no recovery must error"
    );
    assert_eq!(
        on_error_calls.load(Ordering::SeqCst),
        1,
        "on_error must fire exactly once per after()-failure (was twice before A-D-010 fix)"
    );
}

// ---------------------------------------------------------------------------
// A-D-012: a RetrySignal from a before()-failure triggers exactly one retry
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct RetryOnceFromBefore {
    before_calls: Arc<AtomicUsize>,
    on_error_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Middleware for RetryOnceFromBefore {
    fn name(&self) -> &'static str {
        "RetryOnceFromBefore"
    }
    async fn before(
        &self,
        _m: &str,
        inputs: Value,
        _c: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        let n = self.before_calls.fetch_add(1, Ordering::SeqCst);
        // First before() fails; the retried run (inputs.retried==true) passes.
        if inputs.get("retried").and_then(Value::as_bool) == Some(true) {
            Ok(Some(inputs))
        } else {
            let _ = n;
            Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "before() fails on first attempt",
            ))
        }
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
        Ok(None)
    }
    async fn on_error_outcome(
        &self,
        _m: &str,
        inputs: Value,
        _e: &ModuleError,
        _c: &Context<Value>,
    ) -> Result<Option<OnErrorOutcome>, ModuleError> {
        self.on_error_calls.fetch_add(1, Ordering::SeqCst);
        // Retry once: flip the retried flag so the next before() passes.
        let mut m = inputs.as_object().cloned().unwrap_or_default();
        if m.get("retried").and_then(Value::as_bool) == Some(true) {
            // Already retried once — give up.
            return Ok(None);
        }
        m.insert("retried".to_string(), json!(true));
        Ok(Some(OnErrorOutcome::Retry(RetrySignal::new(
            Value::Object(m),
        ))))
    }
}

#[tokio::test]
async fn retry_signal_from_before_failure_retries_exactly_once() {
    let before_calls = Arc::new(AtomicUsize::new(0));
    let on_error_calls = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(Registry::new());
    registry
        .register("test.ok", Box::new(OkModule), dummy_descriptor("test.ok"))
        .unwrap();

    let executor = Executor::new(
        Arc::clone(&registry),
        Arc::new(apcore::Config::from_defaults()),
    );
    executor
        .use_middleware(Box::new(RetryOnceFromBefore {
            before_calls: Arc::clone(&before_calls),
            on_error_calls: Arc::clone(&on_error_calls),
        }))
        .unwrap();

    let ctx = Context::<Value>::new(dummy_identity());
    let result = executor
        .call("test.ok", json!({}), Some(&ctx), None)
        .await
        .expect("retry from before()-failure must succeed");

    assert_eq!(result["ok"], json!(true));
    // before() ran twice: the failing first attempt + the successful retry.
    assert_eq!(
        before_calls.load(Ordering::SeqCst),
        2,
        "before() must run exactly twice (one retry)"
    );
    // on_error_outcome (the retry decision) fired exactly once — the retry was
    // honored, not dropped, and did not double-fire.
    assert_eq!(
        on_error_calls.load(Ordering::SeqCst),
        1,
        "RetrySignal from before()-failure must be honored exactly once (A-D-012)"
    );
}
