/**
 * Global event bus with fan-out delivery, per-subscriber retry, and DLQ.
 */

import { resolveRetry, computeDelayMs } from './retry.js';
import type { RetryConfig } from './retry.js';
import { matchGlob } from '../utils/pattern.js';

export interface ApCoreEvent {
  readonly eventType: string;
  readonly moduleId: string | null;
  readonly timestamp: string;
  readonly severity: string;
  readonly data: Record<string, unknown>;
}

export interface EventSubscriber {
  onEvent(event: ApCoreEvent): void | Promise<void>;
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
  private _pending: Promise<void>[] = [];
  private readonly _maxPending: number;
  private _shutdown = false;

  constructor(maxPending: number = DEFAULT_MAX_PENDING) {
    this._maxPending = maxPending;
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

  emit(event: ApCoreEvent): void {
    // A-D-028: silently drop emits after shutdown.
    if (this._shutdown) return;
    const snapshot = this._getMatchingSubscribers(event.eventType);
    for (const subscriber of snapshot) {
      // Every subscriber — built-in and user-registered, with or without an explicit
      // `retry` block — goes through the unified retry/backoff/DLQ delivery path.
      // A subscriber that omits `retry` receives the spec default policy
      // (max_attempts=3); one that explicitly sets maxAttempts:1 disables retry.
      // (event-system.md §Per-Subscriber Retry Policy; sync finding A-D-005)
      if (this._pending.length >= this._maxPending) {
        // A-D-06: the pending buffer is full. The event was accepted by emit(),
        // so it MUST NOT be silently discarded (event-system.md §Overflow). Route
        // the affected delivery through the dead-letter path with
        // reason: "pending_overflow" instead of a log-only drop. `maxPending`
        // remains a legitimate memory bound — we just fail through the DLQ.
        const overflowPromise = this._emitOverflowDLQ(subscriber, event);
        const tracked = overflowPromise.catch(() => {
          // DLQ subscriber errors handled inside _emitOverflowDLQ
        });
        this._pending.push(tracked);
        tracked.then(() => {
          const idx = this._pending.indexOf(tracked);
          if (idx !== -1) this._pending.splice(idx, 1);
        });
      } else {
        const tracked = this._deliver(subscriber, event).catch(() => {
          // errors handled inside _deliver
        });
        this._pending.push(tracked);
        tracked.then(() => {
          const idx = this._pending.indexOf(tracked);
          if (idx !== -1) this._pending.splice(idx, 1);
        });
      }
    }
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
    let lastError: Error = new Error('Unknown error');

    for (let attempt = 0; attempt < retry.maxAttempts; attempt++) {
      try {
        await subscriber.onEvent(event);
        return;
      } catch (e) {
        lastError = e instanceof Error ? e : new Error(String(e));
        if (attempt + 1 < retry.maxAttempts) {
          await sleep(computeDelayMs(retry, attempt));
        }
      }
    }

    // All attempts exhausted — emit DLQ event and invoke onFailure callback
    await this._emitDLQ(subscriber, event, lastError, retry.maxAttempts);

    if (subscriber.onFailure) {
      try {
        await subscriber.onFailure(event, lastError, retry.maxAttempts);
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
      try {
        const r = dlqSub.onEvent(dlqEvent);
        if (r instanceof Promise) {
          await r;
        }
      } catch (dlqErr) {
        console.error('[apcore:events] DLQ subscriber raised:', dlqErr);
      }
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
    // A delivery that exceeds its budget is abandoned (the next call to flush
    // can re-await any that are still tracked).
    const pending = [...this._pending];
    this._pending = [];
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
