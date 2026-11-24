/**
 * Cross-language sync regressions for the Config Bus.
 *
 * C2 — in namespace mode, legacy `APCORE_*` env overrides must still be
 *      applied to the `apcore` namespace (PROTOCOL_SPEC §9.6.2 routes the
 *      `apcore` namespace through the §9.2 legacy merge rules).
 * W8 — the canonical `obs` namespace must be registered so `APCORE_OBS_*`
 *      env dispatch works, matching apcore-python.
 * C9 — required-field validation must be evaluated against the DECLARED
 *      document (PROTOCOL_SPEC §9.1 "What is required, and why so little is"
 *      and §9.3 step 1). Merging the default table first and checking after
 *      makes the check unreachable.
 */

import { describe, it, expect, afterEach } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { Config, DEFAULTS, getDefault } from '../src/config.js';
import { ConfigError } from '../src/errors.js';

const tmpDirs: string[] = [];

function writeConfig(contents: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-sync-cfg-'));
  tmpDirs.push(dir);
  const file = path.join(dir, 'apcore.yaml');
  fs.writeFileSync(file, contents, 'utf-8');
  return file;
}

afterEach(() => {
  while (tmpDirs.length > 0) {
    const dir = tmpDirs.pop()!;
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

function withEnv<T>(vars: Record<string, string>, fn: () => T): T {
  const saved: Record<string, string | undefined> = {};
  for (const [k, v] of Object.entries(vars)) {
    saved[k] = process.env[k];
    process.env[k] = v;
  }
  try {
    return fn();
  } finally {
    for (const [k, v] of Object.entries(saved)) {
      if (v === undefined) delete process.env[k];
      else process.env[k] = v;
    }
  }
}

// ---------------------------------------------------------------------------
// C2 — legacy APCORE_* overrides in namespace mode
// ---------------------------------------------------------------------------

describe('C2: namespace mode still honours legacy APCORE_* env overrides', () => {
  it('overrides apcore.executor.default_timeout from APCORE_EXECUTOR_DEFAULT__TIMEOUT', () => {
    const file = writeConfig('apcore:\n  executor:\n    default_timeout: 5000\n');
    const config = withEnv({ APCORE_EXECUTOR_DEFAULT__TIMEOUT: '9000' }, () =>
      Config.load(file, { validate: false }),
    );

    expect(config.get('apcore.executor.default_timeout')).toBe(9000);
    // Implicit-apcore retry (§9.9.1) must see the same value.
    expect(config.get('executor.default_timeout')).toBe(9000);
  });

  it('keeps the file value when no legacy env var is set', () => {
    const file = writeConfig('apcore:\n  executor:\n    default_timeout: 5000\n');
    const config = Config.load(file, { validate: false });
    expect(config.get('apcore.executor.default_timeout')).toBe(5000);
  });

  it('does not leak legacy overrides into other namespaces', () => {
    const file = writeConfig('apcore:\n  executor:\n    default_timeout: 5000\nobservability:\n  tracing:\n    enabled: false\n');
    const config = withEnv({ APCORE_EXECUTOR_DEFAULT__TIMEOUT: '9000' }, () =>
      Config.load(file, { validate: false }),
    );
    expect(config.get('apcore.executor.default_timeout')).toBe(9000);
    expect(config.get('observability.tracing.enabled')).toBe(false);
  });

  it('reload() re-applies legacy APCORE_* overrides in namespace mode', () => {
    const file = writeConfig('apcore:\n  executor:\n    default_timeout: 5000\n');
    withEnv({ APCORE_EXECUTOR_DEFAULT__TIMEOUT: '9000' }, () => {
      const config = Config.load(file, { validate: false });
      expect(config.get('apcore.executor.default_timeout')).toBe(9000);
      fs.writeFileSync(file, 'apcore:\n  executor:\n    default_timeout: 1000\n', 'utf-8');
      config.reload();
      expect(config.get('apcore.executor.default_timeout')).toBe(9000);
    });
  });
});

// ---------------------------------------------------------------------------
// W8 — obs namespace registration
// ---------------------------------------------------------------------------

describe('W8: the canonical `obs` namespace is registered', () => {
  it('appears in registeredNamespaces() with the APCORE_OBS env prefix', () => {
    const obs = Config.registeredNamespaces().find((n) => n.name === 'obs');
    expect(obs).toBeDefined();
    expect(obs!.envPrefix).toBe('APCORE_OBS');
  });

  it('dispatches APCORE_OBS_* env vars into obs.*', () => {
    const file = writeConfig('apcore:\n  version: "0.26.0"\n');
    const config = withEnv({ APCORE_OBS_REDACTION_REPLACEMENT: '<<hidden>>' }, () =>
      Config.load(file, { validate: false }),
    );
    expect(config.get('obs.redaction.replacement')).toBe('<<hidden>>');
  });

  it('exposes obs via config.namespace("obs")', () => {
    const file = writeConfig('apcore:\n  version: "0.26.0"\nobs:\n  redaction:\n    sensitive_keys: ["x_custom"]\n');
    const config = Config.load(file, { validate: false });
    const ns = config.namespace('obs') as Record<string, unknown>;
    expect(ns).toBeDefined();
    expect((ns['redaction'] as Record<string, unknown>)['sensitive_keys']).toEqual(['x_custom']);
  });
});

// ---------------------------------------------------------------------------
// C9 — required fields are checked against the DECLARED document
// ---------------------------------------------------------------------------

describe('C9: required-field validation runs against the declared document', () => {
  // -- The check must be REACHABLE ------------------------------------------
  //
  // The point of this whole group: the previous implementation deep-merged
  // `DEFAULTS` (which invented `version: '0.16.0'` and `project.name:
  // 'apcore'`) into the parsed file and only then looked for required fields,
  // so no document could ever fail. These two assertions are what the old
  // test suite could not have caught by construction.

  it('DEFAULTS declares neither version nor project (§9.1: no canonical default exists)', () => {
    expect(DEFAULTS['version']).toBeUndefined();
    expect(DEFAULTS['project']).toBeUndefined();
    expect(getDefault('version')).toBeUndefined();
    expect(getDefault('project.name')).toBeUndefined();
  });

  it('a merged default cannot rescue an undeclared required key', () => {
    // `extensions.root` IS supplied by the merge, so the merged view has it
    // while the declared view does not. If `validate()` ever reads the merged
    // view again, this asymmetry disappears and the check goes dead.
    const file = writeConfig('project:\n  name: minimal\n');
    const config = Config.load(file, { validate: false });

    expect(config.get('extensions.root')).toBe('./extensions');
    expect(config.getDeclared('extensions.root')).toBeUndefined();
    expect(config.getDeclared('project.name')).toBe('minimal');
    expect(config.getDeclared('version')).toBeUndefined();
  });

  // -- Accept direction ------------------------------------------------------

  it('accepts a document declaring only version and project.name', () => {
    const file = writeConfig('version: "1.0.0"\nproject:\n  name: minimal\n');
    expect(() => Config.load(file)).not.toThrow();
  });

  it('accepts a document omitting extensions.root / schema.root / acl.* (all defaulted)', () => {
    const file = writeConfig('version: "1.0.0"\nproject:\n  name: minimal\n');
    const config = Config.load(file);
    expect(config.get('extensions.root')).toBe('./extensions');
    expect(config.get('schema.root')).toBe('./schemas');
    expect(config.get('acl.root')).toBe('./acl');
    expect(config.get('acl.default_effect')).toBe('deny');
  });

  it('accepts version / project.name supplied only by environment overrides', () => {
    const file = writeConfig('extensions:\n  root: ./ext\n');
    withEnv({ APCORE_VERSION: '1.2.3', APCORE_PROJECT_NAME: 'from-env' }, () => {
      const config = Config.load(file);
      expect(config.get('version')).toBe('1.2.3');
      expect(config.get('project.name')).toBe('from-env');
    });
  });

  it('accepts a required key supplied by a runtime set()', () => {
    const file = writeConfig('version: "1.0.0"\n');
    const config = Config.load(file, { validate: false });
    expect(() => config.validate()).toThrow("project.name");
    config.set('project.name', 'later');
    expect(() => config.validate()).not.toThrow();
  });

  // -- Reject direction ------------------------------------------------------

  it('rejects a document that omits version with CONFIG_INVALID', () => {
    const file = writeConfig('project:\n  name: minimal\n');
    try {
      Config.load(file);
      throw new Error('expected Config.load to throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigError);
      expect((e as ConfigError).code).toBe('CONFIG_INVALID');
      expect((e as ConfigError).message).toContain("'version'");
    }
  });

  it('rejects a document that omits project.name with CONFIG_INVALID', () => {
    const file = writeConfig('version: "1.0.0"\nproject:\n  version: "0.1.0"\n');
    try {
      Config.load(file);
      throw new Error('expected Config.load to throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigError);
      expect((e as ConfigError).code).toBe('CONFIG_INVALID');
      expect((e as ConfigError).message).toContain("'project.name'");
    }
  });

  it('does NOT reject a document that omits extensions.root (it carries a default)', () => {
    const file = writeConfig('version: "1.0.0"\nproject:\n  name: minimal\nextensions:\n  auto_discover: false\n');
    expect(() => Config.load(file)).not.toThrow();
  });

  // -- reload() participates --------------------------------------------------

  it('reload() re-runs the declared check when the config was loaded with validation on', () => {
    const file = writeConfig('version: "1.0.0"\nproject:\n  name: ok\n');
    const config = Config.load(file);
    // Drop `version` from the file, then reload.
    fs.writeFileSync(file, 'project:\n  name: ok\n', 'utf-8');
    try {
      config.reload();
      throw new Error('expected reload to throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ConfigError);
      expect((e as ConfigError).code).toBe('CONFIG_INVALID');
      expect((e as ConfigError).message).toContain("'version'");
    }
  });

  it('reload() honours an explicit validate:false from the original load', () => {
    const file = writeConfig('executor:\n  default_timeout: 1\n');
    const config = Config.load(file, { validate: false });
    fs.writeFileSync(file, 'executor:\n  default_timeout: 2\n', 'utf-8');
    expect(() => config.reload()).not.toThrow();
    expect(config.get('executor.default_timeout')).toBe(2);
  });

  // -- Namespace mode is unaffected -------------------------------------------

  it('namespace mode does not run the required-field check (apcore: is metadata)', () => {
    const file = writeConfig('apcore:\n  executor:\n    default_timeout: 5000\n');
    expect(() => Config.load(file)).not.toThrow();
  });
});
