import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Type } from '@sinclair/typebox';
import { describe, expect, it, vi } from 'vitest';
import { APCore } from '../src/client.js';
import { CancelToken, ExecutionCancelledError } from '../src/cancel.js';
import { Config } from '../src/config.js';
import { Context } from '../src/context.js';
import { ModuleError, ModuleExecuteError } from '../src/errors.js';
import { Executor } from '../src/executor.js';
import { Middleware } from '../src/middleware/base.js';
import { Registry } from '../src/registry/registry.js';
import type { StreamCompletionReason, StreamStepOutcome } from '../src/stream-completion.js';
import { findFixturesRoot } from './spec-repo.js';

const moduleId = 'test.completion';
const moduleDefinition = {
  description: 'Completion test module',
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({ nested: Type.Object({ a: Type.Number() }) }),
  execute: () => ({ nested: { a: 1 } }),
};

async function collect(chunks: AsyncIterable<Record<string, unknown>>) {
  const output: Record<string, unknown>[] = [];
  for await (const chunk of chunks) output.push(chunk);
  return output;
}

describe('streamWithCompletion', () => {
  it.each(['stream', 'streamWithCompletion'] as const)('runs return_result once after draining %s', async (method) => {
    const client = new APCore();
    await client.register(moduleId, {
      ...moduleDefinition,
      async *stream() { yield { nested: { a: 1 } }; },
    });
    const finalize = vi.fn();
    client.addStepMiddleware({ beforeStep(name) {
      if (name === 'return_result') finalize();
    } });

    const chunks = method === 'stream' ? client.stream(moduleId) : client.streamWithCompletion(moduleId).chunks;
    await expect(chunks.next()).resolves.toEqual({ value: { nested: { a: 1 } }, done: false });
    expect(finalize).not.toHaveBeenCalled();
    await expect(chunks.next()).resolves.toMatchObject({ done: true });
    expect(finalize).toHaveBeenCalledOnce();
  });

  it('resumes all configured steps with the original trace and step outputs after draining', async () => {
    const client = new APCore();
    await client.register(moduleId, {
      ...moduleDefinition,
      async *stream() { yield { nested: { a: 1 } }; },
    });
    const events: string[] = [];
    const strategy = client.executor.currentStrategy;
    strategy.insertBefore('execute', {
      name: 'prepare', description: 'Prepare stream', removable: true, replaceable: true,
      provides: ['prepared'], async execute() { events.push('prepared'); return { action: 'continue' }; },
    });
    strategy.insertAfter('execute', {
      name: 'postprocess', description: 'Process the merged result', removable: true, replaceable: true,
      requires: ['prepared', 'output'], provides: ['processed'],
      async execute() { events.push('processed'); return { action: 'continue' }; },
    });
    strategy.replace('output_validation', {
      name: 'output_validation', description: 'Validate the merged result', removable: true, replaceable: true,
      requires: ['prepared', 'processed', 'output'], provides: ['validated_output'],
      async execute(context) {
        events.push('validated');
        context.validatedOutput = context.output;
        return { action: 'continue' };
      },
    });
    client.addStepMiddleware({ beforeStep(name, state) {
      if (name === 'output_validation') {
        expect(Object.hasOwn(state.outputs, 'prepare')).toBe(true);
        expect(Object.hasOwn(state.outputs, 'postprocess')).toBe(true);
        expect(state.context.trace?.steps.map((step) => step.name)).toContain('execute');
      }
    } });

    const stream = client.streamWithCompletion(moduleId);
    await expect(stream.chunks.next()).resolves.toEqual({ value: { nested: { a: 1 } }, done: false });
    expect(events).toEqual(['prepared']);
    await expect(stream.chunks.next()).resolves.toMatchObject({ done: true });
    expect(events).toEqual(['prepared', 'processed', 'validated']);
    await expect(stream.completion).resolves.toMatchObject({
      reason: 'completed', outputValidation: { status: 'passed' }, middlewareAfter: { status: 'passed' },
    });
  });

  it.each([null, undefined])('rejects an empty execute result (%s) without reporting successful validation', async (output) => {
    const client = new APCore();
    await client.register(moduleId, { ...moduleDefinition, execute: () => output });
    await expect(client.call(moduleId)).rejects.toMatchObject({ code: 'MODULE_EXECUTE_ERROR' });
    const execution = client.streamWithCompletion(moduleId);
    await expect(collect(execution.chunks)).rejects.toMatchObject({ code: 'MODULE_EXECUTE_ERROR' });
    await expect(execution.completion).resolves.toMatchObject({
      reason: 'failed', errorCode: 'MODULE_EXECUTE_ERROR',
    });
  });

  it('runs the unary fallback once and captures the snapshot before mutating after middleware', async () => {
    const client = new APCore();
    const execute = vi.fn(moduleDefinition.execute);
    await client.register(moduleId, { ...moduleDefinition, execute });
    client.useAfter((_moduleId, _inputs, output) => {
      (output.nested as { a: number }).a = 2;
      return output;
    });

    const stream = client.streamWithCompletion(moduleId);
    expect(execute).not.toHaveBeenCalled();
    expect(await collect(stream.chunks)).toEqual([{ nested: { a: 2 } }]);
    expect(await stream.completion).toEqual({
      reason: 'completed', output: { nested: { a: 1 } }, errorCode: null,
      outputValidation: { status: 'passed' }, middlewareAfter: { status: 'passed' },
    });
    expect(execute).toHaveBeenCalledOnce();
  });

  it('keeps the chunk and reports an ordinary native after failure', async () => {
    const client = new APCore();
    await client.register(moduleId, {
      ...moduleDefinition,
      async *stream() { yield { nested: { a: 9 } }; },
    });
    client.useAfter(() => { throw new ModuleExecuteError(moduleId, 'after failure'); });

    const stream = client.streamWithCompletion(moduleId);
    expect(await collect(stream.chunks)).toEqual([{ nested: { a: 9 } }]);
    expect(await stream.completion).toEqual({
      reason: 'completed', output: { nested: { a: 9 } }, errorCode: null,
      outputValidation: { status: 'passed' }, middlewareAfter: { status: 'failed', disposition: 'unhandled' },
    });
  });

  it('closes before first consumption without starting the module or cancelling the external owner', async () => {
    const registry = new Registry();
    const execute = vi.fn(moduleDefinition.execute);
    await registry.register(moduleId, { ...moduleDefinition, execute });
    const executor = new Executor({ registry });
    const parent = new CancelToken();
    const stream = executor.streamWithCompletion(moduleId, {}, Context.create(null, null, parent));

    await stream.close();
    expect(await stream.chunks.next()).toMatchObject({ done: true });
    expect(await stream.completion).toEqual({
      reason: 'consumer_closed', output: null, errorCode: null,
      outputValidation: { status: 'skipped', reason: 'not_reached' },
      middlewareAfter: { status: 'skipped', reason: 'not_reached' },
    });
    expect(execute).not.toHaveBeenCalled();
    expect(parent.isCancelled).toBe(false);
  });

  it('snapshots the exact validation input after step middleware', async () => {
    const client = new APCore();
    await client.register(moduleId, {
      ...moduleDefinition,
      async *stream() { yield { nested: { a: 1 } }; },
    });
    client.addStepMiddleware({ beforeStep(name, { context }) {
      if (name === 'output_validation' && context.output?.nested) {
        (context.output.nested as { a: number }).a = 42;
      }
    } });

    const stream = client.streamWithCompletion(moduleId);
    expect(await collect(stream.chunks)).toEqual([{ nested: { a: 1 } }]);
    expect((await stream.completion).output).toEqual({ nested: { a: 42 } });
    await stream.close();
    expect((await stream.completion).reason).toBe('completed');
  });

  it('reports a late pipeline error outside the two post-processing step outcomes', async () => {
    const client = new APCore();
    await client.register(moduleId, {
      ...moduleDefinition,
      async *stream() { yield { nested: { a: 1 } }; },
    });
    client.addStepMiddleware({ beforeStep(name, { context }) {
      if (name === 'return_result' && context.output !== null) {
        throw new ModuleExecuteError(moduleId, 'unable to finalize');
      }
    } });

    const stream = client.streamWithCompletion(moduleId);
    expect(await collect(stream.chunks)).toEqual([{ nested: { a: 1 } }]);
    expect(await stream.completion).toEqual({
      reason: 'failed', output: { nested: { a: 1 } }, errorCode: 'MODULE_EXECUTE_ERROR',
      outputValidation: { status: 'passed' }, middlewareAfter: { status: 'passed' },
    });
  });

  it('leaves the output absent when a hook prevents the validation boundary', async () => {
    const client = new APCore();
    await client.register(moduleId, {
      ...moduleDefinition,
      async *stream() { yield { nested: { a: 1 } }; },
    });
    client.addStepMiddleware({ beforeStep(name) {
      if (name === 'output_validation') throw new Error('validation did not run');
    } });

    const stream = client.streamWithCompletion(moduleId);
    expect(await collect(stream.chunks)).toEqual([{ nested: { a: 1 } }]);
    expect(await stream.completion).toMatchObject({
      reason: 'completed', output: null, outputValidation: { status: 'failed', disposition: 'unhandled' },
    });
  });

  it.each(['native', 'fallback'] as const)('honors cancellation of the effective %s context', async (mode) => {
    const client = new APCore();
    await client.register(moduleId, mode === 'fallback' ? moduleDefinition : {
      ...moduleDefinition,
      async *stream() { yield { nested: { a: 1 } }; },
    });
    const parent = new CancelToken();
    client.useAfter((_id, _input, _output, context) => {
      context.cancelToken?.cancel();
      return null;
    });
    const stream = client.streamWithCompletion(moduleId, {}, Context.create(null, null, parent));
    await expect(collect(stream.chunks)).rejects.toMatchObject({ code: 'EXECUTION_CANCELLED' });
    expect(await stream.completion).toMatchObject({
      reason: 'cancelled', errorCode: 'EXECUTION_CANCELLED', output: { nested: { a: 1 } },
    });
    expect(parent.isCancelled).toBe(false);
  });

  it.each(['native', 'fallback'] as const)('applies the configured deadline during %s after middleware', async (mode) => {
    vi.useFakeTimers();
    let release!: () => void;
    let entered!: () => void;
    const started = new Promise<void>((resolve) => { entered = resolve; });
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    const parent = new CancelToken();
    try {
      const client = new APCore({ config: new Config({ executor: { global_timeout: 100 } }) });
      await client.register(moduleId, mode === 'fallback' ? moduleDefinition : {
        ...moduleDefinition,
        async *stream() { yield { nested: { a: 1 } }; },
      });
      class BlockingAfter extends Middleware {
        override async after() { entered(); await blocked; return null; }
      }
      client.use(new BlockingAfter());
      const execution = client.streamWithCompletion(moduleId, {}, Context.create(null, null, parent));
      const interruption = expect(collect(execution.chunks)).rejects.toMatchObject({ code: 'MODULE_TIMEOUT' });
      await started;
      await vi.advanceTimersByTimeAsync(100);
      await interruption;
      expect(await execution.completion).toMatchObject({
        reason: 'timed_out',
        output: { nested: { a: 1 } },
        outputValidation: { status: 'passed' },
        middlewareAfter: { status: 'skipped', reason: 'not_reached' },
      });
      expect(parent.signal.aborted).toBe(false);
    } finally {
      release();
      await parent.drain();
      vi.useRealTimers();
    }
  });

  it('settles close before a pending producer pull or its cleanup has returned', async () => {
    let entered!: () => void;
    let release!: () => void;
    const started = new Promise<void>((resolve) => { entered = resolve; });
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    const cleanedUp = vi.fn();
    const registry = new Registry();
    const parent = new CancelToken();
    let producerSignal: AbortSignal | undefined;
    await registry.register(moduleId, {
      ...moduleDefinition,
      async *stream(_inputs: Record<string, unknown>, context: Context) {
        producerSignal = context.signal;
        try {
          entered();
          await blocked;
          yield { nested: { a: 1 } };
        } finally {
          cleanedUp();
        }
      },
    });
    const executor = new Executor({ registry });
    const stream = executor.streamWithCompletion(moduleId, {}, Context.create(null, null, parent));
    const pending = stream.chunks.next();
    try {
      await started;
      await stream.close();
      expect(await stream.completion).toMatchObject({ reason: 'consumer_closed', output: null });
      expect(await pending).toMatchObject({ done: true });
      expect(producerSignal?.aborted).toBe(true);
      expect(parent.isCancelled).toBe(false);
      expect(cleanedUp).not.toHaveBeenCalled();
      expect(registry.refCount.get(moduleId)).toBe(1);
    } finally {
      release();
      await parent.drain();
    }
    expect(cleanedUp).toHaveBeenCalledOnce();
    expect(registry.refCount.has(moduleId)).toBe(false);
  });
});

