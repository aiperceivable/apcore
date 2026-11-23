/**
 * Cross-language conformance driver for `config_path_typed_keys.json`
 * (PROTOCOL_SPEC §9.2.1, apcore#113).
 *
 * Fixture source: apcore/conformance/fixtures/config_path_typed_keys.json
 * (canonical, read in place — never vendored).
 *
 * The fixture asserts WHICH configuration keys carry filesystem paths and that
 * the SDK publishes the set. It asserts nothing about what a *relative* value
 * in one of them resolves against — that base is §9.2.2's subject and is driven
 * by `conformance-config-project-root.test.ts`.
 *
 * WHAT THE `driver_contract` DEMANDS, AND WHERE EACH DEMAND LANDS
 * ---------------------------------------------------------------
 * - `accessor`: the set is read through `Config.pathTypedKeys()`, the public
 *   static. Reading `PATH_TYPED_CONFIG_KEYS` out of `src/config-key-surface.ts`
 *   would satisfy nothing: the consumer this set exists for (apcore-cli's
 *   `SANDBOX_PATH_TYPED_VARS`) lives in another repository and sees only the
 *   published API.
 * - `comparison` / `both_directions_required`: the accessor is compared as a
 *   SET and reported as a symmetric difference. A one-directional "every
 *   expected key is present" check passes an SDK that marks every config key as
 *   path-typed, which is why the two discriminating cases below exist.
 * - `roots_element_form`: `extensions.roots` is list-valued and both element
 *   forms carry a path, so the schema projection collapses every marker at or
 *   below an array boundary onto the single `extensions.roots[]` entry.
 *
 * THE SCHEMA PROJECTION IS COMPUTED, NOT COPIED
 * ----------------------------------------------
 * `declared_set_matches_schemas` re-derives the set from the canonical schemas
 * by walking `x-apcore-path` markers (following `$ref` into `$defs` and through
 * `oneOf`/`anyOf`/`allOf`), so a marker added upstream reaches this SDK as a
 * failure here rather than as silence. `src/config-key-surface.ts` cannot do
 * this at run time — the schemas live in the spec repo and the npm package
 * ships `dist` only — which is exactly why it carries a committed projection
 * and why this case has to guard it.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';

import { Config } from '../src/config.js';
import { findFixturesRoot, findSchemasRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

interface PathTypedCase {
  readonly id: string;
  readonly comment?: string;
  readonly key?: string;
  readonly keys?: readonly string[];
  readonly config?: Record<string, unknown>;
  readonly env?: Record<string, string>;
  readonly expected: Record<string, unknown>;
}

interface PathTypedFixture {
  readonly description: string;
  readonly canonical_sources: readonly string[];
  readonly driver_contract: Record<string, string>;
  readonly path_typed_keys: readonly string[];
  readonly test_cases: readonly PathTypedCase[];
}

const fixture: PathTypedFixture = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'config_path_typed_keys.json'), 'utf-8'),
);

/** The case with this id, or throw — a renamed case must not silently vanish. */
function caseFor(id: string): PathTypedCase {
  const found = fixture.test_cases.find((c) => c.id === id);
  if (!found) throw new Error(`Fixture case '${id}' not found in config_path_typed_keys.json`);
  return found;
}

// ---------------------------------------------------------------------------
// The canonical schema projection
// ---------------------------------------------------------------------------

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Every dotted key marked `"x-apcore-path": true` in one canonical schema.
 *
 * `$ref` is followed into the document's own `$defs` (most markers live in
 * `$defs` definitions such as `ExtensionsConfig`, reached by `$ref` from the root
 * `properties`) and — `cross_file_ref` — into a sibling schema file:
 * `sys_modules` is a `$ref` to `sys-modules.schema.json`, where the
 * `control.overrides_path` marker lives (D-138).
 *
 * Composition keywords are traversed without descending a level, because a
 * `oneOf` branch describes the SAME node — `ExtensionsConfig` states its single-
 * root and multi-root modes that way, and only the branches carry `properties`.
 */
