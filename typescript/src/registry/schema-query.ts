import { snapshotRegistryData } from './metadata-pure.js';
import type { Registry } from './registry.js';

/** Return a detached schema object for a registered module. */
export function getSchema(registry: Registry, moduleId: string): Record<string, unknown> | null {
  return snapshotRegistryData(readSchema(registry, moduleId));
}

export function readSchema(registry: Registry, moduleId: string): Record<string, unknown> | null {
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
