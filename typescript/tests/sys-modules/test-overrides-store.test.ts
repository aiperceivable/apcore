/**
 * OverridesStore — pluggable persistence for runtime config/toggle overrides.
 *
 * Mirrors the Python `_write_overrides`/`_load_overrides` and Rust
 * `load_overrides`/`write_override` flows, but exposed as a swappable
 * interface so tests and embeddings can inject in-memory storage.
 */

import { afterEach, beforeEach, describe, it, expect, vi } from 'vitest';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import {
  FileOverridesStore,
  InMemoryOverridesStore,
  type OverridesStore,
} from '../../src/sys-modules/overrides.js';
import { Config } from '../../src/config.js';
import { Registry } from '../../src/registry/registry.js';
import { Executor } from '../../src/executor.js';
import { EventEmitter } from '../../src/events/index.js';
import { UpdateConfigModule } from '../../src/sys-modules/control.js';
import { Context, createIdentity } from '../../src/context.js';
import { registerSysModules } from '../../src/sys-modules/registration.js';

describe('InMemoryOverridesStore', () => {
  it('round-trips load/save', async () => {
    const store: OverridesStore = new InMemoryOverridesStore();
    expect(await store.load()).toEqual({});
    await store.save({ 'app.foo': 1, 'app.bar': 'x' });
    expect(await store.load()).toEqual({ 'app.foo': 1, 'app.bar': 'x' });
  });

  it('save replaces previous content', async () => {
    const store = new InMemoryOverridesStore();
    await store.save({ a: 1 });
    await store.save({ b: 2 });
    expect(await store.load()).toEqual({ b: 2 });
  });
});

describe('FileOverridesStore', () => {
  let tmpDir: string;
  let tmpPath: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'apcore-overrides-'));
    tmpPath = path.join(tmpDir, 'overrides.yaml');
  });

  afterEach(() => {
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    } catch {
      /* ignore */
    }
  });

  it('persists across instances', async () => {
    const store1 = new FileOverridesStore(tmpPath);
    await store1.save({ 'app.timeout': 30, 'app.name': 'demo' });

    const store2 = new FileOverridesStore(tmpPath);
    const loaded = await store2.load();
    expect(loaded).toEqual({ 'app.timeout': 30, 'app.name': 'demo' });
  });

  it('returns empty object when file does not exist', async () => {
    const store = new FileOverridesStore(path.join(tmpDir, 'absent.yaml'));
    expect(await store.load()).toEqual({});
  });

  it('returns empty object and warns when file contents are invalid YAML', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    fs.writeFileSync(tmpPath, ': : not valid yaml :\n   foo: [unclosed', 'utf-8');
    const store = new FileOverridesStore(tmpPath);
    expect(await store.load()).toEqual({});
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('returns empty object and warns when YAML root is not a mapping', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    fs.writeFileSync(tmpPath, '- a\n- b\n', 'utf-8');
    const store = new FileOverridesStore(tmpPath);
    expect(await store.load()).toEqual({});
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it('returns empty object on empty file content', async () => {
    fs.writeFileSync(tmpPath, '', 'utf-8');
    const store = new FileOverridesStore(tmpPath);
    expect(await store.load()).toEqual({});
  });

  it('exposes the configured path via the path getter', () => {
    const store = new FileOverridesStore(tmpPath);
    expect(store.path).toBe(tmpPath);
  });

  it('save() logs and aborts when the parent directory cannot be created', async () => {
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    // Plant a regular file where the parent directory should be — mkdirSync
    // (recursive) then fails with ENOTDIR/EEXIST.
    const blockerPath = path.join(tmpDir, 'blocker');
    fs.writeFileSync(blockerPath, 'plain file', 'utf-8');
    const conflictPath = path.join(blockerPath, 'overrides.yaml');

    const store = new FileOverridesStore(conflictPath);
    await store.save({ a: 1 });

    expect(errSpy).toHaveBeenCalled();
    // Original blocker file is untouched.
    expect(fs.readFileSync(blockerPath, 'utf-8')).toBe('plain file');
    errSpy.mockRestore();
  });

  it('save() logs when the rename target is an existing directory and cleans up the tempfile', async () => {
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    // Make the override path itself a populated directory so rename fails.
    fs.mkdirSync(tmpPath, { recursive: true });
    fs.writeFileSync(path.join(tmpPath, 'inside.txt'), 'x', 'utf-8');

    const store = new FileOverridesStore(tmpPath);
    await store.save({ a: 1 });

    expect(errSpy).toHaveBeenCalled();
    // No leftover .tmp files should remain in the parent dir.
    const leftovers = fs.readdirSync(tmpDir).filter(n => n.endsWith('.tmp'));
    expect(leftovers).toEqual([]);
    errSpy.mockRestore();
  });
});

describe('UpdateConfigModule writes to OverridesStore', () => {
  it('persists each successful update via the injected store', async () => {
    const store = new InMemoryOverridesStore();
    const config = new Config({});
    const emitter = new EventEmitter();
    const mod = new UpdateConfigModule(config, emitter, { overridesStore: store });

    const ctx = new Context('t', 'caller', [], null, createIdentity('user', 'user', []));
    mod.execute({ key: 'app.timeout', value: 60, reason: 'tune' }, ctx);
    mod.execute({ key: 'app.retries', value: 3, reason: 'tune' }, ctx);

    expect(await store.load()).toEqual({ 'app.timeout': 60, 'app.retries': 3 });
  });
});

describe('registerSysModules applies persisted overrides on startup', () => {
  it('loads from store and applies before registering modules', () => {
    const store = new InMemoryOverridesStore();
    // Pre-populate as if a previous run had written these.
    void store.save({ 'app.feature_x': true, 'app.threshold': 99 });

    const config = new Config({});
    config.set('sys_modules.enabled', true);
    const registry = new Registry();
    const executor = new Executor({ registry });

    registerSysModules(registry, executor, config, null, { overridesStore: store });

    expect(config.get('app.feature_x')).toBe(true);
    expect(config.get('app.threshold')).toBe(99);
  });
});
