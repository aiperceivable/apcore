import { describe, it, expect } from 'vitest';
import { ACL } from '../src/acl.js';
import type { ACLRule, AuditEntry } from '../src/acl.js';
import { Context, createIdentity } from '../src/context.js';

describe('ACL AuditEntry', () => {
  const allowAll: ACLRule = {
    callers: ['*'],
    targets: ['*'],
    effect: 'allow',
    description: 'Allow all',
  };

  const denyExternal: ACLRule = {
    callers: ['@external'],
    targets: ['admin.*'],
    effect: 'deny',
    description: 'Deny external to admin',
  };

  it('emits audit entry on rule match', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL([allowAll], 'deny', (entry) => entries.push(entry));

    acl.check('module.a', 'module.b');

    expect(entries).toHaveLength(1);
    expect(entries[0].decision).toBe('allow');
    expect(entries[0].reason).toBe('rule_match');
    expect(entries[0].matchedRule).toBe('Allow all');
    expect(entries[0].matchedRuleIndex).toBe(0);
  });

  it('emits audit entry on default effect', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL([denyExternal], 'allow', (entry) => entries.push(entry));

    acl.check('module.a', 'module.b'); // No rule matches, default allow

    expect(entries).toHaveLength(1);
    expect(entries[0].decision).toBe('allow');
    expect(entries[0].reason).toBe('default_effect');
    expect(entries[0].matchedRule).toBeNull();
  });

  it('emits no_rules reason when no rules exist', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL([], 'deny', (entry) => entries.push(entry));

    acl.check('module.a', 'module.b');

    expect(entries).toHaveLength(1);
    expect(entries[0].reason).toBe('no_rules');
  });

  it('includes context information in audit entry', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL([allowAll], 'deny', (entry) => entries.push(entry));
    const identity = createIdentity('user-1', 'user', ['admin']);
    const ctx = new Context('trace-abc', 'caller', ['caller', 'target'], null, identity);

    acl.check('caller', 'target', ctx);

    const entry = entries[0];
    expect(entry.traceId).toBe('trace-abc');
    expect(entry.callDepth).toBe(2);
    expect(entry.identityType).toBe('user');
    expect(entry.roles).toEqual(['admin']);
  });

  it('handles null context gracefully', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL([allowAll], 'deny', (entry) => entries.push(entry));

    acl.check('module.a', 'module.b', null);

    const entry = entries[0];
    expect(entry.traceId).toBeNull();
    expect(entry.callDepth).toBeNull();
    expect(entry.identityType).toBeNull();
    expect(entry.roles).toEqual([]);
  });

  it('does not emit when no audit logger set', () => {
    const acl = new ACL([allowAll], 'deny');
    // Should not throw
    acl.check('module.a', 'module.b');
  });

  it('has ISO 8601 timestamp', () => {
    const entries: AuditEntry[] = [];
    const acl = new ACL([allowAll], 'deny', (entry) => entries.push(entry));
    acl.check('module.a', 'module.b');
    expect(entries[0].timestamp).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });
});
