//! Spec-traced contract tests for the apcore Middleware System (Rust SDK).
//!
//! Source spec: apcore/docs/features/middleware-system.md
//! Contracts:
//!   - Middleware::before
//!   - Middleware::after
//!   - Middleware::on_error
//!   - Middleware::detect_async   (no standalone symbol in this SDK -> ignored)
//!
//! This file MIRRORS the canonical Python suite
//! (`apcore-python/tests/test_middleware_system_spec.py`). Each test carries the
//! verbatim clause id (`middleware_system.<method>.<kind>.<detail>`) in a
//! leading `// clause:` comment so cross-language diffs line up row-for-row.
//!
//! Tests only — production source is never modified.
//!
//! Cross-language notes (Rust-specific):
//!   - Rust `Middleware` hooks are statically typed via `#[async_trait]`.
//!     `module_id`, `inputs`, and `context` are mandatory function parameters
//!     enforced AT COMPILE TIME, so the Python "missing-positional-argument ->
//!     TypeError" runtime failure path does not exist. The `input.*.required`
//!     clauses are mirrored by exercising the real typed signature (all
//!     arguments supplied) and asserting it resolves — the contract that the
//!     parameter is required is satisfied structurally by the type system.
//!   - `execute_before` returns `(Value, Vec<usize>)` (executed indices), not a
//!     `Vec<Middleware>`. The "tracks executed" intent is mirrored on indices.
//!   - A failing `before()` is wrapped as `ModuleError` with code
//!     `MiddlewareChainError` (wire string `MIDDLEWARE_CHAIN_ERROR`); the
//!     original error is recoverable via `unwrap_middleware_chain_error()`.
//!   - `on_error` MUST NOT raise out of the chain: a handler that returns `Err`
//!     is logged and iteration continues (mirrors Python swallow-and-continue).

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};

use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::middleware::{
    AfterAdapter, BeforeAdapter, Middleware, MiddlewareManager, OnErrorOutcome,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A fresh execution context for a single pipeline pass.
fn ctx() -> Context<Value> {
    Context::<Value>::anonymous()
}

/// The SCREAMING_SNAKE wire string Rust emits for an `ErrorCode` variant.
fn wire_code(code: ErrorCode) -> String {
    match serde_json::to_value(code).expect("ErrorCode serializes") {
        Value::String(s) => s,
        other => panic!("ErrorCode did not serialize to a string: {other:?}"),
    }
}

/// Middleware that records ordered observations into a shared sink.
#[derive(Debug)]
struct Recording {
    label: String,
    sink: Arc<Mutex<Vec<String>>>,
    prio: u16,
}

impl Recording {
    fn new(label: &str, sink: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            label: label.to_string(),
            sink,
            prio: 100,
        }
    }
}

#[async_trait]
impl Middleware for Recording {
    fn name(&self) -> &str {
        &self.label
    }

    fn priority(&self) -> u16 {
        self.prio
    }

    async fn before(
        &self,
        _module_id: &str,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.sink.lock().push(format!("before:{}", self.label));
        Ok(None)
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        _output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.sink.lock().push(format!("after:{}", self.label));
        Ok(None)
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.sink.lock().push(format!("on_error:{}", self.label));
        Ok(None)
    }
}

/// A no-op base middleware (mirrors Python's bare `Middleware()`), with all
/// three hooks returning `Ok(None)`.
#[derive(Debug)]
struct NoopMiddleware {
    label: String,
}

impl NoopMiddleware {
    fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
        }
    }
}

