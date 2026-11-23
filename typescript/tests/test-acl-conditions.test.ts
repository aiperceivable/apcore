import { describe, it, expect, afterEach, vi } from 'vitest';
import { ACL, _parseAclRule } from '../src/acl.js';
import type { AuditEntry } from '../src/acl.js';
import { ACLRuleError } from '../src/errors.js';
import type { ACLConditionHandler } from '../src/acl-handlers.js';
import {
  IdentityTypesHandler,
  RolesHandler,
  MaxCallDepthHandler,
  arraysEqual,
  deepEqual,
} from '../src/acl-handlers.js';
import { Context, createIdentity } from '../src/context.js';
import type { ACLRule } from '../src/acl.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeContext(opts: {
  identityType?: string;
  roles?: string[];
  callChain?: string[];
} = {}): Context {
  const identity = opts.identityType
    ? createIdentity('test-user', opts.identityType, opts.roles ?? [])
    : null;
  return new Context(
    'trace-test',
    null,
    opts.callChain ?? [],
    null,
    identity,
  );
}

function makeAclWithCondition(
  conditionKey: string,
  conditionValue: unknown,
  effect: string = 'allow',
): ACL {
  return new ACL([{
    callers: ['*'],
    targets: ['*'],
    effect,
    description: '',
    conditions: { [conditionKey]: conditionValue },
  }], 'deny');
}

// ---------------------------------------------------------------------------
// Handler Registry
// ---------------------------------------------------------------------------

describe('Handler Registry', () => {
  afterEach(() => {
    // Clean up test handlers
    // Access private map via any cast for cleanup
    (ACL as any).conditionHandlers.delete('_test_custom');
    (ACL as any).conditionHandlers.delete('_test_replace');
    (ACL as any).conditionHandlers.delete('_test_async');
  });

  it('registers a custom handler invoked during check (AC-009)', () => {
    const invoked: boolean[] = [];
    ACL.registerCondition('_test_custom', {
      evaluate: (value: unknown, _ctx: Context) => {
        invoked.push(true);
        return value === 'magic';
      },
    });
    const ctx = makeContext({ identityType: 'user' });
    const acl = makeAclWithCondition('_test_custom', 'magic');
    expect(acl.check('caller', 'target', ctx)).toBe(true);
    expect(invoked).toEqual([true]);
  });

  it('replaces handler when same key registered twice (AC-031)', () => {
    const calls: string[] = [];
    ACL.registerCondition('_test_replace', {
      evaluate: () => { calls.push('A'); return true; },
    });
    ACL.registerCondition('_test_replace', {
      evaluate: () => { calls.push('B'); return true; },
    });
    const ctx = makeContext({ identityType: 'user' });
    const acl = makeAclWithCondition('_test_replace', true);
    acl.check('caller', 'target', ctx);
    expect(calls).toEqual(['B']);
  });

  it('built-in handlers are auto-registered', () => {
    for (const key of ['identity_types', 'roles', 'max_call_depth', '$or', '$not']) {
      expect((ACL as any).conditionHandlers.has(key)).toBe(true);
    }
  });
});

// ---------------------------------------------------------------------------
// Built-in Handlers — Unit Tests
// ---------------------------------------------------------------------------

describe('IdentityTypesHandler', () => {
  const handler = new IdentityTypesHandler();

  it('matches when type is in list', () => {
    const ctx = makeContext({ identityType: 'service' });
    expect(handler.evaluate(['service', 'admin'], ctx)).toBe(true);
  });

  it('does not match when type is not in list', () => {
    const ctx = makeContext({ identityType: 'user' });
    expect(handler.evaluate(['service', 'admin'], ctx)).toBe(false);
  });

  it('does not match when identity is null', () => {
    const ctx = makeContext();
    expect(handler.evaluate(['user'], ctx)).toBe(false);
  });

  it('does not match when value is not array', () => {
    const ctx = makeContext({ identityType: 'user' });
    expect(handler.evaluate('user', ctx)).toBe(false);
  });
});

