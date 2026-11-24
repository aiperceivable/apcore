//! Regression tests for v0.22 executor hardening — D-19/D-20/D-21/D-11.
//!
//! Covers:
//! - A-D-EXEC-001 (D-11): per-module `resources.timeout` annotation honored.
//! - A-D-EXEC-002 (D-21): cancel-token check at CallChainGuard and at
//!   BuiltinExecute (mid-pipeline cancel observation).
//! - A-D-EXEC-003 (D-20): ExecutionCancelled short-circuit bypasses
//!   `on_error` middleware so logging/retry middleware cannot swallow it.
//! - A-D-EXEC-004 (D-19): `call_with_trace` runs `on_error` recovery and
//!   returns `(recovered_value, trace)` on successful recovery.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use apcore::cancel::CancelToken;
use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::middleware::base::Middleware;
use apcore::middleware::{RetryConfig, RetryMiddleware};
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::{ModuleDescriptor, Registry};
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Module that sleeps for `delay_ms` before returning {"ok": true}.
struct SleepModule {
    delay_ms: u64,
}

#[async_trait]
impl Module for SleepModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Sleep for a configured duration"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        Ok(json!({"ok": true}))
    }
}

/// Module that always returns Err so on_error middleware can attempt recovery.
struct AlwaysFailModule;

#[async_trait]
impl Module for AlwaysFailModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Always fails"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Err(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "always fails",
        ))
    }
}

/// Middleware whose `on_error` recovers by returning a fixed value.
#[derive(Debug)]
struct RecoveringMiddleware;

#[async_trait]
impl Middleware for RecoveringMiddleware {
    fn name(&self) -> &'static str {
        "recovering"
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
        Ok(Some(json!({"recovered": true})))
    }
}

/// Middleware whose `on_error` records that it was invoked. Used to assert
/// the D-20 short-circuit prevents `on_error` from running on cancellation.
#[derive(Debug, Clone)]
struct SwallowingMiddleware {
    on_error_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Middleware for SwallowingMiddleware {
    fn name(&self) -> &'static str {
        "swallowing"
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
        self.on_error_calls.fetch_add(1, Ordering::SeqCst);
        // Attempt to swallow cancellation by returning a recovery value —
        // the short-circuit MUST prevent this from succeeding.
        Ok(Some(json!({"swallowed": true})))
    }
}

/// Middleware whose `before()` always fails. Used to drive the
/// MiddlewareChainError path in BuiltinMiddlewareBefore (A-D-01).
#[derive(Debug)]
struct FailingBeforeMiddleware;

#[async_trait]
impl Middleware for FailingBeforeMiddleware {
    fn name(&self) -> &'static str {
        "failing-before"
    }
    fn priority(&self) -> u16 {
        // Lower priority runs after the recovering middleware so the
        // recovering middleware is in `executed` when this one fails.
        10
    }
    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        Err(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "before failed",
        ))
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
        Ok(None)
    }
}

fn ctx_with_token(token: CancelToken) -> Context<Value> {
    // Per Issue #66, `cancel_token` is a first-class `Context::create`
    // parameter; no post-hoc assignment is needed.
    Context::<Value>::create(
        Some(Identity::new(
            "@external".to_string(),
            "external".to_string(),
            vec![],
            HashMap::new(),
        )),
        None,
        Some(token),
        None,
        Value::Null,
        None,
    )
}

/// Register `module` under `module_id` with a descriptor that pins
/// `resources.timeout` in the annotations' `extra` map.
fn register_with_timeout(
    registry: &Registry,
    module_id: &str,
    module: Box<dyn Module>,
    timeout_ms: u64,
) {
    let mut annotations = ModuleAnnotations::default();
    annotations
        .extra
        .insert("resources".to_string(), json!({ "timeout": timeout_ms }));
    let descriptor = ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: module.description().to_string(),
        documentation: None,
        input_schema: module.input_schema(),
        output_schema: module.output_schema(),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(annotations),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    registry.register(module_id, module, descriptor).unwrap();
}

// ---------------------------------------------------------------------------
// A-D-EXEC-001 (D-11): per-module resources.timeout overrides default
// ---------------------------------------------------------------------------

