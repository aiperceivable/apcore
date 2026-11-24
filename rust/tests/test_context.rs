//! Tests for Context and Identity.

use apcore::cancel::CancelToken;
use apcore::context::{Context, Identity};
use serde_json::Value;
use std::collections::HashMap;

fn make_identity(id: &str, identity_type: &str, roles: &[&str]) -> Identity {
    Identity::new(
        id.to_string(),
        identity_type.to_string(),
        roles.iter().map(std::string::ToString::to_string).collect(),
        HashMap::new(),
    )
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

#[test]
fn test_identity_basic_fields() {
    let id = make_identity("user-1", "Alice", &["admin", "user"]);
    assert_eq!(id.id(), "user-1");
    assert_eq!(id.identity_type(), "Alice");
    assert_eq!(id.roles().len(), 2);
    assert!(id.roles().contains(&"admin".to_string()));
}

#[test]
fn test_identity_empty_roles() {
    let id = make_identity("svc-1", "Service", &[]);
    assert!(id.roles().is_empty());
}

#[test]
fn test_identity_with_attrs() {
    let mut attrs = HashMap::new();
    attrs.insert("tier".to_string(), serde_json::json!("premium"));
    let id = Identity::new("user-2".to_string(), "Bob".to_string(), vec![], attrs);
    assert_eq!(id.attrs()["tier"], "premium");
}

#[test]
fn test_identity_serialization() {
    let id = make_identity("user-1", "Alice", &["admin"]);
    let json = serde_json::to_string(&id).unwrap();
    let restored: Identity = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.id(), id.id());
    assert_eq!(restored.identity_type(), id.identity_type());
    assert_eq!(restored.roles(), id.roles());
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

#[test]
fn test_context_new_has_unique_trace_id() {
    let id = make_identity("user-1", "Alice", &[]);
    let ctx1: Context<Value> = Context::new(id.clone());
    let ctx2: Context<Value> = Context::new(id);
    assert_ne!(ctx1.trace_id, ctx2.trace_id);
}

#[test]
fn test_context_initial_call_chain_length_is_zero() {
    let id = make_identity("user-1", "Alice", &[]);
    let ctx: Context<Value> = Context::new(id);
    assert_eq!(ctx.call_chain.len(), 0);
}

#[test]
fn test_context_initial_call_chain_is_empty() {
    let id = make_identity("user-1", "Alice", &[]);
    let ctx: Context<Value> = Context::new(id);
    assert!(ctx.call_chain.is_empty());
}

#[test]
fn test_context_no_global_deadline_by_default() {
    let id = make_identity("user-1", "Alice", &[]);
    let ctx: Context<Value> = Context::new(id);
    assert!(ctx.global_deadline.is_none());
}

#[test]
fn test_context_identity_is_preserved() {
    let id = make_identity("svc-42", "MyService", &["reader"]);
    let ctx: Context<Value> = Context::new(id);
    let identity = ctx.identity.as_ref().expect("identity should be Some");
    assert_eq!(identity.id(), "svc-42");
    assert_eq!(identity.roles(), &["reader"]);
}

#[test]
fn test_context_no_cancel_token_by_default() {
    let id = make_identity("user-1", "Alice", &[]);
    let ctx: Context<Value> = Context::new(id);
    assert!(ctx.cancel_token.is_none());
}

#[test]
fn test_context_with_cancel_token() {
    // Per Issue #66, `cancel_token` is a first-class parameter on
    // `Context::create` so this no longer requires post-hoc assignment.
    let id = make_identity("user-1", "Alice", &[]);
    let token = CancelToken::new();
    let ctx: Context<Value> = Context::create(Some(id), None, Some(token), None, Value::Null, None);
    assert!(!ctx.cancel_token.as_ref().unwrap().is_cancelled());
}

#[test]
fn test_context_data_starts_empty() {
    let id = make_identity("user-1", "Alice", &[]);
    let ctx: Context<Value> = Context::new(id);
    assert!(ctx.data.read().is_empty());
}

#[test]
fn test_anonymous_context_has_none_identity() {
    let ctx: Context<Value> = Context::anonymous();
    assert!(ctx.identity.is_none());
    assert!(ctx.call_chain.is_empty());
}

// ---------------------------------------------------------------------------
// D10-002: Context::create accepts Option<Identity> so the @external /
// anonymous-caller path is reachable through the same constructor as
// authenticated callers. Spec core-executor.md:148 declares identity
// optional; apcore-python (context.py:48-103) and apcore-typescript
// (context.ts:80-116) accept None/null. Previously Rust required Identity
// outright, so cross-language fixtures could not call Context::create
// without supplying a fabricated Identity.
// ---------------------------------------------------------------------------

#[test]
fn test_context_create_accepts_none_identity() {
    let ctx: Context<Value> = Context::create(None, None, None, None, Value::Null, None);
    assert!(
        ctx.identity.is_none(),
        "Context::create(None, ...) must leave identity as None — downstream consumers \
         (ACL.check, sys_modules) map None to the @external sentinel at evaluation time, \
         matching apcore-python and apcore-typescript."
    );
    assert!(ctx.call_chain.is_empty());
    assert!(ctx.caller_id.is_none());
}

#[test]
fn test_context_create_preserves_supplied_identity() {
    let id = make_identity("alice", "Alice", &["admin"]);
    let ctx: Context<Value> =
        Context::create(Some(id.clone()), None, None, None, Value::Null, None);
    assert_eq!(
        ctx.identity.as_ref().map(apcore::Identity::id),
        Some("alice")
    );
}

#[test]
fn test_shared_data_between_parent_and_child() {
    let id = make_identity("user-1", "Alice", &[]);
    let parent: Context<Value> = Context::new(id);
    let child = parent.child("child_mod");

    // Write to parent's data
    parent
        .data
        .write()
        .insert("key".to_string(), serde_json::json!("from_parent"));

    // Read from child's data — should see parent's write since they share Arc
    let child_data = child.data.read();
    assert_eq!(
        child_data.get("key").unwrap(),
        &serde_json::json!("from_parent")
    );
}

#[test]
fn test_context_serde_roundtrip() {
    let id = make_identity("user-1", "Alice", &["admin"]);
    let ctx: Context<Value> = Context::new(id);
    ctx.data
        .write()
        .insert("foo".to_string(), serde_json::json!(42));

    let json = serde_json::to_value(&ctx).unwrap();
    let restored: Context<Value> = serde_json::from_value(json).unwrap();

    assert_eq!(restored.trace_id, ctx.trace_id);
    assert_eq!(
        restored.identity.as_ref().unwrap().id(),
        ctx.identity.as_ref().unwrap().id()
    );
    assert_eq!(
        restored.data.read().get("foo"),
        Some(&serde_json::json!(42))
    );
}
