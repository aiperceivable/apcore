// Cross-language conformance tests for Pipeline Hardening (Issue #33).
//
// Fixture source: apcore/conformance/fixtures/pipeline_hardening.json
// Spec reference: apcore/docs/features/core-executor.md (§Pipeline Hardening)
//
// The fixture cases exercise:
//   §1.1 fail_fast_on_step_error        — step error wraps in PipelineStepError
//   §1.1 continue_on_ignored_error      — ignore_errors: true keeps the pipeline alive
//   §1.2 replace_semantic_no_duplicate  — configure_step is idempotent
//   §1.4 run_until_stops_early          — predicate halts the pipeline mid-run
//   §1.5 step_lookup_is_not_linear      — ExecutionStrategy exposes an O(1) name→idx map

#![allow(clippy::missing_panics_doc)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::pipeline::{
    ExecutionStrategy, PipelineContext, PipelineEngine, PipelineState, Step, StepResult,
};

// ---------------------------------------------------------------------------
// Fixture loading (mirrors tests/conformance_test.rs discovery)
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

fn fixture_case<'a>(fixture: &'a Value, id: &str) -> &'a Value {
    fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array")
        .iter()
        .find(|c| c["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("fixture case '{id}' not present"))
}

// ---------------------------------------------------------------------------
// Test step helpers
// ---------------------------------------------------------------------------

/// Step that records every invocation into a shared counter and returns
/// continue. Used to assert which steps did and did not execute.
struct TrackingStep {
    name: String,
    invocations: Arc<AtomicUsize>,
}

impl TrackingStep {
    fn new(name: &str, invocations: Arc<AtomicUsize>) -> Self {
        Self {
            name: name.to_string(),
            invocations,
        }
    }
}

#[async_trait]
impl Step for TrackingStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &'static str {
        "tracking step"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(StepResult::continue_step())
    }
}

/// Step that emits a `skip_to` action targeting a sibling step. Used to
/// verify O(1) skip-to lookup via `ExecutionStrategy::name_to_idx`.
struct SkipToStep {
    name: String,
    target: String,
}

#[async_trait]
impl Step for SkipToStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &'static str {
        "skip-to step"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::skip_to(&self.target))
    }
}

/// Step that always returns an error. The error code is fixed
/// (`GeneralInvalidInput`) so we can assert the wrapper preserves the cause.
struct RaisingStep {
    name: String,
    ignore_errors: bool,
    invocations: Arc<AtomicUsize>,
}

impl RaisingStep {
    fn new(name: &str, ignore_errors: bool, invocations: Arc<AtomicUsize>) -> Self {
        Self {
            name: name.to_string(),
            ignore_errors,
            invocations,
        }
    }
}

#[async_trait]
impl Step for RaisingStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &'static str {
        "raising step"
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
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Err(ModuleError::new(
            ErrorCode::GeneralInvalidInput,
            format!("step '{}' intentionally raised", self.name),
        ))
    }
}

fn make_ctx() -> PipelineContext {
    PipelineContext::new(
        "demo.process",
        serde_json::json!({}),
        Context::<Value>::anonymous(),
        "test",
    )
}

// ---------------------------------------------------------------------------
// §1.1 fail_fast_on_step_error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pipeline_hardening_fail_fast_on_step_error() {
    let fixture = load_fixture("pipeline_hardening");
    let case = fixture_case(&fixture, "fail_fast_on_step_error");
    let expected_steps_executed: Vec<String> = case["expected"]["steps_executed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let expected_error_code = case["expected"]["error_code"].as_str().unwrap();

    let context_count = Arc::new(AtomicUsize::new(0));
    let lookup_count = Arc::new(AtomicUsize::new(0));
    let validate_count = Arc::new(AtomicUsize::new(0));
    let execute_count = Arc::new(AtomicUsize::new(0));

    let strategy = ExecutionStrategy::new(
        "test",
        vec![
            Box::new(TrackingStep::new(
                "context_creation",
                Arc::clone(&context_count),
            )),
            Box::new(TrackingStep::new(
                "module_lookup",
                Arc::clone(&lookup_count),
            )),
            Box::new(RaisingStep::new(
                "input_validation",
                false,
                Arc::clone(&validate_count),
            )),
            Box::new(TrackingStep::new("execute", Arc::clone(&execute_count))),
        ],
    )
    .unwrap();

    let mut ctx = make_ctx();
    let err = PipelineEngine::run(&strategy, &mut ctx)
        .await
        .expect_err("input_validation must fail-fast");

    // §1.1: error code must be PIPELINE_STEP_ERROR
    let serialized = serde_json::to_value(err.code).unwrap();
    assert_eq!(
        serialized.as_str().unwrap(),
        expected_error_code,
        "fixture expected error_code={expected_error_code}",
    );
    assert_eq!(err.code, ErrorCode::PipelineStepError);

    // step_name is preserved on the wrapper.
    assert_eq!(err.step_name(), Some("input_validation"));

    // The original cause is recoverable.
    let underlying = err
        .unwrap_pipeline_step_error()
        .expect("PipelineStepError must carry the original cause");
    assert_eq!(underlying.code, ErrorCode::GeneralInvalidInput);

    // The expected step set ran, and nothing past input_validation ran.
    assert_eq!(context_count.load(Ordering::SeqCst), 1);
    assert_eq!(lookup_count.load(Ordering::SeqCst), 1);
    assert_eq!(validate_count.load(Ordering::SeqCst), 1);
    assert_eq!(execute_count.load(Ordering::SeqCst), 0);

    // Trace mirrors the executed step list (skipped steps excluded).
    let executed_in_trace: Vec<String> = ctx
        .trace
        .steps
        .iter()
        .filter(|s| !s.skipped)
        .map(|s| s.name.clone())
        .collect();
    assert_eq!(executed_in_trace, expected_steps_executed);
}

