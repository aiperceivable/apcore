//! Issue #45.4 — granular reload via `path_filter`.
//!
//! Verifies that ReloadModule with `path_filter` only re-discovers / unregisters
//! modules whose IDs match the glob, leaving others intact.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::context::{Context, Identity};
use apcore::events::emitter::EventEmitter;
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::sys_modules::control::ReloadModule;

fn dummy_ctx() -> Context<serde_json::Value> {
    Context::<serde_json::Value>::new(Identity::new(
        "@test".to_string(),
        "test".to_string(),
        vec![],
        HashMap::default(),
    ))
}

fn register_dummy(registry: &Arc<Registry>, id: &str) {
    struct Dummy;
    #[async_trait::async_trait]
    impl Module for Dummy {
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
            _i: serde_json::Value,
            _c: &Context<serde_json::Value>,
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
        .register_internal(id, Box::new(Dummy), descriptor)
        .expect("register_internal");
}

#[tokio::test]
async fn path_filter_only_reloads_matching_modules() {
    let registry = Arc::new(Registry::new());
    let emitter = Arc::new(EventEmitter::new());

    register_dummy(&registry, "executor.email.send");
    register_dummy(&registry, "executor.email.recv");
    register_dummy(&registry, "executor.sms.send");
    register_dummy(&registry, "common.helpers.format");

    // A discoverer MUST be attached: bulk reload unregisters before re-discovering,
    // so without one the matched modules would be deleted with nothing to restore
    // them. That case is covered by `bulk_reload_without_discoverer_fails_loudly`.
    registry.set_discoverer(Box::new(RestoringDiscoverer {
        ids: vec![
            "executor.email.send".to_string(),
            "executor.email.recv".to_string(),
        ],
    }));

    let reload = ReloadModule::new(Arc::clone(&registry), Arc::clone(&emitter));
    let result = reload
        .execute(
            serde_json::json!({
                "path_filter": "executor.email.*",
                "reason": "granular reload",
            }),
            &dummy_ctx(),
        )
        .await
        .expect("path_filter reload should succeed");

    let reloaded = result["reloaded_modules"]
        .as_array()
        .expect("reloaded_modules array");
    let reloaded_ids: Vec<String> = reloaded
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();

    // Only executor.email.* should be reloaded.
    assert!(
        reloaded_ids.contains(&"executor.email.send".to_string()),
        "executor.email.send should be in reloaded list, got {reloaded_ids:?}"
    );
    assert!(
        reloaded_ids.contains(&"executor.email.recv".to_string()),
        "executor.email.recv should be in reloaded list, got {reloaded_ids:?}"
    );
    assert!(
        !reloaded_ids.contains(&"executor.sms.send".to_string()),
        "executor.sms.send must NOT match executor.email.*, got {reloaded_ids:?}"
    );
    assert!(
        !reloaded_ids.contains(&"common.helpers.format".to_string()),
        "common.helpers.format must NOT match, got {reloaded_ids:?}"
    );

    // Non-matching modules remain registered.
    assert!(registry.has("executor.sms.send"));
    assert!(registry.has("common.helpers.format"));

    // THE ASSERTION THIS TEST WAS MISSING. Before the sync-2026-08-27 fix,
    // `execute_bulk` unregistered every match and never re-discovered, so both
    // modules below were DELETED while `reloaded_modules` listed them as reloaded.
    // The test passed because it only ever checked the NON-matching ids.
    assert!(
        registry.has("executor.email.send"),
        "a module reported as reloaded must still be registered"
    );
    assert!(
        registry.has("executor.email.recv"),
        "a module reported as reloaded must still be registered"
    );
}

#[tokio::test]
async fn module_id_and_path_filter_are_mutually_exclusive() {
    let registry = Arc::new(Registry::new());
    let emitter = Arc::new(EventEmitter::new());
    let reload = ReloadModule::new(registry, emitter);
    let err = reload
        .execute(
            serde_json::json!({
                "module_id": "executor.email.send",
                "path_filter": "executor.*",
                "reason": "should fail",
            }),
            &dummy_ctx(),
        )
        .await
        .expect_err("should error on conflict");
    assert_eq!(
        err.code,
        apcore::errors::ErrorCode::ModuleReloadConflict,
        "expected MODULE_RELOAD_CONFLICT"
    );
}

// ---------------------------------------------------------------------------
// sync-2026-08-27 finding A-D-011 / A-D-012 — reload must not delete modules
// and then report success.
// ---------------------------------------------------------------------------

/// A discoverer that re-supplies a fixed set of ids, standing in for the
/// filesystem discoverer that would normally repopulate the registry.
struct RestoringDiscoverer {
    ids: Vec<String>,
}

#[async_trait::async_trait]
impl apcore::registry::registry::Discoverer for RestoringDiscoverer {
    async fn discover(
        &self,
        _roots: &[String],
    ) -> Result<Vec<apcore::registry::registry::DiscoveredModule>, apcore::errors::ModuleError>
    {
        Ok(self.ids.iter().map(|id| make_discovered(id)).collect())
    }
}

/// A discoverer that always returns nothing — models a discoverer that runs
/// cleanly but fails to bring the unregistered module back.
struct EmptyDiscoverer;

#[async_trait::async_trait]
impl apcore::registry::registry::Discoverer for EmptyDiscoverer {
    async fn discover(
        &self,
        _roots: &[String],
    ) -> Result<Vec<apcore::registry::registry::DiscoveredModule>, apcore::errors::ModuleError>
    {
        Ok(vec![])
    }
}

fn make_discovered(id: &str) -> apcore::registry::registry::DiscoveredModule {
    struct Restored;
    #[async_trait::async_trait]
    impl Module for Restored {
        fn description(&self) -> &'static str {
            "restored"
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
    apcore::registry::registry::DiscoveredModule {
        name: id.to_string(),
        source: "test".to_string(),
        descriptor: ModuleDescriptor {
            module_id: id.to_string(),
            name: None,
            description: "restored".to_string(),
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
        },
        module: Arc::new(Restored),
    }
}

#[tokio::test]
async fn bulk_reload_without_discoverer_fails_loudly() {
    // Before the fix this returned `{"success": true, "reloaded_modules": [...]}`
    // after deleting every match, because nothing re-discovered and nothing
    // verified. With no discoverer there is no way to restore them, so the
    // only safe outcome is a loud RELOAD_FAILED.
    let registry = Arc::new(Registry::new());
    let emitter = Arc::new(EventEmitter::new());
    register_dummy(&registry, "executor.email.send");
    register_dummy(&registry, "common.helpers.format");

    let reload = ReloadModule::new(Arc::clone(&registry), Arc::clone(&emitter));
    let err = reload
        .execute(
            serde_json::json!({ "path_filter": "executor.*", "reason": "no discoverer" }),
            &dummy_ctx(),
        )
        .await
        .expect_err("bulk reload without a discoverer must not report success");

    assert_eq!(err.code, apcore::errors::ErrorCode::ReloadFailed);
    assert!(
        err.message.contains("executor.email.send"),
        "the error must name what was affected, got: {}",
        err.message
    );
    // Untouched by the filter, and must survive the failed batch.
    assert!(registry.has("common.helpers.format"));
}

#[tokio::test]
async fn bulk_reload_reports_modules_the_discoverer_did_not_restore() {
    // Discovery succeeds but brings nothing back: the module is gone, so the
    // call must fail rather than list it as reloaded.
    let registry = Arc::new(Registry::new());
    let emitter = Arc::new(EventEmitter::new());
    register_dummy(&registry, "executor.email.send");
    registry.set_discoverer(Box::new(EmptyDiscoverer));

    let reload = ReloadModule::new(Arc::clone(&registry), Arc::clone(&emitter));
    let err = reload
        .execute(
            serde_json::json!({ "path_filter": "executor.*", "reason": "empty discoverer" }),
            &dummy_ctx(),
        )
        .await
        .expect_err("modules absent after re-discovery must surface as an error");

    assert_eq!(err.code, apcore::errors::ErrorCode::ReloadFailed);
    assert!(
        err.message.contains("executor.email.send"),
        "the error must name the module that vanished, got: {}",
        err.message
    );
}

#[tokio::test]
async fn single_reload_without_discoverer_fails_loudly() {
    // A-D-012: the single-module path swallowed the discovery error as
    // "best-effort" and returned success for a module it had just deleted.
    let registry = Arc::new(Registry::new());
    let emitter = Arc::new(EventEmitter::new());
    register_dummy(&registry, "executor.email.send");

    let reload = ReloadModule::new(Arc::clone(&registry), Arc::clone(&emitter));
    let err = reload
        .execute(
            serde_json::json!({ "module_id": "executor.email.send", "reason": "no discoverer" }),
            &dummy_ctx(),
        )
        .await
        .expect_err("single reload without a discoverer must not report success");

    assert_eq!(err.code, apcore::errors::ErrorCode::ReloadFailed);
}
