// APCore Protocol — Schema validator (Issue #44, PROTOCOL_SPEC §4.15).
//
// Wraps `jsonschema::Validator` (Draft 2020-12) so anyOf/oneOf/allOf/not, recursive
// `$ref`, numerical/string constraints and format keyword handling are all delegated
// to a battle-tested implementation. Compiled validators are cached by SHA-256 of
// the canonical-JSON form of the schema so repeated validation against the same
// schema (or two byte-equivalent copies) only pays the compile cost once.

use std::collections::HashMap;
use std::sync::Arc;

use jsonschema::{error::ValidationErrorKind, Validator};
use parking_lot::Mutex;
use serde_json::Value;

use crate::errors::{ErrorCode, ModuleError, SchemaValidationError};
use crate::module::{ValidationErrorDetail, ValidationResult};
use crate::schema::hardening::{content_hash, format_warnings, FormatWarning};

/// Validates JSON values against JSON Schema documents (Draft 2020-12).
#[derive(Debug)]
pub struct SchemaValidator {
    cache: Arc<Mutex<HashMap<String, Arc<Validator>>>>,
    /// When `true`, a coercion pre-pass runs before validation: string→number/bool
    /// and int↔float widening, applied recursively per the schema's declared
    /// scalar types. When `false` (**the default**), validation is the raw
    /// jsonschema check with no coercion — the same thing the module-invocation
    /// boundary does (`executor::validate_against_schema`).
    coerce_types: bool,
}

impl Default for SchemaValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of [`SchemaValidator::validate_detailed`] — keeps richer error metadata
/// than the legacy [`ValidationResult`] without changing the existing public surface.
#[derive(Debug, Clone)]
pub struct DetailedValidationResult {
    /// `true` only if the input matches the schema.
    pub valid: bool,
    /// Per-failure detail objects (path/message plus optional constraint),
    /// suitable for logging or surfacing to users.
    pub errors: Vec<ValidationErrorDetail>,
    /// SCREAMING_SNAKE_CASE error code derived from the *first* error, mapped to
    /// apcore semantics: `SCHEMA_UNION_NO_MATCH`, `SCHEMA_UNION_AMBIGUOUS`, or
    /// `SCHEMA_VALIDATION_FAILED`. `None` when valid.
    pub error_code: Option<ErrorCode>,
    /// Non-fatal format warnings (SHOULD-level enforcement, opt-in).
    pub warnings: Vec<FormatWarning>,
}

impl SchemaValidator {
    /// Create a new validator with an empty internal compile cache.
    ///
    /// Type coercion is **disabled by default** (`coerce_types = false`), so this
    /// validator agrees with the module-invocation boundary
    /// ([`crate::executor::validate_against_schema`], which `builtin_steps.rs`
    /// calls): a contract that declares `integer` receives an integer, and
    /// `"42"` is a type error. Two validation paths in one SDK disagreeing about
    /// that was the divergence this default closes (TYPE_MAPPING §17.3).
    ///
    /// Use [`Self::with_coerce_types`] when a *caller* — not a module contract —
    /// genuinely wants string→scalar conversion of its own untyped input.
    #[must_use]
    pub fn new() -> Self {
        Self::with_coerce_types(false)
    }

