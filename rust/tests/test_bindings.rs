//! Integration tests for BindingLoader with Registry and FunctionModule
//! using the canonical YAML format defined in protocol-spec §5.12.

use apcore::bindings::{BindingHandler, BindingLoader};
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::registry::registry::Registry;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_echo_handler() -> BindingHandler {
    Arc::new(|inputs: Value, _ctx: &Context<Value>| Box::pin(async move { Ok(inputs) }))
}

fn make_error_handler() -> BindingHandler {
    Arc::new(|_inputs: Value, _ctx: &Context<Value>| {
        Box::pin(async move {
            Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "handler intentionally failed".to_string(),
            ))
        })
    })
}

// ---------------------------------------------------------------------------
// register_into_with_handlers (canonical YAML format)
// ---------------------------------------------------------------------------

#[test]
fn register_single_binding_into_registry() {
    let registry = Registry::new();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("e.binding.yaml");
    std::fs::write(
        &file_path,
        r#"
spec_version: "1.0"
bindings:
  - module_id: test.echo
    target: "test.echo:handler"
    input_schema: {type: object}
    output_schema: {type: object}
"#,
    )
    .unwrap();

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&file_path).unwrap();

    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("test.echo:handler".to_string(), make_echo_handler());

    let count = loader
        .register_into_with_handlers(&registry, handlers)
        .unwrap();

    assert_eq!(count, 1);
    assert!(registry.get("test.echo").unwrap().is_some());
}

#[test]
fn register_multiple_bindings_into_registry() {
    let registry = Registry::new();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("multi.binding.yaml");
    std::fs::write(
        &file_path,
        r#"
spec_version: "1.0"
bindings:
  - module_id: test.b1
    target: "test.b1:handler"
    input_schema: {type: object}
    output_schema: {type: object}
  - module_id: test.b2
    target: "test.b2:handler"
    input_schema: {type: object}
    output_schema: {type: object}
  - module_id: test.b3
    target: "test.b3:handler"
    input_schema: {type: object}
    output_schema: {type: object}
"#,
    )
    .unwrap();

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&file_path).unwrap();

    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    for tgt in ["test.b1:handler", "test.b2:handler", "test.b3:handler"] {
        handlers.insert(tgt.to_string(), make_echo_handler());
    }

    let count = loader
        .register_into_with_handlers(&registry, handlers)
        .unwrap();
    assert_eq!(count, 3);
    assert!(registry.get("test.b1").unwrap().is_some());
    assert!(registry.get("test.b2").unwrap().is_some());
    assert!(registry.get("test.b3").unwrap().is_some());
}

#[test]
fn register_binding_with_top_level_description() {
    let registry = Registry::new();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("d.binding.yaml");
    std::fs::write(
        &file_path,
        r#"
spec_version: "1.0"
bindings:
  - module_id: test.described
    target: "test.described:handler"
    input_schema: {type: object}
    output_schema: {type: object}
    description: "A well-described module"
    documentation: "Long-form details about this module."
"#,
    )
    .unwrap();

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&file_path).unwrap();

    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("test.described:handler".to_string(), make_echo_handler());

    let count = loader
        .register_into_with_handlers(&registry, handlers)
        .unwrap();
    assert_eq!(count, 1);
    assert!(registry.get("test.described").unwrap().is_some());
}

#[test]
fn register_fails_when_handler_missing_for_target() {
    let registry = Registry::new();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("nh.binding.yaml");
    std::fs::write(
        &file_path,
        r#"
spec_version: "1.0"
bindings:
  - module_id: test.x
    target: "test.x:needs_handler"
"#,
    )
    .unwrap();

    let mut loader = BindingLoader::new();
    loader.load_from_yaml(&file_path).unwrap();

    let result = loader.register_into_with_handlers(&registry, HashMap::new());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, ErrorCode::BindingModuleNotFound);
    assert!(err.message.contains("needs_handler"));
}

#[test]
fn register_empty_loader_succeeds_with_zero_count() {
    let registry = Registry::new();
    let loader = BindingLoader::new();
    let count = loader
        .register_into_with_handlers(&registry, HashMap::new())
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn json_load_canonical_format() {
    let registry = Registry::new();
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("bindings.json");
    let body = json!({
        "spec_version": "1.0",
        "bindings": [
            {"module_id": "test.j", "target": "test.j:handler",
             "input_schema":{"type":"object"}, "output_schema":{"type":"object"}}
        ]
    });
    std::fs::write(&file_path, serde_json::to_string(&body).unwrap()).unwrap();

    let mut loader = BindingLoader::new();
    loader.load_from_file(&file_path).unwrap();

    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("test.j:handler".to_string(), make_echo_handler());

    let count = loader
        .register_into_with_handlers(&registry, handlers)
        .unwrap();
    assert_eq!(count, 1);
    assert!(registry.get("test.j").unwrap().is_some());
}

// ---------------------------------------------------------------------------
// End-to-end: load YAML dir, register, execute
// ---------------------------------------------------------------------------

#[tokio::test]
async fn end_to_end_yaml_load_and_registry_execution() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: test.yaml_echo
    target: "test.yaml_echo:echo_fn"
    description: Echo inputs back as output
    input_schema: {type: object}
    output_schema: {type: object}
"#;
    std::fs::write(dir.path().join("echo.binding.yaml"), yaml).unwrap();

    let mut loader = BindingLoader::new();
    let n_loaded = loader.load_binding_dir(dir.path(), None).unwrap();
    assert_eq!(n_loaded, 1);

    let registry = Registry::new();
    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("test.yaml_echo:echo_fn".to_string(), make_echo_handler());

    let registered = loader
        .register_into_with_handlers(&registry, handlers)
        .unwrap();
    assert_eq!(registered, 1);

    let module = registry
        .get("test.yaml_echo")
        .unwrap()
        .expect("module should be registered");
    let identity = apcore::context::Identity::new(
        "tester".to_string(),
        "service".to_string(),
        vec![],
        HashMap::new(),
    );
    let ctx: Context<Value> = Context::new(identity);
    let inputs = json!({"key": "value"});
    let output = module.execute(inputs.clone(), &ctx).await.unwrap();
    assert_eq!(output, inputs);
}

#[tokio::test]
async fn registered_handler_propagates_error() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = r#"
spec_version: "1.0"
bindings:
  - module_id: test.failing
    target: "test.failing:fail_fn"
    input_schema: {type: object}
    output_schema: {type: object}
"#;
    std::fs::write(dir.path().join("fail.binding.yaml"), yaml).unwrap();

    let mut loader = BindingLoader::new();
    loader.load_binding_dir(dir.path(), None).unwrap();

    let registry = Registry::new();
    let mut handlers: HashMap<String, BindingHandler> = HashMap::new();
    handlers.insert("test.failing:fail_fn".to_string(), make_error_handler());

    loader
        .register_into_with_handlers(&registry, handlers)
        .unwrap();

    let module = registry
        .get("test.failing")
        .unwrap()
        .expect("module should be registered");
    let identity = apcore::context::Identity::new(
        "tester".to_string(),
        "service".to_string(),
        vec![],
        HashMap::new(),
    );
    let ctx: Context<Value> = Context::new(identity);
    let result = module.execute(json!({}), &ctx).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .message
        .contains("handler intentionally failed"));
}
