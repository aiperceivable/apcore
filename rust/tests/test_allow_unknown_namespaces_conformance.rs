//! Drive `allow_unknown_namespaces.json` — §9.6.3 `_config.allow_unknown`
//! (#118 D-69).
//!
//! Both halves of the `strict: false` row were inert. `allow_unknown: false` is
//! documented as "silently ignored (not stored)" and the namespace was stored
//! anyway; `allow_unknown: true` is documented as "stored, accessible, **WARN
//! logged**" and nothing logged. Fixing one without the other leaves the row
//! half true, so the fixture drives both — and the legacy-mode boundary
//! besides, so the namespace-only scoping is a decision rather than an omission.

use std::sync::{Arc, Mutex, PoisonError};

use apcore::config::Config;
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("allow_unknown_namespaces.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("allow_unknown_namespaces.json parses")
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

fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
    let buf = CaptureWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let bytes = buf.0.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (out, String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn conformance_allow_unknown_namespaces() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "the fixture drove nothing");

    let base = serde_json::json!({
        "version": "1.0",
        "project": {"name": "allow-unknown-probe"}
    });

    for case in cases {
        let id = case["id"].as_str().expect("case id");
        let input = &case["input"];
        let expected = &case["expected"];

        if input.get("registered_namespace").is_some() {
            drive_registered_namespace_default(id, input, expected, &base);
            continue;
        }

        let mut doc = serde_json::Map::new();
        if input["mode"] == "namespace" {
            doc.insert("apcore".to_string(), base.clone());
        } else {
            for (k, v) in base.as_object().expect("base") {
                doc.insert(k.clone(), v.clone());
            }
        }
        if let Some(meta) = input["config"].as_object() {
            doc.insert("_config".to_string(), Value::Object(meta.clone()));
        }
        let namespace = input["namespace"].as_str();
        if let Some(ns) = namespace {
            doc.insert(ns.to_string(), serde_json::json!({"x": 1}));
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("apcore.yaml");
        std::fs::write(
            &path,
            serde_yaml_ng::to_string(&Value::Object(doc)).expect("yaml"),
        )
        .expect("write");

        let (outcome, logs) = capture(|| Config::from_yaml_file(&path));

        if expected["loads"].as_bool() == Some(false) {
            let err = outcome.expect_err(&format!("case {id} must be rejected"));
            assert_eq!(
                err.code.wire_str(),
                expected["error_code"].as_str().expect("error_code"),
                "case {id}: {}",
                err.message
            );
            let needle = expected["error_message_contains"].as_str().expect("needle");
            assert!(err.message.contains(needle), "case {id}: {}", err.message);
            continue;
        }

        let config = outcome.unwrap_or_else(|e| panic!("case {id}: unexpected error: {e}"));

        if let Some(want) = expected.get("value_readable").and_then(Value::as_bool) {
            let key = format!("{}.x", namespace.expect("namespace"));
            assert_eq!(config.get(&key).is_some(), want, "case {id}: get({key})");
        }
        if let Some(needle) = expected.get("warns_naming").and_then(Value::as_str) {
            let hits = logs
                .lines()
                .filter(|l| l.contains(needle) && l.contains("registered"))
                .count();
            assert_eq!(hits, 1, "case {id}, logs:\n{logs}");
        }
        if let Some(needle) = expected.get("warns_absent").and_then(Value::as_str) {
            assert!(!logs.contains(needle), "case {id}, logs:\n{logs}");
        }
    }
}

/// D-117: a registered namespace's defaults answer only in NAMESPACE mode.
///
/// A legacy document has no namespaces, so a declaration ABOUT a namespace has
/// nothing to say about one. The key is absent from the file by construction —
/// if it were present the document would be answering, not the registration.
fn drive_registered_namespace_default(id: &str, input: &Value, expected: &Value, base: &Value) {
    let registration = &input["registered_namespace"];
    // Namespace registration is process-wide and permanent (§9.6.3 point 5), so
    // each case registers under its own name rather than racing the other.
    let declared = registration["name"].as_str().expect("name");
    let name = format!("{declared}_{}", &id[..12.min(id.len())]);
    let _ = Config::register_namespace(apcore::config::NamespaceRegistration {
        name: name.clone(),
        env_prefix: None,
        defaults: Some(registration["defaults"].clone()),
        schema: None,
        env_style: apcore::config::EnvStyle::Auto,
        max_depth: apcore::config::DEFAULT_MAX_DEPTH,
        env_map: None,
    });

    let mut doc = serde_json::Map::new();
    if input["mode"] == "namespace" {
        doc.insert("apcore".to_string(), base.clone());
    } else {
        for (k, v) in base.as_object().expect("base") {
            doc.insert(k.clone(), v.clone());
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("apcore.yaml");
    std::fs::write(
        &path,
        serde_yaml_ng::to_string(&Value::Object(doc)).expect("yaml"),
    )
    .expect("write");

    let config = Config::from_yaml_file(&path).unwrap_or_else(|e| panic!("case {id}: {e}"));
    let key = input["key"]
        .as_str()
        .expect("key")
        .replacen(declared, &name, 1);
    let value = config.get(&key);

    assert_eq!(
        value.is_some(),
        expected["value_readable"]
            .as_bool()
            .expect("value_readable"),
        "case {id}: get({key}) -> {value:?}; the registration must answer in \
         namespace mode and stay silent for a legacy document"
    );
    if let Some(want) = expected.get("value") {
        assert_eq!(value.as_ref(), Some(want), "case {id}");
    }
}
