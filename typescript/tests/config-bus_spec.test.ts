/**
 * Spec-traced contract tests for the Config Bus feature (TypeScript SDK).
 *
 * Source spec: apcore/docs/features/config-bus.md (## Contract: blocks).
 * Mirrors the canonical Python suite apcore-python/tests/test_config_bus_spec.py
 * row-for-row: each it() name carries the verbatim clause id of the form
 * `config_bus.<method>.<kind>.<detail>` so the cross-language test matrix
 * (python / typescript / rust) lines up.
 *
 * These tests exercise the PUBLIC contract only and never mutate production
 * source. The namespace registry is module-global, so every suite restores it
 * in beforeEach/afterEach to stay isolated.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import {
  Config,
  _globalNsRegistry,
  _globalEnvMap,
  _envMapClaimed,
  _envPrefixUsed,
} from '../src/config.js';
import {
  ConfigBindError,
  ConfigEnvMapConflictError,
  ConfigEnvPrefixConflictError,
  ConfigError,
  ConfigMountError,
  ConfigNamespaceDuplicateError,
  ConfigNamespaceReservedError,
  ConfigNotFoundError,
} from '../src/errors.js';

// ---------------------------------------------------------------------------
// Registry isolation helpers (the namespace registry is module-global)
// ---------------------------------------------------------------------------

const BUILTIN_NAMES = new Set(['observability', 'obs', 'sys_modules']);

/**
 * Snapshot of the env prefixes owned by built-in namespaces; everything else
 * is a test artifact to be dropped on reset.
 */
function builtinEnvPrefixes(): Set<string> {
  const prefixes = new Set<string>();
  for (const [name, reg] of _globalNsRegistry) {
    if (BUILTIN_NAMES.has(name)) prefixes.add(reg.envPrefix);
  }
  return prefixes;
}

function resetRegistry(): void {
  const keepPrefixes = builtinEnvPrefixes();
  for (const key of Array.from(_globalNsRegistry.keys())) {
    if (!BUILTIN_NAMES.has(key)) _globalNsRegistry.delete(key);
  }
  // Drop any env-prefix claims not owned by a built-in namespace.
  for (const prefix of Array.from(_envPrefixUsed)) {
    if (!keepPrefixes.has(prefix)) _envPrefixUsed.delete(prefix);
  }
  _globalEnvMap.clear();
  _envMapClaimed.clear();
}

function writeYaml(filePath: string, body: string): string {
  fs.writeFileSync(filePath, body, 'utf-8');
  return filePath;
}

let tmpDir: string;
function tmp(name: string): string {
  return path.join(tmpDir, name);
}

/** Dataclass-equivalent: a constructor that reads namespace data. */
class PluginCfg {
  timeout: number;
  retries: number;
  constructor(data: Record<string, unknown>) {
    // Mirror Python _PluginCfg(timeout=5000, retries=3): reject unexpected fields
    // so a bind on bad data raises (CONFIG_BIND_ERROR).
    const allowed = new Set(['timeout', 'retries']);
    for (const key of Object.keys(data)) {
      if (!allowed.has(key)) {
        throw new Error(`unexpected field for PluginCfg: '${key}'`);
      }
    }
    this.timeout = (data['timeout'] as number) ?? 5000;
    this.retries = (data['retries'] as number) ?? 3;
  }
}

beforeEach(() => {
  resetRegistry();
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'cfgbus-spec-'));
});

afterEach(() => {
  resetRegistry();
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    /* ignore */
  }
});

// ===========================================================================
// Contract: Config.registerNamespace
// ===========================================================================

