// Regression test for decision D-03 (docs/spec/2026-05-decision-log.md,
// spec v1.32.0 §7.3.1): `ApprovalRequest` MUST carry `caller_id` and `action`,
// populated by `BuiltinApprovalGate` from the same Context and module_id
// already in scope at that call site — `caller_id = context.caller_id`
// (None/null for a top-level call, per §5.7, with no "@external" ACL-only
// substitution), `action = module_id`.
//
// Mirrors conformance/fixtures/approval_request_fields.json in the apcore
// repo (both test cases: caller_id null on a top-level call, and caller_id +
// action populated on a nested call), and follows the same
// PipelineContext + BuiltinApprovalGate.execute() pattern as
// test_approval_request_live_module_metadata.rs.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::{Module, ModuleAnnotations};
use apcore::pipeline::{PipelineContext, Step};
use apcore::registry::registry::Registry;
use apcore::BuiltinApprovalGate;

/// A trivial module requiring approval.
#[derive(Debug)]
struct GatedModule;

#[async_trait]
impl Module for GatedModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &str {
        "gated module for D-03 caller_id/action regression test"
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: true,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

/// Build an "approved" result (`ApprovalResult` is `#[non_exhaustive]`, so it
/// cannot be struct-literal-constructed from outside the crate).
fn approved_result() -> ApprovalResult {
    let mut result = ApprovalResult::default();
    result.status = "approved".to_string();
    result.approved_by = Some("recorder".to_string());
    result
}

/// Approval handler that records the request it received and auto-approves.
#[derive(Debug, Default)]
struct RecordingHandler {
    captured: Mutex<Option<ApprovalRequest>>,
}

#[async_trait]
impl ApprovalHandler for RecordingHandler {
    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        *self.captured.lock().expect("lock poisoned") = Some(request.clone());
        Ok(approved_result())
    }

    async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
        Ok(approved_result())
    }
}

/// Run `BuiltinApprovalGate` for `module_id` under `context` and return the
/// `ApprovalRequest` the handler captured.
async fn run_gate_and_capture(module_id: &str, context: Context<Value>) -> ApprovalRequest {
    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(GatedModule),
            apcore::registry::registry::ModuleDescriptor {
                module_id: module_id.to_string(),
                name: None,
                description: String::new(),
                documentation: None,
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "object"}),
                version: apcore::registry::registry::DEFAULT_MODULE_VERSION.to_string(),
                tags: vec![],
                annotations: Some(ModuleAnnotations {
                    requires_approval: true,
                    ..ModuleAnnotations::default()
                }),
                examples: vec![],
                metadata: std::collections::HashMap::new(),
                display: None,
                sunset_date: None,
                dependencies: vec![],
                enabled: true,
            },
        )
        .expect("register module requiring approval");

    let handler = Arc::new(RecordingHandler::default());

    let mut ctx = PipelineContext::new(module_id, json!({}), context, "standard");
    ctx.registry = Some(registry);
    ctx.approval_handler = Some(handler.clone());

    let result = BuiltinApprovalGate.execute(&mut ctx).await;
    assert!(result.is_ok(), "approved gate should continue: {result:?}");

    let captured = handler
        .captured
        .lock()
        .expect("lock poisoned")
        .clone()
        .expect("handler must have received an ApprovalRequest");
    captured
}

// Fixture case "caller_id_null_on_top_level_call": module.target is invoked
// directly (no caller module) — caller_id is null, action is still populated.
#[tokio::test]
async fn test_approval_request_caller_id_null_on_top_level_call() {
    let module_id = "module.target";
    let context = Context::<Value>::anonymous();

    let captured = run_gate_and_capture(module_id, context).await;

    assert_eq!(
        captured.caller_id, None,
        "a top-level call's ApprovalRequest.caller_id must be None, matching \
         Context.caller_id (§5.7) with no \"@external\" substitution"
    );
    assert_eq!(
        captured.action, module_id,
        "action must equal the invoked module's id even with no caller"
    );
}

// Fixture case "caller_id_and_action_populated_on_nested_call": module.caller
// invokes module.target, which requires approval — the handler-visible
// request names both.
#[tokio::test]
async fn test_approval_request_caller_id_and_action_populated_on_nested_call() {
    let caller_id = "module.caller";
    let target_id = "module.target";

    // A root context whose call_chain's last entry is `caller_id`, so that
    // `.child(target_id)` (the same mechanism the Executor uses for a nested
    // call) resolves `caller_id` from `call_chain.last()` per Context::child.
    let root = Context::<Value>::anonymous();
    let caller_ctx = root.child(caller_id);
    let nested_ctx = caller_ctx.child(target_id);
    assert_eq!(
        nested_ctx.caller_id.as_deref(),
        Some(caller_id),
        "test setup: nested_ctx.caller_id must resolve to the caller module"
    );

    let captured = run_gate_and_capture(target_id, nested_ctx).await;

    assert_eq!(
        captured.caller_id.as_deref(),
        Some(caller_id),
        "caller_id must be read straight off context.caller_id"
    );
    assert_eq!(
        captured.action, target_id,
        "action must equal module_id (the invoked target), not a separate label"
    );
}
