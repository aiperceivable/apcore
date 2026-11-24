import { describe, it, expect, afterEach } from 'vitest';
import { Type } from '@sinclair/typebox';
import {
  SchemaExtractorRegistry,
  extractSchema,
  inferSchemasFromModule,
} from '../../src/schema/extractor.js';

const TYPEBOX_KIND = Symbol.for('TypeBox.Kind');

afterEach(() => {
  // Clean up any adapters registered during tests
  SchemaExtractorRegistry.unregister('test-adapter');
  SchemaExtractorRegistry.unregister('high-priority');
});

describe('extractSchema — TypeBox adapter', () => {
  it('returns null for null value', () => {
    expect(extractSchema(null)).toBeNull();
  });

  it('returns null for non-object value', () => {
    expect(extractSchema('string')).toBeNull();
    expect(extractSchema(42)).toBeNull();
  });

  it('returns null for plain object without TypeBox symbol or JSON Schema keys', () => {
    expect(extractSchema({ foo: 'bar' })).toBeNull();
  });

  it('extracts a TypeBox schema by its symbol', () => {
    const schema = Type.String();
    const result = extractSchema(schema);
    expect(result).toBe(schema);
  });
});

describe('extractSchema — JSON Schema adapter', () => {
  it('converts plain JSON Schema object with "type" key', () => {
    const jsonSchema = { type: 'string' };
    const result = extractSchema(jsonSchema);
    expect(result).not.toBeNull();
  });

  it('converts plain JSON Schema object with "properties" key', () => {
    const jsonSchema = { properties: { name: { type: 'string' } } };
    const result = extractSchema(jsonSchema);
    expect(result).not.toBeNull();
  });

  it('returns null for array input (not a JSON Schema)', () => {
    expect(extractSchema([{ type: 'string' }])).toBeNull();
  });

  it('does not match TypeBox object via json-schema adapter', () => {
    // TypeBox schemas have the TypeBox symbol AND "type" — should be handled by typebox adapter
    const schema = Type.String();
    expect(TYPEBOX_KIND in schema).toBe(true);
    const result = extractSchema(schema);
    // Still returns the schema (via typebox adapter, not json-schema)
    expect(result).toBe(schema);
  });
});

describe('SchemaExtractorRegistry', () => {
  it('names() returns built-in adapters in priority order', () => {
    const names = SchemaExtractorRegistry.names();
    expect(names).toContain('typebox');
    expect(names).toContain('json-schema');
    expect(names.indexOf('typebox')).toBeLessThan(names.indexOf('json-schema'));
  });

  it('register adds a new adapter', () => {
    SchemaExtractorRegistry.register({
      name: 'test-adapter',
      priority: 50,
      detect: () => false,
      extract: () => Type.Unknown(),
    });
    expect(SchemaExtractorRegistry.names()).toContain('test-adapter');
  });

  it('register triggers re-sort so names() returns correct order', () => {
    SchemaExtractorRegistry.register({
      name: 'high-priority',
      priority: 200,
      detect: () => false,
      extract: () => Type.Unknown(),
    });
    const names = SchemaExtractorRegistry.names();
    expect(names.indexOf('high-priority')).toBeLessThan(names.indexOf('typebox'));
  });

  it('unregister removes a registered adapter and returns true', () => {
    SchemaExtractorRegistry.register({
      name: 'test-adapter',
      priority: 50,
      detect: () => false,
      extract: () => Type.Unknown(),
    });
    expect(SchemaExtractorRegistry.unregister('test-adapter')).toBe(true);
    expect(SchemaExtractorRegistry.names()).not.toContain('test-adapter');
  });

  it('unregister returns false for unknown adapter', () => {
    expect(SchemaExtractorRegistry.unregister('does-not-exist')).toBe(false);
  });

  it('custom adapter is used by extractSchema when detect returns true', () => {
    const sentinel = { __custom: true };
    const customSchema = Type.Boolean();
    SchemaExtractorRegistry.register({
      name: 'test-adapter',
      priority: 50,
      detect: (v) => (v as Record<string, unknown>).__custom === true,
      extract: () => customSchema,
    });
    expect(extractSchema(sentinel)).toBe(customSchema);
  });
});

describe('inferSchemasFromModule', () => {
  it('returns null when module has no schema exports', () => {
    expect(inferSchemasFromModule({}, 'myFunc')).toBeNull();
  });

  it('returns null when only inputSchema is found', () => {
    const mod = { inputSchema: Type.String() };
    expect(inferSchemasFromModule(mod, 'myFunc')).toBeNull();
  });

  it('returns null when only outputSchema is found', () => {
    const mod = { outputSchema: Type.String() };
    expect(inferSchemasFromModule(mod, 'myFunc')).toBeNull();
  });

  it('infers schemas from direct named exports', () => {
    const input = Type.String();
    const output = Type.Number();
    const mod = { inputSchema: input, outputSchema: output };
    const result = inferSchemasFromModule(mod, 'myFunc');
    expect(result).not.toBeNull();
    expect(result!.input).toBe(input);
    expect(result!.output).toBe(output);
  });

  it('infers schemas from companion naming convention', () => {
    const input = Type.String();
    const output = Type.Number();
    const mod = { myFuncInputSchema: input, myFuncOutputSchema: output };
    const result = inferSchemasFromModule(mod, 'myFunc');
    expect(result).not.toBeNull();
    expect(result!.input).toBe(input);
    expect(result!.output).toBe(output);
  });

  it('returns null when a schema value cannot be extracted', () => {
    // An unrecognised value that no adapter handles
    const mod = { inputSchema: { notASchema: true }, outputSchema: Type.String() };
    expect(inferSchemasFromModule(mod, 'myFunc')).toBeNull();
  });
});
