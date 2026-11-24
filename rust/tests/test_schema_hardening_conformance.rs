//! Cross-language conformance tests for Schema System Hardening (Issue #44,
//! PROTOCOL_SPEC §4.15).
//!
//! Each test consumes one of the canonical `schema_hardening_*.json` fixtures
//! shipped by the `apcore` spec repo (sibling directory or `CONFORMANCE_SPEC_REPO`).

use serde_json::{json, Value};

use apcore::errors::ErrorCode;
use apcore::schema::{content_hash, format_warnings, FormatWarning, SchemaValidator};

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

fn parse_expected_code(s: &str) -> ErrorCode {
    match s {
        "SCHEMA_UNION_NO_MATCH" => ErrorCode::SchemaUnionNoMatch,
        "SCHEMA_UNION_AMBIGUOUS" => ErrorCode::SchemaUnionAmbiguous,
        // Canonical wire code is SCHEMA_VALIDATION_ERROR (SCREAMING_SNAKE_CASE
        // of ErrorCode::SchemaValidationError); accept the legacy
        // SCHEMA_VALIDATION_FAILED spelling too for fixture compatibility.
        "SCHEMA_VALIDATION_ERROR" | "SCHEMA_VALIDATION_FAILED" => ErrorCode::SchemaValidationError,
        "SCHEMA_MAX_DEPTH_EXCEEDED" => ErrorCode::SchemaMaxDepthExceeded,
        other => panic!("Unknown error code in fixture: {other}"),
    }
}

#[test]
fn conformance_schema_hardening_union() {
    let fixture = load_fixture("schema_hardening_union");
    let validator = SchemaValidator::new();

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let schema = &tc["schema"];
        let input = &tc["input"];
        let expected_valid = tc["expected"]["valid"].as_bool().unwrap();
        let expected_code = tc["expected"]["error_code"].as_str();

        let result = validator.validate_detailed(input, schema);
        assert_eq!(
            result.valid, expected_valid,
            "FAIL [{id}]: expected valid={expected_valid}, got valid={} errors={:?}",
            result.valid, result.errors
        );

        if !expected_valid {
            let want = expected_code
                .unwrap_or_else(|| panic!("FAIL [{id}]: fixture missing expected.error_code"));
            let got = result
                .error_code
                .unwrap_or_else(|| panic!("FAIL [{id}]: validator returned no error_code"));
            assert_eq!(
                got,
                parse_expected_code(want),
                "FAIL [{id}]: expected error_code={want}, got {got:?}"
            );
        }
    }
}

#[test]
fn conformance_schema_hardening_recursive() {
    let fixture = load_fixture("schema_hardening_recursive");
    let validator = SchemaValidator::new();
    let schema = &fixture["schema"];

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let input = &tc["input"];
        let expected_valid = tc["expected"]["valid"].as_bool().unwrap();
        let expected_code = tc["expected"]["error_code"].as_str();

        let result = validator.validate_detailed(input, schema);
        assert_eq!(
            result.valid, expected_valid,
            "FAIL [{id}]: expected valid={expected_valid}, got valid={} errors={:?}",
            result.valid, result.errors
        );

        if !expected_valid {
            let want = expected_code
                .unwrap_or_else(|| panic!("FAIL [{id}]: fixture missing expected.error_code"));
            let got = result
                .error_code
                .unwrap_or_else(|| panic!("FAIL [{id}]: validator returned no error_code"));
            assert_eq!(
                got,
                parse_expected_code(want),
                "FAIL [{id}]: expected error_code={want}, got {got:?}"
            );
        }
    }
}

