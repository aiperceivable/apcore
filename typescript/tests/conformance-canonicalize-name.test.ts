/** Canonical public-path fixtures for PROTOCOL_SPEC §2.2.1 and §2.7. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import * as node from '../src/index.js';
import * as browser from '../src/browser/index.js';
import { findFixturesRoot } from './spec-repo.js';

interface NameCase {
  id: string;
  input: { name: string };
  expected: { original_name: string; canonical_name: string | null; error: string | null };
}

const fixture = JSON.parse(readFileSync(join(findFixturesRoot(), 'canonicalize_name.json'), 'utf8')) as {
  test_cases: NameCase[];
};

describe('canonicalize_name canonical fixtures', () => {
  it('executes every unique canonical case without a fallback or skip', () => {
    expect(fixture.test_cases).toHaveLength(32);
    expect(new Set(fixture.test_cases.map((testCase) => testCase.id)).size).toBe(32);
  });
  it('rejects new expected fields until the driver asserts them', () => {
    for (const testCase of fixture.test_cases) {
      expect(Object.keys(testCase.expected).sort()).toEqual(['canonical_name', 'error', 'original_name']);
      expect(Object.keys(testCase.input)).toEqual(['name']);
    }
  });
  for (const testCase of fixture.test_cases) {
    it(testCase.id, () => {
      const nodeResult: node.CanonicalNameResult = node.canonicalizeName(testCase.input.name);
      const browserResult: browser.CanonicalNameResult = browser.canonicalizeName(testCase.input.name);
      for (const result of [nodeResult, browserResult]) {
        expect(Object.keys(result).sort()).toEqual(['canonicalName', 'error', 'originalName']);
        expect({ original_name: result.originalName, canonical_name: result.canonicalName, error: result.error }).toEqual(testCase.expected);
      }
    });
  }
});
