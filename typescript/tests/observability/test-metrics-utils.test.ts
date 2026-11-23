import { describe, it, expect } from 'vitest';
import { MetricsCollector } from '../../src/observability/metrics.js';
import {
  matchesModuleId,
  computeModuleErrorRate,
  estimateP99FromHistogram,
} from '../../src/observability/metrics-utils.js';

describe('matchesModuleId', () => {
  it('matches exact module ID when followed by comma', () => {
    const key = 'apcore_module_calls_total|module_id=mod.a,status=success';
    expect(matchesModuleId(key, 'mod.a')).toBe(true);
  });

  it('matches exact module ID at end of string', () => {
    const key = 'apcore_module_duration_seconds|module_id=mod.a';
    expect(matchesModuleId(key, 'mod.a')).toBe(true);
  });

  it('does not match when module ID is a suffix of another', () => {
    const key = 'apcore_module_calls_total|module_id=foo.mod.a,status=success';
    expect(matchesModuleId(key, 'mod.a')).toBe(false);
  });

  it('does not match when module ID is a prefix of another', () => {
    const key = 'apcore_module_calls_total|module_id=mod.abc,status=success';
    expect(matchesModuleId(key, 'mod.a')).toBe(false);
  });
});

describe('computeModuleErrorRate', () => {
  it('returns zeros when no metrics recorded', () => {
    const collector = new MetricsCollector();
    const result = computeModuleErrorRate(collector, 'mod.a');
    expect(result).toEqual({ totalCalls: 0, errorCount: 0, errorRate: 0 });
  });

  it('computes correct error rate with mixed success and error calls', () => {
    const collector = new MetricsCollector();
    collector.incrementCalls('mod.a', 'success');
    collector.incrementCalls('mod.a', 'success');
    collector.incrementCalls('mod.a', 'success');
    collector.incrementCalls('mod.a', 'error');

    const result = computeModuleErrorRate(collector, 'mod.a');
    expect(result.totalCalls).toBe(4);
    expect(result.errorCount).toBe(1);
    expect(result.errorRate).toBeCloseTo(0.25);
  });

  it('only counts the specified module metrics, not others', () => {
    const collector = new MetricsCollector();
    collector.incrementCalls('mod.a', 'success');
    collector.incrementCalls('mod.a', 'error');
    collector.incrementCalls('mod.b', 'error');
    collector.incrementCalls('mod.b', 'error');
    collector.incrementCalls('mod.b', 'error');

    const resultA = computeModuleErrorRate(collector, 'mod.a');
    expect(resultA.totalCalls).toBe(2);
    expect(resultA.errorCount).toBe(1);
    expect(resultA.errorRate).toBeCloseTo(0.5);

    const resultB = computeModuleErrorRate(collector, 'mod.b');
    expect(resultB.totalCalls).toBe(3);
    expect(resultB.errorCount).toBe(3);
    expect(resultB.errorRate).toBeCloseTo(1.0);
  });
});

describe('estimateP99FromHistogram', () => {
  it('returns zeros when no observations', () => {
    const collector = new MetricsCollector();
    const result = estimateP99FromHistogram(collector, 'mod.a');
    expect(result).toEqual({ avgLatencyMs: 0, p99LatencyMs: 0 });
  });

  it('returns correct p99 for fast observations', () => {
    const collector = new MetricsCollector();
    // Observe 100 fast values (0.001s each) so p99 target = 99
    // All fall in the 0.005 bucket, so p99 = 0.005 * 1000 = 5ms
    for (let i = 0; i < 100; i++) {
      collector.observeDuration('mod.a', 0.001);
    }

    const result = estimateP99FromHistogram(collector, 'mod.a');
    expect(result.p99LatencyMs).toBe(5);
  });

  it('returns last bucket upper bound when all observations exceed largest bucket', () => {
    const collector = new MetricsCollector();
    // Observe values that exceed every default bucket (max is 60.0)
    collector.observeDuration('mod.a', 100);
    collector.observeDuration('mod.a', 200);

    const result = estimateP99FromHistogram(collector, 'mod.a');
    // Last bucket is 60.0s = 60000ms
    expect(result.p99LatencyMs).toBe(60000);
  });

  it('returns correct avg latency', () => {
    const collector = new MetricsCollector();
    collector.observeDuration('mod.a', 0.1);
    collector.observeDuration('mod.a', 0.3);

    const result = estimateP99FromHistogram(collector, 'mod.a');
    // avg = (0.1 + 0.3) / 2 = 0.2s = 200ms
    expect(result.avgLatencyMs).toBeCloseTo(200);
  });

  it('uses collector custom buckets instead of hardcoded defaults', () => {
    // Custom collector with coarse buckets: 1s, 2s, 5s only
    const collector = new MetricsCollector([1.0, 2.0, 5.0]);
    // Observe 100 values at 0.5s — all fall in the 1.0s bucket
    for (let i = 0; i < 100; i++) {
      collector.observeDuration('mod.custom', 0.5);
    }
    const result = estimateP99FromHistogram(collector, 'mod.custom');
    // p99 target = 99; all 100 land in 1.0s bucket → p99 = 1000ms
    expect(result.p99LatencyMs).toBe(1000);
  });

  it('returns last custom bucket when observations exceed all custom buckets', () => {
    const collector = new MetricsCollector([0.1, 0.5]);
    collector.observeDuration('mod.custom', 10.0); // exceeds all buckets
    const result = estimateP99FromHistogram(collector, 'mod.custom');
    // Last custom bucket is 0.5s = 500ms
    expect(result.p99LatencyMs).toBe(500);
  });
});