#[tokio::test]
async fn per_module_timeout_overrides_default() {
    // The module sleeps for 200 ms; per-module timeout pinned to 50 ms via
    // annotations.extra["resources"]["timeout"]. The call MUST time out
    // before the module returns, regardless of the executor default
    // timeout (which is 30 000 ms in Config::default()).
    let client = APCore::new();
    register_with_timeout(
        client.registry(),
        "slow.module",
        Box::new(SleepModule { delay_ms: 200 }),
        50,
    );

    let start = std::time::Instant::now();
    let result = client.call("slow.module", json!({}), None, None).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "expected timeout error, got {result:?}");
    let err = result.unwrap_err();
    assert_eq!(err.code, ErrorCode::ModuleTimeout);
    assert_eq!(
        err.retryable,
        Some(false),
        "a timed-out non-idempotent module must not be marked retryable"
    );
    assert!(
        elapsed < Duration::from_millis(180),
        "per-module timeout (50 ms) was not honored; took {elapsed:?}"
    );
}

/// Same as [`SleepModule`], but declares `annotations.idempotent: true`.
struct IdempotentSleepModule {
    delay_ms: u64,
}

#[async_trait]
impl Module for IdempotentSleepModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Idempotent module that sleeps for a configured duration"
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            idempotent: true,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        Ok(json!({"ok": true}))
    }
}

#[tokio::test]
async fn per_module_timeout_is_retryable_when_idempotent() {
    let client = APCore::new();
    register_with_timeout(
        client.registry(),
        "slow.idempotent",
        Box::new(IdempotentSleepModule { delay_ms: 200 }),
        50,
    );

    let err = client
        .call("slow.idempotent", json!({}), None, None)
        .await
        .expect_err("expected timeout error");
    assert_eq!(err.code, ErrorCode::ModuleTimeout);
    assert_eq!(err.retryable, Some(true));
}

/// Register `module` with a raw JSON `resources.timeout` value (allows
/// injecting an invalid negative timeout for A-D-W1).
fn register_with_raw_timeout(
    registry: &Registry,
    module_id: &str,
    module: Box<dyn Module>,
    timeout: &Value,
) {
    let mut annotations = ModuleAnnotations::default();
    annotations
        .extra
        .insert("resources".to_string(), json!({ "timeout": timeout }));
    let descriptor = ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: module.description().to_string(),
        documentation: None,
        input_schema: module.input_schema(),
        output_schema: module.output_schema(),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(annotations),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    registry.register(module_id, module, descriptor).unwrap();
}

// ---------------------------------------------------------------------------
// A-D-01: a recovery middleware registered before a failing before-middleware
// MUST have its on_error invoked. Previously BuiltinMiddlewareBefore passed an
// empty &[] to execute_on_error, so on_error never ran on the recovery mw.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn before_middleware_error_invokes_recovery_on_error() {
    let client = APCore::new();
    client
        .register("ok.module", Box::new(SleepModule { delay_ms: 1 }))
        .unwrap();
    // Recovering middleware has default priority 100 (runs first in `before`);
    // the failing middleware has priority 10 (runs second, then fails).
    client
        .use_middleware(Box::new(RecoveringMiddleware))
        .unwrap();
    client
        .use_middleware(Box::new(FailingBeforeMiddleware))
        .unwrap();

    let result = client.call("ok.module", json!({}), None, None).await;

    assert!(
        result.is_ok(),
        "expected recovery via on_error, got {result:?}"
    );
    assert_eq!(result.unwrap(), json!({"recovered": true}));
}

// ---------------------------------------------------------------------------
// A-D-W1: a negative per-module declared timeout MUST raise
// GENERAL_INVALID_INPUT, not be silently swallowed and fall back to default.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn negative_declared_timeout_is_rejected() {
    let client = APCore::new();
    register_with_raw_timeout(
        client.registry(),
        "neg.timeout.module",
        Box::new(SleepModule { delay_ms: 1 }),
        &json!(-1),
    );

    let result = client
        .call("neg.timeout.module", json!({}), None, None)
        .await;

    assert!(
        result.is_err(),
        "expected negative timeout to be rejected, got {result:?}"
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::GeneralInvalidInput);
}

