/**
 * Cross-language conformance driver for acl_agent_scoping.json (Issue #72).
 *
 * Fixture source: apcore/conformance/fixtures/acl_agent_scoping.json
 * (single source of truth). See that fixture's `description` for the contract.
 *
 * One canonical default-deny ruleset (shared `default_effect` + `rules`) scopes
 * AI-agent tool access by caller pattern + identity roles + call-chain depth.
 * Each case is a (caller_id, caller_identity, call_depth, target_id) -> expected
 * access decision. This locks the agent-governance scenario as a cross-language
 * contract: all SDKs MUST produce identical decisions.
 *
 * Machinery mirrors the `acl_evaluation` block in tests/conformance.test.ts:
 * the shared rules/default_effect build a single ACL; per case a Context carries
 * the caller_identity (type+roles) and a call chain of length call_depth, and
 * acl.check(caller_id, target_id, ctx) must equal `expected`.
 *
 * Depth semantics: `max_call_depth` is "must not exceed" and inclusive —
 * call_depth == max is allowed, call_depth > max is denied (MaxCallDepthHandler
 * uses `<=`).
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { ACL, ACLRule } from '../src/acl.js';
import { Context, createIdentity } from '../src/context.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

function loadFixture(name: string): any {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, `${name}.json`), 'utf-8'));
}

describe('Conformance: ACL agent-tool-governance scoping (Issue #72)', () => {
  const fixture = loadFixture('acl_agent_scoping');

  // Build ONE ACL from the shared default_effect + rules; every case reuses it.
  const rules: ACLRule[] = fixture.rules.map((r: any) => ({
    callers: r.callers,
    targets: r.targets,
    effect: r.effect,
    description: r.description || '',
    conditions: r.conditions || null,
  }));
  const acl = new ACL(rules, fixture.default_effect);

  fixture.test_cases.forEach((tc: any) => {
    it(tc.id, () => {
      // Build the evaluation context only when the case provides identity or a
      // non-zero call depth (mirrors the acl_evaluation runner). External
      // callers (caller_id == null) carry no identity and no chain.
      let ctx: Context | null = null;
      const needsContext = tc.caller_identity != null || (tc.call_depth ?? 0) > 0;

      if (needsContext) {
        const identity = tc.caller_identity
          ? createIdentity(
              tc.caller_id || 'unknown',
              tc.caller_identity.type,
              tc.caller_identity.roles || [],
            )
          : null;
        const callChain = Array(tc.call_depth || 0)
          .fill(0)
          .map((_: unknown, i: number) => `_depth_${i}`);
        ctx = new Context('trace-id', tc.caller_id, callChain, null, identity);
      }

      expect(acl.check(tc.caller_id, tc.target_id, ctx)).toBe(tc.expected);
    });
  });
});
