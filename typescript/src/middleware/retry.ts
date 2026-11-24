/**
 * RetryMiddleware for automatic retry of retryable module errors.
 */

import type { Context } from '../context.js';
import { RETRY_COUNT_BASE } from '../context-keys.js';
import type { ModuleError } from '../errors.js';
import { Middleware, RetrySignal } from './base.js';

/**
 * Well-known context.data key prefixes for retry state.
 *
 * `CTX_RETRY_COUNT_PREFIX + moduleId` is identical to
 * `RETRY_COUNT_BASE.scoped(moduleId).name` and stores the per-module attempt
 * counter used by {@link RetryMiddleware}. `CTX_RETRY_DELAY_PREFIX` is retained
 * for backward compatibility with outer retry loops that inspected the legacy
 * advisory delay hint; it is no longer written by the built-in middleware.
 */
export const CTX_RETRY_COUNT_PREFIX = '_apcore.mw.retry.count.';
export const CTX_RETRY_DELAY_PREFIX = '_apcore.mw.retry.delay_ms.';

export interface RetryConfig {
  maxRetries: number;
  strategy: 'exponential' | 'fixed';
  baseDelayMs: number;
  maxDelayMs: number;
  jitter: boolean;
}

const DEFAULT_RETRY_CONFIG: RetryConfig = {
  maxRetries: 3,
  strategy: 'exponential',
  baseDelayMs: 100,
  maxDelayMs: 5000,
  jitter: true,
};

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Middleware that retries failed module executions based on error retryability.
 *
 * When `onError` is called with a retryable error (`error.retryable === true`),
 * this middleware sleeps for a calculated backoff delay and returns a
 * {@link RetrySignal} carrying the original inputs. The executor recognises the
 * signal and re-runs the module; remaining middlewares' `onError` handlers are
 * not invoked for this attempt. After `maxRetries` attempts or for
 * non-retryable errors, it returns `null` so the error propagates.
 *
 * Retry state is tracked per-module in `context.data` under
 * `_apcore.mw.retry.count.{moduleId}` (via {@link RETRY_COUNT_BASE}). The
 * counter is cleared by `after()` on successful completion so `context.data`
 * does not grow unbounded across long call chains that recover before the
 * limit is hit.
 *
 * Cross-language parity with apcore-python `RetryMiddleware`
 * (returns `RetrySignal`) and apcore-rust `RetryMiddleware`
 * (returns `Ok(Some(inputs))`).
 */
export class RetryMiddleware extends Middleware {
  private _config: RetryConfig;

  constructor(config?: Partial<RetryConfig>) {
    super();
    this._config = { ...DEFAULT_RETRY_CONFIG, ...config };
  }

  override async onError(
    moduleId: string,
    inputs: Record<string, unknown>,
    error: Error,
    context: Context,
  ): Promise<RetrySignal | null> {
    const retryable = (error as ModuleError).retryable;
    if (retryable !== true) return null;

    const retryKey = RETRY_COUNT_BASE.scoped(moduleId);
    const retryCount = retryKey.get(context, 0) ?? 0;

    if (retryCount >= this._config.maxRetries) {
      console.warn(
        `[apcore:retry] Max retries (${this._config.maxRetries}) exceeded for module '${moduleId}'`,
      );
      return null;
    }

    const delayMs = this._calculateDelay(retryCount);
    retryKey.set(context, retryCount + 1);

    console.warn(
      `[apcore:retry] Retrying module '${moduleId}' (attempt ${retryCount + 1}/${this._config.maxRetries}) ` +
        `after ${Math.round(delayMs)}ms`,
    );

    await sleep(delayMs);
    return new RetrySignal({ ...inputs });
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    context: Context,
  ): null {
    // Clear the per-module retry counter on successful completion so
    // context.data does not accumulate stale `_apcore.mw.retry.count.*` keys.
    RETRY_COUNT_BASE.scoped(moduleId).delete(context);
    return null;
  }

  private _calculateDelay(attempt: number): number {
    let delay: number;
    if (this._config.strategy === 'fixed') {
      delay = this._config.baseDelayMs;
    } else {
      // Exponential: baseDelayMs * 2^attempt, capped at maxDelayMs
      delay = Math.min(
        this._config.baseDelayMs * Math.pow(2, attempt),
        this._config.maxDelayMs,
      );
    }

    if (this._config.jitter) {
      delay *= 0.5 + Math.random(); // 0.5 to 1.5x
    }

    return delay;
  }
}

/**
 * @deprecated Use {@link RetryMiddleware} instead. This alias previously named
 * an advisory no-op variant that only recorded hints in `context.data`; that
 * behavior is gone — `RetryHintMiddleware` now performs real retries. Will be
 * removed in 1.0.0.
 */
export const RetryHintMiddleware = RetryMiddleware;
