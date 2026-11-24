//! OpenAI structured-outputs strict-mode compatibility detection.
//!
//! This module is deliberately **separate** from [`crate::schema::strict`].
//! `to_strict_schema()` is a general-purpose registry/export transform; baking
//! an OpenAI-specific dialect check into it would leak one vendor's constraints
//! into every other consumer. Detection lives here and is invoked only on the
//! `auto_schema: strict` binding path (protocol-spec §5.12.5).
//!
//! Detection **never rewrites** the schema. In particular an author-written
//! `oneOf` is reported, not silently downgraded to `anyOf`: the two differ in
//! exclusivity, and apcore's own validator relies on the distinction (it counts
//! matching branches and raises `SCHEMA_UNION_AMBIGUOUS`). Rewriting at export
//! time would trade an explicit export-time rejection for a silent runtime
//! mismatch.
//!
//! ---------------------------------------------------------------------------
//! Source of truth for the keyword set below:
//!   OpenAI — "Structured model outputs", sections "Supported schemas /
//!   Supported properties" and "Some type-specific keywords are not yet
//!   supported".
//!   <https://developers.openai.com/api/docs/guides/structured-outputs>
//!   Verified 2026-08-03.
//!
//! What that page states, in substance:
//!   - Supported types: String, Number, Boolean, Integer, Object, Array, Enum,
//!     anyOf.
//!   - Supported `string` properties: `pattern`, `format` (limited to the nine
//!     values in [`SUPPORTED_FORMATS`]).
//!   - Supported `number` properties: `multipleOf`, `maximum`,
//!     `exclusiveMaximum`, `minimum`, `exclusiveMinimum`.
//!   - Supported `array` properties: `minItems`, `maxItems`.
//!   - Not supported — composition: `allOf`, `not`, `dependentRequired`,
//!     `dependentSchemas`, `if`, `then`, `else`.
//!   - `anyOf` is supported but MUST NOT appear at the root (the root must be
//!     an object).
//!   - `$ref` / `$defs` (including recursive `#` self-references) are supported.
//!   - `additionalProperties: false` and "every property listed in `required`"
//!     are requirements, not incompatibilities — `to_strict_schema()` already
//!     produces both, so they are transformed rather than reported here.
//!
//! Anything appearing in neither the supported nor the explicitly-unsupported
//! list (`oneOf`, `minLength`/`maxLength`, `patternProperties`, `uniqueItems`,
//! …) is treated as unsupported: OpenAI rejects unknown or unlisted keywords
//! when `strict: true`.
//!
//! NOTE: this set tracks a moving target. When OpenAI expands structured-output
//! support, update the three SDK copies (`schema/openai_strict.rs`,
//! `schema/openai_strict.py`, `schema/openai-strict.ts`) together and re-run the
//! `openai_strict_compat.json` conformance fixture, which pins the cross-SDK
//! answer.
//! ---------------------------------------------------------------------------

use std::collections::BTreeSet;

use serde_json::Value;

use crate::errors::{ErrorCode, ModuleError};

/// JSON Schema keywords OpenAI structured outputs does not accept under
/// `strict: true`. Kept sorted for reviewability; emission order is normalised
/// by the `BTreeSet` accumulator, so cross-SDK output does not depend on it.
pub const UNSUPPORTED_KEYWORDS: &[&str] = &[
    // Composition — explicitly listed as unsupported by OpenAI.
    "allOf",
    "dependentRequired",
    "dependentSchemas",
    "else",
    "if",
    "not",
    "then",
    // `oneOf` is absent from the supported list (only `anyOf` is named).
    "oneOf",
    // String constraints beyond `pattern` / `format`.
    "maxLength",
    "minLength",
    // Object keywords beyond `properties` / `required` / `additionalProperties`.
    "maxProperties",
    "minProperties",
    "patternProperties",
    "propertyNames",
    "unevaluatedProperties",
    // Array keywords beyond `items` / `minItems` / `maxItems`.
    "additionalItems",
    "contains",
    "maxContains",
    "minContains",
    "prefixItems",
    "unevaluatedItems",
    "uniqueItems",
];

/// The nine `format` values OpenAI accepts for strings.
pub const SUPPORTED_FORMATS: &[&str] = &[
    "date-time",
    "time",
    "date",
    "duration",
    "email",
    "hostname",
    "ipv4",
    "ipv6",
    "uuid",
];

