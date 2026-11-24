/**
 * Cross-language conformance driver for error_serialization.json (A-D-008).
 *
 * Fixture source: apcore/conformance/fixtures/error_serialization.json
 * (single source of truth). See that fixture's `description` for the driver
 * contract.
 *
 * Locks ModuleError wire-format serialization: `toJSON()` emits snake_case
 * top-level keys (trace_id / ai_guidance / user_fixable) AND snake_cases the
 * nested `details` object's keys, while the typed SDK properties remain
 * camelCase. Null/None optional fields are omitted (sparse output).
 */

import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { ModuleError } from '../src/errors.js';
import { findFixturesRoot } from './spec-repo.js';

const FIXTURES_ROOT = findFixturesRoot();

function loadFixture(name: string): any {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES_ROOT, `${name}.json`), 'utf-8'));
}

describe('Conformance: ModuleError serialization (A-D-008)', () => {
  const fixture = loadFixture('error_serialization');

  fixture.test_cases.forEach((tc: any) => {
    it(tc.id, () => {
      // Build the ModuleError from `input`, mapping snake_case wire keys to the
      // SDK's camelCase property names (trace_id -> traceId, etc.). The base
      // ModuleError constructor is positional:
      //   (code, message, details, cause, traceId, retryable, aiGuidance,
      //    userFixable, suggestion)
      // Only fields present in `input` are supplied; absent ones stay undefined
      // so the corresponding optional property is omitted from toJSON().
      const input = tc.input;
      const details = 'details' in input ? input.details : undefined;
      const cause = undefined;
      const traceId = 'trace_id' in input ? input.trace_id : undefined;
      const retryable = 'retryable' in input ? input.retryable : undefined;
      const aiGuidance = 'ai_guidance' in input ? input.ai_guidance : undefined;
      const userFixable = 'user_fixable' in input ? input.user_fixable : undefined;

      const err = new ModuleError(
        input.code,
        input.message,
        details,
        cause,
        traceId,
        retryable,
        aiGuidance,
        userFixable,
      );
      const serialized = err.toJSON();

      for (const key of tc.expected_keys_present as string[]) {
        expect(serialized).toHaveProperty(key);
      }
      for (const key of tc.expected_keys_absent as string[]) {
        expect(serialized).not.toHaveProperty(key);
      }

      const detailsPresent = (tc.expected_detail_keys_present as string[]) ?? [];
      const detailsAbsent = (tc.expected_detail_keys_absent as string[]) ?? [];
      if (detailsPresent.length > 0 || detailsAbsent.length > 0) {
        const details = serialized.details as Record<string, unknown> | undefined;
        expect(details).toBeDefined();
        for (const key of detailsPresent) {
          expect(details).toHaveProperty(key);
        }
        for (const key of detailsAbsent) {
          expect(details).not.toHaveProperty(key);
        }
      }
    });
  });
});
