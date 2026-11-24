/**
 * MOD-001 — YAML annotation overrides were filtered against camelCase names.
 *
 * `ANNOTATION_FIELDS` listed the `ModuleAnnotations` STRUCT field names, and
 * `mergeAnnotations` filtered the raw `*_meta.yaml` mapping against it — but
 * `loadMetadata` returns that mapping un-normalized, so its keys are the wire
 * (snake_case) spellings. Five of the thirteen fields therefore ignored the
 * metadata file entirely: `requires_approval`, `open_world`, `cache_ttl`,
 * `cache_key_fields` and `pagination_style`. The other eight survived only
 * because their two spellings happen to coincide.
 *
 * §4.13 makes the YAML metadata file the highest-priority layer (a MUST), and
 * protocol-spec.md's canonical `*_meta.yaml` example uses `requires_approval:`
 * / `open_world:`. apcore-python matches on its dataclass field names
 * (snake_case) and apcore-rust overlays the raw keys, so both carried the
 * YAML through.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { mergeAnnotations } from '../../src/schema/annotations.js';
import { mergeModuleMetadata } from '../../src/registry/metadata-pure.js';
import type { ModuleAnnotations } from '../../src/module.js';

afterEach(() => {
  vi.restoreAllMocks();
});

const CODE: ModuleAnnotations = {
  readonly: false,
  destructive: false,
  idempotent: false,
  requiresApproval: false,
  openWorld: true,
  streaming: false,
  cacheable: false,
  cacheTtl: 0,
  cacheKeyFields: null,
  paginated: false,
  paginationStyle: 'cursor',
  discoverable: true,
  extra: {},
};

describe('mergeAnnotations honours the wire spelling (MOD-001)', () => {
  it('requires_approval from YAML overrides the code annotation', () => {
    const result = mergeAnnotations({ requires_approval: true }, CODE);
    expect(result.requiresApproval).toBe(true);
  });

  it('open_world from YAML overrides the code annotation', () => {
    const result = mergeAnnotations({ open_world: false }, CODE);
    expect(result.openWorld).toBe(false);
  });

  it('cache_ttl, cache_key_fields and pagination_style come through', () => {
    const result = mergeAnnotations(
      {
        cacheable: true,
        cache_ttl: 600,
        cache_key_fields: ['id', 'region'],
        paginated: true,
        pagination_style: 'offset',
      },
      CODE,
    );
    expect(result.cacheable).toBe(true);
    expect(result.cacheTtl).toBe(600);
    expect(result.cacheKeyFields).toEqual(['id', 'region']);
    expect(result.paginated).toBe(true);
    expect(result.paginationStyle).toBe('offset');
  });

  it('the single-word keys keep working', () => {
    const result = mergeAnnotations(
      { readonly: true, destructive: true, idempotent: true, streaming: true, discoverable: false },
      CODE,
    );
    expect(result.readonly).toBe(true);
    expect(result.destructive).toBe(true);
    expect(result.idempotent).toBe(true);
    expect(result.streaming).toBe(true);
    expect(result.discoverable).toBe(false);
  });

  it('a genuinely unknown key is still ignored', () => {
    const result = mergeAnnotations({ requires_aproval: true }, CODE);
    expect(result.requiresApproval).toBe(false);
    expect((result as unknown as Record<string, unknown>)['requires_aproval']).toBeUndefined();
  });

  it('the camelCase spelling still works but warns that it is not portable', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const result = mergeAnnotations({ requiresApproval: true }, CODE);
    expect(result.requiresApproval).toBe(true);
    expect(warn).toHaveBeenCalled();
    expect(String(warn.mock.calls[0][0])).toContain('requires_approval');
  });

  it('the wire spelling wins when both spellings are present', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const result = mergeAnnotations({ requiresApproval: false, requires_approval: true }, CODE);
    expect(result.requiresApproval).toBe(true);
  });
});

describe('the descriptor built from *_meta.yaml carries the approval requirement', () => {
  it('a module_meta.yaml declaring requires_approval: true reaches the descriptor', () => {
    const moduleObj = {
      description: 'send an email',
      annotations: { ...CODE },
      execute: () => ({}),
    };
    const meta = {
      description: 'send an email',
      annotations: { requires_approval: true, open_world: false },
    };

    const merged = mergeModuleMetadata(moduleObj, meta);
    const annotations = merged['annotations'] as ModuleAnnotations;
    expect(annotations.requiresApproval).toBe(true);
    expect(annotations.openWorld).toBe(false);
  });
});
