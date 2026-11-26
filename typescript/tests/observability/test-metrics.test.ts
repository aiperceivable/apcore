import { describe, it, expect, vi } from 'vitest';
import { MetricsCollector, MetricsMiddleware } from '../../src/observability/metrics.js';
import { InMemoryStorageBackend } from '../../src/observability/storage.js';
import { InMemoryObservabilityStore } from '../../src/observability/store.js';
import { Context } from '../../src/context.js';
import { ModuleError } from '../../src/errors.js';

describe('MetricsCollector', () => {
  it('increments counters', () => {
    const collector = new MetricsCollector();
    collector.increment('test_counter', { method: 'GET' });
    collector.increment('test_counter', { method: 'GET' });
    const snap = collector.snapshot();
    expect((snap['counters'] as Record<string, number>)['test_counter|method=GET']).toBe(2);
  });

  it('observes histogram values', () => {
    const collector = new MetricsCollector();
    collector.observe('test_hist', { module: 'a' }, 0.05);
    collector.observe('test_hist', { module: 'a' }, 0.5);
    const snap = collector.snapshot();
    const hists = snap['histograms'] as Record<string, Record<string, number>>;
    expect(hists['sums']['test_hist|module=a']).toBeCloseTo(0.55);
    expect(hists['counts']['test_hist|module=a']).toBe(2);
  });

  it('sends counter deltas and observations to the explicitly supplied store', () => {
    const store = new InMemoryObservabilityStore();
    const collector = new MetricsCollector({ store });
    const labels = { module_id: 'demo.one', status: 'success' };
    collector.increment('calls', labels, 2);
    collector.increment('calls', labels);
    collector.observe('duration', labels, 0.25);
    labels.status = 'changed';

    const points = store.getMetrics('demo.one');
    expect(points.map(({ name, value }) => [name, value])).toEqual([
      ['calls', 2],
      ['calls', 1],
      ['duration', 0.25],
    ]);
    for (const point of points) {
      expect(point.labels).toEqual({ module_id: 'demo.one', status: 'success' });
      expect(new Date(point.timestamp).toISOString()).toBe(point.timestamp);
    }
  });

  it('does not append history without an explicit sink', () => {
    const collector = new MetricsCollector();
    collector.increment('calls', {});
    collector.observe('duration', {}, 0.25);
    expect(collector.store.getMetrics()).toEqual([]);
  });

  it.each(['throw', 'reject'])('preserves aggregates when the observability store writes %s', async (failure) => {
    const store = new InMemoryObservabilityStore();
    const error = new Error('Sink unavailable');
    vi.spyOn(store, 'recordMetric').mockImplementation(() => {
      if (failure === 'throw') throw error;
      return Promise.reject(error);
    });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const collector = new MetricsCollector({ store });
      collector.increment('calls', {});
      collector.observe('duration', {}, 0.25);
      await Promise.resolve();
      expect(collector.snapshot()).toMatchObject({
        counters: { 'calls|': 1 },
        histograms: { counts: { 'duration|': 1 } },
      });
      expect(warn).toHaveBeenCalledTimes(2);
      expect(warn).toHaveBeenCalledWith('[apcore:metrics] Observability store write failed', error);
    } finally {
      warn.mockRestore();
    }
  });

  it.each(['throw', 'reject'])('keeps histogram samples when metric storage writes %s', async (failure) => {
    const storage = new InMemoryStorageBackend();
    const error = new Error('Storage unavailable');
    vi.spyOn(storage, 'save').mockImplementation(() => {
      if (failure === 'throw') throw error;
      return Promise.reject(error);
    });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const collector = new MetricsCollector({ storage });
      expect(() => collector.observe('test_hist', { module: 'a' }, 0.5)).not.toThrow();
      await Promise.resolve();
      expect(collector.snapshot()).toMatchObject({ histograms: { counts: { 'test_hist|module=a': 1 } } });
      expect(warn).toHaveBeenCalledWith('[apcore:metrics] Storage write failed', error);
    } finally {
      warn.mockRestore();
    }
  });

  it('convenience methods work', () => {
    const collector = new MetricsCollector();
    collector.incrementCalls('mod.a', 'success');
    collector.incrementErrors('mod.a', 'TIMEOUT');
    collector.observeDuration('mod.a', 0.1);

    const snap = collector.snapshot();
    const counters = snap['counters'] as Record<string, number>;
    expect(counters['apcore_module_calls_total|module_id=mod.a,status=success']).toBe(1);
    expect(counters['apcore_module_errors_total|error_code=TIMEOUT,module_id=mod.a']).toBe(1);
  });

  it('reset clears all data', () => {
    const collector = new MetricsCollector();
    collector.incrementCalls('mod.a', 'success');
    collector.observeDuration('mod.a', 0.1);
    collector.reset();
    const snap = collector.snapshot();
    expect(Object.keys(snap['counters'] as Record<string, unknown>)).toHaveLength(0);
  });

  it('snapshot returns counters and histogram sub-keys', () => {
    const collector = new MetricsCollector();
    collector.increment('req_total', { status: '200' });
    collector.observe('req_duration', { route: '/health' }, 0.01);
    const snap = collector.snapshot();
    expect(snap).toHaveProperty('counters');
    expect(snap).toHaveProperty('histograms');
    const hists = snap['histograms'] as Record<string, unknown>;
    expect(hists).toHaveProperty('sums');
    expect(hists).toHaveProperty('counts');
    expect(hists).toHaveProperty('buckets');
  });

  it('exportPrometheus produces valid format', () => {
    const collector = new MetricsCollector();
    collector.incrementCalls('mod.a', 'success');
    collector.observeDuration('mod.a', 0.05);
    const output = collector.exportPrometheus();
    expect(output).toContain('# HELP');
    expect(output).toContain('# TYPE');
    expect(output).toContain('apcore_module_calls_total');
    expect(output).toContain('apcore_module_duration_seconds');
  });

  it('empty collector returns empty prometheus string', () => {
    const collector = new MetricsCollector();
    expect(collector.exportPrometheus()).toBe('');
  });

  it('accepts custom buckets and uses them for observations', () => {
    const collector = new MetricsCollector([0.1, 0.5, 1.0]);
    collector.observe('custom_hist', { op: 'read' }, 0.3);
    const snap = collector.snapshot();
    const hists = snap['histograms'] as Record<string, Record<string, number>>;
    // value 0.3 falls in the 0.5 bucket but not 0.1
    expect(hists['buckets']['custom_hist|op=read|0.1']).toBeUndefined();
    expect(hists['buckets']['custom_hist|op=read|0.5']).toBe(1);
    expect(hists['buckets']['custom_hist|op=read|Inf']).toBe(1);
  });

  it('exportPrometheus omits label braces when metric has no labels', () => {
    const collector = new MetricsCollector();
    // increment with empty labels so parseLabels receives '' and formatLabels receives {}
    collector.increment('no_label_counter', {});
    const output = collector.exportPrometheus();
    expect(output).toContain('no_label_counter 1');
    expect(output).not.toContain('no_label_counter{');
  });
});

