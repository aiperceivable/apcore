/**
 * Spec-traced contract tests for the Extension System (TypeScript SDK).
 *
 * MIRRORS the canonical Python suite
 * apcore-python/tests/test_extension_system_spec.py. Each `it()` name begins
 * with the verbatim clause id of the form
 * 'extension_system.<method>.<kind>.<detail>' so a cross-language diff can match
 * rows by exact clause id.
 *
 * Semantics derive from apcore/docs/features/extension-system.md.
 * These are TESTS ONLY; production source is never modified.
 */

import { describe, it, expect, vi } from 'vitest';
import { ExtensionManager } from '../src/extensions.js';
import { Middleware } from '../src/middleware/index.js';
import { ACL } from '../src/acl.js';
import { InvalidInputError } from '../src/errors.js';
import type { ModuleError } from '../src/errors.js';
import { TracingMiddleware, InMemoryExporter } from '../src/observability/tracing.js';
import type { Registry } from '../src/registry/registry.js';
import type { Executor } from '../src/executor.js';
import type { Discoverer, ModuleValidator } from '../src/registry/registry.js';
import type { SpanExporter, Span } from '../src/observability/tracing.js';

// ---------------------------------------------------------------------------
// Helpers: concrete implementations satisfying the duck-typed interfaces
// ---------------------------------------------------------------------------

class StubDiscoverer implements Discoverer {
  discover(_roots: string[]) {
    return [];
  }
}

class StubValidator implements ModuleValidator {
  validate(_module: unknown) {
    return [];
  }
}

class StubMiddleware extends Middleware {}

class StubExporter implements SpanExporter {
  exported: Span[] = [];
  export(span: Span): void {
    this.exported.push(span);
  }
}

class StubApprovalHandler {
  checkApproval(..._args: unknown[]): unknown {
    return null;
  }
  requestApproval(..._args: unknown[]): unknown {
    return null;
  }
}

/** Build registry/executor mocks with an empty middleware chain. */
function freshTargets(): { registry: Registry; executor: Executor & { use: ReturnType<typeof vi.fn>; setAcl: ReturnType<typeof vi.fn>; setApprovalHandler: ReturnType<typeof vi.fn>; middlewares: unknown[] } } {
  const registry = {
    setDiscoverer: vi.fn(),
    setValidator: vi.fn(),
  } as unknown as Registry;
  const executor = {
    use: vi.fn(),
    setAcl: vi.fn(),
    setApprovalHandler: vi.fn(),
    middlewares: [] as unknown[],
  } as unknown as Executor & { use: ReturnType<typeof vi.fn>; setAcl: ReturnType<typeof vi.fn>; setApprovalHandler: ReturnType<typeof vi.fn>; middlewares: unknown[] };
  return { registry, executor };
}

// ===========================================================================
// Contract: ExtensionManager.register
// ===========================================================================

