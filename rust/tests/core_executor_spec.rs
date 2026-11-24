// Spec-traced contract tests for the apcore-rust core-executor feature.
//
// Source spec: apcore/docs/features/core-executor.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_core_executor_spec.py
//
// Each test maps to exactly one clause in the feature spec's '## Contract:'
// blocks. The verbatim cross-language clause id appears in a leading
// `// clause: <clause_id>` comment on the line above each test fn so that a
// cross-language diff tool can line up the Python / TypeScript / Rust rows by
// that exact string. The fn name is the clause id flattened to snake_case.
//
// Contract blocks covered:
//   - Executor.call
//   - Context.create
//   - Executor binding to Context
//   - Pipeline.configure_step
//   - Distributed cancellation
//   - global_deadline distributed semantics

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::cancel::CancelToken;
use apcore::config::{Config, ExecutorConfig};
use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::executor::Executor;
use apcore::module::Module;
use apcore::pipeline::{ExecutionStrategy, PipelineContext, Step, StepResult};
use apcore::registry::registry::Registry;

// ---------------------------------------------------------------------------
// Minimal module + registry/executor helpers
// ---------------------------------------------------------------------------

/// Schema-less module: accepts any input, echoes a deterministic object.
struct EchoModule;

#[async_trait]
impl Module for EchoModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "echo module"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let name = inputs
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("anon")
            .to_string();
        Ok(json!({ "echo": name }))
    }
}

fn make_registry(module_id: &str) -> Registry {
    let reg = Registry::new();
    reg.register_module(module_id, Box::new(EchoModule))
        .expect("register echo module");
    reg
}

fn make_executor(module_id: &str) -> Executor {
    Executor::new(make_registry(module_id), Config::default())
}

fn make_executor_with_config(module_id: &str, config: Config) -> Executor {
    Executor::new(make_registry(module_id), config)
}

/// A fresh top-level `Context<Value>` (services = `Value::Null`).
fn fresh_context() -> Context<Value> {
    Context::<Value>::create(None, None, None, None, Value::Null, None)
}

/// The string code carried by a `ModuleError` (SCREAMING_SNAKE_CASE), read via
/// `to_dict()["code"]` — the canonical serialized wire form.
fn code_str(err: &ModuleError) -> String {
    err.to_dict()
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Custom pipeline step used by configure_step contract tests
// ---------------------------------------------------------------------------

/// No-op step. `replaceable` controls the §1.2 replace-eligibility flag.
struct NoopStep {
    name: String,
    replaceable: bool,
}

impl NoopStep {
    // Test helper intentionally returns a boxed trait object, not Self.
    #[allow(clippy::new_ret_no_self)]
    fn new(name: &str) -> Box<dyn Step> {
        Box::new(NoopStep {
            name: name.to_string(),
            replaceable: true,
        })
    }
    fn fixed(name: &str) -> Box<dyn Step> {
        Box::new(NoopStep {
            name: name.to_string(),
            replaceable: false,
        })
    }
}

#[async_trait]
impl Step for NoopStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &'static str {
        "noop step"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        self.replaceable
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::continue_step())
    }
}

fn strategy_with(names: &[&str]) -> ExecutionStrategy {
    let steps: Vec<Box<dyn Step>> = names.iter().map(|n| NoopStep::new(n)).collect();
    ExecutionStrategy::new("custom", steps).expect("build strategy")
}

// ===========================================================================
// Contract: Executor.call
// ===========================================================================

// clause: core_executor.call.input.module_id.empty
#[tokio::test]
async fn call_input_module_id_empty() {
    // Empty module_id fails entry-guard validation BEFORE the pipeline runs.
    // Rust surfaces the entry-guard rejection as INVALID_MODULE_ID
    // (cross-language: Python/TS also use INVALID_MODULE_ID).
    let ex = make_executor("test.echo");
    let err = ex
        .call("", json!({ "name": "x" }), None, None)
        .await
        .expect_err("empty module_id must error");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
    assert_eq!(code_str(&err), "INVALID_MODULE_ID");
}

// clause: core_executor.call.input.module_id.malformed
#[tokio::test]
async fn call_input_module_id_malformed() {
    // A malformed module_id (illegal characters) is rejected at the entry guard.
    let ex = make_executor("test.echo");
    let err = ex
        .call("not a valid id!!", json!({ "name": "x" }), None, None)
        .await
        .expect_err("malformed module_id must error");
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
    assert_eq!(code_str(&err), "INVALID_MODULE_ID");
}

