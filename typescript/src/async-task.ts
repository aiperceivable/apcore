/**
 * Async task manager for background module execution.
 */

import { v4 as uuidv4 } from 'uuid';
import { CancelToken } from './cancel.js';
import type { Context } from './context.js';
import { ErrorCodes, ModuleError, TaskLimitExceededError } from './errors.js';
import type { Executor } from './executor.js';

export enum TaskStatus {
  PENDING = 'pending',
  RUNNING = 'running',
  COMPLETED = 'completed',
  FAILED = 'failed',
  CANCELLED = 'cancelled',
}

const TERMINAL_STATUSES = new Set([TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED]);

export interface TaskInfo {
  readonly taskId: string;
  readonly moduleId: string;
  readonly status: TaskStatus;
  readonly submittedAt: number;
  readonly startedAt: number | null;
  readonly completedAt: number | null;
  readonly result: Record<string, unknown> | null;
  readonly error: string | null;
  readonly retryCount: number;
  readonly maxRetries: number;
}

/**
 * Pluggable task storage backend.
 *
 * Spec D-17 (apcore v0.22.0): every method MUST be asynchronous so that
 * Redis-, SQL-, and other I/O-backed stores can be plugged in without
 * blocking the event loop. `InMemoryTaskStore` exposes async signatures
 * even though its operations are CPU-only — uniform shape lets stores
 * compose generically. Supersedes the partially-sync contract that
 * existed in apcore-typescript through v0.21.x.
 */
export interface TaskStore {
  save(task: TaskInfo): Promise<void>;
  get(taskId: string): Promise<TaskInfo | null>;
  list(status?: TaskStatus): Promise<TaskInfo[]>;
  delete(taskId: string): Promise<void>;
  listExpired(beforeTimestamp: number): Promise<TaskInfo[]>;
}

export class InMemoryTaskStore implements TaskStore {
  private readonly _data: Map<string, TaskInfo> = new Map();

  async save(task: TaskInfo): Promise<void> {
    this._data.set(task.taskId, task);
  }

  async get(taskId: string): Promise<TaskInfo | null> {
    return this._data.get(taskId) ?? null;
  }

  async list(status?: TaskStatus): Promise<TaskInfo[]> {
    const all = [...this._data.values()];
    return status ? all.filter(t => t.status === status) : all;
  }

  async delete(taskId: string): Promise<void> {
    this._data.delete(taskId);
  }

  async listExpired(beforeTimestamp: number): Promise<TaskInfo[]> {
    return [...this._data.values()].filter(
      t => TERMINAL_STATUSES.has(t.status) && t.completedAt !== null && t.completedAt < beforeTimestamp,
    );
  }
}

/**
 * Process-wide one-shot deprecation warning bookkeeping for {@link RetryConfig}
 * (sync finding D-08). Mirrors the cross-language convention: warn once per
 * unique deprecation key, then stay silent.
 */
const _RETRY_CONFIG_DEPRECATIONS_EMITTED: Set<string> = new Set();

function _emitRetryConfigDeprecation(key: string, message: string): void {
  if (_RETRY_CONFIG_DEPRECATIONS_EMITTED.has(key)) return;
  _RETRY_CONFIG_DEPRECATIONS_EMITTED.add(key);
  console.warn(message);
}

export class RetryConfig {
  readonly maxRetries: number;
  readonly retryDelayMs: number;
  readonly backoffMultiplier: number;
  readonly maxRetryDelayMs: number;

  constructor(opts: {
    maxRetries?: number;
    retryDelayMs?: number;
    backoffMultiplier?: number;
    maxRetryDelayMs?: number;
  } = {}) {
    this.maxRetries = opts.maxRetries ?? 0;
    this.retryDelayMs = opts.retryDelayMs ?? 1000;
    this.backoffMultiplier = opts.backoffMultiplier ?? 2.0;
    this.maxRetryDelayMs = opts.maxRetryDelayMs ?? 60000;
  }

  /**
   * Compute the delay in milliseconds before the given retry attempt.
   *
   * Canonical cross-language name (sync finding D-08). Mirrors
   * `RetryConfig.compute_delay_ms` in apcore-python and
   * `RetryConfig::compute_delay_ms` in apcore-rust.
   */
  computeDelayMs(attempt: number): number {
    return Math.min(
      this.retryDelayMs * Math.pow(this.backoffMultiplier, attempt),
      this.maxRetryDelayMs,
    );
  }