type Chunk = Record<string, unknown>;
interface CompletionCase {
  id: string;
  input: {
    mode: 'native' | 'fallback';
    validation: 'pass' | 'invalid' | 'removed' | 'no_match' | 'ignored' | 'recovered';
    after: 'pass' | 'mutate' | 'fail' | 'cancel' | 'block';
    action: 'drain' | 'fail_after_first' | 'recover_after_first' | 'close_before_first' |
      'close_after_first' | 'cancel_after_first' | 'timeout_after_first' |
      'timeout_during_after' | 'cancel_during_after';
    chunks: Chunk[];
    max_merge_depth?: number;
  };
  expected: {
    reason: StreamCompletionReason;
    output: Chunk | null;
    output_validation: StreamStepOutcome;
    middleware_after: StreamStepOutcome;
    error_code: string | null;
    execution_count: number;
    chunks: Chunk[];
  };
}

const fixture = JSON.parse(
  readFileSync(join(findFixturesRoot(), 'stream_completion.json'), 'utf8'),
) as { test_cases: CompletionCase[] };

function createFixtureClient(input: CompletionCase['input'], blockAfter: () => Promise<void>): APCore {
  const pipeline = input.validation === 'removed' ? { remove: ['output_validation'] }
    : input.validation === 'no_match' ? { configure: { output_validation: { match_modules: ['other.*'] } } }
    : input.validation === 'ignored' ? { configure: { output_validation: { ignore_errors: true } } }
    : {};
  const client = new APCore({ config: new Config({
    pipeline,
    stream: input.max_merge_depth === undefined ? {} : { max_merge_depth: input.max_merge_depth },
  }) });
  if (input.validation === 'recovered') {
    client.addStepMiddleware({ onStepError: (name) => name === 'output_validation' ? { recovered: true } : null });
  }
  class CaseMiddleware extends Middleware {
    override async after(_moduleId: string, _inputs: Chunk, output: Chunk): Promise<Chunk | null> {
      if (input.after === 'mutate') output.value = 2;
      if (input.after === 'fail') throw new ModuleExecuteError(moduleId, 'after failure');
      if (input.after === 'cancel') throw new ExecutionCancelledError();
      if (input.after === 'block') await blockAfter();
      return null;
    }

    override onError(): Chunk | null {
      return input.action === 'recover_after_first' ? { recovered: true } : null;
    }
  }
  client.use(new CaseMiddleware());
  return client;
}

