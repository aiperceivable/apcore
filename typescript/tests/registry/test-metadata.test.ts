import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import {
  loadMetadata,
  parseDependencies,
  mergeModuleMetadata,
  loadIdMap,
} from '../../src/registry/metadata.js';
import { ConfigError, ConfigNotFoundError } from '../../src/errors.js';
import { createAnnotations } from '../../src/module.js';
import { Registry } from '../../src/registry/registry.js';
import type { ModuleAnnotations } from '../../src/module.js';

describe('loadMetadata', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'metadata-test-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('returns empty object for non-existent file', () => {
    const result = loadMetadata(join(tmpDir, 'does_not_exist.yaml'));
    expect(result).toEqual({});
  });

  it('parses valid YAML and returns record', () => {
    const metaPath = join(tmpDir, '_meta.yaml');
    writeFileSync(metaPath, 'description: hello\nversion: "2.0.0"\ntags:\n  - alpha\n  - beta\n');
    const result = loadMetadata(metaPath);
    expect(result).toEqual({
      description: 'hello',
      version: '2.0.0',
      tags: ['alpha', 'beta'],
    });
  });

  it('returns empty object for null YAML content', () => {
    const metaPath = join(tmpDir, '_meta.yaml');
    writeFileSync(metaPath, '');
    const result = loadMetadata(metaPath);
    expect(result).toEqual({});
  });

  it('returns empty object for YAML file containing only null', () => {
    const metaPath = join(tmpDir, '_meta.yaml');
    writeFileSync(metaPath, 'null\n');
    const result = loadMetadata(metaPath);
    expect(result).toEqual({});
  });

  it('throws ConfigError for invalid YAML syntax', () => {
    const metaPath = join(tmpDir, '_meta.yaml');
    writeFileSync(metaPath, ':\n  :\n    bad: {{{\n');
    expect(() => loadMetadata(metaPath)).toThrow(ConfigError);
  });

  it('throws ConfigError if YAML content is a list instead of mapping', () => {
    const metaPath = join(tmpDir, '_meta.yaml');
    writeFileSync(metaPath, '- item1\n- item2\n');
    expect(() => loadMetadata(metaPath)).toThrow(ConfigError);
    expect(() => loadMetadata(metaPath)).toThrow(/must be a YAML mapping/);
  });
});

describe('parseDependencies', () => {
  it('returns empty array for empty input', () => {
    expect(parseDependencies([])).toEqual([]);
  });

  it('returns empty array for null/undefined input', () => {
    expect(parseDependencies(null as unknown as Array<Record<string, unknown>>)).toEqual([]);
    expect(parseDependencies(undefined as unknown as Array<Record<string, unknown>>)).toEqual([]);
  });

  it('parses dependencies with moduleId, version, optional', () => {
    const raw = [
      { module_id: 'core.auth', version: '1.2.0', optional: true },
      { module_id: 'core.db', version: '3.0.0', optional: false },
    ];
    const result = parseDependencies(raw);
    expect(result).toEqual([
      { moduleId: 'core.auth', version: '1.2.0', optional: true },
      { moduleId: 'core.db', version: '3.0.0', optional: false },
    ]);
  });

  it('skips entries without module_id', () => {
    const raw = [
      { module_id: 'valid.module' },
      { version: '1.0.0' },
      { optional: true },
      {},
    ];
    const result = parseDependencies(raw);
    expect(result).toHaveLength(1);
    expect(result[0].moduleId).toBe('valid.module');
  });

  it('defaults version to null and optional to false', () => {
    const raw = [{ module_id: 'some.module' }];
    const result = parseDependencies(raw);
    expect(result).toEqual([
      { moduleId: 'some.module', version: null, optional: false },
    ]);
  });

  it('handles mixed entries with partial fields', () => {
    const raw = [
      { module_id: 'a', version: '1.0.0' },
      { module_id: 'b', optional: true },
      { module_id: 'c' },
    ];
    const result = parseDependencies(raw);
    expect(result).toEqual([
      { moduleId: 'a', version: '1.0.0', optional: false },
      { moduleId: 'b', version: null, optional: true },
      { moduleId: 'c', version: null, optional: false },
    ]);
  });
});

