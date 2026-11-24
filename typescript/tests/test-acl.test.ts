import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { ACL } from '../src/acl.js';
import type { AuditEntry } from '../src/acl.js';
import type { ACLConditionHandler } from '../src/acl-handlers.js';
import { ACLRuleError, ConfigNotFoundError } from '../src/errors.js';
import { Context, createIdentity } from '../src/context.js';

function makeContext(opts: {
  callerId?: string | null;
  callChain?: string[];
  identityType?: string;
  roles?: string[];
} = {}): Context {
  const identity = opts.identityType
    ? createIdentity('test-user', opts.identityType, opts.roles ?? [])
    : null;
  return new Context(
    'trace-test',
    opts.callerId ?? null,
    opts.callChain ?? [],
    null,
    identity,
  );
}

describe('ACL', () => {
  it('allows access when allow rule matches', () => {
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: '' },
    ]);
    expect(acl.check('module.a', 'module.b')).toBe(true);
  });

  it('denies access when deny rule matches', () => {
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'deny', description: '' },
    ]);
    expect(acl.check('module.a', 'module.b')).toBe(false);
  });

  it('returns default deny when no rule matches', () => {
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: '' },
    ]);
    expect(acl.check('module.x', 'module.y')).toBe(false);
  });

  it('first-match-wins: deny before allow', () => {
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'deny', description: '' },
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: '' },
    ]);
    expect(acl.check('module.a', 'module.b')).toBe(false);
  });

  it('first-match-wins: allow before deny', () => {
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: '' },
      { callers: ['module.a'], targets: ['module.b'], effect: 'deny', description: '' },
    ]);
    expect(acl.check('module.a', 'module.b')).toBe(true);
  });

  it('default effect allow when no rules match', () => {
    const acl = new ACL([], 'allow');
    expect(acl.check('any', 'thing')).toBe(true);
  });

  it('maps null callerId to @external', () => {
    const acl = new ACL([
      { callers: ['@external'], targets: ['public.api'], effect: 'allow', description: '' },
    ]);
    expect(acl.check(null, 'public.api')).toBe(true);
  });

  it('does not match @external for real module caller', () => {
    const acl = new ACL([
      { callers: ['@external'], targets: ['public.api'], effect: 'allow', description: '' },
    ]);
    expect(acl.check('module.a', 'public.api')).toBe(false);
  });

  it('wildcard * matches all callers', () => {
    const acl = new ACL([
      { callers: ['*'], targets: ['public.api'], effect: 'allow', description: '' },
    ]);
    expect(acl.check('module.a', 'public.api')).toBe(true);
    expect(acl.check('module.b', 'public.api')).toBe(true);
  });

  it('wildcard * matches all targets', () => {
    const acl = new ACL([
      { callers: ['module.admin'], targets: ['*'], effect: 'allow', description: '' },
    ]);
    expect(acl.check('module.admin', 'anything')).toBe(true);
  });

  it('prefix wildcard matching', () => {
    const acl = new ACL([
      { callers: ['core.*'], targets: ['data.*'], effect: 'allow', description: '' },
    ]);
    expect(acl.check('core.auth', 'data.store')).toBe(true);
    expect(acl.check('other.x', 'data.y')).toBe(false);
  });

  it('@system matches system identity type', () => {
    const acl = new ACL([
      { callers: ['@system'], targets: ['*'], effect: 'allow', description: '' },
    ]);
    const ctx = makeContext({ identityType: 'system' });
    expect(acl.check('any.module', 'any.target', ctx)).toBe(true);
  });

  it('@system does not match non-system identity', () => {
    const acl = new ACL([
      { callers: ['@system'], targets: ['*'], effect: 'allow', description: '' },
    ]);
    const ctx = makeContext({ identityType: 'user' });
    expect(acl.check('any.module', 'any.target', ctx)).toBe(false);
  });

  it('conditions: identity_types allows matching type', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['admin'], effect: 'allow', description: '',
      conditions: { identity_types: ['admin'] },
    }]);
    const ctx = makeContext({ identityType: 'admin' });
    expect(acl.check('mod.a', 'admin', ctx)).toBe(true);
  });

  it('conditions: identity_types denies non-matching type', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['admin'], effect: 'allow', description: '',
      conditions: { identity_types: ['admin'] },
    }]);
    const ctx = makeContext({ identityType: 'user' });
    expect(acl.check('mod.a', 'admin', ctx)).toBe(false);
  });

  it('conditions: roles allows matching role', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['settings'], effect: 'allow', description: '',
      conditions: { roles: ['editor', 'admin'] },
    }]);
    const ctx = makeContext({ identityType: 'user', roles: ['editor'] });
    expect(acl.check('mod.a', 'settings', ctx)).toBe(true);
  });

  it('conditions: roles denies missing role', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['settings'], effect: 'allow', description: '',
      conditions: { roles: ['admin'] },
    }]);
    const ctx = makeContext({ identityType: 'user', roles: ['viewer'] });
    expect(acl.check('mod.a', 'settings', ctx)).toBe(false);
  });

  it('conditions: max_call_depth allows within limit', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['deep'], effect: 'allow', description: '',
      conditions: { max_call_depth: 3 },
    }]);
    const ctx = makeContext({ callChain: ['a', 'b'] });
    expect(acl.check('mod.a', 'deep', ctx)).toBe(true);
  });

  it('conditions: max_call_depth denies exceeding limit', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['deep'], effect: 'allow', description: '',
      conditions: { max_call_depth: 2 },
    }]);
    const ctx = makeContext({ callChain: ['a', 'b', 'c'] });
    expect(acl.check('mod.a', 'deep', ctx)).toBe(false);
  });

  it('conditions fail when no context provided', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['deep'], effect: 'allow', description: '',
      conditions: { max_call_depth: 5 },
    }]);
    expect(acl.check('mod.a', 'deep')).toBe(false);
  });

  it('addRule adds to highest priority', () => {
    const acl = new ACL([
      { callers: ['*'], targets: ['*'], effect: 'deny', description: '' },
    ]);
    expect(acl.check('mod.a', 'mod.b')).toBe(false);

    acl.addRule({ callers: ['mod.a'], targets: ['mod.b'], effect: 'allow', description: '' });
    expect(acl.check('mod.a', 'mod.b')).toBe(true);
  });

  it('removeRule removes matching rule', () => {
    const acl = new ACL([
      { callers: ['mod.a'], targets: ['mod.b'], effect: 'allow', description: '' },
    ]);
    expect(acl.check('mod.a', 'mod.b')).toBe(true);

    const removed = acl.removeRule(['mod.a'], ['mod.b']);
    expect(removed).toBe(true);
    expect(acl.check('mod.a', 'mod.b')).toBe(false);
  });

  it('removeRule returns false when no match', () => {
    const acl = new ACL([]);
    expect(acl.removeRule(['x'], ['y'])).toBe(false);
  });

  it('removeRule matches only when conditions parameter matches rule conditions', () => {
    const acl = new ACL([
      { callers: ['a'], targets: ['b'], effect: 'allow', description: '', conditions: { roles: ['admin'] } },
      { callers: ['a'], targets: ['b'], effect: 'deny', description: '', conditions: null },
    ]);
    // Should not remove when conditions differ
    const missedRemove = acl.removeRule(['a'], ['b'], { roles: ['user'] });
    expect(missedRemove).toBe(false);
    // Should remove the rule with matching conditions
    const removed = acl.removeRule(['a'], ['b'], { roles: ['admin'] });
    expect(removed).toBe(true);
    // The deny rule (null conditions) should remain
    expect(acl.check('a', 'b')).toBe(false);
  });

  it('removeRule with undefined conditions removes first caller+target match regardless of conditions', () => {
    const acl = new ACL([
      { callers: ['a'], targets: ['b'], effect: 'allow', description: '', conditions: { roles: ['admin'] } },
    ]);
    const removed = acl.removeRule(['a'], ['b']);
    expect(removed).toBe(true);
  });
});

