//! Cross-language conformance driver for `gate_provider_binding.json`
//! (PROTOCOL_SPEC 6.6.5.5 — providers reach the gate they configure, D-129).
//!
//! Fixture source: apcore/conformance/fixtures/gate_provider_binding.json.
//!
//! Every case constructs a real `Executor`, registers `demo.target`, makes one
//! real call as `@external` and then reads `governance_state()` — per
//! `driver_contract.path`, asserting the accessor alone would miss the defect
//! this fixture pins (a call that RAN while the accessor reported a gate).
//!
//! # How this SDK binds providers
//!
//! The built-in gates (`BuiltinACLCheck`, `BuiltinApprovalGate`) are unit
//! structs that hold no provider. The executor injects its ACL, approval
//! handler and policy into every call's `PipelineContext`
//! (`Executor::inject_resources`), and the gates read them from there. So a
//! strategy instance — built by name, by default or by hand — can never run a
//! gate that holds something other than what the executor holds, and
//! `governance_state()` reading the executor fields IS reading what the
//! running gate enforces.
//!
//! # The `instance:own_acl` form
//!
//! That same design makes `instance:own_acl` inexpressible: the public builder
//! (`build_standard_strategy`) takes no provider and the gate step has no field
//! to hold one. The two cases using that form are not skipped silently — the
//! driver asserts the reason (`BuiltinACLCheck` is zero-sized, so it cannot
//! carry an ACL) and fails the moment it stops being true, at which point the
//! form becomes expressible and this driver must be taught it.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::acl::{ACLRule, ACL};
use apcore::approval::AlwaysDenyHandler;
use apcore::builtin_steps::{build_standard_strategy, BuiltinACLCheck, BuiltinApprovalGate};
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use apcore::{ExecutionPolicy, Executor, GovernanceState};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

const MODULE_ID: &str = "demo.target";

/// Cases whose `strategy_form` cannot be built in this SDK — see the module
/// docs. Each entry is checked to still exist in the fixture.
const INEXPRESSIBLE_FORMS: &[&str] = &["instance:own_acl"];

fn fixture() -> Value {
    let path = find_fixtures_root().join("gate_provider_binding.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("gate_provider_binding.json parses")
}

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
        "trivial conformance target"
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

fn registry(requires_approval: bool) -> Arc<Registry> {
    let reg = Arc::new(Registry::new());
    let descriptor = ModuleDescriptor {
        module_id: MODULE_ID.to_string(),
        name: None,
        description: "trivial conformance target".to_string(),
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
    .expect("register demo.target");
    reg
}

fn acl(name: &Value) -> Option<ACL> {
    match name.as_str() {
        None => None,
        Some("deny_all") => Some(ACL::new(vec![], "deny", None)),
        Some("allow_all") => Some(ACL::new(
            vec![ACLRule::new(
                vec!["*".to_string()],
                vec!["*".to_string()],
                "allow",
            )],
            "deny",
            None,
        )),
        Some(other) => panic!("fixture names ACL `{other}` this driver does not build"),
    }
}

fn build(setup: &Value) -> Executor {
    let reg = registry(
        setup["requires_approval"]
            .as_bool()
            .expect("requires_approval"),
    );
    let config = Arc::new(Config::default());
    let executor_acl = acl(&setup["executor_acl"]);
    let handler: Option<Box<dyn apcore::approval::ApprovalHandler>> =
        match setup["approval_handler"].as_str() {
            None => None,
            Some("always_deny") => Some(Box::new(AlwaysDenyHandler)),
            Some(other) => panic!("fixture names handler `{other}` this driver does not build"),
        };

    let form = setup["strategy_form"].as_str().expect("strategy_form");
    let mut executor =
        match form {
            // Through the constructor: the provider-taking entry point.
            "default" => Executor::with_options(reg, config, None, executor_acl.clone(), None),
            "preset:standard" => Executor::with_strategy_name(reg, config, "standard")
                .expect("standard preset exists"),
            "preset:internal" => Executor::with_strategy_name(reg, config, "internal")
                .expect("internal preset exists"),
            "instance:bare" => Executor::with_strategy(reg, config, build_standard_strategy()),
            other => panic!("fixture names strategy_form `{other}` this driver does not build"),
        };
    if form != "default" {
        if let Some(a) = executor_acl {
            executor.set_acl(a);
        }
    }
    if let Some(h) = handler {
        executor.set_approval_handler(h);
    }
    if setup["policy_strict"].as_bool().expect("policy_strict") {
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
        other => panic!("fixture asserts unknown governance field `{other}`"),
    }
}

async fn run_case(tc: &Value) {
    let id = tc["id"].as_str().expect("every case needs an id");
    let setup = &tc["setup"];
    let expected = &tc["expected"];
    let executor = build(setup);

    // `@external`: no Context supplied, so no caller_id.
    let outcome = executor.call(MODULE_ID, json!({}), None, None).await;
    let want_call = expected["call"]
        .as_str()
        .expect("expected.call is a string");
    match (&outcome, want_call) {
        (Ok(out), "ok") => assert_eq!(out, &json!({"ok": true}), "[{id}] call output"),
        (Ok(out), code) => panic!("[{id}] call RAN (returned {out}) — fixture expects {code}"),
        (Err(e), "ok") => panic!(
            "[{id}] call failed with {}: {}",
            e.code.wire_str(),
            e.message
        ),
        (Err(e), code) => assert_eq!(
            e.code.wire_str(),
            code,
            "[{id}] call error_code (message: {})",
            e.message
        ),
    }

    let state = executor.governance_state();
    for (name, want) in expected["governance"]
        .as_object()
        .expect("expected.governance is an object")
    {
        let want = want.as_bool().expect("governance values are booleans");
        assert_eq!(
            field(&state, name),
            want,
            "[{id}] governance_state().{name}"
        );
    }
    for key in expected.as_object().expect("expected is an object").keys() {
        assert!(
            key == "call" || key == "governance",
            "[{id}] fixture grew expectation `{key}` this driver does not check — teach it"
        );
    }
}

#[tokio::test]
async fn conformance_gate_provider_binding() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");

    let mut ran = 0;
    let mut inexpressible = Vec::new();
    for tc in cases {
        let form = tc["setup"]["strategy_form"]
            .as_str()
            .expect("strategy_form");
        if INEXPRESSIBLE_FORMS.contains(&form) {
            inexpressible.push(tc["id"].as_str().expect("id").to_string());
            continue;
        }
        run_case(tc).await;
        ran += 1;
    }
    assert!(
        ran > 0,
        "every case was held out — the driver asserts nothing"
    );
    for form in INEXPRESSIBLE_FORMS {
        assert!(
            cases.iter().any(|tc| tc["setup"]["strategy_form"] == *form),
            "INEXPRESSIBLE_FORMS lists `{form}`, which the fixture no longer uses — drop it"
        );
    }
    if !inexpressible.is_empty() {
        eprintln!(
            "gate_provider_binding: {} case(s) use a strategy form this SDK cannot build \
             (the gate step holds no provider): {}",
            inexpressible.len(),
            inexpressible.join(", ")
        );
    }
}

/// The reason `instance:own_acl` is held out, asserted rather than assumed: a
/// zero-sized gate step carries no ACL, approval handler or policy, so no
/// strategy instance can hold a provider the executor does not inject. If
/// either gate ever grows a field, this fails — and the form becomes
/// expressible and must be driven.
#[test]
fn gate_steps_cannot_hold_a_provider() {
    assert_eq!(std::mem::size_of::<BuiltinACLCheck>(), 0);
    assert_eq!(std::mem::size_of::<BuiltinApprovalGate>(), 0);
}
