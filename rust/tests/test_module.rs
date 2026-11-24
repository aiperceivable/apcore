//! Tests for Module trait, ModuleAnnotations, and ModuleExample.

use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::{Module, ModuleAnnotations, ModuleExample};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test modules
// ---------------------------------------------------------------------------

struct EchoModule;

#[async_trait]
impl Module for EchoModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "value": { "type": "string" } } })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "value": { "type": "string" } } })
    }
    fn description(&self) -> &'static str {
        "Echo the input value back"
    }
    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(input)
    }
}

struct FailingModule;

#[async_trait]
impl Module for FailingModule {
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn output_schema(&self) -> Value {
        json!({})
    }
    fn description(&self) -> &'static str {
        "Always returns an error"
    }
    async fn execute(&self, _input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Err(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "intentional failure",
        ))
    }
}

fn make_ctx() -> Context<Value> {
    Context::new(Identity::new(
        "test".to_string(),
        "Test".to_string(),
        vec![],
        HashMap::new(),
    ))
}

// ---------------------------------------------------------------------------
// Module trait
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_echo_module_returns_input() {
    let ctx = make_ctx();
    let module = EchoModule;
    let input = json!({"value": "hello"});
    let result = module.execute(input.clone(), &ctx).await.unwrap();
    assert_eq!(result, input);
}

#[tokio::test]
async fn test_failing_module_returns_error() {
    let ctx = make_ctx();
    let module = FailingModule;
    let err = module.execute(json!({}), &ctx).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::GeneralInternalError);
}

#[test]
fn test_module_preflight_returns_empty_warnings_by_default() {
    // D11-009: Module::preflight signature aligned with apcore-python /
    // apcore-typescript: takes (inputs, context) and returns a list of
    // advisory warning strings. Default implementation returns an empty
    // Vec (no warnings).
    let module = EchoModule;
    let warnings: Vec<String> = module.preflight(&json!({}), None);
    assert!(warnings.is_empty());
}

#[test]
fn test_module_default_tags_is_empty() {
    // D11-003: Module::tags default returns an empty Vec; modules opt
    // in by overriding (parity with Python `module.tags = [...]` and
    // TypeScript `module['tags'] = [...]`).
    let module = EchoModule;
    let tags: Vec<String> = module.tags();
    assert!(tags.is_empty());
}

#[test]
fn test_module_description() {
    let module = EchoModule;
    assert_eq!(module.description(), "Echo the input value back");
}

#[test]
fn test_module_schemas_are_valid_json() {
    let module = EchoModule;
    // Both schemas must be objects
    assert!(module.input_schema().is_object());
    assert!(module.output_schema().is_object());
}

// ---------------------------------------------------------------------------
// ModuleAnnotations
// ---------------------------------------------------------------------------

#[test]
fn test_annotations_default() {
    let ann = ModuleAnnotations {
        ..Default::default()
    };
    assert!(!ann.readonly);
    assert!(!ann.destructive);
    assert!(!ann.idempotent);
    assert!(!ann.requires_approval);
    assert!(ann.open_world);
    assert!(!ann.streaming);
    assert!(!ann.cacheable);
    assert_eq!(ann.cache_ttl, 0);
    assert!(ann.cache_key_fields.is_none());
    assert!(!ann.paginated);
    assert_eq!(ann.pagination_style, "cursor");
}

#[test]
fn test_annotations_with_tags_and_version() {
    let ann = ModuleAnnotations {
        readonly: true,
        idempotent: true,
        cacheable: true,
        cache_ttl: 300,
        ..Default::default()
    };
    assert!(ann.readonly);
    assert!(ann.idempotent);
    assert!(ann.cacheable);
    assert_eq!(ann.cache_ttl, 300);
}

#[test]
fn test_annotations_serialization_round_trip() {
    let ann = ModuleAnnotations {
        destructive: true,
        requires_approval: true,
        open_world: false,
        ..Default::default()
    };
    let json = serde_json::to_string(&ann).unwrap();
    let restored: ModuleAnnotations = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.destructive, ann.destructive);
    assert_eq!(restored.requires_approval, ann.requires_approval);
    assert_eq!(restored.open_world, ann.open_world);
}

// ---------------------------------------------------------------------------
// ModuleExample
// ---------------------------------------------------------------------------

#[test]
fn test_module_example_fields() {
    let mut ex = ModuleExample::default();
    ex.title = "Add two numbers".to_string();
    ex.description = Some("Returns the sum".to_string());
    ex.inputs = json!({"a": 1, "b": 2});
    ex.output = json!({"result": 3});
    assert_eq!(ex.title, "Add two numbers");
    assert_eq!(ex.inputs["a"], 1);
    assert_eq!(ex.output["result"], 3);
}

#[test]
fn test_module_example_serialization() {
    let mut ex = ModuleExample::default();
    ex.title = "Greet Alice".to_string();
    ex.inputs = json!({"name": "Alice"});
    ex.output = json!({"message": "Hello, Alice!"});
    let json = serde_json::to_string(&ex).unwrap();
    let restored: ModuleExample = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.title, ex.title);
    assert_eq!(restored.inputs, ex.inputs);
}

// ---------------------------------------------------------------------------
// ModuleAnnotations -- extra field (annotations-redesign)
// ---------------------------------------------------------------------------

#[test]
fn test_annotations_default_extra_is_empty() {
    let ann = ModuleAnnotations::default();
    assert!(ann.extra.is_empty());
}

