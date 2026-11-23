//! Tests for approval handler protocol — ApprovalRequest, ApprovalResult,
//! AutoApproveHandler, and AlwaysDenyHandler.

use serde_json::json;

use apcore::approval::{
    AlwaysDenyHandler, ApprovalHandler, ApprovalRequest, ApprovalResult, AutoApproveHandler,
};

// ---------------------------------------------------------------------------
// ApprovalRequest construction and serialization
// ---------------------------------------------------------------------------

#[test]
fn test_approval_request_minimal() {
    let mut req = ApprovalRequest::default();
    req.module_id = "executor.email.send".to_string();
    req.arguments = json!({"to": "user@example.com"});
    assert_eq!(req.module_id, "executor.email.send");
    assert!(req.description.is_none());
    assert!(req.tags.is_empty());
}

#[test]
fn test_approval_request_with_description_and_tags() {
    let mut req = ApprovalRequest::default();
    req.module_id = "executor.fs.delete".to_string();
    req.arguments = json!({"path": "/tmp/data"});
    req.description = Some("Delete temporary data".to_string());
    req.tags = vec!["destructive".to_string(), "filesystem".to_string()];
    assert_eq!(req.description.as_deref(), Some("Delete temporary data"));
    assert_eq!(req.tags.len(), 2);
}

#[test]
fn test_approval_request_serialization_roundtrip() {
    let mut req = ApprovalRequest::default();
    req.module_id = "mod.a".to_string();
    req.arguments = json!({"x": 1});
    req.description = Some("test".to_string());
    req.tags = vec!["tag1".to_string()];
    let json_str = serde_json::to_string(&req).expect("serialize");
    let restored: ApprovalRequest = serde_json::from_str(&json_str).expect("deserialize");
    assert_eq!(restored.module_id, "mod.a");
    assert_eq!(restored.description.as_deref(), Some("test"));
    assert_eq!(restored.tags, vec!["tag1"]);
    // context is skipped during serialization, so it should be None.
    assert!(restored.context.is_none());
}

// ---------------------------------------------------------------------------
// ApprovalResult construction and serialization
// ---------------------------------------------------------------------------

#[test]
fn test_approval_result_approved() {
    let mut result = ApprovalResult::default();
    result.status = "approved".to_string();
    result.approved_by = Some("admin".to_string());
    result.approval_id = Some("apr-123".to_string());
    assert_eq!(result.status, "approved");
    assert_eq!(result.approved_by.as_deref(), Some("admin"));
    assert_eq!(result.approval_id.as_deref(), Some("apr-123"));
}

#[test]
fn test_approval_result_rejected() {
    let mut result = ApprovalResult::default();
    result.status = "rejected".to_string();
    result.reason = Some("Policy violation".to_string());
    assert_eq!(result.status, "rejected");
    assert_eq!(result.reason.as_deref(), Some("Policy violation"));
}

#[test]
fn test_approval_result_serialization_omits_none() {
    let mut result = ApprovalResult::default();
    result.status = "approved".to_string();
    let v = serde_json::to_value(&result).expect("serialize");
    assert!(v.get("approved_by").is_none());
    assert!(v.get("reason").is_none());
    assert!(v.get("approval_id").is_none());
    assert!(v.get("metadata").is_none());
}

// ---------------------------------------------------------------------------
// AutoApproveHandler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_auto_approve_handler_request_approval() {
    let handler = AutoApproveHandler;
    let mut req = ApprovalRequest::default();
    req.module_id = "test.mod".to_string();
    req.arguments = json!({});
    let result = handler
        .request_approval(&req)
        .await
        .expect("should succeed");
    assert_eq!(result.status, "approved");
    assert_eq!(result.approved_by.as_deref(), Some("auto"));
}

#[tokio::test]
async fn test_auto_approve_handler_check_approval() {
    let handler = AutoApproveHandler;
    let result = handler
        .check_approval("any-id")
        .await
        .expect("should succeed");
    assert_eq!(result.status, "approved");
    assert_eq!(result.approved_by.as_deref(), Some("auto"));
}

// ---------------------------------------------------------------------------
// AlwaysDenyHandler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_always_deny_handler_request_approval() {
    let handler = AlwaysDenyHandler;
    let mut req = ApprovalRequest::default();
    req.module_id = "test.mod".to_string();
    req.arguments = json!({});
    let result = handler
        .request_approval(&req)
        .await
        .expect("should succeed");
    assert_eq!(result.status, "rejected");
    assert!(result.approved_by.is_none());
    assert_eq!(result.reason.as_deref(), Some("Always denied"));
}

#[tokio::test]
async fn test_always_deny_handler_check_approval() {
    let handler = AlwaysDenyHandler;
    let result = handler
        .check_approval("any-id")
        .await
        .expect("should succeed");
    assert_eq!(result.status, "rejected");
    assert_eq!(result.reason.as_deref(), Some("Always denied"));
}

// ---------------------------------------------------------------------------
// Trait object usage — handlers behind dyn ApprovalHandler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handler_as_trait_object() {
    let handlers: Vec<Box<dyn ApprovalHandler>> =
        vec![Box::new(AutoApproveHandler), Box::new(AlwaysDenyHandler)];

    let mut req = ApprovalRequest::default();
    req.module_id = "test.mod".to_string();
    req.arguments = json!({});

    let approve_result = handlers[0].request_approval(&req).await.unwrap();
    assert_eq!(approve_result.status, "approved");

    let deny_result = handlers[1].request_approval(&req).await.unwrap();
    assert_eq!(deny_result.status, "rejected");
}
