/**
 * Cross-language conformance driver for `bindings_dir_resolution.json`
 * (PROTOCOL_SPEC §5.12.6, apcore#114 / apcore-typescript#36).
 *
 * Fixture source: apcore/conformance/fixtures/bindings_dir_resolution.json
 * (canonical, read in place — never vendored).
 *
 * §5.12.6 through v1.34.0 was a MUST with no subject: "if `bindings.dir` is
 * configured, implementations MUST scan files matching `pattern` in that
 * directory", saying neither who scans nor when. v1.35.0 names both — a binding
 * loader invoked WITHOUT an explicit directory resolves it from `bindings.dir`
 * under §9.2 precedence (env > file > default `./bindings`), matches on
 * `bindings.pattern` through the same chain, lets an explicit argument win, and
 * MUST NOT scan at client initialisation. v1.36.0 adds clause 5: a resolved
 * directory that does not exist MUST raise, naming that directory, rather than
 * returning an empty result.
 *
 * WHAT THE `driver_contract` DEMANDS, AND WHERE EACH DEMAND LANDS
 * ---------------------------------------------------------------
 * - `entry_point`: every case calls the public `BindingLoader.loadBindingDir`.
 *   No private resolution helper is touched — §5.12.6's subject is the loader
 *   as invoked, and a helper that computes the right directory while the public
 *   entry point ignores it satisfies nothing.
 * - `config_construction`: each case's `config_file` block is written to a real
 *   file on disk and loaded through `Config.discover()`. The FILE tier is the
 *   tier under test; an in-memory mapping bypasses it.
 * - `no_explicit_argument`: `explicit_dir: null` cases pass `undefined` as the
 *   directory argument. Handing the loader a directory the driver computed is
 *   the one path that works under BOTH the old and the corrected behaviour.
 * - `env_isolation`: `APCORE_BINDINGS_DIR` and `APCORE_BINDINGS_PATTERN` are
 *   DELETED (not blanked) for every case that does not list them. An empty
 *   string is itself a §9.2 override; §9.2.1 requirement 5 now makes this SDK
 *   discard it, but a driver that relied on that would be asserting the guard
 *   rather than the precedence chain.
 * - `fs_values_name_a_descriptor`: every `fs` value names a key in the
 *   fixture's `binding_files` map, and that descriptor is written verbatim
 *   apart from the target rewrite below. Module IDs come from the descriptor —
 *   they are distinct on purpose, so the IDs the loader returns identify the
 *   directory it enumerated.
 * - `env_set_after_config_load`: those variables are stubbed AFTER
 *   `Config.discover()` returns and BEFORE the loader runs. That ordering is
 *   the whole clause-2 case: it separates a loader reading the merged `Config`
 *   from one reading the raw environment.
 * - `scan_observation`: `scanned_dir` is read back from the loader's RESULT
 *   against the case's filesystem layout, never from the config value this
 *   driver supplied.
 *
 * THE ONE FIXTURE REWRITE (reported, not silently assumed)
 * --------------------------------------------------------
 * Each descriptor's `target` is `fixture_targets:greet` — a placeholder module
 * path, and this SDK resolves a target by dynamic import, so it must name a
 * real ESM module. The driver writes one target module PER BINDING FILE whose
 * `greet` returns that file's own layout path, and rewrites only the module
 * half of the target. Two consequences, both wanted: the descriptor's
 * `module_id` is used exactly as the fixture declares it, and executing a
 * loaded module reports WHICH FILE it came from. The pattern cases need that
 * second observation — `custom_bindings/greet.bind.yaml` and
 * `custom_bindings/decoy.binding.yaml` both name the `greet` descriptor, so
 * module ID alone cannot tell a configured pattern from the default one.
 *
 * NO `it.fails` REMAINS IN THIS FILE. v1.35.0's
 * `missing_configured_dir_is_not_an_error` wanted an empty result where this
 * SDK raises; v1.36.0 clause 5 replaced it with `missing_configured_dir_raises`,
 * which is this SDK's behaviour, so the divergence and its pin are both gone.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import * as fs from 'node:fs';
import * as path from 'node:path';
import yaml from 'js-yaml';

import { BindingLoader } from '../src/bindings.js';
import { APCore } from '../src/client.js';
import { Config } from '../src/config.js';
import type { Context } from '../src/context.js';
import { BindingFileInvalidError } from '../src/errors.js';
import type { FunctionModule } from '../src/decorator.js';
import { Registry } from '../src/registry/registry.js';
import { findFixturesRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

type JsonRecord = Record<string, unknown>;

interface BindingsDirCase {
  readonly id: string;
  readonly comment?: string;
  readonly config_file?: { readonly path: string; readonly content: JsonRecord };
  readonly env?: Record<string, string>;
  readonly env_set_after_config_load?: Record<string, string>;
  readonly explicit_dir?: string | null;
  readonly invoke_loader?: boolean;
  readonly fs?: Record<string, string>;
  readonly expected: JsonRecord;
}

interface BindingDescriptor {
  readonly bindings: readonly JsonRecord[];
}

interface BindingsDirFixture {
  readonly description: string;
  readonly driver_contract: Record<string, string>;
  readonly binding_files: Record<string, BindingDescriptor & { readonly comment?: string }>;
  readonly test_cases: readonly BindingsDirCase[];
}

const fixture: BindingsDirFixture = JSON.parse(
  fs.readFileSync(path.join(findFixturesRoot(), 'bindings_dir_resolution.json'), 'utf-8'),
);

function caseFor(id: string): BindingsDirCase {
  const found = fixture.test_cases.find((c) => c.id === id);
  if (!found) throw new Error(`Fixture case '${id}' not found in bindings_dir_resolution.json`);
  return found;
}

/** The named descriptor an `fs` value points at (`fs_values_name_a_descriptor`). */
function descriptorFor(name: string): BindingDescriptor {
  const found = fixture.binding_files[name];
  if (!found?.bindings) {
    throw new Error(`bindings_dir_resolution.json has no binding_files descriptor '${name}'`);
  }
  return found;
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

let tmpDir: string;
let originalCwd: string;
let loader: BindingLoader;
let registry: Registry;

beforeEach(() => {
  tmpDir = realpathSync(mkdtempSync(join(tmpdir(), 'apcore-bindings-dir-')));
  originalCwd = process.cwd();
  loader = new BindingLoader();
  registry = new Registry();

  // `env_isolation`: DELETED, never set to ''.
  vi.stubEnv('APCORE_BINDINGS_DIR', undefined);
  vi.stubEnv('APCORE_BINDINGS_PATTERN', undefined);
  vi.stubEnv('APCORE_CONFIG_FILE', undefined);
  // The user-level §9.14 tiers must not reach the real home directory.
  vi.stubEnv('HOME', join(tmpDir, 'nonexistent-home'));

  // `loadBindings` warns once per file about the absent `spec_version` (the
  // fixture's descriptors do not carry one) and that noise is not under test.
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

afterEach(() => {
  process.chdir(originalCwd);
  rmSync(tmpDir, { recursive: true, force: true });
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

/**
 * Write the ESM module a binding file's targets resolve to.
 *
 * One per binding FILE, and its `greet` returns that file's layout path, so a
 * loaded module can be asked which file declared it. `inputSchema` /
 * `outputSchema` are exported because the descriptors carry `auto_schema: true`
 * and TypeScript erases types at run time: without them the loader raises
 * BINDING_SCHEMA_INFERENCE_FAILED and every case fails for a reason that has
 * nothing to do with directory resolution.
 */
function writeTargetModule(relPath: string, callable: string): string {
  const modulePath = join(tmpDir, '__targets__', `${relPath.replace(/[^a-zA-Z0-9]/g, '_')}.mjs`);
  mkdirSync(dirname(modulePath), { recursive: true });
  writeFileSync(
    modulePath,
    "export const inputSchema = { type: 'object', properties: {} };\n" +
      "export const outputSchema = { type: 'object', properties: {} };\n" +
      `export function ${callable}() { return { source_file: ${JSON.stringify(relPath)} }; }\n`,
    'utf-8',
  );
  return modulePath;
}

/** Write the named descriptor into `relPath`, rewriting only the target's module half. */
function writeBindingFile(relPath: string, descriptorName: string): void {
  const absPath = join(tmpDir, relPath);
  mkdirSync(dirname(absPath), { recursive: true });
  const entries = descriptorFor(descriptorName).bindings.map((entry) => {
    const callable = String(entry['target']).split(':')[1] ?? 'greet';
    return { ...entry, target: `${writeTargetModule(relPath, callable)}:${callable}` };
  });
  writeFileSync(absPath, yaml.dump({ bindings: entries }), 'utf-8');
}

/** Materialise a case's `fs` block; each value names a `binding_files` key. */
function writeLayout(testCase: BindingsDirCase): void {
  for (const [relPath, descriptorName] of Object.entries(testCase.fs ?? {})) {
    writeBindingFile(relPath, descriptorName);
  }
}

/** The module IDs the file at `relPath` declares, per its named descriptor. */
function moduleIdsIn(testCase: BindingsDirCase, relPath: string): string[] {
  const descriptorName = (testCase.fs ?? {})[relPath] as string;
  return descriptorFor(descriptorName).bindings.map((b) => String(b['module_id']));
}

/**
 * Write the case's `config_file` block verbatim and load it through discovery.
 *
 * `validate: false` because the fixture's `content` deliberately carries only
 * the keys the case is about; §9.1's two required keys (`version`,
 * `project.name`) are not part of this contract, and inventing them would put
 * content on disk the fixture never declared.
 */
function loadConfig(testCase: BindingsDirCase): Config {
  const block = testCase.config_file;
  if (!block) throw new Error(`Case ${testCase.id} has no config_file block`);
  writeFileSync(join(tmpDir, block.path), yaml.dump(block.content ?? {}), 'utf-8');
  const config = Config.discover({ validate: false });
  expect(config.sourcePath, `${testCase.id}: discovery did not find the case's config file`).toBe(
    block.path,
  );
  return config;
}

/** Apply a case's `env` block after the isolation deletions. */
function applyEnv(testCase: BindingsDirCase): void {
  for (const [name, value] of Object.entries(testCase.env ?? {})) {
    vi.stubEnv(name, value);
  }
}

/**
 * The directory holding the file that declares `moduleId`, per the case layout.
 *
 * This is the `scan_observation` requirement: the answer is derived from the
 * loader's result and the filesystem layout, NOT from the `bindings.dir` value
 * this driver wrote.
 */
function directoryOf(testCase: BindingsDirCase, moduleId: string): string {
  const entry = Object.keys(testCase.fs ?? {}).find((p) =>
    moduleIdsIn(testCase, p).includes(moduleId),
  );
  if (!entry) throw new Error(`${testCase.id}: no layout file declares module '${moduleId}'`);
  return dirname(entry);
}

/** Which layout file a loaded module came from — see "THE ONE FIXTURE REWRITE". */
async function sourceFileOf(module: FunctionModule): Promise<string> {
  const result = await module.execute({}, undefined as unknown as Context);
  return String(result['source_file']);
}

/** Run one `explicit_dir: null | "..."` case end to end. */
async function runCase(testCase: BindingsDirCase): Promise<{
  scannedDir: string | null;
  loadedModuleIds: string[];
  sourceFiles: string[];
}> {
  writeLayout(testCase);
  applyEnv(testCase);
  process.chdir(tmpDir);
  const config = loadConfig(testCase);

  // `env_set_after_config_load`: the merged Config already holds the FILE
  // value; a loader that reads the raw variable would now see something else.
  for (const [name, value] of Object.entries(testCase.env_set_after_config_load ?? {})) {
    vi.stubEnv(name, value);
  }

  // `no_explicit_argument`: genuinely absent, not a directory computed here.
  const explicit = testCase.explicit_dir ?? undefined;
  const modules = await loader.loadBindingDir(explicit, registry, undefined, config);

  const loadedModuleIds = modules.map((m) => m.moduleId ?? '').sort();
  const sourceFiles = (await Promise.all(modules.map(sourceFileOf))).sort();
  const dirs = new Set(loadedModuleIds.map((id) => directoryOf(testCase, id)));
  if (dirs.size > 1) throw new Error(`${testCase.id}: loader spanned directories ${[...dirs]}`);
  return { scannedDir: dirs.size === 1 ? ([...dirs][0] as string) : null, loadedModuleIds, sourceFiles };
}

/** The `{scanned, scanned_dir, loaded_module_ids}` triple the fixture states. */
function assertScan(
  testCase: BindingsDirCase,
  observed: { scannedDir: string | null; loadedModuleIds: string[] },
): void {
  expect({
    scanned: true,
    scanned_dir: observed.scannedDir,
    loaded_module_ids: observed.loadedModuleIds,
  }).toEqual(testCase.expected);
}

// ---------------------------------------------------------------------------

describe('Conformance: binding-directory resolution (§5.12.6)', () => {
  /** Every case id this file registers, recorded AS it registers them. */
  const DRIVEN = new Set<string>();

  /**
   * `it()` that remembers the case id.
   *
   * The coverage check below used to compare the fixture against a
   * hand-maintained `covered` list. The list named all four A25 pattern cases
   * while not one of them had an `it()`, so the assertion asserted its own
   * list — found by `check_case_pinning.py`, which mutated those four and saw
   * nothing go red. Collecting the set at registration makes that
   * unrepresentable: a case is covered exactly when a test exists for it.
   */
  function driveCase(id: string, fn: () => void | Promise<void>): void {
    DRIVEN.add(id);
    it(id, fn);
  }

  driveCase('config_file_dir_is_scanned_with_env_unset', async () => {
    // THE DISCRIMINATING CASE. `bindings.dir` in a config FILE,
    // APCORE_BINDINGS_DIR unset, no explicit directory argument. No SDK passed
    // this before v1.35.0, and TypeScript's deleted raw `process.env` read
    // covered only the environment tier.
    const testCase = caseFor('config_file_dir_is_scanned_with_env_unset');
    assertScan(testCase, await runCase(testCase));
    expect(registry.has('greet')).toBe(true);
  });

  driveCase('default_dir_when_key_absent', async () => {
    // No `bindings.dir` anywhere and no argument: the §9.1.1 default ./bindings.
    const testCase = caseFor('default_dir_when_key_absent');
    assertScan(testCase, await runCase(testCase));
  });

  driveCase('env_overrides_config_file_dir', async () => {
    // §9.2 precedence, top tier. BOTH candidate directories exist, both hold a
    // binding file, and the two descriptors carry DISTINCT module IDs — with a
    // shared ID this case passed whichever directory the implementation
    // scanned, which is the discriminating-power gap v1.36.0 repaired.
    const testCase = caseFor('env_overrides_config_file_dir');
    assertScan(testCase, await runCase(testCase));
    expect(registry.has('from_file_side'), 'the config-file directory was scanned instead').toBe(
      false,
    );
  });

  driveCase('env_var_must_not_be_read_directly_at_the_loader', async () => {
    // §5.12.6 clause 2, and the coverage v1.35.0's fixture lacked entirely: the
    // environment tier reaches the loader through §9.2's ordinary override
    // mechanism, never through a read at the loader. APCORE_BINDINGS_DIR is set
    // AFTER the Config is built, so the merged `bindings.dir` is still the file
    // value. A conforming loader scans from_file; one that reads the raw
    // variable scans from_env — and satisfies every other case in this fixture.
    const testCase = caseFor('env_var_must_not_be_read_directly_at_the_loader');
    const observed = await runCase(testCase);

    assertScan(testCase, observed);
    expect(process.env['APCORE_BINDINGS_DIR'], 'the variable must really be set').toBe(
      './from_env',
    );
    expect(registry.has('from_env_side'), 'the loader read the raw environment').toBe(false);
  });

  driveCase('explicit_argument_wins_over_config', async () => {
    // explicit > env > file > default. All three candidates exist and hold a file.
    const testCase = caseFor('explicit_argument_wins_over_config');
    assertScan(testCase, await runCase(testCase));
    expect(registry.has('from_file_side')).toBe(false);
    expect(registry.has('from_env_side')).toBe(false);
  });

  driveCase('config_file_pattern_is_honoured', async () => {
    // `bindings.pattern` comes from the same precedence chain as the directory,
    // not from a loader-signature default. The decoy matches the DEFAULT
    // pattern, so an SDK that keeps the pattern in its signature loads the
    // wrong file rather than none. Both files name the same descriptor, so the
    // discriminator is the FILE the loaded module came from.
    const testCase = caseFor('config_file_pattern_is_honoured');
    const observed = await runCase(testCase);

    assertScan(testCase, observed);
    expect(observed.sourceFiles, 'the *.binding.yaml default was used instead').toEqual([
      'custom_bindings/greet.bind.yaml',
    ]);
  });

  driveCase('default_pattern_when_key_absent', async () => {
    // No pattern configured: `*.binding.yaml` applies and the sibling that does
    // not match it stays unloaded.
    const testCase = caseFor('default_pattern_when_key_absent');
    const observed = await runCase(testCase);

    assertScan(testCase, observed);
    expect(observed.sourceFiles).toEqual(['custom_bindings/greet.binding.yaml']);
  });

  driveCase('missing_configured_dir_raises', async () => {
    // §5.12.6 clause 5 (v1.36.0). A resolved directory that does not exist is
    // an error naming that directory, not an empty result. Contrast
    // ACL.discover (D-64): discovery is automatic and silent, binding loading
    // is user-invoked, so an absent directory there is a mistake.
    const testCase = caseFor('missing_configured_dir_raises');
    writeLayout(testCase);
    applyEnv(testCase);
    process.chdir(tmpDir);
    const config = loadConfig(testCase);

    const error = await loader
      .loadBindingDir(undefined, registry, undefined, config)
      .catch((e: unknown) => e);

    expect(error).toBeInstanceOf(BindingFileInvalidError);
    const resolvedDir = testCase.expected['scanned_dir'] as string;
    expect({
      scanned: true,
      scanned_dir: resolvedDir,
      error_code: (error as BindingFileInvalidError).code,
      error_message_names_resolved_dir: (error as BindingFileInvalidError).message.includes(
        resolvedDir,
      ),
    }).toEqual(testCase.expected);
    expect(registry.list().length).toBe(0);
  });

  driveCase('no_auto_scan_at_init', async () => {
    // §5.12.6 clause 3. The configured directory exists and holds a well-formed
    // binding file that would load cleanly if anything scanned. Asserting only
    // that construction succeeded would pass trivially, so the assertion is
    // that the module ID is ABSENT from the registry afterwards.
    const testCase = caseFor('no_auto_scan_at_init');
    writeLayout(testCase);
    applyEnv(testCase);
    process.chdir(tmpDir);
    const config = loadConfig(testCase);

    const client = new APCore({ config });

    // `includeHidden`: a startup scan that registered the module as hidden
    // would still be a startup scan, and the default visibility filter would
    // hide it from this assertion.
    const registered = client.registry.list({ includeHidden: true });
    expect(registered).not.toContain('greet');
    expect(client.registry.has('greet')).toBe(false);
    expect({
      scanned: false,
      registered_module_ids: registered.filter((id) => id === 'greet'),
    }).toEqual(testCase.expected);

    // The file really would have loaded: proving the negative needs the
    // positive next to it, or "absent" could just mean "unloadable".
    const modules = await loader.loadBindingDir(undefined, client.registry, undefined, config);
    expect(modules.map((m) => m.moduleId)).toEqual(['greet']);
  });

  // The A25 pattern-dialect cases (§9.2.3, #116). Driven from the fixture
  // rather than hand-written per case: they differ only in the pattern and the
  // files it must and must not match, and every one of them was listed as
  // "covered" by the check below while no `it()` existed for any of them.
  for (const id of [
    'pattern_star_in_the_middle_is_honoured',
    'pattern_first_star_must_not_be_removed_from_the_middle',
    'pattern_question_mark_is_a_wildcard',
    'pattern_bracket_is_a_literal_not_a_character_class',
  ]) {
    driveCase(id, async () => {
      const testCase = caseFor(id);
      const observed = await runCase(testCase);
      assertScan(testCase, observed);
    });
  }

  it('every fixture case is driven', () => {
    // Read back what this file REGISTERED, rather than comparing the fixture
    // against a hand-maintained list. The list said all four A25 pattern cases
    // were covered while not one of them had an `it()` — a coverage assertion
    // that asserts its own list is the defect `check_case_pinning.py` found
    // here, and it cannot recur once the set comes from the suite.
    const driven = new Set(fixture.test_cases.map((c) => c.id));
    const covered = DRIVEN;
    expect(
      [...driven].filter((id) => !covered.has(id)),
      'bindings_dir_resolution.json gained cases this driver ignores',
    ).toEqual([]);
    expect(fixture.test_cases.length, 'the fixture is 13 cases as of spec v1.37.0').toBe(13);
  });
});
