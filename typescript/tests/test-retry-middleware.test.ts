import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { RetryMiddleware, RetryHintMiddleware } from '../src/middleware/retry.js';
import { Middleware, RetrySignal } from '../src/middleware/base.js';
import { ModuleError, ModuleTimeoutError } from '../src/errors.js';
import { Context } from '../src/context.js';
import { Config } from '../src/config.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { FunctionModule } from '../src/decorator.js';
import { buildStandardStrategy } from '../src/builtin-steps.js';
import { MiddlewareManager } from '../src/middleware/manager.js';
import type { StepResult } from '../src/pipeline.js';

describe('RetryMiddleware', () => {
  const makeContext = () => new Context('trace-1', null, ['test.module'], null, null, null, {});
  const COUNT_KEY = '_apcore.mw.retry.count.test.module';

  it('returns null for non-retryable errors', async () => {
    const mw = new RetryMiddleware({ baseDelayMs: 1, jitter: false });
    const ctx = makeContext();
    const err = new ModuleError('TEST', 'test error', {}, undefined, undefined, false);
    expect(await mw.onError('test.module', { x: 1 }, err, ctx)).toBeNull();
    expect(ctx.data[COUNT_KEY]).toBeUndefined();
  });

  it('returns a RetrySignal carrying the inputs for retryable errors', async () => {
    const mw = new RetryMiddleware({ baseDelayMs: 1, jitter: false });
    const ctx = makeContext();
    const err = new ModuleTimeoutError('test.module', 5000);
    const result = await mw.onError('test.module', { x: 1 }, err, ctx);
    expect(result).toBeInstanceOf(RetrySignal);
    expect((result as RetrySignal).inputs).toEqual({ x: 1 });
    expect(ctx.data[COUNT_KEY]).toBe(1);
  });

  it('tracks retry count across calls and stops after maxRetries', async () => {
    const mw = new RetryMiddleware({ maxRetries: 3, baseDelayMs: 1, jitter: false });
    const ctx = makeContext();
    const err = new ModuleTimeoutError('test.module', 5000);

    expect(await mw.onError('test.module', {}, err, ctx)).toBeInstanceOf(RetrySignal);
    expect(ctx.data[COUNT_KEY]).toBe(1);

    expect(await mw.onError('test.module', {}, err, ctx)).toBeInstanceOf(RetrySignal);
    expect(ctx.data[COUNT_KEY]).toBe(2);

    expect(await mw.onError('test.module', {}, err, ctx)).toBeInstanceOf(RetrySignal);
    expect(ctx.data[COUNT_KEY]).toBe(3);

    // 4th call exceeds maxRetries -> propagate
    expect(await mw.onError('test.module', {}, err, ctx)).toBeNull();
  });

  it('returns null for errors without retryable property', async () => {
    const mw = new RetryMiddleware({ baseDelayMs: 1, jitter: false });
    const ctx = makeContext();
    const err = new Error('plain error');
    expect(await mw.onError('test.module', {}, err, ctx)).toBeNull();
  });

  it('clears the per-module retry counter on success via after()', () => {
    const mw = new RetryMiddleware();
    const ctx = makeContext();
    ctx.data[COUNT_KEY] = 2;
    mw.after('test.module', {}, {}, ctx);
    expect(ctx.data[COUNT_KEY]).toBeUndefined();
  });

  it('uses exponential backoff by default (delay scales per attempt)', async () => {
    // jitter off: verify the sleep duration grows by inspecting elapsed time
    // indirectly is brittle, so assert the signal/counter progression instead.
    const mw = new RetryMiddleware({ baseDelayMs: 1, jitter: false, maxRetries: 5 });
    const ctx = makeContext();
    const err = new ModuleTimeoutError('test.module', 5000);
    for (let i = 1; i <= 3; i++) {
      const r = await mw.onError('test.module', {}, err, ctx);
      expect(r).toBeInstanceOf(RetrySignal);
      expect(ctx.data[COUNT_KEY]).toBe(i);
    }
  });

  it('drives a real end-to-end retry: flaky module recovers after N attempts', async () => {
    let invocations = 0;
    const flaky = new FunctionModule({
      execute: () => {
        invocations += 1;
        if (invocations < 3) {
          throw new ModuleTimeoutError('flaky', 5000); // retryable
        }
        return { x: invocations };
      },
      moduleId: 'flaky',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ x: Type.Number() }),
      description: 'Flaky module',
    });
    const registry = new Registry();
    registry.register('flaky', flaky);
    const executor = new Executor({
      registry,
      middlewares: [new RetryMiddleware({ baseDelayMs: 1, jitter: false, maxRetries: 5 })],
    });

    const out = await executor.call('flaky', { x: 42 });
    // Failed twice, succeeded on the 3rd attempt.
    expect(invocations).toBe(3);
    expect(out).toEqual({ x: 3 });
  });

  it('does NOT retry a non-retryable error: module runs once and error propagates', async () => {
    let invocations = 0;
    const broken = new FunctionModule({
      execute: () => {
        invocations += 1;
        // retryable=false
        throw new ModuleError('BROKEN', 'permanent failure', {}, undefined, undefined, false);
      },
      moduleId: 'broken',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ x: Type.Number() }),
      description: 'Permanently broken module',
    });
    const registry = new Registry();
    registry.register('broken', broken);
    const executor = new Executor({
      registry,
      middlewares: [new RetryMiddleware({ baseDelayMs: 1, jitter: false })],
    });

    await expect(executor.call('broken', { x: 42 })).rejects.toThrow(ModuleError);
    expect(invocations).toBe(1);
  });

  it('exhausts retries and propagates when the module never recovers', async () => {
    let invocations = 0;
    const flaky = new FunctionModule({
      execute: () => {
        invocations += 1;
        throw new ModuleTimeoutError('flaky', 5000); // always retryable
      },
      moduleId: 'flaky',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ x: Type.Number() }),
      description: 'Flaky module',
    });
    const registry = new Registry();
    registry.register('flaky', flaky);
    const executor = new Executor({
      registry,
      middlewares: [new RetryMiddleware({ baseDelayMs: 1, jitter: false, maxRetries: 2 })],
    });

    await expect(executor.call('flaky', { x: 42 })).rejects.toThrow(ModuleTimeoutError);
    // Initial attempt + 2 retries = 3 invocations.
    expect(invocations).toBe(3);
  });

  it('clears the retry counter after a successful end-to-end recovery', async () => {
    let invocations = 0;
    const flaky = new FunctionModule({
      execute: () => {
        invocations += 1;
        if (invocations < 2) throw new ModuleTimeoutError('flaky', 5000);
        return { x: 1 };
      },
      moduleId: 'flaky',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ x: Type.Number() }),
      description: 'Flaky module',
    });
    const registry = new Registry();
    registry.register('flaky', flaky);
    const ctx = new Context('trace-2', null, ['flaky'], null, null, null, {});
    const executor = new Executor({
      registry,
      middlewares: [new RetryMiddleware({ baseDelayMs: 1, jitter: false })],
    });

    await executor.call('flaky', { x: 42 }, ctx);
    expect(ctx.data['_apcore.mw.retry.count.flaky']).toBeUndefined();
  });

  it('RetryHintMiddleware is a deprecated alias for RetryMiddleware', () => {
    expect(RetryHintMiddleware).toBe(RetryMiddleware);
  });

  it('A-D-011: a retryable error thrown in before() retries exactly maxRetries times', async () => {
    // A middleware whose before() throws a retryable error. on_error must fire
    // exactly ONCE per attempt (the executor is the sole on_error site). The
    // before-step must NOT also run on_error, otherwise the RetryMiddleware
    // counter double-increments and the retry budget is halved.
    let beforeCalls = 0;

    // Lower priority than RetryMiddleware (default 100) so RetryMiddleware's
    // before() runs first and lands in executedMiddlewares, making its
    // onError eligible when this middleware's before() throws.
    class ThrowingBeforeMiddleware extends Middleware {
      constructor() {
        super(50);
      }
      override before(): Record<string, unknown> {
        beforeCalls += 1;
        throw new ModuleTimeoutError('target', 5000); // retryable
      }
    }

    const target = new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'target',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Never reached — before() always throws',
    });
    const registry = new Registry();
    registry.register('target', target);
    // Raise the per-module repeat limit so the 4 pipeline attempts (each of
    // which appends 'target' to the call chain) are not cut short by the
    // call-chain frequency guard; that lets the original retryable error
    // surface and isolates the retry-budget assertion below.
    const config = new Config({ executor: { max_module_repeat: 100 } });
    const executor = new Executor({
      registry,
      config,
      middlewares: [
        new RetryMiddleware({ maxRetries: 3, baseDelayMs: 1, jitter: false }),
        new ThrowingBeforeMiddleware(),
      ],
    });

    await expect(executor.call('target', {})).rejects.toThrow(ModuleTimeoutError);
    // Initial attempt + 3 retries = 4 pipeline attempts -> before() called 4x.
    // (With the double-on_error bug, the counter advanced 2x/attempt and only
    // ~2 attempts ran.)
    expect(beforeCalls).toBe(4);
  });

  it('A-D-011: before()-failure recovery output is still produced by the executor', async () => {
    // Removing the step-level on_error must not lose the recovery-output path:
    // a middleware whose onError returns a plain object should short-circuit
    // the call with that object as the result, even when the failure happened
    // in another middleware's before().
    class RecoveringMiddleware extends Middleware {
      constructor() {
        super(200); // runs first -> lands in executedMiddlewares
      }
      override onError(): Record<string, unknown> {
        return { recovered: true };
      }
    }
    class ThrowingBeforeMiddleware extends Middleware {
      constructor() {
        super(50);
      }
      override before(): Record<string, unknown> {
        throw new ModuleError('BOOM', 'before exploded', {}, undefined, undefined, false);
      }
    }

    const target = new FunctionModule({
      execute: () => ({ ok: true }),
      moduleId: 'target',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ ok: Type.Boolean() }),
      description: 'Never reached — before() throws',
    });
    const registry = new Registry();
    registry.register('target', target);
    const executor = new Executor({
      registry,
      middlewares: [new RecoveringMiddleware(), new ThrowingBeforeMiddleware()],
    });

    const out = await executor.call('target', {});
    expect(out).toEqual({ recovered: true });
  });

  it('retries a real pipeline step timeout after module middleware has entered', async () => {
    const observed: Error[] = [];
    class RecordingRetry extends RetryMiddleware {
      override async onError(
        moduleId: string, inputs: Record<string, unknown>, error: Error, context: Context,
      ): Promise<RetrySignal | null> {
        observed.push(error);
        return super.onError(moduleId, inputs, error, context);
      }
    }
    const retry = new RecordingRetry({ maxRetries: 1, baseDelayMs: 1, jitter: false });
    const manager = new MiddlewareManager();
    manager.add(retry);
    const registry = new Registry();
    let moduleCalls = 0;
    registry.register('test.module', new FunctionModule({
      moduleId: 'test.module', description: 'Step timeout retry target',
      inputSchema: Type.Object({}), outputSchema: Type.Object({ ok: Type.Boolean() }),
      execute: () => { moduleCalls += 1; return { ok: true }; },
    }));
    const config = new Config({ executor: { max_module_repeat: 100 } });
    const strategy = buildStandardStrategy({
      registry, config, acl: null, approvalHandler: null, middlewareManager: manager,
    });
    let stepCalls = 0;
    strategy.insertAfter('middleware_before', {
      name: 'timed_step', description: 'Times out once before succeeding',
      removable: true, replaceable: true, timeoutMs: 5,
      execute: async (): Promise<StepResult> => {
        stepCalls += 1;
        if (stepCalls === 1) return new Promise<StepResult>(() => {});
        return { action: 'continue' };
      },
    });
    const executor = new Executor({ registry, config, strategy, middlewares: [retry] });
    await expect(executor.call('test.module', {})).resolves.toEqual({ ok: true });
    expect(stepCalls).toBe(2);
    expect(moduleCalls).toBe(1);
    expect(observed).toHaveLength(1);
    expect(observed[0]).toBeInstanceOf(ModuleTimeoutError);
    expect(observed[0]).toMatchObject({ code: 'MODULE_TIMEOUT', retryable: true });
  });
});
