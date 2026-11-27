/**
 * Tests for schema/exporter.ts — SchemaExporter profile exports.
 */

import { describe, it, expect } from 'vitest';
import { SchemaExporter } from '../../src/schema/exporter.js';
import { ExportProfile } from '../../src/schema/types.js';
import type { SchemaDefinition } from '../../src/schema/types.js';

function makeSchemaDef(overrides?: Partial<SchemaDefinition>): SchemaDefinition {
  return {
    moduleId: 'test.module',
    description: 'A test module',
    inputSchema: {
      type: 'object',
      properties: {
        name: { type: 'string', description: 'Name', 'x-llm-description': 'LLM Name' },
      },
      required: ['name'],
    },
    outputSchema: {
      type: 'object',
      properties: {
        result: { type: 'string' },
      },
    },
    definitions: {},
    version: '1.0.0',
    ...overrides,
  };
}

describe('SchemaExporter', () => {
  const exporter = new SchemaExporter();

  describe('exportGeneric', () => {
    it('returns module_id, description, input/output schemas, and definitions', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportGeneric(sd);
      expect(result['module_id']).toBe('test.module');
      expect(result['description']).toBe('A test module');
      expect(result['input_schema']).toEqual(sd.inputSchema);
      expect(result['output_schema']).toEqual(sd.outputSchema);
      expect(result['definitions']).toEqual({});
    });
  });

  it('defaults to the generic profile and supports its explicit name', () => {
    const schema = makeSchemaDef();
    const expected = exporter.exportGeneric(schema);
    expect(exporter.export(schema)).toEqual(expected);
    expect(exporter.export(schema, ExportProfile.GENERIC)).toEqual(expected);
  });
});
