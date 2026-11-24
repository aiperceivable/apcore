/**
 * Spec-traced contract tests for the Middleware System (TypeScript SDK).
 *
 * Mirrors the canonical Python suite
 * `apcore-python/tests/test_middleware_system_spec.py`. Each `it(...)` name
 * begins with a VERBATIM clause id of the form
 * `middleware_system.<method>.<kind>.<detail>` so a cross-language diff lines
 * up row by row. Tests only -- production source is never modified.
 *
 * Source spec: apcore/docs/features/middleware-system.md
 *
 * TS API facts confirmed against the source under test
 * (`src/middleware/manager.ts`, `src/middleware/base.ts`,
 * `src/middleware/adapters.ts`):
 *   - Methods are camelCase: `before` / `after` / `onError`.
 *   - The manager exposes only async methods: `executeBefore`, `executeAfter`,
 *     `executeOnError` (there are NO separate `*_async` variants; every pass is
 *     already a Promise). `executeOnError(moduleId, inputs, error, context,
 *     executedMiddlewares)` takes the executed list explicitly.
 *   - A failing `before()` is wrapped in `MiddlewareChainError` (code
 *     `MIDDLEWARE_CHAIN_ERROR`) carrying `.original` and `.executedMiddlewares`.
 *   - `after()` is fail-fast (reverse order). `onError()` runs reverse over the
 *     executed list; first recovery object wins; handler exceptions are logged
 *     and iteration continues.
 */

import { describe, it, expect } from 'vitest';
import { Context } from '../src/context.js';
import { ModuleError } from '../src/errors.js';
import {
  Middleware,
  MiddlewareManager,
  MiddlewareChainError,
  BeforeMiddleware,
  AfterMiddleware,
} from '../src/middleware/index.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function ctx(): Context {
  return Context.create();
}

function manager(...middlewares: Middleware[]): MiddlewareManager {
  const mgr = new MiddlewareManager();
  for (const mw of middlewares) {
    // allowDuplicate avoids the duplicate-identity warning when several plain
    // Recording/Middleware instances (same constructor.name) are registered.
    mgr.add(mw, { allowDuplicate: true });
  }
  return mgr;
}

class Recording extends Middleware {
  readonly label: string;
  readonly sink: string[];

  constructor(label: string, sink: string[], priority = 100) {
    super(priority);
    this.label = label;
    this.sink = sink;
  }

  override before(): null {
    this.sink.push(`before:${this.label}`);
    return null;
  }

  override after(): null {
    this.sink.push(`after:${this.label}`);
    return null;
  }

  override onError(): null {
    this.sink.push(`on_error:${this.label}`);
    return null;
  }
}

// ===========================================================================
// Contract: Middleware.before
// ===========================================================================

