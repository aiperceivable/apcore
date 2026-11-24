// Spec-traced contract tests for the apcore-rust approval-system feature.
//
// Source spec: apcore/docs/features/approval-system.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_approval_system_spec.py
//
// Contract: ApprovalHandler.request_approval
//
// Each test maps to exactly one clause id of the form
// `approval_system.request_approval.<kind>.<detail>`. The verbatim
// cross-language clause id appears in a leading `// clause: <clause_id>`
// comment on the line above each test fn so a cross-language diff tool can
// line up the Python / TypeScript / Rust rows by that exact string. The fn
// name is the clause id flattened to snake_case.
//
// Tests only — production source (src/) is never modified here.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::approval::{
    AlwaysDenyHandler, ApprovalHandler, ApprovalRequest, ApprovalResult, AutoApproveHandler,
    CallbackApprovalHandler,
};
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::ModuleAnnotations;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a valid `ApprovalRequest` for the handler under test.
///
/// Mirrors the Python `_make_request` helper. Both `ApprovalRequest` and
/// `ApprovalResult` are `#[non_exhaustive]`, so cross-crate construction goes
/// through `Default::default()` + field mutation rather than a struct literal.
fn make_request(module_id: &str, arguments: Value) -> ApprovalRequest {
    let context: Context<Value> = Context::create(None, None, None, None, Value::Null, None);
    let annotations = ModuleAnnotations {
        requires_approval: true,
        ..Default::default()
    };
    let mut req = ApprovalRequest::default();
    req.module_id = module_id.to_string();
    req.arguments = arguments;
    req.context = Some(context);
    req.annotations = annotations;
    req
}

/// Build an `ApprovalResult` carrying just a status (non-exhaustive struct).
fn result_with_status(status: &str) -> ApprovalResult {
    let mut result = ApprovalResult::default();
    result.status = status.to_string();
    result
}

/// A handler whose `request_approval` resolves to the given status.
/// Mirrors the Python `_status_handler` helper.
fn status_handler(status: &'static str) -> CallbackApprovalHandler {
    // apcore#104: `new` is now async + fallible (parity with apcore-python /
    // apcore-typescript); `new_sync` is the in-process, no-I/O form, and still
    // returns a Result so a decision that could not be made is distinguishable
    // from a rejection.
    CallbackApprovalHandler::new_sync(move |_req| {
        let mut result = result_with_status(status);
        if status == "pending" {
            result.approval_id = Some("tok-1".to_string());
        }
        Ok(result)
    })
}

/// Read the canonical SCREAMING_SNAKE_CASE wire code for an `ErrorCode`.
/// `ErrorCode` derives `#[serde(rename_all = "SCREAMING_SNAKE_CASE")]`, so the
/// serialized form is the exact protocol code string (e.g. `APPROVAL_DENIED`).
fn code_string(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .expect("ErrorCode serializes")
        .as_str()
        .expect("ErrorCode serializes to a string")
        .to_string()
}

// ---------------------------------------------------------------------------
// input.<param>.<condition>
// ---------------------------------------------------------------------------

// clause: approval_system.request_approval.input.request.module_id_required
#[test]
fn approval_system_request_approval_input_request_module_id_required() {
    // The contract requires `request` to carry `module_id`. In Rust the field
    // is a non-optional `String` on `ApprovalRequest` (the type guarantees its
    // presence at compile time); we assert it is carried and round-trips.
    //
    // DIVERGENCE: Python raises TypeError when `module_id` is omitted at
    // construction; Rust has no runtime omission case — `module_id: String` is
    // a mandatory struct field, so absence is a compile error, not a raise.
    let req = make_request("test.mod", json!({}));
    assert_eq!(req.module_id, "test.mod");
}