#[async_trait]
impl Middleware for NoopMiddleware {
    fn name(&self) -> &str {
        &self.label
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

// ===========================================================================
// Contract: Middleware.before
// ===========================================================================

// clause: middleware_system.before.input.module_id.required
#[tokio::test]
async fn middleware_system_before_input_module_id_required() {
    // Rust statically requires `module_id: &str`. There is no runtime
    // missing-argument failure path; the required contract is enforced by the
    // type system. Exercise the real signature with module_id supplied.
    let mw = NoopMiddleware::new("base");
    let result = mw.before("mod.id", json!({}), &ctx()).await;
    assert!(
        result.is_ok(),
        "before() with module_id present resolves Ok"
    );
    assert_eq!(result.unwrap(), None, "no-op before returns None");
}

// clause: middleware_system.before.input.inputs.required
#[tokio::test]
async fn middleware_system_before_input_inputs_required() {
    // `inputs: Value` is a mandatory typed parameter (compile-time enforced).
    let mw = NoopMiddleware::new("base");
    let result = mw.before("mod.id", json!({"a": 1}), &ctx()).await;
    assert!(result.is_ok(), "before() with inputs present resolves Ok");
}

// clause: middleware_system.before.input.context.required
#[tokio::test]
async fn middleware_system_before_input_context_required() {
    // `ctx: &Context<Value>` is a mandatory typed parameter (compile-time).
    let mw = NoopMiddleware::new("base");
    let c = ctx();
    let result = mw.before("mod.id", json!({}), &c).await;
    assert!(result.is_ok(), "before() with context present resolves Ok");
}

// clause: middleware_system.before.returns.none_passthrough
#[tokio::test]
async fn middleware_system_before_returns_none_passthrough() {
    // Returning None from before() leaves the inputs unchanged through the
    // manager's execute_before pass.
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(BeforeAdapter::new("noop", |_m, _i, _c| async {
        Ok(None)
    })))
    .unwrap();
    let (final_inputs, executed) = mgr
        .execute_before("mod.id", json!({"a": 1}), &ctx())
        .await
        .expect("execute_before resolves Ok");
    assert_eq!(
        final_inputs,
        json!({"a": 1}),
        "inputs unchanged on passthrough"
    );
    assert_eq!(executed.len(), 1, "one middleware executed");
}

// clause: middleware_system.before.returns.dict_replaces_inputs
#[tokio::test]
async fn middleware_system_before_returns_dict_replaces_inputs() {
    // Returning a dict from before() replaces the inputs seen by downstream
    // middleware and by the module body.
    let seen_downstream: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&seen_downstream);

    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(BeforeAdapter::new(
        "replace",
        |_m, _i, _c| async { Ok(Some(json!({"replaced": true}))) },
    )))
    .unwrap();
    mgr.add(Box::new(BeforeAdapter::new(
        "observe",
        move |_m, i: Value, _c| {
            let seen = Arc::clone(&seen);
            async move {
                *seen.lock() = Some(i);
                Ok(None)
            }
        },
    )))
    .unwrap();

    let (final_inputs, _) = mgr
        .execute_before("mod.id", json!({"orig": 1}), &ctx())
        .await
        .expect("execute_before resolves Ok");
    assert_eq!(
        final_inputs,
        json!({"replaced": true}),
        "final inputs replaced"
    );
    // downstream middleware observed the replacement, not the original.
    assert_eq!(
        *seen_downstream.lock(),
        Some(json!({"replaced": true})),
        "downstream observed the replacement"
    );
}

// clause: middleware_system.before.error.MIDDLEWARE_CHAIN_ERROR
#[tokio::test]
async fn middleware_system_before_error_middleware_chain_error() {
    // A before() that raises is wrapped by the manager in a
    // MiddlewareChainError with code MIDDLEWARE_CHAIN_ERROR, carrying the
    // original cause (recoverable via unwrap_middleware_chain_error()).
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(BeforeAdapter::new("raiser", |_m, _i, _c| async {
        Err(ModuleError::new(
            ErrorCode::ModuleExecuteError,
            "before exploded",
        ))
    })))
    .unwrap();

    let err = mgr
        .execute_before("mod.id", json!({}), &ctx())
        .await
        .expect_err("failing before() must produce an Err");
    assert_eq!(
        err.code,
        ErrorCode::MiddlewareChainError,
        "wrapped error must be MiddlewareChainError"
    );
    assert_eq!(
        wire_code(err.code),
        "MIDDLEWARE_CHAIN_ERROR",
        "wire string must match exactly"
    );
    // The original typed error is preserved for targeted recovery.
    let inner = err
        .unwrap_middleware_chain_error()
        .expect("original error preserved in details");
    assert_eq!(inner.code, ErrorCode::ModuleExecuteError);
    assert_eq!(inner.message, "before exploded");
}

