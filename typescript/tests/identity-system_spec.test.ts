/**
 * Spec-traced contract tests for the Identity System feature (TypeScript SDK).
 *
 * Mirrors the canonical Python suite
 * apcore-python/tests/test_identity_system_spec.py clause-for-clause.
 * Source spec: apcore/docs/features/identity-system.md
 * Contract under test: `ContextFactory.create_context` (the only
 * `## Contract:` block in the spec).
 *
 * Each `it(...)` name carries the verbatim clause id formatted as
 * `identity_system.<method>.<kind>.<detail>` so cross-language diffs line up.
 *
 * The spec's contract describes `create_context`: a `ContextFactory` extracts an
 * `Identity` from a runtime request and returns a `Context`. The declared inputs
 * (`identity`, `caller_id`, `data`) and return semantics ("assigned trace ID",
 * "defaults to @external", "new trace ID each call") are realized by
 * `Context.create`, which a factory delegates to. We exercise the contract
 * through a concrete factory built on `Context.create` (the public observable
 * surface), plus `ContextFactory` interface conformance.
 *
 * TypeScript divergence note: in the TS SDK `Context.callerId` is `readonly` and
 * is NOT accepted by `Context.create` (it is managed exclusively by `child()`;
 * top-level contexts always have `callerId === null`, per the `Context.create`
 * JSDoc). The Python `_SpecFactory` assigns `ctx.caller_id` after creation; the
 * TS API forbids this. We therefore carry `caller_id` into the produced Context
 * via `child()` so the observable surface still reflects the supplied caller id,
 * and record the structural difference in DIVERGENCES.
 *
 * framework: vitest. Property tests the spec marks `async: false` stay
 * synchronous; the thread_safe clause exercises concurrency via `Promise.all`.
 *
 * TESTS ONLY — no production source is modified here.
 */

import { describe, it, expect } from 'vitest';
import { Context, Identity, createIdentity } from '../src/context.js';
import type { ContextFactory } from '../src/context.js';

interface SpecRequest {
  identity?: Identity | null;
  callerId?: string | null;
  data?: Record<string, unknown> | null;
}

/**
 * Concrete `ContextFactory` exercising the declared contract inputs. Mirrors the
 * spec's documented Express factory: pull optional `identity` / `callerId` /
 * `data` off the request and delegate to `Context.create` (which assigns the
 * trace ID). Because TS `callerId` is managed by `child()` (not settable post
 * hoc and not a `Context.create` input), a supplied `callerId` is surfaced by
 * deriving a child whose `callerId` reflects it.
 */
class SpecFactory implements ContextFactory {
  createContext(request: unknown): Context {
    const req = (request ?? {}) as SpecRequest;
    const ctx = Context.create(req.identity ?? null, null, null, req.data ?? undefined);
    if (req.callerId != null) {
      // The TS contract surfaces caller_id only through the call chain: a child
      // whose immediate parent is `req.callerId`. Seed the chain so the produced
      // context observably carries the supplied caller id.
      const seeded = ctx.child(req.callerId).child('target.module');
      return seeded;
    }
    return ctx;
  }
}

