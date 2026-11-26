/**
 * Regression for A-D-006 — when an EventEmitter is wired into the Executor, a
 * stream Phase-3 (post-stream output validation) failure MUST emit an
 * `apcore.stream.post_validation_failed` event (chunks are already delivered, so
 * the error is not re-raised). Mirrors apcore-python executor.py.
 */

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it, expect, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { CancelToken, ExecutionCancelledError } from '../src/cancel.js';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import { Context } from '../src/context.js';
import { ModuleError, ModuleExecuteError } from '../src/errors.js';
import { Executor } from '../src/executor.js';
import { FunctionModule } from '../src/decorator.js';
import { Registry } from '../src/registry/registry.js';
import type { ApCoreEvent } from '../src/events/emitter.js';
import { Middleware } from '../src/middleware/base.js';
import { findFixturesRoot } from './spec-repo.js';

type Chunk = Record<string, unknown>;
interface PostProcessingCase {
  id: string;
  input: { chunks: Chunk[]; after_error_code: string };
  expected: { chunks: Chunk[]; error_code: string | null; on_error_calls: number; post_failure_events: number };
}

const fixture = JSON.parse(
  readFileSync(join(findFixturesRoot(), 'stream_post_processing.json'), 'utf8'),
) as { test_cases: PostProcessingCase[] };

async function drain(gen: AsyncGenerator<Record<string, unknown>>): Promise<Record<string, unknown>[]> {
  const chunks: Record<string, unknown>[] = [];
  for await (const chunk of gen) chunks.push(chunk);
  return chunks;
}

describe('stream Phase-3 post-validation failure emits event (A-D-006)', () => {
  it.each(['MODULE_TIMEOUT', 'EXECUTION_CANCELLED'])('reports %s before a pending after hook has finished', async (code) => {
    vi.useFakeTimers();
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    let enter!: () => void;
    const entered = new Promise<void>((resolve) => { enter = resolve; });
    const parent = new CancelToken();
    try {
      const client = new APCore({ config: new Config({ executor: { global_timeout: 100 } }) });
      const moduleId = 'test.poststream';
      await client.register(moduleId, {
        inputSchema: Type.Object({}),
        outputSchema: Type.Object({ value: Type.Integer() }),
        description: 'Stream with a blocked after hook',
        execute: () => ({ value: 0 }),
        async *stream() { yield { value: 1 }; },
      });
      let signal: AbortSignal | undefined;
      class BlockedAfter extends Middleware {
        override async after(_id: string, _inputs: Chunk, _output: Chunk, context: Context) {
          signal = context.signal;
          enter();
          await blocked;
          return null;
        }
      }
      client.use(new BlockedAfter());
      const stream = client.stream(moduleId, {}, Context.create(null, null, parent));
      await expect(stream.next()).resolves.toEqual({ done: false, value: { value: 1 } });
      let terminal: unknown;
      const lastPull = stream.next().then(
        (result) => { terminal = result; },
        (error: unknown) => { terminal = error; },
      );
      await entered;
      if (code === 'EXECUTION_CANCELLED') parent.cancel();
      await vi.advanceTimersByTimeAsync(code === 'MODULE_TIMEOUT' ? 100 : 0);
      try {
        expect(terminal).toMatchObject({ code });
        expect(signal?.aborted).toBe(true);
        expect(parent.signal.aborted).toBe(code === 'EXECUTION_CANCELLED');
      } finally {
        release();
        await lastPull;
      }
    } finally {
      release();
      await parent.drain();
      vi.useRealTimers();
    }
  });

  it.each(fixture.test_cases)('$id', async ({ input, expected }) => {
    const client = new APCore({
      config: new Config({ sys_modules: { enabled: true, events: { enabled: true } } }),
    });
    const moduleId = 'test.stream';
    await client.register(moduleId, {
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ value: Type.Integer() }),
      description: 'Stream post-processing test module',
      execute: () => ({ value: 0 }),
      async *stream() { yield* input.chunks; },
    });
    const onError = vi.fn(() => null);
    class FailingAfter extends Middleware {
      override after(): never {
        if (input.after_error_code === 'EXECUTION_CANCELLED') throw new ExecutionCancelledError();
        throw new ModuleExecuteError(moduleId, 'after failed');
      }
      override onError(): null { return onError(); }
    }
    client.use(new FailingAfter());
    const events = vi.fn();
    client.on('apcore.stream.post_validation_failed', events);

    const chunks: Chunk[] = [];
    let errorCode: string | null = null;
    try {
      for await (const chunk of client.stream(moduleId, {})) chunks.push(chunk);
    } catch (error) {
      if (!(error instanceof ModuleError)) throw error;
      errorCode = error.code;
    }
    await client.events?.flush();
    expect(chunks).toEqual(expected.chunks);
    expect(errorCode).toBe(expected.error_code);
    expect(onError).toHaveBeenCalledTimes(expected.on_error_calls);
    expect(events).toHaveBeenCalledTimes(expected.post_failure_events);
  });

  it('delivers apcore.stream.post_validation_failed through APCore.on()', async () => {
    const client = new APCore({
      config: new Config({ sys_modules: { enabled: true, events: { enabled: true } } }),
    });
    // Output schema requires an integer `count`, but the stream yields a string,
    // so post-stream output validation (Phase 3) fails after chunks are sent.
    const mod = new FunctionModule({
      execute: () => ({ count: 1 }),
      moduleId: 'bad.stream',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ count: Type.Integer() }),
      description: 'Stream whose accumulated output fails validation',
    }) as FunctionModule & { stream: (i: Record<string, unknown>) => AsyncGenerator<Record<string, unknown>> };
    mod.stream = async function* () {
      yield { count: 'not-an-int' };
    };
    await client.register('bad.stream', mod);

    const events: ApCoreEvent[] = [];
    client.on('apcore.stream.post_validation_failed', (event) => { events.push(event); });

    // Chunks are delivered; Phase-3 failure is swallowed (no throw).
    const chunks = await drain(client.stream('bad.stream', {}));
    expect(chunks).toEqual([{ count: 'not-an-int' }]);

    await client.events?.flush();
    expect(events).toMatchObject([{
      eventType: 'apcore.stream.post_validation_failed',
      moduleId: 'bad.stream',
      severity: 'error',
    }]);
  });

  it('does not throw and does not require an EventEmitter (optional wiring)', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ count: 1 }),
      moduleId: 'bad.stream2',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ count: Type.Integer() }),
      description: 'Stream whose accumulated output fails validation',
    }) as FunctionModule & { stream: (i: Record<string, unknown>) => AsyncGenerator<Record<string, unknown>> };
    mod.stream = async function* () {
      yield { count: 'not-an-int' };
    };
    registry.register('bad.stream2', mod);

    const executor = new Executor({ registry });
    // No emitter wired — must still complete without throwing.
    const chunks = await drain(executor.stream('bad.stream2', {}));
    expect(chunks.length).toBeGreaterThan(0);
  });
});