#[test]
fn conformance_schema_hardening_constraints() {
    let fixture = load_fixture("schema_hardening_constraints");
    let validator = SchemaValidator::new();

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let schema = &tc["schema"];
        let input = &tc["input"];
        let expected_valid = tc["expected"]["valid"].as_bool().unwrap();
        let expected_code = tc["expected"]["error_code"].as_str();

        let result = validator.validate_detailed(input, schema);
        assert_eq!(
            result.valid, expected_valid,
            "FAIL [{id}]: expected valid={expected_valid}, got valid={} errors={:?}",
            result.valid, result.errors
        );

        if !expected_valid {
            let want = expected_code
                .unwrap_or_else(|| panic!("FAIL [{id}]: fixture missing expected.error_code"));
            let got = result
                .error_code
                .unwrap_or_else(|| panic!("FAIL [{id}]: validator returned no error_code"));
            assert_eq!(
                got,
                parse_expected_code(want),
                "FAIL [{id}]: expected error_code={want}, got {got:?}"
            );
        }
    }
}

#[test]
fn conformance_schema_hardening_formats() {
    let fixture = load_fixture("schema_hardening_formats");
    let validator = SchemaValidator::new();

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let schema = &tc["schema"];
        let input = &tc["input"];
        let expected_valid = tc["expected"]["valid"].as_bool().unwrap();
        let expected_warn = tc["expected"]["warn_logged"].as_bool().unwrap_or(false);

        // Format enforcement is SHOULD-level, so the validator must always accept the
        // input — warnings surface separately via `format_warnings`.
        let result = validator.validate_detailed(input, schema);
        assert_eq!(
            result.valid, expected_valid,
            "FAIL [{id}]: expected valid={expected_valid}, got valid={} errors={:?}",
            result.valid, result.errors
        );

        let warnings = format_warnings(input, schema);
        let warn_logged = !warnings.is_empty();
        assert_eq!(
            warn_logged, expected_warn,
            "FAIL [{id}]: expected warn_logged={expected_warn}, got warn_logged={warn_logged} warnings={warnings:?}"
        );
    }
}

#[test]
fn conformance_schema_hardening_cache() {
    let fixture = load_fixture("schema_hardening_cache");

    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let schemas = tc["schemas"].as_array().unwrap();
        assert_eq!(
            schemas.len(),
            2,
            "FAIL [{id}]: cache fixture must have exactly 2 schemas"
        );
        let expected_same = tc["expected"]["same_hash"].as_bool().unwrap();

        let h1 = content_hash(&schemas[0]);
        let h2 = content_hash(&schemas[1]);
        let same = h1 == h2;
        assert_eq!(
            same, expected_same,
            "FAIL [{id}]: expected same_hash={expected_same}, got h1={h1} h2={h2}"
        );
    }
}

/// A-D-037: compute and REPORT content_hash for tricky schemas (float 1.0,
/// unicode keys/values, large integers beyond 2^53, unsorted nested keys, and
/// a baseline). The digests are printed so they can be compared cross-repo for
/// byte-for-byte canonicalization parity. Also asserts key-order invariance
/// (a reordered clone hashes identically).
#[test]
fn conformance_schema_content_hash_reports_digests() {
    let fixture = load_fixture("schema_content_hash");
    println!("=== A-D-037 content_hash digests (Rust SDK) ===");
    for tc in fixture["test_cases"].as_array().unwrap() {
        let id = tc["id"].as_str().unwrap();
        let schema = &tc["schema"];
        let digest = content_hash(schema);
        assert_eq!(digest.len(), 64, "FAIL [{id}]: digest must be 64 hex chars");
        println!("{id}: {digest}");

        // Key-order invariance: a deep clone whose object keys are reversed
        // must hash identically (serde_json::Value already sorts on
        // canonicalization, so build an equal value and re-hash).
        let reparsed: Value =
            serde_json::from_str(&serde_json::to_string(schema).unwrap()).unwrap();
        assert_eq!(
            content_hash(&reparsed),
            digest,
            "FAIL [{id}]: content_hash must be stable across serialize round-trip"
        );
    }
}

