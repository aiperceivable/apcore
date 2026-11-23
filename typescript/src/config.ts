/**
 * Configuration loading, validation, and environment variable overrides (Algorithm A12).
 * Supports legacy mode (flat YAML) and namespace mode (apcore top-level key).
 */

import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import process from 'node:process';
import yaml from 'js-yaml';
import {
  ConfigBindError,
  ConfigEnvMapConflictError,
  ConfigEnvPrefixConflictError,
  ConfigError,
  ConfigMountError,
  ConfigNamespaceDuplicateError,
  ConfigNamespaceReservedError,
  ConfigNotFoundError,
} from './errors.js';
import { jsonSchemaToTypeBox } from './schema/loader-pure.js';
import { SchemaValidator } from './schema/validator.js';
import { DEFAULTS, getDefault } from './config-defaults.js';
import { collectUndeclaredFrameworkKeys, PATH_TYPED_CONFIG_KEYS } from './config-key-surface.js';

// Re-exported so existing `import { DEFAULTS, getDefault } from './config.js'`
// paths keep working unchanged.
export { DEFAULTS, getDefault } from './config-defaults.js';

/** Environment variable prefix for legacy overrides. */
const ENV_PREFIX = 'APCORE_';

/**
 * Environment variable naming the configuration file to load (§9.14 discovery).
 *
 * apcore#88: this variable is an *argument to* `Config.load()` — it selects
 * which document is read — and only happens to share the `APCORE_` prefix that
 * §9.2 turns into configuration overrides. Left in the override map its suffix
 * becomes the dot-path `config.file`, a key no schema declares (checked
 * against `conformance/fixtures/config_key_governance.json`), which then sits
 * inside the **declared** document the §9.1 required-field check runs against.
 * `discoverConfigFile()` consumes it; `applyEnvOverrides` drops it.
 */
const ENV_CONFIG_FILE = 'APCORE_CONFIG_FILE';

/**
 * Configuration keys that MUST be declared explicitly, in legacy mode (dot-paths).
 *
 * PROTOCOL_SPEC §9.1: a key is required **only when it has no canonical
 * default**. Exactly two qualify — `version` and `project.name`. Every other
 * §9.1 key carries a default in `schemas/defaults.schema.json`
 * (`extensions.*`, `schema.*`, `acl.*`, `executor.*`, `sys_modules.*`,
 * `observability.*`, `stream.*`), so requiring it would reject a configuration
 * the framework resolves perfectly well. `schemas/apcore-config.schema.json`
 * declares the same two in its `required` array.
 *
 * §9.3 step 1: requiredness is evaluated against the **declared** document —
 * see {@link Config.getDeclared} — never against the tree `DEFAULTS` has been
 * merged into. A post-merge check can never fail, because the merge has
 * already supplied every key it would look for.
 */
const REQUIRED_FIELDS = ['version', 'project.name'] as const;

/**
 * Field constraints in legacy mode: field -> [validator, errorMessage].
 *
 * @internal Exported only so `system.control.update_config` can enforce the
 * registered constraint after a runtime `set` (post-set check + rollback),
 * mirroring Python's `_CONSTRAINTS`. Not part of the public package API.
 */
/**
 * Type guard mirroring Python's `isinstance(v, (int, float)) and not isinstance(v, bool)`.
 * In JS `typeof true === 'number'` is false, but values may arrive as booleans;
 * exclude them and NaN so numeric constraints behave like Python.
 */
function isNumber(v: unknown): v is number {
  return typeof v === 'number' && !Number.isNaN(v);
}

/**
 * Type guard mirroring Python's `isinstance(v, int) and not isinstance(v, bool)`.
 * Booleans are excluded (`typeof true === 'boolean'` already rules them out).
 */
function isInteger(v: unknown): v is number {
  return typeof v === 'number' && Number.isInteger(v);
}

/**
 * PROTOCOL_SPEC §10.1.1 requirement 3 — an endpoint nothing reads is a rejected
 * configuration.
 *
 * Shared by legacy and namespace mode: the disagreement is between two keys,
 * and which file layout declared them changes nothing about it.
 */
function otlpEndpointMismatchErrors(config: Config): string[] {
  const endpoint = config.get('observability.tracing.otlp_endpoint');
  if (endpoint === undefined || endpoint === null) return [];
  const exporter = config.get('observability.tracing.exporter') ?? 'stdout';
  if (exporter === 'otlp') return [];
  return [
    `observability.tracing.otlp_endpoint is set but observability.tracing.exporter is ` +
      `'${String(exporter)}', which does not read it. Set exporter to 'otlp', or remove the ` +
      `endpoint.`,
  ];
}

export const CONSTRAINTS: Record<string, [(v: unknown) => boolean, string]> = {
  'acl.default_effect': [(v) => v === 'allow' || v === 'deny', "must be 'allow' or 'deny'"],
  'observability.tracing.sampling_rate': [
    (v) => isNumber(v) && v >= 0.0 && v <= 1.0,
    'must be a number in [0.0, 1.0]',
  ],
  'observability.tracing.strategy': [
    (v) => v === 'full' || v === 'proportional' || v === 'error_first' || v === 'off',
    "must be 'full', 'proportional', 'error_first' or 'off'",
  ],
  'observability.tracing.exporter': [
    // `in_memory` is deliberately absent: PROTOCOL_SPEC §10.1.1 requirement 2.
    (v) => v === 'stdout' || v === 'otlp' || v === 'jaeger',
    "must be 'stdout', 'otlp' or 'jaeger'",
  ],
  'observability.tracing.otlp_endpoint': [
    (v) => v === null || (typeof v === 'string' && v.trim().length > 0),
    'must be a non-empty URL string, or null',
  ],
  'extensions.max_depth': [
    (v) => isInteger(v) && v >= 1 && v <= 16,
    'must be an integer in [1, 16]',
  ],
  'executor.default_timeout': [
    (v) => isInteger(v) && v >= 0,
    'must be a non-negative integer (milliseconds)',
  ],
  'executor.global_timeout': [
    (v) => isInteger(v) && v >= 0,
    'must be a non-negative integer (milliseconds)',
  ],
  'executor.max_call_depth': [(v) => isInteger(v) && v >= 1, 'must be a positive integer'],
  'executor.max_module_repeat': [(v) => isInteger(v) && v >= 1, 'must be a positive integer'],
  'sys_modules.error_history.max_entries_per_module': [
    (v) => isInteger(v) && v >= 1,
    'must be a positive integer',
  ],
  'sys_modules.error_history.max_total_entries': [
    (v) => isInteger(v) && v >= 1,
    'must be a positive integer',
  ],
  // A-D-03: middleware circuit-breaker + sys_modules events thresholds
  // (config-bus.md "Contract: Config.validate" value-constraints table).
  // Mirrors Python `_CONSTRAINTS` exactly (src/apcore/config.py).
  'sys_modules.events.thresholds.error_rate': [
    (v) => isNumber(v) && v >= 0.0 && v <= 1.0,
    'must be a number in [0.0, 1.0]',
  ],
  'sys_modules.events.thresholds.latency_p99_ms': [
    (v) => isNumber(v) && v > 0,
    'must be a positive number',
  ],
};

// ---------------------------------------------------------------------------
// Namespace registry (module-level singletons)
// ---------------------------------------------------------------------------

export type EnvStyle = 'nested' | 'flat' | 'auto';

const DEFAULT_MAX_DEPTH = 5;

interface NamespaceRegistration {
  name: string;
  schema: object | string | null;
  envPrefix: string; // auto-derived or explicit (never null after registration)
  defaults: Record<string, unknown> | null;
  envStyle: EnvStyle;
  maxDepth: number;
  envMap: Record<string, string> | null;
}

export const _globalNsRegistry = new Map<string, NamespaceRegistration>();
const _RESERVED_NAMESPACES = new Set(['apcore', '_config']);

/**
 * Public alias of the reserved-namespace set (PROTOCOL_SPEC §9.9.5).
 *
 * Top-level namespace names reserved by the apcore framework. Third-party
 * consumers (custom CLIs, framework integrations) MUST NOT register these
 * via `Config.registerNamespace`. Typed as `ReadonlySet<string>` to enforce
 * caller-side immutability at the type-system level.
 *
 * Re-exported from `apcore` for ergonomic top-level access:
 *   `import { RESERVED_NAMESPACES } from 'apcore';`
 */
export const RESERVED_NAMESPACES: ReadonlySet<string> = _RESERVED_NAMESPACES;
export const _globalEnvMap = new Map<string, string>(); // bare env var → top-level key
export const _envMapClaimed = new Map<string, string>(); // env var → owner (conflict detection)
export const _envPrefixUsed = new Set<string>();

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

function deepMergeDicts(
  base: Record<string, unknown>,
  override: Record<string, unknown>,
): Record<string, unknown> {
  const result: Record<string, unknown> = { ...base };
  for (const [key, value] of Object.entries(override)) {
    if (
      key in result &&
      typeof result[key] === 'object' &&
      result[key] !== null &&
      !Array.isArray(result[key]) &&
      typeof value === 'object' &&
      value !== null &&
      !Array.isArray(value)
    ) {
      result[key] = deepMergeDicts(
        result[key] as Record<string, unknown>,
        value as Record<string, unknown>,
      );
    } else {
      result[key] = value;
    }
  }
  return result;
}

