//! Regression: `Registry::register_module` (2-arg) and `register_versioned`
//! must derive the descriptor's annotations from the module's `annotations()`
//! trait method, NOT discard them with `ModuleAnnotations::default()`.
//!
//! Before the fix, a sensitive module declaring `requires_approval = true`
//! registered via the canonical 2-arg `register_module` ended up with a
//! descriptor carrying `requires_approval = false`. Since the approval gate
//! (`builtin_steps.rs`) decides whether to fire from
//! `registry.get_definition().annotations.requires_approval`, the gate was
//! SILENTLY BYPASSED — a security-relevant divergence from apcore-python /
//! apcore-typescript, whose `register` paths derive annotations from the module.

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::approval::AlwaysDenyHandler;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::Registry;
use apcore::{Config, Executor};

/// A sensitive tool that declares it requires approval via the trait method.
#[derive(Debug)]
struct SensitiveModule;

#[async_trait]
impl Module for SensitiveModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "Delete a record (sensitive)"
    }
    fn tags(&self) -> Vec<String> {
        vec!["sensitive".to_string()]
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: true,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "deleted": true }))
    }
}

#[test]
fn register_module_preserves_module_annotations() {
    let registry = Registry::new();
    registry
        .register_module("executor.crm.delete", Box::new(SensitiveModule))
        .expect("register_module");

    let def = registry
        .get_definition("executor.crm.delete")
        .expect("get_definition ok")
        .expect("module registered");
    let annotations = def.annotations.expect("annotations present");
    assert!(
        annotations.requires_approval,
        "register_module must derive requires_approval from module.annotations()"
    );
    assert!(def.tags.contains(&"sensitive".to_string()));
}

#[test]
fn register_versioned_preserves_module_annotations() {
    let registry = Registry::new();
    registry
        .register_versioned("executor.crm.delete", Box::new(SensitiveModule), None, None)
        .expect("register_versioned");

    let def = registry
        .get_definition("executor.crm.delete")
        .expect("get_definition ok")
        .expect("module registered");
    assert!(
        def.annotations
            .expect("annotations present")
            .requires_approval,
        "register_versioned must derive requires_approval from module.annotations()"
    );
}

/// End-to-end proof the approval gate fires for a module registered via the
/// 2-arg `register_module` — i.e. the gate is no longer silently bypassed.
#[tokio::test]
async fn approval_gate_fires_after_register_module() {
    let registry = Registry::new();
    registry
        .register_module("executor.crm.delete", Box::new(SensitiveModule))
        .expect("register_module");

    let mut executor = Executor::new(registry, Config::default());
    executor.set_approval_handler(Box::new(AlwaysDenyHandler));

    let result = executor
        .call("executor.crm.delete", json!({}), None, None)
        .await;

    let err = result.expect_err("approval gate must deny the sensitive call");
    assert_eq!(
        err.code,
        ErrorCode::ApprovalDenied,
        "gate must fire with ApprovalDenied, not silently execute"
    );
}
