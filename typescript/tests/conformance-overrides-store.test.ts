/**
 * Cross-language conformance driver for `overrides_store.json`
 * (Issue #45.1, D-40 — pluggable persistence behind
 * `system.control.update_config` / `system.control.toggle_feature`).
 *
 * Fixture source: apcore/conformance/fixtures/overrides_store.json (canonical).
 * No `driver_contract` block; the `description` and the per-case descriptions
 * are the contract: file-backed persistence survives a reopen, startup applies
 * overrides *after* the base config (and leaves the base file alone), the
 * in-memory variant is behaviourally identical without disk I/O and shares no
 * state between instances, and a missing file on first run is tolerated.
 *
 * API SHAPE — decision D-47, the whole-map surface
 * ------------------------------------------------
 * The store is `load()` / `save(mapping)` in all three SDKs:
 *   - TS   src/sys-modules/overrides.ts:25-38         `load()` / `save(map)`
 *   - PY   src/apcore/sys_modules/overrides.py:38-53  `load()` / `save(dict)`
 *   - Rust src/sys_modules/overrides.rs:62-74         `load()` / `save(map)`
 * A single-key change is a read-modify-write, which is what the
 * `system.control.*` code paths do and what the fixture's `load_modify_save`
 * operation encodes.
 *
 * The fixture used to speak a per-key vocabulary — `save(key, value)`,
 * `get(key)`, `get_all()`, `delete(key)` — that no SDK implements, inherited
 * from a SHOULD sentence in features/system-modules.md. Three independent
 * implementations agreeing against one sentence is the sentence being wrong;
 * the spec repo resolved it in favour of D-47.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';

import { Config } from '../src/config.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import {
  FileOverridesStore,
  InMemoryOverridesStore,
  type OverridesStore,
} from '../src/sys-modules/overrides.js';
import { registerSysModules } from '../src/sys-modules/registration.js';
import { findFixturesRoot } from './spec-repo.js';

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

interface OverridesOp {
  readonly op: string;
  /** `load_modify_save`: keys to set in the loaded map before saving it back. */
  readonly set?: Record<string, unknown>;
  /** `load_modify_save`: keys to drop from the loaded map before saving it back. */
  readonly remove?: readonly string[];
}

interface OverridesCase {
  readonly id: string;
  readonly description: string;
  readonly input: {
    readonly store_type?: string;
    readonly operations?: readonly OverridesOp[];
    readonly base_config?: Record<string, unknown>;
    readonly overrides_file?: Record<string, unknown>;
    readonly path_exists_at_construction?: boolean;
  };
  readonly expected: Record<string, unknown>;
}

function loadFixture(name: string): { description: string; test_cases: readonly OverridesCase[] } {
  return JSON.parse(fs.readFileSync(path.join(findFixturesRoot(), `${name}.json`), 'utf-8'));
}

const fixture = loadFixture('overrides_store');

function caseById(id: string): OverridesCase {
  const tc = fixture.test_cases.find((c) => c.id === id);
  if (tc === undefined) {
    throw new Error(
      `overrides_store.json no longer contains case '${id}'. The fixture is canonical — ` +
        'update this driver to match it, do not edit the fixture.',
    );
  }
  return tc;
}

// ---------------------------------------------------------------------------
// Per-key operations expressed on the map-level store API (see header)
// ---------------------------------------------------------------------------

async function putKey(store: OverridesStore, key: string, value: unknown): Promise<void> {
  const all = await store.load();
  all[key] = value;
  await store.save(all);
}

async function getKey(store: OverridesStore, key: string): Promise<unknown> {
  const all = await store.load();
  return key in all ? all[key] : null;
}

async function deleteKey(store: OverridesStore, key: string): Promise<void> {
  const all = await store.load();
  delete all[key];
  await store.save(all);
}

// The fixture speaks the shipped D-47 surface: `load_modify_save` carries a
// whole-map edit, because `load()` / `save(mapping)` is all any SDK exposes.
// These extractors project one back onto the single key each case is about.

function settings(tc: OverridesCase): Array<[string, unknown]> {
  return (tc.input.operations ?? []).flatMap((o) => Object.entries(o.set ?? {}));
}

function firstSet(tc: OverridesCase): [string, unknown] {
  const all = settings(tc);
  if (all.length === 0) throw new Error(`[${tc.id}] fixture declares no load_modify_save set`);
  return all[0]!;
}

function removedKeys(tc: OverridesCase): string[] {
  return (tc.input.operations ?? []).flatMap((o) => [...(o.remove ?? [])]);
}

