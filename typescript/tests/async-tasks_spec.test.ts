/**
 * Spec-traced contract tests for the async-tasks feature (TypeScript SDK).
 *
 * MIRRORS the canonical Python suite
 * `apcore-python/tests/test_async_tasks_spec.py` clause-for-clause. Every test
 * name begins with the verbatim clause id (form
 * `async_tasks.<method>.<kind>.<detail>`) so a cross-language diff lines up
 * row-for-row.
 *
 * These tests assert the ACTUAL TypeScript behavior, which diverges from the
 * Python canonical intent in several places (recorded in DIVERGENCES):
 *  - The TS read methods getStatus / getResult / listTasks / cleanup are all
 *    `async` (return Promises) per spec D10-003 / D-17, unlike Python's sync
 *    versions.
 *  - Module-id format rejection raises InvalidInputError with code
 *    INVALID_MODULE_ID (aligned with apcore-python).
 *  - get_result raises a plain Error (not KeyError / RuntimeError subtypes).
 *  - TaskInfo is a readonly interface, not a mutable dataclass — the
 *    shallow-copy contract (D-23) is observed by mutating a cast copy.
 *
 * Framework: vitest.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import {
  AsyncTaskManager,
  TaskStatus,
  InMemoryTaskStore,
  type TaskInfo,
} from '../src/async-task.js';
import { Executor } from '../src/executor.js';
import { FunctionModule } from '../src/decorator.js';
import { Registry } from '../src/registry/registry.js';
import {
  InvalidInputError,
  ModuleError,
  TaskLimitExceededError,
  TaskStoreError,
} from '../src/errors.js';

// === Helper modules ===

function createRegistry(): Registry {
  const registry = new Registry();

  const echoModule = new FunctionModule({
    execute: (inputs) => ({ value: (inputs['x'] as number) ?? 0 }),
    moduleId: 'test.echo',
    inputSchema: Type.Object({ x: Type.Optional(Type.Number()) }),
    outputSchema: Type.Object({ value: Type.Number() }),
    description: 'Echo module',
  });

  const slowModule = new FunctionModule({
    execute: async (inputs) => {
      const ms = ((inputs['delay'] as number) ?? 0.5) * 1000;
      await new Promise((resolve) => setTimeout(resolve, ms));
      return { done: true };
    },
    moduleId: 'test.slow',
    inputSchema: Type.Object({ delay: Type.Optional(Type.Number()) }),
    outputSchema: Type.Object({ done: Type.Boolean() }),
    description: 'Slow module',
  });

  const failingModule = new FunctionModule({
    execute: () => {
      throw new Error('intentional failure');
    },
    moduleId: 'test.failing',
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({}),
    description: 'Failing module',
  });

  registry.register('test.echo', echoModule);
  registry.register('test.slow', slowModule);
  registry.register('test.failing', failingModule);
  return registry;
}

function createManager(
  opts: { maxConcurrent?: number; maxTasks?: number } = {},
): { manager: AsyncTaskManager; executor: Executor } {
  const registry = createRegistry();
  const executor = new Executor({ registry });
  const manager = new AsyncTaskManager({
    executor,
    maxConcurrent: opts.maxConcurrent ?? 10,
    maxTasks: opts.maxTasks ?? 1000,
  });
  return { manager, executor };
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const TERMINAL = new Set([
  TaskStatus.COMPLETED,
  TaskStatus.FAILED,
  TaskStatus.CANCELLED,
]);

/** Await until `taskId` reaches a terminal state and return its snapshot. */
async function drain(
  manager: AsyncTaskManager,
  taskId: string,
  timeoutMs = 2000,
): Promise<TaskInfo> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const info = await manager.getStatus(taskId);
    if (info && TERMINAL.has(info.status)) return info;
    await wait(10);
  }
  throw new Error(`task ${taskId} did not reach a terminal state within ${timeoutMs}ms`);
}

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.submit
// ---------------------------------------------------------------------------

