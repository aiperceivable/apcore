import { CallBudgetExceededError, InvalidInputError, ModuleError } from './errors.js';
import type { Context } from './context.js';
import type { PipelineContext } from './pipeline.js';

export interface CallBudgetOptions {
  readonly maxCalls?: number;
  readonly maxCost?: number;
  readonly estimatedCosts?: Readonly<Record<string, number>>;
}

export interface CallBudgetSnapshot {
  readonly callsUsed: number;
  readonly callsReserved: number;
  readonly costUsed: number;
}

interface Ledger {
  readonly maxCalls: number;
  readonly maxCost: bigint | null;
  readonly estimates: ReadonlyMap<string, bigint>;
  callsUsed: number;
  callsReserved: number;
  costUsed: bigint;
}

const MAX_SAFE_COST = BigInt(Number.MAX_SAFE_INTEGER);
const ledgers = new WeakMap<CallBudget, Ledger>();

function amount(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new InvalidInputError(`${label} must be a non-negative safe integer`);
  }
  return value;
}

/** In-process shared limits for all invocations in a call tree. */
export class CallBudget {
  constructor(options: CallBudgetOptions = {}) {
    const estimates = new Map(Object.entries(options.estimatedCosts ?? {}).map(([id, estimate]) =>
      [id, BigInt(amount(estimate, `Estimated cost for '${id}'`))] as const));
    ledgers.set(this, {
      maxCalls: options.maxCalls === undefined ? Number.MAX_SAFE_INTEGER : amount(options.maxCalls, 'Maximum calls'),
      maxCost: options.maxCost === undefined ? null : BigInt(amount(options.maxCost, 'Maximum cost')),
      estimates, callsUsed: 0, callsReserved: 0, costUsed: 0n,
    });
  }

  /** Report usage. An overrun beyond the numeric range is clamped in this view. */
  snapshot(): CallBudgetSnapshot {
    const { callsUsed, callsReserved, costUsed } = ledgers.get(this)!;
    return Object.freeze({ callsUsed, callsReserved, costUsed: Number(costUsed > MAX_SAFE_COST ? MAX_SAFE_COST : costUsed) });
  }
}

class BudgetPipelineError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE = false;

  constructor() {
    super('PIPELINE_CONFIGURATION_ERROR', 'A budgeted call requires its original context, call-chain guard, and dispatch');
  }
}

interface Attempt {
  readonly id: string;
  readonly parent: Context;
  readonly ledger: Ledger;
  readonly estimate: bigint;
  context?: Context;
  state: 'initial' | 'reserved' | 'running' | 'settled' | 'released';
  removeCancelListener?: () => void;
  actualRecorded?: boolean;
  overrun?: CallBudgetExceededError;
  producer?: AsyncGenerator<Record<string, unknown>>;
}

const budgets = new WeakMap<Context<unknown>, CallBudget>();
const scopes = new WeakMap<Context<unknown>, Attempt>();
const attempts = new WeakMap<PipelineContext, Attempt>();

/** @internal Set a budget on a derived context. */
export function attachBudget(context: Context<unknown>, budget: CallBudget | null): void {
  if (budget !== null && !ledgers.has(budget)) throw new InvalidInputError('Budget must be a CallBudget');
  if (budget === null) budgets.delete(context);
  else budgets.set(context, budget);
  scopes.delete(context);
}

/** @internal Copy the budget to a runtime derivation, sharing settlement only within the same invocation. */
export function inheritBudget(source: Context<unknown>, target: Context<unknown>, inheritSettlement: boolean): void {
  const budget = budgets.get(source);
  if (budget) budgets.set(target, budget);
  const attempt = inheritSettlement ? scopes.get(source) : undefined;
  if (attempt) scopes.set(target, attempt);
}

/** @internal Return whether budgeting has been enabled on this context. */
export function hasCallBudget(context: Context<unknown>): boolean {
  return budgets.has(context);
}

/** @internal Open attempt state before running arbitrary pipeline hooks. */
export function beginBudgetAttempt(ctx: PipelineContext): void {
  const budget = budgets.get(ctx.context);
  if (!budget) return;
  const ledger = ledgers.get(budget)!;
  attempts.set(ctx, { id: ctx.moduleId, parent: ctx.context, ledger, estimate: ledger.estimates.get(ctx.moduleId) ?? 0n, state: 'initial' });
}

/** @internal Bind the invocation child created by the stock context step. */
export function bindBudgetContext(ctx: PipelineContext, context: Context): void {
  const attempt = attempts.get(ctx);
  if (!attempt) return;
  if (ctx.context !== attempt.parent || ctx.moduleId !== attempt.id) throw new BudgetPipelineError();
  attempt.context = context;
}

