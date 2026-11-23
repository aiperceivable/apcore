//! Conformance driver for `approval_request_fields.json` (decision D-03).
//!
//! Fixture source: `apcore/conformance/fixtures/approval_request_fields.json`
//! (single source of truth). See that fixture's `description` and
//! `driver_contract` for the contract driven here.
//!
//! Spec decision D-03 (`docs/spec/2026-05-decision-log.md`, spec v1.32.0
//! §7.3.1): `ApprovalRequest` carries `caller_id` and `action`, populated by
//! `BuiltinApprovalGate` at Executor Step 4.5 from the Context and module ID
//! already in scope at that call site — `caller_id = context.caller_id`
//! (`None` on a top-level call, per §5.7, with NO `"@external"` substitution;
//! that sentinel is ACL-internal), `action = module_id`.
//!
//! `tests/test_approval_request_caller_id_action.rs` asserts both by hand. A
//! hand copy cannot notice when the canonical fixture gains a case, so this
//! driver iterates the fixture's own `test_cases` — a case added upstream is
//! driven here without an edit, and a case whose shape this driver cannot read
//! fails loudly rather than being skipped.
//!
//! Per `driver_contract.no_wire_assertion`, both fields are read off the
//! in-process `ApprovalRequest` the handler was handed, never a serialized
//! round-trip: `ApprovalRequest` skips `context` during serialization and
//! neither field has a wire-format fixture elsewhere.
#![allow(clippy::pedantic)] // fixture-driven test file: layout follows the fixture schema

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

use crate::conformance_env::find_fixtures_root;

const FIXTURE: &str = "approval_request_fields.json";

fn load_fixture() -> Value {
    let path = find_fixtures_root().join(FIXTURE);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON: {e}"))
}

/// The target: requires approval, so Step 4.5 builds an `ApprovalRequest`.
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
        "gated target for the D-03 caller_id/action contract"
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

/// `ApprovalResult` is `#[non_exhaustive]`, so it cannot be struct-literal
/// constructed from outside the crate.
fn approved_result() -> ApprovalResult {
    let mut result = ApprovalResult::default();
    result.status = "approved".to_string();
    result.approved_by = Some("recorder".to_string());
    result
}

/// Records the request it was handed and auto-approves.
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

fn descriptor(module_id: &str) -> apcore::registry::registry::ModuleDescriptor {
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
    }
}

/// Run `BuiltinApprovalGate` for `target_id` under `context` and return the
/// `ApprovalRequest` the handler captured.
async fn run_gate_and_capture(target_id: &str, context: Context<Value>) -> ApprovalRequest {
    let registry = Arc::new(Registry::new());
    registry
        .register(target_id, Box::new(GatedModule), descriptor(target_id))
        .expect("register module requiring approval");

    let handler = Arc::new(RecordingHandler::default());

    let mut ctx = PipelineContext::new(target_id, json!({}), context, "standard");
    ctx.registry = Some(registry);
    ctx.approval_handler = Some(handler.clone());

    let result = BuiltinApprovalGate.execute(&mut ctx).await;
    assert!(result.is_ok(), "an approved gate must continue: {result:?}");

    let captured = handler
        .captured
        .lock()
        .expect("lock poisoned")
        .clone()
        .expect("the handler must have received an ApprovalRequest");
    captured
}

/// The context the call reaches Step 4.5 with.
///
/// `caller_id: null` is a top-level call — a fresh `Context` that never passed
/// through `child()`. A non-null one is a nested call, built the way the
/// Executor builds it: `child()` sets the new context's `caller_id` from the
/// parent's `call_chain`, so chaining through the caller is what puts its ID
/// there. Setting the field by hand instead would pass against a gate that
/// read anything at all off the context it was handed.
fn context_for(caller_id: Option<&str>, target_id: &str) -> Context<Value> {
    let root = Context::<Value>::anonymous();
    match caller_id {
        None => root,
        Some(caller) => {
            let nested = root.child(caller).child(target_id);
            assert_eq!(
                nested.caller_id.as_deref(),
                Some(caller),
                "driver setup: the nested context's caller_id must resolve to the caller module"
            );
            nested
        }
    }
}

#[tokio::test]
async fn approval_request_fields_conformance() {
    let fixture = load_fixture();
    let cases = fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array");
    assert!(!cases.is_empty(), "{FIXTURE} declares no test cases");

    for case in cases {
        let id = case["id"].as_str().expect("every case needs an id");
        let target_id = case["target_id"]
            .as_str()
            .unwrap_or_else(|| panic!("[{FIXTURE} :: {id}] target_id must be a string"));
        // `caller_id` is nullable by design — null IS the top-level case, so a
        // missing key and an explicit null are NOT the same thing here.
        let caller_field = case
            .get("caller_id")
            .unwrap_or_else(|| panic!("[{FIXTURE} :: {id}] must declare caller_id"));
        let caller_id = match caller_field {
            Value::Null => None,
            Value::String(s) => Some(s.as_str()),
            other => panic!("[{FIXTURE} :: {id}] caller_id must be a string or null, got {other}"),
        };

        let expected_caller_id = match &case["expected_request_caller_id"] {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => panic!(
                "[{FIXTURE} :: {id}] expected_request_caller_id must be a string or null, got {other}"
            ),
        };
        let expected_action = case["expected_request_action"].as_str().unwrap_or_else(|| {
            panic!("[{FIXTURE} :: {id}] expected_request_action must be a string")
        });

        let captured = run_gate_and_capture(target_id, context_for(caller_id, target_id)).await;

        assert_eq!(
            captured.caller_id, expected_caller_id,
            "[{FIXTURE} :: {id}] caller_id is read straight off context.caller_id — \
             None on a top-level call, never the \"@external\" ACL sentinel"
        );
        assert_eq!(
            captured.action, expected_action,
            "[{FIXTURE} :: {id}] action must equal the invoked module's id, not a separate label"
        );
    }
}
