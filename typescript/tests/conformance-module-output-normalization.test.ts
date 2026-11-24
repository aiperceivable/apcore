import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { Type } from '@sinclair/typebox';
import { describe, expect, it } from 'vitest';
import { FunctionModule } from '../src/decorator.js';
import { Executor } from '../src/executor.js';
import { Middleware } from '../src/middleware/base.js';
import { Registry } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

interface Case {
  readonly id: string;
  readonly input: { item: { id: string }; updated_at: string };
  readonly expected: {
    output: Record<string, unknown>;
    chunks: Record<string, unknown>[];
  };
}

const fixture = JSON.parse(
  readFileSync(join(findFixturesRoot(), 'module_output_normalization.json'), 'utf-8'),
) as { test_cases: Case[] };

class Item {
  constructor(readonly id: string) {}
}

class Output {
  readonly [key: string]: unknown;

  constructor(readonly item: Item, readonly updated_at: Date) {}
}

const inputSchema = Type.Object({});
const outputSchema = Type.Object({
  item: Type.Object({ id: Type.String() }),
  updated_at: Type.String({ format: 'date-time' }),
});
const moduleId = 'test.typed_output';

class Recorder extends Middleware {
  readonly outputs: Record<string, unknown>[] = [];

  override after(_moduleId: string, _inputs: Record<string, unknown>, output: Record<string, unknown>): null {
    this.outputs.push(output);
    return null;
  }
}

describe('module output normalization', () => {
  for (const { id, input, expected } of fixture.test_cases) {
    it.each(['class', 'wrapped', 'native-stream'])('%s returns plain JSON for ' + id, async (kind) => {
      const payload = new Output(new Item(input.item.id), new Date(input.updated_at));
      const registry = new Registry();
      const options = { description: 'Return a typed object', inputSchema, outputSchema };
      const execute = () => payload;
      const variants = {
        class: { ...options, execute },
        wrapped: new FunctionModule({ ...options, moduleId, execute }),
        'native-stream': {
          ...options,
          execute,
          async *stream() { yield payload; },
        },
      };
      registry.register(moduleId, variants[kind as keyof typeof variants]);
      const executor = new Executor({ registry });
      const recorder = new Recorder();
      executor.use(recorder);

      const result = await executor.call(moduleId, {});
      expect(Object.getPrototypeOf(result)).toBe(Object.prototype);
      expect(Object.getPrototypeOf(result['item'])).toBe(Object.prototype);
      expect(result).toStrictEqual(expected.output);

      const chunks = [];
      for await (const chunk of executor.stream(moduleId, {})) chunks.push(chunk);
      expect(chunks).toStrictEqual(expected.chunks);
      expect(recorder.outputs).toStrictEqual([expected.output, expected.output]);
    });
  }

  it.each([new Date('2026-11-24T12:00:00Z'), new Map(), new Set(), [], 'scalar'])(
    'rejects non-map roots (%s)',
    async (output) => {
      const registry = new Registry();
      registry.registerInternal(moduleId, {
        description: 'Return a non-map value',
        inputSchema,
        outputSchema: Type.Object({}),
        execute: () => output,
        async *stream() { yield output; },
      });
      const executor = new Executor({ registry });

      await expect(executor.call(moduleId, {})).rejects.toMatchObject({ code: 'MODULE_EXECUTE_ERROR' });
      await expect(executor.stream(moduleId, {}).next()).rejects.toMatchObject({
        code: 'GENERAL_INVALID_INPUT',
        details: { code: 'STREAM_CHUNK_NOT_OBJECT', chunk_index: 0 },
      });
    },
  );

  it('copies an own __proto__ property as data', async () => {
    class WithProtoProperty {
      readonly [key: string]: unknown;
      readonly ['__proto__'] = { marker: true };
    }

    const registry = new Registry();
    registry.register(moduleId, {
      description: 'Return an own __proto__ property',
      inputSchema,
      outputSchema: Type.Object({}),
      execute: () => new WithProtoProperty(),
    });
    const executor = new Executor({ registry });
    const result = await executor.call(moduleId, {});

    expect(Object.getPrototypeOf(result)).toBe(Object.prototype);
    expect(Object.hasOwn(result, '__proto__')).toBe(true);
    expect(result['__proto__']).toEqual({ marker: true });
  });
});
