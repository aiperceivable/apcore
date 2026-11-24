/**
 * A rule's `effect` value set is closed, at every entry point —
 * PROTOCOL_SPEC §6.1.5 with §6.1.6 rule 3 (spec v1.30.0, apcore#111).
 *
 * The defect this pins: the check existed and was reachable from one of three
 * doors. `ACL.load()` rejected `effect: "Allow"` — the capitalisation an
 * operator writes by hand — while `new ACL([...])` and `addRule()` accepted it,
 * and the accepted value was then read as `deny` at check time, because every
 * decision site resolved "not `allow`" to a denial. Under
 * `defaultEffect: 'allow'` that turns a rule written to PERMIT into one that
 * denies everything it matches, with no error and nothing from
 * `validateRules()`.
 *
 * Deliberately fixture-independent: these run on a checkout with no apcore spec
 * repo beside it, so the closure stays pinned even where
 * `tests/conformance-acl-effect-value-closure.test.ts` skips. The cross-language
 * contract lives there.
 */

import { describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { ACL } from '../src/acl.js';
import type { ACLRule } from '../src/acl.js';
import { ACLRuleError } from '../src/errors.js';
// Side-effect import installs the Node-side YAML loader onto ACL.load.
import '../src/acl-file.js';

function writeAcl(body: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'acl-effect-'));
  const file = path.join(dir, 'acl.yaml');
  fs.writeFileSync(file, body, 'utf-8');
  return file;
}

/**
 * Build a rule with an arbitrary `effect`.
 *
 * `ACLRule.effect` is declared `string` (see the field's doc comment), so no
 * cast is needed to express the illegal values — which is the whole reason the
 * guarantee has to be a runtime check.
 */
function ruleWithEffect(effect: string, approval?: 'required' | 'not_required'): ACLRule {
  return {
    callers: ['agent.*'],
    targets: ['orders.*'],
    effect,
    description: '',
    conditions: null,
    ...(approval === undefined ? {} : { approval }),
  };
}

function aclYaml(effect: string, defaultEffect = 'deny'): string {
  return (
    `default_effect: ${defaultEffect}\n` +
    'rules:\n  - callers: ["agent.*"]\n    targets: ["orders.*"]\n' +
    `    effect: ${JSON.stringify(effect)}\n`
  );
}

