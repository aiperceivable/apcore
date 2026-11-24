//! Tests for SchemaExporter — exporting schemas to MCP, OpenAI, Anthropic, and Generic formats.
#![allow(clippy::similar_names)] // `exporter`/`exported` and `schema`/`schemas` are intentionally distinct

use apcore::schema::{
    to_strict_schema, ExportOptions, ExportProfile, SchemaExporter, SchemaLoader,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sample_schema() -> serde_json::Value {
    json!({
        "name": "get_weather",
        "description": "Get the current weather for a location",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": { "type": "string" }
            },
            "required": ["location"]
        }
    })
}

// ---------------------------------------------------------------------------
// MCP export
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_mcp_format() {
    let exporter = SchemaExporter::new();
    let parsed = exporter
        .export(&sample_schema(), ExportProfile::Mcp, None)
        .unwrap();
    assert_eq!(parsed["name"], "get_weather");
    assert!(parsed["inputSchema"].is_object());
    assert_eq!(parsed["inputSchema"]["required"][0], "location");
    // Sync SCHEMA-004: MCP envelope MUST carry description, annotations, and
    // _meta blocks aligned with apcore-python and apcore-typescript.
    assert_eq!(
        parsed["description"],
        "Get the current weather for a location"
    );
    assert!(parsed["annotations"].is_object());
    assert!(parsed["_meta"].is_object());
    assert_eq!(parsed["_meta"]["paginationStyle"], "cursor");
}

#[test]
fn test_schema_exporter_mcp_missing_name() {
    let exporter = SchemaExporter::new();
    let schema = json!({ "input_schema": { "type": "object" } });
    let parsed = exporter.export(&schema, ExportProfile::Mcp, None).unwrap();
    assert!(parsed["name"].is_null());
}

#[test]
fn test_schema_exporter_mcp_fallback_input_schema_key() {
    let exporter = SchemaExporter::new();
    let schema = json!({
        "name": "tool",
        "inputSchema": { "type": "string" }
    });
    let parsed = exporter.export(&schema, ExportProfile::Mcp, None).unwrap();
    assert_eq!(parsed["inputSchema"]["type"], "string");
}

#[test]
fn test_schema_exporter_mcp_no_input_schema_defaults_empty_object() {
    let exporter = SchemaExporter::new();
    let schema = json!({ "name": "tool" });
    let parsed = exporter.export(&schema, ExportProfile::Mcp, None).unwrap();
    assert_eq!(parsed["inputSchema"], json!({}));
}

// ---------------------------------------------------------------------------
// OpenAI export
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_openai_format() {
    let exporter = SchemaExporter::new();
    let parsed = exporter
        .export(&sample_schema(), ExportProfile::OpenAi, None)
        .unwrap();
    assert_eq!(parsed["type"], "function");
    assert_eq!(parsed["function"]["name"], "get_weather");
    assert_eq!(
        parsed["function"]["description"],
        "Get the current weather for a location"
    );
    assert!(parsed["function"]["parameters"].is_object());
    assert_eq!(parsed["function"]["strict"], true);
}

#[test]
fn test_schema_exporter_openai_missing_description_defaults_empty() {
    let exporter = SchemaExporter::new();
    let schema = json!({ "name": "tool", "input_schema": {} });
    let parsed = exporter
        .export(&schema, ExportProfile::OpenAi, None)
        .unwrap();
    assert_eq!(parsed["function"]["description"], "");
}

#[test]
fn test_schema_exporter_openai_uses_parameters_key_fallback() {
    let exporter = SchemaExporter::new();
    let schema = json!({
        "name": "tool",
        "parameters": { "type": "object", "properties": { "x": { "type": "integer" } } }
    });
    let parsed = exporter
        .export(&schema, ExportProfile::OpenAi, None)
        .unwrap();
    // Sync SCHEMA-003: the fallback `parameters` key still feeds A23 strict
    // transform; previously-optional properties become nullable arrays.
    assert_eq!(
        parsed["function"]["parameters"]["properties"]["x"]["type"],
        json!(["integer", "null"])
    );
    assert_eq!(
        parsed["function"]["parameters"]["additionalProperties"],
        json!(false)
    );
}

