//! Spec-traced contract tests for the apcore System Modules feature (Rust SDK).
//!
//! Source spec: apcore/docs/features/system-modules.md
//! Canonical clause source: apcore-python/tests/test_system_modules_spec.py
//!
//! Each test mirrors a clause id formatted `system_modules.<method>.<kind>.<detail>`
//! recorded verbatim in a leading `// clause: <clause_id>` comment so cross-language
//! diffs line up row-for-row. These tests assert ACTUAL Rust behavior; clauses whose
//! Python intent has no faithful Rust counterpart are documented inline and, where the
//! target symbol genuinely does not exist, marked `#[ignore]` so the crate still
//! compiles and runs.
//!
//! Notable Rust/Python divergences (handled, not faked):
//!   * `update_config.execute` is `async` in Rust (Module trait), so the Python
//!     `*.property.async_false` clauses invert — Rust asserts the call resolves.
//!   * `update_config.execute` validates the per-key constraint after setting
//!     (Config::validate_key_constraint) and rolls back to old_value on
//!     violation, raising ConfigError (CONFIG_INVALID) — matching Python's
//!     `_validate_post_set` (system-modules.md §311-348).
//!   * `check_module_disabled` / `is_module_disabled` are single-arg free functions
//!     reading a process-global toggle state; the spec's `registry` parameter does
//!     not exist (ignored, matching the Python skip).
//!   * `register_sys_modules` returns a typed `SysModulesContext` struct, not a dict;
//!     the "context_components" clause asserts the real struct fields.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::errors::ErrorCode;
use apcore::events::emitter::EventEmitter;
use apcore::executor::Executor;
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::sys_modules::control::{ReloadModule, ToggleFeatureModule, UpdateConfigModule};
use apcore::sys_modules::{
    check_module_disabled, global_toggle_state_arc, is_module_disabled, register_sys_modules,
    register_sys_modules_with_options, SysModulesOptions, ToggleState,
};
use serde_json::json;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_config() -> Arc<Mutex<Config>> {
    Arc::new(Mutex::new(Config::from_defaults()))
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

/// Resolve an `ErrorCode` to its wire string (SCREAMING_SNAKE_CASE via serde).
fn code_str(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn register_dummy(registry: &Arc<Registry>, id: &str) {
    struct DummyModule;
    #[async_trait::async_trait]
    impl Module for DummyModule {
        fn description(&self) -> &'static str {
            "dummy"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        fn output_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _ctx: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, apcore::errors::ModuleError> {
            Ok(json!({}))
        }
    }

    let descriptor = ModuleDescriptor {
        module_id: id.to_string(),
        name: None,
        description: "dummy".to_string(),
        documentation: None,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
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

fn registry_with_module(id: &str) -> Arc<Registry> {
    let reg = make_registry();
    register_dummy(&reg, id);
    // reload unregisters before re-discovering; without a discoverer the module
    // cannot come back and the call fails with RELOAD_FAILED (matching
    // apcore-python). Tests here assert other properties, so restore it.
    crate::reload_support::RestoringDiscoverer::attach_for(&reg, &[id.to_string()]);
    reg
}

fn update_module() -> UpdateConfigModule {
    UpdateConfigModule::new(make_config(), make_emitter())
}

// ===========================================================================
// Contract 1: system.control.update_config  (UpdateConfigModule::execute)
// ===========================================================================

// clause: system_modules.update_config.input.key_required
#[tokio::test]
async fn update_config_input_key_required() {
    let module = update_module();
    let err = module
        .execute(json!({"key": "", "value": 1, "reason": "r"}), &dummy_ctx())
        .await
        .expect_err("empty key must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: system_modules.update_config.input.reason_required
#[tokio::test]
async fn update_config_input_reason_required() {
    let module = update_module();
    let err = module
        .execute(
            json!({"key": "executor.default_timeout", "value": 1, "reason": ""}),
            &dummy_ctx(),
        )
        .await
        .expect_err("empty reason must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: system_modules.update_config.input.value_any_accepted
#[tokio::test]
async fn update_config_input_value_any_accepted() {
    let module = update_module();
    let result = module
        .execute(
            json!({"key": "some.arbitrary.field", "value": {"nested": [1, 2, 3]}, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("arbitrary JSON value must be accepted");
    assert_eq!(result["new_value"], json!({"nested": [1, 2, 3]}));
}

// clause: system_modules.update_config.error.config_key_restricted
#[tokio::test]
async fn update_config_error_config_key_restricted() {
    let module = update_module();
    let err = module
        .execute(
            json!({"key": "sys_modules.enabled", "value": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("restricted key must be rejected");
    assert_eq!(err.code, ErrorCode::ConfigKeyRestricted);
    assert_eq!(code_str(err.code), "CONFIG_KEY_RESTRICTED");
}

// clause: system_modules.update_config.error.config_constraint
// A value that violates the registered constraint for the key is rejected with
// a ConfigError (CONFIG_INVALID) per system-modules.md §311-348.
#[tokio::test]
async fn update_config_error_config_constraint() {
    let config = make_config();
    // Seed a valid value so the rollback has a prior value to restore.
    config.lock().await.set("acl.default_effect", json!("deny"));
    let module = UpdateConfigModule::new(Arc::clone(&config), make_emitter());
    let err = module
        .execute(
            json!({"key": "acl.default_effect", "value": "maybe", "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("constraint violation must be rejected");
    assert_eq!(err.code, ErrorCode::ConfigInvalid);
    assert_eq!(code_str(err.code), "CONFIG_INVALID");
}

// clause: system_modules.update_config.side_effect.rollback_on_constraint
// On constraint failure the Config is rolled back to old_value (§311-348 step 3).
#[tokio::test]
async fn update_config_side_effect_rollback_on_constraint() {
    let config = make_config();
    config.lock().await.set("acl.default_effect", json!("deny"));
    let module = UpdateConfigModule::new(Arc::clone(&config), make_emitter());
    let _ = module
        .execute(
            json!({"key": "acl.default_effect", "value": "maybe", "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("constraint violation must be rejected");
    // The config still returns the pre-update value (rollback succeeded).
    let value = config.lock().await.get("acl.default_effect");
    assert_eq!(value, Some(json!("deny")));
}

// clause: system_modules.update_config.side_effect.set_and_emit_event
#[tokio::test]
async fn update_config_side_effect_set_and_emit_event() {
    let config = make_config();
    let emitter = make_emitter();
    let module = UpdateConfigModule::new(Arc::clone(&config), emitter);
    module
        .execute(
            json!({"key": "executor.default_timeout", "value": 60000, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("update must succeed");
    let value = config.lock().await.get("executor.default_timeout");
    assert_eq!(value, Some(json!(60000)));
}

// clause: system_modules.update_config.return.success_shape
#[tokio::test]
async fn update_config_return_success_shape() {
    let module = update_module();
    let result = module
        .execute(
            json!({"key": "executor.default_timeout", "value": 60000, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("update must succeed");
    assert_eq!(result["success"], json!(true));
    assert_eq!(result["key"], json!("executor.default_timeout"));
    assert_eq!(result["old_value"], json!(30000));
    assert_eq!(result["new_value"], json!(60000));
}

// clause: system_modules.update_config.return.redacts_sensitive_segments
#[tokio::test]
async fn update_config_return_redacts_sensitive_segments() {
    let module = update_module();
    let result = module
        .execute(
            json!({"key": "platform.api_token", "value": "supersecret", "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("update must succeed");
    assert_ne!(result["new_value"], json!("supersecret"));
    assert_ne!(result["old_value"], json!("supersecret"));
}

// clause: system_modules.update_config.property.idempotent_false
#[tokio::test]
async fn update_config_property_idempotent_false() {
    let config = make_config();
    let module = UpdateConfigModule::new(Arc::clone(&config), make_emitter());
    module
        .execute(
            json!({"key": "executor.default_timeout", "value": 1000, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("first update");
    module
        .execute(
            json!({"key": "executor.default_timeout", "value": 2000, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("second update");
    assert_eq!(
        config.lock().await.get("executor.default_timeout"),
        Some(json!(2000))
    );
}

// clause: system_modules.update_config.property.pure_false
#[tokio::test]
async fn update_config_property_pure_false() {
    let config = make_config();
    let before = config.lock().await.get("executor.default_timeout");
    let module = UpdateConfigModule::new(Arc::clone(&config), make_emitter());
    module
        .execute(
            json!({"key": "executor.default_timeout", "value": 4444, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("update must succeed");
    let after = config.lock().await.get("executor.default_timeout");
    assert_ne!(before, after);
}

// clause: system_modules.update_config.property.async_false
// DIVERGENCE: in Rust `Module::execute` is `async`. The Python clause asserts
// the function is NOT a coroutine; the Rust contract is the inverse, so we
// assert the awaited call resolves to a value.
#[tokio::test]
async fn update_config_property_async_false() {
    let module = update_module();
    let result = module
        .execute(
            json!({"key": "executor.default_timeout", "value": 7000, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("async execute must resolve");
    assert_eq!(result["success"], json!(true));
}

// ===========================================================================
// Contract 2: system.control.reload_module  (ReloadModule::execute)
// ===========================================================================

// clause: system_modules.reload_module.input.module_id_required
#[tokio::test]
async fn reload_module_input_module_id_required() {
    let module = ReloadModule::new(make_registry(), make_emitter());
    let err = module
        .execute(json!({"reason": "r"}), &dummy_ctx())
        .await
        .expect_err("missing module_id and path_filter must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: system_modules.reload_module.input.reason_required
#[tokio::test]
async fn reload_module_input_reason_required() {
    let module = ReloadModule::new(registry_with_module("math.add"), make_emitter());
    let err = module
        .execute(json!({"module_id": "math.add", "reason": ""}), &dummy_ctx())
        .await
        .expect_err("empty reason must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: system_modules.reload_module.error.module_not_found
#[tokio::test]
async fn reload_module_error_module_not_found() {
    let module = ReloadModule::new(make_registry(), make_emitter());
    let err = module
        .execute(
            json!({"module_id": "missing.module", "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("unknown module must error");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// clause: system_modules.reload_module.error.reload_conflict
#[tokio::test]
async fn reload_module_error_reload_conflict() {
    let module = ReloadModule::new(registry_with_module("math.add"), make_emitter());
    let err = module
        .execute(
            json!({"module_id": "math.add", "path_filter": "math.*", "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("module_id + path_filter must conflict");
    assert_eq!(err.code, ErrorCode::ModuleReloadConflict);
    assert_eq!(code_str(err.code), "MODULE_RELOAD_CONFLICT");
}

// clause: system_modules.reload_module.property.async_false
// DIVERGENCE: `Module::execute` is `async` in Rust; assert the call resolves.
#[tokio::test]
async fn reload_module_property_async_false() {
    let module = ReloadModule::new(registry_with_module("math.add"), make_emitter());
    let result = module
        .execute(
            json!({"module_id": "math.add", "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("async execute must resolve");
    assert_eq!(result["success"], json!(true));
}

// clause: system_modules.reload_module.property.requires_approval
// Rust has no static `annotations` attribute on the module struct; the
// `requires_approval` flag is attached at registration time. Verify the
// registered descriptor carries `requires_approval = true`.
#[test]
fn reload_module_property_requires_approval() {
    let registry = make_registry();
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let executor = Executor::new(Arc::clone(&registry), Config::default());
    register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("registration must succeed");

    let def = registry
        .get_definition("system.control.reload_module")
        .expect("get_definition ok")
        .expect("reload_module registered");
    let annotations = def.annotations.expect("annotations present");
    assert!(annotations.requires_approval);
}

// ===========================================================================
// Contract 3: system.control.toggle_feature  (ToggleFeatureModule::execute)
// ===========================================================================

fn toggle_module(registry: Arc<Registry>, state: Arc<ToggleState>) -> ToggleFeatureModule {
    ToggleFeatureModule::new(registry, make_emitter(), state)
}

// clause: system_modules.toggle_feature.input.module_id_required
#[tokio::test]
async fn toggle_feature_input_module_id_required() {
    let module = toggle_module(make_registry(), Arc::new(ToggleState::new()));
    let err = module
        .execute(
            json!({"module_id": "", "enabled": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("empty module_id must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: system_modules.toggle_feature.input.enabled_must_be_bool
#[tokio::test]
async fn toggle_feature_input_enabled_must_be_bool() {
    let module = toggle_module(
        registry_with_module("math.add"),
        Arc::new(ToggleState::new()),
    );
    let err = module
        .execute(
            json!({"module_id": "math.add", "enabled": "false", "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("string enabled must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: system_modules.toggle_feature.input.enabled_int_rejected
#[tokio::test]
async fn toggle_feature_input_enabled_int_rejected() {
    let module = toggle_module(
        registry_with_module("math.add"),
        Arc::new(ToggleState::new()),
    );
    let err = module
        .execute(
            json!({"module_id": "math.add", "enabled": 1, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("integer enabled must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: system_modules.toggle_feature.input.reason_required
#[tokio::test]
async fn toggle_feature_input_reason_required() {
    let module = toggle_module(
        registry_with_module("math.add"),
        Arc::new(ToggleState::new()),
    );
    let err = module
        .execute(
            json!({"module_id": "math.add", "enabled": false, "reason": ""}),
            &dummy_ctx(),
        )
        .await
        .expect_err("empty reason must be rejected");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: system_modules.toggle_feature.error.module_not_found
#[tokio::test]
async fn toggle_feature_error_module_not_found() {
    let module = toggle_module(make_registry(), Arc::new(ToggleState::new()));
    let err = module
        .execute(
            json!({"module_id": "ghost.module", "enabled": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect_err("unregistered module must error");
    assert_eq!(err.code, ErrorCode::ModuleNotFound);
}

// clause: system_modules.toggle_feature.side_effect.disable_sets_state
#[tokio::test]
async fn toggle_feature_side_effect_disable_sets_state() {
    let state = Arc::new(ToggleState::new());
    let module = toggle_module(registry_with_module("math.add"), Arc::clone(&state));
    module
        .execute(
            json!({"module_id": "math.add", "enabled": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("disable must succeed");
    assert!(state.is_disabled("math.add"));
}

// clause: system_modules.toggle_feature.side_effect.enable_clears_state
#[tokio::test]
async fn toggle_feature_side_effect_enable_clears_state() {
    let state = Arc::new(ToggleState::new());
    state.disable("math.add");
    let module = toggle_module(registry_with_module("math.add"), Arc::clone(&state));
    module
        .execute(
            json!({"module_id": "math.add", "enabled": true, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("enable must succeed");
    assert!(!state.is_disabled("math.add"));
}

// clause: system_modules.toggle_feature.side_effect.emits_toggled_event
#[tokio::test]
async fn toggle_feature_side_effect_emits_toggled_event() {
    use apcore::events::emitter::ApCoreEvent;
    use apcore::events::subscribers::EventSubscriber;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct RecordingSubscriber {
        toggled: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl EventSubscriber for RecordingSubscriber {
        async fn on_event(&self, event: &ApCoreEvent) -> Result<(), apcore::errors::ModuleError> {
            if event.event_type == "apcore.module.toggled" {
                self.toggled.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    let toggled = Arc::new(AtomicUsize::new(0));
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(RecordingSubscriber {
        toggled: Arc::clone(&toggled),
    }));
    let emitter_arc = Arc::new(emitter);
    let module = ToggleFeatureModule::new(
        registry_with_module("math.add"),
        Arc::clone(&emitter_arc),
        Arc::new(ToggleState::new()),
    );
    module
        .execute(
            json!({"module_id": "math.add", "enabled": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("toggle must succeed");
    // `emit` dispatches subscribers on spawned tasks; flush awaits them so the
    // assertion observes the delivered event deterministically.
    emitter_arc.flush(2000).await.expect("flush must succeed");
    assert_eq!(toggled.load(Ordering::SeqCst), 1);
}

// clause: system_modules.toggle_feature.return.success_shape
#[tokio::test]
async fn toggle_feature_return_success_shape() {
    let module = toggle_module(
        registry_with_module("math.add"),
        Arc::new(ToggleState::new()),
    );
    let result = module
        .execute(
            json!({"module_id": "math.add", "enabled": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("toggle must succeed");
    assert_eq!(
        result,
        json!({"success": true, "module_id": "math.add", "enabled": false})
    );
}

// clause: system_modules.toggle_feature.property.idempotent_true
#[tokio::test]
async fn toggle_feature_property_idempotent_true() {
    let state = Arc::new(ToggleState::new());
    let module = toggle_module(registry_with_module("math.add"), Arc::clone(&state));
    let r1 = module
        .execute(
            json!({"module_id": "math.add", "enabled": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("first toggle");
    let r2 = module
        .execute(
            json!({"module_id": "math.add", "enabled": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("second toggle");
    assert_eq!(r1, r2);
    assert!(state.is_disabled("math.add"));
}

// clause: system_modules.toggle_feature.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn toggle_feature_property_thread_safe() {
    let module_ids: Vec<String> = (0..8).map(|i| format!("mod.m{i}")).collect();
    let registry = make_registry();
    for mid in &module_ids {
        register_dummy(&registry, mid);
    }
    let state = Arc::new(ToggleState::new());
    let module = Arc::new(toggle_module(Arc::clone(&registry), Arc::clone(&state)));

    let mut handles = Vec::new();
    for mid in &module_ids {
        let module = Arc::clone(&module);
        let mid = mid.clone();
        handles.push(tokio::spawn(async move {
            module
                .execute(
                    json!({"module_id": mid, "enabled": false, "reason": "r"}),
                    &dummy_ctx(),
                )
                .await
        }));
    }

    let mut ok = 0;
    for h in handles {
        let res = h.await.expect("task must not panic");
        if res.is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, 8);
    for mid in &module_ids {
        assert!(state.is_disabled(mid), "module {mid} must be disabled");
    }
}

// clause: system_modules.toggle_feature.property.async_false
// DIVERGENCE: `Module::execute` is `async` in Rust; assert the call resolves.
#[tokio::test]
async fn toggle_feature_property_async_false() {
    let module = toggle_module(
        registry_with_module("math.add"),
        Arc::new(ToggleState::new()),
    );
    let result = module
        .execute(
            json!({"module_id": "math.add", "enabled": false, "reason": "r"}),
            &dummy_ctx(),
        )
        .await
        .expect("async execute must resolve");
    assert_eq!(result["success"], json!(true));
}

// clause: system_modules.toggle_feature.property.requires_approval
#[test]
fn toggle_feature_property_requires_approval() {
    let registry = make_registry();
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let executor = Executor::new(Arc::clone(&registry), Config::default());
    register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("registration must succeed");

    let def = registry
        .get_definition("system.control.toggle_feature")
        .expect("get_definition ok")
        .expect("toggle_feature registered");
    let annotations = def.annotations.expect("annotations present");
    assert!(annotations.requires_approval);
}

// ===========================================================================
// Contract 4: check_module_disabled
// ===========================================================================

// clause: system_modules.check_module_disabled.error.module_disabled
#[test]
fn check_module_disabled_error_module_disabled() {
    // Previously `#[ignore]`d: "no PUBLIC mutator for that global". There is —
    // `global_toggle_state_arc()` is `pub` and hands back the very `Arc`
    // `check_module_disabled` reads, and `ToggleState::disable` is `pub` too.
    // The reason was written once and nothing re-read it; the skip-asymmetry
    // guard could not report it because it read `#[ignore]` as live.
    //
    // The state IS process-global and this file compiles into the shared `it`
    // binary, so the id is unique to this test and the disable is undone before
    // returning: the global set is keyed by module id, so a unique id touches
    // nothing else even when tests share a process.
    let id = "spec.check.disabled.unique.a";
    let toggles = global_toggle_state_arc();
    assert!(check_module_disabled(id).is_ok(), "precondition: enabled");

    toggles.disable(id);
    let denied = check_module_disabled(id);
    toggles.enable(id);

    let err = denied.expect_err("a disabled module must be refused");
    assert_eq!(err.code, ErrorCode::ModuleDisabled);
    assert!(err.message.contains(id), "the refusal must name the module");
    assert!(
        check_module_disabled(id).is_ok(),
        "postcondition: the global is restored"
    );
}

// clause: system_modules.check_module_disabled.return.none_when_enabled
#[test]
fn check_module_disabled_return_none_when_enabled() {
    // Fresh process-global state reports every id as enabled => Ok(()).
    let id = "spec.check.enabled.unique.b";
    assert!(check_module_disabled(id).is_ok());
}

// clause: system_modules.check_module_disabled.property.pure_read_only
#[test]
fn check_module_disabled_property_pure_read_only() {
    // Repeated reads of an enabled id are stable and never mutate state.
    let id = "spec.check.pure.unique.c";
    assert!(check_module_disabled(id).is_ok());
    assert!(!is_module_disabled(id));
    assert!(check_module_disabled(id).is_ok());
    assert!(!is_module_disabled(id));
}

// clause: system_modules.check_module_disabled.input.registry_param
// DIVERGENCE: the spec declares a 2-arg signature (module_id, registry); the
// Rust impl is single-arg over a process-global toggle state — no `registry`
// parameter exists. Mirrors the Python skip.
#[test]
#[ignore = "system_modules.check_module_disabled.input.registry_param: Rust impl is single-arg over global toggle state; no `registry` parameter (contract gap; src/sys_modules/mod.rs:94)"]
fn check_module_disabled_input_registry_param() {
    let _ = check_module_disabled("x");
}

// ===========================================================================
// Contract 5: is_module_disabled
// ===========================================================================

// clause: system_modules.is_module_disabled.return.true_when_disabled
#[test]
fn is_module_disabled_return_true_when_disabled() {
    // Same correction as `check_module_disabled_error_module_disabled` above:
    // the public mutator exists. Unique id, restored before returning.
    let id = "spec.is.true.unique.f";
    let toggles = global_toggle_state_arc();
    assert!(!is_module_disabled(id), "precondition: enabled");

    toggles.disable(id);
    let observed = is_module_disabled(id);
    toggles.enable(id);

    assert!(observed, "a disabled module must read as disabled");
    assert!(
        !is_module_disabled(id),
        "postcondition: the global is restored"
    );
}

// clause: system_modules.is_module_disabled.return.false_when_enabled
#[test]
fn is_module_disabled_return_false_when_enabled() {
    let id = "spec.is.false.unique.e";
    assert!(!is_module_disabled(id));
}

// clause: system_modules.is_module_disabled.error.never_raises_unknown
#[test]
fn is_module_disabled_error_never_raises_unknown() {
    // Returns false (never raises) for unknown IDs — the function signature is
    // total (returns bool, no Result), so the "never raises" intent holds.
    assert!(!is_module_disabled("totally.unknown.module.id.unique"));
}

// clause: system_modules.is_module_disabled.property.pure_read_only
#[test]
fn is_module_disabled_property_pure_read_only() {
    // Repeated reads are stable and side-effect free.
    let id = "spec.is.pure.unique.f";
    assert!(!is_module_disabled(id));
    assert!(!is_module_disabled(id));
}

// clause: system_modules.is_module_disabled.input.registry_param
// DIVERGENCE: single-arg over a global toggle state; no `registry` parameter.
#[test]
#[ignore = "system_modules.is_module_disabled.input.registry_param: Rust impl is single-arg over global toggle state; no `registry` parameter (contract gap; src/sys_modules/mod.rs:89)"]
fn is_module_disabled_input_registry_param() {
    let _ = is_module_disabled("x");
}

// ===========================================================================
// Contract 6: register_sys_modules
// ===========================================================================

// clause: system_modules.register_sys_modules.side_effect.disabled_returns_empty
#[test]
fn register_sys_modules_side_effect_disabled_returns_empty() {
    let registry = make_registry();
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(false));
    let executor = Executor::new(Arc::clone(&registry), Config::default());
    let ctx = register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("Ok expected when disabled");
    assert!(ctx.registered_modules.is_empty());
}

// A-D-11: when `sys_modules.enabled` is OMITTED from config, the default is
// FALSE (opt-in), so zero system modules are registered. Previously the Rust
// fallback defaulted to true, diverging from Python/TS.
#[test]
fn register_sys_modules_defaults_disabled_when_key_absent() {
    let registry = make_registry();
    // Config::default() does not declare sys_modules.enabled in its data tree.
    // Since the CONFIG_DEFAULTS table landed (sync C4), `get()` resolves it to
    // the canonical default `false` from `apcore/schemas/defaults.schema.json`
    // instead of returning None — same outcome, now sourced from the shared
    // cross-SDK default rather than a local fallback.
    let config = Config::default();
    assert!(
        config.get_declared("sys_modules.enabled").is_none(),
        "precondition: key must not be declared for this test"
    );
    assert_eq!(
        config.get("sys_modules.enabled"),
        Some(json!(false)),
        "canonical default for an undeclared sys_modules.enabled is false"
    );
    let executor = Executor::new(Arc::clone(&registry), Config::default());
    let ctx =
        register_sys_modules(Arc::clone(&registry), &executor, &config, None).expect("Ok expected");
    assert!(
        ctx.registered_modules.is_empty(),
        "absent sys_modules.enabled must default to disabled (zero modules)"
    );
}

// clause: system_modules.register_sys_modules.return.context_components
// DIVERGENCE: Rust returns a typed `SysModulesContext` struct (not a dict). The
// Python keys map to struct fields: error_history, usage_collector, emitter,
// toggle_state. Assert the real struct fields exist / are usable.
#[test]
fn register_sys_modules_return_context_components() {
    let registry = make_registry();
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let executor = Executor::new(Arc::clone(&registry), Config::default());
    let ctx = register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("Ok expected when enabled");
    // error_history and usage_collector are concrete components (Python's
    // error_history / usage_collector keys); emitter + toggle_state are the
    // Rust equivalents of the *_middleware components.
    assert!(!ctx.registered_modules.is_empty());
    // Touch each component field to assert it exists and is usable. A freshly
    // built ErrorHistory has zero recorded entries.
    assert_eq!(ctx.error_history.count(), 0);
    let _ = &ctx.usage_collector;
    let _ = &ctx.emitter;
    let _ = &ctx.toggle_state;
}

// clause: system_modules.register_sys_modules.side_effect.registers_health_modules
#[test]
fn register_sys_modules_side_effect_registers_health_modules() {
    let registry = make_registry();
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let executor = Executor::new(Arc::clone(&registry), Config::default());
    register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("registration must succeed");
    assert!(registry.has("system.health.summary"));
    assert!(registry.has("system.manifest.full"));
}

// clause: system_modules.register_sys_modules.input.fail_on_error_default
// Rust models `fail_on_error` via `SysModulesOptions.fail_on_error`; the Default
// impl sets it to `false`, matching the Python default.
#[test]
fn register_sys_modules_input_fail_on_error_default() {
    let options = SysModulesOptions::default();
    assert!(!options.fail_on_error);
}

// clause: system_modules.register_sys_modules.property.idempotent_false
#[test]
fn register_sys_modules_property_idempotent_false() {
    let registry = make_registry();
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    let executor = Executor::new(Arc::clone(&registry), Config::default());

    register_sys_modules(Arc::clone(&registry), &executor, &config, None)
        .expect("first registration must succeed");

    // Second registration with fail_on_error=true must error (already registered).
    let options = SysModulesOptions {
        fail_on_error: true,
        ..Default::default()
    };
    let result =
        register_sys_modules_with_options(Arc::clone(&registry), &executor, &config, None, options);
    assert!(
        result.is_err(),
        "re-registering must fail when fail_on_error=true"
    );
}

// clause: system_modules.register_sys_modules.property.async_false
// register_sys_modules is a synchronous function in Rust (returns Result
// directly, not a Future). Calling it without `.await` compiles and yields a
// Result — verified by using its value in a sync context.
#[test]
fn register_sys_modules_property_async_false() {
    let registry = make_registry();
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(false));
    let executor = Executor::new(Arc::clone(&registry), Config::default());
    let result: Result<_, _> =
        register_sys_modules(Arc::clone(&registry), &executor, &config, None);
    assert!(result.is_ok());
}