// clause: approval_system.request_approval.input.request.caller_id_action_required
#[test]
fn approval_system_request_approval_input_request_caller_id_action_required() {
    // Decision D-03 (docs/spec/2026-05-decision-log.md, spec v1.32.0 §7.3.1):
    // `ApprovalRequest` carries `caller_id` and `action` directly, so a
    // handler can read them without traversing `request.context`. `caller_id`
    // is `None` for a top-level call (there is no caller to name) and
    // `action` is always populated — it is not conditioned on `caller_id`.
    let req = make_request("test.mod", json!({}));
    assert_eq!(req.caller_id, None);
    assert_eq!(req.action, String::new());

    let mut nested = make_request("test.mod", json!({}));
    nested.caller_id = Some("test.caller".to_string());
    nested.action = "test.mod".to_string();
    assert_eq!(nested.caller_id.as_deref(), Some("test.caller"));
    assert_eq!(nested.action, "test.mod");
}

// ---------------------------------------------------------------------------
// error.<CODE>
// ---------------------------------------------------------------------------

// clause: approval_system.request_approval.error.APPROVAL_DENIED
#[tokio::test]
async fn approval_system_request_approval_error_approval_denied() {
    // A handler that rejects yields a result that maps to ApprovalDeniedError
    // with code APPROVAL_DENIED.
    let handler = AlwaysDenyHandler;
    let result = handler
        .request_approval(&make_request("test.mod", json!({})))
        .await
        .expect("handler resolves");
    assert_eq!(result.status, "rejected");

    let err = ModuleError::new(ErrorCode::ApprovalDenied, "approval denied");
    assert_eq!(err.code, ErrorCode::ApprovalDenied);
    assert_eq!(code_string(err.code), "APPROVAL_DENIED");
}

// clause: approval_system.request_approval.error.APPROVAL_TIMEOUT
#[tokio::test]
async fn approval_system_request_approval_error_approval_timeout() {
    // A handler that times out yields a result that maps to
    // ApprovalTimeoutError with code APPROVAL_TIMEOUT.
    let handler = status_handler("timeout");
    let result = handler
        .request_approval(&make_request("test.mod", json!({})))
        .await
        .expect("handler resolves");
    assert_eq!(result.status, "timeout");

    let err = ModuleError::new(ErrorCode::ApprovalTimeout, "approval timed out");
    assert_eq!(err.code, ErrorCode::ApprovalTimeout);
    assert_eq!(code_string(err.code), "APPROVAL_TIMEOUT");
}

// clause: approval_system.request_approval.error.APPROVAL_PENDING
#[tokio::test]
async fn approval_system_request_approval_error_approval_pending() {
    // A handler that defers (Phase B) yields a result that maps to
    // ApprovalPendingError with code APPROVAL_PENDING and carries approval_id.
    let handler = status_handler("pending");
    let result = handler
        .request_approval(&make_request("test.mod", json!({})))
        .await
        .expect("handler resolves");
    assert_eq!(result.status, "pending");
    assert_eq!(result.approval_id.as_deref(), Some("tok-1"));

    let err = ModuleError::new(ErrorCode::ApprovalPending, "approval pending");
    assert_eq!(err.code, ErrorCode::ApprovalPending);
    assert_eq!(code_string(err.code), "APPROVAL_PENDING");
}

// ---------------------------------------------------------------------------
// property.<name>
// ---------------------------------------------------------------------------

// clause: approval_system.request_approval.property.async
#[tokio::test]
async fn approval_system_request_approval_property_async() {
    // request_approval is an async trait method; awaiting it resolves to an
    // ApprovalResult.
    let handler = AutoApproveHandler;
    let req = make_request("test.mod", json!({}));
    let fut = handler.request_approval(&req);
    let result = fut.await.expect("future resolves");
    assert_eq!(result.status, "approved");
}