describe('AsyncTaskManager.submit', () => {
  it('async_tasks.submit.input.module_id.malformed: malformed module_id drives task to FAILED with module-id error', async () => {
    const { manager, executor } = createManager();
    const taskId = await manager.submit('Bad-ID!', { x: 1 });
    const info = await drain(manager, taskId);
    expect(info.status).toBe(TaskStatus.FAILED);
    expect(info.error).not.toBeNull();
    expect(info.error).toContain('Invalid module ID');

    // Direct validation path confirms the declared TYPE + CODE pairing.
    // DIVERGENCE: TS uses Executor.call() (the private _validateModuleId is not
    // exposed like Python's static Executor._validate_module_id). The code is
    // INVALID_MODULE_ID (aligned with apcore-python).
    await expect(executor.call('Bad-ID!', { x: 1 })).rejects.toBeInstanceOf(
      InvalidInputError,
    );
    try {
      await executor.call('Bad-ID!', { x: 1 });
      throw new Error('expected InvalidInputError');
    } catch (err) {
      expect(err).toBeInstanceOf(InvalidInputError);
      expect((err as InvalidInputError).code).toBe('INVALID_MODULE_ID');
    }
    await manager.shutdown();
  });

  it('async_tasks.submit.error.MODULE_NOT_FOUND: unregistered module_id drives task to FAILED', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('no.such.module', { x: 1 });
    const info = await drain(manager, taskId);
    expect(info.status).toBe(TaskStatus.FAILED);
    expect(info.error).not.toBeNull();
    expect(info.error).toContain('no.such.module');
    await manager.shutdown();
  });

  it('async_tasks.submit.error.TASK_LIMIT_EXCEEDED: second active submission over cap raises TaskLimitExceededError(code=TASK_LIMIT_EXCEEDED)', async () => {
    const { manager } = createManager({ maxConcurrent: 10, maxTasks: 1 });
    await manager.submit('test.slow', { delay: 0.5 });
    let caught: unknown;
    try {
      await manager.submit('test.slow', { delay: 0.5 });
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(TaskLimitExceededError);
    expect((caught as TaskLimitExceededError).code).toBe('TASK_LIMIT_EXCEEDED');
    await manager.shutdown();
  });

  it('async_tasks.submit.property.async: submit returns a Promise resolving to a string task_id', async () => {
    const { manager } = createManager();
    const p = manager.submit('test.echo', { x: 7 });
    expect(p).toBeInstanceOf(Promise);
    const taskId = await p;
    expect(typeof taskId).toBe('string');
    expect(taskId.length).toBeGreaterThan(0);
    const info = await drain(manager, taskId);
    expect(info.status).toBe(TaskStatus.COMPLETED);
    await manager.shutdown();
  });

  it('async_tasks.submit.property.thread_safe: >=8 concurrent submissions produce distinct ids and all complete', async () => {
    const { manager } = createManager();
    const n = 12;
    const taskIds = await Promise.all(
      Array.from({ length: n }, (_, i) => manager.submit('test.echo', { x: i })),
    );
    expect(new Set(taskIds).size).toBe(n);
    const infos = await Promise.all(taskIds.map((tid) => drain(manager, tid)));
    expect(infos.every((info) => info.status === TaskStatus.COMPLETED)).toBe(true);
    const values = new Set(infos.map((info) => (info.result as { value: number }).value));
    expect(values).toEqual(new Set(Array.from({ length: n }, (_, i) => i)));
    await manager.shutdown();
  });

  it('async_tasks.submit.property.idempotent_false: two identical submits create two distinct tracked tasks', async () => {
    const { manager } = createManager();
    const first = await manager.submit('test.echo', { x: 1 });
    const second = await manager.submit('test.echo', { x: 1 });
    expect(first).not.toBe(second);
    const ids = new Set((await manager.listTasks()).map((t) => t.taskId));
    expect(ids.has(first)).toBe(true);
    expect(ids.has(second)).toBe(true);
    await manager.shutdown();
  });
});

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.cancel
// ---------------------------------------------------------------------------