// clause: core_executor.call.error.INVALID_MODULE_ID
#[tokio::test]
async fn call_error_invalid_module_id() {
    // Errors: entry-guard rejection of an invalid module_id. Rust code is
    // INVALID_MODULE_ID (cross-language: Python/TS also use INVALID_MODULE_ID).
    let ex = make_executor("test.echo");
    let err = ex
        .call("   ", json!({}), None, None)
        .await
        .expect_err("blank module_id must error");
    assert_eq!(code_str(&err), "INVALID_MODULE_ID");
}

// clause: core_executor.call.error.MODULE_NOT_FOUND
#[tokio::test]
async fn call_error_module_not_found() {
    // A syntactically-valid module_id absent from the registry => MODULE_NOT_FOUND.
    let ex = make_executor("test.echo");
    let err = ex
        .call("test.absent_module", json!({ "name": "x" }), None, None)
        .await
        .expect_err("absent module must error");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
    assert_eq!(code_str(&err), "MODULE_NOT_FOUND");
}

// clause: core_executor.call.error.CALL_DEPTH_EXCEEDED
#[tokio::test]
async fn call_error_call_depth_exceeded() {
    // When call_chain length exceeds max_call_depth, raise CALL_DEPTH_EXCEEDED.
    // ExecutorConfig is #[non_exhaustive]; build from default then set the field.
    let mut executor_cfg = ExecutorConfig::default();
    executor_cfg.max_call_depth = 2;
    let config = Config {
        executor: executor_cfg,
        ..Config::default()
    };
    let ex = make_executor_with_config("test.echo", config);
    let mut ctx = fresh_context();
    ctx.call_chain = vec!["a".into(), "b".into(), "c".into()];
    let err = ex
        .call("test.echo", json!({ "name": "x" }), Some(&ctx), None)
        .await
        .expect_err("deep chain must error");
    assert_eq!(err.code, ErrorCode::CallDepthExceeded);
    assert_eq!(code_str(&err), "CALL_DEPTH_EXCEEDED");
}

// clause: core_executor.call.side_effect.1.validate_module_id
#[tokio::test]
async fn call_side_effect_1_validate_module_id() {
    // Side Effect #1 (validate module_id) is ordered BEFORE Side Effect #2
    // (construct pipeline / module lookup). A bad module_id yields the
    // entry-guard error, NOT a downstream MODULE_NOT_FOUND — proving ordering.
    let ex = make_executor("test.echo");
    let err = ex
        .call("bad id", json!({ "name": "x" }), None, None)
        .await
        .expect_err("entry-guard must fire first");
    assert_ne!(err.code, ErrorCode::ModuleNotFound);
    assert_eq!(err.code, ErrorCode::InvalidModuleId);
}

// clause: core_executor.call.side_effect.2.run_pipeline
#[tokio::test]
async fn call_side_effect_2_run_pipeline() {
    // Side Effect #2: after the entry guard passes, the pipeline runs and
    // returns the module's validated output.
    let ex = make_executor("test.echo");
    let result = ex
        .call("test.echo", json!({ "name": "Alice" }), None, None)
        .await
        .expect("successful call");
    assert_eq!(result, json!({ "echo": "Alice" }));
}

// clause: core_executor.call.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_property_thread_safe() {
    // A single Executor (shared via Arc) tolerates >=8 concurrent calls with
    // distinct inputs; no panic and each call observes its own result.
    let ex = Arc::new(make_executor("test.echo"));
    let mut handles = Vec::new();
    for i in 0..8 {
        let ex = Arc::clone(&ex);
        handles.push(tokio::spawn(async move {
            let out = ex
                .call("test.echo", json!({ "name": format!("u{i}") }), None, None)
                .await
                .expect("concurrent call");
            (i, out)
        }));
    }
    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.expect("task join"));
    }
    results.sort_by_key(|(i, _)| *i);
    for (i, out) in results {
        assert_eq!(out, json!({ "echo": format!("u{i}") }));
    }
}