describe('ExtensionManager.register', () => {
  it('extension_system.register.input.point_name.unknown: unknown point_name raises', () => {
    const mgr = new ExtensionManager();
    let caught: unknown;
    try {
      mgr.register('no_such_point', new StubMiddleware());
    } catch (e) {
      caught = e;
    }
    // D-108: `Error` is satisfied by every throw in the language. Assert the
    // declared code, which is the thing a caller can branch on.
    expect(caught).toBeInstanceOf(InvalidInputError);
    expect((caught as ModuleError).code).toBe('GENERAL_INVALID_INPUT');
    expect((caught as Error).message).toContain('Unknown extension point');
  });

  it('extension_system.register.input.extension.wrong_type: wrong type raises TypeError', () => {
    const mgr = new ExtensionManager();
    let caught: unknown;
    try {
      mgr.register('middleware', 'not-a-middleware');
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(TypeError);
    // TS message identifies the type mismatch via "must satisfy the <T> interface".
    expect((caught as Error).message).toContain('must satisfy');
  });

  it('extension_system.register.error.ExtensionPointNotFoundError: unknown name -> Error naming the offender', () => {
    const mgr = new ExtensionManager();
    let caught: unknown;
    try {
      mgr.register('totally_unknown', new StubDiscoverer());
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(Error);
    expect((caught as Error).message).toContain('totally_unknown');
  });

  it('extension_system.register.error.ExtensionTypeError: Middleware where Discoverer required -> TypeError', () => {
    const mgr = new ExtensionManager();
    let caught: unknown;
    try {
      // A Middleware lacks `discover`, so it fails the discoverer type guard.
      mgr.register('discoverer', new StubMiddleware());
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(TypeError);
    expect((caught as Error).message).toContain('discoverer');
  });

  it('extension_system.register.property.async.false: register returns undefined synchronously', () => {
    const mgr = new ExtensionManager();
    const result = mgr.register('middleware', new StubMiddleware());
    expect(result).toBeUndefined();
    // Not a thenable.
    expect((result as unknown as { then?: unknown })?.then).toBeUndefined();
  });

  it('extension_system.register.property.idempotent.single_replaces: single-cardinality replaces', () => {
    const mgr = new ExtensionManager();
    const first = new StubDiscoverer();
    const second = new StubDiscoverer();
    mgr.register('discoverer', first);
    mgr.register('discoverer', second);
    expect(mgr.get('discoverer')).toBe(second);
    expect(mgr.get('discoverer')).not.toBe(first);
  });

  it('extension_system.register.property.idempotent.multi_accumulates: multi-cardinality accumulates in order', () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    const mw2 = new StubMiddleware();
    mgr.register('middleware', mw1);
    mgr.register('middleware', mw2);
    expect(mgr.getAll('middleware')).toEqual([mw1, mw2]);
  });
});

// ===========================================================================
// Contract: ExtensionManager.get
// ===========================================================================

describe('ExtensionManager.get', () => {
  it('extension_system.get.property.async.false: get returns a plain value, not a thenable', () => {
    const mgr = new ExtensionManager();
    const result = mgr.get('discoverer');
    expect((result as unknown as { then?: unknown })?.then).toBeUndefined();
  });

  it('extension_system.get.error.no_error_returns_none: empty single-cardinality point returns null', () => {
    const mgr = new ExtensionManager();
    expect(mgr.get('acl')).toBeNull();
  });

  it('extension_system.get.property.pure.true: repeated get does not mutate manager state', () => {
    const mgr = new ExtensionManager();
    const disc = new StubDiscoverer();
    mgr.register('discoverer', disc);

    const beforePoints = new Set(mgr.listPoints().map((p) => p.name));
    const first = mgr.get('discoverer');
    const second = mgr.get('discoverer');
    const afterPoints = new Set(mgr.listPoints().map((p) => p.name));

    expect(first).toBe(disc);
    expect(second).toBe(disc);
    expect(afterPoints).toEqual(beforePoints);
    // State for other points is untouched by the query.
    expect(mgr.getAll('middleware')).toEqual([]);
  });

  it('extension_system.get.property.thread_safe.concurrent_reads: >=8 concurrent reads observe same value', async () => {
    const mgr = new ExtensionManager();
    const disc = new StubDiscoverer();
    mgr.register('discoverer', disc);

    const reader = async () => mgr.get('discoverer');
    const results = await Promise.all(Array.from({ length: 16 }, () => reader()));
    expect(results).toHaveLength(16);
    expect(results.every((r) => r === disc)).toBe(true);
  });
});

// ===========================================================================
// Contract: ExtensionManager.getAll
// ===========================================================================

describe('ExtensionManager.getAll', () => {
  it('extension_system.get_all.property.async.false: getAll returns an array synchronously', () => {
    const mgr = new ExtensionManager();
    const result = mgr.getAll('middleware');
    expect(Array.isArray(result)).toBe(true);
    expect((result as unknown as { then?: unknown })?.then).toBeUndefined();
  });

  it('extension_system.get_all.error.no_error_returns_empty: empty multi-cardinality point returns []', () => {
    const mgr = new ExtensionManager();
    expect(mgr.getAll('middleware')).toEqual([]);
  });

  it('extension_system.get_all.returns.registration_order: all extensions in registration order', () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    const mw2 = new StubMiddleware();
    const mw3 = new StubMiddleware();
    mgr.register('middleware', mw1);
    mgr.register('middleware', mw2);
    mgr.register('middleware', mw3);
    expect(mgr.getAll('middleware')).toEqual([mw1, mw2, mw3]);
  });

  it('extension_system.get_all.property.pure.true: returned array is a copy; mutating it does not affect store', () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    mgr.register('middleware', mw1);

    const first = mgr.getAll('middleware');
    first.push(new StubMiddleware()); // mutate the returned array

    const second = mgr.getAll('middleware');
    expect(second).toEqual([mw1]); // internal store unaffected
  });

  it('extension_system.get_all.property.thread_safe.concurrent_reads: >=8 concurrent reads share a snapshot', async () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    const mw2 = new StubMiddleware();
    mgr.register('middleware', mw1);
    mgr.register('middleware', mw2);

    const reader = async () => mgr.getAll('middleware');
    const results = await Promise.all(Array.from({ length: 12 }, () => reader()));
    expect(results).toHaveLength(12);
    expect(results.every((r) => r.length === 2 && r[0] === mw1 && r[1] === mw2)).toBe(true);
  });
});

