/**
 * D-88 (spec v1.49.0) — the §6.5 warning dedupe is keyed by rule INDEX, so any
 * operation that inserts, removes or reorders rules must clear it.
 *
 * This SDK was the decision's AUTHORITY and had no test for the §6.5 warning at
 * all. The decision's status quo recorded that TypeScript "cleared the dedupe
 * set in both `addRule` and `reload`", and the decision generalised to *any*
 * operation that shifts indices — apcore-python and apcore-rust implemented the
 * generalisation, this SDK kept the two sites it already had, and `removeRule`
 * shipped without the clear. Measured, not inferred: before the fix, the
 * surviving rule's warning was suppressed by the REMOVED rule's marker.
 *
 * What makes each test RED: removing the corresponding
 * `_warnedMissingContext.clear()`. The index-collision shape is essential — a
 * test whose surviving rule lands on an index nobody had warned about passes
 * with or without the clear, which is how apcore-rust's `remove_rule` test read
 * as coverage while proving nothing.
 */

import { describe, it, expect, vi } from 'vitest';
import { ACL } from '../src/acl.js';
import type { ACLRule } from '../src/acl.js';

/** A conditional rule matching every target, so every rule is evaluated. */
function conditional(role: string): ACLRule {
  return {
    callers: ['*'],
    targets: ['*'],
    effect: 'deny',
    conditions: { roles: [role] },
    description: `conditional deny for ${role}`,
  };
}

/** Count §6.5 warnings emitted while `f` runs. */
function countWarnings(f: () => void): number {
  const spy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  try {
    f();
    return spy.mock.calls.filter((c) => String(c[0]).includes('supplied no Context')).length;
  } finally {
    spy.mockRestore();
  }
}

describe('D-88: the §6.5 dedupe is cleared on an index shift', () => {
  it('the warning is deduped when nothing has moved', () => {
    // The precondition for every test below: dedupe is genuinely on, so a
    // second warning means a marker was dropped rather than never recorded.
    const acl = new ACL([conditional('admin')], 'allow');
    expect(
      countWarnings(() => {
        acl.check('caller', 'target');
        acl.check('caller', 'target');
      }),
    ).toBe(1);
  });

  it('addRule clears it', () => {
    const acl = new ACL([conditional('admin')], 'allow');
    expect(countWarnings(() => acl.check('caller', 'target'))).toBe(1);

    acl.addRule(conditional('operator'));
    // Two rules, two indices, and neither is suppressed by the marker the
    // pre-insertion rule left at index 0.
    expect(countWarnings(() => acl.check('caller', 'target'))).toBe(2);
  });

  it('removeRule clears it', () => {
    const acl = new ACL([conditional('admin'), conditional('operator')], 'allow');
    expect(countWarnings(() => acl.check('caller', 'target'))).toBe(2);

    expect(acl.removeRule(['*'], ['*'], { roles: ['admin'] })).toBe(true);
    // The surviving rule moved from index 1 to index 0. Without the clear, the
    // marker the REMOVED rule left at index 0 silences it.
    expect(countWarnings(() => acl.check('caller', 'target'))).toBe(1);
  });

  it('a removeRule that removed nothing does not have to clear', () => {
    // No rule removed means no index shifted, so the dedupe still holds. This
    // separates "clears correctly" from "clears unconditionally", which would
    // re-warn on every failed lookup and is the spam D-88 exists to bound.
    const acl = new ACL([conditional('admin')], 'allow');
    expect(countWarnings(() => acl.check('caller', 'target'))).toBe(1);

    expect(acl.removeRule(['nobody'], ['*'])).toBe(false);
    expect(countWarnings(() => acl.check('caller', 'target'))).toBe(0);
  });
});
