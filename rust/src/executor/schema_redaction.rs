//! Local-reference traversal for schema-driven redaction.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

const LOCAL_ROOT_URI: &str = "apcore://local/redaction-root";

struct LocalSchemas<'a> {
    bases: HashMap<*const Value, String>,
    drafts: HashMap<*const Value, referencing::Draft>,
    resources: HashMap<String, &'a Value>,
    anchors: HashMap<String, &'a Value>,
}

impl<'a> LocalSchemas<'a> {
    fn new(root: &'a Value) -> Self {
        let mut local = Self {
            bases: HashMap::new(),
            drafts: HashMap::new(),
            resources: HashMap::from([(LOCAL_ROOT_URI.to_string(), root)]),
            anchors: HashMap::new(),
        };
        local.index_schema(root, LOCAL_ROOT_URI, referencing::Draft::Draft202012);
        local
    }

    fn index_schema(
        &mut self,
        schema: &'a Value,
        parent_base: &str,
        parent_draft: referencing::Draft,
    ) {
        let draft = parent_draft.detect(schema).unwrap_or(parent_draft);
        self.drafts.insert(std::ptr::from_ref(schema), draft);
        let identifier = schema.get("$id").and_then(Value::as_str);
        let resolved = identifier.and_then(|id| resolve_uri(id, parent_base));
        let base = resolved.as_deref().unwrap_or(parent_base);
        let (resource, fragment) = base.split_once('#').unwrap_or((base, ""));
        if resolved.is_some() {
            if fragment.is_empty() {
                self.resources.insert(resource.to_string(), schema);
            } else {
                self.anchors.insert(base.to_string(), schema);
            }
        }
        self.bases
            .insert(std::ptr::from_ref(schema), resource.to_string());
        if let Some(anchor) = schema.get("$anchor").and_then(Value::as_str) {
            self.anchors.insert(format!("{resource}#{anchor}"), schema);
        }
        for child in draft.subresources_of(schema) {
            self.index_schema(child, resource, draft);
        }
    }

    fn resolve(&self, schema: &Value, reference: &str) -> Option<&'a Value> {
        let base = self.bases.get(&std::ptr::from_ref(schema))?;
        let resolved = resolve_uri(reference, base)?;
        let (resource, fragment) = resolved.split_once('#').unwrap_or((&resolved, ""));
        let root = self.resources.get(resource)?;
        let fragment = decode_fragment(fragment)?;
        if fragment.is_empty() {
            Some(root)
        } else if fragment.starts_with('/') {
            root.pointer(&fragment)
        } else {
            self.anchors.get(&format!("{resource}#{fragment}")).copied()
        }
    }
}

fn resolve_uri(reference: &str, base: &str) -> Option<String> {
    let base = referencing::Uri::parse(base).ok()?;
    let reference = referencing::UriRef::parse(reference).ok()?;
    Some(
        reference
            .resolve_against(&base)
            .ok()?
            .normalize()
            .to_string(),
    )
}

fn decode_fragment(fragment: &str) -> Option<String> {
    let encoded = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        if encoded[index] == b'%' {
            let digits = std::str::from_utf8(encoded.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(digits, 16).ok()?);
            index += 3;
        } else {
            decoded.push(encoded[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

pub(super) fn redact(value: &mut Value, schema: &Value) {
    let preserve_object_root = value.is_object();
    redact_value(
        value,
        schema,
        &LocalSchemas::new(schema),
        preserve_object_root,
    );
}

fn redact_value<'a>(
    value: &mut Value,
    schema: &'a Value,
    local: &LocalSchemas<'a>,
    preserve_object_root: bool,
) {
    let mut pending = vec![schema];
    let mut visited = HashSet::new();
    while let Some(schema) = pending.pop() {
        // Identity is stable within the immutable schema and scoped to one data value.
        if !visited.insert(std::ptr::from_ref(schema)) {
            continue;
        }
        if !preserve_object_root && schema.get("x-sensitive") == Some(&Value::Bool(true)) {
            if !value.is_null() {
                *value = Value::String(super::REDACTED_VALUE.to_string());
            }
            return;
        }
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            if let Some(target) = local.resolve(schema, reference) {
                pending.push(target);
            }
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
                pending.extend(branches);
            }
        }
        redact_shape(value, schema, local);
    }
}

fn redact_shape<'a>(value: &mut Value, schema: &'a Value, local: &LocalSchemas<'a>) {
    match value {
        Value::Object(object) => {
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (name, field_schema) in properties {
                    if let Some(field) = object.get_mut(name) {
                        redact_value(field, field_schema, local, false);
                    }
                }
            }
        }
        Value::Array(items) => redact_items(items, schema, local),
        _ => {}
    }
}

fn redact_items<'a>(items: &mut [Value], schema: &'a Value, local: &LocalSchemas<'a>) {
    let modern_items =
        local.drafts.get(&std::ptr::from_ref(schema)) == Some(&referencing::Draft::Draft202012);
    let prefix = modern_items
        .then(|| schema.get("prefixItems").and_then(Value::as_array))
        .flatten();
    if let Some(prefix) = prefix {
        for (item, item_schema) in items.iter_mut().zip(prefix) {
            redact_value(item, item_schema, local, false);
        }
    }
    match schema.get("items") {
        Some(Value::Array(tuple)) if !modern_items => {
            for (item, item_schema) in items.iter_mut().zip(tuple) {
                redact_value(item, item_schema, local, false);
            }
            if let Some(additional) = schema.get("additionalItems") {
                for item in items.iter_mut().skip(tuple.len()) {
                    redact_value(item, additional, local, false);
                }
            }
        }
        Some(Value::Array(_)) | None => {}
        Some(item_schema) => {
            for item in items.iter_mut().skip(prefix.map_or(0, Vec::len)) {
                redact_value(item, item_schema, local, false);
            }
        }
    }
}
