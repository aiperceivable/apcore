/**
 * D-76, D-78 and D-106, pinned in this SDK.
 *
 * Each was implemented here and pinned only in apcore-rust — the recurring
 * shape this audit kept finding: a decision's AUTHORITY is the SDK least likely
 * to be covered, because the SDKs that had to CHANGE got tests as part of the
 * change and the one they were aligned to did not.
 *
 * D-106 is the sharper case. Pinning it turned up a live defect in
 * apcore-python, which the decision names as one of its two authorities: with
 * every observation overflowing the top bucket it returned 0.0 — the failure
 * D-106 forbids — because it collected its bucket ladder from the RECORDED
 * keys, and an all-overflow histogram records nothing but the `inf` key. This
 * SDK iterates `metricsCollector.buckets`, the configured ladder, and is
 * correct; the test below is what stops that drifting.
 */

import { describe, it, expect } from 'vitest';
import { Context, type ContextFactory, Identity } from '../src/context.js';
import { ExtensionManager } from '../src/extensions.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import { Middleware } from '../src/middleware/base.js';
import { MetricsCollector } from '../src/observability/metrics.js';
import { estimateP99FromHistogram } from '../src/observability/metrics-utils.js';

// ---------------------------------------------------------------------------
// D-76 — `ContextFactory.createContext(request)`
// ---------------------------------------------------------------------------

describe('D-76: the context factory takes a request', () => {
  // The Contract declared `(identity, caller_id, data)`; no SDK took that. The
  // decision rewrote it to `create_context(request) -> Context` because that is
  // what the interface is FOR: a web-framework integration exists to extract an
  // identity FROM a request, and a signature that already receives an
  // `Identity` has had that work done for it.

  interface FakeRequest {
    userId: string | null;
    roles: string[];
  }

  class WebFactory implements ContextFactory {
    createContext(request: unknown): Context {
      const req = request as FakeRequest;
      if (req.userId === null) return Context.create();
      return Context.create(new Identity(req.userId, 'user', req.roles));
    }
  }

  it('the factory reads the identity out of the request', () => {
    const ctx = new WebFactory().createContext({ userId: 'u-7', roles: ['viewer'] });
    expect(ctx.identity).not.toBeNull();
    expect(ctx.identity?.id).toBe('u-7');
    expect(ctx.identity?.roles).toEqual(['viewer']);
  });

  it('control: an unauthenticated request yields no identity', () => {
    // Without this, "the identity came from the request" is also satisfied by a
    // factory that manufactures one regardless of what it was handed. A null
    // identity stays null (D-103) — `@external` is the caller-side ACL
    // sentinel, not a principal.
    const ctx = new WebFactory().createContext({ userId: null, roles: [] });
    expect(ctx.identity).toBeNull();
  });

  it('the declared parameter is the request, structurally', () => {
    // `ContextFactory` is an interface, so it is erased at runtime and cannot
    // be inspected the way apcore-python inspects its Protocol. What CAN be
    // asserted is that a one-parameter factory satisfies it and is assignable
    // — the compiler rejects the old `(identity, callerId, data)` shape, and
    // `tsc --noEmit` is part of the gate that runs this file.
    const factory: ContextFactory = new WebFactory();
    expect(factory.createContext.length).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// D-78 — `ExtensionManager.apply` must not drain the store
// ---------------------------------------------------------------------------

describe('D-78: apply retains the store', () => {
  // No Postconditions section existed, and apcore-rust consumed its
  // registrations. Applying one manager to two executors wires both here and
  // only the first there, silently. The decision cites the block's own
  // `idempotent: false` row as decisive: it promises that calling `apply`
  // twice STACKS middleware, and only a non-consuming implementation can
  // produce that observable.

  class Probe extends Middleware {}

  function managerWithOneMiddleware(): { mgr: ExtensionManager; probe: Probe } {
    const mgr = new ExtensionManager();
    const probe = new Probe();
    mgr.register('middleware', probe);
    return { mgr, probe };
  }

  const executor = (): Executor => new Executor({ registry: new Registry() });

  it('the store survives apply', () => {
    const { mgr, probe } = managerWithOneMiddleware();
    mgr.apply(new Registry(), executor());
    expect(mgr.getAll('middleware')).toHaveLength(1);
    expect(mgr.getAll('middleware')[0]).toBe(probe);
  });

  it('one manager wires two executors', () => {
    // The consequence the decision is about: a drained store wires the first
    // executor and silently leaves the second bare.
    const { mgr, probe } = managerWithOneMiddleware();
    const first = executor();
    const second = executor();
    mgr.apply(new Registry(), first);
    mgr.apply(new Registry(), second);

    expect(first.middlewares).toContain(probe);
    expect(second.middlewares).toContain(probe);
  });

  it('applying twice to one executor stacks', () => {
    // `idempotent: false` — the row that settled the decision, and the
    // assertion a consuming implementation cannot satisfy: the second apply
    // has nothing left to wire.
    const { mgr, probe } = managerWithOneMiddleware();
    const ex = executor();
    mgr.apply(new Registry(), ex);
    mgr.apply(new Registry(), ex);

    expect(ex.middlewares.filter((m) => m === probe)).toHaveLength(2);
  });

  it('control: an empty manager wires nothing', () => {
    // Without this, "both executors have the middleware" is also satisfied by
    // an executor that arrives with middleware of its own.
    const ex = executor();
    const before = [...ex.middlewares];
    new ExtensionManager().apply(new Registry(), ex);
    expect(ex.middlewares).toEqual(before);
  });
});

// ---------------------------------------------------------------------------
// D-106 — a p99 beyond the largest bucket is that bucket, not zero
// ---------------------------------------------------------------------------

describe('D-106: p99 falls back to the largest finite bucket', () => {
  // Returning 0.0 reports the FASTEST possible latency for the SLOWEST
  // modules, so a latency alert can never fire for a module slower than the
  // top bucket.

  function collectorWith(seconds: number, moduleId: string): MetricsCollector {
    const m = new MetricsCollector();
    for (let i = 0; i < 5; i++) m.observeDuration(moduleId, seconds);
    return m;
  }

  it('every observation overflowing reports the top bucket', () => {
    const m = new MetricsCollector();
    const top = m.buckets[m.buckets.length - 1] as number;
    for (let i = 0; i < 5; i++) m.observeDuration('executor.d106.slow', top * 2);

    const { p99LatencyMs } = estimateP99FromHistogram(m, 'executor.d106.slow');
    expect(p99LatencyMs).toBeCloseTo(top * 1000, 0);
  });

  it('control: no data still reports zero', () => {
    // Zero must still mean "no data". Without this, the fix could be a blanket
    // "always return the top bucket", which reports the SLOWEST possible
    // latency for a module that was never called.
    const { p99LatencyMs } = estimateP99FromHistogram(
      new MetricsCollector(),
      'executor.d106.never_called',
    );
    expect(p99LatencyMs).toBe(0);
  });

  it('control: an in-range observation is not pushed to the top', () => {
    // The other direction: a fast module must not report the top bucket.
    const m = collectorWith(0.02, 'executor.d106.fast');
    const topMs = (m.buckets[m.buckets.length - 1] as number) * 1000;
    const { p99LatencyMs } = estimateP99FromHistogram(m, 'executor.d106.fast');

    expect(p99LatencyMs).toBeGreaterThan(0);
    expect(p99LatencyMs).toBeLessThan(topMs);
  });
});
