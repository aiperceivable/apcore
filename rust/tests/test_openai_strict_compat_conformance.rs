//! Cross-language conformance test for OpenAI structured-outputs strict-mode
//! compatibility detection (protocol-spec §5.12.5).
//!
//! Consumes the canonical `openai_strict_compat.json` fixture shipped by the
//! `apcore` spec repo (sibling directory or `CONFORMANCE_SPEC_REPO`).
//!
//! DRIVER CONTRACT: this file MUST drive
//! [`apcore::detect_openai_strict_incompatibilities`] — the detector that backs
//! `ErrorCode::BindingStrictSchemaIncompatible` on the `auto_schema: strict`
//! binding path. The detector is the shared deterministic surface across the
//! three SDKs; the binding wrapper around it differs (this SDK performs no
//! runtime type inference, so its strict check runs inside
//! `BindingLoader::register_into_with_typed_handlers`). Feature lists are
//! compared for exact equality INCLUDING ORDER: byte-identical output across
//! SDKs is the point of the fixture.

use serde_json::Value;

use apcore::detect_openai_strict_incompatibilities;

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

#[test]
fn openai_strict_compat_fixture_parity() {
    let fixture = load_fixture("openai_strict_compat");
    let cases = fixture["test_cases"].as_array().expect("test_cases array");
    assert!(!cases.is_empty(), "fixture has no test cases");

    let mut ids: Vec<&str> = Vec::new();
    for case in cases {
        let id = case["id"].as_str().expect("case id");
        ids.push(id);
        let schema = &case["schema"];
        let expected: Vec<String> = case["expected_features"]
            .as_array()
            .expect("expected_features array")
            .iter()
            .map(|v| v.as_str().expect("feature string").to_string())
            .collect();

        let before = schema.clone();
        let got = detect_openai_strict_incompatibilities(schema);

        assert_eq!(
            got,
            expected,
            "[{id}] feature-list mismatch.\n  description: {}\n  schema: {}",
            case["description"].as_str().unwrap_or("(none)"),
            schema
        );
        // The detector reports; it must never rewrite (no oneOf -> anyOf downgrade).
        assert_eq!(&before, schema, "[{id}] detector mutated the input schema");
    }

    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "duplicate case ids in fixture");
}

#[test]
fn fixture_records_keyword_set_provenance() {
    // The supported set moves; the fixture must always say what it was pinned to.
    let fixture = load_fixture("openai_strict_compat");
    let source = &fixture["keyword_set_source"];
    assert!(source["url"]
        .as_str()
        .expect("url")
        .contains("structured-outputs"));
    assert!(source["verified"].as_str().expect("verified").len() >= 8);
}