// ---------------------------------------------------------------------------
// Anthropic export
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_anthropic_format() {
    let exporter = SchemaExporter::new();
    let parsed = exporter
        .export(&sample_schema(), ExportProfile::Anthropic, None)
        .unwrap();
    assert_eq!(parsed["name"], "get_weather");
    assert_eq!(
        parsed["description"],
        "Get the current weather for a location"
    );
    assert!(parsed["input_schema"].is_object());
    assert_eq!(parsed["input_schema"]["required"][0], "location");
    // Anthropic format should NOT have "type": "function" wrapper
    assert!(parsed.get("type").is_none());
}

#[test]
fn test_schema_exporter_anthropic_missing_fields_use_defaults() {
    let exporter = SchemaExporter::new();
    let schema = json!({});
    let parsed = exporter
        .export(&schema, ExportProfile::Anthropic, None)
        .unwrap();
    assert!(parsed["name"].is_null());
    assert_eq!(parsed["description"], "");
    assert_eq!(parsed["input_schema"], json!({}));
}

// ---------------------------------------------------------------------------
// Generic export
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_generic_returns_schema_as_is() {
    let exporter = SchemaExporter::new();
    let schema = sample_schema();
    let parsed = exporter
        .export(&schema, ExportProfile::Generic, None)
        .unwrap();
    assert_eq!(parsed, schema);
}

#[test]
fn test_schema_exporter_generic_preserves_arbitrary_fields() {
    let exporter = SchemaExporter::new();
    let schema = json!({ "custom_field": 42, "nested": { "a": true } });
    let parsed = exporter
        .export(&schema, ExportProfile::Generic, None)
        .unwrap();
    assert_eq!(parsed["custom_field"], 42);
    assert_eq!(parsed["nested"]["a"], true);
}

// ---------------------------------------------------------------------------
// export_all — now returns HashMap<String, Value>
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_export_all_empty_loader() {
    let exporter = SchemaExporter::new();
    let loader = SchemaLoader::new();
    let result = exporter
        .export_all(&loader, ExportProfile::Generic)
        .unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_schema_exporter_export_all_multiple_schemas() {
    let exporter = SchemaExporter::new();
    let mut loader = SchemaLoader::new();
    loader
        .load_from_value(
            "tool_a",
            json!({ "name": "tool_a", "input_schema": { "type": "object" } }),
        )
        .unwrap();
    loader
        .load_from_value(
            "tool_b",
            json!({ "name": "tool_b", "input_schema": { "type": "string" } }),
        )
        .unwrap();

    let result = exporter
        .export_all(&loader, ExportProfile::Anthropic)
        .unwrap();
    assert_eq!(result.len(), 2);
    assert!(result.contains_key("tool_a"));
    assert!(result.contains_key("tool_b"));

    // Verify each exported schema is a JSON object with a name field
    for exported_value in result.values() {
        assert!(exported_value.get("name").is_some());
    }
}

// ---------------------------------------------------------------------------
// ExportProfile equality
// ---------------------------------------------------------------------------

#[test]
fn test_export_profile_equality() {
    assert_eq!(ExportProfile::Mcp, ExportProfile::Mcp);
    assert_ne!(ExportProfile::Mcp, ExportProfile::OpenAi);
    assert_ne!(ExportProfile::OpenAi, ExportProfile::Anthropic);
    assert_ne!(ExportProfile::Anthropic, ExportProfile::Generic);
}

// ---------------------------------------------------------------------------
// Default impl
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_default() {
    let exporter = SchemaExporter;
    let parsed = exporter
        .export(&json!({"name": "x"}), ExportProfile::Generic, None)
        .unwrap();
    assert_eq!(parsed["name"], "x");
}

// ---------------------------------------------------------------------------
// export_serialized — pretty-printed JSON string (legacy behaviour)
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_output_is_pretty_printed() {
    let exporter = SchemaExporter::new();
    let exported = exporter
        .export_serialized(&json!({"name": "test"}), ExportProfile::Generic, None)
        .unwrap();
    // Pretty-printed JSON contains newlines
    assert!(exported.contains('\n'));
}

// ---------------------------------------------------------------------------
// ExportOptions — optional parameters (spec-compatible)
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_export_with_name_override() {
    let exporter = SchemaExporter::new();
    let schema = json!({ "name": "original", "input_schema": {} });
    let opts = ExportOptions {
        name: Some("overridden".to_string()),
        ..Default::default()
    };
    let parsed = exporter
        .export(&schema, ExportProfile::Generic, Some(&opts))
        .unwrap();
    assert_eq!(parsed["name"], "overridden");
}

#[test]
fn test_schema_exporter_export_with_examples() {
    let exporter = SchemaExporter::new();
    let schema = json!({ "name": "tool", "input_schema": {} });
    let opts = ExportOptions {
        examples: Some(json!([{"location": "London"}])),
        ..Default::default()
    };
    let parsed = exporter
        .export(&schema, ExportProfile::Generic, Some(&opts))
        .unwrap();
    assert_eq!(parsed["examples"][0]["location"], "London");
}

#[test]
fn test_schema_exporter_export_with_annotations() {
    let exporter = SchemaExporter::new();
    let schema = json!({ "name": "tool", "input_schema": {} });
    let opts = ExportOptions {
        annotations: Some(json!({ "x-custom": "value" })),
        ..Default::default()
    };
    let parsed = exporter
        .export(&schema, ExportProfile::Generic, Some(&opts))
        .unwrap();
    assert_eq!(parsed["x-custom"], "value");
}

// ---------------------------------------------------------------------------
// SCHEMA-003 — A23 strict transform on OpenAI envelope
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_openai_strict_transform_marks_additional_properties_false() {
    let exporter = SchemaExporter::new();
    let schema = json!({
        "name": "weather",
        "description": "Get weather",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": { "type": "string" }
            },
            "required": ["location"]
        }
    });
    let parsed = exporter
        .export(&schema, ExportProfile::OpenAi, None)
        .unwrap();
    // After A23: additionalProperties:false on the parameters object
    assert_eq!(
        parsed["function"]["parameters"]["additionalProperties"],
        json!(false)
    );
    // All properties become required
    let required = parsed["function"]["parameters"]["required"]
        .as_array()
        .expect("required must be an array");
    assert!(required.iter().any(|v| v == "location"));
}

