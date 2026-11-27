use apcore::{export_generic, to_strict_schema, SchemaDefinition};
use serde_json::json;

#[test]
fn generic_export_preserves_module_id_and_json_schema_extensions() {
    let schema = SchemaDefinition {
        module_id: "demo.weather".into(),
        description: "Get the weather".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"x-location": {"type": "string", "x-sensitive": true}},
            "x-extension": {"source": "module"},
        }),
        output_schema: json!({"type": "object", "properties": {"temperature": {"type": "number"}}}),
        error_schema: None,
        definitions: Some(json!({"coordinates": {"type": "array"}})),
        version: None,
    };

    assert_eq!(
        export_generic(&schema),
        json!({
            "module_id": schema.module_id,
            "description": schema.description,
            "input_schema": schema.input_schema,
            "output_schema": schema.output_schema,
            "definitions": schema.definitions,
        })
    );
}

#[test]
fn strict_schema_hardens_optional_nested_objects() {
    let hardened = to_strict_schema(&json!({
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "details": {
                "type": "object",
                "properties": {"title": {"type": "string"}, "rank": {"type": "integer"}},
                "required": ["title"]
            }
        },
        "required": ["id"]
    }));

    assert_eq!(hardened["additionalProperties"], false);
    assert_eq!(hardened["properties"]["id"]["type"], "string");
    let details = &hardened["properties"]["details"];
    assert_eq!(details["type"], json!(["object", "null"]));
    assert_eq!(details["additionalProperties"], false);
    assert_eq!(details["required"], json!(["rank", "title"]));
    assert_eq!(
        details["properties"]["rank"]["type"],
        json!(["integer", "null"])
    );
}

#[test]
fn strict_schema_preserves_property_and_definition_names() {
    let hardened = to_strict_schema(&json!({
        "type": ["object", "null"],
        "x-owner": "private",
        "properties": {
            "x-trace": {"type": "string", "x-sensitive": true},
            "default": {"type": "string", "default": "sample"},
            "value": {"$ref": "#/$defs/x-item"}
        },
        "required": ["x-trace", "default", "value"],
        "$defs": {"x-item": {"type": "string", "x-owner": "private"}}
    }));

    assert_eq!(hardened["additionalProperties"], false);
    assert_eq!(hardened["properties"]["x-trace"], json!({"type": "string"}));
    assert_eq!(hardened["properties"]["default"], json!({"type": "string"}));
    assert_eq!(hardened["$defs"]["x-item"], json!({"type": "string"}));
    assert!(hardened.get("x-owner").is_none());
}
