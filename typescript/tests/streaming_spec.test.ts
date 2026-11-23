/**
 * Spec-traced contract tests for the apcore streaming feature (TypeScript SDK).
 *
 * MIRRORS the canonical Python suite
 * (apcore-python/tests/test_streaming_spec.py) clause-for-clause. Each
 * `it(...)` name begins with the SAME clause-id string used in the Python
 * docstrings (format `streaming.<method>.<kind>.<detail>`), so a cross-language
 * diff can match rows by exact clause-id.
 *
 * Source spec: apcore/docs/features/streaming.md — "## Contract: Module.stream".
 *
 * The streaming contract is defined on the module's `stream()` method but is
 * *enforced* by the executor's three-phase pipeline (`Executor.stream()`).
 * These tests exercise the contract through `Executor.stream()`.
 *
 * Conventions copied from tests/test-executor-stream.test.ts.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { Context } from '../src/context.js';
import { Middleware } from '../src/middleware/base.js';
import { InvalidInputError, ModuleError, SchemaValidationError } from '../src/errors.js';

// --------------------------------------------------------------------------- //
// Fixtures / helpers (mirror tests/test-executor-stream.test.ts)
// --------------------------------------------------------------------------- //

const CountInputSchema = Type.Object({ count: Type.Integer() });
const CountOutputSchema = Type.Object({ value: Type.Optional(Type.Integer()) });

/** Minimal streaming module: yields {value: i} for i in 1..count. */
function makeStreamingCounter() {
  return {
    description: 'Streaming counter',
    inputSchema: CountInputSchema,
    outputSchema: CountOutputSchema,
    execute: (inputs: Record<string, unknown>): Record<string, unknown> => ({
      value: inputs['count'],
    }),
    async *stream(inputs: Record<string, unknown>): AsyncGenerator<Record<string, unknown>> {
      const count = inputs['count'] as number;
      for (let i = 1; i <= count; i++) {
        yield { value: i };
      }
    },
  };
}

/** Non-streaming module — only execute(). */
function makePlainModule() {
  return {
    description: 'Plain module',
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({}),
    execute: (): Record<string, unknown> => ({ result: 'done' }),
  };
}

/** Yields overlapping nested dicts to exercise the deep-merge accumulator. */
function makeNestedMergeModule() {
  return {
    description: 'Nested merge module',
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({}),
    execute: (): Record<string, unknown> => ({}),
    async *stream(): AsyncGenerator<Record<string, unknown>> {
      yield { content: 'Hello', metadata: { tokens: 1 } };
      yield { content: ' world', metadata: { tokens: 1, model: 'gpt-4' } };
    },
  };
}

/** Yields one valid object chunk then a single non-object chunk. */
function makeBadChunkModule(badChunk: unknown) {
  return {
    description: 'Bad chunk module',
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({}),
    execute: (): Record<string, unknown> => ({}),
    async *stream(): AsyncGenerator<Record<string, unknown>> {
      yield { a: 1 };
      yield badChunk as Record<string, unknown>; // deliberately invalid
    },
  };
}

function makeExecutor(
  module: unknown = null,
  moduleId = 'test.module',
  middlewares: Middleware[] | null = null,
): Executor {
  const reg = new Registry();
  if (module !== null) {
    reg.register(moduleId, module as never);
  }
  return new Executor({ registry: reg, middlewares });
}

async function collect(
  stream: AsyncGenerator<Record<string, unknown>>,
): Promise<Record<string, unknown>[]> {
  const chunks: Record<string, unknown>[] = [];
  for await (const chunk of stream) {
    chunks.push(chunk);
  }
  return chunks;
}

// --------------------------------------------------------------------------- //
// INPUT
// --------------------------------------------------------------------------- //

describe('streaming — inputs', () => {
  it('streaming.stream.input.inputs_validated_against_schema: schema-conformant input streams normally', async () => {
    const ex = makeExecutor(makeStreamingCounter(), 'counter');
    const chunks = await collect(ex.stream('counter', { count: 2 }));
    expect(chunks).toEqual([{ value: 1 }, { value: 2 }]);
  });

  it('streaming.stream.input.context_accepted: explicit Context accepted', async () => {
    const ex = makeExecutor(makeStreamingCounter(), 'counter');
    const ctx = Context.create();
    const chunks = await collect(ex.stream('counter', { count: 1 }, ctx));
    expect(chunks).toEqual([{ value: 1 }]);
  });
});

// --------------------------------------------------------------------------- //
// ERROR
// --------------------------------------------------------------------------- //

