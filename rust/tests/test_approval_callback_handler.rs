//! `CallbackApprovalHandler` — async and fallible (apcore#104), and the
//! semantic constructors of `api-surface-conventions.md` §9.2 rule 4 (#103).
//!
//! Until spec v1.24.0 the Rust constructor took a synchronous, infallible
//! `Fn(&ApprovalRequest) -> ApprovalResult`, while apcore-python and
//! apcore-typescript both take an async callback that can fail. The failure
//! half was the worse divergence: a Rust author whose approval service was
//! down had to choose between panicking inside the approval gate and returning
//! a `"rejected"` result, which reads in the audit log exactly like a human
//! saying no.

#![allow(clippy::pedantic)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult, CallbackApprovalHandler};
use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use serde_json::json;

fn make_request(module_id: &str) -> ApprovalRequest {
    let identity = Identity::new(
        "alice".to_string(),
        "user".to_string(),
        vec!["operator".to_string()],
        HashMap::new(),
    );
    let mut request = ApprovalRequest::default();
    request.module_id = module_id.to_string();
    request.arguments = json!({ "record_id": "C-7" });
    request.context = Some(Context::new(identity));
    request
}

// ---------------------------------------------------------------------------
// #104 — the async, fallible constructor
// ---------------------------------------------------------------------------

// [callback-approval-async] `new` accepts an ordinary closure returning an
// `async move` block. An approval decision that performs I/O now fits the
// convenience handler in Rust, as it already did in Python and TypeScript.
#[tokio::test]
async fn new_accepts_an_async_callback_that_awaits() {
    let handler = CallbackApprovalHandler::new(|req: ApprovalRequest| async move {
        // A real handler would await a webhook here.
        tokio::task::yield_now().await;
        Ok(ApprovalResult::approved(format!("slack:{}", req.module_id)))
    });

    let result = handler
        .request_approval(&make_request("executor.crm.delete"))
        .await
        .expect("resolves");
    assert_eq!(result.status, "approved");
    assert_eq!(
        result.approved_by.as_deref(),
        Some("slack:executor.crm.delete")
    );
}

// [callback-approval-fallible] The divergence #104 called the worse one: a
// failing approval decision is now a first-class outcome, distinct from a
// rejection. `Err` propagates out of `request_approval` rather than being
// flattened into `"rejected"`.
#[tokio::test]
async fn an_async_callback_can_report_failure_rather_than_rejection() {
    let handler = CallbackApprovalHandler::new(|_req: ApprovalRequest| async move {
        Err::<ApprovalResult, _>(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "approval service unreachable",
        ))
    });

    let err = handler
        .request_approval(&make_request("executor.crm.delete"))
        .await
        .expect_err("a failing decision must surface as Err, not as a rejection");
    assert_eq!(err.code, ErrorCode::GeneralInternalError);
    assert!(err.message.contains("approval service unreachable"));
}

// [callback-approval-sync] `new_sync` covers in-process decisions with no I/O,
// and returns a Result for the same reason `new` does.
#[tokio::test]
async fn new_sync_accepts_a_synchronous_callback() {
    let handler = CallbackApprovalHandler::new_sync(|req: &ApprovalRequest| {
        Ok(if req.arguments.get("record_id").is_some() {
            ApprovalResult::approved("policy")
        } else {
            ApprovalResult::rejected("no record_id")
        })
    });

    let result = handler
        .request_approval(&make_request("executor.crm.delete"))
        .await
        .expect("resolves");
    assert_eq!(result.status, "approved");
}

#[tokio::test]
async fn a_sync_callback_can_report_failure_too() {
    let handler = CallbackApprovalHandler::new_sync(|_req: &ApprovalRequest| {
        Err(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "policy store unavailable",
        ))
    });

    let err = handler
        .request_approval(&make_request("a.b"))
        .await
        .expect_err("Err must propagate");
    assert!(err.message.contains("policy store unavailable"));
}

