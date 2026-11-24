//! Integration tests proving that after the registry-unification refactor:
//!
//!  - C2: `disable`/`enable` through the `system.control.toggle_feature`
//!    pipeline is observed by the executor's module-lookup step.
//!  - C3: `APCore` constructors work inside a tokio runtime (no more
//!    `blocking_lock` panics, no more `Arc::get_mut` hack, no more
//!    parallel registries).
//!
//! These tests fail on the pre-refactor codebase because sys modules were
//! registered into a separate `Arc<Mutex<Registry>>` from the one the
//! executor's pipeline inspected.

use std::sync::Arc;

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Test module
// ---------------------------------------------------------------------------

struct Dummy;

#[async_trait]
impl Module for Dummy {
    fn description(&self) -> &'static str {
        "dummy"
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"ok": true}))
    }
}

fn enable_events_config() -> Config {
    let mut config = Config::default();
    // System modules are opt-in (default false; sync finding A-D-11) — enable
    // explicitly so the control modules register.
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    config
}

// ---------------------------------------------------------------------------
// C3: constructor runs fine inside a tokio runtime, and sys modules are
// registered into the SAME registry that the executor pipeline uses.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn apcore_constructor_works_inside_tokio_runtime() {
    // System modules are opt-in (default false; sync finding A-D-11) — enable
    // them so this test can assert they register inside a tokio runtime.
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    let apcore = APCore::with_config(config);

    let registered = apcore.list_modules(None, None);

    // Health, manifest, and usage modules are registered when
    // sys_modules.enabled=true. These used to be absent when the constructor
    // was invoked inside a tokio runtime (the pre-refactor guard bailed out
    // to avoid a blocking_lock panic).
    assert!(
        registered.contains(&"system.health.summary".to_string()),
        "system.health.summary should be auto-registered; got: {registered:?}"
    );
    assert!(
        registered.contains(&"system.manifest.full".to_string()),
        "system.manifest.full should be auto-registered; got: {registered:?}"
    );
    assert!(
        registered.contains(&"system.usage.summary".to_string()),
        "system.usage.summary should be auto-registered; got: {registered:?}"
    );
}

#[tokio::test]
async fn apcore_toggle_feature_is_reachable_from_executor_when_events_enabled() {
    // When events are enabled, control modules including
    // `system.control.toggle_feature` should be registered into the same
    // registry the executor consults.
    let apcore = APCore::with_config(enable_events_config());

    let registered = apcore.list_modules(None, None);
    assert!(
        registered.contains(&"system.control.toggle_feature".to_string()),
        "system.control.toggle_feature should be registered in the executor's registry; got: {registered:?}"
    );

    // And calling it through the executor pipeline must succeed. Use a unique
    // module id so the process-global ToggleState (sync finding A-D-12) cannot
    // collide with other tests in this binary.
    apcore
        .register("demo.module.toggle_reachable", Box::new(Dummy))
        .expect("register should succeed");
    let result = apcore
        .call(
            "system.control.toggle_feature",
            json!({
                "module_id": "demo.module.toggle_reachable",
                "enabled": false,
                "reason": "test",
            }),
            None,
            None,
        )
        .await;
    assert!(
        result.is_ok(),
        "toggle_feature should be callable through executor: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// C2: disable via pipeline is observed by subsequent calls.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn disable_through_pipeline_is_observed_by_executor() {
    // Unique module id avoids collisions in the process-global ToggleState.
    let module_id = "demo.module.disable_observed";
    let apcore = APCore::with_config(enable_events_config());
    apcore
        .register(module_id, Box::new(Dummy))
        .expect("register should succeed");

    // Disable via the pipeline.
    let res = apcore.disable(module_id, Some("integration")).await;
    assert!(res.is_ok(), "disable should succeed: {res:?}");

    // Subsequent call must be blocked by the pipeline's module_lookup step.
    let err = apcore
        .call(module_id, json!({}), None, None)
        .await
        .expect_err("call on disabled module must fail");
    assert_eq!(err.code, ErrorCode::ModuleDisabled);
}

#[tokio::test]
async fn enable_through_pipeline_restores_execution() {
    // Unique module id avoids collisions in the process-global ToggleState.
    let module_id = "demo.module.enable_restores";
    let apcore = APCore::with_config(enable_events_config());
    apcore
        .register(module_id, Box::new(Dummy))
        .expect("register should succeed");

    apcore
        .disable(module_id, Some("first"))
        .await
        .expect("disable should succeed");
    apcore
        .enable(module_id, Some("second"))
        .await
        .expect("enable should succeed");

    let ok = apcore
        .call(module_id, json!({}), None, None)
        .await
        .expect("call after re-enable should succeed");
    assert_eq!(ok["ok"], true);
}

// ---------------------------------------------------------------------------
// Concurrency: interior mutability must allow concurrent register + read.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_register_and_list_no_deadlock() {
    let apcore = Arc::new(APCore::new());

    let mut handles = Vec::new();

    // 100 writers (register) with yield points to force task interleaving.
    for i in 0..100 {
        let ac = Arc::clone(&apcore);
        handles.push(tokio::spawn(async move {
            if i % 10 == 0 {
                tokio::task::yield_now().await;
            }
            let id = format!("demo.module_{i}");
            ac.register(&id, Box::new(Dummy)).expect("register");
        }));
    }

    // 200 readers (list) with a yield up front to widen the interleaving window.
    for _ in 0..200 {
        let ac = Arc::clone(&apcore);
        handles.push(tokio::spawn(async move {
            tokio::task::yield_now().await;
            let _ = ac.list_modules(None, None);
        }));
    }

    // Wait for all tasks with a generous timeout — a deadlock manifests as a
    // timeout here on the multi-threaded runtime.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        futures_util::future::join_all(handles),
    )
    .await;

    let joined = result.expect("concurrent operations timed out — possible deadlock");
    for h in joined {
        h.expect("task must not panic");
    }

    let modules = apcore.list_modules(None, Some("demo."));
    assert_eq!(
        modules.len(),
        100,
        "all 100 demo modules should be registered"
    );
}

// ---------------------------------------------------------------------------
// Issue 2: ToggleFeatureModule TOCTOU — attempting to toggle a non-existent
// module must fail without polluting ToggleState so subsequent registration
// of the same ID is not silently disabled.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn toggle_feature_on_nonexistent_module_does_not_pollute_toggle_state() {
    let apcore = APCore::with_config(enable_events_config());

    // Attempt to disable a non-existent module — must fail.
    let result = apcore.disable("does.not.exist", Some("testing")).await;
    assert!(result.is_err(), "disable of nonexistent module should fail");

    // Now register a module with the same ID and verify it executes
    // successfully (i.e. ToggleState is not holding a stale `disabled` entry).
    apcore
        .register("does.not.exist", Box::new(Dummy))
        .expect("register after failed disable should succeed");

    let ok = apcore
        .call("does.not.exist", json!({}), None, None)
        .await
        .expect("call should succeed — module must not be stuck in disabled state");
    assert_eq!(ok["ok"], true);
}