describe('MetricsMiddleware', () => {
  it('records call metrics on success', () => {
    const collector = new MetricsCollector();
    const mw = new MetricsMiddleware(collector);
    const ctx = Context.create();

    mw.before('mod.a', {}, ctx);
    mw.after('mod.a', {}, { result: 'ok' }, ctx);

    const snap = collector.snapshot();
    const counters = snap['counters'] as Record<string, number>;
    expect(counters['apcore_module_calls_total|module_id=mod.a,status=success']).toBe(1);
  });

  it('records error metrics on failure with plain Error', () => {
    const collector = new MetricsCollector();
    const mw = new MetricsMiddleware(collector);
    const ctx = Context.create();

    mw.before('mod.a', {}, ctx);
    mw.onError('mod.a', {}, new Error('boom'), ctx);

    const snap = collector.snapshot();
    const counters = snap['counters'] as Record<string, number>;
    expect(counters['apcore_module_calls_total|module_id=mod.a,status=error']).toBe(1);
    expect(counters['apcore_module_errors_total|error_code=Error,module_id=mod.a']).toBe(1);
  });

  it('records error code from ModuleError.code instead of constructor name', () => {
    const collector = new MetricsCollector();
    const mw = new MetricsMiddleware(collector);
    const ctx = Context.create();

    mw.before('mod.b', {}, ctx);
    mw.onError('mod.b', {}, new ModuleError('CUSTOM_CODE', 'something went wrong'), ctx);

    const snap = collector.snapshot();
    const counters = snap['counters'] as Record<string, number>;
    expect(counters['apcore_module_errors_total|error_code=CUSTOM_CODE,module_id=mod.b']).toBe(1);
  });

  it('after() returns null without recording metrics when starts is undefined', () => {
    const collector = new MetricsCollector();
    const mw = new MetricsMiddleware(collector);
    const ctx = new Context('trace-id', null, [], null, null);

    const result = mw.after('mod.a', {}, { result: 'ok' }, ctx);

    expect(result).toBeNull();
    const snap = collector.snapshot();
    expect(Object.keys(snap['counters'] as Record<string, unknown>)).toHaveLength(0);
  });

  it('after() returns null without recording metrics when starts array is empty', () => {
    const collector = new MetricsCollector();
    const mw = new MetricsMiddleware(collector);
    const ctx = new Context('trace-id', null, [], null, null);
    ctx.data['_apcore.mw.metrics.starts'] = [];

    const result = mw.after('mod.a', {}, { result: 'ok' }, ctx);

    expect(result).toBeNull();
    const snap = collector.snapshot();
    expect(Object.keys(snap['counters'] as Record<string, unknown>)).toHaveLength(0);
  });

  it('onError() returns null without recording metrics when starts is undefined', () => {
    const collector = new MetricsCollector();
    const mw = new MetricsMiddleware(collector);
    const ctx = new Context('trace-id', null, [], null, null);

    const result = mw.onError('mod.a', {}, new Error('boom'), ctx);

    expect(result).toBeNull();
    const snap = collector.snapshot();
    expect(Object.keys(snap['counters'] as Record<string, unknown>)).toHaveLength(0);
  });

  it('onError() returns null without recording metrics when starts array is empty', () => {
    const collector = new MetricsCollector();
    const mw = new MetricsMiddleware(collector);
    const ctx = new Context('trace-id', null, [], null, null);
    ctx.data['_apcore.mw.metrics.starts'] = [];

    const result = mw.onError('mod.a', {}, new Error('boom'), ctx);

    expect(result).toBeNull();
    const snap = collector.snapshot();
    expect(Object.keys(snap['counters'] as Record<string, unknown>)).toHaveLength(0);
  });
});
