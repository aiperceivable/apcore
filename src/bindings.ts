/**
 * YAML binding loader for zero-code-modification module integration.
 */

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import type { TSchema } from '@sinclair/typebox';
import yaml from 'js-yaml';
import { FunctionModule } from './decorator.js';

import {
  BindingCallableNotFoundError,
  BindingFileInvalidError,
  BindingInvalidTargetError,
  BindingModuleNotFoundError,
  BindingNotCallableError,
  BindingSchemaInferenceFailedError,
  BindingSchemaModeConflictError,
} from './errors.js';
import type { Registry } from './registry/registry.js';
import { inferSchemasFromModule } from './schema/extractor.js';
import { matchGlob } from './utils/pattern.js';
import { jsonSchemaToTypeBox } from './schema/loader.js';
import { assertOpenAiStrictCompatible } from './schema/openai-strict.js';

import type { ModuleAnnotations, ResourceLimits } from './module.js';
import { DEFAULT_ANNOTATIONS } from './module.js';

const SUPPORTED_SPEC_VERSIONS = new Set(['1.0']);
const BINDING_KEYS = new Set(['module_id', 'target', 'description', 'documentation',
  'input_schema', 'output_schema', 'schema_ref', 'auto_schema', 'tags', 'version',
  'annotations', 'resources', 'metadata']);

/** D-139: reject malformed declarations before importing any target code. */
function validateBindingEntry(entry: unknown, filePath: string): asserts entry is Record<string, unknown> {
  if (entry === null || typeof entry !== 'object' || Array.isArray(entry)) {
    throw new BindingFileInvalidError(filePath, 'Binding entry must be a mapping');
  }
  for (const key of Object.keys(entry)) {
    if (!BINDING_KEYS.has(key)) throw new BindingFileInvalidError(filePath, `Unknown binding key '${key}'`);
  }
  if (('input_schema' in entry) !== ('output_schema' in entry)) {
    throw new BindingFileInvalidError(filePath, 'input_schema and output_schema must be provided together');
  }
  if ('auto_schema' in entry && ![true, false, 'true', 'permissive', 'strict'].includes(entry.auto_schema as boolean | string)) {
    throw new BindingFileInvalidError(filePath, "auto_schema must be true, false, 'true', 'permissive', or 'strict'");
  }
}

/**
 * Convert a snake_case YAML annotations dict to a typed ModuleAnnotations.
 * Unknown keys go into `extra`.
 */
function parseAnnotations(raw: Record<string, unknown>): ModuleAnnotations {
  const extra: Record<string, unknown> = {};
  for (const key of Object.keys(raw)) {
    if (
      ![
        'readonly',
        'destructive',
        'idempotent',
        'requires_approval',
        'open_world',
        'streaming',
        'cacheable',
        'cache_ttl',
        'cache_key_fields',
        'paginated',
        'pagination_style',
        'extra',
      ].includes(key)
    ) {
      extra[key] = raw[key];
    }
  }
  const rawExtra = (raw['extra'] as Record<string, unknown>) ?? {};
  return {
    readonly: (raw['readonly'] as boolean) ?? DEFAULT_ANNOTATIONS.readonly,
    destructive: (raw['destructive'] as boolean) ?? DEFAULT_ANNOTATIONS.destructive,
    idempotent: (raw['idempotent'] as boolean) ?? DEFAULT_ANNOTATIONS.idempotent,
    requiresApproval: (raw['requires_approval'] as boolean) ?? DEFAULT_ANNOTATIONS.requiresApproval,
    openWorld: (raw['open_world'] as boolean) ?? DEFAULT_ANNOTATIONS.openWorld,
    streaming: (raw['streaming'] as boolean) ?? DEFAULT_ANNOTATIONS.streaming,
    cacheable: (raw['cacheable'] as boolean) ?? DEFAULT_ANNOTATIONS.cacheable,
    cacheTtl: (raw['cache_ttl'] as number) ?? DEFAULT_ANNOTATIONS.cacheTtl,
    cacheKeyFields:
      (raw['cache_key_fields'] as string[] | null) ?? DEFAULT_ANNOTATIONS.cacheKeyFields,
    paginated: (raw['paginated'] as boolean) ?? DEFAULT_ANNOTATIONS.paginated,
    paginationStyle: (raw['pagination_style'] as string) ?? DEFAULT_ANNOTATIONS.paginationStyle,
    extra: { ...rawExtra, ...extra },
  };
}

