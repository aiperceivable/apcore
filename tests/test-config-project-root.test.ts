/**
 * `Config.projectRoot` and its §13.2 deprecation notice (apcore#113,
 * PROTOCOL_SPEC §9.2.2).
 *
 * `projectRoot` reports the base a relative path-typed value (§9.2.1) is
 * *about*. It applies nothing: `SchemaLoader` still resolves `schema.root`
 * against the CWD and `ACL.discover` still resolves `acl.root` against the
 * config file's directory, unchanged. This suite pins the reported base for
 * one case per §9.14 discovery tier, and pins the warning to the narrow
 * condition it is supposed to fire under.
 */

import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { Config, discoverConfigFile, userLevelConfigPaths } from '../src/config.js';

const MINIMAL_YAML = 'version: "0.29.0"\nproject:\n  name: project-root-test\n';

let tmpDir: string;
let projectDir: string;
let elsewhereDir: string;
let fakeHome: string;
let originalCwd: string;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), 'apcore-project-root-'));
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
    // The live defect in apcore#113: `acl.root: ./acl` written in
    // ~/.config/apcore/config.yaml means "this project's acl/", never
    // "~/.config/apcore/acl/".
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

  it('a user-level path loaded explicitly is still recognised as user-level', () => {
    const configPath = write(xdgConfigPath(fakeHome));
    process.chdir(projectDir);

    expect(Config.load(configPath).projectRoot).toBe(process.cwd());
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

describe('the §13.2 project-root deprecation notice', () => {
  /** Warning lines this notice emitted, ignoring every other console.warn. */
  function noticesFrom(spy: MockInstance<typeof console.warn>): string[] {
    return spy.mock.calls
      .map((call) => String(call[0]))
      .filter((line) => line.includes('PROTOCOL_SPEC §9.2.2'));
  }

  function spyOnWarn() {
    return vi.spyOn(console, 'warn').mockImplementation(() => {});
  }

  it('fires when the project root differs from the CWD and a relative value is present', () => {
    write(join(elsewhereDir, 'custom.yaml'));
    process.chdir(projectDir);
    const warn = spyOnWarn();

    const config = Config.load(join(elsewhereDir, 'custom.yaml'));

    expect(config.projectRoot).not.toBe(process.cwd());
    const notices = noticesFrom(warn);
    expect(notices).toHaveLength(1);
    // The DEFAULTS table supplies these three as relative values, and every
    // one of them re-roots if the base moves.
    expect(notices[0]).toContain('acl.root');
    expect(notices[0]).toContain('schema.root');
    expect(notices[0]).toContain('extensions.root');
    expect(notices[0]).toContain(elsewhereDir);
  });

  it('does NOT fire in the ordinary tier 2-5 case, where the roots coincide', () => {
    write(join(projectDir, 'apcore.yaml'));
    process.chdir(projectDir);
    const warn = spyOnWarn();

    const config = Config.discover();

    expect(config.projectRoot).toBe(process.cwd());
    expect(noticesFrom(warn)).toEqual([]);
  });

  it('does NOT fire for a user-level config, whose project root is already the CWD', () => {
    write(xdgConfigPath(fakeHome));
    process.chdir(projectDir);
    const warn = spyOnWarn();

    Config.discover();

    expect(noticesFrom(warn)).toEqual([]);
  });

  it('does NOT fire when the project root differs but every path-typed value is absolute', () => {
    // The second half of the condition, on its own. Without it this would be a
    // blanket warning on every out-of-tree config, which apcore#113 explicitly
    // rules out.
    // EVERY §9.2.1 key has to be spelled absolutely, `bindings.dir` included:
    // the DEFAULTS table now supplies `./bindings` (spec v1.36.0), so leaving
    // it unstated leaves a relative value standing and the notice correctly
    // fires.
    write(
      join(elsewhereDir, 'custom.yaml'),
      `${MINIMAL_YAML}extensions:\n  root: "${join(tmpDir, 'ext')}"\nschema:\n  root: "${join(tmpDir, 'sch')}"\nacl:\n  root: "${join(tmpDir, 'acl')}"\nbindings:\n  dir: "${join(tmpDir, 'bnd')}"\n`,
    );
    process.chdir(projectDir);
    const warn = spyOnWarn();

    const config = Config.load(join(elsewhereDir, 'custom.yaml'), { validate: false });

    expect(config.projectRoot).not.toBe(process.cwd());
    expect(noticesFrom(warn)).toEqual([]);
  });

  it('counts a relative element of the list-valued extensions.roots', () => {
    // §9.2.1 reports this key as `extensions.roots[]`; one relative element in
    // either accepted form is enough.
    write(
      join(elsewhereDir, 'custom.yaml'),
      `${MINIMAL_YAML}extensions:\n  root: "${join(tmpDir, 'ext')}"\n  roots:\n    - root: "./plugins"\n      namespace: "plugins"\nschema:\n  root: "${join(tmpDir, 'sch')}"\nacl:\n  root: "${join(tmpDir, 'acl')}"\nbindings:\n  dir: "${join(tmpDir, 'bnd')}"\n`,
    );
    process.chdir(projectDir);
    const warn = spyOnWarn();

    Config.load(join(elsewhereDir, 'custom.yaml'), { validate: false });

    const notices = noticesFrom(warn);
    expect(notices).toHaveLength(1);
    expect(notices[0]).toContain('extensions.roots[]');
    expect(notices[0]).not.toContain('schema.root');
  });

  it('counts a bindings.dir the config file declares relative', () => {
    write(
      join(elsewhereDir, 'custom.yaml'),
      `${MINIMAL_YAML}extensions:\n  root: "${join(tmpDir, 'ext')}"\nschema:\n  root: "${join(tmpDir, 'sch')}"\nacl:\n  root: "${join(tmpDir, 'acl')}"\nbindings:\n  dir: "./bindings"\n`,
    );
    process.chdir(projectDir);
    const warn = spyOnWarn();

    Config.load(join(elsewhereDir, 'custom.yaml'), { validate: false });

    const notices = noticesFrom(warn);
    expect(notices).toHaveLength(1);
    expect(notices[0]).toContain('bindings.dir');
  });

  it('fires once per LOAD, not once per process — a reload warns again', () => {
    // PROTOCOL_SPEC §9.2.2 requirement 2: "once per configuration load, not
    // once per process ... implementations MUST NOT suppress it with
    // process-global state". This SDK used to carry exactly that flag.
    const configPath = write(join(elsewhereDir, 'custom.yaml'));
    process.chdir(projectDir);
    const warn = spyOnWarn();

    Config.load(configPath);
    Config.load(configPath);
    Config.load(configPath);

    expect(noticesFrom(warn)).toHaveLength(3);
  });

  it('warns for BOTH of two different affected configurations in one process', () => {
    // The case the once-per-process flag made impossible, and the reason
    // §9.2.2 forbids it: whichever load ran first consumed the warning, so the
    // second affected document was silent and the operator could not tell which
    // one triggered it. Each document leaves a DIFFERENT single key relative,
    // so the two notices are told apart by content, not by count alone.
    // (`acl.root` is unusable as a discriminator: the notice's own prose names
    // it when explaining today's inconsistent bases.)
    const first = write(
      join(elsewhereDir, 'first.yaml'),
      `${MINIMAL_YAML}extensions:\n  root: "${join(tmpDir, 'ext')}"\nschema:\n  root: "./schemas-first"\nacl:\n  root: "${join(tmpDir, 'acl')}"\nbindings:\n  dir: "${join(tmpDir, 'bnd')}"\n`,
    );
    const secondDir = join(tmpDir, 'second-elsewhere');
    const second = write(
      join(secondDir, 'second.yaml'),
      `${MINIMAL_YAML}extensions:\n  root: "./ext-second"\nschema:\n  root: "${join(tmpDir, 'sch')}"\nacl:\n  root: "${join(tmpDir, 'acl')}"\nbindings:\n  dir: "${join(tmpDir, 'bnd')}"\n`,
    );
    process.chdir(projectDir);
    const warn = spyOnWarn();

    Config.load(first, { validate: false });
    Config.load(second, { validate: false });

    const notices = noticesFrom(warn);
    expect(notices).toHaveLength(2);
    // The FIRST document's notice names its own key and its own directory...
    expect(notices[0]).toContain('schema.root');
    expect(notices[0]).not.toContain('extensions.root');
    expect(notices[0]).toContain(elsewhereDir);
    // ...and so does the SECOND's. Under the old flag this notice never existed.
    expect(notices[1]).toContain('extensions.root');
    expect(notices[1]).not.toContain('schema.root');
    expect(notices[1]).toContain(secondDir);
  });

  it('says nothing for a config that never touched the filesystem', () => {
    process.chdir(projectDir);
    const warn = spyOnWarn();

    Config.fromDefaults();
    new Config({ acl: { root: './acl' } });

    expect(noticesFrom(warn)).toEqual([]);
  });
});

describe('projectRoot changes no resolution behaviour', () => {
  it('SchemaLoader still resolves schema.root against the CWD', async () => {
    // The §13.2 phase is a notice, not a move. Pinned here so a premature
    // adoption of the new base cannot land silently.
    const { SchemaLoader } = await import('../src/schema/loader.js');
    write(join(elsewhereDir, 'custom.yaml'), `${MINIMAL_YAML}schema:\n  root: "./schemas"\n`);
    process.chdir(projectDir);
    vi.spyOn(console, 'warn').mockImplementation(() => {});

    const config = Config.load(join(elsewhereDir, 'custom.yaml'));
    const loader = new SchemaLoader(config);

    expect(config.projectRoot).toBe(elsewhereDir);
    expect((loader as unknown as { _schemasDir: string })._schemasDir).toBe(
      join(process.cwd(), 'schemas'),
    );
  });
});