function getNested(
  data: Record<string, unknown>,
  dotPath: string,
  defaultValue?: unknown,
): unknown {
  const parts = dotPath.split('.');
  let current: unknown = data;
  for (const part of parts) {
    if (
      current !== null &&
      typeof current === 'object' &&
      !isForbiddenSegment(part) &&
      Object.hasOwn(current as Record<string, unknown>, part)
    ) {
      current = (current as Record<string, unknown>)[part];
    } else {
      return defaultValue;
    }
  }
  return current;
}

/**
 * Dot-path segments that must never be followed into the host object graph.
 *
 * A configuration dot-path addresses DATA. `__proto__` is an inherited accessor
 * on every plain object, so a membership test written as `part in current` is
 * true for it and `typeof Object.prototype === 'object'`, which let
 * `setNested(data, '__proto__.polluted', v)` walk out of `data` entirely and
 * assign to `Object.prototype` — polluting every object in the process while
 * leaving `data`'s own keys untouched.
 *
 * The path needs no module call and no ACL decision to reach: the `APCORE_`
 * env-override loader turns `APCORE_____PROTO_____POLLUTED=x` into the dot-path
 * `__proto__.polluted` (`__` is the escape for a literal `_`), so merely having
 * that variable in the environment at `Config.load()` was enough.
 * `system.control.update_config` is a second door, its `key` being a
 * caller-supplied string behind a denylist rather than an allowlist.
 *
 * apcore-python and apcore-rust are unaffected — a Python dict has no prototype
 * chain and Rust has no ambient object graph, so both store `__proto__` as an
 * ordinary nested key. Storing it as data is the behaviour this guard restores.
 */
const FORBIDDEN_PATH_SEGMENTS = new Set(['__proto__', 'constructor', 'prototype']);

function isForbiddenSegment(part: string): boolean {
  return FORBIDDEN_PATH_SEGMENTS.has(part);
}

function setNested(data: Record<string, unknown>, dotPath: string, value: unknown): void {
  const parts = dotPath.split('.');
  // Refuse the whole write rather than sanitising a segment out of it: a caller
  // who wrote `__proto__.polluted` gets no key at all, instead of silently
  // getting `polluted` at the root.
  if (parts.some(isForbiddenSegment)) {
    console.warn(
      `[apcore:config] Refusing to set '${dotPath}': the path contains a reserved ` +
        `object-graph segment (${[...FORBIDDEN_PATH_SEGMENTS].join(', ')}). ` +
        'Configuration paths address data, not the host object graph.',
    );
    return;
  }
  let current = data;
  for (const part of parts.slice(0, -1)) {
    // `Object.hasOwn`, not `in`: `in` walks the prototype chain, which is what
    // allowed the escape described above.
    if (
      !Object.hasOwn(current, part) ||
      typeof current[part] !== 'object' ||
      current[part] === null
    ) {
      current[part] = {};
    }
    current = current[part] as Record<string, unknown>;
  }
  current[parts[parts.length - 1]] = value;
}

// Python int()/float() grammars (used by _coerce_env_value). Python strips
// surrounding whitespace and accepts an optional sign; int() additionally
// accepts leading zeros but rejects decimal points and exponents (those fall
// through to float()). We deliberately do NOT accept hex/octal/binary prefixes
// or underscore digit separators here — env values in the wild never use them
// and Rust's i64/f64 parse rejects them too, so excluding them keeps the three
// SDKs aligned. Cross-language parity: apcore-python config.py `_coerce_env_value`
// (int then float) and apcore-rust `coerce_env_value` (sync finding A-D-008).
const PY_INT_RE = /^[+-]?[0-9]+$/;
const PY_FLOAT_RE = /^[+-]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][+-]?[0-9]+)?$/;

function coerceEnvValue(value: string): unknown {
  if (value.toLowerCase() === 'true') return true;
  if (value.toLowerCase() === 'false') return false;
  const trimmed = value.trim();
  // int(value): integral strings coerce to numbers even with leading zeros
  // (e.g. "08") or a leading sign (e.g. "+5").
  if (PY_INT_RE.test(trimmed)) {
    const asInt = Number(trimmed);
    if (Number.isFinite(asInt)) return asInt;
  }
  // float(value): decimals and exponent forms (e.g. "1e0") coerce to numbers.
  if (PY_FLOAT_RE.test(trimmed)) {
    const asFloat = Number(trimmed);
    // Match Rust's serde_json::Number::from_f64 which rejects non-finite
    // values; such inputs stay strings.
    if (Number.isFinite(asFloat)) return asFloat;
  }
  return value;
}

/**
 * The §9.2.1 path-typed key set, spelled the way an environment override can
 * reach it: `extensions.roots[]`'s element marker is notation for "every
 * element of this list", never part of a dot-path a variable produces.
 */
const PATH_TYPED_DOT_PATHS: ReadonlySet<string> = new Set(
  PATH_TYPED_CONFIG_KEYS.map((key) => (key.endsWith('[]') ? key.slice(0, -'[]'.length) : key)),
);

/**
 * PROTOCOL_SPEC §9.2.1 requirement 5 — **an empty string is not a path.**
 *
 * §9.2's override rule and shell ergonomics collide here. `export
 * APCORE_ACL_ROOT=` and a variable inherited empty from a container spec are
 * both *set* as far as the tooling is concerned, so an unguarded
 * implementation lets `''` win the top precedence tier and silently **blank
 * out** a directory the configuration file correctly declared — and `''` then
 * resolves against the working directory, which is the working directory
 * itself. It is a legal relative path to every filesystem API and never the
 * one an operator meant. The value is discarded and resolution falls through
 * to the next tier, exactly as if the variable had been unset.
 *
 * This lives at the point the override is APPLIED, not at each consumer, so a
 * key added to §9.2.1 later is covered without anyone remembering to guard it.
 * The same shape is already handled one line below for `APCORE_CONFIG_FILE`,
 * where an empty value injected a phantom `config.file` key (apcore#88).
 *
 * The warning is §9.2.1's MAY: dropping an override the operator believes they
 * set is exactly the class of silent failure this requirement exists to end.
 */
function discardsEmptyPathValue(dotPath: string, value: unknown, envKey: string): boolean {
  if (value !== '' || !PATH_TYPED_DOT_PATHS.has(dotPath)) return false;
  console.warn(
    `[apcore:config] ${envKey} is set but empty, and '${dotPath}' is a path-typed ` +
      'key (PROTOCOL_SPEC §9.2.1): an empty string is not a path, so the override ' +
      'is discarded and the value falls through to the configuration file or the ' +
      'default. Unset the variable to silence this, or give it a real directory.',
  );
  return true;
}

export function applyEnvOverrides(data: Record<string, unknown>): Record<string, unknown> {
  const result = JSON.parse(JSON.stringify(data)) as Record<string, unknown>;
  const env = process.env;
  // Global env_map (bare env var → dotted config path) applies in legacy mode
  // too, mirroring apcore-python config.py:259 which consults _GLOBAL_ENV_MAP
  // in the legacy env-override path before the APCORE_ prefix scan.
  for (const [envVar, configKey] of _globalEnvMap.entries()) {
    const envValue = env[envVar];
    if (envValue === undefined) continue;
    const coerced = coerceEnvValue(envValue);
    if (discardsEmptyPathValue(configKey, coerced, envVar)) continue;
    setNested(result, configKey, coerced);
  }
  for (const [envKey, envValue] of Object.entries(env)) {
    if (!envKey.startsWith(ENV_PREFIX) || envValue === undefined) continue;
    // apcore#88: the file selector is consumed by discoverConfigFile(); it is
    // an argument to load(), not a value the document declares. Kept here it
    // would inject the phantom key `config.file`.
    if (envKey === ENV_CONFIG_FILE) continue;
    const suffix = envKey.slice(ENV_PREFIX.length);
    if (!suffix) continue;
    // Convert: single _ -> . (separator), double __ -> literal _
    const dotPath = suffix
      .toLowerCase()
      .replace(/__/g, '\x00')
      .replace(/_/g, '.')
      .replace(/\x00/g, '_');
    const coerced = coerceEnvValue(envValue);
    if (discardsEmptyPathValue(dotPath, coerced, envKey)) continue;
    setNested(result, dotPath, coerced);
  }
  return result;
}

/**
 * Apply per-namespace env overrides using longest-prefix-match dispatch.
 *
 * For each env var, finds the registered namespace whose envPrefix is the
 * longest matching prefix, strips it, converts separators, and writes to
 * that namespace's subtree in data.
 */
/**
 * Convert env var suffix to dot-path with a depth limit.
 * After max_depth segments, remaining _ are preserved as literal.
 * Double __ always becomes literal _.
 */
export function envSuffixToDotPathWithDepth(suffix: string, maxDepth: number): string {
  const lower = suffix.toLowerCase();
  const result: string[] = [];
  let dotCount = 0;
  let i = 0;
  while (i < lower.length) {
    if (lower[i] === '_') {
      if (i + 1 < lower.length && lower[i + 1] === '_') {
        result.push('_'); // double __ → literal _
        i += 2;
      } else if (dotCount < maxDepth - 1) {
        // Stop at maxDepth segments (maxDepth - 1 dots)
        result.push('.');
        dotCount++;
        i++;
      } else {
        result.push('_'); // depth limit reached
        i++;
      }
    } else {
      result.push(lower[i]);
      i++;
    }
  }
  return result.join('').replace(/^\.+|\.+$/g, '');
}