// ---------------------------------------------------------------------------
// A-D-EXEC-002 (D-21): cancel-token observed at CallChainGuard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_observed_at_call_chain_guard_short_circuits_pipeline() {
    let client = APCore::new();
    client
        .register("slow.module", Box::new(SleepModule { delay_ms: 500 }))
        .unwrap();

    let token = CancelToken::new();
    token.cancel(); // pre-cancelled
    let ctx = ctx_with_token(token);

    let start = std::time::Instant::now();
    let result = client
        .executor()
        .call("slow.module", json!({}), Some(&ctx), None)
        .await;
    let elapsed = start.elapsed();

    assert!(
        result.is_err(),
        "pre-cancelled context must short-circuit; got Ok({:?}) after {:?}",
        result.ok(),
        elapsed
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::ExecutionCancelled);
    assert!(
        elapsed < Duration::from_millis(100),
        "cancel was not observed early; pipeline ran for {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// A-D-EXEC-003 (D-20): cancellation short-circuits on_error middleware
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancellation_does_not_invoke_on_error_middleware() {
    let client = APCore::new();
    let swallowing = SwallowingMiddleware {
        on_error_calls: Arc::new(AtomicUsize::new(0)),
    };
    let on_error_calls = swallowing.on_error_calls.clone();
    client
        .use_middleware(Box::new(swallowing))
        .expect("middleware registration");
    client
        .register("slow.module", Box::new(SleepModule { delay_ms: 500 }))
        .unwrap();

    let token = CancelToken::new();
    token.cancel(); // pre-cancelled — call_chain_guard short-circuits.
    let ctx = ctx_with_token(token);

    let result = client
        .executor()
        .call("slow.module", json!({}), Some(&ctx), None)
        .await;

    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().code,
        ErrorCode::ExecutionCancelled,
        "cancellation must propagate directly, not via on_error recovery"
    );
    assert_eq!(
        on_error_calls.load(Ordering::SeqCst),
        0,
        "on_error middleware MUST NOT run for ExecutionCancelled (D-20)"
    );
}

// ---------------------------------------------------------------------------
// A-D-EXEC-004 (D-19): call_with_trace runs on_error recovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn call_with_trace_runs_on_error_recovery() {
    let client = APCore::new();
    client
        .use_middleware(Box::new(RecoveringMiddleware))
        .expect("middleware registration");
    client
        .register("fail.module", Box::new(AlwaysFailModule))
        .unwrap();

    let (output, trace) = client
        .executor()
        .call_with_trace("fail.module", json!({}), None, None, None)
        .await
        .expect("on_error recovery should succeed");

    assert_eq!(output, json!({"recovered": true}));
    assert_eq!(trace.module_id, "fail.module");
}

struct TransientModule {
    attempts: AtomicUsize,
}

#[async_trait]
impl Module for TransientModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn description(&self) -> &'static str {
        "Fails transiently before succeeding"
    }

    async fn execute(&self, _inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        assert_eq!(ctx.caller_id.as_deref(), Some("orchestrator.test"));
        assert_eq!(ctx.call_chain, ["orchestrator.test", "retry.module"]);
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt <= 3 {
            return Err(
                ModuleError::new(ErrorCode::GeneralInternalError, "temporary").with_retryable(true),
            );
        }
        Ok(json!({"attempt": attempt}))
    }
}

#[tokio::test]
async fn test_executor_retry_preserves_call_identity_and_trace() {
    for traced in [false, true] {
        let client = APCore::new();
        let mut retry = RetryConfig::default();
        retry.base_delay_ms = 0;
        retry.jitter = false;
        client
            .use_middleware(Box::new(RetryMiddleware::new(retry)))
            .unwrap();
        client
            .register(
                "retry.module",
                Box::new(TransientModule {
                    attempts: AtomicUsize::new(0),
                }),
            )
            .unwrap();
        let parent = Context::<Value>::anonymous().child("orchestrator.test");
        let executor = client.executor();
        let output = if traced {
            let (output, trace) = executor
                .call_with_trace("retry.module", json!({}), Some(&parent), None, None)
                .await
                .unwrap();
            assert!(trace.success);
            assert_eq!(
                trace
                    .steps
                    .iter()
                    .filter(|step| step.name == "execute")
                    .count(),
                4
            );
            output
        } else {
            executor
                .call("retry.module", json!({}), Some(&parent), None)
                .await
                .unwrap()
        };
        assert_eq!(output, json!({"attempt": 4}));
    }
}

#[tokio::test]
async fn test_call_with_trace_rejects_invalid_module_id() {
    let client = APCore::new();
    let result = client
        .executor()
        .call_with_trace("INVALID", json!({}), None, None, None)
        .await;
    assert_eq!(result.unwrap_err().code, ErrorCode::InvalidModuleId);
}

