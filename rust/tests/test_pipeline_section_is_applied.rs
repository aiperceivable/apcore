//! PROTOCOL_SPEC §5.16 requirements 6 and 7 — a configured `pipeline:` section.
//!
//! The section was accepted, validated and then ignored, in all three SDKs
//! (apcore#118, decision D-72). `build_strategy_from_config` existed here as a
//! public function, but its only callers were its own tests: nothing extracted
//! the section from a loaded `Config`, so `pipeline: remove: [acl_check]` left
//! all eleven steps in place and a declared custom step silently never ran.
//!
//! The asymmetry is why these cases exist. Failing to *remove* a step is
//! fail-safe; failing to *insert* one is not — a declared audit, rate-limit or
//! authorization step that never runs is invisible from inside the running
//! system, and the pipeline an operator reads in configuration is not the
//! pipeline that executes.

use apcore::{Config, Executor, Registry};
use std::sync::{Arc, Mutex, PoisonError};

const DEFAULT_STEPS: [&str; 11] = [
    "context_creation",
    "call_chain_guard",
    "module_lookup",
    "acl_check",
    "approval_gate",
    "middleware_before",
    "input_validation",
    "execute",
    "output_validation",
    "middleware_after",
    "return_result",
];

/// Load a `Config` the way an operator does — from a file on disk.
///
/// `Config::set` is deliberately not used here: it does not route through
/// `match_registered_namespace` the way `get` does (sync finding A-D-050), so a
/// probe built on it can agree with the code while disagreeing with the YAML an
/// operator actually writes. That gap is the whole shape of apcore#118.
fn config_with(pipeline_yaml: &str) -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    let body = if pipeline_yaml.is_empty() {
        "version: \"1.0\"\nproject:\n  name: pipeline-probe\n".to_string()
    } else {
        format!("version: \"1.0\"\nproject:\n  name: pipeline-probe\npipeline:\n{pipeline_yaml}")
    };
    std::fs::write(&path, body).expect("write config");
    let config = Config::from_yaml_file(&path).expect("config loads");
    // The TempDir is returned so the caller keeps it alive for the test.
    (config, dir)
}

fn step_names(executor: &Executor) -> Vec<String> {
    executor.strategy().step_names()
}

#[test]
fn no_pipeline_section_keeps_the_default() {
    // The half that keeps this additive for everyone who configures nothing.
    let executor = Executor::new(Registry::new(), Config::default());
    assert_eq!(step_names(&executor), DEFAULT_STEPS);
}

#[test]
fn an_empty_pipeline_section_keeps_the_default() {
    // `pipeline: {}` short-circuits before the builder; `remove: []` goes
    // through it and has to come out the other side unchanged. Both are ways
    // of saying "nothing", and they take different code paths.
    for body in ["  {}\n", "  remove: []\n"] {
        let (config, _dir) = config_with(body);
        let executor = Executor::new(Registry::new(), config);
        assert_eq!(step_names(&executor), DEFAULT_STEPS, "body: {body:?}");
    }
}

#[test]
fn remove_takes_the_named_step_out() {
    let (config, _dir) = config_with("  remove:\n    - output_validation\n");
    let executor = Executor::new(Registry::new(), config);
    let names = step_names(&executor);
    assert!(
        !names.contains(&"output_validation".to_string()),
        "{names:?}"
    );
    assert_eq!(names.len(), DEFAULT_STEPS.len() - 1);
}

#[test]
fn remove_takes_a_security_step_out() {
    // The operator gets what they asked for. Requirement 7's diagnostic covers
    // the fact that they now genuinely get it; it does not refuse the request.
    let (config, _dir) = config_with("  remove:\n    - acl_check\n");
    let executor = Executor::new(Registry::new(), config);
    assert!(!step_names(&executor).contains(&"acl_check".to_string()));
}

#[test]
fn configure_sets_a_field_without_reordering() {
    let (config, _dir) =
        config_with("  configure:\n    input_validation:\n      ignore_errors: true\n");
    let executor = Executor::new(Registry::new(), config);
    assert_eq!(step_names(&executor), DEFAULT_STEPS);
    let strategy = executor.strategy();
    let step = strategy
        .steps()
        .iter()
        .find(|s| s.name() == "input_validation")
        .expect("step present");
    assert!(step.ignore_errors());
}