// clause: middleware_system.before.error.aborts_pipeline_tracks_executed
#[tokio::test]
async fn middleware_system_before_error_aborts_pipeline_tracks_executed() {
    // When a middleware's before() raises, downstream before() hooks are
    // skipped and the executed list carries exactly the middlewares whose
    // before() had been entered (up to and including the raiser).
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(Recording::new("first", Arc::clone(&sink))))
        .unwrap();
    let sink_for_raiser = Arc::clone(&sink);
    mgr.add(Box::new(BeforeAdapter::new("raiser", move |_m, _i, _c| {
        let s = Arc::clone(&sink_for_raiser);
        async move {
            s.lock().push("before:raiser".to_string());
            Err(ModuleError::new(ErrorCode::ModuleExecuteError, "stop"))
        }
    })))
    .unwrap();
    mgr.add(Box::new(Recording::new("downstream", Arc::clone(&sink))))
        .unwrap();

    let err = mgr
        .execute_before("mod.id", json!({}), &ctx())
        .await
        .expect_err("failing before() must produce an Err");
    assert_eq!(err.code, ErrorCode::MiddlewareChainError);

    let observed = sink.lock().clone();
    // downstream before() never ran.
    assert!(
        !observed.contains(&"before:downstream".to_string()),
        "downstream before() must be skipped"
    );
    assert_eq!(
        observed,
        vec!["before:first".to_string(), "before:raiser".to_string()],
        "only first + raiser entered before()"
    );
}

// clause: middleware_system.before.property.async
#[tokio::test]
async fn middleware_system_before_property_async() {
    // Rust hooks are async by construction (#[async_trait]); the manager awaits
    // a coroutine return and applies its result.
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(BeforeAdapter::new(
        "async_before",
        |_m, _i, _c| async {
            tokio::task::yield_now().await;
            Ok(Some(json!({"awaited": true})))
        },
    )))
    .unwrap();
    let (final_inputs, executed) = mgr
        .execute_before("mod.id", json!({"orig": 1}), &ctx())
        .await
        .expect("async execute_before resolves Ok");
    assert_eq!(final_inputs, json!({"awaited": true}));
    assert_eq!(executed.len(), 1);
}

// clause: middleware_system.before.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn middleware_system_before_property_thread_safe() {
    // N concurrent execute_before passes with distinct inputs each return their
    // own distinct result with no cross-talk. Concurrent add() during the
    // passes does not corrupt state (snapshot pattern).
    let n: usize = 12;
    let mgr = Arc::new(MiddlewareManager::new());
    mgr.add(Box::new(BeforeAdapter::new(
        "tag",
        |_m, i: Value, _c| async move {
            let idx = i.get("idx").cloned().unwrap_or(Value::Null);
            Ok(Some(json!({ "echo": idx })))
        },
    )))
    .unwrap();

    let mut handles = Vec::new();
    for idx in 0..n {
        let mgr = Arc::clone(&mgr);
        handles.push(tokio::spawn(async move {
            tokio::task::yield_now().await;
            let (final_inputs, _) = mgr
                .execute_before("mod.id", json!({ "idx": idx }), &ctx())
                .await
                .expect("concurrent execute_before resolves Ok");
            final_inputs
        }));
    }
    // Concurrent churn: add no-op middlewares while passes run.
    let churn_mgr = Arc::clone(&mgr);
    let churn = tokio::spawn(async move {
        for _ in 0..n {
            tokio::task::yield_now().await;
            churn_mgr
                .add(Box::new(BeforeAdapter::new("churn", |_m, _i, _c| async {
                    Ok(None)
                })))
                .unwrap();
        }
    });

    let mut echoes: Vec<u64> = Vec::new();
    for h in handles {
        let final_inputs = h.await.expect("task joins without panic");
        let echo = final_inputs
            .get("echo")
            .and_then(Value::as_u64)
            .expect("echo present");
        echoes.push(echo);
    }
    churn.await.expect("churn task joins without panic");

    echoes.sort_unstable();
    assert_eq!(
        echoes,
        (0..n as u64).collect::<Vec<u64>>(),
        "each pass returned its own distinct result, no cross-talk"
    );
}