#[test]
fn test_schema_exporter_openai_strict_transform_makes_optional_nullable() {
    let exporter = SchemaExporter::new();
    let schema = json!({
        "name": "tool",
        "input_schema": {
            "type": "object",
            "properties": {
                "name":    { "type": "string" },
                "comment": { "type": "string" }
            },
            "required": ["name"]
        }
    });
    let parsed = exporter
        .export(&schema, ExportProfile::OpenAi, None)
        .unwrap();
    let parameters = &parsed["function"]["parameters"];
    // "name" stayed required → unchanged type.
    assert_eq!(parameters["properties"]["name"]["type"], "string");
    // "comment" was optional → becomes nullable array.
    assert_eq!(
        parameters["properties"]["comment"]["type"],
        json!(["string", "null"])
    );
}

#[test]
fn test_schema_exporter_openai_strict_transform_hardens_optional_nested_object() {
    // An optional nested object is made nullable first (`type` becomes
    // ["object", "null"]), so a string-only `type == "object"` test skips it and
    // the node keeps neither `additionalProperties: false` nor a complete
    // `required` list — which OpenAI strict mode rejects.
    let exporter = SchemaExporter::new();
    let schema = json!({
        "name": "tool",
        "input_schema": {
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "sub": {
                    "type": "object",
                    "properties": {
                        "x": { "type": "string" },
                        "y": { "type": "integer" }
                    },
                    "required": ["x"]
                }
            },
            "required": ["id"]
        }
    });
    let parsed = exporter
        .export(&schema, ExportProfile::OpenAi, None)
        .unwrap();

    let sub = &parsed["function"]["parameters"]["properties"]["sub"];
    // Optional → nullable, and still hardened as an object node.
    assert_eq!(sub["type"], json!(["object", "null"]));
    assert_eq!(sub["additionalProperties"], json!(false));
    assert_eq!(sub["required"], json!(["x", "y"]));
    // "y" was optional inside the nested object → nullable.
    assert_eq!(sub["properties"]["y"]["type"], json!(["integer", "null"]));
}

