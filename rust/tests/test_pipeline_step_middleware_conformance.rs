//! Drive `pipeline_step_middleware.json` — the StepMiddleware lifecycle
//! (Issue #33 §2.2) as three implementations must agree on it.
//!
//! This is the fixture-LOADING driver. `tests/test_pipeline_step_middleware.rs`
//! is a hand-written unit test of the same area; a hand copy cannot notice when
//! the canonical fixture gains a case, which is why both exist and why only
//! this one counts as conformance coverage.
//!
//! The fixture's own `driver_contract` names the two ways this file is easy to
//! write vacuously, and both are avoided here:
//!
//!  * Asserting the SET of invoked middlewares instead of the ORDERED list. A
//!    set passes against a straight-through implementation, which is precisely
//!    the defect the onion model exists to prevent.
//!  * Testing recovery with a single recovering handler. With one recovery,
//!    first-wins and last-wins produce the same value, so the case must have
//!    two handlers that would BOTH recover.
//!  * Asserting that one key is ABSENT from `state.outputs` instead of pinning
//!    the exact key set. `!contains("second")` also passes against an engine
//!    that lost `first` and against one that never populated the map at all.
//!//!  * Pinning `state.outputs` at only ONE of the engine's two `step_outputs`
//!    snapshot sites. The recovery branch snapshots separately, and
//!    `state_outputs_excludes_the_current_step_in_every_hook` never reaches it
//!    because it recovers from nothing —
//!    `after_step_fires_after_a_recovered_step` carries that half.
//!
//! The fixture states every error expectation as a WIRE CODE
//! (`wrapper_error_code` / `original_error_code`), never a class name, and its
//! `wrapper_is_load_bearing` contract records that removing the
//! `MiddlewareChainError` wrapping left every driver GREEN before this file
//! asserted it. Each error case below therefore checks `code.wire_str()`
//! against the fixture and then unwraps the wrapper to prove the original
//! error survived.

use std::sync::Arc;

use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::pipeline::{
    ExecutionStrategy, PipelineContext, PipelineEngine, PipelineState, Step, StepMiddleware,
    StepResult,
};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Fixture access
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("pipeline_step_middleware.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("pipeline_step_middleware.json parses")
}

