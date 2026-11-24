// Cross-language conformance tests for Multi-Module Discovery (Issue #32).
//
// Fixture source: apcore/conformance/fixtures/multi_module_discovery.json
// Spec reference: apcore/docs/features/multi-module-discovery.md
//                 apcore/PROTOCOL_SPEC.md §2.1.1
//
// The eight fixture cases exercise:
//   single_class_id_unchanged           — single-class identity guarantee
//   two_classes_distinct_ids            — multi-class ID derivation
//   class_name_snake_case_addition      — snake_case: simple PascalCase
//   class_name_snake_case_math_ops      — snake_case: word boundary
//   class_name_snake_case_https_sender  — snake_case: ALLCAPS run
//   conflict_same_segment               — MODULE_ID_CONFLICT error code
//   full_id_grammar_valid               — derived ID matches canonical grammar
//   disabled_by_default                 — multi_class=false → only base_id

#![allow(clippy::missing_panics_doc)]
// `apcore::errors::ModuleError` is intentionally large (rich structured error
// for an SDK); boxing it across the test API would diverge from the library's
// public surface. Mirrors the crate-wide allow in src/lib.rs.
#![allow(clippy::result_large_err)]

use std::path::PathBuf;

use serde_json::Value;

use apcore::errors::ErrorCode;
use apcore::registry::registry::MODULE_ID_PATTERN;
use apcore::registry::{
    class_name_to_segment, derive_module_ids, DiscoveredClass, DiscoveryConfig,
};

// ---------------------------------------------------------------------------
// Fixture loading (mirrors tests/conformance_test.rs discovery)
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

fn fixture_case<'a>(fixture: &'a Value, id: &str) -> &'a Value {
    fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array")
        .iter()
        .find(|c| c["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("fixture case '{id}' not present"))
}

/// Read each class's per-class multi-class marker (D-107).
///
/// The per-class `multi_class` field is the ONLY opt-in. The fixture used to
/// carry a file-level `multi_class_enabled` — the model decision-log D-06
/// withdrew — and this driver briefly accepted it as a fallback while the
/// fixture migrated. That fallback is deliberately gone: tolerating both models
/// is exactly what let three SDKs implement three different ones and stay
/// green, so a driver that still accepts the withdrawn spelling cancels out the
/// discrimination the corrected fixture exists to provide. A class that omits
/// the marker is un-marked, which is the correct reading of "did not opt in".
fn parse_classes(input: &Value) -> Vec<DiscoveredClass> {
    input["classes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    DiscoveredClass::new(
                        c["name"].as_str().unwrap(),
                        c["implements_module"].as_bool().unwrap_or(true),
                    )
                    .with_multi_class(c["multi_class"].as_bool().unwrap_or(false))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn run_derive(case: &Value) -> Result<Vec<String>, apcore::errors::ModuleError> {
    let input = &case["input"];
    let file_path = PathBuf::from(input["file_path"].as_str().unwrap());
    let extensions_root = input["extensions_root"].as_str().unwrap_or("extensions");
    let classes = parse_classes(input);
    // Inert since D-107: passed only because the signature still takes it.
    derive_module_ids(
        &file_path,
        extensions_root,
        &classes,
        &DiscoveryConfig::default(),
    )
}

// ---------------------------------------------------------------------------
// Conformance tests — one per fixture case (10 total)
// ---------------------------------------------------------------------------

#[test]
fn conformance_single_class_id_unchanged() {
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, "single_class_id_unchanged");
    let ids = run_derive(case).expect("expected success");
    let expected: Vec<String> = case["expected"]["module_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, expected, "single-class identity guarantee");
}

#[test]
fn conformance_two_classes_distinct_ids() {
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, "two_classes_distinct_ids");
    let ids = run_derive(case).expect("expected success");
    let expected: Vec<String> = case["expected"]["module_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, expected, "two-class ID derivation");
}

#[test]
fn conformance_class_name_snake_case_addition() {
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, "class_name_snake_case_addition");
    let class_name = case["input"]["class_name"].as_str().unwrap();
    let expected = case["expected"]["class_segment"].as_str().unwrap();
    assert_eq!(class_name_to_segment(class_name), expected);
}

#[test]
fn conformance_class_name_snake_case_math_ops() {
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, "class_name_snake_case_math_ops");
    let class_name = case["input"]["class_name"].as_str().unwrap();
    let expected = case["expected"]["class_segment"].as_str().unwrap();
    assert_eq!(class_name_to_segment(class_name), expected);
}

#[test]
fn conformance_class_name_snake_case_https_sender() {
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, "class_name_snake_case_https_sender");
    let class_name = case["input"]["class_name"].as_str().unwrap();
    let expected = case["expected"]["class_segment"].as_str().unwrap();
    assert_eq!(class_name_to_segment(class_name), expected);
}

