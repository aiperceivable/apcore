//! Drive `pipeline_failfast_config.json` — Issue #33 configuration fail-fast:
//! missing step references and unmet `requires`/`provides` MUST raise typed
//! errors at parse / construction time, never a warning deferred to the first
//! `call()`.
//!
//! Two adaptations this driver used to carry are gone, both fixed upstream:
//!
//! 1. `configure` shape. The fixture wrote `configure` as an ARRAY of
//!    `{name, ...overrides}` while `schemas/apcore-config.schema.json`
//!    (`$defs.PipelineConfig.configure`) declares an OBJECT keyed by step name
//!    — which is what all three SDKs parse. This driver rewrote the array into
//!    the object form to keep the case meaningful. The fixture now ships the
//!    object form and the rewriter is deleted.
//!
//! 2. `step_middleware`. The fixture carried a case for a
//!    `pipeline.step_middleware:` section that no SDK parses and that the
//!    canonical schema does not declare (`$defs.PipelineConfig` is
//!    `additionalProperties: false`). The case was removed rather than
//!    implemented, so the `#[ignore]`d test that pinned it is gone too.
//!
//! ASSERT THE WIRE CODE. Every case states its failure as `error_code`, never
//! a class name: all three SDKs call this class `ConfigurationError` while
//! they emitted three DIFFERENT codes, so a class-name assertion was green
//! everywhere and proved nothing.

use apcore::errors::{ErrorCode, ModuleError};
use apcore::pipeline::{ExecutionStrategy, PipelineContext, Step, StepResult};
use apcore::pipeline_config::{
    build_strategy_from_config, register_step_type, unregister_step_type,
};
use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("pipeline_failfast_config.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("pipeline_failfast_config.json parses")
}

