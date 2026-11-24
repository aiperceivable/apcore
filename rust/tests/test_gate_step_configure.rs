//! D-130 regression tests (PROTOCOL_SPEC 5.16.1 "Governance gates cannot be
//! weakened by `configure`").
//!
//! The fixture driver (`test_gate_step_configure_conformance.rs`) covers the
//! `pipeline.configure` path. These tests cover what it cannot reach:
//!
//! * the programmatic step-configuration API — `ExecutionStrategy`'s
//!   `configure_step` / `replace` / `replace_with` / `insert_*` / `new` — which
//!   in this SDK installs a whole step, so the rule is enforced on the step
//!   being installed when it identifies itself as a built-in gate;
//! * that a gate configured with an ACCEPTED field (`timeout_ms`) is still
//!   recognised as the gate, so `governance_state()` keeps reporting it wired.

use apcore::builtin_steps::{build_standard_strategy, BuiltinACLCheck, BuiltinApprovalGate};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::pipeline::{BuiltinGate, ExecutionStrategy, PipelineContext, Step, StepResult};
use apcore::pipeline_config::build_strategy_from_config;
use async_trait::async_trait;
use serde_json::json;

/// A step that delegates to a built-in gate and claims its identity, with one
/// weakening field set — what a caller would write to get a non-default field
/// onto a gate through the programmatic API.
struct WeakenedGate {
    inner: Box<dyn Step>,
    ignore_errors: bool,
    match_modules: Option<Vec<String>>,
    pure: Option<bool>,
}

impl WeakenedGate {
    fn acl() -> Self {
        Self {
            inner: Box::new(BuiltinACLCheck),
            ignore_errors: false,
            match_modules: None,
            pure: None,
        }
    }
    fn approval() -> Self {
        Self {
            inner: Box::new(BuiltinApprovalGate),
            ignore_errors: false,
            match_modules: None,
            pure: None,
        }
    }
}

#[async_trait]
impl Step for WeakenedGate {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    fn match_modules(&self) -> Option<&[String]> {
        self.match_modules.as_deref()
    }
    fn ignore_errors(&self) -> bool {
        self.ignore_errors
    }
    fn pure(&self) -> bool {
        self.pure.unwrap_or_else(|| self.inner.pure())
    }
    fn requires(&self) -> &[&str] {
        self.inner.requires()
    }
    fn provides(&self) -> &[&str] {
        self.inner.provides()
    }
    fn builtin_gate(&self) -> Option<BuiltinGate> {
        self.inner.builtin_gate()
    }
    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        self.inner.execute(ctx).await
    }
}

fn assert_rejected(result: Result<(), ModuleError>, step: &str, key: &str) {
    let err = result.expect_err("a weakened gate must be rejected");
    assert_eq!(
        err.code,
        ErrorCode::PipelineConfigurationError,
        "{}",
        err.message
    );
    assert!(
        err.message.contains(step),
        "message names the step: {}",
        err.message
    );
    assert!(
        err.message.contains(key),
        "message names the key: {}",
        err.message
    );
}

#[test]
fn configure_step_rejects_acl_gate_with_ignore_errors() {
    let mut s = build_standard_strategy();
    let step = WeakenedGate {
        ignore_errors: true,
        ..WeakenedGate::acl()
    };
    assert_rejected(
        s.configure_step("acl_check", Box::new(step)),
        "acl_check",
        "ignore_errors",
    );
}

#[test]
fn configure_step_rejects_approval_gate_with_match_modules() {
    let mut s = build_standard_strategy();
    let step = WeakenedGate {
        match_modules: Some(vec!["finance.*".to_string()]),
        ..WeakenedGate::approval()
    };
    assert_rejected(
        s.configure_step("approval_gate", Box::new(step)),
        "approval_gate",
        "match_modules",
    );
}

#[test]
fn replace_rejects_approval_gate_with_pure_true() {
    let mut s = build_standard_strategy();
    let step = WeakenedGate {
        pure: Some(true),
        ..WeakenedGate::approval()
    };
    assert_rejected(
        s.replace("approval_gate", Box::new(step)),
        "approval_gate",
        "pure",
    );
}

#[test]
fn replace_with_rejects_a_weakening_wrapper() {
    let mut s = build_standard_strategy();
    let result = s.replace_with("acl_check", |inner| {
        Box::new(WeakenedGate {
            inner,
            ignore_errors: true,
            match_modules: None,
            pure: None,
        })
    });
    assert_rejected(result, "acl_check", "ignore_errors");
    // A rejected wrap leaves the gate in place, not a placeholder.
    let gate = s
        .steps()
        .iter()
        .find(|st| st.name() == "acl_check")
        .expect("acl_check is still present");
    assert_eq!(gate.builtin_gate(), Some(BuiltinGate::Acl));
    assert!(!gate.ignore_errors());
}