#[test]
fn conformance_conflict_same_segment() {
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, "conflict_same_segment");
    let err = run_derive(case).expect_err("expected MODULE_ID_CONFLICT");
    assert_eq!(
        err.code,
        ErrorCode::ModuleIdConflict,
        "expected MODULE_ID_CONFLICT"
    );
    let expected_segment = case["expected"]["error"]["conflicting_segment"]
        .as_str()
        .unwrap();
    let actual_segment = err
        .details
        .get("conflicting_segment")
        .and_then(|v| v.as_str())
        .expect("conflicting_segment must appear in error details");
    assert_eq!(actual_segment, expected_segment);
}

#[test]
fn conformance_full_id_grammar_valid() {
    // CORRECTED (apcore#93): this test dismissed the case's `expected.module_ids`
    // as "illustrative only", on the belief that the fixture declared
    // `executor.math.arithmetic.addition` where the single-class identity
    // guarantee returns the bare `executor.math.arithmetic`. The fixture
    // declares the bare id — the two agree, and have for as long as this
    // comment has been wrong. With the only fixture-derived assertion discarded
    // and `grammar_valid` / `error` never read, nothing in the case could fail:
    // mutating the whole `expected` block left apcore-rust green.
    //
    // All three declared expectations are now asserted. The extra 2-class
    // derivation below stays: it exercises the multi-class path, which this
    // single-class case does not reach.
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, "full_id_grammar_valid");
    let pattern = regex::Regex::new(MODULE_ID_PATTERN).unwrap();

    let outcome = run_derive(case);
    // `expected.error` is null for a case that must succeed. Compared as a
    // value so a fixture that starts declaring an error code stops passing here.
    let observed_error = match &outcome {
        Ok(_) => Value::Null,
        Err(e) => serde_json::to_value(e.code).unwrap(),
    };
    assert_eq!(
        observed_error, case["expected"]["error"],
        "expected.error; derivation returned {outcome:?}"
    );
    let ids = outcome.expect("expected success");

    let expected_ids: Vec<String> = case["expected"]["module_ids"]
        .as_array()
        .expect("expected.module_ids is an array")
        .iter()
        .map(|v| v.as_str().expect("module id is a string").to_string())
        .collect();
    assert_eq!(
        ids, expected_ids,
        "single-class identity guarantee: the derived ids must be the ones the case declares"
    );

    let all_valid = ids.iter().all(|id| pattern.is_match(id));
    assert_eq!(
        Value::Bool(all_valid),
        case["expected"]["grammar_valid"],
        "expected.grammar_valid against canonical_id grammar {MODULE_ID_PATTERN}; ids were \
         {ids:?}"
    );

    // Two-class variant — exercises the multi-class derivation path.
    let file_path = PathBuf::from(case["input"]["file_path"].as_str().unwrap());
    let extensions_root = case["input"]["extensions_root"].as_str().unwrap();
    let two_classes = vec![
        DiscoveredClass::new("Addition", true).with_multi_class(true),
        DiscoveredClass::new("Subtraction", true).with_multi_class(true),
    ];
    let config = DiscoveryConfig::default();
    let multi_ids = derive_module_ids(&file_path, extensions_root, &two_classes, &config).unwrap();
    for id in &multi_ids {
        assert!(
            pattern.is_match(id),
            "multi-class derived ID '{id}' must match canonical grammar"
        );
    }
    assert_eq!(
        multi_ids,
        vec![
            "executor.math.arithmetic.addition".to_string(),
            "executor.math.arithmetic.subtraction".to_string(),
        ]
    );
}

#[test]
fn conformance_disabled_by_default() {
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, "disabled_by_default");
    let ids = run_derive(case).expect("expected success");
    let expected: Vec<String> = case["expected"]["module_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, expected, "disabled-by-default returns base_id only");
}

fn assert_marked_class_case(id: &str) {
    let fixture = load_fixture("multi_module_discovery");
    let case = fixture_case(&fixture, id);
    let outcome = run_derive(case);
    let observed_error = match &outcome {
        Ok(_) => Value::Null,
        Err(error) => serde_json::json!({"code": error.code.wire_str()}),
    };
    assert_eq!(observed_error, case["expected"]["error"], "case {id}");
    assert_eq!(
        serde_json::json!(outcome.unwrap()),
        case["expected"]["module_ids"],
        "case {id}"
    );
}

#[test]
fn conformance_unmarked_class_beside_marked_ones_is_not_registered() {
    assert_marked_class_case("unmarked_class_beside_marked_ones_is_not_registered");
}

#[test]
fn conformance_one_marked_class_beside_an_unmarked_one_keeps_its_segment() {
    assert_marked_class_case("one_marked_class_beside_an_unmarked_one_keeps_its_segment");
}