fn case_by_id(fx: &Value, id: &str) -> Value {
    fx["test_cases"]
        .as_array()
        .expect("test_cases is an array")
        .iter()
        .find(|tc| tc["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("pipeline_step_middleware.json no longer carries case `{id}`"))
        .clone()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("expected a JSON array of strings")
        .iter()
        .map(|s| s.as_str().expect("array entry is a string").to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

struct OkStep(String);
struct FailingStep(String, ErrorCode, String);

#[async_trait]
impl Step for OkStep {
    fn name(&self) -> &str {
        &self.0
    }
    fn description(&self) -> &str {
        "conformance stand-in"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::continue_step())
    }
}

#[async_trait]
impl Step for FailingStep {
    fn name(&self) -> &str {
        &self.0
    }
    fn description(&self) -> &str {
        "conformance stand-in that fails"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Err(ModuleError::new(self.1, self.2.clone()))
    }
}

/// The code this driver's failing step actually raises.
///
/// The fixture's `original_error_code` is `DEMO_FAILURE`: a *domain* code a
/// module would define, not a framework one. In apcore-python and
/// apcore-typescript an error carries a free-form `code` string, so their
/// drivers can raise `DEMO_FAILURE` literally and assert it round-trips. In
/// Rust `ModuleError::code` is the closed `ErrorCode` enum (`src/errors.rs`),
/// so `DEMO_FAILURE` cannot be constructed or round-tripped at all — asserting
/// it here would mean asserting a string this driver itself invented, which is
/// exactly the vacuous-assertion habit the fixture's `driver_contract` warns
/// about.
///
/// What the case is actually about — the original error is preserved and
/// recoverable through the `PIPELINE_STEP_ERROR` wrapper rather than being
/// replaced by it — IS assertable, and is asserted below via
/// `unwrap_pipeline_step_error()` against both this code and the fixture's own
/// `step_raises.message`.
const STEP_FAILURE_CODE: ErrorCode = ErrorCode::ModuleExecuteError;

fn step_failure_message(tc: &Value) -> String {
    tc["input"]["step_raises"]["message"]
        .as_str()
        .expect("step_raises.message")
        .to_string()
}

/// A step that records its own execution into the shared log.
///
/// `before_step_failure_is_terminal` requires a driver to prove the recovery
/// was DISCARDED by observing that the FOLLOWING step did not run — an
/// implementation that honours the recovery and then happens to fail later
/// satisfies a raise-only assertion while the bypass is live. Asserting on a
/// step body therefore needs the body to leave a trace, which `OkStep` does
/// not. It also carries `ignore_errors`, because that same rule says a step's
/// `ignore_errors` MUST NOT swallow a `MiddlewareChainError`.
struct TrackingStep {
    name: String,
    log: Log,
    ignore_errors: bool,
}

#[async_trait]
impl Step for TrackingStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "conformance stand-in that records whether its body ran"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    fn ignore_errors(&self) -> bool {
        self.ignore_errors
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        self.log.lock().push(format!("step:{}", self.name));
        Ok(StepResult::continue_step())
    }
}

fn failing_step(name: &str, tc: &Value) -> Box<dyn Step> {
    Box::new(FailingStep(
        name.to_string(),
        STEP_FAILURE_CODE,
        step_failure_message(tc),
    ))
}

// ---------------------------------------------------------------------------
// Middlewares
// ---------------------------------------------------------------------------

type Log = Arc<Mutex<Vec<String>>>;

/// Records every hook it is given, labelled with the middleware's own name, so
/// the assertion can be on the ORDER and not merely on membership.
struct Recorder {
    label: String,
    log: Log,
    /// `Some(v)` makes `on_step_error` offer `v` as a recovery result.
    recovery: Option<Value>,
    /// `Some(step)` restricts logging to that step. A case with a PRECEDING
    /// step gets hooks for it too, and its expectations are ordered lists about
    /// the case's OWN step — mixing the two in one log would compare a
    /// four-entry list against a two-entry one and report the wrong defect.
    only_step: Option<String>,
    /// `Some(step)` makes `before_step` fail on that step ONLY, to drive the
    /// executed-only rule. Scoped to one step rather than all of them because
    /// the bypass this pins is a middleware that trips on the *guarded* step
    /// and behaves normally afterwards: failing on every step would stop the
    /// pipeline for the wrong reason and mask whether the recovery was honoured.
    before_fails_on: Option<String>,
}

#[async_trait]
impl StepMiddleware for Recorder {
    async fn before_step(
        &self,
        step_name: &str,
        _state: &PipelineState<'_>,
    ) -> Result<(), ModuleError> {
        if self.logs(step_name) {
            self.log.lock().push(format!("before:{}", self.label));
        }
        if self.before_fails_on.as_deref() == Some(step_name) {
            return Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                format!("{} before_step exploded", self.label),
            ));
        }
        Ok(())
    }

    async fn after_step(
        &self,
        step_name: &str,
        _state: &PipelineState<'_>,
        _result: &Value,
    ) -> Result<(), ModuleError> {
        if self.logs(step_name) {
            self.log.lock().push(format!("after:{}", self.label));
        }
        Ok(())
    }

    async fn on_step_error(
        &self,
        step_name: &str,
        _state: &PipelineState<'_>,
        _error: &ModuleError,
    ) -> Result<Option<Value>, ModuleError> {
        if self.logs(step_name) {
            self.log.lock().push(format!("error:{}", self.label));
        }
        Ok(self.recovery.clone())
    }
}

impl Recorder {
    /// Whether hooks on `step_name` are recorded. Unscoped recorders log
    /// everything; a scoped one logs only its own step.
    fn logs(&self, step_name: &str) -> bool {
        self.only_step.as_deref().is_none_or(|s| s == step_name)
    }
}

fn recorder(label: &str, log: &Log) -> Arc<Recorder> {
    Arc::new(Recorder {
        label: label.to_string(),
        log: Arc::clone(log),
        recovery: None,
        only_step: None,
        before_fails_on: None,
    })
}

fn ctx() -> PipelineContext {
    PipelineContext::new(
        "executor.conformance.step_mw",
        serde_json::json!({}),
        Context::<Value>::anonymous(),
        "conformance",
    )
}

