/** Internal execution ownership; intentionally absent from package exports. */
const parents = new WeakMap<object, object>();
const pendingExecutions = new WeakMap<object, Set<Promise<void>>>();

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