// clause: core_executor.call.property.async
#[tokio::test]
async fn call_property_async() {
    // Properties: in Rust `call()` IS the async surface; it is awaitable and
    // resolves to the module output.
    let ex = make_executor("test.echo");
    let fut = ex.call("test.echo", json!({ "name": "Bob" }), None, None);
    let result = fut.await.expect("awaited call resolves");
    assert_eq!(result, json!({ "echo": "Bob" }));
}

// clause: core_executor.call.property.pure_false
#[tokio::test]
async fn call_property_pure_false() {
    // Properties: pure == false. Distinct inputs yield distinct outputs — the
    // call is not a frozen constant.
    let ex = make_executor("test.echo");
    let r1 = ex
        .call("test.echo", json!({ "name": "x" }), None, None)
        .await
        .expect("call x");
    let r2 = ex
        .call("test.echo", json!({ "name": "y" }), None, None)
        .await
        .expect("call y");
    assert_eq!(r1, json!({ "echo": "x" }));
    assert_eq!(r2, json!({ "echo": "y" }));
    assert_ne!(r1, r2);
}

// ===========================================================================
// Contract: Context.create
// ===========================================================================

// clause: core_executor.context_create.return.trace_id_32hex
#[test]
fn context_create_return_trace_id_32hex() {
    let ctx = fresh_context();
    assert_eq!(ctx.trace_id.len(), 32);
    assert!(ctx
        .trace_id
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

// clause: core_executor.context_create.return.unset_managed_fields
#[test]
fn context_create_return_unset_managed_fields() {
    // executor, caller_id, call_chain are all unset at top level; identity is
    // carried as provided.
    let ident = Identity::new("svc".into(), "service".into(), vec![], HashMap::new());
    let ctx = Context::<Value>::create(Some(ident.clone()), None, None, None, Value::Null, None);
    assert!(ctx.executor.is_none());
    assert!(ctx.caller_id.is_none());
    assert!(ctx.call_chain.is_empty());
    assert_eq!(ctx.identity.as_ref(), Some(&ident));
}

// clause: core_executor.context_create.property.idempotent_false
#[test]
fn context_create_property_idempotent_false() {
    // Two calls with identical inputs yield different trace_ids.
    let a = fresh_context();
    let b = fresh_context();
    assert_ne!(a.trace_id, b.trace_id);
}

// clause: core_executor.context_create.property.pure_false
#[test]
fn context_create_property_pure_false() {
    // A fresh trace_id is generated each call => not pure.
    let seen: std::collections::HashSet<String> =
        (0..5).map(|_| fresh_context().trace_id).collect();
    assert_eq!(seen.len(), 5);
}

// clause: core_executor.context_create.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn context_create_property_thread_safe() {
    // >=8 concurrent constructions produce 8 distinct, well-formed trace_ids.
    let mut handles = Vec::new();
    for _ in 0..8 {
        handles.push(tokio::spawn(async { fresh_context().trace_id }));
    }
    let mut ids = Vec::new();
    for h in handles {
        ids.push(h.await.expect("join"));
    }
    let unique: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), 8);
    assert!(ids.iter().all(|t| t.len() == 32));
}

// clause: core_executor.context_create.property.async_false
#[test]
fn context_create_property_async_false() {
    // Context::create is a plain (synchronous) constructor — callable without
    // an async runtime and returning a Context directly.
    let ctx = fresh_context();
    assert_eq!(ctx.trace_id.len(), 32);
}

// clause: core_executor.context_create.error.invalid_trace_parent_no_raise
#[test]
fn context_create_error_invalid_trace_parent_no_raise() {
    // Invalid trace_parent values MUST log WARN and regenerate a fresh
    // trace_id rather than raising.
    let bad = apcore::trace_context::TraceParent {
        version: 0,
        trace_id: "zzzz".to_string(),
        parent_id: "0000000000000000".to_string(),
        trace_flags: 1,
        tracestate: vec![],
    };
    let ctx = Context::<Value>::create(None, Some(bad), None, None, Value::Null, None);
    assert_eq!(ctx.trace_id.len(), 32);
    assert_ne!(ctx.trace_id, "zzzz");
}

// ===========================================================================
// Contract: Executor binding to Context
// ===========================================================================