struct NamedModule(&'static str);

#[async_trait::async_trait]
impl apcore::module::Module for NamedModule {
    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }
    fn description(&self) -> &str {
        self.0
    }
    async fn execute(
        &self,
        inputs: Value,
        _ctx: &apcore::Context<Value>,
    ) -> Result<Value, apcore::errors::ModuleError> {
        Ok(inputs)
    }
}

#[test]
fn registration_preserves_marked_module_identity_with_interleaved_helpers() {
    use apcore::registry::MultiClassEntry;
    for names in [
        vec![("Helper", false), ("Addition", true), ("Subtraction", true)],
        vec![("Addition", true), ("Helper", false), ("Subtraction", true)],
        vec![("Helper", false), ("Addition", true)],
    ] {
        let registry = apcore::Registry::new();
        let entries = names
            .iter()
            .map(|&(name, marked)| {
                MultiClassEntry::new(name, Box::new(NamedModule(name))).with_multi_class(marked)
            })
            .collect();
        let ids = registry
            .register_multi_class(
                &PathBuf::from("extensions/math/math_ops.py"),
                "extensions",
                entries,
                &DiscoveryConfig::default(),
            )
            .unwrap();
        let expected: Vec<_> = names
            .iter()
            .filter(|(_, marked)| *marked)
            .map(|(name, _)| format!("math.math_ops.{}", class_name_to_segment(name)))
            .collect();
        assert_eq!(ids, expected);
        for (name, marked) in names {
            let id = format!("math.math_ops.{}", class_name_to_segment(name));
            let module = registry.get(&id).unwrap();
            if marked {
                assert_eq!(
                    module.unwrap().description(),
                    name,
                    "module paired with {id}"
                );
            } else {
                assert!(module.is_none(), "unmarked {name} must not be registered");
            }
        }
    }
}

#[test]
fn registration_preserves_empty_single_class_and_unmarked_defaults() {
    use apcore::registry::MultiClassEntry;
    for (names, expected_name) in [
        (vec![], None),
        (vec![("Only", true)], Some("Only")),
        (vec![("Only", false)], Some("Only")),
        (vec![("First", false), ("Second", false)], Some("First")),
    ] {
        let registry = apcore::Registry::new();
        let entries = names
            .into_iter()
            .map(|(name, marked)| {
                MultiClassEntry::new(name, Box::new(NamedModule(name))).with_multi_class(marked)
            })
            .collect();
        let ids = registry
            .register_multi_class(
                &PathBuf::from("extensions/math/math_ops.py"),
                "extensions",
                entries,
                &DiscoveryConfig::default(),
            )
            .unwrap();
        let expected_ids = if expected_name.is_some() {
            vec!["math.math_ops"]
        } else {
            vec![]
        };
        assert_eq!(ids, expected_ids);
        assert_eq!(registry.module_ids(), expected_ids);
        let module = registry.get("math.math_ops").unwrap();
        assert_eq!(
            module.as_ref().map(|module| module.description()),
            expected_name
        );
    }
}

#[test]
fn registration_ignores_an_unmarked_class_with_the_same_segment() {
    use apcore::registry::MultiClassEntry;
    let registry = apcore::Registry::new();
    let ids = registry
        .register_multi_class(
            &PathBuf::from("extensions/math/math_ops.py"),
            "extensions",
            vec![
                MultiClassEntry::new("My_Module", Box::new(NamedModule("helper"))),
                MultiClassEntry::new("MyModule", Box::new(NamedModule("participant")))
                    .with_multi_class(true),
            ],
            &DiscoveryConfig::default(),
        )
        .unwrap();
    assert_eq!(ids, vec!["math.math_ops.my_module"]);
    assert_eq!(registry.module_ids(), ids);
    assert_eq!(
        registry.get(&ids[0]).unwrap().unwrap().description(),
        "participant"
    );
}

#[test]
fn registration_rolls_back_marked_modules_and_preserves_an_existing_duplicate() {
    use apcore::registry::MultiClassEntry;
    let registry = apcore::Registry::new();
    let existing_id = "math.math_ops.subtraction";
    registry
        .register_module(existing_id, Box::new(NamedModule("existing")))
        .unwrap();
    let existing = registry.get(existing_id).unwrap().unwrap();
    let error = registry
        .register_multi_class(
            &PathBuf::from("extensions/math/math_ops.py"),
            "extensions",
            vec![
                MultiClassEntry::new("Helper", Box::new(NamedModule("helper"))),
                MultiClassEntry::new("Addition", Box::new(NamedModule("new")))
                    .with_multi_class(true),
                MultiClassEntry::new("Subtraction", Box::new(NamedModule("duplicate")))
                    .with_multi_class(true),
            ],
            &DiscoveryConfig::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DuplicateModuleId);
    assert_eq!(registry.module_ids(), vec![existing_id]);
    let retained = registry.get(existing_id).unwrap().unwrap();
    assert!(std::sync::Arc::ptr_eq(&existing, &retained));
    assert_eq!(retained.description(), "existing");
}
