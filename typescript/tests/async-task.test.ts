import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { AsyncTaskManager, TaskStatus, InMemoryTaskStore, RetryConfig } from '../src/async-task.js';
import type { TaskInfo } from '../src/async-task.js';
import { Executor } from '../src/executor.js';
import { FunctionModule } from '../src/decorator.js';
import { Registry } from '../src/registry/registry.js';
import { TaskLimitExceededError, TaskStoreError } from '../src/errors.js';

function createRegistry(): Registry {
  const registry = new Registry();

  const simpleModule = new FunctionModule({
    execute: (inputs) => ({ value: (inputs['x'] as number) ?? 0 }),
    moduleId: 'test.simple',
    inputSchema: Type.Object({ x: Type.Optional(Type.Number()) }),
    outputSchema: Type.Object({ value: Type.Number() }),
    description: 'Simple module',
  });

  const failingModule = new FunctionModule({
    execute: () => { throw new Error('intentional failure'); },
    moduleId: 'test.failing',
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({}),
    description: 'Failing module',
  });

  const slowModule = new FunctionModule({
    execute: async (inputs) => {
      const ms = (inputs['delay'] as number) ?? 1000;
      await new Promise((resolve) => setTimeout(resolve, ms));
      return { done: true };
    },
    moduleId: 'test.slow',
    inputSchema: Type.Object({ delay: Type.Optional(Type.Number()) }),
    outputSchema: Type.Object({ done: Type.Boolean() }),
    description: 'Slow module',
  });

  registry.register('test.simple', simpleModule);
  registry.register('test.failing', failingModule);
  registry.register('test.slow', slowModule);

  return registry;
}

