// APCore Protocol — Context-aware logging
// Spec reference: Structured logging with execution context.
//
// Sync finding D-28 — schema alignment with apcore-python and apcore-typescript:
//   1. The emitted `level` field is the lowercase level name ("info"), not
//      uppercase ("INFO"). Cross-language log shipping pipelines key off this
//      field; an uppercase outlier breaks dashboards.
//   2. User-supplied extras are nested under a single `extra` key rather
//      than flattened to top-level. This prevents user keys from colliding
//      with the canonical fields (`level`, `timestamp`, `trace_id`, ...).
//   3. The obs-logging middleware emits `module_id` (not `module`) and
//      `inputs` (not `input`) — both names are protocol-canonical.

use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::io::Write;

use parking_lot::Mutex;

use crate::context::Context;
use crate::errors::ModuleError;
use crate::executor::loggable_value;
use crate::middleware::base::Middleware;
use crate::observability::redaction::RedactionConfig;

/// Log level numeric values matching Python reference.
fn level_value(level: &str) -> u32 {
    match level.to_lowercase().as_str() {
        "trace" => 0,
        "debug" => 10,
        "warn" | "warning" => 30,
        "error" => 40,
        "fatal" => 50,
        _ => 20, // info and unknown levels default to 20
    }
}

/// Output format for log records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Json,
    Text,
}

/// Trait object the logger writes to. Defaults to stderr; tests can substitute
/// an in-memory buffer to inspect emitted records without touching stderr.
type LoggerWriter = Box<dyn Write + Send + Sync>;

/// Logger that injects execution context into log records.
pub struct ContextLogger {
    pub name: String,
    pub level: String,
    pub format: LogFormat,
    pub trace_id: Option<String>,
    pub module_id: Option<String>,
    pub caller_id: Option<String>,
    /// Rules applied to `extra` before it is written. Defaults to
    /// [`RedactionConfig::defaults`], as apcore-python and apcore-typescript
    /// do for a logger used directly rather than through the middleware.
    redaction: RedactionConfig,
    /// Output sink. `None` ⇒ stderr (default). Writes are mutex-guarded so
    /// `ContextLogger` remains `Send + Sync` even with a mutable writer.
    writer: Option<Mutex<LoggerWriter>>,
}

impl std::fmt::Debug for ContextLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextLogger")
            .field("name", &self.name)
            .field("level", &self.level)
            .field("format", &self.format)
            .field("trace_id", &self.trace_id)
            .field("module_id", &self.module_id)
            .field("caller_id", &self.caller_id)
            .field("redaction", &self.redaction)
            .field(
                "writer",
                &self.writer.as_ref().map_or("stderr", |_| "<custom>"),
            )
            .finish()
    }
}

