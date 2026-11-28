import { Type } from '@sinclair/typebox';
import { describe, expect, it, vi } from 'vitest';
import { APCore, CallBudget, CancelToken, Context } from '../src/index.js';
import { AsyncTaskManager } from '../src/async-task.js';

describe('Context interaction callbacks', () => {
  it('derives independent capabilities and excludes them from the wire context', async () => {
    const progress = vi.fn();
    const elicit = vi.fn(() => ({ action: 'accept' as const, content: { name: 'A' } }));
    const schema = { type: 'object' };
    const base = Context.create();
    const derived = base.withProgressReporter(progress).withElicitationHandler(elicit);
    const child = derived.child('interaction.run').withProgressReporter(null);
    const clearedElicitation = derived.withElicitationHandler(null);

    await base.reportProgress(1, 2, 'starting');
    await child.reportProgress(1, 2, 'starting');
    expect(progress).not.toHaveBeenCalled();
    await derived.reportProgress(1, 2, 'starting');
    expect(progress).toHaveBeenCalledExactlyOnceWith(1, 2, 'starting');
    await expect(clearedElicitation.elicit('Name?', schema)).resolves.toBeNull();
    await expect(child.elicit('Name?', schema)).resolves.toEqual({ action: 'accept', content: { name: 'A' } });
    expect(elicit).toHaveBeenCalledExactlyOnceWith('Name?', schema);
    expect(derived.toJSON()).toEqual(base.toJSON());
    await expect(Context.fromJSON(derived.toJSON()).elicit('Name?')).resolves.toBeNull();
  });

  it('carries callbacks through unary and stream execution contexts', async () => {
    const client = new APCore();
    const progress = vi.fn(async () => {});
    const elicit = vi.fn(async () => ({ action: 'decline' as const }));
    const ctx = Context.create().withProgressReporter(progress).withElicitationHandler(elicit);
    const id = 'interaction.run';
    await client.register(id, {
      description: 'Report work and request a response', inputSchema: Type.Object({}), outputSchema: Type.Object({}),
      async execute(_input: unknown, context: Context) {
        await context.reportProgress(1);
        return { result: await context.elicit('Continue?') };
      },
    });

    await expect(client.call(id, {}, ctx)).resolves.toEqual({ result: { action: 'decline' } });
    await expect(client.streamWithCompletion(id, {}, ctx).chunks.next()).resolves.toMatchObject({
      value: { result: { action: 'decline' } },
    });
    expect(progress).toHaveBeenCalledTimes(2);
    expect(elicit).toHaveBeenCalledTimes(2);
  });

  it('interrupts callbacks on cancellation and checks deadlines before invoking the provider', async () => {
    const token = new CancelToken();
    let finish!: () => void;
    let entered!: () => void;
    const started = new Promise<void>(resolve => { entered = resolve; });
    const context = Context.create(null, null, token).withProgressReporter(() => new Promise<void>(resolve => {
      finish = resolve;
      entered();
    }));
    const report = context.reportProgress(1);
    await started;
    token.cancel();
    try {
      await expect(report).rejects.toMatchObject({ code: 'EXECUTION_CANCELLED' });
    } finally {
      finish();
    }
    const expiredProvider = vi.fn(() => ({ action: 'cancel' as const }));
    const expired = Context.create(null, null, null, undefined, undefined, Date.now() / 1000 - 1)
      .withElicitationHandler(expiredProvider);
    await expect(expired.elicit('Continue?')).rejects.toMatchObject({ code: 'MODULE_TIMEOUT' });
    expect(expiredProvider).not.toHaveBeenCalled();
  });

  it('preserves interactions and budget on the background task cancellation context', async () => {
    const client = new APCore();
    const manager = new AsyncTaskManager({ executor: client.executor });
    const id = 'interaction.background';
    await client.register(id, {
      description: 'Call back from a background task', inputSchema: Type.Object({}), outputSchema: Type.Object({}),
      async execute(_input: unknown, context: Context) {
        await context.reportProgress(1);
        context.recordCost(2);
        return { response: await context.elicit('Continue?') };
      },
    });
    const progress = vi.fn();
    const budget = new CallBudget({ maxCalls: 1, maxCost: 2 });
    const context = Context.create().withProgressReporter(progress).withElicitationHandler(() => ({ action: 'cancel' }))
      .withBudget(budget);
    try {
      const task = await manager.submit(id, {}, { context });
      await vi.waitFor(async () => expect((await manager.getStatus(task))?.status).toBe('completed'));
      await expect(manager.getResult(task)).resolves.toEqual({ response: { action: 'cancel' } });
      expect(progress).toHaveBeenCalledOnce();
      expect(budget.snapshot()).toEqual({ callsUsed: 1, callsReserved: 0, costUsed: 2 });
    } finally {
      await manager.shutdown();
    }
  });
});