// clause: middleware_system.before.property.pure
#[tokio::test]
async fn middleware_system_before_property_pure() {
    // Contract declares pure: false. A before() may mutate context.data; the
    // mutation is observable via the public context after execute_before.
    let c = ctx();
    assert!(!c.data.read().contains_key("ext.spec.before_ran"));

    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(BeforeAdapter::new(
        "mutate",
        |_m, _i, c: Context<Value>| async move {
            c.data
                .write()
                .insert("ext.spec.before_ran".to_string(), json!(true));
            Ok(None)
        },
    )))
    .unwrap();
    mgr.execute_before("mod.id", json!({}), &c)
        .await
        .expect("execute_before resolves Ok");

    assert_eq!(
        c.data.read().get("ext.spec.before_ran"),
        Some(&json!(true)),
        "context mutation is observable (pure: false)"
    );
}

// clause: middleware_system.before.side_effect.1.registration_order
#[tokio::test]
async fn middleware_system_before_side_effect_1_registration_order() {
    // before() hooks run in registration order (MW1 then MW2 then MW3).
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(Recording::new("1", Arc::clone(&sink))))
        .unwrap();
    mgr.add(Box::new(Recording::new("2", Arc::clone(&sink))))
        .unwrap();
    mgr.add(Box::new(Recording::new("3", Arc::clone(&sink))))
        .unwrap();

    mgr.execute_before("mod.id", json!({}), &ctx())
        .await
        .expect("execute_before resolves Ok");
    assert_eq!(
        *sink.lock(),
        vec![
            "before:1".to_string(),
            "before:2".to_string(),
            "before:3".to_string()
        ]
    );
}

// ===========================================================================
// Contract: Middleware.after
// ===========================================================================

// clause: middleware_system.after.input.module_id.required
#[tokio::test]
async fn middleware_system_after_input_module_id_required() {
    // `module_id: &str` is a mandatory typed parameter (compile-time enforced).
    let mw = NoopMiddleware::new("base");
    let result = mw.after("mod.id", json!({}), json!({}), &ctx()).await;
    assert!(result.is_ok(), "after() with module_id present resolves Ok");
}

// clause: middleware_system.after.input.inputs.required
#[tokio::test]
async fn middleware_system_after_input_inputs_required() {
    let mw = NoopMiddleware::new("base");
    let result = mw.after("mod.id", json!({"a": 1}), json!({}), &ctx()).await;
    assert!(result.is_ok(), "after() with inputs present resolves Ok");
}

// clause: middleware_system.after.input.output.required
#[tokio::test]
async fn middleware_system_after_input_output_required() {
    let mw = NoopMiddleware::new("base");
    let result = mw.after("mod.id", json!({}), json!({"v": 1}), &ctx()).await;
    assert!(result.is_ok(), "after() with output present resolves Ok");
}

// clause: middleware_system.after.input.context.required
#[tokio::test]
async fn middleware_system_after_input_context_required() {
    let mw = NoopMiddleware::new("base");
    let c = ctx();
    let result = mw.after("mod.id", json!({}), json!({}), &c).await;
    assert!(result.is_ok(), "after() with context present resolves Ok");
}

// clause: middleware_system.after.returns.none_passthrough
#[tokio::test]
async fn middleware_system_after_returns_none_passthrough() {
    // Returning None from after() leaves the output unchanged.
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(AfterAdapter::new(
        "noop",
        |_m, _i, _o, _c| async { Ok(None) },
    )))
    .unwrap();
    let final_output = mgr
        .execute_after("mod.id", json!({}), json!({"v": 1}), &ctx())
        .await
        .expect("execute_after resolves Ok");
    assert_eq!(
        final_output,
        json!({"v": 1}),
        "output unchanged on passthrough"
    );
}

