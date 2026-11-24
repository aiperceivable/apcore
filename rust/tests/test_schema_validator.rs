//! Tests for SchemaValidator — JSON Schema validation of values.

use apcore::schema::SchemaValidator;
use serde_json::json;

// ---------------------------------------------------------------------------
// Type validation
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_valid_string() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string" });
    let result = v.validate(&json!("hello"), &schema);
    assert!(result.valid);
    assert!(result.errors.is_empty());
}

#[test]
fn test_schema_validator_invalid_type_string_expected() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string" });
    let result = v.validate(&json!(42), &schema);
    assert!(!result.valid);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].message.contains("expected type"));
}

#[test]
fn test_schema_validator_valid_integer() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "integer" });
    let result = v.validate(&json!(42), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_valid_number_accepts_float() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "number" });
    let result = v.validate(&json!(1.5_f64), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_valid_number_accepts_integer() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "number" });
    let result = v.validate(&json!(42), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_valid_boolean() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "boolean" });
    let result = v.validate(&json!(true), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_valid_null() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "null" });
    let result = v.validate(&json!(null), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_valid_object() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "object" });
    let result = v.validate(&json!({}), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_valid_array() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "array" });
    let result = v.validate(&json!([1, 2, 3]), &schema);
    assert!(result.valid);
}

// ---------------------------------------------------------------------------
// Union type (array of types)
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_union_type_matches_first() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": ["string", "null"] });
    let result = v.validate(&json!("hello"), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_union_type_matches_second() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": ["string", "null"] });
    let result = v.validate(&json!(null), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_union_type_no_match() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": ["string", "null"] });
    let result = v.validate(&json!(42), &schema);
    assert!(!result.valid);
}

// ---------------------------------------------------------------------------
// Enum validation
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_enum_valid() {
    let v = SchemaValidator::new();
    let schema = json!({ "enum": ["red", "green", "blue"] });
    let result = v.validate(&json!("green"), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_enum_invalid() {
    let v = SchemaValidator::new();
    let schema = json!({ "enum": ["red", "green", "blue"] });
    let result = v.validate(&json!("yellow"), &schema);
    assert!(!result.valid);
    assert!(result.errors[0].message.contains("enum"));
}

// ---------------------------------------------------------------------------
// Required fields
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_required_field_present() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": { "type": "string" }
        }
    });
    let result = v.validate(&json!({ "name": "Alice" }), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_required_field_missing() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": { "type": "string" }
        }
    });
    let result = v.validate(&json!({}), &schema);
    assert!(!result.valid);
    assert!(result.errors[0].message.contains("missing required field"));
}

#[test]
fn test_schema_validator_multiple_required_fields_missing() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "required": ["name", "age"],
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
        }
    });
    let result = v.validate(&json!({}), &schema);
    assert!(!result.valid);
    assert_eq!(result.errors.len(), 2);
}

// ---------------------------------------------------------------------------
// Nested object validation
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_nested_object_valid() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "properties": {
            "address": {
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"]
            }
        }
    });
    let result = v.validate(&json!({ "address": { "city": "NYC" } }), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_nested_object_invalid_type() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "properties": {
            "address": {
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                }
            }
        }
    });
    let result = v.validate(&json!({ "address": { "city": 42 } }), &schema);
    assert!(!result.valid);
    assert!(result.errors[0].message.contains("address.city"));
}

// ---------------------------------------------------------------------------
// additionalProperties: false
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_additional_properties_false_rejects_extra() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        },
        "additionalProperties": false
    });
    let result = v.validate(&json!({ "name": "Alice", "age": 30 }), &schema);
    assert!(!result.valid);
    assert!(result.errors[0]
        .message
        .contains("additional property not allowed"));
}

#[test]
fn test_schema_validator_additional_properties_false_allows_known() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        },
        "additionalProperties": false
    });
    let result = v.validate(&json!({ "name": "Alice" }), &schema);
    assert!(result.valid);
}

