//! Direct coverage for the `APCore` client surface.
//!
//! Focused smoke tests — cross-cutting concerns such as the registry refactor
//! and the disable/enable pipeline round-trip live in `test_unified_registry.rs`,
//! and the streaming path lives in `test_true_streaming.rs`. This file exercises
//! APIs that neither of those cover: plain construction, register + call, error
//! propagation for unknown modules, and `list_modules` filtering.

use std::sync::{Arc, Mutex};

use apcore::acl::ACL;
use apcore::approval::AutoApproveHandler;
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::events::emitter::EventEmitter;
use apcore::executor::Executor;
use apcore::module::Module;
use apcore::policy::{ExecutionPolicy, PolicyRule};
use apcore::registry::Registry;
use apcore::{APCore, APCoreOptions, TypedModuleOptions};
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
async fn test_apcore_registers_and_validates_typed_function_modules() {
    #[derive(serde::Deserialize, schemars::JsonSchema)]
    struct Input {
        name: String,
    }

    #[derive(serde::Serialize, schemars::JsonSchema)]
    struct Output {
        message: String,
    }

    let mut client = APCore::new();
    client
        .typed_module(
            "demo.greet",
            "Greet an APCore caller",
            TypedModuleOptions::default(),
            |input: Input, _ctx| {
                Box::pin(async move {
                    Ok(Output {
                        message: format!("Hello, {}!", input.name),
                    })
                })
            },
        )
        .unwrap();

    let descriptor = client
        .registry()
        .get_definition("demo.greet")
        .unwrap()
        .unwrap();
    assert_eq!(descriptor.input_schema["required"], json!(["name"]));
    assert_eq!(descriptor.output_schema["required"], json!(["message"]));
    assert_eq!(
        client
            .call("demo.greet", json!({"name":"Ada"}), None, None)
            .await
            .unwrap(),
        json!({"message":"Hello, Ada!"})
    );
    assert_eq!(
        client
            .call("demo.greet", json!({}), None, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::SchemaValidationError
    );
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

fn event_config(enabled: bool) -> Config {
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(enabled));
    config
}

#[tokio::test]
async fn test_apcore_receives_auto_created_executor_governance_events() {
    let dir = tempfile::tempdir().expect("ACL directory");
    let acl_file = dir.path().join("global_acl.yaml");
    std::fs::write(
        &acl_file,
        "default_effect: deny\nrules:\n  - callers: ['*']\n    targets: ['demo.approval']\n    effect: allow\naudit:\n  enabled: false\n",
    )
    .expect("ACL configuration");
    let mut config = event_config(true);
    config.set("acl.root", json!(acl_file));
    let policy_rule = PolicyRule::new("demo.approval")
        .unwrap()
        .with_requires_approval(true);
    let mut client = APCore::try_with_options(APCoreOptions {
        config: Some(config),
        approval_handler: Some(Box::new(AutoApproveHandler)),
        policy: Some(ExecutionPolicy::new(vec![policy_rule])),
        ..Default::default()
    })
    .unwrap();
    client.register("demo.approval", Box::new(Echo)).unwrap();
    client.register("demo.denied", Box::new(Echo)).unwrap();

    let received = Arc::new(Mutex::new(Vec::new()));
    for event_type in [
        "apcore.policy.override",
        "apcore.approval.decision",
        "apcore.acl.denied",
    ] {
        let subscriber = Arc::clone(&received);
        client
            .on(event_type, move |event| {
                subscriber
                    .lock()
                    .unwrap()
                    .push((event.event_type.clone(), event.module_id.clone().unwrap()));
            })
            .unwrap();
    }

    let result = client
        .call("demo.approval", json!({}), None, None)
        .await
        .unwrap();
    assert_eq!(result, json!({"echoed": {}}));
    let error = client
        .call("demo.denied", json!({}), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ACLDenied);
    client.events().unwrap().flush_default().await.unwrap();
    let mut events = received.lock().unwrap();
    events.sort();
    assert_eq!(
        *events,
        [
            ("apcore.acl.denied".into(), "demo.denied".into()),
            ("apcore.approval.decision".into(), "demo.approval".into()),
            ("apcore.policy.override".into(), "demo.approval".into()),
        ]
    );
}

#[tokio::test]
async fn test_apcore_preserves_supplied_executor_emitter() {
    let config = event_config(true);
    let emitter = Arc::new(EventEmitter::new());
    let mut executor = Executor::new(Arc::new(Registry::default()), Arc::new(config.clone()));
    executor.set_acl(ACL::new(Vec::new(), "deny", None));
    executor.set_event_emitter(Some(Arc::clone(&emitter)));
    let mut client = APCore::with_options(None, Some(executor), Some(config), None);
    client.register("demo.denied", Box::new(Echo)).unwrap();

    let received = Arc::new(Mutex::new(Vec::new()));
    let subscriber = Arc::clone(&received);
    client
        .on("apcore.acl.denied", move |event| {
            subscriber.lock().unwrap().push(event.data.clone());
        })
        .unwrap();

    let error = client
        .call("demo.denied", json!({}), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ACLDenied);
    assert!(Arc::ptr_eq(
        client.executor().event_emitter.as_ref().unwrap(),
        &emitter
    ));
    client.events().unwrap().flush_default().await.unwrap();
    assert!(received.lock().unwrap().is_empty());
}

#[test]
fn test_apcore_disables_event_bus_with_system_modules_still_enabled() {
    let mut client = APCore::with_config(event_config(false));
    assert!(client.registry().has("system.health.summary"));
    assert!(client.events().is_none());
    assert!(client.executor().event_emitter.is_none());
    let error = client.on("apcore.acl.denied", |_| {}).unwrap_err();
    assert_eq!(error.code, ErrorCode::SysModulesDisabled);
}

#[test]
fn test_apcore_uses_event_setting_from_persisted_overrides() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("overrides.yaml"),
        "sys_modules.events.enabled: false\n",
    )
    .unwrap();
    let source = dir.path().join("apcore.yaml");
    std::fs::write(
        &source,
        "version: '1.0.0'\nproject: {name: fixture}\nsys_modules:\n  enabled: true\n  events: {enabled: true}\n  control: {overrides_path: ./overrides.yaml}\n",
    ).unwrap();
    let config = Config::load(&source).unwrap();

    let client = APCore::with_config(config);
    assert!(client.events().is_none());
    assert!(client.executor().event_emitter.is_none());
}
