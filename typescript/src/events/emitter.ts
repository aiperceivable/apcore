/**
 * Global event bus with fan-out delivery, per-subscriber retry, and DLQ.
 */

import { resolveRetry, computeDelayMs } from './retry.js';
import type { RetryConfig } from './retry.js';
import { matchGlob } from '../utils/pattern.js';
import { lifecycleCallback, LifecycleCompletion } from '../lifecycle.js';

export interface ApCoreEvent {
  readonly eventType: string;
  readonly moduleId: string | null;
  readonly timestamp: string;
  readonly severity: string;
  readonly data: Record<string, unknown>;
}

/** Internal delivery path for wrappers that otherwise isolate subscriber failures. */
export const deliverSubscriber = Symbol('apcore.event.deliverSubscriber');

export interface EventSubscriber {
  onEvent(event: ApCoreEvent): void | Promise<void>;
  /** @internal Return false when delivery is skipped because the circuit is open. */
  [deliverSubscriber]?(event: ApCoreEvent): Promise<boolean>;
  /** Optional stable ID for this subscriber, used in DLQ payloads and identity tracking. */
  readonly subscriberId?: string;
  /**
   * Optional declared subscriber kind (e.g. 'webhook', 'a2a', 'file'). Used as
   * the `subscriber_type` in DLQ payloads. When absent, the type is derived
   * from the constructor name (legacy fallback). Cross-SDK canonical
   * (Python/Rust, A-D-029) reads this declared field first.
   */
  readonly subscriberType?: string;
  /**
   * Optional glob pattern (supports * and ? wildcards). When set, the emitter
   * only delivers events whose `eventType` matches the pattern.
   */
  readonly eventPattern?: string;
  /**
   * Optional retry configuration. When absent, delivery uses the spec default
   * policy (max_attempts=3 with exponential backoff). When present, supplied
   * fields are merged over DEFAULT_RETRY; `maxAttempts: 1` disables retry.
   */
  readonly retry?: RetryConfig;
  /**
   * Optional callback invoked after all retry attempts are exhausted.
   * Receives the original event, the final error, and the total attempt count.
   * Errors thrown from onFailure are logged and suppressed.
   */
  onFailure?(event: ApCoreEvent, error: Error, attemptCount: number): void | Promise<void>;
}