// clause: approval_system.request_approval.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_system_request_approval_property_thread_safe() {
    // N concurrent request_approval calls with distinct inputs complete without
    // panic and each returns the result correlated with its own input.
    let n = 12usize;
    let handler: Arc<dyn ApprovalHandler> = Arc::new(CallbackApprovalHandler::new_sync(|req| {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("mod".to_string(), json!(req.module_id));
        let mut result = result_with_status("approved");
        result.metadata = Some(metadata);
        Ok(result)
    }));

    let mut join_handles = Vec::with_capacity(n);
    for i in 0..n {
        let h = Arc::clone(&handler);
        join_handles.push(tokio::spawn(async move {
            let req = make_request(&format!("mod.{i}"), json!({ "idx": i }));
            h.request_approval(&req).await
        }));
    }

    let mut seen = std::collections::HashSet::new();
    for join_handle in join_handles {
        let result = join_handle
            .await
            .expect("task does not panic")
            .expect("handler resolves");
        assert_eq!(result.status, "approved");
        let m = result.metadata.expect("metadata present");
        seen.insert(m["mod"].as_str().expect("mod is string").to_string());
    }
    let expected: std::collections::HashSet<String> = (0..n).map(|i| format!("mod.{i}")).collect();
    assert_eq!(seen, expected);
}

// clause: approval_system.request_approval.property.idempotent
#[tokio::test]
async fn approval_system_request_approval_property_idempotent() {
    // Contract declares idempotent: false. A handler may legitimately return
    // different outcomes for identical inputs across calls; the type does not
    // force same-outcome. We observe a non-idempotent handler producing
    // distinct results for the same request.
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = Arc::clone(&counter);
    let handler = CallbackApprovalHandler::new_sync(move |_req| {
        let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(result_with_status(if n == 0 {
            "approved"
        } else {
            "rejected"
        }))
    });

    let req = make_request("test.mod", json!({}));
    let first = handler.request_approval(&req).await.expect("resolves");
    let second = handler.request_approval(&req).await.expect("resolves");
    assert_eq!(first.status, "approved");
    assert_eq!(second.status, "rejected");
    assert_ne!(first.status, second.status);
}

// clause: approval_system.request_approval.property.pure
#[tokio::test]
async fn approval_system_request_approval_property_pure() {
    // Contract declares pure: false — the handler may emit notifications or
    // persist state. We observe a side effect (state mutation) caused by a
    // single request_approval call, visible via a public query on the handler.
    #[derive(Debug, Default)]
    struct RecordingHandler {
        calls: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ApprovalHandler for RecordingHandler {
        async fn request_approval(
            &self,
            request: &ApprovalRequest,
        ) -> Result<ApprovalResult, ModuleError> {
            self.calls.lock().unwrap().push(request.module_id.clone());
            Ok(result_with_status("approved"))
        }

        async fn check_approval(&self, _id: &str) -> Result<ApprovalResult, ModuleError> {
            Ok(result_with_status("rejected"))
        }
    }

    let handler = RecordingHandler::default();
    assert!(handler.calls.lock().unwrap().is_empty());
    handler
        .request_approval(&make_request("audit.me", json!({})))
        .await
        .expect("resolves");
    // Observable state changed: the call was recorded (impure / side effect).
    assert_eq!(*handler.calls.lock().unwrap(), vec!["audit.me".to_string()]);
}

// clause: approval_system.request_approval.property.protocol_conformance
#[tokio::test]
async fn approval_system_request_approval_property_protocol_conformance() {
    // All built-in handlers exposing request_approval satisfy the
    // ApprovalHandler trait (verified by holding them behind `dyn`).
    let handlers: Vec<Box<dyn ApprovalHandler>> = vec![
        Box::new(AlwaysDenyHandler),
        Box::new(AutoApproveHandler),
        Box::new(CallbackApprovalHandler::new_sync(|_r| {
            Ok(result_with_status("approved"))
        })),
    ];
    assert_eq!(handlers.len(), 3);
    for handler in &handlers {
        // Each conforms: request_approval is callable and resolves to a result.
        let result = handler
            .request_approval(&make_request("test.mod", json!({})))
            .await
            .expect("resolves");
        assert!(!result.status.is_empty());
    }
}
