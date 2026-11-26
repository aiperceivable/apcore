/**
 * Middleware handling for successful outputs and invalid empty results.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Registry } from '../../src/registry/registry.js';
import { Executor } from '../../src/executor.js';
import { Middleware } from '../../src/middleware/base.js';
import type { Context } from '../../src/context.js';

class Recorder extends Middleware {
  readonly beforeCalls: string[] = [];
  readonly afterCalls: Array<Record<string, unknown>> = [];
  readonly errors: Error[] = [];

  before(moduleId: string, inputs: Record<string, unknown>, _context: Context) {
    this.beforeCalls.push(moduleId);
    return inputs;
  }

  after(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    output: Record<string, unknown>,
    _context: Context,
  ) {
    this.afterCalls.push(output);
    return output;
  }

  override onError(_moduleId: string, _inputs: Record<string, unknown>, error: Error): null {
    this.errors.push(error);
    return null;
  }
}

class Replacer extends Middleware {
  after(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    _context: Context,
  ) {
    return { replaced: true };
  }
}

/** A module whose `execute` returns nothing at all. */
const VOID_MODULE = {
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({}),
  description: 'returns nothing',
  execute: () => {
    /* void */
  },
};

const VALUE_MODULE = {
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({ ok: Type.Boolean() }),
  description: 'returns a value',
  execute: () => ({ ok: true }),
};

function build() {
  const registry = new Registry();
  const executor = new Executor({ registry });
  registry.registerInternal('executor.demo.void', VOID_MODULE);
  registry.registerInternal('executor.demo.value', VALUE_MODULE);
  return { registry, executor };
}

describe('middleware receives only valid execution outcomes', () => {
  it('a missing output runs the error chain and skips successful after hooks', async () => {
    const { executor } = build();
    const mw = new Recorder();
    executor.use(mw);

    await expect(executor.call('executor.demo.void', {}))
      .rejects.toMatchObject({ code: 'MODULE_EXECUTE_ERROR' });

    expect(mw.beforeCalls).toEqual(['executor.demo.void']);
    expect(mw.afterCalls).toHaveLength(0);
    expect(mw.errors).toHaveLength(1);
    expect(mw.errors[0]).toMatchObject({ code: 'MODULE_EXECUTE_ERROR' });
  });

  it('an after() that returns a mapping can replace a valid result', async () => {
    const { executor } = build();
    executor.use(new Replacer());

    const result = await executor.call('executor.demo.value', {});
    expect(result).toEqual({ replaced: true });
  });

  it('a module that does return a value is unaffected', async () => {
    const { executor } = build();
    const mw = new Recorder();
    executor.use(mw);

    const result = await executor.call('executor.demo.value', {});
    expect(result).toEqual({ ok: true });
    expect(mw.afterCalls).toEqual([{ ok: true }]);
  });

  it('validate() still skips the after chain — the execute step never ran', async () => {
    const { executor } = build();
    const mw = new Recorder();
    executor.use(mw);

    await executor.validate('executor.demo.void', {});
    expect(mw.afterCalls).toHaveLength(0);
  });

  // After middleware runs once after the executor has drained and merged the stream.
  it('streaming runs the after chain once, in Phase 3, over the accumulated output', async () => {
    const { registry, executor } = build();
    const mw = new Recorder();
    executor.use(mw);
    registry.registerInternal('executor.demo.stream', {
      inputSchema: Type.Object({}),
      outputSchema: Type.Object({ n: Type.Number() }),
      description: 'streams',
      annotations: { streaming: true },
      stream: async function* () {
        yield { n: 1 };
      },
      execute: () => ({ n: 1 }),
    });

    for await (const _chunk of executor.stream('executor.demo.stream', {})) {
      /* drain */
    }
    expect(mw.afterCalls).toEqual([{ n: 1 }]);
  });
});
