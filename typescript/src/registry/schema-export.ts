/**
 * Schema query and export functions for the registry system.
 */

import yaml from 'js-yaml';
import { InvalidInputError, ModuleNotFoundError } from '../errors.js';
import { deepCopy } from '../utils/index.js';
import { SchemaExporter } from '../schema/exporter.js';
import { stripExtensions, toStrictSchema } from '../schema/strict.js';
import { ExportProfile } from '../schema/types.js';
import { snapshotRegistryData } from './metadata-pure.js';
import { readSchema } from './schema-query.js';
import type { Registry } from './registry.js';

export { getSchema } from './schema-query.js';

export function exportSchema(
  registry: Registry,
  moduleId: string,
  format: string = 'json',
  strict: boolean = false,
  compact: boolean = false,
  profile?: string | null,
): string {
  const schemaDict = readSchema(registry, moduleId);
  if (schemaDict === null) {
    throw new ModuleNotFoundError(moduleId);
  }

  if (profile != null) {
    if (profile !== ExportProfile.GENERIC) {
      throw new InvalidInputError("Only the 'generic' export profile is supported by Core");
    }
    return serialize(new SchemaExporter().exportGeneric({
      moduleId,
      description: schemaDict['description'] as string,
      inputSchema: schemaDict['input_schema'] as Record<string, unknown>,
      outputSchema: schemaDict['output_schema'] as Record<string, unknown>,
      definitions: {},
      version: ((registry.get(moduleId) as Record<string, unknown> | null)?.['version'] as string) ?? '1.0.0',
    }), format);
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
  return snapshotRegistryData(readAllSchemas(registry));
}

function readAllSchemas(registry: Registry): Record<string, Record<string, unknown>> {
  const result: Record<string, Record<string, unknown>> = {};
  for (const moduleId of registry.moduleIds) {
    const schema = readSchema(registry, moduleId);
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
): string {
  const allSchemas = readAllSchemas(registry);

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