/** §9.1.1 default for `bindings.dir`. */
const DEFAULT_BINDING_DIR = './bindings';

/** §9.1.1 default for `bindings.pattern`. */
const DEFAULT_BINDING_PATTERN = '*.binding.yaml';

/**
 * The slice of {@link Config} {@link BindingLoader.loadBindingDir} needs.
 *
 * Structural rather than a `Config` import so a caller can supply any
 * configuration source, and so `bindings.ts` keeps no runtime dependency on
 * `config.ts`. Same shape and motive as `AclConfigLike` in `./acl.ts`.
 */
export interface BindingConfigLike {
  get(key: string, defaultValue?: unknown): unknown;
}

/**
 * A configured value usable as a path or pattern, or `null` so `??` falls
 * through. An unset key reads as `undefined`, and an empty string is not a
 * directory anybody meant.
 */
function asConfiguredString(value: unknown): string | null {
  return typeof value === 'string' && value !== '' ? value : null;
}

export interface BindingLoaderOptions {
  /**
   * Optional allowlist of module-path prefixes. When set, `resolveTarget`
   * rejects (with `BindingInvalidTargetError`) any target whose module path
   * does not start with one of these prefixes BEFORE attempting the dynamic
   * import. Mirrors apcore-python's `trusted_package_prefixes` enforcement.
   * When unset (the default), any importable target is permitted (back-compat).
   */
  trustedPackagePrefixes?: string[];
}


/**
 * PROTOCOL_SPEC §9.1.2 requirement 6 — the semver.org grammar, unmodified and
 * written into the specification as a literal so three implementations cannot
 * invent three. `1.0` does NOT match: the patch component is required.
 */
const SEMVER_RE =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

function positiveIntLimit(value: unknown): number | null {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 ? value : null;
}

/**
 * Apply the `validation.binding.*` limits to one binding entry (§9.1.2).
 *
 * **Every limit is unconstrained by default.** apcore does not impose limits on
 * the content its users author; it offers them, and an operator opts in. A key
 * left at its default (`null` / `false`) checks nothing here, so a binding file
 * that loaded before this function existed still loads after it.
 *
 * When a limit IS configured it is enforced, not warned about: the operator
 * asked for a limit, and a limit that only warns is the `regex_patterns`
 * failure of §10.6.1 in another place.
 */
function validateBindingLimits(
  entry: Record<string, unknown>,
  config: BindingConfigLike | null,
  filePath: string,
): void {
  if (config == null) return;
  const moduleId = String(entry['module_id'] ?? '<unknown>');

  for (const [field, key] of [
    ['description', 'validation.binding.description_max_length'],
    ['documentation', 'validation.binding.documentation_max_length'],
  ] as const) {
    const max = positiveIntLimit(config.get(key));
    const value = entry[field];
    if (max === null || typeof value !== 'string') continue;
    // §9.1.2 requirement 4: characters, not bytes.
    if ([...value].length > max) {
      throw new BindingFileInvalidError(
        filePath,
        `binding '${moduleId}' field '${field}' is ${[...value].length} characters, over the ${max} configured by ${key}`,
      );
    }
  }

  const tagsPattern = config.get('validation.binding.tags_pattern');
  const tags = entry['tags'];
  if (typeof tagsPattern === 'string' && tagsPattern !== '' && Array.isArray(tags)) {
    let compiled: RegExp;
    try {
      compiled = new RegExp(tagsPattern);
    } catch (e) {
      // §9.2.3 requirement 6d: never skipped in silence.
      throw new BindingFileInvalidError(
        filePath,
        `validation.binding.tags_pattern ${JSON.stringify(tagsPattern)} does not compile (${String(e)}), so no tag could be checked against it`,
      );
    }
    for (const tag of tags) {
      if (typeof tag === 'string' && !compiled.test(tag)) {
        throw new BindingFileInvalidError(
          filePath,
          `binding '${moduleId}' tag ${JSON.stringify(tag)} does not match validation.binding.tags_pattern ${JSON.stringify(tagsPattern)}`,
        );
      }
    }
  }

  if (config.get('validation.binding.version_require_semver') === true) {
    const version = entry['version'];
    if (typeof version === 'string' && !SEMVER_RE.test(version)) {
      throw new BindingFileInvalidError(
        filePath,
        `binding '${moduleId}' version ${JSON.stringify(version)} is not SemVer, required by validation.binding.version_require_semver`,
      );
    }
  }
}