#[tokio::test]
async fn execution_closes_the_middleware_chain_that_entered() {
    #[derive(Debug)]
    struct Hooks(
        &'static str,
        Arc<parking_lot::Mutex<Vec<(&'static str, &'static str)>>>,
    );

    #[async_trait]
    impl Middleware for Hooks {
        fn name(&self) -> &str {
            self.0
        }

        async fn before(
            &self,
            _: &str,
            _: Value,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            self.1.lock().push((self.0, "before"));
            Ok(None)
        }

        async fn after(
            &self,
            _: &str,
            _: Value,
            _: Value,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            self.1.lock().push((self.0, "after"));
            Ok(None)
        }

        async fn on_error(
            &self,
            _: &str,
            _: Value,
            _: &ModuleError,
            _: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            self.1.lock().push((self.0, "error"));
            Ok(None)
        }
    }

    for fail in [false, true] {
        let hooks = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let mut client = APCore::new();
        let entered = client
            .use_middleware_handle(Box::new(Hooks("entered", hooks.clone())))
            .unwrap();
        let (started_by_module, release_module) = (started.clone(), release.clone());
        client
            .module(
                "held.module",
                "Wait for test release",
                json!({}),
                json!({}),
                None,
                vec![],
                None,
                None,
                vec![],
                None,
                move |_, _| {
                    let (started, release) = (started_by_module.clone(), release_module.clone());
                    Box::pin(async move {
                        started.notify_one();
                        release.notified().await;
                        if fail {
                            return Err(ModuleError::new(ErrorCode::ModuleExecuteError, "failed"));
                        }
                        Ok(json!({}))
                    })
                },
            )
            .unwrap();
        let client = Arc::new(client);
        let caller = client.clone();
        let running =
            tokio::spawn(async move { caller.call("held.module", json!({}), None, None).await });
        started.notified().await;
        assert!(client.remove_handle(entered));
        client
            .use_middleware_handle(Box::new(Hooks("replacement", hooks.clone())))
            .unwrap();
        release.notify_one();
        assert_eq!(running.await.unwrap().is_err(), fail);
        let exit = if fail { "error" } else { "after" };
        assert_eq!(*hooks.lock(), [("entered", "before"), ("entered", exit)]);
    }
}

// ---------------------------------------------------------------------------
// sync-2026-08-26 A-D-007: stream()'s non-streaming fallback honours the same
// timeout BuiltinExecute applies.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_fallback_honours_per_module_timeout() {
    use futures_util::StreamExt;

    // `stream()` drives run_until_step(.., "execute"), so BuiltinExecute never
    // runs on this path. Its fallback used to await module.execute() bare —
    // no per-module `resources.timeout`, no global-deadline clamp — so a
    // non-streaming slow module hung indefinitely, where apcore-python and
    // apcore-typescript raise MODULE_TIMEOUT (both run the full pipeline in
    // stream Phase 1, so their fallback goes through BuiltinExecute).
    let registry = Arc::new(Registry::new());
    register_with_timeout(
        &registry,
        "slow.module",
        Box::new(SleepModule { delay_ms: 5_000 }),
        50,
    );
    let executor =
        apcore::executor::Executor::new(registry, Arc::new(apcore::config::Config::default()));

    let started = std::time::Instant::now();
    let mut stream = executor.stream("slow.module", json!({}), None, None);
    let first = stream.next().await;
    let elapsed = started.elapsed();

    match first {
        Some(Err(e)) => assert_eq!(
            e.code,
            ErrorCode::ModuleTimeout,
            "expected MODULE_TIMEOUT, got {e:?}"
        ),
        other => panic!("expected a MODULE_TIMEOUT error chunk, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_millis(2_000),
        "the timeout must fire at ~50ms, not wait out the module's 5s sleep (took {elapsed:?})"
    );
}

// ---------------------------------------------------------------------------
// sync-2026-08-26 A-D-006: a pipeline ABORT surfaces from call_with_trace
// exactly as it does from call().
// ---------------------------------------------------------------------------

/// A step that aborts the pipeline, standing in for a replacement gate step
/// installed via the §1.2 Replace Semantic.
struct AbortingStep;

