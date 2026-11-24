/**
 * Cross-language conformance driver for openai_strict_compat.json
 * (protocol-spec §5.12.5).
 *
 * Fixture source: apcore/conformance/fixtures/openai_strict_compat.json
 * (single source of truth). See that fixture's `driver_contract`.
 *
 * DRIVER CONTRACT: this suite MUST drive `detectOpenAiStrictIncompatibilities()`
 * — the detector that backs `BindingStrictSchemaIncompatibleError` on the
 * `auto_schema: strict` binding path. The detector is the shared deterministic
 * surface across the three SDKs; the binding wrapper around it differs (Rust
 * performs no runtime type inference). Feature lists are compared for exact
 * equality INCLUDING ORDER: byte-identical output across SDKs is the point.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { detectOpenAiStrictIncompatibilities } from '../src/schema/openai-strict.js';
import { findFixturesRoot } from './spec-repo.js';

interface OpenAiStrictCase {
  readonly id: string;
  readonly description?: string;
  /** `true` in one case: a boolean schema, which the walker must tolerate. */
  readonly schema: Record<string, unknown> | boolean;
  readonly expected_features: readonly string[];
}

function loadFixture(name: string): {
  test_cases: readonly OpenAiStrictCase[];
  keyword_set_source: Record<string, unknown>;
} {
  const file = path.join(findFixturesRoot(), `${name}.json`);
  return JSON.parse(fs.readFileSync(file, 'utf-8'));
}

describe('Conformance: OpenAI structured-outputs strict compatibility', () => {
  const fixture = loadFixture('openai_strict_compat');

  fixture.test_cases.forEach((tc) => {
    it(tc.id, () => {
      const schema = tc.schema as Record<string, unknown>;
      const before = JSON.stringify(tc.schema);

      const got = detectOpenAiStrictIncompatibilities(schema);

      expect(got, `[${tc.id}] ${tc.description ?? ''}`).toEqual([...tc.expected_features]);
      // The detector reports; it must never rewrite (no oneOf → anyOf downgrade).
      expect(JSON.stringify(tc.schema), `[${tc.id}] detector mutated the input schema`).toBe(
        before,
      );
    });
  });

  it('fixture case ids are unique', () => {
    const ids = fixture.test_cases.map((tc) => tc.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('fixture records the OpenAI keyword-set provenance', () => {
    // The supported set moves; the fixture must always say what it was pinned to.
    expect(fixture.keyword_set_source['url']).toContain('structured-outputs');
    expect(fixture.keyword_set_source['verified']).toBeTruthy();
  });
});
