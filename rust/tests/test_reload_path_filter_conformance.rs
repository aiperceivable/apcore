//! Drive `reload_path_filter.json` — granular reload via the `path_filter`
//! glob on `system.control.reload_module`
//! (docs/features/system-modules.md#14-granular-reload-via-path-filtering).
//!
//! `tests/test_reload_path_filter.rs` hand-transcribes two of these four cases;
//! this file replays the canonical fixture so a case added upstream reaches
//! Rust automatically.
//!
//! Response-shape note: the bulk (`path_filter`) branch answers with a
//! `reloaded_modules` array, while the single (`module_id`) branch answers with
//! a scalar `module_id` and no array — identical in apcore-python
//! (`src/apcore/sys_modules/control.py`). The fixture's `reloaded_modules_set`
//! is therefore read from the array when present and from `{module_id}`
//! otherwise; nothing is assumed when neither is present.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::context::{Context, Identity};
use apcore::events::emitter::EventEmitter;
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::registry::{ModuleDescriptor, Registry};
use apcore::sys_modules::control::ReloadModule;
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("reload_path_filter.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("reload_path_filter.json parses")
}

fn dummy_ctx() -> Context<Value> {
    Context::<Value>::new(Identity::new(
        "@conformance".to_string(),
        "conformance".to_string(),
        vec![],
        HashMap::default(),
    ))
}

fn register_dummy(registry: &Arc<Registry>, id: &str) {
    struct Dummy;
    #[async_trait::async_trait]
    impl Module for Dummy {
        fn description(&self) -> &'static str {
            "conformance fixture module"
        }
        fn input_schema(&self) -> Value {
            serde_json::json!({})
        }
        fn output_schema(&self) -> Value {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _i: Value,
            _c: &Context<Value>,
        ) -> Result<Value, apcore::errors::ModuleError> {
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
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    registry
        .register_internal(id, Box::new(Dummy), descriptor)
        .expect("register_internal");
}

/// Derive the fixture's `reloaded_modules_set` from a reload response.
fn reloaded_set(response: &Value) -> Vec<String> {
    if let Some(arr) = response.get("reloaded_modules").and_then(Value::as_array) {
        let mut ids: Vec<String> = arr
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("reloaded_modules entry is a string")
                    .to_string()
            })
            .collect();
        ids.sort();
        return ids;
    }
    match response.get("module_id") {
        Some(Value::String(id)) => vec![id.clone()],
        _ => panic!(
            "reload response carries neither `reloaded_modules` nor a scalar \
             `module_id`, so the reloaded set cannot be determined: {response}"
        ),
    }
}