describe('ACL.load', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'acl-test-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('loads valid ACL from a YAML file', () => {
    const yamlContent = `
rules:
  - callers: ["module.a"]
    targets: ["module.b"]
    effect: allow
    description: "allow a to b"
`;
    const filePath = join(tmpDir, 'acl.yaml');
    writeFileSync(filePath, yamlContent, 'utf-8');

    const acl = ACL.load(filePath);
    expect(acl.check('module.a', 'module.b')).toBe(true);
    expect(acl.check('module.x', 'module.y')).toBe(false);
  });

  it('loads ACL with custom default_effect from YAML', () => {
    const yamlContent = `
default_effect: allow
rules: []
`;
    const filePath = join(tmpDir, 'acl.yaml');
    writeFileSync(filePath, yamlContent, 'utf-8');

    const acl = ACL.load(filePath);
    expect(acl.check('any.caller', 'any.target')).toBe(true);
  });

  it('throws ACLRuleError for an explicit default_effect: null in YAML', () => {
    // An ABSENT `default_effect` key defaults to 'deny' (see the test above's
    // sibling with no key at all, and the "missing required 'rules' key" test
    // for a file with `default_effect` and no `rules`). An explicit `null` is
    // a different thing: the operator wrote a value, and `null` is not in the
    // 'allow' / 'deny' enum `schemas/acl-config.schema.json` declares — so it
    // must be rejected exactly like `default_effect: block`, not silently
    // normalized to 'deny' by a `?? 'deny'` fallback that cannot tell "absent"
    // from "present and null" apart.
    const yamlContent = `
default_effect: null
rules: []
`;
    const filePath = join(tmpDir, 'acl.yaml');
    writeFileSync(filePath, yamlContent, 'utf-8');

    expect(() => ACL.load(filePath)).toThrow(ACLRuleError);
    expect(() => ACL.load(filePath)).toThrow(/Invalid default_effect 'null'/);
  });

  it('throws ConfigNotFoundError for missing file', () => {
    const missingPath = join(tmpDir, 'nonexistent.yaml');
    expect(() => ACL.load(missingPath)).toThrow(ConfigNotFoundError);
  });

  it('throws ACLRuleError for invalid YAML syntax', () => {
    const filePath = join(tmpDir, 'bad.yaml');
    writeFileSync(filePath, ':\n  :\n    - [invalid', 'utf-8');

    expect(() => ACL.load(filePath)).toThrow(ACLRuleError);
  });

  it('throws ACLRuleError when YAML is not a mapping', () => {
    const filePath = join(tmpDir, 'array.yaml');
    writeFileSync(filePath, '- item1\n- item2\n', 'utf-8');

    expect(() => ACL.load(filePath)).toThrow(ACLRuleError);
    expect(() => ACL.load(filePath)).toThrow(/must be a mapping/);
  });

  it('throws ACLRuleError when YAML is a scalar', () => {
    const filePath = join(tmpDir, 'scalar.yaml');
    writeFileSync(filePath, 'just a string\n', 'utf-8');

    expect(() => ACL.load(filePath)).toThrow(ACLRuleError);
    expect(() => ACL.load(filePath)).toThrow(/must be a mapping/);
  });

  it('throws ACLRuleError when rules key is missing', () => {
    const filePath = join(tmpDir, 'norules.yaml');
    writeFileSync(filePath, 'default_effect: allow\n', 'utf-8');

    expect(() => ACL.load(filePath)).toThrow(ACLRuleError);
    expect(() => ACL.load(filePath)).toThrow(/missing required 'rules' key/);
  });

  it('throws ACLRuleError when rules is not an array', () => {
    const filePath = join(tmpDir, 'badrules.yaml');
    writeFileSync(filePath, 'rules: "not-a-list"\n', 'utf-8');

    expect(() => ACL.load(filePath)).toThrow(ACLRuleError);
    expect(() => ACL.load(filePath)).toThrow(/'rules' must be a list/);
  });

  it('loads ACL with multiple rules and conditions', () => {
    const yamlContent = `
rules:
  - callers: ["*"]
    targets: ["admin.panel"]
    effect: allow
    description: "admin access"
    conditions:
      roles: ["admin"]
  - callers: ["*"]
    targets: ["*"]
    effect: deny
    description: "deny all"
`;
    const filePath = join(tmpDir, 'multi.yaml');
    writeFileSync(filePath, yamlContent, 'utf-8');

    const acl = ACL.load(filePath);
    const adminCtx = makeContext({ identityType: 'user', roles: ['admin'] });
    const userCtx = makeContext({ identityType: 'user', roles: ['viewer'] });

    expect(acl.check('mod.a', 'admin.panel', adminCtx)).toBe(true);
    expect(acl.check('mod.a', 'admin.panel', userCtx)).toBe(false);
  });
});

