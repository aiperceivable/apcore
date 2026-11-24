import { describe, it, expect, beforeEach } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Registry } from '../src/registry/registry.js';
import { Config } from '../src/config.js';
import { InvalidInputError, ModuleNotFoundError } from '../src/errors.js';
import { ManifestModule, ManifestFullModule } from '../src/sys-modules/manifest.js';

/** Dummy module with all manifest-relevant properties. */
function createDummyModule(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    description: overrides['description'] ?? 'A test module',
    documentation: overrides['documentation'] ?? null,
    inputSchema: overrides['inputSchema'] ?? Type.Object({ name: Type.String() }),
    outputSchema: overrides['outputSchema'] ?? Type.Object({ result: Type.Boolean() }),
    tags: overrides['tags'] ?? ['test'],
    annotations: overrides['annotations'] ?? {
      readonly: false,
      destructive: false,
      idempotent: true,
      requiresApproval: false,
      openWorld: false,
      streaming: false,
    },
    execute: overrides['execute'] ?? (() => ({ result: true })),
    metadata: overrides['metadata'] ?? {},
  };
}

function createConfigWithSourceRoot(): Config {
  return new Config({ project: { name: 'test', source_root: '/src' } });
}

describe('ManifestModule', () => {
  let registry: Registry;

  beforeEach(() => {
    registry = new Registry();
    registry.registerInternal(
      'system.test_mod',
      createDummyModule({ description: 'Test module description' }),
    );
  });

  it('throws InvalidInputError when module_id is missing', () => {
    const manifest = new ManifestModule(registry);
    expect(() => manifest.execute({}, null)).toThrow(InvalidInputError);
  });

  it('throws InvalidInputError when module_id is empty string', () => {
    const manifest = new ManifestModule(registry);
    expect(() => manifest.execute({ module_id: '' }, null)).toThrow(InvalidInputError);
  });

  it('throws InvalidInputError when module_id is not a string', () => {
    const manifest = new ManifestModule(registry);
    expect(() => manifest.execute({ module_id: 42 }, null)).toThrow(InvalidInputError);
  });

  it('throws ModuleNotFoundError for unknown module', () => {
    const manifest = new ManifestModule(registry);
    expect(() => manifest.execute({ module_id: 'system.nonexistent' }, null)).toThrow(
      ModuleNotFoundError,
    );
  });

  it('returns full manifest with all expected fields', () => {
    const inputSchema = Type.Object({ query: Type.String() });
    const outputSchema = Type.Object({ data: Type.Number() });
    const annotations = {
      readonly: true,
      destructive: false,
      idempotent: true,
      requiresApproval: false,
      openWorld: false,
      streaming: false,
    };
    registry.registerInternal(
      'system.detailed',
      createDummyModule({
        description: 'Detailed module',
        inputSchema,
        outputSchema,
        tags: ['search', 'query'],
        annotations,
      }),
    );

    const manifest = new ManifestModule(registry);
    const result = manifest.execute({ module_id: 'system.detailed' }, null);

    expect(result['module_id']).toBe('system.detailed');
    expect(result['description']).toBe('Detailed module');
    expect(result['input_schema']).toBe(inputSchema);
    expect(result['output_schema']).toBe(outputSchema);
    // SYS-5: annotations are emitted in the snake_case WIRE shape, as
    // apcore-python and apcore-rust emit them and as system-modules.md's own
    // output example shows. The camelCase struct stays internal.
    expect(result['annotations']).toMatchObject({
      readonly: true,
      destructive: false,
      idempotent: true,
      requires_approval: false,
      open_world: false,
      streaming: false,
    });
    expect(result['tags']).toEqual(['search', 'query']);
    expect(result['metadata']).toEqual({});
  });

  it('includes a dependencies field (empty by default)', () => {
    const manifest = new ManifestModule(registry);
    const result = manifest.execute({ module_id: 'system.test_mod' }, null);
    expect(result['dependencies']).toEqual([]);
  });

  it('surfaces dependencies declared in module metadata', () => {
    const deps = [{ module_id: 'system.other', version: '>=1.0.0' }];
    registry.registerInternal(
      'system.with_deps',
      createDummyModule({ metadata: { dependencies: deps } }),
    );
    const manifest = new ManifestModule(registry);
    const result = manifest.execute({ module_id: 'system.with_deps' }, null);
    expect(result['dependencies']).toEqual(deps);
  });

  it('includes source_path when config has project.source_root', () => {
    const config = createConfigWithSourceRoot();
    const manifest = new ManifestModule(registry, config);
    const result = manifest.execute({ module_id: 'system.test_mod' }, null);

    expect(result['source_path']).toBe('/src/system/test_mod.ts');
  });

  it('source_path is null when no config is provided', () => {
    const manifest = new ManifestModule(registry);
    const result = manifest.execute({ module_id: 'system.test_mod' }, null);

    expect(result['source_path']).toBeNull();
  });

  it('source_path is null when config has no project.source_root', () => {
    const config = new Config({ project: { name: 'test' } });
    const manifest = new ManifestModule(registry, config);
    const result = manifest.execute({ module_id: 'system.test_mod' }, null);

    expect(result['source_path']).toBeNull();
  });

  it('converts dots in module_id to path separators for source_path', () => {
    const config = createConfigWithSourceRoot();
    registry.registerInternal('system.sub.deep_mod', createDummyModule());
    const manifest = new ManifestModule(registry, config);
    const result = manifest.execute({ module_id: 'system.sub.deep_mod' }, null);

    expect(result['source_path']).toBe('/src/system/sub/deep_mod.ts');
  });
});

