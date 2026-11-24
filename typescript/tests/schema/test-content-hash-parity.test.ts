/**
 * Cross-language conformance driver for schema_content_hash.json (A-D-037).
 *
 * Fixture source: apcore/conformance/fixtures/schema_content_hash.json
 * (single source of truth). The cases are LOADED from the fixture rather than
 * hand-mirrored here: a hand-copy silently drifts from the canonical fixture,
 * which defeats the purpose of a cross-repo hash-parity check.
 *
 * The fixture intentionally records no `expected` hash — each SDK computes it
 * and the harness compares the digests across repos byte-for-byte.
 *
 * The fixture text is parsed with `JSON.parse` (not imported as a module) so
 * large-integer literals never become JS source-level numbers. That is also
 * the SDK's real canonicalization path: JSON in -> contentHash out.
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { contentHash } from '../../src/schema/loader.js';
import { findFixturesRoot } from '../spec-repo.js';

interface ContentHashCase {
  id: string;
  schema: unknown;
  note?: string;
}

const FIXTURE = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'schema_content_hash.json'), 'utf-8'),
) as { test_cases: ContentHashCase[] };

describe('Conformance: schema_content_hash.json (A-D-037)', () => {
  it('loads every canonical case from the fixture', () => {
    expect(FIXTURE.test_cases.length).toBeGreaterThan(0);
    for (const tc of FIXTURE.test_cases) {
      expect(typeof tc.id).toBe('string');
      expect(tc.schema).toBeDefined();
    }
  });

  it('computes a stable 64-char hex digest for each tricky schema and reports them', () => {
    const digests: Record<string, string> = {};
    for (const tc of FIXTURE.test_cases) {
      const hex = contentHash(tc.schema);
      digests[tc.id] = hex;
      expect(hex, `case ${tc.id}`).toMatch(/^[0-9a-f]{64}$/);
    }
    // Determinism: hashing again yields identical digests.
    for (const tc of FIXTURE.test_cases) {
      expect(contentHash(tc.schema), `case ${tc.id}`).toBe(digests[tc.id]);
    }
    // Surface the digests for cross-repo comparison.
    // eslint-disable-next-line no-console
    console.log('A-D-037 contentHash digests (apcore-js):\n' + JSON.stringify(digests, null, 2));
  });

  it('produces distinct digests for distinct schemas', () => {
    const digests = FIXTURE.test_cases.map((tc) => contentHash(tc.schema));
    expect(new Set(digests).size).toBe(digests.length);
  });
});