export function createEvent(
  eventType: string,
  moduleId: string | null,
  severity: string,
  data: Record<string, unknown>,
): ApCoreEvent {
  return {
    eventType,
    moduleId,
    timestamp: new Date().toISOString(),
    severity,
    data,
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const DEFAULT_MAX_PENDING = 1000;

/**
 * Global event bus with fan-out delivery, per-subscriber retry, and DLQ.
 *
 * Key behaviors:
 * - All subscribers get exponential-backoff retries up to `maxAttempts`. A subscriber
 *   that omits `retry` receives the spec default policy (max_attempts=3); one that sets
 *   `maxAttempts: 1` disables retry (single attempt).
 * - After retry exhaustion: DLQ event + optional `onFailure` callback.
 * - `eventPattern` (glob) filters which events a subscriber receives.
 * - `flush()` drains all in-flight async deliveries.
 */
/**
 * The DECLARED subscriber kind, with the DLQ path's fallback.
 *
 * A-D-029: prefer the declared `subscriberType` field; derive from the
 * constructor name only when it is absent.
 *
 * Exported and shared because D-116 found the circuit breaker reporting a
 * DIFFERENT value for the same subscriber — a raw constructor name, without
 * even this normalisation — so a consumer routing on `subscriber_type` got one
 * answer for a delivery failure and another for the circuit opening. Two copies
 * of a default are how a surface drifts from the one it must agree with, which
 * is why this is one function rather than two expressions.
 */
export function resolveSubscriberType(subscriber: {
  subscriberType?: unknown;
}): string {
  if (typeof subscriber.subscriberType === 'string') return subscriber.subscriberType;
  return ((subscriber as { constructor?: { name?: string } }).constructor?.name ?? 'unknown')
    .replace('Subscriber', '')
    .toLowerCase();
}

export class EventEmitter {
  private _subscribers: EventSubscriber[] = [];
  private readonly _pending = new Set<Promise<void>>();
  private readonly _overflow = new Set<Promise<void>>();
  private readonly _maxPending: number;
  private _shutdown = false;

  constructor(maxPending: number = DEFAULT_MAX_PENDING) {
    this._maxPending = Math.max(0, Math.floor(maxPending));
  }

  subscribe(subscriber: EventSubscriber): void {
    this._subscribers.push(subscriber);
  }

  unsubscribe(subscriber: EventSubscriber): void {
    const idx = this._subscribers.indexOf(subscriber);
    if (idx !== -1) {
      this._subscribers.splice(idx, 1);
    }
  }

  /**
   * Stop accepting new emits. After shutdown, {@link emit} is a no-op (A-D-028,
   * cross-SDK canonical with Python/Rust which drop post-shutdown emits). Drains
   * any in-flight deliveries via {@link flush}.
   */
  async shutdown(timeoutMs: number = 5000): Promise<void> {
    if (this._shutdown) return;
    this._shutdown = true;
    await this.flush(timeoutMs);
  }

  /** @internal Seal emission and give the whole stage one deadline. */
  async shutdownWithin(timeoutMs: number): Promise<'completed' | 'timeout'> {
    this._shutdown = true;
    if (!this.hasPendingDeliveries()) return 'completed';
    if (timeoutMs === 0) return 'timeout';
    const completion = new LifecycleCompletion();
    void Promise.all([...this._pending, ...this._overflow]).then(() => completion.settle());
    await completion.wait(timeoutMs);
    return this.hasPendingDeliveries() ? 'timeout' : 'completed';
  }

  /** @internal Remaining accepted deliveries, including overflow handling. */
  hasPendingDeliveries(): boolean {
    return this._pending.size > 0 || this._overflow.size > 0;
  }

  emit(event: ApCoreEvent): void {
    if (!this.tryEmit(event) && !this._shutdown) {
      console.warn('[apcore:events] Pending deliveries at capacity; event was not accepted:', event.eventType);
    }
  }

  /** Return false if shutdown or saturation prevents accepting the event. */
  tryEmit(event: ApCoreEvent): boolean {
    if (this._shutdown) return false;
    const isDlq = event.eventType === 'apcore.event.delivery_failed';
    const snapshot = this._getMatchingSubscribers(event.eventType, isDlq);
    const available = this._maxPending - this._pending.size;
    const overflow = snapshot.slice(available);
    if (overflow.length > 0 && (this._overflow.size !== 0 || isDlq)) {
      return false;
    }
    // Reserve every slot before invoking subscriber code, which can emit reentrantly.
    const start = snapshot.slice(0, available).map(subscriber =>
      this._reserve(this._pending, () => isDlq
        ? this._deliverDlq(subscriber, event)
        : this._deliver(subscriber, event)));
    if (overflow.length > 0) {
      start.push(this._reserve(this._overflow, async () => {
        for (const subscriber of overflow) await this._emitOverflowDLQ(subscriber, event);
      }));
    }
    for (const deliver of start) deliver();
    return true;
  }

  private _reserve(pending: Set<Promise<void>>, deliver: () => Promise<void>): () => void {
    let settle!: () => void;
    const slot = new Promise<void>(resolve => { settle = resolve; });
    pending.add(slot);
    const release = () => {
      pending.delete(slot);
      settle();
    };
    return () => {
      void lifecycleCallback([this], deliver).then(release, error => {
        release();
        console.error('[apcore:events] Delivery task failed:', error);
      });
    };
  }

  private _getMatchingSubscribers(eventType: string, isDlq = false): EventSubscriber[] {
    return this._subscribers.filter((sub) => {
      // A-D-026: DLQ events (apcore.event.delivery_failed) are delivered ONLY
      // to subscribers with an explicit, non-wildcard pattern. Catch-all
      // subscribers (no pattern, or pattern '*') never receive DLQ events —
      // this prevents cascading failures where a catch-all subscriber would
      // recursively fail on the DLQ event about its own delivery failure.
      const pattern = sub.eventPattern ?? '*';
      if (isDlq && pattern === '*') return false;
      return matchGlob(pattern, eventType);
    });
  }

  private async _deliver(subscriber: EventSubscriber, event: ApCoreEvent): Promise<void> {
    const retry = resolveRetry(subscriber.retry);
    let lastError: Error | undefined;
    let attempts = 0;

    for (let attempt = 0; attempt < retry.maxAttempts; attempt++) {
      try {
        const wrappedDeliver = subscriber[deliverSubscriber];
        if (wrappedDeliver === undefined) {
          await subscriber.onEvent(event);
          return;
        }
        if (await wrappedDeliver.call(subscriber, event)) return;
        // An already-open circuit skips delivery. A circuit that opens during
        // this event still reports any attempt that actually failed.
        break;
      } catch (e) {
        attempts++;
        lastError = e instanceof Error ? e : new Error(String(e));
        if (attempt + 1 < retry.maxAttempts) {
          await sleep(computeDelayMs(retry, attempt));
        }
      }
    }

    // All attempts exhausted — emit DLQ event and invoke onFailure callback
    if (lastError === undefined) return;
    await this._emitDLQ(subscriber, event, lastError, attempts);

    if (subscriber.onFailure) {
      try {
        await subscriber.onFailure(event, lastError, attempts);
      } catch (cbError) {
        console.error('[apcore:events] onFailure callback raised:', cbError);
      }
    }
  }

  /**
   * Route an overflowed delivery through the dead-letter path (A-D-06).
   *
   * When the pending buffer is at capacity, the affected delivery cannot be
   * tracked normally, so instead of dropping it we emit
   * `apcore.event.delivery_failed` with `reason: "pending_overflow"`. This keeps
   * the "accepted events are never silently lost" guarantee while preserving
   * `maxPending` as a memory bound (event-system.md §Overflow, normative).
   */
  private async _emitOverflowDLQ(
    subscriber: EventSubscriber,
    originalEvent: ApCoreEvent,
  ): Promise<void> {
    const error = new Error(
      `pending buffer at capacity (${this._maxPending}) — delivery routed to dead-letter`,
    );
    await this._emitDLQ(subscriber, originalEvent, error, 0, 'pending_overflow');
  }

  private async _emitDLQ(
    subscriber: EventSubscriber,
    originalEvent: ApCoreEvent,
    error: Error,
    attemptCount: number,
    reason?: string,
  ): Promise<void> {
    const subscriberType = resolveSubscriberType(subscriber);
    const subscriberId = subscriber.subscriberId ?? this._identifySubscriber(subscriber);

    const dlqEvent = createEvent(
      'apcore.event.delivery_failed',
      null,
      'error',
      {
        subscriber_type: subscriberType,
        subscriber_id: subscriberId,
        original_event: {
          name: originalEvent.eventType,
          payload: originalEvent.data,
          metadata: {
            module_id: originalEvent.moduleId,
            timestamp: originalEvent.timestamp,
          },
        },
        error: { type: error.constructor?.name ?? 'Error', message: error.message },
        attempt_count: attemptCount,
        timestamp: new Date().toISOString(),
        // A-D-06: present only on pending-buffer overflow ("pending_overflow").
        ...(reason !== undefined ? { reason } : {}),
      },
    );

    // Deliver DLQ event with NO retry — single attempt only; DLQ subscriber errors are logged.
    // isDlq=true excludes catch-all ('*'/no-pattern) subscribers (A-D-026).
    const dlqSubscribers = this._getMatchingSubscribers(dlqEvent.eventType, true);
    for (const dlqSub of dlqSubscribers) {
      await this._deliverDlq(dlqSub, dlqEvent);
    }
  }

  private async _deliverDlq(subscriber: EventSubscriber, event: ApCoreEvent): Promise<void> {
    try {
      const wrappedDeliver = subscriber[deliverSubscriber];
      if (wrappedDeliver === undefined) await subscriber.onEvent(event);
      else await wrappedDeliver.call(subscriber, event);
    } catch (error) {
      console.error('[apcore:events] DLQ subscriber raised:', error);
    }
  }

  private _identifySubscriber(sub: EventSubscriber): string {
    if (sub.subscriberId) return sub.subscriberId;
    const ctorName = (sub as { constructor?: { name?: string } }).constructor?.name;
    if (ctorName && ctorName !== 'Object') return ctorName;
    return `subscriber@${this._subscribers.indexOf(sub)}`;
  }

  /**
   * Wait for all pending async event deliveries to complete.
   * @param timeoutMs - Maximum milliseconds to wait. Defaults to 5000 (5s);
   *                   pass `0` to wait indefinitely.
   *                   Cross-language note: Rust uses ms (matches here);
   *                   Python's flush() uses seconds (5.0 default) — same wall-clock semantics.
   *                   (sync finding A-D-503)
   */
  async flush(timeoutMs: number = 5000): Promise<void> {
    // A-D-027: per-pending semantics (Python canonical) — each pending delivery
    // gets up to `timeoutMs` to settle, rather than sharing one total deadline.
    // A timed-out delivery remains tracked so later calls can continue waiting.
    const pending = [...this._pending, ...this._overflow];
    for (const p of pending) {
      if (timeoutMs > 0) {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const timeout = new Promise<void>((resolve) => {
          timer = setTimeout(resolve, timeoutMs);
        });
        await Promise.race([p.catch(() => {}), timeout]);
        if (timer !== undefined) clearTimeout(timer);
      } else {
        await p.catch(() => {});
      }
    }
  }
}
