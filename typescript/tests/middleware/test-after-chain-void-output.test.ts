/**
 * MW-001 — the after-middleware chain was skipped on a void output.
 *
 * `BuiltinMiddlewareAfter` guarded with `if (ctx.output == null) return`, so a
 * module that returned nothing skipped the ENTIRE after chain. The guard was
 * meant to cover "the execute step did not run" (streaming Phase 1, dry run),
 * but it tested the output's truthiness instead, and a void module produces
 * exactly the same `null` as a step that never ran.
 *
 * middleware-system.md makes `after()` the onion's closing half of `before()`:
 * a middleware that acquired state in `before()` never got its matching
 * `after()`, so metrics went unrecorded, tracing spans unpopped and audit
 * lines unwritten. apcore-python runs the chain with `{}`
 * (builtin_steps.py:~1191) and apcore-rust runs it with the module's `null`.
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

describe('the after chain runs for a module that returns nothing (MW-001)', () => {
  it('before() and after() are both called, once each', async () => {
    const { executor } = build();
    const mw = new Recorder();
    executor.use(mw);

    await executor.call('executor.demo.void', {});

    expect(mw.beforeCalls).toEqual(['executor.demo.void']);
    expect(mw.afterCalls).toHaveLength(1);
  });

  it('a void output reaches after() as an empty object, never null', async () => {
    const { executor } = build();
    const mw = new Recorder();
    executor.use(mw);

    await executor.call('executor.demo.void', {});

    expect(mw.afterCalls[0]).toEqual({});
  });

  it('an after() that returns a mapping can replace a void result', async () => {
    const { executor } = build();
    executor.use(new Replacer());

    const result = await executor.call('executor.demo.void', {});
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

  // Streaming does NOT skip the after chain: `BuiltinExecute` returns
  // `skip_to: 'return_result'` in Phase 1, and `Executor.stream` re-runs
  // `output_validation` / `middleware_after` / `return_result` in Phase 3 over
  // the ACCUMULATED result. Pinned here so the MW-001 guard change cannot
  // silently take that call away.
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