export class BindingLoader {
  private readonly _trustedPackagePrefixes: readonly string[] | null;

  constructor(options: BindingLoaderOptions = {}) {
    this._trustedPackagePrefixes = options.trustedPackagePrefixes ?? null;
  }

  /**
   * Load a binding file and register every module it declares.
   *
   * @param config Optional configuration supplying the `validation.binding.*`
   *   limits of PROTOCOL_SPEC §9.1.2. All four are **unconstrained by
   *   default**, so omitting this argument — as every caller before v1.38.0
   *   did — checks nothing and rejects nothing.
   */
  async loadBindings(
    filePath: string,
    registry: Registry,
    config?: BindingConfigLike | null,
  ): Promise<FunctionModule[]> {
    const bindingFileDir = dirname(filePath);

    let content: string;
    try {
      content = readFileSync(filePath, 'utf-8');
    } catch (e) {
      throw new BindingFileInvalidError(filePath, String(e));
    }

    let data: unknown;
    try {
      data = yaml.load(content);
    } catch (e) {
      throw new BindingFileInvalidError(filePath, `YAML parse error: ${e}`);
    }

    if (data === null || data === undefined) {
      throw new BindingFileInvalidError(filePath, 'File is empty');
    }

    if (typeof data !== 'object' || Array.isArray(data)) {
      throw new BindingFileInvalidError(filePath, 'Top-level must be a mapping');
    }

    const dataObj = data as Record<string, unknown>;
    for (const key of Object.keys(dataObj)) {
      if (!['spec_version', 'bindings'].includes(key)) throw new BindingFileInvalidError(filePath, `Unknown top-level key '${key}'`);
    }

    const specVersion = dataObj['spec_version'] as string | undefined;
    if (specVersion == null) {
      console.warn(
        `[apcore:bindings] ${filePath}: spec_version missing; defaulting to '1.0'. ` +
          'See protocol-spec §5.12.2',
      );
    } else if (!SUPPORTED_SPEC_VERSIONS.has(specVersion)) {
      console.warn(
        `[apcore:bindings] ${filePath}: spec_version '${specVersion}' is newer than supported; proceeding best-effort.`,
      );
    }

    if (!('bindings' in dataObj)) {
      throw new BindingFileInvalidError(filePath, "Missing 'bindings' key");
    }

    const bindings = dataObj['bindings'];
    if (!Array.isArray(bindings)) {
      throw new BindingFileInvalidError(filePath, "'bindings' must be a list");
    }

    const results: FunctionModule[] = [];
    for (const entry of bindings) {
      validateBindingEntry(entry, filePath);
      const entryObj = entry as Record<string, unknown>;
      if (!('module_id' in entryObj)) {
        throw new BindingFileInvalidError(filePath, "Binding entry missing 'module_id'");
      }
      if (!('target' in entryObj)) {
        throw new BindingFileInvalidError(filePath, "Binding entry missing 'target'");
      }

      validateBindingLimits(entryObj, config ?? null, filePath);
      const fm = await this._createModuleFromBinding(entryObj, bindingFileDir, filePath);
      await registry.register(entryObj['module_id'] as string, fm);
      results.push(fm);
    }

    return results;
  }