  /**
   * @deprecated Use {@link RetryConfig.computeDelayMs} instead. Will be removed
   * in the next minor release. Sync finding D-08 — the canonical cross-language
   * method name is `computeDelayMs` (Python `compute_delay_ms`,
   * Rust `compute_delay_ms`).
   */
  computeDelay(attempt: number): number {
    _emitRetryConfigDeprecation(
      'RetryConfig.computeDelay',
      '[apcore] RetryConfig.computeDelay is deprecated; use computeDelayMs',
    );
    return this.computeDelayMs(attempt);
  }
}

export interface ReaperHandle {
  stop(): Promise<void>;
}

type MutableTaskInfo = { -readonly [K in keyof TaskInfo]: TaskInfo[K] };

interface InternalTask {
  promise: Promise<void>;
  cancelled: boolean;
  resolve: () => void;
  // D-18 (apcore v0.22.0): the same CancelToken is wired into the per-attempt
  // child Context so `cancel()` produces a real AbortSignal abort at the next
  // Web-API await point inside the executing module — not merely a cooperative
  // flag.
  cancelToken: CancelToken;
}

interface AsyncTaskManagerOptions {
  executor: Executor;
  store?: TaskStore;
  maxConcurrent?: number;
  maxTasks?: number;
}

/**
 * Manages background execution of modules via Promises.
 *
 * Uses a simple counter-based concurrency limiter instead of a semaphore.
 */
export class AsyncTaskManager {
  private readonly _executor: Executor;
  private readonly _store: TaskStore;
  private readonly _maxConcurrent: number;
  private readonly _maxTasks: number;
  private readonly _internal: Map<string, InternalTask> = new Map();
  private readonly _workers: Set<Promise<void>> = new Set();
  private _runningCount: number = 0;
  private readonly _waitQueue: Array<() => void> = [];
  private _reaper: ReaperHandle | null = null;
  /**
   * Admission serializer for {@link submit} (mirrors apcore-python's
   * `_admission_lock` and apcore-rust's `admission_lock`). A single-threaded
   * event loop needs no mutex primitive — chaining the critical section onto
   * one promise is enough to make "count the active tasks, then persist the
   * PENDING record" atomic with respect to other submits.
   */
  private _admitChain: Promise<unknown> = Promise.resolve();

  constructor(opts: AsyncTaskManagerOptions) {
    this._executor = opts.executor;
    this._store = opts.store ?? new InMemoryTaskStore();
    this._maxConcurrent = opts.maxConcurrent ?? 10;
    this._maxTasks = opts.maxTasks ?? 1000;
  }

  get store(): TaskStore {
    return this._store;
  }

  /**
   * Submit a module for background execution.
   *
   * Returns the generated task_id. Async to satisfy the cross-SDK protocol contract.
   */
  async submit(
    moduleId: string,
    inputs: Record<string, unknown>,
    opts?: { context?: Context | null; retry?: RetryConfig },
  ): Promise<string> {
    const taskId = uuidv4();
    const retry = opts?.retry ?? null;
    const info: MutableTaskInfo = {
      taskId,
      moduleId,
      status: TaskStatus.PENDING,
      submittedAt: Date.now() / 1000,
      startedAt: null,
      completedAt: null,
      result: null,
      error: null,
      retryCount: 0,
      maxRetries: retry?.maxRetries ?? 0,
    };

    // Check capacity and persist the PENDING record atomically. Both steps
    // await, and without the admission serializer a second submit interleaves
    // between them: at `maxTasks: 1` with an empty store, two concurrent
    // submits both observed active=0 and both persisted, leaving two PENDING
    // tasks under a cap of one and never raising TaskLimitExceededError.
    // apcore-python holds `_admission_lock` across the same pair and
    // apcore-rust holds `admission_lock`.
    //
    // A-D-AT-01 (apcore v0.22.0): max_tasks counts active (PENDING|RUNNING)
    // tasks only — completed/failed/cancelled tasks remain in `_internal` for
    // bookkeeping but must not count toward the live concurrency cap. Mirrors
    // Python `_ACTIVE_STATUSES` filter.
    await this._admit(async () => {
      const active = await this._countActiveTasks();
      if (active >= this._maxTasks) {
        throw new TaskLimitExceededError(this._maxTasks);
      }
      await this._store.save(info);
    });

    let resolvePromise!: () => void;
    const promise = new Promise<void>((resolve) => {
      resolvePromise = resolve;
    });

    // D-18: every submitted task owns a CancelToken so AsyncTaskManager.cancel()
    // produces a real AbortSignal abort, not just a cooperative flag.
    const cancelToken = new CancelToken();

    const internal: InternalTask = {
      promise,
      cancelled: false,
      resolve: resolvePromise,
      cancelToken,
    };

    this._internal.set(taskId, internal);
    this._enqueue(taskId, moduleId, inputs, opts?.context ?? null, retry, cancelToken);
    return taskId;
  }