fn walk(node: &Value, path: &str, is_root: bool, found: &mut BTreeSet<String>) {
    let Some(map) = node.as_object() else {
        return;
    };

    for keyword in UNSUPPORTED_KEYWORDS {
        if map.contains_key(*keyword) {
            found.insert(format!("{path}.{keyword}"));
        }
    }

    // `anyOf` is supported, but not as the root schema.
    if is_root && map.contains_key("anyOf") {
        found.insert(format!("{path}.anyOf"));
    }

    if let Some(fmt) = map.get("format").and_then(Value::as_str) {
        if !SUPPORTED_FORMATS.contains(&fmt) {
            found.insert(format!("{path}.format={fmt}"));
        }
    }

    if let Some(properties) = map.get("properties").and_then(Value::as_object) {
        // `serde_json::Map` iterates in insertion order unless `preserve_order`
        // is off; collect keys into a BTreeSet so traversal order is stable and
        // matches the Python/TypeScript `sorted()` walks.
        let keys: BTreeSet<&String> = properties.keys().collect();
        for key in keys {
            walk(&properties[key], &format!("{path}.{key}"), false, found);
        }
    }

    if let Some(items) = map.get("items") {
        if items.is_object() {
            walk(items, &format!("{path}[]"), false, found);
        }
    }

    if let Some(additional) = map.get("additionalProperties") {
        if additional.is_object() {
            walk(
                additional,
                &format!("{path}.additionalProperties"),
                false,
                found,
            );
        }
    }

    // Recurse into union branches only. `allOf`/`not`/`if`/`then`/`else` are
    // already reported by keyword; descending into them would add noise beneath
    // a finding the caller must fix by removing the branch anyway.
    for combinator in ["anyOf", "oneOf"] {
        if let Some(branches) = map.get(combinator).and_then(Value::as_array) {
            for (index, branch) in branches.iter().enumerate() {
                walk(
                    branch,
                    &format!("{path}.{combinator}[{index}]"),
                    false,
                    found,
                );
            }
        }
    }

    for defs_key in ["$defs", "definitions"] {
        if let Some(defs) = map.get(defs_key).and_then(Value::as_object) {
            let keys: BTreeSet<&String> = defs.keys().collect();
            for key in keys {
                walk(
                    &defs[key],
                    &format!("{path}.{defs_key}.{key}"),
                    false,
                    found,
                );
            }
        }
    }
}

/// Return every feature OpenAI structured outputs rejects under `strict: true`.
///
/// Entries are JSON-path-ish descriptors, sorted and de-duplicated so that
/// Python, TypeScript and Rust produce identical lists:
///
/// - `$.tags.uniqueItems`   — unsupported keyword at that location
/// - `$.created.format=uri` — unsupported `format` value
/// - `$.anyOf`              — `anyOf` at the root (the root must be an object)
///
/// An empty vector means the schema is compatible.
#[must_use]
pub fn detect_openai_strict_incompatibilities(schema: &Value) -> Vec<String> {
    let mut found = BTreeSet::new();
    walk(schema, "$", true, &mut found);
    found.into_iter().collect()
}

/// Return `Err(BindingStrictSchemaIncompatible)` when `schema` contains any
/// feature OpenAI structured outputs rejects under `strict: true`.
///
/// Invoked on the `auto_schema: strict` binding path only
/// (protocol-spec §5.12.5). `Ok(())` for compatible schemas.
///
/// `side` prefixes every reported feature (e.g. `input` / `output`); pass
/// `None` to report bare paths. The message matches the canonical template in
/// protocol-spec §5.12.8.
///
/// # Errors
///
/// Returns [`ErrorCode::BindingStrictSchemaIncompatible`] listing every
/// incompatible feature found.
pub fn assert_openai_strict_compatible(
    schema: &Value,
    module_id: &str,
    side: Option<&str>,
    file_path: Option<&str>,
) -> Result<(), ModuleError> {
    let features = detect_openai_strict_incompatibilities(schema);
    if features.is_empty() {
        return Ok(());
    }

    let listed: Vec<String> = match side {
        Some(side) => features.iter().map(|f| format!("{side}:{f}")).collect(),
        None => features,
    };
    let loc = file_path.map_or_else(String::new, |p| format!("{p}: "));

    let mut details = std::collections::HashMap::new();
    details.insert(
        "module_id".to_string(),
        Value::String(module_id.to_string()),
    );
    details.insert(
        "features_listed".to_string(),
        Value::Array(listed.iter().cloned().map(Value::String).collect()),
    );
    if let Some(path) = file_path {
        details.insert("file_path".to_string(), Value::String(path.to_string()));
    }

    Err(ModuleError::new(
        ErrorCode::BindingStrictSchemaIncompatible,
        format!(
            "{loc}binding '{module_id}' uses auto_schema: strict but inferred schema contains incompatible features: {}. See protocol-spec §5.12.5",
            listed.join(", ")
        ),
    )
    .with_details(details))
}
