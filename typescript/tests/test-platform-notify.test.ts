import { describe, it, expect } from 'vitest';
import { PlatformNotifyMiddleware } from '../src/middleware/platform-notify.js';
import { EventEmitter } from '../src/events/emitter.js';
import type { ApCoreEvent } from '../src/events/emitter.js';
import { MetricsCollector } from '../src/observability/metrics.js';
import { Context } from '../src/context.js';

describe('PlatformNotifyMiddleware', () => {
  it('does not emit when no metrics collector', () => {
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const mw = new PlatformNotifyMiddleware(emitter);
    const ctx = Context.create();

    mw.onError('mod.a', {}, new Error('boom'), ctx);
    expect(events).toHaveLength(0);
  });

  it('after hook does not throw without metrics', () => {
    const emitter = new EventEmitter();
    const mw = new PlatformNotifyMiddleware(emitter);
    const ctx = Context.create();

    const result = mw.after('mod.a', {}, {}, ctx);
    expect(result).toBeNull();
  });

  it('onError returns null', () => {
    const emitter = new EventEmitter();
    const mw = new PlatformNotifyMiddleware(emitter);
    const ctx = Context.create();

    const result = mw.onError('mod.a', {}, new Error('boom'), ctx);
    expect(result).toBeNull();
  });

  it('emits error_threshold_exceeded when error rate crosses threshold', () => {
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const metrics = new MetricsCollector();
    // 2 errors out of 10 calls = 20% error rate, above default 10% threshold
    for (let i = 0; i < 8; i++) {
      metrics.incrementCalls('mod.a', 'success');
    }
    for (let i = 0; i < 2; i++) {
      metrics.incrementCalls('mod.a', 'error');
    }

    const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1);
    const ctx = Context.create();

    mw.onError('mod.a', {}, new Error('boom'), ctx);

    // Issue #36: v0.22.0 removed the legacy alias; only the canonical event remains.
    expect(events).toHaveLength(1);
    const canonical = events.find(e => e.eventType === 'apcore.health.error_threshold_exceeded');
    const legacy = events.find(e => e.eventType === 'error_threshold_exceeded');
    expect(canonical).toBeDefined();
    expect(legacy).toBeUndefined();
    expect(canonical!.moduleId).toBe('mod.a');
    expect(canonical!.data['error_rate']).toBe(0.2);
    expect(canonical!.data['threshold']).toBe(0.1);
  });

  it('does not emit duplicate error_threshold_exceeded alerts (hysteresis)', () => {
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const metrics = new MetricsCollector();
    for (let i = 0; i < 5; i++) metrics.incrementCalls('mod.a', 'success');
    for (let i = 0; i < 5; i++) metrics.incrementCalls('mod.a', 'error');

    const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1);
    const ctx = Context.create();

    mw.onError('mod.a', {}, new Error('boom'), ctx);
    mw.onError('mod.a', {}, new Error('boom'), ctx);

    // Only one alert despite multiple errors
    const errorEvents = events.filter(e => e.eventType === 'apcore.health.error_threshold_exceeded');
    expect(errorEvents).toHaveLength(1);
  });

  it('emits apcore.health.recovered when error rate recovers below threshold * 0.5', () => {
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const metrics = new MetricsCollector();
    // Start with high error rate to trigger alert
    for (let i = 0; i < 5; i++) metrics.incrementCalls('mod.a', 'success');
    for (let i = 0; i < 5; i++) metrics.incrementCalls('mod.a', 'error');

    const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1);
    const ctx = Context.create();

    // Trigger alert
    mw.onError('mod.a', {}, new Error('boom'), ctx);
    expect(events.filter(e => e.eventType === 'apcore.health.error_threshold_exceeded')).toHaveLength(1);

    // Add many successful calls to bring error rate below threshold * 0.5
    for (let i = 0; i < 200; i++) metrics.incrementCalls('mod.a', 'success');

    // Recovery check happens in after()
    mw.after('mod.a', {}, {}, ctx);

    const recoveryEvents = events.filter(e => e.eventType === 'apcore.health.recovered');
    expect(recoveryEvents).toHaveLength(1);
    expect(recoveryEvents[0].data['status']).toBe('recovered');
  });

  it('emits latency_threshold_exceeded when p99 exceeds threshold', () => {
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const metrics = new MetricsCollector();
    // Record many slow durations (6 seconds each, above 5s default threshold)
    for (let i = 0; i < 10; i++) {
      metrics.observeDuration('mod.a', 6.0);
    }

    const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1, 5000);
    const ctx = Context.create();

    mw.after('mod.a', {}, {}, ctx);

    const latencyEvents = events.filter(e => e.eventType === 'apcore.health.latency_threshold_exceeded');
    expect(latencyEvents).toHaveLength(1);
    expect(latencyEvents[0].moduleId).toBe('mod.a');
    expect(latencyEvents[0].data['threshold']).toBe(5000);
  });

  it('does not emit latency alert when latency is below threshold', () => {
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const metrics = new MetricsCollector();
    // Record fast durations (0.01s = 10ms, well below 5000ms threshold)
    for (let i = 0; i < 10; i++) {
      metrics.observeDuration('mod.a', 0.01);
    }

    const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1, 5000);
    const ctx = Context.create();

    mw.after('mod.a', {}, {}, ctx);

    const latencyEvents = events.filter(e => e.eventType === 'apcore.health.latency_threshold_exceeded');
    expect(latencyEvents).toHaveLength(0);
  });

  it('does not emit duplicate latency_threshold_exceeded alerts (hysteresis)', () => {
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const metrics = new MetricsCollector();
    for (let i = 0; i < 10; i++) {
      metrics.observeDuration('mod.a', 6.0);
    }

    const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1, 5000);
    const ctx = Context.create();

    mw.after('mod.a', {}, {}, ctx);
    mw.after('mod.a', {}, {}, ctx);
    mw.after('mod.a', {}, {}, ctx);

    const latencyEvents = events.filter(e => e.eventType === 'apcore.health.latency_threshold_exceeded');
    expect(latencyEvents).toHaveLength(1);
  });

  it('returns last bucket upper bound when all observations exceed largest bucket', () => {
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    const metrics = new MetricsCollector();
    // Record durations exceeding the largest bucket (60s)
    for (let i = 0; i < 10; i++) {
      metrics.observeDuration('mod.a', 120.0);
    }

    const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1, 5000);
    const ctx = Context.create();

    mw.after('mod.a', {}, {}, ctx);

    const latencyEvents = events.filter(e => e.eventType === 'apcore.health.latency_threshold_exceeded');
    expect(latencyEvents).toHaveLength(1);
    // p99 should be 60000ms (last bucket * 1000), not 0
    expect(latencyEvents[0].data['p99_latency_ms']).toBe(60000);
  });

  it('before hook returns null', () => {
    const emitter = new EventEmitter();
    const mw = new PlatformNotifyMiddleware(emitter);
    const ctx = Context.create();

    const result = mw.before('mod.a', {}, ctx);
    expect(result).toBeNull();
  });
});