function projectPathTypedKeys(schemaPath: string): string[] {
  const schemaDir = path.dirname(schemaPath);
  const documents = new Map<string, JsonObject>();
  function load(file: string): JsonObject {
    let doc = documents.get(file);
    if (doc === undefined) {
      doc = JSON.parse(fs.readFileSync(path.join(schemaDir, file), 'utf-8')) as JsonObject;
      documents.set(file, doc);
    }
    return doc;
  }
  const rootFile = path.basename(schemaPath);
  const root = load(rootFile);
  const found = new Set<string>();

  /** Resolve a `$ref` chain within a document or across sibling files. */
  function deref(node: JsonObject, file: string): [JsonObject, string] | null {
    let current: JsonObject = node;
    let currentFile = file;
    for (let hop = 0; hop < 10; hop++) {
      const ref = current['$ref'];
      if (typeof ref !== 'string') return [current, currentFile];
      const [refFile, fragment = ''] = ref.split('#');
      if (refFile !== '') currentFile = refFile;
      let target: unknown = load(currentFile);
      if (fragment !== '') {
        if (!fragment.startsWith('/$defs/')) return null;
        const defs = (target as JsonObject)['$defs'];
        target = isObject(defs) ? defs[fragment.slice('/$defs/'.length)] : undefined;
      }
      if (!isObject(target)) return null;
      current = target;
    }
    return null;
  }

  function visit(node: unknown, keyPath: string, depth: number, file = rootFile): void {
    if (!isObject(node) || depth > 25) return;
    const hit = deref(node, file);
    if (hit === null) return;
    const [resolved, resolvedFile] = hit;

    if (resolved['x-apcore-path'] === true && keyPath !== '') {
      // `roots_element_form`: a marker at or below an array boundary is reported
      // under the element key, so `extensions.roots[].root` collapses onto
      // `extensions.roots[]` rather than becoming a sixth key.
      const boundary = keyPath.indexOf('[]');
      found.add(boundary === -1 ? keyPath : keyPath.slice(0, boundary + 2));
    }

    for (const keyword of ['oneOf', 'anyOf', 'allOf']) {
      const branches = resolved[keyword];
      if (Array.isArray(branches)) {
        for (const branch of branches) visit(branch, keyPath, depth + 1, resolvedFile);
      }
    }

    const properties = resolved['properties'];
    if (isObject(properties)) {
      for (const [name, child] of Object.entries(properties)) {
        visit(child, keyPath === '' ? name : `${keyPath}.${name}`, depth + 1, resolvedFile);
      }
    }

    const items = resolved['items'];
    if (isObject(items)) visit(items, `${keyPath}[]`, depth + 1, resolvedFile);
  }

  visit(root, '', 0);
  return [...found].sort();
}

/** Symmetric difference, reported as the fixture's two named halves. */
function symmetricDifference(
  declared: readonly string[],
  actual: readonly string[],
): { missing_from_sdk: string[]; extra_in_sdk: string[] } {
  const declaredSet = new Set(declared);
  const actualSet = new Set(actual);
  return {
    missing_from_sdk: declared.filter((k) => !actualSet.has(k)).sort(),
    extra_in_sdk: actual.filter((k) => !declaredSet.has(k)).sort(),
  };
}

// ---------------------------------------------------------------------------

/**
 * The two keys §9.1 leaves without a canonical default, and therefore the only
 * two `Config.validate` requires. Cases here are about the path-typed key set,
 * not about required-field validation.
 */
const REQUIRED_BASE = { version: '1.0.0', project: { name: 'fixture' } } as const;

