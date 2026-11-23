//! Integration tests for sys_modules control modules.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::errors::ErrorCode;
use apcore::events::emitter::EventEmitter;
use apcore::executor::Executor;
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::sys_modules::control::{ReloadModule, ToggleFeatureModule};
use apcore::sys_modules::{
    check_module_disabled, is_module_disabled, register_sys_modules, ToggleState,
};
use apcore::UpdateConfigModule;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_config() -> Arc<Mutex<Config>> {
    Arc::new(Mutex::new(Config::default()))
}

fn make_emitter() -> Arc<EventEmitter> {
    Arc::new(EventEmitter::new())
}

fn make_registry() -> Arc<Registry> {
    Arc::new(Registry::new())
}

fn dummy_ctx() -> Context<serde_json::Value> {
    Context::<serde_json::Value>::new(Identity::new(
        "@test".to_string(),
        "test".to_string(),
        vec![],
        HashMap::default(),
    ))
}

fn register_dummy(registry: &Arc<Registry>, id: &str) {
    struct DummyModule;
    #[async_trait::async_trait]
    impl Module for DummyModule {
        fn description(&self) -> &'static str {
            "dummy"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn output_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, apcore::errors::ModuleError> {
            Ok(serde_json::json!({}))
        }
    }

    let descriptor = ModuleDescriptor {
        module_id: id.to_string(),
        name: None,
        description: String::new(),
        documentation: None,
        input_schema: serde_json::json!({}),
        output_schema: serde_json::json!({}),
        version: "1.0.0".to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: std::collections::HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    registry
        .register_internal(id, Box::new(DummyModule), descriptor)
        .expect("register_internal should succeed");
}

// ---------------------------------------------------------------------------
// ToggleState tests
// ---------------------------------------------------------------------------

#[test]
fn test_toggle_state_new_has_nothing_disabled() {
    let ts = ToggleState::new();
    assert!(!ts.is_disabled("my_module"));
}

#[test]
fn test_toggle_state_disable_marks_module() {
    let ts = ToggleState::new();
    ts.disable("mod_a");
    assert!(ts.is_disabled("mod_a"));
    assert!(!ts.is_disabled("mod_b"));
}

#[test]
fn test_toggle_state_enable_removes_module() {
    let ts = ToggleState::new();
    ts.disable("mod_a");
    ts.enable("mod_a");
    assert!(!ts.is_disabled("mod_a"));
}

#[test]
fn test_toggle_state_clear_empties_set() {
    let ts = ToggleState::new();
    ts.disable("mod_a");
    ts.disable("mod_b");
    ts.clear();
    assert!(!ts.is_disabled("mod_a"));
    assert!(!ts.is_disabled("mod_b"));
}

#[test]
fn test_is_module_disabled_and_check_module_disabled() {
    // The global toggle state persists across tests in the same process, so
    // use a unique ID to avoid interference from other tests.
    let unique_id = "test_global_toggle_unique_12345";
    assert!(!is_module_disabled(unique_id));
    assert!(check_module_disabled(unique_id).is_ok());
}

// ---------------------------------------------------------------------------
// UpdateConfigModule tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_update_config_module_returns_correct_result() {
    let config = make_config();
    let emitter = make_emitter();
    let module = UpdateConfigModule::new(config, emitter);
    let ctx = dummy_ctx();

    let inputs = serde_json::json!({
        "key": "max_call_depth",
        "value": 64,
        "reason": "increase depth for testing"
    });

    let result = module
        .execute(inputs, &ctx)
        .await
        .expect("execute should succeed");

    assert_eq!(result["success"], serde_json::json!(true));
    assert_eq!(result["key"], serde_json::json!("max_call_depth"));
    assert_eq!(result["new_value"], serde_json::json!(64));
}

