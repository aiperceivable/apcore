import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { Config, _globalEnvMap, _envMapClaimed } from '../src/config.js';
import { ConfigError, ConfigNotFoundError } from '../src/errors.js';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as os from 'node:os';

describe('Config', () => {
  it('creates with provided data', () => {
    const cfg = new Config({ name: 'test' });
    expect(cfg.get('name')).toBe('test');
  });

  it('creates with no arguments', () => {
    const cfg = new Config();
    expect(cfg.get('anything')).toBeUndefined();
  });

  it('returns various value types', () => {
    const cfg = new Config({
      str: 'hello',
      num: 42,
      bool: true,
      arr: [1, 2, 3],
      obj: { nested: true },
      nil: null,
    });
    expect(cfg.get('str')).toBe('hello');
    expect(cfg.get('num')).toBe(42);
    expect(cfg.get('bool')).toBe(true);
    expect(cfg.get('arr')).toEqual([1, 2, 3]);
    expect(cfg.get('obj')).toEqual({ nested: true });
    expect(cfg.get('nil')).toBeNull();
  });

  it('traverses nested objects with dot-path', () => {
    const cfg = new Config({
      database: {
        host: 'db.example.com',
        port: 5432,
        credentials: { user: 'admin', password: 'secret' },
      },
    });
    expect(cfg.get('database.host')).toBe('db.example.com');
    expect(cfg.get('database.port')).toBe(5432);
    expect(cfg.get('database.credentials.user')).toBe('admin');
  });

  it('returns nested object for partial path', () => {
    const cfg = new Config({ a: { b: { c: 'deep' } } });
    expect(cfg.get('a.b')).toEqual({ c: 'deep' });
  });

  it('returns undefined when key missing and no default', () => {
    const cfg = new Config({ x: 1 });
    expect(cfg.get('y')).toBeUndefined();
  });

  it('returns default value when key missing', () => {
    const cfg = new Config({ x: 1 });
    expect(cfg.get('y', 'fallback')).toBe('fallback');
    expect(cfg.get('y', 42)).toBe(42);
  });

  it('returns default when dot-path partially exists', () => {
    const cfg = new Config({ a: { b: 1 } });
    expect(cfg.get('a.c', 'default')).toBe('default');
    expect(cfg.get('a.b.c.d', 'deep-default')).toBe('deep-default');
  });

  it('returns default when traversal hits non-object', () => {
    const cfg = new Config({ a: 'string-value' });
    expect(cfg.get('a.b', 'default')).toBe('default');
  });

  it('returns default when traversal hits null', () => {
    const cfg = new Config({ a: null });
    expect(cfg.get('a.b', 'default')).toBe('default');
  });
});

describe('Config.set', () => {
  it('sets a top-level value', () => {
    const cfg = new Config({ x: 1 });
    cfg.set('y', 2);
    expect(cfg.get('y')).toBe(2);
  });

  it('sets a nested value via dot-path', () => {
    const cfg = new Config({});
    cfg.set('a.b.c', 'deep');
    expect(cfg.get('a.b.c')).toBe('deep');
  });

  it('overwrites existing value', () => {
    const cfg = new Config({ x: 1 });
    cfg.set('x', 99);
    expect(cfg.get('x')).toBe(99);
  });
});

describe('Config.data', () => {
  it('returns a deep copy', () => {
    const cfg = new Config({ a: { b: 1 } });
    const d = cfg.data;
    (d['a'] as Record<string, unknown>)['b'] = 999;
    expect(cfg.get('a.b')).toBe(1); // Original unchanged
  });
});

/**
 * A Config carrying the canonical defaults PLUS the two keys that a
 * configuration must DECLARE for itself.
 *
 * `Config.fromDefaults()` on its own no longer passes `validate()`:
 * PROTOCOL_SPEC §9.1 gives `version` and `project.name` no canonical default,
 * which is precisely why they are the only required keys, and §9.3 step 1
 * evaluates requiredness against the declared document. Defaults resolve
 * values; they do not declare a project.
 */