fn case_by_id(fx: &Value, id: &str) -> Value {
    fx["test_cases"]
        .as_array()
        .expect("test_cases is an array")
        .iter()
        .find(|tc| tc["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("pipeline_failfast_config.json no longer carries case `{id}`"))
        .clone()
}

/// Cases held out of the always-on run. Empty: the one case that sat here
/// drove a `pipeline.step_middleware:` config section that no SDK parses, and
/// it was removed from the fixture rather than implemented.
const QUARANTINED: &[&str] = &[];

// ---------------------------------------------------------------------------
// Step used to materialise the fixture's `strategy.steps` entries
// ---------------------------------------------------------------------------

struct FixtureStep {
    name: String,
    requires: Vec<&'static str>,
    provides: Vec<&'static str>,
}

#[async_trait]
impl Step for FixtureStep {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.name
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    fn requires(&self) -> &[&'static str] {
        &self.requires
    }
    fn provides(&self) -> &[&'static str] {
        &self.provides
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::continue_step())
    }
}

/// `Step::requires`/`provides` are `&'static str`; fixture data is owned, so
/// leak it. Test-process lifetime, bounded by the fixture's size.
fn static_strs(value: &Value) -> Vec<&'static str> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| {
                    let owned = v.as_str().expect("requires/provides entry is a string");
                    &*Box::leak(owned.to_string().into_boxed_str())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strategy_from(spec: &Value) -> Result<ExecutionStrategy, ModuleError> {
    let steps: Vec<Box<dyn Step>> = spec["steps"]
        .as_array()
        .expect("strategy.steps is an array")
        .iter()
        .map(|s| {
            Box::new(FixtureStep {
                name: s["name"].as_str().expect("step.name").to_string(),
                requires: static_strs(&s["requires"]),
                provides: static_strs(&s["provides"]),
            }) as Box<dyn Step>
        })
        .collect();
    ExecutionStrategy::new(spec["name"].as_str().unwrap_or("custom"), steps)
}

/// Assert the fixture's `yaml.pipeline` shape and hand it to the SDK unchanged.
///
/// This is a SHAPE GUARD, not a rewriter. An earlier fixture revision encoded
/// `pipeline.configure` as an ARRAY; apcore#81 corrected it to the object map
/// that `schemas/apcore-config.schema.json` `$defs/PipelineConfig` declares and
/// that all three SDKs parse. `as_object()` here is what fails loudly if the
/// array shape ever returns, instead of the driver silently coercing it.
fn canonical_pipeline_config(pipeline: &Value) -> Value {
    let mut out = Map::new();
    for (key, value) in pipeline.as_object().expect(
        "yaml.pipeline is an object map keyed by section name — an array-shaped \
         `configure` is the pre-apcore#81 encoding and must not come back",
    ) {
        out.insert(key.clone(), value.clone());
    }
    Value::Object(out)
}

fn expected_code(wire: &str) -> ErrorCode {
    match wire {
        "PIPELINE_CONFIGURATION_ERROR" => ErrorCode::PipelineConfigurationError,
        "PIPELINE_DEPENDENCY_ERROR" => ErrorCode::PipelineDependencyError,
        other => panic!(
            "pipeline_failfast_config.json names error_code `{other}` this driver \
             cannot map to an ErrorCode — teach the driver, do not skip it"
        ),
    }
}

/// Register the step type the `pipeline.steps` case names, so the case reaches
/// the INSERTION path.
///
/// `driver_contract.steps_entries_are_closed_too`: without a registered type
/// the entry fails on "step type not registered" instead, which is a different
/// error and would pass an assertion on `error_code` while testing nothing. The
/// rejection under test happens before the factory lookup, so this registration
/// is what proves the rejection is about the unknown KEY.
fn with_fixture_step_types<T>(tc: &Value, body: impl FnOnce() -> T) -> T {
    let types: Vec<String> = tc["input"]
        .get("yaml")
        .and_then(|y| y["pipeline"].get("steps"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e.get("type").and_then(|t| t.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    for name in &types {
        let owned = name.clone();
        let _ = register_step_type(
            name,
            Box::new(move |_cfg: &Value| -> Result<Box<dyn Step>, ModuleError> {
                Ok(Box::new(FixtureStep {
                    name: owned.clone(),
                    requires: Vec::new(),
                    provides: Vec::new(),
                }))
            }),
        );
    }
    let out = body();
    for name in &types {
        let _ = unregister_step_type(name);
    }
    out
}

fn message_fragments(want: &Value) -> Vec<String> {
    match want {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .map(|v| v.as_str().expect("fragment is a string").to_string())
            .collect(),
        other => panic!("error_message_contains must be a string or array, got {other}"),
    }
}

/// Run one case. `raised_at` doubles as the assertion that the error surfaced
/// from the constructor rather than from a later `call()`: `parse_time` cases
/// go through `build_strategy_from_config`, `strategy_construction` cases
/// through `ExecutionStrategy::new`, and nothing in this driver ever executes
/// the pipeline.
fn run_case(tc: &Value) {
    let id = tc["id"].as_str().expect("every case needs an id");
    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

    let outcome: Result<ExecutionStrategy, ModuleError> =
        if let Some(yaml) = tc["input"].get("yaml") {
            let cfg = canonical_pipeline_config(&yaml["pipeline"]);
            with_fixture_step_types(tc, || build_strategy_from_config(&cfg))
        } else if let Some(strategy) = tc["input"].get("strategy") {
            strategy_from(strategy)
        } else {
            panic!("[{id}] case input has neither `yaml` nor `strategy`")
        };

    for (field, want) in expected {
        match field.as_str() {
            "raises" => {
                let raises = want.as_bool().expect("raises is a bool");
                assert_eq!(
                    outcome.is_err(),
                    raises,
                    "[{id}] raises: {:?}",
                    outcome.as_ref().err().map(|e| e.message.clone())
                );
            }
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
            // A prose note in the fixture, not an assertion.
            "error_class_name_is_not_the_contract" => {}
            "error_message_contains" => {
                let err = outcome
                    .as_ref()
                    .err()
                    .unwrap_or_else(|| panic!("[{id}] expected an error, got Ok"));
                for fragment in message_fragments(want) {
                    assert!(
                        err.message.contains(&fragment),
                        "[{id}] error message must mention `{fragment}`, got: {}",
                        err.message
                    );
                }
            }
            "raised_at" => {
                let where_ = want.as_str().expect("raised_at is a string");
                let via_yaml = tc["input"].get("yaml").is_some();
                let expected_api = match where_ {
                    "parse_time" => true,
                    "strategy_construction" => false,
                    other => panic!("[{id}] unknown raised_at `{other}`"),
                };
                assert_eq!(
                    via_yaml, expected_api,
                    "[{id}] raised_at `{where_}` must come from the matching constructor"
                );
                assert!(
                    outcome.is_err(),
                    "[{id}] raised_at implies the constructor itself failed"
                );
            }
            "deferred_to_first_call" => {
                assert!(
                    !want.as_bool().expect("deferred_to_first_call is a bool"),
                    "[{id}] fixture must not expect deferral"
                );
                // Nothing in this driver calls the pipeline, so an error here
                // could only have come from construction.
                assert!(outcome.is_err(), "[{id}] construction must have failed");
            }
            "strategy_callable" => {
                assert!(
                    want.as_bool().expect("strategy_callable is a bool"),
                    "[{id}] unexpected strategy_callable=false"
                );
                let strategy = outcome
                    .as_ref()
                    .unwrap_or_else(|e| panic!("[{id}] construction failed: {}", e.message));
                // A `strategy:` case declares the exact step list; a `yaml:`
                // case configures the STANDARD strategy, which has its own.
                if let Some(steps) = tc["input"]
                    .get("strategy")
                    .and_then(|s| s.get("steps"))
                    .and_then(|v| v.as_array())
                {
                    let names: Vec<&str> = strategy.steps().iter().map(|s| s.name()).collect();
                    let want_names: Vec<&str> = steps
                        .iter()
                        .map(|s| s["name"].as_str().expect("step.name"))
                        .collect();
                    assert_eq!(
                        names, want_names,
                        "[{id}] a callable strategy must retain every declared step in order"
                    );
                } else {
                    assert!(
                        !strategy.steps().is_empty(),
                        "[{id}] a callable strategy must retain the standard steps"
                    );
                }
            }
            // `driver_contract.read_the_field_back_off_the_step`: assert each
            // value on the BUILT step. `raises: false` alone also passes
            // against an implementation that accepts the keys and applies
            // none of them — which is what this SDK did with a warning.
            "configured_step_fields" => {
                let want = want
                    .as_object()
                    .expect("configured_step_fields is an object");
                let strategy = outcome
                    .as_ref()
                    .unwrap_or_else(|e| panic!("[{id}] construction failed: {}", e.message));
                let step_name = want["step_name"].as_str().expect("step_name is a string");
                let step = strategy
                    .steps()
                    .iter()
                    .find(|s| s.name() == step_name)
                    .unwrap_or_else(|| panic!("[{id}] configured step `{step_name}` is gone"));
                for (field, value) in want {
                    match field.as_str() {
                        "step_name" => {}
                        "match_modules" => {
                            let got: Vec<&str> = step
                                .match_modules()
                                .unwrap_or(&[])
                                .iter()
                                .map(String::as_str)
                                .collect();
                            let expect: Vec<&str> = value
                                .as_array()
                                .expect("match_modules is an array")
                                .iter()
                                .map(|v| v.as_str().expect("glob is a string"))
                                .collect();
                            assert_eq!(got, expect, "[{id}] match_modules on `{step_name}`");
                        }
                        "ignore_errors" => assert_eq!(
                            step.ignore_errors(),
                            value.as_bool().expect("ignore_errors is a bool"),
                            "[{id}] ignore_errors on `{step_name}`"
                        ),
                        "pure" => assert_eq!(
                            step.pure(),
                            value.as_bool().expect("pure is a bool"),
                            "[{id}] pure on `{step_name}`"
                        ),
                        "timeout_ms" => assert_eq!(
                            step.timeout_ms(),
                            value.as_u64().expect("timeout_ms is an integer"),
                            "[{id}] timeout_ms on `{step_name}`"
                        ),
                        other => panic!(
                            "[{id}] configured_step_fields grew `{other}` that this \
                             driver does not read back — teach it, do not skip it"
                        ),
                    }
                }
            }
            other => panic!(
                "[{id}] pipeline_failfast_config.json grew expectation `{other}` that \
                 this driver does not check — teach the driver, do not skip it"
            ),
        }
    }
}

#[test]
fn conformance_pipeline_failfast_config() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");

    for id in QUARANTINED {
        let _ = case_by_id(&fx, id);
    }

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        if QUARANTINED.contains(&id) {
            continue; // QUARANTINED is empty — see its doc comment
        }
        run_case(tc);
    }
}
