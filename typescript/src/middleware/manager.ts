/**
 * MiddlewareManager -- onion model execution engine for the middleware pipeline.
 */

import type { Context } from '../context.js';
import { ModuleError } from '../errors.js';
import type { StepMiddleware } from '../pipeline.js';
import { type Middleware, RetrySignal } from './base.js';

export class MiddlewareChainError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  readonly original: Error;
  /**
   * The middlewares that had already run when the chain failed, in
   * registration order. Module-level chains carry `Middleware`; the pipeline
   * engine reuses this error for a throwing `StepMiddleware.beforeStep`, so
   * the element type is the union of both.
   */
  readonly executedMiddlewares: (Middleware | StepMiddleware)[];

  constructor(original: Error, executedMiddlewares: (Middleware | StepMiddleware)[]) {
    super('MIDDLEWARE_CHAIN_ERROR', String(original), undefined, original);
    this.name = 'MiddlewareChainError';
    this.original = original;
    this.executedMiddlewares = executedMiddlewares;
  }
}

export class MiddlewareManager {
  private _middlewares: Middleware[] = [];
  private _registrations: Map<string, { middleware: Middleware; stack: string }> = new Map();

  add(middleware: Middleware, opts?: { allowDuplicate?: boolean; identityKey?: string }): void {
    // A-D-017: reject priority > 1000 at the manager level (Python/Rust parity).
    // The Middleware base class also guards this at construction, but a plain
    // object or a subclass bypassing super() could carry an out-of-range value.
    if (middleware.priority > 1000) {
      throw new RangeError(
        `Middleware '${middleware.constructor.name}' has priority ${middleware.priority} ` +
          `which exceeds the maximum allowed value of 1000`,
      );
    }

    // A-D-018(a) DEFERRED: default identity stays constructor.name. A robust
    // module/package-qualified identity (Python uses module.qualname) is not
    // reliably available for a JS class at runtime, so we keep the bare name.
    const identity = opts?.identityKey ?? middleware.constructor.name;

    const prior = this._registrations.get(identity);
    if (!opts?.allowDuplicate && prior !== undefined) {
      console.warn(
        `[apcore:middleware] Duplicate middleware registration detected for identity '${identity}' ` +
        `(class: ${middleware.constructor.name}). ` +
        `If intentional, pass allowDuplicate: true or use a unique identityKey. ` +
        `Note: identity defaults to constructor.name, which may collide across packages. ` +
        `Prior registration site:\n${prior.stack}\nCurrent registration site:\n${new Error().stack ?? '(unavailable)'}`,
      );
    }

    // A-D-019(b) first-registration-wins: only record the identity when it is
    // absent. A duplicate must NOT overwrite the original registration site.
    // A-D-021/022(d): identity is recorded even when allowDuplicate is true —
    // only the warning above is suppressed by allowDuplicate, not the recording.
    if (prior === undefined) {
      const stack = new Error().stack ?? '(unavailable)';
      this._registrations.set(identity, { middleware, stack });
    }

    // Stable insertion: find the first middleware with a strictly lower priority
    // and insert before it. This keeps higher-priority middlewares first and
    // preserves registration order among equal priorities.
    let insertAt = this._middlewares.length;
    for (let i = 0; i < this._middlewares.length; i++) {
      if (this._middlewares[i].priority < middleware.priority) {
        insertAt = i;
        break;
      }
    }
    this._middlewares.splice(insertAt, 0, middleware);
  }

  remove(middleware: Middleware): boolean {
    for (let i = 0; i < this._middlewares.length; i++) {
      if (this._middlewares[i] === middleware) {
        this._middlewares.splice(i, 1);
        // Remove from identity registry so the same class can be re-added cleanly
        for (const [key, reg] of this._registrations.entries()) {
          if (reg.middleware === middleware) {
            this._registrations.delete(key);
            break;
          }
        }
        return true;
      }
    }
    return false;
  }

  snapshot(): Middleware[] {
    return [...this._middlewares];
  }

  /**
   * Run all `before` hooks in priority-descending order, awaiting any Promise
   * a middleware returns.
   *
   * Always-async (sync finding A-D-403) — matches apcore-rust's design and
   * removes the silent-Promise-into-currentInputs trap.
   */
  async executeBefore(
    moduleId: string,
    inputs: Record<string, unknown>,
    context: Context,
  ): Promise<[Record<string, unknown>, Middleware[]]> {
    let currentInputs = inputs;
    const executedMiddlewares: Middleware[] = [];
    const middlewares = this.snapshot();

    for (const mw of middlewares) {
      executedMiddlewares.push(mw);
      try {
        const result = await mw.before(moduleId, currentInputs, context);
        if (result !== null && result !== undefined) {
          currentInputs = result as Record<string, unknown>;
        }
      } catch (e) {
        throw new MiddlewareChainError(e as Error, executedMiddlewares);
      }
    }

    return [currentInputs, executedMiddlewares];
  }

  async executeAfter(
    moduleId: string,
    inputs: Record<string, unknown>,
    output: Record<string, unknown>,
    context: Context,
  ): Promise<Record<string, unknown>> {
    let currentOutput = output;
    const middlewares = this.snapshot();

    // Fail-fast: propagate the first error immediately (matches Python/Rust behaviour).
    for (let i = middlewares.length - 1; i >= 0; i--) {
      const result = await middlewares[i].after(moduleId, inputs, currentOutput, context);
      if (result !== null && result !== undefined) {
        currentOutput = result as Record<string, unknown>;
      }
    }

    return currentOutput;
  }

  async executeOnError(
    moduleId: string,
    inputs: Record<string, unknown>,
    error: Error,
    context: Context,
    executedMiddlewares: Middleware[],
  ): Promise<Record<string, unknown> | RetrySignal | null> {
    for (let i = executedMiddlewares.length - 1; i >= 0; i--) {
      try {
        const result = await executedMiddlewares[i].onError(moduleId, inputs, error, context);
        // Strict recovery type check (sync finding A-D-404):
        // only a non-null object or a RetrySignal counts as recovery.
        // `undefined` (typical of arrow functions without a return) does NOT trigger recovery.
        if (
          result !== null
          && result !== undefined
          && (result instanceof RetrySignal || (typeof result === 'object'))
        ) {
          // RetrySignal short-circuits and propagates up to the executor's
          // call() loop, which re-runs the pipeline with new inputs.
          // Plain objects become the recovery output. (sync finding A-D-017)
          return result;
        }
      } catch (e) {
        console.warn('[apcore:middleware] Error in onError handler, continuing:', e);
        continue;
      }
    }
    return null;
  }
}