// ---------------------------------------------------------------------------
// Array items validation
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_array_items_valid() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "array",
        "items": { "type": "string" }
    });
    let result = v.validate(&json!(["a", "b", "c"]), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_array_items_invalid_element() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "array",
        "items": { "type": "string" }
    });
    let result = v.validate(&json!(["a", 42, "c"]), &schema);
    assert!(!result.valid);
    assert!(result.errors[0].message.contains("[1]"));
}

#[test]
fn test_schema_validator_empty_array_valid() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "array",
        "items": { "type": "string" }
    });
    let result = v.validate(&json!([]), &schema);
    assert!(result.valid);
}

// ---------------------------------------------------------------------------
// Pattern validation
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_pattern_matches() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string", "pattern": "^[a-z]+$" });
    let result = v.validate(&json!("hello"), &schema);
    assert!(result.valid);
}

#[test]
fn test_schema_validator_pattern_no_match() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string", "pattern": "^[a-z]+$" });
    let result = v.validate(&json!("Hello123"), &schema);
    assert!(!result.valid);
    assert!(result.errors[0].message.contains("pattern"));
}

#[test]
fn test_schema_validator_pattern_not_applied_to_non_string() {
    let v = SchemaValidator::new();
    // pattern only fires when the value is a string
    let schema = json!({ "pattern": "^[a-z]+$" });
    let result = v.validate(&json!(42), &schema);
    assert!(result.valid);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_empty_schema_accepts_anything() {
    let v = SchemaValidator::new();
    let schema = json!({});
    assert!(v.validate(&json!("hello"), &schema).valid);
    assert!(v.validate(&json!(42), &schema).valid);
    assert!(v.validate(&json!(null), &schema).valid);
    assert!(v.validate(&json!([1, 2]), &schema).valid);
}

#[test]
fn test_schema_validator_boolean_schema_true_accepts_anything() {
    let v = SchemaValidator::new();
    // non-object schema => permissive
    let schema = json!(true);
    assert!(v.validate(&json!("hello"), &schema).valid);
}

#[test]
fn test_schema_validator_warnings_always_empty() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string" });
    let result = v.validate(&json!("ok"), &schema);
    assert!(result.warnings.is_empty());
}

// ---------------------------------------------------------------------------
// validate_or_error
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_validate_or_error_ok() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string" });
    assert!(v.validate_or_error(&json!("hello"), &schema).is_ok());
}

#[test]
fn test_schema_validator_validate_or_error_returns_module_error() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string" });
    let err = v.validate_or_error(&json!(42), &schema).unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaValidationError);
    assert!(err.message.contains("validation failed"));
    assert!(err.details.contains_key("errors"));
}

// ---------------------------------------------------------------------------
// D11-010: validate_input / validate_output cross-language parity
// ---------------------------------------------------------------------------
//
// Python (`validator.py:69`) exposes `validate_input(data, model)` and
// `validate_output(data, model)` returning the validated dict, raising
// SchemaValidationError on failure. TypeScript (`validator.ts:78`) exposes
// `validateInput` / `validateOutput` with the same role. Rust previously
// only exposed `validate`, `validate_detailed`, `validate_or_error` — user
// code calling the SDK validator directly could not port between languages.

#[test]
fn test_validate_input_returns_data_on_success() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string" });
    let data = json!("hello");
    let returned = v.validate_input(&data, &schema).expect("valid input");
    assert_eq!(
        returned, data,
        "validate_input must return the input on success"
    );
}

#[test]
fn test_validate_input_raises_on_failure() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "string" });
    let err = v.validate_input(&json!(42), &schema).unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaValidationError);
}

#[test]
fn test_validate_output_returns_data_on_success() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "object", "required": ["ok"], "properties": { "ok": { "type": "boolean" } } });
    let data = json!({"ok": true});
    let returned = v.validate_output(&data, &schema).expect("valid output");
    assert_eq!(returned, data);
}

