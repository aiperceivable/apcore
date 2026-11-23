import { describe, it, expect, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Executor } from '../src/executor.js';
import { FunctionModule } from '../src/decorator.js';
import { Registry } from '../src/registry/registry.js';
import { Middleware } from '../src/middleware/base.js';
import { InvalidInputError, ModuleNotFoundError, ModuleTimeoutError, SchemaValidationError } from '../src/errors.js';

function createSimpleModule(id: string): FunctionModule {
  return new FunctionModule({
    execute: (inputs) => ({ greeting: `Hello, ${inputs['name'] ?? 'world'}!` }),
    moduleId: id,
    inputSchema: Type.Object({ name: Type.Optional(Type.String()) }),
    outputSchema: Type.Object({ greeting: Type.String() }),
    description: 'Greet module',
  });
}

/**
 * Creates a module with a stream() async generator that yields chunks.
 */
function createStreamingModule(id: string): FunctionModule & { stream: (inputs: Record<string, unknown>) => AsyncGenerator<Record<string, unknown>> } {
  const mod = new FunctionModule({
    execute: (inputs) => ({ greeting: `Hello, ${inputs['name'] ?? 'world'}!` }),
    moduleId: id,
    inputSchema: Type.Object({ name: Type.Optional(Type.String()) }),
    outputSchema: Type.Object({ greeting: Type.String() }),
    description: 'Streaming greet module',
  });

  // Attach a stream method to the module
  const streamingMod = mod as FunctionModule & { stream: (inputs: Record<string, unknown>) => AsyncGenerator<Record<string, unknown>> };
  streamingMod.stream = async function* (inputs: Record<string, unknown>): AsyncGenerator<Record<string, unknown>> {
    const name = (inputs['name'] as string) ?? 'world';
    yield { greeting: `Hello, ` };
    yield { greeting: `${name}` };
    yield { greeting: `!` };
  };

  return streamingMod;
}

async function collectChunks(gen: AsyncGenerator<Record<string, unknown>>): Promise<Record<string, unknown>[]> {
  const chunks: Record<string, unknown>[] = [];
  for await (const chunk of gen) {
    chunks.push(chunk);
  }
  return chunks;
}

