/**
 * Middleware base class for apcore.
 */

import type { Context } from '../context.js';

/**
 * Return value from `Middleware.onError` requesting a retry.
 *
 * Distinct from returning a plain object — a plain object is interpreted by
 * `MiddlewareManager` as the *final recovery output* of the call. A
 * `RetrySignal` instead asks the executor to re-run the module with the
 * given inputs; no recovery output is produced.
 *
 * Cross-language parity with apcore-python `apcore.middleware.RetrySignal`
 * and apcore-rust `apcore::middleware::RetrySignal` (sync finding A-D-017).
 */
export class RetrySignal {
  readonly inputs: Record<string, unknown>;

  constructor(inputs: Record<string, unknown>) {
    this.inputs = inputs;
  }
}

export class Middleware {
  /**
   * Execution priority (0-1000). Higher priority executes first.
   * When priorities are equal, registration order is used as tiebreaker.
   */
  readonly priority: number;

  constructor(priority: number = 100) {
    if (priority < 0 || priority > 1000) {
      throw new RangeError(`priority must be between 0 and 1000, got ${priority}`);
    }
    this.priority = priority;
  }

  /**
   * Hooks may return either a synchronous value or a Promise. The
   * MiddlewareManager always awaits the return value (Issue #42), so async
   * middleware (`async before()`, higher-order Promise-returning wrappers,
   * thenables) work transparently — no leaked Promises into currentInputs /
   * currentOutput / recovery values.
   */
  before(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    _context: Context,
  ):
    | Record<string, unknown>
    | null
    | Promise<Record<string, unknown> | null | undefined>
    | undefined {
    return null;
  }

  after(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    _context: Context,
  ):
    | Record<string, unknown>
    | null
    | Promise<Record<string, unknown> | null | undefined>
    | undefined {
    return null;
  }

  onError(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    _error: Error,
    _context: Context,
  ):
    | Record<string, unknown>
    | RetrySignal
    | null
    | Promise<Record<string, unknown> | RetrySignal | null | undefined>
    | undefined {
    return null;
  }
}
