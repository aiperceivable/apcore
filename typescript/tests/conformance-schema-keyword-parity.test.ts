/**
 * Cross-language conformance driver for schema_keyword_parity.json
 * (spec §4.15, JSON Schema 2020-12 §6 / §7.2.1 / §10.2).
 *
 * Fixture source: apcore/conformance/fixtures/schema_keyword_parity.json
 * (single source of truth). See that fixture's `driver_contract` for the
 * required code path.
 *
 * DRIVER CONTRACT: this suite MUST exercise the path a module invocation takes,
 * i.e. `jsonSchemaToTypeBox()` conversion followed by `SchemaValidator.validate()`
 * — the same pair `BuiltinInputValidation` uses in src/builtin-steps.ts. Driving
 * a raw JSON Schema validator instead would hide converter defects (dropped
 * `type` members, discarded combinator siblings, vanished array constraints),
 * which is precisely what this fixture exists to catch.
 */

import { describe, it, expect, vi } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { jsonSchemaToTypeBox } from '../src/schema/loader-pure.js';
import { SchemaValidator } from '../src/schema/validator.js';
import { findFixturesRoot } from './spec-repo.js';

interface KeywordParityCase {
  readonly id: string;
  readonly description?: string;
  readonly schema: Record<string, unknown>;
  /** Every case validates an object against an object-typed root schema. */
  readonly input: Record<string, unknown>;
  readonly expected_valid: boolean;
}

function loadFixture(name: string): { test_cases: readonly KeywordParityCase[] } {
  const file = path.join(findFixturesRoot(), `${name}.json`);
  return JSON.parse(fs.readFileSync(file, 'utf-8'));
}

describe('Conformance: JSON Schema keyword parity at the validation boundary', () => {
  const fixture = loadFixture('schema_keyword_parity');

  fixture.test_cases.forEach((tc) => {
    it(tc.id, () => {
      // A SHOULD-level format warning is expected for at least one case
      // (recognised-but-unsatisfied `format`); silence it so it does not
      // pollute the suite output. Coercion stays off, matching the executor.
      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      let result: ReturnType<SchemaValidator['validate']>;
      try {
        const converted = jsonSchemaToTypeBox(tc.schema);
        result = new SchemaValidator(false).validate(tc.input, converted);
      } finally {
        warnSpy.mockRestore();
      }

      // Conformance failures usually mean a cross-language divergence, so the
      // message carries everything needed to reproduce the case standalone.
      const detail = [
        `case: ${tc.id}`,
        tc.description ? `description: ${tc.description}` : null,
        `schema: ${JSON.stringify(tc.schema)}`,
        `input: ${JSON.stringify(tc.input)}`,
        `expected valid: ${tc.expected_valid}`,
        `actual valid: ${result.valid}`,
        `errors: ${JSON.stringify(result.errors ?? [])}`,
      ]
        .filter((line) => line !== null)
        .join('\n');

      expect(result.valid, detail).toBe(tc.expected_valid);
    });
  });

  it('drives every fixture case', () => {
    expect(fixture.test_cases.length).toBe(122);
  });
});