#[test]
fn test_validate_output_raises_on_failure() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "object", "required": ["ok"] });
    let err = v.validate_output(&json!({}), &schema).unwrap_err();
    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaValidationError);
}

// ---------------------------------------------------------------------------
// Default impl
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_default() {
    let v = SchemaValidator::default();
    let schema = json!({ "type": "string" });
    assert!(v.validate(&json!("ok"), &schema).valid);
}

// ---------------------------------------------------------------------------
// A-D-005 / A-D-006: type-coercion engine, now OPT-IN. `SchemaValidator::new()`
// performs no coercion so it agrees with the module-invocation boundary
// (`executor::validate_against_schema`, which `builtin_steps.rs` calls) —
// TYPE_MAPPING §17.3. `with_coerce_types(true)` is the library-level opt-in for
// a caller validating its own untyped input.
// ---------------------------------------------------------------------------

#[test]
fn test_string_for_integer_rejected_by_default() {
    // Regression: `new()` used to default coerce_types=true, so this SDK had two
    // validation paths that disagreed — `validate_against_schema` rejected
    // {"a": "42"} while `SchemaValidator::new().validate_detailed` accepted it.
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "properties": { "age": { "type": "integer" } },
        "required": ["age"]
    });
    let result = v.validate(&json!({ "age": "42" }), &schema);
    assert!(
        !result.valid,
        "coerce_types defaults false: \"42\" is a type error for {{type: integer}}"
    );
    assert!(!v.coerce_types(), "new() must not coerce");

    // ...and the module-invocation boundary agrees, which is the whole point.
    assert!(
        apcore::executor::validate_against_schema(&json!({ "age": "42" }), &schema, "Input")
            .is_err(),
        "module boundary must reject the same input"
    );
}

#[test]
fn test_coerce_string_to_integer_accepted_when_enabled() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({
        "type": "object",
        "properties": { "age": { "type": "integer" } },
        "required": ["age"]
    });
    let result = v.validate(&json!({ "age": "42" }), &schema);
    assert!(
        result.valid,
        "with_coerce_types(true): \"42\" coerces to 42"
    );
}

#[test]
fn test_coerce_string_to_integer_rejected_when_disabled() {
    let v = SchemaValidator::with_coerce_types(false);
    let schema = json!({
        "type": "object",
        "properties": { "age": { "type": "integer" } },
        "required": ["age"]
    });
    let result = v.validate(&json!({ "age": "42" }), &schema);
    assert!(
        !result.valid,
        "coerce_types=false: raw jsonschema rejects \"42\" for integer"
    );
}

#[test]
fn test_coerce_non_numeric_string_to_integer_rejected() {
    // "abc" cannot coerce — always invalid (matches Py/TS + fixture).
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({
        "type": "object",
        "properties": { "count": { "type": "integer" } },
        "required": ["count"]
    });
    let result = v.validate(&json!({ "count": "abc" }), &schema);
    assert!(!result.valid, "non-numeric string is never coerced");
}

#[test]
fn test_coerce_string_to_number_float() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({ "type": "object", "properties": { "x": { "type": "number" } } });
    assert!(v.validate(&json!({ "x": "3.14" }), &schema).valid);
}

