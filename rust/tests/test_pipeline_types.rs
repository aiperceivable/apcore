// Integration tests for pipeline types
// Validates Step trait, StepResult, PipelineContext, ExecutionStrategy, PipelineEngine, and trace types.

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::pipeline::{
    ExecutionStrategy, PipelineContext, PipelineEngine, PipelineTrace, Step, StepResult, StepTrace,
    StrategyInfo,
};
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Test helper: a configurable fake step
// ---------------------------------------------------------------------------

struct TestStep {
    name: String,
    description: String,
    removable: bool,
    replaceable: bool,
    result: StepResult,
}

impl TestStep {
    fn new(name: &str, removable: bool, replaceable: bool) -> Self {
        Self {
            name: name.to_string(),
            description: format!("Test step: {name}"),
            removable,
            replaceable,
            result: StepResult::continue_step(),
        }
    }

    fn with_result(mut self, result: StepResult) -> Self {
        self.result = result;
        self
    }

    fn boxed(name: &str, removable: bool, replaceable: bool) -> Box<dyn Step> {
        Box::new(Self::new(name, removable, replaceable))
    }
}

#[async_trait]
impl Step for TestStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn removable(&self) -> bool {
        self.removable
    }
    fn replaceable(&self) -> bool {
        self.replaceable
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(self.result.clone())
    }
}

// ---------------------------------------------------------------------------
// StepResult tests
// ---------------------------------------------------------------------------

#[test]
fn test_step_result_constructors() {
    let cont = StepResult::continue_step();
    assert_eq!(cont.action, "continue");

    let abort = StepResult::abort("denied");
    assert_eq!(abort.action, "abort");
    assert_eq!(abort.explanation.as_deref(), Some("denied"));

    let skip = StepResult::skip_to("execute");
    assert_eq!(skip.action, "skip_to");
    assert_eq!(skip.skip_to.as_deref(), Some("execute"));
}

