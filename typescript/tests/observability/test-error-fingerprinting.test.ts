/**
 * Issue #43 §4 — Error fingerprinting.
 *
 * ErrorHistory MUST dedupe by SHA-256 fingerprint of
 *   error_code + ":" + module_id + ":" + normalized_message
 * where the normalizer replaces UUIDs, ISO timestamps, and integers with
 * length >= 4 digits with placeholder tokens.
 */
import { describe, expect, it } from 'vitest';
import { ModuleError } from '../../src/errors.js';
import {
  ErrorHistory,
  computeFingerprint,
  normalizeMessage,
} from '../../src/observability/error-history.js';

describe('Error fingerprinting (Issue #43 §4)', () => {
  it('normalizes UUIDs to <UUID>', () => {
    const msg = 'failed to fetch user 550e8400-e29b-41d4-a716-446655440000 from db';
    expect(normalizeMessage(msg)).toContain('<uuid>');
  });

  it('normalizes ISO timestamps to <TIMESTAMP>', () => {
    const msg = 'job at 2024-01-15T10:30:45Z exceeded budget';
    expect(normalizeMessage(msg)).toContain('<timestamp>');
  });

  it('normalizes integers >= 4 digits to <ID>', () => {
    const msg = 'request 123456 failed for tenant 78901';
    const out = normalizeMessage(msg);
    expect(out).toContain('<id>');
    expect(out).not.toContain('123456');
    expect(out).not.toContain('78901');
  });

  it('produces a 64-char hex SHA-256 fingerprint', () => {
    const fp = computeFingerprint('TIMEOUT', 'mod.a', 'request 12345 timed out');
    expect(fp).toMatch(/^[0-9a-f]{64}$/);
  });

  it('produces identical fingerprints for messages differing only in UUID', () => {
    const a = computeFingerprint(
      'NOT_FOUND',
      'mod.x',
      'user 550e8400-e29b-41d4-a716-446655440000 not found',
    );
    const b = computeFingerprint(
      'NOT_FOUND',
      'mod.x',
      'user 11111111-2222-3333-4444-555555555555 not found',
    );
    expect(a).toBe(b);
  });

  it('produces different fingerprints across distinct error codes', () => {
    const a = computeFingerprint('TIMEOUT', 'mod.x', 'failed');
    const b = computeFingerprint('NOT_FOUND', 'mod.x', 'failed');
    expect(a).not.toBe(b);
  });

  it('ErrorHistory dedupes UUID-bearing messages into a single entry', () => {
    const history = new ErrorHistory();
    history.record(
      'mod.users',
      new ModuleError('NOT_FOUND', 'user 550e8400-e29b-41d4-a716-446655440000 not found'),
    );
    history.record(
      'mod.users',
      new ModuleError('NOT_FOUND', 'user 11111111-2222-3333-4444-555555555555 not found'),
    );

    const entries = history.get('mod.users');
    expect(entries).toHaveLength(1);
    expect(entries[0].count).toBe(2);
  });

  it('ErrorHistory dedupes integer-ID-bearing messages into a single entry', () => {
    const history = new ErrorHistory();
    history.record('mod.api', new ModuleError('REQ_FAIL', 'request 123456 failed'));
    history.record('mod.api', new ModuleError('REQ_FAIL', 'request 987654 failed'));

    const entries = history.get('mod.api');
    expect(entries).toHaveLength(1);
    expect(entries[0].count).toBe(2);
  });
});