/// Entries in `log` matching `prefix`, in the order they were recorded.
fn hooks(log: &Log, prefix: &str) -> Vec<String> {
    log.lock()
        .iter()
        .filter_map(|e| e.strip_prefix(prefix).map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_before_after_invocation_order() {
    let tc = case_by_id(&fixture(), "before_after_invocation_order");
    let step_name = tc["input"]["step"].as_str().expect("step");
    let log: Log = Arc::default();

    let mut strategy =
        ExecutionStrategy::new("conformance", vec![Box::new(OkStep(step_name.to_string()))])
            .unwrap();
    for label in strings(&tc["input"]["register_order"]) {
        strategy.add_step_middleware(recorder(&label, &log) as Arc<dyn StepMiddleware>);
    }

    PipelineEngine::run(&strategy, &mut ctx())
        .await
        .expect("the step succeeds");

    assert_eq!(
        hooks(&log, "before:"),
        strings(&tc["expected"]["before_step_order"]),
        "before_step must run in REGISTRATION order"
    );
    assert_eq!(
        hooks(&log, "after:"),
        strings(&tc["expected"]["after_step_order"]),
        "after_step must run in REVERSE registration order (onion model)"
    );
}

#[tokio::test]
async fn conformance_on_step_error_recovery_short_circuits() {
    let tc = case_by_id(&fixture(), "on_step_error_recovery_short_circuits");
    let step_name = tc["input"]["step"].as_str().expect("step");
    let log: Log = Arc::default();

    let mut strategy =
        ExecutionStrategy::new("conformance", vec![failing_step(step_name, &tc)]).unwrap();
    // Two of the three handlers offer a recovery, so this case can actually
    // tell first-wins from last-wins.
    let returns = &tc["input"]["on_step_error_returns"];
    for label in strings(&tc["input"]["register_order"]) {
        let recovery = match returns.get(&label) {
            Some(Value::Null) | None => None,
            Some(v) => Some(v.clone()),
        };
        strategy.add_step_middleware(Arc::new(Recorder {
            label: label.clone(),
            log: Arc::clone(&log),
            recovery,
            only_step: None,
            before_fails_on: None,
        }));
    }

    let mut pipeline_ctx = ctx();
    let outcome = PipelineEngine::run(&strategy, &mut pipeline_ctx).await;

    assert_eq!(
        hooks(&log, "error:"),
        strings(&tc["expected"]["on_step_error_invoked"]),
        "on_step_error runs in REVERSE order and the FIRST recovery short-circuits the rest"
    );
    assert_eq!(
        outcome.is_err(),
        tc["expected"]["error_propagated"]
            .as_bool()
            .expect("error_propagated"),
        "a recovered step must not propagate the original error"
    );
    // `step_output_is_a_MUST`: the recovery value BECOMES the step's output,
    // it is not merely reported. Asserting the ORDER alone cannot catch an
    // implementation that consults the handlers correctly and then drops what
    // they returned. The value must be mw_b's — the first handler to recover
    // in reverse order — never mw_a's.
    assert_eq!(
        pipeline_ctx.output.as_ref(),
        Some(&tc["expected"]["step_output"]),
        "the winning on_step_error recovery value MUST become the step output"
    );
}

#[tokio::test]
async fn conformance_on_step_error_null_propagates_error() {
    let tc = case_by_id(&fixture(), "on_step_error_null_propagates_error");
    let step_name = tc["input"]["step"].as_str().expect("step");
    let log: Log = Arc::default();

    let mut strategy =
        ExecutionStrategy::new("conformance", vec![failing_step(step_name, &tc)]).unwrap();
    for label in strings(&tc["input"]["register_order"]) {
        strategy.add_step_middleware(recorder(&label, &log) as Arc<dyn StepMiddleware>);
    }

    let outcome = PipelineEngine::run(&strategy, &mut ctx()).await;

    assert_eq!(
        hooks(&log, "error:"),
        strings(&tc["expected"]["on_step_error_invoked"]),
        "every handler is consulted, in reverse order, when none recovers"
    );
    let err = outcome.expect_err("all-None recovery must let the original error propagate");
    assert_eq!(
        err.code.wire_str(),
        tc["expected"]["wrapper_error_code"]
            .as_str()
            .expect("wrapper_error_code"),
        "an unrecovered step failure MUST surface wrapped in PIPELINE_STEP_ERROR"
    );
    // The fixture's `original_error_code` is the domain code `DEMO_FAILURE`,
    // which Rust's closed `ErrorCode` enum cannot represent (see
    // STEP_FAILURE_CODE). What the expectation is FOR — the original error
    // survives the wrapper instead of being replaced by it — is asserted here
    // against the code this driver's step really raised and the message the
    // fixture itself supplies.
    let original = err
        .unwrap_pipeline_step_error()
        .expect("PIPELINE_STEP_ERROR MUST preserve the original error in details.cause");
    assert_eq!(
        original.code, STEP_FAILURE_CODE,
        "the wrapped cause MUST be the error the step raised"
    );
    assert_eq!(
        original.message,
        step_failure_message(&tc),
        "the wrapped cause MUST carry the step's own message, not the wrapper's"
    );
}

#[tokio::test]
async fn conformance_on_step_error_only_executed_middlewares() {
    let tc = case_by_id(&fixture(), "on_step_error_only_executed_middlewares");
    let step_name = tc["input"]["step"].as_str().expect("step");
    let failing = tc["input"]["before_step_raises_in"]
        .as_str()
        .expect("before_step_raises_in");
    let log: Log = Arc::default();

    let mut strategy =
        ExecutionStrategy::new("conformance", vec![Box::new(OkStep(step_name.to_string()))])
            .unwrap();
    for label in strings(&tc["input"]["register_order"]) {
        strategy.add_step_middleware(Arc::new(Recorder {
            label: label.clone(),
            log: Arc::clone(&log),
            recovery: None,
            only_step: None,
            before_fails_on: (label == failing).then(|| step_name.to_string()),
        }));
    }

    let outcome = PipelineEngine::run(&strategy, &mut ctx()).await;

    assert_eq!(
        hooks(&log, "before:"),
        strings(&tc["expected"]["before_step_invoked"]),
        "the chain stops at the middleware whose before_step failed"
    );
    assert_eq!(
        hooks(&log, "error:"),
        strings(&tc["expected"]["on_step_error_invoked"]),
        "only middlewares that actually ran may observe the failure, in reverse order"
    );
    assert!(
        hooks(&log, "after:").is_empty(),
        "a failed before_step must prevent the step body, hence any after_step"
    );
    let err = outcome.expect_err("a before_step failure must not be swallowed");
    // `wrapper_is_load_bearing`: this is the single behaviour the before_step
    // fix exists to provide, and it was verified to leave every driver GREEN
    // while unasserted. Returning the bare error, or wrapping it in
    // PIPELINE_STEP_ERROR like a step-body failure, must fail here.
    assert_eq!(
        err.code.wire_str(),
        tc["expected"]["wrapper_error_code"]
            .as_str()
            .expect("wrapper_error_code"),
        "a failing before_step MUST be wrapped in MIDDLEWARE_CHAIN_ERROR, \
         identical to the module-level middleware contract"
    );
    let inner = err
        .unwrap_middleware_chain_error()
        .expect("MIDDLEWARE_CHAIN_ERROR MUST preserve the original error in details.inner_error");
    assert!(
        inner.message.contains(failing),
        "the wrapped cause MUST be the error {failing}'s before_step raised, got: {}",
        inner.message
    );
}

/// Records the two literal strings the fixture declares, so the assertion is on
/// the exact sequence rather than on a shape this driver invented.
struct AsyncRecorder {
    log: Log,
    before_mark: String,
    after_mark: String,
}

#[async_trait]
impl StepMiddleware for AsyncRecorder {
    async fn before_step(
        &self,
        _step_name: &str,
        _state: &PipelineState<'_>,
    ) -> Result<(), ModuleError> {
        // Yield first: if the engine dropped the future instead of awaiting it,
        // execution would not resume past this point and the mark would be lost.
        tokio::task::yield_now().await;
        self.log.lock().push(self.before_mark.clone());
        Ok(())
    }

    async fn after_step(
        &self,
        _step_name: &str,
        _state: &PipelineState<'_>,
        _result: &Value,
    ) -> Result<(), ModuleError> {
        tokio::task::yield_now().await;
        self.log.lock().push(self.after_mark.clone());
        Ok(())
    }
}

#[tokio::test]
async fn conformance_async_middleware_awaited() {
    let tc = case_by_id(&fixture(), "async_middleware_awaited");
    let step_name = tc["input"]["step"].as_str().expect("step");
    let log: Log = Arc::default();

    let mut strategy =
        ExecutionStrategy::new("conformance", vec![Box::new(OkStep(step_name.to_string()))])
            .unwrap();
    strategy.add_step_middleware(Arc::new(AsyncRecorder {
        log: Arc::clone(&log),
        before_mark: tc["input"]["async_before_step_records"]
            .as_str()
            .expect("async_before_step_records")
            .to_string(),
        after_mark: tc["input"]["async_after_step_records"]
            .as_str()
            .expect("async_after_step_records")
            .to_string(),
    }));

    PipelineEngine::run(&strategy, &mut ctx())
        .await
        .expect("the step succeeds");

    // Every hook is `async fn` in Rust, so "was it awaited" is observable only
    // as the side effect having landed before the pipeline moved on. Each hook
    // yields first: an unawaited future would never resume and the mark would
    // be missing entirely.
    assert_eq!(
        log.lock().clone(),
        strings(&tc["expected"]["recorded_in_order"]),
        "async StepMiddleware callbacks MUST be awaited before the pipeline advances"
    );
}

#[tokio::test]
async fn conformance_before_step_return_value_is_ignored() {
    let tc = case_by_id(&fixture(), "before_step_return_value_is_ignored");
    let step_name = tc["input"]["step"].as_str().expect("step");
    let log: Log = Arc::default();

    let mut strategy =
        ExecutionStrategy::new("conformance", vec![Box::new(OkStep(step_name.to_string()))])
            .unwrap();
    for label in strings(&tc["input"]["register_order"]) {
        strategy.add_step_middleware(recorder(&label, &log) as Arc<dyn StepMiddleware>);
    }

    let mut pipeline_ctx = PipelineContext::new(
        "executor.conformance.step_mw",
        tc["input"]["original_call_inputs"].clone(),
        Context::<Value>::anonymous(),
        "conformance",
    );
    let outcome = PipelineEngine::run(&strategy, &mut pipeline_ctx).await;

    assert_eq!(
        pipeline_ctx.inputs, tc["expected"]["module_received_inputs"],
        "before_step is an observation hook: a Step is execute(ctx) and has no \
         inputs parameter, so nothing a middleware returns may reach the module"
    );
    assert_eq!(
        hooks(&log, "before:"),
        strings(&tc["input"]["register_order"])
    );
    assert_eq!(
        outcome.is_err(),
        tc["expected"]["error_raised"]
            .as_bool()
            .expect("error_raised")
    );
}

#[tokio::test]
async fn conformance_before_step_failure_recovery_is_discarded() {
    let tc = case_by_id(&fixture(), "before_step_failure_recovery_is_discarded");
    let step_name = tc["input"]["step"].as_str().expect("step");
    let following = tc["input"]["following_step"]
        .as_str()
        .expect("following_step");
    let failing = tc["input"]["before_step_raises_in"]
        .as_str()
        .expect("before_step_raises_in");
    let ignore_errors = tc["input"]["ignore_errors"]
        .as_bool()
        .expect("ignore_errors");
    assert!(
        ignore_errors,
        "the case only tests `ignore_errors_applies: false` if the guarded step \
         really declares ignore_errors; the fixture must keep it true"
    );
    let log: Log = Arc::default();

    // `acl_check` — the fixture uses a real gate on purpose: honouring the
    // recovery here would skip the authorization check outright.
    let mut strategy = ExecutionStrategy::new(
        "conformance",
        vec![
            Box::new(TrackingStep {
                name: step_name.to_string(),
                log: Arc::clone(&log),
                ignore_errors,
            }),
            Box::new(TrackingStep {
                name: following.to_string(),
                log: Arc::clone(&log),
                ignore_errors: false,
            }),
        ],
    )
    .unwrap();
    let returns = &tc["input"]["on_step_error_returns"];
    for label in strings(&tc["input"]["register_order"]) {
        let recovery = match returns.get(&label) {
            Some(Value::Null) | None => None,
            Some(v) => Some(v.clone()),
        };
        strategy.add_step_middleware(Arc::new(Recorder {
            label: label.clone(),
            log: Arc::clone(&log),
            recovery,
            only_step: None,
            before_fails_on: (label == failing).then(|| step_name.to_string()),
        }));
    }

    let mut pipeline_ctx = ctx();
    let outcome = PipelineEngine::run(&strategy, &mut pipeline_ctx).await;

    // `before_step_failure_is_terminal`: the recovery is proven DISCARDED by
    // the FOLLOWING step never running, not merely by an error coming back.
    // An implementation that honoured the recovery would continue past
    // `acl_check` — a silent authorization bypass — and could still surface an
    // error later, satisfying a raise-only assertion while the bypass is live.
    // This is asserted FIRST so that a live bypass reports itself as a bypass:
    // honouring the recovery also re-runs the hooks for the following step, so
    // a hook-order assertion placed above would fire first and describe the
    // symptom instead of the breach.
    let bodies = hooks(&log, "step:");
    assert!(
        !bodies.contains(&following.to_string()),
        "expected following step `{following}` NOT to execute; step bodies that ran: {bodies:?}"
    );
    assert_eq!(
        bodies.contains(&following.to_string()),
        tc["expected"]["following_step_executed"]
            .as_bool()
            .expect("following_step_executed"),
        "honouring a before_step recovery would advance the pipeline past a \
         step whose body never ran"
    );
    assert!(
        bodies.is_empty(),
        "the guarded step's own body must not run either; step bodies that ran: {bodies:?}"
    );
    assert_eq!(
        pipeline_ctx.output, None,
        "a discarded recovery MUST NOT become the step's output ({})",
        tc["expected"]["step_output"]
    );

    assert_eq!(
        hooks(&log, "before:"),
        strings(&tc["expected"]["before_step_invoked"]),
        "the chain stops at the middleware whose before_step failed"
    );
    assert_eq!(
        hooks(&log, "error:"),
        strings(&tc["expected"]["on_step_error_invoked"]),
        "the already-entered middlewares are still told, in reverse order — \
         for observation and cleanup only"
    );
    assert_eq!(
        hooks(&log, "after:"),
        strings(&tc["expected"]["after_step_invoked"]),
        "no step body ran, so there is nothing for after_step to close over"
    );

    let err = outcome.expect_err("a before_step failure must not be swallowed");
    assert_eq!(
        err.code.wire_str(),
        tc["expected"]["wrapper_error_code"]
            .as_str()
            .expect("wrapper_error_code"),
        "`ignore_errors` declares that THIS STEP's failure is tolerable; a broken \
         middleware chain is not a step failure, so MIDDLEWARE_CHAIN_ERROR MUST \
         propagate regardless — and MUST NOT be re-wrapped as PIPELINE_STEP_ERROR"
    );
    assert_ne!(
        tc["expected"]["ignore_errors_applies"].as_bool(),
        Some(true),
        "the fixture pins that ignore_errors does not apply on this path"
    );
    let inner = err
        .unwrap_middleware_chain_error()
        .expect("MIDDLEWARE_CHAIN_ERROR MUST preserve the original error in details.inner_error");
    assert!(
        inner.message.contains(failing),
        "the wrapped cause MUST be the error {failing}'s before_step raised, got: {}",
        inner.message
    );
}

#[tokio::test]
async fn conformance_after_step_fires_after_a_recovered_step() {
    let tc = case_by_id(&fixture(), "after_step_fires_after_a_recovered_step");
    let step_name = tc["input"]["step"].as_str().expect("step");
    // `preceding_step` exists so `state.outputs` has a non-empty key set to be
    // exact about on the recovery path. Without it, "excludes the current step"
    // and "the map is empty" are indistinguishable.
    let preceding = tc["input"]["preceding_step"]
        .as_str()
        .expect("preceding_step")
        .to_string();
    let log: Log = Arc::default();

    let mut strategy = ExecutionStrategy::new(
        "conformance",
        vec![
            Box::new(OkStep(preceding.clone())),
            failing_step(step_name, &tc),
        ],
    )
    .unwrap();
    let returns = &tc["input"]["on_step_error_returns"];
    for label in strings(&tc["input"]["register_order"]) {
        let recovery = match returns.get(&label) {
            Some(Value::Null) | None => None,
            Some(v) => Some(v.clone()),
        };
        strategy.add_step_middleware(Arc::new(Recorder {
            label: label.clone(),
            log: Arc::clone(&log),
            recovery,
            // The preceding step runs every hook too; this case's expectations
            // are ordered lists about `step_name` alone.
            only_step: Some(step_name.to_string()),
            before_fails_on: None,
        }));
    }
    // `both_snapshot_sites`: the recovery branch has its OWN `step_outputs`
    // snapshot, and `state_outputs_excludes_the_current_step_in_every_hook`
    // cannot reach it — that case recovers from nothing. Registered last, so it
    // is the first `after_step` to run in the reverse-order unwind and cannot
    // be shielded by a recorder ahead of it. Its `on_step_error` returns None,
    // leaving first-recovery-wins to mw_b exactly as the fixture expects.
    let observer = Arc::new(OutputsObserver::default());
    strategy.add_step_middleware(Arc::clone(&observer) as Arc<dyn StepMiddleware>);

    let mut pipeline_ctx = ctx();
    let outcome = PipelineEngine::run(&strategy, &mut pipeline_ctx).await;

    assert_eq!(
        outcome.is_err(),
        tc["expected"]["error_propagated"]
            .as_bool()
            .expect("error_propagated"),
        "a recovered step body must not propagate the original error"
    );
    assert_eq!(
        hooks(&log, "error:"),
        strings(&tc["expected"]["on_step_error_invoked"]),
        "on_step_error runs in REVERSE order and the FIRST recovery short-circuits the rest"
    );
    // The rule this case exists for: apcore-rust used to jump straight from the
    // recovery branch to the next step, skipping after_step entirely, so a
    // middleware that acquired something in before_step never released it on
    // the recovery path. The onion MUST close in REVERSE registration order,
    // exactly as it does after a naturally successful body.
    assert_eq!(
        hooks(&log, "after:"),
        strings(&tc["expected"]["after_step_invoked"]),
        "after_step MUST fire after a RECOVERED step body, in reverse registration order"
    );
    // The SECOND snapshot site. A recovered step is still the current step
    // while its `after_step` runs, so the recovery branch MUST insert into
    // `step_outputs` AFTER the hooks, exactly as the naturally successful path
    // does. Swapping those two lines at the recovery site left every other case
    // in this file green.
    assert_eq!(
        observer.keys_at("after", step_name),
        sorted_strings(&tc["expected"]["outputs_keys_in_after_step"]),
        "after_step on a RECOVERED step MUST see exactly the steps that \
         completed before it — `{step_name}` itself MUST NOT be a key"
    );
    assert_eq!(
        pipeline_ctx.output.as_ref(),
        Some(&tc["expected"]["step_output"]),
        "the winning on_step_error recovery value MUST become the step output"
    );

    // Interleaving, composed entirely from the fixture's own ordered lists:
    // every before_step, then the error handlers, then the after_step chain.
    // Asserting the three lists separately cannot catch an implementation that
    // closes the onion before consulting the handler that recovered.
    let mut expected_log: Vec<String> = strings(&tc["input"]["register_order"])
        .iter()
        .map(|l| format!("before:{l}"))
        .collect();
    expected_log.extend(
        strings(&tc["expected"]["on_step_error_invoked"])
            .iter()
            .map(|l| format!("error:{l}")),
    );
    expected_log.extend(
        strings(&tc["expected"]["after_step_invoked"])
            .iter()
            .map(|l| format!("after:{l}")),
    );
    assert_eq!(
        log.lock().clone(),
        expected_log,
        "recovery closes the onion AFTER the handler that recovered, not before"
    );
}

// ---------------------------------------------------------------------------
// state.outputs — the EXACT key set, in every hook
// ---------------------------------------------------------------------------

/// A step that sets `ctx.output`, so `state.outputs` has something to be exact
/// ABOUT. `OkStep` leaves `ctx.output` untouched, and a map that is empty
/// because nothing ever produced an output would satisfy two of this case's
/// three expectations without the engine doing anything right.
struct OutputStep {
    name: String,
    output: Value,
}

#[async_trait]
impl Step for OutputStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "conformance stand-in that produces an output"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        ctx.output = Some(self.output.clone());
        Ok(StepResult::continue_step())
    }
}

