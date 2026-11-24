import type { OverridesStore } from './overrides.js';

/** File-backed stores distinguish an empty file from a failed update read. */
export const readOverridesForUpdate = Symbol('readOverridesForUpdate');

type UpdateStore = OverridesStore & {
  [readOverridesForUpdate]?: () => ReturnType<OverridesStore['load']>;
};

const pendingUpdates = new WeakMap<OverridesStore, Promise<void>>();

/** Serialize whole-map updates shared by the control modules. */
export function persistOverride(store: UpdateStore, key: string, value: unknown): void {
  const reportError = (error: unknown): void => {
    console.warn(`[apcore:control] Failed to persist override for key '${key}':`, error);
  };
  const write = (): void | Promise<void> => {
    const existing = store[readOverridesForUpdate]?.() ?? store.load();
    const save = (snapshot: Record<string, unknown>): void | Promise<void> => {
      if (snapshot === null || typeof snapshot !== 'object' || Array.isArray(snapshot)) {
        throw new TypeError('OverridesStore.load() must return a mapping');
      }
      return store.save({ ...snapshot, [key]: value });
    };
    return typeof (existing as Partial<Promise<unknown>> | null)?.then === 'function'
      ? Promise.resolve(existing).then(save)
      : save(existing as Record<string, unknown>);
  };

  try {
    const pending = pendingUpdates.get(store);
    const result = pending === undefined ? write() : pending.then(write);
    if (result !== undefined) {
      const tracked = Promise.resolve(result).catch(reportError).finally(() => {
        if (pendingUpdates.get(store) === tracked) pendingUpdates.delete(store);
      });
      pendingUpdates.set(store, tracked);
    }
  } catch (error) {
    reportError(error);
  }
}