describe('streaming — errors', () => {
  it('streaming.stream.error.schema_validation_on_bad_inputs: SchemaValidationError(SCHEMA_VALIDATION_ERROR) on bad inputs', async () => {
    const ex = makeExecutor(makeStreamingCounter(), 'counter');
    let err: unknown;
    try {
      // count must be int; a string fails CountInput validation.
      await collect(ex.stream('counter', { count: 'not-an-int' }));
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(SchemaValidationError);
    expect((err as ModuleError).code).toBe('SCHEMA_VALIDATION_ERROR');
  });

  it('streaming.stream.error.mid_stream_error_surfaced: mid-stream error surfaced, chunks before it delivered', async () => {
    const midFailModule = {
      description: 'Mid-fail module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: (): Record<string, unknown> => ({}),
      async *stream(): AsyncGenerator<Record<string, unknown>> {
        yield { ok: 1 };
        throw new Error('boom mid-stream');
      },
    };
    const ex = makeExecutor(midFailModule, 'midfail');
    const delivered: Record<string, unknown>[] = [];
    let err: unknown;
    try {
      for await (const chunk of ex.stream('midfail', {})) {
        delivered.push(chunk);
      }
    } catch (e) {
      err = e;
    }
    // Executor wraps the mid-stream exception into a ModuleError subclass
    // (ModuleExecuteError) before surfacing it to the consumer.
    expect(err).toBeInstanceOf(ModuleError);
    expect(delivered).toEqual([{ ok: 1 }]);
  });
});

// --------------------------------------------------------------------------- //
// RETURN  (D-58 chunk-shape rule + lazy async iterator)
// --------------------------------------------------------------------------- //

describe('streaming — return', () => {
  it('streaming.stream.return.async_iterator_of_objects: returns lazy async iterator of object chunks', async () => {
    const ex = makeExecutor(makeStreamingCounter(), 'counter');
    const stream = ex.stream('counter', { count: 3 });
    expect(typeof (stream as { [Symbol.asyncIterator]?: unknown })[Symbol.asyncIterator]).toBe(
      'function',
    );
    expect(typeof (stream as { next?: unknown }).next).toBe('function');
    const chunks = await collect(stream);
    expect(chunks).toEqual([{ value: 1 }, { value: 2 }, { value: 3 }]);
    expect(chunks.every((c) => typeof c === 'object' && c !== null && !Array.isArray(c))).toBe(true);
  });

  it('streaming.stream.return.d58_reject_non_object_string: string chunk rejected (actual_type=string) before delivery', async () => {
    const ex = makeExecutor(makeBadChunkModule('nope'), 'bad');
    const delivered: unknown[] = [];
    let err: unknown;
    try {
      for await (const chunk of ex.stream('bad', {})) {
        delivered.push(chunk);
      }
    } catch (e) {
      err = e;
    }
    expect(delivered).toEqual([{ a: 1 }]); // invalid chunk never delivered
    expect(err).toBeInstanceOf(InvalidInputError);
    const e = err as InvalidInputError;
    expect(e.code).toBe('GENERAL_INVALID_INPUT');
    expect(e.details['code']).toBe('STREAM_CHUNK_NOT_OBJECT');
    expect(e.details['actual_type']).toBe('string');
    expect(e.details['chunk_index']).toBe(1);
  });

  it('streaming.stream.return.d58_reject_non_object_array: array chunk rejected (actual_type=array) before delivery', async () => {
    const ex = makeExecutor(makeBadChunkModule([1, 2]), 'bad');
    const delivered: unknown[] = [];
    let err: unknown;
    try {
      for await (const chunk of ex.stream('bad', {})) {
        delivered.push(chunk);
      }
    } catch (e) {
      err = e;
    }
    expect(delivered).toEqual([{ a: 1 }]);
    expect(err).toBeInstanceOf(InvalidInputError);
    expect((err as InvalidInputError).details['actual_type']).toBe('array');
  });

  it.each([
    [3, 'number'],
    [true, 'bool'],
    [null, 'null'],
  ])(
    'streaming.stream.return.d58_reject_non_object_scalars: scalar chunk %p rejected (actual_type=%s)',
    async (badChunk, expectedType) => {
      const ex = makeExecutor(makeBadChunkModule(badChunk), 'bad');
      const delivered: unknown[] = [];
      let err: unknown;
      try {
        for await (const chunk of ex.stream('bad', {})) {
          delivered.push(chunk);
        }
      } catch (e) {
        err = e;
      }
      expect(delivered).toEqual([{ a: 1 }]);
      expect(err).toBeInstanceOf(InvalidInputError);
      expect((err as InvalidInputError).details['actual_type']).toBe(expectedType);
      expect((err as InvalidInputError).details['chunk_index']).toBe(1);
    },
  );
});

// --------------------------------------------------------------------------- //
// PROPERTY
// --------------------------------------------------------------------------- //

describe('streaming — properties', () => {
  it('streaming.stream.property.async: stream() returns an async iterator', async () => {
    const mod = makeStreamingCounter();
    // The module's stream() is an async generator function.
    expect(mod.stream.constructor.name).toBe('AsyncGeneratorFunction');
    const ex = makeExecutor(mod, 'counter');
    const result = ex.stream('counter', { count: 1 });
    expect(typeof (result as { next?: unknown }).next).toBe('function');
    const first = await result.next();
    expect(first.value).toEqual({ value: 1 });
  });

  it('streaming.stream.property.thread_safe_false: >=8 independent concurrent streams stay isolated', async () => {
    const ex = makeExecutor(makeStreamingCounter(), 'counter');
    const run = async (n: number): Promise<Record<string, unknown>[]> =>
      collect(ex.stream('counter', { count: n }));
    const results = await Promise.all([1, 2, 3, 4, 5, 6, 7, 8].map((n) => run(n)));
    expect(results).toHaveLength(8);
    results.forEach((res, idx) => {
      const n = idx + 1;
      const expected = Array.from({ length: n }, (_, i) => ({ value: i + 1 }));
      expect(res).toEqual(expected);
    });
  });

  it('streaming.stream.property.idempotent_false: stateful module yields differing sequences across calls', async () => {
    let calls = 0;
    const counterStateModule = {
      description: 'Stateful counter',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: (): Record<string, unknown> => ({}),
      async *stream(): AsyncGenerator<Record<string, unknown>> {
        calls += 1;
        yield { call: calls };
      },
    };
    const ex = makeExecutor(counterStateModule, 'stateful');
    const first = await collect(ex.stream('stateful', {}));
    const second = await collect(ex.stream('stateful', {}));
    expect(first).toEqual([{ call: 1 }]);
    expect(second).toEqual([{ call: 2 }]);
    expect(first).not.toEqual(second);
  });

  it('streaming.stream.property.pure_false: module mutates external state while streaming', async () => {
    const sideEffects: number[] = [];
    const impureModule = {
      description: 'Impure module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: (): Record<string, unknown> => ({}),
      async *stream(): AsyncGenerator<Record<string, unknown>> {
        for (let i = 0; i < 3; i++) {
          sideEffects.push(i);
          yield { i };
        }
      },
    };
    const ex = makeExecutor(impureModule, 'impure');
    await collect(ex.stream('impure', {}));
    expect(sideEffects).toEqual([0, 1, 2]);
  });
});

// --------------------------------------------------------------------------- //
// SIDE_EFFECT  (ordering, accumulation, fallback, post-validation phase)
// --------------------------------------------------------------------------- //

describe('streaming — side effects', () => {
  it('streaming.stream.side_effect.chunk_ordering_preserved: chunks yielded in source order', async () => {
    const ex = makeExecutor(makeStreamingCounter(), 'counter');
    const chunks = await collect(ex.stream('counter', { count: 5 }));
    const expected = Array.from({ length: 5 }, (_, i) => ({ value: i + 1 }));
    expect(chunks).toEqual(expected);
  });

  it('streaming.stream.side_effect.deep_merge_accumulation: nested chunks deep-merged, right wins for scalars', async () => {
    let captured: Record<string, unknown> = {};
    class CaptureAfter extends Middleware {
      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
      ): null {
        captured = { ...output };
        return null;
      }
    }
    const ex = makeExecutor(makeNestedMergeModule(), 'merge', [new CaptureAfter()]);
    await collect(ex.stream('merge', {}));
    expect(captured).toEqual({
      content: ' world',
      metadata: { tokens: 1, model: 'gpt-4' },
    });
  });

  it('streaming.stream.side_effect.fallback_single_chunk: non-streaming module falls back to single execute() chunk', async () => {
    const ex = makeExecutor(makePlainModule(), 'plain');
    const chunks = await collect(ex.stream('plain', {}));
    expect(chunks).toEqual([{ result: 'done' }]);
  });

  it('streaming.stream.side_effect.after_middleware_post_accumulation: before runs first, after runs last on merged output', async () => {
    const order: string[] = [];
    let captured: Record<string, unknown> = {};
    class OrderMiddleware extends Middleware {
      override before(): null {
        order.push('before');
        return null;
      }
      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
      ): null {
        order.push('after');
        captured = { ...output };
        return null;
      }
    }
    const ex = makeExecutor(makeNestedMergeModule(), 'merge', [new OrderMiddleware()]);
    const chunks = await collect(ex.stream('merge', {}));
    expect(chunks).toHaveLength(2);
    expect(order).toEqual(['before', 'after']);
    expect(captured['metadata']).toEqual({ tokens: 1, model: 'gpt-4' });
  });

  it('streaming.stream.side_effect.deep_merge_depth_capped: deep merge depth-capped, right wins at cap (spec)', async () => {
    function nested(depth: number, leaf: unknown): Record<string, unknown> {
      let node: Record<string, unknown> = { leaf };
      for (let i = 0; i < depth; i++) {
        node = { n: node };
      }
      return node;
    }
    const deepModule = {
      description: 'Deep module',
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({}),
      execute: (): Record<string, unknown> => ({}),
      async *stream(): AsyncGenerator<Record<string, unknown>> {
        yield nested(40, 'a');
        yield nested(40, 'b');
      },
    };
    let captured: Record<string, unknown> = {};
    class CaptureAfter extends Middleware {
      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
      ): null {
        captured = output;
        return null;
      }
    }
    const ex = makeExecutor(deepModule, 'deep', [new CaptureAfter()]);
    // Must complete without stack overflow despite 40 > 32 levels.
    const chunks = await collect(ex.stream('deep', {}));
    expect(chunks).toHaveLength(2);
    let node: Record<string, unknown> = captured;
    for (let i = 0; i < 40; i++) {
      node = node['n'] as Record<string, unknown>;
    }
    // Spec mandates right-value-wins at the depth cap.
    expect(node['leaf']).toBe('b');
  });
});