  /**
   * Return the TaskInfo for a task, or null if not found.
   */
  async getStatus(taskId: string): Promise<TaskInfo | null> {
    const info = await this._store.get(taskId);
    return info ? { ...info } : null;
  }

  /**
   * Return the result of a completed task.
   *
   * Throws if the task is not found or not in COMPLETED status.
   */
  async getResult(taskId: string): Promise<Record<string, unknown>> {
    const info = await this._store.get(taskId);
    if (!info) {
      throw new Error(`Task not found: ${taskId}`);
    }
    if (info.status !== TaskStatus.COMPLETED) {
      throw new Error(`Task ${taskId} is not completed (status=${info.status})`);
    }
    return info.result!;
  }

  /**
   * Cancel a running or pending task.
   *
   * Returns true if the task was successfully marked as cancelled.
   *
   * D-18 (v0.22.0): calling `cancel()` aborts the running executor invocation
   * via the CancelToken's underlying AbortSignal — modules performing standard
   * Web-API I/O (`fetch`, `setTimeout`, Web Streams) participate in real abort.
   */
  async cancel(taskId: string): Promise<boolean> {
    const info = await this._store.get(taskId) as MutableTaskInfo | null;
    if (!info) return false;

    if (info.status !== TaskStatus.PENDING && info.status !== TaskStatus.RUNNING) {
      return false;
    }

    // A task can be active in the store with no in-process handle: a
    // persistent or shared TaskStore keeps PENDING/RUNNING records written by
    // another process or by a previous run of this one. A missing handle is
    // therefore NOT a third "false" case — the cancel Contract defines false
    // as "the task did not exist or had already reached a terminal state",
    // both already answered above. Returning false here left every such
    // record permanently uncancellable and still consuming the maxTasks
    // budget, and it broke `shutdown()`'s postcondition ("every task that was
    // PENDING or RUNNING at the time of the call will be in CANCELLED
    // state"), which is delegated to this method. apcore-rust
    // (async_task.rs:559) likewise treats the missing handle as a skipped
    // interrupt, not an early return.
    const internal = this._internal.get(taskId);
    if (internal) {
      internal.cancelled = true;
      internal.cancelToken.cancel();
    }

    info.status = TaskStatus.CANCELLED;
    info.completedAt = Date.now() / 1000;
    await this._store.save(info);
    return true;
  }

  /**
   * Return all tasks, optionally filtered by status.
   */
  async listTasks(status?: TaskStatus): Promise<TaskInfo[]> {
    const items = await this._store.list(status);
    return items.map(t => ({ ...t }));
  }

  /**
   * Remove terminal-state tasks older than maxAgeSeconds seconds.
   *
   * Returns the number of tasks removed.
   */
  async cleanup(maxAgeSeconds: number = 3600): Promise<number> {
    const threshold = Date.now() / 1000 - maxAgeSeconds;
    let removed = 0;

    for (const task of await this._store.list()) {
      if (!TERMINAL_STATUSES.has(task.status)) continue;
      const refTime = task.completedAt ?? task.submittedAt;
      if (refTime <= threshold) {
        await this._store.delete(task.taskId);
        this._internal.delete(task.taskId);
        removed++;
      }
    }

    return removed;
  }

  /**
   * Cancel all pending and running tasks and wait for them to settle.
   */
  async shutdown(): Promise<void> {
    // Stop the reaper BEFORE anything else, matching apcore-python
    // (`await self.stop_reaper()`) and apcore-rust (`self.stop_reaper()`), both
    // of which do it as their first statement. This used to run after the
    // cancellations had settled, which leaks no timer — the throw comes later —
    // but does leave a sweep able to fire *during* the cancel loop and delete
    // records the loop is walking. The window is real whenever shutdown is slow:
    // a remote store, a large task set, or D-122's best-effort loop grinding
    // through a backend that is failing every write.
    if (this._reaper) {
      await this._reaper.stop();
    }

    // Physical workers remain owned even after terminal records are reclaimed.
    const workers = [...this._workers];
    for (const internal of this._internal.values()) {
      internal.cancelled = true;
      internal.cancelToken.cancel();
    }
    const cancels: Promise<boolean>[] = [];
    let listFailure: { error: unknown } | undefined;
    try {
      for (const task of await this._store.list()) {
        if (task.status === TaskStatus.PENDING || task.status === TaskStatus.RUNNING) {
          cancels.push(this.cancel(task.taskId));
        }
      }
    } catch (error) {
      listFailure = { error };
    }
    const outcomes = await Promise.allSettled(cancels);
    await Promise.allSettled(workers);
    const failedCancel = outcomes.find((outcome) => outcome.status === 'rejected');
    if (listFailure !== undefined) throw listFailure.error;
    if (failedCancel?.status === 'rejected') throw failedCancel.reason;
  }