// ---------------------------------------------------------------------------
// §1.1 continue_on_ignored_error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pipeline_hardening_continue_on_ignored_error() {
    let fixture = load_fixture("pipeline_hardening");
    let case = fixture_case(&fixture, "continue_on_ignored_error");
    assert!(!case["expected"]["stopped"].as_bool().unwrap());
    assert!(case["expected"]["continued"].as_bool().unwrap());

    let pre_count = Arc::new(AtomicUsize::new(0));
    let raised_count = Arc::new(AtomicUsize::new(0));
    let post_count = Arc::new(AtomicUsize::new(0));

    let strategy = ExecutionStrategy::new(
        "test",
        vec![
            Box::new(TrackingStep::new("step_a", Arc::clone(&pre_count))),
            Box::new(RaisingStep::new(
                "input_validation",
                true,
                Arc::clone(&raised_count),
            )),
            Box::new(TrackingStep::new("step_c", Arc::clone(&post_count))),
        ],
    )
    .unwrap();

    let mut ctx = make_ctx();
    let (_output, trace) = PipelineEngine::run(&strategy, &mut ctx)
        .await
        .expect("ignore_errors=true must let the pipeline complete");

    assert!(trace.success);
    assert_eq!(pre_count.load(Ordering::SeqCst), 1);
    assert_eq!(raised_count.load(Ordering::SeqCst), 1);
    assert_eq!(post_count.load(Ordering::SeqCst), 1);

    let ignored_trace = trace
        .steps
        .iter()
        .find(|s| s.name == "input_validation")
        .expect("input_validation must appear in the trace");
    assert_eq!(ignored_trace.skip_reason.as_deref(), Some("error_ignored"));
}

// ---------------------------------------------------------------------------
// §1.2 replace_semantic_no_duplicate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pipeline_hardening_replace_semantic_no_duplicate() {
    let fixture = load_fixture("pipeline_hardening");
    let case = fixture_case(&fixture, "replace_semantic_no_duplicate");
    let expected_count =
        usize::try_from(case["expected"]["step_count_for_name"].as_u64().unwrap()).unwrap();

    let count = Arc::new(AtomicUsize::new(0));
    let mut strategy = ExecutionStrategy::new(
        "test",
        vec![
            Box::new(TrackingStep::new("a", Arc::clone(&count))),
            Box::new(TrackingStep::new("input_validation", Arc::clone(&count))),
            Box::new(TrackingStep::new("b", Arc::clone(&count))),
        ],
    )
    .unwrap();

    let original_idx = *strategy.name_to_idx().get("input_validation").unwrap();

    let times = case["input"]["times"].as_u64().unwrap();
    for n in 0..times {
        let replacement = TrackingStep::new("input_validation", Arc::clone(&count));
        strategy
            .configure_step("input_validation", Box::new(replacement))
            .unwrap_or_else(|e| panic!("configure_step #{n} must succeed: {e}"));
    }

    let occurrences = strategy
        .step_names()
        .iter()
        .filter(|n| n.as_str() == "input_validation")
        .count();
    assert_eq!(occurrences, expected_count);
    assert_eq!(
        *strategy.name_to_idx().get("input_validation").unwrap(),
        original_idx,
        "configure_step must preserve the step's position",
    );

    // configure_step on a missing target must surface PIPELINE_STEP_NOT_FOUND.
    let missing = strategy.configure_step(
        "nonexistent",
        Box::new(TrackingStep::new("nonexistent", Arc::clone(&count))),
    );
    let err = missing.expect_err("nonexistent target must error");
    assert_eq!(err.code, ErrorCode::PipelineStepNotFound);
}

