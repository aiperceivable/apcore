import { awaitExecution, type CancelToken, ExecutionCancelledError } from './cancel.js';
import { ModuleError, ModuleTimeoutError, PolicyEvaluationError } from './errors.js';
import { trackExecution } from './execution-lifetime.js';
import { isModuleIdempotent } from './module.js';
import type { PipelineContext, StepTrace } from './pipeline.js';
import { policyInvocationOf } from './policy-evaluation.js';

export type StreamCompletionReason = 'completed' | 'failed' | 'cancelled' | 'timed_out' | 'consumer_closed';

export type StreamStepOutcome =
  | { readonly status: 'passed' }
  | { readonly status: 'failed'; readonly disposition: 'unhandled' | 'ignored' | 'recovered' }
  | { readonly status: 'skipped'; readonly reason: 'not_configured' | 'not_reached' | 'no_match' | 'strategy_skipped' };

type Chunk = Record<string, unknown>;

export interface StreamCompletion {
  readonly reason: StreamCompletionReason;
  /** Detached invocation output at the output-validation boundary. May contain sensitive data. */
  readonly output: Chunk | null;
  readonly outputValidation: StreamStepOutcome;
  readonly middlewareAfter: StreamStepOutcome;
  readonly errorCode: string | null;
}

export interface StreamExecution {
  readonly chunks: AsyncIterableIterator<Chunk> & { return(): Promise<IteratorResult<Chunk>> };
  /** Drain or close chunks before awaiting this promise. Execution failures are returned as values. */
  readonly completion: Promise<StreamCompletion>;
  /** Stop consumption and signal the producer. Physical cleanup may continue independently. */
  close(): Promise<void>;
}

function classifyStep(step: StepTrace | undefined, configured: boolean): StreamStepOutcome {
  if (step === undefined) {
    return { status: 'skipped', reason: configured ? 'not_reached' : 'not_configured' };
  }
  switch (step.skipReason) {
    case 'error_ignored': return { status: 'failed', disposition: 'ignored' };
    case 'error_recovered': return { status: 'failed', disposition: 'recovered' };
    case 'no_match': return { status: 'skipped', reason: 'no_match' };
  }
  if (step.skipped) return { status: 'skipped', reason: 'strategy_skipped' };
  if (step.result.action === 'abort') return { status: 'failed', disposition: 'unhandled' };
  return { status: 'passed' };
}

/** @internal Per-invocation state shared by the producer and public iterator. */
export class StreamCompletionTracker {
  context: PipelineContext | null = null;
  output: Chunk | null = null;
  inPostProcessing = false;
  terminalErrorCode: string | null = null;
  armDeadline?: (deadline: number | null) => void;
  watchPostCancellation?: (token: CancelToken | null) => void;

  constructor(private readonly configuredSteps: readonly string[]) {}

  result(reason: StreamCompletionReason, errorCode: string | null): StreamCompletion {
    const context = this.context;
    // A native Phase 1 deliberately skips post steps. Only the real Phase 3
    // trace may assign an outcome to those steps.
    const trace = context?.outputStream != null && !this.inPostProcessing ? null : context?.trace;
    const step = (name: string) => classifyStep(
      trace?.steps.find((entry) => entry.name === name),
      (context?.strategy?.steps.some((candidate) => candidate.name === name)) ?? this.configuredSteps.includes(name),
    );
    return {
      reason, errorCode, output: this.output,
      outputValidation: step('output_validation'), middlewareAfter: step('middleware_after'),
    };
  }
}

function errorCodeOf(error: unknown): string {
  return error instanceof ModuleError ? error.code : 'MODULE_EXECUTE_ERROR';
}

function errorReason(code: string): StreamCompletionReason {
  if (code === 'EXECUTION_CANCELLED') return 'cancelled';
  if (code === 'MODULE_TIMEOUT') return 'timed_out';
  return 'failed';
}

