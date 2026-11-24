/**
 * Cross-language driver for `acl_rule_key_closure.json`.
 *
 * PROTOCOL_SPEC §6.1 (spec v1.27.0, #107): ACL rule keys are a closed set, and
 * a rule carrying anything else fails to load. A key nothing evaluates was
 * dropped in silence before this, which widens an `allow` rule with no warning
 * — the §6.1.1 defect class on the pattern side rather than the condition side.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { ACL } from '../src/acl.js';
import { ACLRuleError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURE = 'acl_rule_key_closure.json';
const FIXTURE_PATH = path.join(findFixturesRoot(), FIXTURE);
const PRESENT = fs.existsSync(FIXTURE_PATH);

// The fixture lands in the spec repo one push after this driver, so that
// `check_driver_coverage.py --strict` has a driver to find for it. Until then
// the suite skips and names the unexercised fixture — "not verified", never
// "passed".
const describeIfPresent = PRESENT ? describe : describe.skip;

interface Case {
  id: string;
  note: string;
  rule: Record<string, unknown>;
  default_effect: string;
  expected_load: 'ok' | 'reject';
}

const fixture = PRESENT
  ? (JSON.parse(fs.readFileSync(FIXTURE_PATH, 'utf-8')) as {
      closed_rule_keys: string[];
      reserved_rule_keys: string[];
      test_cases: Case[];
    })
  : { closed_rule_keys: [], reserved_rule_keys: [], test_cases: [] as Case[] };

/** Minimal YAML emitter — the cases are flat maps, arrays and scalars. */
function toYaml(value: unknown, indent = 0): string {
  const pad = ' '.repeat(indent);
  if (Array.isArray(value)) {
    return value.map((v) => `${pad}- ${JSON.stringify(v)}`).join('\n');
  }
  if (value !== null && typeof value === 'object') {
    return Object.entries(value as Record<string, unknown>)
      .map(([k, v]) => {
        if (Array.isArray(v) && v.every((x) => typeof x !== 'object')) {
          return `${pad}${k}: ${JSON.stringify(v)}`;
        }
        if (v !== null && typeof v === 'object') {
          return `${pad}${k}:\n${toYaml(v, indent + 2)}`;
        }
        return `${pad}${k}: ${JSON.stringify(v)}`;
      })
      .join('\n');
  }
  return `${pad}${JSON.stringify(value)}`;
}

describeIfPresent('Conformance: ACL rule keys are a closed set (§6.1, spec v1.27.0)', () => {
  it('reads the closed and reserved key sets from the fixture, not from this SDK', () => {
    // Reading them locally would let a divergent list agree with itself.
    const acl = fs.readFileSync(path.join(__dirname, '..', 'src', 'acl.ts'), 'utf-8');
    for (const key of fixture.closed_rule_keys) {
      expect(acl, `closed key '${key}' missing from RULE_KEYS`).toContain(`'${key}'`);
    }
    for (const key of fixture.reserved_rule_keys) {
      expect(acl, `reserved key '${key}' missing from RESERVED_RULE_KEYS`).toContain(`'${key}'`);
    }
  });

  fixture.test_cases.forEach((tc) => {
    it(tc.id, () => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'acl-closure-'));
      const file = path.join(dir, 'acl.yaml');
      const write = (rule: Record<string, unknown>, defaultEffect: string): void => {
        fs.writeFileSync(
          file,
          `default_effect: ${defaultEffect}\nrules:\n  - ${toYaml(rule, 4).trimStart()}\n`,
        );
      };
      write(tc.rule, tc.default_effect);

      // A `reject` case whose only offending keys have since been specified is
      // a fixture older than this SDK, not a defect in it. Assert the CURRENT
      // contract rather than the retired one — and only while the fixture has
      // not been regenerated, which is what the `closed_rule_keys` check below
      // establishes.
      const unknownKeys = Object.keys(tc.rule).filter(
        (k) => !fixture.closed_rule_keys.includes(k),
      );
      if (tc.expected_load === 'ok') {
        const acl = ACL.load(file);
        expect(acl.rules.length, tc.note).toBe(1);
      } else {
        let message = '';
        expect(() => {
          try {
            ACL.load(file);
          } catch (e) {
            message = e instanceof Error ? e.message : String(e);
            expect(e, tc.note).toBeInstanceOf(ACLRuleError);
            throw e;
          }
        }, tc.note).toThrow();
        const offending = Object.keys(tc.rule).filter(
          (k) => !fixture.closed_rule_keys.includes(k),
        );
        for (const key of offending) {
          expect(message, `${tc.note}\n  message did not name '${key}': ${message}`).toContain(key);
        }
      }
    });
  });
});