// clause: middleware_system.after.returns.dict_replaces_output
#[tokio::test]
async fn middleware_system_after_returns_dict_replaces_output() {
    // Returning a dict from after() replaces the output passed onward.
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(AfterAdapter::new(
        "wrap",
        |_m, _i, o: Value, _c| async move { Ok(Some(json!({ "wrapped": o }))) },
    )))
    .unwrap();
    let final_output = mgr
        .execute_after("mod.id", json!({}), json!({"v": 1}), &ctx())
        .await
        .expect("execute_after resolves Ok");
    assert_eq!(final_output, json!({"wrapped": {"v": 1}}));
}

// clause: middleware_system.after.error.fail_fast_propagates
#[tokio::test]
async fn middleware_system_after_error_fail_fast_propagates() {
    // An after() that raises propagates immediately (fail-fast): the chain
    // stops at the first error and remaining hooks do not run.
    // after() runs in REVERSE order: register the raiser LAST so it runs FIRST,
    // and the recorder (registered first) never runs.
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(Recording::new("never", Arc::clone(&sink))))
        .unwrap();
    mgr.add(Box::new(AfterAdapter::new(
        "raiser",
        |_m, _i, _o, _c| async {
            Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "after exploded",
            ))
        },
    )))
    .unwrap();

    let err = mgr
        .execute_after("mod.id", json!({}), json!({"v": 1}), &ctx())
        .await
        .expect_err("failing after() must propagate as Err (fail-fast)");
    assert_eq!(err.message, "after exploded");
    assert!(
        !sink.lock().contains(&"after:never".to_string()),
        "remaining after() hook must NOT run after fail-fast"
    );
}

// clause: middleware_system.after.property.async
#[tokio::test]
async fn middleware_system_after_property_async() {
    // Rust after() hooks are async by construction; the manager awaits them.
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(AfterAdapter::new(
        "async_after",
        |_m, _i, o: Value, _c| async move {
            tokio::task::yield_now().await;
            Ok(Some(json!({ "awaited": o })))
        },
    )))
    .unwrap();
    let final_output = mgr
        .execute_after("mod.id", json!({}), json!({"v": 1}), &ctx())
        .await
        .expect("async execute_after resolves Ok");
    assert_eq!(final_output, json!({"awaited": {"v": 1}}));
}

// clause: middleware_system.after.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn middleware_system_after_property_thread_safe() {
    // N concurrent execute_after passes with distinct outputs each return their
    // own result without cross-talk or panic.
    let n: usize = 12;
    let mgr = Arc::new(MiddlewareManager::new());
    mgr.add(Box::new(AfterAdapter::new(
        "echo",
        |_m, _i, o: Value, _c| async move {
            let idx = o.get("idx").cloned().unwrap_or(Value::Null);
            Ok(Some(json!({ "echo": idx })))
        },
    )))
    .unwrap();

    let mut handles = Vec::new();
    for idx in 0..n {
        let mgr = Arc::clone(&mgr);
        handles.push(tokio::spawn(async move {
            tokio::task::yield_now().await;
            mgr.execute_after("mod.id", json!({}), json!({ "idx": idx }), &ctx())
                .await
                .expect("concurrent execute_after resolves Ok")
        }));
    }

    let mut echoes: Vec<u64> = Vec::new();
    for h in handles {
        let out = h.await.expect("task joins without panic");
        echoes.push(
            out.get("echo")
                .and_then(Value::as_u64)
                .expect("echo present"),
        );
    }
    echoes.sort_unstable();
    assert_eq!(echoes, (0..n as u64).collect::<Vec<u64>>(), "no cross-talk");
}

// clause: middleware_system.after.side_effect.1.reverse_order
#[tokio::test]
async fn middleware_system_after_side_effect_1_reverse_order() {
    // after() hooks run in REVERSE registration order (MW3 then MW2 then MW1).
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(Recording::new("1", Arc::clone(&sink))))
        .unwrap();
    mgr.add(Box::new(Recording::new("2", Arc::clone(&sink))))
        .unwrap();
    mgr.add(Box::new(Recording::new("3", Arc::clone(&sink))))
        .unwrap();

    mgr.execute_after("mod.id", json!({}), json!({"v": 1}), &ctx())
        .await
        .expect("execute_after resolves Ok");
    assert_eq!(
        *sink.lock(),
        vec![
            "after:3".to_string(),
            "after:2".to_string(),
            "after:1".to_string()
        ]
    );
}

