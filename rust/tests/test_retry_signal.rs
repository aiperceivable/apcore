//! Tests for RetrySignal middleware retry semantics (sync finding A-D-017).

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

#[derive(Debug)]
struct FailUntilRetriedModule {
    attempts: Arc<AtomicUsize>,
}

#[async_trait]
impl Module for FailUntilRetriedModule {
    fn description(&self) -> &'static str {
        "fails on first attempt with non-retried inputs; succeeds when inputs include retried=true"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if inputs.get("retried").and_then(serde_json::Value::as_bool) == Some(true) {
            Ok(json!({ "ok": true, "attempt": self.attempts.load(Ordering::SeqCst) }))
        } else {
            Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "first attempt always fails",
            ))
        }
    }
}

#[derive(Debug)]
struct RetryWithFlagMiddleware;

#[async_trait]
impl Middleware for RetryWithFlagMiddleware {
    fn name(&self) -> &'static str {
        "RetryWithFlag"
    }

    async fn before(
        &self,
        _module_id: &str,
        inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
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
        // Default path returns None; the retry path is in on_error_outcome.
        Ok(None)
    }

    async fn on_error_outcome(
        &self,
        _module_id: &str,
        inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<OnErrorOutcome>, ModuleError> {
        // First failure → flip the retried flag and retry the pipeline once.
        let mut new_inputs = inputs.as_object().cloned().unwrap_or_default();
        if new_inputs
            .get("retried")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            // Already retried — give up and let the error propagate.
            return Ok(None);
        }
        new_inputs.insert("retried".to_string(), json!(true));
        Ok(Some(OnErrorOutcome::Retry(RetrySignal::new(
            Value::Object(new_inputs),
        ))))
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
async fn middleware_retry_signal_re_runs_pipeline_with_new_inputs() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(Registry::new());
    registry
        .register(
            "test.retry",
            Box::new(FailUntilRetriedModule {
                attempts: Arc::clone(&attempts),
            }),
            dummy_descriptor("test.retry"),
        )
        .unwrap();

    let config = Arc::new(apcore::Config::from_defaults());
    let executor = Executor::new(Arc::clone(&registry), config);
    executor
        .use_middleware(Box::new(RetryWithFlagMiddleware))
        .unwrap();

    let ctx = Context::<Value>::new(dummy_identity());
    let result = executor
        .call("test.retry", json!({}), Some(&ctx), None)
        .await
        .expect("retry should succeed");

    assert_eq!(attempts.load(Ordering::SeqCst), 2, "exactly 2 attempts");
    assert_eq!(result["ok"], json!(true));
}
