import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { Type, FormatRegistry } from '@sinclair/typebox';
import { Value } from '@sinclair/typebox/value';
import { Config } from '../../src/config.js';
import { SchemaNotFoundError, SchemaParseError } from '../../src/errors.js';
import { SchemaLoader, jsonSchemaToTypeBox, contentHash } from '../../src/schema/loader.js';
import { SchemaValidator } from '../../src/schema/validator.js';

describe('SchemaLoader', () => {
  let tmpDir: string;
  let schemasDir: string;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `apcore-test-loader-${Date.now()}-${Math.random().toString(36).slice(2)}`);
    schemasDir = join(tmpDir, 'schemas');
    mkdirSync(schemasDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  function writeSchema(relPath: string, content: string): void {
    const fullPath = join(schemasDir, relPath);
    const dir = fullPath.replace(/\/[^/]+$/, '');
    mkdirSync(dir, { recursive: true });
    writeFileSync(fullPath, content, 'utf-8');
  }

  function makeConfig(overrides?: Record<string, unknown>): Config {
    return new Config({
      schema: { root: schemasDir, strategy: 'yaml_first', ...overrides },
    });
  }

  describe('load', () => {
    it('loads a valid YAML schema file', () => {
      writeSchema('greeter.schema.yaml', `
description: A greeter module
input_schema:
  type: object
  properties:
    name:
      type: string
  required:
    - name
output_schema:
  type: object
  properties:
    message:
      type: string
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const sd = loader.load('greeter');

      expect(sd.moduleId).toBe('greeter');
      expect(sd.description).toBe('A greeter module');
      expect(sd.inputSchema).toEqual({
        type: 'object',
        properties: { name: { type: 'string' } },
        required: ['name'],
      });
      expect(sd.version).toBe('1.0.0');
    });

    it('loads a schema with dot-separated module ID (nested path)', () => {
      writeSchema('math/add.schema.yaml', `
description: Add numbers
input_schema:
  type: object
  properties:
    a:
      type: number
output_schema:
  type: object
  properties:
    result:
      type: number
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const sd = loader.load('math.add');

      expect(sd.moduleId).toBe('math.add');
      expect(sd.description).toBe('Add numbers');
    });

    it('throws SchemaNotFoundError for non-existent schema', () => {
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      expect(() => loader.load('nonexistent')).toThrow(SchemaNotFoundError);
    });

    it('throws SchemaParseError for invalid YAML', () => {
      writeSchema('bad.schema.yaml', '{ invalid yaml:: [');
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      expect(() => loader.load('bad')).toThrow(SchemaParseError);
    });

    it('throws SchemaParseError for empty file', () => {
      writeSchema('empty.schema.yaml', '');
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      expect(() => loader.load('empty')).toThrow(SchemaParseError);
    });

    it('throws SchemaParseError for array YAML', () => {
      writeSchema('arr.schema.yaml', '- item1\n- item2\n');
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      expect(() => loader.load('arr')).toThrow(SchemaParseError);
    });

    it('throws SchemaParseError when required field is missing', () => {
      writeSchema('noinput.schema.yaml', `
description: Missing input_schema
output_schema:
  type: object
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      expect(() => loader.load('noinput')).toThrow(SchemaParseError);
      expect(() => loader.load('noinput')).toThrow(/Missing required field/);
    });

    it('merges definitions and $defs', () => {
      writeSchema('withdefs.schema.yaml', `
description: Has definitions
input_schema:
  type: object
output_schema:
  type: object
definitions:
  Foo:
    type: string
$defs:
  Bar:
    type: integer
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const sd = loader.load('withdefs');
      expect(sd.definitions).toEqual({ Foo: { type: 'string' }, Bar: { type: 'integer' } });
    });

    it('returns cached result on second call', () => {
      writeSchema('cached.schema.yaml', `
description: Cached
input_schema:
  type: object
output_schema:
  type: object
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const first = loader.load('cached');
      const second = loader.load('cached');
      expect(first).toBe(second);
    });

    it('preserves optional fields (version, documentation, errorSchema)', () => {
      writeSchema('full.schema.yaml', `
description: Full schema
version: "2.0.0"
documentation: "Some docs"
input_schema:
  type: object
output_schema:
  type: object
error_schema:
  type: object
  properties:
    code:
      type: string
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const sd = loader.load('full');
      expect(sd.version).toBe('2.0.0');
      expect(sd.documentation).toBe('Some docs');
      expect(sd.errorSchema).toEqual({ type: 'object', properties: { code: { type: 'string' } } });
    });
  });

  describe('resolve', () => {
    it('resolves a schema definition into TypeBox schemas', () => {
      writeSchema('resolvable.schema.yaml', `
description: Resolvable
input_schema:
  type: object
  properties:
    query:
      type: string
  required:
    - query
output_schema:
  type: object
  properties:
    result:
      type: string
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const sd = loader.load('resolvable');
      const [inputRs, outputRs] = loader.resolve(sd);

      expect(inputRs.moduleId).toBe('resolvable');
      expect(inputRs.direction).toBe('input');
      expect(Value.Check(inputRs.schema, { query: 'hello' })).toBe(true);
      expect(Value.Check(inputRs.schema, {})).toBe(false);

      expect(outputRs.moduleId).toBe('resolvable');
      expect(outputRs.direction).toBe('output');
      expect(Value.Check(outputRs.schema, { result: 'world' })).toBe(true);
    });
  });

  describe('getSchema', () => {
    const validYaml = `
description: Test module
input_schema:
  type: object
  properties:
    x:
      type: string
  required:
    - x
output_schema:
  type: object
  properties:
    y:
      type: string
`;

    it('uses yaml_first strategy and finds YAML', () => {
      writeSchema('mod.schema.yaml', validYaml);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const [inputRs, outputRs] = loader.getSchema('mod');

      expect(inputRs.direction).toBe('input');
      expect(outputRs.direction).toBe('output');
      expect(Value.Check(inputRs.schema, { x: 'hi' })).toBe(true);
    });

    it('uses yaml_first strategy and falls back to native when YAML not found', () => {
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const nativeInput = Type.Object({ a: Type.String() });
      const nativeOutput = Type.Object({ b: Type.Number() });

      const [inputRs, outputRs] = loader.getSchema('missing', nativeInput, nativeOutput);

      expect(inputRs.direction).toBe('input');
      expect(inputRs.moduleId).toBe('missing');
      expect(Value.Check(inputRs.schema, { a: 'test' })).toBe(true);
      expect(outputRs.direction).toBe('output');
      expect(Value.Check(outputRs.schema, { b: 42 })).toBe(true);
    });

    it('uses yaml_first strategy and throws when YAML not found and no native schemas', () => {
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      expect(() => loader.getSchema('missing')).toThrow(SchemaNotFoundError);
    });

    it('uses native_first strategy and prefers native when available', () => {
      writeSchema('native.schema.yaml', validYaml);
      const config = makeConfig({ strategy: 'native_first' });
      const loader = new SchemaLoader(config, schemasDir);

      const nativeInput = Type.Object({ custom: Type.Boolean() });
      const nativeOutput = Type.Object({ out: Type.Boolean() });

      const [inputRs] = loader.getSchema('native', nativeInput, nativeOutput);
      // Should use native, not YAML
      expect(Value.Check(inputRs.schema, { custom: true })).toBe(true);
      expect(Value.Check(inputRs.schema, { x: 'string' })).toBe(false);
    });

    it('uses native_first strategy and falls back to YAML when no native', () => {
      writeSchema('fallback.schema.yaml', validYaml);
      const config = makeConfig({ strategy: 'native_first' });
      const loader = new SchemaLoader(config, schemasDir);

      const [inputRs] = loader.getSchema('fallback');
      expect(Value.Check(inputRs.schema, { x: 'hi' })).toBe(true);
    });

    it('uses yaml_only strategy', () => {
      writeSchema('yamlonly.schema.yaml', validYaml);
      const config = makeConfig({ strategy: 'yaml_only' });
      const loader = new SchemaLoader(config, schemasDir);

      const [inputRs] = loader.getSchema('yamlonly');
      expect(Value.Check(inputRs.schema, { x: 'hi' })).toBe(true);
    });

    it('uses yaml_only strategy and throws when YAML not found', () => {
      const config = makeConfig({ strategy: 'yaml_only' });
      const loader = new SchemaLoader(config, schemasDir);

      const nativeInput = Type.Object({ a: Type.String() });
      const nativeOutput = Type.Object({ b: Type.Number() });
      // yaml_only ignores native schemas
      expect(() => loader.getSchema('nope', nativeInput, nativeOutput)).toThrow(SchemaNotFoundError);
    });

    it('caches getSchema results', () => {
      writeSchema('cacheme.schema.yaml', validYaml);
      const loader = new SchemaLoader(makeConfig(), schemasDir);

      const first = loader.getSchema('cacheme');
      const second = loader.getSchema('cacheme');
      expect(first).toBe(second);
    });

    it('shares compiled YAML schemas without sharing module identity', () => {
      writeSchema('first.schema.yaml', validYaml);
      writeSchema('second.schema.yaml', validYaml);
      const loader = new SchemaLoader(makeConfig(), schemasDir);

      const first = loader.getSchema('first');
      const second = loader.getSchema('second');

      expect(second.map((schema) => schema.moduleId)).toEqual(['second', 'second']);
      expect(second[0].schema).toBe(first[0].schema);
      expect(loader.getSchema('second')).toBe(second);
    });

    it('returns distinct output schemas for modules with identical input but different output (regression: cache hash collision)', () => {
      // Two modules: same input schema, different output schemas.
      // Before the fix, both resolved to the first module's output schema.
      writeSchema('mod-a.schema.yaml', `
description: Module A
input_schema:
  type: object
  properties:
    name:
      type: string
  required: [name]
output_schema:
  type: object
  properties:
    result_a:
      type: string
  required: [result_a]
`);
      writeSchema('mod-b.schema.yaml', `
description: Module B
input_schema:
  type: object
  properties:
    name:
      type: string
  required: [name]
output_schema:
  type: object
  properties:
    result_b:
      type: integer
  required: [result_b]
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);
      const [, outputA] = loader.getSchema('mod-a');
      const [, outputB] = loader.getSchema('mod-b');

      expect(outputA.moduleId).toBe('mod-a');
      expect(outputB.moduleId).toBe('mod-b');

      // Module A's output accepts strings, not integers
      expect(Value.Check(outputA.schema, { result_a: 'hello' })).toBe(true);
      expect(Value.Check(outputA.schema, { result_b: 42 })).toBe(false);

      // Module B's output accepts integers, not strings
      expect(Value.Check(outputB.schema, { result_b: 42 })).toBe(true);
      expect(Value.Check(outputB.schema, { result_a: 'hello' })).toBe(false);
    });
  });

  describe('clearCache', () => {
    it('clears all caches so next load/getSchema reloads from disk', () => {
      writeSchema('clearable.schema.yaml', `
description: Clearable
input_schema:
  type: object
  properties:
    v:
      type: string
output_schema:
  type: object
`);
      const loader = new SchemaLoader(makeConfig(), schemasDir);

      const sd1 = loader.load('clearable');
      expect(sd1.description).toBe('Clearable');

      // Overwrite file
      writeSchema('clearable.schema.yaml', `
description: Updated
input_schema:
  type: object
output_schema:
  type: object
`);

      // Without clear, returns cached
      const sd2 = loader.load('clearable');
      expect(sd2.description).toBe('Clearable');

      // After clear, reloads from disk
      loader.clearCache();
      const sd3 = loader.load('clearable');
      expect(sd3.description).toBe('Updated');
    });
  });

  describe('constructor', () => {
    it('uses config schema.root when schemasDir not provided', () => {
      writeSchema('fromconfig.schema.yaml', `
description: From config
input_schema:
  type: object
output_schema:
  type: object
`);
      const config = new Config({ schema: { root: schemasDir } });
      const loader = new SchemaLoader(config);
      const sd = loader.load('fromconfig');
      expect(sd.description).toBe('From config');
    });

    it('uses default ./schemas when config has no schema.root', () => {
      const config = new Config({});
      // This just constructs without error; actual path may not exist
      const loader = new SchemaLoader(config);
      expect(loader).toBeInstanceOf(SchemaLoader);
    });
  });
});

describe('JSON Schema boolean subschemas', () => {
  it('retains boolean constraints for properties, items, and references', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: {
        free: true,
        forbidden: false,
        empty: { type: 'array', items: false },
        absent: { $ref: '#/$defs/never' },
      },
      $defs: { never: false },
    });
    expect(Value.Check(schema, { free: 5, empty: [] })).toBe(true);
    expect(Value.Check(schema, { forbidden: 1 })).toBe(false);
    expect(Value.Check(schema, { empty: [1] })).toBe(false);
    expect(Value.Check(schema, { absent: 1 })).toBe(false);
  });

  it('requires declared properties whose names are inherited on plain objects', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', required: ['constructor'] });
    expect(Value.Check(schema, {})).toBe(false);
    expect(Value.Check(schema, { constructor: 'example' })).toBe(true);
  });

  it('preserves ref siblings and counts property annotations through referenced allOf branches', () => {
    const schema = jsonSchemaToTypeBox({
      $defs: { entry: { properties: { label: { type: 'string' } } } },
      allOf: [{ $ref: '#/$defs/entry' }],
      unevaluatedProperties: false,
    });
    expect(Value.Check(schema, { label: 'example' })).toBe(true);
    expect(Value.Check(schema, { label: 'example', other: true })).toBe(false);

    const refWithSibling = jsonSchemaToTypeBox({
      $defs: { entry: { properties: { label: { type: 'string' } } } },
      $ref: '#/$defs/entry',
      unevaluatedProperties: false,
    });
    expect(Value.Check(refWithSibling, { label: 'example' })).toBe(true);
    expect(Value.Check(refWithSibling, { label: 'example', other: true })).toBe(false);
  });
});

