import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Registry } from '../../src/registry/registry.js';
import { FunctionModule } from '../../src/decorator.js';
import { InvalidInputError, ModuleNotFoundError } from '../../src/errors.js';
import {
  getSchema,
  exportSchema,
  getAllSchemas,
  exportAllSchemas,
} from '../../src/registry/schema-export.js';

const inputSchema = Type.Object({
  prompt: Type.String({ description: 'The input prompt' }),
  temperature: Type.Optional(Type.Number({ description: 'Sampling temperature' })),
});

const outputSchema = Type.Object({
  text: Type.String(),
});

function createModule(
  id: string,
  overrides?: Partial<ConstructorParameters<typeof FunctionModule>[0]>,
): FunctionModule {
  return new FunctionModule({
    execute: () => ({ text: 'hello' }),
    moduleId: id,
    inputSchema,
    outputSchema,
    description: 'A test module. It does many things.\nSecond paragraph.',
    tags: ['ai', 'test'],
    version: '2.0.0',
    annotations: {
      readonly: true,
      destructive: false,
      idempotent: true,
      requiresApproval: false,
      openWorld: false,
      streaming: false,
      cacheable: false,
      cacheTtl: 0,
      cacheKeyFields: null,
      paginated: false,
      paginationStyle: 'cursor' as const, extra: {},
    },
    examples: [
      {
        title: 'Basic example',
        inputs: { prompt: 'hi' },
        output: { text: 'hello' },
        description: 'Simple greeting',
      },
    ],
    ...overrides,
  });
}

function makeRegistry(...modules: Array<[string, FunctionModule]>): Registry {
  const registry = new Registry();
  for (const [id, mod] of modules) {
    registry.register(id, mod);
  }
  return registry;
}

describe('getSchema', () => {
  it('returns null for unregistered module', () => {
    const registry = new Registry();
    expect(getSchema(registry, 'no.such.module')).toBeNull();
  });

  it('returns exactly the four keys of the canonical export envelope', () => {
    const mod = createModule('test.gen');
    const registry = makeRegistry(['test.gen', mod]);

    const schema = getSchema(registry, 'test.gen');
    expect(schema).not.toBeNull();
    // schemas/module-schema-export.schema.json — additionalProperties: false.
    expect(Object.keys(schema!).sort()).toEqual([
      'description',
      'input_schema',
      'module_id',
      'output_schema',
    ]);
    expect(schema!['module_id']).toBe('test.gen');
    expect(schema!['description']).toBe('A test module. It does many things.\nSecond paragraph.');
    expect(schema!['input_schema']).toBeDefined();
    expect(schema!['output_schema']).toBeDefined();
  });

  it('carries no descriptor metadata — that is system.manifest.module', () => {
    // These five rode along until the envelope was pinned, which made this a
    // partial, non-conforming duplicate of sys-manifest-module.schema.json and
    // left the three SDKs emitting three different shapes.
    const mod = createModule('test.meta');
    const registry = makeRegistry(['test.meta', mod]);
    const schema = getSchema(registry, 'test.meta')!;

    for (const key of ['name', 'version', 'tags', 'annotations', 'examples']) {
      expect(schema).not.toHaveProperty(key);
    }
  });
});

describe('exportSchema', () => {
  it('returns JSON string by default', () => {
    const mod = createModule('test.json');
    const registry = makeRegistry(['test.json', mod]);

    const result = exportSchema(registry, 'test.json');
    const parsed = JSON.parse(result);
    expect(parsed['module_id']).toBe('test.json');
    // No `version` — the export envelope carries schemas, not descriptor metadata.
    expect(parsed['version']).toBeUndefined();
  });

  it('returns YAML string when format is yaml', () => {
    const mod = createModule('test.yaml');
    const registry = makeRegistry(['test.yaml', mod]);

    const result = exportSchema(registry, 'test.yaml', 'yaml');
    expect(result).toContain('module_id:');
    expect(result).toContain('test.yaml');
  });

  it('throws ModuleNotFoundError for unregistered module', () => {
    const registry = new Registry();
    expect(() => exportSchema(registry, 'no.such.module')).toThrow(ModuleNotFoundError);
  });

  it('applies strict mode to input and output schemas', () => {
    const mod = createModule('test.strict');
    const registry = makeRegistry(['test.strict', mod]);

    const result = exportSchema(registry, 'test.strict', 'json', true);
    const parsed = JSON.parse(result);
    expect((parsed['input_schema'] as Record<string, unknown>)['additionalProperties']).toBe(false);
    expect((parsed['output_schema'] as Record<string, unknown>)['additionalProperties']).toBe(false);
  });

  it('compact mode truncates description at first sentence boundary', () => {
    const mod = createModule('test.compact');
    const registry = makeRegistry(['test.compact', mod]);

    const result = exportSchema(registry, 'test.compact', 'json', false, true);
    const parsed = JSON.parse(result);
    expect(parsed['description']).toBe('A test module.');
  });

  it('compact mode removes examples and documentation', () => {
    const mod = createModule('test.compact.ex', { documentation: 'Full docs' });
    const registry = makeRegistry(['test.compact.ex', mod]);

    const result = exportSchema(registry, 'test.compact.ex', 'json', false, true);
    const parsed = JSON.parse(result);
    expect(parsed['examples']).toBeUndefined();
    expect(parsed['documentation']).toBeUndefined();
  });

  it('strict takes precedence over compact when both are true', () => {
    const mod = createModule('test.both');
    const registry = makeRegistry(['test.both', mod]);

    const result = exportSchema(registry, 'test.both', 'json', true, true);
    const parsed = JSON.parse(result);
    expect((parsed['input_schema'] as Record<string, unknown>)['additionalProperties']).toBe(false);
    expect(parsed['description']).toBe('A test module. It does many things.\nSecond paragraph.');
  });
});

