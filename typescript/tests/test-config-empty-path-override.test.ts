/**
 * PROTOCOL_SPEC §9.2.1 requirement 5 — **an empty string is not a path**
 * (apcore#115).
 *
 * §9.2 treats a *set but empty* `APCORE_*` variable as an override like any
 * other, and `export APCORE_ACL_ROOT=` or a variable inherited empty from a
 * container spec is "set" as far as the tooling is concerned. Unguarded, that
 * empty string wins the top precedence tier and silently blanks a directory the
 * configuration file correctly declared — and `''` then resolves against the
 * working directory, which *is* the working directory. It is a legal relative
 * path to every filesystem API and never the one an operator meant.
 *
 * The guard lives in `applyEnvOverrides`, where the override is APPLIED, rather
 * than at each consumer: it covers every key in §9.2.1's closed set at once, so
 * a key added to that set later needs no consumer to remember it. The suite
 * therefore drives the set through `Config.pathTypedKeys()` instead of a
 * hand-written list.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { BindingLoader } from '../src/bindings.js';
import { Config } from '../src/config.js';
import { Registry } from '../src/registry/registry.js';

const MINIMAL_YAML = 'version: "0.29.0"\nproject:\n  name: empty-path-override-test\n';

let tmpDir: string;
let originalCwd: string;

/**
 * The `APCORE_*` variable that carries a dot-path key, per §9.2's convention.
 *
 * `extensions.roots[]` has none: §9.2.1 requirement 3 forbids inventing a
 * delimiter-separated `APCORE_EXTENSIONS_ROOTS` encoding for the list-valued
 * key, so it is excluded from the scalar sweep below rather than guessed at.
 */
function envVarFor(dotKey: string): string {
  return `APCORE_${dotKey.toUpperCase().replace(/\./g, '_')}`;
}

/** The §9.2.1 keys an environment variable can actually spell. */
function scalarPathTypedKeys(): string[] {
  return Config.pathTypedKeys().filter((key) => !key.endsWith('[]'));
}

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), 'apcore-empty-path-'));
  originalCwd = process.cwd();
  process.chdir(tmpDir);
  vi.stubEnv('APCORE_CONFIG_FILE', undefined);
  vi.stubEnv('HOME', join(tmpDir, 'nonexistent-home'));
  for (const key of scalarPathTypedKeys()) {
    vi.stubEnv(envVarFor(key), undefined);
  }
});

afterEach(() => {
  process.chdir(originalCwd);
  rmSync(tmpDir, { recursive: true, force: true });
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

/** A config file declaring a real directory for every scalar path-typed key. */
function writeConfigDeclaringEveryPathKey(): string {
  const declared = Object.fromEntries(
    scalarPathTypedKeys().map((key) => [key, join(tmpDir, `declared-${key.replace('.', '-')}`)]),
  );
  const filePath = join(tmpDir, 'apcore.yaml');
  writeFileSync(
    filePath,
    [
      MINIMAL_YAML,
      `acl:\n  root: "${declared['acl.root']}"`,
      `bindings:\n  dir: "${declared['bindings.dir']}"`,
      `extensions:\n  root: "${declared['extensions.root']}"`,
      `schema:\n  root: "${declared['schema.root']}"`,
      `id_map:\n  overrides: "${declared['id_map.overrides']}"`,
      `sys_modules:\n  control:\n    overrides_path: "${declared['sys_modules.control.overrides_path']}"`,
      '',
    ].join('\n'),
    'utf-8',
  );
  return filePath;
}

describe('an empty APCORE_* override of a path-typed key is discarded (§9.2.1 req 5)', () => {
  it('falls through to the configuration file for EVERY path-typed key', () => {
    // The whole closed set at once, read from the public accessor: a key added
    // to §9.2.1 later joins this sweep without anyone editing it.
    const configPath = writeConfigDeclaringEveryPathKey();
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    for (const key of scalarPathTypedKeys()) {
      vi.stubEnv(envVarFor(key), '');
    }

    const config = Config.load(configPath);

    for (const key of scalarPathTypedKeys()) {
      expect(config.get(key), `${key} was blanked by its empty override`).toBe(
        join(tmpDir, `declared-${key.replace('.', '-')}`),
      );
      expect(config.get(key)).not.toBe('');
    }
  });

  it('falls through to the §9.1.1 default when no file declares the key', () => {
    // The second tier down. Without the guard `bindings.dir` resolves to `''`,
    // which the filesystem reads as the working directory — the failure §9.2.1
    // calls silent rather than loud.
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.stubEnv('APCORE_BINDINGS_DIR', '');
    vi.stubEnv('APCORE_ACL_ROOT', '');
    vi.stubEnv('APCORE_SCHEMA_ROOT', '');
    vi.stubEnv('APCORE_EXTENSIONS_ROOT', '');

    const config = Config.fromDefaults();

    expect(config.get('bindings.dir')).toBe('./bindings');
    expect(config.get('acl.root')).toBe('./acl');
    expect(config.get('schema.root')).toBe('./schemas');
    expect(config.get('extensions.root')).toBe('./extensions');
  });

  it('leaves a NON path-typed key alone — an empty string is a legal value there', () => {
    // The guard is scoped to §9.2.1's set, not to empty strings generally.
    // `bindings.pattern` is the deliberate near-miss: it sits in the same
    // section and is explicitly NOT path-typed (requirement 4).
    vi.stubEnv('APCORE_BINDINGS_PATTERN', '');
    vi.stubEnv('APCORE_PROJECT_NAME', '');

    const config = Config.fromDefaults();

    expect(config.get('bindings.pattern')).toBe('');
    expect(config.get('project.name')).toBe('');
  });

  it('logs a warning naming the key it discarded (§9.2.1 req 5, the MAY)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.stubEnv('APCORE_ACL_ROOT', '');

    Config.fromDefaults();

    const lines = warn.mock.calls.map((call) => String(call[0]));
    const notice = lines.find((line) => line.includes('PROTOCOL_SPEC §9.2.1'));
    expect(notice, 'no §9.2.1 notice was emitted').toBeDefined();
    expect(notice).toContain('APCORE_ACL_ROOT');
    expect(notice).toContain('acl.root');
  });

  it('keeps the binding loader on the configured directory, not the CWD', () => {
    // The consumer-visible half. `loadBindingDir` reads `bindings.dir` off the
    // merged Config (§5.12.6 clause 2), so an empty override that survived the
    // merge would send it to `''` — the working directory — instead of the
    // directory the file declares.
    const bindDir = join(tmpDir, 'declared-bindings');
    mkdirSync(bindDir);
    const configPath = join(tmpDir, 'apcore.yaml');
    writeFileSync(configPath, `${MINIMAL_YAML}bindings:\n  dir: "${bindDir}"\n`, 'utf-8');
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.stubEnv('APCORE_BINDINGS_DIR', '');

    const config = Config.load(configPath);

    expect(config.get('bindings.dir')).toBe(bindDir);
    // Reached through the loader's own resolution, not just the config getter.
    const loader = new BindingLoader();
    return expect(
      loader.loadBindingDir(undefined, new Registry(), undefined, config),
    ).resolves.toEqual([]);
  });
});
