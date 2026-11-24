//! Cross-language conformance driver for `governance_state.json`
//! (PROTOCOL_SPEC 6.6.5 — configured vs. actually wired).
//!
//! Fixture source: apcore/conformance/fixtures/governance_state.json (canonical).
//!
//! Drives the real `Executor::governance_state()` on a real Registry and a real
//! strategy. All nine booleans are asserted as the SDK returned them — the
//! derived flag included, per
//! `driver_contract.derived_flag_is_asserted_not_recomputed`: a driver that
//! recomputes it from the other eight is green against an implementation that
//! never computes it at all.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::config::Config;
use apcore::context::Context;
use apcore::module::{Module, ModuleAnnotations};
use apcore::pipeline::{ExecutionStrategy, PipelineContext, Step, StepResult};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{ExecutionPolicy, Executor, GovernanceState, ACL};
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

/// 6.6.5.3 — the accessor reports a handler is attached; it must never consult it.
#[derive(Debug)]
struct NeverCalledHandler;

#[async_trait::async_trait]
impl apcore::approval::ApprovalHandler for NeverCalledHandler {
    async fn request_approval(
        &self,
        _request: &apcore::approval::ApprovalRequest,
    ) -> Result<apcore::approval::ApprovalResult, apcore::errors::ModuleError> {
        panic!("governance_state() must not invoke the approval handler");
    }
    async fn check_approval(
        &self,
        _approval_id: &str,
    ) -> Result<apcore::approval::ApprovalResult, apcore::errors::ModuleError> {
        panic!("governance_state() must not invoke the approval handler");
    }
}

/// A step whose NAME is `acl_check` and whose capability marker is `None`.
struct LookalikeAclCheck;

#[async_trait::async_trait]
impl Step for LookalikeAclCheck {
    fn name(&self) -> &str {
        "acl_check"
    }
    fn description(&self) -> &str {
        "looks like the ACL gate, consults no ACL"
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(
        &self,
        _ctx: &mut PipelineContext,
    ) -> Result<StepResult, apcore::errors::ModuleError> {
        Ok(StepResult::continue_step())
    }
}

fn register(registry: &Arc<Registry>, id: &str, requires_approval: bool) {
    struct Dummy;
    #[async_trait::async_trait]
    impl Module for Dummy {
        fn description(&self) -> &'static str {
            "conformance control module"
        }
        fn input_schema(&self) -> Value {
            json!({})
        }
        fn output_schema(&self) -> Value {
            json!({})
        }
        async fn execute(
            &self,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, apcore::errors::ModuleError> {
            Ok(json!({}))
        }
    }

    let mut annotations = ModuleAnnotations::default();
    annotations.requires_approval = requires_approval;
    let descriptor = ModuleDescriptor {
        module_id: id.to_string(),
        name: None,
        description: String::new(),
        documentation: None,
        input_schema: json!({}),
        output_schema: json!({}),
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
    registry
        .register_internal(id, Box::new(Dummy), descriptor)
        .expect("register_internal should succeed");
}

fn build(setup: &Value) -> Executor {
    let registry = Arc::new(Registry::new());
    for entry in setup["control_modules"]
        .as_array()
        .expect("control_modules")
    {
        register(
            &registry,
            entry["module_id"].as_str().expect("module_id"),
            entry["requires_approval"]
                .as_bool()
                .expect("requires_approval"),
        );
    }
    if setup["read_modules"].as_bool().unwrap_or(false) {
        register(&registry, "system.health.summary", false);
    }

    let config = Arc::new(Config::default());
    let strategy = setup["strategy"].as_str().expect("strategy");
    let mut executor = match strategy {
        "standard" => Executor::new(registry, config),
        "lookalike_acl_check" => {
            let s =
                ExecutionStrategy::new("lookalike_acl_check", vec![Box::new(LookalikeAclCheck)])
                    .expect("strategy should build");
            Executor::with_strategy(registry, config, s)
        }
        other => Executor::with_strategy_name(registry, config, other)
            .unwrap_or_else(|_| panic!("preset {other} must exist")),
    };

    if setup["acl_attached"].as_bool().unwrap_or(false) {
        executor.set_acl(ACL::new(vec![], "deny", None));
    }
    if setup["approval_handler_attached"]
        .as_bool()
        .unwrap_or(false)
    {
        executor.set_approval_handler(Box::new(NeverCalledHandler));
    }
    if setup["policy_strict"].as_bool().unwrap_or(false) {
        executor.set_policy(Some(ExecutionPolicy::new(vec![]).with_strict(true)));
    }
    executor
}

fn field(state: &GovernanceState, name: &str) -> bool {
    match name {
        "control_modules_registered" => state.control_modules_registered,
        "read_modules_registered" => state.read_modules_registered,
        "acl_configured" => state.acl_configured,
        "builtin_acl_gate_wired" => state.builtin_acl_gate_wired,
        "approval_handler_configured" => state.approval_handler_configured,
        "builtin_approval_gate_wired" => state.builtin_approval_gate_wired,
        "policy_strict" => state.policy_strict,
        "all_control_modules_require_approval" => state.all_control_modules_require_approval,
        "unprotected_control_surface" => state.unprotected_control_surface,
        other => panic!("fixture declares unknown field {other:?}"),
    }
}

#[test]
fn conformance_governance_state() {
    let fixture = load_fixture("governance_state");
    let cases = fixture["test_cases"].as_array().expect("test_cases");
    assert!(!cases.is_empty(), "fixture must declare cases");

    for case in cases {
        let id = case["id"].as_str().expect("id");
        let note = case["note"].as_str().unwrap_or("");
        let state = build(&case["setup"]).governance_state();

        for (name, expected) in case["expected"].as_object().expect("expected") {
            let want = expected.as_bool().expect("expected values are booleans");
            assert_eq!(
                field(&state, name),
                want,
                "{id}: {name} is {}, fixture expects {want}\n  {note}",
                field(&state, name)
            );
        }
    }
}

#[test]
fn driver_contract_purity() {
    // Two reads are equal and the handler is never invoked (it panics if it is).
    let setup = json!({
        "control_modules": [{"module_id": "system.control.reload_module", "requires_approval": true}],
        "read_modules": true,
        "strategy": "standard",
        "acl_attached": false,
        "approval_handler_attached": true,
        "policy_strict": false
    });
    let executor = build(&setup);
    assert_eq!(executor.governance_state(), executor.governance_state());
}

#[test]
fn driver_contract_liveness() {
    // A cached value passes every static case.
    let setup = json!({
        "control_modules": [{"module_id": "system.control.reload_module", "requires_approval": true}],
        "read_modules": true,
        "strategy": "standard",
        "acl_attached": false,
        "approval_handler_attached": false,
        "policy_strict": false
    });
    let mut executor = build(&setup);
    let before = executor.governance_state();
    executor.set_acl(ACL::new(vec![], "deny", None));
    let after = executor.governance_state();

    assert!(!before.acl_configured);
    assert!(after.acl_configured);
}