describe('ACL.reload', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'acl-reload-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('reloads updated rules from the same YAML file', () => {
    const filePath = join(tmpDir, 'acl.yaml');
    writeFileSync(filePath, `
rules:
  - callers: ["module.a"]
    targets: ["module.b"]
    effect: deny
    description: "initial deny"
`, 'utf-8');

    const acl = ACL.load(filePath);
    expect(acl.check('module.a', 'module.b')).toBe(false);

    writeFileSync(filePath, `
rules:
  - callers: ["module.a"]
    targets: ["module.b"]
    effect: allow
    description: "updated allow"
`, 'utf-8');

    acl.reload();
    expect(acl.check('module.a', 'module.b')).toBe(true);
  });

  it('throws ACLRuleError when ACL was not loaded from a file', () => {
    const acl = new ACL([
      { callers: ['*'], targets: ['*'], effect: 'allow', description: '' },
    ]);

    expect(() => acl.reload()).toThrow(ACLRuleError);
    expect(() => acl.reload()).toThrow(/Cannot reload/);
  });
});

describe('ACL constructor validation', () => {
  it('throws ACLRuleError for invalid defaultEffect', () => {
    expect(() => new ACL([], 'block')).toThrow(ACLRuleError);
    expect(() => new ACL([], 'block')).toThrow(/Invalid default_effect/);
  });

  it('throws ACLRuleError for empty string defaultEffect', () => {
    expect(() => new ACL([], '')).toThrow(ACLRuleError);
  });

  it('throws ACLRuleError for an explicit null defaultEffect', () => {
    // A default *parameter* only substitutes for `undefined`, never for an
    // explicitly passed `null` — so `new ACL([], null as unknown as string)`
    // must reach `_rejectInvalidDefaultEffect(null)` unchanged, the same way
    // `ACL.load()` must for `default_effect: null` in a YAML file.
    expect(() => new ACL([], null as unknown as string)).toThrow(ACLRuleError);
    expect(() => new ACL([], null as unknown as string)).toThrow(/Invalid default_effect 'null'/);
  });
});