describe('Executor.stream()', () => {
  it('falls back to single chunk when module has no stream()', async () => {
    const registry = new Registry();
    const mod = createSimpleModule('greet');
    registry.register('greet', mod);

    const executor = new Executor({ registry });
    const chunks = await collectChunks(executor.stream('greet', { name: 'Alice' }));

    expect(chunks).toHaveLength(1);
    expect(chunks[0]['greeting']).toBe('Hello, Alice!');
  });

  it('yields multiple chunks from streaming module', async () => {
    const registry = new Registry();
    const mod = createStreamingModule('greet');
    registry.register('greet', mod);

    const executor = new Executor({ registry });
    const chunks = await collectChunks(executor.stream('greet', { name: 'Bob' }));

    expect(chunks.length).toBeGreaterThan(1);
    expect(chunks[0]['greeting']).toBe('Hello, ');
    expect(chunks[1]['greeting']).toBe('Bob');
    expect(chunks[2]['greeting']).toBe('!');
  });

  it('throws ModuleNotFoundError for unknown module', async () => {
    const registry = new Registry();
    const executor = new Executor({ registry });

    const chunks: Record<string, unknown>[] = [];
    await expect(async () => {
      for await (const chunk of executor.stream('nonexistent')) {
        chunks.push(chunk);
      }
    }).rejects.toThrow(ModuleNotFoundError);
  });

  it('runs before-middleware before streaming and after-middleware on accumulated result', async () => {
    const registry = new Registry();
    const mod = createStreamingModule('echo');
    registry.register('echo', mod);

    const calls: string[] = [];
    class TrackingMiddleware extends Middleware {
      override before() { calls.push('before'); return null; }
      override after() { calls.push('after'); return null; }
    }

    const executor = new Executor({ registry, middlewares: [new TrackingMiddleware()] });
    const chunks = await collectChunks(executor.stream('echo', { name: 'Test' }));

    expect(chunks.length).toBeGreaterThan(0);
    expect(calls).toContain('before');
    expect(calls).toContain('after');
    // before must come first
    expect(calls.indexOf('before')).toBeLessThan(calls.indexOf('after'));
  });

  it('runs before-middleware before fallback and after-middleware on result', async () => {
    const registry = new Registry();
    const mod = createSimpleModule('echo');
    registry.register('echo', mod);

    const calls: string[] = [];
    class TrackingMiddleware extends Middleware {
      override before() { calls.push('before'); return null; }
      override after() { calls.push('after'); return null; }
    }

    const executor = new Executor({ registry, middlewares: [new TrackingMiddleware()] });
    const chunks = await collectChunks(executor.stream('echo', { name: 'Test' }));

    expect(chunks).toHaveLength(1);
    expect(calls).toEqual(['before', 'after']);
  });

  it('handles middleware error recovery in streaming mode', async () => {
    const registry = new Registry();
    const failMod = new FunctionModule({
      execute: () => { throw new Error('stream-boom'); },
      moduleId: 'fail',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      description: 'Failing module',
    });
    registry.register('fail', failMod);

    class RecoveryMiddleware extends Middleware {
      override onError() { return { recovered: true }; }
    }

    const executor = new Executor({ registry, middlewares: [new RecoveryMiddleware()] });
    // Non-streaming module fallback with error should recover
    const chunks = await collectChunks(executor.stream('fail'));
    expect(chunks).toHaveLength(1);
    expect(chunks[0]['recovered']).toBe(true);
  });

  it('accumulates chunks via shallow merge for after-middleware', async () => {
    const registry = new Registry();
    const mod = {
      description: 'multi-key streaming module',
      inputSchema: Type.Object({ prefix: Type.String() }),
      outputSchema: Type.Object({ a: Type.Optional(Type.String()), b: Type.Optional(Type.String()) }),
      execute: async (inputs: Record<string, unknown>) => ({ a: `${inputs['prefix']}_a`, b: `${inputs['prefix']}_b` }),
      async *stream(inputs: Record<string, unknown>) {
        yield { a: `${inputs['prefix']}_a` };
        yield { b: `${inputs['prefix']}_b` };
      },
    };
    registry.register('multi', mod);

    let afterOutput: Record<string, unknown> | null = null;
    const executor = new Executor({ registry });
    executor.useAfter((_mid, _inputs, output) => {
      afterOutput = { ...output };
      return null;
    });

    const chunks: Record<string, unknown>[] = [];
    for await (const chunk of executor.stream('multi', { prefix: 'test' })) {
      chunks.push(chunk);
    }

    expect(chunks).toHaveLength(2);
    expect(chunks[0]).toEqual({ a: 'test_a' });
    expect(chunks[1]).toEqual({ b: 'test_b' });
    // After-middleware should receive the MERGED result
    expect(afterOutput).toEqual({ a: 'test_a', b: 'test_b' });
  });

  it('swallows phase-3 after() failure without re-invoking onError (sync A-D-011)', async () => {
    // Per spec (sync finding A-D-011): chunks are already delivered when
    // phase-3 (post-stream after-middleware/output validation) runs, so
    // failures MUST NOT be re-thrown. Furthermore, the middleware `onError`
    // chain MUST NOT run after partial output emission — that contract is
    // "produce a recovery output before any output is visible", and once
    // chunks are yielded the contract is no longer applicable. apcore-python
    // and apcore-rust only log/emit at this point; TS now matches.
    const registry = new Registry();
    const mod = createStreamingModule('stream_after_fail');
    registry.register('stream_after_fail', mod);

    let onErrorCalled = false;
    class FailingAfter extends Middleware {
      override after(): null {
        throw new Error('post-stream after boom');
      }
      override onError(): null {
        onErrorCalled = true;
        return null;
      }
    }

    const executor = new Executor({ registry, middlewares: [new FailingAfter()] });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    // Chunks were yielded before phase 3, so consumption succeeds; the
    // post-stream failure is swallowed (logged via console.warn).
    const chunks = await collectChunks(executor.stream('stream_after_fail', { name: 'Test' }));
    expect(chunks.length).toBeGreaterThan(0);
    // Cross-language parity: onError MUST NOT be invoked once chunks are out.
    expect(onErrorCalled).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('phase-3 failure'));
    warnSpy.mockRestore();
  });

  it('validates output schema on accumulated streaming result', async () => {
    const registry = new Registry();
    const mod = new FunctionModule({
      execute: () => ({ greeting: 'fallback' }),
      moduleId: 'validated',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ greeting: Type.String() }),
      description: 'Validated module',
    });

    // Attach a stream that produces valid accumulated output
    const streamingMod = mod as FunctionModule & { stream: (inputs: Record<string, unknown>) => AsyncGenerator<Record<string, unknown>> };
    streamingMod.stream = async function* (): AsyncGenerator<Record<string, unknown>> {
      yield { greeting: 'chunk1' };
      yield { greeting: 'chunk2' };
    };

    registry.register('validated', streamingMod);

    const executor = new Executor({ registry });
    // The last chunk is used as the accumulated output for validation
    const chunks = await collectChunks(executor.stream('validated'));
    expect(chunks).toHaveLength(2);
  });

  it('D-99/D-100: enforces global_deadline between chunks from Context.globalDeadline', async () => {
    // Spec v1.50.0 D-100: the budget is the first-class
    // `Context.globalDeadline` field; D-99 puts it on the epoch-SECONDS
    // clock. This case previously pre-populated the private
    // `context.data[CTX_GLOBAL_DEADLINE]` slot in milliseconds — a slot
    // `data`'s shared-by-reference lifetime made unusable as a per-call
    // budget, and a clock a spec-following caller could not write to.
    const registry = new Registry();
    const mod = {
      description: 'slow streaming module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ chunk: Type.Optional(Type.Number()) }),
      execute: () => ({ chunk: 0 }),
      async *stream(): AsyncGenerator<Record<string, unknown>> {
        for (let i = 0; i < 5; i++) {
          yield { chunk: i };
          // Sleep long enough between chunks to push past the deadline.
          await new Promise((resolve) => setTimeout(resolve, 30));
        }
      },
    };
    registry.register('slow.stream', mod);

    const executor = new Executor({ registry });
    // Pre-populate the context with an already-expired global deadline so
    // the first chunk-loop check trips. Use the canonical context.data slot.
    const { Context } = await import('../src/context.js');
    const ctx = Context.create(null, null, null, undefined, null, (Date.now() + 10) / 1000); // ~10ms from now

    const collected: Record<string, unknown>[] = [];
    let caught: Error | null = null;
    try {
      for await (const chunk of executor.stream('slow.stream', {}, ctx)) {
        collected.push(chunk);
      }
    } catch (e) {
      caught = e as Error;
    }

    expect(caught).toBeInstanceOf(ModuleTimeoutError);
    // The first chunk MUST have been yielded before the timeout fires; the
    // deadline is checked between chunks, not before the first one.
    expect(collected.length).toBeGreaterThanOrEqual(1);
    expect(collected.length).toBeLessThan(5);
  });

  it('swallows phase-3 SchemaValidationError when accumulated output is invalid (sync A-D-012)', async () => {
    // Per spec: phase-3 output schema validation runs AFTER chunks are
    // delivered. Spec mandates the failure is swallowed (logged) rather
    // than re-thrown to the consumer.
    const registry = new Registry();
    const streamingMod = new FunctionModule({
      execute: () => ({ count: 1 }),
      moduleId: 'phase3.fail',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ count: Type.Number() }),
      description: 'Streaming module with invalid accumulated output',
    }) as FunctionModule & { stream: () => AsyncGenerator<Record<string, unknown>> };

    streamingMod.stream = async function* () {
      yield { count: 'not-a-number' }; // violates outputSchema
    };

    registry.register('phase3.fail', streamingMod);
    const executor = new Executor({ registry });

    // Should NOT throw — invalid chunk is delivered then phase-3 swallows
    // the validation failure with a console.warn.
    const chunks = await collectChunks(executor.stream('phase3.fail'));
    expect(chunks).toHaveLength(1);
    expect(chunks[0]).toEqual({ count: 'not-a-number' });
  });

  it('D10-001: rejects a non-object (string) stream chunk before delivery with STREAM_CHUNK_NOT_OBJECT', async () => {
    // Cross-SDK parity with apcore-rust deep_merge_chunks_checked / D-58:
    // a chunk that is not a plain JSON object MUST be rejected BEFORE it is
    // yielded. The bad chunk MUST NOT reach the consumer.
    const registry = new Registry();
    const mod = {
      description: 'module streaming a bad string chunk',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ a: Type.Optional(Type.Number()) }),
      execute: () => ({ a: 1 }),
      async *stream(): AsyncGenerator<Record<string, unknown>> {
        yield { a: 1 };
        yield 'not an object' as unknown as Record<string, unknown>;
      },
    };
    registry.register('bad.string.stream', mod);

    const executor = new Executor({ registry });
    const collected: Record<string, unknown>[] = [];
    let caught: unknown = null;
    try {
      for await (const chunk of executor.stream('bad.string.stream')) {
        collected.push(chunk);
      }
    } catch (e) {
      caught = e;
    }

    // The bad string chunk MUST NOT have been delivered.
    expect(collected).toEqual([{ a: 1 }]);
    expect(collected).not.toContain('not an object');

    expect(caught).toBeInstanceOf(InvalidInputError);
    const err = caught as InvalidInputError;
    expect(err.code).toBe('GENERAL_INVALID_INPUT');
    expect(err.details['code']).toBe('STREAM_CHUNK_NOT_OBJECT');
    expect(err.details['actual_type']).toBe('string');
    expect(err.details['chunk_index']).toBe(1);
  });

  it('D10-001: rejects a non-object (array) stream chunk with actual_type "array"', async () => {
    const registry = new Registry();
    const mod = {
      description: 'module streaming a bad array chunk',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ a: Type.Optional(Type.Number()) }),
      execute: () => ({ a: 1 }),
      async *stream(): AsyncGenerator<Record<string, unknown>> {
        yield { a: 1 };
        yield [1, 2, 3] as unknown as Record<string, unknown>;
      },
    };
    registry.register('bad.array.stream', mod);

    const executor = new Executor({ registry });
    const collected: Record<string, unknown>[] = [];
    let caught: unknown = null;
    try {
      for await (const chunk of executor.stream('bad.array.stream')) {
        collected.push(chunk);
      }
    } catch (e) {
      caught = e;
    }

    expect(collected).toEqual([{ a: 1 }]);
    expect(caught).toBeInstanceOf(InvalidInputError);
    const err = caught as InvalidInputError;
    expect(err.code).toBe('GENERAL_INVALID_INPUT');
    expect(err.details['code']).toBe('STREAM_CHUNK_NOT_OBJECT');
    expect(err.details['actual_type']).toBe('array');
    expect(err.details['chunk_index']).toBe(1);
  });

  it('D10-001: all-object stream is delivered and accumulated unchanged', async () => {
    const registry = new Registry();
    let afterOutput: Record<string, unknown> | null = null;
    const mod = {
      description: 'all-object streaming module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ a: Type.Optional(Type.Number()), b: Type.Optional(Type.Number()) }),
      execute: () => ({ a: 1, b: 2 }),
      async *stream(): AsyncGenerator<Record<string, unknown>> {
        yield { a: 1 };
        yield { b: 2 };
      },
    };
    registry.register('all.object.stream', mod);

    const executor = new Executor({ registry });
    executor.useAfter((_mid, _inputs, output) => {
      afterOutput = { ...output };
      return null;
    });

    const chunks = await collectChunks(executor.stream('all.object.stream'));
    expect(chunks).toEqual([{ a: 1 }, { b: 2 }]);
    expect(afterOutput).toEqual({ a: 1, b: 2 });
  });
});