describe('getAllSchemas', () => {
  it('returns empty object for empty registry', () => {
    const registry = new Registry();
    expect(getAllSchemas(registry)).toEqual({});
  });

  it('returns all module schemas keyed by module id', () => {
    const modA = createModule('alpha');
    const modB = createModule('beta', { version: '3.0.0' });
    const registry = makeRegistry(['alpha', modA], ['beta', modB]);

    const result = getAllSchemas(registry);
    expect(Object.keys(result).sort()).toEqual(['alpha', 'beta']);
    expect(result['alpha']['module_id']).toBe('alpha');
    expect(result['beta']['module_id']).toBe('beta');
  });
});

describe('exportAllSchemas', () => {
  it('serializes all schemas to JSON', () => {
    const modA = createModule('a.mod');
    const modB = createModule('b.mod');
    const registry = makeRegistry(['a.mod', modA], ['b.mod', modB]);

    const result = exportAllSchemas(registry);
    const parsed = JSON.parse(result);
    expect(Object.keys(parsed).sort()).toEqual(['a.mod', 'b.mod']);
  });

  it('supports YAML format', () => {
    const mod = createModule('yaml.mod');
    const registry = makeRegistry(['yaml.mod', mod]);

    const result = exportAllSchemas(registry, 'yaml');
    expect(result).toContain('yaml.mod');
  });

  it('applies strict mode to all schemas', () => {
    const modA = createModule('strict.a');
    const registry = makeRegistry(['strict.a', modA]);

    const result = exportAllSchemas(registry, 'json', true);
    const parsed = JSON.parse(result);
    expect((parsed['strict.a']['input_schema'] as Record<string, unknown>)['additionalProperties']).toBe(false);
  });

  it('applies compact mode to all schemas', () => {
    const mod = createModule('compact.all');
    const registry = makeRegistry(['compact.all', mod]);

    const result = exportAllSchemas(registry, 'json', false, true);
    const parsed = JSON.parse(result);
    expect(parsed['compact.all']['description']).toBe('A test module.');
    expect(parsed['compact.all']['examples']).toBeUndefined();
  });

  it('returns empty JSON object for empty registry', () => {
    const registry = new Registry();
    expect(JSON.parse(exportAllSchemas(registry))).toEqual({});
  });
});