describe('ACL condition validation', () => {
  it('returns false when identity_types condition is not an array', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { identity_types: 'admin' },
    }]);
    const ctx = makeContext({ identityType: 'admin' });
    expect(acl.check('mod.a', 'target', ctx)).toBe(false);
  });

  it('returns false when roles condition is not an array', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { roles: 'admin' },
    }]);
    const ctx = makeContext({ identityType: 'user', roles: ['admin'] });
    expect(acl.check('mod.a', 'target', ctx)).toBe(false);
  });

  it('returns false when max_call_depth condition is not a number', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { max_call_depth: '5' },
    }]);
    const ctx = makeContext({ callChain: ['a'] });
    expect(acl.check('mod.a', 'target', ctx)).toBe(false);
  });

  it('supports max_call_depth with object { lte: N } format', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { max_call_depth: { lte: 3 } },
    }]);
    const ctxWithin = makeContext({ callChain: ['a', 'b'] });
    const ctxExceeds = makeContext({ callChain: ['a', 'b', 'c', 'd'] });
    expect(acl.check('mod.a', 'target', ctxWithin)).toBe(true);
    expect(acl.check('mod.a', 'target', ctxExceeds)).toBe(false);
  });

  it('returns false for async condition handler used in sync check()', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const asyncHandler: ACLConditionHandler = {
      evaluate: () => Promise.resolve(true) as unknown as boolean,
    };
    ACL.registerCondition('test_async_in_sync', asyncHandler);
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { test_async_in_sync: true },
    }]);
    const ctx = makeContext({ callChain: [] });
    expect(acl.check('module.a', 'target', ctx)).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('Async condition'));
    warnSpy.mockRestore();
    (ACL as any).conditionHandlers.delete('test_async_in_sync');
  });

  it('throws ACLRuleError when targets is not an array in parseAclRule (via load)', () => {
    const tmpDir = mkdtempSync(join(tmpdir(), 'acl-targets-test-'));
    const filePath = join(tmpDir, 'acl.yaml');
    writeFileSync(filePath, 'rules:\n  - callers: ["*"]\n    targets: "not_a_list"\n    effect: allow\n');
    try {
      expect(() => ACL.load(filePath)).toThrow(ACLRuleError);
    } finally {
      rmSync(tmpDir, { recursive: true });
    }
  });

  it('returns false for roles condition when identity is null', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { roles: ['admin'] },
    }]);
    const ctx = makeContext({});
    expect(acl.check('mod.a', 'target', ctx)).toBe(false);
  });

  it('returns false for identity_types condition when identity is null', () => {
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { identity_types: ['admin'] },
    }]);
    const ctx = makeContext({});
    expect(acl.check('mod.a', 'target', ctx)).toBe(false);
  });

  it('calls auditLogger when rule matches in check()', () => {
    const entries: unknown[] = [];
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: 'allow a->b' },
    ], 'deny', (entry) => entries.push(entry));
    expect(acl.check('module.a', 'module.b')).toBe(true);
    expect(entries).toHaveLength(1);
    expect((entries[0] as Record<string, unknown>)['decision']).toBe('allow');
    expect((entries[0] as Record<string, unknown>)['reason']).toBe('rule_match');
  });

  it('calls auditLogger with default_effect reason when no rules match in check()', () => {
    const entries: unknown[] = [];
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: '' },
    ], 'deny', (entry) => entries.push(entry));
    expect(acl.check('other.caller', 'other.target')).toBe(false);
    expect(entries).toHaveLength(1);
    expect((entries[0] as Record<string, unknown>)['reason']).toBe('default_effect');
  });

  it('calls auditLogger with no_rules reason when rule list is empty in check()', () => {
    const entries: unknown[] = [];
    const acl = new ACL([], 'deny', (entry) => entries.push(entry));
    expect(acl.check('module.a', 'module.b')).toBe(false);
    expect(entries).toHaveLength(1);
    expect((entries[0] as Record<string, unknown>)['reason']).toBe('no_rules');
  });

  it('returns false when sync condition handler throws', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const throwingHandler: ACLConditionHandler = {
      evaluate: () => { throw new Error('sync handler error'); },
    };
    ACL.registerCondition('test_throwing_sync', throwingHandler);
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { test_throwing_sync: true },
    }]);
    const ctx = makeContext({ callChain: [] });
    expect(acl.check('module.a', 'target', ctx)).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('threw:'));
    warnSpy.mockRestore();
    (ACL as any).conditionHandlers.delete('test_throwing_sync');
  });

  // Regression: sync finding A-D-026 — handlerError must be populated in AuditEntry
  // when a condition handler throws. Parity with apcore-python's contextvar-based
  // handler_error capture.
  it('populates AuditEntry.handlerError when sync condition handler throws', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const entries: AuditEntry[] = [];
    const throwingHandler: ACLConditionHandler = {
      evaluate: () => { throw new Error('boom in sync handler'); },
    };
    ACL.registerCondition('test_throwing_audited', throwingHandler);
    const acl = new ACL(
      [{ callers: ['*'], targets: ['target'], effect: 'allow', description: '',
         conditions: { test_throwing_audited: true } }],
      'deny',
      (entry) => entries.push(entry),
    );
    const ctx = makeContext({ callChain: [] });
    expect(acl.check('module.a', 'target', ctx)).toBe(false);
    expect(entries).toHaveLength(1);
    expect(entries[0].handlerError).toContain('test_throwing_audited');
    expect(entries[0].handlerError).toContain('boom in sync handler');
    warnSpy.mockRestore();
    (ACL as any).conditionHandlers.delete('test_throwing_audited');
  });
});