describe('AsyncTaskManager.cancel', () => {
  it('async_tasks.cancel.property.async: cancel returns a Promise resolving to a bool', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.slow', { delay: 1.0 });
    const p = manager.cancel(taskId);
    expect(p).toBeInstanceOf(Promise);
    const result = await p;
    expect(result).toBe(true);
    const info = await manager.getStatus(taskId);
    expect(info).not.toBeNull();
    expect(info!.status).toBe(TaskStatus.CANCELLED);
    await manager.shutdown();
  });

  it('async_tasks.cancel.return.unknown_task_false: cancelling a non-existent task returns false', async () => {
    const { manager } = createManager();
    const result = await manager.cancel('does-not-exist');
    expect(result).toBe(false);
    await manager.shutdown();
  });

  it('async_tasks.cancel.property.thread_safe: concurrently cancelling >=8 running tasks leaves all CANCELLED', async () => {
    const { manager } = createManager({ maxConcurrent: 20, maxTasks: 1000 });
    const n = 8;
    const taskIds: string[] = [];
    for (let i = 0; i < n; i++) {
      taskIds.push(await manager.submit('test.slow', { delay: 1.0 }));
    }
    await wait(50); // let them reach RUNNING
    const results = await Promise.all(taskIds.map((tid) => manager.cancel(tid)));
    expect(results.every((r) => r === true)).toBe(true);
    for (const tid of taskIds) {
      const info = await manager.getStatus(tid);
      expect(info).not.toBeNull();
      expect(info!.status).toBe(TaskStatus.CANCELLED);
    }
    await manager.shutdown();
  });

  it('async_tasks.cancel.property.idempotent: first cancel returns true, second returns false, state stays CANCELLED', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.slow', { delay: 1.0 });
    await wait(20);
    const first = await manager.cancel(taskId);
    const second = await manager.cancel(taskId);
    expect(first).toBe(true);
    expect(second).toBe(false);
    const info = await manager.getStatus(taskId);
    expect(info).not.toBeNull();
    expect(info!.status).toBe(TaskStatus.CANCELLED);
    await manager.shutdown();
  });
});

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.get_status
// ---------------------------------------------------------------------------

describe('AsyncTaskManager.getStatus', () => {
  it('async_tasks.get_status.property.async_false: TS getStatus is async (returns a Promise) — DIVERGES from Python sync contract (D10-003)', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.echo', { x: 1 });
    const p = manager.getStatus(taskId);
    // DIVERGENCE: Python get_status is synchronous; TS returns a Promise.
    expect(p).toBeInstanceOf(Promise);
    const info = await p;
    expect(info).not.toBeNull();
    expect(info!.taskId).toBe(taskId);
    await manager.shutdown();
  });

  it('async_tasks.get_status.return.shallow_copy: mutating the returned snapshot does not propagate to the store (D-23)', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.echo', { x: 1 });
    const info = await manager.getStatus(taskId);
    expect(info).not.toBeNull();
    // TaskInfo is readonly; cast to mutate the returned copy.
    (info as { moduleId: string }).moduleId = 'tampered';
    const again = await manager.getStatus(taskId);
    expect(again).not.toBeNull();
    expect(again!.moduleId).toBe('test.echo');
    await manager.shutdown();
  });

  it('async_tasks.get_status.property.idempotent: repeated reads return equal snapshots', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.echo', { x: 9 });
    await drain(manager, taskId);
    const a = await manager.getStatus(taskId);
    const b = await manager.getStatus(taskId);
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();
    expect([a!.taskId, a!.status, a!.result]).toEqual([b!.taskId, b!.status, b!.result]);
    await manager.shutdown();
  });

  it('async_tasks.get_status.return.unknown_none: unknown task_id returns null', async () => {
    const { manager } = createManager();
    expect(await manager.getStatus('nope')).toBeNull();
    await manager.shutdown();
  });
});

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.get_result
// ---------------------------------------------------------------------------

