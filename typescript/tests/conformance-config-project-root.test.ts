/** Cross-language project-root and path consumer cases for PROTOCOL_SPEC §9.2.2. */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { ACL } from '../src/acl.js';
import { Config, getDefault } from '../src/config.js';
import { SchemaLoader } from '../src/schema/loader.js';
import { findFixturesRoot } from './spec-repo.js';

interface ProjectRootCase {
  readonly id: string;
  readonly cwd: string;
  readonly env?: Record<string, string>;
  readonly absolute_env?: Record<string, string>;
  readonly fs?: Record<string, string>;
  readonly symlinks?: Record<string, string>;
  readonly config_from_mapping?: Record<string, unknown>;
  readonly config_path?: string;
  readonly consume_from?: string;
  readonly expected: {
    readonly config_source_dir: string | null;
    readonly project_root: string;
    readonly project_root_equals_cwd: boolean;
    readonly resolved_config_paths: Record<string, string | string[]>;
    readonly resolved_acl_root?: string;
    readonly resolved_schema_root?: string;
  };
}

interface ProjectRootFixture {
  readonly layout: {
    readonly dirs: readonly string[];
    readonly home: string;
  };
  readonly test_cases: readonly ProjectRootCase[];
}

const fixture: ProjectRootFixture = JSON.parse(
  readFileSync(join(findFixturesRoot(), 'config_project_root.json'), 'utf-8'),
);
const home = fixture.layout.home;
const tier6Config = process.platform === 'darwin'
  ? `${home}/Library/Application Support/apcore/config.yaml`
  : `${home}/.config/apcore/config.yaml`;
const tier7Config = `${home}/.apcore/config.yaml`;
const homeTokens: Record<string, string> = {
  '<tier6_config>': tier6Config,
  '<tier6_dir>': dirname(tier6Config),
  '<tier7_config>': tier7Config,
  '<tier7_dir>': dirname(tier7Config),
};

let root: string;
let originalCwd: string;

function layoutRelative(path: string): string {
  return path.replace(/^<[^>]+>/, (token) => {
    const replacement = homeTokens[token];
    if (replacement === undefined) throw new Error(`Unknown fixture token '${token}'`);
    return replacement;
  });
}

function layoutPath(path: string): string {
  return join(root, layoutRelative(path));
}

/** Preserve dot segments when testing absolute values passed to Config. */
function absoluteSpelling(path: string): string {
  return `${root}/${layoutRelative(path)}`;
}

beforeEach(() => {
  root = realpathSync(mkdtempSync(join(tmpdir(), 'apcore-project-root-conf-')));
  originalCwd = process.cwd();
  for (const dir of fixture.layout.dirs) mkdirSync(layoutPath(dir), { recursive: true });
  for (const key of Object.keys(process.env)) {
    if (key.startsWith('APCORE_')) vi.stubEnv(key, undefined);
  }
  vi.stubEnv('HOME', layoutPath(home));
  vi.stubEnv('USERPROFILE', layoutPath(home));
  vi.stubEnv('XDG_CONFIG_HOME', layoutPath(`${home}/.config`));
});

afterEach(() => {
  process.chdir(originalCwd);
  rmSync(root, { recursive: true, force: true });
  vi.unstubAllEnvs();
});

function loadConfig(testCase: ProjectRootCase): Config {
  for (const [relative, content] of Object.entries(testCase.fs ?? {})) {
    const target = layoutPath(relative);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, content, 'utf-8');
  }
  for (const [relative, target] of Object.entries(testCase.symlinks ?? {})) {
    const link = layoutPath(relative);
    mkdirSync(dirname(link), { recursive: true });
    const resolvedTarget = layoutPath(target);
    symlinkSync(resolvedTarget, link, statSync(resolvedTarget).isDirectory() ? 'dir' : 'file');
  }
  for (const [name, value] of Object.entries(testCase.env ?? {})) {
    vi.stubEnv(name, name === 'APCORE_CONFIG_FILE' ? layoutPath(value) : value);
  }
  for (const [name, value] of Object.entries(testCase.absolute_env ?? {})) {
    vi.stubEnv(name, absoluteSpelling(value));
  }
  if (testCase.config_from_mapping !== undefined) return new Config(testCase.config_from_mapping);
  if (testCase.config_path !== undefined) return Config.load(layoutPath(testCase.config_path), { validate: false });
  return Config.discover({ validate: false });
}

function assertConfigPaths(config: Config, testCase: ProjectRootCase): void {
  const verbatim = new Set(Object.values(testCase.absolute_env ?? {}));
  const expectedPath = (path: string): string => verbatim.has(path) ? absoluteSpelling(path) : layoutPath(path);
  for (const [key, expected] of Object.entries(testCase.expected.resolved_config_paths)) {
    if (key === 'extensions.roots[]') {
      const entries = config.get('extensions.roots') as Array<string | { root: string }>;
      const paths = entries.map((entry) => config.resolvePath(typeof entry === 'string' ? entry : entry.root));
      expect(paths, key).toEqual((expected as string[]).map(expectedPath));
    } else {
      const value = config.get(key, getDefault(key));
      expect(typeof value, key).toBe('string');
      expect(config.resolvePath(value as string), key).toBe(expectedPath(expected as string));
    }
  }
}

function assertConsumers(config: Config, expected: ProjectRootCase['expected']): void {
  if (expected.resolved_acl_root !== undefined) {
    expect(ACL.discover(config)?.rules[0]?.description).toBe(expected.resolved_acl_root);
  }
  if (expected.resolved_schema_root !== undefined) {
    expect(new SchemaLoader(config).load('probe').description).toBe(expected.resolved_schema_root);
  }
}

describe('Conformance: project-root path resolution', () => {
  it.each(fixture.test_cases)('$id', (testCase) => {
    const cwd = layoutPath(testCase.cwd);
    process.chdir(cwd);
    const config = loadConfig(testCase);
    const source = config.sourcePath;
    const expected = testCase.expected;
    expect(source === null ? null : dirname(resolve(source))).toBe(
      expected.config_source_dir === null ? null : layoutPath(expected.config_source_dir),
    );

    if (testCase.consume_from !== undefined) process.chdir(layoutPath(testCase.consume_from));
    expect(isAbsolute(config.projectRoot)).toBe(true);
    expect(config.projectRoot).toBe(layoutPath(expected.project_root));
    expect(config.projectRoot === cwd).toBe(expected.project_root_equals_cwd);
    assertConfigPaths(config, testCase);
    assertConsumers(config, expected);
  });
});
