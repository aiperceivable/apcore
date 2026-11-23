//! `Executor::governance_state()` — configured vs. actually wired.
//!
//! PROTOCOL_SPEC 6.6.5. The accessor exists because `acl.is_some()` is not the
//! answer to "what is gating this registry": the gates are pipeline steps, and
//! the `internal` / `testing` / `minimal` presets remove them. apcore-rust
//! already exposed `acl`, `approval_handler` and `policy` as public fields —
//! raw state with no defined semantics, which is precisely what an adapter
//! would read to reach the wrong conclusion.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::config::Config;
use apcore::context::Context;
use apcore::module::{Module, ModuleAnnotations};
use apcore::pipeline::{ExecutionStrategy, PipelineContext, Step, StepResult};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::{ExecutionPolicy, Executor, ACL};
use serde_json::json;

fn registry_with(control: &[(&str, bool)], read: bool) -> Arc<Registry> {
    let registry = Arc::new(Registry::new());
    for (id, requires_approval) in control {
        register(&registry, id, *requires_approval);
    }
    if read {
        register(&registry, "system.health.summary", false);
    }
    registry
}

fn register(registry: &Arc<Registry>, id: &str, requires_approval: bool) {
    struct Dummy;
    #[async_trait::async_trait]
    impl Module for Dummy {
        fn description(&self) -> &'static str {
            "dummy"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }
        fn output_schema(&self) -> serde_json::Value {
            json!({})
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, apcore::errors::ModuleError> {
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

fn executor(registry: Arc<Registry>) -> Executor {
    Executor::new(registry, Arc::new(Config::default()))
}

fn deny_all() -> ACL {
    ACL::new(vec![], "deny", None)
}

#[test]
fn no_system_modules_is_not_an_unprotected_surface() {
    let state = executor(Arc::new(Registry::new())).governance_state();
    assert!(!state.control_modules_registered);
    assert!(!state.read_modules_registered);
    assert!(!state.unprotected_control_surface);
}

#[test]
fn read_modules_only_is_not_a_control_surface() {
    // Six read-only modules and no ACL is an information-disclosure question,
    // not a control-plane one. The flag must not fire where there is no write
    // surface at all.
    let state = executor(registry_with(&[], true)).governance_state();
    assert!(state.read_modules_registered);
    assert!(!state.control_modules_registered);
    assert!(!state.unprotected_control_surface);
}

#[test]
fn control_modules_with_no_gates() {
    let state = executor(registry_with(
        &[("system.control.reload_module", true)],
        true,
    ))
    .governance_state();
    assert!(state.control_modules_registered);
    assert!(state.unprotected_control_surface);
}

#[test]
fn acl_configured_and_wired_on_the_standard_strategy() {
    let mut ex = executor(registry_with(
        &[("system.control.reload_module", true)],
        true,
    ));
    ex.set_acl(deny_all());
    let state = ex.governance_state();
    assert!(state.acl_configured);
    assert!(state.builtin_acl_gate_wired);
    assert!(!state.unprotected_control_surface);
}

#[test]
fn acl_configured_but_the_internal_strategy_has_no_acl_step() {
    // The case the accessor exists for. `acl.is_some()` reports this as
    // protected; it is not.
    let registry = registry_with(&[("system.control.reload_module", true)], true);
    let mut ex = Executor::with_strategy_name(registry, Arc::new(Config::default()), "internal")
        .expect("internal preset must exist");
    ex.set_acl(deny_all());
    let state = ex.governance_state();
    assert!(state.acl_configured);
    assert!(!state.builtin_acl_gate_wired);
    assert!(state.unprotected_control_surface);
}

#[test]
fn a_custom_step_named_acl_check_is_not_the_builtin_gate() {
    // PROTOCOL_SPEC 6.6.5.2. A name test would set the flag here, and a false
    // `builtin_acl_gate_wired` is the one direction that must never happen: it
    // reports a gate that is not there. `Step::builtin_gate` defaults to
    // `None`, so this step cannot claim to be the gate.
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

    let registry = registry_with(&[("system.control.reload_module", true)], true);
    let strategy = ExecutionStrategy::new("lookalike", vec![Box::new(LookalikeAclCheck)])
        .expect("strategy should build");
    let mut ex = Executor::with_strategy(registry, Arc::new(Config::default()), strategy);
    ex.set_acl(deny_all());

    let state = ex.governance_state();
    assert!(state.acl_configured);
    assert!(
        !state.builtin_acl_gate_wired,
        "a step merely NAMED acl_check must not count as the gate"
    );
    assert!(state.unprotected_control_surface);
}

#[test]
fn a_handler_does_not_gate_an_unannotated_control_module() {
    // PROTOCOL_SPEC 6.6.5.1.1 — approval_gate is per-module conditional. It
    // returns before consulting the handler when the module does not declare
    // requires_approval, so a wired gate plus a handler gates nothing. The
    // v1.15.0 formula answered `false` here.
    let registry = registry_with(&[("system.control.custom_thing", false)], true);
    let mut ex = executor(registry);
    ex.set_approval_handler(Box::new(NeverCalledHandler));

    let state = ex.governance_state();
    assert!(state.approval_handler_configured);
    assert!(state.builtin_approval_gate_wired);
    assert!(!state.all_control_modules_require_approval);
    assert!(state.unprotected_control_surface);
}

#[test]
fn strict_policy_does_not_gate_an_unannotated_control_module() {
    let registry = registry_with(&[("system.control.custom_thing", false)], true);
    let mut ex = executor(registry);
    ex.set_policy(Some(ExecutionPolicy::new(vec![]).with_strict(true)));

    let state = ex.governance_state();
    assert!(state.policy_strict);
    assert!(!state.all_control_modules_require_approval);
    assert!(state.unprotected_control_surface);
}

#[test]
fn all_annotated_with_a_handler_is_gated() {
    let registry = registry_with(&[("system.control.reload_module", true)], true);
    let mut ex = executor(registry);
    ex.set_approval_handler(Box::new(NeverCalledHandler));

    let state = ex.governance_state();
    assert!(state.all_control_modules_require_approval);
    assert!(!state.unprotected_control_surface);
}

#[test]
fn one_unannotated_control_module_is_a_hole_in_the_surface() {
    let registry = registry_with(
        &[
            ("system.control.reload_module", true),
            ("system.control.custom_thing", false),
        ],
        true,
    );
    let mut ex = executor(registry);
    ex.set_approval_handler(Box::new(NeverCalledHandler));

    let state = ex.governance_state();
    assert!(!state.all_control_modules_require_approval);
    assert!(state.unprotected_control_surface);
}

#[test]
fn the_accessor_is_a_pure_read() {
    // PROTOCOL_SPEC 6.6.5.3: never enforces, warns, panics or mutates.
    let ex = executor(registry_with(
        &[("system.control.reload_module", true)],
        true,
    ));
    assert_eq!(ex.governance_state(), ex.governance_state());
}

#[test]
fn the_accessor_is_live_not_cached() {
    let mut ex = executor(registry_with(
        &[("system.control.reload_module", true)],
        true,
    ));
    let before = ex.governance_state();
    ex.set_acl(deny_all());
    let after = ex.governance_state();
    assert!(!before.acl_configured);
    assert!(after.acl_configured);
}

/// Panics if invoked. `governance_state()` is a pure read (6.6.5.3): it reports
/// that a handler is attached, and must never consult it.
#[derive(Debug)]
struct NeverCalledHandler;

#[async_trait::async_trait]
impl apcore::approval::ApprovalHandler for NeverCalledHandler {
    async fn request_approval(
        &self,
        _request: &apcore::approval::ApprovalRequest,
    ) -> Result<apcore::approval::ApprovalResult, apcore::errors::ModuleError> {
        panic!("governance_state must not invoke the handler");
    }

    async fn check_approval(
        &self,
        _approval_id: &str,
    ) -> Result<apcore::approval::ApprovalResult, apcore::errors::ModuleError> {
        panic!("governance_state must not invoke the handler");
    }
}