describe('Middleware.before', () => {
  // -- Inputs --------------------------------------------------------------
  // TS NOTE: JavaScript does not raise on a missing positional argument; the
  // value is simply `undefined`. The base `Middleware.before` ignores its args
  // and returns null. There is no TypeError failure path the way Python's
  // call-arg checking provides. We assert the ACTUAL TS behavior (no throw,
  // returns null) and flag the divergence in the report.

  it('middleware_system.before.input.module_id.required: missing module_id does not throw (TS)', () => {
    const mw = new Middleware();
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.before(undefined, {}, ctx())).not.toThrow();
  });

  it('middleware_system.before.input.inputs.required: missing inputs does not throw (TS)', () => {
    const mw = new Middleware();
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.before('mod.id', undefined, ctx())).not.toThrow();
  });

  it('middleware_system.before.input.context.required: missing context does not throw (TS)', () => {
    const mw = new Middleware();
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.before('mod.id', {})).not.toThrow();
  });

  // -- Returns -------------------------------------------------------------
  it('middleware_system.before.returns.none_passthrough: null leaves inputs unchanged', async () => {
    const mgr = manager(new BeforeMiddleware(() => null));
    const inputs = { a: 1 };
    const [final, executed] = await mgr.executeBefore('mod.id', inputs, ctx());
    expect(final).toEqual({ a: 1 });
    expect(final).toBe(inputs); // identity preserved confirms passthrough
    expect(executed).toHaveLength(1);
  });

  it('middleware_system.before.returns.dict_replaces_inputs: object replaces inputs downstream', async () => {
    const seenDownstream: Record<string, unknown> = {};
    const replace = new BeforeMiddleware(() => ({ replaced: true }));
    const observe = new BeforeMiddleware((_m, i) => {
      Object.assign(seenDownstream, i);
      return null;
    });
    const mgr = manager(replace, observe);
    const [final] = await mgr.executeBefore('mod.id', { orig: 1 }, ctx());
    expect(final).toEqual({ replaced: true });
    expect(seenDownstream).toEqual({ replaced: true });
  });

  // -- Errors --------------------------------------------------------------
  it('middleware_system.before.error.MIDDLEWARE_CHAIN_ERROR: raise wrapped in MiddlewareChainError', async () => {
    const boom = new Error('before exploded');
    const mgr = manager(
      new BeforeMiddleware(() => {
        throw boom;
      }),
    );
    let caught: unknown;
    try {
      await mgr.executeBefore('mod.id', {}, ctx());
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(MiddlewareChainError);
    expect(caught).toBeInstanceOf(ModuleError);
    const err = caught as MiddlewareChainError;
    expect(err.code).toBe('MIDDLEWARE_CHAIN_ERROR');
    expect(err.original).toBe(boom);
  });

  it('middleware_system.before.error.aborts_pipeline_tracks_executed: downstream skipped, executed tracked', async () => {
    const sink: string[] = [];
    const first = new Recording('first', sink);
    const raisingMw = new BeforeMiddleware(() => {
      sink.push('before:raiser');
      throw new Error('stop');
    });
    const downstream = new Recording('downstream', sink);
    const mgr = manager(first, raisingMw, downstream);

    let caught: unknown;
    try {
      await mgr.executeBefore('mod.id', {}, ctx());
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(MiddlewareChainError);
    expect(sink).not.toContain('before:downstream');
    expect(sink).toEqual(['before:first', 'before:raiser']);
    expect((caught as MiddlewareChainError).executedMiddlewares).toEqual([first, raisingMw]);
  });

  // -- Properties ----------------------------------------------------------
  it('middleware_system.before.property.async: async before() awaited and applied', async () => {
    class AsyncBefore extends Middleware {
      override async before(): Promise<Record<string, unknown>> {
        await Promise.resolve();
        return { awaited: true };
      }
    }
    const mgr = manager(new AsyncBefore());
    const passed = mgr.executeBefore('mod.id', { orig: 1 }, ctx());
    expect(passed).toBeInstanceOf(Promise);
    const [final, executed] = await passed;
    expect(final).toEqual({ awaited: true });
    expect(executed).toHaveLength(1);
  });

  it('middleware_system.before.property.thread_safe: N concurrent passes, no cross-talk', async () => {
    const n = 10;
    const mgr = manager(new BeforeMiddleware((_m, i) => ({ echo: i.idx })));

    const one = async (idx: number): Promise<Record<string, unknown>> => {
      await Promise.resolve();
      const [final] = await mgr.executeBefore('mod.id', { idx }, ctx());
      return final;
    };

    const churn = async (): Promise<void> => {
      for (let k = 0; k < n; k++) {
        await Promise.resolve();
        mgr.add(new BeforeMiddleware(() => null), { allowDuplicate: true });
      }
    };

    const [results] = await Promise.all([
      Promise.all(Array.from({ length: n }, (_v, i) => one(i))),
      churn(),
    ]);
    expect(results.map((r) => r.echo)).toEqual(Array.from({ length: n }, (_v, i) => i));
  });

  it('middleware_system.before.property.pure: before() may mutate context.data (pure=false)', async () => {
    const c = ctx();
    expect('ext.spec.before_ran' in c.data).toBe(false);
    const mgr = manager(
      new BeforeMiddleware((_m, _i, context) => {
        context.data['ext.spec.before_ran'] = true;
        return null;
      }),
    );
    await mgr.executeBefore('mod.id', {}, c);
    expect(c.data['ext.spec.before_ran']).toBe(true);
  });

  // -- Side effects --------------------------------------------------------
  it('middleware_system.before.side_effect.1.registration_order: runs in registration order', async () => {
    const sink: string[] = [];
    const mgr = manager(
      new Recording('1', sink),
      new Recording('2', sink),
      new Recording('3', sink),
    );
    await mgr.executeBefore('mod.id', {}, ctx());
    expect(sink).toEqual(['before:1', 'before:2', 'before:3']);
  });
});

