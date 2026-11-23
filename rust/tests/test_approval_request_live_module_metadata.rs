// Regression test for D10-003: BuiltinApprovalGate must source the
// ApprovalRequest's annotations / description / tags from the resolved LIVE
// module instance (per PROTOCOL_SPEC §7.4 Step 5), NOT from the registry's
// ModuleDescriptor.
//
// Cross-SDK parity: apcore-python (`builtin_steps.py`) reads
// `getattr(module, "annotations"/"description"/"tags")` and apcore-typescript
// (`builtin-steps.ts`) reads `mod['annotations'/'description'/'tags']` — both
// from the live module object. This test registers a module whose live
// metadata deliberately differs from the descriptor's metadata and asserts the
// captured ApprovalRequest reflects the LIVE module.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::{Module, ModuleAnnotations};
use apcore::pipeline::{PipelineContext, Step};
use apcore::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use apcore::BuiltinApprovalGate;

/// A module whose live `description()` / `tags()` / `annotations()` are
/// distinct from anything a stale registry descriptor would carry.
#[derive(Debug)]
struct LiveMetadataModule;

impl LiveMetadataModule {
    const LIVE_DESCRIPTION: &'static str = "LIVE module description";
    const LIVE_TAG: &'static str = "live-tag";
    /// A live annotation marker that the descriptor does NOT carry.
    const LIVE_CACHE_TTL: u64 = 4242;
}

#[async_trait]
impl Module for LiveMetadataModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        Self::LIVE_DESCRIPTION
    }
    fn tags(&self) -> Vec<String> {
        vec![Self::LIVE_TAG.to_string()]
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: true,
            cache_ttl: Self::LIVE_CACHE_TTL,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

/// Build an "approved" result (ApprovalResult is `#[non_exhaustive]`, so it
/// cannot be struct-literal-constructed from outside the crate).
fn approved_result(approved_by: Option<String>) -> ApprovalResult {
    let mut result = ApprovalResult::default();
    result.status = "approved".to_string();
    result.approved_by = approved_by;
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
        Ok(approved_result(Some("recorder".to_string())))
    }

    async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
        Ok(approved_result(None))
    }
}

/// Build a descriptor whose metadata deliberately DIFFERS from the live module:
/// stale description, stale tags, and annotations missing the live `cache_ttl`.
/// The gate fires on the LIVE module's `requires_approval` (PROTOCOL_SPEC §7.4
/// Step 5 — see `test_approval_gate_decides_from_live_module.rs`); the
/// descriptor keeps it only so the two sources differ in the metadata fields
/// alone, which is what this test is about.
fn stale_descriptor(module_id: &str) -> ModuleDescriptor {
    let annotations = ModuleAnnotations {
        requires_approval: true,
        cache_ttl: 0, // STALE: live module reports 4242
        ..ModuleAnnotations::default()
    };
    ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: "STALE descriptor description".to_string(),
        documentation: None,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        version: DEFAULT_MODULE_VERSION.to_string(),
        tags: vec!["stale-tag".to_string()],
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
async fn test_approval_request_metadata_sourced_from_live_module() {
    let module_id = "live.metadata";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(LiveMetadataModule),
            stale_descriptor(module_id),
        )
        .expect("register module requiring approval");

    let handler = Arc::new(RecordingHandler::default());

    let context = Context::<Value>::anonymous();
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

    // Description must come from the LIVE module, not the stale descriptor.
    assert_eq!(
        captured.description.as_deref(),
        Some(LiveMetadataModule::LIVE_DESCRIPTION),
        "description must be sourced from the live module instance"
    );

    // Tags must come from the LIVE module, not the stale descriptor.
    assert_eq!(
        captured.tags,
        vec![LiveMetadataModule::LIVE_TAG.to_string()],
        "tags must be sourced from the live module instance"
    );

    // Annotations must come from the LIVE module — the descriptor's annotations
    // have cache_ttl: 0, while the live module reports 4242.
    assert_eq!(
        captured.annotations.cache_ttl,
        LiveMetadataModule::LIVE_CACHE_TTL,
        "annotations must be sourced from the live module instance"
    );
    assert!(
        captured.annotations.requires_approval,
        "live annotations should preserve requires_approval"
    );
}