describe('mergeModuleMetadata', () => {
  it('YAML values win over code values for scalar top-level fields', () => {
    const moduleObj = {
      description: 'code desc',
      name: 'code-name',
      tags: ['code-tag'],
      version: '1.0.0',
      documentation: 'code docs',
      metadata: { codeKey: 'codeVal' },
    };
    const meta = {
      description: 'yaml desc',
      name: 'yaml-name',
      tags: ['yaml-tag'],
      version: '2.0.0',
      documentation: 'yaml docs',
      metadata: { yamlKey: 'yamlVal' },
    };
    const result = mergeModuleMetadata(moduleObj, meta);
    expect(result['description']).toBe('yaml desc');
    expect(result['name']).toBe('yaml-name');
    expect(result['tags']).toEqual(['yaml-tag']);
    expect(result['version']).toBe('2.0.0');
    expect(result['documentation']).toBe('yaml docs');
  });

  it('code values used as fallback when YAML is empty', () => {
    const moduleObj = {
      description: 'code desc',
      name: 'code-name',
      tags: ['code-tag'],
      version: '1.0.0',
      metadata: { codeKey: 'codeVal' },
    };
    const meta: Record<string, unknown> = {};
    const result = mergeModuleMetadata(moduleObj, meta);
    expect(result['description']).toBe('code desc');
    expect(result['name']).toBe('code-name');
    expect(result['tags']).toEqual(['code-tag']);
    expect(result['version']).toBe('1.0.0');
    expect(result['metadata']).toEqual({ codeKey: 'codeVal' });
  });

  it('metadata records are shallow-merged with YAML spread over code', () => {
    const moduleObj = { metadata: { shared: 'from-code', codeOnly: 'value' } };
    const meta = { metadata: { shared: 'from-yaml', yamlOnly: 'value' } };
    const result = mergeModuleMetadata(moduleObj, meta);
    expect(result['metadata']).toEqual({
      shared: 'from-yaml',
      codeOnly: 'value',
      yamlOnly: 'value',
    });
  });

  it('default values used when both code and YAML are absent', () => {
    const result = mergeModuleMetadata({}, {});
    expect(result['description']).toBe('');
    expect(result['name']).toBeNull();
    expect(result['tags']).toEqual([]);
    expect(result['version']).toBe('1.0.0');
    expect(result['annotations']).toBeNull();
    expect(result['examples']).toEqual([]);
    expect(result['metadata']).toEqual({});
    expect(result['documentation']).toBeNull();
    expect(result['dependencies']).toEqual([]);
  });

  // --- `dependencies` (issue #35) --------------------------------------------
  // The merge used to build the stored metadata from a fixed eight-key list
  // that omitted `dependencies`. Discovery reads dependencies off the raw YAML
  // *before* merging, so load ordering was unaffected and the gap only showed
  // up post-registration — where `ReloadModule` reads it to order a bulk
  // reload. Same three cases as every other scalar field.

  it('code-declared dependencies survive the merge when YAML declares none', () => {
    const moduleObj = { dependencies: [{ module_id: 'common.db' }] };
    const result = mergeModuleMetadata(moduleObj, {});
    expect(result['dependencies']).toEqual([{ module_id: 'common.db' }]);
  });

  it('YAML dependencies win over code dependencies', () => {
    const moduleObj = { dependencies: [{ module_id: 'common.db' }] };
    const meta = { dependencies: [{ module_id: 'common.cache', optional: true }] };
    const result = mergeModuleMetadata(moduleObj, meta);
    expect(result['dependencies']).toEqual([{ module_id: 'common.cache', optional: true }]);
  });

  it('YAML empty array for dependencies overrides code dependencies', () => {
    // Mirrors the `tags` rule: an explicit empty YAML list is a deliberate
    // "this module depends on nothing", not an absent value.
    const moduleObj = { dependencies: [{ module_id: 'common.db' }] };
    const result = mergeModuleMetadata(moduleObj, { dependencies: [] });
    expect(result['dependencies']).toEqual([]);
  });

  it('dependencies default to an empty array when neither side declares them', () => {
    expect(mergeModuleMetadata({ description: 'x' }, {})['dependencies']).toEqual([]);
  });

  it('a non-array code dependencies value degrades to an empty array', () => {
    const result = mergeModuleMetadata({ dependencies: 'common.db' }, {});
    expect(result['dependencies']).toEqual([]);
  });

  it('YAML empty array for tags overrides code tags', () => {
    const moduleObj = { tags: ['code-tag'] };
    const meta = { tags: [] };
    const result = mergeModuleMetadata(moduleObj, meta);
    expect(result['tags']).toEqual([]);
  });

  // ---------------------------------------------------------------------------
  // PROTOCOL_SPEC.md §4.13 — Annotation Conflict Rules
  //
  // YAML annotations MUST be merged field-by-field over code annotations.
  // A YAML override that flips one flag must NOT blow away unrelated flags
  // set on the code-level annotations object. (Regression: prior to wiring
  // schema.mergeAnnotations into mergeModuleMetadata, the YAML annotation
  // dict was passed through verbatim, silently dropping every code-set
  // flag the YAML did not also set.)
  // ---------------------------------------------------------------------------

  it('annotations are field-level merged per spec §4.13', () => {
    const moduleObj = {
      description: 'd',
      annotations: createAnnotations({ readonly: true, idempotent: true }),
    };
    // YAML only overrides `destructive`; readonly/idempotent must survive.
    const meta = { annotations: { destructive: true } };
    const result = mergeModuleMetadata(moduleObj, meta);
    const merged = result['annotations'] as ModuleAnnotations;
    expect(merged).not.toBeNull();
    expect(merged.destructive).toBe(true);
    expect(merged.readonly).toBe(true);
    expect(merged.idempotent).toBe(true);
  });

  it('YAML-only annotations are honored when no code annotations exist', () => {
    const moduleObj = { description: 'd' };
    const meta = { annotations: { readonly: true } };
    const result = mergeModuleMetadata(moduleObj, meta);
    const merged = result['annotations'] as ModuleAnnotations;
    expect(merged).not.toBeNull();
    expect(merged.readonly).toBe(true);
  });

  it('annotations are null when neither code nor YAML provide them', () => {
    const moduleObj = { description: 'd' };
    const meta: Record<string, unknown> = {};
    const result = mergeModuleMetadata(moduleObj, meta);
    expect(result['annotations']).toBeNull();
  });

  it('YAML examples take full priority over code examples per spec §4.13', () => {
    const moduleObj = {
      description: 'd',
      examples: [{ title: 'from_code', inputs: {}, output: {} }],
    };
    const meta = {
      examples: [{ title: 'from_yaml', inputs: { x: 1 }, output: { y: 2 } }],
    };
    const result = mergeModuleMetadata(moduleObj, meta);
    const examples = result['examples'] as Array<Record<string, unknown>>;
    expect(examples).toHaveLength(1);
    expect(examples[0].title).toBe('from_yaml');
    expect(examples[0].inputs).toEqual({ x: 1 });
  });

  it('unknown YAML annotation keys are silently dropped during merge', () => {
    const moduleObj = {
      description: 'd',
      annotations: createAnnotations({ readonly: true }),
    };
    // `vendor.foo` is not a canonical ModuleAnnotations field — it must
    // not appear on the merged result. Code-set readonly must survive.
    const meta = { annotations: { 'vendor.foo': 'bar' } };
    const result = mergeModuleMetadata(moduleObj, meta);
    const merged = result['annotations'] as ModuleAnnotations;
    expect(merged).not.toBeNull();
    expect(merged.readonly).toBe(true);
    expect((merged as unknown as Record<string, unknown>)['vendor.foo']).toBeUndefined();
  });
});

