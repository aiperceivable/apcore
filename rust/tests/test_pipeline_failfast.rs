// Integration tests for pipeline fail-fast configuration (Issue #33 §1.2 / §2.1).
//
// §1.2: YAML referring to a nonexistent step (in `remove` or `configure`)
//       MUST surface an error rather than a silent `tracing::warn!`.
// §2.1: Strategy construction with unmet `requires`/`provides` MUST fail
//       with a `PipelineDependencyError`-style error rather than a warning.

use apcore::errors::{ErrorCode, ModuleError};
use apcore::pipeline::{ExecutionStrategy, PipelineContext, Step, StepResult};
use apcore::pipeline_config::build_strategy_from_config;
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Test step helpers
// ---------------------------------------------------------------------------

struct DepStep {
    name: String,
    requires: Vec<&'static str>,
    provides: Vec<&'static str>,
}

impl DepStep {
    fn boxed(
        name: &str,
        requires: Vec<&'static str>,
        provides: Vec<&'static str>,
    ) -> Box<dyn Step> {
        Box::new(Self {
            name: name.to_string(),
            requires,
            provides,
        })
    }
}

#[async_trait]
impl Step for DepStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.name
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    fn requires(&self) -> &[&'static str] {
        &self.requires
    }
    fn provides(&self) -> &[&'static str] {
        &self.provides
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::continue_step())
    }
}

// ---------------------------------------------------------------------------
// §2.1 strategy construction fail-fast
// ---------------------------------------------------------------------------

#[tokio::test]
async fn strategy_new_returns_error_when_required_field_not_provided() {
    // Step "b" requires "x" but no preceding step provides "x".
    let steps = vec![
        DepStep::boxed("a", vec![], vec!["y"]),
        DepStep::boxed("b", vec!["x"], vec![]),
    ];

    let result = ExecutionStrategy::new("dep_test", steps);
    let err = result.expect_err("strategy with unmet requires must fail-fast");
    assert_eq!(
        err.code,
        ErrorCode::PipelineDependencyError,
        "expected PipelineDependencyError, got {:?}",
        err.code
    );
    assert!(err.message.contains('x'));
    assert!(err.message.contains('b'));
}

#[tokio::test]
async fn strategy_new_succeeds_when_provides_satisfies_requires() {
    let steps = vec![
        DepStep::boxed("a", vec![], vec!["x"]),
        DepStep::boxed("b", vec!["x"], vec![]),
    ];

    let result = ExecutionStrategy::new("ok_test", steps);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// §1.2 YAML config fail-fast on nonexistent step
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_strategy_from_config_remove_unknown_step_returns_error() {
    let config: Value = json!({
        "remove": ["__nonexistent_step__"],
    });
    let result = build_strategy_from_config(&config);
    let err = result.expect_err("remove of unknown step must fail-fast");
    // Either PipelineConfigInvalid or PipelineStepNotFound is acceptable;
    // both signal a structural YAML/config error.
    assert!(
        matches!(
            err.code,
            ErrorCode::PipelineConfigInvalid
                | ErrorCode::PipelineStepNotFound
                | ErrorCode::PipelineConfigurationError
        ),
        "expected PipelineConfigInvalid, PipelineStepNotFound, or PipelineConfigurationError, got {:?}",
        err.code
    );
    assert!(err.message.contains("__nonexistent_step__"));
}

#[tokio::test]
async fn build_strategy_from_config_configure_unknown_step_returns_error() {
    let config: Value = json!({
        "configure": {
            "__nonexistent_step__": {"timeout_ms": 1000}
        }
    });
    let result = build_strategy_from_config(&config);
    let err = result.expect_err("configure of unknown step must fail-fast");
    assert!(
        matches!(
            err.code,
            ErrorCode::PipelineConfigInvalid
                | ErrorCode::PipelineStepNotFound
                | ErrorCode::PipelineConfigurationError
        ),
        "expected PipelineConfigInvalid, PipelineStepNotFound, or PipelineConfigurationError, got {:?}",
        err.code
    );
    assert!(err.message.contains("__nonexistent_step__"));
}

#[tokio::test]
async fn build_strategy_from_config_step_without_anchor_returns_error() {
    // Custom step inserted without `after`/`before` anchors must fail-fast
    // rather than silently warn-and-skip.
    let config: Value = json!({
        "steps": [{
            "name": "orphan",
            "type": "__never_registered__",
        }]
    });
    let result = build_strategy_from_config(&config);
    assert!(result.is_err(), "step without anchor must surface an error");
}
