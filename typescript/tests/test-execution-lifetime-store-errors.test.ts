import { describe, expect, it } from 'vitest';
import { AsyncTaskManager, InMemoryTaskStore, type TaskInfo } from '../src/async-task.js';
import type { Executor } from '../src/executor.js';

const turn = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

describe('shutdown ownership during store failure', () => {
  it.each([new Error('list failed'), undefined])('joins a worker before propagating list rejection %s', async (failure) => {
    let release!: () => void;
    let started!: () => void;
    const ready = new Promise<void>((resolve) => { started = resolve; });
    const body = new Promise<void>((resolve) => { release = resolve; });
    class FailingStore extends InMemoryTaskStore {
      fail = false;
      override async list(): Promise<TaskInfo[]> {
        if (this.fail) throw failure;
        return super.list();
      }
    }
    const store = new FailingStore();
    const executor = { call: async () => { started(); await body; return {}; } } as unknown as Executor;
    const manager = new AsyncTaskManager({ executor, store });
    await manager.submit('test.cleanup', {});
    await ready;
    store.fail = true;
    let stopped = false;
    const stopping = manager.shutdown().finally(() => { stopped = true; });
    const checked = expect(stopping).rejects.toBe(failure);
    try { await turn(); expect(stopped).toBe(false); }
    finally { release(); await checked; }
  });
});