  /**
   * Scan a directory for binding files and register everything they declare
   * (PROTOCOL_SPEC §5.12.6).
   *
   * The directory resolves as **explicit argument > `bindings.dir` > the
   * `'./bindings'` default**, and `pattern` resolves the same way against
   * `bindings.pattern` with the `'*.binding.yaml'` default. `config` is the
   * §9.2 merge — environment variable > configuration file > default — so
   * `APCORE_BINDINGS_DIR` still reaches this loader, but through
   * `applyEnvOverrides` like every other `APCORE_*` variable rather than a
   * second, competing reader.
   *
   * §5.12.6 forbids reading `APCORE_BINDINGS_DIR` directly here, and that is
   * why: until apcore-typescript#36 this method read `process.env` at the
   * consumption site, so one variable had two readers that could disagree —
   * `bindings.dir` written in `apcore.yaml` was returned by `Config.get` and
   * ignored here, and the `'./bindings'` default never applied because the
   * method threw instead.
   *
   * Nothing in this SDK calls this method: loading bindings is an action the
   * application takes. §5.12.6 clause 3 forbids scanning automatically during
   * client or framework initialisation.
   */
  async loadBindingDir(
    dirPath: string | undefined,
    registry: Registry,
    pattern?: string,
    config?: BindingConfigLike | null,
  ): Promise<FunctionModule[]> {
    const actualPath =
      dirPath ?? asConfiguredString(config?.get('bindings.dir')) ?? DEFAULT_BINDING_DIR;
    const actualPattern =
      pattern ?? asConfiguredString(config?.get('bindings.pattern')) ?? DEFAULT_BINDING_PATTERN;

    if (!existsSync(actualPath) || !statSync(actualPath).isDirectory()) {
      throw new BindingFileInvalidError(actualPath, 'Directory does not exist');
    }

    const files = readdirSync(actualPath)
      .filter((f) => {
        // PROTOCOL_SPEC 5.12.6 clause 1 / 9.2.3: the pattern is matched with
        // Algorithm A25 against the FILENAME. The suffix comparison this
        // replaces was written for the default value and was wrong in the
        // direction that LOADS A FILE NOBODY ASKED FOR: `replace('*','')`
        // deletes the star wherever it sits, so `a*b.yaml` became `ab.yaml`
        // and `endsWith` accepted `zab.yaml` (#116).
        return matchGlob(actualPattern, f);
      })
      .sort();
    const results: FunctionModule[] = [];
    for (const f of files) {
      results.push(...(await this.loadBindings(join(actualPath, f), registry, config)));
    }
    return results;
  }

  async resolveTarget(targetString: string): Promise<(...args: unknown[]) => unknown> {
    if (!targetString.includes(':')) {
      throw new BindingInvalidTargetError(targetString);
    }

    const [modulePath, callableName] = targetString.split(':', 2);

    if (modulePath.includes('..')) {
      throw new BindingInvalidTargetError(
        `Module path '${modulePath}' must not contain '..' segments`,
      );
    }

    if (modulePath.startsWith('file:')) {
      throw new BindingInvalidTargetError(`Module path '${modulePath}' must not use file: URLs`);
    }

    // Enforce the trusted-package allowlist (when configured) BEFORE importing,
    // so an untrusted module is never loaded. Mirrors apcore-python.
    if (
      this._trustedPackagePrefixes !== null &&
      !this._trustedPackagePrefixes.some((prefix) => modulePath.startsWith(prefix))
    ) {
      throw new BindingInvalidTargetError(
        `Module path '${modulePath}' is not in the trusted package prefixes allowlist`,
      );
    }

    let mod: Record<string, unknown>;
    try {
      mod = await import(modulePath);
    } catch (e) {
      throw new BindingModuleNotFoundError(modulePath);
    }

    if (callableName.includes('.')) {
      const [className, methodName] = callableName.split('.', 2);
      const cls = mod[className];
      if (cls == null) {
        throw new BindingCallableNotFoundError(className, modulePath);
      }
      let instance: Record<string, unknown>;
      try {
        instance = new (cls as new () => Record<string, unknown>)();
      } catch (e) {
        throw new BindingNotCallableError(callableName, { cause: e as Error });
      }
      const method = instance[methodName];
      if (method == null) {
        throw new BindingCallableNotFoundError(callableName, modulePath);
      }
      if (typeof method !== 'function') {
        throw new BindingNotCallableError(targetString);
      }
      return method.bind(instance) as (...args: unknown[]) => unknown;
    }

    const result = mod[callableName];
    if (result == null) {
      throw new BindingCallableNotFoundError(callableName, modulePath);
    }
    if (typeof result !== 'function') {
      throw new BindingNotCallableError(targetString);
    }
    return result as (...args: unknown[]) => unknown;
  }