describe('AsyncTaskManager.getResult', () => {
  it('async_tasks.get_result.error.task_not_found: getResult rejects when no task exists', async () => {
    const { manager } = createManager();
    // DIVERGENCE: Python raises KeyError; TS rejects with a plain Error
    // ("Task not found: <id>").
    await expect(manager.getResult('missing')).rejects.toThrow(/not found/i);
    await manager.shutdown();
  });

  it('async_tasks.get_result.error.not_completed: getResult rejects when task exists but is not COMPLETED', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.slow', { delay: 1.0 });
    await wait(20);
    // DIVERGENCE: Python raises RuntimeError; TS rejects with a plain Error
    // ("Task <id> is not completed (status=<value>)").
    await expect(manager.getResult(taskId)).rejects.toThrow(/not completed/i);
    await manager.cancel(taskId);
    await manager.shutdown();
  });

  it('async_tasks.get_result.return.completed_result: getResult returns module output once COMPLETED', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.echo', { x: 42 });
    await drain(manager, taskId);
    expect(await manager.getResult(taskId)).toEqual({ value: 42 });
    await manager.shutdown();
  });

  it('async_tasks.get_result.property.idempotent: two getResult calls on a completed task return identical output', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.echo', { x: 5 });
    await drain(manager, taskId);
    const a = await manager.getResult(taskId);
    const b = await manager.getResult(taskId);
    expect(a).toEqual({ value: 5 });
    expect(b).toEqual({ value: 5 });
    await manager.shutdown();
  });
});

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.list_tasks
// ---------------------------------------------------------------------------

describe('AsyncTaskManager.listTasks', () => {
  it('async_tasks.list_tasks.input.status.filter: only tasks with the exact supplied status are returned', async () => {
    const { manager } = createManager();
    const completedId = await manager.submit('test.echo', { x: 1 });
    await drain(manager, completedId);
    const runningId = await manager.submit('test.slow', { delay: 1.0 });
    await wait(20);
    const completed = await manager.listTasks(TaskStatus.COMPLETED);
    expect(completed.map((t) => t.taskId)).toEqual([completedId]);
    const running = await manager.listTasks(TaskStatus.RUNNING);
    expect(new Set(running.map((t) => t.taskId)).has(runningId)).toBe(true);
    await manager.cancel(runningId);
    await manager.shutdown();
  });

  it('async_tasks.list_tasks.return.shallow_copy: mutating a listed entry does not affect the stored task (D-23)', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.echo', { x: 1 });
    await drain(manager, taskId);
    const listed = await manager.listTasks();
    expect(listed.length).toBeGreaterThan(0);
    (listed[0] as { moduleId: string }).moduleId = 'tampered';
    const again = await manager.getStatus(taskId);
    expect(again).not.toBeNull();
    expect(again!.moduleId).toBe('test.echo');
    await manager.shutdown();
  });

  it('async_tasks.list_tasks.property.idempotent: repeated listTasks return the same set of ids', async () => {
    const { manager } = createManager();
    const ids: string[] = [];
    for (let i = 0; i < 3; i++) {
      ids.push(await manager.submit('test.echo', { x: i }));
    }
    for (const tid of ids) await drain(manager, tid);
    const first = new Set((await manager.listTasks()).map((t) => t.taskId));
    const second = new Set((await manager.listTasks()).map((t) => t.taskId));
    expect(first).toEqual(second);
    expect(first).toEqual(new Set(ids));
    await manager.shutdown();
  });
});

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.cleanup
// ---------------------------------------------------------------------------

describe('AsyncTaskManager.cleanup', () => {
  it('async_tasks.cleanup.eligible.terminal_only: cleanup removes eligible terminal tasks and never PENDING/RUNNING', async () => {
    const { manager } = createManager();
    const doneId = await manager.submit('test.echo', { x: 1 });
    await drain(manager, doneId);
    const runningId = await manager.submit('test.slow', { delay: 1.0 });
    await wait(20);
    // maxAgeSeconds=0 makes every terminal task eligible immediately.
    const removed = await manager.cleanup(0);
    expect(removed).toBe(1);
    expect(await manager.getStatus(doneId)).toBeNull();
    expect(await manager.getStatus(runningId)).not.toBeNull();
    await manager.cancel(runningId);
    await manager.shutdown();
  });

  it('async_tasks.cleanup.property.idempotent_false: first cleanup removes 1, second removes 0', async () => {
    const { manager } = createManager();
    const doneId = await manager.submit('test.echo', { x: 1 });
    await drain(manager, doneId);
    const first = await manager.cleanup(0);
    const second = await manager.cleanup(0);
    expect(first).toBe(1);
    expect(second).toBe(0);
    await manager.shutdown();
  });
});

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.shutdown
// ---------------------------------------------------------------------------