/// Records the EXACT key set of `state.outputs` at every hook it is handed,
/// labelled with the hook and the step it was observed on.
#[derive(Default)]
struct OutputsObserver {
    /// `(hook, step_name, sorted keys of state.outputs)`, in invocation order.
    seen: Mutex<Vec<(String, String, Vec<String>)>>,
    /// The `result` argument `after_step` was handed, per step.
    after_results: Mutex<Vec<(String, Value)>>,
    /// `Some(v)` makes `on_step_error` recover with `v`, which routes
    /// `after_step` through the recovery branch instead of the naturally
    /// successful one — a second, separate snapshot site.
    recovery: Option<Value>,
}

impl OutputsObserver {
    fn record(&self, hook: &str, step_name: &str, state: &PipelineState<'_>) {
        // `outputs_is_a_live_reference`: `PipelineState::outputs` borrows the
        // engine's own map rather than handing out a copy, so a driver that
        // kept the reference and read it after the run would see the FINAL map
        // on every entry and could never fail. Rust's borrow checker refuses
        // that shape outright — the lifetime is tied to the hook call — but the
        // key set is copied HERE, inside the hook, so the assertion is about
        // the map as the hook saw it and not as the run left it.
        let mut keys: Vec<String> = state.outputs.keys().cloned().collect();
        keys.sort();
        self.seen
            .lock()
            .push((hook.to_string(), step_name.to_string(), keys));
    }

