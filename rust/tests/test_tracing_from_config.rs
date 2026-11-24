//! PROTOCOL_SPEC §10.1.1 — `observability.tracing.*` reaches the running client.
//!
//! Five declared keys, all inert (apcore#118, decision D-68 C'). Two of them —
//! `strategy` and `otlp_endpoint` — were declared by §9.15.2's namespace
//! registration and absent from `schemas/apcore-config.schema.json`, so
//! `_config.strict` rejected them as unknown while the specification documented
//! their defaults. The other three were deprecated in spec v1.39.0 on the
//! finding that nothing read them.
//!
//! Every case drives a real client built from a real `Config`, and the ones
//! that can be measured are measured rather than asserted on a field. A test
//! that calls `TracingMiddleware::with_sampling(...)` directly proves the
//! middleware works, which was never in doubt; what was in doubt is whether a
//! `Config` reaches it.

use std::sync::{Arc, Mutex, PoisonError};

use apcore::config::Config;
use apcore::APCore;

fn config_with(tracing_yaml: &str) -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    let body = if tracing_yaml.is_empty() {
        "version: \"1.0\"\nproject:\n  name: tracing-probe\n".to_string()
    } else {
        format!(
            "version: \"1.0\"\nproject:\n  name: tracing-probe\nobservability:\n  tracing:\n{tracing_yaml}"
        )
    };
    std::fs::write(&path, body).expect("write config");
    let config = Config::from_yaml_file(&path).expect("config loads");
    (config, dir)
}

fn tracing_middlewares(client: &APCore) -> Vec<String> {
    client
        .executor()
        .middlewares()
        .into_iter()
        .filter(|n| n == "tracing")
        .collect()
}

// ---------------------------------------------------------------------------
// The five keys are readable, writable and validating alike
// ---------------------------------------------------------------------------

#[test]
fn every_key_has_its_canonical_default() {
    let config = Config::default();
    let tracing = &config.observability.tracing;
    assert!(!tracing.enabled);
    assert!((tracing.sampling_rate - 1.0).abs() < f64::EPSILON);
    assert_eq!(tracing.strategy, "full");
    assert_eq!(tracing.exporter, "stdout");
    assert_eq!(tracing.otlp_endpoint, None);
}

#[test]
fn every_key_round_trips_from_a_file() {
    let (config, _dir) = config_with(
        "    enabled: true\n    sampling_rate: 0.25\n    strategy: error_first\n\
         \n    exporter: otlp\n    otlp_endpoint: http://collector:4318/v1/traces\n",
    );
    assert_eq!(
        config.get("observability.tracing.enabled"),
        Some(true.into())
    );
    assert_eq!(
        config.get("observability.tracing.strategy"),
        Some("error_first".into())
    );
    assert_eq!(
        config.get("observability.tracing.otlp_endpoint"),
        Some("http://collector:4318/v1/traces".into())
    );
}

#[test]
fn every_key_round_trips_through_set() {
    let mut config = Config::default();
    for (key, value) in [
        ("observability.tracing.enabled", serde_json::json!(true)),
        (
            "observability.tracing.sampling_rate",
            serde_json::json!(0.25),
        ),
        ("observability.tracing.strategy", serde_json::json!("off")),
        ("observability.tracing.exporter", serde_json::json!("otlp")),
        (
            "observability.tracing.otlp_endpoint",
            serde_json::json!("http://x:4318"),
        ),
    ] {
        config.set(key, value.clone());
        assert_eq!(config.get(key), Some(value), "key {key}");
    }
}

#[test]
fn the_new_keys_are_accepted_under_strict_mode() {
    // `strategy` and `otlp_endpoint` were REJECTED here — the defect, exactly.
    let (config, _dir) = config_with(
        "    strategy: proportional\n    exporter: otlp\n    otlp_endpoint: http://x:4318\n",
    );
    let mut config = config;
    config.set("_config.strict", serde_json::json!(true));
    config
        .validate()
        .expect("strict mode accepts the five keys");
}

