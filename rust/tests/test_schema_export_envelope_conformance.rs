//! Drive `schema_export_envelope.json` — the `Registry::export_schema` envelope.
//!
//! Four keys, no more. Rust already emitted the correct shape; this pins it so
//! it stays that way. The other two SDKs did not: apcore-python carried an
//! always-empty `definitions` and apcore-typescript carried `name` / `version` /
//! `tags` / `annotations` / `examples`, making its export a partial,
//! non-conforming duplicate of `system.manifest.module`.

use apcore::errors::ModuleError;
use apcore::registry::ModuleDescriptor;
use apcore::{Context, Module, Registry};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("schema_export_envelope.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("fixture parses")
}

#[derive(Debug)]
struct FixtureModule {
    description: String,
    input_schema: Value,
    output_schema: Value,
}

#[async_trait]
impl Module for FixtureModule {
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }
    fn output_schema(&self) -> Value {
        self.output_schema.clone()
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }
}

/// Register the fixture's module, carrying whatever descriptor metadata it
/// declares — so the test proves the exporter DROPS that metadata rather than
/// proving it was never there.
fn register(registry: &Registry, spec: &Value) -> String {
    let module_id = spec["module_id"].as_str().expect("module_id").to_string();
    let module = FixtureModule {
        description: spec["description"].as_str().unwrap_or("").to_string(),
        input_schema: spec["input_schema"].clone(),
        output_schema: spec["output_schema"].clone(),
    };
    // The fixture's module spec uses the descriptor's own field names, and every
    // optional field carries #[serde(default)] — so deserializing it directly
    // carries whatever metadata the case declares (version, tags, annotations,
    // examples) without this helper having to enumerate them.
    let descriptor: ModuleDescriptor =
        serde_json::from_value(spec.clone()).expect("module spec deserializes as a descriptor");
    registry
        .register(&module_id, Box::new(module), descriptor)
        .expect("fixture module registers");
    module_id
}

#[test]
fn export_schema_envelope_matches_the_fixture() {
    let fx = fixture();
    let envelope_keys: Vec<&str> = fx["envelope_keys"]
        .as_array()
        .expect("envelope_keys")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    for case in fx["test_cases"].as_array().expect("test_cases") {
        let id = case["id"].as_str().expect("id");
        let strict = case["strict"].as_bool().expect("strict");
        let registry = Registry::new();

        let module_id = if let Some(spec) = case.get("module") {
            register(&registry, spec)
        } else {
            case["module_id"].as_str().expect("module_id").to_string()
        };

        let got = registry.export_schema(&module_id, strict);

        if case["expected"].is_null() {
            assert!(
                got.is_none(),
                "{id}: an unregistered module must export None, got {got:?}"
            );
            continue;
        }

        let got = got.unwrap_or_else(|| panic!("{id}: expected an envelope, got None"));
        let obj = got
            .as_object()
            .unwrap_or_else(|| panic!("{id}: not an object"));

        // EXACT key set — a subset check would not catch the extra keys this pins.
        let mut got_keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        got_keys.sort_unstable();
        let mut want_keys = envelope_keys.clone();
        want_keys.sort_unstable();
        assert_eq!(got_keys, want_keys, "{id}: envelope key set");

        assert_eq!(got, case["expected"], "{id}: envelope contents");
    }
}

/// `$defs` live inside `input_schema` where JSON Schema puts them; a top-level
/// `definitions` was always empty on this path and gave callers a second place
/// to look.
#[test]
fn export_schema_has_no_sibling_definitions_key() {
    let fx = fixture();
    let case = fx["test_cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "defs_stay_inside_input_schema_no_sibling_definitions_key")
        .expect("the $defs case is present");

    let registry = Registry::new();
    let module_id = register(&registry, &case["module"]);
    let got = registry.export_schema(&module_id, false).expect("envelope");

    assert!(got.get("definitions").is_none());
    assert!(got["input_schema"].get("$defs").is_some());
}