    /// The key set observed in `hook` on `step_name`.
    ///
    /// Panics when the hook never fired rather than returning an empty vec: a
    /// hook that was never invoked must not read as "observed an empty key
    /// set", which would turn two of the three expectations into passes for an
    /// engine that skipped the hook entirely.
    fn keys_at(&self, hook: &str, step_name: &str) -> Vec<String> {
        let seen = self.seen.lock();
        seen.iter()
            .find(|(h, s, _)| h == hook && s == step_name)
            .map(|(_, _, keys)| keys.clone())
            .unwrap_or_else(|| {
                panic!("`{hook}` never fired on step `{step_name}`; observed: {seen:?}")
            })
    }

    fn after_result(&self, step_name: &str) -> Value {
        self.after_results
            .lock()
            .iter()
            .find(|(s, _)| s == step_name)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("after_step never fired on step `{step_name}`"))
    }

    /// `current_step_never_present`, checked across EVERY hook invocation of
    /// the run rather than only the three the fixture names explicitly.
    fn assert_current_step_never_present(&self, tc: &Value) {
        assert_eq!(
            tc["expected"]["current_step_never_present"].as_bool(),
            Some(true),
            "the fixture pins that the current step is absent from state.outputs \
             in every hook; this driver has no branch for the opposite"
        );
        for (hook, step_name, keys) in self.seen.lock().iter() {
            assert!(
                !keys.contains(step_name),
                "`{hook}` on step `{step_name}` saw the CURRENT step in \
                 state.outputs (keys: {keys:?}). The map holds exactly the steps \
                 that completed BEFORE this one; the current step's output \
                 reaches after_step as the `result` parameter instead."
            );
        }
    }
}

