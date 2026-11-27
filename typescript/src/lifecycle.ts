import { uuidV4 as uuidv4 } from './utils/uuid.js';
import { ErrorCodes, InvalidInputError, ModuleError } from './errors.js';

/** Options for the registry drain and this client's event shutdown. */
export interface ShutdownOptions {
  drainTimeoutMs?: number;
  onDrainTimeout?: 'continue_waiting' | 'cancel_then_force';
  cancelGraceMs?: number;
  eventFlushTimeoutMs?: number;
}

/** Options for an ordered batch of generation-specific removals. */
export interface UnregisterOptions {
  drainTimeoutMs?: number;
  onDrainTimeout?: 'continue_waiting' | 'cancel_then_force';
  cancelGraceMs?: number;
  operationTimeoutMs?: number;
}

/** Options for retiring a changed source before notifying the host. */
export type WatchOptions = Omit<UnregisterOptions, 'operationTimeoutMs'>;

export type LifecycleStatus = 'pending' | 'completed' | 'failed';
export type LifecycleEventFlush = 'not_applicable' | 'pending' | 'completed' | 'timeout';
export type DrainAbortReason = 'drain_timeout' | 'drain_error';

/** Supplied to the module while its current work drains. */
export interface DrainContext {
  readonly operationId: string;
  readonly moduleId: string;
  readonly reasonKind: 'shutdown' | 'unregister' | 'reload';
  readonly signal: AbortSignal;
  remainingMs(): number;
}

/** Supplied when the registry asks an existing module to resume service. */
export interface DrainAbortedContext {
  readonly operationId: string;
  readonly moduleId: string;
  readonly signal: AbortSignal;
  readonly abortReason: DrainAbortReason;
}

export interface LifecycleError {
  readonly code: string;
  readonly phase: 'shutdown' | 'draining' | 'recovering' | 'unloading' | 'loading' | 'resuming' | 'event_flush';
}

export interface LifecycleModuleReport {
  readonly moduleId: string;
  readonly phase: 'pending' | 'draining' | 'recovering' | 'unloading' | 'loading' | 'resuming' | 'done';
  readonly outcome?: 'removed' | 'forced_removed' | 'absent' | 'already_removing' | 'not_started'
    | 'retained_ready' | 'retained_unavailable' | 'reloaded' | 'failed';
  readonly drainTimedOut?: boolean;
  readonly activeHook?: 'on_drain' | 'on_drain_aborted' | 'on_suspend' | 'on_unload' | 'on_load' | 'on_resume';
  readonly activeAtUnload?: number;
  readonly cleanupPending?: boolean;
  readonly error?: LifecycleError;
  readonly abortReason?: DrainAbortReason;
  readonly compensation?: 'not_required' | 'restored' | 'failed';
}

export interface LifecycleReport {
  readonly operationId: string;
  readonly kind: 'shutdown' | 'unregister_many' | 'reload';
  readonly status: LifecycleStatus;
  readonly modules: readonly LifecycleModuleReport[];
  readonly eventFlush: LifecycleEventFlush;
  readonly stopReason?: 'operation_deadline';
  readonly error?: LifecycleError;
}

/** A handle to work owned by the registry and its client. */
export interface LifecycleOperation {
  snapshot(): LifecycleReport;
  /** Return a snapshot on timeout. An abort cancels only this wait. */
  wait(waitTimeoutMs?: number, signal?: AbortSignal): Promise<LifecycleReport>;
}

/** @internal Mutable state of one published report row. */
export type ModuleLifecycleProgress = { -readonly [K in keyof LifecycleModuleReport]: LifecycleModuleReport[K] };

type Progress = Omit<{ -readonly [K in keyof LifecycleReport]: LifecycleReport[K] }, 'modules'> & { modules: ModuleLifecycleProgress[] };

/** @internal Validate a duration before starting work or creating a timer. */
export function validateLifecycleDuration(name: string, value: unknown): void {
  if (value === undefined) return;
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > 2_147_483_647) {
    throw new InvalidInputError(`${name} must be an integer in 0..2147483647`);
  }
}

