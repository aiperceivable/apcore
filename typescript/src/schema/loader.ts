/**
 * SchemaLoader — primary entry point for the Node-side schema system.
 *
 * Uses TypeBox for schema representation. Since TypeBox schemas ARE JSON
 * Schema, the conversion layer is minimal.
 *
 * Browser-safe pieces (`jsonSchemaToTypeBox`, `contentHashAsync`) live in
 * `./loader-pure.ts` and are re-exported here so existing imports keep
 * working from the Node entry. Code that must run in the browser should
 * import them from `./loader-pure.ts` (or from the package `./browser`
 * entry) directly.
 */

import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { existsSync, readFileSync, realpathSync } from 'node:fs';
import { isWithinFilesystemPath } from '../utils/filesystem-path.js';
import type { TSchema } from '@sinclair/typebox';
import yaml from 'js-yaml';
import type { Config } from '../config.js';
import { SchemaNotFoundError, SchemaParseError } from '../errors.js';
import { negotiateDocumentVersion } from '../version.js';
import { moduleSchemaPath, RefResolver } from './ref-resolver.js';
import type { ResolvedSchema, SchemaDefinition } from './types.js';
import { SchemaStrategy } from './types.js';
import { jsonSchemaToTypeBox, sortedKeysStringify } from './loader-pure.js';

export {
  jsonSchemaToTypeBox,
  contentHashAsync,
  sortedKeysStringify,
} from './loader-pure.js';

type CompiledSchema = Pick<ResolvedSchema, 'jsonSchema' | 'schema'>;

export class SchemaLoader {
  private _config: Config;
  private _schemasDir: string;
  private _resolver: RefResolver;
  private _schemaCache: Map<string, SchemaDefinition> = new Map();
  private _resolvedCache = new Map<string, [ResolvedSchema, ResolvedSchema]>();
  private _contentCache = new Map<string, [CompiledSchema, CompiledSchema]>();

  constructor(config: Config, schemasDir?: string | null) {
    this._config = config;
    if (schemasDir != null) {
      this._schemasDir = resolve(schemasDir);
    } else {
      const schemaRoot = config.get('schema.root', './schemas') as string;
      const configured = config.resolvePath(schemaRoot || './schemas');
      this._schemasDir = existsSync(configured) ? realpathSync.native(configured) : configured;
    }
    const maxDepth = config.get('schema.max_ref_depth', 32) as number;
    this._resolver = new RefResolver(this._schemasDir, maxDepth);
  }

  load(moduleId: string): SchemaDefinition {
    const cached = this._schemaCache.get(moduleId);
    if (cached) return cached;

    const declaredPath = moduleSchemaPath(this._schemasDir, moduleId);
    let filePath: string;
    let physicalRoot: string;
    try {
      filePath = realpathSync.native(declaredPath);
      physicalRoot = realpathSync.native(this._schemasDir);
    } catch {
      throw new SchemaNotFoundError(moduleId);
    }
    if (!isWithinFilesystemPath(physicalRoot, filePath)) throw new SchemaNotFoundError(moduleId);

    let data: unknown;
    try {
      data = yaml.load(readFileSync(filePath, 'utf-8'));
    } catch (e) {
      throw new SchemaParseError(`Invalid YAML in schema for '${moduleId}': ${e}`);
    }

    if (data === null || data === undefined || typeof data !== 'object' || Array.isArray(data)) {
      throw new SchemaParseError(`Schema file for '${moduleId}' is empty or not a mapping`);
    }

    const dataObj = data as Record<string, unknown>;
    const version = dataObj['version'];
    if (version != null) {
      if (typeof version !== 'string') {
        throw new SchemaParseError(`Schema version for '${moduleId}' must be a string`);
      }
      negotiateDocumentVersion(version);
    }
    for (const fieldName of ['input_schema', 'output_schema', 'description']) {
      if (!(fieldName in dataObj)) {
        throw new SchemaParseError(
          `Missing required field: ${fieldName} in schema for '${moduleId}'`,
        );
      }
    }

    const definitions: Record<string, unknown> = {
      ...((dataObj['definitions'] as Record<string, unknown>) ?? {}),
      ...((dataObj['$defs'] as Record<string, unknown>) ?? {}),
    };

    const sd: SchemaDefinition = {
      moduleId: (dataObj['module_id'] as string) ?? moduleId,
      description: dataObj['description'] as string,
      inputSchema: dataObj['input_schema'] as Record<string, unknown>,
      outputSchema: dataObj['output_schema'] as Record<string, unknown>,
      errorSchema: (dataObj['error_schema'] as Record<string, unknown>) ?? null,
      definitions,
      sourcePath: filePath,
      version: version ?? '1.0.0',
      documentation: (dataObj['documentation'] as string) ?? null,
      schemaUrl: (dataObj['$schema'] as string) ?? null,
    };

    this._schemaCache.set(moduleId, sd);
    return sd;
  }