#[test]
fn new_and_insert_reject_a_weakened_gate() {
    let step = WeakenedGate {
        ignore_errors: true,
        ..WeakenedGate::acl()
    };
    let err = ExecutionStrategy::new("custom", vec![Box::new(step)])
        .err()
        .expect("a strategy holding a weakened gate must not build");
    assert_eq!(err.code, ErrorCode::PipelineConfigurationError);

    let mut s = build_standard_strategy();
    s.remove("acl_check").expect("acl_check is removable");
    let step = WeakenedGate {
        ignore_errors: true,
        ..WeakenedGate::acl()
    };
    assert_rejected(
        s.insert_after("module_lookup", Box::new(step)),
        "acl_check",
        "ignore_errors",
    );
}

#[test]
fn configure_step_accepts_an_unweakened_gate() {
    let mut s = build_standard_strategy();
    s.configure_step("acl_check", Box::new(WeakenedGate::acl()))
        .expect("a gate with default fields is accepted");
    s.configure_step("approval_gate", Box::new(WeakenedGate::approval()))
        .expect("a gate with default fields is accepted");
}

#[test]
fn config_path_names_every_offending_key() {
    let err = build_strategy_from_config(&json!({
        "configure": {"approval_gate": {"ignore_errors": true, "match_modules": [], "pure": true}}
    }))
    .err()
    .expect("weakening keys are rejected");
    assert_eq!(err.code, ErrorCode::PipelineConfigurationError);
    for key in ["approval_gate", "ignore_errors", "match_modules", "pure"] {
        assert!(
            err.message.contains(key),
            "message names `{key}`: {}",
            err.message
        );
    }
}

/// An empty `match_modules` list matches nothing, so it exempts every module:
/// it is rejected like any other list.
#[test]
fn config_path_rejects_empty_match_modules_on_acl_check() {
    let err = build_strategy_from_config(&json!({
        "configure": {"acl_check": {"match_modules": []}}
    }))
    .err()
    .expect("an empty match_modules list is rejected");
    assert_eq!(err.code, ErrorCode::PipelineConfigurationError);
    assert!(err.message.contains("match_modules"), "{}", err.message);
}

/// The ACL gate is pure by default (`validate()` runs it), so `pure` is not a
/// weakening key there: either value is accepted.
#[test]
fn config_path_accepts_either_pure_value_on_acl_check() {
    for pure in [true, false] {
        let s = build_strategy_from_config(&json!({
            "configure": {"acl_check": {"pure": pure}}
        }))
        .unwrap_or_else(|e| panic!("pure: {pure} on acl_check is accepted: {}", e.message));
        let gate = s
            .steps()
            .iter()
            .find(|st| st.name() == "acl_check")
            .unwrap();
        assert_eq!(gate.pure(), pure);
        assert_eq!(gate.builtin_gate(), Some(BuiltinGate::Acl));
    }
}

#[test]
fn config_path_accepts_default_values_on_gates() {
    build_strategy_from_config(&json!({
        "configure": {
            "acl_check": {"ignore_errors": false},
            "approval_gate": {"pure": false, "ignore_errors": false, "match_modules": null}
        }
    }))
    .expect("default values weaken nothing");
}

/// A gate configured with an accepted field is still the gate: the overlay
/// forwards `builtin_gate()` (and its capability contract), so
/// `governance_state()` does not stop reporting it as wired.
#[test]
fn configured_gate_keeps_its_identity() {
    let s = build_strategy_from_config(&json!({
        "configure": {"acl_check": {"timeout_ms": 500}, "approval_gate": {"timeout_ms": 30000}}
    }))
    .expect("timeout_ms is configurable on a gate");
    let acl = s
        .steps()
        .iter()
        .find(|st| st.name() == "acl_check")
        .unwrap();
    let approval = s
        .steps()
        .iter()
        .find(|st| st.name() == "approval_gate")
        .unwrap();
    assert_eq!(acl.timeout_ms(), 500);
    assert_eq!(acl.builtin_gate(), Some(BuiltinGate::Acl));
    assert_eq!(acl.requires(), BuiltinACLCheck.requires());
    assert_eq!(approval.timeout_ms(), 30000);
    assert_eq!(approval.builtin_gate(), Some(BuiltinGate::Approval));
}