#[async_trait]
impl StepMiddleware for OutputsObserver {
    async fn before_step(
        &self,
        step_name: &str,
        state: &PipelineState<'_>,
    ) -> Result<(), ModuleError> {
        self.record("before", step_name, state);
        Ok(())
    }

    async fn after_step(
        &self,
        step_name: &str,
        state: &PipelineState<'_>,
        result: &Value,
    ) -> Result<(), ModuleError> {
        self.record("after", step_name, state);
        self.after_results
            .lock()
            .push((step_name.to_string(), result.clone()));
        Ok(())
    }

    async fn on_step_error(
        &self,
        step_name: &str,
        state: &PipelineState<'_>,
        _error: &ModuleError,
    ) -> Result<Option<Value>, ModuleError> {
        self.record("error", step_name, state);
        Ok(self.recovery.clone())
    }
}

/// Sorted copy of a fixture key list. `assert_the_exact_key_set` is a statement
/// about the SET, and `state.outputs` is a `HashMap` whose iteration order is
/// randomized per process, so both sides are sorted before comparison.
fn sorted_strings(v: &Value) -> Vec<String> {
    let mut out = strings(v);
    out.sort();
    out
}

/// The pipeline the case declares: the first two steps produce their fixture
/// output, the third raises. The failure is what lets `on_step_error` be
/// observed on a step that has predecessors — proving the map KEEPS earlier
/// steps while excluding the current one. An all-empty map would satisfy the
/// `before_step` and `after_step` expectations on their own.
fn outputs_case_pipeline(tc: &Value) -> Vec<Box<dyn Step>> {
    let names = strings(&tc["input"]["steps"]);
    let declared = &tc["input"]["step_outputs"];
    let failing = failing_step_name(tc);
    names
        .iter()
        .map(|name| -> Box<dyn Step> {
            if *name == failing {
                Box::new(FailingStep(
                    name.clone(),
                    STEP_FAILURE_CODE,
                    tc["input"]["third_step_raises"]["message"]
                        .as_str()
                        .expect("third_step_raises.message")
                        .to_string(),
                ))
            } else {
                Box::new(OutputStep {
                    name: name.clone(),
                    output: declared[name].clone(),
                })
            }
        })
        .collect()
}

