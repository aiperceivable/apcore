// APCore Protocol — Schema hardening utilities (Issue #44, PROTOCOL_SPEC §4.15).
//
// Provides:
// - `content_hash` — SHA-256 of canonical (sorted-keys) JSON; used for content-addressable caching.
// - `format_warnings` — opt-in semantic format check (date-time, email, uri, …) that emits
//   non-fatal warnings rather than hard errors (SHOULD-level enforcement).

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// SHA-256 hex digest (lowercase, 64 chars) of the canonical-JSON form of `schema`.
///
/// Canonical form: object keys are recursively sorted; primitive serialization is
/// the same as `serde_json::to_string`. Two byte-equivalent schemas with different
/// key orderings hash to the same digest — this is the deduplication invariant
/// required by PROTOCOL_SPEC §4.15.5.
#[must_use]
pub fn content_hash(schema: &Value) -> String {
    let canonical = canonical_json(schema);
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Serialize a JSON value with object keys sorted at every level. Output uses the
/// same compact form as `serde_json::to_string` (no whitespace).
///
/// Crate-visible because the registry's D-89 deprecation-warning dedupe key
/// needs the same "equal by value, regardless of how it was built" rendering
/// for an `x-deprecation` block (spec v1.59.0).
pub(crate) fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                // INVARIANT: serializing a string value via serde_json never fails.
                out.push_str(&serde_json::to_string(key).expect("key is a String"));
                out.push(':');
                // INVARIANT: every iterated key is present in `map`.
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => {
            // INVARIANT: serializing a primitive JSON value never fails.
            out.push_str(&serde_json::to_string(other).expect("primitive serialization"));
        }
    }
}

/// A non-fatal format-mismatch warning produced by [`format_warnings`].
///
/// Format enforcement is SHOULD-level per PROTOCOL_SPEC §4.15.4 — invalid
/// `format` values do not fail validation; they surface here so callers can
/// log or surface them as warnings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatWarning {
    /// JSON pointer to the offending field, e.g. `/occurred_at`.
    pub path: String,
    /// The declared format, e.g. `date-time`.
    pub format: String,
    /// The string value that did not parse as the declared format.
    pub value: String,
}

/// Walk `data` against `schema` and return one [`FormatWarning`] for each
/// string field whose declared `format` does not parse.
///
/// Unmapped formats are ignored. Non-string values are ignored (per Draft 2020-12,
/// `format` only constrains strings). The traversal mirrors the Python reference
/// implementation in `apcore.schema.hardening._check_formats_and_warn` and
/// apcore-typescript's `SchemaValidator._walkFormats`.
///
/// The same annotation can be reached through more than one combinator branch;
/// duplicates are collapsed so a field is reported once.
#[must_use]
pub fn format_warnings(data: &Value, schema: &Value) -> Vec<FormatWarning> {
    let mut warnings = Vec::new();
    let mut cache = WalkCache::default();
    walk_format(data, schema, "", &mut warnings, &mut cache);
    // Order-preserving de-duplication. An array of N offending elements yields
    // N warnings, so the membership test has to be O(1), not a linear rescan.
    let mut seen: HashSet<FormatWarning> = HashSet::with_capacity(warnings.len());
    warnings
        .into_iter()
        .filter(|warning| seen.insert(warning.clone()))
        .collect()
}

/// Memo table shared by one [`format_warnings`] traversal.
///
/// Entries are keyed by the *address* of a schema node. The schema is borrowed
/// immutably for the whole walk, so every node keeps a stable address and two
/// lookups with the same address always describe the same subtree. The table is
/// created per call and dropped with it, so an address can never be reused
/// while an entry for it is live.
///
/// Without it, a union nested `d` levels deep re-derived `declares_format` over
/// the same subtree once per level — quadratic in the schema size.
#[derive(Default)]
struct WalkCache {
    declares_format: HashMap<usize, bool>,
}

impl WalkCache {
    /// `true` when `schema` carries a `format` keyword anywhere in its subtree.
    ///
    /// Used to skip a union branch that could not produce a warning anyway.
    fn declares_format(&mut self, schema: &Value) -> bool {
        let key = std::ptr::from_ref(schema) as usize;
        if let Some(hit) = self.declares_format.get(&key) {
            return *hit;
        }
        let found = match schema {
            Value::Object(map) => {
                map.get("format").is_some_and(Value::is_string)
                    || map.values().any(|child| self.declares_format(child))
            }
            Value::Array(items) => items.iter().any(|item| self.declares_format(item)),
            _ => false,
        };
        self.declares_format.insert(key, found);
        found
    }
}

