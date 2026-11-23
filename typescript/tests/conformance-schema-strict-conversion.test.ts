/**
 * Cross-language conformance driver for schema_strict_conversion.json
 * (PROTOCOL_SPEC §4.16 / ALGORITHMS A23).
 *
 * Fixture source: apcore/conformance/fixtures/schema_strict_conversion.json
 * (single source of truth). See that fixture's `driver_contract`.
 *
 * DRIVER CONTRACT: this suite MUST drive `toStrictSchema()` — the A23 entry
 * point — not the exporter and not the binding wrapper. A23 is the shared
 * deterministic surface; the three SDKs must emit the same strict schema for
 * the same input.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { Value } from '@sinclair/typebox/value';
import { jsonSchemaToTypeBox } from '../src/schema/loader.js';
import { toStrictSchema } from '../src/schema/strict.js';
import { findFixturesRoot } from './spec-repo.js';

interface StrictConversionCase {
  readonly id: string;
  readonly description?: string;
  readonly schema: Record<string, unknown>;
  readonly expected: Record<string, unknown>;
  readonly validation_cases?: readonly { readonly data: unknown; readonly valid: boolean }[];
}

function loadFixture(name: string): { test_cases: readonly StrictConversionCase[] } {
  const file = path.join(findFixturesRoot(), `${name}.json`);
  return JSON.parse(fs.readFileSync(file, 'utf-8'));
}

describe('Conformance: to_strict_schema (A23)', () => {
  const fixture = loadFixture('schema_strict_conversion');

  fixture.test_cases.forEach((tc) => {
    it(tc.id, () => {
      const before = JSON.stringify(tc.schema);

      const got = toStrictSchema(tc.schema);

      expect(got, `[${tc.id}] ${tc.description ?? ''}`).toEqual(tc.expected);
      if (tc.validation_cases) {
        const validator = jsonSchemaToTypeBox(got);
        for (const example of tc.validation_cases) {
          expect(Value.Check(validator, example.data), `[${tc.id}] ${JSON.stringify(example.data)}`)
            .toBe(example.valid);
        }
      }
      // A23 MUST deep-copy — the caller's schema is never mutated.
      expect(JSON.stringify(tc.schema), `[${tc.id}] to_strict_schema mutated its input`).toBe(
        before,
      );
    });
  });

  it('fixture case ids are unique', () => {
    const ids = fixture.test_cases.map((tc) => tc.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