#[tokio::test]
async fn conformance_reload_path_filter() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");

        let registry = Arc::new(Registry::new());
        for module_id in tc["registered_modules"]
            .as_array()
            .unwrap_or_else(|| panic!("[{id}] case has no registered_modules"))
        {
            register_dummy(
                &registry,
                module_id
                    .as_str()
                    .expect("registered module id is a string"),
            );
        }

        // Attach a discoverer that still "sees" every registered module, modelling
        // a filesystem discoverer whose files are untouched by the reload.
        //
        // Without this the driver asserted the fixture's semantics on a false
        // premise: the fixture states "Only matching IDs are re-discovered and
        // reloaded", but the registry had no discoverer at all, so the only way
        // this driver could pass was an implementation that reported success
        // WITHOUT re-discovering — which is exactly the defect fixed in
        // sync-2026-08-27 (A-D-011). apcore-python's driver for this same fixture
        // patches `_rediscover_module` with a restoring stub for the same reason.
        let all_ids: Vec<String> = tc["registered_modules"]
            .as_array()
            .expect("registered_modules is an array")
            .iter()
            .map(|v| v.as_str().expect("module id is a string").to_string())
            .collect();
        registry.set_discoverer(Box::new(RestoringDiscoverer { ids: all_ids }));

        let audit_store: Arc<dyn apcore::sys_modules::audit::AuditStore> =
            Arc::new(apcore::sys_modules::audit::InMemoryAuditStore::new());
        let reload = ReloadModule::new(Arc::clone(&registry), Arc::new(EventEmitter::new()))
            .with_audit_store(Some(Arc::clone(&audit_store)));
        let outcome = reload.execute(tc["input"].clone(), &dummy_ctx()).await;

        let expected = tc["expected"]
            .as_object()
            .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

        for (field, want) in expected {
            match field.as_str() {
                // Prose the fixture attaches to the expectation block.
                "_note" => {}
                "error_code" => {
                    let err = outcome
                        .as_ref()
                        .err()
                        .unwrap_or_else(|| panic!("[{id}] expected an error, got Ok"));
                    let actual = serde_json::to_value(err.code).expect("ErrorCode serializes");
                    assert_eq!(&actual, want, "[{id}] error_code");
                }
                "error" => {
                    // The fixture states `error: null` — the call must succeed.
                    assert!(want.is_null(), "[{id}] unexpected non-null `error` shape");
                    assert!(
                        outcome.is_ok(),
                        "[{id}] expected no error, got {:?}",
                        outcome.as_ref().err()
                    );
                }
                "success" => {
                    let response = outcome
                        .as_ref()
                        .unwrap_or_else(|e| panic!("[{id}] expected success, got {e:?}"));
                    assert_eq!(&response["success"], want, "[{id}] success");
                }
                "reloaded_modules_set" => {
                    let response = outcome
                        .as_ref()
                        .unwrap_or_else(|e| panic!("[{id}] expected success, got {e:?}"));
                    let mut want_ids: Vec<String> = want
                        .as_array()
                        .expect("reloaded_modules_set is an array")
                        .iter()
                        .map(|v| v.as_str().expect("module id is a string").to_string())
                        .collect();
                    want_ids.sort();
                    // An empty expected set means the response must report no
                    // reload at all, which the single-reload shape cannot express.
                    let actual = if want_ids.is_empty() {
                        response
                            .get("reloaded_modules")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().map(std::string::ToString::to_string).collect())
                            .unwrap_or_else(|| {
                                panic!("[{id}] zero-match reload must still return an array")
                            })
                    } else {
                        reloaded_set(response)
                    };
                    assert_eq!(actual, want_ids, "[{id}] reloaded_modules_set");
                }
                "audit_target_module_ids_set" => {
                    // D-111. Asserted as a SET of concrete ids rather than a
                    // count: an aggregate entry keyed on the glob has count 1,
                    // which a count assertion accepts whenever the glob matches
                    // one module, and its target is one `query(module_id)` can
                    // never find.
                    let mut want_ids: Vec<String> = want
                        .as_array()
                        .expect("audit_target_module_ids_set is an array")
                        .iter()
                        .map(|v| v.as_str().expect("module id is a string").to_string())
                        .collect();
                    want_ids.sort();
                    let entries = audit_store.query(None, None, None).await.expect("query");
                    let mut got: Vec<String> =
                        entries.iter().map(|e| e.target_module_id.clone()).collect();
                    got.sort();
                    assert_eq!(got, want_ids, "[{id}] audit targets");
                    for module_id in &want_ids {
                        let found = audit_store
                            .query(Some(module_id), None, None)
                            .await
                            .expect("query by module id");
                        assert_eq!(
                            found
                                .iter()
                                .map(|e| e.target_module_id.clone())
                                .collect::<Vec<_>>(),
                            vec![module_id.clone()],
                            "[{id}] {module_id} is not findable by the accessor the store exists for"
                        );
                    }
                }
                "audit_correlation_ids_are_equal_and_non_empty" => {
                    // A property, not a value: the id is generated per call.
                    // All equal rejects a fresh id per entry, which groups
                    // nothing; non-empty rejects leaving the field unset.
                    assert_eq!(want, &serde_json::Value::Bool(true));
                    let entries = audit_store.query(None, None, None).await.expect("query");
                    let ids: std::collections::HashSet<&str> =
                        entries.iter().map(|e| e.correlation_id.as_str()).collect();
                    assert_eq!(
                        ids.len(),
                        1,
                        "[{id}] entries from one bulk reload must share one id: {ids:?}"
                    );
                    assert!(!ids.contains(""), "[{id}] the correlation id must be set");
                }
                other => panic!(
                    "[{id}] reload_path_filter.json grew expectation `{other}` that this \
                     driver does not check — teach the driver, do not skip it"
                ),
            }
        }
    }
}

/// Re-supplies a fixed set of module ids on `discover()`, standing in for the
/// filesystem discoverer that repopulates the registry after an unregister.
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
        Ok(self
            .ids
            .iter()
            .map(|id| apcore::registry::registry::DiscoveredModule {
                name: id.clone(),
                source: "conformance".to_string(),
                descriptor: ModuleDescriptor {
                    module_id: id.clone(),
                    name: None,
                    description: "restored".to_string(),
                    documentation: None,
                    input_schema: serde_json::json!({}),
                    output_schema: serde_json::json!({}),
                    version: "1.0.0".to_string(),
                    tags: vec![],
                    annotations: Some(ModuleAnnotations::default()),
                    examples: vec![],
                    metadata: HashMap::new(),
                    display: None,
                    sunset_date: None,
                    dependencies: vec![],
                    enabled: true,
                },
                module: Arc::new(Restored),
            })
            .collect())
    }
}