describe('loadIdMap', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'idmap-test-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('throws ConfigNotFoundError for non-existent file', () => {
    expect(() => loadIdMap(join(tmpDir, 'nonexistent.yaml'))).toThrow(ConfigNotFoundError);
  });

  it('throws ConfigError for invalid YAML syntax', () => {
    const idMapPath = join(tmpDir, 'id_map.yaml');
    writeFileSync(idMapPath, ':\n  bad: {{{\n');
    expect(() => loadIdMap(idMapPath)).toThrow(ConfigError);
  });

  it('throws ConfigError when mappings key is missing', () => {
    const idMapPath = join(tmpDir, 'id_map.yaml');
    writeFileSync(idMapPath, 'some_key: value\n');
    expect(() => loadIdMap(idMapPath)).toThrow(ConfigError);
  });

  it('throws ConfigError when mappings is not an array', () => {
    const idMapPath = join(tmpDir, 'id_map.yaml');
    writeFileSync(idMapPath, 'mappings:\n  key: value\n');
    expect(() => loadIdMap(idMapPath)).toThrow(ConfigError);
  });

  it('parses valid mappings into file -> id; an entry carries file and id only (D-138)', () => {
    const idMapPath = join(tmpDir, 'id_map.yaml');
    writeFileSync(
      idMapPath,
      ['mappings:', '  - file: module_a.ts', '    id: custom.module.a', '    class: ModuleA', '  - file: module_b.ts', '    id: custom.module.b', ''].join('\n'),
    );
    const result = loadIdMap(idMapPath);
    expect(result['module_a.ts']).toEqual({ id: 'custom.module.a' });
    expect(result['module_b.ts']).toEqual({ id: 'custom.module.b' });
  });

  it('skips entries without file field', () => {
    const idMapPath = join(tmpDir, 'id_map.yaml');
    writeFileSync(
      idMapPath,
      ['mappings:', '  - file: valid.ts', '    id: valid.id', '  - id: orphan.id', ''].join('\n'),
    );
    const result = loadIdMap(idMapPath);
    expect(Object.keys(result)).toEqual(['valid.ts']);
  });

  it('handles empty mappings array', () => {
    const idMapPath = join(tmpDir, 'id_map.yaml');
    writeFileSync(idMapPath, 'mappings: []\n');
    const result = loadIdMap(idMapPath);
    expect(result).toEqual({});
  });
});

