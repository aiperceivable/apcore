/**
 * PlatformNotifyMiddleware -- threshold sensor with hysteresis.
 *
 * Monitors error rates and latency, emits threshold events with hysteresis.
 * Emits apcore.health.error_threshold_exceeded when a module's error rate
 * crosses the configured threshold, apcore.health.latency_threshold_exceeded
 * when p99 latency exceeds the limit, and apcore.health.recovered when a
 * previously alerted module recovers below threshold * 0.5.
 *
 * Issue #36 removed legacy aliases in v0.22.0 — only the canonical
 * `apcore.health.*` event names are emitted.
 */

import type { Context } from '../context.js';
import type { EventEmitter } from '../events/emitter.js';
import { createEvent } from '../events/emitter.js';
import type { MetricsCollector } from '../observability/metrics.js';
import { computeModuleErrorRate, estimateP99FromHistogram } from '../observability/metrics-utils.js';
import { Middleware } from './base.js';

export class PlatformNotifyMiddleware extends Middleware {
  private readonly _emitter: EventEmitter;
  private readonly _metricsCollector: MetricsCollector | null;
  private readonly _errorRateThreshold: number;
  private readonly _latencyP99ThresholdMs: number;
  private readonly _alerted: Map<string, Set<string>> = new Map();

  constructor(
    eventEmitter: EventEmitter,
    metricsCollector: MetricsCollector | null = null,
    errorRateThreshold: number = 0.1,
    latencyP99ThresholdMs: number = 5000,
  ) {
    super();
    this._emitter = eventEmitter;
    this._metricsCollector = metricsCollector;
    this._errorRateThreshold = errorRateThreshold;
    this._latencyP99ThresholdMs = latencyP99ThresholdMs;
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    _context: Context,
  ): Record<string, unknown> | null {
    this._checkLatencyThreshold(moduleId);
    this._checkErrorRecovery(moduleId);
    return null;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _error: Error,
    _context: Context,
  ): Record<string, unknown> | null {
    this._checkErrorRateThreshold(moduleId);
    // Also check recovery on error paths so a module whose error rate is
    // falling (e.g., because the histogram window is decaying) can clear
    // the alert state even when no successful call arrives.
    this._checkErrorRecovery(moduleId);
    return null;
  }

  private _getAlerted(moduleId: string): Set<string> {
    let set = this._alerted.get(moduleId);
    if (!set) {
      set = new Set();
      this._alerted.set(moduleId, set);
    }
    return set;
  }

  private _computeErrorRate(moduleId: string): number {
    if (!this._metricsCollector) return 0;
    return computeModuleErrorRate(this._metricsCollector, moduleId).errorRate;
  }

  private _checkErrorRateThreshold(moduleId: string): void {
    const errorRate = this._computeErrorRate(moduleId);
    const alerted = this._getAlerted(moduleId);
    if (errorRate >= this._errorRateThreshold && !alerted.has('error_rate')) {
      this._emitter.emit(createEvent(
        'apcore.health.error_threshold_exceeded',
        moduleId,
        'error',
        { error_rate: errorRate, threshold: this._errorRateThreshold },
      ));
      alerted.add('error_rate');
    }
  }

  private _checkLatencyThreshold(moduleId: string): void {
    if (!this._metricsCollector) return;
    const alerted = this._getAlerted(moduleId);
    if (alerted.has('latency')) return;

    const p99Ms = this._estimateP99Ms(moduleId);
    if (p99Ms >= this._latencyP99ThresholdMs) {
      this._emitter.emit(createEvent(
        'apcore.health.latency_threshold_exceeded',
        moduleId,
        'warn',
        { p99_latency_ms: p99Ms, threshold: this._latencyP99ThresholdMs },
      ));
      alerted.add('latency');
    }
  }

  private _estimateP99Ms(moduleId: string): number {
    if (!this._metricsCollector) return 0;
    return estimateP99FromHistogram(this._metricsCollector, moduleId).p99LatencyMs;
  }

  private _checkErrorRecovery(moduleId: string): void {
    const alerted = this._alerted.get(moduleId);
    if (!alerted || !alerted.has('error_rate')) return;

    const errorRate = this._computeErrorRate(moduleId);
    if (errorRate < this._errorRateThreshold * 0.5) {
      this._emitter.emit(createEvent(
        'apcore.health.recovered',
        moduleId,
        'info',
        { status: 'recovered', error_rate: errorRate },
      ));
      alerted.delete('error_rate');
    }
  }
}
