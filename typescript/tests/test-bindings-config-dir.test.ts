/**
 * `bindings.dir` / `bindings.pattern` reach the binding loader (apcore#114,
 * apcore-typescript#36, PROTOCOL_SPEC §5.12.6).
 *
 * Every pre-existing `loadBindingDir` test passes an explicit directory
 * argument, which is the one path that behaves identically before and after
 * the fix. The cases here are the discriminating ones: a directory supplied by
 * a config *file*, the `./bindings` default, and the deleted raw
 * `process.env.APCORE_BINDINGS_DIR` read.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { BindingLoader } from '../src/bindings.js';
import { Config } from '../src/config.js';
import { BindingFileInvalidError } from '../src/errors.js';
import { Registry } from '../src/registry/registry.js';

let tmpDir: string;
let originalCwd: string;
let loader: BindingLoader;
let registry: Registry;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), 'apcore-bindings-config-'));
  originalCwd = process.cwd();
  loader = new BindingLoader();
  registry = new Registry();
  // Neither variable may leak in from the ambient environment: this suite is
  // about which tier a value arrives through. Deleted rather than blanked —
  // `APCORE_BINDINGS_DIR=''` is itself a §9.2 override and would blank the
  // value a config file declares.
  vi.stubEnv('APCORE_CONFIG_FILE', undefined);
  vi.stubEnv('APCORE_BINDINGS_DIR', undefined);
  vi.stubEnv('HOME', join(tmpDir, 'nonexistent-home'));
  // The project-root deprecation notice (apcore#113) is expected in most of
  // these cases — the config files live outside the CWD — and is not what this
  // suite is testing.
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

afterEach(() => {
  process.chdir(originalCwd);
  rmSync(tmpDir, { recursive: true, force: true });
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

/** An importable ESM module exposing one no-argument function per name. */
function writeModule(name: string, fns: string[]): string {
  const filePath = join(tmpDir, name);
  const body = fns.map((fn) => `export function ${fn}() { return { ok: '${fn}' }; }`).join('\n');
  writeFileSync(filePath, `${body}\nexport const inputSchema = {type:'object'};\nexport const outputSchema = {type:'object'};\n`, 'utf-8');
  return filePath;
}

/** A binding file declaring one module, written under `dir`. */
function writeBinding(dir: string, fileName: string, moduleId: string, target: string): void {
  writeFileSync(
    join(dir, fileName),
    `bindings:\n  - module_id: "${moduleId}"\n    target: "${target}"\n`,
    'utf-8',
  );
}

/** A minimal valid config file; `extra` is appended verbatim. */
function writeConfig(dir: string, fileName: string, extra: string): string {
  const filePath = join(dir, fileName);
  writeFileSync(
    filePath,
    `version: "0.29.0"\nproject:\n  name: bindings-config-test\n${extra}`,
    'utf-8',
  );
  return filePath;
}

