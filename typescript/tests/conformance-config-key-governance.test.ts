/**
 * Drive `config_key_governance.json` — the configuration key-surface guard.
 *
 * The fixture derives its `allowed_keys` / `canonical_defaults` from the
 * canonical schemas, so this suite is really asking: does `src/config.ts`'s idea
 * of the config surface still match `apcore/schemas/`?
 *
 * It exists because four separate instances of the same defect shipped
 * undetected: `schema.validation.*` validated by every SDK and declared by no
 * schema, a frozen `version`/`project` default pair that made the required-field
 * check unreachable, `middleware.circuit_breaker.*` forbidden by
 * `apcore-config.schema.json` yet validated everywhere and read nowhere, and a
 * missing Rust default table that resolved 15 documented keys to null. None was
 * findable by any existing test.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import yaml from 'js-yaml';

import { Config, CONSTRAINTS, _globalNsRegistry } from '../src/config.js';
import { DEFAULTS } from '../src/config-defaults.js';
import { FRAMEWORK_CONFIG_KEYS } from '../src/config-key-surface.js';
import { ConfigError } from '../src/errors.js';
import { findFixturesRoot, findSchemasRoot } from './spec-repo.js';

interface GovernanceFixture {
  allowed_keys: string[];
  canonical_defaults: Record<string, unknown>;
  canonical_sources: string[];
  driver_contract: {
    sources: string;
    strict_enumerates_every_key: string;
    default_tier_must_be_asserted_by_reading_it_back: string;
  };
  test_cases: Array<{
    id: string;
    expected: Record<string, unknown>;
    config?: Record<string, unknown>;
  }>;
}

const fixture: GovernanceFixture = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'config_key_governance.json'), 'utf-8'),
);
const allowed = new Set(fixture.allowed_keys);
const canonical = fixture.canonical_defaults;

/** Flatten a nested default table to { dotPath: value }. An empty object is a
 * leaf — a declared value, not a subtree to descend. */
function flatten(tree: Record<string, unknown>, prefix = ''): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, val] of Object.entries(tree)) {
    const dotPath = prefix ? `${prefix}.${key}` : key;
    if (val !== null && typeof val === 'object' && !Array.isArray(val)
        && Object.keys(val as object).length > 0) {
      Object.assign(out, flatten(val as Record<string, unknown>, dotPath));
    } else {
      out[dotPath] = val;
    }
  }
  return out;
}

const defaultKeys = flatten(DEFAULTS);
const constraintKeys = Object.keys(CONSTRAINTS);

/** The case's own `expected`, so the empty lists come from the fixture rather
 * than from a literal typed twice into this file. */
function expectedFor(caseId: string): Record<string, unknown> {
  return caseFor(caseId).expected;
}

function caseFor(caseId: string): GovernanceFixture['test_cases'][number] {
  const found = (fixture.test_cases ?? []).find((c) => c.id === caseId);
  if (!found) throw new Error(`Fixture case '${caseId}' not found`);
  return found;
}

/** Expand a case's flat `{ "executor.max_call_depth": 7 }` into the nested
 * mapping a YAML config file actually holds. */
function nestDotPaths(flat: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [dotPath, value] of Object.entries(flat)) {
    const parts = dotPath.split('.');
    let node = out;
    for (const part of parts.slice(0, -1)) {
      if (!(part in node) || typeof node[part] !== 'object' || node[part] === null) {
        node[part] = {};
      }
      node = node[part] as Record<string, unknown>;
    }
    node[parts[parts.length - 1] as string] = value;
  }
  return out;
}

/**
 * The two required keys, added to every case's config.
 *
 * PROTOCOL_SPEC §9.1 makes `version` and `project.name` the only keys with no
 * canonical default, and therefore the only two `Config.validate` requires. The
 * fixture's `config` maps list the keys each case is *about*; without these two
 * every case would fail on a missing-required-field error that has nothing to do
 * with what it is testing — and the strict case would then "pass" a raise-only
 * assertion for entirely the wrong reason.
 */
const REQUIRED_BASE = { version: '1.0.0', project: { name: 'fixture' } } as const;

