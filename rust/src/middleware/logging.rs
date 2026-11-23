// APCore Protocol — Logging middleware
// Spec reference: Middleware lifecycle with structured logging and redaction

use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;

use super::base::Middleware;
use crate::context::Context;
use crate::errors::ModuleError;
use crate::executor::loggable_value;

/// Structured logging middleware with security-aware redaction.
///
/// Logs module call start, completion (with duration), and errors from
/// `context.redacted_inputs` / `context.redacted_output`, never the raw values
/// (PROTOCOL_SPEC §10.6.1 requirement 5). Thread-safe
/// by storing per-call timing via context data markers and interior
/// atomics — no mutable self required.
#[derive(Debug)]
pub struct LoggingMiddleware {
    log_inputs: bool,
    log_outputs: bool,
    log_errors: bool,
    /// Per-call start times indexed by a nonce stored in context.data.
    /// Using a concurrent map keyed by `trace_id` + `module_id` to stay thread-safe.
    start_times: parking_lot::Mutex<std::collections::HashMap<String, Instant>>,
}

impl LoggingMiddleware {
    /// Create a new logging middleware with the given configuration flags.
    #[must_use]
    pub fn new(log_inputs: bool, log_outputs: bool, log_errors: bool) -> Self {
        Self {
            log_inputs,
            log_outputs,
            log_errors,
            start_times: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Create with all logging flags enabled (default).
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(true, true, true)
    }

    /// Build a key for the start-time map from context and `module_id`.
    fn timing_key(module_id: &str, ctx: &Context<Value>) -> String {
        format!("{}:{}", ctx.trace_id, module_id)
    }
}

impl Default for LoggingMiddleware {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[async_trait]
impl Middleware for LoggingMiddleware {
    fn name(&self) -> &'static str {
        "logging"
    }

    fn priority(&self) -> u16 {
        // Spec recommends 700-799 for logging middleware.
        700
    }

    async fn before(
        &self,
        module_id: &str,
        inputs: Value,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Record start time in our interior map.
        let key = Self::timing_key(module_id, ctx);
        {
            let mut times = self.start_times.lock();
            times.insert(key, Instant::now());
        }

        if self.log_inputs {
            // D-131: the captured `redacted_inputs`, never the raw inputs.
            let display_inputs = loggable_value(ctx.redacted_inputs.as_ref(), &inputs, None);

            tracing::info!(
                trace_id = %ctx.trace_id,
                module_id = module_id,
                caller_id = ?ctx.caller_id,
                inputs = %display_inputs,
                "START {}",
                module_id,
            );
        }

        Ok(None)
    }

    async fn after(
        &self,
        module_id: &str,
        _inputs: Value,
        output: Value,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        let key = Self::timing_key(module_id, ctx);
        let duration_ms = {
            let mut times = self.start_times.lock();
            times
                .remove(&key)
                .map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0)
        };

        if self.log_outputs {
            // D-131: the captured `redacted_output`, never the raw output.
            let display_output = loggable_value(ctx.redacted_output.as_ref(), &output, None);
            tracing::info!(
                trace_id = %ctx.trace_id,
                module_id = module_id,
                duration_ms = duration_ms,
                output = %display_output,
                "END {} ({:.2}ms)",
                module_id,
                duration_ms,
            );
        }

        Ok(None)
    }

    async fn on_error(
        &self,
        module_id: &str,
        inputs: Value,
        error: &ModuleError,
        ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        // Clean up timing entry if present.
        let key = Self::timing_key(module_id, ctx);
        {
            let mut times = self.start_times.lock();
            times.remove(&key);
        }

        if self.log_errors {
            // D-131: the captured `redacted_inputs`, never the raw inputs.
            let display_inputs = loggable_value(ctx.redacted_inputs.as_ref(), &inputs, None);

            tracing::error!(
                trace_id = %ctx.trace_id,
                module_id = module_id,
                error_code = ?error.code,
                error_message = %error.message,
                inputs = %display_inputs,
                "ERROR {}: {}",
                module_id,
                error.message,
            );
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Context, Identity};

    fn test_ctx() -> Context<Value> {
        let identity = Identity::new(
            "test-user".to_string(),
            "user".to_string(),
            vec![],
            std::collections::HashMap::new(),
        );
        Context::new(identity)
    }

    #[tokio::test]
    async fn test_logging_middleware_name_and_priority() {
        let mw = LoggingMiddleware::with_defaults();
        assert_eq!(mw.name(), "logging");
        assert_eq!(mw.priority(), 700);
    }

    #[tokio::test]
    async fn test_logging_middleware_before_returns_none() {
        let mw = LoggingMiddleware::with_defaults();
        let ctx = test_ctx();
        let result = mw
            .before("test.module", serde_json::json!({"key": "value"}), &ctx)
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_logging_middleware_after_returns_none() {
        let mw = LoggingMiddleware::with_defaults();
        let ctx = test_ctx();
        // Call before first to record start time.
        let _ = mw.before("test.module", serde_json::json!({}), &ctx).await;
        let result = mw
            .after(
                "test.module",
                serde_json::json!({}),
                serde_json::json!({"result": 42}),
                &ctx,
            )
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_logging_middleware_on_error_returns_none() {
        let mw = LoggingMiddleware::with_defaults();
        let ctx = test_ctx();
        let error = ModuleError::new(
            crate::errors::ErrorCode::ModuleExecuteError,
            "test error".to_string(),
        );
        let result = mw
            .on_error("test.module", serde_json::json!({}), &error, &ctx)
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_logging_middleware_with_disabled_flags() {
        let mw = LoggingMiddleware::new(false, false, false);
        let ctx = test_ctx();

        // All hooks should still succeed even with logging disabled.
        let before = mw.before("test.module", serde_json::json!({}), &ctx).await;
        assert!(before.is_ok());

        let after = mw
            .after(
                "test.module",
                serde_json::json!({}),
                serde_json::json!({}),
                &ctx,
            )
            .await;
        assert!(after.is_ok());

        let error = ModuleError::new(
            crate::errors::ErrorCode::ModuleExecuteError,
            "err".to_string(),
        );
        let on_err = mw
            .on_error("test.module", serde_json::json!({}), &error, &ctx)
            .await;
        assert!(on_err.is_ok());
    }

    #[test]
    fn test_logging_middleware_default() {
        let mw = LoggingMiddleware::default();
        assert!(mw.log_inputs);
        assert!(mw.log_outputs);
        assert!(mw.log_errors);
    }
}