describe('Conformance: OverridesStore persistence (overrides_store.json)', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-conf-overrides-'));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    } catch {
      /* ignore */
    }
  });

  // -------------------------------------------------------------------------
  it('save_persists_override', async () => {
    const tc = caseById('save_persists_override');
    const storePath = path.join(tmpDir, 'overrides.yaml');
    const [key, value] = firstSet(tc);

    const store = new FileOverridesStore(storePath);
    await putKey(store, key, value);

    // "reopen_store": a brand-new instance over the same path.
    const reopened = new FileOverridesStore(storePath);
    expect(await getKey(reopened, key)).toEqual(tc.expected['value_after_reopen']);
  });

  // -------------------------------------------------------------------------
  it('startup_loads_overrides_after_base_config', () => {
    const tc = caseById('startup_loads_overrides_after_base_config');
    const basePath = path.join(tmpDir, 'apcore.yaml');
    const overridesPath = path.join(tmpDir, 'overrides.yaml');

    // Base config file, written in the nested form a real apcore.yaml uses.
    const baseYaml = [
      'version: "1.0"',
      'project:',
      '  name: conformance',
      'executor:',
      `  default_timeout: ${tc.input.base_config!['executor.default_timeout']}`,
      'sys_modules:',
      `  enabled: ${tc.input.base_config!['sys_modules.enabled']}`,
      '',
    ].join('\n');
    fs.writeFileSync(basePath, baseYaml, 'utf-8');
    const baseBytesBefore = fs.readFileSync(basePath, 'utf-8');

    // Overrides file, persisted through the store exactly as a prior run would.
    const store = new FileOverridesStore(overridesPath);
    store.save({ ...tc.input.overrides_file });

    const config = Config.load(basePath);
    const registry = new Registry();
    const executor = new Executor({ registry });
    registerSysModules(registry, executor, config, null, { overridesStore: store });

    for (const [key, value] of Object.entries(
      tc.expected['effective_config'] as Record<string, unknown>,
    )) {
      expect(config.get(key), `effective config for '${key}'`).toEqual(value);
    }

    const baseModified = fs.readFileSync(basePath, 'utf-8') !== baseBytesBefore;
    expect(baseModified).toBe(tc.expected['base_file_modified']);
  });

  // -------------------------------------------------------------------------
  it('inmemory_store_for_tests', async () => {
    const tc = caseById('inmemory_store_for_tests');
    const [key, value] = firstSet(tc);

    // `disk_writes` is measured by observing the filesystem: vitest cannot spy
    // on the `node:fs` ESM namespace ("Module namespace is not configurable in
    // ESM"), so instead every file the store could create is counted under a
    // sandbox directory. Control first — the file-backed store MUST register,
    // otherwise the `disk_writes: 0` assertion below would pass vacuously.
    const sandbox = path.join(tmpDir, 'sandbox');
    fs.mkdirSync(sandbox, { recursive: true });
    const filesIn = (dir: string): string[] =>
      fs
        .readdirSync(dir, { recursive: true, withFileTypes: true })
        .filter((e) => e.isFile())
        .map((e) => path.join(e.parentPath ?? dir, e.name));

    new FileOverridesStore(path.join(sandbox, 'control.yaml')).save({ probe: 1 });
    expect(
      filesIn(sandbox).length,
      'the file-backed store must be observable this way, otherwise disk_writes is unmeasurable',
    ).toBeGreaterThan(0);
    fs.rmSync(path.join(sandbox, 'control.yaml'));
    expect(filesIn(sandbox)).toEqual([]);

    const store = new InMemoryOverridesStore();
    await putKey(store, key, value);
    expect(await getKey(store, key)).toEqual(tc.expected['first_load_value']);

    // "new_store_instance": a second instance must share nothing.
    const second = new InMemoryOverridesStore();
    expect(await getKey(second, key)).toEqual(tc.expected['second_instance_load_value']);

    expect(filesIn(sandbox).length).toBe(tc.expected['disk_writes']);
    // `InMemoryOverridesStore` also takes no path, so it has nowhere on disk to
    // write in the first place — the sandbox count corroborates the API shape.
    expect('path' in store).toBe(false);
  });

  // -------------------------------------------------------------------------
  it('missing_path_first_run_ok', async () => {
    const tc = caseById('missing_path_first_run_ok');
    const storePath = path.join(tmpDir, 'nested', 'never-created.yaml');
    expect(fs.existsSync(storePath)).toBe(tc.input.path_exists_at_construction);

    const [key, value] = firstSet(tc);

    let constructionError: unknown = null;
    let store: FileOverridesStore | null = null;
    try {
      store = new FileOverridesStore(storePath);
    } catch (err) {
      constructionError = err;
    }
    expect(constructionError !== null).toBe(tc.expected['construction_raised_error']);

    expect(await store!.load()).toEqual(tc.expected['get_all_before_save']);

    await putKey(store!, key, value);
    expect(fs.existsSync(storePath)).toBe(tc.expected['path_exists_after_save']);
    expect(await store!.load()).toEqual({ [key]: value });
  });

  // -------------------------------------------------------------------------
  it('delete_removes_override', async () => {
    const tc = caseById('delete_removes_override');
    const storePath = path.join(tmpDir, 'overrides.yaml');
    const [key, value] = firstSet(tc);
    const removals = removedKeys(tc);

    const store = new FileOverridesStore(storePath);
    let error: unknown = null;
    try {
      await putKey(store, key, value);
      // Both removals run; the second targets an already-absent key.
      for (const removed of removals) {
        await deleteKey(store, removed);
      }
    } catch (err) {
      error = err;
    }

    expect(error !== null, `unexpected error: ${String(error)}`).toBe(tc.expected['raised_error']);
    expect(Object.keys(await store.load()).sort()).toEqual(tc.expected['get_all_keys']);
    // The removal is persisted, not merely forgotten in memory.
    expect(Object.keys(await new FileOverridesStore(storePath).load()).sort()).toEqual(
      tc.expected['get_all_keys'],
    );
  });

  // -------------------------------------------------------------------------
  it('drives every fixture case', () => {
    expect(fixture.test_cases.map((c) => c.id)).toEqual([
      'save_persists_override',
      'startup_loads_overrides_after_base_config',
      'inmemory_store_for_tests',
      'missing_path_first_run_ok',
      'delete_removes_override',
    ]);
  });
});
