//! Regression tests for A-D-011 + A-D-012 (SECURITY).
//!
//! A panicking ACL condition handler MUST NOT unwind out of `ACL::check()` /
//! `ACL::async_check()`. Instead the panic must be caught, recorded as the
//! emitted `AuditEntry.handler_error`, and the decision must fail closed
//! (deny). Mirrors apcore-python `acl.py` (try/except → deny + record) and
//! apcore-typescript `acl.ts` (try/catch → deny + record).

use apcore::acl::{ACLRule, AuditEntry, ACL};
use apcore::acl_handlers::{register_condition, ACLConditionHandler};
use apcore::context::{Context, Identity};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A condition handler that panics during evaluation.
struct PanickingHandler;

#[async_trait]
impl ACLConditionHandler for PanickingHandler {
    async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
        panic!("simulated handler panic");
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

/// Build an ACL with a single allow rule guarded by the panicking condition,
/// plus an audit logger that captures every emitted `AuditEntry`. Default
/// effect is `deny`, so a caught panic (treated as unsatisfied) must result
/// in a denied decision.
fn make_acl_capturing(captured: Arc<Mutex<Vec<AuditEntry>>>) -> ACL {
    ACL::init_builtin_handlers();
    register_condition("_panicking_rs", Arc::new(PanickingHandler));

    let mut conditions = serde_json::Map::new();
    conditions.insert("_panicking_rs".to_string(), json!(true));
    let mut rule = ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], "allow");
    rule.conditions = Some(Value::Object(conditions));

    let mut acl = ACL::new(vec![rule], "deny", None);
    acl.set_audit_logger(move |entry: &AuditEntry| {
        captured.lock().unwrap().push(entry.clone());
    });
    acl
}

/// Suppress the default panic-hook stderr noise for the duration of a closure
/// so caught panics in these tests don't spam the test log.
fn silence_panic_hook<T>(f: impl FnOnce() -> T) -> T {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = f();
    std::panic::set_hook(prev);
    out
}

/// A-D-011 (sync): a panicking handler must not unwind; `check()` returns
/// false (deny) and the audit entry carries handler_error.
#[test]
fn sync_check_denies_and_records_on_handler_panic() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let acl = make_acl_capturing(Arc::clone(&captured));
    let ctx = make_context();

    let decision = silence_panic_hook(|| acl.check(Some("caller.mod"), "target.mod", Some(&ctx)));

    assert!(
        !decision,
        "a panicking condition handler must fail closed (deny)"
    );

    let entries = captured.lock().unwrap();
    let entry = entries.last().expect("an audit entry must be emitted");
    let handler_error = entry
        .handler_error
        .as_deref()
        .expect("audit entry must record handler_error on panic");
    assert!(
        handler_error.contains("_panicking_rs") && handler_error.contains("panicked"),
        "handler_error must identify the panicking condition: {handler_error}"
    );
    assert!(
        handler_error.contains("simulated handler panic"),
        "handler_error must carry the panic message: {handler_error}"
    );
}

/// A-D-011 (async): same fail-closed + record behavior on the async path.
#[tokio::test]
async fn async_check_denies_and_records_on_handler_panic() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let acl = make_acl_capturing(Arc::clone(&captured));
    let ctx = make_context();

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let decision = acl
        .async_check(Some("caller.mod"), "target.mod", Some(&ctx))
        .await;
    std::panic::set_hook(prev);

    assert!(
        !decision,
        "a panicking condition handler must fail closed (deny) on the async path"
    );

    let entries = captured.lock().unwrap();
    let entry = entries.last().expect("an audit entry must be emitted");
    let handler_error = entry
        .handler_error
        .as_deref()
        .expect("audit entry must record handler_error on panic (async)");
    assert!(
        handler_error.contains("_panicking_rs") && handler_error.contains("panicked"),
        "handler_error must identify the panicking condition: {handler_error}"
    );
}
