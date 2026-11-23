//! Tests for Executor, MiddlewareManager, and APCore client integration.

use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::middleware::base::Middleware;
use apcore::module::Module;
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test module
// ---------------------------------------------------------------------------

struct AddModule;

#[async_trait]
impl Module for AddModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object", "properties": {"result": {"type": "integer"}}})
    }
    fn description(&self) -> &'static str {
        "Add two numbers"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let a = inputs["a"].as_i64().unwrap_or(0);
        let b = inputs["b"].as_i64().unwrap_or(0);
        Ok(json!({"result": a + b}))
    }
}

// ---------------------------------------------------------------------------
// Test middleware
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PrefixMiddleware;

#[async_trait]
impl Middleware for PrefixMiddleware {
    fn name(&self) -> &'static str {
        "prefix"
    }
    async fn before(
        &self,
        _module_id: &str,
        mut inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Add a prefix field to prove before() ran
        inputs["_prefixed"] = json!(true);
        Ok(Some(inputs))
    }
    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        mut output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Add a suffix field to prove after() ran
        output["_suffixed"] = json!(true);
        Ok(Some(output))
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

#[derive(Debug)]
struct TagMiddleware;

#[async_trait]
impl Middleware for TagMiddleware {
    fn name(&self) -> &'static str {
        "tag"
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
        mut output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        output["_tagged"] = json!(true);
        Ok(Some(output))
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
// Tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Global deadline (sync A-D-201)
// ---------------------------------------------------------------------------

/// Module that asserts ctx.global_deadline is set when it executes.
#[derive(Debug)]
struct DeadlineProbeModule;

#[async_trait]
impl Module for DeadlineProbeModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "probe"
    }
    async fn execute(&self, _inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({
            "global_deadline": ctx.global_deadline,
        }))
    }
}

#[tokio::test]
async fn test_global_deadline_set_by_context_creation() {
    // Spec (A-D-201): when executor.global_timeout > 0, BuiltinContextCreation
    // MUST populate ctx.global_deadline with now + global_timeout (fractional
    // seconds since UNIX epoch — same unit BuiltinExecute reads at line 500).
    let mut config = apcore::config::Config::default();
    config.set("sys_modules.enabled", json!(false));
    config.set("executor.global_timeout", json!(60_000_u64));
    let client = APCore::with_config(config);
    client
        .register("probe.deadline", Box::new(DeadlineProbeModule))
        .unwrap();

    let now_before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    let result = client
        .call("probe.deadline", json!({}), None, None)
        .await
        .expect("call should succeed");

    let deadline = result["global_deadline"]
        .as_f64()
        .expect("global_deadline should be set as a number");
    // Should be ~60s in the future from the time we measured.
    assert!(
        deadline >= now_before + 50.0 && deadline <= now_before + 70.0,
        "deadline {deadline} should be ~60s past now {now_before}"
    );
}

#[tokio::test]
async fn test_global_deadline_unset_when_timeout_zero() {
    // executor.global_timeout = 0 means "no global deadline" — leave it None.
    let mut config = apcore::config::Config::default();
    config.set("sys_modules.enabled", json!(false));
    config.set("executor.global_timeout", json!(0_u64));
    let client = APCore::with_config(config);
    client
        .register("probe.deadline", Box::new(DeadlineProbeModule))
        .unwrap();

    let result = client
        .call("probe.deadline", json!({}), None, None)
        .await
        .expect("call should succeed");
    assert!(
        result["global_deadline"].is_null(),
        "global_deadline must be null when global_timeout=0, got {result:?}"
    );
}

#[tokio::test]
async fn test_apcore_register_and_call() {
    let client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();

    let result = client
        .call("math.add", json!({"a": 10, "b": 5}), None, None)
        .await
        .unwrap();

    assert_eq!(result["result"], 15);
}