describe("§6.1.5 a rule's effect value set is closed at every entry point", () => {
  // The door that already worked. Kept so a regression here is told apart from
  // a regression in the two that did not.
  it('rejects an out-of-enum effect at load, naming the rule index and the value', () => {
    const file = writeAcl(aclYaml('Allow'));
    expect(() => ACL.load(file)).toThrow(ACLRuleError);
    expect(() => ACL.load(file)).toThrow(/Rule 0 has invalid effect 'Allow'/);
  });

  it('rejects an out-of-enum effect on direct construction', () => {
    // The defect: this used to CONSTRUCT, from a configuration §6.1's field
    // table and acl-config.schema.json both already called invalid.
    expect(() => new ACL([ruleWithEffect('Allow')], 'deny')).toThrow(ACLRuleError);
    expect(() => new ACL([ruleWithEffect('Allow')], 'deny')).toThrow(
      /Rule 0 has invalid effect 'Allow'/,
    );
  });

  it('rejects an out-of-enum effect on runtime insertion, and does not insert it', () => {
    const acl = new ACL([], 'deny');
    expect(() => acl.addRule(ruleWithEffect('Allow'))).toThrow(ACLRuleError);
    // §6.1.6 rule 3: `addRule` returns nothing, which is not an exemption —
    // throwing is how TypeScript signals an unconstructable value. The list is
    // untouched, so a caller that ignored the throw cannot end up enforcing it.
    expect(acl.rules.length).toBe(0);
  });

  it('names the offending rule index on construction, not always rule 0', () => {
    expect(
      () => new ACL([ruleWithEffect('allow'), ruleWithEffect('deny'), ruleWithEffect('alow')]),
    ).toThrow(/Rule 2 has invalid effect 'alow'/);
  });

  it('rejects the deny-side mistake for the same reason, not by accident', () => {
    // `DENY` would previously survive at two doors and then behave "correctly",
    // because a non-`allow` string lands on the branch that was going to deny
    // anyway. Correct-by-accident is the state §6.1.5 exists to end.
    const file = writeAcl(aclYaml('DENY'));
    expect(() => ACL.load(file)).toThrow(ACLRuleError);
    expect(() => new ACL([ruleWithEffect('DENY')])).toThrow(ACLRuleError);
    expect(() => new ACL([]).addRule(ruleWithEffect('DENY'))).toThrow(ACLRuleError);
  });

  it('rejects an empty effect — it is not a shorter way of saying deny', () => {
    expect(() => ACL.load(writeAcl(aclYaml('')))).toThrow(ACLRuleError);
    expect(() => new ACL([ruleWithEffect('')])).toThrow(ACLRuleError);
    expect(() => new ACL([]).addRule(ruleWithEffect(''))).toThrow(ACLRuleError);
  });

  it('rejects rather than flipping a permit rule into a blanket denial', () => {
    // The case where reading an unknown effect as `deny` is visibly WRONG and
    // not merely silent. Pre-fix, this ACL constructed and then answered `deny`
    // for `agent.planner -> orders.create` — a rule its author wrote to permit,
    // under a default that would have permitted the call anyway.
    expect(() => new ACL([ruleWithEffect('alow')], 'allow')).toThrow(ACLRuleError);
  });

  it('fails on the effect before the approval pairing, so the report is the real fault', () => {
    // §6.1.6 rule 2's check reads `effect` to decide whether the pair is the
    // meaningless one, so an out-of-enum effect must be caught first or it
    // slips past that rule's `!== 'deny'` early return.
    expect(() => new ACL([ruleWithEffect('Deny', 'required')])).toThrow(
      /Rule 0 has invalid effect 'Deny'/,
    );
  });

  it('still accepts the closed set itself at all three doors', () => {
    for (const effect of ['allow', 'deny']) {
      expect(ACL.load(writeAcl(aclYaml(effect))).rules.length).toBe(1);
      expect(new ACL([ruleWithEffect(effect)], 'deny').rules.length).toBe(1);
      const acl = new ACL([], 'deny');
      acl.addRule(ruleWithEffect(effect));
      expect(acl.rules.length).toBe(1);
    }
  });

  it('leaves a well-formed ACL deciding exactly as before', () => {
    // Backward compatibility: closing the value set changes nothing for a
    // configuration that was already legal.
    const acl = new ACL([ruleWithEffect('allow')], 'deny');
    expect(acl.check('agent.planner', 'orders.create')).toBe(true);
    expect(acl.check('agent.planner', 'billing.charge')).toBe(false);
  });
});

describe('§6.1.5 default_effect is closed on the same terms', () => {
  it('rejects an out-of-enum default_effect on construction, naming the value', () => {
    expect(() => new ACL([], 'Allow')).toThrow(ACLRuleError);
    expect(() => new ACL([], 'Allow')).toThrow(/Invalid default_effect 'Allow'/);
  });

  it('rejects an out-of-enum default_effect at load', () => {
    expect(() => ACL.load(writeAcl(aclYaml('allow', 'Allow')))).toThrow(ACLRuleError);
  });

  it('rejects an empty default_effect rather than falling back to deny', () => {
    // `?? 'deny'` in the loader is a NULLISH default, so the empty string is
    // carried through to the constructor and rejected there rather than
    // silently becoming the house rule.
    expect(() => ACL.load(writeAcl(aclYaml('allow', '""')))).toThrow(ACLRuleError);
    expect(() => new ACL([], '')).toThrow(ACLRuleError);
  });

  it('still accepts allow, which is legal and merely discouraged', () => {
    expect(new ACL([], 'allow').defaultEffect).toBe('allow');
    expect(ACL.load(writeAcl(aclYaml('allow', 'allow'))).defaultEffect).toBe('allow');
  });
});