#[async_trait]
impl apcore::pipeline::Step for AbortingStep {
    fn name(&self) -> &str {
        "acl_check"
    }
    fn description(&self) -> &str {
        "test step that aborts"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(
        &self,
        _ctx: &mut apcore::pipeline::PipelineContext,
    ) -> Result<apcore::pipeline::StepResult, ModuleError> {
        Ok(apcore::pipeline::StepResult::abort("denied by test step"))
    }
}

#[tokio::test]
async fn call_with_trace_surfaces_a_pipeline_abort_like_call_does() {
    // The engine signals a step-returned `action: "abort"` as Ok((output,
    // trace)) with trace.success == false rather than as an Err, so every
    // caller carries the obligation to re-check the flag. `call` did;
    // `call_with_trace` handed back Ok for an aborted pipeline while both peer
    // SDKs raised. core-executor.md §Trace Variants (D-19): an error that
    // propagates in call() MUST also propagate in the trace variant.
    let registry = Arc::new(Registry::new());
    register_with_timeout(
        &registry,
        "some.module",
        Box::new(SleepModule { delay_ms: 0 }),
        1_000,
    );

    let mut strategy = apcore::builtin_steps::build_standard_strategy();
    strategy
        .replace("acl_check", Box::new(AbortingStep))
        .expect("acl_check is replaceable");

    let executor = apcore::executor::Executor::with_strategy(
        registry,
        Arc::new(apcore::config::Config::default()),
        strategy,
    );

    let via_call = executor.call("some.module", json!({}), None, None).await;
    let via_trace = executor
        .call_with_trace("some.module", json!({}), None, None, None)
        .await;

    assert!(via_call.is_err(), "call() must surface the abort");
    assert!(
        via_trace.is_err(),
        "call_with_trace() must surface it too — got Ok for an aborted pipeline"
    );
    assert_eq!(
        via_call.unwrap_err().code,
        via_trace.unwrap_err().code,
        "both entry points must report the same error code"
    );
}

// ---------------------------------------------------------------------------
// sync-2026-08-26 A-D-008: validate() gates module introspection on the handle
// the PIPELINE resolved, not on an independent registry lookup.
// ---------------------------------------------------------------------------

/// Records whether `preflight()` was invoked, so the test asserts on the module
/// actually running rather than only on the shape of the returned checks.
struct PreflightSpyModule {
    called: Arc<AtomicUsize>,
}

#[async_trait]
impl Module for PreflightSpyModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "records preflight invocations"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
    fn preflight(&self, _inputs: &Value, _ctx: Option<&Context<Value>>) -> Vec<String> {
        self.called.fetch_add(1, Ordering::SeqCst);
        vec!["would delete /etc/passwd".to_string()]
    }
}

#[tokio::test]
async fn validate_does_not_introspect_a_disabled_module() {
    // `module_lookup` raises ModuleDisabled for a module toggled off via
    // system.control.toggle_feature BEFORE assigning ctx.module, so the
    // pipeline deliberately refuses to resolve it. validate() used to re-query
    // `self.registry.get(module_id)` independently — a different question, and
    // one the registry still answers — so a disabled module had its
    // module-authored preflight() run and its warnings returned to the caller,
    // where apcore-python and apcore-typescript emit no module_preflight check
    // at all (both guard on pipe_ctx.module / pipeCtx.module).
    let called = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(Registry::new());
    register_with_timeout(
        &registry,
        "executor.fs.delete_file",
        Box::new(PreflightSpyModule {
            called: Arc::clone(&called),
        }),
        1_000,
    );

    let toggle_state = Arc::new(apcore::sys_modules::ToggleState::new());
    toggle_state.disable("executor.fs.delete_file");

    let mut executor = apcore::executor::Executor::new(
        Arc::clone(&registry),
        Arc::new(apcore::config::Config::default()),
    );
    executor.set_toggle_state(toggle_state);

    let result = executor
        .validate("executor.fs.delete_file", &json!({}), None)
        .await
        .expect("validate is non-throwing");

    assert_eq!(
        called.load(Ordering::SeqCst),
        0,
        "a module the pipeline refused to resolve must not have preflight() run"
    );
    assert!(
        !result.checks.iter().any(|c| c.check == "module_preflight"),
        "no module_preflight check may be emitted for a disabled module, got {:?}",
        result.checks.iter().map(|c| &c.check).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn validate_still_introspects_an_enabled_module() {
    // The complement: the guard must not over-reach into refusing introspection
    // for a module the pipeline resolved normally.
    let called = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(Registry::new());
    register_with_timeout(
        &registry,
        "executor.fs.read_file",
        Box::new(PreflightSpyModule {
            called: Arc::clone(&called),
        }),
        1_000,
    );
    let executor =
        apcore::executor::Executor::new(registry, Arc::new(apcore::config::Config::default()));

    let result = executor
        .validate("executor.fs.read_file", &json!({}), None)
        .await
        .expect("validate is non-throwing");

    assert_eq!(called.load(Ordering::SeqCst), 1, "preflight() must run");
    assert!(
        result.checks.iter().any(|c| c.check == "module_preflight"),
        "the module_preflight check must be emitted"
    );
}
