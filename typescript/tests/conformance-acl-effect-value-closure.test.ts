/**
 * Cross-language driver for `acl_effect_value_closure.json`.
 *
 * PROTOCOL_SPEC §6.1.5 with §6.1.6 rule 3 (spec v1.30.0, #111): a rule's
 * `effect` is a CLOSED value set — `allow` and `deny` and nothing else — and a
 * rule carrying anything else is rejected with `ACLRuleError` naming the rule
 * index and the offending value.
 *
 * **The entry point is the substance of this fixture, not a detail.** #107
 * closed the rule KEY set; this closes a legal key's VALUE, and it was found
 * because the check already existed and was reachable from only one of three
 * doors: `ACL.load()` rejected `effect: "Allow"` while `new ACL([...])` and
 * `addRule()` accepted it. So every case runs against EACH door the fixture
 * lists — `expected_load: "ok"` means accepted at all of them, `"reject"` means
 * rejected at all of them. There is deliberately no per-door expectation in the
 * fixture, because a value legal through one door and illegal through another
 * is precisely the defect.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { ACL } from '../src/acl.js';
import type { ACLRule } from '../src/acl.js';
import { ACLRuleError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURE = 'acl_effect_value_closure.json';
const FIXTURE_PATH = path.join(findFixturesRoot(), FIXTURE);
const PRESENT = fs.existsSync(FIXTURE_PATH);

// The fixture lands in the spec repo one push after this driver, so that
// `check_driver_coverage.py --strict` has a driver to find for it. Until then
// the suite skips and names the unexercised fixture — "not verified", never
// "passed".
const describeIfPresent = PRESENT ? describe : describe.skip;

/** The three doors §6.1.6 rule 3 names. This SDK exposes all three. */
const DOORS = ['load', 'construct', 'add_rule'] as const;
type Door = (typeof DOORS)[number];

/**
 * The closed set itself, spelled here rather than imported from `src/acl.ts`.
 *
 * The fixture carries no `legal_effects` list to read it from, and reading the
 * SDK's own would let a divergent set agree with itself. Two values, fixed by
 * §6.1's field table since long before v1.30.0 — what changed in v1.30.0 is
 * where the set is enforced, not what is in it.
 */
const LEGAL = new Set<string>(['allow', 'deny']);

interface Case {
  id: string;
  note: string;
  rule: Record<string, unknown>;
  default_effect: string;
  /** Absent in a fixture predating v1.30.0 — the load door was the only one. */
  entry_points?: string[];
  expected_load: 'ok' | 'reject';
}

const fixture = PRESENT
  ? (JSON.parse(fs.readFileSync(FIXTURE_PATH, 'utf-8')) as { test_cases: Case[] })
  : { test_cases: [] as Case[] };

/** Minimal YAML emitter — the cases are flat maps of scalars and string lists. */
function toYaml(value: Record<string, unknown>, indent: number): string {
  const pad = ' '.repeat(indent);
  return Object.entries(value)
    .map(([k, v]) => `${pad}${k}: ${JSON.stringify(v)}`)
    .join('\n');
}

function writeAclFile(rule: Record<string, unknown>, defaultEffect: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'acl-effect-closure-'));
  const file = path.join(dir, 'acl.yaml');
  fs.writeFileSync(
    file,
    `default_effect: ${JSON.stringify(defaultEffect)}\nrules:\n  - ${toYaml(rule, 4).trimStart()}\n`,
    'utf-8',
  );
  return file;
}

/**
 * Push the case's rule through one door.
 *
 * Returns the resulting ACL so the caller can assert the rule actually landed;
 * throws whatever the door throws, unwrapped, so the rejection assertions see
 * the real error rather than a wrapper.
 */
function openDoor(door: Door, tc: Case): ACL {
  const rule = tc.rule as unknown as ACLRule;
  switch (door) {
    case 'load':
      return ACL.load(writeAclFile(tc.rule, tc.default_effect));
    case 'construct':
      return new ACL([rule], tc.default_effect);
    case 'add_rule': {
      // `add_rule` accepts a rule and no default_effect, which is why the
      // fixture does not list this door on its `default_effect` cases. Build the
      // base ACL with the case's default when it is legal so the door is
      // exercised in the case's own conditions, and with the house rule when it
      // is not — the base ACL is scaffolding here, never the thing under test.
      const base = LEGAL.has(tc.default_effect) ? tc.default_effect : 'deny';
      const acl = new ACL([], base);
      acl.addRule(rule);
      return acl;
    }
  }
}

describeIfPresent("Conformance: a rule's effect value set is closed (§6.1.5, spec v1.30.0)", () => {
  it('names only entry points this driver actually exercises', () => {
    // A fixture door left unimplemented would silently narrow the contract to
    // the doors that happen to be covered — which is the shape of the defect
    // itself. Fail loudly instead of skipping it.
    const named = new Set(fixture.test_cases.flatMap((tc) => tc.entry_points ?? ['load']));
    for (const door of named) {
      expect(DOORS as readonly string[], `fixture names an unexercised door '${door}'`).toContain(
        door,
      );
    }
  });

  fixture.test_cases.forEach((tc) => {
    it(tc.id, () => {
      const doors = (tc.entry_points ?? ['load']) as Door[];
      expect(doors.length, `${tc.id} lists no entry point`).toBeGreaterThan(0);

      // Which field the case is about. A rule's `effect` is reported with its
      // index (§6.1.5); `default_effect` is not a rule, so it is not.
      const ruleEffect = String(tc.rule['effect']);
      const ruleEffectInvalid = !LEGAL.has(ruleEffect);
      const offending = ruleEffectInvalid ? ruleEffect : tc.default_effect;

      for (const door of doors) {
        if (tc.expected_load === 'ok') {
          const acl = openDoor(door, tc);
          expect(acl.rules.length, `${door}: ${tc.note}`).toBe(1);
          expect(acl.rules[0].effect, `${door}: ${tc.note}`).toBe(ruleEffect);
          continue;
        }

        let message = '';
        expect(() => {
          try {
            openDoor(door, tc);
          } catch (e) {
            message = e instanceof Error ? e.message : String(e);
            expect(e, `${door}: ${tc.note}`).toBeInstanceOf(ACLRuleError);
            throw e;
          }
        }, `${door}: ${tc.note}`).toThrow();

        // The message names the offending value, so the operator can find the
        // line they mistyped. An empty offending value has nothing to look for.
        if (offending !== '') {
          expect(message, `${door}: message did not name '${offending}': ${message}`).toContain(
            offending,
          );
        }
        // ...and the legal pair, so they know what to write instead.
        expect(message, `${door}: ${message}`).toContain('allow');
        expect(message, `${door}: ${message}`).toContain('deny');
        // A rule fault names the rule index. Every case carries one rule, so
        // that index is 0 at all three doors — `add_rule` inserts at the head.
        if (ruleEffectInvalid) {
          expect(message, `${door}: message did not name the rule index: ${message}`).toContain(
            'Rule 0',
          );
        }
      }
    });
  });
});
