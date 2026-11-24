import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as os from 'node:os';
import { Config, discoverConfigFile } from '../src/config.js';

const VALID_YAML = `
version: "0.8.0"
extensions:
  root: ./extensions
  auto_discover: false
schema:
  root: ./schemas
acl:
  root: ./acl
  default_effect: deny
project:
  name: discovery-test
`.trim();

function makeTempDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-discovery-'));
}

function cleanDir(dir: string): void {
  fs.rmSync(dir, { recursive: true, force: true });
}

describe('discoverConfigFile', () => {
  let tmpDir: string;
  let originalCwd: string;

  beforeEach(() => {
    tmpDir = makeTempDir();
    originalCwd = process.cwd();
    process.chdir(tmpDir);
    vi.stubEnv('APCORE_CONFIG_FILE', '');
  });

  afterEach(() => {
    process.chdir(originalCwd);
    cleanDir(tmpDir);
    vi.unstubAllEnvs();
  });

  it('returns APCORE_CONFIG_FILE env var path when set', () => {
    const configPath = path.join(tmpDir, 'custom.yaml');
    fs.writeFileSync(configPath, VALID_YAML);
    vi.stubEnv('APCORE_CONFIG_FILE', configPath);
    const result = discoverConfigFile();
    expect(result).toBe(configPath);
  });

  it('returns null when no config file is found', () => {
    vi.stubEnv('APCORE_CONFIG_FILE', '');
    // tmpDir has no config files; HOME is set to a non-existent dir
    vi.stubEnv('HOME', path.join(tmpDir, 'nonexistent_home'));
    const result = discoverConfigFile();
    expect(result).toBeNull();
  });

  it('returns project.yaml when present in cwd', () => {
    vi.stubEnv('APCORE_CONFIG_FILE', '');
    fs.writeFileSync(path.join(tmpDir, 'project.yaml'), VALID_YAML);
    const result = discoverConfigFile();
    expect(result).toBe('project.yaml');
  });

  it('returns project.yml when present in cwd and project.yaml absent', () => {
    vi.stubEnv('APCORE_CONFIG_FILE', '');
    fs.writeFileSync(path.join(tmpDir, 'project.yml'), VALID_YAML);
    const result = discoverConfigFile();
    expect(result).toBe('project.yml');
  });

  it('returns apcore.yaml when present in cwd', () => {
    vi.stubEnv('APCORE_CONFIG_FILE', '');
    fs.writeFileSync(path.join(tmpDir, 'apcore.yaml'), VALID_YAML);
    const result = discoverConfigFile();
    expect(result).toBe('apcore.yaml');
  });

  it('returns apcore.yml when it is the only cwd candidate', () => {
    vi.stubEnv('APCORE_CONFIG_FILE', '');
    fs.writeFileSync(path.join(tmpDir, 'apcore.yml'), VALID_YAML);
    const result = discoverConfigFile();
    expect(result).toBe('apcore.yml');
  });

  it('env var takes priority over cwd file', () => {
    const envFile = path.join(tmpDir, 'env.yaml');
    fs.writeFileSync(envFile, VALID_YAML);
    fs.writeFileSync(path.join(tmpDir, 'project.yaml'), VALID_YAML);
    vi.stubEnv('APCORE_CONFIG_FILE', envFile);
    const result = discoverConfigFile();
    expect(result).toBe(envFile);
  });
});

describe('Config.load discovery integration', () => {
  let tmpDir: string;
  let originalCwd: string;

  beforeEach(() => {
    tmpDir = makeTempDir();
    originalCwd = process.cwd();
    process.chdir(tmpDir);
    vi.stubEnv('APCORE_CONFIG_FILE', '');
  });

  afterEach(() => {
    process.chdir(originalCwd);
    cleanDir(tmpDir);
    vi.unstubAllEnvs();
  });

  it('returns fromDefaults() when no file found', () => {
    vi.stubEnv('HOME', path.join(tmpDir, 'nonexistent_home'));
    const config = Config.load();
    expect(config).toBeInstanceOf(Config);
  });

  it('loads discovered file and reads project name', () => {
    vi.stubEnv('APCORE_CONFIG_FILE', '');
    const configPath = path.join(tmpDir, 'project.yaml');
    fs.writeFileSync(configPath, VALID_YAML);
    const config = Config.load();
    expect(config.get('project.name')).toBe('discovery-test');
  });

  it('loads file from APCORE_CONFIG_FILE env var', () => {
    const configPath = path.join(tmpDir, 'custom.yaml');
    fs.writeFileSync(configPath, VALID_YAML);
    vi.stubEnv('APCORE_CONFIG_FILE', configPath);
    const config = Config.load();
    expect(config.get('project.name')).toBe('discovery-test');
  });

  it('explicit path still works', () => {
    const configPath = path.join(tmpDir, 'explicit.yaml');
    fs.writeFileSync(configPath, VALID_YAML);
    const config = Config.load(configPath);
    expect(config.get('project.name')).toBe('discovery-test');
  });
});