// ===========================================================================
// Contract: Middleware.after
// ===========================================================================

describe('Middleware.after', () => {
  // -- Inputs --------------------------------------------------------------
  it('middleware_system.after.input.module_id.required: missing module_id does not throw (TS)', () => {
    const mw = new Middleware();
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.after(undefined, {}, {}, ctx())).not.toThrow();
  });

  it('middleware_system.after.input.inputs.required: missing inputs does not throw (TS)', () => {
    const mw = new Middleware();
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.after('mod.id', undefined, {}, ctx())).not.toThrow();
  });

  it('middleware_system.after.input.output.required: missing output does not throw (TS)', () => {
    const mw = new Middleware();
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.after('mod.id', {}, undefined, ctx())).not.toThrow();
  });

  it('middleware_system.after.input.context.required: missing context does not throw (TS)', () => {
    const mw = new Middleware();
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.after('mod.id', {}, {})).not.toThrow();
  });

  // -- Returns -------------------------------------------------------------
  it('middleware_system.after.returns.none_passthrough: null leaves output unchanged', async () => {
    const mgr = manager(new AfterMiddleware(() => null));
    const out = { v: 1 };
    const final = await mgr.executeAfter('mod.id', {}, out, ctx());
    expect(final).toEqual({ v: 1 });
    expect(final).toBe(out);
  });

  it('middleware_system.after.returns.dict_replaces_output: object replaces output', async () => {
    const mgr = manager(new AfterMiddleware((_m, _i, o) => ({ wrapped: o })));
    const final = await mgr.executeAfter('mod.id', {}, { v: 1 }, ctx());
    expect(final).toEqual({ wrapped: { v: 1 } });
  });

  // -- Errors --------------------------------------------------------------
  it('middleware_system.after.error.fail_fast_propagates: raise propagates, remaining hooks skipped', async () => {
    const sink: string[] = [];
    const raiser = new AfterMiddleware(() => {
      throw new Error('after exploded');
    });
    // after() runs in REVERSE order: last-registered runs first. Register
    // raiser last so it runs first and the recorder never runs.
    const recorder = new Recording('never', sink);
    const mgr = manager(recorder, raiser);
    await expect(mgr.executeAfter('mod.id', {}, { v: 1 }, ctx())).rejects.toThrow('after exploded');
    expect(sink).not.toContain('after:never');
  });

  // -- Properties ----------------------------------------------------------
  it('middleware_system.after.property.async: async after() awaited and applied', async () => {
    class AsyncAfter extends Middleware {
      override async after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
      ): Promise<Record<string, unknown>> {
        await Promise.resolve();
        return { awaited: output };
      }
    }
    const mgr = manager(new AsyncAfter());
    const passed = mgr.executeAfter('mod.id', {}, { v: 1 }, ctx());
    expect(passed).toBeInstanceOf(Promise);
    const final = await passed;
    expect(final).toEqual({ awaited: { v: 1 } });
  });

  it('middleware_system.after.property.thread_safe: N concurrent passes, no cross-talk', async () => {
    const n = 10;
    const mgr = manager(new AfterMiddleware((_m, _i, o) => ({ echo: o.idx })));

    const one = async (idx: number): Promise<Record<string, unknown>> => {
      await Promise.resolve();
      return mgr.executeAfter('mod.id', {}, { idx }, ctx());
    };

    const results = await Promise.all(Array.from({ length: n }, (_v, i) => one(i)));
    expect(results.map((r) => r.echo)).toEqual(Array.from({ length: n }, (_v, i) => i));
  });

  // -- Side effects --------------------------------------------------------
  it('middleware_system.after.side_effect.1.reverse_order: runs in reverse registration order', async () => {
    const sink: string[] = [];
    const mgr = manager(
      new Recording('1', sink),
      new Recording('2', sink),
      new Recording('3', sink),
    );
    await mgr.executeAfter('mod.id', {}, { v: 1 }, ctx());
    expect(sink).toEqual(['after:3', 'after:2', 'after:1']);
  });
});