/** @internal */
export function validateLifecycleOptions(options: ShutdownOptions | UnregisterOptions, budget?: 'eventFlushTimeoutMs' | 'operationTimeoutMs'): void {
  if (options === null || typeof options !== 'object' || Array.isArray(options)) {
    throw new InvalidInputError('Lifecycle options must be an object');
  }
  for (const name of Object.keys(options)) {
    if (name !== 'drainTimeoutMs' && name !== 'onDrainTimeout' && name !== 'cancelGraceMs' && name !== budget) {
      throw new InvalidInputError(`Unrecognized lifecycle option '${name}'`);
    }
  }
  const values = options as Record<string, unknown>;
  validateLifecycleDuration('drainTimeoutMs', values['drainTimeoutMs']);
  if (budget !== undefined) validateLifecycleDuration(budget, values[budget]);
  validateLifecycleDuration('cancelGraceMs', values['cancelGraceMs']);
  if (options.onDrainTimeout !== undefined && options.onDrainTimeout !== 'continue_waiting' &&
      options.onDrainTimeout !== 'cancel_then_force') {
    throw new InvalidInputError('onDrainTimeout must be continue_waiting or cancel_then_force');
  }
  if (options.cancelGraceMs !== undefined && options.onDrainTimeout !== 'cancel_then_force') {
    throw new InvalidInputError('cancelGraceMs requires cancel_then_force');
  }
}

/** @internal */
export function lifecycleError(error: unknown, phase: LifecycleError['phase']): LifecycleError {
  return Object.freeze({ code: error instanceof ModuleError ? error.code : ErrorCodes.GENERAL_INTERNAL_ERROR, phase });
}

/** @internal */
export function freezeLifecycleReport(progress: LifecycleReport): LifecycleReport {
  return Object.freeze({ ...progress, modules: Object.freeze(progress.modules.map(item => Object.freeze({ ...item }))) });
}

/** @internal The canonical report shape for system.control.reload_module. */
export function lifecycleReportToWire(report: LifecycleReport): Record<string, unknown> {
  return {
    operation_id: report.operationId, kind: report.kind, status: report.status, event_flush: report.eventFlush,
    modules: report.modules.map(row => ({
      module_id: row.moduleId, phase: row.phase,
      ...(row.outcome === undefined ? {} : { outcome: row.outcome }),
      ...(row.drainTimedOut === undefined ? {} : { drain_timed_out: row.drainTimedOut }),
      ...(row.activeHook === undefined ? {} : { active_hook: row.activeHook }),
      ...(row.activeAtUnload === undefined ? {} : { active_at_unload: row.activeAtUnload }),
      ...(row.cleanupPending === undefined ? {} : { cleanup_pending: row.cleanupPending }),
      ...(row.abortReason === undefined ? {} : { abort_reason: row.abortReason }),
      ...(row.compensation === undefined ? {} : { compensation: row.compensation }),
      ...(row.error === undefined ? {} : { error: row.error }),
    })),
    ...(report.error === undefined ? {} : { error: report.error }),
    ...(report.stopReason === undefined ? {} : { stop_reason: report.stopReason }),
  };
}

/** @internal Completion independent of waiters, including those that time out. */
export class LifecycleCompletion {
  private _settled = false;
  private readonly _waiters = new Set<() => void>();

  get settled(): boolean { return this._settled; }

  settle(): void {
    this._settled = true;
    for (const finish of this._waiters) finish();
  }

  async wait(timeoutMs?: number, signal?: AbortSignal): Promise<void> {
    signal?.throwIfAborted();
    if (this._settled || timeoutMs === 0) return;
    await new Promise<void>((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | undefined;
      const cleanup = (): void => {
        this._waiters.delete(finish);
        if (timer !== undefined) clearTimeout(timer);
        signal?.removeEventListener('abort', abort);
      };
      const finish = (): void => {
        cleanup();
        resolve();
      };
      const abort = (): void => {
        cleanup();
        reject(signal?.reason ?? new DOMException('Lifecycle wait aborted', 'AbortError'));
      };
      this._waiters.add(finish);
      signal?.addEventListener('abort', abort, { once: true });
      if (timeoutMs !== undefined) timer = setTimeout(finish, timeoutMs);
    });
  }
}

