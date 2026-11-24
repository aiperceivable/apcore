/**
 * System manifest modules -- module metadata and full system manifest.
 */

import type { Registry } from '../registry/registry.js';
import type { Config } from '../config.js';
import { InvalidInputError, ModuleNotFoundError } from '../errors.js';
import type { ModuleDescriptor } from '../registry/types.js';
import type { ModuleAnnotations } from '../module.js';
import { annotationsToJSON } from '../module.js';
import { governanceUnion } from '../schema/annotations.js';

/** D-110: the project name reported when `project.name` is not configured. */
const DEFAULT_PROJECT_NAME = 'apcore';

/**
 * SYS-5: emit `annotations` in the snake_case WIRE shape.
 *
 * The manifest modules used to hand `descriptor.annotations` — the camelCase
 * `ModuleAnnotations` STRUCT — straight to the caller, so a consumer reading
 * `requires_approval` / `open_world` / `cache_ttl` off a TypeScript host got
 * `undefined` where apcore-python and apcore-rust answered. The spec's own
 * output example (system-modules.md:196-202) is snake_case, and this repo
 * already exported `annotationsToJSON` for exactly this and never called it.
 *
 * A descriptor with no annotations still emits `null`, unchanged: the output
 * schema and `sys-manifest-module.schema.json` both allow it.
 */
/**
 * Project the annotations an agent will actually be held to.
 *
 * The two governance flags come from the D-96 union of the live module instance
 * and the registry's declared annotations — the same source the approval gate
 * reads. Everything else is the descriptor's, unchanged.
 *
 * The manifest is what an agent reads to decide whether to call a module, so
 * advertising a governance value the gate does not enforce is worse than
 * advertising none: the descriptor alone could say `requires_approval: false`
 * for a module whose instance declares it, and `true` for one the gate lets
 * straight through.
 */
function governanceWire(
  registry: Registry,
  moduleId: string,
  annotations: ModuleAnnotations | null | undefined,
): Record<string, unknown> | null {
  const wire = annotationsWire(annotations);
  if (wire === null) return null;
  const effective = governanceUnion(
    (registry.get(moduleId) as Record<string, unknown> | null)?.['annotations'],
    registry.getDeclaredAnnotations(moduleId),
  );
  if (effective !== null) {
    wire['requires_approval'] = Boolean(effective.requiresApproval);
    wire['destructive'] = Boolean(effective.destructive);
  }
  return wire;
}

function annotationsWire(
  annotations: ModuleAnnotations | null | undefined,
): Record<string, unknown> | null {
  return annotations == null ? null : annotationsToJSON(annotations);
}

/**
 * Emit the descriptor's parsed dependencies in the snake_case wire shape.
 *
 * This read `descriptor.metadata['dependencies']`, which was empty in this
 * SDK for every module — `mergeModuleMetadata` extracts `dependencies` as a
 * canonical field, so it never appeared under `metadata`. `system.manifest.*`
 * therefore reported `dependencies: []` for a module that declared them,
 * while apcore-python reported the real list. Now reads the typed
 * `descriptor.dependencies` field (sync finding A-D-004).
 *
 * Shared by both manifest modules (SYS-8): `sys-manifest-full.schema.json`
 * `$ref`s `sys-manifest-module.schema.json`, so one emitter is what keeps the
 * two entry shapes from drifting apart again.
 */
function dependenciesWire(descriptor: ModuleDescriptor): unknown[] {
  return (descriptor.dependencies ?? []).map((d) => ({
    module_id: d.moduleId,
    ...(d.version != null ? { version: d.version } : {}),
    ...(d.optional ? { optional: true } : {}),
  }));
}

