import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { ACL, Context, type ACLRule } from '../src/index.js';
import { findFixturesRoot } from './spec-repo.js';

interface DelegationCase {
  readonly id: string;
  readonly input: {
    readonly identity: Record<string, unknown>;
    readonly rules?: Omit<ACLRule, 'description'>[];
    readonly default_effect?: 'allow' | 'deny';
  };
  readonly expected: {
    readonly valid: boolean;
    readonly depth?: number;
    readonly subject_id?: string;
    readonly effect?: 'allow' | 'deny';
  };
}

const fixture = JSON.parse(readFileSync(join(findFixturesRoot(), 'identity_delegation.json'), 'utf8')) as {
  readonly test_cases: readonly DelegationCase[];
};

describe('identity_delegation.json', () => {
  it.each(fixture.test_cases)('$id', async ({ input, expected }) => {
    const restore = (): Context => Context.fromJSON({ identity: input.identity });
    if (!expected.valid) {
      expect(restore).toThrow(TypeError);
      return;
    }
    const context = restore();
    expect(context.identity?.delegationDepth).toBe(expected.depth);
    expect(context.identity?.rootSubject.id).toBe(expected.subject_id);
    if (input.rules !== undefined) {
      const acl = new ACL(input.rules.map(rule => ({ description: '', ...rule })), input.default_effect);
      expect(acl.checkAccess(null, 'delegation.target', context).access).toBe(expected.effect);
      expect((await acl.asyncCheckAccess(null, 'delegation.target', context)).access).toBe(expected.effect);
      expect(acl.evaluateVisibility(null, 'delegation.target', context).visible).toBe(expected.effect === 'allow');
    }
  });
});