describe('jsonSchemaToTypeBox', () => {
  it('converts string type', () => {
    const schema = jsonSchemaToTypeBox({ type: 'string' });
    expect(Value.Check(schema, 'hello')).toBe(true);
    expect(Value.Check(schema, 123)).toBe(false);
  });

  it('converts integer type', () => {
    const schema = jsonSchemaToTypeBox({ type: 'integer' });
    expect(Value.Check(schema, 42)).toBe(true);
    expect(Value.Check(schema, 3.14)).toBe(false);
  });

  it('converts number type', () => {
    const schema = jsonSchemaToTypeBox({ type: 'number' });
    expect(Value.Check(schema, 3.14)).toBe(true);
    expect(Value.Check(schema, 'abc')).toBe(false);
  });

  it('converts boolean type', () => {
    const schema = jsonSchemaToTypeBox({ type: 'boolean' });
    expect(Value.Check(schema, true)).toBe(true);
    expect(Value.Check(schema, 'true')).toBe(false);
  });

  it('converts null type', () => {
    const schema = jsonSchemaToTypeBox({ type: 'null' });
    expect(Value.Check(schema, null)).toBe(true);
    expect(Value.Check(schema, undefined)).toBe(false);
  });

  it('converts object with properties', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: {
        name: { type: 'string' },
        age: { type: 'integer' },
      },
      required: ['name'],
    });
    expect(Value.Check(schema, { name: 'Alice', age: 30 })).toBe(true);
    expect(Value.Check(schema, { name: 'Alice' })).toBe(true);
    expect(Value.Check(schema, { age: 30 })).toBe(false);
  });

  it('converts array type', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'array',
      items: { type: 'string' },
    });
    expect(Value.Check(schema, ['a', 'b'])).toBe(true);
    expect(Value.Check(schema, [1, 2])).toBe(false);
  });

  it('converts enum', () => {
    const schema = jsonSchemaToTypeBox({ enum: ['a', 'b', 'c'] });
    expect(Value.Check(schema, 'a')).toBe(true);
    expect(Value.Check(schema, 'd')).toBe(false);
  });

  it('converts anyOf', () => {
    const schema = jsonSchemaToTypeBox({
      anyOf: [{ type: 'string' }, { type: 'number' }],
    });
    expect(Value.Check(schema, 'hello')).toBe(true);
    expect(Value.Check(schema, 42)).toBe(true);
    expect(Value.Check(schema, true)).toBe(false);
  });

  it('returns Unknown for unrecognized schema', () => {
    const schema = jsonSchemaToTypeBox({});
    expect(Value.Check(schema, 'anything')).toBe(true);
    expect(Value.Check(schema, 42)).toBe(true);
  });

  it('converts string with constraints', () => {
    const schema = jsonSchemaToTypeBox({ type: 'string', minLength: 2, maxLength: 5 });
    expect(Value.Check(schema, 'ab')).toBe(true);
    expect(Value.Check(schema, 'a')).toBe(false);
    expect(Value.Check(schema, 'abcdef')).toBe(false);
  });

  it('converts object without properties', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object' });
    expect(Value.Check(schema, { any: 'value' })).toBe(true);
  });

  it('converts array without items', () => {
    const schema = jsonSchemaToTypeBox({ type: 'array' });
    expect(Value.Check(schema, [1, 'two', true])).toBe(true);
  });

  it('converts oneOf and tags schema with ONEOF_MARKER', () => {
    const schema = jsonSchemaToTypeBox({
      oneOf: [
        { type: 'object', properties: { kind: { const: 'a' } }, required: ['kind'] },
        { type: 'object', properties: { kind: { const: 'b' } }, required: ['kind'] },
      ],
    });
    // The marker is used by SchemaValidator to apply exhaustive oneOf semantics
    expect((schema as Record<string, unknown>)['x-apcore-keyword']).toBe('oneOf');
  });

  it('converts allOf and validates all branches are satisfied', () => {
    const schema = jsonSchemaToTypeBox({
      allOf: [
        { type: 'object', properties: { name: { type: 'string' } }, required: ['name'] },
        { type: 'object', properties: { age: { type: 'integer' } }, required: ['age'] },
      ],
    });
    expect(Value.Check(schema, { name: 'Alice', age: 30 })).toBe(true);
    expect(Value.Check(schema, { name: 'Bob' })).toBe(false);
  });

  it('converts not keyword', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { status: { not: { const: 'deleted' } } },
      required: ['status'],
    });
    expect(Value.Check(schema, { status: 'active' })).toBe(true);
    expect(Value.Check(schema, { status: 'deleted' })).toBe(false);
  });

  it('converts const keyword', () => {
    const schema = jsonSchemaToTypeBox({ const: 'fixed' });
    expect(Value.Check(schema, 'fixed')).toBe(true);
    expect(Value.Check(schema, 'other')).toBe(false);
  });

  it('converts recursive schema with $id and $ref: "#"', () => {
    const schema = jsonSchemaToTypeBox({
      $id: 'TreeNode',
      type: 'object',
      properties: {
        value: { type: 'string' },
        children: { type: 'array', items: { $ref: '#' } },
      },
      required: ['value'],
    });
    const root = { value: 'root', children: [{ value: 'child', children: [{ value: 'leaf' }] }] };
    expect(Value.Check(schema, root)).toBe(true);
    expect(Value.Check(schema, { value: 42 })).toBe(false);
  });

  it('converts numeric constraints (minimum, maximum, exclusiveMinimum)', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: {
        score: { type: 'integer', minimum: 1, maximum: 100 },
        positive: { type: 'number', exclusiveMinimum: 0 },
      },
      required: ['score', 'positive'],
    });
    expect(Value.Check(schema, { score: 50, positive: 0.1 })).toBe(true);
    expect(Value.Check(schema, { score: 0, positive: 0.1 })).toBe(false);
    expect(Value.Check(schema, { score: 50, positive: 0 })).toBe(false);
  });

  it('carries an unrecognised format through without touching the global registry', () => {
    // `format` is an annotation (apexe#32), so the value must survive the
    // conversion. Neutralising it for TypeBox's structural check belongs to
    // SchemaValidator, which scopes the override to one check — the converter
    // must NOT mutate the process-global FormatRegistry (see schema/formats.ts).
    const schema = jsonSchemaToTypeBox({ type: 'string', format: 'loader-probe-format' });
    expect((schema as Record<string, unknown>)['format']).toBe('loader-probe-format');
    expect(FormatRegistry.Has('loader-probe-format')).toBe(false);
    expect(new SchemaValidator(false).validate('/tmp/z' as never, schema).valid).toBe(true);
  });
});