/**
 * Try to match suffix against keys in tree (recursive).
 * Returns resolved dot-path or null if no match.
 */
function matchSuffixToTree(
  suffix: string,
  tree: Record<string, unknown>,
  depth: number,
  maxDepth: number,
): string | null {
  // 1. Try full suffix as a flat key.
  if (suffix in tree) return suffix;

  // 2. Depth limit reached.
  if (depth >= maxDepth - 1) return null;

  // 3. Try splitting at each underscore position.
  for (let i = 1; i < suffix.length - 1; i++) {
    if (suffix[i] !== '_') continue;
    const prefix = suffix.slice(0, i);
    const remainder = suffix.slice(i + 1);
    const subtree = tree[prefix];
    if (subtree !== null && typeof subtree === 'object' && !Array.isArray(subtree)) {
      const sub = matchSuffixToTree(
        remainder,
        subtree as Record<string, unknown>,
        depth + 1,
        maxDepth,
      );
      if (sub !== null) return prefix + '.' + sub;
    }
  }
  return null;
}

/**
 * Resolve env var suffix to a config key using auto mode.
 * Matches against defaults tree, falls back to nested conversion.
 */
function autoResolveSuffix(
  suffix: string,
  defaults: Record<string, unknown> | null,
  maxDepth: number,
): string {
  const lower = suffix.toLowerCase();
  if (defaults === null) return envSuffixToDotPathWithDepth(lower, maxDepth);
  const result = matchSuffixToTree(lower, defaults, 0, maxDepth);
  if (result !== null) return result;
  return envSuffixToDotPathWithDepth(lower, maxDepth);
}

/**
 * Resolve env var suffix to { key, isNested } based on registration env_style.
 */
function resolveEnvSuffix(
  suffix: string,
  reg: NamespaceRegistration,
): { key: string; isNested: boolean } {
  if (reg.envStyle === 'flat') {
    // A-D-048: flat-style keys are the suffix lowercased verbatim (Python/Rust
    // canonical). Do NOT collapse `__`→`_` or strip a leading `_`; the literal
    // key (including double underscores) is what the flat namespace expects.
    const key = suffix.toLowerCase();
    return { key, isNested: false };
  }
  if (reg.envStyle === 'auto') {
    const key = autoResolveSuffix(suffix, reg.defaults, reg.maxDepth);
    return { key, isNested: key.includes('.') };
  }
  // nested (default)
  const key = envSuffixToDotPathWithDepth(suffix, reg.maxDepth);
  return { key, isNested: true };
}

export function applyNamespaceEnvOverrides(data: Record<string, unknown>): Record<string, unknown> {
  const result = JSON.parse(JSON.stringify(data)) as Record<string, unknown>;
  const env = process.env;

  // Sort registrations by envPrefix length descending (longest first)
  const apcoreRegistration: NamespaceRegistration = {
    name: 'apcore', schema: null, envPrefix: 'APCORE', defaults: DEFAULTS,
    envStyle: 'nested', maxDepth: DEFAULT_MAX_DEPTH, envMap: null,
  };
  const registrations = [..._globalNsRegistry.values(), apcoreRegistration]
    .filter((r) => r.envPrefix)
    .sort((a, b) => b.envPrefix.length - a.envPrefix.length);

  for (const [envKey, envValue] of Object.entries(env)) {
    if (envValue === undefined) continue;
    if (envKey === ENV_CONFIG_FILE) continue;
    const coerced = coerceEnvValue(envValue);

    // 1. Global env_map (bare env var → top-level key).
    if (_globalEnvMap.has(envKey)) {
      const configKey = _globalEnvMap.get(envKey)!;
      setNested(result, configKey, coerced);
      continue;
    }

    // 2. Namespace env_map (bare env var → namespace key).
    // ... search in all registered namespaces ...
    let handledByNsMap = false;
    for (const reg of _globalNsRegistry.values()) {
      if (reg.envMap && envKey in reg.envMap) {
        const configKey = reg.envMap[envKey];
        if (typeof result[reg.name] !== 'object' || result[reg.name] === null) {
          result[reg.name] = {};
        }
        setNested(result[reg.name] as Record<string, unknown>, configKey, coerced);
        handledByNsMap = true;
        break;
      }
    }
    if (handledByNsMap) continue;

    // 3. Prefix-based dispatch.
    for (const reg of registrations) {
      if (envKey.startsWith(`${reg.envPrefix}_`)) {
        let suffix = envKey.slice(reg.envPrefix.length);
        if (!suffix) continue;
        // Strip leading _ separator between prefix and suffix
        if (suffix.startsWith('_')) suffix = suffix.slice(1);
        if (!suffix) continue;

        const { key, isNested } = resolveEnvSuffix(suffix, reg);
        if (!key) continue;
        // §9.2.1 requirement 5 applies in namespace mode too; the path-typed
        // set is spelled from the apcore namespace's root.
        const fullPath = reg.name === 'apcore' ? key : `${reg.name}.${key}`;
        if (discardsEmptyPathValue(fullPath, coerced, envKey)) break;

        if (typeof result[reg.name] !== 'object' || result[reg.name] === null) {
          result[reg.name] = {};
        }

        if (isNested) {
          setNested(result[reg.name] as Record<string, unknown>, key, coerced);
        } else {
          (result[reg.name] as Record<string, unknown>)[key] = coerced;
        }
        break;
      }
    }
  }
  return result;
}

/**
 * Resolve the namespace from a dot-path like "apcore-mcp.transport".
 * The namespace portion may contain hyphens, so we cannot simply split on ".".
 * We match against known registered namespaces (longest match first).
 */
function resolveNamespacePath(key: string): { namespace: string; subPath: string } | null {
  // Sort known namespaces by length descending for longest-match
  const knownNamespaces = Array.from(_globalNsRegistry.keys())
    .concat(Array.from(_RESERVED_NAMESPACES))
    .sort((a, b) => b.length - a.length);

  for (const ns of knownNamespaces) {
    if (key === ns) {
      return { namespace: ns, subPath: '' };
    }
    if (key.startsWith(ns + '.')) {
      return { namespace: ns, subPath: key.slice(ns.length + 1) };
    }
  }

  // Fallback: use naive first-segment split
  const dotIndex = key.indexOf('.');
  if (dotIndex === -1) {
    return { namespace: key, subPath: '' };
  }
  return { namespace: key.slice(0, dotIndex), subPath: key.slice(dotIndex + 1) };
}

// ---------------------------------------------------------------------------
// Config discovery (§9.14)
// ---------------------------------------------------------------------------

/**
 * Search for a config file in the standard discovery order (§9.14).
 * Returns the path of the first found file, or null if none found.
 *
 * `$APCORE_CONFIG_FILE` is *consumed* here: `applyEnvOverrides` skips it so it
 * never becomes the `config.file` override (apcore#88).
 */
export function discoverConfigFile(): string | null {
  const env = process.env;

  const envPath = env[ENV_CONFIG_FILE];
  if (envPath) return envPath;

  const cwdCandidates = ['project.yaml', 'project.yml', 'apcore.yaml', 'apcore.yml'];
  for (const name of cwdCandidates) {
    if (existsSync(name)) return name;
  }

  for (const candidate of userLevelConfigPaths()) {
    if (existsSync(candidate)) return candidate;
  }

  return null;
}

/**
 * The §9.14 **user-level** configuration paths — discovery tiers 6 and 7 — in
 * discovery order: the XDG location (`~/Library/Application Support/apcore/`
 * on macOS, `~/.config/apcore/` elsewhere), then legacy `~/.apcore/`.
 *
 * Split out of {@link discoverConfigFile} because {@link Config.projectRoot}
 * needs the same two paths for the opposite purpose: to *recognise* a config
 * that came from one of these tiers, which is the case where the config file's
 * directory is the wrong project root (apcore#113). Keeping one definition
 * keeps the two answers from drifting apart.
 */
export function userLevelConfigPaths(): string[] {
  const home = homedir();
  const xdgConfig =
    process.platform === 'darwin'
      ? join(home, 'Library', 'Application Support', 'apcore', 'config.yaml')
      : join(home, '.config', 'apcore', 'config.yaml');
  return [xdgConfig, join(home, '.apcore', 'config.yaml')];
}

// ---------------------------------------------------------------------------
/**
 * PROTOCOL_SPEC §9.2.4 — the declared configuration keys that reach no
 * consumer in any implementation (apcore#118). Order is the order they are
 * reported in, so two SDKs name them the same way.
 *
 * Ten when the window opened in spec v1.39.0; seven since v1.44.0, which gave
 * `observability.tracing.enabled` / `.sampling_rate` / `.exporter` consumers
 * (§10.1.1) and cancelled their withdrawal. A key that has left the table MUST
 * NOT warn — §9.2.4 requirement 1.
 *
 * `acl.default_effect` joined in v1.47.0 as §9.1.3's first application: it is
 * read from the ACL FILE, and the `apcore.yaml` twin reaches nothing. Ordered
 * first because §9.2.4's table lists it first.
 */
const DEPRECATED_INERT_KEYS: readonly string[] = [
  'acl.default_effect',
  'observability.metrics.enabled',
  'observability.metrics.exporter',
  'logging.level',
  'logging.format',
  'acl.audit.enabled',
  'acl.audit.include_denied',
  'acl.audit.log_level',
  // D-137: names built-in middleware that is not installed; remove() or
  // pipeline.remove is the working mechanism.
  'middleware.disabled',
  // D-150: discovery runs when discover() is called; nothing reads this.
  'extensions.auto_discover',
];