// ===========================================================================
// Contract: Middleware.on_error
// ===========================================================================

// clause: middleware_system.on_error.input.module_id.required
#[tokio::test]
async fn middleware_system_on_error_input_module_id_required() {
    // `module_id: &str` is a mandatory typed parameter (compile-time enforced).
    let mw = NoopMiddleware::new("base");
    let err = ModuleError::new(ErrorCode::GeneralInvalidInput, "x");
    let result = mw.on_error("mod.id", json!({}), &err, &ctx()).await;
    assert!(
        result.is_ok(),
        "on_error() with module_id present resolves Ok"
    );
}

// clause: middleware_system.on_error.input.inputs.required
#[tokio::test]
async fn middleware_system_on_error_input_inputs_required() {
    let mw = NoopMiddleware::new("base");
    let err = ModuleError::new(ErrorCode::GeneralInvalidInput, "x");
    let result = mw.on_error("mod.id", json!({"a": 1}), &err, &ctx()).await;
    assert!(result.is_ok(), "on_error() with inputs present resolves Ok");
}

// clause: middleware_system.on_error.input.error.required
#[tokio::test]
async fn middleware_system_on_error_input_error_required() {
    // `error: &ModuleError` is a mandatory typed parameter (compile-time).
    let mw = NoopMiddleware::new("base");
    let err = ModuleError::new(ErrorCode::GeneralInvalidInput, "x");
    let result = mw.on_error("mod.id", json!({}), &err, &ctx()).await;
    assert!(result.is_ok(), "on_error() with error present resolves Ok");
}

// clause: middleware_system.on_error.input.context.required
#[tokio::test]
async fn middleware_system_on_error_input_context_required() {
    let mw = NoopMiddleware::new("base");
    let err = ModuleError::new(ErrorCode::GeneralInvalidInput, "x");
    let c = ctx();
    let result = mw.on_error("mod.id", json!({}), &err, &c).await;
    assert!(
        result.is_ok(),
        "on_error() with context present resolves Ok"
    );
}

/// Recovery middleware that records its label and returns a configurable value.
#[derive(Debug)]
struct Recover {
    label: String,
    value: Option<Value>,
    sink: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Middleware for Recover {
    fn name(&self) -> &str {
        &self.label
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
        Ok(None)
    }

    async fn on_error(
        &self,
        _module_id: &str,
        _inputs: Value,
        _error: &ModuleError,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.sink.lock().push(self.label.clone());
        Ok(self.value.clone())
    }
}

// clause: middleware_system.on_error.returns.first_recovery_wins
#[tokio::test]
async fn middleware_system_on_error_returns_first_recovery_wins() {
    // on_error() runs in reverse over executed middlewares; the first handler
    // to return a non-None dict provides recovery and short-circuits the rest.
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mgr = MiddlewareManager::new();
    // Registration order: A, B, C. Reverse run order: C, B, A.
    mgr.add(Box::new(Recover {
        label: "A".to_string(),
        value: Some(json!({"by": "A"})),
        sink: Arc::clone(&sink),
    }))
    .unwrap();
    mgr.add(Box::new(Recover {
        label: "B".to_string(),
        value: Some(json!({"by": "B"})),
        sink: Arc::clone(&sink),
    }))
    .unwrap();
    mgr.add(Box::new(Recover {
        label: "C".to_string(),
        value: None,
        sink: Arc::clone(&sink),
    }))
    .unwrap();

    let err = ModuleError::new(ErrorCode::ModuleExecuteError, "x");
    let result = mgr
        .execute_on_error("mod.id", json!({}), &err, &ctx(), &[0, 1, 2])
        .await;
    // C ran (returned None), B recovered, A was short-circuited.
    assert_eq!(result, Some(json!({"by": "B"})));
    assert_eq!(
        *sink.lock(),
        vec!["C".to_string(), "B".to_string()],
        "C then B ran; A short-circuited"
    );
    assert!(!sink.lock().contains(&"A".to_string()));
}

// clause: middleware_system.on_error.returns.none_passthrough
#[tokio::test]
async fn middleware_system_on_error_returns_none_passthrough() {
    // When every on_error returns None, the manager returns None (the original
    // error keeps propagating).
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(NoopMiddleware::new("a"))).unwrap();
    mgr.add(Box::new(NoopMiddleware::new("b"))).unwrap();
    let err = ModuleError::new(ErrorCode::ModuleExecuteError, "x");
    let result = mgr
        .execute_on_error("mod.id", json!({}), &err, &ctx(), &[0, 1])
        .await;
    assert_eq!(result, None, "no recovery -> None");
}

