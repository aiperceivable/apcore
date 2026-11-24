//! Regression test for sync finding A-D-002.
//!
//! Sync `ACL::check()` must record a condition handler's reported error in the
//! emitted `AuditEntry.handler_error`, matching `async_check()` and the
//! Python/TypeScript SDKs. Previously the sync path ran handler-error reporting
//! OUTSIDE any capture scope (a documented no-op), so `AuditEntry.handler_error`
//! was ALWAYS null on the sync path even when a condition handler errored.

use apcore::acl::{ACLRule, AuditEntry, ACL};
use apcore::acl_handlers::{register_condition, report_handler_error, ACLConditionHandler};
use apcore::context::{Context, Identity};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A condition handler that simulates an internal failure: it reports a
/// handler error (cross-language equivalent of "throwing") and returns false.
struct ThrowingHandler;

#[async_trait]
impl ACLConditionHandler for ThrowingHandler {
    async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
        report_handler_error("_throwing_rs: simulated handler failure");
        false
    }
}

fn make_context() -> Context<Value> {
    let identity = Identity::new(
        "test-user".to_string(),
        "user".to_string(),
        vec![],
        HashMap::new(),
    );
    Context::new(identity)
}

/// Build an ACL with a single rule whose only condition is `_throwing_rs`,
/// plus an audit logger that captures every emitted `AuditEntry`.
fn make_acl_capturing(captured: Arc<Mutex<Vec<AuditEntry>>>) -> ACL {
    ACL::init_builtin_handlers();
    register_condition("_throwing_rs", Arc::new(ThrowingHandler));

    let mut conditions = serde_json::Map::new();
    conditions.insert("_throwing_rs".to_string(), json!(true));
    let mut rule = ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], "allow");
    rule.conditions = Some(Value::Object(conditions));

    let mut acl = ACL::new(vec![rule], "deny", None);
    acl.set_audit_logger(move |entry: &AuditEntry| {
        captured.lock().unwrap().push(entry.clone());
    });
    acl
}

#[tokio::test]
async fn async_check_records_handler_error_baseline() {
    // Baseline: async_check() already populates handler_error. This anchors the
    // expected behavior that the sync path must match.
    let captured = Arc::new(Mutex::new(Vec::new()));
    let acl = make_acl_capturing(Arc::clone(&captured));
    let ctx = make_context();

    let _ = acl
        .async_check(Some("caller.mod"), "target.mod", Some(&ctx))
        .await;

    let entries = captured.lock().unwrap();
    let entry = entries.last().expect("an audit entry must be emitted");
    assert_eq!(
        entry.handler_error.as_deref(),
        Some("_throwing_rs: simulated handler failure"),
        "async_check must record the handler error (baseline)"
    );
}

#[test]
fn sync_check_records_handler_error() {
    // The regression: sync check() must populate AuditEntry.handler_error when
    // a matching rule's condition handler reports an error.
    let captured = Arc::new(Mutex::new(Vec::new()));
    let acl = make_acl_capturing(Arc::clone(&captured));
    let ctx = make_context();

    let _ = acl.check(Some("caller.mod"), "target.mod", Some(&ctx));

    let entries = captured.lock().unwrap();
    let entry = entries.last().expect("an audit entry must be emitted");
    assert_eq!(
        entry.handler_error.as_deref(),
        Some("_throwing_rs: simulated handler failure"),
        "sync check() must record the handler error, matching async_check / Python / TS"
    );
}
