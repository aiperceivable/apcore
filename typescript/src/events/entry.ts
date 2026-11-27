import { EventEmitter as RuntimeEventEmitter } from './emitter.js';
import { type LifecycleCallbacks } from '../lifecycle.js';
import { createNodeLifecycleCallbacks } from '../lifecycle-node.js';

/** An event emitter with per-instance asynchronous callback ownership on Node. */
export class EventEmitter extends RuntimeEventEmitter {
  constructor(maxPending?: number, callbacks?: LifecycleCallbacks) {
    super(maxPending, callbacks ?? createNodeLifecycleCallbacks());
  }
}

export { createEvent, resolveSubscriberType } from './emitter.js';
export type { ApCoreEvent, EventSubscriber } from './emitter.js';
export { WebhookSubscriber, FileSubscriber, StdoutSubscriber, FilterSubscriber } from './subscribers.js';
export { CircuitBreakerWrapper, CircuitState } from './circuit-breaker.js';
export type { CircuitBreakerConfig } from './circuit-breaker.js';
export type { RetryConfig, RetryConfig as EventRetryConfig } from './retry.js';