// clause: middleware_system.on_error.error.handler_must_not_raise
#[tokio::test]
async fn middleware_system_on_error_error_handler_must_not_raise() {
    // on_error MUST NOT raise out of the chain: a handler that returns Err is
    // logged and iteration continues with the next handler, which can recover.
    #[derive(Debug)]
    struct Boom {
        sink: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl Middleware for Boom {
        fn name(&self) -> &'static str {
            "boom"
        }
        async fn before(
            &self,
            _m: &str,
            _i: Value,
            _c: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
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
            self.sink.lock().push("boom".to_string());
            Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "handler blew up",
            ))
        }
    }

    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mgr = MiddlewareManager::new();
    // Reverse run order: boom first (returns Err, swallowed), then recover.
    mgr.add(Box::new(Recover {
        label: "recover".to_string(),
        value: Some(json!({"recovered": true})),
        sink: Arc::clone(&sink),
    }))
    .unwrap();
    mgr.add(Box::new(Boom {
        sink: Arc::clone(&sink),
    }))
    .unwrap();

    let err = ModuleError::new(ErrorCode::ModuleExecuteError, "x");
    let result = mgr
        .execute_on_error("mod.id", json!({}), &err, &ctx(), &[0, 1])
        .await;
    assert_eq!(
        result,
        Some(json!({"recovered": true})),
        "handler error swallowed; next handler recovers"
    );
    assert_eq!(
        *sink.lock(),
        vec!["boom".to_string(), "recover".to_string()]
    );
}

// clause: middleware_system.on_error.property.async
#[tokio::test]
async fn middleware_system_on_error_property_async() {
    // Rust on_error hooks are async by construction; the manager awaits them.
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(Recover {
        label: "async_recover".to_string(),
        value: Some(json!({"recovered_async": true})),
        sink: Arc::clone(&sink),
    }))
    .unwrap();
    let err = ModuleError::new(ErrorCode::ModuleExecuteError, "x");
    let result = mgr
        .execute_on_error("mod.id", json!({}), &err, &ctx(), &[0])
        .await;
    assert_eq!(result, Some(json!({"recovered_async": true})));
}

// clause: middleware_system.on_error.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn middleware_system_on_error_property_thread_safe() {
    // N concurrent on_error recovery passes with distinct inputs each yield
    // their own recovery output without cross-talk or panic.
    #[derive(Debug)]
    struct EchoRecover;
    #[async_trait]
    impl Middleware for EchoRecover {
        fn name(&self) -> &'static str {
            "echo_recover"
        }
        async fn before(
            &self,
            _m: &str,
            _i: Value,
            _c: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
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
            inputs: Value,
            _e: &ModuleError,
            _c: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            tokio::task::yield_now().await;
            let idx = inputs.get("idx").cloned().unwrap_or(Value::Null);
            Ok(Some(json!({ "echo": idx })))
        }
    }

    let n: usize = 10;
    let mgr = Arc::new(MiddlewareManager::new());
    mgr.add(Box::new(EchoRecover)).unwrap();

    let mut handles = Vec::new();
    for idx in 0..n {
        let mgr = Arc::clone(&mgr);
        handles.push(tokio::spawn(async move {
            let err = ModuleError::new(ErrorCode::ModuleExecuteError, "x");
            mgr.execute_on_error("mod.id", json!({ "idx": idx }), &err, &ctx(), &[0])
                .await
        }));
    }

    let mut echoes: Vec<u64> = Vec::new();
    for h in handles {
        let result = h.await.expect("task joins without panic");
        let out = result.expect("recovery present");
        echoes.push(
            out.get("echo")
                .and_then(Value::as_u64)
                .expect("echo present"),
        );
    }
    echoes.sort_unstable();
    assert_eq!(echoes, (0..n as u64).collect::<Vec<u64>>(), "no cross-talk");
}