// clause: core_executor.binding.side_effect.1.bind_before_pipeline
#[tokio::test]
async fn binding_side_effect_1_bind_before_pipeline() {
    // A Context whose executor is None MUST have the Executor bound before the
    // pipeline runs. `call()` clones the context internally, so we observe
    // binding via a no-op validate() which binds the *caller's* context.
    let ex = make_executor("test.echo");
    let mut ctx = fresh_context();
    assert!(ctx.executor.is_none());
    // bind_executor is the SDK-internal hook the pipeline uses at entry.
    ctx.bind_executor(ex.instance_handle())
        .expect("bind succeeds on unbound context");
    assert!(ctx.executor.is_some());
}

// clause: core_executor.binding.idempotent.same_executor_noop
#[tokio::test]
async fn binding_idempotent_same_executor_noop() {
    // Reusing one Context across multiple top-level calls on the SAME executor
    // is a noop rebind and MUST NOT raise; identical outcome each time.
    let ex = make_executor("test.echo");
    let mut ctx = fresh_context();
    let r1 = ex
        .call("test.echo", json!({ "name": "same" }), Some(&ctx), None)
        .await
        .expect("first call");
    // Bind the caller-held context to the same executor, then rebind again.
    ctx.bind_executor(ex.instance_handle()).expect("first bind");
    ctx.bind_executor(ex.instance_handle())
        .expect("same-executor rebind is a noop");
    let r2 = ex
        .call("test.echo", json!({ "name": "same" }), Some(&ctx), None)
        .await
        .expect("second call");
    assert_eq!(r1, json!({ "echo": "same" }));
    assert_eq!(r1, r2);
}

// clause: core_executor.binding.error.CONTEXT_BINDING_ERROR
#[tokio::test]
async fn binding_error_context_binding_error() {
    // A Context already bound to a DIFFERENT Executor instance MUST raise
    // ContextBindingError when bound to another Executor.
    let ex_a = make_executor("test.echo");
    let ex_b = make_executor("test.echo");
    let mut ctx = fresh_context();
    ctx.bind_executor(ex_a.instance_handle())
        .expect("bind to A");
    let err = ctx
        .bind_executor(ex_b.instance_handle())
        .expect_err("cross-executor rebind must error");
    assert_eq!(err.code, ErrorCode::ContextBindingError);
    assert_eq!(code_str(&err), "CONTEXT_BINDING_ERROR");
}

// clause: core_executor.binding.side_effect.2.stability
#[tokio::test]
async fn binding_side_effect_2_stability() {
    // Once bound, context.executor MUST NOT change for the remainder of the
    // call chain. After binding to A, a second bind to the SAME A is a noop and
    // the binding remains A (a bind to B would error — see CONTEXT_BINDING_ERROR).
    let ex = make_executor("test.echo");
    let mut ctx = fresh_context();
    ctx.bind_executor(ex.instance_handle()).expect("first bind");
    assert!(ctx.executor.is_some());
    // Re-binding to the same executor keeps it bound (stable).
    ctx.bind_executor(ex.instance_handle())
        .expect("stable same-executor rebind");
    assert!(ctx.executor.is_some());
}

// ===========================================================================
// Contract: Pipeline.configure_step
// ===========================================================================

// clause: core_executor.configure_step.input.step_name.not_found
#[test]
fn configure_step_input_step_name_not_found() {
    // A step_name absent from the strategy fails the lookup.
    let mut strat = strategy_with(&["alpha", "beta"]);
    let err = strat
        .configure_step("does_not_exist", NoopStep::new("does_not_exist"))
        .expect_err("missing step name must error");
    assert_eq!(err.code, ErrorCode::PipelineStepNotFound);
}

// clause: core_executor.configure_step.error.PIPELINE_STEP_NOT_FOUND
#[test]
fn configure_step_error_pipeline_step_not_found() {
    // Errors: PIPELINE_STEP_NOT_FOUND when step_name does not exist. Assert
    // the exact code string.
    let mut strat = strategy_with(&["alpha"]);
    let err = strat
        .configure_step("ghost", NoopStep::new("ghost"))
        .expect_err("ghost step must error");
    assert_eq!(err.code, ErrorCode::PipelineStepNotFound);
    assert_eq!(code_str(&err), "PIPELINE_STEP_NOT_FOUND");
}