function defaultsPlusRequired(): Config {
  const cfg = Config.fromDefaults();
  cfg.set('version', '1.0.0');
  cfg.set('project.name', 'validate-test');
  return cfg;
}

describe('Config.validate', () => {
  it('passes with all required fields present', () => {
    const cfg = defaultsPlusRequired();
    expect(() => cfg.validate()).not.toThrow();
  });

  it('fails on a bare Config.fromDefaults() — defaults do not declare a config', () => {
    // Regression guard for the dead-check bug: DEFAULTS used to invent
    // `version: '0.16.0'` and `project.name: 'apcore'`, so this passed and the
    // required-field loop could never fire for any input.
    const cfg = Config.fromDefaults();
    try {
      cfg.validate();
      throw new Error('expected validate() to throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigError);
      expect((e as ConfigError).code).toBe('CONFIG_INVALID');
      expect((e as ConfigError).message).toContain("'version'");
      expect((e as ConfigError).message).toContain("'project.name'");
    }
  });

  it('does not require keys that carry a canonical default', () => {
    // extensions.root / schema.root / acl.root / acl.default_effect all have
    // defaults in defaults.schema.json, so their absence from the declared
    // document is not an error (§9.1).
    const cfg = new Config({ version: '1.0.0', project: { name: 'minimal' } });
    expect(() => cfg.validate()).not.toThrow();
  });

  it('fails when required fields missing', () => {
    const cfg = new Config({});
    expect(() => cfg.validate()).toThrow(ConfigError);
    expect(() => cfg.validate()).toThrow('Missing required field');
  });

  it('collects multiple errors', () => {
    const cfg = new Config({});
    try {
      cfg.validate();
    } catch (e) {
      expect((e as ConfigError).message).toContain('version');
      expect((e as ConfigError).message).toContain('project.name');
    }
  });

  it('validates constraints', () => {
    const cfg = defaultsPlusRequired();
    cfg.set('acl.default_effect', 'invalid');
    expect(() => cfg.validate()).toThrow("must be 'allow' or 'deny'");
  });

  it('validates sampling_rate range', () => {
    const cfg = defaultsPlusRequired();
    cfg.set('observability.tracing.sampling_rate', 2.0);
    expect(() => cfg.validate()).toThrow('[0.0, 1.0]');
  });

  // `middleware.circuit_breaker.*` was removed from the constraint table: the
  // canonical `apcore-config.schema.json` declares MiddlewareConfig as
  // `{ disabled }` with additionalProperties:false, so those keys were rejected
  // by the config schema while all three SDKs happily validated them — and no
  // SDK ever read them. The breaker's knobs are its constructor options and the
  // declarative middleware-chain config, not a flat config namespace.
  it('does not validate middleware.circuit_breaker.* — not a config key', () => {
    for (const key of [
      'middleware.circuit_breaker.open_threshold',
      'middleware.circuit_breaker.recovery_window_ms',
      'middleware.circuit_breaker.window_size',
      'middleware.circuit_breaker.min_samples',
    ]) {
      const cfg = defaultsPlusRequired();
      // A value that WOULD have been rejected by the old constraint.
      cfg.set(key, -1);
      expect(() => cfg.validate()).not.toThrow();
    }
  });

  it('rejects out-of-range sys_modules.events.thresholds.error_rate with CONFIG_INVALID', () => {
    const cfg = defaultsPlusRequired();
    cfg.set('sys_modules.events.thresholds.error_rate', 1.5);
    try {
      cfg.validate();
      throw new Error('expected validate() to throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigError);
      expect((e as ConfigError).code).toBe('CONFIG_INVALID');
      expect((e as ConfigError).message).toContain('sys_modules.events.thresholds.error_rate');
    }
  });

  it('rejects sys_modules.events.thresholds.latency_p99_ms of 0 (must be > 0)', () => {
    const cfg = defaultsPlusRequired();
    cfg.set('sys_modules.events.thresholds.latency_p99_ms', 0);
    expect(() => cfg.validate()).toThrow('sys_modules.events.thresholds.latency_p99_ms');
  });

  it('rejects new sys_modules.error_history integer constraints (< 1)', () => {
    const cfg = defaultsPlusRequired();
    cfg.set('sys_modules.error_history.max_entries_per_module', 0);
    expect(() => cfg.validate()).toThrow('sys_modules.error_history.max_entries_per_module');

    const cfg2 = defaultsPlusRequired();
    cfg2.set('sys_modules.error_history.max_total_entries', 0);
    expect(() => cfg2.validate()).toThrow('sys_modules.error_history.max_total_entries');
  });

  it('passes for a fully-valid config that exercises the new constraints', () => {
    const cfg = defaultsPlusRequired();
    cfg.set('sys_modules.error_history.max_entries_per_module', 50);
    cfg.set('sys_modules.error_history.max_total_entries', 500);
    cfg.set('sys_modules.events.thresholds.error_rate', 0.1);
    cfg.set('sys_modules.events.thresholds.latency_p99_ms', 5000);
    expect(() => cfg.validate()).not.toThrow();
  });
});

describe('Config.fromDefaults', () => {
  it('creates config with default values', () => {
    const cfg = Config.fromDefaults();
    expect(cfg.get('executor.default_timeout')).toBe(30000);
    expect(cfg.get('acl.default_effect')).toBe('deny');
    expect(cfg.get('extensions.root')).toBe('./extensions');
  });

  it('does NOT invent a version or a project name', () => {
    // §9.1: these two keys have no canonical default. The table used to carry
    // `version: '0.16.0'` (a frozen number that was neither the SDK version
    // nor a spec value) and `project.name: 'apcore'` purely so the
    // required-field check would always find them.
    const cfg = Config.fromDefaults();
    expect(cfg.get('version')).toBeUndefined();
    expect(cfg.get('project.name')).toBeUndefined();
  });
});

describe('Config.load', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-config-test-'));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it('loads a valid YAML file', () => {
    const yamlContent = `
version: "1.0.0"
extensions:
  root: ./ext
schema:
  root: ./schemas
acl:
  root: ./acl
  default_effect: allow
project:
  name: test-project
`;
    const yamlPath = path.join(tmpDir, 'config.yaml');
    fs.writeFileSync(yamlPath, yamlContent);

    const cfg = Config.load(yamlPath);
    expect(cfg.get('version')).toBe('1.0.0');
    expect(cfg.get('project.name')).toBe('test-project');
    expect(cfg.get('acl.default_effect')).toBe('allow');
    // Defaults merged in
    expect(cfg.get('executor.default_timeout')).toBe(30000);
  });

  it('throws ConfigNotFoundError for missing file', () => {
    expect(() => Config.load('/nonexistent/config.yaml')).toThrow(ConfigNotFoundError);
  });

  it('throws ConfigError for invalid YAML', () => {
    const yamlPath = path.join(tmpDir, 'bad.yaml');
    fs.writeFileSync(yamlPath, '{{{{invalid yaml');
    expect(() => Config.load(yamlPath)).toThrow(ConfigError);
  });

  it('throws ConfigError for non-mapping YAML', () => {
    const yamlPath = path.join(tmpDir, 'list.yaml');
    fs.writeFileSync(yamlPath, '- item1\n- item2\n');
    expect(() => Config.load(yamlPath)).toThrow('must be a mapping');
  });

  it('skips validation when validate=false', () => {
    const yamlPath = path.join(tmpDir, 'empty.yaml');
    fs.writeFileSync(yamlPath, '{}');
    // Would fail validation (missing required fields), but we skip it
    expect(() => Config.load(yamlPath, { validate: false })).not.toThrow();
  });

  it('supports reload', () => {
    const yamlPath = path.join(tmpDir, 'reload.yaml');
    const yaml1 = `
version: "1.0.0"
extensions: { root: ./ext }
schema: { root: ./schemas }
acl: { root: ./acl, default_effect: deny }
project: { name: v1 }
`;
    fs.writeFileSync(yamlPath, yaml1);
    const cfg = Config.load(yamlPath);
    expect(cfg.get('project.name')).toBe('v1');

    const yaml2 = yaml1.replace('name: v1', 'name: v2');
    fs.writeFileSync(yamlPath, yaml2);
    cfg.reload();
    expect(cfg.get('project.name')).toBe('v2');
  });

  it('throws on reload without yaml path', () => {
    const cfg = new Config({ version: '1.0.0' });
    expect(() => cfg.reload()).toThrow('not loaded from a YAML file');
  });
});

