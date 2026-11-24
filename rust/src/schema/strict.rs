// APCore Protocol — Strict schema conversion (Algorithm A23)
// Spec reference: PROTOCOL_SPEC.md Algorithm A23 — to_strict_schema
//
// Converts a JSON Schema to strict mode by:
// 1. Stripping x-* extension keys and "default" keys
// 2. Setting additionalProperties: false on all object schemas
// 3. Making all properties required (optional ones become nullable)
// 4. Recursively processing nested schemas

use serde_json::{Map, Value};

/// Convert a JSON Schema to strict mode (Algorithm A23).
///
/// Deep-clones the input, strips extension keys (`x-*`) and `default` keys,
/// then enforces strict mode rules: `additionalProperties: false`, all
/// properties required, and previously-optional fields become nullable.
#[must_use]
pub fn to_strict_schema(schema: &Value) -> Value {
    let mut result = schema.clone();
    strip_extensions(&mut result, false, true);
    convert_to_strict(&mut result);
    result
}

/// Replace `description` with `x-llm-description` (when present) at every
/// node in the schema tree. Returns a deep clone with the substitution
/// applied; the original schema is unmodified.
///
/// Sync finding A-D-030: spec §schema-system says adapters SHOULD prefer
/// `x-llm-description` over `description` when exporting. apcore-python's
/// `_apply_llm_descriptions` and apcore-typescript's `applyLlmDescriptions`
/// both run this substitution before `to_strict_schema` / `strip_extensions`
/// in the OpenAI/Anthropic export paths so the LLM-tuned description
/// survives the strip. Rust previously had no equivalent step — the x-*
/// keys were silently dropped without copy-down.
#[must_use]
pub fn apply_llm_descriptions(schema: &Value) -> Value {
    let mut result = schema.clone();
    apply_llm_descriptions_in_place(&mut result, false);
    result
}

fn schema_name_map(key: &str) -> bool {
    matches!(
        key,
        "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas"
    )
}

fn apply_llm_descriptions_in_place(node: &mut Value, names: bool) {
    if let Some(obj) = node.as_object_mut() {
        // Both keys present: substitute description with x-llm-description.
        // (Apcore-python parity — Python requires both to be present;
        // apcore-typescript injects description even if absent. We follow
        // Python's stricter rule to avoid silently fabricating descriptions
        // that the spec did not declare.)
        if !names && obj.contains_key("description") && obj.contains_key("x-llm-description") {
            if let Some(llm_desc) = obj.get("x-llm-description").cloned() {
                obj.insert("description".to_string(), llm_desc);
            }
        }
        // Recurse into all nested values.
        for (key, v) in obj.iter_mut() {
            apply_llm_descriptions_in_place(v, !names && schema_name_map(key));
        }
    } else if let Some(arr) = node.as_array_mut() {
        for v in arr.iter_mut() {
            apply_llm_descriptions_in_place(v, false);
        }
    }
}

/// Strip extension keywords without deleting property or definition names.
pub(crate) fn strip_extension_keywords(schema: &Value) -> Value {
    let mut result = schema.clone();
    strip_extensions(&mut result, false, false);
    result
}