    /// Create a new validator with explicit coercion behavior.
    ///
    /// `coerce_types = true` enables the coercion pre-pass; `false` (the
    /// [`Self::new`] default) performs the raw jsonschema check with no
    /// coercion. This is a library-level knob for callers doing their own
    /// validation — it has no effect on the module-invocation boundary, which
    /// never coerces regardless of how any host is configured.
    ///
    /// What the pre-pass coerces is normative (TYPE_MAPPING §11, spec v1.12.0):
    /// from a string only, only toward a declared type — `"42"`/`"-7"` →
    /// `integer`, `"1.5"`/`"-0.5"` → `number`, and exactly `"true"`/`"false"`
    /// (case-sensitive) → `boolean`. Nothing else. `"yes"`, `"on"`, `"1"`,
    /// `"0"` and `"True"` are NOT booleans here (apcore#95).
    #[must_use]
    pub fn with_coerce_types(coerce_types: bool) -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            coerce_types,
        }
    }

    /// Whether this validator coerces types before validating.
    #[must_use]
    pub fn coerce_types(&self) -> bool {
        self.coerce_types
    }

    /// Legacy API: validate `value` against `schema` and return a coarse-grained
    /// [`ValidationResult`] with stringified errors. Format warnings are dropped
    /// here — use [`Self::validate_detailed`] to receive them.
    #[must_use]
    pub fn validate(&self, value: &Value, schema: &Value) -> ValidationResult {
        let detailed = self.validate_detailed(value, schema);
        ValidationResult {
            valid: detailed.valid,
            errors: detailed.errors,
            warnings: Vec::new(),
        }
    }

    /// Validate `value` against `schema`, returning a [`DetailedValidationResult`]
    /// with mapped error codes and format warnings.
    #[must_use]
    pub fn validate_detailed(&self, value: &Value, schema: &Value) -> DetailedValidationResult {
        if self.coerce_types {
            let coerced = coerce_value(value, schema);
            return self.validate_detailed_raw(&coerced, schema);
        }
        self.validate_detailed_raw(value, schema)
    }

    /// Validate `value` against `schema` with NO coercion pre-pass.
    fn validate_detailed_raw(&self, value: &Value, schema: &Value) -> DetailedValidationResult {
        let validator = match self.compile_cached(schema) {
            Ok(v) => v,
            Err(message) => {
                return DetailedValidationResult {
                    valid: false,
                    errors: vec![ValidationErrorDetail::message_only(format!(
                        "invalid schema: {message}"
                    ))],
                    error_code: Some(ErrorCode::SchemaParseError),
                    warnings: Vec::new(),
                };
            }
        };

        let raw_errors: Vec<_> = validator.iter_errors(value).collect();

        if raw_errors.is_empty() {
            return DetailedValidationResult {
                valid: true,
                errors: Vec::new(),
                error_code: None,
                warnings: format_warnings(value, schema),
            };
        }

        let error_code = Some(map_error_code(&raw_errors));
        let errors = raw_errors.iter().map(build_error_detail).collect();
        DetailedValidationResult {
            valid: false,
            errors,
            error_code,
            warnings: Vec::new(),
        }
    }

    /// Validate inputs against a schema, returning the validated (and, when
    /// `coerce_types` is enabled, coerced) value on success or a [`ModuleError`]
    /// carrying the mapped apcore [`ErrorCode`] on failure.
    ///
    /// Cross-language parity with apcore-python `SchemaValidator.validate_input`
    /// (returns `model_dump()`) and apcore-typescript `SchemaValidator.validateInput`
    /// (returns `Value.Decode(...)`). When coercion is enabled the returned value
    /// reflects the coerced types (e.g. `{"age":"42"}` → `{"age":42}`); when
    /// disabled the input is returned unchanged (A-D-017).
    pub fn validate_input(&self, data: &Value, schema: &Value) -> Result<Value, ModuleError> {
        self.validate_and_coerce(data, schema)
    }

    /// Validate outputs against a schema, returning the validated/coerced value
    /// on success or a [`ModuleError`] on failure. Mirror of [`Self::validate_input`]
    /// for the executor's output-validation step (A-D-017).
    pub fn validate_output(&self, data: &Value, schema: &Value) -> Result<Value, ModuleError> {
        self.validate_and_coerce(data, schema)
    }

    /// Coerce (if enabled), validate, and return the resulting value, or a
    /// `ModuleError` on validation failure.
    fn validate_and_coerce(&self, data: &Value, schema: &Value) -> Result<Value, ModuleError> {
        let candidate = if self.coerce_types {
            coerce_value(data, schema)
        } else {
            data.clone()
        };
        // Validate the (possibly coerced) candidate with no second coercion pass.
        let detailed = self.validate_detailed_raw(&candidate, schema);
        if detailed.valid {
            return Ok(candidate);
        }
        Err(Self::detailed_to_error(&detailed))
    }

    /// Validate and return `Ok(())` on success, or a `ModuleError` carrying the
    /// mapped apcore [`ErrorCode`] and structured per-failure details.
    pub fn validate_or_error(&self, value: &Value, schema: &Value) -> Result<(), ModuleError> {
        let detailed = self.validate_detailed(value, schema);
        if detailed.valid {
            return Ok(());
        }
        Err(Self::detailed_to_error(&detailed))
    }

    /// Build a `ModuleError` from a failed [`DetailedValidationResult`].
    fn detailed_to_error(detailed: &DetailedValidationResult) -> ModuleError {
        let error_maps: Vec<HashMap<String, String>> = detailed
            .errors
            .iter()
            .map(|detail| {
                let mut m = HashMap::new();
                m.insert("message".to_string(), detail.message.clone());
                m.insert("path".to_string(), detail.path.clone());
                m.insert(
                    "keyword".to_string(),
                    detail
                        .constraint
                        .clone()
                        .unwrap_or_else(|| "schema".to_string()),
                );
                m
            })
            .collect();
        let message = format!(
            "Schema validation failed with {} error(s)",
            detailed.errors.len()
        );
        let mut err = SchemaValidationError::new(message, error_maps).to_module_error();
        if let Some(code) = detailed.error_code {
            err.code = code;
        }
        err
    }

    fn compile_cached(&self, schema: &Value) -> Result<Arc<Validator>, String> {
        let digest = content_hash(schema);

        if let Some(v) = self.cache.lock().get(&digest) {
            return Ok(Arc::clone(v));
        }

        let arc = Arc::new(build_validator(schema)?);

        // Another thread may have populated the entry while we were compiling;
        // both Arcs point to equivalent compiled validators, so overwriting is harmless.
        self.cache.lock().insert(digest, Arc::clone(&arc));
        Ok(arc)
    }

    /// Clear the internal compile cache. Useful for tests or long-running services
    /// that want to reclaim memory after schemas churn.
    pub fn clear_cache(&self) {
        self.cache.lock().clear();
    }

    /// Number of distinct schemas currently held in the compile cache.
    #[must_use]
    pub fn cache_len(&self) -> usize {
        self.cache.lock().len()
    }
}