describe('Conformance: configuration key-surface governance', () => {
  it('DEFAULTS declares no key the canonical schemas do not allow', () => {
    // apcore-config.schema.json is additionalProperties:false, so a user config
    // carrying such a key fails the canonical schema while the SDK quietly
    // supplies a value for it.
    const violations = Object.keys(defaultKeys).filter((k) => !allowed.has(k)).sort();
    expect(
      violations,
      `DEFAULTS declares keys no canonical schema allows:\n  ${violations
        .map((k) => `${k} = ${JSON.stringify(defaultKeys[k])}`)
        .join('\n  ')}\nEither add them to a schema in apcore/schemas/ (and regenerate ` +
        'the fixture) or remove them from DEFAULTS.',
    ).toEqual(expectedFor('sdk_default_table_declares_no_undeclared_key')['violations']);
  });

  it('CONSTRAINTS validates no key the canonical schemas do not allow', () => {
    // Validating a key the canonical schema forbids is worse than not
    // validating it: it tells the operator the key is understood.
    const violations = constraintKeys.filter((k) => !allowed.has(k)).sort();
    expect(
      violations,
      `CONSTRAINTS validates keys no canonical schema allows:\n  ${violations.join('\n  ')}`,
    ).toEqual(expectedFor('sdk_constraint_table_declares_no_undeclared_key')['violations']);
  });

  it('reproduces every canonical default', () => {
    // A missing entry means the key resolves to undefined here while its peers
    // return the documented value.
    const missing = Object.keys(canonical).filter((k) => !(k in defaultKeys)).sort();
    expect(
      missing,
      `defaults.schema.json declares defaults DEFAULTS does not carry:\n  ${missing
        .map((k) => `${k} = ${JSON.stringify(canonical[k])}`)
        .join('\n  ')}`,
    ).toEqual(expectedFor('sdk_reproduces_every_canonical_default')['missing']);
  });

  it('default values match the canonical values', () => {
    // Reported as one list rather than one assertion per key: `mismatched` is
    // the fixture's unit, and a per-key loop that stops at the first failure
    // hides how wide the drift is.
    const mismatched = Object.keys(canonical)
      .sort()
      .filter((k) => k in defaultKeys && JSON.stringify(defaultKeys[k]) !== JSON.stringify(canonical[k]))
      .map((k) => `${k}: SDK ${JSON.stringify(defaultKeys[k])} != canonical ${JSON.stringify(canonical[k])}`);
    expect(
      mismatched,
      `DEFAULTS disagrees with defaults.schema.json:\n  ${mismatched.join('\n  ')}`,
    ).toEqual(expectedFor('sdk_default_values_match_canonical_defaults')['mismatched']);
  });

  it('FRAMEWORK_CONFIG_KEYS is the canonical key surface, verbatim', () => {
    // `src/config-key-surface.ts` cannot parse the schemas at run time — they
    // live in the spec repo and the npm package ships `dist` only — so it
    // carries a committed projection of them. THIS is what stops that
    // projection from being a hardcoded list that drifts: `allowed_keys` is
    // regenerated from schemas/ by the spec repo's own generator, so a section
    // added to apcore-config.schema.json turns up here as a diff rather than as
    // a key strict mode silently rejects.
    expect(
      [...FRAMEWORK_CONFIG_KEYS],
      'src/config-key-surface.ts has drifted from schemas/. Regenerate the ' +
        'fixture (python3 conformance/generate_config_key_governance.py --write) ' +
        'and copy allowed_keys into FRAMEWORK_CONFIG_KEYS.',
    ).toEqual(fixture.allowed_keys);
  });

  it('the fixture is derived, not authored', () => {
    // Guard the guard: if the fixture ever stops naming its generator, the next
    // person to hand-edit it will make it a second source of truth.
    expect(fixture.driver_contract.sources).toContain('regenerated');
    expect(fixture.driver_contract.sources).toContain('do NOT hand-edit');
    expect(fixture.canonical_sources).toEqual([
      'schemas/apcore-config.schema.json',
      'schemas/defaults.schema.json',
      'schemas/sys-modules.schema.json',
    ]);
  });
});