describe('ManifestFullModule', () => {
  let registry: Registry;

  beforeEach(() => {
    registry = new Registry();
    registry.registerInternal(
      'system.alpha',
      createDummyModule({ description: 'Alpha module', tags: ['core', 'search'] }),
    );
    registry.registerInternal(
      'system.beta',
      createDummyModule({ description: 'Beta module', tags: ['core'] }),
    );
    registry.registerInternal(
      'system.gamma',
      createDummyModule({ description: 'Gamma module', tags: ['analytics'] }),
    );
  });

  it('returns all modules with module_count', () => {
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({}, null);

    expect(result['module_count']).toBe(3);
    const modules = result['modules'] as Record<string, unknown>[];
    expect(modules).toHaveLength(3);

    const ids = modules.map((m) => m['module_id']);
    expect(ids).toContain('system.alpha');
    expect(ids).toContain('system.beta');
    expect(ids).toContain('system.gamma');
  });

  it('includes project_name from config', () => {
    const config = createConfigWithSourceRoot();
    const manifest = new ManifestFullModule(registry, config);
    const result = manifest.execute({}, null);

    expect(result['project_name']).toBe('test');
  });

  it('project_name defaults to "apcore" when no config (D-110)', () => {
    // This asserted `''` until spec v1.51.0. It was pinning the half of a
    // contradiction: `system.health.summary` already reported `"apcore"` in the
    // same process, so the two system modules disagreed about the same fact.
    // D-110 settled it on that internal consistency rather than an SDK majority.
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({}, null);

    expect(result['project_name']).toBe('apcore');
  });

  it('filters by prefix', () => {
    registry.registerInternal('system.sub.one', createDummyModule());
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({ prefix: 'system.sub' }, null);

    const modules = result['modules'] as Record<string, unknown>[];
    expect(result['module_count']).toBe(1);
    expect(modules[0]['module_id']).toBe('system.sub.one');
  });

  it('filters by tags', () => {
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({ tags: ['search'] }, null);

    const modules = result['modules'] as Record<string, unknown>[];
    expect(result['module_count']).toBe(1);
    expect(modules[0]['module_id']).toBe('system.alpha');
  });

  it('filters by multiple tags requiring all to match', () => {
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({ tags: ['core', 'search'] }, null);

    const modules = result['modules'] as Record<string, unknown>[];
    expect(result['module_count']).toBe(1);
    expect(modules[0]['module_id']).toBe('system.alpha');
  });

  it('returns empty when no modules match tags', () => {
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({ tags: ['nonexistent'] }, null);

    expect(result['module_count']).toBe(0);
    expect(result['modules']).toEqual([]);
  });

  it('excludes schemas when include_schemas is false', () => {
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({ include_schemas: false }, null);

    const modules = result['modules'] as Record<string, unknown>[];
    for (const mod of modules) {
      expect(mod['input_schema']).toBeNull();
      expect(mod['output_schema']).toBeNull();
    }
  });

  it('includes schemas by default', () => {
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({}, null);

    const modules = result['modules'] as Record<string, unknown>[];
    for (const mod of modules) {
      expect(mod['input_schema']).not.toBeNull();
      expect(mod['output_schema']).not.toBeNull();
    }
  });

  it('excludes source_paths when include_source_paths is false', () => {
    const config = createConfigWithSourceRoot();
    const manifest = new ManifestFullModule(registry, config);
    const result = manifest.execute({ include_source_paths: false }, null);

    const modules = result['modules'] as Record<string, unknown>[];
    for (const mod of modules) {
      expect(mod['source_path']).toBeNull();
    }
  });

  it('includes source_paths by default when config has source_root', () => {
    const config = createConfigWithSourceRoot();
    const manifest = new ManifestFullModule(registry, config);
    const result = manifest.execute({}, null);

    const modules = result['modules'] as Record<string, unknown>[];
    for (const mod of modules) {
      expect(mod['source_path']).not.toBeNull();
      expect(mod['source_path']).toMatch(/^\/src\//);
    }
  });

  it('combines prefix and tags filters', () => {
    registry.registerInternal(
      'system.sub.tagged',
      createDummyModule({ tags: ['analytics'] }),
    );
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({ prefix: 'system.sub', tags: ['analytics'] }, null);

    const modules = result['modules'] as Record<string, unknown>[];
    expect(result['module_count']).toBe(1);
    expect(modules[0]['module_id']).toBe('system.sub.tagged');
  });

  it('each module entry includes all expected fields', () => {
    const manifest = new ManifestFullModule(registry);
    const result = manifest.execute({}, null);

    const modules = result['modules'] as Record<string, unknown>[];
    for (const mod of modules) {
      expect(mod).toHaveProperty('module_id');
      expect(mod).toHaveProperty('description');
      expect(mod).toHaveProperty('documentation');
      expect(mod).toHaveProperty('source_path');
      expect(mod).toHaveProperty('input_schema');
      expect(mod).toHaveProperty('output_schema');
      expect(mod).toHaveProperty('annotations');
      expect(mod).toHaveProperty('tags');
      expect(mod).toHaveProperty('metadata');
    }
  });
});