// [callback-approval-dyn-coercion] Both constructors share one storage form,
// so the struct stays non-generic and still coerces to `Box<dyn
// ApprovalHandler>` / `Arc<dyn ApprovalHandler>` — the property that lets it
// be handed to `Executor::set_approval_handler`.
#[tokio::test]
async fn both_constructors_coerce_to_the_trait_object() {
    let handlers: Vec<Box<dyn ApprovalHandler>> = vec![
        Box::new(CallbackApprovalHandler::new(
            |_req: ApprovalRequest| async { Ok(ApprovalResult::approved("async")) },
        )),
        Box::new(CallbackApprovalHandler::new_sync(|_req| {
            Ok(ApprovalResult::approved("sync"))
        })),
    ];

    for handler in &handlers {
        let result = handler
            .request_approval(&make_request("a.b"))
            .await
            .expect("resolves");
        assert_eq!(result.status, "approved");
    }

    let _shared: Arc<dyn ApprovalHandler> = Arc::new(CallbackApprovalHandler::new_sync(|_req| {
        Ok(ApprovalResult::approved("shared"))
    }));
}

// [callback-approval-owned-request] The callback takes the request by value.
// That clone shares `Context.data` (an `Arc<RwLock<…>>`), so the spec's
// by-reference `data` semantics survive it.
#[tokio::test]
async fn the_owned_request_still_shares_context_data_by_reference() {
    let request = make_request("a.b");
    let ctx = request.context.clone().expect("context present");
    ctx.data.write().insert("seen".to_string(), json!(false));

    let handler = CallbackApprovalHandler::new(|req: ApprovalRequest| async move {
        let context = req.context.expect("the clone carries the context");
        context.data.write().insert("seen".to_string(), json!(true));
        Ok(ApprovalResult::approved("callback"))
    });

    handler.request_approval(&request).await.expect("resolves");

    assert_eq!(
        ctx.data.read().get("seen"),
        Some(&json!(true)),
        "Context.data sits behind an Arc<RwLock<…>>, so the request clone shares it and the \
         spec's by-reference `data` semantics survive taking the request by value"
    );
}

// [callback-approval-check-approval] Phase B is still unsupported, and says so
// through the semantic constructor rather than a struct literal.
#[tokio::test]
async fn check_approval_rejects_with_a_reason() {
    let handler =
        CallbackApprovalHandler::new_sync(|_req| Ok(ApprovalResult::approved("callback")));

    let result = handler.check_approval("tok-1").await.expect("resolves");
    assert_eq!(result.status, "rejected");
    assert!(result
        .reason
        .as_deref()
        .expect("reason")
        .contains("Phase B"));
}

// [callback-approval-called-once-per-request]
#[tokio::test]
async fn the_callback_runs_once_per_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let handler = CallbackApprovalHandler::new(move |_req: ApprovalRequest| {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(ApprovalResult::approved("callback"))
        }
    });

    for _ in 0..3 {
        handler.request_approval(&make_request("a.b")).await.ok();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

// ---------------------------------------------------------------------------
// #103 — semantic constructors (api-surface-conventions.md §9.2 rule 4)
// ---------------------------------------------------------------------------

// [approval-result-constructors] A default-constructed `ApprovalResult` is
// structurally valid and semantically empty — `status` is `""`, which is not a
// decision. The constructors are what stop a caller producing one.
#[test]
fn approval_result_constructors_set_a_canonical_status() {
    assert_eq!(ApprovalResult::default().status, "");

    let approved = ApprovalResult::approved("slack_user");
    assert_eq!(approved.status, "approved");
    assert_eq!(approved.approved_by.as_deref(), Some("slack_user"));
    assert!(approved.reason.is_none());

    let rejected = ApprovalResult::rejected("out of budget");
    assert_eq!(rejected.status, "rejected");
    assert_eq!(rejected.reason.as_deref(), Some("out of budget"));
    assert!(rejected.approved_by.is_none());
}

// [change-constructor] `Change` has three required fields and no `Default`
// path that fills them; `#[non_exhaustive]` blocks a downstream struct
// literal, so the constructor is the only usable construction path from a
// foreign crate.
#[test]
fn change_and_preview_result_constructors_set_the_required_fields() {
    use apcore::{Change, PreviewResult};

    let mut change = Change::new("delete", "users.42", "Delete user 42");
    assert_eq!(change.action, "delete");
    assert_eq!(change.target, "users.42");
    assert_eq!(change.summary, "Delete user 42");
    assert!(change.before.is_none() && change.after.is_none());
    assert!(change.extra.is_empty());

    change.before = Some(json!({ "active": true }));

    let preview = PreviewResult::new(vec![change]);
    assert_eq!(preview.changes.len(), 1);
    assert_eq!(preview.changes[0].action, "delete");

    // Round-trips through the wire format the RFC defines.
    let wire = serde_json::to_value(&preview).expect("serializes");
    assert_eq!(wire["changes"][0]["summary"], "Delete user 42");
}