/**
 * The two behavioural halves of the same rule (PROTOCOL_SPEC §9.10,
 * `reject_unknown_framework_keys`): a key no schema declares is KEPT by
 * default and REJECTED under `_config.strict`.
 *
 * Both cases run in legacy mode, which is where the clause is easiest to get
 * wrong — step 1 of Algorithm A12-NS runs the sub-algorithm there too, on a
 * document that has no `apcore:` key because the whole file *is* the `apcore`
 * namespace. `describes both modes` below repeats the pair in namespace mode.
 */
describe('Conformance: unknown keys inside a framework section', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-framework-keys-'));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  /** Write a case's config as a legacy-mode YAML file and return its path. */
  function writeLegacy(config: Record<string, unknown>): string {
    const file = path.join(tmpDir, 'apcore.yaml');
    fs.writeFileSync(file, yaml.dump({ ...REQUIRED_BASE, ...nestDotPaths(config) }));
    return file;
  }

  /** The same config nested one level down under `apcore:`, which is what
   * flips mode detection to namespace mode. */
  function writeNamespace(config: Record<string, unknown>): string {
    const file = path.join(tmpDir, 'apcore.yaml');
    const nested = nestDotPaths(config);
    // `_config` is a top-level sibling of `apcore`, never a key inside it.
    const meta = nested['_config'];
    delete nested['_config'];
    const doc: Record<string, unknown> = { apcore: { ...REQUIRED_BASE, ...nested } };
    if (meta !== undefined) doc['_config'] = meta;
    fs.writeFileSync(file, yaml.dump(doc));
    return file;
  }

  /** Split a case's config into the key it declares legally and the key(s) it
   * does not, read off `allowed_keys` rather than retyped — the case is about
   * the distinction, so the driver should not be free to disagree about which
   * key is which. */
  function partition(config: Record<string, unknown>): {
    declared: string[];
    undeclared: string[];
  } {
    const keys = Object.keys(config).filter((k) => !k.startsWith('_config.'));
    return {
      declared: keys.filter((k) => allowed.has(k)),
      undeclared: keys.filter((k) => !allowed.has(k)),
    };
  }

  it('unknown_framework_key_is_retained_by_default', () => {
    const tc = caseFor('unknown_framework_key_is_retained_by_default');
    const config = tc.config as Record<string, unknown>;
    const { declared, undeclared } = partition(config);
    expect(declared).toHaveLength(1);
    expect(undeclared).toHaveLength(1);
    const [declaredKey] = declared as [string];
    const [undeclaredKey] = undeclared as [string];

    const cfg = Config.load(writeLegacy(config), { validate: true });

    // The assertion the fixture's `default_tier_must_be_asserted_by_reading_it_back`
    // demands. `expect(() => load()).not.toThrow()` is ALSO satisfied by an
    // implementation that parsed the key into a typed record and dropped it on
    // the floor, which is the exact defect this case exists to catch — so the
    // key is read back through get(), the public path an operator would use.
    expect(cfg.get(undeclaredKey), `'${undeclaredKey}' was written to the config and vanished`).toBe(
      tc.expected['get_undeclared_key'],
    );
    // Its declared neighbour still resolves, i.e. retaining the stray key did
    // not cost the section its normal parse.
    expect(cfg.get(declaredKey)).toBe(tc.expected['get_declared_key']);
  });

  it('unknown_framework_key_is_rejected_under_strict', () => {
    const tc = caseFor('unknown_framework_key_is_rejected_under_strict');
    const config = tc.config as Record<string, unknown>;
    const offending = tc.expected['error_names_all_offending_keys'] as string[];
    // The case declares its two undeclared keys in DIFFERENT sections on
    // purpose; if that ever collapses to one section, or to one key, the
    // enumeration assertion below stops meaning anything.
    expect(partition(config).undeclared.sort()).toEqual([...offending].sort());
    expect(new Set(offending.map((k) => k.split('.')[0])).size).toBe(2);

    let raised: unknown;
    try {
      Config.load(writeLegacy(config), { validate: true });
    } catch (e) {
      raised = e;
    }

    expect(raised, 'strict mode accepted a key no canonical schema declares').toBeInstanceOf(
      ConfigError,
    );
    expect((raised as ConfigError).code).toBe(tc.expected['error_code']);

    // `strict_enumerates_every_key`: reported as the set of keys MISSING from
    // the message rather than one `toContain` per key, because a per-key loop
    // stops at the first and an implementation that fails on the first
    // offending key would then look like it merely mis-worded one message —
    // when what it actually costs the operator is one restart per typo.
    const message = (raised as ConfigError).message;
    const unreported = offending.filter((key) => !message.includes(key));
    expect(
      unreported,
      `the error named only some of the offending keys. Missing: ${unreported.join(
        ', ',
      )}\nActual message:\n${message}`,
    ).toEqual([]);
  });

  it('applies in namespace mode as well as legacy mode', () => {
    // §9.10 runs the sub-algorithm from step 1 (legacy) AND step 2 (the
    // `apcore` namespace). The two cases above pin legacy; this pins that
    // moving the same keys under an `apcore:` key changes nothing.
    const retained = caseFor('unknown_framework_key_is_retained_by_default');
    const retainedConfig = retained.config as Record<string, unknown>;
    const cfg = Config.load(writeNamespace(retainedConfig), { validate: true });
    for (const key of partition(retainedConfig).undeclared) {
      expect(cfg.get(key), `'${key}' vanished in namespace mode`).toBe(
        retained.expected['get_undeclared_key'],
      );
    }

    const rejected = caseFor('unknown_framework_key_is_rejected_under_strict');
    const rejectedConfig = rejected.config as Record<string, unknown>;
    const offending = rejected.expected['error_names_all_offending_keys'] as string[];
    let raised: unknown;
    try {
      Config.load(writeNamespace(rejectedConfig), { validate: true });
    } catch (e) {
      raised = e;
    }
    expect(raised).toBeInstanceOf(ConfigError);
    const message = (raised as ConfigError).message;
    expect(offending.filter((key) => !message.includes(key))).toEqual([]);
  });

  it('leaves a config with no undeclared key alone under strict', () => {
    // The counterweight: strict mode must not start rejecting the payload-shaped
    // values the schemas declare as leaves. `pipeline.steps` entries and
    // `id_map.overrides` are user-shaped maps — walking INTO them would report
    // every one of their keys as undeclared.
    const file = path.join(tmpDir, 'apcore.yaml');
    fs.writeFileSync(
      file,
      yaml.dump({
        ...REQUIRED_BASE,
        _config: { strict: true },
        id_map: { overrides: { 'legacy.name': 'executor.email.send' } },
        pipeline: { steps: [{ name: 'custom', type: 'middleware', handler: 'x.y' }] },
        obs: { redaction: { sensitive_keys: ['token'] } },
        sys_modules: { enabled: true, events: { enabled: true } },
      }),
    );
    expect(() => Config.load(file, { validate: true })).not.toThrow();
  });
});

