import { Type } from '@sinclair/typebox';
import { describe, expect, it, vi } from 'vitest';
import {
  APCore, CallBudget, CallbackApprovalHandler, CancelToken, Context, Executor, Middleware, Registry,
  RetrySignal, STREAMING_MARKER, createAnnotations, createApprovalResult,
} from '../src/index.js';
import { AsyncTaskManager, RetryConfig } from '../src/async-task.js';

const id = 'budget.run';
const emptySchema = Type.Object({});

function deferred() {
  let release!: () => void;
  const promise = new Promise<void>(resolve => { release = resolve; });
  return { promise, release };
}

function definition(execute: (_inputs: unknown, context: Context) => unknown) {
  return { description: 'Run a budgeted module', inputSchema: emptySchema, outputSchema: emptySchema, execute };
}

describe('CallBudget', () => {
  it('settles actual cost once, copies estimates, and keeps capabilities out of wire contexts', async () => {
    const estimatedCosts = { [id]: 4 };
    const budget = new CallBudget({ maxCalls: 2, maxCost: 6, estimatedCosts });
    estimatedCosts[id] = 9;
    const client = new APCore();
    const root = Context.create();
    const context = root.withBudget(budget);
    root.recordCost(1);
    expect(() => context.recordCost(1)).toThrow(expect.objectContaining({ code: 'GENERAL_INVALID_INPUT' }));
    expect(context.child(id).toJSON().data).toEqual({});
    expect(context.toJSON()).toEqual(root.toJSON());

    const execute = vi.fn((_inputs: unknown, invocation: Context) => {
      expect(budget.snapshot()).toEqual({ callsUsed: 1, callsReserved: 0, costUsed: 4 });
      invocation.recordCost(0);
      expect(() => invocation.recordCost(1)).toThrow(expect.objectContaining({ code: 'GENERAL_INVALID_INPUT' }));
      return {};
    });
    await client.register(id, definition(execute));
    expect((await client.validate(id, {}, context)).valid).toBe(true);
    expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 0, costUsed: 0 });
    await expect(client.call(id, {}, context)).resolves.toEqual({});
    expect(budget.snapshot()).toEqual({ callsUsed: 1, callsReserved: 0, costUsed: 0 });
    expect(execute).toHaveBeenCalledOnce();
  });

  it('keeps concurrent call reservations and releases pending approval without spending', async () => {
    const registry = new Registry();
    const handlerEntered = deferred();
    const answer = deferred();
    const approvalHandler = new CallbackApprovalHandler(async () => {
      handlerEntered.release();
      await answer.promise;
      return createApprovalResult({ status: 'pending', approvalId: 'wait' });
    });
    const executor = new Executor({ registry, approvalHandler });
    const execute = vi.fn(() => ({}));
    await registry.register(id, { ...definition(execute), annotations: createAnnotations({ requiresApproval: true }) });
    const budget = new CallBudget({ maxCalls: 1 });
    const context = Context.create().withBudget(budget);
    const pending = executor.call(id, {}, context);
    await handlerEntered.promise;
    try {
      expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 1, costUsed: 0 });
      await expect(executor.call(id, {}, context.child('budget.parent'))).rejects.toMatchObject({
        code: 'CALL_BUDGET_EXCEEDED', retryable: false, details: { dimension: 'calls' },
      });
    } finally {
      answer.release();
    }
    await expect(pending).rejects.toMatchObject({ code: 'APPROVAL_PENDING' });
    expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 0, costUsed: 0 });
    expect(execute).not.toHaveBeenCalled();
  });

  it('charges a failing business attempt and its actual retry, including traced calls', async () => {
    const registry = new Registry();
    const execute = vi.fn().mockImplementationOnce(() => { throw new Error('retry'); }).mockReturnValue({});
    const executor = new Executor({ registry });
    class RetryOnce extends Middleware {
      override onError(): RetrySignal { return new RetrySignal({}); }
    }
    executor.use(new RetryOnce());
    await registry.register(id, definition(execute));
    const budget = new CallBudget({ maxCalls: 2, maxCost: 6, estimatedCosts: { [id]: 3 } });
    const [output, trace] = await executor.callWithTrace(id, {}, Context.create().withBudget(budget));
    expect(output).toEqual({});
    expect(trace.success).toBe(true);
    expect(budget.snapshot()).toEqual({ callsUsed: 2, callsReserved: 0, costUsed: 6 });
  });

  it('rejects estimates above the remaining limit and keeps reported overrun charged', async () => {
    const client = new APCore();
    const execute = vi.fn((_inputs: unknown, context: Context) => {
      context.recordCost(7);
      return {};
    });
    await client.register(id, definition(execute));
    const estimatedCosts = { [id]: 3 };
    const low = new CallBudget({ maxCost: 2, estimatedCosts });
    await expect(client.call(id, {}, Context.create().withBudget(low))).rejects.toMatchObject({
      code: 'CALL_BUDGET_EXCEEDED', details: { dimension: 'cost' },
    });
    expect(low.snapshot()).toEqual({ callsUsed: 0, callsReserved: 0, costUsed: 0 });
    expect(execute).not.toHaveBeenCalled();
    const high = new CallBudget({ maxCost: 6, estimatedCosts });
    await expect(client.call(id, {}, Context.create().withBudget(high))).rejects.toMatchObject({
      code: 'CALL_BUDGET_EXCEEDED', details: { dimension: 'cost' },
    });
    expect(high.snapshot()).toEqual({ callsUsed: 1, callsReserved: 0, costUsed: 7 });
  });

  it('charges a stream only when its registered producer starts and supports actual settlement', async () => {
    const client = new APCore();
    const stream = vi.fn(async function* (_inputs: unknown, context: Context) {
      context.recordCost(2);
      yield { done: true };
    });
    await client.register(id, { ...definition(() => ({})), stream, [STREAMING_MARKER]: true,
      annotations: createAnnotations({ streaming: true }) });
    const budget = new CallBudget({ maxCalls: 1, maxCost: 6, estimatedCosts: { [id]: 3 } });
    const context = Context.create().withBudget(budget);
    const abandoned = client.streamWithCompletion(id, {}, context);
    await abandoned.close();
    expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 0, costUsed: 0 });
    const running = client.streamWithCompletion(id, {}, context);
    expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 0, costUsed: 0 });
    await expect(running.chunks.next()).resolves.toMatchObject({ value: { done: true }, done: false });
    await expect(running.chunks.next()).resolves.toMatchObject({ done: true });
    await expect(running.completion).resolves.toMatchObject({ reason: 'completed' });
    expect(budget.snapshot()).toEqual({ callsUsed: 1, callsReserved: 0, costUsed: 2 });
    expect(stream).toHaveBeenCalledOnce();
  });

  it('releases pre-dispatch cancellation reservations and rejects a strategy without a guard', async () => {
    const registry = new Registry();
    const execute = vi.fn(() => ({}));
    await registry.register(id, definition(execute));
    const executor = new Executor({ registry });
    const guardPassed = deferred();
    const release = deferred();
    executor.addStepMiddleware({ async beforeStep(name) {
      if (name === 'execute') {
        guardPassed.release();
        await release.promise;
      }
    } });
    const token = new CancelToken();
    const budget = new CallBudget({ maxCalls: 1 });
    const invocation = executor.call(id, {}, Context.create(null, null, token).withBudget(budget));
    await guardPassed.promise;
    expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 1, costUsed: 0 });
    token.cancel();
    expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 0, costUsed: 0 });
    release.release();
    await expect(invocation).rejects.toMatchObject({ code: 'EXECUTION_CANCELLED' });
    expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 0, costUsed: 0 });
    const minimal = new Executor({ registry, strategy: 'minimal' });
    await expect(minimal.call(id, {}, Context.create().withBudget(budget))).rejects.toMatchObject({
      code: 'PIPELINE_CONFIGURATION_ERROR',
    });
    expect(execute).not.toHaveBeenCalled();
  });

  it('rejects skipped dispatch and does not spend the reserved call or return injected output', async () => {
    const registry = new Registry();
    const execute = vi.fn(() => ({}));
    await registry.register(id, definition(execute));
    const executor = new Executor({ registry });
    executor.currentStrategy.insertAfter('input_validation', {
      name: 'skip_dispatch', description: 'Skip the business body', removable: true, replaceable: true,
      async execute(ctx) {
        ctx.output = { injected: true };
        return { action: 'skip_to', skipTo: 'return_result' };
      },
    });
    const budget = new CallBudget({ maxCalls: 1 });
    const context = Context.create().withBudget(budget);
    await expect(executor.call(id, {}, context)).rejects.toMatchObject({ code: 'PIPELINE_CONFIGURATION_ERROR' });
    const stream = executor.streamWithCompletion(id, {}, context);
    await expect(stream.chunks.next()).rejects.toMatchObject({ code: 'PIPELINE_CONFIGURATION_ERROR' });
    await expect(stream.completion).resolves.toMatchObject({ reason: 'failed' });
    expect(execute).not.toHaveBeenCalled();
    expect(budget.snapshot()).toEqual({ callsUsed: 0, callsReserved: 0, costUsed: 0 });
  });

  it('does not retry exhausted budgets in the background task loop', async () => {
    const client = new APCore();
    const execute = vi.fn(() => ({}));
    await client.register(id, definition(execute));
    const manager = new AsyncTaskManager({ executor: client.executor });
    const budget = new CallBudget({ maxCalls: 0 });
    try {
      const taskId = await manager.submit(id, {}, {
        context: Context.create().withBudget(budget),
        retry: new RetryConfig({ maxRetries: 1, retryDelayMs: 0 }),
      });
      await vi.waitFor(async () => expect((await manager.getStatus(taskId))?.status).toBe('failed'));
      expect(await manager.getStatus(taskId)).toMatchObject({ retryCount: 0 });
      expect(execute).not.toHaveBeenCalled();
    } finally {
      await manager.shutdown();
    }
  });
});
