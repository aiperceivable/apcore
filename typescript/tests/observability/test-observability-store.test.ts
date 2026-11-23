/**
 * ObservabilityStore read path: `getErrors` / `getMetrics`.
 *
 * Both are part of the public `ObservabilityStore` interface, and both had
 * zero callers in `src/` and zero tests — only the write path and `clear()`
 * were exercised, which left every filter branch in the default in-memory
 * implementation unverified.
 */

import { describe, it, expect } from 'vitest';
import {
  InMemoryObservabilityStore,
  type MetricPoint,
  type ObservabilityStore,
} from '../../src/observability/store.js';

function metric(
  name: string,
  value: number,
  moduleId: string | null,
  labels: Record<string, string> = {},
): MetricPoint {
  return { name, value, moduleId, labels, timestamp: '2026-09-14T00:00:00Z' };
}

describe('InMemoryObservabilityStore.getErrors', () => {
  it('returns every recorded entry, in insertion order, when no filter is given', () => {
    const store: ObservabilityStore = new InMemoryObservabilityStore();
    store.recordError({ moduleId: 'a.one', code: 'E1' });
    store.recordError({ moduleId: 'b.two', code: 'E2' });

    expect(store.getErrors()).toEqual([
      { moduleId: 'a.one', code: 'E1' },
      { moduleId: 'b.two', code: 'E2' },
    ]);
  });

  it('filters by moduleId', () => {
    const store = new InMemoryObservabilityStore();
    store.recordError({ moduleId: 'a.one', code: 'E1' });
    store.recordError({ moduleId: 'b.two', code: 'E2' });
    store.recordError({ moduleId: 'a.one', code: 'E3' });

    expect(store.getErrors('a.one')).toEqual([
      { moduleId: 'a.one', code: 'E1' },
      { moduleId: 'a.one', code: 'E3' },
    ]);
    expect(store.getErrors('nobody.here')).toEqual([]);
  });

  it('treats null/undefined moduleId and limit as "no filter"', () => {
    const store = new InMemoryObservabilityStore();
    store.recordError({ moduleId: 'a.one', code: 'E1' });
    store.recordError({ moduleId: 'b.two', code: 'E2' });

    expect(store.getErrors(null, null)).toHaveLength(2);
    expect(store.getErrors(undefined, undefined)).toHaveLength(2);
  });

  it('applies limit as a leading slice, after the moduleId filter', () => {
    const store = new InMemoryObservabilityStore();
    store.recordError({ moduleId: 'a.one', code: 'E1' });
    store.recordError({ moduleId: 'b.two', code: 'E2' });
    store.recordError({ moduleId: 'a.one', code: 'E3' });

    expect(store.getErrors(null, 2)).toEqual([
      { moduleId: 'a.one', code: 'E1' },
      { moduleId: 'b.two', code: 'E2' },
    ]);
    // The limit counts entries that survived the filter, not raw entries:
    // slicing first would have returned only E1 here.
    expect(store.getErrors('a.one', 2)).toEqual([
      { moduleId: 'a.one', code: 'E1' },
      { moduleId: 'a.one', code: 'E3' },
    ]);
    // A limit larger than the result set is not an error.
    expect(store.getErrors(null, 99)).toHaveLength(3);
    expect(store.getErrors(null, 0)).toEqual([]);
  });

  it('returns a copy — mutating the result does not touch stored state', () => {
    const store = new InMemoryObservabilityStore();
    store.recordError({ moduleId: 'a.one', code: 'E1' });

    store.getErrors().push({ moduleId: 'injected', code: 'X' });

    expect(store.getErrors()).toHaveLength(1);
  });

  it('is empty after clear()', () => {
    const store = new InMemoryObservabilityStore();
    store.recordError({ moduleId: 'a.one', code: 'E1' });
    store.clear();
    expect(store.getErrors()).toEqual([]);
  });
});

describe('InMemoryObservabilityStore.getMetrics', () => {
  it('returns every recorded metric when no filter is given', () => {
    const store: ObservabilityStore = new InMemoryObservabilityStore();
    store.recordMetric(metric('calls', 1, 'a.one'));
    store.recordMetric(metric('latency_ms', 12, 'b.two'));

    expect(store.getMetrics().map((m) => m.name)).toEqual(['calls', 'latency_ms']);
    expect(store.getMetrics(null, null)).toHaveLength(2);
  });

  it('filters by moduleId', () => {
    const store = new InMemoryObservabilityStore();
    store.recordMetric(metric('calls', 1, 'a.one'));
    store.recordMetric(metric('calls', 2, 'b.two'));
    store.recordMetric(metric('latency_ms', 12, 'a.one'));

    expect(store.getMetrics('a.one').map((m) => m.name)).toEqual(['calls', 'latency_ms']);
    // A metric recorded without a module is not attributed to any module.
    store.recordMetric(metric('calls', 3, null));
    expect(store.getMetrics('a.one')).toHaveLength(2);
  });

  it('filters by metricName', () => {
    const store = new InMemoryObservabilityStore();
    store.recordMetric(metric('calls', 1, 'a.one'));
    store.recordMetric(metric('calls', 2, 'b.two'));
    store.recordMetric(metric('latency_ms', 12, 'a.one'));

    expect(store.getMetrics(null, 'calls').map((m) => m.value)).toEqual([1, 2]);
    expect(store.getMetrics(null, 'no_such_metric')).toEqual([]);
  });

  it('combines the moduleId and metricName filters', () => {
    const store = new InMemoryObservabilityStore();
    store.recordMetric(metric('calls', 1, 'a.one', { status: 'ok' }));
    store.recordMetric(metric('calls', 2, 'b.two'));
    store.recordMetric(metric('latency_ms', 12, 'a.one'));

    const found = store.getMetrics('a.one', 'calls');
    expect(found).toHaveLength(1);
    expect(found[0]!.value).toBe(1);
    expect(found[0]!.labels).toEqual({ status: 'ok' });
  });

  it('returns a copy — mutating the result does not touch stored state', () => {
    const store = new InMemoryObservabilityStore();
    store.recordMetric(metric('calls', 1, 'a.one'));

    store.getMetrics().push(metric('injected', 99, 'a.one'));

    expect(store.getMetrics()).toHaveLength(1);
  });

  it('is empty after clear(), and flush() leaves the read path intact', () => {
    const store = new InMemoryObservabilityStore();
    store.recordMetric(metric('calls', 1, 'a.one'));

    store.flush();
    expect(store.getMetrics()).toHaveLength(1);

    store.clear();
    expect(store.getMetrics()).toEqual([]);
  });
});
