//! Cross-language conformance driver for `gate_step_configure.json`
//! (PROTOCOL_SPEC 5.16.1 — governance gates cannot be weakened by
//! `configure`, D-130).
//!
//! Fixture source: apcore/conformance/fixtures/gate_step_configure.json.
//!
//! Per `driver_contract.path`, each case builds the strategy through
//! `build_strategy_from_config` — the same public config path
//! `pipeline_failfast_config.json` drives — and asserts at that point, never
//! executing a call. Per `assert_the_wire_code`, the error is asserted by its
//! wire code, not its type.

use apcore::errors::{ErrorCode, ModuleError};
use apcore::pipeline::ExecutionStrategy;
use apcore::pipeline_config::build_strategy_from_config;
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("gate_step_configure.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("gate_step_configure.json parses")
}

fn expected_code(wire: &str) -> ErrorCode {
    match wire {
        "PIPELINE_CONFIGURATION_ERROR" => ErrorCode::PipelineConfigurationError,
        other => panic!(
            "gate_step_configure.json names error_code `{other}` this driver cannot map — \
             teach the driver, do not skip it"
        ),
    }
}

fn run_case(tc: &Value) {
    let id = tc["id"].as_str().expect("every case needs an id");
    let pipeline = &tc["input"]["yaml"]["pipeline"];
    assert!(
        pipeline.is_object(),
        "[{id}] input.yaml.pipeline must be an object map"
    );
    let outcome: Result<ExecutionStrategy, ModuleError> = build_strategy_from_config(pipeline);

    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));
    for (field, want) in expected {
        match field.as_str() {
            "raises" => assert_eq!(
                outcome.is_err(),
                want.as_bool().expect("raises is a bool"),
                "[{id}] raises: {:?}",
                outcome.as_ref().err().map(|e| e.message.clone())
            ),
            "error_code" => {
                let err = outcome
                    .as_ref()
                    .err()
                    .unwrap_or_else(|| panic!("[{id}] expected an error, got Ok"));
                assert_eq!(
                    err.code,
                    expected_code(want.as_str().expect("error_code is a string")),
                    "[{id}] error_code (message: {})",
                    err.message
                );
            }
            "error_message_contains" => {
                let err = outcome
                    .as_ref()
                    .err()
                    .unwrap_or_else(|| panic!("[{id}] expected an error, got Ok"));
                for fragment in want.as_array().expect("error_message_contains is an array") {
                    let fragment = fragment.as_str().expect("fragment is a string");
                    assert!(
                        err.message.contains(fragment),
                        "[{id}] error message must mention `{fragment}`, got: {}",
                        err.message
                    );
                }
            }
            other => panic!(
                "[{id}] gate_step_configure.json grew expectation `{other}` that this driver \
                 does not check — teach the driver, do not skip it"
            ),
        }
    }
}

#[test]
fn conformance_gate_step_configure() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");
    for tc in cases {
        run_case(tc);
    }
}