// ===========================================================================
// Contract: ExtensionManager.unregister
// ===========================================================================

describe('ExtensionManager.unregister', () => {
  it('extension_system.unregister.property.async.false: unregister resolves synchronously', () => {
    const mgr = new ExtensionManager();
    const mw = new StubMiddleware();
    mgr.register('middleware', mw);
    const result = mgr.unregister('middleware', mw);
    expect((result as unknown as { then?: unknown })?.then).toBeUndefined();
    expect(typeof result).toBe('boolean');
  });

  it('extension_system.unregister.removes.identity: removes the exact object', () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    const mw2 = new StubMiddleware();
    mgr.register('middleware', mw1);
    mgr.register('middleware', mw2);
    mgr.unregister('middleware', mw1);
    // `toEqual` is a DEEP comparison, so on two structurally identical stubs it
    // cannot tell identity from equality — which is the whole question here.
    // Assert the surviving REFERENCE.
    expect(mgr.getAll('middleware')).toHaveLength(1);
    expect(mgr.getAll('middleware')[0]).toBe(mw2);
  });

  it('D-128: removal is by identity, not structural equality', () => {
    // apcore-python used `list.remove`, which compares with `__eq__`, so
    // `unregister(second)` deleted `first`: a host removing the second of two
    // identically-configured middlewares kept the one it wanted gone and lost
    // the one it wanted kept, silently. This SDK uses `indexOf`, which is
    // `===` — the behaviour the contract's Inputs row ("identity comparison")
    // has always required and which apcore-python was corrected to match.
    //
    // The two registrations MUST be structurally equal and MUST NOT be the
    // same reference, or the test cannot distinguish the two semantics.
    const mgr = new ExtensionManager();
    const first = new StubMiddleware();
    const second = new StubMiddleware();
    expect(first).toEqual(second);
    expect(first).not.toBe(second);

    mgr.register('middleware', first);
    mgr.register('middleware', second);

    expect(mgr.unregister('middleware', second)).toBe(true);
    const survivors = mgr.getAll('middleware');
    expect(survivors).toHaveLength(1);
    expect(survivors[0]).toBe(first);
  });

  it('D-128 control: an equal but unregistered object is a no-op', () => {
    // Without this, "removal is by identity" would also be satisfied by an
    // implementation that removed the LAST structurally equal entry — still
    // wrong, and still passing the test above whenever only one is registered.
    const mgr = new ExtensionManager();
    const registered = new StubMiddleware();
    const neverRegistered = new StubMiddleware();
    expect(registered).toEqual(neverRegistered);

    mgr.register('middleware', registered);

    expect(mgr.unregister('middleware', neverRegistered)).toBe(false);
    expect(mgr.getAll('middleware')[0]).toBe(registered);
  });

  it('extension_system.unregister.error.missing_is_silent_no_op: missing extension -> false, state intact', () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    const never = new StubMiddleware();
    mgr.register('middleware', mw1);

    const result = mgr.unregister('middleware', never); // silent no-op
    expect(result).toBe(false);
    expect(mgr.getAll('middleware')).toEqual([mw1]);
  });

  it('extension_system.unregister.property.pure.false: unregister mutates store (observable via getAll)', () => {
    const mgr = new ExtensionManager();
    const mw = new StubMiddleware();
    mgr.register('middleware', mw);
    expect(mgr.getAll('middleware')).toEqual([mw]);
    mgr.unregister('middleware', mw);
    expect(mgr.getAll('middleware')).toEqual([]);
  });
});

