//! The executor's ACL step takes the ASYNC path (spec v1.50.0, D-105).
//!
//! §6.1.3 defines what each ACL entry point resolves and never said which one
//! the pipeline calls. This SDK called the synchronous `check_access` from
//! inside an already-`async` step, which makes any condition registered through
//! `register_async_condition` "async only" on that path — it resolves to
//! UNEVALUABLE, so an `allow` rule carrying it stops granting and a `deny` rule
//! carrying it denies unconditionally. Both directions wrong, and the entire
//! async condition registry unreachable from the only path that enforces.
//!
//! All three SDKs take the async path today. Nothing said so: a whole extension
//! point reachable from every door except the enforcing one is invisible from
//! the door, and a registry that accepts a handler is not evidence anything
//! calls it.
//!
//! The discriminator is a SYNC handler answering `false` and an ASYNC handler
//! answering `true` for the same key. Counting invocations would not separate
//! the paths — both invoke *a* handler. Only the verdict does.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::acl::{ACLRule, ACL};
use apcore::acl_handlers::ACLConditionHandler;
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::executor::Executor;
use apcore::module::Module;
use apcore::registry::registry::Registry;

const MODULE_ID: &str = "executor.probe.async_acl";
// One key per test: `register_condition` / `register_async_condition` are
// associated functions writing into process-level registries, so a handler
// registered by one test is visible to every later ACL in the process.
const KEY_ASYNC_WINS: &str = "probe_rs_async_only";
const KEY_SYNC_ONLY: &str = "probe_rs_sync_only";

#[derive(Debug)]
struct ProbeModule;

#[async_trait]
impl Module for ProbeModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "probe"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"ran": true}))
    }
}

struct SaysNo;

#[async_trait]
impl ACLConditionHandler for SaysNo {
    async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
        false
    }
}

struct SaysYes;

#[async_trait]
impl ACLConditionHandler for SaysYes {
    async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
        true
    }
}

fn executor_with(key: &str, with_async: bool) -> Executor {
    let registry = Arc::new(Registry::new());
    registry
        .register_module(MODULE_ID, Box::new(ProbeModule))
        .expect("register module");

    // The sync handler also satisfies the structural precheck, which rejects a
    // rule naming a condition no handler claims — without it the rule would be
    // unevaluable for a second, unrelated reason.
    ACL::register_condition(key, Arc::new(SaysNo));
    if with_async {
        ACL::register_async_condition(key, Arc::new(SaysYes));
    }

    // `conditions` is assigned on the returned value, as ACLRule's own doc says.
    let mut rule = ACLRule::new(vec!["*".to_string()], vec![MODULE_ID.to_string()], "allow");
    rule.conditions = Some(json!({ key: true }));
    let acl = ACL::new(vec![rule], "deny", None);

    let mut executor = Executor::new(registry, Arc::new(Config::default()));
    executor.set_acl(acl);
    executor
}

#[tokio::test]
async fn an_async_only_condition_decides_the_call() {
    let executor = executor_with(KEY_ASYNC_WINS, true);

    let out = executor
        .call(MODULE_ID, json!({}), None, None)
        .await
        .expect("the async handler answers true and the allow rule grants");

    assert_eq!(out["ran"], json!(true));
}

#[tokio::test]
async fn the_sync_handler_is_the_one_that_would_deny() {
    // The control: it proves the two handlers genuinely disagree, so the test
    // above separates the paths rather than passing because the condition is
    // satisfied either way.
    let executor = executor_with(KEY_SYNC_ONLY, false);

    let err = executor
        .call(MODULE_ID, json!({}), None, None)
        .await
        .expect_err("the sync handler answers false, so the allow rule does not grant");
    assert_eq!(err.code.wire_str(), "ACL_DENIED");
}