// ---------------------------------------------------------------------------
// Traversal coverage — a `format` annotation must be found wherever it is
// reachable, not only under `properties` / `items`.
// ---------------------------------------------------------------------------

fn formats_of(warnings: &[FormatWarning]) -> Vec<(String, String)> {
    warnings
        .iter()
        .map(|w| (w.path.clone(), w.format.clone()))
        .collect()
}

#[test]
fn format_warnings_descend_into_anyof_branches() {
    let schema = json!({
        "type": "object",
        "properties": {
            "contact": {
                "anyOf": [
                    { "type": "string", "format": "email" },
                    { "type": "null" }
                ]
            }
        }
    });
    let warnings = format_warnings(&json!({ "contact": "not-an-email" }), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/contact".to_string(), "email".to_string())]
    );
}

#[test]
fn format_warnings_anyof_only_reports_the_branch_the_data_satisfies() {
    // Branches that differ ONLY by `format`. apcore compiles every schema with
    // format assertions disabled, so a compiled validator finds both branches
    // equally satisfied and cannot discriminate — the valid email would be
    // reported as a malformed uuid. The branch that comes back clean is what
    // identifies the shape the value was meant for.
    let schema = json!({
        "type": "object",
        "properties": {
            "id": {
                "anyOf": [
                    { "type": "string", "format": "uuid" },
                    { "type": "string", "format": "email" }
                ]
            }
        }
    });
    let warnings = format_warnings(&json!({ "id": "alice@example.com" }), &schema);
    assert!(
        warnings.is_empty(),
        "a valid email must not be reported as a bad uuid: {warnings:?}"
    );

    // …and the mirror case: a valid uuid must not be reported as a bad email.
    let warnings = format_warnings(
        &json!({ "id": "5f0e5b1a-9f5c-4c1e-8a0e-1c2d3e4f5a6b" }),
        &schema,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn format_warnings_anyof_reports_every_branch_when_no_branch_fits() {
    // Guard against over-suppression: when nothing fits, staying silent would
    // hide a real mismatch, so every surviving branch reports.
    let schema = json!({
        "type": "object",
        "properties": {
            "id": {
                "anyOf": [
                    { "type": "string", "format": "uuid" },
                    { "type": "string", "format": "email" }
                ]
            }
        }
    });
    let warnings = format_warnings(&json!({ "id": "neither-of-them" }), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![
            ("/id".to_string(), "uuid".to_string()),
            ("/id".to_string(), "email".to_string()),
        ]
    );
}

#[test]
fn format_warnings_nullable_ref_branch_is_not_silenced() {
    // The canonical "nullable reference" shape. The branch cannot be compiled in
    // isolation (`#/$defs/Email` is unreachable from the branch alone), and
    // treating an undecidable branch as unsatisfied silenced this whole family
    // of schemas — including the branch's own `format`.
    let schema = json!({
        "type": "object",
        "properties": {
            "contact": {
                "anyOf": [
                    { "allOf": [{ "$ref": "#/$defs/Email" }], "format": "email" },
                    { "type": "null" }
                ]
            }
        },
        "$defs": { "Email": { "type": "string" } }
    });

    let warnings = format_warnings(&json!({ "contact": "not-an-email" }), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/contact".to_string(), "email".to_string())]
    );
    assert!(format_warnings(&json!({ "contact": "alice@example.com" }), &schema).is_empty());
}

#[test]
fn format_warnings_undecidable_branch_survives_alongside_a_rejected_one() {
    // Two format-declaring branches: one cannot be compiled, the other compiles
    // and is structurally rejected. "Cannot decide" must not collapse into
    // "does not match", or the only branch that could warn disappears.
    let schema = json!({
        "anyOf": [
            { "allOf": [{ "$ref": "#/$defs/Str" }], "format": "email" },
            { "type": "integer", "format": "int64" }
        ],
        "$defs": { "Str": { "type": "string" } }
    });
    let warnings = format_warnings(&json!("not-an-email"), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/".to_string(), "email".to_string())]
    );
}

#[test]
fn format_warnings_anyof_drops_the_structurally_contradicted_branch() {
    // A discriminated union: only the `user` branch can describe this value, so
    // the `session` branch's uuid annotation must not be reported.
    let schema = json!({
        "anyOf": [
            {
                "type": "object",
                "properties": { "kind": { "const": "user" }, "v": { "format": "email" } }
            },
            {
                "type": "object",
                "properties": { "kind": { "const": "session" }, "v": { "format": "uuid" } }
            }
        ]
    });
    let warnings = format_warnings(&json!({ "kind": "user", "v": "not-an-email" }), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/v".to_string(), "email".to_string())]
    );
}

#[test]
fn format_warnings_descend_into_oneof_branches() {
    let schema = json!({
        "type": "object",
        "properties": {
            "id": {
                "oneOf": [
                    { "type": "string", "format": "uuid" },
                    { "type": "integer" }
                ]
            }
        }
    });
    let warnings = format_warnings(&json!({ "id": "not-a-uuid" }), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/id".to_string(), "uuid".to_string())]
    );
}

#[test]
fn format_warnings_descend_into_all_allof_members() {
    let schema = json!({
        "type": "object",
        "properties": {
            "when": {
                "allOf": [
                    { "type": "string" },
                    { "format": "date-time" }
                ]
            }
        }
    });
    let warnings = format_warnings(&json!({ "when": "yesterday" }), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/when".to_string(), "date-time".to_string())]
    );
}

#[test]
fn format_warnings_descend_into_prefix_items() {
    let schema = json!({
        "type": "array",
        "prefixItems": [
            { "type": "string", "format": "uuid" },
            { "type": "string", "format": "date" }
        ]
    });
    let warnings = format_warnings(&json!(["not-a-uuid", "not-a-date"]), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![
            ("/0".to_string(), "uuid".to_string()),
            ("/1".to_string(), "date".to_string()),
        ]
    );
}

#[test]
fn format_warnings_descend_into_additional_properties_subschema() {
    let schema = json!({
        "type": "object",
        "properties": { "known": { "type": "string" } },
        "additionalProperties": { "type": "string", "format": "email" }
    });
    let warnings = format_warnings(
        &json!({ "known": "plain", "extra": "not-an-email" }),
        &schema,
    );
    assert_eq!(
        formats_of(&warnings),
        vec![("/extra".to_string(), "email".to_string())]
    );
}

#[test]
fn format_warnings_descend_into_pattern_properties_subschema() {
    let schema = json!({
        "type": "object",
        "patternProperties": { "^ts_": { "type": "string", "format": "date-time" } }
    });
    let warnings = format_warnings(&json!({ "ts_created": "yesterday" }), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/ts_created".to_string(), "date-time".to_string())]
    );
}

#[test]
fn format_warnings_pattern_matched_key_is_not_checked_against_additional_properties() {
    // Draft 2020-12 §10.3.2.3: `additionalProperties` applies only to keys that
    // neither `properties` nor `patternProperties` matched. Checking `ts_created`
    // against the `email` subschema too was a spec violation, not a lax reading.
    let schema = json!({
        "type": "object",
        "patternProperties": { "^ts_": { "type": "string", "format": "date-time" } },
        "additionalProperties": { "type": "string", "format": "email" }
    });
    let warnings = format_warnings(&json!({ "ts_created": "2020-01-01T00:00:00Z" }), &schema);
    assert!(
        warnings.is_empty(),
        "a pattern-matched key must not be judged by additionalProperties: {warnings:?}"
    );
}

#[test]
fn format_warnings_additional_properties_still_covers_unmatched_keys() {
    let schema = json!({
        "type": "object",
        "patternProperties": { "^ts_": { "type": "string", "format": "date-time" } },
        "additionalProperties": { "type": "string", "format": "email" }
    });
    let warnings = format_warnings(&json!({ "owner": "not-an-email" }), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/owner".to_string(), "email".to_string())]
    );
}

#[test]
fn format_warnings_prefix_items_boundary_hands_the_tail_to_items() {
    let schema = json!({
        "type": "array",
        "prefixItems": [{ "type": "string", "format": "uuid" }],
        "items": { "type": "string", "format": "email" }
    });
    let warnings = format_warnings(&json!(["not-a-uuid", "nope", "also-nope"]), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![
            ("/0".to_string(), "uuid".to_string()),
            ("/1".to_string(), "email".to_string()),
            ("/2".to_string(), "email".to_string()),
        ]
    );
}

#[test]
fn format_warnings_items_false_contributes_nothing_past_the_prefix() {
    let schema = json!({
        "type": "array",
        "prefixItems": [{ "type": "string", "format": "uuid" }],
        "items": false
    });
    let warnings = format_warnings(&json!(["not-a-uuid", "extra"]), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/0".to_string(), "uuid".to_string())]
    );
}

#[test]
fn format_warnings_prefix_items_longer_than_the_data_is_not_an_error() {
    let schema = json!({
        "type": "array",
        "prefixItems": [
            { "type": "string", "format": "uuid" },
            { "type": "string", "format": "date" }
        ]
    });
    let warnings = format_warnings(&json!(["not-a-uuid"]), &schema);
    assert_eq!(
        formats_of(&warnings),
        vec![("/0".to_string(), "uuid".to_string())]
    );
}

#[test]
fn format_warnings_deduplicate_annotations_reached_through_several_branches() {
    let schema = json!({
        "type": "object",
        "properties": {
            "contact": {
                "anyOf": [
                    { "type": "string", "format": "email" },
                    { "type": "string", "format": "email" }
                ]
            }
        }
    });
    let warnings = format_warnings(&json!({ "contact": "not-an-email" }), &schema);
    assert_eq!(
        warnings.len(),
        1,
        "duplicate annotations must collapse: {warnings:?}"
    );
}

// ---------------------------------------------------------------------------
// Cost guards. Both bounds are an order of magnitude above the measured runtime
// of the linear implementation on a debug build (19 ms and 3 ms respectively),
// so they flag a return to quadratic behaviour without tripping on a slow
// machine.
// ---------------------------------------------------------------------------

#[test]
fn format_warnings_scale_linearly_across_a_large_array() {
    // De-duplication used `Vec::contains` — an O(n) rescan per warning. One
    // warning per element makes the pass quadratic.
    let schema = json!({
        "type": "array",
        "items": { "type": "string", "format": "email" }
    });
    let data = Value::Array(
        (0..16_000)
            .map(|i| Value::String(format!("bad-{i}")))
            .collect(),
    );

    let started = std::time::Instant::now();
    let warnings = format_warnings(&data, &schema);
    let elapsed = started.elapsed();

    assert_eq!(warnings.len(), 16_000);
    assert!(
        elapsed < std::time::Duration::from_millis(400),
        "de-duplication looks quadratic again: 16k warnings took {elapsed:?}"
    );
}

#[test]
fn format_warnings_scale_across_deeply_nested_unions() {
    // Every union level used to compile a `jsonschema::Validator` for its whole
    // inner subtree, and re-derive `declares_format` over it, so the cost grew
    // super-linearly with nesting depth.
    let mut schema = json!({ "type": "string", "format": "email" });
    for _ in 0..120 {
        schema = json!({ "anyOf": [schema, { "type": "string", "format": "uuid" }] });
    }

    let started = std::time::Instant::now();
    let warnings = format_warnings(&json!("not-an-email"), &schema);
    let elapsed = started.elapsed();

    assert!(!warnings.is_empty());
    assert!(
        elapsed < std::time::Duration::from_millis(300),
        "nested unions look super-linear again: depth 120 took {elapsed:?}"
    );
}