impl ContextLogger {
    /// Create a new context logger with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            level: "info".to_string(),
            format: LogFormat::Json,
            trace_id: None,
            module_id: None,
            caller_id: None,
            redaction: RedactionConfig::defaults(),
            writer: None,
        }
    }

    /// Create a logger with context derived from execution context.
    pub fn from_context(ctx: &Context<serde_json::Value>, name: impl Into<String>) -> Self {
        let module_id = ctx.call_chain.last().cloned();
        let caller_id = ctx.caller_id.clone();
        Self {
            name: name.into(),
            level: "info".to_string(),
            format: LogFormat::Json,
            trace_id: Some(ctx.trace_id.clone()),
            module_id,
            caller_id,
            redaction: RedactionConfig::defaults(),
            writer: None,
        }
    }

    /// Set the minimum log level.
    pub fn set_level(&mut self, level: impl Into<String>) {
        self.level = level.into();
    }

    /// Set the output format.
    pub fn set_format(&mut self, format: LogFormat) {
        self.format = format;
    }

    /// Replace the redaction rules applied to `extra` — e.g.
    /// `RedactionConfig::from_config(&config)` for the configured
    /// `obs.redaction.*` rules, or `RedactionConfig::new()` for none.
    pub fn set_redaction_config(&mut self, config: RedactionConfig) {
        self.redaction = config;
    }

    /// Substitute the output sink (default: stderr). Useful for tests that
    /// need to inspect emitted records.
    pub fn set_writer(&mut self, writer: LoggerWriter) {
        self.writer = Some(Mutex::new(writer));
    }

    fn write_line(&self, line: &str) {
        if let Some(ref w) = self.writer {
            let mut guard = w.lock();
            let _ = writeln!(*guard, "{line}");
        } else {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
    }

    /// Emit a log record if level meets threshold.
    ///
    /// `extra` keys go under a nested `extra` object (D-28), redacted with
    /// this logger's rules at every depth; `trace_id`, `caller_id`,
    /// `module_id` and `span_id` are never redacted.
    pub fn emit(
        &self,
        level_name: &str,
        message: &str,
        extra: Option<&HashMap<String, serde_json::Value>>,
    ) {
        let threshold = level_value(&self.level);
        let msg_level = level_value(level_name);
        if msg_level < threshold {
            return;
        }

        match self.format {
            LogFormat::Json => {
                let mut record = serde_json::Map::new();
                record.insert(
                    "timestamp".to_string(),
                    serde_json::Value::String(Utc::now().to_rfc3339()),
                );
                // D-28: lowercase level name (matches Python+TS).
                record.insert(
                    "level".to_string(),
                    serde_json::Value::String(level_name.to_lowercase()),
                );
                record.insert(
                    "logger".to_string(),
                    serde_json::Value::String(self.name.clone()),
                );
                record.insert(
                    "message".to_string(),
                    serde_json::Value::String(message.to_string()),
                );
                if let Some(ref trace_id) = self.trace_id {
                    record.insert(
                        "trace_id".to_string(),
                        serde_json::Value::String(trace_id.clone()),
                    );
                }
                if let Some(ref module_id) = self.module_id {
                    record.insert(
                        "module_id".to_string(),
                        serde_json::Value::String(module_id.clone()),
                    );
                }
                if let Some(ref caller_id) = self.caller_id {
                    record.insert(
                        "caller_id".to_string(),
                        serde_json::Value::String(caller_id.clone()),
                    );
                }
                // D-28: nest user-supplied extras under a single `extra` key
                // so they cannot collide with the canonical top-level fields.
                if let Some(extra_map) = extra {
                    let mut nested = serde_json::Value::Object(
                        extra_map
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    );
                    self.redaction.redact(&mut nested);
                    record.insert("extra".to_string(), nested);
                }
                let json_str =
                    serde_json::to_string(&serde_json::Value::Object(record)).unwrap_or_default();
                self.write_line(&json_str);
            }
            LogFormat::Text => {
                let ts = Utc::now().to_rfc3339();
                let ctx_str = match (&self.trace_id, &self.module_id) {
                    (Some(tid), Some(mid)) => format!(" [trace={tid} module={mid}]"),
                    (Some(tid), None) => format!(" [trace={tid}]"),
                    (None, Some(mid)) => format!(" [module={mid}]"),
                    (None, None) => String::new(),
                };
                self.write_line(&format!(
                    "{} {} {}{} {}",
                    ts,
                    level_name.to_uppercase(),
                    self.name,
                    ctx_str,
                    message
                ));
            }
        }
    }

    /// Log a trace message.
    pub fn trace(&self, message: &str) {
        self.emit("trace", message, None);
    }

    /// Log a debug message.
    pub fn debug(&self, message: &str) {
        self.emit("debug", message, None);
    }

    /// Log an info message.
    pub fn info(&self, message: &str) {
        self.emit("info", message, None);
    }

    /// Log a warning message.
    pub fn warn(&self, message: &str) {
        self.emit("warn", message, None);
    }

    /// Log a warning message (alias).
    pub fn warning(&self, message: &str) {
        self.emit("warn", message, None);
    }

    /// Log an error message.
    pub fn error(&self, message: &str) {
        self.emit("error", message, None);
    }

    /// Log a fatal message.
    pub fn fatal(&self, message: &str) {
        self.emit("fatal", message, None);
    }
}

/// Middleware that logs before/after execution.
///
/// WARNING: The internal start-time stack is not safe for concurrent use on
/// the same middleware instance. Use separate instances per concurrent pipeline.
///
/// Inputs and output are logged from `context.redacted_inputs` /
/// `context.redacted_output` — the values the executor's capture point
/// already redacted with the schema's `x-sensitive` rule and the configured
/// rules — never from the raw values the hook receives (PROTOCOL_SPEC §10.6.1
/// requirement 5). When constructed with [`Self::with_redaction_config`], the
/// supplied `RedactionConfig` is applied on top: any field/value matched by
/// EITHER rule set is replaced before being logged. `trace_id`, `caller_id`, and `module_id`
/// are never redacted (correlation-required fields).
#[derive(Debug)]
pub struct ObsLoggingMiddleware {
    logger: ContextLogger,
    log_inputs: bool,
    log_outputs: bool,
    redaction: Option<RedactionConfig>,
    starts: Mutex<HashMap<String, std::time::Instant>>,
}

impl ObsLoggingMiddleware {
    /// Create a new logging middleware.
    #[must_use]
    pub fn new(logger: ContextLogger) -> Self {
        Self {
            logger,
            log_inputs: true,
            log_outputs: true,
            redaction: None,
            starts: Mutex::new(HashMap::new()),
        }
    }

