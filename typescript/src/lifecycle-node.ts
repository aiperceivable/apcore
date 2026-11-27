import { AsyncLocalStorage } from 'node:async_hooks';
import type { APCorePlugin } from './client.js';
import { LifecycleCallbacks, type LifecycleCallbackScope } from './lifecycle.js';

export function createNodeLifecycleCallbacks(): LifecycleCallbacks {
  return new LifecycleCallbacks(new AsyncLocalStorage<LifecycleCallbackScope>());
}

/** Preserve callback ownership across asynchronous suspension for one Node client. */
export function createNodeLifecyclePlugin(): APCorePlugin {
  return { createLifecycleCallbacks: createNodeLifecycleCallbacks };
}
