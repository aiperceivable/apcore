/** Unit coverage for stable project roots and configuration consumers. */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { ACL } from '../src/acl.js';
import { Config, discoverConfigFile, userLevelConfigPaths } from '../src/config.js';
import { SchemaLoader } from '../src/schema/loader.js';

const MINIMAL_YAML = 'version: "1.0.0"\nproject:\n  name: project-root-test\n';

let tmpDir: string;
let projectDir: string;
let elsewhereDir: string;
let fakeHome: string;
let originalCwd: string;

beforeEach(() => {
  tmpDir = realpathSync(mkdtempSync(join(tmpdir(), 'apcore-project-root-')));
  projectDir = join(tmpDir, 'project');
  elsewhereDir = join(tmpDir, 'elsewhere');
  fakeHome = join(tmpDir, 'home');
  mkdirSync(projectDir);
  mkdirSync(elsewhereDir);
  mkdirSync(fakeHome);
  originalCwd = process.cwd();
  vi.stubEnv('APCORE_CONFIG_FILE', undefined);
  vi.stubEnv('HOME', fakeHome);
});

afterEach(() => {
  process.chdir(originalCwd);
  rmSync(tmpDir, { recursive: true, force: true });
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

function write(filePath: string, body = MINIMAL_YAML): string {
  mkdirSync(dirname(filePath), { recursive: true });
  writeFileSync(filePath, body, 'utf-8');
  return filePath;
}

/**
 * The tier 6 (XDG) user-level path, spelled out here rather than taken from
 * `userLevelConfigPaths()` so the assertion does not agree with the code by
 * construction.
 */
function xdgConfigPath(home: string): string {
  return process.platform === 'darwin'
    ? join(home, 'Library', 'Application Support', 'apcore', 'config.yaml')
    : join(home, '.config', 'apcore', 'config.yaml');
}

/** The tier 7 (legacy) user-level path. */
function legacyConfigPath(home: string): string {
  return join(home, '.apcore', 'config.yaml');
}

describe('Config.projectRoot — one case per §9.14 discovery tier', () => {
  it('tier 1: $APCORE_CONFIG_FILE outside the CWD roots at the config file directory', () => {
    const configPath = write(join(elsewhereDir, 'custom.yaml'));
    process.chdir(projectDir);
    vi.stubEnv('APCORE_CONFIG_FILE', configPath);

    expect(discoverConfigFile()).toBe(configPath);
    expect(Config.discover().projectRoot).toBe(elsewhereDir);
    expect(process.cwd()).not.toBe(elsewhereDir);
  });

  it('tier 1: a path passed straight to Config.load is equally "explicitly pointed at"', () => {
    const configPath = write(join(elsewhereDir, 'custom.yaml'));
    process.chdir(projectDir);

    expect(Config.load(configPath).projectRoot).toBe(elsewhereDir);
  });

  it.each(['project.yaml', 'project.yml', 'apcore.yaml', 'apcore.yml'])(
    'tiers 2-5: a project-local ./%s roots at the CWD',
    (fileName) => {
      write(join(projectDir, fileName));
      process.chdir(projectDir);

      const config = Config.discover();
      expect(config.sourcePath).toBe(fileName);
      expect(config.projectRoot).toBe(process.cwd());
    },
  );

  it('tier 6: a user-level XDG config roots at the CWD, not next to the config file', () => {
    // User-level discovery applies paths to the project running the SDK.
    const configPath = write(xdgConfigPath(fakeHome));
    process.chdir(projectDir);

    const config = Config.discover();
    expect(config.sourcePath).toBe(configPath);
    expect(config.projectRoot).toBe(process.cwd());
    expect(config.projectRoot).not.toBe(dirname(configPath));
  });

  it('tier 7: the legacy ~/.apcore config roots at the CWD too', () => {
    const configPath = write(legacyConfigPath(fakeHome));
    process.chdir(projectDir);

    const config = Config.discover();
    expect(config.sourcePath).toBe(configPath);
    expect(config.projectRoot).toBe(process.cwd());
  });

  it('loads a home configuration as an explicit file when selected by its path', () => {
    const configPath = write(xdgConfigPath(fakeHome));
    process.chdir(projectDir);

    expect(Config.load(configPath).projectRoot).toBe(dirname(configPath));
    vi.stubEnv('APCORE_CONFIG_FILE', configPath);
    expect(Config.discover().projectRoot).toBe(dirname(configPath));
  });

  it('keeps the project root and YAML source stable across chdir and reload', () => {
    write(join(projectDir, 'apcore.yaml'));
    process.chdir(projectDir);
    const config = Config.load('apcore.yaml');

    process.chdir(elsewhereDir);
    expect(config.projectRoot).toBe(projectDir);
    write(join(projectDir, 'apcore.yaml'), `${MINIMAL_YAML}schema: {root: ./updated}\n`);
    config.reload();

    expect(config.sourcePath).toBe('apcore.yaml');
    expect(config.projectRoot).toBe(projectDir);
    expect(config.get('schema.root')).toBe('./updated');
  });

  it('reloads a selected symlink using the link directory as project root', () => {
    const target = write(join(elsewhereDir, 'target.yaml'));
    symlinkSync(target, join(projectDir, 'apcore.yaml'), 'file');
    process.chdir(projectDir);
    const config = Config.discover();

    write(target, `${MINIMAL_YAML}schema: {root: ./updated}\n`);
    process.chdir(elsewhereDir);
    config.reload();

    expect(config.projectRoot).toBe(projectDir);
    expect(config.resolvePath(config.get('schema.root') as string)).toBe(join(projectDir, 'updated'));
  });

  it('no config file: fromDefaults and a bare Config root at the CWD', () => {
    process.chdir(projectDir);

    expect(discoverConfigFile()).toBeNull();
    expect(Config.discover().projectRoot).toBe(process.cwd());
    expect(Config.fromDefaults().projectRoot).toBe(process.cwd());
    expect(new Config({}).projectRoot).toBe(process.cwd());
  });

  it('is always absolute, even for a relative sourcePath', () => {
    write(join(projectDir, 'apcore.yaml'));
    process.chdir(projectDir);

    const config = Config.load('apcore.yaml');
    expect(config.sourcePath).toBe('apcore.yaml');
    expect(config.projectRoot.startsWith('/')).toBe(true);
  });

  it('userLevelConfigPaths lists tier 6 then tier 7', () => {
    expect(userLevelConfigPaths()).toEqual([xdgConfigPath(fakeHome), legacyConfigPath(fakeHome)]);
  });
});

describe('project-root path resolution', () => {
  it.each([false, true])('keeps numeric path env names in namespace mode %s', (namespace) => {
    const yaml = namespace ? 'apcore:\n  version: "1.0.0"\n  project: {name: probe}\n' : MINIMAL_YAML;
    write(join(projectDir, 'config.yaml'), yaml);
    write(join(projectDir, '123', 'probe.schema.yaml'), 'description: numeric path\ninput_schema: {}\noutput_schema: {}\n');
    vi.stubEnv('APCORE_SCHEMA_ROOT', '123');

    const config = Config.load(join(projectDir, 'config.yaml'));
    expect(config.get('schema.root')).toBe('123');
    expect(new SchemaLoader(config).load('probe').description).toBe('numeric path');
  });

  it.each(['ids.yaml/', 'ids.yaml/.'])('retains a directory qualifier in %s', (relative) => {
    write(join(projectDir, 'ids.yaml'));
    const config = Config.load(write(join(projectDir, 'config.yaml')));
    expect(config.resolvePath(relative)).toBe(`${projectDir}/${relative}`);
  });

  it('rejects a missing yaml_only schema root relative to the config file', () => {
    const file = write(join(projectDir, 'config.yaml'), `${MINIMAL_YAML}schema: {strategy: yaml_only, root: ./missing}\n`);
    mkdirSync(join(elsewhereDir, 'missing'));
    process.chdir(elsewhereDir);
    expect(() => Config.load(file)).toThrow('schema.root');
  });

  it('keeps validated state when a reload fails', () => {
    const file = write(join(projectDir, 'config.yaml'), `${MINIMAL_YAML}executor: {default_timeout: 50}\n`);
    const config = Config.load(file);
    write(file, `${MINIMAL_YAML}executor: {default_timeout: -1}\n`);

    expect(() => config.reload()).toThrow('executor.default_timeout');
    expect(config.get('executor.default_timeout')).toBe(50);
    expect(config.getDeclared('executor.default_timeout')).toBe(50);
  });

  it('updates the same unqualified apcore key it reads in namespace mode', () => {
    const file = write(join(projectDir, 'config.yaml'),
      'apcore:\n  version: "1.0.0"\n  project: {name: probe}\n  executor: {default_timeout: 50}\n_config: {strict: true}\n');
    const config = Config.load(file);
    expect(config.get('executor.default_timeout')).toBe(50);
    config.set('executor.default_timeout', 75);

    expect(config.get('apcore.executor.default_timeout')).toBe(75);
    expect(config.getDeclared('apcore.executor.default_timeout')).toBe(75);
    expect(() => config.validate()).not.toThrow();
  });

  it('checks explicit field types and namespace redaction regexes', () => {
    const file = write(join(projectDir, 'config.yaml'), 'version: 1\nproject: {name: [one]}\n');
    expect(() => Config.load(file)).toThrow(/version.*string/s);
    write(file, 'apcore:\n  version: "1.0.0"\n  project: {name: probe}\nobs:\n  redaction: {regex_patterns: ["["]}\n');
    expect(() => Config.load(file)).toThrow('regex_patterns');
  });

  it.each(['relative', 'absolute'])('uses filesystem traversal for %s paths containing a symlink and parent segment', (kind) => {
    const deep = join(elsewhereDir, 'deep');
    mkdirSync(deep);
    symlinkSync(deep, join(projectDir, 'link'), 'dir');
    for (const [base, marker] of [[projectDir, 'project'], [elsewhereDir, 'physical']]) {
      write(join(base, 'probe', 'global_acl.yaml'), [
        'default_effect: deny',
        'audit: {enabled: false, include_denied: true}',
        'rules:',
        '  - effect: deny',
        '    callers: ["@external"]',
        '    targets: [probe]',
        `    description: ${marker}`,
      ].join('\n'));
      write(join(base, 'probe', 'probe.schema.yaml'), `description: ${marker}\ninput_schema: {}\noutput_schema: {}\n`);
    }
    process.chdir(projectDir);
    const relative = 'link/../probe';
    const absolute = `${projectDir}/${relative}`;
    const path = kind === 'absolute' ? absolute : relative;
    const config = new Config({ acl: { root: path }, schema: { root: path } });

    expect(config.resolvePath(path)).toBe(absolute);
    expect(ACL.discover(config)?.rules[0]?.description).toBe('physical');
    expect(new SchemaLoader(config).load('probe').description).toBe('physical');
  });

  it('loads SchemaLoader input from the config project outside CWD', () => {
    write(join(elsewhereDir, 'custom.yaml'), `${MINIMAL_YAML}schema:\n  root: "./schemas"\n`);
    write(join(elsewhereDir, 'schemas', 'probe.schema.yaml'), 'description: config root\ninput_schema: {}\noutput_schema: {}\n');
    write(join(projectDir, 'schemas', 'probe.schema.yaml'), 'description: CWD\ninput_schema: {}\noutput_schema: {}\n');
    process.chdir(projectDir);
    vi.spyOn(console, 'warn').mockImplementation(() => {});

    const config = Config.load(join(elsewhereDir, 'custom.yaml'));
    const loader = new SchemaLoader(config);

    expect(config.projectRoot).toBe(elsewhereDir);
    expect(loader.load('probe').description).toBe('config root');
  });

  it('uses the default directories for empty programmatic path values', () => {
    process.chdir(projectDir);
    const config = new Config({ acl: { root: '' }, schema: { root: '' } });
    write(join(projectDir, 'global_acl.yaml'), 'default_effect: deny\nrules: []\n');
    write(join(projectDir, 'acl', 'global_acl.yaml'), [
      'default_effect: deny',
      'rules:',
      '  - effect: deny',
      '    callers: ["@external"]',
      '    targets: [probe]',
      '    description: default directory',
    ].join('\n'));
    write(join(projectDir, 'probe.schema.yaml'), 'description: project root\ninput_schema: {}\noutput_schema: {}\n');
    write(join(projectDir, 'schemas', 'probe.schema.yaml'), 'description: default directory\ninput_schema: {}\noutput_schema: {}\n');

    expect(ACL.discover(config)?.rules[0]?.description).toBe('default directory');
    expect(new SchemaLoader(config).load('probe').description).toBe('default directory');
  });
});
