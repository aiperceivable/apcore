//! Execution-time governance policy (apcore#76).
//!
//! An [`ExecutionPolicy`] lets a platform operator override a module's
//! governance annotations (`requires_approval`, `destructive`) at execution
//! time, without touching the module's source or its registration. It attaches
//! to the [`Executor`] and is consulted by the approval gate (pipeline step 5).
//!
//! This example shows the three controls, in the order an operator meets them:
//!
//! 1. `PolicyRule` — force approval for a namespace, then exempt a known-safe
//!    module inside it (the more specific rule wins; on a specificity tie the
//!    more restrictive rule wins).
//! 2. `gate_destructive` — gate anything annotated `destructive: true`, even
//!    when no rule names it. This closes the #76 footgun where a module is
//!    `destructive: true` but `requires_approval: false`.
//! 3. `strict` — fail CLOSED when approval is required but no
//!    `ApprovalHandler` is configured, instead of warning and proceeding.
//!
//! Cross-language parity: apcore-python `examples/execution_policy.py` and
//! apcore-typescript `examples/execution-policy.ts`.
//!
//! Run with: `cargo run --example execution_policy`

use std::collections::HashMap;
use std::sync::Arc;

use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::executor::Executor;
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use apcore::{ExecutionPolicy, PolicyRule};
use async_trait::async_trait;
use serde_json::{json, Value};

/// A trivial module; the point of this example is the governance layer above it.
#[derive(Debug)]
struct EchoModule;

#[async_trait]
impl Module for EchoModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "status": { "type": "string" } } })
    }
    fn description(&self) -> &'static str {
        "Echo module used to demonstrate execution policy"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "status": "executed" }))
    }
}

fn descriptor(module_id: &str, requires_approval: bool, destructive: bool) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: "Echo module used to demonstrate execution policy".to_string(),
        documentation: None,
        input_schema: json!({ "type": "object" }),
        output_schema: json!({ "type": "object" }),
        version: DEFAULT_MODULE_VERSION.to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations {
            requires_approval,
            destructive,
            ..ModuleAnnotations::default()
        }),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

/// Approval handler that prints what the gate asked for, then approves.
///
/// The `ApprovalRequest` carries the *effective* annotations — i.e. after the
/// policy has been applied — so the handler sees the platform's decision, not
/// the module author's declaration.
#[derive(Debug)]
struct PrintingApprovalHandler;

#[async_trait]
impl ApprovalHandler for PrintingApprovalHandler {
    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        println!(
            "    -> approval requested for '{}' (destructive={})",
            request.module_id, request.annotations.destructive
        );
        let mut result = ApprovalResult::default();
        result.status = "approved".to_string();
        Ok(result)
    }

    async fn check_approval(&self, _request_id: &str) -> Result<ApprovalResult, ModuleError> {
        let mut result = ApprovalResult::default();
        result.status = "approved".to_string();
        Ok(result)
    }
}

fn context() -> Context<Value> {
    Context::new(Identity::new(
        "operator-1".to_string(),
        "user".to_string(),
        vec!["ops".to_string()],
        HashMap::new(),
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Three modules with deliberately mismatched governance annotations.
    let registry = Arc::new(Registry::new());
    registry.register(
        "executor.payments.read_balance",
        Box::new(EchoModule),
        descriptor("executor.payments.read_balance", false, false),
    )?;
    registry.register(
        "executor.payments.refund",
        Box::new(EchoModule),
        descriptor("executor.payments.refund", false, false),
    )?;
    // The #76 footgun: destructive work that does NOT ask for approval.
    registry.register(
        "executor.orders.delete_order",
        Box::new(EchoModule),
        descriptor("executor.orders.delete_order", false, true),
    )?;

    let policy = ExecutionPolicy::new(vec![
        // Force approval across the whole payments namespace...
        PolicyRule::new("executor.payments.*")?
            .with_requires_approval(true)
            .with_reason("PCI scope — platform requires human sign-off"),
        // ...but exempt the read-only balance lookup. The more specific
        // pattern wins (Algorithm A10 specificity scoring).
        PolicyRule::new("executor.payments.read_balance")?
            .with_requires_approval(false)
            .with_reason("read-only balance lookup"),
    ])
    // No rule names `executor.orders.delete_order`, but it is annotated
    // destructive, so this flag gates it anyway.
    .with_gate_destructive(true)
    // Fail closed if approval is required and no handler is configured.
    .with_strict(true);

    let mut executor = Executor::new(Arc::clone(&registry), Config::from_defaults());
    executor.set_policy(Some(policy));
    executor.set_approval_handler(Box::new(PrintingApprovalHandler));

    let ctx = context();
    for module_id in [
        "executor.payments.read_balance", // exempted by the specific rule
        "executor.payments.refund",       // gated by the namespace rule
        "executor.orders.delete_order",   // gated by gate_destructive
    ] {
        println!("calling {module_id}");
        let output = executor
            .call(module_id, json!({}), Some(&ctx), None)
            .await?;
        println!("    result: {output}");
    }

    // `strict = true` with NO approval handler fails closed rather than
    // silently proceeding — a misconfigured governance control must never
    // read as "allow".
    let mut unguarded = Executor::new(Arc::clone(&registry), Config::from_defaults());
    unguarded.set_policy(Some(
        ExecutionPolicy::new(vec![PolicyRule::new("executor.payments.*")?
            .with_requires_approval(true)
            .with_reason("PCI scope")])
        .with_strict(true),
    ));
    println!("calling executor.payments.refund with strict policy and no approval handler");
    match unguarded
        .call("executor.payments.refund", json!({}), Some(&ctx), None)
        .await
    {
        Ok(output) => println!("    unexpectedly allowed: {output}"),
        Err(e) => println!("    correctly failed closed: [{:?}] {}", e.code, e.message),
    }

    Ok(())
}