describe('ACL.asyncCheck', () => {
  it('allows access when allow rule matches', async () => {
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: '' },
    ]);
    expect(await acl.asyncCheck('module.a', 'module.b')).toBe(true);
  });

  it('denies access when deny rule matches', async () => {
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'deny', description: '' },
    ]);
    expect(await acl.asyncCheck('module.a', 'module.b')).toBe(false);
  });

  it('returns default deny when no rule matches', async () => {
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: '' },
    ]);
    expect(await acl.asyncCheck('other.x', 'other.y')).toBe(false);
  });

  it('handles null callerId as @external', async () => {
    const acl = new ACL([
      { callers: ['@external'], targets: ['module.b'], effect: 'allow', description: '' },
    ]);
    expect(await acl.asyncCheck(null, 'module.b')).toBe(true);
  });

  it('calls auditLogger with rule_match when rule matches', async () => {
    const entries: unknown[] = [];
    const acl = new ACL([
      { callers: ['module.a'], targets: ['module.b'], effect: 'allow', description: 'allow a->b' },
    ], 'deny', (entry) => entries.push(entry));
    expect(await acl.asyncCheck('module.a', 'module.b')).toBe(true);
    expect(entries).toHaveLength(1);
    expect((entries[0] as Record<string, unknown>)['reason']).toBe('rule_match');
  });

  it('calls auditLogger with default_effect when no rule matches', async () => {
    const entries: unknown[] = [];
    const acl = new ACL([], 'allow', (entry) => entries.push(entry));
    expect(await acl.asyncCheck('module.a', 'module.b')).toBe(true);
    expect(entries).toHaveLength(1);
    expect((entries[0] as Record<string, unknown>)['reason']).toBe('no_rules');
  });

  it('supports $or compound operator via asyncCheck', async () => {
    const acl = new ACL([
      { callers: ['$or', 'module.x', 'module.y'], targets: ['target.z'], effect: 'allow', description: '' },
    ]);
    expect(await acl.asyncCheck('module.x', 'target.z')).toBe(true);
    expect(await acl.asyncCheck('module.y', 'target.z')).toBe(true);
    expect(await acl.asyncCheck('module.other', 'target.z')).toBe(false);
  });

  it('supports $not compound operator via asyncCheck', async () => {
    const acl = new ACL([
      { callers: ['$not', 'module.blocked'], targets: ['target.z'], effect: 'allow', description: '' },
    ]);
    expect(await acl.asyncCheck('module.other', 'target.z')).toBe(true);
    expect(await acl.asyncCheck('module.blocked', 'target.z')).toBe(false);
  });

  it('returns false for unknown condition in asyncCheck', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { unknown_condition_xyz: true },
    }]);
    const ctx = makeContext({ callChain: [] });
    expect(await acl.asyncCheck('module.a', 'target', ctx)).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('Unknown ACL condition'));
    warnSpy.mockRestore();
  });

  it('returns false when async condition handler throws', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const throwingHandler: ACLConditionHandler = {
      evaluate: () => { throw new Error('handler error'); },
    };
    ACL.registerCondition('test_throwing_async', throwingHandler);
    const acl = new ACL([{
      callers: ['*'], targets: ['target'], effect: 'allow', description: '',
      conditions: { test_throwing_async: true },
    }]);
    const ctx = makeContext({ callChain: [] });
    expect(await acl.asyncCheck('module.a', 'target', ctx)).toBe(false);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('threw:'));
    warnSpy.mockRestore();
    // Cleanup: unregister the test handler
    (ACL as any).conditionHandlers.delete('test_throwing_async');
  });
});
