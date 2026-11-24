import type { EventSubscriber, ApCoreEvent } from './emitter.js';
import { resolveSubscriberType } from './emitter.js';

export enum CircuitState {
  CLOSED = 'CLOSED',
  OPEN = 'OPEN',
  HALF_OPEN = 'HALF_OPEN',
}

export interface CircuitBreakerConfig {
  timeoutMs?: number;
  openThreshold?: number;
  recoveryWindowMs?: number;
  /** Explicit type name for circuit events. Defaults to constructor.name, which is unreliable in minified builds. */
  subscriberType?: string;
}

/**
 * Wraps an EventSubscriber with a per-subscriber circuit-breaker.
 *
 * State machine:
 *   CLOSED → (consecutive_failures >= openThreshold) → OPEN
 *   OPEN → (recoveryWindowMs elapsed) → HALF_OPEN
 *   HALF_OPEN → success → CLOSED
 *   HALF_OPEN → failure → OPEN
 */
export class CircuitBreakerWrapper implements EventSubscriber {
  private _state: CircuitState = CircuitState.CLOSED;
  private _consecutiveFailures: number = 0;
  private _lastFailureAt: Date | null = null;
  private readonly _timeoutMs: number;
  private readonly _openThreshold: number;
  private readonly _recoveryWindowMs: number;
  private readonly _subscriberType: string;

  constructor(
    private readonly _subscriber: EventSubscriber,
    private readonly _emitter: { emit(event: ApCoreEvent): void },
    config: CircuitBreakerConfig = {},
  ) {
    this._timeoutMs = config.timeoutMs ?? 5000;
    this._openThreshold = config.openThreshold ?? 5;
    this._recoveryWindowMs = config.recoveryWindowMs ?? 60000;
    // D-116: the DECLARED kind, resolved exactly as the DLQ path resolves it.
    // This used to be the raw constructor name — neither the declared kind nor
    // the value this same SDK puts in its own `apcore.event.delivery_failed`
    // payload, so a consumer routing on `subscriber_type` saw two answers for
    // one subscriber. An explicit `config.subscriberType` still wins, because
    // that is a caller override rather than a second default.
    this._subscriberType = config.subscriberType ?? resolveSubscriberType(this._subscriber);
  }

  /**
   * EVT-001: the wrapper stands in for the wrapped subscriber in the emitter's
   * subscriber list, so it must carry that subscriber's identity.
   *
   * `EventEmitter._getMatchingSubscribers` falls back to `'*'` when
   * `eventPattern` is absent, so a wrapper that declared none silently widened
   * a FILTERED subscriber to catch-all — an event the operator had excluded
   * was POSTed to the webhook anyway. `subscriberId` and `subscriberType` are
   * forwarded for the same reason: they are the identity fields of the DLQ
   * payload, and the wrapper is not the subscriber that failed. apcore-rust
   * forwards all three.
   *
   * `undefined` is forwarded as `undefined`, so an unfiltered subscriber
   * still wraps as catch-all and an anonymous one still falls back to the
   * emitter's derivation.
   */
  get eventPattern(): string | undefined {
    return this._subscriber.eventPattern;
  }

  get subscriberId(): string | undefined {
    return this._subscriber.subscriberId;
  }

  get subscriberType(): string | undefined {
    return this._subscriber.subscriberType ?? this._subscriberType;
  }

  get state(): CircuitState {
    return this._state;
  }

  get consecutiveFailures(): number {
    return this._consecutiveFailures;
  }

  /** Transition OPEN → HALF_OPEN if recoveryWindowMs has elapsed since last failure. */
  checkRecovery(): void {
    if (this._state !== CircuitState.OPEN || this._lastFailureAt === null) return;
    if (Date.now() - this._lastFailureAt.getTime() >= this._recoveryWindowMs) {
      this._state = CircuitState.HALF_OPEN;
    }
  }

  async onEvent(event: ApCoreEvent): Promise<void> {
    this.checkRecovery();
    if (this._state === CircuitState.OPEN) return;

    let circuitEvent: ApCoreEvent | null = null;
    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        Promise.resolve(this._subscriber.onEvent(event)),
        new Promise<never>((_, reject) => {
          timeoutId = setTimeout(
            () => reject(new Error(`Delivery timeout after ${this._timeoutMs}ms`)),
            this._timeoutMs,
          );
        }),
      ]);
      circuitEvent = this._onSuccess();
    } catch (err: unknown) {
      circuitEvent = this._onFailure(err);
    } finally {
      clearTimeout(timeoutId);
    }

    if (circuitEvent !== null) {
      try {
        this._emitter.emit(circuitEvent);
      } catch {
        // circuit events must not surface to callers
      }
    }
  }

  private _onSuccess(): ApCoreEvent | null {
    if (this._state === CircuitState.HALF_OPEN) {
      this._state = CircuitState.CLOSED;
      this._consecutiveFailures = 0;
      // subscriber_id names WHICH breaker changed state: two subscribers of
      // one type are otherwise indistinguishable (protocol-spec §9.16.2).
      return this._makeEvent('apcore.subscriber.circuit_closed', 'info', {
        subscriber_id: this.subscriberId ?? null,
        subscriber_type: this._subscriberType,
        recovery_attempt: true,
      });
    }
    this._consecutiveFailures = 0;
    return null;
  }

  private _onFailure(error: unknown): ApCoreEvent | null {
    this._consecutiveFailures += 1;
    this._lastFailureAt = new Date();

    const opens =
      this._state === CircuitState.HALF_OPEN ||
      (this._state === CircuitState.CLOSED && this._consecutiveFailures >= this._openThreshold);

    if (opens) {
      this._state = CircuitState.OPEN;
      console.warn(
        `[apcore:events] Circuit opened for subscriber ${this._subscriberType} after ${this._consecutiveFailures} consecutive failures:`,
        error,
      );
      return this._makeEvent('apcore.subscriber.circuit_opened', 'warn', {
        subscriber_id: this.subscriberId ?? null,
        subscriber_type: this._subscriberType,
        consecutive_failures: this._consecutiveFailures,
      });
    }
    return null;
  }

  private _makeEvent(
    eventType: string,
    severity: string,
    data: Record<string, unknown>,
  ): ApCoreEvent {
    return {
      eventType,
      moduleId: null,
      timestamp: new Date().toISOString(),
      severity,
      data,
    };
  }
}
