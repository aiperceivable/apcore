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
struct Login {
    declares_schema: bool,
}

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
        if self.declares_schema {
            input_schema()
        } else {
            json!({})
        }
    }
    fn output_schema(&self) -> Value {
        if self.declares_schema {
            output_schema()
        } else {
            json!({})
        }
    }
    fn description(&self) -> &'static str {
        "returns a session for a passphrase"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let field = if self.declares_schema {
            "session"
        } else {
            "_secret_session"
        };
        Ok(json!({"user": inputs["username"], (field): OUTPUT_SECRET}))
    }
}

fn executor() -> Executor {
    executor_with_schema(true)
}

fn executor_with_schema(declares_schema: bool) -> Executor {
    let reg = Arc::new(Registry::new());
    let login = Login { declares_schema };
    let descriptor = ModuleDescriptor {
        module_id: MODULE_ID.to_string(),
        name: None,
        description: "returns a session for a passphrase".to_string(),
        documentation: None,
        input_schema: login.input_schema(),
        output_schema: login.output_schema(),
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
    reg.register(MODULE_ID, Box::new(login), descriptor)
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

#[tokio::test]
async fn test_logging_schema_independent_redaction() {
    let buf = Buf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let executor = executor_with_schema(false);
    executor
        .use_middleware(Box::new(LoggingMiddleware::with_defaults()))
        .unwrap();

    executor
        .call(
            MODULE_ID,
            json!({"username": "alice", "_secret_passphrase": INPUT_SECRET}),
            None,
            None,
        )
        .await
        .unwrap();

    assert_redacted(&buf.text());
}

#[tokio::test]
async fn test_executor_warns_once_about_ignored_version_hints_across_call_and_stream() {
    use futures_util::StreamExt;

    let buf = Buf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let executor = executor();
    executor
        .call(MODULE_ID, inputs(), None, None)
        .await
        .unwrap();
    assert!(!buf.text().contains("version_hint is deprecated"));
    executor
        .call(MODULE_ID, inputs(), None, Some("1.0.0"))
        .await
        .unwrap();
    let mut result = executor.stream(MODULE_ID, inputs(), None, Some("2.0.0"));
    assert!(result.next().await.unwrap().is_ok());
    assert_eq!(buf.text().matches("version_hint is deprecated").count(), 1);
}
