/**
 * Metadata and ID map loading for the registry system (Node-side).
 *
 * Pure helpers (`parseDependencies`, `mergeModuleMetadata`) live in
 * `./metadata-pure.ts` and are re-exported here so existing
 * `import { … } from './metadata.js'` paths keep working from Node.
 * Browser consumers must import from `./metadata-pure.js` directly to
 * avoid pulling `node:fs` into the bundle.
 */

import { existsSync, readFileSync } from 'node:fs';
import yaml from 'js-yaml';
import { ConfigError, ConfigNotFoundError } from '../errors.js';

export { parseDependencies, mergeModuleMetadata } from './metadata-pure.js';

export function loadMetadata(metaPath: string): Record<string, unknown> {
  if (!existsSync(metaPath)) return {};

  const content = readFileSync(metaPath, 'utf-8');
  let parsed: unknown;
  try {
    parsed = yaml.load(content);
  } catch (e) {
    throw new ConfigError(`Invalid YAML in metadata file: ${metaPath}`);
  }

  if (parsed === null || parsed === undefined) return {};
  if (typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new ConfigError(`Metadata file must be a YAML mapping: ${metaPath}`);
  }

  return parsed as Record<string, unknown>;
}

export function loadIdMap(idMapPath: string): Record<string, Record<string, unknown>> {
  if (!existsSync(idMapPath)) {
    throw new ConfigNotFoundError(idMapPath);
  }

  const content = readFileSync(idMapPath, 'utf-8');
  let parsed: unknown;
  try {
    parsed = yaml.load(content);
  } catch (e) {
    throw new ConfigError(`Invalid YAML in ID map file: ${idMapPath}`);
  }

  if (typeof parsed !== 'object' || parsed === null || !('mappings' in (parsed as Record<string, unknown>))) {
    throw new ConfigError("ID map must contain a 'mappings' list");
  }

  const mappings = (parsed as Record<string, unknown>)['mappings'];
  if (!Array.isArray(mappings)) {
    throw new ConfigError("ID map must contain a 'mappings' list");
  }

  const result: Record<string, Record<string, unknown>> = {};
  for (const entry of mappings) {
    const filePath = (entry as Record<string, unknown>)['file'] as string;
    if (!filePath) {
      console.warn(`[apcore:metadata] ID map entry missing 'file' field, skipping`);
      continue;
    }
    // An entry carries `file` and `id` only (protocol-spec §2.2 rule 4, D-138).
    result[filePath] = {
      id: ((entry as Record<string, unknown>)['id'] as string) ?? filePath,
    };
  }
  return result;
}