    /// Create with explicit input/output logging flags.
    #[must_use]
    pub fn with_options(logger: ContextLogger, log_inputs: bool, log_outputs: bool) -> Self {
        Self {
            logger,
            log_inputs,
            log_outputs,
            redaction: None,
            starts: Mutex::new(HashMap::new()),
        }
    }

    /// Attach a runtime-configurable redaction policy. Applied as a union
    /// with any schema-level `x-sensitive` redaction performed upstream.
    #[must_use]
    pub fn with_redaction_config(mut self, config: RedactionConfig) -> Self {
        // The logger's own pass applies the same rules, not its broader
        // defaults — matching apcore-python, which hands the config to both.
        self.logger.set_redaction_config(config.clone());
        self.redaction = Some(config);
        self
    }

    /// Apply the configured `RedactionConfig` (if any) to a JSON value in
    /// place. No-op when no config is attached.
    fn apply_redaction(&self, value: &mut serde_json::Value) {
        if let Some(ref cfg) = self.redaction {
            cfg.redact(value);
        }
    }
}

#[async_trait]
impl Middleware for ObsLoggingMiddleware {
    fn name(&self) -> &'static str {
        "logging"
    }

    async fn before(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        // Store start time keyed by trace_id for concurrency safety
        {
            let mut starts = self.starts.lock();
            starts.insert(ctx.trace_id.clone(), std::time::Instant::now());
        }

        let mut extra = HashMap::new();
        // D-28: protocol-canonical names are `module_id` / `inputs`.
        extra.insert(
            "module_id".to_string(),
            serde_json::Value::String(module_id.to_string()),
        );
        extra.insert(
            "trace_id".to_string(),
            serde_json::Value::String(ctx.trace_id.clone()),
        );
        if let Some(ref cid) = ctx.caller_id {
            extra.insert(
                "caller_id".to_string(),
                serde_json::Value::String(cid.clone()),
            );
        }
        if self.log_inputs {
            // D-131: the captured `redacted_inputs`, never the raw inputs —
            // the schema's `x-sensitive` rule applies whether or not this
            // middleware was given a `RedactionConfig`.
            let mut payload = loggable_value(
                ctx.redacted_inputs.as_ref(),
                &inputs,
                self.redaction.as_ref(),
            );
            self.apply_redaction(&mut payload);
            extra.insert("inputs".to_string(), payload);
        }
        self.logger
            .emit("info", &format!("Starting {module_id}"), Some(&extra));

        Ok(None)
    }

    async fn after(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        // Remove start time by trace_id and compute duration
        let duration_ms = {
            let mut starts = self.starts.lock();
            starts
                .remove(&ctx.trace_id)
                .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0)
        };

        let mut extra = HashMap::new();
        extra.insert(
            "module_id".to_string(),
            serde_json::Value::String(module_id.to_string()),
        );
        extra.insert(
            "trace_id".to_string(),
            serde_json::Value::String(ctx.trace_id.clone()),
        );
        extra.insert("duration_ms".to_string(), serde_json::json!(duration_ms));
        if self.log_outputs {
            // D-131: the captured `redacted_output`, never the raw output.
            let mut payload = loggable_value(
                ctx.redacted_output.as_ref(),
                &output,
                self.redaction.as_ref(),
            );
            self.apply_redaction(&mut payload);
            extra.insert("output".to_string(), payload);
        }
        self.logger.emit(
            "info",
            &format!("Completed {module_id} in {duration_ms:.2}ms"),
            Some(&extra),
        );

        Ok(None)
    }

    async fn on_error(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        error: &ModuleError,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        // Remove start time by trace_id and compute duration
        let duration_ms = {
            let mut starts = self.starts.lock();
            starts
                .remove(&ctx.trace_id)
                .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0)
        };

        let mut extra = HashMap::new();
        extra.insert(
            "module_id".to_string(),
            serde_json::Value::String(module_id.to_string()),
        );
        extra.insert(
            "trace_id".to_string(),
            serde_json::Value::String(ctx.trace_id.clone()),
        );
        extra.insert("duration_ms".to_string(), serde_json::json!(duration_ms));
        extra.insert(
            "error".to_string(),
            serde_json::Value::String(error.message.clone()),
        );
        // ERR-001: a field named `error_code` carries the protocol's canonical
        // wire code (SCREAMING_SNAKE), never the Rust enum's `Debug` name —
        // the same rule the metrics label and the tracing span attribute follow.
        extra.insert(
            "error_code".to_string(),
            serde_json::Value::String(error.code.wire_str()),
        );
        self.logger.emit(
            "error",
            &format!(
                "Error in {} after {:.2}ms: {}",
                module_id, duration_ms, error.message
            ),
            Some(&extra),
        );

        Ok(None)
    }
}
