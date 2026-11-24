//! Exercise the external schema mode through the public binding loaders.

use apcore::{BindingLoader, Config};
use serde_json::{json, Value};
use std::fs;

use super::conformance_env;

#[test]
fn binding_schema_ref_scope() {
    let path = conformance_env::find_fixtures_root().join("binding_schema_ref_scope.json");
    let fixture: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    for case in fixture["test_cases"].as_array().unwrap() {
        let root = tempfile::tempdir().unwrap();
        let binding_dir = root.path().join("bindings");
        fs::create_dir(&binding_dir).unwrap();
        let mut reference = case["schema_ref"].as_str().unwrap().to_string();
        if let Some(schema_file) = case["schema_file"].as_str() {
            let path = root.path().join(schema_file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let contents = if case["invalid_document"].as_bool().unwrap_or(false) {
                "[".into()
            } else {
                json!({"input_schema": {"type": "object"}, "output_schema": {"type": "object"}})
                    .to_string()
            };
            fs::write(&path, contents).unwrap();
            if case["absolute"].as_bool().unwrap_or(false) {
                reference = path.to_str().unwrap().to_string();
            }
        }

        let module_id = "fixture.echo";
        let binding_path = binding_dir.join("fixture.binding.yaml");
        let document = json!({
            "spec_version": "1.0",
            "bindings": [{
                "module_id": module_id,
                "target": "json:dumps",
                "description": "Echo",
                "schema_ref": reference
            }]
        });
        fs::write(&binding_path, document.to_string()).unwrap();

        let mut loader = BindingLoader::new();
        let result = match case["schema_root"].as_str() {
            Some(schema_root) => {
                let config_path = root.path().join("apcore.yaml");
                let config = json!({
                    "version": "2.0.0",
                    "project": {"name": "fixture"},
                    "schema": {"root": schema_root}
                });
                fs::write(&config_path, config.to_string()).unwrap();
                let config = Config::load(&config_path).unwrap();
                loader
                    .load_binding_dir_with_config(Some(&binding_dir), None, Some(&config))
                    .map(|_| ())
            }
            None => loader.load_from_yaml(&binding_path),
        };
        let actual_error = result.err().map_or(Value::Null, |error| json!(error.code));
        assert_eq!(
            actual_error, case["expected"]["error_code"],
            "{}",
            case["id"]
        );
        assert_eq!(
            loader.list_bindings().contains(&module_id),
            case["expected"]["registered"].as_bool().unwrap(),
            "{}",
            case["id"]
        );
    }
}
