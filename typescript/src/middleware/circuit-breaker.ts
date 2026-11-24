/**
 * CircuitBreakerMiddleware — per-(module_id, caller_id) rolling-window circuit breaker (Issue #42).
 *
 * State machine:
 *   CLOSED  → (error_rate >= open_threshold, samples >= min_samples) → OPEN
 *   OPEN    → (recovery_window_ms elapsed)                → HALF_OPEN
 *   HALF_OPEN → (probe success)                           → CLOSED  (emits apcore.circuit.closed)
 *   HALF_OPEN → (probe failure)                           → OPEN    (emits apcore.circuit.opened)
 */

import type { Context } from '../context.js';
import { CircuitBreakerOpenError } from '../errors.js';
import type { ApCoreEvent } from '../events/emitter.js';
import { createEvent } from '../events/emitter.js';
import { Middleware } from './base.js';

export const CTX_CIRCUIT_STATE = '_apcore.mw.circuit.state';

export enum CircuitBreakerState {
  CLOSED = 'CLOSED',
  OPEN = 'OPEN',
  HALF_OPEN = 'HALF_OPEN',
}

class RollingWindow {
  private readonly _buf: boolean[];
  private _head = 0;
  private _len = 0;
  private _errors = 0;

  constructor(private readonly _cap: number) {
    this._buf = new Array(_cap).fill(false);
  }

  record(isError: boolean): void {
    if (this._len < this._cap) {
      this._buf[this._len++] = isError;
    } else {
      const evicted = this._buf[this._head];
      this._buf[this._head] = isError;
      this._head = (this._head + 1) % this._cap;
      if (evicted) this._errors--;
    }
    if (isError) this._errors++;
  }

  get errorRate(): number {
    return this._len === 0 ? 0 : this._errors / this._len;
  }

  get length(): number {
    return this._len;
  }
}

interface CircuitRecord {
  state: CircuitBreakerState;
  window: RollingWindow;
  openedAt: number | null;
  probeInFlight: boolean;
}

export interface CircuitBreakerOptions {
  /** Error rate in [0,1] at which the circuit opens. Default: 0.5 */
  openThreshold?: number;
  /** Milliseconds before OPEN transitions to HALF_OPEN. Default: 30000 */
  recoveryWindowMs?: number;
  /** Number of recent calls tracked in the rolling window. Default: 20 */
  windowSize?: number;
  /**
   * Minimum number of observations in the window before the circuit may open.
   * Decoupled from windowSize for parity with Python/Rust. Default: 5
   */
  minSamples?: number;
  /** Optional EventEmitter to receive circuit state-change events. */
  emitter?: { emit(event: ApCoreEvent): void };
  /** Middleware priority (0–1000). Default: 100 */
  priority?: number;
}

export class CircuitBreakerMiddleware extends Middleware {
  private readonly _openThreshold: number;
  private readonly _recoveryWindowMs: number;
  private readonly _windowSize: number;
  private readonly _minSamples: number;
  private readonly _emitter: { emit(event: ApCoreEvent): void } | null;
  private readonly _circuits = new Map<string, CircuitRecord>();

  constructor(options: CircuitBreakerOptions = {}) {
    super(options.priority ?? 100);
    const openThreshold = options.openThreshold ?? 0.5;
    const recoveryWindowMs = options.recoveryWindowMs ?? 30000;
    const windowSize = options.windowSize ?? 20;
    let minSamples = options.minSamples ?? 5;

    // Constructor validation + clamp — mirrors Python circuit_breaker.py:90-105.
    if (!(openThreshold >= 0.0 && openThreshold <= 1.0)) {
      throw new Error(`open_threshold must be in [0.0, 1.0], got ${openThreshold}`);
    }
    if (recoveryWindowMs < 0) {
      throw new Error(`recovery_window_ms must be >= 0, got ${recoveryWindowMs}`);
    }
    if (windowSize < 1) {
      throw new Error(`window_size must be >= 1, got ${windowSize}`);
    }
    if (minSamples < 1) {
      throw new Error(`min_samples must be >= 1, got ${minSamples}`);
    }
    if (minSamples > windowSize) {
      console.warn(
        `[apcore:middleware:circuit_breaker] min_samples (${minSamples}) exceeds ` +
          `window_size (${windowSize}); clamping min_samples to window_size. ` +
          'Otherwise the breaker could never open.',
      );
      minSamples = windowSize;
    }

    this._openThreshold = openThreshold;
    this._recoveryWindowMs = recoveryWindowMs;
    this._windowSize = windowSize;
    this._minSamples = minSamples;
    this._emitter = options.emitter ?? null;
  }