// ===========================================================================
// Contract: Middleware.onError
// ===========================================================================

describe('Middleware.onError', () => {
  // -- Inputs --------------------------------------------------------------
  it('middleware_system.on_error.input.module_id.required: missing module_id does not throw (TS)', () => {
    const mw = new Middleware();
    const err = new ModuleError('X', 'x');
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.onError(undefined, {}, err, ctx())).not.toThrow();
  });

  it('middleware_system.on_error.input.inputs.required: missing inputs does not throw (TS)', () => {
    const mw = new Middleware();
    const err = new ModuleError('X', 'x');
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.onError('mod.id', undefined, err, ctx())).not.toThrow();
  });

  it('middleware_system.on_error.input.error.required: missing error does not throw (TS)', () => {
    const mw = new Middleware();
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.onError('mod.id', {}, undefined, ctx())).not.toThrow();
  });

  it('middleware_system.on_error.input.context.required: missing context does not throw (TS)', () => {
    const mw = new Middleware();
    const err = new ModuleError('X', 'x');
    // @ts-expect-error intentionally omitting required positional args
    expect(() => mw.onError('mod.id', {}, err)).not.toThrow();
  });

  // -- Returns -------------------------------------------------------------
  it('middleware_system.on_error.returns.first_recovery_wins: first non-null recovery short-circuits', async () => {
    const sink: string[] = [];

    class Recover extends Middleware {
      constructor(
        readonly label: string,
        readonly value: Record<string, unknown> | null,
      ) {
        super();
      }
      override onError(): Record<string, unknown> | null {
        sink.push(this.label);
        return this.value;
      }
    }

    // Registration order: A, B, C. Reverse run order: C, B, A.
    const a = new Recover('A', { by: 'A' });
    const b = new Recover('B', { by: 'B' });
    const c = new Recover('C', null);
    const mgr = manager(a, b, c);
    const executed = [a, b, c];
    const result = await mgr.executeOnError('mod.id', {}, new Error('x'), ctx(), executed);
    expect(result).toEqual({ by: 'B' });
    expect(sink).toEqual(['C', 'B']);
    expect(sink).not.toContain('A');
  });

  it('middleware_system.on_error.returns.none_passthrough: all-null returns null', async () => {
    const mgr = manager(new Middleware(), new Middleware());
    const executed = mgr.snapshot();
    const result = await mgr.executeOnError('mod.id', {}, new Error('x'), ctx(), executed);
    expect(result).toBeNull();
  });

  // -- Errors --------------------------------------------------------------
  it('middleware_system.on_error.error.handler_must_not_raise: handler exception swallowed, iteration continues', async () => {
    const sink: string[] = [];

    class Boom extends Middleware {
      override onError(): Record<string, unknown> {
        sink.push('boom');
        throw new Error('handler blew up');
      }
    }
    class Recover extends Middleware {
      override onError(): Record<string, unknown> {
        sink.push('recover');
        return { recovered: true };
      }
    }
    const recover = new Recover();
    const boom = new Boom();
    // Reverse run order: boom first (raises, swallowed), then recover.
    const mgr = manager(recover, boom);
    const executed = [recover, boom];
    const result = await mgr.executeOnError('mod.id', {}, new Error('x'), ctx(), executed);
    expect(result).toEqual({ recovered: true });
    expect(sink).toEqual(['boom', 'recover']);
  });

  // -- Properties ----------------------------------------------------------
  it('middleware_system.on_error.property.async: async onError() awaited and applied', async () => {
    class AsyncRecover extends Middleware {
      override async onError(): Promise<Record<string, unknown>> {
        await Promise.resolve();
        return { recovered_async: true };
      }
    }
    const mw = new AsyncRecover();
    const mgr = manager(mw);
    const passed = mgr.executeOnError('mod.id', {}, new Error('x'), ctx(), [mw]);
    expect(passed).toBeInstanceOf(Promise);
    const result = await passed;
    expect(result).toEqual({ recovered_async: true });
  });

  it('middleware_system.on_error.property.thread_safe: N concurrent recovery passes, no cross-talk', async () => {
    const n = 8;

    class EchoRecover extends Middleware {
      override async onError(
        _moduleId: string,
        inputs: Record<string, unknown>,
      ): Promise<Record<string, unknown>> {
        await Promise.resolve();
        return { echo: inputs.idx };
      }
    }
    const mw = new EchoRecover();
    const mgr = manager(mw);

    const one = (idx: number): Promise<Record<string, unknown> | unknown> =>
      mgr.executeOnError('mod.id', { idx }, new Error(String(idx)), ctx(), [mw]);

    const results = await Promise.all(Array.from({ length: n }, (_v, i) => one(i)));
    expect(results.map((r) => (r as Record<string, unknown>).echo)).toEqual(
      Array.from({ length: n }, (_v, i) => i),
    );
  });

  // -- Side effects --------------------------------------------------------
  it('middleware_system.on_error.side_effect.1.reverse_over_executed: reverse over executed only', async () => {
    const sink: string[] = [];
    const mw1 = new Recording('1', sink);
    const mw2 = new Recording('2', sink);
    const mw3 = new Recording('3', sink);
    const mgr = manager(mw1, mw2, mw3);
    // Simulate failure after mw1, mw2 ran before() (mw3 did not).
    const executed = [mw1, mw2];
    await mgr.executeOnError('mod.id', {}, new Error('x'), ctx(), executed);
    expect(sink).toEqual(['on_error:2', 'on_error:1']);
    expect(sink).not.toContain('on_error:3');
  });
});

