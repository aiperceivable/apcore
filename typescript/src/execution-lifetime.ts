/** Internal execution ownership; intentionally absent from package exports. */
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
}
