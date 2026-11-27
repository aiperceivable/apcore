use serde_json::{json, Value};

use super::SchemaDefinition;

/// Export a module's generic schema with its JSON Schema extensions intact.
#[must_use]
pub fn export_generic(schema: &SchemaDefinition) -> Value {
    json!({
        "module_id": schema.module_id,
        "description": schema.description,
        "input_schema": schema.input_schema,
        "output_schema": schema.output_schema,
        "definitions": schema.definitions,
    })
}