// ---------------------------------------------------------------------------
// §1.4 run_until_stops_early
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pipeline_hardening_run_until_stops_early() {
    let fixture = load_fixture("pipeline_hardening");
    let case = fixture_case(&fixture, "run_until_stops_early");
    let last_step_executed = case["expected"]["last_step_executed"]
        .as_str()
        .unwrap()
        .to_string();
    let stop_after = case["input"]["run_until_after"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(last_step_executed, stop_after);

    let context_count = Arc::new(AtomicUsize::new(0));
    let lookup_count = Arc::new(AtomicUsize::new(0));
    let execute_count = Arc::new(AtomicUsize::new(0));
    let return_count = Arc::new(AtomicUsize::new(0));

    let strategy = ExecutionStrategy::new(
        "test",
        vec![
            Box::new(TrackingStep::new(
                "context_creation",
                Arc::clone(&context_count),
            )),
            Box::new(TrackingStep::new(
                "module_lookup",
                Arc::clone(&lookup_count),
            )),
            Box::new(TrackingStep::new("execute", Arc::clone(&execute_count))),
            Box::new(TrackingStep::new(
                "return_result",
                Arc::clone(&return_count),
            )),
        ],
    )
    .unwrap();

    let mut ctx = make_ctx();
    let predicate_target = stop_after.clone();
    let (_output, trace) =
        PipelineEngine::run_until(&strategy, &mut ctx, move |state: &PipelineState| {
            state.step_name == predicate_target
        })
        .await
        .expect("predicate-based termination is not an error");

    assert!(trace.success);
    assert_eq!(context_count.load(Ordering::SeqCst), 1);
    assert_eq!(lookup_count.load(Ordering::SeqCst), 1);

    // `steps_after_skipped` — every step positioned AFTER the one the predicate
    // matched on must not have run. Counted from the steps' own side-effects,
    // so a pipeline that kept going past the predicate turns this red.
    let steps_after_skipped =
        execute_count.load(Ordering::SeqCst) == 0 && return_count.load(Ordering::SeqCst) == 0;
    assert_eq!(
        steps_after_skipped,
        case["expected"]["steps_after_skipped"].as_bool().unwrap(),
        "steps_after_skipped mismatch — execute ran {} time(s), return_result ran {} time(s) \
         after run_until returned true on {stop_after}",
        execute_count.load(Ordering::SeqCst),
        return_count.load(Ordering::SeqCst),
    );

    // The last entry in the trace is the step the predicate matched on.
    let last_executed = trace
        .steps
        .iter()
        .rev()
        .find(|s| !s.skipped)
        .map(|s| s.name.clone())
        .expect("at least one step must have executed");
    assert_eq!(last_executed, last_step_executed);
}

// ---------------------------------------------------------------------------
// §1.5 step_lookup_is_not_linear (O(1) compliance)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pipeline_hardening_step_lookup_is_not_linear() {
    let fixture = load_fixture("pipeline_hardening");
    let case = fixture_case(&fixture, "step_lookup_is_not_linear");
    let expected_step_count =
        usize::try_from(case["input"]["step_count"].as_u64().unwrap()).unwrap();
    assert_eq!(
        case["expected"]["lookup_complexity"].as_str().unwrap(),
        "O(1)"
    );

    // Build a strategy with the documented number of steps and verify the
    // name→idx map is consistent with the ordered step list. This proves the
    // O(1) lookup index exists and stays in sync after construction.
    let count = Arc::new(AtomicUsize::new(0));
    let names: Vec<String> = (0..expected_step_count)
        .map(|i| format!("step_{i}"))
        .collect();
    let steps: Vec<Box<dyn Step>> = names
        .iter()
        .map(|name| Box::new(TrackingStep::new(name, Arc::clone(&count))) as Box<dyn Step>)
        .collect();
    let mut strategy = ExecutionStrategy::new("test", steps).unwrap();

    assert_eq!(strategy.step_names().len(), expected_step_count);
    assert_eq!(strategy.name_to_idx().len(), expected_step_count);
    for (expected_idx, name) in names.iter().enumerate() {
        assert_eq!(
            *strategy.name_to_idx().get(name).unwrap(),
            expected_idx,
            "name_to_idx must reflect the ordered step list",
        );
    }

    // Mutations must keep the index in sync.
    strategy.remove("step_3").unwrap();
    assert!(!strategy.name_to_idx().contains_key("step_3"));
    assert_eq!(strategy.name_to_idx().len(), expected_step_count - 1);
    let inserted = TrackingStep::new("inserted_after_step_2", Arc::clone(&count));
    strategy.insert_after("step_2", Box::new(inserted)).unwrap();
    assert_eq!(
        *strategy.name_to_idx().get("inserted_after_step_2").unwrap(),
        3,
    );

    // Verify skip_to uses the index by skipping the bulk of the pipeline.
    let head_count = Arc::new(AtomicUsize::new(0));
    let tail_count = Arc::new(AtomicUsize::new(0));
    let mut steps: Vec<Box<dyn Step>> = Vec::new();
    steps.push(Box::new(SkipToStep {
        name: "step_0".into(),
        target: "step_8".into(),
    }));
    for i in 1..=7 {
        steps.push(Box::new(TrackingStep::new(
            &format!("step_{i}"),
            Arc::clone(&head_count),
        )));
    }
    steps.push(Box::new(TrackingStep::new(
        "step_8",
        Arc::clone(&tail_count),
    )));

    let strategy = ExecutionStrategy::new("skipto", steps).unwrap();
    let mut ctx = make_ctx();
    PipelineEngine::run(&strategy, &mut ctx)
        .await
        .expect("skip_to via O(1) lookup should succeed");
    assert_eq!(
        head_count.load(Ordering::SeqCst),
        0,
        "skipped middle steps must NOT execute",
    );
    assert_eq!(tail_count.load(Ordering::SeqCst), 1);
}