// ===========================================================================
// Contract: ExtensionManager.apply
// ===========================================================================

describe('ExtensionManager.apply', () => {
  it('extension_system.apply.property.async.false: apply returns undefined synchronously', () => {
    const mgr = new ExtensionManager();
    const { registry, executor } = freshTargets();
    const result = mgr.apply(registry, executor);
    expect(result).toBeUndefined();
  });

  it('extension_system.apply.side_effect.1.set_discoverer: registry.setDiscoverer called with discoverer', () => {
    const mgr = new ExtensionManager();
    const disc = new StubDiscoverer();
    mgr.register('discoverer', disc);
    const { registry, executor } = freshTargets();
    mgr.apply(registry, executor);
    expect(registry.setDiscoverer).toHaveBeenCalledTimes(1);
    expect(registry.setDiscoverer).toHaveBeenCalledWith(disc);
  });

  it('extension_system.apply.side_effect.2.set_validator: registry.setValidator called with validator', () => {
    const mgr = new ExtensionManager();
    const val = new StubValidator();
    mgr.register('module_validator', val);
    const { registry, executor } = freshTargets();
    mgr.apply(registry, executor);
    expect(registry.setValidator).toHaveBeenCalledTimes(1);
    expect(registry.setValidator).toHaveBeenCalledWith(val);
  });

  it('extension_system.apply.side_effect.3.set_acl: executor.setAcl called with acl', () => {
    const mgr = new ExtensionManager();
    const acl = new ACL([]);
    mgr.register('acl', acl);
    const { registry, executor } = freshTargets();
    mgr.apply(registry, executor);
    expect(executor.setAcl).toHaveBeenCalledTimes(1);
    expect(executor.setAcl).toHaveBeenCalledWith(acl);
  });

  it('extension_system.apply.side_effect.4.set_approval_handler: executor.setApprovalHandler called with handler', () => {
    const mgr = new ExtensionManager();
    const handler = new StubApprovalHandler();
    mgr.register('approval_handler', handler);
    const { registry, executor } = freshTargets();
    mgr.apply(registry, executor);
    expect(executor.setApprovalHandler).toHaveBeenCalledTimes(1);
    expect(executor.setApprovalHandler).toHaveBeenCalledWith(handler);
  });

  it('extension_system.apply.side_effect.5.use_middleware_in_order: executor.use called per middleware in order', () => {
    const mgr = new ExtensionManager();
    const mw1 = new StubMiddleware();
    const mw2 = new StubMiddleware();
    mgr.register('middleware', mw1);
    mgr.register('middleware', mw2);
    const { registry, executor } = freshTargets();
    mgr.apply(registry, executor);

    const used = executor.use.mock.calls.map((c) => c[0]);
    expect(used).toEqual([mw1, mw2]);
  });

  it('extension_system.apply.side_effect.6.single_span_exporter_direct: single exporter set directly on TracingMiddleware', () => {
    const mgr = new ExtensionManager();
    const exporter = new StubExporter();
    mgr.register('span_exporter', exporter);

    const tracingMw = new TracingMiddleware(new InMemoryExporter());
    const { registry, executor } = freshTargets();
    executor.middlewares = [tracingMw];
    mgr.apply(registry, executor);

    expect((tracingMw as unknown as Record<string, unknown>)['_exporter']).toBe(exporter);
  });

  it('extension_system.apply.side_effect.6.multiple_span_exporters_composite: composite wraps all, error-isolated fan-out', () => {
    const mgr = new ExtensionManager();

    class FailingExporter implements SpanExporter {
      export(_span: Span): void {
        throw new Error('boom');
      }
    }

    const failing = new FailingExporter();
    const good = new StubExporter();
    mgr.register('span_exporter', failing);
    mgr.register('span_exporter', good);

    const tracingMw = new TracingMiddleware(new InMemoryExporter());
    const { registry, executor } = freshTargets();
    executor.middlewares = [tracingMw];
    mgr.apply(registry, executor);

    const composite = (tracingMw as unknown as Record<string, unknown>)['_exporter'] as Record<string, unknown> & {
      export: (s: unknown) => void;
    };
    expect(composite).not.toBe(failing);
    expect(composite).not.toBe(good);
    expect(composite['_exporters']).toEqual([failing, good]);

    // Error isolation: failing exporter throws, good exporter still receives.
    const sentinel = { id: 'sentinel' } as unknown as Span;
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    composite.export(sentinel);
    warnSpy.mockRestore();
    expect(good.exported).toEqual([sentinel]);
  });

  it('extension_system.apply.side_effect.6.no_tracing_middleware_no_raise: no TracingMiddleware -> no raise, no wiring', () => {
    const mgr = new ExtensionManager();
    mgr.register('span_exporter', new StubExporter());
    const { registry, executor } = freshTargets(); // empty middleware chain
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    // Must not raise despite the spec's ExtensionApplyError clause.
    expect(() => mgr.apply(registry, executor)).not.toThrow();
    warnSpy.mockRestore();
    // No TracingMiddleware -> nothing to wire the exporter onto.
    expect(executor.middlewares).toEqual([]);
  });

  it('extension_system.apply.property.idempotent.false: apply twice stacks middleware (not deduped)', () => {
    const mgr = new ExtensionManager();
    const mw = new StubMiddleware();
    mgr.register('middleware', mw);
    const { registry, executor } = freshTargets();

    mgr.apply(registry, executor);
    mgr.apply(registry, executor);

    const used = executor.use.mock.calls.map((c) => c[0]);
    expect(used).toEqual([mw, mw]);
  });

  it('extension_system.apply.side_effect.ordered.full_sequence: discoverer -> module_validator -> acl -> approval_handler -> middleware', () => {
    const order: string[] = [];

    const mgr = new ExtensionManager();
    mgr.register('discoverer', new StubDiscoverer());
    mgr.register('module_validator', new StubValidator());
    mgr.register('acl', new ACL([]));
    mgr.register('approval_handler', new StubApprovalHandler());
    mgr.register('middleware', new StubMiddleware());

    const registry = {
      setDiscoverer: vi.fn(() => order.push('discoverer')),
      setValidator: vi.fn(() => order.push('module_validator')),
    } as unknown as Registry;
    const executor = {
      use: vi.fn(() => order.push('middleware')),
      setAcl: vi.fn(() => order.push('acl')),
      setApprovalHandler: vi.fn(() => order.push('approval_handler')),
      middlewares: [] as unknown[],
    } as unknown as Executor;

    mgr.apply(registry, executor);

    expect(order).toEqual([
      'discoverer',
      'module_validator',
      'acl',
      'approval_handler',
      'middleware',
    ]);
  });
});
