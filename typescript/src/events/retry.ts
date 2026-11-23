/**
 * Retry configuration types and helpers for event delivery.
 */

export interface RetryConfig {
  maxAttempts?: number;
  initialBackoffMs?: number;
  maxBackoffMs?: number;
  backoffMultiplier?: number;
}

export interface ResolvedRetryConfig {
  maxAttempts: number;
  initialBackoffMs: number;
  maxBackoffMs: number;
  backoffMultiplier: number;
}

/**
 * Spec default retry policy (event-system.md §Per-Subscriber Retry Policy).
 * Applied when a subscriber omits the `retry` block entirely, and as the base
 * for merging when individual fields are omitted. `max_attempts` default is 3 —
 * uniform across Python/TypeScript/Rust SDKs.
 */
export const DEFAULT_RETRY: ResolvedRetryConfig = {
  maxAttempts: 3,
  initialBackoffMs: 100,
  maxBackoffMs: 30_000,
  backoffMultiplier: 2.0,
};

/**
 * Merge caller-supplied retry config with the spec defaults.
 * A subscriber that does NOT supply `retry` receives the full DEFAULT_RETRY
 * policy (max_attempts=3), per event-system.md §Per-Subscriber Retry Policy —
 * built-in and user-registered subscribers share the same default. A subscriber
 * that explicitly sets `maxAttempts: 1` disables retry (single attempt).
 */
export function resolveRetry(config?: RetryConfig): ResolvedRetryConfig {
  if (config === undefined) {
    return { ...DEFAULT_RETRY };
  }
  return { ...DEFAULT_RETRY, ...config };
}

/** Exponential-backoff delay for attempt `attempt` (0-based, clamped to ≥ 0). */
export function computeDelayMs(cfg: ResolvedRetryConfig, attempt: number): number {
  return Math.min(
    cfg.maxBackoffMs,
    Math.floor(cfg.initialBackoffMs * Math.pow(cfg.backoffMultiplier, Math.max(0, attempt))),
  );
}
