/**
 * Tests for schema/exporter.ts — SchemaExporter profile exports.
 */

import { describe, it, expect } from 'vitest';
import { SchemaExporter } from '../../src/schema/exporter.js';
import { ExportProfile } from '../../src/schema/types.js';
import type { SchemaDefinition } from '../../src/schema/types.js';
import type { ModuleAnnotations, ModuleExample } from '../../src/module.js';

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

  describe('exportMcp', () => {
    it('returns MCP tool format with annotations', () => {
      const sd = makeSchemaDef();
      const annotations: ModuleAnnotations = {
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
        paginationStyle: 'cursor', extra: {},
      };
      const result = exporter.exportMcp(sd, annotations, 'MyTool');
      expect(result['name']).toBe('MyTool');
      expect(result['description']).toBe('A test module');
      expect(result['inputSchema']).toEqual(sd.inputSchema);
      const annots = result['annotations'] as Record<string, unknown>;
      expect(annots['readOnlyHint']).toBe(true);
      expect(annots['destructiveHint']).toBe(false);
      expect(annots['idempotentHint']).toBe(true);
      expect(annots['openWorldHint']).toBe(false);
      expect(annots).not.toHaveProperty('streaming');
      expect(result['_meta']).not.toHaveProperty('streaming');
    });

    it('includes streaming hint when annotation is true', () => {
      const sd = makeSchemaDef();
      const annotations: ModuleAnnotations = {
        readonly: false,
        destructive: false,
        idempotent: false,
        requiresApproval: false,
        openWorld: true,
        streaming: true,
        cacheable: false,
        cacheTtl: 0,
        cacheKeyFields: null,
        paginated: false,
        paginationStyle: 'cursor', extra: {},
      };
      const result = exporter.exportMcp(sd, annotations);
      const annots = result['annotations'] as Record<string, unknown>;
      expect(annots).not.toHaveProperty('streaming');
      expect(result['_meta']).toHaveProperty('streaming', true);
    });

    it('falls back to moduleId when name is null', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportMcp(sd, null, null);
      expect(result['name']).toBe('test.module');
    });

    it('includes _meta with cache and pagination defaults when annotations is null', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportMcp(sd, null);
      const meta = result['_meta'] as Record<string, unknown>;
      expect(meta['cacheable']).toBe(false);
      expect(meta['cacheTtl']).toBe(0);
      expect(meta['cacheKeyFields']).toBeNull();
      expect(meta['paginated']).toBe(false);
      expect(meta['paginationStyle']).toBe('cursor');
    });

    it('includes _meta with cache and pagination values from annotations', () => {
      const sd = makeSchemaDef();
      const annotations: ModuleAnnotations = {
        readonly: false,
        destructive: false,
        idempotent: false,
        requiresApproval: false,
        openWorld: true,
        streaming: false,
        cacheable: true,
        cacheTtl: 120,
        cacheKeyFields: ['userId'],
        paginated: true,
        paginationStyle: 'offset', extra: {},
      };
      const result = exporter.exportMcp(sd, annotations);
      const meta = result['_meta'] as Record<string, unknown>;
      expect(meta['cacheable']).toBe(true);
      expect(meta['cacheTtl']).toBe(120);
      expect(meta['cacheKeyFields']).toEqual(['userId']);
      expect(meta['paginated']).toBe(true);
      expect(meta['paginationStyle']).toBe('offset');
    });

    it('uses default annotation values when annotations is null', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportMcp(sd, null);
      const annots = result['annotations'] as Record<string, unknown>;
      expect(annots['readOnlyHint']).toBe(false);
      expect(annots['destructiveHint']).toBe(false);
      expect(annots['idempotentHint']).toBe(false);
      expect(annots['openWorldHint']).toBe(true);
      expect(annots).not.toHaveProperty('streaming');
      expect(result['_meta']).not.toHaveProperty('streaming');
    });
  });

  describe('exportOpenai', () => {
    it('returns OpenAI function calling format with strict schema', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportOpenai(sd);
      expect(result['type']).toBe('function');
      const fn = result['function'] as Record<string, unknown>;
      expect(fn['name']).toBe('test_module');
      expect(fn['description']).toBe('A test module');
      expect(fn['strict']).toBe(true);
      const params = fn['parameters'] as Record<string, unknown>;
      expect(params['additionalProperties']).toBe(false);
    });

    it('applies x-llm-description to properties', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportOpenai(sd);
      const fn = result['function'] as Record<string, unknown>;
      const params = fn['parameters'] as Record<string, unknown>;
      const props = params['properties'] as Record<string, Record<string, unknown>>;
      expect(props['name']['description']).toBe('LLM Name');
    });
  });

  describe('exportAnthropic', () => {
    it('returns Anthropic tool format', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportAnthropic(sd);
      expect(result['name']).toBe('test_module');
      expect(result['description']).toBe('A test module');
      expect(result['input_schema']).toBeDefined();
    });

    it('includes input_examples when examples are provided', () => {
      const sd = makeSchemaDef();
      const examples: ModuleExample[] = [
        { title: 'Ex1', inputs: { name: 'Alice' }, output: { result: 'ok' } },
      ];
      const result = exporter.exportAnthropic(sd, examples);
      const inputExamples = result['input_examples'] as Array<Record<string, unknown>>;
      expect(inputExamples).toHaveLength(1);
      expect(inputExamples[0]).toEqual({ name: 'Alice' });
    });

    it('omits input_examples when no examples', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportAnthropic(sd, []);
      expect(result['input_examples']).toBeUndefined();
    });

    it('strips x- extensions from schema', () => {
      const sd = makeSchemaDef();
      const result = exporter.exportAnthropic(sd);
      const schema = result['input_schema'] as Record<string, unknown>;
      const props = schema['properties'] as Record<string, Record<string, unknown>>;
      expect(props['name']['x-llm-description']).toBeUndefined();
    });
  });

  describe('export dispatch', () => {
    it('dispatches to MCP profile', () => {
      const sd = makeSchemaDef();
      const result = exporter.export(sd, ExportProfile.MCP);
      expect(result['name']).toBe('test.module');
      expect(result['annotations']).toBeDefined();
    });

    it('dispatches to OpenAI profile', () => {
      const sd = makeSchemaDef();
      const result = exporter.export(sd, ExportProfile.OPENAI);
      expect(result['type']).toBe('function');
    });

    it('dispatches to Anthropic profile', () => {
      const sd = makeSchemaDef();
      const result = exporter.export(sd, ExportProfile.ANTHROPIC);
      expect(result['input_schema']).toBeDefined();
    });

    it('dispatches to Generic profile', () => {
      const sd = makeSchemaDef();
      const result = exporter.export(sd, ExportProfile.GENERIC);
      expect(result['module_id']).toBe('test.module');
    });
  });
});