describe('jsonSchemaToTypeBox — keywords apexe contracts rely on', () => {
  it('enforces additionalProperties: false', () => {
    // apcore-python maps this to pydantic extra="forbid" and apcore-rust
    // delegates to the jsonschema crate; both reject the unexpected key.
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string' } },
      required: ['a'],
      additionalProperties: false,
    });
    expect(Value.Check(schema, { a: 'x' })).toBe(true);
    expect(Value.Check(schema, { a: 'x', b: 'y' })).toBe(false);
  });

  it('allows unknown properties when additionalProperties is absent or true', () => {
    const open = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string' } },
      required: ['a'],
    });
    expect(Value.Check(open, { a: 'x', b: 'y' })).toBe(true);
    const explicit = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string' } },
      required: ['a'],
      additionalProperties: true,
    });
    expect(Value.Check(explicit, { a: 'x', b: 'y' })).toBe(true);
  });

  it('constrains unknown properties to an additionalProperties sub-schema', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { a: { type: 'string' } },
      required: ['a'],
      additionalProperties: { type: 'integer' },
    });
    expect(Value.Check(schema, { a: 'x', count: 3 })).toBe(true);
    expect(Value.Check(schema, { a: 'x', count: 'three' })).toBe(false);
  });

  it('enforces additionalProperties: false on an object with no properties', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', additionalProperties: false });
    expect(Value.Check(schema, {})).toBe(true);
    expect(Value.Check(schema, { a: 1 })).toBe(false);
  });

  it('converts a type array to a union of its members', () => {
    // apexe emits ["string", "boolean"] for value-optional flags. Converting it
    // to `unknown` accepted every other type as well; apcore-rust rejects them.
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { flag: { type: ['string', 'boolean'] } },
      required: ['flag'],
    });
    expect(Value.Check(schema, { flag: 'value' })).toBe(true);
    expect(Value.Check(schema, { flag: true })).toBe(true);
    expect(Value.Check(schema, { flag: 42 })).toBe(false);
    expect(Value.Check(schema, { flag: { nested: 1 } })).toBe(false);
  });

  it('converts a nullable type array ["string", "null"]', () => {
    const schema = jsonSchemaToTypeBox({ type: ['string', 'null'] });
    expect(Value.Check(schema, 'x')).toBe(true);
    expect(Value.Check(schema, null)).toBe(true);
    expect(Value.Check(schema, 7)).toBe(false);
  });

  it('keeps per-branch constraints when converting a type array', () => {
    const schema = jsonSchemaToTypeBox({ type: ['string', 'null'], minLength: 3 });
    expect(Value.Check(schema, 'abc')).toBe(true);
    expect(Value.Check(schema, 'ab')).toBe(false);
    expect(Value.Check(schema, null)).toBe(true);
  });

  it('keeps an enum sibling of a type array (apexe value-optional flag)', () => {
    // apexe emits exactly this for `ls --color[=WHEN]`. Converting only the
    // `type` half let `--color=bogus` through; apcore-rust and apcore-python
    // both reject it because `type` and `enum` are independent assertions.
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: {
        color: {
          type: ['string', 'boolean'],
          enum: ['always', 'auto', 'never'],
          description: 'colorize the output',
        },
      },
      required: ['color'],
      additionalProperties: false,
    });
    expect(Value.Check(schema, { color: 'always' })).toBe(true);
    expect(Value.Check(schema, { color: 'never' })).toBe(true);
    expect(Value.Check(schema, { color: 'bogus-not-in-enum' })).toBe(false);
    // `true` is in the declared type union but not in the enum.
    expect(Value.Check(schema, { color: true })).toBe(false);
    expect(Value.Check(schema, { color: 42 })).toBe(false);
  });

  it('keeps an enum whose members span every branch of a type array', () => {
    const schema = jsonSchemaToTypeBox({ type: ['string', 'boolean'], enum: ['a', true] });
    expect(Value.Check(schema, 'a')).toBe(true);
    expect(Value.Check(schema, true)).toBe(true);
    expect(Value.Check(schema, 'b')).toBe(false);
    expect(Value.Check(schema, false)).toBe(false);
    expect(Value.Check(schema, 1)).toBe(false);
  });

  it('keeps a const sibling of a type array', () => {
    const schema = jsonSchemaToTypeBox({ type: ['string', 'boolean'], const: 'a' });
    expect(Value.Check(schema, 'a')).toBe(true);
    expect(Value.Check(schema, true)).toBe(false);
    expect(Value.Check(schema, 'b')).toBe(false);
  });

  it('keeps an anyOf sibling of a type array', () => {
    const schema = jsonSchemaToTypeBox({
      type: ['string', 'integer'],
      anyOf: [{ type: 'string', minLength: 3 }, { type: 'integer', minimum: 10 }],
    });
    expect(Value.Check(schema, 'abc')).toBe(true);
    expect(Value.Check(schema, 12)).toBe(true);
    expect(Value.Check(schema, 'ab')).toBe(false);
    expect(Value.Check(schema, 5)).toBe(false);
    expect(Value.Check(schema, true)).toBe(false);
  });

  it('keeps an allOf sibling of a type array', () => {
    const schema = jsonSchemaToTypeBox({
      type: ['string', 'null'],
      allOf: [{ type: 'string', minLength: 2 }, { type: 'string', maxLength: 4 }],
    });
    expect(Value.Check(schema, 'abc')).toBe(true);
    expect(Value.Check(schema, 'a')).toBe(false);
    expect(Value.Check(schema, 'abcde')).toBe(false);
  });

  it('keeps a not sibling of a type array', () => {
    const schema = jsonSchemaToTypeBox({ type: ['string', 'boolean'], not: { const: 'banned' } });
    expect(Value.Check(schema, 'allowed')).toBe(true);
    expect(Value.Check(schema, true)).toBe(true);
    expect(Value.Check(schema, 'banned')).toBe(false);
    expect(Value.Check(schema, 42)).toBe(false);
  });

  it('keeps an enum sibling of a scalar type', () => {
    // Same independence rule with a non-array `type`.
    const schema = jsonSchemaToTypeBox({ type: 'string', enum: ['red', 'green'] });
    expect(Value.Check(schema, 'red')).toBe(true);
    expect(Value.Check(schema, 'blue')).toBe(false);
    expect(Value.Check(schema, 1)).toBe(false);
  });

  it('puts description and title on the union node, not on each branch', () => {
    const schema = jsonSchemaToTypeBox({
      type: ['string', 'boolean'],
      description: 'colorize the output',
      title: 'color',
    }) as Record<string, unknown>;
    expect(schema['description']).toBe('colorize the output');
    expect(schema['title']).toBe('color');
    for (const branch of schema['anyOf'] as Record<string, unknown>[]) {
      expect(branch['description']).toBeUndefined();
      expect(branch['title']).toBeUndefined();
    }
  });

  it('accepts a required property whose value is present but empty', () => {
    // `required` is about presence, not emptiness — apcore-python and
    // apcore-rust both accept "" and []. Locked so it cannot drift.
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { name: { type: 'string' }, items: { type: 'array', items: { type: 'string' } } },
      required: ['name', 'items'],
    });
    expect(Value.Check(schema, { name: '', items: [] })).toBe(true);
    expect(Value.Check(schema, { items: [] })).toBe(false);
  });
});

