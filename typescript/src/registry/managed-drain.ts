import {
  type DrainAbortReason,
  type DrainContext,
  type LifecycleCallbacks,
  LifecycleCompletion,
  lifecycleError,
  type LifecycleProgress,
  type ModuleLifecycleProgress,
} from '../lifecycle.js';
import type { Module } from '../module.js';

/** @internal Opt-in policy for one generation. */
export interface ManagedDrainPolicy {
  readonly timeoutMs: number;
  readonly action: 'continue_waiting' | 'abort_reload' | 'cancel_then_force';
  readonly cancelGraceMs?: number;
}

interface DrainOptions {
  readonly owner: LifecycleProgress;
  readonly callbackOwner: object;
  readonly module: Module;
  readonly progress: ModuleLifecycleProgress;
  readonly policy: ManagedDrainPolicy;
  readonly activeCount: () => number;
  readonly idle: Promise<unknown>;
  readonly cancelExecutions: () => void;
  readonly reportTimeout: (activeCount: number) => void;
}

/** @internal Stop or wait for exactly the executions retained by one removal. */
export async function runManagedDrain(options: DrainOptions): Promise<{ forced: boolean; abort?: DrainAbortReason }> {
  const { owner, callbackOwner, module, progress, policy, activeCount, idle } = options;
  const deadline = performance.now() + policy.timeoutMs;
  const signal = new AbortController();
  const decision = new LifecycleCompletion();
  const isReload = owner.report.kind === 'reload';
  const canAdvance = (): boolean => progress.activeHook === undefined &&
    ((isReload && progress.error !== undefined) || activeCount() === 0);
  const wakeIfReady = (): void => { if (canAdvance()) decision.settle(); };
  const context: DrainContext = {
    operationId: owner.report.operationId, moduleId: progress.moduleId,
    reasonKind: owner.report.kind === 'unregister_many' ? 'unregister' : owner.report.kind, signal: signal.signal,
    remainingMs: () => Math.max(0, Math.floor(deadline - performance.now())),
  };
  void idle.then(wakeIfReady);
  const hook = invokeDrainHook(module, context, progress, owner.callbacks, [callbackOwner, owner], wakeIfReady);
  wakeIfReady();
  await decision.wait(Math.max(0, Math.ceil(deadline - performance.now())));

  // Resolve simultaneous failure/deadline observations as drain_error.
  if (isReload && progress.error) return { forced: false, abort: 'drain_error' };
  if (canAdvance()) return { forced: false };

  progress.drainTimedOut = true;
  options.reportTimeout(activeCount());
  if (progress.activeHook === 'on_drain') signal.abort();
  if (policy.action === 'abort_reload') {
    await hook;
    return { forced: false, abort: 'drain_timeout' };
  }
  if (policy.action === 'cancel_then_force') {
    const grace = new LifecycleCompletion();
    void idle.then(() => grace.settle());
    options.cancelExecutions();
    await Promise.all([hook, grace.wait(policy.cancelGraceMs ?? 1_000)]);
    return { forced: activeCount() > 0 };
  }
  await decision.wait();
  return isReload && progress.error ? { forced: false, abort: 'drain_error' } : { forced: false };
}

function invokeDrainHook(
  module: Module,
  context: DrainContext,
  progress: ModuleLifecycleProgress,
  callbacks: LifecycleCallbacks,
  owners: readonly object[],
  onSettle: () => void,
): void | Promise<void> {
  const finish = (): void => { delete progress.activeHook; onSettle(); };
  const fail = (error: unknown): void => {
    progress.error = lifecycleError(error, 'draining');
    finish();
  };
  try {
    const hook = module.onDrain;
    if (typeof hook !== 'function') return;
    progress.activeHook = 'on_drain';
    const work = callbacks.run(owners, () => hook.call(module, context));
    if (work !== undefined) return Promise.resolve(work).then(finish, fail);
    finish();
  } catch (error) {
    fail(error);
  }
}