describe('Contract: Config.registerNamespace', () => {
  it('config_bus.register_namespace.input.name.reserved_apcore: reject reserved name "apcore"', () => {
    try {
      Config.registerNamespace({ name: 'apcore' });
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigNamespaceReservedError);
      expect((e as ConfigNamespaceReservedError).code).toBe('CONFIG_NAMESPACE_RESERVED');
    }
  });

  it('config_bus.register_namespace.input.name.reserved_config: reject reserved name "_config"', () => {
    try {
      Config.registerNamespace({ name: '_config' });
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigNamespaceReservedError);
      expect((e as ConfigNamespaceReservedError).code).toBe('CONFIG_NAMESPACE_RESERVED');
    }
  });

  it('config_bus.register_namespace.error.CONFIG_NAMESPACE_DUPLICATE: second registration raises', () => {
    Config.registerNamespace({ name: 'dup_spec_ns' });
    try {
      Config.registerNamespace({ name: 'dup_spec_ns' });
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigNamespaceDuplicateError);
      expect((e as ConfigNamespaceDuplicateError).code).toBe('CONFIG_NAMESPACE_DUPLICATE');
    }
  });

  it('config_bus.register_namespace.error.CONFIG_ENV_PREFIX_CONFLICT: shared env prefix raises', () => {
    Config.registerNamespace({ name: 'alpha_spec', envPrefix: 'SHARED_PREFIX' });
    try {
      Config.registerNamespace({ name: 'beta_spec', envPrefix: 'SHARED_PREFIX' });
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigEnvPrefixConflictError);
      expect((e as ConfigEnvPrefixConflictError).code).toBe('CONFIG_ENV_PREFIX_CONFLICT');
    }
  });

  it('config_bus.register_namespace.error.CONFIG_ENV_MAP_CONFLICT: env var claimed twice raises', () => {
    Config.envMap({ SPEC_PORT: 'port' });
    try {
      Config.registerNamespace({ name: 'gamma_spec', envMap: { SPEC_PORT: 'server_port' } });
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigEnvMapConflictError);
      expect((e as ConfigEnvMapConflictError).code).toBe('CONFIG_ENV_MAP_CONFLICT');
    }
  });

  it('config_bus.register_namespace.property.async: synchronous, returns void (no awaitable)', () => {
    const result = Config.registerNamespace({ name: 'sync_spec_ns' });
    expect(result).toBeUndefined();
  });

  it('config_bus.register_namespace.property.idempotent: second identical registration raises (idempotent: false)', () => {
    Config.registerNamespace({ name: 'once_spec_ns' });
    expect(() => Config.registerNamespace({ name: 'once_spec_ns' })).toThrow(
      ConfigNamespaceDuplicateError,
    );
    const names = Config.registeredNamespaces().map((r) => r.name);
    expect(names.filter((n) => n === 'once_spec_ns').length).toBe(1);
  });

  it('config_bus.register_namespace.property.pure: mutates class-level registry (pure: false)', () => {
    const before = new Set(Config.registeredNamespaces().map((r) => r.name));
    expect(before.has('pure_spec_ns')).toBe(false);
    Config.registerNamespace({ name: 'pure_spec_ns' });
    const after = new Set(Config.registeredNamespaces().map((r) => r.name));
    expect(after.has('pure_spec_ns')).toBe(true);
  });

  it.skip(
    'config_bus.register_namespace.property.thread_safe: clause too vague — thread_safe: false, ' +
      'concurrent registration is explicitly unsupported (call before any concurrent Config.load())',
    () => {
      /* intentionally not asserted, mirrors Python skip */
    },
  );
});

// ===========================================================================
// Contract: Config.load
// ===========================================================================