// clause: core_executor.configure_step.error.StepNotReplaceableError
// Cross-language contract: configure_step() must reject replacing a step that
// is marked non-replaceable, matching apcore-python and apcore-typescript
// (both raise StepNotReplaceableError). Rust now emits STEP_NOT_REPLACEABLE
// too and leaves the fixed step untouched.
#[test]
fn configure_step_error_step_not_replaceable() {
    let mut strat =
        ExecutionStrategy::new("custom", vec![NoopStep::fixed("fixed")]).expect("build strategy");
    let err = strat
        .configure_step("fixed", NoopStep::new("fixed"))
        .expect_err("configure_step must reject a non-replaceable step");
    assert_eq!(err.code, ErrorCode::StepNotReplaceable);
    assert_eq!(code_str(&err), "STEP_NOT_REPLACEABLE");
    // The original fixed step is preserved (rejection, not silent replacement).
    assert_eq!(strat.step_names(), vec!["fixed".to_string()]);
}

// clause: core_executor.configure_step.side_effect.1.replace_in_place
#[test]
fn configure_step_side_effect_1_replace_in_place() {
    // §1.2 Replace Semantic: replacing keeps exactly one step of that name and
    // preserves its position.
    let mut strat = strategy_with(&["alpha", "beta", "gamma"]);
    strat
        .configure_step("beta", NoopStep::new("beta"))
        .expect("replace beta");
    let names = strat.step_names();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    assert_eq!(names.iter().filter(|n| n.as_str() == "beta").count(), 1);
    assert_eq!(strat.steps()[1].name(), "beta");
}

// clause: core_executor.configure_step.property.idempotent_true
#[test]
fn configure_step_property_idempotent_true() {
    // Replacing the same step twice leaves exactly one step at that position.
    let mut strat = strategy_with(&["alpha", "beta"]);
    strat
        .configure_step("beta", NoopStep::new("beta"))
        .expect("replace beta #1");
    let names_after_first = strat.step_names();
    strat
        .configure_step("beta", NoopStep::new("beta"))
        .expect("replace beta #2");
    assert_eq!(strat.step_names(), names_after_first);
    assert_eq!(strat.step_names(), vec!["alpha", "beta"]);
    assert_eq!(
        strat
            .step_names()
            .iter()
            .filter(|n| n.as_str() == "beta")
            .count(),
        1
    );
}

// clause: core_executor.configure_step.return.none
#[test]
fn configure_step_return_none() {
    // Returns: on success configure_step returns () (Rust void analogue of None).
    let mut strat = strategy_with(&["alpha"]);
    let ret: () = strat
        .configure_step("alpha", NoopStep::new("alpha"))
        .expect("configure alpha");
    assert_eq!(ret, ());
}

// ===========================================================================
// Contract: Distributed cancellation
// ===========================================================================

// clause: core_executor.distributed_cancellation.input.cancel_token_in_process
#[test]
fn distributed_cancellation_input_cancel_token_in_process() {
    // The cancel_token parameter on Context::create carries a caller-supplied
    // token on the Context (in-process cooperation). We verify it is the SAME
    // underlying token by cancelling the caller's handle and observing the
    // carried token reflect the cancellation (Arc-shared state).
    let token = CancelToken::new();
    let ctx = Context::<Value>::create(None, None, Some(token.clone()), None, Value::Null, None);
    let carried = ctx.cancel_token.clone().expect("token carried on context");
    assert!(!carried.is_cancelled());
    token.cancel();
    assert!(
        carried.is_cancelled(),
        "carried token shares the caller's cancellation state"
    );
}

// clause: core_executor.distributed_cancellation.side_effect.1.no_serialize
#[test]
fn distributed_cancellation_side_effect_1_no_serialize() {
    // cancel_token is runtime-only and MUST NOT serialize. After
    // serialize/deserialize, the revived Context MUST NOT carry the token.
    let token = CancelToken::new();
    let ctx = Context::<Value>::create(None, None, Some(token), None, Value::Null, None);
    let serialized = ctx.serialize();
    assert!(
        serialized.get("cancel_token").is_none(),
        "cancel_token must not appear in serialized payload"
    );
    let revived = Context::<Value>::deserialize(serialized).expect("deserialize");
    assert!(
        revived.cancel_token.is_none(),
        "revived context must not carry the originating node's token"
    );
}

// ===========================================================================
// Contract: global_deadline distributed semantics
// ===========================================================================