describe('loadBindingDir resolves the directory from bindings.dir', () => {
  it('scans the directory a config FILE declares, with APCORE_BINDINGS_DIR unset', async () => {
    // The discriminating case from apcore#114 and apcore-typescript#36. Before
    // the fix this threw: no explicit argument and no environment variable.
    const bindDir = join(tmpDir, 'declared-bindings');
    mkdirSync(bindDir);
    const modPath = writeModule('file_mod.mjs', ['alpha']);
    writeBinding(bindDir, 'alpha.binding.yaml', 'cfgfile.alpha', `${modPath}:alpha`);

    const configPath = writeConfig(tmpDir, 'apcore.yaml', `bindings:\n  dir: "${bindDir}"\n`);
    const config = Config.load(configPath);
    expect(config.get('bindings.dir')).toBe(bindDir);

    const results = await loader.loadBindingDir(undefined, registry, undefined, config);

    expect(results).toHaveLength(1);
    expect(registry.has('cfgfile.alpha')).toBe(true);
  });

  it('keeps APCORE_BINDINGS_DIR working, through the §9.2 merge rather than a raw read', async () => {
    // Back-compat guarantee for apcore-typescript#36: the environment tier
    // still reaches the loader with no config file present at all, because
    // `applyEnvOverrides` lowers the variable into `bindings.dir`.
    const bindDir = join(tmpDir, 'env-bindings');
    mkdirSync(bindDir);
    const modPath = writeModule('env_mod.mjs', ['beta']);
    writeBinding(bindDir, 'beta.binding.yaml', 'envtier.beta', `${modPath}:beta`);

    process.chdir(tmpDir);
    vi.stubEnv('APCORE_BINDINGS_DIR', bindDir);

    // No config file exists: `discover()` falls through to `fromDefaults()`,
    // which still applies environment overrides.
    const config = Config.discover({ validate: false });
    expect(config.sourcePath).toBeNull();
    expect(config.get('bindings.dir')).toBe(bindDir);

    const results = await loader.loadBindingDir(undefined, registry, undefined, config);

    expect(results).toHaveLength(1);
    expect(registry.has('envtier.beta')).toBe(true);
  });

  it('lets a config file override an environment variable only per §9.2 precedence', async () => {
    // Sanity check on the direction of the chain: env beats file, so the
    // scanned directory is the environment one even though the file names
    // another.
    const envDir = join(tmpDir, 'env-wins');
    const fileDir = join(tmpDir, 'file-loses');
    mkdirSync(envDir);
    mkdirSync(fileDir);
    const modPath = writeModule('prec_mod.mjs', ['fromEnv', 'fromFile']);
    writeBinding(envDir, 'e.binding.yaml', 'prec.from_env', `${modPath}:fromEnv`);
    writeBinding(fileDir, 'f.binding.yaml', 'prec.from_file', `${modPath}:fromFile`);

    vi.stubEnv('APCORE_BINDINGS_DIR', envDir);
    const configPath = writeConfig(tmpDir, 'apcore.yaml', `bindings:\n  dir: "${fileDir}"\n`);
    const config = Config.load(configPath);

    await loader.loadBindingDir(undefined, registry, undefined, config);

    expect(registry.has('prec.from_env')).toBe(true);
    expect(registry.has('prec.from_file')).toBe(false);
  });

  it('an explicit directory argument still wins over the config', async () => {
    const explicitDir = join(tmpDir, 'explicit');
    const configuredDir = join(tmpDir, 'configured');
    mkdirSync(explicitDir);
    mkdirSync(configuredDir);
    const modPath = writeModule('arg_mod.mjs', ['fromArg', 'fromCfg']);
    writeBinding(explicitDir, 'a.binding.yaml', 'arg.from_arg', `${modPath}:fromArg`);
    writeBinding(configuredDir, 'c.binding.yaml', 'arg.from_cfg', `${modPath}:fromCfg`);

    const configPath = writeConfig(tmpDir, 'apcore.yaml', `bindings:\n  dir: "${configuredDir}"\n`);
    const config = Config.load(configPath);

    await loader.loadBindingDir(explicitDir, registry, undefined, config);

    expect(registry.has('arg.from_arg')).toBe(true);
    expect(registry.has('arg.from_cfg')).toBe(false);
  });

  it('falls back to ./bindings when neither an argument nor a config supplies one', async () => {
    // PROTOCOL_SPEC §5.12.6: "If neither is configured, implementations SHOULD
    // default to scanning bindings/". Before the fix this threw instead.
    const bindDir = join(tmpDir, 'bindings');
    mkdirSync(bindDir);
    const modPath = writeModule('default_mod.mjs', ['gamma']);
    writeBinding(bindDir, 'gamma.binding.yaml', 'defaultdir.gamma', `${modPath}:gamma`);

    process.chdir(tmpDir);
    const results = await loader.loadBindingDir(undefined, registry);

    expect(results).toHaveLength(1);
    expect(registry.has('defaultdir.gamma')).toBe(true);
  });

  it('never reads APCORE_BINDINGS_DIR itself — an unconfigured loader still uses ./bindings', async () => {
    // §5.12.6 clause 2: "An implementation MUST NOT read APCORE_BINDINGS_DIR
    // directly at the loader". With the variable set and no Config handed
    // over, the default must win — the loader has no second channel.
    const defaultDir = join(tmpDir, 'bindings');
    const envDir = join(tmpDir, 'env-only');
    mkdirSync(defaultDir);
    mkdirSync(envDir);
    const modPath = writeModule('raw_mod.mjs', ['fromDefault', 'fromRawEnv']);
    writeBinding(defaultDir, 'd.binding.yaml', 'raw.from_default', `${modPath}:fromDefault`);
    writeBinding(envDir, 'r.binding.yaml', 'raw.from_raw_env', `${modPath}:fromRawEnv`);

    process.chdir(tmpDir);
    vi.stubEnv('APCORE_BINDINGS_DIR', envDir);

    await loader.loadBindingDir(undefined, registry);

    expect(registry.has('raw.from_default')).toBe(true);
    expect(registry.has('raw.from_raw_env')).toBe(false);
  });

  it('reports the resolved default path when ./bindings does not exist', async () => {
    process.chdir(tmpDir);
    const err = await loader.loadBindingDir(undefined, registry).catch((e: unknown) => e);

    expect(err).toBeInstanceOf(BindingFileInvalidError);
    expect((err as BindingFileInvalidError).code).toBe('BINDING_FILE_INVALID');
    expect(String((err as BindingFileInvalidError).message)).toContain('./bindings');
  });
});

