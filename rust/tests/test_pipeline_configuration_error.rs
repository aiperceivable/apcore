//! Sync alignment (W-7): pipeline-configuration errors that are NOT about
//! `requires`/`provides` mismatch should surface as
//! `ErrorCode::PipelineConfigurationError` (or the existing `PipelineConfigInvalid`
//! when missing-anchor/missing-step) — distinct from
//! `PipelineDependencyError` which is reserved for graph-dependency failures.
//!
//! Behaviour expected:
//!   - removing a non-existent step -> PipelineConfigurationError
//!   - configuring a non-existent step -> PipelineConfigurationError
//!   - inserting after/before an unknown anchor -> PipelineConfigurationError
//!   - missing both anchor (after/before) on a custom step -> PipelineConfigurationError
//!
//! NONE of these should yield `PipelineDependencyError`.

use apcore::errors::{ErrorCode, ModuleError};
use apcore::pipeline::{PipelineContext, Step, StepResult};
use apcore::pipeline_config::{
    build_strategy_from_config, register_step_type, unregister_step_type,
};
use async_trait::async_trait;
use serde_json::{json, Value};

struct NoopStep;
#[async_trait]
impl Step for NoopStep {
    fn name(&self) -> &'static str {
        "noop_for_test"
    }
    fn description(&self) -> &'static str {
        "no-op"
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

fn ensure_noop_type() {
    let _ = register_step_type(
        "noop_for_test",
        Box::new(|_cfg: &Value| -> Result<Box<dyn Step>, ModuleError> { Ok(Box::new(NoopStep)) }),
    );
}

fn cleanup_noop_type() {
    let _ = unregister_step_type("noop_for_test");
}

#[test]
fn missing_step_in_remove_yields_configuration_error_not_dependency() {
    let cfg = json!({
        "remove": ["nonexistent.step"]
    });
    let err = build_strategy_from_config(&cfg).expect_err("must fail");
    assert_ne!(
        err.code,
        ErrorCode::PipelineDependencyError,
        "missing-step removal must not be classified as a dependency error"
    );
    assert_eq!(err.code, ErrorCode::PipelineConfigurationError);
}

#[test]
fn missing_step_in_configure_yields_configuration_error_not_dependency() {
    let cfg = json!({
        "configure": {
            "nonexistent.step": { "ignore_errors": true }
        }
    });
    let err = build_strategy_from_config(&cfg).expect_err("must fail");
    assert_ne!(err.code, ErrorCode::PipelineDependencyError);
    assert_eq!(err.code, ErrorCode::PipelineConfigurationError);
}

#[test]
fn missing_anchor_yields_configuration_error_not_dependency() {
    ensure_noop_type();
    let cfg = json!({
        "steps": [
            {
                "name": "custom",
                "type": "noop_for_test",
                "after": "no.such.anchor",
                "config": {}
            }
        ]
    });
    let result = build_strategy_from_config(&cfg);
    cleanup_noop_type();
    let err = result.expect_err("must fail");
    assert_ne!(err.code, ErrorCode::PipelineDependencyError);
    // Missing-anchor is structurally a configuration / step-not-found problem.
    assert!(
        matches!(
            err.code,
            ErrorCode::PipelineConfigurationError | ErrorCode::PipelineStepNotFound
        ),
        "expected PipelineConfigurationError or PipelineStepNotFound, got {:?}",
        err.code
    );
}