  private async _createModuleFromBinding(
    binding: Record<string, unknown>,
    bindingFileDir: string,
    filePath?: string,
  ): Promise<FunctionModule> {
    const targetString = binding['target'] as string;
    const func = await this.resolveTarget(targetString);
    const moduleId = binding['module_id'] as string;

    // Detect schema mode conflicts (protocol-spec §5.12.2)
    const modes: string[] = [];
    if ('auto_schema' in binding) modes.push('auto_schema');
    if ('input_schema' in binding || 'output_schema' in binding)
      modes.push('input_schema/output_schema');
    if ('schema_ref' in binding) modes.push('schema_ref');
    if (modes.length > 1) {
      throw new BindingSchemaModeConflictError(moduleId, modes, filePath);
    }
    if (binding['auto_schema'] === false) {
      throw new BindingSchemaInferenceFailedError(targetString, moduleId, filePath);
    }

    let inputSchema: TSchema;
    let outputSchema: TSchema;

    if ('input_schema' in binding || 'output_schema' in binding) {
      // Mode 1: explicit schemas
      const inputSchemaDict = (binding['input_schema'] as Record<string, unknown>) ?? {};
      const outputSchemaDict = (binding['output_schema'] as Record<string, unknown>) ?? {};
      inputSchema = jsonSchemaToTypeBox(inputSchemaDict);
      outputSchema = jsonSchemaToTypeBox(outputSchemaDict);
    } else if ('schema_ref' in binding) {
      // Mode 2: external reference
      const refPath = resolve(bindingFileDir, binding['schema_ref'] as string);
      if (!existsSync(refPath)) {
        throw new BindingFileInvalidError(refPath, 'Schema reference file not found');
      }
      let refData: Record<string, unknown>;
      try {
        refData = (yaml.load(readFileSync(refPath, 'utf-8')) as Record<string, unknown>) ?? {};
      } catch (e) {
        throw new BindingFileInvalidError(refPath, `YAML parse error: ${e}`);
      }
      inputSchema = jsonSchemaToTypeBox((refData['input_schema'] as Record<string, unknown>) ?? {});
      outputSchema = jsonSchemaToTypeBox(
        (refData['output_schema'] as Record<string, unknown>) ?? {},
      );
    } else {
      // Mode 3 (explicit auto_schema) or Mode 4 (implicit default = auto)
      // Try to infer schemas from the target module's exports.
      const [modulePath, symbolName] = targetString.split(':', 2);
      let inferred: { input: TSchema; output: TSchema } | null = null;
      try {
        const mod = (await import(modulePath)) as Record<string, unknown>;
        inferred = inferSchemasFromModule(mod, symbolName);
      } catch {
        // Module already resolved in resolveTarget; if re-import fails,
        // fall through to permissive/error below.
      }

      if (inferred) {
        inputSchema = inferred.input;
        outputSchema = inferred.output;
        // auto_schema: strict promises an OpenAI/Anthropic strict-compatible
        // schema. Reject at parse time when the inferred schema cannot be made
        // one (protocol-spec §5.12.5).
        if (binding['auto_schema'] === 'strict') {
          assertOpenAiStrictCompatible(inputSchema as unknown as Record<string, unknown>, {
            moduleId,
            side: 'input',
            filePath,
          });
          assertOpenAiStrictCompatible(outputSchema as unknown as Record<string, unknown>, {
            moduleId,
            side: 'output',
            filePath,
          });
        }
      } else if ('auto_schema' in binding && binding['auto_schema'] !== false) {
        // Explicit auto_schema but inference failed → error
        throw new BindingSchemaInferenceFailedError(targetString, moduleId, filePath);
      } else if ('auto_schema' in binding && binding['auto_schema'] === false) {
        // Explicit auto_schema: false → error (no mode left)
        throw new BindingSchemaInferenceFailedError(
          targetString,
          moduleId,
          filePath,
          'auto_schema is explicitly false; provide input_schema/output_schema or schema_ref instead.',
        );
      } else {
        throw new BindingSchemaInferenceFailedError(targetString, moduleId, filePath);
      }
    }

    return new FunctionModule({
      execute: async (inputs, context) => {
        const result = await func(inputs, context);
        if (result === null || result === undefined) return {};
        if (typeof result === 'object' && !Array.isArray(result))
          return result as Record<string, unknown>;
        return { result };
      },
      moduleId,
      inputSchema,
      outputSchema,
      description: (binding['description'] as string) ?? undefined,
      documentation: (binding['documentation'] as string) ?? undefined,
      tags: (binding['tags'] as string[]) ?? null,
      version: (binding['version'] as string) ?? '1.0.0',
      annotations: binding['annotations']
        ? parseAnnotations(binding['annotations'] as Record<string, unknown>)
        : undefined,
      metadata: binding['metadata'] as Record<string, unknown> | undefined,
      resources: binding['resources'] as ResourceLimits | undefined,
    });
  }
}
