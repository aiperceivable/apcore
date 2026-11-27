//! Local-reference traversal for schema-driven redaction.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;

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
    SchemaTraversal {
        local: LocalSchemas::new(schema),
        paths: None,
    }
    .visit(value, schema, preserve_object_root);
}

pub(super) fn sensitive_fields(value: &Value, schema: &Value) -> Vec<String> {
    // The traversal only reads values in collecting mode.
    let mut snapshot = value.clone();
    let mut traversal = SchemaTraversal {
        local: LocalSchemas::new(schema),
        paths: Some(SensitivePaths::default()),
    };
    traversal.visit(&mut snapshot, schema, false);
    traversal
        .paths
        .into_iter()
        .flat_map(|paths| paths.fields)
        .collect()
}

#[derive(Default)]
struct SensitivePaths {
    current: String,
    fields: BTreeSet<String>,
}

#[derive(Clone, Copy)]
enum Segment<'a> {
    Key(&'a str),
    Index(usize),
}

struct SchemaTraversal<'a> {
    local: LocalSchemas<'a>,
    paths: Option<SensitivePaths>,
}

impl<'a> SchemaTraversal<'a> {
    fn visit(&mut self, value: &mut Value, schema: &'a Value, preserve_object_root: bool) {
        let mut pending = vec![schema];
        let mut visited = HashSet::new();
        while let Some(schema) = pending.pop() {
            // Identity is stable within the immutable schema and scoped to one data value.
            if !visited.insert(std::ptr::from_ref(schema)) {
                continue;
            }
            if !preserve_object_root && schema.get("x-sensitive") == Some(&Value::Bool(true)) {
                if let Some(paths) = &mut self.paths {
                    paths.fields.insert(paths.current.clone());
                } else if !value.is_null() {
                    *value = Value::String(super::REDACTED_VALUE.to_string());
                }
                return;
            }
            if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
                if let Some(target) = self.local.resolve(schema, reference) {
                    pending.push(target);
                }
            }
            for keyword in ["allOf", "anyOf", "oneOf"] {
                if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
                    pending.extend(branches);
                }
            }
            match value {
                Value::Object(object) => {
                    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                        for (name, field_schema) in properties {
                            if let Some(field) = object.get_mut(name) {
                                self.visit_child(field, field_schema, Segment::Key(name));
                            }
                        }
                    }
                }
                Value::Array(items) => self.visit_items(items, schema),
                _ => {}
            }
        }
    }

    fn visit_child(&mut self, value: &mut Value, schema: &'a Value, segment: Segment<'_>) {
        let previous_length = self.paths.as_mut().map(|paths| {
            let length = paths.current.len();
            paths.current.push('/');
            match segment {
                Segment::Key(name) => {
                    for character in name.chars() {
                        match character {
                            '~' => paths.current.push_str("~0"),
                            '/' => paths.current.push_str("~1"),
                            character => paths.current.push(character),
                        }
                    }
                }
                Segment::Index(index) => {
                    let _ = write!(paths.current, "{index}");
                }
            }
            length
        });
        self.visit(value, schema, false);
        if let (Some(paths), Some(length)) = (&mut self.paths, previous_length) {
            paths.current.truncate(length);
        }
    }

    fn visit_items(&mut self, items: &mut [Value], schema: &'a Value) {
        let modern_items = self.local.drafts.get(&std::ptr::from_ref(schema))
            == Some(&referencing::Draft::Draft202012);
        let prefix = modern_items
            .then(|| schema.get("prefixItems").and_then(Value::as_array))
            .flatten();
        if let Some(prefix) = prefix {
            for (index, (item, item_schema)) in items.iter_mut().zip(prefix).enumerate() {
                self.visit_child(item, item_schema, Segment::Index(index));
            }
        }
        match schema.get("items") {
            Some(Value::Array(tuple)) if !modern_items => {
                for (index, (item, item_schema)) in items.iter_mut().zip(tuple).enumerate() {
                    self.visit_child(item, item_schema, Segment::Index(index));
                }
                if let Some(additional) = schema.get("additionalItems") {
                    for (index, item) in items.iter_mut().enumerate().skip(tuple.len()) {
                        self.visit_child(item, additional, Segment::Index(index));
                    }
                }
            }
            Some(Value::Array(_)) | None => {}
            Some(item_schema) => {
                for (index, item) in items
                    .iter_mut()
                    .enumerate()
                    .skip(prefix.map_or(0, Vec::len))
                {
                    self.visit_child(item, item_schema, Segment::Index(index));
                }
            }
        }
    }
}