describe('jsonSchemaToTypeBox — non-scalar enum and const members', () => {
  it('accepts the exact object a const names and rejects a different one', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', const: { a: 1 } });
    expect(Value.Check(schema, { a: 1 })).toBe(true);
    expect(Value.Check(schema, { a: 2 })).toBe(false);
    expect(Value.Check(schema, {})).toBe(false);
  });

  it('accepts the exact object an enum lists and rejects a different one', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', enum: [{ a: 1 }, { b: 2 }] });
    expect(Value.Check(schema, { a: 1 })).toBe(true);
    expect(Value.Check(schema, { b: 2 })).toBe(true);
    expect(Value.Check(schema, { a: 2 })).toBe(false);
  });

  it('accepts the exact array an enum lists and rejects a reordered one', () => {
    const schema = jsonSchemaToTypeBox({ type: 'array', enum: [[1, 2]] });
    expect(Value.Check(schema, [1, 2])).toBe(true);
    expect(Value.Check(schema, [2, 1])).toBe(false);
    expect(Value.Check(schema, [1, 2, 3])).toBe(false);
  });

  it('compares a nested const by value, not by object identity', () => {
    const schema = jsonSchemaToTypeBox({ const: { a: { b: [1, { c: true }] } } });
    expect(Value.Check(schema, { a: { b: [1, { c: true }] } })).toBe(true);
    expect(Value.Check(schema, { a: { b: [1, { c: false }] } })).toBe(false);
  });

  it('ignores key order when comparing a const object', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', const: { a: 1, b: 2 } });
    expect(Value.Check(schema, { b: 2, a: 1 })).toBe(true);
  });

  it('keeps the const keyword verbatim in the emitted schema', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', const: { a: 1 } });
    expect(JSON.parse(JSON.stringify(schema))).toMatchObject({
      allOf: [expect.anything(), { const: { a: 1 } }],
    });
  });

  it('still uses a scalar literal for a scalar const', () => {
    const schema = jsonSchemaToTypeBox({ const: 'fixed' });
    expect(Value.Check(schema, 'fixed')).toBe(true);
    expect(Value.Check(schema, 'other')).toBe(false);
    expect([...Value.Errors(schema, 'other')][0]?.message).toMatch(/fixed/);
  });
});