  /**
   * Run `critical` with exclusive access to the submit admission section.
   *
   * Each call chains onto the previous one, so the check-then-save pair runs
   * to completion before the next submit starts its own check. A rejected
   * critical section (typically {@link TaskLimitExceededError}) must not
   * poison the chain for later submits, so the chain itself continues from a
   * settled promise whose outcome is discarded — the rejection is still
   * delivered to this caller through the returned promise.
   */
  private _admit<T>(critical: () => Promise<T>): Promise<T> {
    const run = this._admitChain.then(critical, critical);
    this._admitChain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  /**
   * Re-read the stored record and report whether it has already reached a
   * terminal status.
   *
   * Ports the semantics of apcore-rust's `save_terminal_if_not_cancelled`:
   * every status write by the runner is preceded by a fresh read, so a
   * concurrent `cancel()` — including one issued by another process against
   * a shared store — is never overwritten. The read must happen BEFORE the
   * caller mutates its own copy: `InMemoryTaskStore` hands out the live
   * object, so mutating first would destroy the evidence this check needs.
   *
   * The read/check/write sequence is not atomic at the store level, so a
   * perfectly-timed cancel can still slip in between; closing that last
   * window would need compare-and-set support in the `TaskStore` interface.
   * The window left here is small and the cost of slipping is one extra
   * terminal write that the next cancel no-ops against.
   */
  private async _isStoredTerminal(taskId: string): Promise<boolean> {
    const current = await this._store.get(taskId);
    return current !== null && TERMINAL_STATUSES.has(current.status);
  }

  /** Count tasks in PENDING or RUNNING state across the store. */
  private async _countActiveTasks(): Promise<number> {
    let count = 0;
    for (const t of await this._store.list()) {
      if (t.status === TaskStatus.PENDING || t.status === TaskStatus.RUNNING) {
        count++;
      }
    }
    return count;
  }

  /**
   * Start a background reaper that periodically deletes expired terminal tasks.
   *
   * Returns a handle to stop the reaper. Throws `ModuleError` with code
   * `REAPER_ALREADY_RUNNING` if a reaper is already running.
   *
   * Spec `docs/features/async-tasks.md` "Contract: AsyncTaskManager.start_reaper"
   * (decision D-11) names `await manager.startReaper(...) -> Promise<ReaperHandle>`
   * as the canonical TypeScript signature, uniform with Python's `await
   * manager.start_reaper(...)` and Rust's `.start_reaper(...).await`. There is
   * no async setup this manager needs to perform before the reaper is ready
   * today — the sweep loop's own `await`s (`this._store.listExpired(...)`,
   * `this._store.delete(...)`) already happen inside the scheduled `sweep()`
   * closure, never before the handle is returned — so this method is
   * deliberately NOT declared `async`; it stays synchronous end to end and
   * wraps only the returned handle in `Promise.resolve(...)`. That preserves
   * the existing idempotency guard below as a genuine SYNCHRONOUS throw
   * (a caller doing `manager.startReaper()` a second time while one is
   * running still throws immediately, rather than the throw being swallowed
   * into a rejected Promise the caller may never await) while still handing
   * back a real `Promise<ReaperHandle>` for a future implementation that
   * *does* need to await something first.
   */
  startReaper(opts: { ttlSeconds?: number; sweepIntervalMs?: number } = {}): Promise<ReaperHandle> {
    if (this._reaper !== null) {
      throw new ModuleError(
        ErrorCodes.REAPER_ALREADY_RUNNING,
        '[apcore:async-task] Reaper already running; call stop() before starting again',
      );
    }

    const ttlSeconds = opts.ttlSeconds ?? 3600;
    const sweepIntervalMs = opts.sweepIntervalMs ?? 300000;

    let stopped = false;
    let timeoutHandle: ReturnType<typeof setTimeout> | null = null;
    let activeSweep: Promise<void> | null = null;
    let stopping: Promise<void> | null = null;

    const sweep = async (): Promise<void> => {
      if (stopped) return;
      const threshold = Date.now() / 1000 - ttlSeconds;
      try {
        const expired = await this._store.listExpired(threshold);
        for (const task of expired) {
          await this._store.delete(task.taskId);
          this._internal.delete(task.taskId);
        }
      } catch (err) {
        console.warn('[apcore:async-task] Reaper sweep failed:', err);
      }
      if (!stopped) {
        timeoutHandle = setTimeout(() => { activeSweep = sweep(); }, sweepIntervalMs);
      }
    };

    timeoutHandle = setTimeout(() => { activeSweep = sweep(); }, sweepIntervalMs);

    const handle: ReaperHandle = {
      stop: (): Promise<void> => {
        if (stopping) return stopping;
        stopped = true;
        if (timeoutHandle !== null) {
          clearTimeout(timeoutHandle);
          timeoutHandle = null;
        }
        stopping = Promise.resolve(activeSweep).then(() => {
          if (this._reaper === handle) this._reaper = null;
        });
        return stopping;
      },
    };

    this._reaper = handle;
    return Promise.resolve(handle);
  }

  private _acquireSlot(): Promise<void> {
    if (this._runningCount < this._maxConcurrent) {
      this._runningCount++;
      return Promise.resolve();
    }
    return new Promise<void>((resolve) => {
      this._waitQueue.push(() => {
        this._runningCount++;
        resolve();
      });
    });
  }

  private _releaseSlot(): void {
    this._runningCount--;
    if (this._waitQueue.length > 0) {
      const next = this._waitQueue.shift()!;
      next();
    }
  }

  private _enqueue(
    taskId: string,
    moduleId: string,
    inputs: Record<string, unknown>,
    context: Context | null,
    retry: RetryConfig | null,
    cancelToken: CancelToken,
  ): void {
    const run = async (): Promise<void> => {
      const internal = this._internal.get(taskId);
      if (!internal) return;

      // Per-task Context augmented with the task's cancel token so the
      // executor sees real cancellation (D-18). When the caller already
      // supplied a Context we shallow-clone it to attach the token without
      // mutating the caller's instance.
      const taskContext = await this._buildTaskContext(context, cancelToken);

      // slotHeld tracks whether we currently hold a concurrency slot.
      // We release it during backoff waits so other tasks can run.
      let slotHeld = false;

      try {
        // eslint-disable-next-line no-constant-condition
        while (true) {
          await this._acquireSlot();
          slotHeld = true;

          if (internal.cancelled) return;

          const info = await this._store.get(taskId) as MutableTaskInfo | null;
          if (!info) return;

          // The stored record is the authority on whether this task is still
          // ours to advance. `internal.cancelled` only sees cancels issued
          // through THIS manager instance; a cancel written by another
          // process against a shared store, or by a `cancel()` that found no
          // in-process handle, shows up only here. Overwriting it with
          // RUNNING would be a cancelled -> running transition, which
          // protocol-spec.md section 5.8 forbids.
          if (TERMINAL_STATUSES.has(info.status)) return;

          info.status = TaskStatus.RUNNING;
          // A-D-AT-08: preserve startedAt across retries (set only on first
          // attempt). Mirrors Python + Rust semantics — `startedAt` reflects
          // when the task first started running, not the latest attempt.
          if (info.startedAt === null) {
            info.startedAt = Date.now() / 1000;
          }
          await this._store.save(info);

          let execError: unknown = null;
          let succeeded = false;
          // Set when the stored record reached a terminal status while this
          // attempt was running: the task is no longer ours to finish, so we
          // stop without writing anything further.
          let abandoned = false;

          try {
            const result = await this._executor.call(moduleId, inputs, taskContext);
            if (!internal.cancelled) {
              if (await this._isStoredTerminal(taskId)) {
                abandoned = true;
              } else {
                info.status = TaskStatus.COMPLETED;
                info.completedAt = Date.now() / 1000;
                info.result = result;
                await this._store.save(info);
                succeeded = true;
              }
            }
          } catch (err) {
            execError = err;
          }

          if (succeeded || abandoned || internal.cancelled) return;

          if (retry && info.retryCount < retry.maxRetries) {
            const delay = retry.computeDelayMs(info.retryCount);
            // Re-checked before the write: a cancel that landed while the
            // attempt was failing must not be resurrected as PENDING.
            if (await this._isStoredTerminal(taskId)) return;
            info.retryCount += 1;
            info.status = TaskStatus.PENDING;
            // Intentionally do NOT reset startedAt / completedAt (A-D-AT-08):
            // startedAt reflects first-run timestamp; completedAt remains null
            // because the task has not yet terminated.
            await this._store.save(info);

            // Publish the logical retry outcome promptly, but do not let a
            // retry or another task overlap this attempt's deferred finalizers.
            await cancelToken.drain();
            this._releaseSlot();
            slotHeld = false;

            // A-D-004 (apcore v0.22.0): race the backoff wait against the
            // task's cancel signal so a cancel() issued mid-backoff unwinds
            // immediately instead of waiting out the full delay (up to
            // maxRetryDelayMs). Mirrors Python/Rust cancellable-sleep
            // semantics during retry backoff.
            await this._sleepUntilCancelled(delay, cancelToken);

            if (internal.cancelled) return;
            // Loop continues: re-acquire slot and retry
          } else {
            if (await this._isStoredTerminal(taskId)) return;
            info.status = TaskStatus.FAILED;
            info.completedAt = Date.now() / 1000;
            info.error = execError instanceof Error ? execError.message : String(execError);
            await this._store.save(info);
            return;
          }
        }
      } finally {
        await cancelToken.drain();
        if (slotHeld) this._releaseSlot();
        internal.resolve();
        // The handle holds a Promise, a resolve closure and a live
        // AbortController; keeping one per completed task for the lifetime of
        // a long-lived manager is a leak, because the only other removals —
        // `cleanup()` and the reaper sweep — are opt-in (async-tasks.md
        // section 1.3: "The Reaper MUST be opt-in"). apcore-python pops its
        // handle in a done-callback and apcore-rust removes it at the end of
        // the spawned task. Those other removals stay as belt and braces for
        // tasks whose runner never started.
        this._internal.delete(taskId);
      }
    };

    const worker = run().catch((err) => {
      console.warn('[apcore:async-task] Unexpected error in task runner:', err);
    });
    this._workers.add(worker);
    void worker.then(() => { this._workers.delete(worker); });
  }

  /**
   * Sleep for `delayMs`, resolving early if `cancelToken` is aborted (A-D-004).
   *
   * The timer is cleared and the abort listener detached on whichever path
   * resolves first, so neither leaks. A cancel issued mid-backoff resolves
   * promptly rather than waiting out the full retry delay.
   */
  private _sleepUntilCancelled(delayMs: number, cancelToken: CancelToken): Promise<void> {
    if (cancelToken.signal.aborted) {
      return Promise.resolve();
    }
    return new Promise<void>((resolve) => {
      const signal = cancelToken.signal;
      const timer = setTimeout(() => {
        signal.removeEventListener('abort', onAbort);
        resolve();
      }, delayMs);
      const onAbort = (): void => {
        clearTimeout(timer);
        signal.removeEventListener('abort', onAbort);
        resolve();
      };
      signal.addEventListener('abort', onAbort, { once: true });
    });
  }

  /**
   * Build the Context to hand to `executor.call()` for a task attempt.
   *
   * If the caller supplied a Context we shallow-clone it with the task's
   * CancelToken attached; if no Context was supplied we create a fresh one
   * via the public `Context.create()` factory with the token already bound
   * (per Issue #66, `cancelToken` is now a first-class create() parameter).
   * Either way the executor sees a Context whose `cancelToken` is the same
   * instance that `AsyncTaskManager.cancel()` aborts (D-18).
   */
  private async _buildTaskContext(
    callerContext: Context | null,
    cancelToken: CancelToken,
  ): Promise<Context> {
    // Lazy import to avoid a static circular dep between async-task and context.
    const { Context: CtxClass } = await import('./context.js');
    if (callerContext === null) {
      // Executor binding is deferred to the first executor.call() — Issue #66
      // removes `executor` from Context.create()'s public surface.
      return CtxClass.create(undefined, undefined, cancelToken);
    }
    // Shallow clone (preserve identity/data/etc.) and overwrite cancelToken.
    const cloned = new CtxClass(
      callerContext.traceId,
      callerContext.callerId,
      [...callerContext.callChain],
      callerContext.executor,
      callerContext.identity,
      callerContext.redactedInputs,
      callerContext.data,
      cancelToken,
      callerContext.services,
      callerContext.globalDeadline,
    );
    return cloned;
  }
}