#[test]
fn test_strict_schema_hardens_object_declared_as_type_array() {
    // Same rule reached directly: a `type` array containing "object" is an
    // object node, whoever wrote it that way.
    let hardened = to_strict_schema(&json!({
        "type": ["object", "null"],
        "properties": { "x": { "type": "string" } }
    }));

    assert_eq!(hardened["additionalProperties"], json!(false));
    assert_eq!(hardened["required"], json!(["x"]));
}

// ---------------------------------------------------------------------------
// SCHEMA-004 — MCP / Anthropic envelope completeness via SchemaDefinition
// ---------------------------------------------------------------------------

#[test]
fn test_schema_exporter_export_def_mcp_includes_full_envelope() {
    use apcore::module::ModuleAnnotations;
    use apcore::schema::SchemaDefinition;

    let exporter = SchemaExporter::new();
    let def = SchemaDefinition {
        module_id: "weather.get".to_string(),
        description: "Get the current weather".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": { "location": { "type": "string" } }
        }),
        output_schema: json!({}),
        error_schema: None,
        definitions: None,
        version: None,
    };
    let ann = ModuleAnnotations {
        cacheable: true,
        cache_ttl: 60,
        idempotent: true,
        paginated: true,
        pagination_style: "cursor".to_string(),
        ..ModuleAnnotations::default()
    };

    let envelope = exporter
        .export_def(&def, ExportProfile::Mcp, Some(&ann), None, None)
        .unwrap();
    assert_eq!(envelope["name"], "weather.get");
    assert_eq!(envelope["description"], "Get the current weather");
    assert!(envelope["inputSchema"].is_object());
    // annotations block
    assert_eq!(envelope["annotations"]["idempotentHint"], true);
    assert_eq!(envelope["annotations"]["readOnlyHint"], false);
    assert_eq!(envelope["annotations"]["openWorldHint"], true);
    // _meta block
    assert_eq!(envelope["_meta"]["cacheable"], true);
    assert_eq!(envelope["_meta"]["cacheTtl"], 60);
    assert_eq!(envelope["_meta"]["paginated"], true);
    assert_eq!(envelope["_meta"]["paginationStyle"], "cursor");
}

#[test]
fn test_schema_exporter_export_def_anthropic_includes_input_examples() {
    use apcore::module::ModuleExample;
    use apcore::schema::SchemaDefinition;

    let exporter = SchemaExporter::new();
    let def = SchemaDefinition {
        module_id: "tools.echo".to_string(),
        description: "Echo".to_string(),
        input_schema: json!({"type": "object"}),
        output_schema: json!({}),
        error_schema: None,
        definitions: None,
        version: None,
    };
    let mut ex = ModuleExample::default();
    ex.title = "hello".to_string();
    ex.inputs = json!({"text": "hi"});
    ex.output = json!({"text": "hi"});
    let examples = vec![ex];
    let envelope = exporter
        .export_def(&def, ExportProfile::Anthropic, None, Some(&examples), None)
        .unwrap();
    assert_eq!(envelope["name"], "tools_echo");
    assert!(envelope["input_examples"].is_array());
    assert_eq!(envelope["input_examples"][0]["text"], "hi");
}

#[test]
fn test_schema_exporter_export_def_openai_uses_strict_transform() {
    use apcore::schema::SchemaDefinition;

    let exporter = SchemaExporter::new();
    let def = SchemaDefinition {
        module_id: "ns.tool".to_string(),
        description: "desc".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": { "x": { "type": "integer" } },
            "required": ["x"]
        }),
        output_schema: json!({}),
        error_schema: None,
        definitions: None,
        version: None,
    };
    let envelope = exporter
        .export_def(&def, ExportProfile::OpenAi, None, None, None)
        .unwrap();
    assert_eq!(envelope["function"]["name"], "ns_tool");
    assert_eq!(
        envelope["function"]["parameters"]["additionalProperties"],
        json!(false)
    );
}
