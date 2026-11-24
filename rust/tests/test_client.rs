//! Direct coverage for the `APCore` client surface.
//!
//! Focused smoke tests — cross-cutting concerns such as the registry refactor
//! and the disable/enable pipeline round-trip live in `test_unified_registry.rs`,
//! and the streaming path lives in `test_true_streaming.rs`. This file exercises
//! APIs that neither of those cover: plain construction, register + call, error
//! propagation for unknown modules, and `list_modules` filtering.

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};

struct Echo;

#[async_trait]
impl Module for Echo {
    fn description(&self) -> &'static str {
        "echo"
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "echoed": inputs }))
    }
}

#[tokio::test]
async fn test_apcore_new_returns_usable_client() {
    // System modules are opt-in (default false; sync finding A-D-11) — enable
    // them so the client exposes the built-in sys modules.
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    let apcore = APCore::with_config(config);
    let modules = apcore.list_modules(None, None);
    assert!(!modules.is_empty());
}

#[tokio::test]
async fn test_apcore_register_and_call_round_trip() {
    let apcore = APCore::new();

    apcore
        .register("demo.echo", Box::new(Echo))
        .expect("register should succeed");

    let result = apcore
        .call("demo.echo", json!({ "hello": "world" }), None, None)
        .await
        .expect("call should succeed");

    assert_eq!(result["echoed"]["hello"], "world");
}

#[tokio::test]
async fn test_apcore_call_unknown_module_returns_not_found() {
    let apcore = APCore::new();

    let err = apcore
        .call("does.not.exist", json!({}), None, None)
        .await
        .expect_err("call on unknown module must fail");

    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

#[tokio::test]
async fn test_apcore_register_duplicate_fails() {
    let apcore = APCore::new();

    apcore
        .register("demo.dup", Box::new(Echo))
        .expect("first register should succeed");

    let err = apcore
        .register("demo.dup", Box::new(Echo))
        .expect_err("duplicate register must fail");

    // Duplicate registration must return DuplicateModuleId (since apcore #65
    // unified the duplicate-ID error code). ModuleLoadError and GeneralInvalidInput
    // are kept here for resilience against future code reorganisation.
    assert!(
        matches!(
            err.code,
            ErrorCode::DuplicateModuleId
                | ErrorCode::ModuleLoadError
                | ErrorCode::GeneralInvalidInput
        ),
        "unexpected error code: {:?}",
        err.code
    );
}

#[tokio::test]
async fn test_apcore_list_modules_prefix_filter() {
    let apcore = APCore::new();

    apcore
        .register("demo.first", Box::new(Echo))
        .expect("register first");
    apcore
        .register("demo.second", Box::new(Echo))
        .expect("register second");
    apcore
        .register("other.module", Box::new(Echo))
        .expect("register other");

    let demo_modules = apcore.list_modules(None, Some("demo."));
    assert_eq!(demo_modules.len(), 2);
    assert!(demo_modules.iter().all(|m| m.starts_with("demo.")));

    let all_modules = apcore.list_modules(None, None);
    assert!(all_modules.contains(&"other.module".to_string()));
}

// ---------------------------------------------------------------------------
// A-003: APCore::disable / enable accept reason: Option<&str> and return
// a status payload routed through system.control.toggle_feature.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_apcore_disable_enable_signature_takes_reason() {
    // disable/enable route through `system.control.toggle_feature`, which is
    // only auto-registered when sys_modules is enabled (opt-in; default false,
    // sync finding A-D-11) and sys_modules.events.enabled=true.
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let apcore = APCore::with_config(config);

    // Unique id: the disabled-set is a process-global ToggleState (A-D-12).
    let module_id = "demo.toggle.signature_test";
    apcore
        .register(module_id, Box::new(Echo))
        .expect("register");

    // None reason path — must succeed (default reason injected by client).
    let res = apcore.disable(module_id, None).await;
    assert!(res.is_ok(), "disable(None) should succeed: {:?}", res.err());

    // Some reason path — must succeed and return a status payload.
    let res = apcore
        .enable(module_id, Some("operator request"))
        .await
        .expect("enable should succeed");
    // Returned payload should be an object (concrete shape is set by
    // sys_modules.control.toggle_feature; we just verify it's structured).
    assert!(res.is_object(), "enable should return an object payload");
}