describe('loadBindingDir resolves the pattern from bindings.pattern', () => {
  it('matches files against the pattern a config declares', async () => {
    const bindDir = join(tmpDir, 'patterned');
    mkdirSync(bindDir);
    const modPath = writeModule('pat_mod.mjs', ['matched', 'ignored']);
    writeBinding(bindDir, 'yes.bind.yaml', 'pat.matched', `${modPath}:matched`);
    writeBinding(bindDir, 'no.binding.yaml', 'pat.ignored', `${modPath}:ignored`);

    const configPath = writeConfig(
      tmpDir,
      'apcore.yaml',
      `bindings:\n  dir: "${bindDir}"\n  pattern: "*.bind.yaml"\n`,
    );
    const config = Config.load(configPath);

    const results = await loader.loadBindingDir(undefined, registry, undefined, config);

    expect(results).toHaveLength(1);
    expect(registry.has('pat.matched')).toBe(true);
    expect(registry.has('pat.ignored')).toBe(false);
  });

  it('an explicit pattern argument wins over the configured one', async () => {
    const bindDir = join(tmpDir, 'patterned-arg');
    mkdirSync(bindDir);
    const modPath = writeModule('pat_arg_mod.mjs', ['viaArg', 'viaCfg']);
    writeBinding(bindDir, 'x.arg.yaml', 'patarg.via_arg', `${modPath}:viaArg`);
    writeBinding(bindDir, 'y.bind.yaml', 'patarg.via_cfg', `${modPath}:viaCfg`);

    const configPath = writeConfig(
      tmpDir,
      'apcore.yaml',
      `bindings:\n  dir: "${bindDir}"\n  pattern: "*.bind.yaml"\n`,
    );
    const config = Config.load(configPath);

    await loader.loadBindingDir(undefined, registry, '*.arg.yaml', config);

    expect(registry.has('patarg.via_arg')).toBe(true);
    expect(registry.has('patarg.via_cfg')).toBe(false);
  });

  it('defaults to *.binding.yaml when no pattern is configured', async () => {
    const bindDir = join(tmpDir, 'pattern-default');
    mkdirSync(bindDir);
    const modPath = writeModule('pat_def_mod.mjs', ['kept', 'skipped']);
    writeBinding(bindDir, 'k.binding.yaml', 'patdef.kept', `${modPath}:kept`);
    writeBinding(bindDir, 's.other.yaml', 'patdef.skipped', `${modPath}:skipped`);

    const configPath = writeConfig(tmpDir, 'apcore.yaml', `bindings:\n  dir: "${bindDir}"\n`);
    const config = Config.load(configPath);

    const results = await loader.loadBindingDir(undefined, registry, undefined, config);

    expect(results).toHaveLength(1);
    expect(registry.has('patdef.kept')).toBe(true);
  });
});

describe('a missing resolved directory raises, naming it (§5.12.6 clause 5)', () => {
  // Clause 5 (spec v1.36.0) holds for all three provenances of the resolved
  // directory — "whether the directory came from an explicit argument, from
  // `bindings.dir`, or from the `./bindings` default" — and requires the
  // message to NAME the directory, so an operator can see which one was
  // resolved. The default provenance is pinned above; these are the other two.
  // Contrast ACL.discover (D-64), where a missing `acl.root` attaches nothing:
  // discovery is automatic, binding loading is user-invoked.

  it('names the directory when it came from an explicit argument', async () => {
    const missing = join(tmpDir, 'no-such-explicit-dir');

    const err = await loader.loadBindingDir(missing, registry).catch((e: unknown) => e);

    expect(err).toBeInstanceOf(BindingFileInvalidError);
    expect((err as BindingFileInvalidError).code).toBe('BINDING_FILE_INVALID');
    expect(String((err as BindingFileInvalidError).message)).toContain(missing);
    expect(registry.list()).toHaveLength(0);
  });

  it('names the directory when it came from bindings.dir', async () => {
    const missing = join(tmpDir, 'no-such-configured-dir');
    const configPath = writeConfig(tmpDir, 'apcore.yaml', `bindings:\n  dir: "${missing}"\n`);
    const config = Config.load(configPath);

    const err = await loader
      .loadBindingDir(undefined, registry, undefined, config)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(BindingFileInvalidError);
    expect((err as BindingFileInvalidError).code).toBe('BINDING_FILE_INVALID');
    expect(String((err as BindingFileInvalidError).message)).toContain(missing);
    expect(registry.list()).toHaveLength(0);
  });

  it('does NOT return an empty result — the absence is an error, not a no-op', async () => {
    // The half clause 5 exists to forbid: returning zero modules silently
    // reproduces the "configuration key that quietly does nothing" defect
    // §5.12.6 was rewritten to remove.
    process.chdir(tmpDir);
    await expect(loader.loadBindingDir(undefined, registry)).rejects.toBeInstanceOf(
      BindingFileInvalidError,
    );
  });
});