/// Recursive worker for [`format_warnings`].
///
/// Traversal covers `properties`, `patternProperties`, `additionalProperties`,
/// `items`, `prefixItems`, `anyOf`, `oneOf` and `allOf`.
///
/// Known limitations, all deliberate:
/// - `$ref` is **not** dereferenced. Doing so needs the full document plus
///   cycle detection (a recursive schema such as a tree node would otherwise
///   loop forever); resolve the schema with [`crate::schema::RefResolver`]
///   first if annotations behind a `$ref` matter. Missing a warning is
///   harmless — format enforcement is SHOULD-level.
/// - Tuple-form `items` (`"items": [ … ]`, draft-07 and earlier) is not walked;
///   its Draft 2020-12 spelling `prefixItems` is.
fn walk_format(
    data: &Value,
    schema: &Value,
    path: &str,
    out: &mut Vec<FormatWarning>,
    cache: &mut WalkCache,
) {
    let Some(schema_obj) = schema.as_object() else {
        return;
    };

    // Union: the annotation lives on the branches, not on the union node.
    for keyword in ["anyOf", "oneOf"] {
        if let Some(branches) = schema_obj.get(keyword).and_then(|v| v.as_array()) {
            walk_union(data, branches, path, out, cache);
        }
    }

    // Intersection: every member applies, so every member's annotations count.
    if let Some(members) = schema_obj.get("allOf").and_then(|v| v.as_array()) {
        for member in members {
            walk_format(data, member, path, out, cache);
        }
    }

    if let (Some(fmt), Some(s)) = (
        schema_obj.get("format").and_then(|v| v.as_str()),
        data.as_str(),
    ) {
        if !check_format(fmt, s) {
            out.push(FormatWarning {
                path: if path.is_empty() {
                    "/".to_string()
                } else {
                    path.to_string()
                },
                format: fmt.to_string(),
                value: s.to_string(),
            });
        }
    }

    let named = schema_obj.get("properties").and_then(|p| p.as_object());
    let patterns = compile_pattern_properties(schema_obj);

    if let (Some(props), Some(obj)) = (named, data.as_object()) {
        for (name, prop_schema) in props {
            if let Some(value) = obj.get(name) {
                let child_path = format!("{path}/{name}");
                walk_format(value, prop_schema, &child_path, out, cache);
            }
        }
    }

    // `patternProperties` applies to every key its pattern matches, on top of
    // whatever `properties` already said about that key (Draft 2020-12 §10.3.2.2).
    if let Some(obj) = data.as_object() {
        for (pattern, sub_schema) in &patterns {
            for (name, value) in obj {
                if pattern.is_match(name) {
                    let child_path = format!("{path}/{name}");
                    walk_format(value, sub_schema, &child_path, out, cache);
                }
            }
        }
    }

    // `additionalProperties` covers only the keys named by neither `properties`
    // nor `patternProperties` (Draft 2020-12 §10.3.2.3). Checking a
    // pattern-matched key against it as well reported formats the key was never
    // declared to carry. A boolean value carries no annotations and is skipped.
    if let (Some(additional), Some(obj)) = (
        schema_obj
            .get("additionalProperties")
            .filter(|v| v.is_object()),
        data.as_object(),
    ) {
        for (name, value) in obj {
            if named.is_some_and(|props| props.contains_key(name)) {
                continue;
            }
            if patterns.iter().any(|(pattern, _)| pattern.is_match(name)) {
                continue;
            }
            let child_path = format!("{path}/{name}");
            walk_format(value, additional, &child_path, out, cache);
        }
    }

    // Draft 2020-12 tuple form: `prefixItems` positionally covers the leading
    // elements, `items` covers whatever follows.
    let prefix_len = match (
        schema_obj.get("prefixItems").and_then(|v| v.as_array()),
        data.as_array(),
    ) {
        (Some(prefix_schemas), Some(arr)) => {
            for (i, (item, item_schema)) in arr.iter().zip(prefix_schemas).enumerate() {
                let child_path = format!("{path}/{i}");
                walk_format(item, item_schema, &child_path, out, cache);
            }
            prefix_schemas.len()
        }
        _ => 0,
    };

    if let (Some(items_schema), Some(arr)) = (schema_obj.get("items"), data.as_array()) {
        for (i, item) in arr.iter().enumerate().skip(prefix_len) {
            let child_path = format!("{path}/{i}");
            walk_format(item, items_schema, &child_path, out, cache);
        }
    }
}

/// Compile the `patternProperties` regexes declared on one schema node.
///
/// JSON Schema mandates ECMA-262 regular expressions; the `regex` crate covers
/// the subset real schemas use. A pattern it cannot compile is skipped, so the
/// keys that pattern would have claimed fall through to `additionalProperties`
/// — the pre-fix behaviour, which can only add a warning, never hide one.
fn compile_pattern_properties(schema_obj: &Map<String, Value>) -> Vec<(Regex, &Value)> {
    let Some(patterns) = schema_obj
        .get("patternProperties")
        .and_then(|v| v.as_object())
    else {
        return Vec::new();
    };
    patterns
        .iter()
        .filter_map(|(pattern, sub_schema)| {
            Regex::new(pattern).ok().map(|regex| (regex, sub_schema))
        })
        .collect()
}