describe('Config env overrides', () => {
  const envKeys: string[] = [];

  afterEach(() => {
    for (const key of envKeys) {
      delete process.env[key];
    }
    envKeys.length = 0;
  });

  function setEnv(key: string, value: string): void {
    process.env[key] = value;
    envKeys.push(key);
  }

  it('applies APCORE_ env overrides', () => {
    setEnv('APCORE_PROJECT_NAME', 'env-project');
    const cfg = Config.fromDefaults();
    expect(cfg.get('project.name')).toBe('env-project');
  });

  it('handles double underscore as literal underscore', () => {
    setEnv('APCORE_ACL_DEFAULT__EFFECT', 'allow');
    const cfg = Config.fromDefaults();
    expect(cfg.get('acl.default_effect')).toBe('allow');
  });

  it('coerces numeric strings', () => {
    setEnv('APCORE_EXECUTOR_DEFAULT__TIMEOUT', '5000');
    const cfg = Config.fromDefaults();
    expect(cfg.get('executor.default_timeout')).toBe(5000);
  });

  it('coerces boolean strings', () => {
    setEnv('APCORE_EXTENSIONS_AUTO__DISCOVER', 'false');
    const cfg = Config.fromDefaults();
    expect(cfg.get('extensions.auto_discover')).toBe(false);
  });

  it('coerces leading-zero integer strings to numbers (A-D-008)', () => {
    // Python int("08") == 8; the old String(parsed) === value guard kept
    // "08" a string because String(8) !== "08".
    setEnv('APCORE_CUSTOM_LEADING__ZERO', '08');
    const cfg = Config.fromDefaults();
    expect(cfg.get('custom.leading_zero')).toBe(8);
  });

  it('coerces signed integer strings to numbers (A-D-008)', () => {
    // Python int("+5") == 5; String(5) !== "+5" under the old guard.
    setEnv('APCORE_CUSTOM_SIGNED__INT', '+5');
    const cfg = Config.fromDefaults();
    expect(cfg.get('custom.signed_int')).toBe(5);
  });

  it('coerces exponent float strings to numbers (A-D-008)', () => {
    // Python int("1e0") fails, then float("1e0") == 1.0.
    setEnv('APCORE_CUSTOM_EXP__FLOAT', '1e0');
    const cfg = Config.fromDefaults();
    expect(cfg.get('custom.exp_float')).toBe(1);
    expect(typeof cfg.get('custom.exp_float')).toBe('number');
  });
});