#[tokio::test]
async fn test_apcore_call_missing_module() {
    let client = APCore::new();
    let err = client
        .call("nonexistent", json!({}), None, None)
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

#[tokio::test]
async fn test_apcore_middleware_before_and_after() {
    let client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();
    client.use_middleware(Box::new(PrefixMiddleware)).unwrap();

    let result = client
        .call("math.add", json!({"a": 1, "b": 2}), None, None)
        .await
        .unwrap();

    // after() should have added _suffixed
    assert_eq!(result["_suffixed"], true);
    // result should still have the computation
    assert_eq!(result["result"], 3);
}

#[tokio::test]
async fn test_apcore_remove_middleware() {
    let client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();
    client.use_middleware(Box::new(PrefixMiddleware)).unwrap();

    // Remove the middleware
    let removed = client.remove("prefix");
    assert!(removed);

    // Call without middleware — no _suffixed field
    let result = client
        .call("math.add", json!({"a": 1, "b": 2}), None, None)
        .await
        .unwrap();

    assert!(result.get("_suffixed").is_none());
    assert_eq!(result["result"], 3);
}

#[tokio::test]
async fn test_apcore_list_modules() {
    let client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();

    let modules = client.list_modules(None, None);
    assert!(modules.contains(&"math.add".to_string()));
}

#[tokio::test]
async fn test_apcore_describe_module() {
    let client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();

    // describe() returns the cross-SDK Markdown envelope (0.27), not the bare
    // one-line description.
    let desc = client.describe("math.add").expect("module is registered");
    assert!(desc.starts_with("# math.add"), "got: {desc}");
    assert!(desc.contains("Add two numbers"), "got: {desc}");
}

#[tokio::test]
async fn test_apcore_registry_accessor() {
    // Disable auto-registration of sys_modules so the registry only holds
    // what this test registers explicitly.
    let mut config = apcore::config::Config::default();
    config.set("sys_modules.enabled", json!(false));
    let client = APCore::with_config(config);
    client.register("math.add", Box::new(AddModule)).unwrap();

    assert!(client.registry().has("math.add"));
    assert_eq!(client.registry().count(), 1);
}

#[tokio::test]
async fn test_apcore_with_components() {
    use apcore::config::Config;
    use apcore::registry::registry::Registry;

    let registry = Registry::new();
    // Verify with_components builds a working client from registry + config
    let config = Config::default();
    let client = APCore::with_components(registry, config);
    client.register("math.add", Box::new(AddModule)).unwrap();

    let result = client
        .call("math.add", json!({"a": 3, "b": 7}), None, None)
        .await
        .unwrap();
    assert_eq!(result["result"], 10);
}

#[tokio::test]
async fn test_apcore_disable_enable() {
    // Enable sys_modules (opt-in; default is false per spec — sync finding
    // A-D-11) and sys_modules.events so the control modules get registered.
    let mut config = apcore::config::Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let client = APCore::with_config(config);
    // Unique module id: the disabled-set is a process-global ToggleState (sync
    // finding A-D-12), so a shared id could leak into other tests in this
    // binary while this test transiently disables it.
    let module_id = "math.add.toggle_test";
    client.register(module_id, Box::new(AddModule)).unwrap();

    // Disable through the pipeline.
    let result = client.disable(module_id, Some("test")).await;
    assert!(result.is_ok(), "disable should succeed: {result:?}");

    // Next call should fail with ModuleDisabled.
    let call_err = client
        .call(module_id, json!({"a": 1, "b": 2}), None, None)
        .await
        .expect_err("call on disabled module should fail");
    assert_eq!(call_err.code, ErrorCode::ModuleDisabled);

    // Re-enable and call should succeed again.
    let result = client.enable(module_id, Some("test")).await;
    assert!(result.is_ok(), "enable should succeed: {result:?}");

    let ok = client
        .call(module_id, json!({"a": 1, "b": 2}), None, None)
        .await
        .expect("call should succeed after re-enable");
    assert_eq!(ok["result"], 3);
}

#[tokio::test]
async fn test_apcore_disable_nonexistent_module() {
    // System modules are opt-in (default false; sync finding A-D-11).
    let mut config = apcore::config::Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let client = APCore::with_config(config);

    let err = client
        .disable("nonexistent.module", None)
        .await
        .expect_err("disable on nonexistent should fail");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

#[tokio::test]
async fn test_apcore_middleware_chaining() {
    let client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();

    // Verify middleware methods are truly chainable via Result<&mut Self>
    client
        .use_middleware(Box::new(PrefixMiddleware))
        .unwrap()
        .use_middleware(Box::new(TagMiddleware))
        .unwrap();

    // Verify both middleware were applied
    let result = client
        .call("math.add", json!({"a": 1, "b": 2}), None, None)
        .await
        .unwrap();
    assert!(
        result.get("_suffixed").is_some(),
        "PrefixMiddleware after() should add _suffixed"
    );
    assert!(
        result.get("_tagged").is_some(),
        "TagMiddleware after() should add _tagged"
    );
}

#[tokio::test]
async fn test_apcore_list_modules_with_tags() {
    // Disable auto-registration so sys_modules don't add unexpected
    // `system.*` modules to the list.
    let mut config = apcore::config::Config::default();
    config.set("sys_modules.enabled", json!(false));
    let client = APCore::with_config(config);

    // Verify list_modules accepts &[&str] for tags
    let modules = client.list_modules(Some(&["math"]), None);
    assert!(modules.is_empty());

    let modules = client.list_modules(None, Some("system."));
    assert!(modules.is_empty());
}

// Tests for A-001 / A-002 — call_with_trace() and describe_pipeline() on Executor.

#[tokio::test]
async fn test_call_with_trace_returns_output_and_trace() {
    use apcore::pipeline::PipelineTrace;

    let client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();

    let (output, trace): (serde_json::Value, PipelineTrace) = client
        .executor()
        .call_with_trace("math.add", json!({"a": 3, "b": 4}), None, None, None)
        .await
        .unwrap();

    // Output should contain the correct result.
    assert_eq!(output["result"], 7);

    // Trace must contain at least one recorded step.
    assert!(
        !trace.steps.is_empty(),
        "PipelineTrace should have at least one step; got zero"
    );

    // Trace should reference the correct module.
    assert_eq!(trace.module_id, "math.add");

    // Pipeline must have completed successfully.
    assert!(
        trace.success,
        "PipelineTrace.success should be true after a successful call"
    );
}

#[tokio::test]
async fn test_describe_pipeline_returns_strategy_info() {
    use apcore::pipeline::StrategyInfo;

    let client = APCore::new();
    let info: StrategyInfo = client.executor().describe_pipeline();

    // The default strategy has a non-empty name.
    assert!(
        !info.name.is_empty(),
        "StrategyInfo.name should not be empty"
    );

    // There must be at least one step in a non-trivial strategy.
    assert!(
        info.step_count > 0,
        "StrategyInfo.step_count should be > 0; got {}",
        info.step_count
    );

    // step_names length must match step_count.
    assert_eq!(
        info.step_names.len(),
        info.step_count,
        "step_names.len() should equal step_count"
    );

    // Description should be non-empty.
    assert!(
        !info.description.is_empty(),
        "StrategyInfo.description should not be empty"
    );
}

// Regression for sync finding A-002 — Executor.validate() must accept an
// optional context parameter per PROTOCOL_SPEC §12.2 line 6405. Aligns Rust
// signature with apcore-python and apcore-typescript.
#[tokio::test]
async fn test_validate_accepts_optional_context() {
    use apcore::context::{Context, Identity};

    let client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();

    // Path 1: validate with no context — anonymous external context synthesized internally.
    let r1 = client
        .validate("math.add", &json!({"a": 1, "b": 2}), None)
        .await
        .unwrap();
    assert!(
        r1.valid,
        "validate(.., None) should pass for a valid module"
    );

    // Path 2: validate with an explicit context — call_chain checks see real caller state.
    let identity = Identity::new(
        "test_caller".to_string(),
        "user".to_string(),
        vec!["tester".to_string()],
        HashMap::default(),
    );
    let ctx: Context<serde_json::Value> = Context::new(identity);
    let r2 = client
        .validate("math.add", &json!({"a": 1, "b": 2}), Some(&ctx))
        .await
        .unwrap();
    assert!(
        r2.valid,
        "validate(.., Some(ctx)) should pass for a valid module"
    );
    assert_eq!(
        r1.checks.len(),
        r2.checks.len(),
        "context shape should not change the number of preflight checks executed"
    );
}

// Regression for sync finding A-D-010 — Executor::validate() is a NON-THROWING
// preflight: a malformed module_id MUST surface as a failed `module_id` check
// in the returned PreflightResult, not bubble up as a `ModuleError`. Aligns
// Rust with apcore-python and apcore-typescript.
#[tokio::test]
async fn test_validate_returns_preflight_failure_for_invalid_module_id() {
    let client = APCore::new();
    let result = client
        .validate("INVALID_UPPERCASE_ID", &json!({}), None)
        .await
        .expect("validate must NOT throw on malformed module_id (sync A-D-010)");
    assert!(
        !result.valid,
        "PreflightResult.valid must be false for a malformed module_id"
    );
    assert!(
        result
            .checks
            .iter()
            .any(|c| c.check == "module_id" && !c.passed),
        "checks must include a failed `module_id` entry, got {:?}",
        result.checks
    );
}
