/**
 * Issue #42 — async on_error correctness.
 *
 * Async middleware hooks (before/after/onError) MUST be awaited so a returned
 * Promise never leaks into `currentInputs` / `currentOutput` / recovery value.
 *
 * These tests exercise the middleware manager directly (where the bug would
 * surface). The TypeScript implementation always awaits — so these tests act
 * as regression guards.
 */
import { describe, expect, it } from 'vitest';
import { Context } from '../../src/context.js';
import { Middleware, RetrySignal } from '../../src/middleware/base.js';
import { MiddlewareManager } from '../../src/middleware/manager.js';

class AsyncBeforeMW extends Middleware {
  override async before(
    _id: string,
    inputs: Record<string, unknown>,
    _ctx: Context,
  ): Promise<Record<string, unknown>> {
    await Promise.resolve();
    return { ...inputs, asyncTouched: true };
  }
}

class AsyncAfterMW extends Middleware {
  override async after(
    _id: string,
    _inputs: Record<string, unknown>,
    output: Record<string, unknown>,
    _ctx: Context,
  ): Promise<Record<string, unknown>> {
    await Promise.resolve();
    return { ...output, asyncEnriched: true };
  }
}

class AsyncRecoveryMW extends Middleware {
  override async onError(
    _id: string,
    _inputs: Record<string, unknown>,
    _err: Error,
    _ctx: Context,
  ): Promise<Record<string, unknown>> {
    await Promise.resolve();
    return { recovered: 'async-value' };
  }
}

class AsyncRetryMW extends Middleware {
  override async onError(
    _id: string,
    _inputs: Record<string, unknown>,
    _err: Error,
    _ctx: Context,
  ): Promise<RetrySignal> {
    await Promise.resolve();
    return new RetrySignal({ retried: true });
  }
}

describe('Async middleware correctness (Issue #42)', () => {
  const ctx = new Context('trace-test');

  it('awaits async before() and merges resolved value, never leaking the Promise', async () => {
    const mgr = new MiddlewareManager();
    mgr.add(new AsyncBeforeMW());

    const [result] = await mgr.executeBefore('mod.x', { original: 1 }, ctx);
    expect(result).toEqual({ original: 1, asyncTouched: true });
    // The leaked-Promise bug would cause `result` to be a Promise object
    // rather than the resolved record.
    expect(typeof (result as { then?: unknown }).then).toBe('undefined');
  });

  it('awaits async after() and merges resolved value', async () => {
    const mgr = new MiddlewareManager();
    mgr.add(new AsyncAfterMW());

    const result = await mgr.executeAfter('mod.x', { i: 1 }, { y: 1 }, ctx);
    expect(result).toEqual({ y: 1, asyncEnriched: true });
    expect(typeof (result as { then?: unknown }).then).toBe('undefined');
  });

  it('awaits async onError() and surfaces the resolved recovery object', async () => {
    const mgr = new MiddlewareManager();
    const mw = new AsyncRecoveryMW();
    mgr.add(mw);

    const result = await mgr.executeOnError('mod.x', { i: 1 }, new Error('boom'), ctx, [mw]);
    expect(result).toEqual({ recovered: 'async-value' });
    expect(typeof (result as { then?: unknown }).then).toBe('undefined');
  });

  it('awaits async onError() returning a RetrySignal and short-circuits the chain', async () => {
    const mgr = new MiddlewareManager();
    const retryMw = new AsyncRetryMW();
    const recoverMw = new AsyncRecoveryMW();
    mgr.add(recoverMw); // higher priority -> runs first in before, last in after/onError reverse
    mgr.add(retryMw);

    // executeOnError iterates `executedMiddlewares` in reverse, so the *last*
    // executed middleware (retryMw) runs first.
    const result = await mgr.executeOnError('mod.x', { i: 1 }, new Error('boom'), ctx, [
      recoverMw,
      retryMw,
    ]);
    expect(result).toBeInstanceOf(RetrySignal);
    expect((result as RetrySignal).inputs).toEqual({ retried: true });
  });

  it('treats `undefined` from async onError() as "no recovery"', async () => {
    class UndefMW extends Middleware {
      override async onError(): Promise<undefined> {
        await Promise.resolve();
        return undefined;
      }
    }
    const mgr = new MiddlewareManager();
    const mw = new UndefMW();
    mgr.add(mw);

    const result = await mgr.executeOnError('mod.x', {}, new Error('e'), ctx, [mw]);
    expect(result).toBeNull();
  });
});