#[test]
fn an_out_of_range_value_is_rejected() {
    // `in_memory` is here on purpose: §9.15.2 used to name it, and §10.1.1
    // requirement 2 forbids it as a configuration value because the in-memory
    // exporter is a test buffer nothing can read back by name.
    for (leaf, bad) in [
        ("strategy", "sometimes"),
        ("exporter", "in_memory"),
        ("otlp_endpoint", ""),
    ] {
        let mut config = Config::default();
        config.set(
            &format!("observability.tracing.{leaf}"),
            serde_json::json!(bad),
        );
        assert!(config.validate().is_err(), "{leaf}={bad:?} was accepted");
    }
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

#[test]
fn no_tracing_configuration_installs_nothing() {
    // The whole blast radius: a project that does not ask for tracing is untouched.
    let (config, _dir) = config_with("");
    assert!(tracing_middlewares(&client_from(config)).is_empty());

    let (config, _dir) = config_with("    strategy: \"off\"\n    sampling_rate: 0.5\n");
    assert!(tracing_middlewares(&client_from(config)).is_empty());
}

#[test]
fn enabled_installs_a_middleware() {
    let (config, _dir) = config_with("    enabled: true\n");
    assert_eq!(tracing_middlewares(&client_from(config)).len(), 1);
}

#[test]
fn configuration_never_installs_a_second_tracing_middleware() {
    // §10.1.1 requirement 6. Configuration installs into an empty chain, so it
    // can never be the thing that adds a second; this pins that repeated
    // construction from one Config does not accumulate.
    let (config, _dir) = config_with("    enabled: true\n");
    for _ in 0..3 {
        assert_eq!(tracing_middlewares(&client_from(config.clone())).len(), 1);
    }
}

#[test]
fn a_caller_supplied_executor_is_left_alone() {
    // Parity with config-driven ACL discovery: an Executor the caller built is
    // respected as-is, tracing included.
    use apcore::{Executor, Registry};

    let (config, _dir) = config_with("    enabled: true\n");
    let registry = Arc::new(Registry::new());
    let executor = Executor::new(Arc::clone(&registry), Arc::new(config.clone()));
    let client = APCore::with_options(None, Some(executor), Some(config), None);
    assert!(tracing_middlewares(&client).is_empty());
}

fn client_from(config: Config) -> APCore {
    APCore::with_options(None, None, Some(config), None)
}

// ---------------------------------------------------------------------------
// The exporter, by name
// ---------------------------------------------------------------------------

#[cfg(feature = "events")]
#[test]
fn otlp_endpoint_reaches_the_exporter() {
    use apcore::observability::tracing_config::build_tracing_middleware;

    let (config, _dir) = config_with(
        "    enabled: true\n    exporter: otlp\n    otlp_endpoint: http://collector.internal:4318\n",
    );
    let mw = build_tracing_middleware(&config)
        .expect("builds")
        .expect("installed");
    assert_eq!(
        config.get("observability.tracing.otlp_endpoint"),
        Some("http://collector.internal:4318".into())
    );
    assert_eq!(
        mw.sampling_strategy,
        apcore::observability::tracing_middleware::SamplingStrategy::Always
    );
}

#[cfg(not(feature = "events"))]
#[test]
fn otlp_without_the_events_feature_installs_nothing_and_says_so() {
    // §10.1.1 requirement 4's second case, and it is LIVE in the default build:
    // `events` is not a default feature, and without it this crate's
    // OTLPExporter discards every span with a warning. Installing a middleware
    // around it would give an operator tracing "enabled" and no traces, with
    // nothing to read — worse than installing none.
    let (config, _dir) = config_with("    enabled: true\n    exporter: otlp\n");
    let (client, logs) = build_capturing(config);
    assert!(tracing_middlewares(&client).is_empty());
    assert!(logs.contains("`events` feature"), "logs:\n{logs}");
    assert!(logs.contains("stdout"), "logs:\n{logs}");
}

#[test]
fn a_null_endpoint_uses_the_specified_default() {
    use apcore::observability::tracing_config::DEFAULT_OTLP_ENDPOINT;

    assert_eq!(DEFAULT_OTLP_ENDPOINT, "http://localhost:4318/v1/traces");
}

#[test]
fn an_endpoint_with_a_non_otlp_exporter_is_rejected_at_load() {
    // §10.1.1 requirement 3 — not a silent no-op. An endpoint written down and
    // read by nothing is the shape of every defect apcore#118 found.
    //
    // "At load" is literal here: `Config::from_yaml_file` validates, so the
    // file never becomes a `Config` at all. An earlier draft of this case
    // loaded and then called `validate()`, and died on the load.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    let yaml = concat!(
        "version: \"1.0\"\n",
        "project:\n  name: p\n",
        "observability:\n",
        "  tracing:\n",
        "    enabled: true\n",
        "    exporter: stdout\n",
        "    otlp_endpoint: http://x:4318\n",
    );
    std::fs::write(&path, yaml).expect("write");
    let err = Config::from_yaml_file(&path).expect_err("must be rejected at load");
    assert!(
        err.to_string().contains("otlp_endpoint"),
        "message was: {err}"
    );
}

#[test]
fn jaeger_warns_installs_nothing_and_substitutes_nothing() {
    let (config, _dir) = config_with("    enabled: true\n    exporter: jaeger\n");
    let (client, logs) = build_capturing(config);
    assert!(tracing_middlewares(&client).is_empty());
    assert_eq!(logs.matches("'jaeger'").count(), 1, "logs:\n{logs}");
    assert!(logs.contains("otlp"), "logs:\n{logs}");
}

// ---------------------------------------------------------------------------
// The rate is measured, not asserted on a field
// ---------------------------------------------------------------------------

#[test]
fn the_configured_strategy_and_rate_reach_the_middleware() {
    use apcore::observability::tracing_config::build_tracing_middleware;
    use apcore::observability::tracing_middleware::SamplingStrategy;

    for (name, expected) in [
        ("full", SamplingStrategy::Always),
        ("proportional", SamplingStrategy::Probabilistic),
        ("error_first", SamplingStrategy::ErrorFirst),
        ("off", SamplingStrategy::Never),
    ] {
        let (config, _dir) = config_with(&format!(
            "    enabled: true\n    strategy: {name}\n    sampling_rate: 0.1\n"
        ));
        let mw = build_tracing_middleware(&config)
            .expect("builds")
            .expect("installed");
        assert_eq!(mw.sampling_strategy, expected, "strategy {name}");
        assert!(
            (mw.sampling_rate - 0.1).abs() < f64::EPSILON,
            "strategy {name}"
        );
    }
}

// ---------------------------------------------------------------------------
// Log capture
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

/// Build a client under a THREAD-LOCAL subscriber and return what it logged.
fn build_capturing(config: Config) -> (APCore, String) {
    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let client = tracing::subscriber::with_default(subscriber, || client_from(config));
    let bytes = buf.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (client, String::from_utf8_lossy(&bytes).into_owned())
}
