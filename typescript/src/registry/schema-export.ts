/**
 * Schema query and export functions for the registry system.
 */

import yaml from 'js-yaml';
import type { ModuleAnnotations, ModuleExample } from '../module.js';
import { InvalidInputError, ModuleNotFoundError } from '../errors.js';
import { deepCopy } from '../utils/index.js';
import { SchemaExporter } from '../schema/exporter.js';
import { stripExtensions, toStrictSchema } from '../schema/strict.js';
import { ExportProfile, type SchemaDefinition } from '../schema/types.js';
import type { Registry } from './registry.js';

export function getSchema(registry: Registry, moduleId: string): Record<string, unknown> | null {
  const module = registry.get(moduleId);
  if (module === null) return null;

  const mod = module as Record<string, unknown>;

  // A schema export, not a module manifest — exactly the four keys
  // `schemas/module-schema-export.schema.json` declares. `name`, `version`,
  // `tags`, `annotations` and `examples` used to ride along here, which made
  // this envelope a partial, non-conforming duplicate of `system.manifest.module`
  // (sys-manifest-module.schema.json) and left the three SDKs emitting three
  // different shapes. Descriptor metadata belongs to the manifest module;
  // callers that need it should ask for it there. TypeBox schemas are already
  // JSON Schema, so they pass through unchanged.
  return {
    module_id: moduleId,
    description: (mod['description'] as string) ?? '',
    input_schema: (mod['inputSchema'] as Record<string, unknown>) ?? {},
    output_schema: (mod['outputSchema'] as Record<string, unknown>) ?? {},
  };
}

export function exportSchema(
  registry: Registry,
  moduleId: string,
  format: string = 'json',
  strict: boolean = false,
  compact: boolean = false,
  profile?: string | null,
): string {
  const schemaDict = getSchema(registry, moduleId);
  if (schemaDict === null) {
    throw new ModuleNotFoundError(moduleId);
  }

  if (profile != null) {
    return exportWithProfile(registry, moduleId, schemaDict, profile, format);
  }

  const result = deepCopy(schemaDict);

  if (strict) {
    result['input_schema'] = toStrictSchema(result['input_schema'] as Record<string, unknown>);
    result['output_schema'] = toStrictSchema(result['output_schema'] as Record<string, unknown>);
  } else if (compact) {
    applyCompact(result);
  }

  return serialize(result, format);
}

export function getAllSchemas(registry: Registry): Record<string, Record<string, unknown>> {
  const result: Record<string, Record<string, unknown>> = {};
  for (const moduleId of registry.moduleIds) {
    const schema = getSchema(registry, moduleId);
    if (schema !== null) {
      result[moduleId] = schema;
    }
  }
  return result;
}

export function exportAllSchemas(
  registry: Registry,
  format: string = 'json',
  strict: boolean = false,
  compact: boolean = false,
  profile?: string | null,
): string {
  const allSchemas = getAllSchemas(registry);

  if (strict || compact) {
    for (const [moduleId, schema] of Object.entries(allSchemas)) {
      const result = deepCopy(schema);
      if (strict) {
        result['input_schema'] = toStrictSchema(result['input_schema'] as Record<string, unknown>);
        result['output_schema'] = toStrictSchema(result['output_schema'] as Record<string, unknown>);
      } else if (compact) {
        applyCompact(result);
      }
      allSchemas[moduleId] = result;
    }
  }

  return serialize(allSchemas, format);
}

function exportWithProfile(
  registry: Registry,
  moduleId: string,
  schemaDict: Record<string, unknown>,
  profile: string,
  format: string,
): string {
  const schemaDef: SchemaDefinition = {
    moduleId,
    description: schemaDict['description'] as string,
    inputSchema: schemaDict['input_schema'] as Record<string, unknown>,
    outputSchema: schemaDict['output_schema'] as Record<string, unknown>,
    definitions: {},
    version:
      ((registry.get(moduleId) as Record<string, unknown> | null)?.['version'] as string) ??
      '1.0.0',
  };
  const module = registry.get(moduleId);
  const annotations = module ? (module as Record<string, unknown>)['annotations'] as ModuleAnnotations | undefined : undefined;
  const examples = module ? ((module as Record<string, unknown>)['examples'] as ModuleExample[]) ?? [] : [];
  const name = module ? (module as Record<string, unknown>)['name'] as string | undefined : undefined;

  const validProfiles = new Set(Object.values(ExportProfile));
  if (!validProfiles.has(profile as ExportProfile)) {
    throw new InvalidInputError(
      `Invalid export profile: '${profile}'. Must be one of: ${[...validProfiles].join(', ')}`,
    );
  }

  const exported = new SchemaExporter().export(
    schemaDef,
    profile as ExportProfile,
    annotations,
    examples,
    name,
  );
  return serialize(exported, format);
}

function applyCompact(schemaDict: Record<string, unknown>): void {
  const desc = schemaDict['description'] as string;
  if (desc) {
    schemaDict['description'] = truncateDescription(desc);
  }
  stripExtensions(schemaDict['input_schema'] as Record<string, unknown> ?? {});
  stripExtensions(schemaDict['output_schema'] as Record<string, unknown> ?? {});
  delete schemaDict['documentation'];
  delete schemaDict['examples'];
}

function truncateDescription(description: string): string {
  const dotSpace = description.indexOf('. ');
  const newline = description.indexOf('\n');

  const candidates: number[] = [];
  if (dotSpace >= 0) candidates.push(dotSpace + 1);
  if (newline >= 0) candidates.push(newline);

  if (candidates.length > 0) {
    const cut = Math.min(...candidates);
    return description.slice(0, cut).trimEnd();
  }

  return description;
}

function serialize(data: unknown, format: string): string {
  if (format === 'yaml') {
    return yaml.dump(data, { flowLevel: -1 });
  }
  return JSON.stringify(data, null, 2);
}
