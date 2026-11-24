// APCore Protocol — Retry middleware
// Spec reference: Automatic retry with configurable backoff

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use super::base::Middleware;
use crate::context::Context;
use crate::context_keys::RETRY_COUNT_BASE;
use crate::errors::ModuleError;

/// Configuration for retry behavior.
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RetryConfig {
    pub max_retries: u32,
    /// Backoff strategy: "exponential" or "fixed".
    #[serde(default = "default_strategy")]
    pub strategy: String,
    /// Base delay in milliseconds.
    #[serde(default = "default_base_delay")]
    pub base_delay_ms: u64,
    /// Maximum delay in milliseconds (cap for exponential backoff).
    #[serde(default = "default_max_delay")]
    pub max_delay_ms: u64,
    /// Whether to add random jitter to delays.
    #[serde(default = "default_jitter")]
    pub jitter: bool,
}

fn default_strategy() -> String {
    "exponential".to_string()
}
fn default_base_delay() -> u64 {
    100
}
fn default_max_delay() -> u64 {
    5000
}
fn default_jitter() -> bool {
    true
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            strategy: "exponential".to_string(),
            base_delay_ms: 100,
            max_delay_ms: 5000,
            jitter: true,
        }
    }
}

/// Middleware that retries failed executions according to `RetryConfig`.
///
/// When `on_error` is called with a retryable error (`error.retryable == Some(true)`),
/// this middleware sleeps for a calculated delay and returns `Some(inputs)` to signal
/// the pipeline to retry execution. After `max_retries` attempts or for non-retryable
/// errors, it returns `None` so the error propagates.
///
/// Retry state is tracked per-call in `context.data` under the
/// `_apcore.mw.retry.count.{module_id}` key (see [`RETRY_COUNT_BASE`]),
/// matching apcore-python / apcore-typescript. Storing the counter in the
/// per-call context (rather than a shared instance map) keeps concurrent
/// invocations of the same module isolated and makes the count visible to
/// peer middleware on the same context.
#[derive(Debug)]
pub struct RetryMiddleware {
    pub config: RetryConfig,
}

impl RetryMiddleware {
    /// Create a new retry middleware with the given config.
    #[must_use]
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Read the current retry count for `module_id` from `context.data`.
    fn retry_count(ctx: &Context<serde_json::Value>, module_id: &str) -> u32 {
        RETRY_COUNT_BASE
            .scoped(module_id)
            .get(ctx)
            .and_then(|n: i64| u32::try_from(n).ok())
            .unwrap_or(0)
    }

    /// Store `count` as the retry count for `module_id` in `context.data`.
    fn set_retry_count(ctx: &Context<serde_json::Value>, module_id: &str, count: u32) {
        RETRY_COUNT_BASE
            .scoped(module_id)
            .set(ctx, i64::from(count));
    }

    /// Calculate delay in milliseconds for the given attempt number.
    ///
    /// - Exponential: `base_delay_ms * 2^attempt`, capped at `max_delay_ms`.
    /// - Fixed: `base_delay_ms`.
    /// - With jitter: multiply by a factor in [0.5, 1.5).
    fn calculate_delay(&self, attempt: u32) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        // intentional: delay values are small (ms) and won't lose meaningful precision
        let delay = if self.config.strategy == "fixed" {
            self.config.base_delay_ms as f64
        } else {
            // Exponential backoff
            #[allow(clippy::cast_possible_wrap)] // attempt won't exceed i32::MAX in practice
            let exp_delay = self.config.base_delay_ms as f64 * 2.0_f64.powi(attempt as i32);
            #[allow(clippy::cast_precision_loss)]
            // intentional: delay values are small (ms) and won't lose meaningful precision
            let max = self.config.max_delay_ms as f64;
            exp_delay.min(max)
        };

        if self.config.jitter {
            // Simple jitter using system time nanos as pseudo-random source.
            // Produces a factor in [0.5, 1.5) matching Python's random.uniform(0.5, 1.5).
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();
            let factor = 0.5 + (f64::from(nanos) % 1_000_000.0) / 1_000_000.0;
            delay * factor
        } else {
            delay
        }
    }
}