#[test]
fn test_annotations_extra_with_values() {
    let mut extra = HashMap::new();
    extra.insert(
        "mcp.category".to_string(),
        serde_json::Value::String("tools".to_string()),
    );
    let ann = ModuleAnnotations {
        extra,
        ..Default::default()
    };
    assert_eq!(
        ann.extra.get("mcp.category").unwrap(),
        &serde_json::Value::String("tools".to_string())
    );
}

#[test]
fn test_annotations_legacy_flattened_form_still_accepted() {
    // PROTOCOL_SPEC §4.4.1 rule 6: legacy top-level overflow keys are
    // tolerated for backward compatibility and normalized into `extra`.
    let data = r#"{
        "readonly": true,
        "future_field": 42,
        "another_unknown": "hello"
    }"#;
    let ann: ModuleAnnotations = serde_json::from_str(data).unwrap();
    assert!(ann.readonly);
    assert_eq!(ann.extra.get("future_field").unwrap(), &json!(42));
    assert_eq!(ann.extra.get("another_unknown").unwrap(), &json!("hello"));
}

#[test]
fn test_annotations_canonical_nested_extra_round_trip() {
    // PROTOCOL_SPEC §4.4.1 producer rules: extra MUST be serialized as a
    // nested object under the `"extra"` key, never flattened.
    let mut extra = HashMap::new();
    extra.insert("mcp.category".to_string(), json!("tools"));
    extra.insert("cli.approval_message".to_string(), json!("Are you sure?"));
    let ann = ModuleAnnotations {
        readonly: true,
        extra,
        ..Default::default()
    };
    let json_value = serde_json::to_value(&ann).unwrap();
    let obj = json_value.as_object().unwrap();
    // Producer MUST emit a nested `extra` object.
    assert!(obj.contains_key("extra"));
    let extra_obj = obj.get("extra").unwrap().as_object().unwrap();
    assert_eq!(extra_obj.get("mcp.category").unwrap(), &json!("tools"));
    assert_eq!(
        extra_obj.get("cli.approval_message").unwrap(),
        &json!("Are you sure?")
    );
    // Producer MUST NOT flatten extension keys to the root.
    assert!(!obj.contains_key("mcp.category"));
    assert!(!obj.contains_key("cli.approval_message"));

    // Deserialize back and verify lossless round trip.
    let restored: ModuleAnnotations = serde_json::from_value(json_value).unwrap();
    assert!(restored.readonly);
    assert_eq!(restored.extra.get("mcp.category").unwrap(), &json!("tools"));
    assert_eq!(
        restored.extra.get("cli.approval_message").unwrap(),
        &json!("Are you sure?")
    );
    // Critical: must NOT have an `extra` key inside extra (the pre-0.17.2 bug).
    assert!(!restored.extra.contains_key("extra"));
}

#[test]
fn test_annotations_nested_extra_wins_over_top_level_collision() {
    // PROTOCOL_SPEC §4.4.1 rule 7: when the same key appears both nested and
    // at the root, the nested value MUST win.
    let data = r#"{
        "readonly": false,
        "mcp.category": "LEGACY_VALUE",
        "extra": {
            "mcp.category": "CANONICAL_VALUE",
            "cli.approval_message": "from nested only"
        }
    }"#;
    let ann: ModuleAnnotations = serde_json::from_str(data).unwrap();
    assert_eq!(
        ann.extra.get("mcp.category").unwrap(),
        &json!("CANONICAL_VALUE")
    );
    assert_eq!(
        ann.extra.get("cli.approval_message").unwrap(),
        &json!("from nested only")
    );
}

#[test]
fn test_annotations_python_typescript_payload_round_trips() {
    // Regression for the pre-0.17.2 bug: a payload produced by apcore-python
    // or apcore-typescript (nested `extra`) used to land in `extra["extra"]`.
    let data = r#"{
        "readonly": true,
        "extra": {
            "mcp.category": "tools"
        }
    }"#;
    let ann: ModuleAnnotations = serde_json::from_str(data).unwrap();
    assert_eq!(ann.extra.get("mcp.category").unwrap(), &json!("tools"));
    assert!(!ann.extra.contains_key("extra"));
}

#[test]
fn test_annotations_null_extra_treated_as_empty() {
    // PROTOCOL_SPEC §4.4.1 rule 8: when `extra` is absent or null, treat as empty.
    let data = r#"{ "readonly": true, "extra": null }"#;
    let ann: ModuleAnnotations = serde_json::from_str(data).unwrap();
    assert!(ann.readonly);
    assert!(ann.extra.is_empty());
}

#[test]
fn test_annotations_pagination_style_accepts_custom_string() {
    let ann = ModuleAnnotations {
        pagination_style: "custom".to_string(),
        ..Default::default()
    };
    assert_eq!(ann.pagination_style, "custom");
}

#[test]
fn test_annotations_extra_round_trip() {
    let mut extra = HashMap::new();
    extra.insert("cli.approval_message".to_string(), json!("Are you sure?"));
    let ann = ModuleAnnotations {
        destructive: true,
        extra,
        ..Default::default()
    };
    let serialized = serde_json::to_string(&ann).unwrap();
    let restored: ModuleAnnotations = serde_json::from_str(&serialized).unwrap();
    assert!(restored.destructive);
    assert_eq!(
        restored.extra.get("cli.approval_message").unwrap(),
        &json!("Are you sure?")
    );
}

#[test]
fn test_annotations_deserialization_missing_fields_use_defaults() {
    let ann: ModuleAnnotations = serde_json::from_str("{}").unwrap();
    assert!(!ann.readonly);
    assert!(ann.open_world);
    assert_eq!(ann.pagination_style, "cursor");
    assert!(ann.extra.is_empty());
}