// Project root (§9.2.2 deprecation phase — apcore#113)
// ---------------------------------------------------------------------------

/**
 * NOTE ON CADENCE — there is deliberately no `_projectRootDeprecationWarned`
 * flag here, and no test hook to reset one.
 *
 * PROTOCOL_SPEC §9.2.2 requirement 2 fixes the cadence at **once per
 * configuration load, never once per process**, and says implementations
 * **MUST NOT** suppress the notice with process-global state. This SDK had
 * exactly that flag. It made emission order-dependent: in a process that loads
 * several configurations, whichever load ran first consumed the warning, so a
 * later affected document was silent and the operator could not tell which one
 * triggered it. The same global was a test-isolation hazard — one test consumed
 * the warning another test needed — which is why the hook below it existed at
 * all. De-duplication for log volume is the host logging layer's job.
 *
 * See {@link Config._warnProjectRootDeprecation}.
 */

/**
 * Whether `value` is a path-typed value that a change of resolution base would
 * move: a non-empty relative string. An absolute path is already anchored, and
 * a missing or non-string value is not a path at all.
 */
function isRelativePathValue(value: unknown): boolean {
  return typeof value === 'string' && value !== '' && !isAbsolute(value);
}

/**
 * The path carried by one `extensions.roots` element, which §9.2.1 allows in
 * either the bare-string form or the `{ root, namespace }` form.
 */