#[test]
fn test_coerce_string_to_bool() {
    // CORRECTED (apcore#95): this used to assert `"1"` and `"0"` coerce too.
    // TYPE_MAPPING §11 "What the knob coerces, when it exists" (spec v1.12.0)
    // CAPS the accepted set at JSON's own two boolean literals rather than
    // removing the coercion: offering the knob stays a MAY, but an SDK that
    // offers one MUST coerce exactly `"true"`/`"false"` (case-sensitive) and
    // MUST NOT coerce anything else. The old assertions are corrected, not
    // deleted, because the positive half of the contract is unchanged.
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({ "type": "object", "properties": { "flag": { "type": "boolean" } } });
    assert!(v.validate(&json!({ "flag": "true" }), &schema).valid);
    assert!(v.validate(&json!({ "flag": "false" }), &schema).valid);
    // `"1"` / `"0"` are shell and INI spellings, not JSON's. `"0"` is the
    // sharpest of the removed twelve: R5 makes the NUMBER 0 a MUST-reject for
    // `boolean` at the module-invocation boundary, so accepting the string
    // `"0"` here put two paths of this SDK on opposite sides of one value.
    assert!(!v.validate(&json!({ "flag": "1" }), &schema).valid);
    assert!(!v.validate(&json!({ "flag": "0" }), &schema).valid);
}

/// TYPE_MAPPING §11: the boolean row is exactly `"true"` and `"false"`.
/// Every other spelling apcore-rust once accepted MUST now be left alone for
/// the raw jsonschema check to reject (apcore#95).
#[test]
fn test_coerce_rejects_the_twelve_spelling_dialect() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({ "type": "object", "properties": { "flag": { "type": "boolean" } } });
    for spelling in [
        "yes", "no", "on", "off", "y", "n", "t", "f", "1", "0", // the removed ten
        "True", "False", "TRUE", "FALSE", "Yes", "ON", // case-insensitivity removed too
        " true", "true ", "", "maybe",
    ] {
        let result = v.validate(&json!({ "flag": spelling }), &schema);
        assert!(
            !result.valid,
            "{spelling:?} MUST NOT coerce to a boolean (TYPE_MAPPING §11)"
        );
    }
}

/// The coerced VALUE, not just the verdict: an implementation that coerced every
/// non-empty string to `true` would pass the `"true"` case above on its own.
#[test]
fn test_coerce_string_false_yields_false_not_truthy() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({
        "type": "object",
        "properties": { "flag": { "type": "boolean" } },
        "required": ["flag"]
    });
    let coerced = v
        .validate_input(&json!({ "flag": "false" }), &schema)
        .expect("\"false\" coerces");
    assert_eq!(coerced["flag"], json!(false));
    let coerced = v
        .validate_input(&json!({ "flag": "true" }), &schema)
        .expect("\"true\" coerces");
    assert_eq!(coerced["flag"], json!(true));
}

/// TYPE_MAPPING §11, integer row: `"3.14"` MUST NOT be accepted for `integer`
/// even though it parses as a number — the DECLARED type bounds the coercion.
#[test]
fn test_coerce_rejects_fractional_string_for_integer() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({
        "type": "object",
        "properties": { "count": { "type": "integer" } },
        "required": ["count"]
    });
    assert!(!v.validate(&json!({ "count": "3.14" }), &schema).valid);
    assert!(!v.validate(&json!({ "count": "-0.5" }), &schema).valid);
    // ...while the same strings ARE accepted for a declared `number`.
    let number_schema = json!({
        "type": "object",
        "properties": { "count": { "type": "number" } },
        "required": ["count"]
    });
    assert!(
        v.validate(&json!({ "count": "3.14" }), &number_schema)
            .valid
    );
    assert!(
        v.validate(&json!({ "count": "-0.5" }), &number_schema)
            .valid
    );
}

/// Coercion is FROM a string only. A number is never coerced to a boolean —
/// this is R5's rule and it holds on the knob's path too (TYPE_MAPPING §11).
#[test]
fn test_coerce_never_turns_a_number_into_a_boolean() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({ "type": "object", "properties": { "flag": { "type": "boolean" } } });
    assert!(!v.validate(&json!({ "flag": 0 }), &schema).valid);
    assert!(!v.validate(&json!({ "flag": 1 }), &schema).valid);
    // ...and a boolean is never coerced toward a number.
    let int_schema = json!({ "type": "object", "properties": { "n": { "type": "integer" } } });
    assert!(!v.validate(&json!({ "n": true }), &int_schema).valid);
}

