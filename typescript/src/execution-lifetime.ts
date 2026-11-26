/** Internal execution ownership; intentionally absent from package exports. */
import type { Context } from './context.js';
import type { Span } from './observability/tracing.js';

const parents = new WeakMap<object, object>();
const pendingExecutions = new WeakMap<object, Set<Promise<void>>>();
const callCleanup = new WeakMap<object, Set<() => void>>();

export function linkExecutionParent(scope: object, parent: object | null): void {
  if (parent !== null) parents.set(scope, parent);
}

/** Register physical execution, including all ancestor cancellation owners. */
export function trackExecution(scope: object, execution: PromiseLike<unknown>): void {
  const settled = Promise.resolve(execution).then(() => {}, () => {});
  const owners: Set<Promise<void>>[] = [];
  for (let current: object | undefined = scope; current; current = parents.get(current)) {
    let pending = pendingExecutions.get(current);
    if (!pending) {
      pending = new Set();
      pendingExecutions.set(current, pending);
    }
    pending.add(settled);
    owners.push(pending);
  }
  void settled.then(() => {
    for (const pending of owners) pending.delete(settled);
  });
}

export async function drainExecutions(scope: object): Promise<void> {
  const pending = pendingExecutions.get(scope);
  while (pending && pending.size > 0) await Promise.all([...pending]);
}

/** Register cleanup for a call regardless of how its middleware chain settles. */
export function addCallCleanup(scope: object, cleanup: () => void): () => void {
  const callbacks = callCleanup.get(scope) ?? new Set<() => void>();
  callCleanup.set(scope, callbacks);
  callbacks.add(cleanup);
  return () => { callbacks.delete(cleanup); };
}

export function finishCall(scope: object): void {
  const callbacks = callCleanup.get(scope);
  callCleanup.delete(scope);
  for (const cleanup of callbacks ?? []) cleanup();
  const observation = observations.get(scope);
  observation?.timings.clear();
  if (observation) observation.spans.length = 0;
}

interface ObservationState {
  timings: Map<object, number>;
  spans: Array<{ owner: object; span: Span }>;
  parentSpan?: Span;
  parentId?: string;
  lastSpanId?: string;
}

const observations = new WeakMap<object, ObservationState>();

function observationState(context: Context<unknown>): ObservationState {
  let state = observations.get(context);
  if (!state) {
    state = { timings: new Map(), spans: [] };
    observations.set(context, state);
  }
  return state;
}

export function copyObservationState(from: Context<unknown>, to: Context<unknown>): void {
  const state = observations.get(from);
  if (state) observations.set(to, state);
}

export function inheritObservationParent(from: Context<unknown>, to: Context<unknown>): void {
  const parent = observations.get(from);
  if (!parent) return;
  const span = currentSpan(from);
  const state = observationState(to);
  state.parentSpan = span ?? parent.parentSpan;
  state.parentId = span?.spanId ?? parent.lastSpanId ?? parent.parentId;
}

export function setRemoteSpanParent(context: Context<unknown>, parentId: string): void {
  observationState(context).parentId = parentId;
}

export function startMiddlewareTiming(context: Context<unknown>, owner: object, now: number): void {
  observationState(context).timings.set(owner, now);
}

export function takeMiddlewareTiming(context: Context<unknown>, owner: object): number | undefined {
  const timings = observations.get(context)?.timings;
  const started = timings?.get(owner);
  timings?.delete(owner);
  return started;
}

export function currentSpan(context: Context<unknown>): Span | undefined {
  return observations.get(context)?.spans.at(-1)?.span;
}

export function legacySpan(context: Context<unknown>): Span | undefined {
  const spans = context.data['_apcore.mw.tracing.spans'];
  return Array.isArray(spans) ? (spans.at(-1) as Span | undefined) : undefined;
}

export function spanParentId(context: Context<unknown>): string | null {
  return currentSpan(context)?.spanId ?? observations.get(context)?.parentId ?? legacySpan(context)?.spanId ?? null;
}

export function auditSpan(context: Context<unknown>): Span | undefined {
  const span = currentSpan(context) ?? observations.get(context)?.parentSpan;
  return span?.endTime === null ? span : legacySpan(context);
}

export function startSpan(context: Context<unknown>, owner: object, span: Span): void {
  const state = observationState(context);
  state.spans.push({ owner, span });
  state.lastSpanId = span.spanId;
}

export function removeSpan(context: Context<unknown>, owner: object): Span | undefined {
  const spans = observations.get(context)?.spans;
  if (!spans) return undefined;
  for (let index = spans.length - 1; index >= 0; index--) {
    if (spans[index].owner === owner) return spans.splice(index, 1)[0].span;
  }
  return undefined;
}