describe('AsyncTaskManager.shutdown', () => {
  it('async_tasks.shutdown.property.async: shutdown returns a Promise resolving to void', async () => {
    const { manager } = createManager();
    const p = manager.shutdown();
    expect(p).toBeInstanceOf(Promise);
    expect(await p).toBeUndefined();
  });

  it('async_tasks.shutdown.side_effect.1.cancel_active: after shutdown every PENDING/RUNNING task is CANCELLED', async () => {
    const { manager } = createManager({ maxConcurrent: 20, maxTasks: 1000 });
    const ids: string[] = [];
    for (let i = 0; i < 8; i++) {
      ids.push(await manager.submit('test.slow', { delay: 1.0 }));
    }
    await wait(50);
    await manager.shutdown();
    for (const tid of ids) {
      const info = await manager.getStatus(tid);
      expect(info).not.toBeNull();
      expect(info!.status).toBe(TaskStatus.CANCELLED);
    }
  });

  it('async_tasks.shutdown.property.idempotent: calling shutdown twice is a no-op, state unchanged', async () => {
    const { manager } = createManager();
    const taskId = await manager.submit('test.slow', { delay: 1.0 });
    await wait(50);
    await manager.shutdown();
    const before = await manager.getStatus(taskId);
    await manager.shutdown();
    const after = await manager.getStatus(taskId);
    expect(before).not.toBeNull();
    expect(after).not.toBeNull();
    expect(before!.status).toBe(TaskStatus.CANCELLED);
    expect(after!.status).toBe(TaskStatus.CANCELLED);
  });
});

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.start_reaper
// ---------------------------------------------------------------------------

describe('AsyncTaskManager.startReaper', () => {
  it('async_tasks.start_reaper.property.async: startReaper returns a ReaperHandle whose stop() is awaitable', async () => {
    const { manager } = createManager();
    // Spec D-11: startReaper() -> Promise<ReaperHandle>, uniform with Python's
    // `await manager.start_reaper(...)` and Rust's `.start_reaper(...).await`.
    // There is no is_running() on the TS ReaperHandle — only stop(). We assert
    // stop() is awaitable.
    const handle = await manager.startReaper({ ttlSeconds: 3600, sweepIntervalMs: 300000 });
    expect(typeof handle.stop).toBe('function');
    const stopP = handle.stop();
    expect(stopP).toBeInstanceOf(Promise);
    await stopP;
    await manager.shutdown();
  });

  it('async_tasks.start_reaper.property.idempotent_false: starting a second reaper while one runs throws ModuleError(REAPER_ALREADY_RUNNING)', async () => {
    const { manager } = createManager();
    const handle = await manager.startReaper({ ttlSeconds: 3600, sweepIntervalMs: 300000 });
    // The idempotency guard still throws SYNCHRONOUSLY (before any Promise is
    // created) — startReaper() is not declared `async`, precisely so this
    // guard is not silently turned into an unhandled rejection for a caller
    // who does not await the second call.
    let caught: unknown;
    try {
      manager.startReaper({ ttlSeconds: 3600, sweepIntervalMs: 300000 });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ModuleError);
    expect((caught as ModuleError).code).toBe('REAPER_ALREADY_RUNNING');
    await handle.stop();
    await manager.shutdown();
  });
});

// ---------------------------------------------------------------------------
// TaskInfo factory for store-level tests (TaskInfo is a plain interface in TS).
// ---------------------------------------------------------------------------

function makeTaskInfo(
  taskId: string,
  moduleId: string,
  status: TaskStatus,
  extra: Partial<TaskInfo> = {},
): TaskInfo {
  return {
    taskId,
    moduleId,
    status,
    submittedAt: extra.submittedAt ?? 1.0,
    startedAt: extra.startedAt ?? null,
    completedAt: extra.completedAt ?? null,
    result: extra.result ?? null,
    error: extra.error ?? null,
    retryCount: extra.retryCount ?? 0,
    maxRetries: extra.maxRetries ?? 0,
  };
}

// ---------------------------------------------------------------------------
// Contract: TaskStore.save
// ---------------------------------------------------------------------------