#[test]
fn test_coerce_does_not_widen_int_to_string() {
    // pydantic lax mode does NOT coerce int->str; this must stay invalid.
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({ "type": "string" });
    assert!(!v.validate(&json!(42), &schema).valid);
}

#[test]
fn test_coerce_int_to_float_widening() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({ "type": "object", "properties": { "x": { "type": "number" } } });
    // integer already valid for number; widening is a no-op but must stay valid.
    assert!(v.validate(&json!({ "x": 42 }), &schema).valid);
}

#[test]
fn test_coerce_recurses_into_nested_objects_and_arrays() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({
        "type": "object",
        "properties": {
            "nested": {
                "type": "object",
                "properties": { "n": { "type": "integer" } }
            },
            "items": {
                "type": "array",
                "items": { "type": "integer" }
            }
        }
    });
    let result = v.validate(
        &json!({ "nested": { "n": "7" }, "items": ["1", "2"] }),
        &schema,
    );
    assert!(
        result.valid,
        "coercion must recurse into nested objects and array items"
    );
}

// A-D-017: validate_input/validate_output must RETURN the coerced value.
#[test]
fn test_validate_input_returns_coerced_value() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({
        "type": "object",
        "properties": { "age": { "type": "integer" } },
        "required": ["age"]
    });
    let returned = v
        .validate_input(&json!({ "age": "42" }), &schema)
        .expect("valid after coercion");
    assert_eq!(
        returned,
        json!({ "age": 42 }),
        "validate_input must return the coerced value, not the raw input"
    );
}

#[test]
fn test_validate_output_returns_coerced_value() {
    let v = SchemaValidator::with_coerce_types(true);
    let schema = json!({
        "type": "object",
        "properties": { "n": { "type": "number" } }
    });
    let returned = v
        .validate_output(&json!({ "n": "2.5" }), &schema)
        .expect("valid after coercion");
    assert_eq!(returned, json!({ "n": 2.5 }));
}

#[test]
fn test_validate_input_no_coercion_returns_raw_when_disabled() {
    let v = SchemaValidator::with_coerce_types(false);
    let schema = json!({ "type": "string" });
    let returned = v.validate_input(&json!("hello"), &schema).expect("valid");
    assert_eq!(returned, json!("hello"));
}

// ---------------------------------------------------------------------------
// D10-002: structured per-failure error details (path/message/constraint),
// cross-language parity with apcore-python/typescript SchemaValidationResult.
// ---------------------------------------------------------------------------

#[test]
fn test_schema_validator_errors_are_structured_details() {
    let v = SchemaValidator::new();
    let schema = json!({
        "type": "object",
        "properties": { "address": { "type": "object", "properties": { "city": { "type": "integer" } } } }
    });
    let result = v.validate(&json!({ "address": { "city": "not-an-int" } }), &schema);

    assert!(!result.valid);
    assert_eq!(result.errors.len(), 1);

    let detail = &result.errors[0];
    // Structured shape: dedicated `path` and `message` fields (not a flat string).
    assert_eq!(detail.path, "/address/city");
    assert!(detail.message.contains("expected type"));
    assert_eq!(detail.constraint.as_deref(), Some("type"));
}

#[test]
fn test_schema_validator_required_error_detail_has_constraint() {
    let v = SchemaValidator::new();
    let schema = json!({ "type": "object", "required": ["name"] });
    let result = v.validate(&json!({}), &schema);

    assert!(!result.valid);
    let detail = &result.errors[0];
    assert!(detail.message.contains("missing required field"));
    assert_eq!(detail.constraint.as_deref(), Some("required"));
}

