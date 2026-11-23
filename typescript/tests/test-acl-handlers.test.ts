/**
 * Tests for acl-handlers.ts: IdentityTypesHandler, RolesHandler, MaxCallDepthHandler,
 * OrHandler, NotHandler, arraysEqual, and deepEqual.
 */

import { describe, it, expect } from 'vitest';
import {
  IdentityTypesHandler,
  RolesHandler,
  MaxCallDepthHandler,
  OrHandler,
  NotHandler,
  arraysEqual,
  deepEqual,
} from '../src/acl-handlers.js';
import type { ConditionOutcome } from '../src/acl-handlers.js';
import { Context, createIdentity } from '../src/context.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeContext(opts: {
  identityType?: string;
  roles?: string[];
  callChain?: string[];
  hasIdentity?: boolean;
} = {}): Context {
  const identity =
    opts.hasIdentity === false
      ? null
      : opts.identityType !== undefined
      ? createIdentity('user-1', opts.identityType, opts.roles ?? [])
      : null;
  return new Context('trace-id', null, opts.callChain ?? [], null, identity);
}

// ---------------------------------------------------------------------------
// IdentityTypesHandler
// ---------------------------------------------------------------------------

describe('IdentityTypesHandler', () => {
  const handler = new IdentityTypesHandler();

  it('returns true when identity type is in allowed list', () => {
    const ctx = makeContext({ identityType: 'admin' });
    expect(handler.evaluate(['admin', 'superuser'], ctx)).toBe(true);
  });

  it('returns false when identity type is not in allowed list', () => {
    const ctx = makeContext({ identityType: 'user' });
    expect(handler.evaluate(['admin'], ctx)).toBe(false);
  });

  it('returns false when identity is null', () => {
    const ctx = makeContext({ hasIdentity: false });
    expect(handler.evaluate(['admin'], ctx)).toBe(false);
  });

  it('returns false when value is not an array', () => {
    const ctx = makeContext({ identityType: 'admin' });
    expect(handler.evaluate('admin', ctx)).toBe(false);
    expect(handler.evaluate(null, ctx)).toBe(false);
    expect(handler.evaluate(42, ctx)).toBe(false);
  });

  it('returns false for empty allowed list', () => {
    const ctx = makeContext({ identityType: 'user' });
    expect(handler.evaluate([], ctx)).toBe(false);
  });

  it('returns true with exact single-item match', () => {
    const ctx = makeContext({ identityType: 'service' });
    expect(handler.evaluate(['service'], ctx)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// RolesHandler
// ---------------------------------------------------------------------------

describe('RolesHandler', () => {
  const handler = new RolesHandler();

  it('returns true when identity has at least one required role', () => {
    const ctx = makeContext({ identityType: 'user', roles: ['editor', 'viewer'] });
    expect(handler.evaluate(['admin', 'editor'], ctx)).toBe(true);
  });

  it('returns false when identity has none of the required roles', () => {
    const ctx = makeContext({ identityType: 'user', roles: ['viewer'] });
    expect(handler.evaluate(['admin', 'editor'], ctx)).toBe(false);
  });

  it('returns false when identity is null', () => {
    const ctx = makeContext({ hasIdentity: false });
    expect(handler.evaluate(['admin'], ctx)).toBe(false);
  });

  it('returns false when value is not an array', () => {
    const ctx = makeContext({ identityType: 'user', roles: ['admin'] });
    expect(handler.evaluate('admin', ctx)).toBe(false);
    expect(handler.evaluate({ role: 'admin' }, ctx)).toBe(false);
  });

  it('returns false for empty roles list', () => {
    const ctx = makeContext({ identityType: 'user', roles: ['admin'] });
    expect(handler.evaluate([], ctx)).toBe(false);
  });

  it('returns false when identity has no roles', () => {
    const ctx = makeContext({ identityType: 'user', roles: [] });
    expect(handler.evaluate(['admin'], ctx)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// MaxCallDepthHandler
// ---------------------------------------------------------------------------

describe('MaxCallDepthHandler', () => {
  const handler = new MaxCallDepthHandler();

  it('returns true when call chain length is within numeric threshold', () => {
    const ctx = makeContext({ callChain: ['a', 'b'] });
    expect(handler.evaluate(3, ctx)).toBe(true);
  });

  it('returns true when call chain length exactly equals threshold', () => {
    const ctx = makeContext({ callChain: ['a', 'b'] });
    expect(handler.evaluate(2, ctx)).toBe(true);
  });

  it('returns false when call chain exceeds numeric threshold', () => {
    const ctx = makeContext({ callChain: ['a', 'b', 'c'] });
    expect(handler.evaluate(2, ctx)).toBe(false);
  });

  it('returns true with object { lte: N } format within limit', () => {
    const ctx = makeContext({ callChain: ['a'] });
    expect(handler.evaluate({ lte: 3 }, ctx)).toBe(true);
  });

  it('returns false with object { lte: N } format exceeding limit', () => {
    const ctx = makeContext({ callChain: ['a', 'b', 'c', 'd'] });
    expect(handler.evaluate({ lte: 3 }, ctx)).toBe(false);
  });

  it('returns false when value is not a number or valid object', () => {
    const ctx = makeContext({ callChain: [] });
    expect(handler.evaluate('5', ctx)).toBe(false);
    expect(handler.evaluate(null, ctx)).toBe(false);
    expect(handler.evaluate([], ctx)).toBe(false);
  });

  it('returns true when call chain is empty and threshold is 0', () => {
    const ctx = makeContext({ callChain: [] });
    expect(handler.evaluate(0, ctx)).toBe(true);
  });

  it('fails closed on fractional numeric threshold (A-D-013)', () => {
    const ctx = makeContext({ callChain: ['a', 'b'] });
    // 5.5 would otherwise ALLOW (2 <= 5.5); must reject as non-integer.
    expect(handler.evaluate(5.5, ctx)).toBe(false);
  });

  it('fails closed on fractional { lte } threshold (A-D-013)', () => {
    const ctx = makeContext({ callChain: ['a', 'b'] });
    expect(handler.evaluate({ lte: 5.5 }, ctx)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// OrHandler
// ---------------------------------------------------------------------------

describe('OrHandler', () => {
  it('returns true when at least one sub-condition passes', () => {
    let evalCallCount = 0;
    const evalFn = (conds: Record<string, unknown>): ConditionOutcome => {
      evalCallCount++;
      return conds['match'] === true ? 'satisfied' : 'unsatisfied';
    };
    const handler = new OrHandler(evalFn);
    const ctx = makeContext();
    const value = [{ match: false }, { match: true }];
    expect(handler.evaluate(value, ctx)).toBe(true);
    // Both branches are visited: $or short-circuits only on a SATISFIED child,
    // and the first child here is UNSATISFIED.
    expect(evalCallCount).toBe(2);
  });

  it('returns false when all sub-conditions fail', () => {
    const evalFn = (): ConditionOutcome => 'unsatisfied';
    const handler = new OrHandler(evalFn);
    const ctx = makeContext();
    expect(handler.evaluate([{ a: 1 }, { b: 2 }], ctx)).toBe(false);
  });

  it('reports UNEVALUABLE when value is not an array (§6.1.1 case 4)', () => {
    // A value malformed for its key is unevaluable, not false. A handler handed
    // `$or: "typo"` that returns false looks exactly like one that answered
    // "no", which put a deny rule carrying the typo back into the inert state
    // §6.1.1 exists to end.
    const evalFn = (): ConditionOutcome => 'satisfied';
    const handler = new OrHandler(evalFn);
    const ctx = makeContext();
    expect(handler.evaluateOutcome({ sub: true }, ctx)).toBe('unevaluable');
    expect(handler.evaluateOutcome('conditions', ctx)).toBe('unevaluable');
    // The legacy boolean surface still reports "not satisfied".
    expect(handler.evaluate('conditions', ctx)).toBe(false);
  });

  it('returns false for empty array', () => {
    const evalFn = (): ConditionOutcome => 'satisfied';
    const handler = new OrHandler(evalFn);
    const ctx = makeContext();
    expect(handler.evaluate([], ctx)).toBe(false);
  });

  it('reports UNEVALUABLE for non-object, null, and array sub-conditions', () => {
    const evalFn = (): ConditionOutcome => 'satisfied';
    const handler = new OrHandler(evalFn);
    const ctx = makeContext();
    // A branch that is not a condition object is malformed, not absent.
    expect(handler.evaluateOutcome(['string', null, [1, 2]], ctx)).toBe('unevaluable');
    expect(handler.evaluate(['string', null, [1, 2]], ctx)).toBe(false);
  });

  it('passes context and the §6.1.4 child path to the inner evaluate function', () => {
    let receivedContext: Context | null = null;
    let receivedPath: string | null = null;
    const evalFn = (
      _conds: Record<string, unknown>,
      context: Context,
      childPath: string,
    ): ConditionOutcome => {
      receivedContext = context;
      receivedPath = childPath;
      return 'satisfied';
    };
    const handler = new OrHandler(evalFn);
    const ctx = makeContext({ identityType: 'user' });
    handler.evaluate([{ x: 1 }], ctx);
    expect(receivedContext).toBe(ctx);
    // A branch's path carries its 0-based index, so a key inside it is
    // reported as `$or[0].k` rather than under the operator's own key.
    expect(receivedPath).toBe('$or[0]');
  });
});

// ---------------------------------------------------------------------------
// NotHandler
// ---------------------------------------------------------------------------

describe('NotHandler', () => {
  it('returns true when inner condition fails', () => {
    const evalFn = (): ConditionOutcome => 'unsatisfied';
    const handler = new NotHandler(evalFn);
    const ctx = makeContext();
    expect(handler.evaluate({ something: true }, ctx)).toBe(true);
  });

  it('returns false when inner condition passes', () => {
    const evalFn = (): ConditionOutcome => 'satisfied';
    const handler = new NotHandler(evalFn);
    const ctx = makeContext();
    expect(handler.evaluate({ something: true }, ctx)).toBe(false);
  });

  it('reports UNEVALUABLE when value is not a plain object (§6.1.1 case 4)', () => {
    const evalFn = (): ConditionOutcome => 'satisfied';
    const handler = new NotHandler(evalFn);
    const ctx = makeContext();
    for (const bad of ['string', null, [{ x: 1 }]] as unknown[]) {
      expect(handler.evaluateOutcome(bad, ctx)).toBe('unevaluable');
      expect(handler.evaluate(bad, ctx)).toBe(false);
    }
  });

  it('passes context to inner evaluate function', () => {
    let receivedContext: Context | null = null;
    const evalFn = (_conds: Record<string, unknown>, context: Context): ConditionOutcome => {
      receivedContext = context;
      return 'unsatisfied';
    };
    const handler = new NotHandler(evalFn);
    const ctx = makeContext({ identityType: 'admin' });
    handler.evaluate({ x: 1 }, ctx);
    expect(receivedContext).toBe(ctx);
  });
});

// ---------------------------------------------------------------------------
// arraysEqual
// ---------------------------------------------------------------------------

describe('arraysEqual', () => {
  it('returns true for two identical arrays', () => {
    expect(arraysEqual(['a', 'b', 'c'], ['a', 'b', 'c'])).toBe(true);
  });

  it('returns false for arrays of different lengths', () => {
    expect(arraysEqual(['a', 'b'], ['a'])).toBe(false);
  });

  it('returns false for arrays with different elements', () => {
    expect(arraysEqual(['a', 'b'], ['a', 'c'])).toBe(false);
  });

  it('returns true for two empty arrays', () => {
    expect(arraysEqual([], [])).toBe(true);
  });

  it('uses reference equality for elements', () => {
    const obj = { key: 'val' };
    expect(arraysEqual([obj], [obj])).toBe(true);
    expect(arraysEqual([{ key: 'val' }], [{ key: 'val' }])).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// deepEqual
// ---------------------------------------------------------------------------

describe('deepEqual', () => {
  it('returns true for identical primitives', () => {
    expect(deepEqual(1, 1)).toBe(true);
    expect(deepEqual('hello', 'hello')).toBe(true);
    expect(deepEqual(null, null)).toBe(true);
  });

  it('returns false for different primitives', () => {
    expect(deepEqual(1, 2)).toBe(false);
    expect(deepEqual('a', 'b')).toBe(false);
    expect(deepEqual(null, undefined)).toBe(false);
  });

  it('returns true for deeply equal objects', () => {
    expect(deepEqual({ a: 1, b: { c: 2 } }, { a: 1, b: { c: 2 } })).toBe(true);
  });

  it('returns false for objects with different values', () => {
    expect(deepEqual({ a: 1 }, { a: 2 })).toBe(false);
  });

  it('returns false for objects with different keys', () => {
    expect(deepEqual({ a: 1 }, { b: 1 })).toBe(false);
  });

  it('returns true for equal nested arrays', () => {
    expect(deepEqual([1, [2, 3]], [1, [2, 3]])).toBe(true);
  });

  it('returns false for arrays of different lengths', () => {
    expect(deepEqual([1, 2], [1])).toBe(false);
  });

  it('returns false when one is an array and the other is an object', () => {
    expect(deepEqual([1, 2], { 0: 1, 1: 2 })).toBe(false);
  });

  it('returns false for different types', () => {
    expect(deepEqual(1, '1')).toBe(false);
    expect(deepEqual({}, null)).toBe(false);
  });

  it('returns true for same object reference', () => {
    const obj = { x: 1 };
    expect(deepEqual(obj, obj)).toBe(true);
  });
});
