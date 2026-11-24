//! Drive `tracing_from_config.json` — §10.1.1 (#118 D-68 C').
//!
//! Every case goes through a real `Config` and `APCore`. A driver that called
//! `build_tracing_middleware` directly would prove the builder works, which was
//! never in doubt; what was inert for the whole life of these keys is the step
//! before it — no SDK extracted `observability.tracing.*` from a `Config` and
//! installed anything.

use std::sync::{Arc, Mutex, PoisonError};

use apcore::config::Config;
use apcore::APCore;
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("tracing_from_config.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("tracing_from_config.json parses")
}

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

/// Write the case's config as a real file, load it, and build a client.
///
/// `Config::from_yaml_file` validates, which is the door `expected.loads`
/// decides. Everything the load emits is captured under a thread-local
/// subscriber so the `jaeger` and deprecation cases can read it.
fn run(config_doc: &Value) -> (Option<APCore>, Option<(String, String)>, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(
        &path,
        serde_json::to_string(config_doc).expect("json is valid yaml"),
    )
    .expect("write");

    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let outcome =
        tracing::subscriber::with_default(subscriber, || match Config::from_yaml_file(&path) {
            Ok(config) => (
                Some(APCore::with_options(None, None, Some(config), None)),
                None,
            ),
            // The WIRE code, never the Rust variant name: `Display` prints
            // `ConfigInvalid` while the contract is `CONFIG_INVALID`, and an
            // assertion on the rendered message passes on neither.
            Err(e) => (None, Some((e.code.wire_str(), e.to_string()))),
        });
    let bytes = buf.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (
        outcome.0,
        outcome.1,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn tracing_count(client: &APCore) -> usize {
    client
        .executor()
        .middlewares()
        .into_iter()
        .filter(|n| n == "tracing")
        .count()
}

#[test]
fn conformance_tracing_from_config() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "the fixture drove nothing");

    for case in cases {
        let id = case["id"].as_str().expect("case id");
        let expected = &case["expected"];
        let (client, error, logs) = run(&case["input"]["config"]);

        if expected["loads"].as_bool() == Some(false) {
            let (code, message) =
                error.unwrap_or_else(|| panic!("case {id}: the configuration must be rejected"));
            assert_eq!(
                code,
                expected["error_code"].as_str().expect("error_code"),
                "case {id}: {message}"
            );
            let needle = expected["error_message_contains"].as_str().expect("needle");
            assert!(message.contains(needle), "case {id}: {message}");
            continue;
        }

        let client = client.unwrap_or_else(|| {
            panic!(
                "case {id}: unexpected rejection: {}",
                error.map(|(_, m)| m).unwrap_or_default()
            )
        });
        let installed = tracing_count(&client);

        if let Some(want) = expected
            .get("tracing_middleware_count")
            .and_then(Value::as_u64)
        {
            assert_eq!(installed as u64, want, "case {id}");
        }

        if let Some(kind) = expected.get("exporter_kind").and_then(Value::as_str) {
            if installed == 0 && expected["otlp_may_be_unavailable"].as_bool() == Some(true) {
                // §10.1.1 requirement 4: without the `events` feature this
                // crate's OTLP exporter discards every span, so refusing to
                // install IS the conformant answer.
                assert!(
                    logs.contains("`events` feature"),
                    "case {id}: nothing installed and nothing said:\n{logs}"
                );
                continue;
            }
            assert!(installed > 0, "case {id}: expected an {kind} exporter");
        }

        // The strategy and rate are read back off the CONFIG the client was
        // built from: the middleware chain exposes names, not handles, in this
        // SDK, and `test_tracing_from_config.rs` pins the field values through
        // the builder directly.
        if let Some(strategy) = expected.get("sampling_strategy").and_then(Value::as_str) {
            let (config, _dir) = reload(&case["input"]["config"]);
            assert_eq!(config.observability.tracing.strategy, strategy, "case {id}");
        }
        if let Some(rate) = expected.get("sampling_rate").and_then(Value::as_f64) {
            let (config, _dir) = reload(&case["input"]["config"]);
            assert!(
                (config.observability.tracing.sampling_rate - rate).abs() < f64::EPSILON,
                "case {id}"
            );
        }

        match expected.get("deprecation_warning").and_then(Value::as_bool) {
            Some(true) => {
                let naming = expected["warns_naming"].as_str().expect("warns_naming");
                assert!(logs.contains(naming), "case {id}: logs:\n{logs}");
            }
            Some(false) => {
                assert!(
                    !logs.contains("§9.2.4") && !logs.contains("9.2.4"),
                    "case {id}: a wired key warned as deprecated:\n{logs}"
                );
            }
            None => {
                if let Some(naming) = expected.get("warns_naming").and_then(Value::as_str) {
                    assert!(logs.contains(naming), "case {id}: logs:\n{logs}");
                }
            }
        }
    }
}

fn reload(config_doc: &Value) -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(&path, serde_json::to_string(config_doc).expect("json")).expect("write");
    (Config::from_yaml_file(&path).expect("loads"), dir)
}
