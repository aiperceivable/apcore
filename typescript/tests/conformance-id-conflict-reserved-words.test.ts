/**
 * Cross-language driver for `id_conflict_reserved_words.json`.
 *
 * PROTOCOL_SPEC §2.6 step 2, narrowed to the **first segment** in spec v1.26.0
 * (#99). A reserved word claims a namespace, not a token, so `foo.system.bar`
 * and `executor.schema.validate` are legal and `system.custom_module` is not.
 *
 * The driver exercises the **public** `register()` path deliberately.
 * `registerInternal()` bypasses the reserved-word check by design, so running
 * the cases through it would report agreement while testing nothing.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { Registry, RESERVED_WORDS } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURE = 'id_conflict_reserved_words.json';
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
  new_id: string;
  existing_ids?: string[];
  expected: string | null;
}

function loadFixture(): {
  reserved_words: string[];
  error_code_by_conflict: Record<string, string>;
  test_cases: Case[];
} {
  return JSON.parse(fs.readFileSync(FIXTURE_PATH, 'utf-8'));
}

const fixture = PRESENT ? loadFixture() : { reserved_words: [], test_cases: [] as Case[] };

const conformantModule = {
  description: 'conformance fixture module',
  inputSchema: () => ({ type: 'object' }),
  outputSchema: () => ({ type: 'object' }),
  execute: async () => ({}),
};

describeIfPresent('Conformance: reserved-word ID conflicts (§2.6 step 2, spec v1.26.0)', () => {
  it('reads the canonical reserved-word set from the fixture, not from this SDK', () => {
    // Reading it from `apcore-js` would let a divergent local list agree with
    // itself: every case would be computed from the same wrong set and pass.
    expect(new Set(fixture.reserved_words)).toEqual(new Set(RESERVED_WORDS));
  });

  fixture.test_cases.forEach((tc) => {
    it(tc.id, () => {
      const registry = new Registry();
      for (const existing of tc.existing_ids ?? []) {
        registry.register(existing, conformantModule);
      }

      if (tc.expected === null) {
        // Must register cleanly. A throw here is the pre-v1.26.0 per-segment
        // reading resurfacing.
        expect(() => registry.register(tc.new_id, conformantModule), tc.note).not.toThrow();
        expect(registry.get(tc.new_id), tc.note).toBeTruthy();
      } else {
        // The fixture names the conflict `type`; SDKs surface it through their
        // own error classes, so assert the registration was refused and that
        // the message identifies the offending id, rather than pinning a class
        // name the three languages do not share.
        let message = '';
        let code: unknown;
        expect(() => {
          try {
            registry.register(tc.new_id, conformantModule);
          } catch (e) {
            message = e instanceof Error ? e.message : String(e);
            code = (e as { code?: unknown }).code;
            throw e;
          }
        }, tc.note).toThrow();
        expect(
          message.includes(tc.new_id) || message.includes(tc.new_id.split('.')[0]),
          `${tc.note} — message was: ${message}`,
        ).toBe(true);

        // ...and the WIRE CODE says WHICH conflict it was. Without this the
        // driver only asserts "refused", so `check_case_pinning.py` could
        // mutate `expected` from `reserved_word` to `duplicate_id` — a value
        // this very fixture uses — and every SDK stayed green: the distinction
        // the fixture exists to draw was asserted by nobody. The mapping lives
        // in the fixture, not here, because it is one rule and §8's codes are
        // the cross-language contract.
        expect(code, `${tc.id}: the ${tc.expected} conflict must surface its wire code`).toBe(
          loadFixture().error_code_by_conflict[tc.expected as string],
        );
      }
    });
  });
});
