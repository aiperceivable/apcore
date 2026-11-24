//! The redaction contract at the surface an operator configures.
//!
//! Asserts the emitted LOG RECORD from [`ObsLoggingMiddleware`], not the helper
//! the record happens to call. The distinction is the point of the file: a
//! `RedactionConfig::redact` that is correct for nested objects and arrays —
//! with green unit tests — is still no evidence about what gets written, when
//! the middleware and the logger hold two different rule sets and each redacts
//! a different part of one record.
//!
//! That is exactly how apcore-typescript leaked. Its middleware redacted
//! `inputs` with the caller's config through a FLAT helper, and its
//! `ContextLogger` then walked the whole record recursively under
//! `RedactionConfig.default()` — a rule set carrying no `regex_patterns` — so
//! `{"items": ["sk-…"]}` and `{"nested": {"key": "sk-…"}}` were written in
//! plaintext under precisely the wiring `docs/features/observability.md`
//! documents.
//!
//! This SDK has no such split: `apply_redaction` uses the middleware's own
//! config and calls the RECURSIVE `redact`, and the logger performs no second
//! configured pass. These cases pin that arrangement at the surface where the
//! other SDK broke, so the three are asserted alike.

use apcore::middleware::Middleware;
use apcore::observability::logging::{ContextLogger, LogFormat, ObsLoggingMiddleware};
use apcore::observability::redaction::RedactionConfig;
use apcore::Context;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const SECRET: &str = "sk-abcdef123456";

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

/// A middleware writing into an in-memory buffer, plus the buffer.
fn harness(config: Option<RedactionConfig>) -> (ObsLoggingMiddleware, Arc<Mutex<Vec<u8>>>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let mut logger = ContextLogger::new("test");
    logger.set_format(LogFormat::Json);
    logger.set_writer(Box::new(CapturingWriter(buf.clone())));
    let mw = ObsLoggingMiddleware::new(logger);
    let mw = match config {
        Some(cfg) => mw.with_redaction_config(cfg),
        None => mw,
    };
    (mw, buf)
}

fn records(buf: &Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
    let raw = buf.lock().unwrap();
    String::from_utf8_lossy(&raw)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("logger output must be valid JSON"))
        .collect()
}

fn value_rule() -> RedactionConfig {
    RedactionConfig::builder()
        .sensitive_keys(Vec::<String>::new())
        .value_patterns(["sk-[A-Za-z0-9]{6,}"])
        .try_build()
        .expect("the probe pattern compiles")
}

fn payload() -> Value {
    json!({ "top": SECRET, "items": [SECRET], "nested": { "key": SECRET } })
}

fn ctx() -> Context<Value> {
    Context::create(None, None, None, None, Value::Null, None)
}

#[tokio::test]
async fn a_logged_input_is_redacted_at_every_position() {
    let (mw, buf) = harness(Some(value_rule()));
    mw.before("executor.x.y", payload(), &ctx())
        .await
        .expect("the middleware does not fail the call");

    let recs = records(&buf);
    let inputs = &recs[0]["extra"]["inputs"];
    assert_eq!(inputs["top"], json!("***REDACTED***"));
    // The two apcore-typescript leaked. Neither is reachable by a flat,
    // one-level helper — only by a recursive walk holding the right rules.
    assert_eq!(inputs["items"], json!(["***REDACTED***"]));
    assert_eq!(inputs["nested"], json!({ "key": "***REDACTED***" }));
}

#[tokio::test]
async fn a_logged_output_is_redacted_at_every_position() {
    let (mw, buf) = harness(Some(value_rule()));
    let c = ctx();
    mw.before("executor.x.y", json!({}), &c)
        .await
        .expect("before");
    mw.after("executor.x.y", json!({}), payload(), &c)
        .await
        .expect("after");

    let recs = records(&buf);
    let output = &recs[1]["extra"]["output"];
    assert_eq!(output["top"], json!("***REDACTED***"));
    assert_eq!(output["items"], json!(["***REDACTED***"]));
    assert_eq!(output["nested"], json!({ "key": "***REDACTED***" }));
}

#[tokio::test]
async fn a_narrowed_rule_set_is_not_widened_again_by_a_logger_default() {
    // The reverse error the same split produces elsewhere: an operator writing
    // `sensitive_keys: []` has disabled key-based redaction, and a logger
    // holding its own shipped default would apply it underneath anyway.
    let empty = RedactionConfig::builder()
        .sensitive_keys(Vec::<String>::new())
        .value_patterns(Vec::<String>::new())
        .try_build()
        .expect("an empty rule set is valid");
    let (mw, buf) = harness(Some(empty));
    mw.before("executor.x.y", json!({ "password": "hunter2" }), &ctx())
        .await
        .expect("before");

    assert_eq!(
        records(&buf)[0]["extra"]["inputs"]["password"],
        json!("hunter2")
    );
}

#[tokio::test]
async fn correlation_fields_survive_a_rule_that_would_match_them() {
    let all = RedactionConfig::builder()
        .sensitive_keys(Vec::<String>::new())
        .value_patterns([".*"])
        .try_build()
        .expect("compiles");
    let (mw, buf) = harness(Some(all));
    mw.before("executor.x.y", json!({}), &ctx())
        .await
        .expect("before");

    assert_eq!(
        records(&buf)[0]["extra"]["module_id"],
        json!("executor.x.y")
    );
}