describe('caller extension metadata survives registration (sync finding A-D-003)', () => {
  // `mergeModuleMetadata` read only `meta['metadata']`, so the caller's own
  // top-level keys — the `x-` extension layer of the three-layer metadata
  // model — were dropped: `register(id, mod, null, { 'x-owner': 'billing' })`
  // produced `descriptor.metadata === {}`. apcore-rust stores the caller's map
  // verbatim on the descriptor and apcore-python surfaces it through its
  // versioned-metadata store, so this SDK was the only one losing it.
  const mod = {
    description: 'd',
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    execute: async () => ({}),
  };

  it('keeps x- prefixed extension keys', async () => {
    const registry = new Registry();
    await registry.register('a.b', mod, null, { 'x-owner': 'billing' });
    expect(registry.getDefinition('a.b')?.metadata).toMatchObject({ 'x-owner': 'billing' });
  });

  it('keeps arbitrary non-canonical keys', async () => {
    const registry = new Registry();
    await registry.register('a.c', mod, null, { foo: 1 });
    expect(registry.getDefinition('a.c')?.metadata).toMatchObject({ foo: 1 });
  });

  it('still honours a nested metadata block', async () => {
    const registry = new Registry();
    await registry.register('a.d', mod, null, { metadata: { bar: 2 } });
    expect(registry.getDefinition('a.d')?.metadata).toMatchObject({ bar: 2 });
  });

  it('does not leak canonical keys into metadata', async () => {
    const registry = new Registry();
    await registry.register('a.e', mod, null, { description: 'yaml-desc', tags: ['t'], foo: 1 });
    const d = registry.getDefinition('a.e');
    expect(d?.description).toBe('yaml-desc');
    expect(d?.tags).toEqual(['t']);
    // description/tags are extracted into their own descriptor fields, so
    // re-emitting them under `metadata` would duplicate them.
    expect(d?.metadata).toEqual({ foo: 1 });
  });
});

describe('descriptor carries typed dependencies (sync finding A-D-004)', () => {
  // PROTOCOL_SPEC §12.2 requires a `dependencies` entry in `metadata` to reach
  // the registered module's descriptor. apcore-rust carried a typed
  // Vec<DependencyInfo>; this SDK surfaced it on NEITHER `getDefinition()` nor
  // `descriptor.metadata` — only through `getModuleMetadata()`. The visible
  // consequence: `system.manifest.*` reported `dependencies: []` for every
  // module that declared them, because it read `descriptor.metadata`.
  const mod = {
    description: 'd',
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    execute: async () => ({}),
  };

  it('parses dependencies into typed objects', async () => {
    const registry = new Registry();
    await registry.register('a.b', mod, null, {
      dependencies: [{ module_id: 'a.dep', version: '^1.0', optional: true }],
    });
    expect(registry.getDefinition('a.b')?.dependencies).toEqual([
      { moduleId: 'a.dep', version: '^1.0', optional: true },
    ]);
  });

  it('is an empty array when none are declared', async () => {
    const registry = new Registry();
    await registry.register('a.c', mod);
    expect(registry.getDefinition('a.c')?.dependencies).toEqual([]);
  });

  it('agrees with the accessor reload ordering reads', async () => {
    const registry = new Registry();
    await registry.register('a.d', mod, null, { dependencies: [{ module_id: 'a.dep' }] });
    const fromDescriptor = registry.getDefinition('a.d')?.dependencies.map((d) => d.moduleId);
    const raw = (registry.getModuleMetadata('a.d')?.['dependencies'] ?? []) as Array<
      Record<string, unknown>
    >;
    expect(fromDescriptor).toEqual(raw.map((d) => d['module_id']));
    expect(fromDescriptor).toEqual(['a.dep']);
  });
});