struct SlowStep {
    name: String,
    sleep_ms: u64,
    timeout_ms: u64,
    completed: Arc<AtomicUsize>,
}

#[async_trait]
impl Step for SlowStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Fixture-driven timed step"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
        self.completed.fetch_add(1, Ordering::SeqCst);
        Ok(StepResult::continue_step())
    }
}

struct EchoModule;

#[async_trait]
impl apcore::module::Module for EchoModule {
    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }
    fn description(&self) -> &str {
        "Step timeout fixture module"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(inputs)
    }
}

async fn assert_timeout_case(id: &str) {
    let fixture = load_fixture("pipeline_hardening");
    let case = fixture_case(&fixture, id);
    let input = &case["input"];
    let custom = &input["custom_step"];
    let completed = Arc::new(AtomicUsize::new(0));
    let step = SlowStep {
        name: custom["name"].as_str().unwrap().to_string(),
        sleep_ms: custom["sleep_ms"].as_u64().unwrap(),
        timeout_ms: input["timeout_ms"].as_u64().unwrap(),
        completed: completed.clone(),
    };
    let registry = apcore::Registry::new();
    registry
        .register_module("fixture.module", Box::new(EchoModule))
        .unwrap();
    let mut strategy = apcore::builtin_steps::build_standard_strategy();
    strategy
        .insert_after(custom["after"].as_str().unwrap(), Box::new(step))
        .unwrap();
    let executor = apcore::Executor::with_strategy(registry, apcore::Config::default(), strategy);
    let result = executor
        .call("fixture.module", serde_json::json!({}), None, None)
        .await;
    let expected = &case["expected"];
    if let Some(code) = expected.get("call_error_code") {
        let error = result.expect_err("step exceeding its deadline must fail the call");
        assert_eq!(serde_json::json!(error.code.wire_str()), *code, "case {id}");
        assert_eq!(completed.load(Ordering::SeqCst), 0, "case {id}");
    } else if let Some(succeeds) = expected.get("call_succeeds") {
        assert_eq!(
            serde_json::json!(result.is_ok()),
            *succeeds,
            "case {id}: {result:?}"
        );
        assert_eq!(result.unwrap(), serde_json::json!({}), "case {id}");
        assert_eq!(completed.load(Ordering::SeqCst), 1, "case {id}");
    } else {
        panic!("case {id}: unhandled timeout expectation");
    }
}

#[tokio::test]
async fn pipeline_hardening_step_timeout_raises_module_timeout() {
    assert_timeout_case("step_timeout_raises_module_timeout").await;
}

#[tokio::test]
async fn pipeline_hardening_step_within_its_timeout_runs() {
    assert_timeout_case("step_within_its_timeout_runs").await;
}