describe('Contract: Config.load', () => {
  it('config_bus.load.error.CONFIG_NOT_FOUND: missing path raises', () => {
    const missing = tmp('does_not_exist.yaml');
    try {
      Config.load(missing);
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigNotFoundError);
      expect((e as ConfigNotFoundError).code).toBe('CONFIG_NOT_FOUND');
    }
  });

  it('config_bus.load.error.CONFIG_INVALID: malformed YAML raises ConfigError(code=CONFIG_INVALID)', () => {
    const p = writeYaml(tmp('bad.yaml'), 'key: [unterminated\n');
    try {
      Config.load(p);
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigError);
      expect((e as ConfigError).code).toBe('CONFIG_INVALID');
    }
  });

  it('config_bus.load.error.CONFIG_INVALID.non_mapping: YAML root that is not a mapping raises', () => {
    const p = writeYaml(tmp('list.yaml'), '- a\n- b\n');
    try {
      Config.load(p);
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigError);
      expect((e as ConfigError).code).toBe('CONFIG_INVALID');
    }
  });

  it('config_bus.load.property.async: load returns a Config, not a Promise', () => {
    const p = writeYaml(tmp('ok.yaml'), "version: '0.15.0'\n");
    const result = Config.load(p, { validate: false });
    expect(result).toBeInstanceOf(Config);
    expect(typeof (result as unknown as { then?: unknown }).then).not.toBe('function');
  });

  it('config_bus.load.property.idempotent: loading the same file twice yields equivalent Config', () => {
    const p = writeYaml(
      tmp('idem.yaml'),
      "version: '0.15.0'\nexecutor:\n  default_timeout: 30000\n",
    );
    const first = Config.load(p, { validate: false });
    const second = Config.load(p, { validate: false });
    expect(first.get('executor.default_timeout')).toBe(second.get('executor.default_timeout'));
    expect(first.data).toEqual(second.data);
  });

  it('config_bus.load.property.pure: result reflects on-disk content (pure: false, reads FS)', () => {
    const p = writeYaml(tmp('fs.yaml'), "version: '9.9.9'\n");
    const config = Config.load(p, { validate: false });
    expect(config.get('version')).toBe('9.9.9');
  });
});

// ===========================================================================
// Contract: Config.get
// ===========================================================================

describe('Contract: Config.get', () => {
  it('config_bus.get.input.key.empty: an empty key returns the default, it is not an error', () => {
    // D-74: an empty key resolves no value and returns the default, exactly
    // like any other absent key. This was skipped in all three SDKs with the
    // reason "spec says empty key is rejected" — written before D-74 deleted
    // that row and recorded that it "described behaviour no SDK has ever had".
    // A symmetric skip is invisible to the skip-asymmetry guard.
    const config = Config.fromDefaults();
    expect(config.get('')).toBeUndefined();
    const sentinel = Symbol('sentinel');
    expect(config.get('', sentinel)).toBe(sentinel);
  });

  it('config_bus.get.input.default.missing_key: missing key returns provided default (no error)', () => {
    const config = Config.fromDefaults();
    const sentinel = Symbol('sentinel');
    expect(config.get('definitely.absent.key', sentinel)).toBe(sentinel);
  });

  it('config_bus.get.property.async: get returns a plain value, not a Promise', () => {
    const config = Config.fromDefaults();
    const result = config.get('version');
    expect(typeof (result as { then?: unknown })?.then).not.toBe('function');
  });

  it('config_bus.get.property.idempotent: two identical calls on same state return identical outcomes', () => {
    const p = writeYaml(tmp('g.yaml'), 'executor:\n  default_timeout: 1234\n');
    const config = Config.load(p, { validate: false });
    const first = config.get('executor.default_timeout');
    const second = config.get('executor.default_timeout');
    expect(first).toBe(second);
    expect(first).toBe(1234);
  });

  it('config_bus.get.property.pure: get does not mutate config state', () => {
    const p = writeYaml(tmp('p.yaml'), 'executor:\n  default_timeout: 77\n');
    const config = Config.load(p, { validate: false });
    const snapshot = config.data;
    config.get('executor.default_timeout');
    config.get('missing.key', 'x');
    expect(config.data).toEqual(snapshot);
  });

  it('config_bus.get.property.thread_safe: >=8 concurrent reads with distinct keys all correct', async () => {
    Config.registerNamespace({ name: 'concur_ns' });
    let body = "apcore:\n  version: '0.15.0'\nconcur_ns:\n";
    for (let i = 0; i < 8; i++) body += `  k${i}: ${i}\n`;
    const p = writeYaml(tmp('c.yaml'), body);
    const config = Config.load(p, { validate: false });

    const read = async (i: number): Promise<unknown> => config.get(`concur_ns.k${i}`);
    const results = await Promise.all([...Array(8).keys()].map((i) => read(i)));
    expect(results).toEqual([...Array(8).keys()]);
  });
});