describe('Conformance: the closed set of path-typed configuration keys (§9.2.1)', () => {
  beforeEach(() => {
    // `env_isolation` in spirit: an ambient APCORE_EXTENSIONS_ROOTS would make
    // the no-scalar-encoding case assert someone's shell. Deleted rather than
    // blanked — an empty string is itself a §9.2 override.
    vi.stubEnv('APCORE_EXTENSIONS_ROOTS', undefined);
    vi.stubEnv('APCORE_CONFIG_FILE', undefined);
  });

  afterEach(() => {
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  it('declared_set_matches_schemas: the fixture is the schemas projection', () => {
    // Guards the FIXTURE against the schemas it claims to project. A key that
    // gains `x-apcore-path` upstream shows up here as a diff.
    const schemasRoot = findSchemasRoot();
    const fromConfigSchema = projectPathTypedKeys(
      path.join(schemasRoot, 'apcore-config.schema.json'),
    );
    const expected = caseFor('declared_set_matches_schemas').expected['path_typed_keys'];

    expect(
      fromConfigSchema,
      'apcore-config.schema.json no longer projects the fixture\'s declared set. ' +
        'Either a key gained/lost "x-apcore-path" upstream, or this driver\'s ' +
        'projection needs to follow a schema construct it does not yet handle.',
    ).toEqual(expected);
    expect(fromConfigSchema).toEqual([...fixture.path_typed_keys]);

    // defaults.schema.json MIRRORS the three keys that carry a default there;
    // it may not invent a path-typed key of its own (§9.2.1: the set is closed
    // and apcore-config.schema.json declares it).
    const fromDefaultsSchema = projectPathTypedKeys(
      path.join(schemasRoot, 'defaults.schema.json'),
    );
    expect(
      fromDefaultsSchema.filter((k) => !fromConfigSchema.includes(k)),
      'defaults.schema.json marks a path-typed key that apcore-config.schema.json does not.',
    ).toEqual([]);

    expect([...fixture.canonical_sources]).toEqual([
      'schemas/apcore-config.schema.json',
      'schemas/sys-modules.schema.json',
      'schemas/defaults.schema.json',
    ]);
  });

  it('sdk_accessor_matches_declared_set: symmetric difference is empty in BOTH directions', () => {
    // `both_directions_required`: reported as two lists rather than a boolean,
    // so an SDK that drops one key and invents another is named for both.
    const diff = symmetricDifference([...fixture.path_typed_keys], Config.pathTypedKeys());
    const expected = caseFor('sdk_accessor_matches_declared_set').expected;

    expect(
      diff,
      `Config.pathTypedKeys() has drifted from §9.2.1.\n` +
        `  declared: ${JSON.stringify([...fixture.path_typed_keys])}\n` +
        `  SDK     : ${JSON.stringify(Config.pathTypedKeys())}`,
    ).toEqual({
      missing_from_sdk: expected['missing_from_sdk'],
      extra_in_sdk: expected['extra_in_sdk'],
    });
  });

  it('sdk accessor is public API, not a private constant', () => {
    // §9.2.1 requirement 1. The consumer is apcore-cli, in another repo: it can
    // only see what the package exports.
    expect(typeof Config.pathTypedKeys).toBe('function');
    expect(Array.isArray(Config.pathTypedKeys())).toBe(true);
  });

  it('bindings_pattern_is_not_path_typed: the sibling key with a filename-shaped default', () => {
    // DISCRIMINATING CASE. `bindings.pattern` shares a section with
    // `bindings.dir` and defaults to `*.binding.yaml`, so an implementation
    // that classifies by section or by "looks like a filename" sweeps it in.
    // §9.2.1 requirement 4: it is a glob matched WITHIN `bindings.dir`.
    const testCase = caseFor('bindings_pattern_is_not_path_typed');
    const key = testCase.key as string;
    const isPathTyped = Config.pathTypedKeys().includes(key);

    expect({ path_typed: isPathTyped }).toEqual(testCase.expected);
    // Pin the sibling that IS path-typed, so this case cannot pass by the
    // accessor simply being empty.
    expect(Config.pathTypedKeys()).toContain('bindings.dir');
  });

  it('non_path_string_keys_are_not_path_typed: string-valued keys that are not paths', () => {
    // Second discriminating case: an implementation that marks every STRING key
    // as path-typed fails here and passes every presence-only assertion.
    const testCase = caseFor('non_path_string_keys_are_not_path_typed');
    const declared = Config.pathTypedKeys();
    const wrongly = (testCase.keys ?? []).filter((k) => declared.includes(k));

    expect(
      wrongly.length === 0 ? { path_typed: false } : { path_typed: true, keys: wrongly },
      `These keys are not filesystem paths but the SDK reports them as path-typed: ${wrongly.join(', ')}`,
    ).toEqual(testCase.expected);
  });

  it('id_map_overrides_is_path_typed: the ID map file and the runtime overrides file (D-138)', () => {
    const testCase = caseFor('id_map_overrides_is_path_typed');
    const declared = Config.pathTypedKeys();
    const missing = (testCase.keys ?? []).filter((k) => !declared.includes(k));
    expect(
      missing.length === 0 ? { path_typed: true } : { path_typed: false, keys: missing },
      `These keys carry filesystem paths but the SDK does not report them: ${missing.join(', ')}`,
    ).toEqual(testCase.expected);
  });

  it('extensions_roots_elements_are_path_typed: both element forms, one reported key', () => {
    const testCase = caseFor('extensions_roots_elements_are_path_typed');
    const declared = Config.pathTypedKeys();

    // The observed side is read from the SDK, not from `expected`. It used to
    // take `reported_key` OUT of the expectation and feed it back in as the
    // lookup, so mutating the expectation moved both sides together and the
    // assertion cancelled itself — `check_case_pinning.py` rewrote `expected`
    // and nothing went red. `roots_element_form` already requires exactly one
    // `extensions.roots*` entry, so that entry IS the observation.
    const observed = declared.filter((k) => k.startsWith('extensions.roots'));
    expect({ path_typed: observed.length === 1, reported_key: observed[0] }).toEqual(
      testCase.expected,
    );
    const reportedKey = testCase.expected['reported_key'] as string;

    // `roots_element_form`: the list is ONE entry. Neither the bare list nor the
    // object form's inner `root` may appear as a separate key.
    expect(declared).not.toContain('extensions.roots');
    expect(declared).not.toContain('extensions.roots[].root');

    // "an SDK that models only one form is reported as a violation of the
    // `extensions.roots[]` entry": both forms must survive a real load.
    const config = new Config({ ...REQUIRED_BASE, ...(testCase.config ?? {}) });
    expect(config.get('extensions.roots')).toEqual(
      (testCase.config as { extensions: { roots: unknown[] } }).extensions.roots,
    );
  });

  it('no_scalar_env_encoding_for_roots: APCORE_EXTENSIONS_ROOTS makes no list', () => {
    // §9.2.1 requirement 3. `extensions.roots` is list-valued and §9.2's scalar
    // convention does not reach it; an implementation MUST NOT invent a
    // delimiter-separated encoding. The variable is not rejected — it is simply
    // never read as a list.
    const testCase = caseFor('no_scalar_env_encoding_for_roots');
    for (const [name, value] of Object.entries(testCase.env ?? {})) {
      vi.stubEnv(name, value);
    }

    const config = Config.fromDefaults();
    const raw = config.get('extensions.roots');
    const rootsFromEnv = Array.isArray(raw) ? raw : null;

    expect(
      { roots_from_env: rootsFromEnv },
      'APCORE_EXTENSIONS_ROOTS was decoded into a roots LIST. §9.2.1 requirement 3 ' +
        'forbids a delimiter-separated scalar encoding for this list-valued key.',
    ).toEqual(testCase.expected);
  });

  it('accessor_is_stable_across_config_instances', () => {
    // The set is a property of the SPECIFICATION, not of a loaded document.
    const testCase = caseFor('accessor_is_stable_across_config_instances');

    const fromDefaults = Config.fromDefaults();
    const fromDocument = new Config({
      ...REQUIRED_BASE,
      logging: { level: 'DEBUG' },
    });
    // Neither config declares a path-typed key, so any difference between the
    // two answers would be the accessor reading the document.
    expect(fromDocument.getDeclared('schema.root')).toBeUndefined();

    const setA = [...Config.pathTypedKeys()].sort();
    void fromDefaults;
    const setB = [...Config.pathTypedKeys()].sort();

    expect({ same_set: JSON.stringify(setA) === JSON.stringify(setB) }).toEqual(testCase.expected);
    expect(setA).toEqual([...fixture.path_typed_keys]);
  });

  it('every fixture case is driven', () => {
    // Guard the driver: a case added upstream must not sit unexercised.
    const driven = new Set([
      'declared_set_matches_schemas',
      'sdk_accessor_matches_declared_set',
      'bindings_pattern_is_not_path_typed',
      'non_path_string_keys_are_not_path_typed',
      'extensions_roots_elements_are_path_typed',
      'no_scalar_env_encoding_for_roots',
      'accessor_is_stable_across_config_instances',
      'id_map_overrides_is_path_typed',
    ]);
    const undriven = fixture.test_cases.map((c) => c.id).filter((id) => !driven.has(id));
    expect(undriven, `config_path_typed_keys.json gained cases this driver ignores`).toEqual([]);
  });
});