// ---------------------------------------------------------------------------
// Draft parity with `executor::validate_against_schema`
//
// `format` belongs to the format-annotation vocabulary in JSON Schema 2020-12
// (§7.2.1): an unsatisfied format MUST NOT fail validation. Draft-07 treats it
// as an assertion, so a validator that let the declared draft decide would
// reach the opposite verdict on the same input. Both entry points compile with
// format assertions disabled instead (parity with apcore-python /
// apcore-typescript, where format is SHOULD-level and surfaces as a warning),
// leaving every other keyword's draft semantics intact.
// ---------------------------------------------------------------------------

#[test]
fn test_executor_validate_draft07_schema_treats_format_as_annotation() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": { "contact": { "type": "string", "format": "email" } },
        "required": ["contact"]
    });
    let value = json!({ "contact": "not-an-email" });

    assert!(
        apcore::executor::validate_against_schema(&value, &schema, "Input").is_ok(),
        "an unsatisfied `format` must not fail validation, even under a draft-07 $schema"
    );
}

#[test]
fn test_executor_validate_and_schema_validator_agree_on_format() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": { "contact": { "type": "string", "format": "email" } },
        "required": ["contact"]
    });
    let value = json!({ "contact": "not-an-email" });

    let validator_verdict = SchemaValidator::new().validate(&value, &schema).valid;
    let executor_verdict =
        apcore::executor::validate_against_schema(&value, &schema, "Input").is_ok();

    assert_eq!(
        validator_verdict, executor_verdict,
        "SchemaValidator and executor::validate_against_schema must reach the same verdict"
    );
    assert!(validator_verdict);
}

#[test]
fn test_executor_validate_still_rejects_real_type_errors_under_draft07() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": { "count": { "type": "integer" } },
        "required": ["count"]
    });
    let err =
        apcore::executor::validate_against_schema(&json!({ "count": "abc" }), &schema, "Input")
            .unwrap_err();

    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaValidationError);
    let errors = err.details.get("errors").expect("details.errors present");
    let arr = errors.as_array().expect("details.errors is an array");
    assert!(!arr.is_empty());
    assert!(arr[0].get("path").is_some());
    assert!(arr[0].get("keyword").is_some());
    assert!(arr[0].get("message").is_some());
}

#[test]
fn test_schema_validator_still_enforces_constraints_under_draft07_schema_keyword() {
    // A declared draft must never turn validation off. Compiling a draft-07
    // document under a pinned Draft 2020-12 made `jsonschema` build an
    // accept-everything validator; honouring the declaration keeps the real
    // constraints (`type`, `required`, …) enforced.
    let v = SchemaValidator::with_coerce_types(false);
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": { "count": { "type": "integer" } },
        "required": ["count"]
    });

    assert!(!v.validate(&json!({ "count": "abc" }), &schema).valid);
    assert!(!v.validate(&json!({}), &schema).valid);
    assert!(v.validate(&json!({ "count": 3 }), &schema).valid);
}

// ---------------------------------------------------------------------------
// Draft handling: the document's own draft is honoured, nested `$schema`
// declarations are not.
//
// Pinning Draft 2020-12 regressed two ways at once: legal draft-07 syntax
// (tuple-form `items`) stopped compiling at all, and a subtree that redeclared
// `$schema` was still compiled as its own embedded resource — an
// accept-everything validator for that subtree.
// ---------------------------------------------------------------------------

#[test]
fn test_draft07_tuple_items_schema_compiles_and_validates_positionally() {
    // Legal draft-07: `items` as an array is the tuple form. Under a pinned
    // Draft 2020-12 meta-schema this is not a valid schema at all, so the
    // compile failed and every input was rejected.
    let v = SchemaValidator::with_coerce_types(false);
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "array",
        "items": [{ "type": "string" }, { "type": "integer" }]
    });

    let ok = v.validate(&json!(["a", 1]), &schema);
    assert!(
        ok.valid,
        "draft-07 tuple form must compile: {:?}",
        ok.errors
    );
    assert!(!v.validate(&json!([1, "a"]), &schema).valid);
}