  private _key(moduleId: string, callerId: string | null): string {
    return `${moduleId}:${callerId ?? ''}`;
  }

  private _getRecord(moduleId: string, callerId: string | null): CircuitRecord {
    const key = this._key(moduleId, callerId);
    if (!this._circuits.has(key)) {
      this._circuits.set(key, {
        state: CircuitBreakerState.CLOSED,
        window: new RollingWindow(this._windowSize),
        openedAt: null,
        probeInFlight: false,
      });
    }
    return this._circuits.get(key)!;
  }

  private _maybeHalfOpen(record: CircuitRecord): void {
    if (record.state === CircuitBreakerState.OPEN && record.openedAt !== null) {
      if (Date.now() - record.openedAt >= this._recoveryWindowMs) {
        record.state = CircuitBreakerState.HALF_OPEN;
        record.probeInFlight = false;
      }
    }
  }

  private _openCircuit(moduleId: string, callerId: string | null, record: CircuitRecord): void {
    record.state = CircuitBreakerState.OPEN;
    record.openedAt = Date.now();
    record.probeInFlight = false;
    console.warn(
      `[apcore:middleware] Circuit OPEN for module '${moduleId}' (caller: ${callerId ?? 'unknown'})`,
    );
    this._emit('apcore.circuit.opened', moduleId, callerId, 'warn');
  }

  private _closeCircuit(moduleId: string, callerId: string | null, record: CircuitRecord): void {
    record.state = CircuitBreakerState.CLOSED;
    record.openedAt = null;
    record.probeInFlight = false;
    console.warn(
      `[apcore:middleware] Circuit CLOSED for module '${moduleId}' (caller: ${callerId ?? 'unknown'})`,
    );
    this._emit('apcore.circuit.closed', moduleId, callerId, 'info');
  }

  private _emit(
    eventType: string,
    moduleId: string,
    callerId: string | null,
    severity: string,
  ): void {
    if (!this._emitter) return;
    try {
      this._emitter.emit(createEvent(eventType, moduleId, severity, { callerId: callerId ?? null }));
    } catch (err) {
      console.warn(`[apcore:middleware] Circuit event emission failed for '${eventType}':`, err);
    }
  }

  override before(
    moduleId: string,
    _inputs: Record<string, unknown>,
    context: Context,
  ): Record<string, unknown> | null {
    const callerId = context.callerId;
    const record = this._getRecord(moduleId, callerId);

    this._maybeHalfOpen(record);
    context.data[CTX_CIRCUIT_STATE] = record.state;

    if (record.state === CircuitBreakerState.OPEN) {
      throw new CircuitBreakerOpenError(moduleId, callerId);
    }

    if (record.state === CircuitBreakerState.HALF_OPEN) {
      if (record.probeInFlight) {
        throw new CircuitBreakerOpenError(moduleId, callerId);
      }
      record.probeInFlight = true;
    }

    return null;
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    context: Context,
  ): Record<string, unknown> | null {
    const callerId = context.callerId;
    const record = this._getRecord(moduleId, callerId);

    record.window.record(false);

    if (record.state === CircuitBreakerState.HALF_OPEN) {
      this._closeCircuit(moduleId, callerId, record);
      context.data[CTX_CIRCUIT_STATE] = CircuitBreakerState.CLOSED;
    }

    return null;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    error: Error,
    context: Context,
  ): Record<string, unknown> | null {
    if (error instanceof CircuitBreakerOpenError) {
      return null;
    }

    const callerId = context.callerId;
    const record = this._getRecord(moduleId, callerId);

    record.window.record(true);

    if (record.state === CircuitBreakerState.HALF_OPEN) {
      this._openCircuit(moduleId, callerId, record);
      context.data[CTX_CIRCUIT_STATE] = CircuitBreakerState.OPEN;
    } else if (record.state === CircuitBreakerState.CLOSED) {
      if (
        record.window.length >= this._minSamples &&
        record.window.errorRate >= this._openThreshold
      ) {
        this._openCircuit(moduleId, callerId, record);
        context.data[CTX_CIRCUIT_STATE] = CircuitBreakerState.OPEN;
      }
    }

    return null;
  }

  /** Return the current circuit state for a given (moduleId, callerId) pair. */
  getState(moduleId: string, callerId: string | null = null): CircuitBreakerState {
    const record = this._getRecord(moduleId, callerId);
    this._maybeHalfOpen(record);
    return record.state;
  }

  /** Manually reset the circuit to CLOSED (e.g., for operator override or tests). */
  reset(moduleId: string, callerId: string | null = null): void {
    this._circuits.delete(this._key(moduleId, callerId));
  }
}