#[test]
fn with_options_applies_the_section_too() {
    // `new` is not the only door into a default strategy.
    let (config, _dir) = config_with("  remove:\n    - output_validation\n");
    let executor = Executor::with_options(Registry::new(), config, None, None, None);
    assert!(!step_names(&executor).contains(&"output_validation".to_string()));
}

#[test]
fn an_explicit_strategy_still_wins() {
    // D-73's precedence: an API argument beats `Config`. Without this the
    // wiring would take a caller's hand-built strategy away from them whenever
    // a `pipeline:` section happened to be present.
    let (config, _dir) = config_with("  remove:\n    - acl_check\n");
    let executor =
        Executor::with_strategy(Registry::new(), config, apcore::build_standard_strategy());
    assert!(step_names(&executor).contains(&"acl_check".to_string()));
}

#[test]
fn a_named_preset_still_wins() {
    let (config, _dir) = config_with("  remove:\n    - acl_check\n");
    let executor =
        Executor::with_strategy_name(Registry::new(), config, "standard").expect("preset resolves");
    assert!(step_names(&executor).contains(&"acl_check".to_string()));
}

#[test]
fn an_unbuildable_section_falls_back_to_the_standard_pipeline() {
    // Python and TypeScript propagate the error out of their constructors;
    // `Executor::new` returns `Self` and cannot. The fallback is the fail-safe
    // direction — every built-in protection stays — and it is logged at error
    // level rather than passed over. See `strategy_from_config`.
    let (config, _dir) = config_with("  remove:\n    - no_such_step\n");
    let executor = Executor::new(Registry::new(), config);
    assert_eq!(step_names(&executor), DEFAULT_STEPS);
}

#[test]
fn the_module_lookup_step_stays_bound_to_the_instance_toggle_store() {
    // Seeding a config-built strategy from the plain `build_standard_strategy`
    // would bind the process-global toggle store, reintroducing issue #71
    // through a different door. `APCore` rebinds the five presets by name, and
    // a config-built strategy keeps the name "standard", so it is covered —
    // this pins that, since it is the property a rename would quietly break.
    let (config, _dir) = config_with("  remove:\n    - output_validation\n");
    let executor = Executor::new(Registry::new(), config);
    assert_eq!(executor.strategy().name(), "standard");
}

// ---------------------------------------------------------------------------
// §5.16 requirement 7 — the diagnostic
// ---------------------------------------------------------------------------

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

/// Build an executor under a THREAD-LOCAL subscriber and return what it logged.
///
/// Thread-local, not global, so these cases neither steal nor are polluted by
/// the output of anything else the harness runs beside them.
fn build_capturing(pipeline_yaml: &str) -> String {
    let (config, _dir) = config_with(pipeline_yaml);
    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let _ = Executor::new(Registry::new(), config);
    });
    let bytes = buf.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn removing_a_security_step_warns_once() {
    for step in ["acl_check", "approval_gate"] {
        let logs = build_capturing(&format!("  remove:\n    - {step}\n"));
        let hits = logs.matches("pipeline.remove takes security step").count();
        assert_eq!(hits, 1, "step {step}, logs:\n{logs}");
        assert!(logs.contains(step), "step {step} not named, logs:\n{logs}");
    }
}

#[test]
fn removing_both_security_steps_warns_once_naming_both() {
    // Once per configuration load, per §9.2.2's cadence — not once per step.
    let logs = build_capturing("  remove:\n    - acl_check\n    - approval_gate\n");
    assert_eq!(
        logs.matches("pipeline.remove takes security step").count(),
        1
    );
    assert!(logs.contains("acl_check, approval_gate"), "logs:\n{logs}");
}

#[test]
fn removing_an_ordinary_step_is_silent() {
    // The other half: the notice is about protections, not every removal.
    let logs = build_capturing("  remove:\n    - output_validation\n");
    assert!(
        !logs.contains("pipeline.remove takes security step"),
        "logs:\n{logs}"
    );
}

#[test]
fn an_unbuildable_section_is_logged_at_error_level() {
    // The fallback must never be silent — a declared step that does not run is
    // the defect requirement 6 exists to remove.
    let logs = build_capturing("  remove:\n    - no_such_step\n");
    assert!(logs.contains("ERROR"), "logs:\n{logs}");
    assert!(
        logs.contains("failed to build; running the standard pipeline"),
        "logs:\n{logs}"
    );
}
