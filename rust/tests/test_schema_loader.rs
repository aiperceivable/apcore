//! Tests for SchemaLoader — loading schemas from files and inline values.

use apcore::schema::{SchemaLoader, SchemaStrategy};
use apcore::Config;
use serde_json::json;
use std::io::Write;

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

#[test]
fn test_schema_loader_new_defaults_to_yaml_first() {
    let loader = SchemaLoader::new();
    assert_eq!(loader.strategy, SchemaStrategy::YamlFirst);
}

#[test]
fn test_schema_loader_with_strategy() {
    let loader = SchemaLoader::with_strategy(SchemaStrategy::NativeFirst);
    assert_eq!(loader.strategy, SchemaStrategy::NativeFirst);
}

#[test]
fn test_schema_loader_with_strategy_yaml_only() {
    let loader = SchemaLoader::with_strategy(SchemaStrategy::YamlOnly);
    assert_eq!(loader.strategy, SchemaStrategy::YamlOnly);
}

#[test]
fn test_schema_loader_default() {
    let loader = SchemaLoader::default();
    assert_eq!(loader.strategy, SchemaStrategy::YamlFirst);
}

// ---------------------------------------------------------------------------
// load_from_value
// ---------------------------------------------------------------------------

#[test]
fn test_schema_loader_load_from_value_and_get() {
    let mut loader = SchemaLoader::new();
    let schema = json!({ "type": "object" });
    loader
        .load_from_value("test_schema", schema.clone())
        .unwrap();
    let result = loader.get("test_schema").unwrap();
    assert_eq!(result, &schema);
}

#[test]
fn test_schema_loader_load_from_value_overwrites() {
    let mut loader = SchemaLoader::new();
    loader
        .load_from_value("s", json!({ "type": "string" }))
        .unwrap();
    loader
        .load_from_value("s", json!({ "type": "integer" }))
        .unwrap();
    assert_eq!(loader.get("s").unwrap(), &json!({ "type": "integer" }));
}

#[test]
fn test_schema_loader_get_missing_returns_none() {
    let loader = SchemaLoader::new();
    assert!(loader.get("nonexistent").is_none());
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[test]
fn test_schema_loader_list_empty() {
    let loader = SchemaLoader::new();
    assert!(loader.list().is_empty());
}

#[test]
fn test_schema_loader_list_returns_all_names() {
    let mut loader = SchemaLoader::new();
    loader.load_from_value("a", json!({})).unwrap();
    loader.load_from_value("b", json!({})).unwrap();
    let mut names = loader.list();
    names.sort_unstable();
    assert_eq!(names, vec!["a", "b"]);
}

// ---------------------------------------------------------------------------
// load_from_file — JSON
// ---------------------------------------------------------------------------

#[test]
fn test_schema_loader_load_from_json_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.json");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        r#"{{"type": "object", "properties": {{"x": {{"type": "integer"}}}}}}"#
    )
    .unwrap();
    drop(file);

    let mut loader = SchemaLoader::new();
    loader.load_from_file("json_schema", &path).unwrap();
    let schema = loader.get("json_schema").unwrap();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["x"].is_object());
}

// ---------------------------------------------------------------------------
// load_from_file — YAML
// ---------------------------------------------------------------------------

#[test]
fn test_schema_loader_load_from_yaml_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.yaml");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        "type: object\nproperties:\n  name:\n    type: string\n"
    )
    .unwrap();
    drop(file);

    let mut loader = SchemaLoader::new();
    loader.load_from_file("yaml_schema", &path).unwrap();
    let schema = loader.get("yaml_schema").unwrap();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["name"]["type"], "string");
}

#[test]
fn test_schema_loader_load_from_yml_extension() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.yml");
    let mut file = std::fs::File::create(&path).unwrap();
    writeln!(file, "type: string").unwrap();
    drop(file);

    let mut loader = SchemaLoader::new();
    loader.load_from_file("yml_schema", &path).unwrap();
    assert_eq!(
        loader.get("yml_schema").unwrap(),
        &json!({ "type": "string" })
    );
}

// ---------------------------------------------------------------------------
// load_from_file — error conditions
// ---------------------------------------------------------------------------

#[test]
fn test_schema_loader_load_from_file_not_found() {
    let mut loader = SchemaLoader::new();
    let result = loader.load_from_file("missing", std::path::Path::new("/nonexistent/schema.json"));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaNotFound);
    assert!(err.message.contains("Failed to read"));
}

#[test]
fn test_schema_loader_load_from_file_invalid_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(file, "{{not valid json}}").unwrap();
    drop(file);

    let mut loader = SchemaLoader::new();
    let result = loader.load_from_file("bad", &path);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaParseError);
    assert!(err.message.contains("Failed to parse JSON"));
}