describe('TaskStore.save', () => {
  it('async_tasks.save.property.async: save returns a Promise resolving to void and persists the record', async () => {
    const store = new InMemoryTaskStore();
    const info = makeTaskInfo('t1', 'test.echo', TaskStatus.PENDING, {
      submittedAt: Date.now() / 1000,
    });
    const p = store.save(info);
    expect(p).toBeInstanceOf(Promise);
    expect(await p).toBeUndefined();
    expect(await store.get('t1')).not.toBeNull();
  });

  it('async_tasks.save.property.idempotent: saving twice with the same task_id overwrites — one record remains', async () => {
    const store = new InMemoryTaskStore();
    await store.save(makeTaskInfo('t1', 'test.echo', TaskStatus.PENDING));
    await store.save(makeTaskInfo('t1', 'test.echo', TaskStatus.COMPLETED));
    const all = await store.list();
    expect(all.length).toBe(1);
    const stored = await store.get('t1');
    expect(stored).not.toBeNull();
    expect(stored!.status).toBe(TaskStatus.COMPLETED);
  });

  it('async_tasks.save.error.TASK_STORE_UNAVAILABLE: the canonical type exists and carries its code', async () => {
    // This was `it.skip`ped with the reason "missing symbol
    // TaskStoreError/TASK_STORE_UNAVAILABLE (contract gap)" — true when it was
    // written, and made false by D-92, which required all three SDKs to define
    // and export the type. Nothing turned red when the gap closed, because a
    // disabled test explaining why something cannot be tested keeps explaining
    // it after it can.
    const store = new InMemoryTaskStore();

    // The bundled in-memory store cannot fail, so it must not raise it.
    await expect(
      store.save(makeTaskInfo('t1', 'test.echo', TaskStatus.PENDING)),
    ).resolves.toBeUndefined();

    // A network-backed store has one canonical type to raise, and a caller has
    // one type to catch.
    const err = new TaskStoreError('save', 'connection refused');
    expect(err).toBeInstanceOf(ModuleError);
    expect(err.code).toBe('TASK_STORE_UNAVAILABLE');
    expect(err.message).toContain('save');
    expect(err.message).toContain('connection refused');
  });
});

// ---------------------------------------------------------------------------
// Contract: TaskStore.get
// ---------------------------------------------------------------------------