  resolve(schemaDef: SchemaDefinition): [ResolvedSchema, ResolvedSchema] {
    // D-104 (spec v1.50.0): hand the resolver the file this definition came
    // from, so a local `#/…` reference resolves against the FILE ROOT first
    // and falls back to the schema node. Both layouts are normative; passing
    // nothing here left only the node addressable, which is why the spec's own
    // §4.11 example — `definitions:` as a top-level sibling of `input_schema`
    // — loaded on apcore-rust alone. It is also what makes
    // `SchemaDefinition.definitions` live: it was dead precisely because the
    // refs that would have used it could not resolve.
    const sourcePath = schemaDef.sourcePath ?? null;
    const resolvedInput = this._resolver.resolve(schemaDef.inputSchema, sourcePath);
    const resolvedOutput = this._resolver.resolve(schemaDef.outputSchema, sourcePath);

    const inputSchema = jsonSchemaToTypeBox(resolvedInput);
    const outputSchema = jsonSchemaToTypeBox(resolvedOutput);

    const inputRs: ResolvedSchema = {
      jsonSchema: resolvedInput,
      schema: inputSchema,
      moduleId: schemaDef.moduleId,
      direction: 'input',
    };
    const outputRs: ResolvedSchema = {
      jsonSchema: resolvedOutput,
      schema: outputSchema,
      moduleId: schemaDef.moduleId,
      direction: 'output',
    };
    return [inputRs, outputRs];
  }

  getSchema(
    moduleId: string,
    nativeInputSchema?: TSchema | null,
    nativeOutputSchema?: TSchema | null,
  ): [ResolvedSchema, ResolvedSchema] {
    const rawStrategy = this._config.get('schema.strategy', 'yaml_first') as string;
    const strategyMap: Record<string, SchemaStrategy> = {
      yaml_first: SchemaStrategy.YAML_FIRST,
      native_first: SchemaStrategy.NATIVE_FIRST,
      yaml_only: SchemaStrategy.YAML_ONLY,
    };
    const strategy = strategyMap[rawStrategy] ?? SchemaStrategy.YAML_FIRST;

    if (strategy === SchemaStrategy.NATIVE_FIRST && nativeInputSchema && nativeOutputSchema) {
      return this._wrapNative(moduleId, nativeInputSchema, nativeOutputSchema);
    }
    try {
      return this._loadAndResolve(moduleId);
    } catch (error) {
      if (strategy === SchemaStrategy.YAML_FIRST && error instanceof SchemaNotFoundError &&
          nativeInputSchema && nativeOutputSchema) {
        return this._wrapNative(moduleId, nativeInputSchema, nativeOutputSchema);
      }
      throw error;
    }
  }

  private _loadAndResolve(moduleId: string): [ResolvedSchema, ResolvedSchema] {
    const cached = this._resolvedCache.get(moduleId);
    if (cached) return cached;
    const definition = this.load(moduleId);
    const sourcePath = definition.sourcePath ?? null;
    const input = this._resolver.resolve(definition.inputSchema, sourcePath);
    const output = this._resolver.resolve(definition.outputSchema, sourcePath);
    const digest = contentHash({ input, output });
    let compiled = this._contentCache.get(digest);
    if (!compiled) {
      compiled = [
        { jsonSchema: input, schema: jsonSchemaToTypeBox(input) },
        { jsonSchema: output, schema: jsonSchemaToTypeBox(output) },
      ];
      this._contentCache.set(digest, compiled);
    }
    const result: [ResolvedSchema, ResolvedSchema] = [
      { ...compiled[0], moduleId: definition.moduleId, direction: 'input' },
      { ...compiled[1], moduleId: definition.moduleId, direction: 'output' },
    ];
    this._resolvedCache.set(moduleId, result);
    return result;
  }

  private _wrapNative(
    moduleId: string,
    inputSchema: TSchema,
    outputSchema: TSchema,
  ): [ResolvedSchema, ResolvedSchema] {
    const inputRs: ResolvedSchema = {
      jsonSchema: inputSchema as unknown as Record<string, unknown>,
      schema: inputSchema,
      moduleId,
      direction: 'input',
    };
    const outputRs: ResolvedSchema = {
      jsonSchema: outputSchema as unknown as Record<string, unknown>,
      schema: outputSchema,
      moduleId,
      direction: 'output',
    };
    return [inputRs, outputRs];
  }

  clearCache(): void {
    this._schemaCache.clear();
    this._resolvedCache.clear();
    this._contentCache.clear();
    this._resolver.clearCache();
  }
}

/**
 * Compute the SHA-256 hex digest of the canonical JSON serialization of a
 * schema. Sync — Node-only. Use {@link contentHashAsync} from the browser
 * entry (or this file) for the WebCrypto-backed equivalent.
 */
export function contentHash(schema: unknown): string {
  const canonical = sortedKeysStringify(schema);
  return createHash('sha256').update(canonical).digest('hex');
}