#[test]
fn test_schema_loader_load_from_file_invalid_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.yaml");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(file, ":\n  - :\n    bad: [unclosed").unwrap();
    drop(file);

    let mut loader = SchemaLoader::new();
    let result = loader.load_from_file("bad", &path);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaParseError);
}

// ---------------------------------------------------------------------------
// SchemaStrategy equality
// ---------------------------------------------------------------------------

#[test]
fn test_schema_strategy_equality() {
    assert_eq!(SchemaStrategy::YamlFirst, SchemaStrategy::YamlFirst);
    assert_ne!(SchemaStrategy::YamlFirst, SchemaStrategy::NativeFirst);
    assert_ne!(SchemaStrategy::NativeFirst, SchemaStrategy::YamlOnly);
}

// ---------------------------------------------------------------------------
// with_config (spec-compatible constructor)
// ---------------------------------------------------------------------------

#[test]
fn test_schema_loader_with_config_defaults_yaml_first() {
    let config = Config::default();
    let loader = SchemaLoader::with_config(&config, None);
    assert_eq!(loader.strategy, SchemaStrategy::YamlFirst);
    assert!(loader.list().is_empty());
}

#[test]
fn test_schema_loader_with_config_explicit_schemas_dir() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::default();
    let loader = SchemaLoader::with_config(&config, Some(dir.path()));
    assert_eq!(loader.strategy, SchemaStrategy::YamlFirst);
}

#[test]
fn test_schema_loader_with_config_uses_modules_path_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        modules_path: Some(dir.path().to_path_buf()),
        ..Default::default()
    };
    let loader = SchemaLoader::with_config(&config, None);
    assert_eq!(loader.strategy, SchemaStrategy::YamlFirst);
}

// A-C-003: `schema.strategy` used to be silently ignored — `with_config`
// hardcoded `YamlFirst` regardless of the config value. Parity with
// apcore-python (loader.py:1037) and apcore-typescript (loader.ts:138).
#[test]
fn test_schema_loader_with_config_reads_native_first_strategy() {
    let mut config = Config::default();
    config.set("schema.strategy", json!("native_first"));
    let loader = SchemaLoader::with_config(&config, None);
    assert_eq!(loader.strategy, SchemaStrategy::NativeFirst);
}

#[test]
fn test_schema_loader_with_config_reads_yaml_only_strategy() {
    let mut config = Config::default();
    config.set("schema.strategy", json!("yaml_only"));
    let loader = SchemaLoader::with_config(&config, None);
    assert_eq!(loader.strategy, SchemaStrategy::YamlOnly);
}

#[test]
fn test_schema_loader_with_config_unparseable_strategy_falls_back_to_yaml_first() {
    let mut config = Config::default();
    config.set("schema.strategy", json!("not_a_real_strategy"));
    let loader = SchemaLoader::with_config(&config, None);
    assert_eq!(loader.strategy, SchemaStrategy::YamlFirst);
}

// ---------------------------------------------------------------------------
// load() — spec-compatible method
// ---------------------------------------------------------------------------

#[test]
fn test_schema_loader_load_from_json_file_via_load() {
    let dir = tempfile::tempdir().unwrap();
    // Sync SCHEMA-002: filename is `<module_id>.schema.json`.
    let path = dir.path().join("my_module.schema.json");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        r#"{{"module_id":"my_module","description":"A module","input_schema":{{"type":"object"}},"output_schema":{{"type":"object"}}}}"#
    )
    .unwrap();
    drop(file);

    let config = Config::default();
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let def = loader.load("my_module").unwrap();
    assert_eq!(def.module_id, "my_module");
    assert_eq!(def.description, "A module");
    assert_eq!(def.input_schema["type"], "object");
}

#[test]
fn test_schema_loader_load_from_value_via_load() {
    let mut loader = SchemaLoader::new();
    let schema = json!({
        "module_id": "my_mod",
        "description": "desc",
        "input_schema": { "type": "string" },
        "output_schema": { "type": "string" }
    });
    loader.load_from_value("my_mod", schema).unwrap();
    let def = loader.load("my_mod").unwrap();
    assert_eq!(def.module_id, "my_mod");
    assert_eq!(def.input_schema["type"], "string");
}

#[test]
fn test_schema_loader_load_missing_module_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::default();
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let result = loader.load("nonexistent_module");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaNotFound);
}

#[test]
fn test_schema_loader_load_yaml_file_via_load() {
    let dir = tempfile::tempdir().unwrap();
    // Sync SCHEMA-002: filename is `<module_id>.schema.yaml`.
    let path = dir.path().join("email_module.schema.yaml");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        "module_id: email_module\ndescription: Send email\ninput_schema:\n  type: object\noutput_schema:\n  type: object\n"
    )
    .unwrap();
    drop(file);

    let config = Config::default();
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let def = loader.load("email_module").unwrap();
    assert_eq!(def.module_id, "email_module");
    assert_eq!(def.description, "Send email");
}