describe('jsonSchemaToTypeBox — every combinator sibling applies', () => {
  it('enforces a not sibling of an enum', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'string',
      enum: ['a', 'b'],
      not: { const: 'a' },
    });
    expect(Value.Check(schema, 'b')).toBe(true);
    expect(Value.Check(schema, 'a')).toBe(false);
    expect(Value.Check(schema, 'c')).toBe(false);
  });

  it('enforces an allOf sibling of an anyOf', () => {
    const schema = jsonSchemaToTypeBox({
      anyOf: [{ type: 'string' }],
      allOf: [{ type: 'string', minLength: 5 }],
    });
    expect(Value.Check(schema, 'abcde')).toBe(true);
    expect(Value.Check(schema, 'ab')).toBe(false);
  });

  it('enforces a const sibling of an enum', () => {
    const schema = jsonSchemaToTypeBox({ enum: ['a', 'b'], const: 'b' });
    expect(Value.Check(schema, 'b')).toBe(true);
    expect(Value.Check(schema, 'a')).toBe(false);
  });

  it('enforces an anyOf sibling of an oneOf', () => {
    // `oneOf` used to win the early-return race and drop the `anyOf` bound.
    const schema = jsonSchemaToTypeBox({
      anyOf: [{ type: 'integer', maximum: 20 }],
      oneOf: [{ type: 'integer', minimum: 10 }],
    });
    expect(Value.Check(schema, 12)).toBe(true);
    expect(Value.Check(schema, 5)).toBe(false); // fails the oneOf minimum
    expect(Value.Check(schema, 25)).toBe(false); // fails the anyOf maximum
  });

  it('enforces every sibling when four combinators coexist', () => {
    const schema = jsonSchemaToTypeBox({
      enum: ['aa', 'bbb', 'cccc'],
      anyOf: [{ type: 'string', minLength: 3 }],
      allOf: [{ type: 'string', maxLength: 3 }],
      not: { const: 'ccc' },
    });
    expect(Value.Check(schema, 'bbb')).toBe(true);
    expect(Value.Check(schema, 'aa')).toBe(false); // fails anyOf minLength
    expect(Value.Check(schema, 'cccc')).toBe(false); // fails allOf maxLength
    expect(Value.Check(schema, 'ccc')).toBe(false); // fails not, and is not an enum member
  });

  it('enforces the type half of a type array carrying an enum', () => {
    // The distinguishing case: `1` is an enum member but neither a string nor a
    // boolean, so the `type` half must reject it.
    const schema = jsonSchemaToTypeBox({ type: ['string', 'boolean'], enum: ['a', 1] });
    expect(Value.Check(schema, 'a')).toBe(true);
    expect(Value.Check(schema, 1)).toBe(false);
  });
});

