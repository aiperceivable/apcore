import { describe, it, expect, vi } from 'vitest';
import { computeFingerprint, ErrorHistory } from '../src/observability/error-history.js';
import { InMemoryStorageBackend, STORAGE_NAMESPACE_ERROR_HISTORY } from '../src/observability/storage.js';
import { InMemoryObservabilityStore } from '../src/observability/store.js';
import { ErrorHistoryMiddleware } from '../src/middleware/error-history.js';
import { ModuleError, ModuleExecuteError } from '../src/errors.js';
import { Context } from '../src/context.js';

describe('ErrorHistory', () => {
  it('mirrors only live fingerprints in the default backend and sink', async () => {
    const history = new ErrorHistory({ maxEntriesPerModule: 1, maxTotalEntries: 2 });
    const storage = history.storage;
    const old = new ModuleError('E1', 'old');
    const repeat = new ModuleError('E2', 'repeat');
    history.record('mod.a', old);
    for (let index = 0; index < 40; index++) history.record('mod.a', repeat);
    history.record('mod.b', new ModuleError('E3', 'second module'));
    history.record('mod.c', new ModuleError('E4', 'third module'));
    expect((await storage.list(STORAGE_NAMESPACE_ERROR_HISTORY)).map(([, row]) => row['module_id']).sort())
      .toEqual(['mod.b', 'mod.c']);
    expect(history.store.getErrors()).toEqual(history.getAll().reverse());

    history.clearModule('mod.b');
    expect(await storage.list(STORAGE_NAMESPACE_ERROR_HISTORY)).toHaveLength(1);
    expect(history.store.getErrors()).toMatchObject([{ moduleId: 'mod.c' }]);
    history.clear();
    await Promise.resolve();
    expect(await storage.list(STORAGE_NAMESPACE_ERROR_HISTORY)).toEqual([]);
    expect(history.store.getErrors()).toEqual([]);
  });

  it('leaves retention to an explicitly supplied backend and sink', () => {
    const storage = new InMemoryStorageBackend();
    const store = new InMemoryObservabilityStore();
    const history = new ErrorHistory({ storage, store, maxTotalEntries: 1 });
    history.record('mod.a', new ModuleError('E1', 'first'));
    history.record('mod.b', new ModuleError('E2', 'second'));
    expect(history.getAll()).toHaveLength(1);
    expect(storage.list(STORAGE_NAMESPACE_ERROR_HISTORY)).toHaveLength(2);
    expect(store.getErrors()).toHaveLength(2);
  });

  it('records and retrieves errors', () => {
    const history = new ErrorHistory();
    const error = new ModuleError('TEST_ERROR', 'test message');
    history.record('mod.a', error);
    const entries = history.get('mod.a');
    expect(entries).toHaveLength(1);
    expect(entries[0].code).toBe('TEST_ERROR');
    expect(entries[0].message).toBe('test message');
    expect(entries[0].count).toBe(1);
  });

  it('deduplicates by code and message', () => {
    const history = new ErrorHistory();
    const error = new ModuleError('TEST_ERROR', 'same message');
    history.record('mod.a', error);
    history.record('mod.a', error);
    const entries = history.get('mod.a');
    expect(entries).toHaveLength(1);
    expect(entries[0].count).toBe(2);
  });

  it('persists the latest count in order even when a previous storage write is still active', async () => {
    const memory = new InMemoryStorageBackend();
    let release!: () => void;
    const firstWrite = new Promise<void>(resolve => { release = resolve; });
    const save = vi.fn(async (namespace: string, key: string, value: Record<string, unknown>) => {
      if (value['count'] === 1) await firstWrite;
      memory.save(namespace, key, value);
    });
    const history = new ErrorHistory({ storage: {
      save,
      get: memory.get.bind(memory),
      list: memory.list.bind(memory),
      delete: memory.delete.bind(memory),
    } });
    const error = new ModuleError('E1', 'repeat');
    history.record('mod.a', error);
    history.record('mod.a', error);
    history.record('mod.a', error);
    expect(save).toHaveBeenCalledTimes(1);
    release();
    const key = computeFingerprint(error.code, 'mod.a', error.message);
    await vi.waitFor(() => expect(memory.get(STORAGE_NAMESPACE_ERROR_HISTORY, key)).toMatchObject({ count: 3 }));
  });

  it('returns newest first', () => {
    const history = new ErrorHistory();
    history.record('mod.a', new ModuleError('E1', 'first'));
    history.record('mod.a', new ModuleError('E2', 'second'));
    const entries = history.get('mod.a');
    expect(entries[0].code).toBe('E2');
    expect(entries[1].code).toBe('E1');
  });

  it('respects limit parameter', () => {
    const history = new ErrorHistory();
    for (let i = 0; i < 5; i++) {
      history.record('mod.a', new ModuleError(`E${i}`, `msg ${i}`));
    }
    expect(history.get('mod.a', 2)).toHaveLength(2);
  });

  it('evicts when per-module limit exceeded', () => {
    const history = new ErrorHistory({ maxEntriesPerModule: 3, maxTotalEntries: 1000 });
    for (let i = 0; i < 5; i++) {
      history.record('mod.a', new ModuleError(`E${i}`, `msg ${i}`));
    }
    expect(history.get('mod.a').length).toBeLessThanOrEqual(3);
  });

  it('evicts when total limit exceeded', () => {
    const history = new ErrorHistory({ maxEntriesPerModule: 50, maxTotalEntries: 3 });
    history.record('mod.a', new ModuleError('E1', 'msg'));
    history.record('mod.b', new ModuleError('E2', 'msg'));
    history.record('mod.c', new ModuleError('E3', 'msg'));
    history.record('mod.d', new ModuleError('E4', 'msg'));
    expect(history.getAll().length).toBeLessThanOrEqual(3);
  });

  it('returns empty for unknown module', () => {
    expect(new ErrorHistory().get('unknown')).toEqual([]);
  });

  it('clear removes all entries', () => {
    const history = new ErrorHistory();
    history.record('mod.a', new ModuleError('E1', 'msg'));
    history.record('mod.b', new ModuleError('E2', 'msg'));
    history.clear();
    expect(history.getAll()).toEqual([]);
    expect(history.get('mod.a')).toEqual([]);
    expect(history.get('mod.b')).toEqual([]);
  });

  it('clearModule removes only that module', () => {
    const history = new ErrorHistory();
    history.record('mod.a', new ModuleError('E1', 'msg'));
    history.record('mod.b', new ModuleError('E2', 'msg'));
    history.clearModule('mod.a');
    expect(history.get('mod.a')).toEqual([]);
    expect(history.get('mod.b')).toHaveLength(1);
  });

  it('getAll returns across modules sorted by lastOccurred desc', () => {
    const history = new ErrorHistory();
    history.record('mod.a', new ModuleError('E1', 'msg1'));
    history.record('mod.b', new ModuleError('E2', 'msg2'));
    const all = history.getAll();
    expect(all.length).toBe(2);
    expect(all[0].lastOccurred >= all[1].lastOccurred).toBe(true);
  });

  it('getAll respects limit argument', () => {
    const history = new ErrorHistory();
    for (let i = 0; i < 5; i++) {
      history.record(`mod.${i}`, new ModuleError(`E${i}`, `msg ${i}`));
    }
    expect(history.getAll(3)).toHaveLength(3);
  });
});

describe('ErrorHistoryMiddleware', () => {
  it('records ModuleError on onError', () => {
    const history = new ErrorHistory();
    const mw = new ErrorHistoryMiddleware(history);
    const ctx = Context.create();
    mw.onError('mod.a', {}, new ModuleExecuteError('mod.a', 'boom'), ctx);
    const entries = history.get('mod.a');
    expect(entries).toHaveLength(1);
    expect(entries[0].code).toBe('MODULE_EXECUTE_ERROR');
  });

  it('ignores non-ModuleError exceptions', () => {
    const history = new ErrorHistory();
    const mw = new ErrorHistoryMiddleware(history);
    mw.onError('mod.a', {}, new Error('generic'), Context.create());
    expect(history.get('mod.a')).toHaveLength(0);
  });

  it('returns null (never recovers)', () => {
    const history = new ErrorHistory();
    const mw = new ErrorHistoryMiddleware(history);
    expect(mw.onError('mod.a', {}, new ModuleError('E', 'msg'), Context.create())).toBeNull();
  });
});