/// Compile `schema` into a [`Validator`], letting the document's own top-level
/// `$schema` declaration select the draft while `format` stays an annotation.
///
/// Two normalisations happen before the build:
///
/// 1. **Nested `$schema` declarations are dropped**; the top-level one is kept.
///    `jsonschema` 0.28 treats a subtree that redeclares `$schema` as an
///    embedded resource compiled under *its own* meta-schema. Mixing a draft-07
///    subtree into a 2020-12 root that way produced a subtree validator which
///    accepted everything — the `$defs` entry was silently unchecked. Removing
///    the inner declarations keeps one draft authoritative for the whole
///    document. Only *string* values are stripped, so a schema that describes a
///    property literally named `$schema` (whose value is a subschema object)
///    survives untouched.
/// 2. **`format` assertion is disabled for every draft.** Draft-07 and earlier
///    make `format` an assertion, so a module schema declaring draft-07 would
///    hard-fail on an unsatisfied `format` while a 2020-12 sibling would not.
///    Draft 2020-12 §7.2.1 puts `format` in the format-annotation vocabulary —
///    it MUST NOT fail validation. apcore enforces the formats it recognises at
///    SHOULD level instead (see [`format_warnings`]), matching apcore-python and
///    apcore-typescript.
///
/// The draft is deliberately **not** pinned. Pinning 2020-12 rejected legal
/// draft-07 contracts outright (tuple-form `items`, `"exclusiveMinimum": true`,
/// …) at compile time; auto-detection keeps each draft's structural keywords
/// meaningful while normalisation 2 removes the only cross-draft divergence
/// apcore cares about.
pub(crate) fn build_validator(schema: &Value) -> Result<Validator, String> {
    let normalised = strip_nested_schema(schema, true);
    jsonschema::options()
        .should_validate_formats(false)
        .build(&normalised)
        .map_err(|e| e.to_string())
}

/// Deep-copy `value`, removing every nested `"$schema": "<uri>"` entry.
///
/// `top` marks the root object, whose declaration is preserved so the document's
/// own draft is still detected. Non-string `$schema` values are kept: they can
/// only be a property *named* `$schema`, never a meta-schema declaration.
fn strip_nested_schema(value: &Value, top: bool) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, child) in map {
                if key == "$schema" && !top && child.is_string() {
                    continue;
                }
                out.insert(key.clone(), strip_nested_schema(child, false));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| strip_nested_schema(item, false))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Recursively coerce `value` toward the scalar types declared by `schema`.
