//! Drive `multi_root_discovery.json` — §9.1.1 `extensions.roots` (#118 D-70).
//!
//! This SDK is the one that read the key and honoured half of it: the paths
//! reached `Registry::set_extension_roots_from_config`, and the namespaces were
//! dropped on the floor — so `roots` gave multiple roots here and multiple
//! NAMESPACED roots in apcore-python and apcore-typescript, from one document.
//!
//! The two roots derive the SAME unprefixed ID on purpose. Without the prefix
//! they collide, so the namespace is observable rather than cosmetic.

use std::sync::Arc;

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::Module;
use apcore::registry::Registry;
use async_trait::async_trait;
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

struct StubModule;

#[async_trait]
impl Module for StubModule {
    fn description(&self) -> &'static str {
        "Discoverable probe module."
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(serde_json::json!({}))
    }
}

fn fixture() -> Value {
    let path = find_fixtures_root().join("multi_root_discovery.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("multi_root_discovery.json parses")
}

/// Rewrite the fixture's `./alpha` / `./beta` to absolute paths under `dir`.
///
/// The fixture states them relative to the project root; this SDK's discoverer
/// takes paths as given, and the test process's working directory is the crate,
/// not the tree.
fn absolutise(value: &Value, dir: &std::path::Path) -> Value {
    match value {
        Value::String(s) if s.starts_with("./") => {
            Value::String(dir.join(&s[2..]).to_string_lossy().into_owned())
        }
        Value::Array(items) => Value::Array(items.iter().map(|i| absolutise(i, dir)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), absolutise(v, dir)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[tokio::test]
async fn conformance_multi_root_discovery() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "the fixture drove nothing");

    for case in cases {
        let id = case["id"].as_str().expect("case id");
        let expected = &case["expected"];

        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["alpha", "beta"] {
            let leaf = dir.path().join(name).join("executor").join("svc");
            std::fs::create_dir_all(&leaf).expect("mkdir");
            std::fs::write(leaf.join("mod.rs"), "// probe\n").expect("write");
        }

        let extensions = absolutise(&case["input"]["extensions"], dir.path());
        let raw = serde_json::json!({
            "version": "1.0",
            "project": {"name": "multi-root-probe"},
            "extensions": extensions
        });
        let config: Config = serde_json::from_value(raw).expect("probe config parses");

        let factory: apcore::ModuleFactory =
            Arc::new(|_file, _entry| Ok(Some(Arc::new(StubModule) as Arc<dyn Module>)));
        let registry = Arc::new(Registry::new());
        registry.set_extension_roots_from_config(&config);
        registry.set_discoverer(Box::new(
            apcore::DefaultDiscoverer::from_config(&config).with_factory(factory),
        ));

        let outcome = registry.discover_internal().await;

        if expected["raises"].as_bool() == Some(true) {
            let err = outcome.expect_err(&format!("case {id} must be rejected"));
            let needle = expected["error_message_contains"].as_str().expect("needle");
            assert!(err.message.contains(needle), "case {id}: {}", err.message);
            continue;
        }

        outcome.unwrap_or_else(|e| panic!("case {id}: unexpected error: {e}"));
        let mut ids = registry.module_ids();
        ids.sort();
        let want: Vec<String> = expected["module_ids"]
            .as_array()
            .expect("module_ids")
            .iter()
            .map(|v| v.as_str().expect("id").to_string())
            .collect();
        assert_eq!(ids, want, "case {id}");
    }
}
