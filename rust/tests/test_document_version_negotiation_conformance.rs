//! Drive A14 through both public on-disk document loaders.

use apcore::{Config, SchemaLoader};
use serde_json::{json, Value};
use std::fs;

use super::conformance_env;

#[test]
fn document_version_negotiation() {
    let fixture = conformance_env::find_fixtures_root().join("document_version_negotiation.json");
    let document: Value = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
    for case in document["test_cases"].as_array().unwrap() {
        let root = tempfile::tempdir().unwrap();
        let (path, mut document) = match case["format"].as_str().unwrap() {
            "config" => (
                root.path().join("apcore.yaml"),
                json!({"apcore": {"project": {"name": "conformance"}}}),
            ),
            "schema" => (
                root.path().join("example.schema.yaml"),
                json!({
                    "module_id": "example",
                    "description": "Conformance example",
                    "input_schema": {"type": "object"},
                    "output_schema": {"type": "object"}
                }),
            ),
            format => panic!("unknown fixture format {format}"),
        };
        if let Some(version) = case.get("version") {
            if document.get("apcore").is_some() {
                document["apcore"]["version"] = version.clone();
            } else {
                document["version"] = version.clone();
            }
        }
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let result = if document.get("apcore").is_some() {
            Config::load(&path).map(|_| ())
        } else {
            SchemaLoader::with_config(&Config::default(), Some(root.path()))
                .load("example")
                .map(|_| ())
        };
        let actual = result.err().map_or(Value::Null, |error| json!(error.code));
        assert_eq!(actual, case["expected"]["error_code"], "{}", case["id"]);
    }
}