///
/// The accepted set is normative: TYPE_MAPPING §11 "What the knob coerces, when
/// it exists" (spec v1.12.0) makes offering this knob a MAY but its behaviour a
/// MUST, so apcore-python, apcore-typescript and apcore-rust all coerce exactly
/// this and nothing more. The boolean row in particular is NARROWER than
/// pydantic's lax mode (apcore#95).
///
/// Coercions applied (and ONLY these):
/// - string → integer: `"42"` → `42`, `"42.0"` → `42` (trailing whitespace
///   trimmed); rejected (left unchanged) if not an integral numeric string.
/// - string → number: `"3.14"` → `3.14`.
/// - string → boolean: exactly `"true"` / `"false"`, case-sensitive
///   (TYPE_MAPPING §11; narrowed from a twelve-spelling dialect by apcore#95).
/// - integer → number: widening is a no-op for serde_json (numbers are unified),
///   so no transformation is needed.
///
/// NOT applied: number → string, boolean → string, non-integral float →
/// integer, number → boolean, boolean → number, anything → string.
///
/// Unrecognized / non-coercible values are returned unchanged so the downstream
/// jsonschema validator produces the canonical rejection. Recurses into object
/// `properties` and array `items` per the schema shape; `oneOf`/`anyOf`/`allOf`
/// branches are left untouched (the raw validator handles unions).
fn coerce_value(value: &Value, schema: &Value) -> Value {
    let Some(schema_obj) = schema.as_object() else {
        return value.clone();
    };

    // Determine the declared scalar type(s). `type` may be a string or an array.
    let declared_types: Vec<&str> = match schema_obj.get("type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(arr)) => arr.iter().filter_map(|t| t.as_str()).collect(),
        _ => Vec::new(),
    };

    // Object: recurse into declared properties.
    if declared_types.contains(&"object") {
        if let Some(map) = value.as_object() {
            let props = schema_obj.get("properties").and_then(|p| p.as_object());
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                match props.and_then(|p| p.get(k)) {
                    Some(prop_schema) => out.insert(k.clone(), coerce_value(v, prop_schema)),
                    None => out.insert(k.clone(), v.clone()),
                };
            }
            return Value::Object(out);
        }
        return value.clone();
    }

    // Array: recurse into items.
    if declared_types.contains(&"array") {
        if let (Some(arr), Some(items_schema)) = (value.as_array(), schema_obj.get("items")) {
            let coerced: Vec<Value> = arr
                .iter()
                .map(|item| coerce_value(item, items_schema))
                .collect();
            return Value::Array(coerced);
        }
        return value.clone();
    }

    // Scalar coercion: only attempt when the value is a string and the schema
    // declares a numeric/boolean target — TYPE_MAPPING §11 makes coercion
    // string-sourced only. If the value already satisfies one of the declared
    // types, leave it.
    if let Value::String(s) = value {
        // boolean target
        if declared_types.contains(&"boolean") {
            if let Some(b) = coerce_str_to_bool(s) {
                return Value::Bool(b);
            }
        }
        // integer target
        if declared_types.contains(&"integer") {
            if let Some(n) = coerce_str_to_integer(s) {
                return Value::Number(n.into());
            }
        }
        // number target (float)
        if declared_types.contains(&"number") {
            if let Some(f) = coerce_str_to_number(s) {
                if let Some(num) = serde_json::Number::from_f64(f) {
                    return Value::Number(num);
                }
            }
        }
    }

    value.clone()
}

/// Coerce a string to an integer iff it represents an integral numeric value.
///
/// TYPE_MAPPING §11's integer row: a string whose entire content parses as an
/// integer. `"42"` → 42, `"-7"` → -7; the integral-float and surrounding-
/// whitespace tolerances (`"42.0"` → 42, `" 42 "` → 42) are shared with
/// apcore-python and apcore-typescript. `"3.14"` is a MUST-reject — the
/// declared type bounds the coercion, not the string's parseability — as are
/// `"abc"` and `""`.
fn coerce_str_to_integer(s: &str) -> Option<i64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(i) = trimmed.parse::<i64>() {
        return Some(i);
    }
    // Accept integral float strings like "42.0". The range/`fract` guards make
    // the cast exact for the values we accept; precision loss only affects the
    // bounds comparison (acceptable — out-of-range values are rejected anyway).
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    if let Ok(f) = trimmed.parse::<f64>() {
        if f.is_finite() && f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
            return Some(f as i64);
        }
    }
    None
}

/// Coerce a string to a float. Mirrors pydantic: `"3.14"` → 3.14, `" 42 "` → 42.0.
fn coerce_str_to_number(s: &str) -> Option<f64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok().filter(|f| f.is_finite())
}