#[async_trait]
impl Middleware for RetryMiddleware {
    fn name(&self) -> &'static str {
        "retry"
    }

    async fn before(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }

    async fn after(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        _output: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        // Reset retry count on successful execution so retries don't persist
        // across separate call sequences.
        RETRY_COUNT_BASE.scoped(module_id).delete(ctx);
        Ok(None)
    }

    async fn on_error(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        error: &ModuleError,
        ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        // Only retry if the error is explicitly marked as retryable.
        if error.retryable != Some(true) {
            return Ok(None);
        }

        let retry_count = Self::retry_count(ctx, module_id);

        if retry_count >= self.config.max_retries {
            tracing::warn!(
                "Max retries ({}) exceeded for module '{}'",
                self.config.max_retries,
                module_id,
            );
            return Ok(None);
        }

        let delay_ms = self.calculate_delay(retry_count);

        tracing::info!(
            "Retrying module '{}' (attempt {}/{}) after {:.0}ms",
            module_id,
            retry_count + 1,
            self.config.max_retries,
            delay_ms,
        );

        // Sleep for the calculated delay.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // delay_ms is non-negative and won't exceed u64::MAX
        let delay_ms_u64 = delay_ms as u64;
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms_u64)).await;

        // Increment the retry count in context.data.
        Self::set_retry_count(ctx, module_id, retry_count + 1);

        // Return the original inputs to signal retry.
        Ok(Some(inputs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;
    use serde_json::json;

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.strategy, "exponential");
        assert_eq!(config.base_delay_ms, 100);
        assert_eq!(config.max_delay_ms, 5000);
        assert!(config.jitter);
    }

    #[test]
    fn test_retry_config_serde_defaults() {
        let config: RetryConfig = serde_json::from_str(r#"{"max_retries": 5}"#).unwrap();
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.strategy, "exponential");
        assert_eq!(config.base_delay_ms, 100);
    }

    #[test]
    fn test_calculate_delay_fixed_no_jitter() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            strategy: "fixed".to_string(),
            base_delay_ms: 200,
            max_delay_ms: 5000,
            jitter: false,
        });
        assert!((mw.calculate_delay(0) - 200.0).abs() < f64::EPSILON);
        assert!((mw.calculate_delay(1) - 200.0).abs() < f64::EPSILON);
        assert!((mw.calculate_delay(5) - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_delay_exponential_no_jitter() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 5,
            strategy: "exponential".to_string(),
            base_delay_ms: 100,
            max_delay_ms: 5000,
            jitter: false,
        });
        assert!((mw.calculate_delay(0) - 100.0).abs() < f64::EPSILON);
        assert!((mw.calculate_delay(1) - 200.0).abs() < f64::EPSILON);
        assert!((mw.calculate_delay(2) - 400.0).abs() < f64::EPSILON);
        assert!((mw.calculate_delay(3) - 800.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_delay_exponential_capped() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 10,
            strategy: "exponential".to_string(),
            base_delay_ms: 100,
            max_delay_ms: 500,
            jitter: false,
        });
        assert!((mw.calculate_delay(4) - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_delay_with_jitter_in_range() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            strategy: "fixed".to_string(),
            base_delay_ms: 1000,
            max_delay_ms: 5000,
            jitter: true,
        });
        for _ in 0..20 {
            let delay = mw.calculate_delay(0);
            assert!(delay >= 500.0, "delay {delay} should be >= 500");
            assert!(delay < 1500.0, "delay {delay} should be < 1500");
        }
    }

    #[test]
    fn test_retry_middleware_name() {
        let mw = RetryMiddleware::new(RetryConfig::default());
        assert_eq!(mw.name(), "retry");
    }

    #[tokio::test]
    async fn test_before_returns_none() {
        let mw = RetryMiddleware::new(RetryConfig::default());
        let ctx = Context::<serde_json::Value>::anonymous();
        let result = mw.before("test.mod", json!({"a": 1}), &ctx).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_after_resets_retry_count() {
        let mw = RetryMiddleware::new(RetryConfig::default());
        let ctx = Context::<serde_json::Value>::anonymous();
        RetryMiddleware::set_retry_count(&ctx, "mod.a", 2);
        mw.after("mod.a", json!({}), json!({}), &ctx).await.unwrap();
        assert_eq!(RetryMiddleware::retry_count(&ctx, "mod.a"), 0);
        assert!(!RETRY_COUNT_BASE.scoped("mod.a").exists(&ctx));
    }

    /// The gate is `retryable != Some(true)`, so BOTH non-retryable shapes must
    /// be refused: a code the policy resolves to `Some(false)`, and a code it
    /// leaves `None` for the module author.
    ///
    /// Before apcore#36 every `ModuleError::new` carried `None`, so a test that
    /// used `MODULE_EXECUTE_ERROR` alone covered the whole space. It no longer
    /// does — `Some(false)` is now reachable and is the shape a governance
    /// refusal takes.
    #[tokio::test]
    async fn test_on_error_non_retryable_returns_none() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            strategy: "fixed".to_string(),
            base_delay_ms: 1,
            max_delay_ms: 1,
            jitter: false,
        });
        let ctx = Context::<serde_json::Value>::anonymous();

        // Resolved to Some(false) from the code — a refusal, terminal by policy.
        let denied = ModuleError::new(ErrorCode::ACLDenied, "denied");
        assert_eq!(denied.retryable, Some(false), "precondition");
        let result = mw
            .on_error("mod.a", json!({}), &denied, &ctx)
            .await
            .unwrap();
        assert_eq!(result, None, "a Some(false) code must not be retried");

        // Left unset for the module author to supply.
        let unset = ModuleError::new(ErrorCode::ModuleExecuteError, "fail");
        assert_eq!(unset.retryable, None, "precondition");
        let result = mw.on_error("mod.a", json!({}), &unset, &ctx).await.unwrap();
        assert_eq!(result, None, "an unset code must not be retried");

        assert_eq!(
            RetryMiddleware::retry_count(&ctx, "mod.a"),
            0,
            "neither refusal may consume a retry budget"
        );
    }

    /// apcore#36 declared this as a BREAKING behaviour change: a `MODULE_TIMEOUT`
    /// now resolves to `retryable: Some(true)` from its code alone, so a host
    /// with `RetryMiddleware` installed re-runs the pipeline for it without ever
    /// calling `with_retryable`. Pinned here because the release note promises
    /// it and nothing else asserts it.
    ///
    /// `GENERAL_INTERNAL_ERROR` is checked alongside it — the other of the two
    /// newly-`true` codes that can actually reach this gate (both are raised at
    /// step 8, after `BuiltinMiddlewareBefore` at step 6).
    #[tokio::test]
    async fn test_on_error_retries_codes_the_policy_now_defaults_to_retryable() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            strategy: "fixed".to_string(),
            base_delay_ms: 1,
            max_delay_ms: 1,
            jitter: false,
        });
        let inputs = json!({"key": "val"});

        for code in [ErrorCode::ModuleTimeout, ErrorCode::GeneralInternalError] {
            let ctx = Context::<serde_json::Value>::anonymous();
            // No `.with_retryable(true)` anywhere — the code alone decides.
            let err = ModuleError::new(code, "boom");
            assert_eq!(err.retryable, Some(true), "precondition for {code:?}");

            let result = mw
                .on_error("mod.a", inputs.clone(), &err, &ctx)
                .await
                .unwrap();
            assert_eq!(
                result,
                Some(inputs.clone()),
                "{code:?} must be retried on its code-derived default alone"
            );
            assert_eq!(RetryMiddleware::retry_count(&ctx, "mod.a"), 1);
        }
    }

    /// `with_retryable` must beat the code-derived default in BOTH directions.
    /// Every other override site in the suite uses `MODULE_EXECUTE_ERROR`, whose
    /// default is `None` — that exercises "set from unset", not "override".
    #[tokio::test]
    async fn test_with_retryable_overrides_a_resolved_default() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            strategy: "fixed".to_string(),
            base_delay_ms: 1,
            max_delay_ms: 1,
            jitter: false,
        });

        // Some(false) -> true: the host opts a refusal back in.
        let ctx = Context::<serde_json::Value>::anonymous();
        let forced_on = ModuleError::new(ErrorCode::ACLDenied, "denied").with_retryable(true);
        assert_eq!(forced_on.retryable, Some(true));
        let result = mw
            .on_error("mod.a", json!({}), &forced_on, &ctx)
            .await
            .unwrap();
        assert!(
            result.is_some(),
            "explicit true must beat a Some(false) code"
        );

        // Some(true) -> false: the documented opt-out for a non-idempotent module.
        let ctx = Context::<serde_json::Value>::anonymous();
        let forced_off = ModuleError::new(ErrorCode::ModuleTimeout, "slow").with_retryable(false);
        assert_eq!(forced_off.retryable, Some(false));
        let result = mw
            .on_error("mod.a", json!({}), &forced_off, &ctx)
            .await
            .unwrap();
        assert_eq!(
            result, None,
            "explicit false must beat a Some(true) code — the CHANGELOG's opt-out"
        );
    }

    #[tokio::test]
    async fn test_on_error_retryable_returns_inputs() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 3,
            strategy: "fixed".to_string(),
            base_delay_ms: 1,
            max_delay_ms: 1,
            jitter: false,
        });
        let ctx = Context::<serde_json::Value>::anonymous();
        let err = ModuleError::new(ErrorCode::ModuleExecuteError, "fail").with_retryable(true);
        let inputs = json!({"key": "val"});
        let result = mw
            .on_error("mod.a", inputs.clone(), &err, &ctx)
            .await
            .unwrap();
        assert_eq!(result, Some(inputs));
        assert_eq!(RetryMiddleware::retry_count(&ctx, "mod.a"), 1);
    }

    #[tokio::test]
    async fn test_on_error_exceeds_max_retries() {
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 2,
            strategy: "fixed".to_string(),
            base_delay_ms: 1,
            max_delay_ms: 1,
            jitter: false,
        });
        let ctx = Context::<serde_json::Value>::anonymous();
        RetryMiddleware::set_retry_count(&ctx, "mod.a", 2);
        let err = ModuleError::new(ErrorCode::ModuleExecuteError, "fail").with_retryable(true);
        let result = mw.on_error("mod.a", json!({}), &err, &ctx).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_retry_count_isolated_per_context() {
        // [mw-retry-counter] Two contexts for the same module_id must track
        // retry counts independently (the counter lives in context.data, not a
        // shared instance map), matching Python/TS.
        let mw = RetryMiddleware::new(RetryConfig {
            max_retries: 5,
            strategy: "fixed".to_string(),
            base_delay_ms: 1,
            max_delay_ms: 1,
            jitter: false,
        });
        let ctx_a = Context::<serde_json::Value>::anonymous();
        let ctx_b = Context::<serde_json::Value>::anonymous();
        let err = ModuleError::new(ErrorCode::ModuleExecuteError, "fail").with_retryable(true);

        // Drive ctx_a twice, ctx_b once — same module_id.
        mw.on_error("mod.a", json!({}), &err, &ctx_a).await.unwrap();
        mw.on_error("mod.a", json!({}), &err, &ctx_a).await.unwrap();
        mw.on_error("mod.a", json!({}), &err, &ctx_b).await.unwrap();

        assert_eq!(RetryMiddleware::retry_count(&ctx_a, "mod.a"), 2);
        assert_eq!(RetryMiddleware::retry_count(&ctx_b, "mod.a"), 1);
    }

    #[tokio::test]
    async fn test_retry_count_uses_canonical_context_key() {
        // The key must be exactly `_apcore.mw.retry.count.{module_id}` (parity).
        let ctx = Context::<serde_json::Value>::anonymous();
        RetryMiddleware::set_retry_count(&ctx, "mod.a", 3);
        let data = ctx.data.read();
        assert_eq!(
            data.get("_apcore.mw.retry.count.mod.a")
                .and_then(serde_json::Value::as_i64),
            Some(3)
        );
    }
}