describe('Config legacy-mode global env map (A-D-04)', () => {
  const envKeys: string[] = [];

  afterEach(() => {
    for (const key of envKeys) {
      delete process.env[key];
    }
    envKeys.length = 0;
    _globalEnvMap.clear();
    _envMapClaimed.clear();
  });

  function setEnv(key: string, value: string): void {
    process.env[key] = value;
    envKeys.push(key);
  }

  it('applies global env map in legacy mode (peer: python config.py:259)', () => {
    Config.envMap({ PORT: 'port' });
    setEnv('PORT', '3000');

    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-cfg-'));
    const yamlPath = path.join(dir, 'apcore.yaml');
    // Legacy YAML: no top-level "apcore" mapping key.
    fs.writeFileSync(yamlPath, 'project:\n  name: legacy-app\n');
    try {
      const cfg = Config.load(yamlPath, { validate: false });
      expect(cfg.get('port')).toBe(3000);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe('path-typed configuration keys (PROTOCOL_SPEC §9.2.1)', () => {
  const EXPECTED = [
    'acl.root',
    'bindings.dir',
    'extensions.root',
    'extensions.roots[]',
    'id_map.overrides',
    'schema.root',
    'sys_modules.control.overrides_path',
  ];

  it('accessor matches the declared set in both directions', () => {
    const actual = new Set(Config.pathTypedKeys());
    const expected = new Set(EXPECTED);
    const invented = [...actual].filter((k) => !expected.has(k));
    const missing = [...expected].filter((k) => !actual.has(k));
    expect(invented).toEqual([]);
    expect(missing).toEqual([]);
  });

  it('bindings.pattern is not path-typed', () => {
    // Discriminating case. It sits in the same section as `bindings.dir` and its
    // default (`*.binding.yaml`) looks like a filename, so an implementation that
    // classifies by section sweeps it in. It is a glob matched WITHIN
    // `bindings.dir`, never resolved as a path itself.
    expect(Config.pathTypedKeys()).not.toContain('bindings.pattern');
  });

  it('non-path string keys are not path-typed', () => {
    // An implementation that marks every string key as path-typed passes any
    // presence-only assertion and fails here.
    for (const key of [
      'acl.default_effect',
      'schema.strategy',
      'logging.level',
      'observability.tracing.exporter',
      'project.name',
    ]) {
      expect(Config.pathTypedKeys()).not.toContain(key);
    }
  });

  it('is a property of the spec, not of a loaded document', () => {
    expect(Config.pathTypedKeys()).toEqual(EXPECTED);
  });
});

// ---------------------------------------------------------------------------
// D-74 (spec v1.49.0) — `Config.get('')` is not an error
// ---------------------------------------------------------------------------

describe('D-74: an empty key is not an error', () => {
  // The `Config.get` Inputs row said an empty key "is rejected with
  // ValueError/ConfigInvalidError". The same block's Errors row said "No
  // errors raised under normal operation", and no SDK had ever rejected it —
  // apcore-rust's `get` has no error channel at all. The clause described
  // behaviour that never existed, and a conformance case written from it would
  // have failed on all three. It was deleted: an empty key resolves no value
  // and returns the default, like any other absent key.
  //
  // Two assertions, because "does not throw" alone is satisfied by a
  // short-circuit `if (!key) return undefined`, which ignores a caller-supplied
  // default and is NOT "like any other absent key".

  it('does not throw', () => {
    const cfg = new Config({ a: { b: 1 } });
    expect(() => cfg.get('')).not.toThrow();
    expect(cfg.get('')).toBeUndefined();
  });

  it('takes the ordinary absent-key path, honouring the default', () => {
    const cfg = new Config({ a: { b: 1 } });
    expect(cfg.get('', 'SENTINEL')).toBe('SENTINEL');
  });

  it('control: a present key is unaffected', () => {
    // Without this, an implementation returning the default for EVERY key
    // satisfies both assertions above.
    const cfg = new Config({ a: { b: 1 } });
    expect(cfg.get('a.b')).toBe(1);
    expect(cfg.get('a.b', 'SENTINEL')).toBe(1);
  });

  it('control: an absent non-empty key behaves identically', () => {
    // "Like any other absent key" is the decision's own wording, so the two
    // paths are asserted to AGREE rather than each being checked alone.
    const cfg = new Config({ a: { b: 1 } });
    expect(cfg.get('')).toBe(cfg.get('no.such.key'));
    expect(cfg.get('', 'SENTINEL')).toBe(cfg.get('no.such.key', 'SENTINEL'));
  });
});