describe('default tiers mirror the canonical schemas (A-D-021)', () => {
  const schemasRoot = findSchemasRoot();

  /** Every `default:` a schema declares, as full dot-paths. */
  function schemaDefaults(node: unknown, prefix = ''): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    const props = (node as { properties?: Record<string, Record<string, unknown>> } | null)
      ?.properties;
    for (const [key, value] of Object.entries(props ?? {})) {
      const dotted = prefix ? `${prefix}.${key}` : key;
      if ('default' in value) out[dotted] = value.default;
      if (value.properties) Object.assign(out, schemaDefaults(value, dotted));
    }
    return out;
  }

  function leaves(node: unknown, prefix = ''): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const [key, value] of Object.entries((node ?? {}) as Record<string, unknown>)) {
      const dotted = prefix ? `${prefix}.${key}` : key;
      if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
        Object.assign(out, leaves(value, dotted));
      } else {
        out[dotted] = value;
      }
    }
    return out;
  }

  it('DEFAULTS is defaults.schema.json, key for key', () => {
    // Narrower than "every default key is allowed by some schema": that passes
    // on a table with extra keys, because `sys-modules.schema.json` allows them
    // too. `defaults.schema.json` IS the legacy default table, and it is
    // `additionalProperties: false`. apcore-python carried six extra
    // `sys_modules` leaves under exactly that gap.
    const canonical = schemaDefaults(
      JSON.parse(fs.readFileSync(path.join(schemasRoot, 'defaults.schema.json'), 'utf-8')),
    );
    expect(Object.keys(leaves(DEFAULTS)).sort()).toEqual(Object.keys(canonical).sort());
    for (const [key, expected] of Object.entries(canonical)) {
      expect(leaves(DEFAULTS)[key], `default for ${key}`).toEqual(expected);
    }
  });

  it('the sys_modules namespace supplies every default its own schema declares', () => {
    // §9.15.3 gives `sys-modules.schema.json` ownership of this namespace, and
    // it declares fourteen defaults. This registration supplied eleven —
    // `error_history.*` and `events.subscribers` were missing, so those keys
    // resolved to undefined in namespace mode while the schema documents a
    // value. `control.overrides_path` is excluded: its declared default is
    // null, which a namespace default cannot express distinctly from absence.
    const declared = schemaDefaults(
      JSON.parse(fs.readFileSync(path.join(schemasRoot, 'sys-modules.schema.json'), 'utf-8')),
    );
    const expected = Object.fromEntries(
      Object.entries(declared).filter(([, value]) => value !== null),
    );
    expect(Object.keys(expected).length).toBeGreaterThanOrEqual(13);

    const supplied = leaves(_globalNsRegistry.get('sys_modules')?.defaults ?? {});
    const missing = Object.keys(expected).filter((key) => !(key in supplied));
    expect(missing, 'defaults declared by the schema but not supplied').toEqual([]);
    for (const [key, value] of Object.entries(expected)) {
      expect(supplied[key], `namespace default for ${key}`).toEqual(value);
    }
  });
});