/// Sync SCHEMA-002: a dotted module_id like `executor.email.send_email` must
/// resolve to `<schemas_dir>/executor/email/send_email.schema.yaml`. Mirrors
/// apcore-python loader.py:62 and apcore-typescript loader.ts:53.
#[test]
fn test_schema_loader_dotted_module_id_resolves_to_subdirectories() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("executor").join("email");
    std::fs::create_dir_all(&nested).unwrap();
    let path = nested.join("send_email.schema.yaml");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        "module_id: executor.email.send_email\ndescription: Send email via SMTP\ninput_schema:\n  type: object\noutput_schema:\n  type: object\n"
    )
    .unwrap();
    drop(file);

    let config = Config::default();
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let def = loader
        .load("executor.email.send_email")
        .expect("dotted module_id must resolve to nested .schema.yaml file");
    assert_eq!(def.module_id, "executor.email.send_email");
    assert_eq!(def.description, "Send email via SMTP");
}

/// Sync SCHEMA-002: `.schema.yml` is a valid fallback after `.schema.yaml`.
#[test]
fn test_schema_loader_falls_back_to_schema_yml_extension() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy_mod.schema.yml");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        "module_id: legacy_mod\ndescription: legacy\ninput_schema:\n  type: object\noutput_schema:\n  type: object\n"
    )
    .unwrap();
    drop(file);

    let config = Config::default();
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let def = loader.load("legacy_mod").unwrap();
    assert_eq!(def.module_id, "legacy_mod");
}

// ---------------------------------------------------------------------------
// get_schema() — strategy-driven native/YAML resolution (A-C-003)
// ---------------------------------------------------------------------------

#[test]
fn test_get_schema_yaml_first_prefers_file_over_native() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.schema.yaml");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        "module_id: m\ndescription: from yaml\ninput_schema:\n  type: object\noutput_schema:\n  type: object\n"
    )
    .unwrap();
    drop(file);

    let mut config = Config::default();
    config.set("schema.strategy", json!("yaml_first"));
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let native_input = json!({"type": "string"});
    let native_output = json!({"type": "string"});
    let def = loader
        .get_schema("m", Some(&native_input), Some(&native_output))
        .unwrap();
    assert_eq!(def.description, "from yaml");
    assert_eq!(def.input_schema, json!({"type": "object"}));
}

#[test]
fn test_get_schema_yaml_first_falls_back_to_native_when_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.set("schema.strategy", json!("yaml_first"));
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let native_input = json!({"type": "string"});
    let native_output = json!({"type": "integer"});
    let def = loader
        .get_schema("no_file_module", Some(&native_input), Some(&native_output))
        .expect("no YAML file, but native schemas are available");
    assert_eq!(def.input_schema, native_input);
    assert_eq!(def.output_schema, native_output);
}

#[test]
fn test_get_schema_yaml_first_errors_when_no_file_and_no_native() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.set("schema.strategy", json!("yaml_first"));
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let err = loader
        .get_schema("no_file_no_native", None, None)
        .expect_err("no YAML file and no native fallback must error");
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaNotFound);
}

#[test]
fn test_get_schema_native_first_prefers_native_over_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.schema.yaml");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        "module_id: m\ndescription: from yaml\ninput_schema:\n  type: object\noutput_schema:\n  type: object\n"
    )
    .unwrap();
    drop(file);

    let mut config = Config::default();
    config.set("schema.strategy", json!("native_first"));
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let native_input = json!({"type": "string"});
    let native_output = json!({"type": "string"});
    let def = loader
        .get_schema("m", Some(&native_input), Some(&native_output))
        .unwrap();
    assert_eq!(def.input_schema, native_input);
    assert_eq!(def.output_schema, native_output);
}

#[test]
fn test_get_schema_native_first_falls_back_to_file_when_no_native() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.schema.yaml");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        "module_id: m\ndescription: from yaml\ninput_schema:\n  type: object\noutput_schema:\n  type: object\n"
    )
    .unwrap();
    drop(file);

    let mut config = Config::default();
    config.set("schema.strategy", json!("native_first"));
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let def = loader.get_schema("m", None, None).unwrap();
    assert_eq!(def.description, "from yaml");
}

#[test]
fn test_get_schema_yaml_only_ignores_native_even_when_provided() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.set("schema.strategy", json!("yaml_only"));
    let mut loader = SchemaLoader::with_config(&config, Some(dir.path()));
    let native_input = json!({"type": "string"});
    let native_output = json!({"type": "string"});
    let err = loader
        .get_schema("no_file_module", Some(&native_input), Some(&native_output))
        .expect_err("yaml_only must never fall back to native, even when provided");
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaNotFound);
}