/// The step the fixture's `third_step_raises` refers to — the third and last
/// of `input.steps`, read from the fixture rather than hard-coded.
fn failing_step_name(tc: &Value) -> String {
    let names = strings(&tc["input"]["steps"]);
    assert_eq!(
        names.len(),
        3,
        "`third_step_raises` names the THIRD step; the case must declare three"
    );
    names[2].clone()
}

fn outputs_case() -> Value {
    case_by_id(
        &fixture(),
        "state_outputs_excludes_the_current_step_in_every_hook",
    )
}

#[tokio::test]
async fn conformance_state_outputs_excludes_the_current_step_in_every_hook() {
    let tc = outputs_case();
    let observed_on = tc["input"]["observe_hooks_on"]
        .as_str()
        .expect("observe_hooks_on")
        .to_string();
    let failing = failing_step_name(&tc);

    let mut strategy = ExecutionStrategy::new("conformance", outputs_case_pipeline(&tc)).unwrap();
    let observer = Arc::new(OutputsObserver::default());
    strategy.add_step_middleware(Arc::clone(&observer) as Arc<dyn StepMiddleware>);

    let outcome = PipelineEngine::run(&strategy, &mut ctx()).await;
    outcome.expect_err("the third step raises and nothing recovers it");

    // `assert_the_exact_key_set`: the EXACT set, never `!contains("second")`.
    // A key-absence assertion also passes against an implementation that lost
    // `first`, and against one that never populated the map at all.
    assert_eq!(
        observer.keys_at("before", &observed_on),
        sorted_strings(&tc["expected"]["outputs_keys_in_before_step"]),
        "before_step on `{observed_on}` sees exactly the steps that completed \
         before it — the current step has not run"
    );
    // The one that bites when the snapshot is ordered before the hook: an
    // engine that inserts the current step's output first hands after_step
    // {first, second} here instead of {first}.
    assert_eq!(
        observer.keys_at("after", &observed_on),
        sorted_strings(&tc["expected"]["outputs_keys_in_after_step"]),
        "after_step on `{observed_on}` MUST NOT see the current step: it \
         succeeded, and its output is the `result` parameter. Implementations \
         MUST NOT insert it into state.outputs before invoking the hook."
    );
    // Observed on the THIRD step on purpose: this is the expectation that
    // proves the map really does carry earlier steps, so the two above cannot
    // be satisfied by an engine that never populates it.
    assert_eq!(
        observer.keys_at("error", &failing),
        sorted_strings(&tc["expected"]["outputs_keys_in_on_step_error"]),
        "on_step_error on `{failing}` keeps every earlier step and excludes the \
         failing one — it ran, but there is no output"
    );
    observer.assert_current_step_never_present(&tc);

    // The current step's output is not MISSING from after_step for lack of it:
    // it arrives as `result`. Carrying the same value down two paths is how the
    // two drift apart, which is why only one path exists.
    assert_eq!(
        observer.after_result(&observed_on),
        tc["input"]["step_outputs"][&observed_on],
        "after_step receives the current step's output as `result`"
    );
}

#[test]
fn drives_every_fixture_case() {
    let fx = fixture();
    let ids: Vec<&str> = fx["test_cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tc| tc["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "before_after_invocation_order",
            "on_step_error_recovery_short_circuits",
            "on_step_error_null_propagates_error",
            "on_step_error_only_executed_middlewares",
            "async_middleware_awaited",
            "before_step_return_value_is_ignored",
            "before_step_failure_recovery_is_discarded",
            "after_step_fires_after_a_recovered_step",
            "state_outputs_excludes_the_current_step_in_every_hook",
        ],
        "every case above has a #[tokio::test] in this file; teach the driver \
         when the fixture grows, do not skip the case"
    );
}