#[test]
fn test_executor_validate_accepts_draft07_tuple_items_schema() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "array",
        "items": [{ "type": "string" }, { "type": "integer" }]
    });

    assert!(apcore::executor::validate_against_schema(&json!(["a", 1]), &schema, "Input").is_ok());
    assert!(apcore::executor::validate_against_schema(&json!([1, "a"]), &schema, "Input").is_err());
}

#[test]
fn test_nested_schema_declaration_does_not_disable_subtree_validation() {
    // The `$defs` entry redeclares `$schema`. Stripping only the top-level
    // declaration left this subtree compiled as a separate draft-07 resource
    // that accepted everything, so `{"n": "abc"}` slipped through unchecked.
    let v = SchemaValidator::with_coerce_types(false);
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "p": { "$ref": "#/$defs/Inner" } },
        "$defs": {
            "Inner": {
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": { "n": { "type": "integer" } },
                "required": ["n"]
            }
        }
    });

    assert!(!v.validate(&json!({ "p": { "n": "abc" } }), &schema).valid);
    assert!(v.validate(&json!({ "p": { "n": 7 } }), &schema).valid);
}

#[test]
fn test_nested_schema_declaration_with_id_does_not_disable_subtree_validation() {
    // Same hole, spelled the way an embedded resource usually is: `$schema`
    // alongside an `$id`.
    let v = SchemaValidator::with_coerce_types(false);
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "p": { "$ref": "#/$defs/Inner" } },
        "$defs": {
            "Inner": {
                "$id": "https://example.com/inner",
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": { "n": { "type": "integer" } },
                "required": ["n"]
            }
        }
    });

    assert!(!v.validate(&json!({ "p": { "n": "abc" } }), &schema).valid);
}

#[test]
fn test_recursive_self_ref_schema_still_validates() {
    let v = SchemaValidator::with_coerce_types(false);
    let schema = json!({
        "type": "object",
        "properties": { "child": { "$ref": "#" } },
        "additionalProperties": false
    });

    assert!(
        v.validate(&json!({ "child": { "child": {} } }), &schema)
            .valid
    );
    assert!(
        !v.validate(&json!({ "child": { "extra": 1 } }), &schema)
            .valid
    );
}

#[test]
fn test_property_literally_named_schema_survives_normalisation() {
    // Only a *string* `$schema` is a meta-schema declaration. A property named
    // `$schema` carries a subschema object and must keep constraining its key.
    let v = SchemaValidator::with_coerce_types(false);
    let schema = json!({
        "type": "object",
        "properties": { "$schema": { "type": "string" } },
        "required": ["$schema"],
        "additionalProperties": false
    });

    assert!(v.validate(&json!({ "$schema": "ok" }), &schema).valid);
    assert!(!v.validate(&json!({ "$schema": 42 }), &schema).valid);
}

#[test]
fn test_executor_validate_invalid_schema_reports_parse_error_with_details() {
    // The *schema* is broken, not the value. SCHEMA_VALIDATION_ERROR is flagged
    // caller-fixable, which would point the caller at arguments that were never
    // at fault, so this path reports SCHEMA_PARSE_ERROR — the same code
    // `SchemaValidator` uses when a compile fails.
    let schema = json!({ "type": "object", "properties": { "bad": { "type": 42 } } });
    let err = apcore::executor::validate_against_schema(&json!({}), &schema, "Input").unwrap_err();

    assert_eq!(err.code, apcore::errors::ErrorCode::SchemaParseError);
    assert_eq!(apcore::errors::user_fixable_for_code(err.code), None);
    let arr = err
        .details
        .get("errors")
        .expect("details.errors present")
        .as_array()
        .expect("details.errors is an array");
    assert_eq!(arr.len(), 1);
    assert!(arr[0].get("field").is_some());
    assert!(arr[0].get("message").is_some());
    assert!(err.ai_guidance.is_some());
}
