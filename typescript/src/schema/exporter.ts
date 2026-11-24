/**
 * SchemaExporter — converts schemas to platform-specific export formats.
 */

import type { ModuleAnnotations, ModuleExample } from '../module.js';
import { deepCopy } from '../utils/index.js';
import { applyLlmDescriptions, stripExtensions, toStrictSchema } from './strict.js';
import { ExportProfile, type SchemaDefinition } from './types.js';

export class SchemaExporter {
  export(
    schemaDef: SchemaDefinition,
    profile: ExportProfile,
    annotations?: ModuleAnnotations | null,
    examples?: ModuleExample[] | null,
    name?: string | null,
  ): Record<string, unknown> {
    if (profile === ExportProfile.MCP) {
      return this.exportMcp(schemaDef, annotations, name);
    }
    if (profile === ExportProfile.OPENAI) {
      return this.exportOpenai(schemaDef);
    }
    if (profile === ExportProfile.ANTHROPIC) {
      return this.exportAnthropic(schemaDef, examples);
    }
    return this.exportGeneric(schemaDef);
  }

  exportMcp(
    schemaDef: SchemaDefinition,
    annotations?: ModuleAnnotations | null,
    name?: string | null,
  ): Record<string, unknown> {
    return {
      name: name ?? schemaDef.moduleId,
      description: schemaDef.description,
      inputSchema: schemaDef.inputSchema,
      annotations: {
        readOnlyHint: annotations?.readonly ?? false,
        destructiveHint: annotations?.destructive ?? false,
        idempotentHint: annotations?.idempotent ?? false,
        openWorldHint: annotations?.openWorld ?? true,
      },
      _meta: {
        ...(annotations?.requiresApproval ? { requiresApproval: true } : {}),
        ...(annotations?.streaming ? { streaming: true } : {}),
        cacheable: annotations?.cacheable ?? false,
        cacheTtl: annotations?.cacheTtl ?? 0,
        cacheKeyFields: annotations?.cacheKeyFields ?? null,
        paginated: annotations?.paginated ?? false,
        paginationStyle: annotations?.paginationStyle ?? 'cursor',
      },
    };
  }

  exportOpenai(schemaDef: SchemaDefinition): Record<string, unknown> {
    const schema = deepCopy(schemaDef.inputSchema);
    applyLlmDescriptions(schema);
    const strictSchema = toStrictSchema(schema);
    return {
      type: 'function',
      function: {
        name: schemaDef.moduleId.replace(/\./g, '_'),
        description: schemaDef.description,
        parameters: strictSchema,
        strict: true,
      },
    };
  }

  exportAnthropic(
    schemaDef: SchemaDefinition,
    examples?: ModuleExample[] | null,
  ): Record<string, unknown> {
    const schema = deepCopy(schemaDef.inputSchema);
    applyLlmDescriptions(schema);
    stripExtensions(schema, false);
    const result: Record<string, unknown> = {
      name: schemaDef.moduleId.replace(/\./g, '_'),
      description: schemaDef.description,
      input_schema: schema,
    };
    if (examples && examples.length > 0) {
      result['input_examples'] = examples.map((ex) => ex.inputs);
    }
    return result;
  }

  exportGeneric(schemaDef: SchemaDefinition): Record<string, unknown> {
    return {
      module_id: schemaDef.moduleId,
      description: schemaDef.description,
      input_schema: schemaDef.inputSchema,
      output_schema: schemaDef.outputSchema,
      definitions: schemaDef.definitions,
    };
  }
}