describe('jsonSchemaToTypeBox — union keyword marker', () => {
  it('tags an anyOf union with the anyOf keyword', () => {
    const schema = jsonSchemaToTypeBox({ anyOf: [{ type: 'string' }, { type: 'number' }] });
    expect((schema as Record<string, unknown>)['x-apcore-keyword']).toBe('anyOf');
  });

  it('keeps the marker reachable when a type sibling nests the union', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      anyOf: [{ type: 'object' }],
    }) as Record<string, unknown>;
    const members = schema['allOf'] as Record<string, unknown>[];
    expect(members.some((m) => m['x-apcore-keyword'] === 'anyOf')).toBe(true);
  });

  it('leaves a plain TypeBox union unmarked', () => {
    expect((Type.Union([Type.String(), Type.Number()]) as Record<string, unknown>)['x-apcore-keyword'])
      .toBeUndefined();
  });
});

describe('jsonSchemaToTypeBox — array validation keywords (JSON Schema 2020-12 §6.4)', () => {
  it('keeps minItems', () => {
    const schema = jsonSchemaToTypeBox({ type: 'array', items: { type: 'integer' }, minItems: 2 });
    expect(Value.Check(schema, [1])).toBe(false);
    expect(Value.Check(schema, [1, 2])).toBe(true);
  });

  it('keeps maxItems', () => {
    const schema = jsonSchemaToTypeBox({ type: 'array', items: { type: 'integer' }, maxItems: 2 });
    expect(Value.Check(schema, [1, 2, 3])).toBe(false);
    expect(Value.Check(schema, [1, 2])).toBe(true);
  });

  it('keeps uniqueItems', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'array',
      items: { type: 'integer' },
      uniqueItems: true,
    });
    expect(Value.Check(schema, [1, 1])).toBe(false);
    expect(Value.Check(schema, [1, 2])).toBe(true);
  });

  it('keeps contains and converts its sub-schema', () => {
    const schema = jsonSchemaToTypeBox({ type: 'array', contains: { type: 'string', minLength: 2 } });
    expect(Value.Check(schema, [1, 2])).toBe(false);
    expect(Value.Check(schema, [1, 'a'])).toBe(false); // minLength inside contains applies
    expect(Value.Check(schema, [1, 'ab'])).toBe(true);
  });

  it('keeps minContains and maxContains beside a contains', () => {
    const min = jsonSchemaToTypeBox({ type: 'array', contains: { type: 'string' }, minContains: 2 });
    expect(Value.Check(min, [1, 'a'])).toBe(false);
    expect(Value.Check(min, ['a', 'b'])).toBe(true);

    const max = jsonSchemaToTypeBox({ type: 'array', contains: { type: 'string' }, maxContains: 1 });
    expect(Value.Check(max, ['a', 'b'])).toBe(false);
    expect(Value.Check(max, [1, 'a'])).toBe(true);
  });

  it('ignores minContains and maxContains without a contains', () => {
    // §6.4.4/§6.4.5 make them meaningless on their own. TypeBox applies them
    // anyway and rejects every array, so the converter must not emit them.
    expect(Value.Check(jsonSchemaToTypeBox({ type: 'array', minContains: 5 }), [1])).toBe(true);
    expect(Value.Check(jsonSchemaToTypeBox({ type: 'array', maxContains: 1 }), [1, 2, 3])).toBe(true);
  });

  it('keeps array keywords on a bare array with no items', () => {
    const schema = jsonSchemaToTypeBox({ type: 'array', minItems: 1 });
    expect(Value.Check(schema, [])).toBe(false);
    expect(Value.Check(schema, ['anything'])).toBe(true);
  });
});

describe('jsonSchemaToTypeBox — object validation keywords (JSON Schema 2020-12 §6.5)', () => {
  it('keeps minProperties alongside declared properties', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', properties: {}, minProperties: 1 });
    expect(Value.Check(schema, {})).toBe(false);
    expect(Value.Check(schema, { a: 1 })).toBe(true);
  });

  it('keeps maxProperties alongside declared properties', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', properties: {}, maxProperties: 1 });
    expect(Value.Check(schema, { a: 1, b: 2 })).toBe(false);
    expect(Value.Check(schema, { a: 1 })).toBe(true);
  });

  it('keeps minProperties on an object with no declared properties', () => {
    // This branch converts to Type.Record, which must carry the option too.
    const schema = jsonSchemaToTypeBox({ type: 'object', minProperties: 1 });
    expect(Value.Check(schema, {})).toBe(false);
    expect(Value.Check(schema, { a: 1 })).toBe(true);
  });

  it('keeps minProperties beside additionalProperties: false', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      additionalProperties: false,
      minProperties: 1,
    });
    expect(Value.Check(schema, {})).toBe(false);
  });
});

describe('jsonSchemaToTypeBox — a type array branch keeps its own constraints', () => {
  // The cross-SDK divergence that surfaced this: python and rust rejected [1],
  // TypeScript accepted it because minItems never reached the array branch.
  const nullableArray = {
    type: ['array', 'null'],
    items: { type: 'integer' },
    minItems: 2,
  };

  it('rejects an array shorter than minItems', () => {
    expect(Value.Check(jsonSchemaToTypeBox(nullableArray), [1])).toBe(false);
  });

  it('accepts an array satisfying minItems', () => {
    expect(Value.Check(jsonSchemaToTypeBox(nullableArray), [1, 2])).toBe(true);
  });

  it('accepts null, the other branch of the type array', () => {
    expect(Value.Check(jsonSchemaToTypeBox(nullableArray), null)).toBe(true);
  });

  it('routes minProperties to the object branch of a type array', () => {
    const schema = jsonSchemaToTypeBox({ type: ['object', 'null'], minProperties: 1 });
    expect(Value.Check(schema, {})).toBe(false);
    expect(Value.Check(schema, { a: 1 })).toBe(true);
    expect(Value.Check(schema, null)).toBe(true);
  });

  it('applies each branch constraint only to the type that owns it', () => {
    const schema = jsonSchemaToTypeBox({
      type: ['array', 'string'],
      minItems: 2,
      minLength: 4,
    });
    expect(Value.Check(schema, [1])).toBe(false);
    expect(Value.Check(schema, [1, 2])).toBe(true);
    expect(Value.Check(schema, 'abc')).toBe(false);
    expect(Value.Check(schema, 'abcd')).toBe(true);
  });
});