#[test]
fn test_step_result_serialization() {
    let r = StepResult {
        action: "abort".into(),
        explanation: Some("ACL denied".into()),
        confidence: Some(0.99),
        alternatives: Some(vec!["use_admin".into(), "retry_later".into()]),
        ..Default::default()
    };
    let json = serde_json::to_value(&r).expect("serialize");
    assert_eq!(json["action"], "abort");
    assert_eq!(json["confidence"], 0.99);
    assert!(json.get("skip_to").is_none()); // skip_serializing_if = None

    let r2: StepResult = serde_json::from_value(json).expect("deserialize");
    assert_eq!(r2.alternatives.as_ref().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// PipelineTrace / StepTrace tests
// ---------------------------------------------------------------------------

#[test]
fn test_pipeline_trace_serialization() {
    let mut trace = PipelineTrace::new("mod_a".into(), "default".into());
    trace.steps.push(StepTrace {
        name: "acl_check".into(),
        duration_ms: 1.5,
        result: StepResult::continue_step(),
        skipped: false,
        decision_point: false,
        skip_reason: None,
    });
    trace.total_duration_ms = 10.0;
    trace.success = true;

    let json = serde_json::to_string(&trace).expect("serialize trace");
    let t2: PipelineTrace = serde_json::from_str(&json).expect("deserialize trace");
    assert_eq!(t2.module_id, "mod_a");
    assert_eq!(t2.steps.len(), 1);
    assert!(t2.success);
}

// ---------------------------------------------------------------------------
// StrategyInfo tests
// ---------------------------------------------------------------------------

#[test]
fn test_strategy_info_serialization() {
    let info = StrategyInfo {
        name: "default".into(),
        step_count: 3,
        step_names: vec!["a".into(), "b".into(), "c".into()],
        description: "a: do a; b: do b; c: do c".into(),
    };
    let json = serde_json::to_value(&info).expect("serialize");
    assert_eq!(json["step_count"], 3);
}

// ---------------------------------------------------------------------------
// ExecutionStrategy tests
// ---------------------------------------------------------------------------

#[test]
fn test_strategy_lifecycle() {
    // Create a strategy mimicking the standard pipeline subset.
    let mut strategy = ExecutionStrategy::new(
        "default",
        vec![
            TestStep::boxed("context_creation", false, false),
            TestStep::boxed("safety_check", true, true),
            TestStep::boxed("module_lookup", false, false),
            TestStep::boxed("acl_check", true, true),
            TestStep::boxed("execute", false, true),
            TestStep::boxed("return_result", false, false),
        ],
    )
    .expect("create strategy");

    // insert_after
    strategy
        .insert_after("acl_check", TestStep::boxed("approval_gate", true, true))
        .expect("insert_after");
    assert_eq!(strategy.step_names()[4], "approval_gate");

    // insert_before
    strategy
        .insert_before("execute", TestStep::boxed("input_validation", true, true))
        .expect("insert_before");

    // remove removable step
    strategy
        .remove("safety_check")
        .expect("remove safety_check");
    assert!(!strategy.step_names().contains(&"safety_check".to_string()));

    // remove non-removable step fails
    let err = strategy.remove("context_creation");
    assert!(err.is_err());

    // replace replaceable step
    strategy
        .replace("execute", TestStep::boxed("execute", false, true))
        .expect("replace execute");

    // replace non-replaceable step fails
    let err = strategy.replace(
        "context_creation",
        TestStep::boxed("context_creation", false, false),
    );
    assert!(err.is_err());

    // info
    let info = strategy.info();
    assert_eq!(info.name, "default");
    assert!(info.step_count > 0);
}

#[test]
fn test_strategy_duplicate_insert_rejected() {
    let mut s = ExecutionStrategy::new("s", vec![TestStep::boxed("a", true, true)]).unwrap();
    assert!(s
        .insert_after("a", TestStep::boxed("a", true, true))
        .is_err());
    assert!(s
        .insert_before("a", TestStep::boxed("a", true, true))
        .is_err());
}

#[test]
fn test_strategy_unknown_anchor_rejected() {
    let mut s = ExecutionStrategy::new("s", vec![TestStep::boxed("a", true, true)]).unwrap();
    assert!(s
        .insert_after("z", TestStep::boxed("b", true, true))
        .is_err());
    assert!(s
        .insert_before("z", TestStep::boxed("b", true, true))
        .is_err());
    assert!(s.remove("z").is_err());
    assert!(s.replace("z", TestStep::boxed("z", true, true)).is_err());
}

// ---------------------------------------------------------------------------
// Step trait async execution test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_step_execute_returns_result() {
    let step = TestStep::new("test", true, true).with_result(StepResult::abort("not allowed"));

    let ctx_inner = Context::<serde_json::Value>::anonymous();
    let mut pctx = PipelineContext::new("my_module", serde_json::json!({}), ctx_inner, "default");

    let result = step.execute(&mut pctx).await.expect("execute");
    assert_eq!(result.action, "abort");
    assert_eq!(result.explanation.as_deref(), Some("not allowed"));
}

#[tokio::test]
async fn test_pipeline_context_fields_initially_none() {
    let ctx_inner = Context::<serde_json::Value>::anonymous();
    let pctx = PipelineContext::new("m", serde_json::json!({"x": 1}), ctx_inner, "default");
    assert!(pctx.module.is_none());
    assert!(pctx.validated_inputs.is_none());
    assert!(pctx.output.is_none());
    assert!(pctx.validated_output.is_none());
}

// ---------------------------------------------------------------------------
// PipelineEngine tests
// ---------------------------------------------------------------------------

/// Helper: build a PipelineContext for engine tests.
fn make_ctx() -> PipelineContext {
    PipelineContext::new(
        "test_mod",
        serde_json::json!({}),
        Context::<serde_json::Value>::anonymous(),
        "test_strategy",
    )
}

#[tokio::test]
async fn test_pipeline_engine_continue_all_steps() {
    let strategy = ExecutionStrategy::new(
        "s",
        vec![
            TestStep::boxed("a", true, true),
            TestStep::boxed("b", true, true),
            TestStep::boxed("c", true, true),
        ],
    )
    .unwrap();

    let mut ctx = make_ctx();
    let (output, trace) = PipelineEngine::run(&strategy, &mut ctx).await.unwrap();

    assert!(trace.success);
    assert_eq!(trace.steps.len(), 3);
    assert_eq!(trace.steps[0].name, "a");
    assert_eq!(trace.steps[1].name, "b");
    assert_eq!(trace.steps[2].name, "c");
    assert!(output.is_none()); // no step set ctx.output
}

#[tokio::test]
async fn test_pipeline_engine_abort_stops_early() {
    let strategy = ExecutionStrategy::new(
        "s",
        vec![
            Box::new(TestStep::new("a", true, true)),
            Box::new(TestStep::new("b", true, true).with_result(StepResult::abort("denied"))),
            Box::new(TestStep::new("c", true, true)),
        ],
    )
    .unwrap();

    let mut ctx = make_ctx();
    let (_output, trace) = PipelineEngine::run(&strategy, &mut ctx).await.unwrap();

    assert!(!trace.success);
    // Only a and b executed; c was never reached.
    assert_eq!(trace.steps.len(), 2);
    assert_eq!(trace.steps[1].name, "b");
    assert_eq!(trace.steps[1].result.action, "abort");
}

#[tokio::test]
async fn test_pipeline_engine_skip_to() {
    let strategy = ExecutionStrategy::new(
        "s",
        vec![
            Box::new(TestStep::new("a", true, true).with_result(StepResult::skip_to("d"))),
            Box::new(TestStep::new("b", true, true)),
            Box::new(TestStep::new("c", true, true)),
            Box::new(TestStep::new("d", true, true)),
        ],
    )
    .unwrap();

    let mut ctx = make_ctx();
    let (_output, trace) = PipelineEngine::run(&strategy, &mut ctx).await.unwrap();

    assert!(trace.success);
    // Trace: a (executed), b (skipped), c (skipped), d (executed).
    assert_eq!(trace.steps.len(), 4);
    assert_eq!(trace.steps[0].name, "a");
    assert!(!trace.steps[0].skipped);
    assert!(trace.steps[1].skipped);
    assert_eq!(trace.steps[1].name, "b");
    assert!(trace.steps[2].skipped);
    assert_eq!(trace.steps[2].name, "c");
    assert!(!trace.steps[3].skipped);
    assert_eq!(trace.steps[3].name, "d");
}
