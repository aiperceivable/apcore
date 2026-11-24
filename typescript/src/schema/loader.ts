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
import { join, resolve } from 'node:path';
import { existsSync, readFileSync } from 'node:fs';
import type { TSchema } from '@sinclair/typebox';
import yaml from 'js-yaml';
import type { Config } from '../config.js';
import { SchemaNotFoundError, SchemaParseError } from '../errors.js';
import { RefResolver } from './ref-resolver.js';
import type { ResolvedSchema, SchemaDefinition } from './types.js';
import { SchemaStrategy } from './types.js';
import { jsonSchemaToTypeBox, sortedKeysStringify } from './loader-pure.js';

export {
  jsonSchemaToTypeBox,
  contentHashAsync,
  sortedKeysStringify,
} from './loader-pure.js';

export class SchemaLoader {
  private _config: Config;
  private _schemasDir: string;
  private _resolver: RefResolver;
  private _schemaCache: Map<string, SchemaDefinition> = new Map();
  private _pathIndex: Map<string, string> = new Map();
  private _contentCache: Map<string, [ResolvedSchema, ResolvedSchema]> = new Map();

  constructor(config: Config, schemasDir?: string | null) {
    this._config = config;
    if (schemasDir != null) {
      this._schemasDir = resolve(schemasDir);
    } else {
      this._schemasDir = resolve(config.get('schema.root', './schemas') as string);
    }
    const maxDepth = config.get('schema.max_ref_depth', 32) as number;
    this._resolver = new RefResolver(this._schemasDir, maxDepth);
  }

  load(moduleId: string): SchemaDefinition {
    const cached = this._schemaCache.get(moduleId);
    if (cached) return cached;

    const filePath = join(this._schemasDir, moduleId.replace(/\./g, '/') + '.schema.yaml');
    if (!existsSync(filePath)) {
      throw new SchemaNotFoundError(moduleId);
    }

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
      version: (dataObj['version'] as string) ?? '1.0.0',
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

    const pathKey = `${moduleId}:${strategy}`;
    const cachedHash = this._pathIndex.get(pathKey);
    if (cachedHash) {
      return this._contentCache.get(cachedHash)!;
    }

    let result: [ResolvedSchema, ResolvedSchema] | null = null;

    if (strategy === SchemaStrategy.YAML_FIRST) {
      try {
        result = this._loadAndResolve(moduleId);
      } catch (e) {
        if (e instanceof SchemaNotFoundError && nativeInputSchema && nativeOutputSchema) {
          result = this._wrapNative(moduleId, nativeInputSchema, nativeOutputSchema);
        } else {
          throw e;
        }
      }
    } else if (strategy === SchemaStrategy.NATIVE_FIRST) {
      if (nativeInputSchema && nativeOutputSchema) {
        result = this._wrapNative(moduleId, nativeInputSchema, nativeOutputSchema);
      } else {
        result = this._loadAndResolve(moduleId);
      }
    } else if (strategy === SchemaStrategy.YAML_ONLY) {
      result = this._loadAndResolve(moduleId);
    }

    if (result === null) {
      throw new SchemaNotFoundError(moduleId);
    }

    const digest = contentHash({ input: result[0].jsonSchema, output: result[1].jsonSchema });
    if (!this._contentCache.has(digest)) {
      this._contentCache.set(digest, result);
    }
    this._pathIndex.set(pathKey, digest);

    return this._contentCache.get(digest)!;
  }

  private _loadAndResolve(moduleId: string): [ResolvedSchema, ResolvedSchema] {
    const sd = this.load(moduleId);
    return this.resolve(sd);
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
    this._pathIndex.clear();
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
