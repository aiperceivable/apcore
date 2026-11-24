import { describe, it, expect, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { CancelToken, ExecutionCancelledError } from '../src/cancel.js';
import { Context } from '../src/context.js';
import { Executor } from '../src/executor.js';
import { FunctionModule } from '../src/decorator.js';
import { Registry } from '../src/registry/registry.js';

describe('CancelToken', () => {
  it('is initially not cancelled', () => {
    const token = new CancelToken();
    expect(token.isCancelled).toBe(false);
  });

  it('sets flag after cancel()', () => {
    const token = new CancelToken();
    token.cancel();
    expect(token.isCancelled).toBe(true);
  });

  it('check() does nothing when not cancelled', () => {
    const token = new CancelToken();
    expect(() => token.check()).not.toThrow();
  });

  it('check() throws ExecutionCancelledError when cancelled', () => {
    const token = new CancelToken();
    token.cancel();
    expect(() => token.check()).toThrow(ExecutionCancelledError);
  });

  it('reset() clears cancellation', () => {
    const token = new CancelToken();
    token.cancel();
    expect(token.isCancelled).toBe(true);
    token.reset();
    expect(token.isCancelled).toBe(false);
    expect(() => token.check()).not.toThrow();
  });

  // docs/features/cancellation.md "Contract: CancelToken.raise_if_cancelled" —
  // the spec's canonical method name (idiomatic TS casing). Identical
  // behavior to check(), which is kept as-is for existing callers.
  it('raiseIfCancelled() does nothing when not cancelled', () => {
    const token = new CancelToken();
    expect(() => token.raiseIfCancelled()).not.toThrow();
  });

  it('raiseIfCancelled() throws ExecutionCancelledError when cancelled', () => {
    const token = new CancelToken();
    token.cancel();
    expect(() => token.raiseIfCancelled()).toThrow(ExecutionCancelledError);
  });
});

describe('CancelToken D-18 — real abort via AbortSignal', () => {
  it('exposes an AbortSignal that is not aborted initially', () => {
    const token = new CancelToken();
    expect(token.signal).toBeInstanceOf(AbortSignal);
    expect(token.signal.aborted).toBe(false);
  });

  it('aborts the AbortSignal when cancel() is called', () => {
    const token = new CancelToken();
    const observed: boolean[] = [];
    token.signal.addEventListener('abort', () => observed.push(true));
    token.cancel();
    expect(token.signal.aborted).toBe(true);
    expect(observed).toEqual([true]);
  });

  // reset() used to install a FRESH AbortController, which detached every
  // consumer holding the pre-reset signal: reset() then cancel() left that
  // signal un-aborted, so a module that had composed it into an in-flight
  // fetch never saw the cancel. The signal is the D-18 real-abort channel
  // (async-tasks.md makes it normative for TypeScript specifically), and the
  // divergence was invisible to cooperative checkers. One controller now
  // lives for the whole token, mirroring the in-place reset Python and Rust
  // perform on their flag.
  it('reset() keeps one signal identity for the life of the token', () => {
    const token = new CancelToken();
    const captured = token.signal;
    token.reset();
    expect(token.signal).toBe(captured);
  });

  it('a holder of the pre-reset signal still observes a later cancel', () => {
    const token = new CancelToken();
    const captured = token.signal;
    const observed: boolean[] = [];
    captured.addEventListener('abort', () => observed.push(true));

    token.reset();
    token.cancel();

    expect(captured.aborted).toBe(true);
    expect(observed).toEqual([true]);
  });

  it('reset() clears the cooperative state and warns once that the signal cannot be re-armed', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const token = new CancelToken();
      token.cancel();
      token.reset();

      // Cooperative state resets exactly as in apcore-python/apcore-rust...
      expect(token.isCancelled).toBe(false);
      expect(() => token.check()).not.toThrow();
      // ...but an AbortSignal can never be un-aborted, and that is reported
      // rather than silently pretended away — once per token.
      expect(token.signal.aborted).toBe(true);
      expect(warn).toHaveBeenCalledTimes(1);
      token.reset();
      expect(warn).toHaveBeenCalledTimes(1);
    } finally {
      warn.mockRestore();
    }
  });
});

describe('Context.signal D-18 — exposes cancel signal to modules', () => {
  it('returns the cancel token signal when one is bound', () => {
    const token = new CancelToken();
    const ctx = new Context('t', null, [], null, null, null, {}, token);
    expect(ctx.signal).toBe(token.signal);
  });

  it('returns a never-aborted signal when no cancel token is bound', () => {
    const ctx = new Context('t', null, [], null, null, null, {}, null);
    expect(ctx.signal).toBeInstanceOf(AbortSignal);
    expect(ctx.signal.aborted).toBe(false);
  });
});

describe('Executor cancellation', () => {
  it('respects cancelled token before execution', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ result: 'ok' }),
      moduleId: 'test.module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ result: Type.String() }),
      description: 'Simple module',
    });
    registry.register('test.module', mod);

    const executor = new Executor({ registry });
    const token = new CancelToken();
    token.cancel();

    const ctx = new Context(
      'trace-1',
      null,
      [],
      executor,
      null,
      null,
      {},
      token,
    );

    await expect(executor.call('test.module', {}, ctx)).rejects.toThrow(ExecutionCancelledError);
  });

  it('D-18 — cancelling mid-execution interrupts an awaiting module via signal', async () => {
    // Regression for A-D-AT-02 / D-18: a module that awaits the cancel signal
    // (e.g. via AbortSignal.timeout or fetch's signal option) must be
    // interrupted by cancelToken.cancel() rather than running to completion.
    const registry = new Registry();
    let moduleFinished = false;

    const mod = new FunctionModule({
      execute: async (_inputs, context) => {
        // Simulate a Web-API I/O call that participates in the signal.
        await new Promise<void>((resolve, reject) => {
          const t = setTimeout(() => { moduleFinished = true; resolve(); }, 5000);
          context!.signal.addEventListener('abort', () => {
            clearTimeout(t);
            reject(new Error('aborted via signal'));
          });
        });
        return { result: 'never' };
      },
      moduleId: 'test.longrun',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ result: Type.String() }),
      description: 'long-running module',
    });
    registry.register('test.longrun', mod);

    const executor = new Executor({ registry });
    const token = new CancelToken();
    const ctx = new Context('trace-1', null, [], executor, null, null, {}, token);

    const callPromise = executor.call('test.longrun', {}, ctx);
    // Allow the module's await to start
    await new Promise((r) => setTimeout(r, 20));
    token.cancel();

    await expect(callPromise).rejects.toThrow();
    expect(moduleFinished).toBe(false);
  });
});