describe('TaskStore.get', () => {
  it('async_tasks.get.property.async: get returns a Promise resolving to the stored record', async () => {
    const store = new InMemoryTaskStore();
    await store.save(makeTaskInfo('g1', 'test.echo', TaskStatus.PENDING, {
      submittedAt: Date.now() / 1000,
    }));
    const p = store.get('g1');
    expect(p).toBeInstanceOf(Promise);
    const fetched = await p;
    expect(fetched).not.toBeNull();
    expect(fetched!.taskId).toBe('g1');
  });

  it('async_tasks.get.property.idempotent: two gets return equal records and never mutate the store', async () => {
    const store = new InMemoryTaskStore();
    await store.save(makeTaskInfo('g1', 'test.echo', TaskStatus.PENDING));
    const a = await store.get('g1');
    const b = await store.get('g1');
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();
    expect(a!.taskId).toBe(b!.taskId);
    expect((await store.list()).length).toBe(1);
  });

  it('async_tasks.get.return.unknown_none: unknown task_id returns null; in-memory store never throws', async () => {
    const store = new InMemoryTaskStore();
    expect(await store.get('absent')).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Contract: TaskStore.list
// ---------------------------------------------------------------------------

describe('TaskStore.list', () => {
  it('async_tasks.list.property.async: list returns a Promise resolving to an array of records', async () => {
    const store = new InMemoryTaskStore();
    await store.save(makeTaskInfo('l1', 'test.echo', TaskStatus.PENDING));
    const p = store.list();
    expect(p).toBeInstanceOf(Promise);
    const items = await p;
    expect(items.map((i) => i.taskId)).toEqual(['l1']);
  });

  it('async_tasks.list.input.status.filter: only matching records are returned when a status is supplied', async () => {
    const store = new InMemoryTaskStore();
    await store.save(makeTaskInfo('l1', 'm', TaskStatus.PENDING));
    await store.save(makeTaskInfo('l2', 'm', TaskStatus.COMPLETED));
    const done = await store.list(TaskStatus.COMPLETED);
    expect(done.map((i) => i.taskId)).toEqual(['l2']);
  });

  it('async_tasks.list.property.idempotent: repeated list calls return the same ids', async () => {
    const store = new InMemoryTaskStore();
    await store.save(makeTaskInfo('l1', 'm', TaskStatus.PENDING));
    await store.save(makeTaskInfo('l2', 'm', TaskStatus.PENDING));
    const a = new Set((await store.list()).map((i) => i.taskId));
    const b = new Set((await store.list()).map((i) => i.taskId));
    expect(a).toEqual(b);
    expect(a).toEqual(new Set(['l1', 'l2']));
  });
});

// ---------------------------------------------------------------------------
// Contract: TaskStore.delete
// ---------------------------------------------------------------------------

describe('TaskStore.delete', () => {
  it('async_tasks.delete.property.async: delete returns a Promise resolving to void and removes the record', async () => {
    const store = new InMemoryTaskStore();
    await store.save(makeTaskInfo('d1', 'm', TaskStatus.COMPLETED));
    const p = store.delete('d1');
    expect(p).toBeInstanceOf(Promise);
    expect(await p).toBeUndefined();
    expect(await store.get('d1')).toBeNull();
  });

  it('async_tasks.delete.property.idempotent: deleting an already-absent task_id succeeds silently', async () => {
    const store = new InMemoryTaskStore();
    await store.save(makeTaskInfo('d1', 'm', TaskStatus.COMPLETED));
    await store.delete('d1');
    await store.delete('d1'); // second delete is a silent no-op
    expect(await store.get('d1')).toBeNull();
    expect(await store.list()).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// Contract: TaskStore.list_expired
// ---------------------------------------------------------------------------

describe('TaskStore.listExpired', () => {
  it('async_tasks.list_expired.property.async: listExpired returns a Promise resolving to an array', async () => {
    const store = new InMemoryTaskStore();
    await store.save(
      makeTaskInfo('e1', 'm', TaskStatus.COMPLETED, { completedAt: 10.0 }),
    );
    const p = store.listExpired(100.0);
    expect(p).toBeInstanceOf(Promise);
    const expired = await p;
    expect(expired.map((i) => i.taskId)).toEqual(['e1']);
  });

  it('async_tasks.list_expired.eligible.terminal_only: only terminal tasks with completed_at < before are returned', async () => {
    const store = new InMemoryTaskStore();
    await store.save(
      makeTaskInfo('done', 'm', TaskStatus.COMPLETED, { completedAt: 10.0 }),
    );
    await store.save(makeTaskInfo('pending', 'm', TaskStatus.PENDING));
    await store.save(
      makeTaskInfo('running', 'm', TaskStatus.RUNNING, { startedAt: 2.0 }),
    );
    const expired = await store.listExpired(100.0);
    expect(expired.map((i) => i.taskId)).toEqual(['done']);
  });

  it('async_tasks.list_expired.input.before_timestamp.strict: expiry is strict (completed_at < before)', async () => {
    const store = new InMemoryTaskStore();
    await store.save(
      makeTaskInfo('eq', 'm', TaskStatus.COMPLETED, { completedAt: 50.0 }),
    );
    expect(await store.listExpired(50.0)).toEqual([]);
    const expired = await store.listExpired(50.0001);
    expect(expired.map((i) => i.taskId)).toEqual(['eq']);
  });

  it('async_tasks.list_expired.property.idempotent: repeated listExpired return the same ids and never mutate state', async () => {
    const store = new InMemoryTaskStore();
    await store.save(
      makeTaskInfo('e1', 'm', TaskStatus.COMPLETED, { completedAt: 10.0 }),
    );
    const a = new Set((await store.listExpired(100.0)).map((i) => i.taskId));
    const b = new Set((await store.listExpired(100.0)).map((i) => i.taskId));
    expect(a).toEqual(b);
    expect(a).toEqual(new Set(['e1']));
    expect((await store.list()).length).toBe(1);
  });
});
