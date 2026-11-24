// D-28 alignment: ContextLogger MUST emit lowercase level names, nest
// caller-supplied extras under an `extra` key, and the obs-logging middleware
// MUST use module_id (not module) and inputs (not input) to match Python+TS.

use apcore::observability::logging::{ContextLogger, LogFormat};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A ContextLogger wired to write into an in-memory buffer rather than stderr,
/// so we can inspect the produced JSON record.
struct CapturingWriter(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for CapturingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn parse_record(buf: &[u8]) -> serde_json::Value {
    let s = String::from_utf8_lossy(buf);
    let line = s.lines().next().unwrap_or_default();
    serde_json::from_str(line).expect("logger output must be valid JSON")
}

#[test]
fn level_is_lowercase() {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let mut logger = ContextLogger::new("test");
    logger.set_format(LogFormat::Json);
    logger.set_writer(Box::new(CapturingWriter(buf.clone())));
    logger.info("hi");
    let rec = parse_record(&buf.lock().unwrap());
    assert_eq!(
        rec.get("level").and_then(|v| v.as_str()),
        Some("info"),
        "level must be lowercase"
    );
}

#[test]
fn extras_are_nested_under_extra_key() {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let mut logger = ContextLogger::new("test");
    logger.set_format(LogFormat::Json);
    logger.set_writer(Box::new(CapturingWriter(buf.clone())));

    let mut extra = HashMap::new();
    extra.insert("k".to_string(), serde_json::json!("v"));
    logger.emit("info", "hello", Some(&extra));

    let rec = parse_record(&buf.lock().unwrap());
    let nested = rec
        .get("extra")
        .expect("extras must be nested under `extra` key");
    assert_eq!(nested.get("k").and_then(|v| v.as_str()), Some("v"));
    // Top-level MUST NOT contain the user-supplied keys.
    assert!(
        rec.get("k").is_none(),
        "user extras must not be flattened to top-level"
    );
}

#[test]
fn middleware_extras_use_module_id_and_inputs() {
    use apcore::context::{Context, Identity};
    use apcore::middleware::base::Middleware;
    use apcore::observability::logging::ObsLoggingMiddleware;
    use serde_json::json;

    let buf = Arc::new(Mutex::new(Vec::new()));
    let mut logger = ContextLogger::new("test");
    logger.set_format(LogFormat::Json);
    logger.set_writer(Box::new(CapturingWriter(buf.clone())));

    let mw = ObsLoggingMiddleware::new(logger);
    let ctx = Context::<serde_json::Value>::new(Identity::new(
        "@caller".to_string(),
        "user".to_string(),
        Vec::new(),
        HashMap::new(),
    ));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let _ = mw
            .before("executor.m", json!({"x": 1}), &ctx)
            .await
            .unwrap();
    });

    let rec = parse_record(&buf.lock().unwrap());
    let extra = rec.get("extra").expect("extras nested");
    assert!(
        extra.get("module_id").is_some(),
        "middleware extra must use 'module_id', not 'module'"
    );
    assert!(
        extra.get("module").is_none(),
        "middleware extra must not emit legacy 'module' field"
    );
    assert!(
        extra.get("inputs").is_some(),
        "middleware extra must use 'inputs', not 'input'"
    );
    assert!(
        extra.get("input").is_none(),
        "middleware extra must not emit legacy 'input' field"
    );
}