describe('exportSchema with profile', () => {
  it('exports with mcp profile and returns valid JSON with MCP shape', () => {
    const mod = createModule('mcp.mod');
    const registry = makeRegistry(['mcp.mod', mod]);

    const result = exportSchema(registry, 'mcp.mod', 'json', false, false, 'mcp');
    const parsed = JSON.parse(result);
    expect(parsed['name']).toBeDefined();
    expect(parsed['description']).toBeDefined();
    expect(parsed['inputSchema']).toBeDefined();
    expect(parsed['annotations']).toBeDefined();
    expect(parsed['annotations']['readOnlyHint']).toBe(true);
    expect(parsed['annotations']['destructiveHint']).toBe(false);
  });

  it('exports with openai profile and returns function tool shape', () => {
    const mod = createModule('openai.mod');
    const registry = makeRegistry(['openai.mod', mod]);

    const result = exportSchema(registry, 'openai.mod', 'json', false, false, 'openai');
    const parsed = JSON.parse(result);
    expect(parsed['type']).toBe('function');
    expect(parsed['function']).toBeDefined();
    expect(parsed['function']['name']).toBe('openai_mod');
    expect(parsed['function']['description']).toBeDefined();
    expect(parsed['function']['parameters']).toBeDefined();
    expect(parsed['function']['strict']).toBe(true);
  });

  it('exports with anthropic profile and returns tool shape with input_examples', () => {
    const mod = createModule('anthro.mod');
    const registry = makeRegistry(['anthro.mod', mod]);

    const result = exportSchema(registry, 'anthro.mod', 'json', false, false, 'anthropic');
    const parsed = JSON.parse(result);
    expect(parsed['name']).toBe('anthro_mod');
    expect(parsed['description']).toBeDefined();
    expect(parsed['input_schema']).toBeDefined();
    expect(parsed['input_examples']).toBeDefined();
    expect((parsed['input_examples'] as unknown[]).length).toBe(1);
  });

  it('exports with generic profile and returns module_id and schema fields', () => {
    const mod = createModule('generic.mod');
    const registry = makeRegistry(['generic.mod', mod]);

    const result = exportSchema(registry, 'generic.mod', 'json', false, false, 'generic');
    const parsed = JSON.parse(result);
    expect(parsed['module_id']).toBe('generic.mod');
    expect(parsed['description']).toBeDefined();
    expect(parsed['input_schema']).toBeDefined();
    expect(parsed['output_schema']).toBeDefined();
    expect(parsed['definitions']).toBeDefined();
  });

  it('exports with profile using yaml format', () => {
    const mod = createModule('mcp.yaml');
    const registry = makeRegistry(['mcp.yaml', mod]);

    const result = exportSchema(registry, 'mcp.yaml', 'yaml', false, false, 'mcp');
    expect(result).toContain('name:');
    expect(result).toContain('inputSchema:');
  });

  it('throws InvalidInputError for an unrecognized profile name', () => {
    const mod = createModule('bad.profile');
    const registry = makeRegistry(['bad.profile', mod]);

    expect(() =>
      exportSchema(registry, 'bad.profile', 'json', false, false, 'not_a_real_profile'),
    ).toThrow(InvalidInputError);
  });

  it('includes the invalid profile name in the error message', () => {
    const mod = createModule('err.profile');
    const registry = makeRegistry(['err.profile', mod]);

    expect(() =>
      exportSchema(registry, 'err.profile', 'json', false, false, 'bogus'),
    ).toThrowError(/bogus/);
  });
});

describe('Registry.exportSchema method', () => {
  it('returns a plain object matching the standalone getSchema result', () => {
    const mod = createModule('method.test');
    const registry = makeRegistry(['method.test', mod]);

    const methodResult = registry.exportSchema('method.test');
    const standaloneResult = JSON.parse(exportSchema(registry, 'method.test'));
    expect(JSON.parse(JSON.stringify(methodResult))).toEqual(standaloneResult);
  });

  it('supports strict mode', () => {
    const mod = createModule('method.strict');
    const registry = makeRegistry(['method.strict', mod]);

    const methodResult = registry.exportSchema('method.strict', true);
    const standaloneResult = JSON.parse(exportSchema(registry, 'method.strict', 'json', true));
    expect(methodResult).toEqual(standaloneResult);
    expect((methodResult!['input_schema'] as Record<string, unknown>)['additionalProperties']).toBe(false);
  });

  it('returns null for unregistered module', () => {
    const registry = new Registry();
    expect(registry.exportSchema('no.such.module')).toBeNull();
  });
});

describe('truncateDescription edge cases', () => {
  it('returns the full string when there is no dot-space or newline', () => {
    const mod = createModule('no.boundary', {
      description: 'A simple description with no sentence boundary',
    });
    const registry = makeRegistry(['no.boundary', mod]);

    const result = exportSchema(registry, 'no.boundary', 'json', false, true);
    const parsed = JSON.parse(result);
    expect(parsed['description']).toBe('A simple description with no sentence boundary');
  });

  it('truncates at the earlier boundary when both dot-space and newline are present', () => {
    // newline comes before dot-space: "Line one\nSecond sentence. More text."
    const mod = createModule('newline.first', {
      description: 'Line one\nSecond sentence. More text.',
    });
    const registry = makeRegistry(['newline.first', mod]);

    const result = exportSchema(registry, 'newline.first', 'json', false, true);
    const parsed = JSON.parse(result);
    expect(parsed['description']).toBe('Line one');
  });

  it('truncates at dot-space when it comes before a newline', () => {
    // dot-space before newline: "First sentence. Second line\nThird."
    const mod = createModule('dotspace.first', {
      description: 'First sentence. Second line\nThird.',
    });
    const registry = makeRegistry(['dotspace.first', mod]);

    const result = exportSchema(registry, 'dotspace.first', 'json', false, true);
    const parsed = JSON.parse(result);
    expect(parsed['description']).toBe('First sentence.');
  });
});
