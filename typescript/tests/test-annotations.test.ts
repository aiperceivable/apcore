/**
 * Tests for ModuleAnnotations redesign: extra field, pagination_style,
 * createAnnotations(), toJSON/fromJSON wire format.
 */

import { describe, it, expect } from 'vitest';
import {
  DEFAULT_ANNOTATIONS,
  createAnnotations,
  annotationsToJSON,
  annotationsFromJSON,
} from '../src/module.js';
import type { ModuleAnnotations } from '../src/module.js';

describe('ModuleAnnotations extra field', () => {
  it('AC-005: default extra is empty object', () => {
    expect(DEFAULT_ANNOTATIONS.extra).toEqual({});
  });

  it('extra preserves values', () => {
    const ann = createAnnotations({ extra: { 'mcp.category': 'tools' } });
    expect(ann.extra['mcp.category']).toBe('tools');
  });

  it('extra is frozen', () => {
    const ann = createAnnotations({ extra: { k: 'v' } });
    expect(Object.isFrozen(ann.extra)).toBe(true);
  });
});

describe('ModuleAnnotations pagination_style', () => {
  it('AC-007: accepts arbitrary strings', () => {
    const ann = createAnnotations({ paginationStyle: 'custom' });
    expect(ann.paginationStyle).toBe('custom');
  });

  it('default is cursor', () => {
    expect(DEFAULT_ANNOTATIONS.paginationStyle).toBe('cursor');
  });
});

describe('createAnnotations factory', () => {
  it('AC-008: fills defaults for unspecified fields', () => {
    const ann = createAnnotations({ destructive: true });
    expect(ann.destructive).toBe(true);
    expect(ann.readonly).toBe(false);
    expect(ann.openWorld).toBe(true);
    expect(ann.cacheable).toBe(false);
    expect(ann.cacheTtl).toBe(0);
    expect(ann.cacheKeyFields).toBeNull();
    expect(ann.paginated).toBe(false);
    expect(ann.paginationStyle).toBe('cursor');
    expect(ann.extra).toEqual({});
  });

  it('returns frozen object', () => {
    const ann = createAnnotations();
    expect(Object.isFrozen(ann)).toBe(true);
  });

  it('no arguments returns defaults', () => {
    const ann = createAnnotations();
    expect(ann.readonly).toBe(false);
    expect(ann.openWorld).toBe(true);
    expect(ann.extra).toEqual({});
  });

  it('AC-027: negative cacheTtl clamped to 0', () => {
    const ann = createAnnotations({ cacheTtl: -5 });
    expect(ann.cacheTtl).toBe(0);
  });
});

describe('annotationsToJSON', () => {
  it('AC-024: produces snake_case keys', () => {
    const ann = createAnnotations({ requiresApproval: true });
    const json = annotationsToJSON(ann);
    expect(json['requires_approval']).toBe(true);
    expect(json['open_world']).toBe(true);
    expect(json['cache_ttl']).toBe(0);
    expect(json['cache_key_fields']).toBeNull();
    expect(json['pagination_style']).toBe('cursor');
  });

  it('includes extra', () => {
    const ann = createAnnotations({ extra: { 'mcp.cat': 'tools' } });
    const json = annotationsToJSON(ann);
    expect((json['extra'] as Record<string, unknown>)['mcp.cat']).toBe('tools');
  });
});

describe('annotationsFromJSON', () => {
  it('AC-025: converts snake_case to camelCase', () => {
    const ann = annotationsFromJSON({ requires_approval: true, open_world: false });
    expect(ann.requiresApproval).toBe(true);
    expect(ann.openWorld).toBe(false);
  });

  it('AC-006: unknown keys go to extra', () => {
    const ann = annotationsFromJSON({ readonly: true, future_field: 42 });
    expect(ann.readonly).toBe(true);
    expect(ann.extra['future_field']).toBe(42);
  });

  it('explicit extra merged with unknown', () => {
    const ann = annotationsFromJSON({
      extra: { 'mcp.cat': 'tools' },
      new_field: 'val',
    });
    expect(ann.extra['mcp.cat']).toBe('tools');
    expect(ann.extra['new_field']).toBe('val');
  });

  it('§4.4.1 rule 7: nested extra wins over top-level collision', () => {
    const ann = annotationsFromJSON({
      'mcp.category': 'LEGACY_VALUE',
      extra: { 'mcp.category': 'CANONICAL_VALUE' },
    });
    expect(ann.extra['mcp.category']).toBe('CANONICAL_VALUE');
  });

  it('legacy flattened form still accepted', () => {
    const ann = annotationsFromJSON({
      readonly: true,
      'mcp.category': 'tools',
      'cli.approval_message': 'ok?',
    });
    expect(ann.readonly).toBe(true);
    expect(ann.extra).toEqual({
      'mcp.category': 'tools',
      'cli.approval_message': 'ok?',
    });
  });

  it('missing fields use defaults', () => {
    const ann = annotationsFromJSON({});
    expect(ann.readonly).toBe(false);
    expect(ann.openWorld).toBe(true);
    expect(ann.paginationStyle).toBe('cursor');
    expect(ann.extra).toEqual({});
  });

  it('returns frozen object', () => {
    const ann = annotationsFromJSON({});
    expect(Object.isFrozen(ann)).toBe(true);
  });
});

describe('round-trip', () => {
  it('AC-026: fromJSON(toJSON(annotations)) preserves all fields', () => {
    const original = createAnnotations({
      destructive: true,
      requiresApproval: true,
      openWorld: false,
      cacheable: true,
      cacheTtl: 60,
      cacheKeyFields: ['id', 'name'],
      paginated: true,
      paginationStyle: 'offset',
      extra: { 'cli.approval_message': 'Are you sure?' },
    });
    const json = annotationsToJSON(original);
    const restored = annotationsFromJSON(json);

    expect(restored.destructive).toBe(original.destructive);
    expect(restored.requiresApproval).toBe(original.requiresApproval);
    expect(restored.openWorld).toBe(original.openWorld);
    expect(restored.cacheable).toBe(original.cacheable);
    expect(restored.cacheTtl).toBe(original.cacheTtl);
    expect(restored.cacheKeyFields).toEqual(original.cacheKeyFields);
    expect(restored.paginated).toBe(original.paginated);
    expect(restored.paginationStyle).toBe(original.paginationStyle);
    expect(restored.extra).toEqual(original.extra);
  });
});

describe('DEFAULT_ANNOTATIONS', () => {
  it('is frozen', () => {
    expect(Object.isFrozen(DEFAULT_ANNOTATIONS)).toBe(true);
  });

  it('has all required fields', () => {
    const ann: ModuleAnnotations = DEFAULT_ANNOTATIONS;
    expect(ann.readonly).toBe(false);
    expect(ann.destructive).toBe(false);
    expect(ann.idempotent).toBe(false);
    expect(ann.requiresApproval).toBe(false);
    expect(ann.openWorld).toBe(true);
    expect(ann.streaming).toBe(false);
    expect(ann.cacheable).toBe(false);
    expect(ann.cacheTtl).toBe(0);
    expect(ann.cacheKeyFields).toBeNull();
    expect(ann.paginated).toBe(false);
    expect(ann.paginationStyle).toBe('cursor');
    expect(ann.extra).toEqual({});
  });
});