describe('config_key_governance.json — built-in namespaces (D-144)', () => {
  let tmp: string;
  beforeEach(() => {
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-ns-governance-'));
  });
  afterEach(() => {
    fs.rmSync(tmp, { recursive: true, force: true });
  });

  /** A namespace-mode document that declares nothing under any namespace. */
  function probeConfig(): Config {
    const file = path.join(tmp, 'probe.yaml');
    fs.writeFileSync(
      file,
      yaml.dump({ apcore: { version: '1.0.0', project: { name: 'probe' } } }),
      'utf-8',
    );
    return Config.load(file, { validate: false });
  }

  it('observability_namespace_declares_what_the_schema_declares', () => {
    const tc = caseFor('observability_namespace_declares_what_the_schema_declares') as unknown as {
      builtin_namespace: string;
      expected: { violations: string[]; required_keys: string[] };
    };
    const ns = tc.builtin_namespace;
    const leavesOfNs = Object.keys(flatten(probeConfig().namespace(ns))).map(
      (key) => `${ns}.${key}`,
    );
    const violations = leavesOfNs.filter((key) => !allowed.has(key)).sort();
    expect(violations, 'keys the built-in registration declares but the schemas do not').toEqual(
      tc.expected.violations,
    );
    const missing = tc.expected.required_keys.filter((key) => !leavesOfNs.includes(key));
    expect(missing, 'required keys the registration does not supply').toEqual([]);
  });

  it('the_three_builtin_namespaces_are_registered', () => {
    const tc = caseFor('the_three_builtin_namespaces_are_registered') as unknown as {
      builtin_namespaces_registered: Record<string, string>;
      expected: { missing: string[] };
    };
    const missing = Object.entries(tc.builtin_namespaces_registered)
      .filter(([name, prefix]) => _globalNsRegistry.get(name)?.envPrefix !== prefix)
      .map(([name]) => name);
    expect(missing).toEqual(tc.expected.missing);
  });

  it('drives every fixture case', () => {
    expect((fixture.test_cases ?? []).map((c) => c.id).sort()).toEqual(
      [
        'observability_namespace_declares_what_the_schema_declares',
        'sdk_constraint_table_declares_no_undeclared_key',
        'sdk_default_table_declares_no_undeclared_key',
        'sdk_default_values_match_canonical_defaults',
        'sdk_reproduces_every_canonical_default',
        'the_three_builtin_namespaces_are_registered',
        'unknown_framework_key_is_rejected_under_strict',
        'unknown_framework_key_is_retained_by_default',
      ].sort(),
    );
  });
});