// ===========================================================================
// Contract: Config.namespace
// ===========================================================================

describe('Contract: Config.namespace', () => {
  it('config_bus.namespace.input.name.unregistered: unregistered namespace returns empty object', () => {
    const config = Config.fromDefaults();
    expect(config.namespace('never_registered_ns')).toEqual({});
  });

  it('config_bus.namespace.returns.merged: returns defaults + YAML merged values', () => {
    Config.registerNamespace({ name: 'nsret', defaults: { retries: 3 } });
    const p = writeYaml(
      tmp('n.yaml'),
      "apcore:\n  version: '0.15.0'\nnsret:\n  timeout: 10000\n",
    );
    const config = Config.load(p, { validate: false });
    const result = config.namespace('nsret');
    expect(result['timeout']).toBe(10000);
    expect(result['retries']).toBe(3);
  });

  it('config_bus.namespace.property.async: returns an object, not a Promise', () => {
    const config = Config.fromDefaults();
    const result = config.namespace('anything');
    expect(typeof result).toBe('object');
    expect(typeof (result as { then?: unknown }).then).not.toBe('function');
  });

  it('config_bus.namespace.property.pure: returns a copy — mutating result does not affect config', () => {
    Config.registerNamespace({ name: 'nspure', defaults: { a: 1 } });
    const p = writeYaml(tmp('np.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    const result = config.namespace('nspure');
    result['a'] = 999;
    result['injected'] = true;
    const fresh = config.namespace('nspure');
    expect(fresh['a']).toBe(1);
    expect('injected' in fresh).toBe(false);
  });

  it('config_bus.namespace.property.thread_safe: >=8 concurrent namespace() calls stay consistent', async () => {
    Config.registerNamespace({ name: 'nsconcur', defaults: { v: 42 } });
    const p = writeYaml(tmp('nc.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });

    const read = async (): Promise<Record<string, unknown>> => config.namespace('nsconcur');
    const results = await Promise.all([...Array(8)].map(() => read()));
    expect(results.every((r) => r['v'] === 42)).toBe(true);
  });
});

// ===========================================================================
// Contract: Config.bind
// ===========================================================================

describe('Contract: Config.bind', () => {
  it('config_bus.bind.error.CONFIG_BIND_ERROR: unexpected field raises ConfigBindError', () => {
    Config.registerNamespace({ name: 'bindbad' });
    const p = writeYaml(
      tmp('bb.yaml'),
      "apcore:\n  version: '0.15.0'\nbindbad:\n  not_a_field: 1\n",
    );
    const config = Config.load(p, { validate: false });
    try {
      config.bind('bindbad', PluginCfg);
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigBindError);
      expect((e as ConfigBindError).code).toBe('CONFIG_BIND_ERROR');
    }
  });

  it('config_bus.bind.returns.instance: successful bind returns a populated instance', () => {
    Config.registerNamespace({ name: 'bindok', defaults: { timeout: 5000, retries: 3 } });
    const p = writeYaml(
      tmp('bo.yaml'),
      "apcore:\n  version: '0.15.0'\nbindok:\n  timeout: 8000\n",
    );
    const config = Config.load(p, { validate: false });
    const result = config.bind('bindok', PluginCfg);
    expect(result).toBeInstanceOf(PluginCfg);
    expect(result.timeout).toBe(8000);
    expect(result.retries).toBe(3);
  });

  it('config_bus.bind.property.async: bind returns an instance, not a Promise', () => {
    Config.registerNamespace({ name: 'bindasync', defaults: { timeout: 1, retries: 1 } });
    const p = writeYaml(tmp('ba.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    const result = config.bind('bindasync', PluginCfg);
    expect(typeof (result as unknown as { then?: unknown }).then).not.toBe('function');
  });

  it('config_bus.bind.property.pure: bind reads a snapshot and does not mutate config', () => {
    Config.registerNamespace({ name: 'bindpure', defaults: { timeout: 1, retries: 1 } });
    const p = writeYaml(tmp('bp.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    const snapshot = config.data;
    config.bind('bindpure', PluginCfg);
    expect(config.data).toEqual(snapshot);
  });

  it('config_bus.bind.property.thread_safe: >=8 concurrent binds succeed and agree', async () => {
    Config.registerNamespace({ name: 'bindconcur', defaults: { timeout: 5000, retries: 3 } });
    const p = writeYaml(tmp('bc.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });

    const doBind = async (): Promise<PluginCfg> => config.bind('bindconcur', PluginCfg);
    const results = await Promise.all([...Array(8)].map(() => doBind()));
    expect(results.every((r) => r.timeout === 5000 && r.retries === 3)).toBe(true);
  });
});

// ===========================================================================
// Contract: Config.mount
// ===========================================================================

describe('Contract: Config.mount', () => {
  it('config_bus.mount.input.namespace.reserved_config: mounting into "_config" raises', () => {
    const p = writeYaml(tmp('m.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    try {
      config.mount('_config', { fromDict: { strict: true } });
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigMountError);
      expect((e as ConfigMountError).code).toBe('CONFIG_MOUNT_ERROR');
    }
  });

  it('config_bus.mount.error.CONFIG_MOUNT_ERROR.missing_file: mounting a missing file raises', () => {
    Config.registerNamespace({ name: 'mountmiss' });
    const p = writeYaml(tmp('mm.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    try {
      config.mount('mountmiss', { fromFile: tmp('nope.yaml') });
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigMountError);
      expect((e as ConfigMountError).code).toBe('CONFIG_MOUNT_ERROR');
    }
  });

  it('config_bus.mount.error.CONFIG_MOUNT_ERROR.not_a_mapping: file that is not a mapping raises', () => {
    Config.registerNamespace({ name: 'mountlist' });
    const bad = writeYaml(tmp('listmount.yaml'), '- a\n- b\n');
    const p = writeYaml(tmp('ml.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    try {
      config.mount('mountlist', { fromFile: bad });
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigMountError);
      expect((e as ConfigMountError).code).toBe('CONFIG_MOUNT_ERROR');
    }
  });

  it('config_bus.mount.side_effect.1.merge_over_defaults: mounted dict data merged, observable via get()', () => {
    Config.registerNamespace({ name: 'mountmerge', defaults: { timeout: 1 } });
    const p = writeYaml(tmp('mg.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    config.mount('mountmerge', { fromDict: { timeout: 10000 } });
    expect(config.get('mountmerge.timeout')).toBe(10000);
  });

  it('config_bus.mount.property.async: mount returns void, not a Promise', () => {
    Config.registerNamespace({ name: 'mountasync' });
    const p = writeYaml(tmp('ma.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    const result = config.mount('mountasync', { fromDict: { x: 1 } });
    expect(result).toBeUndefined();
  });

  it('config_bus.mount.property.idempotent: mounting twice stacks/overwrites (idempotent: false)', () => {
    Config.registerNamespace({ name: 'mountstack', defaults: { items: [] } });
    const p = writeYaml(tmp('ms.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    config.mount('mountstack', { fromDict: { counter: 1 } });
    const first = config.namespace('mountstack');
    config.mount('mountstack', { fromDict: { counter: 2 } });
    const second = config.namespace('mountstack');
    expect(first['counter']).toBe(1);
    expect(second['counter']).toBe(2);
  });

  it('config_bus.mount.property.pure: mount mutates config state (pure: false)', () => {
    Config.registerNamespace({ name: 'mountmut', defaults: { v: 0 } });
    const p = writeYaml(tmp('mu.yaml'), "apcore:\n  version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    const before = config.namespace('mountmut');
    config.mount('mountmut', { fromDict: { v: 5 } });
    const after = config.namespace('mountmut');
    expect(before['v']).toBe(0);
    expect(after['v']).toBe(5);
  });

  it.skip(
    'config_bus.mount.property.thread_safe: clause too vague — thread_safe: false, ' +
      'concurrent mutation is explicitly unsupported (do not call concurrently with reads)',
    () => {
      /* intentionally not asserted, mirrors Python skip */
    },
  );
});

// ===========================================================================
// Contract: Config.reload
// ===========================================================================

describe('Contract: Config.reload', () => {
  it('config_bus.reload.error.CONFIG_NOT_FOUND: source file removed before reload raises', () => {
    const p = writeYaml(tmp('r.yaml'), "version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    fs.unlinkSync(p);
    try {
      config.reload();
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigNotFoundError);
      expect((e as ConfigNotFoundError).code).toBe('CONFIG_NOT_FOUND');
    }
  });

  it('config_bus.reload.error.CONFIG_INVALID: source becomes invalid YAML before reload raises', () => {
    const p = writeYaml(tmp('ri.yaml'), "version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    fs.writeFileSync(p, 'bad: [unterminated\n', 'utf-8');
    try {
      config.reload();
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigError);
      expect((e as ConfigError).code).toBe('CONFIG_INVALID');
    }
  });

  it('config_bus.reload.property.async: reload returns void, not a Promise', () => {
    const p = writeYaml(tmp('ra.yaml'), "version: '0.15.0'\n");
    const config = Config.load(p, { validate: false });
    const result = config.reload();
    expect(result).toBeUndefined();
  });

  it('config_bus.reload.property.idempotent: two reloads with unchanged files produce identical state', () => {
    const p = writeYaml(tmp('rid.yaml'), 'executor:\n  default_timeout: 2222\n');
    const config = Config.load(p, { validate: false });
    config.reload();
    const first = config.data;
    config.reload();
    const second = config.data;
    expect(first).toEqual(second);
  });

  it('config_bus.reload.side_effect.1.reread_filesystem: post-load edit visible only after reload()', () => {
    const p = writeYaml(tmp('rc.yaml'), 'executor:\n  default_timeout: 1\n');
    const config = Config.load(p, { validate: false });
    expect(config.get('executor.default_timeout')).toBe(1);
    fs.writeFileSync(p, 'executor:\n  default_timeout: 999\n', 'utf-8');
    expect(config.get('executor.default_timeout')).toBe(1);
    config.reload();
    expect(config.get('executor.default_timeout')).toBe(999);
  });

  it('config_bus.reload.property.pure: reload mutates state and re-applies stored mounts (pure: false)', () => {
    Config.registerNamespace({ name: 'reloadmount', defaults: { v: 0 } });
    const p = writeYaml(
      tmp('rm.yaml'),
      "apcore:\n  version: '0.15.0'\nreloadmount:\n  v: 1\n",
    );
    const config = Config.load(p, { validate: false });
    config.mount('reloadmount', { fromDict: { mounted: true } });
    config.reload();
    expect(config.get('reloadmount.v')).toBe(1);
    expect(config.get('reloadmount.mounted')).toBe(true);
  });

  it.skip(
    'config_bus.reload.property.thread_safe: clause too vague — thread_safe: false, ' +
      'concurrent reload is explicitly unsupported (no in-flight read protection)',
    () => {
      /* intentionally not asserted, mirrors Python skip */
    },
  );
});