#[tokio::test]
async fn test_update_config_module_missing_key_returns_error() {
    let config = make_config();
    let emitter = make_emitter();
    let module = UpdateConfigModule::new(config, emitter);
    let ctx = dummy_ctx();

    let inputs = serde_json::json!({
        "value": 64,
        "reason": "no key provided"
    });

    let err = module
        .execute(inputs, &ctx)
        .await
        .expect_err("should fail on missing key");
    assert!(err.message.contains("'key'"));
}

#[tokio::test]
async fn test_update_config_module_missing_reason_returns_error() {
    let config = make_config();
    let emitter = make_emitter();
    let module = UpdateConfigModule::new(config, emitter);
    let ctx = dummy_ctx();

    let inputs = serde_json::json!({
        "key": "max_call_depth",
        "value": 64
    });

    let err = module
        .execute(inputs, &ctx)
        .await
        .expect_err("should fail on missing reason");
    assert!(err.message.contains("'reason'"));
}

#[tokio::test]
async fn test_update_config_module_restricted_key_returns_error() {
    let config = make_config();
    let emitter = make_emitter();
    let module = UpdateConfigModule::new(config, emitter);
    let ctx = dummy_ctx();

    let inputs = serde_json::json!({
        "key": "sys_modules.enabled",
        "value": false,
        "reason": "trying to disable sys modules"
    });

    let err = module
        .execute(inputs, &ctx)
        .await
        .expect_err("should reject restricted key");
    assert!(err.message.contains("cannot be changed at runtime"));
}

// ---------------------------------------------------------------------------
// ToggleFeatureModule tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_toggle_feature_module_disables_and_enables() {
    let registry = make_registry();
    register_dummy(&registry, "my.module");

    let emitter = make_emitter();
    let toggle_state = Arc::new(ToggleState::new());
    let module =
        ToggleFeatureModule::new(Arc::clone(&registry), emitter, Arc::clone(&toggle_state));
    let ctx = dummy_ctx();

    // Disable
    let result = module
        .execute(
            serde_json::json!({
                "module_id": "my.module",
                "enabled": false,
                "reason": "testing disable"
            }),
            &ctx,
        )
        .await
        .expect("disable should succeed");

    assert_eq!(result["success"], serde_json::json!(true));
    assert_eq!(result["module_id"], serde_json::json!("my.module"));
    assert_eq!(result["enabled"], serde_json::json!(false));
    assert!(toggle_state.is_disabled("my.module"));

    // Re-enable
    let result = module
        .execute(
            serde_json::json!({
                "module_id": "my.module",
                "enabled": true,
                "reason": "testing enable"
            }),
            &ctx,
        )
        .await
        .expect("enable should succeed");

    assert_eq!(result["enabled"], serde_json::json!(true));
    assert!(!toggle_state.is_disabled("my.module"));
}

// A-D-12: a feature toggle MUST survive a registry reload. Disabling a module
// flips only the ToggleState (not the registry descriptor's `enabled` flag),
// so re-registering the descriptor (as a reload does) leaves the module
// disabled.
#[tokio::test]
async fn test_toggle_disable_survives_descriptor_reregister() {
    let registry = make_registry();
    register_dummy(&registry, "my.module");

    let emitter = make_emitter();
    let toggle_state = Arc::new(ToggleState::new());
    let module =
        ToggleFeatureModule::new(Arc::clone(&registry), emitter, Arc::clone(&toggle_state));
    let ctx = dummy_ctx();

    // Disable the module via the toggle.
    module
        .execute(
            serde_json::json!({
                "module_id": "my.module",
                "enabled": false,
                "reason": "disable for reload test"
            }),
            &ctx,
        )
        .await
        .expect("disable should succeed");
    assert!(toggle_state.is_disabled("my.module"));

    // Simulate a reload: re-register the descriptor with enabled=true.
    registry
        .unregister("my.module")
        .expect("unregister should succeed");
    register_dummy(&registry, "my.module");

    // The toggle MUST still report the module as disabled.
    assert!(
        toggle_state.is_disabled("my.module"),
        "toggle disable must survive a descriptor re-register (reload)"
    );
}