function extractRootPath(element: unknown): unknown {
  if (typeof element === 'string') return element;
  if (element !== null && typeof element === 'object' && !Array.isArray(element)) {
    return (element as Record<string, unknown>)['root'];
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Config class
// ---------------------------------------------------------------------------

/**
 * Configuration system with YAML loading, env overrides, and validation.
 *
 * Merge priority (highest wins): environment variables > mount data > config file > namespace defaults > defaults.
 *
 * Two modes:
 * - Legacy mode: top-level YAML has no "apcore" key. Backward compatible.
 * - Namespace mode: top-level YAML has "apcore" key. Enables namespace features.
 *
 * Backward compatible: `new Config(data)` still works for in-memory configuration.
 */
/**
 * Report every `obs.redaction.regex_patterns` entry that does not compile.
 *
 * PROTOCOL_SPEC §9.2.3 requirement 6d. All three SDKs previously handled an
 * unusable pattern the same way — skip it and carry on — so the divergence was
 * never in the failure *policy* but in what counts as a failure: JavaScript
 * rejects an inline `(?i)`, the Rust `regex` crate refuses lookaround and
 * backreferences by design. Each SDK reports what ITS engine cannot compile,
 * which is what makes an engine-specific rejection visible at deploy rather
 * than never.
 */
function uncompilableRegexPatternErrors(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const out: string[] = [];
  value.forEach((pattern, index) => {
    if (typeof pattern !== 'string' || pattern === '') return;
    try {
      new RegExp(pattern, 'i');
    } catch (err) {
      out.push(
        `obs.redaction.regex_patterns[${index}] does not compile and would redact nothing: ` +
          `${JSON.stringify(pattern)} (${String(err)}). Patterns should stay inside the ` +
          `portable subset — no lookaround, no backreferences, no inline (?i) flags ` +
          `(PROTOCOL_SPEC 9.2.3 requirement 6).`,
      );
    }
  });
  return out;
}

/**
 * PROTOCOL_SPEC §9.6.3's `allow_unknown` row (apcore#118, decision D-69).
 *
 * Both halves of that row were inert. `allow_unknown: false` is documented as
 * "silently ignored (not stored)" and the namespace was stored anyway, so
 * `get()` answered for it; `allow_unknown: true` is documented as "stored,
 * accessible, **WARN logged**" and no implementation logged anything. Fixing
 * one without the other would leave the row half true, and both live here.
 *
 * **Namespace mode only, by construction.** §9.6.3 is about *namespaces*, and a
 * legacy document has none — its root IS the `apcore` namespace, so an
 * unrecognised top-level key there is a framework key governed by §9.14's walk
 * under `strict`, not by this field. `strict`'s own clause (b) says it "applies
 * in legacy mode too", which is the specification saying clause (a) does not.
 *
 * Only a deployment that explicitly writes `allow_unknown: false` changes
 * behaviour, and what changes is that it finally gets the published contract
 * instead of a no-op — but the change is real: a `get()` that returned a value
 * now returns `undefined`.
 */
function applyAllowUnknown(merged: Record<string, unknown>): Record<string, unknown> {
  // An ABSENT `_config` is the default pair `strict: false, allow_unknown:
  // true`, not an exemption: §9.6.3's matrix describes the defaults, so a
  // document that declares an unregistered namespace and no `_config` at all is
  // the row that warns. This is not the blanket warning §9.2.2 rejects — it
  // fires on a condition specific to the document (there IS an unregistered
  // namespace), never on every configuration ever loaded.
  const rawMeta = merged['_config'];
  const meta: Record<string, unknown> =
    rawMeta !== null && typeof rawMeta === 'object' && !Array.isArray(rawMeta)
      ? (rawMeta as Record<string, unknown>)
      : {};
  if (meta['strict'] === true) {
    // `strict` already rejects an unknown namespace outright (clause a), so this
    // field is "only relevant when strict: false" per §9.6.3's own comment.
    return merged;
  }

  const known = new Set([...Array.from(_globalNsRegistry.keys()), 'apcore', '_config']);
  const unknown = Object.keys(merged).filter((k) => !known.has(k)).sort();
  if (unknown.length === 0) return merged;

  if (meta['allow_unknown'] !== false) {
    console.warn(
      `[apcore:config] Configuration declares ${unknown.length} namespace(s) that no ` +
        `package has registered: ${unknown.join(', ')}. They are stored and readable ` +
        `through get(), and NOT validated against any schema (PROTOCOL_SPEC §9.6.3). Set ` +
        `_config.allow_unknown: false to have them dropped instead, or _config.strict: ` +
        `true to reject them.`,
    );
    return merged;
  }

  const dropped: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(merged)) {
    if (!unknown.includes(key)) dropped[key] = value;
  }
  return dropped;
}


export class Config {
  private _data: Record<string, unknown>;
  /**
   * The **declared** document: the configuration exactly as its source states
   * it — parsed file + environment overrides + runtime `set()`/`mount()` —
   * with the `DEFAULTS` table (and namespace defaults) NOT merged in.
   *
   * `_data` answers "what value does this key resolve to?"; `_declared`
   * answers "did anybody actually say so?". `validate()` needs the second
   * question for its required-field check (PROTOCOL_SPEC §9.3 step 1);
   * asking the first can never fail once defaults are merged.
   */
  private _declared: Record<string, unknown>;
  private _yamlPath: string | null = null;
  private _mode: 'legacy' | 'namespace' = 'legacy';
  private _mounts: Map<string, Record<string, unknown>> = new Map();
  /**
   * Whether the originating `Config.load()` requested validation. `reload()`
   * re-applies the same policy, so a config loaded with the default
   * `validate: true` is re-validated on every reload instead of silently
   * accepting a file that has since become invalid.
   */
  private _validateOnLoad = true;

  constructor(data?: Record<string, unknown>, _envStyle: EnvStyle = 'auto') {
    this._data = data ?? {};
    // A bare `new Config(data)` has no default table merged into it, so every
    // key the caller passed is declared. `Config.load` / `Config.fromDefaults`
    // overwrite this with the pre-merge document.
    this._declared = JSON.parse(JSON.stringify(this._data)) as Record<string, unknown>;
  }

  // -------------------------------------------------------------------------
  // Static namespace registry methods
  // -------------------------------------------------------------------------

  /**
   * Returns true if the current environment is a browser (filesystem not
   * available).
   *
   * @deprecated since v0.21.1 — environment detection is now bundler-time
   * via `package.json` `exports.browser` / `node` conditions, so the Node
   * build always returns `false` and the browser build never imports
   * `Config` at all. This method is retained for downstream code that
   * historically branched on it; remove your call sites and the method
   * itself will be removed in a future minor release.
   */
  static isBrowser(): boolean {
    return typeof process === 'undefined' || !process.versions?.node;
  }

  /**
   * The closed set of path-typed configuration keys (PROTOCOL_SPEC §9.2.1).
   *
   * A path-typed key is one whose value is a filesystem path. The set is
   * declared canonically by `"x-apcore-path": true` in
   * `schemas/apcore-config.schema.json`; this returns that projection, sorted.
   *
   * `extensions.roots` is reported as `extensions.roots[]` because it is
   * list-valued and every element carries a path.
   *
   * Note what this does NOT tell you: what a *relative* value in one of these
   * keys resolves against. That base is unspecified as of spec v1.34.0 and
   * currently differs between keys — `acl.root` resolves against the config
   * file's directory, `schema.root` against the process CWD.
   */
  static pathTypedKeys(): readonly string[] {
    return PATH_TYPED_CONFIG_KEYS;
  }

  /**
   * Register a namespace globally.
   *
   * Throws:
   * - ConfigNamespaceReservedError if name is in the reserved set.
   * - ConfigNamespaceDuplicateError if name is already registered.
   * - ConfigEnvPrefixConflictError if envPrefix is already used or matches the
   *   an already-registered prefix.
   */
  static registerNamespace(options: {
    name: string;
    schema?: object | string | null;
    envPrefix?: string | null;
    defaults?: Record<string, unknown> | null;
    envStyle?: EnvStyle | null;
    maxDepth?: number | null;
    envMap?: Record<string, string> | null;
  }): void {
    const {
      name,
      schema = null,
      envPrefix: rawEnvPrefix = null,
      defaults = null,
      envStyle: rawEnvStyle = null,
      maxDepth: rawMaxDepth = null,
      envMap = null,
    } = options;
    const envStyle: EnvStyle = rawEnvStyle ?? 'auto';
    if (envStyle !== 'nested' && envStyle !== 'flat' && envStyle !== 'auto') {
      throw new Error(`envStyle must be 'nested', 'flat', or 'auto', got '${envStyle as string}'`);
    }
    const maxDepth = rawMaxDepth ?? DEFAULT_MAX_DEPTH;
    const envPrefix = rawEnvPrefix ?? name.toUpperCase().replace(/-/g, '_');

    if (_RESERVED_NAMESPACES.has(name)) {
      throw new ConfigNamespaceReservedError(name);
    }
    if (envPrefix === 'APCORE') throw new ConfigNamespaceReservedError(name);
    if (_globalNsRegistry.has(name)) {
      throw new ConfigNamespaceDuplicateError(name);
    }
    if (_envPrefixUsed.has(envPrefix)) {
      throw new ConfigEnvPrefixConflictError(envPrefix);
    }

    // Validate env_map: no env var can be claimed twice.
    if (envMap !== null) {
      for (const envVar of Object.keys(envMap)) {
        if (_envMapClaimed.has(envVar)) {
          throw new ConfigEnvMapConflictError(envVar, _envMapClaimed.get(envVar)!);
        }
      }
      for (const envVar of Object.keys(envMap)) {
        _envMapClaimed.set(envVar, name);
      }
    }

    _globalNsRegistry.set(name, { name, schema, envPrefix, defaults, envStyle, maxDepth, envMap });
    _envPrefixUsed.add(envPrefix);
  }

  /**
   * Register global bare env var → top-level config key mappings.
   */
  static envMap(mapping: Record<string, string>): void {
    for (const envVar of Object.keys(mapping)) {
      if (_envMapClaimed.has(envVar)) {
        throw new ConfigEnvMapConflictError(envVar, _envMapClaimed.get(envVar)!);
      }
    }
    for (const [envVar, configKey] of Object.entries(mapping)) {
      _globalEnvMap.set(envVar, configKey);
      _envMapClaimed.set(envVar, '__global__');
    }
  }

  /**
   * Return a snapshot of all registered namespaces.
   */
  static registeredNamespaces(): Array<{
    name: string;
    envPrefix: string | null;
    hasSchema: boolean;
  }> {
    return Array.from(_globalNsRegistry.values()).map((r) => ({
      name: r.name,
      envPrefix: r.envPrefix,
      hasSchema: r.schema !== null,
    }));
  }

  /**
   * Top-level namespace names reserved by the apcore framework
   * (PROTOCOL_SPEC §9.9.5).
   *
   * Returns the single source of truth referenced by
   * {@link Config.registerNamespace} to enforce `CONFIG_NAMESPACE_RESERVED`
   * (§9.5.1 rules 3 and 4). Static getter — callable without
   * instantiating `Config`, so third-party consumers (custom CLIs,
   * framework integrations) can fail-fast on user-supplied namespace
   * names before invoking `registerNamespace`.
   *
   * Typed as `ReadonlySet<string>` so caller-side mutation attempts fail
   * at compile time.
   */
  static get reservedNamespaces(): ReadonlySet<string> {
    return _RESERVED_NAMESPACES;
  }

  // -------------------------------------------------------------------------
  // Static factory methods
  // -------------------------------------------------------------------------

  /**
   * Load configuration from a YAML file with env overrides.
   *
   * Auto-detects mode:
   * - Namespace mode: top-level "apcore" key present.
   * - Legacy mode: otherwise (backward compatible).
   */
  static load(yamlPath?: string, options?: { validate?: boolean }): Config {
    if (yamlPath === undefined || yamlPath === null) {
      const found = discoverConfigFile();
      if (found === null) return Config.fromDefaults();
      yamlPath = found;
    }

    if (!existsSync(yamlPath)) {
      throw new ConfigNotFoundError(yamlPath);
    }

    let fileData: unknown;
    try {
      const content = readFileSync(yamlPath, 'utf-8');
      fileData = yaml.load(content);
    } catch (e) {
      if (e instanceof ConfigNotFoundError) throw e;
      throw new ConfigError(`Invalid YAML in ${yamlPath}: ${e}`);
    }

    if (fileData === null || fileData === undefined) {
      fileData = {};
    }
    if (typeof fileData !== 'object' || Array.isArray(fileData)) {
      throw new ConfigError(`Config file must be a mapping, got ${typeof fileData}`);
    }

    const rawData = fileData as Record<string, unknown>;
    // Namespace mode requires "apcore" key to be an object/mapping — not null, scalar, or array.
    const apcoreValue = rawData['apcore'];
    const isNamespaceMode =
      apcoreValue !== null &&
      apcoreValue !== undefined &&
      typeof apcoreValue === 'object' &&
      !Array.isArray(apcoreValue);

    let config: Config;
    // The declared document (§9.3 step 1): the file as written, plus env
    // overrides, with NO default table merged in. Built alongside the merged
    // tree rather than derived from it — once defaults are merged the two are
    // indistinguishable, which is what made the required-field check dead.
    let declared: Record<string, unknown>;

    if (isNamespaceMode) {
      // Namespace mode: apply namespace defaults, file data, then env overrides
      let merged: Record<string, unknown> = {};

      // Apply namespace defaults first
      for (const reg of _globalNsRegistry.values()) {
        if (reg.defaults !== null) {
          merged[reg.name] = JSON.parse(JSON.stringify(reg.defaults));
        }
      }

      // Merge file data over defaults
      merged = deepMergeDicts(merged, rawData);

      // D-146: one dispatch pass, including the unmatched APCORE_ fallback.
      merged = applyNamespaceEnvOverrides(merged);

      // Same pipeline over the raw file only — no namespace defaults seeded.
      declared = JSON.parse(JSON.stringify(rawData)) as Record<string, unknown>;
      declared = applyNamespaceEnvOverrides(declared);

      // §9.6.3's `allow_unknown` row, both halves of which were inert
      // (apcore#118, decision D-69). Applied AFTER the env overrides so a
      // namespace that exists only because of an `APCORE_*` variable is
      // treated the same as one written in the file.
      merged = applyAllowUnknown(merged);

      config = new Config(merged);
      config._mode = 'namespace';
    } else {
      // Legacy mode: merge defaults < file < env
      let merged = deepMergeDicts(DEFAULTS, rawData);
      merged = applyEnvOverrides(merged);
      declared = applyEnvOverrides(rawData);
      config = new Config(merged);
      config._mode = 'legacy';
    }

    config._declared = declared;
    config._yamlPath = yamlPath;
    config._validateOnLoad = options?.validate !== false;

    if (config._validateOnLoad) {
      config.validate();
    }

    // §13.2 deprecation phase for apcore#113. After validation, so a config
    // that is rejected outright does not also lecture about path resolution.
    config._warnProjectRootDeprecation();
    // §9.2.4 deprecation phase for apcore#118 — the ten keys that reach no
    // consumer. Emitted alongside the §9.2.2 notice, on the same once-per-load
    // cadence, and equally behaviour-free.
    config._warnDeprecatedInertKeys();

    return config;
  }

  /**
   * Create a Config from default values with env overrides applied.
   *
   * The result declares nothing except whatever the environment supplies, so
   * `validate()` on it fails the §9.1 required-field check unless
   * `APCORE_VERSION` / `APCORE_PROJECT_NAME` are set or the caller `set()`s
   * them. That is intentional and matches apcore-rust, where a bare
   * `Config::default()` is likewise rejected: defaults resolve values, they do
   * not declare a project.
   */
  static fromDefaults(): Config {
    const data = applyEnvOverrides({ ...DEFAULTS });
    const config = new Config(data);
    config._declared = applyEnvOverrides({});
    return config;
  }

  /**
   * Discover and load the project's config file using the canonical search
   * order, falling back to defaults when no file is found.
   *
   * Search order matches `discoverConfigFile()` and apcore-python /
   * apcore-rust:
   *   1. `$APCORE_CONFIG_FILE`
   *   2. `./project.yaml`, `./project.yml`, `./apcore.yaml`, `./apcore.yml`
   *   3. XDG config dir (`~/Library/Application Support/apcore/config.yaml`
   *      on macOS, `~/.config/apcore/config.yaml` elsewhere)
   *   4. Legacy `~/.apcore/config.yaml`
   *   5. `Config.fromDefaults()` if no file found
   *
   * Equivalent to apcore-rust's `Config::discover()` and to apcore-python's
   * `Config.load(path=None)` no-arg form (sync finding A-004).
   */
  static discover(options?: { validate?: boolean }): Config {
    const path = discoverConfigFile();
    if (path === null) return Config.fromDefaults();
    return Config.load(path, options);
  }

  // -------------------------------------------------------------------------
  // Instance methods
  // -------------------------------------------------------------------------

  /** Get a configuration value by dot-path key. */
  get(key: string, defaultValue?: unknown): unknown {
    return this._getFrom(this._data, key, defaultValue);
  }

  /**
   * Like {@link Config.get} but reads the **declared** document: it answers
   * only with values the configuration actually states (parsed file,
   * environment override, `mount()`, or runtime `set()`), never with a value
   * supplied by the `DEFAULTS` table or by a namespace's registered defaults.
   *
   * `get('extensions.root')` returns `'./extensions'` for a file that never
   * mentions it; `getDeclared('extensions.root')` returns `undefined`.
   *
   * Used by {@link Config.validate} for the §9.3 step 1 required-field check.
   * Mirrors apcore-rust's `Config::get_declared`.
   */
  getDeclared(key: string, defaultValue?: unknown): unknown {
    return this._getFrom(this._declared, key, defaultValue);
  }

  /** Shared read path for {@link Config.get} and {@link Config.getDeclared}. */
  private _getFrom(
    source: Record<string, unknown>,
    key: string,
    defaultValue?: unknown,
  ): unknown {
    if (this._mode === 'namespace') {
      const resolved = resolveNamespacePath(key);
      if (resolved === null) return defaultValue;
      const nsData = source[resolved.namespace];
      if (nsData === undefined || nsData === null) {
        // §9.9.1: Fallback to implicit "apcore" namespace if no registered namespace matches.
        if (resolved.namespace !== 'apcore') {
          return this._getFrom(source, `apcore.${key}`, defaultValue);
        }
        return defaultValue;
      }
      if (!resolved.subPath) return nsData;
      return getNested(nsData as Record<string, unknown>, resolved.subPath, defaultValue);
    }
    return getNested(source, key, defaultValue);
  }

  /**
   * Set a configuration value by dot-path key.
   *
   * Writes both the resolved tree and the declared document: a value set at
   * runtime is stated by the caller, not inherited from a default table, so it
   * satisfies the required-field check.
   */
  set(key: string, value: unknown): void {
    this._setInto(this._data, key, value);
    this._setInto(this._declared, key, value);
  }

  /** Shared write path for {@link Config.set}. */
  private _setInto(target: Record<string, unknown>, key: string, value: unknown): void {
    if (this._mode === 'namespace') {
      const resolved = resolveNamespacePath(key);
      if (resolved === null) {
        setNested(target, key, value);
        return;
      }
      if (!resolved.subPath) {
        target[resolved.namespace] = value;
        return;
      }
      if (typeof target[resolved.namespace] !== 'object' || target[resolved.namespace] === null) {
        target[resolved.namespace] = {};
      }
      setNested(target[resolved.namespace] as Record<string, unknown>, resolved.subPath, value);
      return;
    }
    setNested(target, key, value);
  }

  /** Return a deep copy of the raw config data. */
  get data(): Record<string, unknown> {
    return JSON.parse(JSON.stringify(this._data));
  }

  /** Return the detected mode: 'legacy' or 'namespace'. */
  get mode(): 'legacy' | 'namespace' {
    return this._mode;
  }

  /**
   * Absolute or relative path of the YAML file this config was loaded from,
   * or `null` when the config was built from defaults / a raw object (e.g.
   * `Config.fromDefaults()` or `new Config({...})`).
   *
   * Used by `ACL.discover()` to resolve `acl.root` relative to the config
   * file's directory rather than the process CWD when the source is known.
   * Mirrors apcore-python's `Config.source_path` and apcore-rust's
   * `Config::source_path()` (D-64 / issue #74).
   */
  get sourcePath(): string | null {
    return this._yamlPath;
  }

  /**
   * The directory a relative path-typed value (§9.2.1) is *about* — the
   * project this configuration configures.
   *
   * ```
   * projectRoot =
   *     directory of the config file   when it came from §9.14 tier 1-5
   *                                    (explicitly pointed at, or project-local)
   *     process CWD                    when it came from tier 6-7 (user-level),
   *                                    or when no config file was found
   * ```
   *
   * The tier split is the whole point. For tiers 2-5 — `./project.yaml`,
   * `./apcore.yaml` and friends — the config file's directory *is* the CWD, so
   * the two candidate bases coincide and this is the overwhelmingly common
   * case. They diverge only for a config explicitly pointed at from elsewhere
   * (`$APCORE_CONFIG_FILE`, or a path passed to {@link Config.load}), where the
   * file's directory is the better answer, and for a **user-level** config,
   * where it is the wrong one: `extensions.root: ./extensions` written in
   * `~/.config/apcore/config.yaml` means "this project's extensions", not
   * `~/.config/apcore/extensions`.
   *
   * **This accessor changes nothing.** It reports a base; it does not apply
   * one. `SchemaLoader` still resolves `schema.root` against the CWD and
   * `ACL.discover` still resolves `acl.root` against the config file's
   * directory for every tier, exactly as before. Adopting this as *the* base
   * for every path-typed key is a behaviour change to deployed configurations
   * and therefore a major-version move; this is the §13.2 deprecation phase of
   * it (apcore#113, PROTOCOL_SPEC §9.2.2).
   *
   * Always an absolute path.
   */
  get projectRoot(): string {
    const source = this._yamlPath;
    if (source === null) return process.cwd();

    const resolvedSource = resolve(source);
    // Tiers 6-7: a per-user default's relative paths are per-project by
    // intent, so they cannot mean "next to the config file".
    for (const candidate of userLevelConfigPaths()) {
      if (resolve(candidate) === resolvedSource) return process.cwd();
    }
    return dirname(resolvedSource);
  }

  /**
   * The path-typed keys (§9.2.1) this configuration resolves to a **relative**
   * value — the ones whose meaning would move if the resolution base moved.
   *
   * Reads the merged view, so a key left to its `DEFAULTS` entry counts:
   * `schema.root` is `'./schemas'` in a file that never mentions it, and that
   * default re-roots under a new base just as a written value does.
   *
   * `extensions.roots` is list-valued and reported under the `[]` element key
   * §9.2.1 gives it; one relative element is enough to list it.
   */
  private _relativePathTypedKeys(): string[] {
    const affected: string[] = [];
    for (const key of Config.pathTypedKeys()) {
      if (key.endsWith('[]')) {
        const elements = this.get(key.slice(0, -'[]'.length));
        if (!Array.isArray(elements)) continue;
        if (elements.some((element) => isRelativePathValue(extractRootPath(element)))) {
          affected.push(key);
        }
        continue;
      }
      if (isRelativePathValue(this.get(key))) affected.push(key);
    }
    return affected;
  }

  /**
   * Emit the §13.2 deprecation notice for apcore#113, **once per configuration
   * load** (PROTOCOL_SPEC §9.2.2 requirement 2).
   *
   * Deliberately narrow: it fires only when {@link Config.projectRoot} differs
   * from the CWD *and* this configuration actually carries a relative
   * path-typed value. Both conditions have to hold for the coming base change
   * to move anything, and a blanket warning on every load would train everyone
   * to ignore it. In the ordinary tier 2-5 project the first condition is
   * false and nothing is printed.
   *
   * The cadence is the load, not the process. The notice is a property of *the
   * document being loaded*, so every load that satisfies both conditions emits
   * it and a load that does not emits nothing — including a `reload()` that
   * re-reads an edited file. §9.2.2 forbids the process-global suppression this
   * method used to carry, because it made the warning order-dependent: the
   * first affected load consumed it and every later one was silent.
   */
  private _warnProjectRootDeprecation(): void {
    const root = this.projectRoot;
    const cwd = process.cwd();
    if (resolve(root) === resolve(cwd)) return;

    const affected = this._relativePathTypedKeys();
    if (affected.length === 0) return;

    console.warn(
      '[apcore:config] DEPRECATION: this configuration resolves from ' +
        `'${this._yamlPath}', whose project root ('${root}') is not the ` +
        `working directory ('${cwd}'), and it carries relative path-typed ` +
        `values (${affected.join(', ')}). Those values resolve against ` +
        'inconsistent bases today — the directory of the config file for ' +
        'acl.root, the working directory for the rest — and a future major ' +
        'will resolve every one of them against the project root ' +
        '(PROTOCOL_SPEC §9.2.2, apcore#113). Nothing changes yet. Make ' +
        'these values absolute, or run from the project root, to be ' +
        'unaffected.',
    );
  }

  /**
   * PROTOCOL_SPEC §9.2.4 — warn for declared keys that reach no consumer.
   *
   * Driven by the **declared** document, never the merged view (requirement 2).
   * Every one of these keys has a default, so a merged-view check would fire for
   * every configuration ever loaded — the blanket warning §9.2.2 rejects, which
   * trains operators to ignore the one that matters.
   *
   * Behaviour is unchanged (requirement 3): the keys still parse, still
   * validate, still answer `get()`, and are still accepted under
   * `_config.strict`. This adds the one thing they have never had — a way for an
   * operator to find out that setting them does nothing.
   */
  private _warnDeprecatedInertKeys(): void {
    const declared = DEPRECATED_INERT_KEYS.filter(
      (key) => this.getDeclared(key) !== undefined,
    );
    if (declared.length === 0) return;

    console.warn(
      `[apcore:config] DEPRECATION (apcore#118, PROTOCOL_SPEC §9.2.4): this ` +
        `configuration declares ${declared.length} key(s) that reach no consumer in ` +
        `any apcore SDK and have no effect: ${declared.join(', ')}. They keep parsing ` +
        `and validating for the whole 1.x line and are removed no earlier than v2.0 ` +
        `(§13.2 / §13.4). Nothing has changed in this release — the keys did nothing ` +
        `before this warning existed.`,
    );
  }

  /**
   * Attach external config data to a namespace.
   *
   * Exactly one of fromFile or fromDict must be provided.
   * Throws ConfigMountError if namespace is "_config" or file not found.
   */
  mount(
    namespace: string,
    options: { fromFile?: string; fromDict?: Record<string, unknown> },
  ): void {
    if (namespace === '_config') {
      throw new ConfigMountError("Cannot mount to reserved namespace '_config'");
    }

    const { fromFile, fromDict } = options;
    const hasFile = fromFile !== undefined;
    const hasDict = fromDict !== undefined;

    if (hasFile && hasDict) {
      throw new ConfigMountError("Specify exactly one of 'fromFile' or 'fromDict', not both");
    }
    if (!hasFile && !hasDict) {
      throw new ConfigMountError("One of 'fromFile' or 'fromDict' is required");
    }

    let mountData: Record<string, unknown>;

    if (hasFile) {
      if (!existsSync(fromFile!)) {
        throw new ConfigMountError(`Mount file not found: ${fromFile}`);
      }
      let parsed: unknown;
      try {
        const content = readFileSync(fromFile!, 'utf-8');
        parsed = yaml.load(content);
      } catch (e) {
        throw new ConfigMountError(`Failed to parse mount file '${fromFile}': ${e}`);
      }
      if (parsed === null || parsed === undefined) {
        parsed = {};
      }
      if (typeof parsed !== 'object' || Array.isArray(parsed)) {
        throw new ConfigMountError(`Mount file must be a YAML mapping: ${fromFile}`);
      }
      mountData = parsed as Record<string, unknown>;
    } else {
      mountData = fromDict!;
    }

    this._mounts.set(namespace, mountData);

    // Merge mount data into namespace subtree
    const existing = (this._data[namespace] ?? {}) as Record<string, unknown>;
    this._data[namespace] = deepMergeDicts(existing, mountData);

    // Mounted data is an external configuration source, not a default table,
    // so it is part of the declared document too.
    const existingDeclared = (this._declared[namespace] ?? {}) as Record<string, unknown>;
    this._declared[namespace] = deepMergeDicts(existingDeclared, mountData);
  }

  /**
   * Return a deep copy of a namespace subtree.
   */
  namespace(name: string): Record<string, unknown> {
    const subtree = this._data[name];
    if (subtree === undefined || subtree === null) {
      return {};
    }
    return JSON.parse(JSON.stringify(subtree)) as Record<string, unknown>;
  }

  /**
   * Typed get with coercion and validation.
   * Applies the coerce function to the raw value and returns the result.
   * Throws ConfigBindError (CONFIG_BIND_ERROR) if the value is missing.
   * Both `undefined` (absent key) and a stored `null` are treated as missing,
   * mirroring Python/Rust (`if value is None`).
   */
  getTyped<T>(path: string, coerce: (v: unknown) => T): T {
    const value = this.get(path);
    if (value === undefined || value === null) {
      throw new ConfigBindError(`No value at path '${path}'`);
    }
    return coerce(value);
  }

  /**
   * Deserialize a namespace subtree into a class instance.
   * The schema constructor receives the namespace data as a plain object.
   * Throws ConfigBindError if instantiation fails.
   */
  bind<T>(namespace: string, schema: new (data: Record<string, unknown>) => T): T {
    const data = this.namespace(namespace);
    try {
      return new schema(data);
    } catch (e) {
      throw new ConfigBindError(`Failed to bind namespace '${namespace}': ${e}`);
    }
  }

  /**
   * Validate the configuration per Algorithm A12.
   *
   * In legacy mode: checks required fields, type constraints, and semantic rules.
   * In namespace mode (A12-NS): validates data.apcore; throws on unknown namespaces
   * if strict mode is enabled via data._config.strict.
   * Collects all errors before raising.
   */
  validate(): void {
    if (this._mode === 'namespace') {
      this._validateNamespaceMode();
      return;
    }

    const errors: string[] = [];

    // 1. Required field check (§9.3 step 1).
    //
    // Deliberately reads the DECLARED document, not `this._data`. `_data` has
    // the `DEFAULTS` table merged into it, so every key this loop could ask
    // about would already be present and the loop would be a no-op that looks
    // like validation. Only `version` and `project.name` are checked, because
    // they are the only §9.1 keys with no canonical default.
    for (const field of REQUIRED_FIELDS) {
      const value = this.getDeclared(field);
      if (value === undefined || value === null) {
        errors.push(`Missing required field: '${field}'`);
      }
    }

    // 2. Constraint validation
    for (const [field, [checkFn, errMsg]] of Object.entries(CONSTRAINTS)) {
      const value = getNested(this._data, field);
      if (value !== undefined && value !== null && !checkFn(value)) {
        errors.push(`Invalid value for '${field}': ${errMsg} (got ${JSON.stringify(value)})`);
      }
    }

    // 3. reject_unknown_framework_keys (§9.10). Step 1 of Algorithm A12-NS runs
    //    it in legacy mode too, where the whole document *is* the `apcore`
    //    namespace — the closedness of the schema sections does not depend on
    //    which of the two file layouts the operator picked.
    errors.push(...this._undeclaredFrameworkKeyErrors(this._data));

    // 4. PROTOCOL_SPEC §9.2.3 requirement 6d / §10.6.1: an
    //    `obs.redaction.regex_patterns` entry the engine cannot compile MUST be
    //    reported here, not skipped at the first log record. A redaction rule
    //    that redacts nothing is indistinguishable, from the outside, from one
    //    that works — and on this surface the difference is credentials in
    //    plaintext. JavaScript is the engine that rejects the inline `(?i)`
    //    flag the other two accept, so this fires on portable-LOOKING patterns
    //    and not only on malformed ones.
    errors.push(...uncompilableRegexPatternErrors(this.get('obs.redaction.regex_patterns')));

    // 5. PROTOCOL_SPEC §10.1.1 requirement 3: an OTLP endpoint set against an
    //    exporter that does not read it is a rejected configuration, not a
    //    silent no-op. A value an operator wrote down and nothing reads is the
    //    shape of every defect apcore#118 found.
    errors.push(...otlpEndpointMismatchErrors(this));

    if (errors.length > 0) {
      throw new ConfigError(
        `Configuration validation failed (${errors.length} error(s)):\n` +
          errors.map((e) => `  - ${e}`).join('\n'),
      );
    }
  }

  private _validateNamespaceMode(): void {
    const errors: string[] = [];

    const apcore = this._data['apcore'];
    if (apcore !== undefined && apcore !== null) {
      // Run A12 checks on data.apcore subtree
      const apcoreData = apcore as Record<string, unknown>;
      for (const [field, [checkFn, errMsg]] of Object.entries(CONSTRAINTS)) {
        const value = getNested(apcoreData, field);
        if (value !== undefined && value !== null && !checkFn(value)) {
          errors.push(
            `Invalid value for 'apcore.${field}': ${errMsg} (got ${JSON.stringify(value)})`,
          );
        }
      }
    }

    // Per-namespace schema validation (sync finding A-D-021).
    // Each registered namespace with a non-null schema validates its data subtree
    // against that schema. Mirrors apcore-python's _validate_namespace_schema.
    // Errors accumulate before raising so all problems surface in one ConfigError.
    for (const reg of _globalNsRegistry.values()) {
      if (reg.schema === null) continue;
      const nsData = this._data[reg.name];
      if (nsData === undefined || nsData === null) continue;

      const loadedSchema = this._loadNamespaceSchema(reg.name, reg.schema);
      if (loadedSchema === null) continue; // unresolved file path → warn-and-skip

      const issues = this._validateAgainstJsonSchema(reg.name, nsData, loadedSchema);
      errors.push(...issues);
    }

    // §9.10 step 2: reject_unknown_framework_keys over the `apcore` namespace.
    if (apcore !== undefined && apcore !== null) {
      errors.push(...this._undeclaredFrameworkKeyErrors(apcore as Record<string, unknown>));
    }

    // §10.1.1 requirement 3, as in legacy mode: the disagreement is between two
    // keys, and which file layout declared them changes nothing about it.
    errors.push(...otlpEndpointMismatchErrors(this));

    // Strict mode: reject unknown namespaces (§9.10 step 3b).
    //
    // Collected rather than thrown on sight, and collected BEFORE the single
    // throw below rather than after it, per §9.10 step 4 ("If any errors
    // collected → throw with all errors"). Failing here on the first unknown
    // namespace, as this did, costs the operator one restart per typo and hides
    // every other problem in the file behind whichever one the map happened to
    // yield first.
    if (this._strictModeEnabled()) {
      const knownKeys = new Set([...Array.from(_globalNsRegistry.keys()), 'apcore', '_config']);
      for (const key of Object.keys(this._data)) {
        if (!knownKeys.has(key)) {
          errors.push(`Unknown namespace '${key}' in strict mode`);
        }
      }
    }

    if (errors.length > 0) {
      throw new ConfigError(
        `Configuration validation failed (${errors.length} error(s)):\n` +
          errors.map((e) => `  - ${e}`).join('\n'),
      );
    }
  }

  /**
   * Whether `_config.strict` is enabled (PROTOCOL_SPEC §9.6.3).
   *
   * Read from `this._data` so it works in both modes: `_config` is a top-level
   * key of the document in namespace mode and of the legacy document alike.
   */
  private _strictModeEnabled(): boolean {
    const meta = this._data['_config'];
    return (
      meta !== null &&
      typeof meta === 'object' &&
      (meta as Record<string, unknown>)['strict'] === true
    );
  }

  /**
   * `reject_unknown_framework_keys(apcoreData, meta_config)` — PROTOCOL_SPEC §9.10.
   *
   * Every framework section in `schemas/apcore-config.schema.json` is
   * `additionalProperties: false`. That closedness is enforced in two tiers:
   *
   * - **`strict` absent or `false` (the default).** Step 1 returns immediately.
   *   The unknown key stays in the tree and stays readable through
   *   {@link Config.get}; it is never pruned. "The operator wrote it and it
   *   vanished" is indistinguishable from "the operator never wrote it", so
   *   discarding it is worse than keeping it.
   * - **`strict: true`.** Every offending key is reported, in one
   *   `CONFIG_INVALID`, so one restart shows the whole problem.
   *
   * `allow_unknown` deliberately does not participate. §9.6.3 defines it for
   * unknown top-level *namespaces*; stretching one field across two
   * granularities would make its meaning depend on where it is read.
   *
   * @returns The error strings to accumulate — empty unless strict is on.
   */
  private _undeclaredFrameworkKeyErrors(apcoreData: Record<string, unknown>): string[] {
    if (!this._strictModeEnabled()) return [];
    return collectUndeclaredFrameworkKeys(apcoreData).map(
      (key) => `Unknown key '${key}' (strict mode enabled)`,
    );
  }

  /**
   * Resolve a namespace's `schema` registration into a JSON-schema dict.
   *
   * Accepts either an inline object or a filesystem path to a JSON file.
   * Path resolution failures emit a warning and return null (no validation
   * is performed for that namespace), mirroring apcore-python's
   * `_validate_namespace_schema` warn-and-skip behavior on missing files.
   */
  private _loadNamespaceSchema(
    namespace: string,
    schema: object | string,
  ): Record<string, unknown> | null {
    if (typeof schema === 'object' && schema !== null) {
      return schema as Record<string, unknown>;
    }
    if (typeof schema === 'string') {
      try {
        if (!existsSync(schema)) {
          console.warn(
            `[apcore:config] Schema file for namespace '${namespace}' not found: ${schema}`,
          );
          return null;
        }
        const raw = readFileSync(schema, 'utf-8');
        return JSON.parse(raw) as Record<string, unknown>;
      } catch (e) {
        console.warn(
          `[apcore:config] Failed to load schema file for namespace '${namespace}': ${e instanceof Error ? e.message : String(e)}`,
        );
        return null;
      }
    }
    return null;
  }

  /**
   * Validate `data` against a JSON-schema dict. Returns a list of human-readable
   * error messages prefixed with the namespace name; empty array on success.
   *
   * Uses the existing `SchemaValidator` (TypeBox-backed) by converting the JSON
   * schema via `jsonSchemaToTypeBox`. Any conversion failure is treated as an
   * accept-all (with a warning) so an unsupported schema feature does not block
   * `validate()`.
   */
  private _validateAgainstJsonSchema(
    namespace: string,
    data: unknown,
    schema: Record<string, unknown>,
  ): string[] {
    let typeBoxSchema;
    try {
      typeBoxSchema = jsonSchemaToTypeBox(schema);
    } catch (e) {
      console.warn(
        `[apcore:config] Could not convert schema for namespace '${namespace}' to TypeBox; skipping validation: ${e instanceof Error ? e.message : String(e)}`,
      );
      return [];
    }

    const validator = new SchemaValidator(/* coerceTypes */ false);
    const result = validator.validate(data as Record<string, unknown>, typeBoxSchema);
    if (result.valid) return [];
    return result.errors.map(
      (err) =>
        `Namespace '${namespace}' failed schema validation at '${err.path || '/'}': ${err.message}`,
    );
  }

  /**
   * Re-read configuration from the original YAML file.
   * Only works if the Config was created via Config.load().
   * In namespace mode, re-applies namespace defaults, env overrides, and mount data.
   *
   * Validation is re-run when the originating `Config.load()` requested it
   * (the default). A file that has since dropped `version` or `project.name`
   * therefore fails the reload rather than being adopted silently. A config
   * loaded with an explicit `{ validate: false }` keeps that opt-out.
   */
  reload(): void {
    if (this._yamlPath === null) {
      throw new ConfigError('Cannot reload: Config was not loaded from a YAML file');
    }
    const previousMounts = new Map(this._mounts);
    // Load unvalidated: validation must run after mounts and env overrides are
    // re-applied, otherwise it would judge an incomplete tree.
    const reloaded = Config.load(this._yamlPath, { validate: false });
    this._data = reloaded._data;
    this._declared = reloaded._declared;
    this._mode = reloaded._mode;
    this._mounts = new Map();

    // Re-apply mounts
    for (const [namespace, mountData] of previousMounts) {
      this.mount(namespace, { fromDict: mountData });
    }

    // Re-apply namespace env overrides in namespace mode
    if (this._mode === 'namespace') {
      this._data = applyNamespaceEnvOverrides(this._data);
      this._declared = applyNamespaceEnvOverrides(this._declared);
    }

    if (this._validateOnLoad) {
      this.validate();
    }
  }
}

// ---------------------------------------------------------------------------
// Bootstrap: register apcore built-in namespaces (§9.15)
// ---------------------------------------------------------------------------

// W-13: Use snake_case keys to match Python defaults and YAML config conventions.
// camelCase keys would silently diverge from cross-language YAML configs.
//
// `observability` declares `tracing` and `metrics` only — the blocks the
// schema's ObservabilityConfig declares (protocol-spec §9.15.2, D-144). Error
// history, event thresholds and redaction live under `sys_modules.error_history.*`,
// `sys_modules.events.thresholds.*` and `obs.redaction.*`.
Config.registerNamespace({
  name: 'observability',
  envPrefix: 'APCORE_OBSERVABILITY',
  defaults: {
    tracing: {
      enabled: false,
      strategy: 'full',
      sampling_rate: 1.0,
      exporter: 'stdout',
      otlp_endpoint: null,
    },
    metrics: { enabled: false, exporter: 'stdout' },
  },
});

/**
 * Default `obs.redaction.sensitive_keys` list (Issue #43 §5).
 *
 * Duplicated verbatim from `DEFAULT_REDACTION_FIELD_PATTERNS` in
 * `observability/context-logger.ts` (and from apcore-python's
 * `_DEFAULT_OBS_REDACTION_SENSITIVE_KEYS`) rather than imported, so that
 * `config.ts` does not pull the middleware-bearing observability module into
 * its import graph. Keep the three lists in sync.
 */
const _DEFAULT_OBS_REDACTION_SENSITIVE_KEYS: string[] = [
  '_secret_*',
  'password',
  'passwd',
  'secret',
  'token',
  'api_key',
  'apikey',
  'apiKey',
  'access_key',
  'private_key',
  'authorization',
  'auth',
  'credential',
  'cookie',
  'session',
  'bearer',
];

// The canonical `obs.*` namespace (redaction_config.json fixture). Registered
// so `APCORE_OBS_*` env dispatch works and `config.namespace('obs')` resolves,
// matching apcore-python config.py. `APCORE_OBS` does not collide with
// `APCORE_OBSERVABILITY` — dispatch is longest-prefix-match.
Config.registerNamespace({
  name: 'obs',
  envPrefix: 'APCORE_OBS',
  defaults: {
    redaction: {
      regex_patterns: [] as string[],
      sensitive_keys: [..._DEFAULT_OBS_REDACTION_SENSITIVE_KEYS],
      replacement: '***REDACTED***',
    },
  },
});

Config.registerNamespace({
  name: 'sys_modules',
  envPrefix: 'APCORE_SYS',
  defaults: {
    // Activation is off by default: PROTOCOL_SPEC §6.6.3 states
    // `sys_modules.enabled = false (default)` -> 0 modules registered, and
    // schemas/sys-modules.schema.json declares `default: false`. Registering
    // `true` here made namespace-mode projects stand up the six read modules
    // without asking — an information-disclosure surface §6.6.3 calls out by
    // name. The per-module sub-flags stay true: they select WHICH modules
    // register once activation has happened.
    enabled: false,
    health: { enabled: true },
    manifest: { enabled: true },
    usage: { enabled: true, retention_hours: 168, bucketing_strategy: 'hourly' },
    control: { enabled: true },
    // `error_history` and `events.subscribers` are declared with defaults by
    // schemas/sys-modules.schema.json and were missing here, so this namespace
    // answered for eleven of its own schema's fourteen keys.
    // `control.overrides_path` is the one deliberate omission: its declared
    // default is null, which a namespace default cannot express distinctly
    // from absence (sync finding A-D-021).
    error_history: { max_entries_per_module: 50, max_total_entries: 1000 },
    events: {
      enabled: false,
      subscribers: [],
      thresholds: { error_rate: 0.1, latency_p99_ms: 5000.0 },
    },
  },
});