/** @internal */
export function lifecycleHandle(
  snapshot: () => LifecycleReport,
  completion: LifecycleCompletion,
  callbackWaitError: () => InvalidInputError | undefined,
): LifecycleOperation {
  return Object.freeze({ snapshot, async wait(timeoutMs?: number, signal?: AbortSignal): Promise<LifecycleReport> {
    validateLifecycleDuration('waitTimeoutMs', timeoutMs);
    const error = timeoutMs === 0 || completion.settled ? undefined : callbackWaitError();
    if (error) throw error;
    await completion.wait(timeoutMs, signal);
    return snapshot();
  } });
}

/** @internal Registry-owned progress and task. */
export class LifecycleProgress {
  readonly report: Progress;
  readonly operation: LifecycleOperation;
  private readonly _completion = new LifecycleCompletion();

  constructor(
    kind: LifecycleReport['kind'], modules: ModuleLifecycleProgress[], callbackOwners: readonly object[],
    readonly callbacks: LifecycleCallbacks,
  ) {
    this.report = { operationId: uuidv4(), kind, status: 'pending', modules, eventFlush: 'not_applicable' };
    const owners = [...callbackOwners, this];
    this.operation = lifecycleHandle(() => freezeLifecycleReport(this.report), this._completion, () => callbacks.waitError(owners));
  }

  finish(): void {
    const error = this.report.modules.find(item => item.error !== undefined)?.error;
    if (error) this.report.error = error;
    const incompleteReload = this.report.kind === 'reload' && this.report.modules.some(item => item.outcome !== 'reloaded');
    this.report.status = error || this.report.stopReason || incompleteReload ? 'failed' : 'completed';
    this._completion.settle();
  }

  run(work: () => Promise<void>): Promise<void> {
    const completed = Promise.resolve().then(work).then(() => this.finish(), (error: unknown) => {
      this.report.status = 'failed';
      this.report.error = lifecycleError(error, this.report.kind === 'shutdown' ? 'shutdown' : 'unloading');
      this._completion.settle();
      throw error;
    });
    // The operation owns the task even when no waiter has attached to it.
    void completed.catch(() => {});
    return completed;
  }
}

export interface LifecycleCallbackScope {
  readonly owners: readonly object[];
  readonly parent: LifecycleCallbackScope | undefined;
  active: boolean;
}

export interface LifecycleCallbackContext {
  getStore(): LifecycleCallbackScope | undefined;
  run<T>(scope: LifecycleCallbackScope, callback: () => T): T;
}

function synchronousCallbackContext(): LifecycleCallbackContext {
  let current: LifecycleCallbackScope | undefined;
  return {
    getStore: () => current,
    run<T>(scope: LifecycleCallbackScope, callback: () => T): T {
      const previous = current;
      current = scope;
      try {
        return callback();
      } finally {
        current = previous;
      }
    },
  };
}

/** Tracks which callbacks are currently holding up a lifecycle operation. */
export class LifecycleCallbacks {
  constructor(private readonly context: LifecycleCallbackContext = synchronousCallbackContext()) {}

  waitError(owners: readonly object[]): InvalidInputError | undefined {
    for (let scope = this.context.getStore(); scope !== undefined; scope = scope.parent) {
      if (scope.active && scope.owners.some(owner => owners.includes(owner))) {
        return new InvalidInputError('A lifecycle callback cannot wait for its own operation');
      }
    }
    return undefined;
  }

  run<T>(owners: readonly object[], callback: () => T): T {
    const scope: LifecycleCallbackScope = { owners, parent: this.context.getStore(), active: true };
    const finish = (): void => { scope.active = false; };
    try {
      const value = this.context.run(scope, callback);
      if (value != null && typeof (value as { then?: unknown }).then === 'function') {
        void Promise.resolve(value).then(finish, finish);
      } else {
        finish();
      }
      return value;
    } catch (error) {
      finish();
      throw error;
    }
  }
}
