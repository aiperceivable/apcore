//! Drive `id_map_from_config.json` — §2.2 / §9.1.1 `id_map.overrides`.
//!
//! Every case runs REAL discovery over a real tree and reads the registered
//! module IDs (`driver_contract.real_discovery`): which file a map entry
//! matches is decided inside stage 2 of `DefaultDiscoverer`, so a driver that
//! only checked which map path the discoverer holds could not see an entry
//! that matches nothing (D-138).
//!
//! Its own test binary: each case sets `APCORE_ID__MAP_OVERRIDES` and the
//! process working directory, neither of which `tests/it.rs` can share across
//! its threads.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::Module;
use apcore::registry::registry::Registry;
use async_trait::async_trait;
use serde_json::{json, Value};

#[path = "conformance_env.rs"]
mod conformance_env;

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("id_map_from_config.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("id_map_from_config.json parses")
}

struct Probe;

#[async_trait]
impl Module for Probe {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "id-map probe"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

/// `driver_contract.config_map_entries`: `.py` becomes this SDK's `.rs`.
fn map_document(entries: &[Value]) -> String {
    let entries: Vec<Value> = entries
        .iter()
        .map(|entry| {
            let file = entry["file"].as_str().expect("file");
            let file = file
                .strip_suffix(".py")
                .map_or_else(|| file.to_string(), |stem| format!("{stem}.rs"));
            json!({"file": file, "id": entry["id"]})
        })
        .collect();
    serde_yaml_ng::to_string(&json!({ "mappings": entries })).expect("map serializes")
}

fn write_tree(root: &Path, case: &Value) {
    std::fs::create_dir_all(root.join("ext/executor/orig")).expect("tree");
    std::fs::write(root.join("ext/executor/orig/mod.rs"), "// probe").expect("module file");

    let config_entries = case["input"]["config_map_entries"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| {
            vec![json!({"file": "executor/orig/mod.py", "id": "executor.renamed.mod"})]
        });
    std::fs::write(root.join("map.yaml"), map_document(&config_entries)).expect("map.yaml");
    std::fs::write(
        root.join("explicit.yaml"),
        map_document(&[json!({"file": "executor/orig/mod.py", "id": "executor.explicit.mod"})]),
    )
    .expect("explicit.yaml");

    let mut document = json!({
        "version": "1.0.0",
        "project": {"name": "id-map-probe"},
        "extensions": {"root": "./ext"}
    });
    if case["input"]["declare_override"]
        .as_bool()
        .expect("declare_override")
    {
        document["id_map"] = json!({"overrides": "./map.yaml"});
    }
    std::fs::write(
        root.join("apcore.json"),
        serde_json::to_string_pretty(&document).expect("document"),
    )
    .expect("apcore.json");
}

fn discovered_ids(case: &Value) -> HashSet<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tree(dir.path(), case);

    let env: Vec<(String, String)> = case["input"]["env"]
        .as_object()
        .map(|vars| {
            vars.iter()
                .map(|(k, v)| (k.clone(), v.as_str().expect("env value").to_string()))
                .collect()
        })
        .unwrap_or_default();

    let previous_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir.path()).expect("enter the case root");
    // SAFETY: this binary runs one test; nothing reads the environment
    // concurrently.
    unsafe { std::env::remove_var("APCORE_ID__MAP_OVERRIDES") };
    for (key, value) in &env {
        unsafe { std::env::set_var(key, value) };
    }

    let result = {
        let config = Config::load(Path::new("apcore.json")).expect("config loads");
        let mut discoverer = apcore::registry::DefaultDiscoverer::from_config(&config)
            .with_factory(Arc::new(|_file, _entry| {
                Ok(Some(Arc::new(Probe) as Arc<dyn Module>))
            }));
        if case["input"]["explicit_argument"]
            .as_bool()
            .expect("explicit_argument")
        {
            discoverer = discoverer.with_id_map(Some("./explicit.yaml"));
        }
        let registry = Registry::new();
        registry.set_extension_roots_from_config(&config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        runtime
            .block_on(registry.discover(&discoverer))
            .expect("discovery succeeds");
        registry
            .list(None, None, None)
            .into_iter()
            .collect::<HashSet<_>>()
    };

    for (key, _) in &env {
        unsafe { std::env::remove_var(key) };
    }
    std::env::set_current_dir(previous_cwd).expect("restore cwd");
    result
}

#[test]
fn conformance_id_map_from_config() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "the fixture drove nothing");

    for case in cases {
        let id = case["id"].as_str().expect("case id");
        let expected: HashSet<String> = case["expected"]["module_ids"]
            .as_array()
            .expect("module_ids")
            .iter()
            .map(|v| v.as_str().expect("id").to_string())
            .collect();
        assert_eq!(discovered_ids(case), expected, "case {id}");
    }
}