#[tokio::test]
async fn test_toggle_feature_module_not_found_returns_error() {
    let registry = make_registry();
    let emitter = make_emitter();
    let toggle_state = Arc::new(ToggleState::new());
    let module = ToggleFeatureModule::new(registry, emitter, toggle_state);
    let ctx = dummy_ctx();

    let err = module
        .execute(
            serde_json::json!({
                "module_id": "nonexistent.module",
                "enabled": false,
                "reason": "should not matter"
            }),
            &ctx,
        )
        .await
        .expect_err("should fail when module not in registry");

    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// ---------------------------------------------------------------------------
// register_sys_modules integration tests (C-3)
// ---------------------------------------------------------------------------

#[test]
fn test_register_sys_modules_returns_none_when_disabled() {
    let registry = Arc::new(Registry::new());
    let mut config = Config::default();
    config.set("sys_modules.enabled", serde_json::json!(false));
    let executor = Executor::new(Arc::clone(&registry), Config::default());

    let result = register_sys_modules(Arc::clone(&registry), &executor, &config, None);
    let ctx = result.expect("Ok variant expected when sys_modules.enabled=false");
    assert!(
        ctx.registered_modules.is_empty(),
        "no modules should be registered when sys_modules.enabled=false"
    );
}

#[test]
fn test_register_sys_modules_registers_health_but_not_control_when_events_disabled() {
    let registry = Arc::new(Registry::new());
    let mut config = Config::default();
    config.set("sys_modules.enabled", serde_json::json!(true));
    // events.enabled defaults to false, but set explicitly for clarity
    config.set("sys_modules.events.enabled", serde_json::json!(false));
    let executor = Executor::new(Arc::clone(&registry), Config::default());

    let _ctx = register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("should return Ok when sys_modules enabled but events disabled");

    assert!(
        registry.has("system.health.summary"),
        "health.summary should be registered even without events"
    );
    assert!(
        registry.has("system.manifest.full"),
        "manifest.full should be registered even without events"
    );
    assert!(
        registry.has("system.usage.summary"),
        "usage.summary should be registered even without events"
    );
    assert!(
        !registry.has("system.control.update_config"),
        "control modules should NOT be registered when events.enabled=false"
    );
    assert!(
        !registry.has("system.control.toggle_feature"),
        "toggle_feature should NOT be registered when events.enabled=false"
    );
}

#[test]
fn test_register_sys_modules_registers_control_modules_into_caller_registry() {
    let registry = Arc::new(Registry::new());
    let mut config = Config::default();
    config.set("sys_modules.enabled", serde_json::json!(true));
    config.set("sys_modules.events.enabled", serde_json::json!(true));
    let executor = Executor::new(Arc::clone(&registry), Config::default());

    let _ctx = register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("should return Ok when sys_modules.enabled=true");

    assert!(
        registry.has("system.control.update_config"),
        "update_config should be in caller's registry"
    );
    assert!(
        registry.has("system.control.reload_module"),
        "reload_module should be in caller's registry"
    );
    assert!(
        registry.has("system.control.toggle_feature"),
        "toggle_feature should be in caller's registry"
    );
}

// ---------------------------------------------------------------------------
// Sync SM-006 — ReloadModule full discover pipeline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_reload_module_captures_actual_previous_version_from_descriptor() {
    use apcore::events::emitter::EventEmitter;

    struct Demo;
    #[async_trait::async_trait]
    impl Module for Demo {
        fn description(&self) -> &'static str {
            "demo"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn output_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _i: serde_json::Value,
            _c: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, apcore::errors::ModuleError> {
            Ok(serde_json::json!({}))
        }
    }

    let registry = Arc::new(Registry::new());
    let emitter = Arc::new(EventEmitter::new());

    // Register a dummy module with a non-default version. The descriptor's
    // version field is what reload_module should capture as previous_version.
    let descriptor = ModuleDescriptor {
        module_id: "demo.mod".to_string(),
        name: None,
        description: String::new(),
        documentation: None,
        input_schema: serde_json::json!({}),
        output_schema: serde_json::json!({}),
        version: "2.4.7".to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations::default()),
        examples: vec![],
        metadata: std::collections::HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    registry
        .register_internal("demo.mod", Box::new(Demo), descriptor)
        .unwrap();
    crate::reload_support::RestoringDiscoverer::attach_for(&registry, &["demo.mod".to_string()]);

    let reload = ReloadModule::new(Arc::clone(&registry), Arc::clone(&emitter));
    let result = reload
        .execute(
            serde_json::json!({
                "module_id": "demo.mod",
                "reason": "test reload",
            }),
            &dummy_ctx(),
        )
        .await
        .expect("reload should succeed");

    // SM-006: previous_version is captured from the descriptor, not "unknown".
    assert_eq!(
        result["previous_version"], "2.4.7",
        "reload_module should capture the actual version, got {result:#?}"
    );
    assert_eq!(result["success"], true);
    // A reload that reports success must leave the module registered.
    //
    // This assertion previously read `assert!(!registry.has("demo.mod"))` with the
    // comment "after reload (with no discoverer attached), the module is
    // unregistered" — it pinned success-plus-deletion as correct. Reload
    // unregisters and then re-discovers; if re-discovery cannot restore the module
    // the call now fails with RELOAD_FAILED (matching apcore-python) instead of
    // returning success over an empty registry.
    assert!(
        registry.has("demo.mod"),
        "a reload reporting success must leave the module registered"
    );
}