// clause: middleware_system.on_error.side_effect.1.reverse_over_executed
#[tokio::test]
async fn middleware_system_on_error_side_effect_1_reverse_over_executed() {
    // on_error() runs in reverse order over ONLY the executed middlewares.
    // If only MW1 and MW2 executed before the failure, on_error runs MW2 then
    // MW1 and MW3 is never touched.
    let sink: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mgr = MiddlewareManager::new();
    mgr.add(Box::new(Recording::new("1", Arc::clone(&sink))))
        .unwrap();
    mgr.add(Box::new(Recording::new("2", Arc::clone(&sink))))
        .unwrap();
    mgr.add(Box::new(Recording::new("3", Arc::clone(&sink))))
        .unwrap();

    let err = ModuleError::new(ErrorCode::ModuleExecuteError, "x");
    // Simulate failure after mw1, mw2 ran their before() (mw3 did not).
    mgr.execute_on_error("mod.id", json!({}), &err, &ctx(), &[0, 1])
        .await;
    let observed = sink.lock().clone();
    assert_eq!(
        observed,
        vec!["on_error:2".to_string(), "on_error:1".to_string()]
    );
    assert!(!observed.contains(&"on_error:3".to_string()));
}

// ===========================================================================
// Contract: Middleware.detect_async
// ---------------------------------------------------------------------------
// The spec declares `## Contract: Middleware.detect_async` (a pure, idempotent,
// thread-safe sync predicate returning true for async handlers). The Rust SDK
// does NOT expose a `Middleware::detect_async` symbol: async handlers are
// statically typed via `#[async_trait]` and the compiler ENFORCES that callers
// `.await` the returned Future — there is no runtime "shape" detection and none
// is possible (spec §1.5: "Async handlers are statically typed via async_trait;
// no runtime detection is needed or possible"). Each clause is therefore a
// missing-symbol contract gap, marked #[ignore] so the crate still compiles and
// the cross-language row lines up rather than producing a compile failure.
// ===========================================================================

// clause: middleware_system.detect_async.input.handler.required
#[tokio::test]
#[ignore = "middleware_system.detect_async.input.handler.required: missing symbol Middleware::detect_async (contract gap) -- Rust uses async_trait static typing, no runtime detect_async"]
async fn middleware_system_detect_async_input_handler_required() {
    unreachable!("ignored contract gap: detect_async not present in Rust SDK");
}

// clause: middleware_system.detect_async.returns.bool
#[tokio::test]
#[ignore = "middleware_system.detect_async.returns.bool: missing symbol Middleware::detect_async (contract gap) -- Rust uses async_trait static typing, no runtime detect_async"]
async fn middleware_system_detect_async_returns_bool() {
    unreachable!("ignored contract gap: detect_async not present in Rust SDK");
}

// clause: middleware_system.detect_async.property.pure
#[tokio::test]
#[ignore = "middleware_system.detect_async.property.pure: missing symbol Middleware::detect_async (contract gap) -- Rust uses async_trait static typing, no runtime detect_async"]
async fn middleware_system_detect_async_property_pure() {
    unreachable!("ignored contract gap: detect_async not present in Rust SDK");
}

// clause: middleware_system.detect_async.property.idempotent
#[tokio::test]
#[ignore = "middleware_system.detect_async.property.idempotent: missing symbol Middleware::detect_async (contract gap) -- Rust uses async_trait static typing, no runtime detect_async"]
async fn middleware_system_detect_async_property_idempotent() {
    unreachable!("ignored contract gap: detect_async not present in Rust SDK");
}

// Keep OnErrorOutcome referenced so its import documents the recovery-vs-retry
// surface even though these tests exercise the simpler `execute_on_error` path.
#[allow(dead_code)]
fn _outcome_type_is_in_scope(o: OnErrorOutcome) -> OnErrorOutcome {
    o
}