describe('jsonSchemaToTypeBox — a schema with constraints but no `type`', () => {
  // The constraint tables were only consulted from the type-specific
  // converters, so a schema declaring no `type` fell through to the combinator
  // path, found no combinator keyword, and became Type.Unknown() — every
  // constraint silently dropped. Expectations below match the `jsonschema`
  // reference implementation (Draft 2020-12).

  it('enforces minimum on a number', () => {
    const schema = jsonSchemaToTypeBox({ minimum: 3 });
    expect(Value.Check(schema, 5)).toBe(true);
    expect(Value.Check(schema, 1)).toBe(false);
  });

  // JSON Schema 2020-12 §6: a validation keyword constrains only instances of
  // its own type and is inert on all others. Narrowing `{minimum: 3}` to
  // "must be a number >= 3" would be stricter than the spec, not merely
  // different — these four are the guard against over-correcting.
  it.each([
    ['a string', 'x'],
    ['an array', [1]],
    ['null', null],
    ['a boolean', true],
    ['an object', { a: 1 }],
  ])('leaves a numeric constraint inert on %s', (_label, value) => {
    expect(Value.Check(jsonSchemaToTypeBox({ minimum: 3 }), value)).toBe(true);
  });

  it('enforces minLength on a string and stays inert on a number', () => {
    const schema = jsonSchemaToTypeBox({ minLength: 3 });
    expect(Value.Check(schema, 'abcd')).toBe(true);
    expect(Value.Check(schema, 'ab')).toBe(false);
    expect(Value.Check(schema, 42)).toBe(true);
  });

  it('enforces pattern on a string and stays inert on a number', () => {
    const schema = jsonSchemaToTypeBox({ pattern: '^a' });
    expect(Value.Check(schema, 'abc')).toBe(true);
    expect(Value.Check(schema, 'bbc')).toBe(false);
    expect(Value.Check(schema, 42)).toBe(true);
  });

  it('enforces minItems on an array and stays inert on a string', () => {
    const schema = jsonSchemaToTypeBox({ minItems: 2 });
    expect(Value.Check(schema, [1, 2])).toBe(true);
    expect(Value.Check(schema, [1])).toBe(false);
    expect(Value.Check(schema, 'ab')).toBe(true);
  });

  it('enforces uniqueItems on an array and stays inert on a string', () => {
    const schema = jsonSchemaToTypeBox({ uniqueItems: true });
    expect(Value.Check(schema, [1, 1])).toBe(false);
    expect(Value.Check(schema, 'aa')).toBe(true);
  });

  it('enforces minProperties on an object and stays inert on an array', () => {
    const schema = jsonSchemaToTypeBox({ minProperties: 1 });
    expect(Value.Check(schema, { a: 1 })).toBe(true);
    expect(Value.Check(schema, {})).toBe(false);
    expect(Value.Check(schema, [])).toBe(true);
  });

  it('enforces each group independently when several coexist', () => {
    const schema = jsonSchemaToTypeBox({ minimum: 3, minLength: 3 });
    expect(Value.Check(schema, 5)).toBe(true);
    expect(Value.Check(schema, 1)).toBe(false);
    expect(Value.Check(schema, 'abcd')).toBe(true);
    expect(Value.Check(schema, 'ab')).toBe(false);
    expect(Value.Check(schema, null)).toBe(true);
  });

  it('enforces a constraint alongside a combinator sibling', () => {
    const schema = jsonSchemaToTypeBox({ minimum: 3, not: { const: 5 } });
    expect(Value.Check(schema, 4)).toBe(true);
    expect(Value.Check(schema, 5)).toBe(false);
    expect(Value.Check(schema, 1)).toBe(false);
    expect(Value.Check(schema, 'x')).toBe(true);
  });

  it('keeps `format` an annotation rather than an assertion', () => {
    // §7.2.1 — the SHOULD-level warning is the validator's job, not a rejection.
    const result = new SchemaValidator(false).validate(
      'not-an-email' as never,
      jsonSchemaToTypeBox({ format: 'email' }),
    );
    expect(result.valid).toBe(true);
  });

  it('still converts a schema carrying no keyword at all to accept-anything', () => {
    const schema = jsonSchemaToTypeBox({});
    expect(Value.Check(schema, 1)).toBe(true);
    expect(Value.Check(schema, 'x')).toBe(true);
  });
});

describe('jsonSchemaToTypeBox — type-less constraints in every nested position', () => {
  // Every position that may hold a subschema recurses through the same
  // conversion entry point, so one fix covers them all. Each case here failed
  // before that fix.
  it('enforces them inside an anyOf branch', () => {
    const schema = jsonSchemaToTypeBox({ anyOf: [{ minLength: 3 }] });
    expect(Value.Check(schema, 'abcd')).toBe(true);
    expect(Value.Check(schema, 'ab')).toBe(false);
    expect(Value.Check(schema, 42)).toBe(true);
  });

  it('enforces them inside a oneOf branch', () => {
    const schema = jsonSchemaToTypeBox({ oneOf: [{ minimum: 3 }] });
    expect(Value.Check(schema, 5)).toBe(true);
    expect(Value.Check(schema, 1)).toBe(false);
  });

  it('enforces them inside an allOf branch', () => {
    const schema = jsonSchemaToTypeBox({ allOf: [{ minimum: 3 }] });
    expect(Value.Check(schema, 5)).toBe(true);
    expect(Value.Check(schema, 1)).toBe(false);
  });

  it('enforces them inside a `not` subschema', () => {
    const schema = jsonSchemaToTypeBox({ not: { minimum: 3 } });
    expect(Value.Check(schema, 1)).toBe(true);
    expect(Value.Check(schema, 5)).toBe(false);
    // "x" satisfies the inert `{minimum: 3}`, so `not` excludes it.
    expect(Value.Check(schema, 'x')).toBe(false);
  });

  it('enforces them inside an additionalProperties subschema', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      additionalProperties: { minimum: 3 },
    });
    expect(Value.Check(schema, { z: 5 })).toBe(true);
    expect(Value.Check(schema, { z: 1 })).toBe(false);
    expect(Value.Check(schema, { z: 'x' })).toBe(true);
  });

  it('enforces them inside a declared property subschema', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { v: { minLength: 3 } },
    });
    expect(Value.Check(schema, { v: 'abcd' })).toBe(true);
    expect(Value.Check(schema, { v: 'ab' })).toBe(false);
    expect(Value.Check(schema, { v: true })).toBe(true);
  });

  it('enforces them inside an items subschema', () => {
    const schema = jsonSchemaToTypeBox({ type: 'array', items: { minimum: 3 } });
    expect(Value.Check(schema, [5])).toBe(true);
    expect(Value.Check(schema, [1])).toBe(false);
    expect(Value.Check(schema, ['x'])).toBe(true);
  });

  it('enforces them inside a contains subschema', () => {
    const schema = jsonSchemaToTypeBox({ type: 'array', contains: { minLength: 3 } });
    expect(Value.Check(schema, ['abcd'])).toBe(true);
    expect(Value.Check(schema, ['ab'])).toBe(false);
  });
});

