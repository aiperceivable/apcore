//! D-129 regression tests (PROTOCOL_SPEC 6.6.5.5): a provider given to an
//! `Executor` reaches the built-in gate of a strategy passed as an instance.
//!
//! The defect D-129 names was found in apcore-python: an executor built with a
//! deny-all ACL and a pre-built standard strategy ran every call while
//! `governance_state()` reported the ACL configured and the gate wired. This
//! SDK is not affected — its gate steps hold no provider and read the
//! executor's, injected per call — and these tests pin that for each of the
//! three providers, on the exact shape of the original repro.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::acl::ACL;
use apcore::approval::AlwaysDenyHandler;
use apcore::builtin_steps::build_standard_strategy;
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use apcore::{ExecutionPolicy, Executor};
use async_trait::async_trait;
use serde_json::{json, Value};

const MODULE_ID: &str = "demo.target";

#[derive(Debug)]
struct Target {
    requires_approval: bool,
}

#[async_trait]
impl Module for Target {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "trivial target"
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: self.requires_approval,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"ok": true}))
    }
}

/// An executor running a pre-built standard strategy instance.
fn executor_on_instance(requires_approval: bool) -> Executor {
    let reg = Arc::new(Registry::new());
    let descriptor = ModuleDescriptor {
        module_id: MODULE_ID.to_string(),
        name: None,
        description: "trivial target".to_string(),
        documentation: None,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        version: DEFAULT_MODULE_VERSION.to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations {
            requires_approval,
            ..ModuleAnnotations::default()
        }),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    reg.register(
        MODULE_ID,
        Box::new(Target { requires_approval }),
        descriptor,
    )
    .expect("register");
    Executor::with_strategy(reg, Arc::new(Config::default()), build_standard_strategy())
}

#[tokio::test]
async fn instance_strategy_enforces_executor_acl() {
    let mut executor = executor_on_instance(false);
    executor.set_acl(ACL::new(vec![], "deny", None));

    let err = executor
        .call(MODULE_ID, json!({}), None, None)
        .await
        .expect_err("a deny-all ACL must stop the call");
    assert_eq!(err.code, ErrorCode::ACLDenied);

    let state = executor.governance_state();
    assert!(state.acl_configured);
    assert!(state.builtin_acl_gate_wired);
}

#[tokio::test]
async fn instance_strategy_enforces_executor_approval_handler() {
    let mut executor = executor_on_instance(true);
    executor.set_approval_handler(Box::new(AlwaysDenyHandler));

    let err = executor
        .call(MODULE_ID, json!({}), None, None)
        .await
        .expect_err("an always-deny handler must stop the call");
    assert_eq!(err.code, ErrorCode::ApprovalDenied);

    let state = executor.governance_state();
    assert!(state.approval_handler_configured);
    assert!(state.builtin_approval_gate_wired);
}

#[tokio::test]
async fn instance_strategy_enforces_executor_strict_policy() {
    let mut executor = executor_on_instance(true);
    executor.set_policy(Some(ExecutionPolicy::new(vec![]).with_strict(true)));

    let err = executor
        .call(MODULE_ID, json!({}), None, None)
        .await
        .expect_err("strict policy with no handler must fail closed");
    assert_eq!(err.code, ErrorCode::ApprovalDenied);

    let state = executor.governance_state();
    assert!(state.policy_strict);
    assert!(state.builtin_approval_gate_wired);
}
