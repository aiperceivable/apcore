// Regression test for D10-001: BuiltinApprovalGate must emit an
// `approval_decision` audit/observability event when an approval is resolved.
//
// Cross-SDK parity with apcore-python (`_emit_audit`) and apcore-typescript:
// when a tracing span stack is present in `ctx.data` (TRACING_SPANS), the gate
// appends an `approval_decision` event carrying
// `{module_id, status, approved_by, reason, approval_id}` to the active span.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::{Module, ModuleAnnotations};
use apcore::pipeline::{PipelineContext, Step};
use apcore::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use apcore::{AutoApproveHandler, BuiltinApprovalGate, TRACING_SPANS};

#[derive(Debug)]
struct NoopModule;

#[async_trait]
impl Module for NoopModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "noop module that requires approval"
    }
    /// Declared on the INSTANCE: PROTOCOL_SPEC §7.4 Step 5 gates on
    /// `module.annotations`, so this is what makes the gate fire. The
    /// descriptor below carries the same value for introspection.
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

fn descriptor_requiring_approval(module_id: &str, module: &NoopModule) -> ModuleDescriptor {
    let annotations = ModuleAnnotations {
        requires_approval: true,
        ..ModuleAnnotations::default()
    };
    ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: module.description().to_string(),
        documentation: None,
        input_schema: module.input_schema(),
        output_schema: module.output_schema(),
        version: DEFAULT_MODULE_VERSION.to_string(),
        tags: vec![],
        annotations: Some(annotations),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

#[tokio::test]
async fn test_approval_gate_emits_approval_decision_span_event() {
    let module_id = "needs.approval";

    let registry = Arc::new(Registry::new());
    let descriptor = descriptor_requiring_approval(module_id, &NoopModule);
    registry
        .register(module_id, Box::new(NoopModule), descriptor)
        .expect("register module requiring approval");

    let context = Context::<Value>::anonymous();

    // Seed an active tracing span stack in context data, mirroring how the
    // tracing middleware would populate it cross-SDK.
    TRACING_SPANS.set(
        &context,
        vec![json!({
            "name": "apcore.module.execute",
            "events": [],
        })],
    );

    let mut ctx = PipelineContext::new(module_id, json!({}), context, "standard");
    ctx.registry = Some(registry);
    ctx.approval_handler = Some(Arc::new(AutoApproveHandler));

    let result = BuiltinApprovalGate.execute(&mut ctx).await;
    assert!(
        result.is_ok(),
        "auto-approved gate should continue: {result:?}"
    );

    let spans = TRACING_SPANS
        .get(&ctx.context)
        .expect("span stack should still be present");
    let events = spans[0]["events"]
        .as_array()
        .expect("span should carry an events array");

    let decision = events
        .iter()
        .find(|e| e["name"] == json!("approval_decision"))
        .expect("an approval_decision event must be emitted");

    assert_eq!(decision["module_id"], json!(module_id));
    assert_eq!(decision["status"], json!("approved"));
    assert_eq!(decision["approved_by"], json!("auto"));
    assert_eq!(decision["reason"], json!(""));
    assert_eq!(decision["approval_id"], json!(""));
}