describe('conformance: stream_completion', () => {
  it.each(fixture.test_cases)('$id', async ({ input, expected }) => {
    const timed = input.action === 'timeout_after_first' || input.action === 'timeout_during_after';
    if (timed) vi.useFakeTimers();
    const parent = new CancelToken();
    let afterEntered!: () => void;
    const afterStarted = new Promise<void>((resolve) => { afterEntered = resolve; });
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    try {
      const client = createFixtureClient(input, () => { afterEntered(); return blocked; });
      let executionCount = 0;
      const chunks = input.chunks.map((chunk) => structuredClone(chunk));
      const execute = (): Chunk => { executionCount++; return chunks[0]; };
      const module = {
        ...moduleDefinition,
        execute,
        outputSchema: input.validation === 'pass' ? Type.Object({}) : Type.Object({ value: Type.Integer() }),
      };
      await client.register(moduleId, input.mode === 'fallback' ? module : {
        ...module,
        async *stream() {
          executionCount++;
          yield* chunks;
          if (input.action === 'timeout_after_first') await blocked;
          if (input.action === 'fail_after_first' || input.action === 'recover_after_first') {
            throw new ModuleExecuteError(moduleId, 'stream failure');
          }
        },
      });
      const context = Context.create(null, null, parent, undefined, null, timed ? (Date.now() + 100) / 1000 : null);
      const stream = client.streamWithCompletion(moduleId, {}, context);
      const actualChunks: Chunk[] = [];
      if (input.action === 'close_before_first') await stream.close();
      const consume = (async () => {
        try {
          for await (const chunk of stream.chunks) {
            actualChunks.push(chunk);
            if (input.action === 'close_after_first') break;
            if (input.action === 'cancel_after_first') parent.cancel();
          }
        } catch (error) {
          if (!(error instanceof ModuleError)) throw error;
          expect(error.code).toBe(expected.error_code);
        }
      })();
      if (input.after === 'block') await afterStarted;
      if (input.action === 'cancel_during_after') parent.cancel();
      if (timed) await vi.advanceTimersByTimeAsync(100);
      await consume;

      const completion = await stream.completion;
      expect(completion.reason).toEqual(expected.reason);
      expect(completion.output).toEqual(expected.output);
      expect(completion.outputValidation).toEqual(expected.output_validation);
      expect(completion.middlewareAfter).toEqual(expected.middleware_after);
      expect(completion.errorCode).toEqual(expected.error_code);
      expect(executionCount).toEqual(expected.execution_count);
      expect(actualChunks).toEqual(expected.chunks);
    } finally {
      release();
      await parent.drain();
      if (timed) vi.useRealTimers();
    }
  });
});