describe('RolesHandler', () => {
  const handler = new RolesHandler();

  it('matches when roles overlap', () => {
    const ctx = makeContext({ identityType: 'user', roles: ['admin', 'viewer'] });
    expect(handler.evaluate(['admin'], ctx)).toBe(true);
  });

  it('does not match when no overlap', () => {
    const ctx = makeContext({ identityType: 'user', roles: ['viewer'] });
    expect(handler.evaluate(['admin'], ctx)).toBe(false);
  });

  it('does not match when identity is null', () => {
    const ctx = makeContext();
    expect(handler.evaluate(['admin'], ctx)).toBe(false);
  });
});

describe('MaxCallDepthHandler', () => {
  const handler = new MaxCallDepthHandler();

  it('passes when within limit', () => {
    const ctx = makeContext({ identityType: 'user', callChain: ['a', 'b'] });
    expect(handler.evaluate(5, ctx)).toBe(true);
  });

  it('passes when at limit', () => {
    const ctx = makeContext({ identityType: 'user', callChain: ['a', 'b', 'c'] });
    expect(handler.evaluate(3, ctx)).toBe(true);
  });

  it('fails when exceeds limit', () => {
    const ctx = makeContext({ identityType: 'user', callChain: ['a', 'b', 'c', 'd'] });
    expect(handler.evaluate(3, ctx)).toBe(false);
  });

  it('fails when value is not a number', () => {
    const ctx = makeContext({ identityType: 'user' });
    expect(handler.evaluate('5', ctx)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Compound Handlers
// ---------------------------------------------------------------------------

describe('$or handler', () => {
  it('passes when any sub-condition matches (AC-011)', () => {
    const ctx = makeContext({ identityType: 'user', roles: ['admin'] });
    const acl = makeAclWithCondition('$or', [
      { roles: ['admin'] },
      { identity_types: ['service'] },
    ]);
    expect(acl.check('caller', 'target', ctx)).toBe(true);
  });

  it('fails when no sub-condition matches', () => {
    const ctx = makeContext({ identityType: 'user', roles: ['viewer'] });
    const acl = makeAclWithCondition('$or', [
      { roles: ['admin'] },
      { identity_types: ['service'] },
    ]);
    expect(acl.check('caller', 'target', ctx)).toBe(false);
  });

  it('returns false for empty list (AC-029)', () => {
    const ctx = makeContext({ identityType: 'user' });
    const acl = makeAclWithCondition('$or', []);
    expect(acl.check('caller', 'target', ctx)).toBe(false);
  });

  it('returns false for non-array value', () => {
    const ctx = makeContext({ identityType: 'user' });
    const acl = makeAclWithCondition('$or', 'invalid');
    expect(acl.check('caller', 'target', ctx)).toBe(false);
  });
});

describe('$not handler', () => {
  it('negates conditions — allows user, denies service (AC-012)', () => {
    const ctxUser = makeContext({ identityType: 'user' });
    const ctxService = makeContext({ identityType: 'service' });
    const acl = makeAclWithCondition('$not', { identity_types: ['service'] });
    expect(acl.check('caller', 'target', ctxUser)).toBe(true);
    expect(acl.check('caller', 'target', ctxService)).toBe(false);
  });

  it('returns false for non-dict value (AC-030)', () => {
    const ctx = makeContext({ identityType: 'user' });
    const acl = makeAclWithCondition('$not', 'invalid');
    expect(acl.check('caller', 'target', ctx)).toBe(false);
  });
});

describe('Nested compound conditions (AC-032)', () => {
  it('handles AND within sub-dict and OR across sub-dicts', () => {
    const ctx = makeContext({ identityType: 'service', callChain: ['a', 'b'] });
    const acl = makeAclWithCondition('$or', [
      { roles: ['admin'] },
      { identity_types: ['service'], max_call_depth: 5 },
    ]);
    expect(acl.check('caller', 'target', ctx)).toBe(true);
  });

  it('fails when depth exceeded in nested compound', () => {
    const ctx = makeContext({
      identityType: 'service',
      callChain: Array(10).fill('a'),
    });
    const acl = makeAclWithCondition('$or', [
      { roles: ['admin'] },
      { identity_types: ['service'], max_call_depth: 5 },
    ]);
    expect(acl.check('caller', 'target', ctx)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Fail-closed behavior
// ---------------------------------------------------------------------------

describe('Fail-closed behavior', () => {
  it('unknown condition fails-closed with warn (AC-010)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const ctx = makeContext({ identityType: 'user' });
    const acl = makeAclWithCondition('nonexistent', true);
    expect(acl.check('caller', 'target', ctx)).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Unknown ACL condition'),
    );
    warnSpy.mockRestore();
  });
});

// ---------------------------------------------------------------------------
// asyncCheck
// ---------------------------------------------------------------------------

describe('asyncCheck', () => {
  afterEach(() => {
    (ACL as any).conditionHandlers.delete('_test_async_magic');
  });

  it('basic async check with sync handlers', async () => {
    const ctx = makeContext({ identityType: 'user', roles: ['admin'] });
    const acl = makeAclWithCondition('roles', ['admin']);
    expect(await acl.asyncCheck('caller', 'target', ctx)).toBe(true);
  });

  it('awaits async handlers (AC-013)', async () => {
    ACL.registerCondition('_test_async_magic', {
      evaluate: async (value: unknown) => {
        return value === 'async_magic';
      },
    });
    const ctx = makeContext({ identityType: 'user' });
    const acl = makeAclWithCondition('_test_async_magic', 'async_magic');
    expect(await acl.asyncCheck('caller', 'target', ctx)).toBe(true);
  });

  it('returns default deny when no rules', async () => {
    const ctx = makeContext({ identityType: 'user' });
    const acl = new ACL([], 'deny');
    expect(await acl.asyncCheck('caller', 'target', ctx)).toBe(false);
  });

  it('returns default allow when no rules', async () => {
    const ctx = makeContext({ identityType: 'user' });
    const acl = new ACL([], 'allow');
    expect(await acl.asyncCheck('caller', 'target', ctx)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// removeRule fix (AC-034)
// ---------------------------------------------------------------------------

describe('removeRule element-wise comparison (AC-034)', () => {
  it('removes rule with same arrays', () => {
    const rule: ACLRule = {
      callers: ['a', 'b'],
      targets: ['c', 'd'],
      effect: 'allow',
      description: 'test',
    };
    const acl = new ACL([rule]);
    expect(acl.removeRule(['a', 'b'], ['c', 'd'])).toBe(true);
  });

  it('does not remove when arrays differ', () => {
    const rule: ACLRule = {
      callers: ['a', 'b'],
      targets: ['c', 'd'],
      effect: 'allow',
      description: 'test',
    };
    const acl = new ACL([rule]);
    expect(acl.removeRule(['b', 'a'], ['c', 'd'])).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// arraysEqual and deepEqual utilities
// ---------------------------------------------------------------------------

describe('arraysEqual', () => {
  it('returns true for equal arrays', () => {
    expect(arraysEqual(['a', 'b'], ['a', 'b'])).toBe(true);
  });

  it('returns false for different lengths', () => {
    expect(arraysEqual(['a'], ['a', 'b'])).toBe(false);
  });

  it('returns false for different elements', () => {
    expect(arraysEqual(['a', 'b'], ['a', 'c'])).toBe(false);
  });
});

describe('deepEqual', () => {
  it('returns true for equal objects', () => {
    expect(deepEqual({ a: 1, b: 'x' }, { a: 1, b: 'x' })).toBe(true);
  });

  it('returns false for different objects', () => {
    expect(deepEqual({ a: 1 }, { a: 2 })).toBe(false);
  });

  it('returns true for nested objects', () => {
    expect(deepEqual({ a: { b: [1, 2] } }, { a: { b: [1, 2] } })).toBe(true);
  });

  it('handles null equality', () => {
    expect(deepEqual(null, null)).toBe(true);
    expect(deepEqual(null, {})).toBe(false);
  });
});

describe('asyncCheck with async condition handler inside $or/$not', () => {
  afterEach(() => {
    // clean up any test-registered conditions
  });

  it('correctly evaluates a Promise-returning condition handler inside $or via asyncCheck()', async () => {
    const asyncHandler: ACLConditionHandler = {
      evaluate: (_value: unknown, context: Context): Promise<boolean> => {
        return Promise.resolve(context.traceId === 'allowed-trace');
      },
    };
    ACL.registerCondition('async_trace_check', asyncHandler);

    const acl = new ACL([
      {
        callers: ['agent.*'],
        targets: ['*.resource'],
        effect: 'allow',
        description: 'allow if any $or branch passes',
        conditions: {
          $or: [
            { async_trace_check: true },
          ],
        },
      },
    ]);

    const ctx = new Context('allowed-trace', null, [], null, null);
    const result = await acl.asyncCheck('agent.foo', 'my.resource', ctx);
    expect(result).toBe(true);

    const ctxDenied = new Context('other-trace', null, [], null, null);
    const resultDenied = await acl.asyncCheck('agent.foo', 'my.resource', ctxDenied);
    expect(resultDenied).toBe(false);
  });

  it('correctly evaluates a Promise-returning condition inside $not via asyncCheck()', async () => {
    const asyncBlockedHandler: ACLConditionHandler = {
      evaluate: (_value: unknown, context: Context): Promise<boolean> => {
        return Promise.resolve(context.traceId === 'blocked-trace');
      },
    };
    ACL.registerCondition('async_blocked_check', asyncBlockedHandler);

    const acl = new ACL([
      {
        callers: ['agent.*'],
        targets: ['*.resource'],
        effect: 'allow',
        description: 'allow unless blocked',
        conditions: {
          $not: { async_blocked_check: true },
        },
      },
    ]);

    const ctxAllowed = new Context('other-trace', null, [], null, null);
    expect(await acl.asyncCheck('agent.foo', 'my.resource', ctxAllowed)).toBe(true);

    const ctxBlocked = new Context('blocked-trace', null, [], null, null);
    expect(await acl.asyncCheck('agent.foo', 'my.resource', ctxBlocked)).toBe(false);
  });
});

describe('malformed conditions fail closed (sync finding A-D-009)', () => {
  // Regression: `_parseAclRule` took `conditions` with a bare `as` cast, so a scalar
  // reached the gate. `Object.entries(5)` is `[]`, the AND-loop was vacuously
  // satisfied, and an `effect: allow` rule became UNCONDITIONAL — the one direction
  // a default-deny ACL must never fail in.
  const ctx = new Context(
    'trace-malformed',
    'caller.a',
    [],
    null,
    createIdentity('u1', 'user', ['admin']),
  );

  // Values with no own enumerable properties: these are the fail-open vector.
  for (const bad of [5, 0, 1.5, true, false] as unknown[]) {
    it(`denies an allow-rule whose conditions is ${JSON.stringify(bad)}`, () => {
      const acl = new ACL(
        [{ callers: ['caller.a'], targets: ['target.b'], effect: 'allow', description: '', conditions: bad as never }],
        'deny',
      );
      expect(acl.check('caller.a', 'target.b', ctx)).toBe(false);
    });
  }

  // Strings and arrays already denied (their Object.entries yields index keys →
  // unknown condition), but pin them so the guard cannot regress into allowing them.
  for (const bad of ['abc', [{ roles: ['admin'] }]] as unknown[]) {
    it(`denies an allow-rule whose conditions is ${JSON.stringify(bad)}`, () => {
      const acl = new ACL(
        [{ callers: ['caller.a'], targets: ['target.b'], effect: 'allow', description: '', conditions: bad as never }],
        'deny',
      );
      expect(acl.check('caller.a', 'target.b', ctx)).toBe(false);
    });
  }

  it('still treats an empty mapping as vacuously satisfied', () => {
    const acl = new ACL(
      [{ callers: ['caller.a'], targets: ['target.b'], effect: 'allow', description: '', conditions: {} }],
      'deny',
    );
    expect(acl.check('caller.a', 'target.b', ctx)).toBe(true);
  });

  it('records the malformed shape as a handler_error on the audit entry', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL(
      [{ callers: ['caller.a'], targets: ['target.b'], effect: 'allow', description: '', conditions: 5 as never }],
      'deny',
      (e) => entries.push(e),
    );
    acl.check('caller.a', 'target.b', ctx);
    expect(entries).toHaveLength(1);
    expect(entries[0].handlerError).toMatch(/must be a mapping, got number/);
  });

  it('rejects a malformed conditions at YAML parse time', () => {
    expect(() =>
      _parseAclRule({ callers: ['a'], targets: ['b'], effect: 'allow', conditions: 5 }, 0),
    ).toThrow(ACLRuleError);
  });
});