/// Coerce a string to a boolean: exactly `"true"` and `"false"`, case-sensitive.
///
/// NARROWED (apcore#95, TYPE_MAPPING §11 "What the knob coerces, when it
/// exists", normative as of spec v1.12.0). This used to accept a twelve-spelling
/// case-insensitive dialect — `true/yes/on/y/t/1` and their negatives, lowercased
/// first — that no document described; apcore-typescript ported it verbatim
/// during apcore#93, so this function was the dialect's origin. §11 caps the set
/// at JSON's own two boolean literals: those are the same value written as text,
/// while `"yes"`, `"on"`, `"y"`, `"t"`, `"1"`, `"0"` are shell and INI
/// conventions belonging to whatever parses `argv`.
///
/// `"0"` → `false` is the sharpest removal: R5 makes the *number* `0` a
/// MUST-reject for `boolean` at the module-invocation boundary, so accepting the
/// string `"0"` here put two paths of this SDK on opposite sides of one value.
///
/// Case-sensitive because JSON's boolean literals are lowercase: `"True"` is not
/// one of them.
fn coerce_str_to_bool(s: &str) -> Option<bool> {
    match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Map a list of raw validator errors to a single apcore [`ErrorCode`].
///
/// The first error decides: top-level `oneOf` ambiguity outranks plain failures
/// because it's a stricter classification (the input was almost-valid).
fn map_error_code(errors: &[jsonschema::ValidationError<'_>]) -> ErrorCode {
    for error in errors {
        match &error.kind {
            ValidationErrorKind::OneOfMultipleValid => return ErrorCode::SchemaUnionAmbiguous,
            ValidationErrorKind::OneOfNotValid | ValidationErrorKind::AnyOf => {
                return ErrorCode::SchemaUnionNoMatch;
            }
            _ => {}
        }
    }
    ErrorCode::SchemaValidationError
}

/// Build a structured [`ValidationErrorDetail`] from a single raw validator error.
///
/// `message` preserves the legacy substring-friendly text (see
/// [`format_error_message`]); `path` is the dot/bracket instance path; and
/// `constraint` is the violated JSON Schema keyword when identifiable.
fn build_error_detail(error: &jsonschema::ValidationError<'_>) -> ValidationErrorDetail {
    ValidationErrorDetail {
        path: error.instance_path.to_string(),
        message: format_error_message(error),
        constraint: constraint_name(&error.kind),
        expected: None,
        actual: None,
    }
}

/// Map a raw validator error kind to the JSON Schema keyword it violated.
pub(crate) fn constraint_name(kind: &ValidationErrorKind) -> Option<String> {
    let name = match kind {
        ValidationErrorKind::Required { .. } => "required",
        ValidationErrorKind::AdditionalProperties { .. } => "additionalProperties",
        ValidationErrorKind::Type { .. } => "type",
        ValidationErrorKind::Pattern { .. } => "pattern",
        ValidationErrorKind::Enum { .. } => "enum",
        ValidationErrorKind::Constant { .. } => "const",
        ValidationErrorKind::MinLength { .. } => "minLength",
        ValidationErrorKind::MaxLength { .. } => "maxLength",
        ValidationErrorKind::Minimum { .. } => "minimum",
        ValidationErrorKind::Maximum { .. } => "maximum",
        ValidationErrorKind::ExclusiveMinimum { .. } => "exclusiveMinimum",
        ValidationErrorKind::ExclusiveMaximum { .. } => "exclusiveMaximum",
        ValidationErrorKind::OneOfMultipleValid | ValidationErrorKind::OneOfNotValid => "oneOf",
        ValidationErrorKind::AnyOf => "anyOf",
        ValidationErrorKind::Not { .. } => "not",
        _ => return None,
    };
    Some(name.to_string())
}

/// Render a single validator error as the legacy substring-friendly message format.
///
/// The existing test suite asserts that error strings contain phrases like
/// "expected type", "missing required field", "additional property not allowed",
/// dot-separated paths (`address.city`), and bracketed array indices (`[1]`).
/// We keep that contract while delegating actual checking to the jsonschema crate.
fn format_error_message(error: &jsonschema::ValidationError<'_>) -> String {
    let path = format_instance_path(error.instance_path.as_str());
    let display_path = if path.is_empty() {
        "<root>".to_string()
    } else {
        path
    };

    match &error.kind {
        ValidationErrorKind::Required { property } => {
            let field = property.as_str().unwrap_or("?");
            let scoped = if display_path == "<root>" {
                field.to_string()
            } else {
                format!("{display_path}.{field}")
            };
            format!("{scoped}: missing required field")
        }
        ValidationErrorKind::AdditionalProperties { unexpected } => {
            let mut msgs = Vec::with_capacity(unexpected.len());
            for key in unexpected {
                let scoped = if display_path == "<root>" {
                    key.clone()
                } else {
                    format!("{display_path}.{key}")
                };
                msgs.push(format!("{scoped}: additional property not allowed"));
            }
            // The validator emits one error per group of unexpected keys; collapse
            // multi-key groups into one comma-separated message so the wrapper
            // still produces one string per error.
            msgs.join("; ")
        }
        ValidationErrorKind::Type { kind } => {
            format!(
                "{display_path}: expected type {kind:?}, got {}",
                error.instance
            )
        }
        ValidationErrorKind::Pattern { pattern } => {
            format!("{display_path}: value does not match pattern {pattern:?}")
        }
        ValidationErrorKind::Enum { .. } => {
            format!(
                "{display_path}: value {} is not one of the allowed enum values",
                error.instance
            )
        }
        ValidationErrorKind::Constant { expected_value } => {
            format!(
                "{display_path}: expected const {expected_value}, got {}",
                error.instance
            )
        }
        ValidationErrorKind::MinLength { limit } => {
            format!("{display_path}: minLength {limit} not satisfied")
        }
        ValidationErrorKind::MaxLength { limit } => {
            format!("{display_path}: maxLength {limit} exceeded")
        }
        ValidationErrorKind::Minimum { limit } => {
            format!("{display_path}: minimum {limit} not satisfied")
        }
        ValidationErrorKind::Maximum { limit } => {
            format!("{display_path}: maximum {limit} exceeded")
        }
        ValidationErrorKind::ExclusiveMinimum { limit } => {
            format!("{display_path}: exclusiveMinimum {limit} not satisfied")
        }
        ValidationErrorKind::ExclusiveMaximum { limit } => {
            format!("{display_path}: exclusiveMaximum {limit} exceeded")
        }
        ValidationErrorKind::OneOfMultipleValid => {
            format!("{display_path}: oneOf — input matched more than one branch")
        }
        ValidationErrorKind::OneOfNotValid => {
            format!("{display_path}: oneOf — input matched no branch")
        }
        ValidationErrorKind::AnyOf => {
            format!("{display_path}: anyOf — input matched no branch")
        }
        ValidationErrorKind::Not { schema: _ } => {
            format!("{display_path}: not — input matched the negated schema")
        }
        _ => format!("{display_path}: {error}"),
    }
}

/// Convert the validator's JSON Pointer (`/address/city`, `/items/1`) into the
/// dot-and-bracket form the existing tests expect (`address.city`, `[1]`).
fn format_instance_path(pointer: &str) -> String {
    use std::fmt::Write as _;

    if pointer.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for segment in pointer.split('/').filter(|s| !s.is_empty()) {
        if let Ok(idx) = segment.parse::<usize>() {
            // INVARIANT: writing to a String never fails.
            let _ = write!(out, "[{idx}]");
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(segment);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validator_compile_cache_reuses_compiled_schema() {
        let v = SchemaValidator::new();
        let schema_a = json!({ "type": "object", "required": ["x"] });
        let schema_b = json!({ "required": ["x"], "type": "object" }); // same content, different key order

        let _ = v.validate(&json!({ "x": 1 }), &schema_a);
        let _ = v.validate(&json!({ "x": 1 }), &schema_b);

        // Two byte-equivalent schemas hash to the same digest, so the cache
        // contains exactly one compiled validator.
        assert_eq!(v.cache_len(), 1);
    }

    #[test]
    fn test_validator_one_of_ambiguous_returns_dedicated_error_code() {
        let v = SchemaValidator::new();
        let schema = json!({
            "oneOf": [
                { "type": "object", "properties": { "value": { "type": "integer" } }, "required": ["value"] },
                { "type": "object", "properties": { "value": { "type": "number" } }, "required": ["value"] }
            ]
        });
        let detailed = v.validate_detailed(&json!({ "value": 42 }), &schema);
        assert!(!detailed.valid);
        assert_eq!(detailed.error_code, Some(ErrorCode::SchemaUnionAmbiguous));
    }

    #[test]
    fn test_validator_one_of_no_match_returns_no_match_code() {
        let v = SchemaValidator::new();
        let schema = json!({
            "oneOf": [
                { "type": "object", "properties": { "kind": { "const": "circle" } }, "required": ["kind"] },
                { "type": "object", "properties": { "kind": { "const": "rect" } }, "required": ["kind"] }
            ]
        });
        let detailed = v.validate_detailed(&json!({ "kind": "pentagon" }), &schema);
        assert!(!detailed.valid);
        assert_eq!(detailed.error_code, Some(ErrorCode::SchemaUnionNoMatch));
    }
}