// ===========================================================================
// Contract: Middleware.detect_async
// ---------------------------------------------------------------------------
// The spec declares `## Contract: Middleware.detect_async` (a pure, idempotent,
// thread-safe predicate). Like apcore-python, this SDK exposes NO
// `Middleware.detect_async` method/function symbol. Async detection is handled
// by the standalone `isAsyncHandler` helper as a fast path, but the authoritative
// awaitability decision is inlined in MiddlewareManager via runtime `await`. To
// keep the cross-language row aligned with the Python canonical (which skips
// these as a missing-symbol contract gap), each clause is recorded as a skip.
// ===========================================================================

const DETECT_ASYNC_MISSING =
  'missing symbol Middleware.detect_async (contract gap) -- apcore-typescript has ' +
  'no standalone Middleware.detect_async method; the isAsyncHandler helper is a ' +
  'fast path and awaitability is resolved inline in MiddlewareManager.';

describe('Middleware.detect_async (contract gap)', () => {
  it.skip(`middleware_system.detect_async.input.handler.required: ${DETECT_ASYNC_MISSING}`, () => {
    /* skipped contract gap */
  });

  it.skip(`middleware_system.detect_async.returns.bool: ${DETECT_ASYNC_MISSING}`, () => {
    /* skipped contract gap */
  });

  it.skip(`middleware_system.detect_async.property.pure: ${DETECT_ASYNC_MISSING}`, () => {
    /* skipped contract gap */
  });

  it.skip(`middleware_system.detect_async.property.idempotent: ${DETECT_ASYNC_MISSING}`, () => {
    /* skipped contract gap */
  });
});
