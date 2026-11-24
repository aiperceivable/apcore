// APCore Protocol — Event subscriber retry configuration
// Spec reference: event-system.md §Delivery Semantics

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Retry configuration for event subscribers.
///
/// Defaults align with event-system.md §Per-Subscriber Retry Policy:
/// `max_attempts=3`, `initial_backoff_ms=100`, `max_backoff_ms=30_000`,
/// `backoff_multiplier=2.0`. Callers that genuinely want single-attempt
/// (fire-and-forget) semantics should use [`Self::no_retry`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct EventRetryConfig {
    /// Total maximum delivery attempts (1 = no retry, 2 = one retry, etc.).
    pub max_attempts: u32,
    /// Backoff delay for the first retry, in milliseconds.
    pub initial_backoff_ms: u64,
    /// Maximum backoff delay cap, in milliseconds.
    pub max_backoff_ms: u64,
    /// Multiplier applied to the backoff delay on each successive attempt.
    pub backoff_multiplier: f64,
}

impl Default for EventRetryConfig {
    /// Default: spec-aligned (3 attempts, 100 ms initial backoff, 30 s cap, 2.0x).
    ///
    /// Spec: docs/features/event-system.md §"Per-Subscriber Retry Policy"
    /// — `max_attempts=3`, `initial_backoff_ms=100`,
    /// `max_backoff_ms=30000`, `backoff_multiplier=2.0`.
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 100,
            max_backoff_ms: 30_000,
            backoff_multiplier: 2.0,
        }
    }
}

impl EventRetryConfig {
    /// Single-attempt delivery (no retry). Use for fire-and-forget subscribers
    /// where transient failures must not be retried (e.g. stdout, local file
    /// rotation, idempotent push to in-memory metrics).
    #[must_use]
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            backoff_multiplier: 1.0,
        }
    }

    /// Compute backoff delay for a zero-based attempt index.
    ///
    /// `attempt=0` is the first retry (delay before the second delivery try).
    /// Returns a value clamped to `[0, max_backoff_ms]`.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn compute_delay_ms(&self, attempt: u32) -> u64 {
        // SAFETY: initial_backoff_ms fits in f64 for any realistic value.
        // attempt fits in i32 since it's bounded by max_attempts (u32 <= i32::MAX in practice).
        // The result is clamped to max_backoff_ms so truncation is safe.
        let raw = (self.initial_backoff_ms as f64)
            * self
                .backoff_multiplier
                .powi(i32::try_from(attempt).unwrap_or(i32::MAX));
        raw.min(self.max_backoff_ms as f64) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_matches_spec() {
        // Spec: docs/features/event-system.md §Per-Subscriber Retry Policy
        let cfg = EventRetryConfig::default();
        assert_eq!(cfg.max_attempts, 3);
        assert_eq!(cfg.initial_backoff_ms, 100);
        assert_eq!(cfg.max_backoff_ms, 30_000);
        assert!((cfg.backoff_multiplier - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_no_retry_helper_is_single_attempt() {
        let cfg = EventRetryConfig::no_retry();
        assert_eq!(cfg.max_attempts, 1);
    }

    #[test]
    fn test_compute_delay_ms_exponential() {
        let cfg = EventRetryConfig {
            max_attempts: 5,
            initial_backoff_ms: 100,
            max_backoff_ms: 30_000,
            backoff_multiplier: 2.0,
        };
        assert_eq!(cfg.compute_delay_ms(0), 100);
        assert_eq!(cfg.compute_delay_ms(1), 200);
        assert_eq!(cfg.compute_delay_ms(2), 400);
        assert_eq!(cfg.compute_delay_ms(3), 800);
    }

    #[test]
    fn test_compute_delay_ms_caps_at_max() {
        let cfg = EventRetryConfig {
            max_attempts: 10,
            initial_backoff_ms: 1000,
            max_backoff_ms: 5_000,
            backoff_multiplier: 2.0,
        };
        // 1000 * 2^3 = 8000, capped at 5000
        assert_eq!(cfg.compute_delay_ms(3), 5_000);
    }
}