/** @internal Adapt one lazy generator to a closeable chunk stream and completion receipt. */
export function createStreamExecution(
  source: AsyncGenerator<Chunk>,
  token: CancelToken,
  moduleId: string,
  tracker: StreamCompletionTracker,
  callerDeadline: number | null,
): StreamExecution {
  let resolve!: (receipt: StreamCompletion) => void;
  const completion = new Promise<StreamCompletion>((publish) => { resolve = publish; });
  let settled = false;
  let closed = false;
  let startedAt: number | undefined;
  let deadlineMs: number | undefined;
  let deadlineTimer: ReturnType<typeof setTimeout> | undefined;
  let interruption: ModuleError | undefined;
  let postSignal: AbortSignal | undefined;
  const end = { done: true, value: undefined } as const;

  function settle(reason: StreamCompletionReason, errorCode = tracker.terminalErrorCode): void {
    if (settled) return;
    settled = true;
    clearTimeout(deadlineTimer);
    token.signal.removeEventListener('abort', onCancelled);
    postSignal?.removeEventListener('abort', onCancelled);
    token.dispose();
    resolve(tracker.result(errorCode === null ? reason : errorReason(errorCode), errorCode));
  }

  function stopSource(): void {
    const cleanup = source.return(undefined);
    trackExecution(token, cleanup);
    void cleanup.catch(() => console.warn(`[apcore:executor] Stream cleanup failed for '${moduleId}'`));
  }

  function onCancelled(): void {
    interruption = new ExecutionCancelledError();
    settle('cancelled', interruption.code);
    token.cancel();
    stopSource();
  }

  function expireDeadline(): void {
    const budget = Math.max(0, Math.ceil(deadlineMs! - startedAt!));
    const context = tracker.context;
    interruption = context !== null && policyInvocationOf(context)?.evaluating
      ? new PolicyEvaluationError(moduleId)
      : new ModuleTimeoutError(moduleId, budget, { retryable: isModuleIdempotent(context?.module) });
    // Publish the deadline's own error before the interruption reaches the
    // producer as cancellation. Its physical shutdown may still be pending.
    settle('timed_out', interruption.code);
    token.cancel();
    stopSource();
  }

  function scheduleDeadline(): void {
    const remaining = deadlineMs! - Date.now();
    if (remaining <= 0) expireDeadline();
    else deadlineTimer = setTimeout(scheduleDeadline, Math.min(Math.ceil(remaining), 2_147_483_647));
  }

  tracker.armDeadline = (deadline) => {
    if (settled || deadlineMs !== undefined || deadline === null || !Number.isFinite(deadline)) return;
    deadlineMs = deadline * 1000;
    scheduleDeadline();
  };
  tracker.watchPostCancellation = (postToken) => {
    if (settled || postSignal || postToken === null) return;
    postSignal = postToken.signal;
    if (postSignal.aborted) onCancelled();
    else postSignal.addEventListener('abort', onCancelled, { once: true });
  };

  async function close(): Promise<void> {
    if (settled) return;
    closed = true;
    settle('consumer_closed');
    token.cancel();
    stopSource();
  }

  const chunks: StreamExecution['chunks'] = {
    [Symbol.asyncIterator]() { return this; },
    async next() {
      if (settled) return end;
      try {
        if (startedAt === undefined) {
          startedAt = Date.now();
          if (token.signal.aborted) onCancelled();
          else token.signal.addEventListener('abort', onCancelled, { once: true });
          tracker.armDeadline?.(callerDeadline);
          if (interruption) throw interruption;
        }
        const result = await awaitExecution(source.next(), token, moduleId, 0, false);
        if (closed) return end;
        if (!settled && deadlineMs !== undefined && Date.now() >= deadlineMs) expireDeadline();
        if (interruption) throw interruption;
        if (result.done) settle('completed');
        return result;
      } catch (error) {
        if (closed) return end;
        const terminalError = interruption ?? error;
        settle('failed', errorCodeOf(terminalError));
        throw terminalError;
      }
    },
    async return() {
      await close();
      return end;
    },
  };
  return { chunks, completion, close };
}