/// Collect the warnings an `anyOf` / `oneOf` node contributes.
///
/// Only branches that declare a `format` can contribute at all. Of those, a
/// branch the data provably contradicts is dropped ([`contradicts`]): a sibling
/// branch describes a different shape, and warning from it would report a format
/// the value was never meant to carry.
///
/// The survivors are then probed individually. If **any** survivor comes back
/// clean — every `format` it declares checks out — the data unambiguously
/// belongs to that branch and the union contributes nothing. Otherwise every
/// survivor's warnings are reported: the value fits no branch, and
/// over-reporting a SHOULD-level warning beats staying silent about a real one.
///
/// The cleanliness probe, not the structural check, is what separates branches
/// differing *only* by `format`: apcore compiles every schema with format
/// assertions disabled (see [`crate::schema::validator::build_validator`]), so
/// no structural check could ever tell `{"format": "uuid"}` from
/// `{"format": "email"}`.
fn walk_union(
    data: &Value,
    branches: &[Value],
    path: &str,
    out: &mut Vec<FormatWarning>,
    cache: &mut WalkCache,
) {
    let survivors: Vec<&Value> = branches
        .iter()
        .filter(|branch| cache.declares_format(branch))
        .filter(|branch| !contradicts(branch, data))
        .collect();

    let mut probes: Vec<Vec<FormatWarning>> = Vec::with_capacity(survivors.len());
    for branch in survivors {
        let mut probe = Vec::new();
        walk_format(data, branch, path, &mut probe, cache);
        if probe.is_empty() {
            return;
        }
        probes.push(probe);
    }
    for probe in probes {
        out.extend(probe);
    }
}

/// `true` only when `data` **definitely** cannot match `schema`.
///
/// A deliberately partial check. It inspects the keywords whose verdict is both
/// certain and cheap — `type`, `const`, `enum`, `required`, and those same
/// keywords beneath `properties`, `allOf`, `anyOf` and `oneOf` — and answers
/// `false`, "cannot tell", for everything else, `$ref` included.
///
/// Three reasons it is hand-rolled rather than delegated to a compiled
/// `jsonschema::Validator`, which is what this guard used to do:
/// - A branch holding a `$ref` into the root document cannot be compiled in
///   isolation at all. Reading that failure as "does not match" silenced every
///   nullable-reference schema (`anyOf: [{allOf: [{$ref}], format}, {null}]`)
///   permanently. Here such a branch is simply undecidable, and survives.
/// - Compiling a branch costs time proportional to its entire subtree, and a
///   union nested `d` levels deep paid that once per level.
/// - `format` must not participate anyway, and it is the one keyword a compiled
///   validator would have contributed over this check.
///
/// Erring permissively costs one extra SHOULD-level warning; erring strictly
/// hides a real one, so every uncertain case resolves to `false`.
fn contradicts(schema: &Value, data: &Value) -> bool {
    let Some(obj) = schema.as_object() else {
        return false;
    };

    match obj.get("type") {
        Some(Value::String(name)) if !type_matches(name, data) => return true,
        Some(Value::Array(names))
            if !names
                .iter()
                .filter_map(Value::as_str)
                .any(|name| type_matches(name, data)) =>
        {
            return true;
        }
        _ => {}
    }

    if obj.get("const").is_some_and(|expected| expected != data) {
        return true;
    }

    if let Some(Value::Array(allowed)) = obj.get("enum") {
        if !allowed.contains(data) {
            return true;
        }
    }

    if let (Some(Value::Array(names)), Some(map)) = (obj.get("required"), data.as_object()) {
        if names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| !map.contains_key(name))
        {
            return true;
        }
    }

    if let (Some(Value::Object(props)), Some(map)) = (obj.get("properties"), data.as_object()) {
        if props
            .iter()
            .any(|(name, sub)| map.get(name).is_some_and(|value| contradicts(sub, value)))
        {
            return true;
        }
    }

    if let Some(Value::Array(members)) = obj.get("allOf") {
        if members.iter().any(|member| contradicts(member, data)) {
            return true;
        }
    }

    // A union is contradicted only when every one of its branches is.
    for keyword in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = obj.get(keyword) {
            if !branches.is_empty() && branches.iter().all(|branch| contradicts(branch, data)) {
                return true;
            }
        }
    }

    false
}