/// Named schema maps contain identifiers, not schema keywords.
fn strip_extensions(node: &mut Value, names: bool, remove_defaults: bool) {
    let Some(obj) = node.as_object_mut() else {
        return;
    };

    let keys_to_remove: Vec<String> = obj
        .keys()
        .filter(|k| !names && (k.starts_with("x-") || (remove_defaults && *k == "default")))
        .cloned()
        .collect();

    for k in keys_to_remove {
        obj.remove(&k);
    }

    // Recurse into all nested values.
    let values: Vec<String> = obj.keys().cloned().collect();
    for key in values {
        if let Some(val) = obj.get_mut(&key) {
            match val {
                Value::Object(_) => {
                    strip_extensions(val, !names && schema_name_map(&key), remove_defaults);
                }
                Value::Array(arr) => {
                    for item in arr.iter_mut() {
                        if item.is_object() {
                            strip_extensions(item, false, remove_defaults);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Enforce strict mode rules on a schema node. Mutates in place.
fn convert_to_strict(node: &mut Value) {
    let Some(obj) = node.as_object_mut() else {
        return;
    };

    // If this is an object schema with properties, enforce strict rules.
    // `type` may be a string ("object"), an array (["object", "null"] — what
    // `make_nullable` produces for an optional nested object), or absent
    // entirely: `properties` alone already implies an object schema, and
    // requiring a `type` keyword let `{"properties": {…}}` through unhardened.
    let is_object_with_props = obj.contains_key("properties")
        && (obj.get("type").is_none() || declares_object_type(obj.get("type")));

    if is_object_with_props {
        obj.insert("additionalProperties".to_string(), Value::Bool(false));

        // Collect existing required set.
        let existing_required: Vec<String> = obj
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Get all property names.
        let all_names: Vec<String> = obj
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|props| props.keys().cloned().collect())
            .unwrap_or_default();

        // Make optional properties nullable.
        if let Some(properties) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) {
            for name in &all_names {
                if existing_required.contains(name) {
                    continue;
                }
                if let Some(prop) = properties.get_mut(name) {
                    make_nullable(prop);
                }
            }
        }

        // Set required to all property names (sorted).
        let mut sorted_names = all_names;
        sorted_names.sort();
        let required_arr: Vec<Value> = sorted_names.into_iter().map(Value::String).collect();
        obj.insert("required".to_string(), Value::Array(required_arr));
    }

    // Recurse into nested structures.
    recurse_into_nested(obj);
}

/// `true` when a `type` keyword declares "object", in either the string form
/// (`"object"`) or the union form (`["object", "null"]`).
fn declares_object_type(type_value: Option<&Value>) -> bool {
    match type_value {
        Some(Value::String(s)) => s == "object",
        Some(Value::Array(items)) => items.iter().any(|v| v.as_str() == Some("object")),
        _ => false,
    }
}

/// Make a property nullable without weakening its non-null constraints.
fn make_nullable(prop: &mut Value) {
    let Some(prop_obj) = prop.as_object_mut() else {
        return;
    };
    let constrained = [
        "enum", "const", "$ref", "allOf", "anyOf", "oneOf", "not", "if",
    ]
    .iter()
    .any(|key| prop_obj.contains_key(*key));
    if !prop_obj.contains_key("type") || constrained {
        // Constraints can reject null independently of the type keyword.
        // Preserve the complete original schema in the non-null branch.
        let original = Value::Object(std::mem::take(prop_obj));
        *prop = serde_json::json!({"anyOf": [original, {"type": "null"}]});
    } else if let Some(type_val) = prop_obj.get_mut("type") {
        match type_val {
            Value::String(s) if s != "null" => {
                *type_val = serde_json::json!([s, "null"]);
            }
            Value::Array(arr) if !arr.iter().any(|v| v.as_str() == Some("null")) => {
                arr.push(Value::String("null".to_string()));
            }
            _ => {}
        }
    }
}

/// Recurse into properties, items, prefixItems, allOf/anyOf/oneOf, and
/// definitions/$defs.
fn recurse_into_nested(obj: &mut Map<String, Value>) {
    // properties
    if let Some(properties) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) {
        for prop in properties.values_mut() {
            convert_to_strict(prop);
        }
    }

    // items
    if let Some(items) = obj.get_mut("items") {
        if items.is_object() {
            convert_to_strict(items);
        }
    }

    // prefixItems — the Draft 2020-12 tuple form. Without this, an object
    // sitting at a tuple position was never hardened.
    if let Some(prefix_items) = obj.get_mut("prefixItems").and_then(|v| v.as_array_mut()) {
        for item in prefix_items.iter_mut() {
            convert_to_strict(item);
        }
    }

    // composition keywords
    for keyword in &["oneOf", "anyOf", "allOf"] {
        if let Some(arr) = obj.get_mut(*keyword).and_then(|v| v.as_array_mut()) {
            for sub in arr.iter_mut() {
                convert_to_strict(sub);
            }
        }
    }

    // definitions / $defs
    for defs_key in &["definitions", "$defs"] {
        if let Some(defs) = obj.get_mut(*defs_key).and_then(|v| v.as_object_mut()) {
            for defn in defs.values_mut() {
                convert_to_strict(defn);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_strict_conversion() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            },
            "required": ["name"]
        });

        let result = to_strict_schema(&schema);

        assert_eq!(result["additionalProperties"], json!(false));
        // All properties should be required (sorted).
        assert_eq!(result["required"], json!(["age", "name"]));
        // "name" was already required — should stay as string type.
        assert_eq!(result["properties"]["name"]["type"], json!("string"));
        // "age" was optional — should become nullable.
        assert_eq!(
            result["properties"]["age"]["type"],
            json!(["integer", "null"])
        );
    }

    #[test]
    fn test_strips_extensions_and_defaults() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "x-llm-description": "The user name",
                    "default": "anonymous"
                }
            },
            "x-custom": "value"
        });

        let result = to_strict_schema(&schema);

        assert!(result.get("x-custom").is_none());
        assert!(result["properties"]["name"]
            .get("x-llm-description")
            .is_none());
        assert!(result["properties"]["name"].get("default").is_none());
    }

    #[test]
    fn test_nested_object_strict() {
        let schema = json!({
            "type": "object",
            "properties": {
                "address": {
                    "type": "object",
                    "properties": {
                        "street": {"type": "string"},
                        "city": {"type": "string"}
                    }
                }
            },
            "required": ["address"]
        });

        let result = to_strict_schema(&schema);

        let address = &result["properties"]["address"];
        assert_eq!(address["additionalProperties"], json!(false));
        assert_eq!(address["required"], json!(["city", "street"]));
        // Both were optional in the nested object.
        assert_eq!(
            address["properties"]["street"]["type"],
            json!(["string", "null"])
        );
    }

    #[test]
    fn test_array_items_recursion() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "key": {"type": "string"},
                            "value": {"type": "string"}
                        }
                    }
                }
            }
        });

        let result = to_strict_schema(&schema);

        let items = &result["properties"]["tags"]["items"];
        assert_eq!(items["additionalProperties"], json!(false));
        assert_eq!(items["required"], json!(["key", "value"]));
    }

    #[test]
    fn test_composition_keywords_recursion() {
        let schema = json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "a": {"type": "string"}
                    }
                },
                {
                    "type": "object",
                    "properties": {
                        "b": {"type": "integer"}
                    }
                }
            ]
        });

        let result = to_strict_schema(&schema);

        assert_eq!(result["oneOf"][0]["additionalProperties"], json!(false));
        assert_eq!(result["oneOf"][1]["additionalProperties"], json!(false));
    }

    #[test]
    fn test_defs_recursion() {
        let schema = json!({
            "type": "object",
            "properties": {
                "ref_field": {"$ref": "#/$defs/Thing"}
            },
            "$defs": {
                "Thing": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"}
                    }
                }
            }
        });

        let result = to_strict_schema(&schema);

        let thing = &result["$defs"]["Thing"];
        assert_eq!(thing["additionalProperties"], json!(false));
        assert_eq!(thing["required"], json!(["id"]));
    }

    #[test]
    fn test_ref_only_property_becomes_nullable() {
        let schema = json!({
            "type": "object",
            "properties": {
                "required_field": {"type": "string"},
                "optional_ref": {"$ref": "#/$defs/Other"}
            },
            "required": ["required_field"]
        });

        let result = to_strict_schema(&schema);

        // optional_ref had no "type", so it is wrapped in a nullable union.
        // `anyOf`, not `oneOf`: OpenAI structured outputs — the consumer strict
        // mode exists for — accepts only `anyOf`.
        let optional = &result["properties"]["optional_ref"];
        assert!(optional.get("oneOf").is_none());
        let any_of = optional["anyOf"].as_array().unwrap();
        assert_eq!(any_of.len(), 2);
        assert_eq!(any_of[0], json!({"$ref": "#/$defs/Other"}));
        assert_eq!(any_of[1], json!({"type": "null"}));
    }

    #[test]
    fn test_object_with_properties_but_no_type_keyword_is_hardened() {
        // `properties` alone already makes this an object schema. Requiring a
        // `type` keyword let it through with neither `additionalProperties:
        // false` nor a `required` list — exactly what strict mode must add.
        let schema = json!({
            "properties": {
                "x": {"type": "string"}
            }
        });

        let result = to_strict_schema(&schema);

        assert_eq!(result["additionalProperties"], json!(false));
        assert_eq!(result["required"], json!(["x"]));
        assert_eq!(result["properties"]["x"]["type"], json!(["string", "null"]));
    }

    #[test]
    fn test_nested_object_without_type_keyword_is_hardened() {
        let schema = json!({
            "type": "object",
            "properties": {
                "inner": {
                    "properties": {"y": {"type": "integer"}}
                }
            },
            "required": ["inner"]
        });

        let result = to_strict_schema(&schema);

        let inner = &result["properties"]["inner"];
        assert_eq!(inner["additionalProperties"], json!(false));
        assert_eq!(inner["required"], json!(["y"]));
    }

    #[test]
    fn test_prefix_items_entries_are_hardened() {
        // Draft 2020-12 tuple form. Without recursion into `prefixItems`, an
        // object sitting at a tuple position was returned unhardened.
        let schema = json!({
            "type": "object",
            "properties": {
                "pair": {
                    "type": "array",
                    "prefixItems": [
                        {"type": "object", "properties": {"a": {"type": "string"}}},
                        {"type": "string"}
                    ]
                }
            },
            "required": ["pair"]
        });

        let result = to_strict_schema(&schema);

        let first = &result["properties"]["pair"]["prefixItems"][0];
        assert_eq!(first["additionalProperties"], json!(false));
        assert_eq!(first["required"], json!(["a"]));
    }

    #[test]
    fn test_already_nullable_not_doubled() {
        let schema = json!({
            "type": "object",
            "properties": {
                "field": {"type": ["string", "null"]}
            }
        });

        let result = to_strict_schema(&schema);

        // Should not add a second "null".
        assert_eq!(
            result["properties"]["field"]["type"],
            json!(["string", "null"])
        );
    }

    #[test]
    fn test_non_object_schema_passthrough() {
        let schema = json!({
            "type": "string",
            "minLength": 1
        });

        let result = to_strict_schema(&schema);

        // No additionalProperties should be added to non-object types.
        assert!(result.get("additionalProperties").is_none());
        assert_eq!(result["type"], json!("string"));
    }

    #[test]
    fn test_input_not_mutated() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            }
        });

        let _ = to_strict_schema(&schema);

        // Original should be unchanged.
        assert!(schema.get("additionalProperties").is_none());
        assert!(schema.get("required").is_none());
    }
}