/**
 * apcore#88: `$APCORE_CONFIG_FILE` selects the document, it is not in it.
 *
 * §9.2 turns every `APCORE_*` variable into a configuration override, so the
 * file selector used to lower to the dot-path `config.file` and land in the
 * **declared** document — the view §9.1's required-field check runs against.
 * `config.file` is declared by no schema
 * (`conformance/fixtures/config_key_governance.json`).
 *
 * These tests assert the **exact** declared key set, not merely that
 * `config.file` is gone: absence alone would also hold for an implementation
 * that dropped a key the file really does declare.
 */
describe('APCORE_CONFIG_FILE is not a configuration override', () => {
  const MINIMAL_YAML = 'version: "1.0.0"\nproject:\n  name: demo\n';
  const NAMESPACE_YAML = 'apcore:\n  version: "1.0.0"\n  project:\n    name: demo\n';

  let tmpDir: string;
  let savedApcoreEnv: Record<string, string | undefined>;

  function flatten(data: Record<string, unknown>, prefix = ''): string[] {
    const paths: string[] = [];
    for (const [key, value] of Object.entries(data)) {
      const dotPath = `${prefix}${key}`;
      if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
        const nested = value as Record<string, unknown>;
        if (Object.keys(nested).length > 0) {
          paths.push(...flatten(nested, `${dotPath}.`));
          continue;
        }
      }
      paths.push(dotPath);
    }
    return paths.sort();
  }

  function declaredOf(config: Config): string[] {
    return flatten((config as unknown as { _declared: Record<string, unknown> })._declared);
  }

  beforeEach(() => {
    tmpDir = makeTempDir();
    // Remove, not blank: an empty string is still a value the override pass
    // would write, so stubbing to '' would inject the very phantom under test.
    savedApcoreEnv = {};
    for (const name of Object.keys(process.env)) {
      if (name.startsWith('APCORE_')) {
        savedApcoreEnv[name] = process.env[name];
        delete process.env[name];
      }
    }
  });

  afterEach(() => {
    for (const name of Object.keys(process.env)) {
      if (name.startsWith('APCORE_')) delete process.env[name];
    }
    for (const [name, value] of Object.entries(savedApcoreEnv)) {
      if (value !== undefined) process.env[name] = value;
    }
    cleanDir(tmpDir);
  });

  it('legacy mode declares exactly the keys the file declares', () => {
    const configPath = path.join(tmpDir, 'custom.yaml');
    fs.writeFileSync(configPath, MINIMAL_YAML);
    process.env['APCORE_CONFIG_FILE'] = configPath;

    const config = Config.load(configPath);

    expect(declaredOf(config)).toEqual(['project.name', 'version']);
  });

  it('namespace mode declares exactly the keys the file declares', () => {
    const configPath = path.join(tmpDir, 'ns.yaml');
    fs.writeFileSync(configPath, NAMESPACE_YAML);
    process.env['APCORE_CONFIG_FILE'] = configPath;

    const config = Config.load(configPath);

    expect(declaredOf(config)).toEqual(['apcore.project.name', 'apcore.version']);
  });

  it('real overrides still reach the declared document', () => {
    // The exemption is one variable wide: `bindings.dir` IS a declared key, so
    // `APCORE_BINDINGS_DIR` is §9.2 working as designed.
    const configPath = path.join(tmpDir, 'custom.yaml');
    fs.writeFileSync(configPath, MINIMAL_YAML);
    process.env['APCORE_CONFIG_FILE'] = configPath;
    process.env['APCORE_BINDINGS_DIR'] = './generated';

    const config = Config.load(configPath);

    expect(declaredOf(config)).toEqual(['bindings.dir', 'project.name', 'version']);
  });
});