describe('jsonSchemaToTypeBox — applicator keywords (JSON Schema 2020-12 §10.3 / §11)', () => {
  it('checks each prefixItems position', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'array',
      prefixItems: [{ type: 'string' }, { type: 'integer' }],
    });
    expect(Value.Check(schema, ['a', 1])).toBe(true);
    expect(Value.Check(schema, ['a', 'b'])).toBe(false);
    // §10.3.1.1 constrains only the positions that exist.
    expect(Value.Check(schema, ['a'])).toBe(true);
  });

  it('applies `items` only past the prefix', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'array',
      prefixItems: [{ type: 'string' }],
      items: { type: 'integer' },
    });
    expect(Value.Check(schema, ['a', 3])).toBe(true);
    expect(Value.Check(schema, ['a', 'b'])).toBe(false);
    expect(Value.Check(schema, [1, 3])).toBe(false);
  });

  it('checks patternProperties values and leaves additionalProperties to the unmatched keys', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      patternProperties: { '^S_': { type: 'string' } },
      additionalProperties: false,
    });
    expect(Value.Check(schema, { S_a: 'x' })).toBe(true);
    expect(Value.Check(schema, { S_a: 1 })).toBe(false);
    expect(Value.Check(schema, { other: 1 })).toBe(false);
  });

  it('checks propertyNames against the key strings', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', propertyNames: { maxLength: 3 } });
    expect(Value.Check(schema, { ab: 1 })).toBe(true);
    expect(Value.Check(schema, { abcdef: 1 })).toBe(false);
  });

  it('enforces dependentRequired only when the trigger key is present', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', dependentRequired: { a: ['b'] } });
    expect(Value.Check(schema, { a: 1, b: 2 })).toBe(true);
    expect(Value.Check(schema, { a: 1 })).toBe(false);
    expect(Value.Check(schema, { c: 1 })).toBe(true);
  });

  it('applies a dependentSchemas subschema to the whole object', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      dependentSchemas: { a: { required: ['b'] } },
    });
    expect(Value.Check(schema, { a: 1, b: 2 })).toBe(true);
    expect(Value.Check(schema, { a: 1 })).toBe(false);
  });

  it('selects the then/else branch from the if outcome', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      if: { properties: { k: { const: 'x' } }, required: ['k'] },
      // biome-ignore lint/suspicious/noThenProperty: `then` is the JSON Schema keyword, not a thenable.
      then: { required: ['extra'] },
      else: { required: ['other'] },
    });
    expect(Value.Check(schema, { k: 'x', extra: 1 })).toBe(true);
    expect(Value.Check(schema, { k: 'x' })).toBe(false);
    expect(Value.Check(schema, { k: 'y', other: 1 })).toBe(true);
    expect(Value.Check(schema, { k: 'y' })).toBe(false);
  });

  it('asserts nothing for an `if` with no then/else', () => {
    const schema = jsonSchemaToTypeBox({ type: 'object', if: { required: ['k'] } });
    expect(Value.Check(schema, {})).toBe(true);
  });

  it('subtracts sibling annotations before applying unevaluatedProperties', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      allOf: [{ properties: { a: { type: 'string' } } }],
      unevaluatedProperties: false,
    });
    expect(Value.Check(schema, { a: 'x' })).toBe(true);
    expect(Value.Check(schema, { a: 'x', z: 1 })).toBe(false);
  });

  it('counts pattern matches toward unevaluatedProperties', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      patternProperties: { '^x-': { type: 'string' } },
      unevaluatedProperties: false,
    });
    expect(Value.Check(schema, { 'x-label': 'blue' })).toBe(true);
    expect(Value.Check(schema, { 'x-label': 'blue', stray: 'red' })).toBe(false);
  });

  it('counts properties only from triggered dependent schemas', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      properties: { trigger: { type: 'boolean' } },
      dependentSchemas: {
        trigger: { properties: { detail: { type: 'string' } } },
        unused: true,
      },
      unevaluatedProperties: false,
    });
    expect(Value.Check(schema, { trigger: true, detail: 'blue' })).toBe(true);
    expect(Value.Check(schema, { detail: 'blue' })).toBe(false);
  });

  it('counts properties already checked by additionalProperties', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'object',
      additionalProperties: { type: 'string' },
      unevaluatedProperties: false,
    });
    expect(Value.Check(schema, { extra: 'blue' })).toBe(true);
    expect(Value.Check(schema, { extra: 5 })).toBe(false);
  });

  it('counts the array tail already checked by items', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'array',
      prefixItems: [{ type: 'string' }],
      items: { type: 'number' },
      unevaluatedItems: false,
    });
    expect(Value.Check(schema, ['a', 2, 3])).toBe(true);
    expect(Value.Check(schema, ['a', 'b'])).toBe(false);
  });

  it('subtracts prefixItems positions before applying unevaluatedItems', () => {
    const schema = jsonSchemaToTypeBox({
      type: 'array',
      prefixItems: [{ type: 'string' }],
      unevaluatedItems: false,
    });
    expect(Value.Check(schema, ['a'])).toBe(true);
    expect(Value.Check(schema, ['a', 2])).toBe(false);
  });

  it('is inert on instances of the wrong type', () => {
    expect(Value.Check(jsonSchemaToTypeBox({ prefixItems: [{ type: 'string' }] }), 'x')).toBe(true);
    expect(Value.Check(jsonSchemaToTypeBox({ patternProperties: { '^S_': {} } }), 42)).toBe(true);
    expect(Value.Check(jsonSchemaToTypeBox({ propertyNames: { maxLength: 3 } }), [1, 2])).toBe(true);
    expect(Value.Check(jsonSchemaToTypeBox({ dependentRequired: { a: ['b'] } }), 's')).toBe(true);
  });

  it('enforces a bare `required` subschema, which if/then/else relies on', () => {
    const schema = jsonSchemaToTypeBox({ required: ['b'] });
    expect(Value.Check(schema, { b: 1 })).toBe(true);
    expect(Value.Check(schema, { a: 1 })).toBe(false);
    // §6.5.3 applies to objects only.
    expect(Value.Check(schema, 'x')).toBe(true);
  });
});
