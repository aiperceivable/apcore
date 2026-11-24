//! D-131 regression tests (PROTOCOL_SPEC §10.6.1 "Where the rules apply",
//! requirement 5): the built-in logging middlewares log `context.redacted_inputs`
//! / `context.redacted_output`, never the raw values they are handed, so an
//! `x-sensitive` field is redacted in the log line whether or not a
//! `RedactionConfig` was supplied.
//!
//! Each test runs a real call through a real `Executor` — the capture point that
//! fills the redacted fields is a pipeline step, so a middleware driven by hand
//! would test a context the pipeline never produces.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::executor::{Executor, REDACTED_VALUE};
use apcore::middleware::LoggingMiddleware;
use apcore::module::Module;
use apcore::observability::logging::{ContextLogger, ObsLoggingMiddleware};
use apcore::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use async_trait::async_trait;
use serde_json::{json, Value};

const MODULE_ID: &str = "demo.login";
const INPUT_SECRET: &str = "hunter2-input-secret";
const OUTPUT_SECRET: &str = "tok-output-secret";

#[derive(Debug)]
struct Login;

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "username": {"type": "string"},
            "passphrase": {"type": "string", "x-sensitive": true}
        }
    })
}

fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "user": {"type": "string"},
            "session": {"type": "string", "x-sensitive": true}
        }
    })
}

#[async_trait]
impl Module for Login {
    fn input_schema(&self) -> Value {
        input_schema()
    }
    fn output_schema(&self) -> Value {
        output_schema()
    }
    fn description(&self) -> &'static str {
        "returns a session for a passphrase"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"user": inputs["username"], "session": OUTPUT_SECRET}))
    }
}

fn executor() -> Executor {
    let reg = Arc::new(Registry::new());
    let descriptor = ModuleDescriptor {
        module_id: MODULE_ID.to_string(),
        name: None,
        description: "returns a session for a passphrase".to_string(),
        documentation: None,
        input_schema: input_schema(),
        output_schema: output_schema(),
        version: DEFAULT_MODULE_VERSION.to_string(),
        tags: vec![],
        annotations: None,
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    };
    reg.register(MODULE_ID, Box::new(Login), descriptor)
        .expect("register");
    Executor::new(reg, Arc::new(Config::default()))
}

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Buf {
    fn text(&self) -> String {
        String::from_utf8_lossy(
            &self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .into_owned()
    }
}

impl std::io::Write for Buf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for Buf {
    type Writer = Self;
    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

fn inputs() -> Value {
    json!({"username": "alice", "passphrase": INPUT_SECRET})
}

fn assert_redacted(log: &str) {
    assert!(
        !log.contains(INPUT_SECRET),
        "x-sensitive input leaked: {log}"
    );
    assert!(
        !log.contains(OUTPUT_SECRET),
        "x-sensitive output leaked: {log}"
    );
    assert!(
        log.matches(REDACTED_VALUE).count() >= 2,
        "both the input and the output field carry the marker: {log}"
    );
    assert!(
        log.contains("alice"),
        "non-sensitive fields are still logged: {log}"
    );
}

/// `ObsLoggingMiddleware` with NO `RedactionConfig`: the schema's
/// `x-sensitive` rule still applies, because it logs the captured values.
#[tokio::test]
async fn obs_logging_middleware_logs_redacted_values_without_config() {
    let buf = Buf::default();
    let mut logger = ContextLogger::new("test");
    logger.set_writer(Box::new(buf.clone()));
    let executor = executor();
    executor
        .use_middleware(Box::new(ObsLoggingMiddleware::new(logger)))
        .expect("add middleware");

    let out = executor
        .call(MODULE_ID, inputs(), None, None)
        .await
        .expect("call succeeds");
    // The caller still receives the real value; only the log is redacted.
    assert_eq!(out["session"], OUTPUT_SECRET);

    assert_redacted(&buf.text());
}

#[tokio::test]
async fn logging_middleware_logs_redacted_values() {
    let buf = Buf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let executor = executor();
    executor
        .use_middleware(Box::new(LoggingMiddleware::with_defaults()))
        .expect("add middleware");
    executor
        .call(MODULE_ID, inputs(), None, None)
        .await
        .expect("call succeeds");

    let log = buf.text();
    assert!(log.contains("START demo.login"), "middleware logged: {log}");
    assert_redacted(&log);
}
