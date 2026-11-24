/**
 * Tests for module.ts: ModuleAnnotations, createAnnotations, annotationsToJSON,
 * annotationsFromJSON, createPreflightResult, and related types.
 */

import { describe, it, expect, vi } from 'vitest';
import {
  DEFAULT_ANNOTATIONS,
  createAnnotations,
  annotationsToJSON,
  annotationsFromJSON,
  createPreflightResult,
} from '../src/module.js';
import type {
  ModuleAnnotations,
  PreflightCheckResult,
} from '../src/module.js';

// ---------------------------------------------------------------------------
// DEFAULT_ANNOTATIONS
// ---------------------------------------------------------------------------

describe('DEFAULT_ANNOTATIONS', () => {
  it('has expected defaults', () => {
    expect(DEFAULT_ANNOTATIONS.readonly).toBe(false);
    expect(DEFAULT_ANNOTATIONS.destructive).toBe(false);
    expect(DEFAULT_ANNOTATIONS.idempotent).toBe(false);
    expect(DEFAULT_ANNOTATIONS.requiresApproval).toBe(false);
    expect(DEFAULT_ANNOTATIONS.openWorld).toBe(true);
    expect(DEFAULT_ANNOTATIONS.streaming).toBe(false);
    expect(DEFAULT_ANNOTATIONS.cacheable).toBe(false);
    expect(DEFAULT_ANNOTATIONS.cacheTtl).toBe(0);
    expect(DEFAULT_ANNOTATIONS.cacheKeyFields).toBeNull();
    expect(DEFAULT_ANNOTATIONS.paginated).toBe(false);
    expect(DEFAULT_ANNOTATIONS.paginationStyle).toBe('cursor');
    expect(DEFAULT_ANNOTATIONS.extra).toEqual({});
  });

  it('is frozen', () => {
    expect(Object.isFrozen(DEFAULT_ANNOTATIONS)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// createAnnotations
// ---------------------------------------------------------------------------

describe('createAnnotations', () => {
  it('returns default annotations when called with no overrides', () => {
    const a = createAnnotations();
    expect(a.readonly).toBe(false);
    expect(a.cacheTtl).toBe(0);
    expect(a.paginationStyle).toBe('cursor');
  });

  it('applies provided overrides', () => {
    const a = createAnnotations({ readonly: true, idempotent: true, cacheTtl: 60 });
    expect(a.readonly).toBe(true);
    expect(a.idempotent).toBe(true);
    expect(a.cacheTtl).toBe(60);
    // Unoveridden defaults are still present
    expect(a.destructive).toBe(false);
  });

  it('clamps negative cacheTtl to 0 with a warning', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const a = createAnnotations({ cacheTtl: -5 });
    expect(a.cacheTtl).toBe(0);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('negative'));
    warnSpy.mockRestore();
  });

  it('returns a frozen object', () => {
    const a = createAnnotations({ cacheable: true });
    expect(Object.isFrozen(a)).toBe(true);
  });

  it('freezes the extra object', () => {
    const a = createAnnotations({ extra: { myKey: 'value' } });
    expect(Object.isFrozen(a.extra)).toBe(true);
    expect(a.extra['myKey']).toBe('value');
  });

  it('sets cacheKeyFields when provided', () => {
    const a = createAnnotations({ cacheKeyFields: ['id', 'locale'] });
    expect(a.cacheKeyFields).toEqual(['id', 'locale']);
  });

  it('sets streaming and requiresApproval correctly', () => {
    const a = createAnnotations({ streaming: true, requiresApproval: true });
    expect(a.streaming).toBe(true);
    expect(a.requiresApproval).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// annotationsToJSON
// ---------------------------------------------------------------------------

describe('annotationsToJSON', () => {
  it('serializes to snake_case keys', () => {
    const a = createAnnotations({
      requiresApproval: true,
      openWorld: false,
      cacheKeyFields: ['id'],
      paginationStyle: 'offset',
    });
    const json = annotationsToJSON(a);
    expect(json['requires_approval']).toBe(true);
    expect(json['open_world']).toBe(false);
    expect(json['cache_key_fields']).toEqual(['id']);
    expect(json['pagination_style']).toBe('offset');
  });

  it('includes all expected keys', () => {
    const a = createAnnotations();
    const json = annotationsToJSON(a);
    const expectedKeys = [
      'readonly', 'destructive', 'idempotent', 'requires_approval',
      'open_world', 'streaming', 'cacheable', 'cache_ttl',
      'cache_key_fields', 'paginated', 'pagination_style', 'extra',
    ];
    for (const key of expectedKeys) {
      expect(json).toHaveProperty(key);
    }
  });

  it('round-trips through annotationsFromJSON', () => {
    const original = createAnnotations({
      readonly: true,
      cacheTtl: 300,
      cacheable: true,
      cacheKeyFields: ['userId'],
      extra: { tier: 'premium' },
    });
    const json = annotationsToJSON(original);
    const restored = annotationsFromJSON(json as Record<string, unknown>);
    expect(restored.readonly).toBe(original.readonly);
    expect(restored.cacheTtl).toBe(original.cacheTtl);
    expect(restored.cacheable).toBe(original.cacheable);
    expect(restored.cacheKeyFields).toEqual(original.cacheKeyFields);
    expect(restored.extra['tier']).toBe('premium');
  });
});

// ---------------------------------------------------------------------------
// annotationsFromJSON
// ---------------------------------------------------------------------------

describe('annotationsFromJSON', () => {
  it('parses a complete snake_case JSON record', () => {
    const data = {
      readonly: true,
      destructive: false,
      idempotent: true,
      requires_approval: false,
      open_world: true,
      streaming: false,
      cacheable: true,
      cache_ttl: 120,
      cache_key_fields: ['id'],
      paginated: false,
      pagination_style: 'cursor',
      extra: {},
    };
    const a = annotationsFromJSON(data);
    expect(a.readonly).toBe(true);
    expect(a.idempotent).toBe(true);
    expect(a.cacheTtl).toBe(120);
    expect(a.cacheKeyFields).toEqual(['id']);
  });

  it('applies defaults for missing fields', () => {
    const a = annotationsFromJSON({});
    expect(a.readonly).toBe(false);
    expect(a.cacheTtl).toBe(0);
    expect(a.openWorld).toBe(true);
    expect(a.paginationStyle).toBe('cursor');
  });

  it('clamps negative cache_ttl to 0 with a warning', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const a = annotationsFromJSON({ cache_ttl: -10 });
    expect(a.cacheTtl).toBe(0);
    expect(warnSpy).toHaveBeenCalledWith(expect.stringContaining('negative'));
    warnSpy.mockRestore();
  });

  it('merges unknown (legacy overflow) keys into extra', () => {
    const a = annotationsFromJSON({ legacy_key: 'foo', extra: {} });
    expect(a.extra['legacy_key']).toBe('foo');
  });

  it('nested extra wins over legacy top-level overflow (§4.4.1 rule 7)', () => {
    const a = annotationsFromJSON({ shared_key: 'overflow', extra: { shared_key: 'nested' } });
    expect(a.extra['shared_key']).toBe('nested');
  });

  it('returns a frozen object', () => {
    const a = annotationsFromJSON({});
    expect(Object.isFrozen(a)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// createPreflightResult
// ---------------------------------------------------------------------------

describe('createPreflightResult', () => {
  it('returns valid=true when all checks pass', () => {
    const checks: PreflightCheckResult[] = [
      { check: 'schema', passed: true },
      { check: 'auth', passed: true },
    ];
    const result = createPreflightResult(checks);
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
    expect(result.requiresApproval).toBe(false);
  });

  it('returns valid=false when any check fails', () => {
    const checks: PreflightCheckResult[] = [
      { check: 'schema', passed: true },
      { check: 'quota', passed: false, error: { reason: 'over limit' } },
    ];
    const result = createPreflightResult(checks);
    expect(result.valid).toBe(false);
    expect(result.errors).toHaveLength(1);
    expect(result.errors[0]).toEqual({ reason: 'over limit' });
  });

  it('includes requiresApproval when provided', () => {
    const checks: PreflightCheckResult[] = [{ check: 'x', passed: true }];
    const result = createPreflightResult(checks, true);
    expect(result.requiresApproval).toBe(true);
  });

  it('does not include checks without error in errors array', () => {
    const checks: PreflightCheckResult[] = [
      { check: 'a', passed: false },
      { check: 'b', passed: false, error: { msg: 'bad' } },
    ];
    const result = createPreflightResult(checks);
    expect(result.errors).toHaveLength(1);
  });

  it('preserves checks array on result', () => {
    const checks: PreflightCheckResult[] = [
      { check: 'c1', passed: true },
      { check: 'c2', passed: false },
    ];
    const result = createPreflightResult(checks);
    expect(result.checks).toHaveLength(2);
    expect(result.checks[0].check).toBe('c1');
  });

  it('handles empty checks array', () => {
    const result = createPreflightResult([]);
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
  });
});
