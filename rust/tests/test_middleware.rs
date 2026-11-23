//! Tests for middleware pipeline and RetryMiddleware.

use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::middleware::base::Middleware;
use apcore::middleware::{RetryConfig, RetryMiddleware};
use apcore::module::Module;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

// -- Test module that fails N times then succeeds --

#[allow(dead_code)]
struct FailNTimesModule {
    #[allow(dead_code)]
    fail_count: std::sync::atomic::AtomicU32,
    max_fails: u32,
}

#[allow(dead_code)]
impl FailNTimesModule {
    fn new(max_fails: u32) -> Self {
        Self {
            fail_count: std::sync::atomic::AtomicU32::new(0),
            max_fails,
        }
    }
}

#[async_trait]
impl Module for FailNTimesModule {
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    fn description(&self) -> &'static str {
        "Fails N times then succeeds"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let count = self
            .fail_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count < self.max_fails {
            Err(
                ModuleError::new(ErrorCode::ModuleExecuteError, "intentional failure")
                    .with_retryable(true),
            )
        } else {
            Ok(json!({"ok": true}))
        }
    }
}

// -- Test middleware that tracks calls --

#[derive(Debug)]
struct TrackingMiddleware {
    name: String,
    before_calls: std::sync::atomic::AtomicU32,
    after_calls: std::sync::atomic::AtomicU32,
}

impl TrackingMiddleware {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            before_calls: std::sync::atomic::AtomicU32::new(0),
            after_calls: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl Middleware for TrackingMiddleware {
    fn name(&self) -> &str {
        &self.name
    }
    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.before_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(None)
    }
    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        _output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.after_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

// -- Tests --

#[test]
fn test_retry_config_defaults() {
    let config = RetryConfig::default();
    assert_eq!(config.max_retries, 3);
    assert_eq!(config.strategy, "exponential");
    assert_eq!(config.base_delay_ms, 100);
    assert_eq!(config.max_delay_ms, 5000);
    assert!(config.jitter);
}

#[tokio::test]
async fn test_retry_middleware_skips_non_retryable() {
    let mw = RetryMiddleware::new(RetryConfig::default());
    let ctx = Context::<Value>::new(Identity::new(
        "test".into(),
        "test".into(),
        vec![],
        HashMap::default(),
    ));
    // A code the policy resolves to Some(false) — a governance refusal, which
    // is the shape this test exists to refuse. `MODULE_EXECUTE_ERROR` alone
    // would only prove the `None` branch, and since apcore#36 that is no longer
    // the only non-retryable shape.
    let error = ModuleError::new(ErrorCode::ACLDenied, "denied");
    assert_eq!(error.retryable, Some(false), "precondition");
    let result = mw
        .on_error("test.mod", json!({}), &error, &ctx)
        .await
        .unwrap();
    assert!(result.is_none(), "Should not retry non-retryable errors");

    // The other non-retryable shape: left unset for the module author.
    let unset = ModuleError::new(ErrorCode::ModuleExecuteError, "fail");
    assert_eq!(unset.retryable, None, "precondition");
    let result = mw
        .on_error("test.mod", json!({}), &unset, &ctx)
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "Should not retry errors with retryable unset"
    );
}

#[tokio::test]
async fn test_retry_middleware_retries_retryable_error() {
    let mut retry_cfg = RetryConfig::default();
    retry_cfg.max_retries = 2;
    retry_cfg.strategy = "fixed".to_string();
    retry_cfg.base_delay_ms = 1; // minimal delay for tests
    retry_cfg.max_delay_ms = 1;
    retry_cfg.jitter = false;
    let mw = RetryMiddleware::new(retry_cfg);
    let ctx = Context::<Value>::new(Identity::new(
        "test".into(),
        "test".into(),
        vec![],
        HashMap::default(),
    ));
    let error = ModuleError::new(ErrorCode::ModuleExecuteError, "fail").with_retryable(true);

    // First retry should succeed (count 0 < max 2)
    let result = mw
        .on_error("test.mod", json!({"x": 1}), &error, &ctx)
        .await
        .unwrap();
    assert!(result.is_some(), "Should return inputs for retry");

    // Second retry (count 1 < max 2)
    let result = mw
        .on_error("test.mod", json!({"x": 1}), &error, &ctx)
        .await
        .unwrap();
    assert!(result.is_some(), "Should return inputs for second retry");

    // Third attempt should be rejected (count 2 >= max 2)
    let result = mw
        .on_error("test.mod", json!({"x": 1}), &error, &ctx)
        .await
        .unwrap();
    assert!(result.is_none(), "Should stop after max_retries");
}