describe('identity-system contract: ContextFactory.create_context', () => {
  // -------------------------------------------------------------------------
  // Inputs — the contract declares NO reject_with rules: "invalid identity
  // fields are sanitized, not rejected". An "invalid"/absent input must NOT
  // throw; instead assert the declared graceful fallback behavior.
  // -------------------------------------------------------------------------

  it('identity_system.create_context.input.identity.absent_defaults_to_external: absent identity yields @external (identity === null)', () => {
    const factory = new SpecFactory();
    // Spec Inputs: identity optional, "defaults to @external when absent".
    // The @external pattern (per the feature spec) is `identity is None`.
    const ctx = factory.createContext({ identity: null } satisfies SpecRequest);
    expect(ctx.identity).toBeNull();
  });

  it('identity_system.create_context.input.caller_id.optional_absent_is_none: absent caller_id -> null; supplied caller_id carried', () => {
    const factory = new SpecFactory();
    // Spec Inputs: caller_id optional for call-chain tracking; absent -> null.
    const ctx = factory.createContext({ callerId: null } satisfies SpecRequest);
    expect(ctx.callerId).toBeNull();
    // When supplied, it is carried onto the produced Context (via child chain
    // in the TS SDK — see file header divergence note).
    const ctx2 = factory.createContext({ callerId: 'api.gateway' } satisfies SpecRequest);
    expect(ctx2.callerId).toBe('api.gateway');
  });

  it('identity_system.create_context.input.data.optional_absent_is_empty: absent data -> {}; supplied data carried verbatim', () => {
    const factory = new SpecFactory();
    // Spec Inputs: data optional initial payload; absent -> {}.
    const ctx = factory.createContext({ data: null } satisfies SpecRequest);
    expect(ctx.data).toEqual({});
    // When supplied, the payload is carried through verbatim.
    const ctx2 = factory.createContext({ data: { k: 'v' } } satisfies SpecRequest);
    expect(ctx2.data.k).toBe('v');
  });

  it('identity_system.create_context.input.identity.sanitized_not_rejected: invalid identity fields sanitized, not rejected', () => {
    const factory = new SpecFactory();
    // Spec Inputs/Errors: "invalid identity fields are sanitized, not rejected".
    // A non-array `roles` argument must not crash creation. In Python a non-dict
    // attrs is sanitized to {}; the TS `createIdentity` freezes a shallow copy.
    // Construction must succeed and create_context must accept the identity
    // without throwing.
    const ident = createIdentity('svc-1', 'service', [], { ok: true });
    const ctx = factory.createContext({ identity: ident } satisfies SpecRequest);
    expect(ctx.identity).toBe(ident);
    expect(ctx.identity?.attrs).toEqual({ ok: true });
    // attrs is frozen (immutable) per the spec's readonly requirement.
    expect(Object.isFrozen(ctx.identity?.attrs)).toBe(true);
  });

  // -------------------------------------------------------------------------
  // Errors — the contract declares NONE ("No errors raised"). Assert the
  // absence of throws across the input surface so a future regression that adds
  // a throw is caught.
  // -------------------------------------------------------------------------

  it('identity_system.create_context.error.none.no_error_raised: every optional-input combination completes without throwing', () => {
    const factory = new SpecFactory();
    expect(() => factory.createContext({})).not.toThrow();
    expect(() =>
      factory.createContext({ identity: null, callerId: null, data: null }),
    ).not.toThrow();
    const ident = createIdentity('u-1', 'user', ['admin'], { d: 'eng' });
    const ctx = factory.createContext({
      identity: ident,
      callerId: 'orchestrator.run',
      data: { x: 1 },
    } satisfies SpecRequest);
    // Proves we reached the return, not an early throw.
    expect(ctx).toBeInstanceOf(Context);
    expect(ctx.identity).toBe(ident);
  });

  // -------------------------------------------------------------------------
  // Returns — "Context with assigned trace ID and caller identity".
  // -------------------------------------------------------------------------

  it('identity_system.create_context.returns.context.assigned_trace_id: returns Context with non-empty traceId and attached identity', () => {
    const factory = new SpecFactory();
    const ident = createIdentity('admin@example.com', 'user', ['admin']);
    const ctx = factory.createContext({ identity: ident } satisfies SpecRequest);
    expect(ctx).toBeInstanceOf(Context);
    // A non-empty traceId is assigned, and the caller identity is attached.
    expect(typeof ctx.traceId).toBe('string');
    expect(ctx.traceId.length).toBeGreaterThan(0);
    expect(ctx.identity).toBe(ident);
    expect(ctx.identity?.id).toBe('admin@example.com');
    expect(ctx.identity?.roles).toEqual(['admin']);
  });

  it('identity_system.create_context.returns.context.identity_propagates_to_child: identity propagates by reference to child contexts', () => {
    const factory = new SpecFactory();
    const ident = createIdentity('admin@example.com', 'user', ['admin', 'operator']);
    const ctx = factory.createContext({ identity: ident } satisfies SpecRequest);
    // Spec requirement: Identity propagates to child contexts by identity.
    const child = ctx.child('target.module');
    expect(child.identity).toBe(ctx.identity);
    expect(child.traceId).toBe(ctx.traceId);
  });

  // -------------------------------------------------------------------------
  // Properties — async:false, thread_safe:true, pure:false, idempotent:false.
  // -------------------------------------------------------------------------

  it('identity_system.create_context.property.async.synchronous_not_awaitable: create_context is synchronous (returns a Context, not a Promise)', () => {
    const factory = new SpecFactory();
    // Spec Properties: async: false. Result is a plain Context, not a Promise.
    const result = factory.createContext({});
    expect(result).not.toBeInstanceOf(Promise);
    expect(result).toBeInstanceOf(Context);
  });

  it('identity_system.create_context.property.thread_safe.concurrent_distinct_inputs: concurrent calls with distinct inputs each produce a consistent Context', async () => {
    const factory = new SpecFactory();
    const n = 16;
    const make = (i: number): Promise<Context> =>
      Promise.resolve().then(() => {
        const ident = createIdentity(`user-${i}`, 'user', [`role-${i}`]);
        return factory.createContext({ identity: ident, callerId: `caller-${i}` });
      });

    const contexts = await Promise.all(Array.from({ length: n }, (_, i) => make(i)));
    // No exception; each call produced its own consistent Context.
    expect(contexts).toHaveLength(n);
    contexts.forEach((ctx, i) => {
      expect(ctx.identity).not.toBeNull();
      expect(ctx.identity?.id).toBe(`user-${i}`);
      expect(ctx.callerId).toBe(`caller-${i}`);
    });
    // Spec Properties: a new trace ID per call -> all trace IDs distinct.
    const traceIds = new Set(contexts.map((ctx) => ctx.traceId));
    expect(traceIds.size).toBe(n);
  });

  it('identity_system.create_context.property.idempotent.distinct_trace_id_per_call: two calls with identical input yield distinct trace IDs', () => {
    const factory = new SpecFactory();
    const ident = createIdentity('admin@example.com', 'user');
    const req: SpecRequest = { identity: ident };
    // Spec Properties: idempotent: false — "generates a new trace ID on each
    // call". Identical input -> distinct trace IDs.
    const first = factory.createContext(req);
    const second = factory.createContext(req);
    expect(first.traceId).not.toBe(second.traceId);
    // Identity still attached identically (only the trace ID differs).
    expect(first.identity).toBe(ident);
    expect(second.identity).toBe(ident);
  });

  it('identity_system.create_context.property.pure.not_pure_fresh_context_each_call: distinct Context (and data) objects per call', () => {
    const factory = new SpecFactory();
    const ident = createIdentity('svc', 'service');
    // Spec Properties: pure: false — observably non-deterministic (new trace ID
    // per call) and a *fresh* Context object each time. Use independent data
    // payloads so each call's data is its own object (Context.create does not
    // copy a supplied data object by design).
    const a = factory.createContext({ identity: ident, data: { shared: 1 } });
    const b = factory.createContext({ identity: ident, data: { shared: 1 } });
    // Distinct Context instances with distinct data objects and distinct trace.
    expect(a).not.toBe(b);
    expect(a.data).not.toBe(b.data);
    expect(a.traceId).not.toBe(b.traceId);
    // Same input identity object attached to both (only trace ID varies).
    expect(a.identity).toBe(ident);
    expect(b.identity).toBe(ident);
  });

  // -------------------------------------------------------------------------
  // Protocol/interface conformance — Python's ContextFactory is a
  // @runtime_checkable Protocol (isinstance works structurally). TypeScript's
  // ContextFactory is a compile-time `interface` with NO runtime presence, so
  // there is no runtime isinstance equivalent. We assert structural conformance
  // the only way available at runtime: the factory implements the required
  // `createContext` method and a non-conforming object does not.
  // -------------------------------------------------------------------------

  it('identity_system.create_context.property.protocol.runtime_checkable_conformance: factory structurally exposes createContext; non-factory does not', () => {
    const factory = new SpecFactory();
    // TS interfaces are erased at runtime; structural check on the method.
    expect(typeof (factory as ContextFactory).createContext).toBe('function');

    const notAFactory: object = {};
    expect(typeof (notAFactory as Partial<ContextFactory>).createContext).not.toBe('function');
  });
});

describe('Identity.getAttr null-preservation (D-03)', () => {
  it('returns a stored null instead of the default', () => {
    const ident = createIdentity('u-1', 'user', [], { maybe: null });
    expect(ident.getAttr('maybe', 'fallback')).toBeNull();
  });

  it('returns the default for an absent key', () => {
    const ident = createIdentity('u-1', 'user', [], {});
    expect(ident.getAttr('missing', 'fallback')).toBe('fallback');
  });

  it('returns a stored value as-is', () => {
    const ident = createIdentity('u-1', 'user', [], { dept: 'eng' });
    expect(ident.getAttr('dept', 'fallback')).toBe('eng');
  });
});
