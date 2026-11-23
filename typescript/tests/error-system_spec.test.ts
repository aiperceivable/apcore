/**
 * Spec-traced contract tests for the apcore Error System (TypeScript SDK).
 *
 * Source spec: apcore/docs/features/error-system.md
 * Canonical suite mirrored: apcore-python/tests/test_error_system_spec.py
 * Contract under test: `ModuleError.toJSON` (TS equivalent of Python's `to_dict`).
 *
 * Each `it(...)` name carries the verbatim clause id formatted
 * `error_system.<method>.<kind>.<detail>` so cross-language diffs line up
 * row-for-row with the Python and Rust suites. The clause method segment is
 * kept as `to_dict` (the canonical/normative method name) even though the TS
 * surface spells it `toJSON`, so the clause ids match across languages.
 *
 * These tests are READ-ONLY contract verification — they never modify
 * production source.
 */

import { describe, it, expect } from 'vitest';
import {
  ModuleError,
  ModuleNotFoundError,
  SchemaValidationError,
} from '../src/errors.js';

// ---------------------------------------------------------------------------
// Returns contract: guaranteed keys
// ---------------------------------------------------------------------------

describe('error-system contract (TS toJSON)', () => {
  it('error_system.to_dict.returns.code_key: serialized object always carries a string code', () => {
    const err = new ModuleError('TEST_CODE', 'something failed');
    const result = err.toJSON();
    expect('code' in result).toBe(true);
    expect(result['code']).toBe('TEST_CODE');
    expect(typeof result['code']).toBe('string');
  });

  it('error_system.to_dict.returns.message_key: serialized object always carries a string message', () => {
    const err = new ModuleError('TEST_CODE', 'something failed');
    const result = err.toJSON();
    expect('message' in result).toBe(true);
    expect(result['message']).toBe('something failed');
    expect(typeof result['message']).toBe('string');
  });

  it('error_system.to_dict.returns.ai_guidance_key: ai_guidance appears as a non-empty string when present', () => {
    // ModuleNotFoundError sets a non-empty default ai_guidance.
    const err = new ModuleNotFoundError('missing.mod');
    const result = err.toJSON();
    expect('ai_guidance' in result).toBe(true);
    expect(typeof result['ai_guidance']).toBe('string');
    expect((result['ai_guidance'] as string).length).toBeGreaterThan(0);
  });

  it('error_system.to_dict.returns.timestamp_key: timestamp is an emitted ISO 8601 UTC string', () => {
    const err = new ModuleError('TEST_CODE', 'boom');
    const result = err.toJSON();
    expect('timestamp' in result).toBe(true);
    expect(typeof result['timestamp']).toBe('string');
    expect((result['timestamp'] as string).includes('T')).toBe(true);
  });

  it('error_system.to_dict.returns.details_key: details key round-trips the supplied mapping when populated', () => {
    const err = new ModuleError('TEST_CODE', 'boom', { field: 'email' });
    const result = err.toJSON();
    // TS snake_cases detail keys for wire output; `field` has no camelCase
    // boundary so it stays `field`.
    expect(result['details']).toEqual({ field: 'email' });
  });

  // -------------------------------------------------------------------------
  // Properties
  // -------------------------------------------------------------------------

  it('error_system.to_dict.property.async: toJSON is synchronous, returning a concrete object (not a Promise)', () => {
    const err = new ModuleError('TEST_CODE', 'boom');
    const result = err.toJSON();
    expect(result).not.toBeInstanceOf(Promise);
    expect(typeof result).toBe('object');
    expect(result).not.toBeNull();
  });

  it('error_system.to_dict.property.pure: toJSON does not mutate observable state on the instance', () => {
    const err = new ModuleError(
      'TEST_CODE',
      'boom',
      { field: 'email' },
      undefined,
      undefined,
      false,
      'fix the input',
      true,
      'correct the field',
    );
    const before = JSON.stringify([
      err.code,
      err.message,
      { ...err.details },
      err.cause ?? null,
      err.traceId ?? null,
      err.timestamp,
      err.retryable,
      err.aiGuidance,
      err.userFixable,
      err.suggestion,
    ]);
    err.toJSON();
    const after = JSON.stringify([
      err.code,
      err.message,
      { ...err.details },
      err.cause ?? null,
      err.traceId ?? null,
      err.timestamp,
      err.retryable,
      err.aiGuidance,
      err.userFixable,
      err.suggestion,
    ]);
    expect(after).toBe(before);
  });

  it('error_system.to_dict.property.pure.fresh_top_level: mutating the returned object does not feed back; a fresh serialization still reflects original state', () => {
    const err = new ModuleError('TEST_CODE', 'boom', { field: 'email' });
    const result = err.toJSON();
    result['code'] = 'TAMPERED';
    result['message'] = 'tampered';
    // Top-level mutation of the returned mapping does not touch the instance.
    expect(err.code).toBe('TEST_CODE');
    expect(err.message).toBe('boom');
    // A fresh serialization still reflects the original state.
    const fresh = err.toJSON();
    expect(fresh['code']).toBe('TEST_CODE');
    expect(fresh['message']).toBe('boom');
  });

  it('error_system.to_dict.property.idempotent: two successive calls produce equal output and leave state identical', () => {
    const err = new SchemaValidationError('Schema validation failed', [
      { path: 'email', msg: 'invalid' },
    ]);
    const first = err.toJSON();
    const stateAfterFirst = JSON.stringify([
      err.code,
      err.message,
      { ...err.details },
      err.timestamp,
    ]);
    const second = err.toJSON();
    const stateAfterSecond = JSON.stringify([
      err.code,
      err.message,
      { ...err.details },
      err.timestamp,
    ]);
    expect(second).toEqual(first);
    expect(stateAfterSecond).toBe(stateAfterFirst);
  });

  it('error_system.to_dict.property.thread_safe: N>=8 concurrent serializations of distinct errors via Promise.all all succeed and stay consistent', async () => {
    async function serialize(code: string, message: string): Promise<Record<string, unknown>> {
      const err = new ModuleError(code, message, { i: code });
      // Yield control so the calls genuinely interleave on the event loop.
      await Promise.resolve();
      return err.toJSON();
    }

    const tasks = Array.from({ length: 12 }, (_, i) =>
      serialize(`CODE_${i}`, `message ${i}`),
    );
    const results = await Promise.all(tasks);

    expect(results.length).toBe(12);
    results.forEach((result, i) => {
      expect(result['code']).toBe(`CODE_${i}`);
      expect(result['message']).toBe(`message ${i}`);
      expect(result['details']).toEqual({ i: `CODE_${i}` });
    });
  });
});