#[tokio::test]
async fn test_retry_middleware_resets_on_success() {
    let mut retry_cfg = RetryConfig::default();
    retry_cfg.max_retries = 2;
    retry_cfg.strategy = "fixed".to_string();
    retry_cfg.base_delay_ms = 1;
    retry_cfg.max_delay_ms = 1;
    retry_cfg.jitter = false;
    let mw = RetryMiddleware::new(retry_cfg);
    let ctx = Context::<Value>::new(Identity::new(
        "test".into(),
        "test".into(),
        vec![],
        HashMap::default(),
    ));
    let error = ModuleError::new(ErrorCode::ModuleExecuteError, "fail").with_retryable(true);

    // First retry
    let _ = mw
        .on_error("test.mod", json!({}), &error, &ctx)
        .await
        .unwrap();

    // Simulate success — after() should reset count
    let _ = mw
        .after("test.mod", json!({}), json!({}), &ctx)
        .await
        .unwrap();

    // After reset, retry should work again (count back to 0)
    let result = mw
        .on_error("test.mod", json!({}), &error, &ctx)
        .await
        .unwrap();
    assert!(result.is_some(), "Should retry after reset");
}

#[tokio::test]
async fn test_middleware_manager_pipeline_order() {
    use apcore::middleware::MiddlewareManager;

    let mgr = MiddlewareManager::new();
    // add() takes Box, so we verify snapshot order
    mgr.add(Box::new(TrackingMiddleware::new("first"))).unwrap();
    mgr.add(Box::new(TrackingMiddleware::new("second")))
        .unwrap();

    let names = mgr.snapshot();
    assert_eq!(names, vec!["first", "second"]);
}

#[test]
fn test_middleware_manager_remove() {
    use apcore::middleware::MiddlewareManager;

    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(TrackingMiddleware::new("alpha"))).unwrap();
    mgr.add(Box::new(TrackingMiddleware::new("beta"))).unwrap();

    assert!(mgr.remove("alpha"));
    assert!(!mgr.remove("alpha")); // already removed
    assert_eq!(mgr.snapshot(), vec!["beta"]);
}

// ---------------------------------------------------------------------------
// A-D-402: BeforeAdapter + AfterAdapter let closures be registered as Middleware
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_before_adapter_registers_and_runs_in_manager() {
    use apcore::middleware::adapters::BeforeAdapter;
    use apcore::middleware::MiddlewareManager;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    let captured: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured);

    let adapter = BeforeAdapter::new(
        "my-before",
        move |module_id: String, inputs: Value, _ctx: Context<Value>| {
            let captured = Arc::clone(&captured_clone);
            async move {
                captured.lock().unwrap().push(module_id);
                // Modify inputs by injecting a marker.
                let mut obj = inputs.as_object().cloned().unwrap_or_default();
                obj.insert("via_adapter".into(), json!(true));
                Ok(Some(serde_json::Value::Object(obj)))
            }
        },
    );

    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(adapter)).expect("register adapter");

    let ctx = Context::<Value>::new(Identity::new(
        "tester".into(),
        "test".into(),
        vec![],
        HashMap::default(),
    ));
    let (modified, _executed) = mgr
        .execute_before("my.module", json!({"x": 1}), &ctx)
        .await
        .expect("execute_before");

    assert_eq!(
        captured.lock().unwrap().as_slice(),
        &["my.module".to_string()]
    );
    assert_eq!(modified.get("via_adapter"), Some(&json!(true)));
    assert_eq!(modified.get("x"), Some(&json!(1)));
}

#[tokio::test]
async fn test_after_adapter_registers_and_runs_in_manager() {
    use apcore::middleware::adapters::AfterAdapter;
    use apcore::middleware::MiddlewareManager;

    let adapter = AfterAdapter::new(
        "my-after",
        |_module_id: String, _inputs: Value, output: Value, _ctx: Context<Value>| async move {
            let mut obj = output.as_object().cloned().unwrap_or_default();
            obj.insert("after_marker".into(), json!("seen"));
            Ok(Some(serde_json::Value::Object(obj)))
        },
    );

    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(adapter)).expect("register adapter");

    let ctx = Context::<Value>::new(Identity::new(
        "tester".into(),
        "test".into(),
        vec![],
        HashMap::default(),
    ));
    let modified = mgr
        .execute_after("my.module", json!({"in": 1}), json!({"out": 2}), &ctx)
        .await
        .expect("execute_after");

    assert_eq!(modified.get("after_marker"), Some(&json!("seen")));
    assert_eq!(modified.get("out"), Some(&json!(2)));
}