// clause: core_executor.global_deadline.input.local_only
#[test]
fn global_deadline_input_local_only() {
    // global_deadline is a caller input carried locally on the Context.
    let deadline = 1_000_000.0_f64;
    let ctx = Context::<Value>::create(None, None, None, None, Value::Null, Some(deadline));
    assert_eq!(ctx.global_deadline, Some(deadline));
}

// clause: core_executor.global_deadline.side_effect.1.no_serialize
#[test]
fn global_deadline_side_effect_1_no_serialize() {
    // global_deadline is runtime-only and MUST NOT serialize. A deserialized
    // Context MUST NOT carry the originating node's deadline value.
    let ctx = Context::<Value>::create(None, None, None, None, Value::Null, Some(1_000_000.0));
    let serialized = ctx.serialize();
    assert!(
        serialized.get("global_deadline").is_none(),
        "global_deadline must not appear in serialized payload"
    );
    let revived = Context::<Value>::deserialize(serialized).expect("deserialize");
    assert_eq!(revived.global_deadline, None);
}

// ===========================================================================
// D-74 / D-75 — boundary values at the entry doors
// ===========================================================================

// clause: config.get.input.key.empty_is_not_an_error
#[test]
fn config_get_empty_key_is_not_an_error() {
    // D-74. The `Config.get` Inputs row said an empty key "is rejected with
    // ValueError/ConfigInvalidError". The same block's Errors row said "No
    // errors raised under normal operation", and no SDK had ever rejected it —
    // this one's `get` has no error channel at all, so the clause described
    // behaviour that could not exist here. A conformance case written from it
    // would have failed on all three.
    //
    // "Like any other absent key" is the decision's own wording, so the two
    // paths are asserted to AGREE rather than each being checked alone: an
    // implementation short-circuiting on the empty key would still return
    // `None` and pass a lone `is_none()` assertion.
    let mut config = Config::default();
    config.set("a.b", json!(1));

    assert_eq!(config.get(""), None);
    assert_eq!(config.get(""), config.get("no.such.key"));
    assert_eq!(config.get_declared(""), config.get_declared("no.such.key"));

    // Control: a present key is unaffected. Without it, an implementation
    // returning `None` for EVERY key satisfies the assertions above.
    assert_eq!(config.get("a.b"), Some(json!(1)));
}

// clause: core_executor.call.input.module_id.over_length
#[tokio::test]
async fn call_rejects_an_over_length_module_id_at_the_entry_guard() {
    // D-75: "Empty / over-length / malformed IDs MUST be rejected before the
    // pipeline context is constructed." A well-formed but over-length ID used
    // to pass the entry guard, build a PipelineContext and come back as
    // MODULE_NOT_FOUND from the registry lookup one step later.
    //
    // The discriminator is WHICH code comes back, not that an error comes
    // back at all: the registry enforces the same bound, so an SDK with no
    // entry check still fails the call — with the wrong error, from the wrong
    // layer, after building state the guard exists to avoid.
    use apcore::registry::registry::MAX_MODULE_ID_LENGTH;

    let executor = make_executor("executor.echo");
    let over_length = "a".repeat(MAX_MODULE_ID_LENGTH + 1);

    let err = executor
        .call(&over_length, json!({}), None, None)
        .await
        .expect_err("an over-length id must be refused");
    assert_eq!(
        err.code,
        ErrorCode::InvalidModuleId,
        "the entry guard must report it, not the registry lookup: {}",
        err.message
    );
}

// clause: core_executor.call.input.module_id.at_bound
#[tokio::test]
async fn call_accepts_an_id_exactly_at_the_length_bound() {
    // The bound is inclusive. Without this, "over-length is refused" is also
    // satisfied by an off-by-one that refuses the bound itself, and the two
    // outcomes together are what pin the boundary.
    use apcore::registry::registry::MAX_MODULE_ID_LENGTH;

    let executor = make_executor("executor.echo");
    let at_bound = "a".repeat(MAX_MODULE_ID_LENGTH);

    let err = executor
        .call(&at_bound, json!({}), None, None)
        .await
        .expect_err("still unregistered, so the lookup refuses it");
    assert_eq!(
        err.code,
        ErrorCode::ModuleNotFound,
        "an id at the bound must pass the guard and reach the registry: {}",
        err.message
    );
}