/// `true` when `data` could be an instance of the JSON Schema type `declared`.
///
/// `integer` is treated as plain `number`: telling `3.0` from `3` adds nothing
/// a union realistically discriminates on, and a wrong rejection here silences
/// a warning.
fn type_matches(declared: &str, data: &Value) -> bool {
    match declared {
        "null" => data.is_null(),
        "boolean" => data.is_boolean(),
        "string" => data.is_string(),
        "integer" | "number" => data.is_number(),
        "array" => data.is_array(),
        "object" => data.is_object(),
        // An unknown `type` value is not something this check can rule on.
        _ => true,
    }
}

/// Returns `true` when `value` parses as a member of the declared `format`,
/// `false` when it does not. Unmapped formats return `true` (no opinion).
fn check_format(format: &str, value: &str) -> bool {
    match format {
        "date-time" => {
            DateTime::parse_from_rfc3339(value).is_ok() || value.parse::<DateTime<Utc>>().is_ok()
        }
        "date" => NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok(),
        "time" => {
            NaiveTime::parse_from_str(value, "%H:%M:%S").is_ok()
                || NaiveTime::parse_from_str(value, "%H:%M:%S%.f").is_ok()
        }
        "uuid" => Uuid::parse_str(value).is_ok(),
        "ipv4" => value.parse::<Ipv4Addr>().is_ok(),
        "ipv6" => value.parse::<Ipv6Addr>().is_ok(),
        "email" => is_email(value),
        "uri" => is_uri(value),
        _ => true,
    }
}

fn is_email(value: &str) -> bool {
    // Mirrors apcore-python: ^[^@\s]+@[^@\s]+\.[^@\s]+$
    let bytes = value.as_bytes();
    let Some(at_idx) = value.find('@') else {
        return false;
    };
    if at_idx == 0 || at_idx == bytes.len() - 1 {
        return false;
    }
    let (local, domain_with_at) = value.split_at(at_idx);
    let domain = &domain_with_at[1..];
    if local.contains(char::is_whitespace) || domain.contains(char::is_whitespace) {
        return false;
    }
    if local.contains('@') || domain.contains('@') {
        return false;
    }
    let Some(dot_idx) = domain.find('.') else {
        return false;
    };
    dot_idx != 0 && dot_idx != domain.len() - 1
}

fn is_uri(value: &str) -> bool {
    // Mirrors apcore-python: scheme://… requires alpha + alnum/+/-/. then "://"
    let Some(scheme_end) = value.find("://") else {
        return false;
    };
    let scheme = &value[..scheme_end];
    if scheme.is_empty() {
        return false;
    }
    let mut chars = scheme.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_content_hash_canonical_form_is_key_order_invariant() {
        let a = json!({ "type": "object", "required": ["name"], "properties": { "name": { "type": "string" } } });
        let b = json!({ "properties": { "name": { "type": "string" } }, "required": ["name"], "type": "object" });
        assert_eq!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn test_content_hash_distinguishes_different_schemas() {
        let a = json!({ "type": "object", "properties": { "name": { "type": "string" } } });
        let b = json!({ "type": "object", "properties": { "age": { "type": "integer" } } });
        assert_ne!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn test_content_hash_empty_object_is_stable() {
        let a = json!({});
        let b = json!({});
        let h = content_hash(&a);
        assert_eq!(h.len(), 64);
        assert_eq!(h, content_hash(&b));
    }

    #[test]
    fn test_format_warnings_invalid_datetime_emits_warning() {
        let schema = json!({
            "type": "object",
            "properties": { "occurred_at": { "type": "string", "format": "date-time" } }
        });
        let data = json!({ "occurred_at": "not-a-date" });
        let warnings = format_warnings(&data, &schema);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].format, "date-time");
        assert_eq!(warnings[0].path, "/occurred_at");
    }

    #[test]
    fn test_format_warnings_valid_datetime_no_warning() {
        let schema = json!({
            "type": "object",
            "properties": { "occurred_at": { "type": "string", "format": "date-time" } }
        });
        let data = json!({ "occurred_at": "2026-04-28T12:00:00Z" });
        assert_eq!(format_warnings(&data, &schema), Vec::<FormatWarning>::new());
    }

    #[test]
    fn test_format_warnings_unmapped_format_silent() {
        let schema = json!({
            "type": "object",
            "properties": { "exotic": { "type": "string", "format": "made-up-format" } }
        });
        let data = json!({ "exotic": "anything" });
        assert_eq!(format_warnings(&data, &schema), Vec::<FormatWarning>::new());
    }

    #[test]
    fn test_check_format_email() {
        assert!(check_format("email", "alice@example.com"));
        assert!(!check_format("email", "not-an-email"));
        assert!(!check_format("email", "@nope.com"));
        assert!(!check_format("email", "bare@nodot"));
    }

    #[test]
    fn test_check_format_uri() {
        assert!(check_format("uri", "https://example.com/path"));
        assert!(check_format("uri", "ftp://example.org"));
        assert!(!check_format("uri", "not-a-uri"));
        assert!(!check_format("uri", "://missing-scheme"));
    }
}
