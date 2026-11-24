//! Cross-language conformance test for Algorithm A23 `to_strict_schema()`
//! (PROTOCOL_SPEC §4.16 / ALGORITHMS A23).
//!
//! Consumes the canonical `schema_strict_conversion.json` fixture shipped by
//! the `apcore` spec repo (sibling directory or `CONFORMANCE_SPEC_REPO`).
//!
//! DRIVER CONTRACT: this file MUST drive [`apcore::to_strict_schema`] — the A23
//! entry point — not the exporter and not the binding wrapper. A23 is the
//! shared deterministic surface; the three SDKs must emit the same strict
//! schema for the same input.

use serde_json::Value;

use apcore::to_strict_schema;

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

#[test]
fn schema_strict_conversion_fixture_parity() {
    let fixture = load_fixture("schema_strict_conversion");
    let cases = fixture["test_cases"].as_array().expect("test_cases array");
    assert!(!cases.is_empty(), "fixture has no test cases");

    let mut ids: Vec<&str> = Vec::new();
    for case in cases {
        let id = case["id"].as_str().expect("case id");
        ids.push(id);
        let schema = &case["schema"];
        let expected = &case["expected"];

        let before = schema.clone();
        let got = to_strict_schema(schema);

        assert_eq!(
            &got,
            expected,
            "[{id}] strict-schema mismatch.\n  description: {}\n  input: {schema}",
            case["description"].as_str().unwrap_or("(none)")
        );
        if let Some(examples) = case["validation_cases"].as_array() {
            let validator = jsonschema::validator_for(&got).expect("valid strict schema");
            for example in examples {
                assert_eq!(
                    validator.is_valid(&example["data"]),
                    example["valid"].as_bool().expect("boolean validity"),
                    "[{id}] unexpected validation result for {}",
                    example["data"]
                );
            }
        }
        // A23 MUST deep-copy — the caller's schema is never mutated.
        assert_eq!(&before, schema, "[{id}] to_strict_schema mutated its input");
    }

    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "duplicate case ids in fixture");
}