function boundAttempt(ctx: PipelineContext): Attempt | undefined {
  const attempt = attempts.get(ctx);
  if (attempt && (attempt.context !== ctx.context || attempt.id !== ctx.moduleId)) throw new BudgetPipelineError();
  return attempt;
}

function releaseReservation(attempt: Attempt): void {
  if (attempt.state !== 'reserved') return;
  attempt.ledger.callsReserved--;
  attempt.state = 'released';
  attempt.removeCancelListener?.();
}

/** @internal Reserve the call at the stock call-chain guard. */
export function reserveBudgetCall(ctx: PipelineContext): void {
  const attempt = boundAttempt(ctx);
  if (!attempt) return;
  if (attempt.state !== 'initial') throw new BudgetPipelineError();
  const { ledger } = attempt;
  if (ledger.callsUsed + ledger.callsReserved >= ledger.maxCalls) throw new CallBudgetExceededError('calls');
  ledger.callsReserved++;
  attempt.state = 'reserved';
  const signal = ctx.context.signal;
  const release = (): void => releaseReservation(attempt);
  signal.addEventListener('abort', release, { once: true });
  attempt.removeCancelListener = () => signal.removeEventListener('abort', release);
}

/** @internal Spend the reserved call and its estimate together at business dispatch. */
export function commitBudgetCall(ctx: PipelineContext): (() => void) | undefined {
  const attempt = boundAttempt(ctx);
  if (!attempt) return undefined;
  if (attempt.state !== 'reserved') throw new BudgetPipelineError();
  const { ledger } = attempt;
  const cost = ledger.costUsed + attempt.estimate;
  if (ledger.maxCost !== null && cost > ledger.maxCost) throw new CallBudgetExceededError('cost');
  ledger.callsReserved--;
  ledger.callsUsed++;
  ledger.costUsed = cost;
  attempt.removeCancelListener?.();
  attempt.state = 'running';
  scopes.set(ctx.context, attempt);
  return () => { attempt.state = 'settled'; };
}

/** @internal Replace an active attempt's estimate with actual cost once. */
export function recordBudgetCost(context: Context<unknown>, actual: number): void {
  if (!budgets.has(context)) return;
  const attempt = scopes.get(context);
  if (attempt?.state !== 'running' || attempt.actualRecorded) {
    throw new InvalidInputError('Cost can be reported once during an active budgeted execution');
  }
  const settled = BigInt(amount(actual, 'Actual cost'));
  attempt.actualRecorded = true;
  const { ledger } = attempt;
  ledger.costUsed += settled - attempt.estimate;
  if (ledger.maxCost !== null && ledger.costUsed > ledger.maxCost) {
    attempt.overrun = new CallBudgetExceededError('cost');
    throw attempt.overrun;
  }
}

/** @internal Keep the registered stream producer separate from mutable step state. */
export function recordBudgetProducer(ctx: PipelineContext, producer: AsyncGenerator<Record<string, unknown>>): void {
  const attempt = boundAttempt(ctx);
  if (!attempt) return;
  if (attempt.state !== 'reserved') throw new BudgetPipelineError();
  attempt.producer = producer;
}

/** @internal Resolve the registered producer for budgeted calls or the existing producer for other calls. */
export function budgetStreamProducer(ctx: PipelineContext): PipelineContext['outputStream'] {
  const attempt = attempts.get(ctx);
  return attempt ? attempt.producer : ctx.outputStream;
}

/** @internal Fail a budgeted traversal if it bypassed stock dispatch. */
export function assertBudgetDispatch(ctx: PipelineContext, allowProducer: boolean = false): void {
  const attempt = boundAttempt(ctx);
  if (attempt && attempt.state !== 'running' && attempt.state !== 'settled' && !(allowProducer && attempt.producer)) {
    throw new BudgetPipelineError();
  }
  if (attempt?.overrun) throw attempt.overrun;
}

/** @internal Release a count that could not reach business dispatch. */
export function releaseBudgetReservation(ctx: PipelineContext): void {
  const attempt = attempts.get(ctx);
  if (attempt) releaseReservation(attempt);
}

/** @internal Clear attempt state before a retry or after a completed traversal. */
export function endBudgetAttempt(ctx: PipelineContext): void {
  releaseBudgetReservation(ctx);
  attempts.delete(ctx);
}

/** @internal Budget control failures cannot be recovered into successful outputs. */
export function isBudgetControlError(error: unknown): boolean {
  return error instanceof CallBudgetExceededError || error instanceof BudgetPipelineError;
}