function createManager(maxConcurrent: number = 10): { manager: AsyncTaskManager; executor: Executor } {
  const registry = createRegistry();
  const executor = new Executor({ registry });
  const manager = new AsyncTaskManager({ executor, maxConcurrent });
  return { manager, executor };
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

describe('TaskStatus', () => {
  it('has correct enum values', () => {
    expect(TaskStatus.PENDING).toBe('pending');
    expect(TaskStatus.RUNNING).toBe('running');
    expect(TaskStatus.COMPLETED).toBe('completed');
    expect(TaskStatus.FAILED).toBe('failed');
    expect(TaskStatus.CANCELLED).toBe('cancelled');
  });
});

describe('AsyncTaskManager', () => {
  describe('submit and status transitions', () => {
    it('submits a task and transitions to COMPLETED', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.simple', { x: 42 });

      const info = await manager.getStatus(taskId);
      expect(info).not.toBeNull();
      expect(info!.moduleId).toBe('test.simple');

      await wait(100);

      const completed = await manager.getStatus(taskId);
      expect(completed).not.toBeNull();
      expect(completed!.status).toBe(TaskStatus.COMPLETED);
      expect(completed!.result).toEqual({ value: 42 });
      expect(completed!.startedAt).not.toBeNull();
      expect(completed!.completedAt).not.toBeNull();
      expect(completed!.error).toBeNull();
    });

    it('returns a unique task id', async () => {
      const { manager } = createManager();
      const id1 = await manager.submit('test.simple', { x: 1 });
      const id2 = await manager.submit('test.simple', { x: 2 });
      expect(id1).not.toBe(id2);
    });

    it('forwards context via opts.context', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.simple', { x: 7 }, { context: null });
      await wait(100);
      const info = (await manager.getStatus(taskId))!;
      expect(info.status).toBe(TaskStatus.COMPLETED);
      expect(info.result).toEqual({ value: 7 });
    });
  });

  describe('task failure', () => {
    it('sets status to FAILED with error message', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.failing', {});

      await wait(100);

      const info = await manager.getStatus(taskId);
      expect(info).not.toBeNull();
      expect(info!.status).toBe(TaskStatus.FAILED);
      expect(info!.error).toContain('intentional failure');
      expect(info!.completedAt).not.toBeNull();
      expect(info!.result).toBeNull();
    });
  });

  describe('task cancellation', () => {
    it('cancels a running task', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.slow', { delay: 60000 });

      await wait(100);

      const info = await manager.getStatus(taskId);
      expect(info).not.toBeNull();
      expect(info!.status).toBe(TaskStatus.RUNNING);

      const cancelled = await manager.cancel(taskId);
      expect(cancelled).toBe(true);

      const after = await manager.getStatus(taskId);
      expect(after).not.toBeNull();
      expect(after!.status).toBe(TaskStatus.CANCELLED);
      expect(after!.completedAt).not.toBeNull();
    });

    it('returns false for nonexistent task', async () => {
      const { manager } = createManager();
      const result = await manager.cancel('no-such-id');
      expect(result).toBe(false);
    });

    it('returns false for already completed task', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.simple', { x: 1 });
      await wait(100);

      expect((await manager.getStatus(taskId))!.status).toBe(TaskStatus.COMPLETED);

      const result = await manager.cancel(taskId);
      expect(result).toBe(false);
    });

    // A-D-004: a cancel issued mid-backoff must unwind the runner promptly
    // instead of waiting out the full retry delay (here 5000ms). The failing
    // module fails on the first attempt, so the task is in PENDING backoff
    // when we cancel. `cancel()` already flips the persisted status to
    // CANCELLED, so the meaningful observable is the runner's settle promise
    // (awaited by shutdown()): with the bug it stays blocked behind the
    // setTimeout(delay); with the fix it resolves immediately on abort.
    it('honors cancel during retry backoff without waiting out the delay', async () => {
      const { manager } = createManager();
      const retry = new RetryConfig({
        maxRetries: 3,
        retryDelayMs: 5000,
        backoffMultiplier: 1,
        maxRetryDelayMs: 5000,
      });

      const taskId = await manager.submit('test.failing', {}, { retry });

      // Let the first attempt fail and the task enter the backoff window.
      await wait(80);
      const duringBackoff = await manager.getStatus(taskId);
      expect(duringBackoff!.status).toBe(TaskStatus.PENDING);
      expect(duringBackoff!.retryCount).toBe(1);

      // shutdown() cancels the still-PENDING task and awaits the runner's
      // in-flight promise. With the bug the runner is parked in
      // setTimeout(5000) and shutdown() blocks the full delay; with the fix
      // the abort wakes the runner immediately.
      const start = Date.now();
      await manager.shutdown();
      const elapsed = Date.now() - start;

      expect((await manager.getStatus(taskId))!.status).toBe(TaskStatus.CANCELLED);
      expect(elapsed).toBeLessThan(2000);
    });
  });

  describe('concurrency limit', () => {
    it('limits concurrent executions to maxConcurrent', async () => {
      const { manager } = createManager(2);

      const taskIds = await Promise.all([
        manager.submit('test.slow', { delay: 60000 }),
        manager.submit('test.slow', { delay: 60000 }),
        manager.submit('test.slow', { delay: 60000 }),
        manager.submit('test.slow', { delay: 60000 }),
      ]);

      await wait(200);

      const running = await manager.listTasks(TaskStatus.RUNNING);
      const pending = await manager.listTasks(TaskStatus.PENDING);

      expect(running.length).toBeLessThanOrEqual(2);
      expect(running.length + pending.length).toBe(4);

      await Promise.all(taskIds.map(id => manager.cancel(id)));
    });
  });

  describe('getResult', () => {
    it('returns result for completed task', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.simple', { x: 99 });
      await wait(100);

      const result = await manager.getResult(taskId);
      expect(result).toEqual({ value: 99 });
    });

    it('rejects for unknown task', async () => {
      const { manager } = createManager();
      await expect(manager.getResult('no-such-task')).rejects.toThrow('Task not found');
    });

    it('rejects for non-completed task', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.slow', { delay: 60000 });

      await expect(manager.getResult(taskId)).rejects.toThrow('not completed');

      await manager.cancel(taskId);
    });
  });

  describe('getStatus', () => {
    it('returns null for unknown task', async () => {
      const { manager } = createManager();
      expect(await manager.getStatus('nonexistent')).toBeNull();
    });
  });

  describe('listTasks', () => {
    it('returns all tasks', async () => {
      const { manager } = createManager();
      await manager.submit('test.simple', { x: 1 });
      await manager.submit('test.simple', { x: 2 });
      await wait(100);

      const all = await manager.listTasks();
      expect(all.length).toBe(2);
    });

    it('filters tasks by status', async () => {
      const { manager } = createManager();
      await manager.submit('test.simple', { x: 1 });
      await manager.submit('test.failing', {});
      await wait(100);

      const completed = await manager.listTasks(TaskStatus.COMPLETED);
      const failed = await manager.listTasks(TaskStatus.FAILED);
      expect(completed.length).toBe(1);
      expect(failed.length).toBe(1);
    });
  });

  describe('max tasks limit', () => {
    it('throws when task limit is exceeded', async () => {
      const { manager } = createManager();
      // Create a manager with a small limit
      const registry = createRegistry();
      const executor = new Executor({ registry });
      const limitedManager = new AsyncTaskManager({ executor, maxConcurrent: 10, maxTasks: 3 });

      await limitedManager.submit('test.simple', { x: 1 });
      await limitedManager.submit('test.simple', { x: 2 });
      await limitedManager.submit('test.simple', { x: 3 });

      await expect(limitedManager.submit('test.simple', { x: 4 })).rejects.toThrow(
        'Task limit reached (3)',
      );
    });

    // A-D-003: over-capacity submit must reject with a typed
    // TaskLimitExceededError (code TASK_LIMIT_EXCEEDED), matching
    // Python/Rust — not a bare Error.
    it('rejects with TaskLimitExceededError when over capacity', async () => {
      const registry = createRegistry();
      const executor = new Executor({ registry });
      const limitedManager = new AsyncTaskManager({ executor, maxConcurrent: 10, maxTasks: 1 });

      await limitedManager.submit('test.simple', { x: 1 });

      await expect(limitedManager.submit('test.simple', { x: 2 })).rejects.toBeInstanceOf(
        TaskLimitExceededError,
      );
      await limitedManager
        .submit('test.simple', { x: 3 })
        .then(
          () => {
            throw new Error('expected submit to reject');
          },
          (err: unknown) => {
            expect(err).toBeInstanceOf(TaskLimitExceededError);
            expect((err as TaskLimitExceededError).code).toBe('TASK_LIMIT_EXCEEDED');
          },
        );
    });

    // Admission is a critical section: `_countActiveTasks()` and the PENDING
    // save are separated by two suspension points. Without serialization,
    // call A suspends inside the count while B runs the same check, so both
    // observe active=0 at maxTasks=1 and both persist — two PENDING tasks
    // under a cap of one, and no TaskLimitExceededError. apcore-python holds
    // `_admission_lock` across the pair; apcore-rust holds `admission_lock`.
    it('serializes concurrent submits at capacity — exactly one is admitted', async () => {
      const registry = createRegistry();
      const executor = new Executor({ registry });
      const limitedManager = new AsyncTaskManager({ executor, maxConcurrent: 10, maxTasks: 1 });

      const results = await Promise.allSettled([
        limitedManager.submit('test.slow', { delay: 300 }),
        limitedManager.submit('test.slow', { delay: 300 }),
      ]);

      const fulfilled = results.filter((r) => r.status === 'fulfilled');
      const rejected = results.filter((r) => r.status === 'rejected');
      expect(fulfilled).toHaveLength(1);
      expect(rejected).toHaveLength(1);
      expect((rejected[0] as PromiseRejectedResult).reason).toBeInstanceOf(
        TaskLimitExceededError,
      );

      // The store is the real evidence: the cap is on persisted active tasks.
      const active = (await limitedManager.listTasks()).filter(
        (t) => t.status === TaskStatus.PENDING || t.status === TaskStatus.RUNNING,
      );
      expect(active).toHaveLength(1);

      await limitedManager.shutdown();
    });

    it('a rejected admission does not poison the serializer for later submits', async () => {
      const registry = createRegistry();
      const executor = new Executor({ registry });
      const limitedManager = new AsyncTaskManager({ executor, maxConcurrent: 10, maxTasks: 1 });

      const first = await limitedManager.submit('test.slow', { delay: 300 });
      await expect(limitedManager.submit('test.slow', { delay: 300 })).rejects.toBeInstanceOf(
        TaskLimitExceededError,
      );

      // Free the slot, then submit again: the chain must still run.
      expect(await limitedManager.cancel(first)).toBe(true);
      const third = await limitedManager.submit('test.simple', { x: 1 });
      expect(third).toBeDefined();

      await limitedManager.shutdown();
    });

    it('allows submissions after cleanup frees slots', async () => {
      const registry = createRegistry();
      const executor = new Executor({ registry });
      const limitedManager = new AsyncTaskManager({ executor, maxConcurrent: 10, maxTasks: 2 });

      await limitedManager.submit('test.simple', { x: 1 });
      await limitedManager.submit('test.simple', { x: 2 });

      await wait(100);
      await limitedManager.cleanup(0);

      // Should succeed after cleanup
      const taskId = await limitedManager.submit('test.simple', { x: 3 });
      expect(taskId).toBeDefined();
    });
  });

  describe('double release fix', () => {
    it('does not corrupt concurrency counter when cancelling a queued task', async () => {
      // Use maxConcurrent=1 so the second task queues behind the first.
      // Delay/wait values are intentionally generous (500ms task / 700ms
      // settle) so the PENDING assertion below is robust under heavy
      // test-suite load — Node's setTimeout is best-effort and
      // `wait(20)` cumulative can stretch well past 50ms when many
      // test files run in parallel.
      const registry = createRegistry();
      const executor = new Executor({ registry });
      const mgr = new AsyncTaskManager({ executor, maxConcurrent: 1 });

      // Submit a long-enough task to fill the single slot
      const firstId = await mgr.submit('test.slow', { delay: 500 });
      await wait(10);

      // Submit a second task -- it will be waiting in the queue
      const queuedId = await mgr.submit('test.slow', { delay: 500 });
      await wait(10);

      // The queued task should still be PENDING (firstId has ~480ms left)
      expect((await mgr.getStatus(queuedId))!.status).toBe(TaskStatus.PENDING);

      // Cancel the queued task while it's waiting for a slot
      await mgr.cancel(queuedId);

      // Wait for the first task to complete, which releases the slot
      // and wakes the cancelled queued task. The queued task should
      // see cancelled=true and return; finally releases the slot once.
      await wait(700);

      // The running count should be 0 (not negative from double release)
      const runningCount = (mgr as unknown as { _runningCount: number })._runningCount;
      expect(runningCount).toBe(0);

      // Also verify the first task completed successfully
      expect((await mgr.getStatus(firstId))!.status).toBe(TaskStatus.COMPLETED);
      expect((await mgr.getStatus(queuedId))!.status).toBe(TaskStatus.CANCELLED);
    });
  });

  describe('async API contract', () => {
    it('submit() returns a Promise', async () => {
      const { manager } = createManager();
      const result = manager.submit('test.simple', { x: 1 });
      expect(result).toBeInstanceOf(Promise);
      await result;
    });

    it('cancel() returns a Promise', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.simple', { x: 1 });
      const result = manager.cancel(taskId);
      expect(result).toBeInstanceOf(Promise);
      await result;
    });

    it('shutdown() returns a Promise', async () => {
      const { manager } = createManager();
      const result = manager.shutdown();
      expect(result).toBeInstanceOf(Promise);
      await result;
    });
  });

  describe('shutdown', () => {
    it('cancels all pending and running tasks', async () => {
      const { manager } = createManager(1);
      const [taskId1, taskId2] = await Promise.all([
        manager.submit('test.slow', { delay: 200 }),
        manager.submit('test.slow', { delay: 200 }),
      ]);

      await wait(100);

      await manager.shutdown();

      expect((await manager.getStatus(taskId1))!.status).toBe(TaskStatus.CANCELLED);
      expect((await manager.getStatus(taskId2))!.status).toBe(TaskStatus.CANCELLED);
    });

    it('is a no-op when no tasks are running', async () => {
      const { manager } = createManager();
      await expect(manager.shutdown()).resolves.toBeUndefined();
    });

    it('waits for in-flight tasks to settle after cancellation', async () => {
      const { manager } = createManager();
      // Short delay so the task is running but finishes quickly once cancel is called
      const taskId = await manager.submit('test.slow', { delay: 200 });

      await wait(50); // let task start running
      expect((await manager.getStatus(taskId))!.status).toBe(TaskStatus.RUNNING);

      await manager.shutdown();

      // After shutdown resolves, the task should be in a terminal state
      const finalStatus = (await manager.getStatus(taskId))!.status;
      expect([TaskStatus.CANCELLED, TaskStatus.COMPLETED, TaskStatus.FAILED]).toContain(finalStatus);
    });
  });

  /* ---------------------------------------------------------
   * D-81 (spec v1.49.0) — "Store errors reach the caller (all
   * manager methods)". A manager that absorbs a TaskStoreError into
   * a normal return value reports "no such task" / "no tasks" for a
   * store that is merely unreachable, and — worst — a shutdown() that
   * swallows a failed save resolves while asserting its own
   * postcondition (every PENDING/RUNNING task is now CANCELLED) for a
   * task whose CANCELLED record never landed.
   * --------------------------------------------------------- */

  describe('D-81: store errors reach the caller', () => {
    class UnavailableStore extends InMemoryTaskStore {
      failSave = false;
      failGet = false;
      failList = false;
      failDelete = false;

      // The CANONICAL type D-92 requires the SDK to define and export, which
      // is the one type a host writing a network-backed store has to raise.
      // These used to throw `new Error('TASK_STORE_UNAVAILABLE')` — a
      // stand-in written before the type existed — and that left every
      // assertion below green against a manager that swallowed
      // `TaskStoreError` specifically while re-throwing everything else,
      // which is exactly the failure D-81 forbids.
      private down(operation: string): never {
        throw new TaskStoreError(operation, 'backing store is unreachable');
      }

      override async save(task: TaskInfo): Promise<void> {
        if (this.failSave) this.down('save');
        return super.save(task);
      }

      override async get(taskId: string): Promise<TaskInfo | null> {
        if (this.failGet) this.down('get');
        return super.get(taskId);
      }

      override async list(status?: TaskStatus): Promise<TaskInfo[]> {
        if (this.failList) this.down('list');
        return super.list(status);
      }

      override async delete(taskId: string): Promise<void> {
        if (this.failDelete) this.down('delete');
        return super.delete(taskId);
      }
    }

    /** Asserts the rejection is the store's OWN error, not merely an error. */
    async function rejectsUnavailable(what: string, p: Promise<unknown>): Promise<void> {
      await p.then(
        (v) => {
          throw new Error(`${what} absorbed a store outage into ${JSON.stringify(v)}`);
        },
        (e: unknown) => {
          expect(e, `${what} must propagate the store's own error type`).toBeInstanceOf(
            TaskStoreError,
          );
          expect((e as TaskStoreError).code).toBe('TASK_STORE_UNAVAILABLE');
        },
      );
    }

    function managerOver(store: UnavailableStore): AsyncTaskManager {
      const registry = createRegistry();
      const executor = new Executor({ registry });
      return new AsyncTaskManager({ executor, store });
    }

    it('getStatus() rejects rather than reporting "not found"', async () => {
      const store = new UnavailableStore();
      const manager = managerOver(store);
      store.failGet = true;
      await rejectsUnavailable('getStatus', manager.getStatus('any'));
    });

    it('cancel() rejects rather than returning false', async () => {
      const store = new UnavailableStore();
      const manager = managerOver(store);
      store.failGet = true;
      await rejectsUnavailable('cancel', manager.cancel('any'));
    });

    it('listTasks() rejects rather than returning an empty list', async () => {
      const store = new UnavailableStore();
      const manager = managerOver(store);
      store.failList = true;
      await rejectsUnavailable('listTasks', manager.listTasks());
    });

    it('cleanup() rejects rather than reporting zero removals', async () => {
      const store = new UnavailableStore();
      const manager = managerOver(store);
      store.failList = true;
      await rejectsUnavailable('cleanup', manager.cleanup(0));
    });

    it('shutdown() rejects when a cancellation write fails', async () => {
      // The store lists the active task fine, so shutdown() gets past its own
      // store touch; the CANCELLED write is what fails. Resolving normally here
      // would assert the postcondition for a record that never landed.
      const store = new UnavailableStore();
      const manager = managerOver(store);
      await store.save({
        taskId: 'external-pending',
        moduleId: 'test.simple',
        status: TaskStatus.PENDING,
        submittedAt: Date.now() / 1000,
        startedAt: null,
        completedAt: null,
        result: null,
        error: null,
        retryCount: 0,
        maxRetries: 0,
      });
      store.failSave = true;
      await rejectsUnavailable('shutdown', manager.shutdown());
    });

    it('submit() rejects rather than reporting a task id that never persisted', async () => {
      const store = new UnavailableStore();
      const manager = managerOver(store);
      store.failSave = true;
      await rejectsUnavailable('submit', manager.submit('test.simple', {}));
    });

    it('getResult() rejects rather than reporting "not found"', async () => {
      const store = new UnavailableStore();
      const manager = managerOver(store);
      store.failGet = true;
      await rejectsUnavailable('getResult', manager.getResult('any'));
    });

    it('shutdown() still resolves normally when the store is healthy', async () => {
      const store = new UnavailableStore();
      const manager = managerOver(store);
      const taskId = await manager.submit('test.slow', { delay: 200 });
      await wait(50);
      await expect(manager.shutdown()).resolves.toBeUndefined();
      expect((await manager.getStatus(taskId))?.status).toBe(TaskStatus.CANCELLED);
    });
  });

  /* ---------------------------------------------------------
   * D-122 (spec v1.52.0) — "shutdown() attempts every cancellation
   * before it reports". This SDK was the authority for that decision:
   * apcore-python and apcore-rust returned at the first failing
   * cancel() and were changed to match this behaviour. The tests above
   * pin only that a failure REACHES the caller (D-81), which
   * fail-fast satisfies too — so nothing here stopped a refactor from
   * short-circuiting the loop and quietly re-opening the divergence.
   *
   * What the decision turns on: when one task's store write fails,
   * stopping leaves every remaining active task uncancelled, and an
   * uncancelled task in a SHARED store holds a `maxTasks` slot for
   * every manager pointed at that store and outlives the process that
   * could have cancelled it. A slower shutdown is transient.
   * --------------------------------------------------------- */

  describe('D-122: shutdown attempts every cancellation', () => {
    /** Fails the CANCELLED write for exactly one task id; healthy otherwise. */
    class OneBadRecordStore extends InMemoryTaskStore {
      constructor(private readonly badTaskId: string) {
        super();
      }

      override async save(task: TaskInfo): Promise<void> {
        if (task.taskId === this.badTaskId && task.status === TaskStatus.CANCELLED) {
          throw new Error('TASK_STORE_UNAVAILABLE');
        }
        return super.save(task);
      }
    }

    function seedPending(store: InMemoryTaskStore, taskId: string): Promise<void> {
      return store.save({
        taskId,
        moduleId: 'test.simple',
        status: TaskStatus.PENDING,
        submittedAt: Date.now() / 1000,
        startedAt: null,
        completedAt: null,
        result: null,
        error: null,
        retryCount: 0,
        maxRetries: 0,
      });
    }

    it('cancels the tasks after a failing one, and still reports the failure', async () => {
      const store = new OneBadRecordStore('t2');
      await seedPending(store, 't1');
      await seedPending(store, 't2');
      await seedPending(store, 't3');

      const registry = createRegistry();
      const executor = new Executor({ registry });
      const manager = new AsyncTaskManager({ executor, store });

      await expect(manager.shutdown()).rejects.toThrow('TASK_STORE_UNAVAILABLE');

      const byId = new Map((await store.list()).map((t) => [t.taskId, t.status]));
      expect(byId.get('t1')).toBe(TaskStatus.CANCELLED);
      // The one the decision is about: fail-fast strands it holding a
      // `maxTasks` slot in a store that outlives this process.
      expect(byId.get('t3')).toBe(TaskStatus.CANCELLED);
      // 't2' is deliberately not asserted — its CANCELLED write is the one
      // that failed, so whether its record shows PENDING or a partially
      // applied state is a property of this test double, not of D-122.
    });

    it('stops the reaper before it starts cancelling', async () => {
      // All three SDKs stop the reaper during shutdown; only the ORDER
      // differed — apcore-python and apcore-rust make it their first statement,
      // this SDK ran it after the cancellations had settled. That leaks no
      // timer (the throw comes later), but it leaves a sweep able to fire
      // *during* the cancel loop and delete records the loop is walking. The
      // window is real whenever shutdown is slow: a remote store, a large task
      // set, or D-122's best-effort loop grinding through a failing backend.
      const order: string[] = [];

      class RecordingStore extends InMemoryTaskStore {
        override async save(task: TaskInfo): Promise<void> {
          if (task.status === TaskStatus.CANCELLED) order.push(`cancel:${task.taskId}`);
          return super.save(task);
        }
      }

      const store = new RecordingStore();
      await seedPending(store, 't1');

      const registry = createRegistry();
      const executor = new Executor({ registry });
      const manager = new AsyncTaskManager({ executor, store });
      // Replace the reaper handle with a spy: `startReaper` would install a real
      // timer, and what is under test is the ordering, not the sweep.
      (manager as unknown as { _reaper: unknown })._reaper = {
        stop: async () => { order.push('reaper-stop'); },
      };

      await manager.shutdown();

      expect(order[0]).toBe('reaper-stop');
      expect(order).toContain('cancel:t1');
    });
  });

  // A persistent or shared TaskStore holds active records this manager has no
  // in-process handle for: written by another process, or by a previous run.
  describe('store-resident tasks without an in-process handle', () => {
    function seed(store: InMemoryTaskStore, taskId: string, status: TaskStatus): Promise<void> {
      return store.save({
        taskId,
        moduleId: 'test.simple',
        status,
        submittedAt: Date.now() / 1000,
        startedAt: status === TaskStatus.RUNNING ? Date.now() / 1000 : null,
        completedAt: null,
        result: null,
        error: null,
        retryCount: 0,
        maxRetries: 0,
      });
    }

    function managerWith(store: InMemoryTaskStore): AsyncTaskManager {
      const registry = createRegistry();
      const executor = new Executor({ registry });
      return new AsyncTaskManager({ executor, store });
    }

    it('cancel() transitions them to CANCELLED and returns true', async () => {
      const store = new InMemoryTaskStore();
      await seed(store, 'external-pending', TaskStatus.PENDING);
      const manager = managerWith(store);

      // The cancel Contract defines false as "did not exist or already
      // terminal". A missing in-process handle is neither, so returning false
      // here left the record permanently uncancellable and still counted
      // against maxTasks.
      expect(await manager.cancel('external-pending')).toBe(true);
      expect((await manager.getStatus('external-pending'))!.status).toBe(TaskStatus.CANCELLED);
      expect((await manager.getStatus('external-pending'))!.completedAt).not.toBeNull();
    });

    it('cancel() still returns false for unknown and already-terminal tasks', async () => {
      const store = new InMemoryTaskStore();
      await seed(store, 'external-done', TaskStatus.COMPLETED);
      const manager = managerWith(store);

      expect(await manager.cancel('no-such-task')).toBe(false);
      expect(await manager.cancel('external-done')).toBe(false);
      expect((await manager.getStatus('external-done'))!.status).toBe(TaskStatus.COMPLETED);
    });

    it('shutdown() leaves none of them active — its stated postcondition', async () => {
      const store = new InMemoryTaskStore();
      await seed(store, 'external-pending', TaskStatus.PENDING);
      await seed(store, 'external-running', TaskStatus.RUNNING);
      const manager = managerWith(store);

      await manager.shutdown();

      expect((await manager.getStatus('external-pending'))!.status).toBe(TaskStatus.CANCELLED);
      expect((await manager.getStatus('external-running'))!.status).toBe(TaskStatus.CANCELLED);
    });
  });

  describe('terminal-status guards in the runner', () => {
    it('does not overwrite a terminal status written out of band', async () => {
      const store = new InMemoryTaskStore();
      const registry = createRegistry();
      const executor = new Executor({ registry });
      const manager = new AsyncTaskManager({ executor, store });

      const taskId = await manager.submit('test.slow', { delay: 200 });
      await wait(60);
      expect((await manager.getStatus(taskId))!.status).toBe(TaskStatus.RUNNING);

      // Another process cancels through the shared store. This manager's
      // in-process `cancelled` flag knows nothing about it, so before the
      // guards the runner wrote COMPLETED over it — a cancelled -> completed
      // transition that protocol-spec.md §5.8 forbids.
      const current = (await store.get(taskId))!;
      await store.save({
        ...current,
        status: TaskStatus.CANCELLED,
        completedAt: Date.now() / 1000,
      });

      await wait(300);
      expect((await manager.getStatus(taskId))!.status).toBe(TaskStatus.CANCELLED);
    });

    it('does not resurrect an out-of-band cancel as a PENDING retry', async () => {
      const store = new InMemoryTaskStore();
      const registry = createRegistry();
      const executor = new Executor({ registry });
      const manager = new AsyncTaskManager({ executor, store });

      const retry = new RetryConfig({ maxRetries: 3, retryDelayMs: 50, backoffMultiplier: 1 });
      const taskId = await manager.submit('test.failing', {}, { retry });

      const current = (await store.get(taskId))!;
      await store.save({
        ...current,
        status: TaskStatus.CANCELLED,
        completedAt: Date.now() / 1000,
      });

      await wait(300);
      expect((await manager.getStatus(taskId))!.status).toBe(TaskStatus.CANCELLED);
      expect((await manager.getStatus(taskId))!.retryCount).toBe(0);
    });

    it('prunes the in-process handle when a task settles', async () => {
      const { manager } = createManager();
      const internals = (manager as unknown as { _internal: Map<string, unknown> })._internal;

      const taskId = await manager.submit('test.simple', { x: 1 });
      expect(internals.has(taskId)).toBe(true);

      await wait(100);

      // Each handle pins a Promise, a resolve closure and a live
      // AbortController. cleanup() and the reaper are both opt-in, so without
      // this the manager kept one per completed task for the process lifetime.
      expect((await manager.getStatus(taskId))!.status).toBe(TaskStatus.COMPLETED);
      expect(internals.has(taskId)).toBe(false);
    });
  });

  describe('cleanup', () => {
    it('removes old completed tasks', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.simple', { x: 1 });
      await wait(100);

      expect((await manager.getStatus(taskId))!.status).toBe(TaskStatus.COMPLETED);

      const removed = await manager.cleanup(0);
      expect(removed).toBe(1);
      expect(await manager.getStatus(taskId)).toBeNull();
    });

    it('preserves recent tasks', async () => {
      const { manager } = createManager();
      await manager.submit('test.simple', { x: 1 });
      await wait(100);

      const removed = await manager.cleanup(3600);
      expect(removed).toBe(0);
      expect((await manager.listTasks()).length).toBe(1);
    });

    it('preserves running tasks', async () => {
      const { manager } = createManager();
      const taskId = await manager.submit('test.slow', { delay: 60000 });
      await wait(100);

      const removed = await manager.cleanup(0);
      expect(removed).toBe(0);

      await manager.cancel(taskId);
    });
  });
});