/** @internal */
export class ManifestModule {
  readonly description = 'Full manifest for a registered module including source path';
  readonly annotations = { readonly: true, destructive: false, idempotent: true, requiresApproval: false, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      module_id: { type: 'string' as const, description: 'ID of the module to inspect' },
    },
    required: ['module_id'],
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      module_id: { type: 'string' as const, description: 'Module identifier' },
      description: { type: 'string' as const, description: 'Module description' },
      documentation: { description: 'Module documentation (Markdown)' },
      source_path: { type: 'string' as const, description: 'Computed source file path' },
      input_schema: { type: 'object' as const, description: 'Module input JSON Schema' },
      output_schema: { type: 'object' as const, description: 'Module output JSON Schema' },
      annotations: { type: 'object' as const, description: 'Module annotations' },
      tags: { type: 'array' as const, description: 'Module tags' },
      dependencies: { type: 'array' as const, description: 'Module dependencies' },
      metadata: { type: 'object' as const, description: 'Additional metadata' },
    },
    required: ['module_id', 'description'],
  };

  private readonly _registry: Registry;
  private readonly _config: Config | null;

  constructor(
    registry: Registry,
    config: Config | null = null,
  ) {
    this._registry = registry;
    this._config = config;
  }

  execute(inputs: Record<string, unknown>, _context: unknown): Record<string, unknown> {
    const moduleId = inputs['module_id'];
    if (typeof moduleId !== 'string' || !moduleId) {
      throw new InvalidInputError('module_id is required');
    }

    const descriptor = this._registry.getDefinition(moduleId);
    if (!descriptor) {
      throw new ModuleNotFoundError(moduleId);
    }

    const sourcePath = this._computeSourcePath(moduleId);
    return {
      module_id: descriptor.moduleId,
      description: descriptor.description,
      documentation: descriptor.documentation,
      source_path: sourcePath,
      input_schema: descriptor.inputSchema,
      output_schema: descriptor.outputSchema,
      annotations: governanceWire(this._registry, descriptor.moduleId, descriptor.annotations),
      tags: descriptor.tags,
      dependencies: dependenciesWire(descriptor),
      metadata: descriptor.metadata ?? {},
    };
  }

  private _computeSourcePath(moduleId: string): string | null {
    if (!this._config) return null;
    const sourceRoot = this._config.get('project.source_root', '') as string;
    if (!sourceRoot) return null;
    const relativePath = moduleId.replace(/\./g, '/') + '.ts';
    return `${sourceRoot}/${relativePath}`;
  }
}

/** @internal */
export class ManifestFullModule {
  readonly description = 'Complete system manifest with filtering by prefix and tags';
  readonly annotations = { readonly: true, destructive: false, idempotent: true, requiresApproval: false, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      include_schemas: { type: 'boolean' as const, description: 'Whether to include input/output schemas', default: true },
      include_source_paths: { type: 'boolean' as const, description: 'Whether to include source paths', default: true },
      prefix: { type: 'string' as const, description: 'Filter modules by ID prefix' },
      tags: { type: 'array' as const, items: { type: 'string' as const }, description: 'Filter modules by tags' },
    },
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      project_name: { type: 'string' as const, description: 'Project name from config' },
      module_count: { type: 'integer' as const, description: 'Number of modules returned' },
      modules: { type: 'array' as const, description: 'Module manifest entries' },
    },
    required: ['project_name', 'module_count', 'modules'],
  };

  private readonly _registry: Registry;
  private readonly _config: Config | null;

  constructor(
    registry: Registry,
    config: Config | null = null,
  ) {
    this._registry = registry;
    this._config = config;
  }

  execute(inputs: Record<string, unknown>, _context: unknown): Record<string, unknown> {
    const includeSchemas = inputs['include_schemas'] !== false;
    const includeSourcePaths = inputs['include_source_paths'] !== false;
    const prefix = inputs['prefix'] as string | undefined;
    const tags = inputs['tags'] as string[] | undefined;

    const moduleIds = this._registry.list({ prefix, tags });
    const modules: Record<string, unknown>[] = [];

    for (const mid of moduleIds) {
      const descriptor = this._registry.getDefinition(mid);
      if (!descriptor) continue;

      const sourcePath = includeSourcePaths ? this._computeSourcePath(mid) : null;
      modules.push({
        module_id: descriptor.moduleId,
        description: descriptor.description,
        documentation: descriptor.documentation,
        source_path: sourcePath,
        input_schema: includeSchemas ? descriptor.inputSchema : null,
        output_schema: includeSchemas ? descriptor.outputSchema : null,
        annotations: governanceWire(this._registry, descriptor.moduleId, descriptor.annotations),
        tags: descriptor.tags,
        // SYS-8: `dependencies` was emitted by `manifest.module` and omitted
        // here, but `sys-manifest-full.schema.json` `$ref`s
        // `sys-manifest-module.schema.json` — one entry shape, so a consumer
        // written against either must be able to read both.
        dependencies: dependenciesWire(descriptor),
        metadata: descriptor.metadata ?? {},
      });
    }

    // D-110: `"apcore"` when unconfigured, not `""`. `system.health.summary`
    // already reported `"apcore"` here while this module reported `""`, so the
    // two system modules disagreed about the same fact in the same process. The
    // decision was taken on that internal consistency rather than an SDK
    // majority — `""` would have meant changing two system modules per SDK and
    // leaving the contradiction in place.
    const projectName =
      ((this._config?.get('project.name', '') ?? '') as string) || DEFAULT_PROJECT_NAME;
    return { project_name: projectName, module_count: modules.length, modules };
  }

  private _computeSourcePath(moduleId: string): string | null {
    if (!this._config) return null;
    const sourceRoot = this._config.get('project.source_root', '') as string;
    if (!sourceRoot) return null;
    return `${sourceRoot}/${moduleId.replace(/\./g, '/')}.ts`;
  }
}
