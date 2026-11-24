//! Drive `pipeline_section_wiring.json` — §5.16 requirements 6 and 7 (#118 D-72).
//!
//! Every case goes through `Executor::new` on a `Config` loaded from a real
//! file. That is the whole point of the fixture, and it is why the three
//! fixtures that already cover the pipeline builder could not have caught this:
//! they hand the section straight to `build_strategy_from_config(section, …)`,
//! whose first parameter is a `Value` the CALLER supplies. Nothing extracted
//! that value from a `Config`, so `pipeline: remove: [acl_check]` left all
//! eleven steps in place while every builder fixture stayed green.
//!
//! A driver here that called the builder directly would reproduce the defect
//! and pass.

use std::sync::{Arc, Mutex, PoisonError};

use apcore::errors::ModuleError;
use apcore::pipeline::{PipelineContext, Step, StepResult};
use apcore::pipeline_config::{register_step_type, unregister_step_type};
use apcore::{Config, Executor, Registry};
use async_trait::async_trait;
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("pipeline_section_wiring.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("pipeline_section_wiring.json parses")
}

/// No-op step registered under the fixture's `register_step_type` name.
struct ProbeStep(String);

#[async_trait]
impl Step for ProbeStep {
    fn name(&self) -> &str {
        &self.0
    }
    fn description(&self) -> &str {
        "No-op step registered for the pipeline_section_wiring fixture."
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::continue_step())
    }
}

// --- log capture -----------------------------------------------------------

#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for CaptureWriter {
    type Writer = Self;
    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

/// Build an executor from `config_doc` under a THREAD-LOCAL subscriber.
///
/// Thread-local, not global, so these cases neither steal nor are polluted by
/// the output of anything else the harness runs beside them.
fn build(config_doc: &Value) -> (Executor, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, serde_yaml_ng::to_string(config_doc).expect("yaml")).expect("write");
    let config = Config::from_yaml_file(&path).expect("config loads");

    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let executor =
        tracing::subscriber::with_default(subscriber, || Executor::new(Registry::new(), config));
    let bytes = buf.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (executor, String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn conformance_pipeline_section_wiring() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "the fixture drove nothing");

    for case in cases {
        let id = case["id"].as_str().expect("case id");
        let input = &case["input"];
        let expected = &case["expected"];

        let type_name = input.get("register_step_type").and_then(Value::as_str);
        if let Some(name) = type_name {
            let owned = name.to_string();
            register_step_type(
                name,
                Box::new(move |_cfg: &Value| {
                    Ok(Box::new(ProbeStep(owned.clone())) as Box<dyn Step>)
                }),
            )
            .expect("step type registers");
        }

        let (executor, logs) = build(&input["config"]);

        if let Some(name) = type_name {
            let _ = unregister_step_type(name);
        }

        let want: Vec<&str> = expected["steps"]
            .as_array()
            .expect("expected.steps")
            .iter()
            .map(|v| v.as_str().expect("step name"))
            .collect();
        assert_eq!(executor.strategy().step_names(), want, "case {id}");

        if let Some(configured) = expected.get("configured_step") {
            let step_name = configured["name"].as_str().expect("configured_step.name");
            let field = configured["field"].as_str().expect("configured_step.field");
            let strategy = executor.strategy();
            let step = strategy
                .steps()
                .iter()
                .find(|s| s.name() == step_name)
                .unwrap_or_else(|| panic!("case {id}: {step_name} is not in the pipeline"));
            // The fixture names one field; this SDK exposes it on the Step trait.
            let actual = match field {
                "ignore_errors" => Value::Bool(step.ignore_errors()),
                "pure" => Value::Bool(step.pure()),
                other => panic!("case {id}: no accessor for configured field {other:?}"),
            };
            assert_eq!(actual, configured["value"], "case {id}: {field}");
        }

        let hits = logs.matches("pipeline.remove takes security step").count();
        if expected["security_step_warning"].as_bool() == Some(true) {
            // Once per configuration load, per §9.2.2's cadence, not once per step.
            assert_eq!(hits, 1, "case {id}, logs:\n{logs}");
            let named = expected["warning_names_step"]
                .as_str()
                .expect("warning_names_step");
            assert!(
                logs.contains(named),
                "case {id}: {named} not named:\n{logs}"
            );
        } else {
            assert_eq!(hits, 0, "case {id}, logs:\n{logs}");
        }
    }
}