#[tokio::test]
async fn test_reload_module_unknown_module_returns_module_not_found() {
    use apcore::events::emitter::EventEmitter;

    let registry = Arc::new(Registry::new());
    let emitter = Arc::new(EventEmitter::new());
    let reload = ReloadModule::new(registry, emitter);
    let err = reload
        .execute(
            serde_json::json!({
                "module_id": "no.such.module",
                "reason": "x",
            }),
            &dummy_ctx(),
        )
        .await
        .expect_err("unknown module must error");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// ---------------------------------------------------------------------------
// sync-2026-08-26 A-D-015: `sys_modules.control.overrides_path` must be read
// from Config, not only from SysModulesOptions.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overrides_path_is_resolved_from_the_config_key() {
    // apcore-python resolves this key from Config when no explicit path is
    // passed (registration.py:382). This SDK read `SysModulesOptions` alone,
    // so a project declaring the key in apcore.yaml got neither persistence
    // nor restore.
    //
    // Asserted through an observable consequence: `load_overrides` routes
    // `toggle.*` keys into the shared ToggleState, so a restored disable proves
    // the file named by the config key was actually read.
    use apcore::config::Config;
    use apcore::executor::Executor;
    use apcore::registry::registry::Registry;
    use apcore::sys_modules::register_sys_modules;

    let dir = tempfile::tempdir().expect("tempdir");
    let overrides = dir.path().join("overrides.yaml");
    std::fs::write(&overrides, "toggle.executor.email.send: false\n").expect("write overrides");

    let mut cfg = Config::from_defaults();
    cfg.set("sys_modules.enabled", serde_json::json!(true));
    cfg.set(
        "sys_modules.control.overrides_path",
        serde_json::json!(overrides.to_string_lossy()),
    );

    let registry = Arc::new(Registry::new());
    let config = Arc::new(cfg);
    let executor = Executor::new(Arc::clone(&registry), Arc::clone(&config));

    let ctx = register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("registration succeeds");

    assert!(
        ctx.toggle_state.is_disabled("executor.email.send"),
        "the overrides file named by the config key must be loaded and applied"
    );
}
