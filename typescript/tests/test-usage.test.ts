import { describe, it, expect } from 'vitest';
import { UsageCollector, UsageMiddleware, bucketKey } from '../src/observability/usage.js';
import { Context } from '../src/context.js';

describe('bucketKey', () => {
  it('returns YYYY-MM-DDTHH format', () => {
    const date = new Date('2026-03-08T14:30:00Z');
    expect(bucketKey(date)).toBe('2026-03-08T14');
  });
});

describe('UsageCollector', () => {
  it('records and retrieves usage summary', () => {
    const collector = new UsageCollector();
    collector.record('mod.a', 'caller1', 50, true);
    collector.record('mod.a', 'caller2', 100, false);

    const summaries = collector.getSummary('24h');
    expect(summaries).toHaveLength(1);
    expect(summaries[0].moduleId).toBe('mod.a');
    expect(summaries[0].callCount).toBe(2);
    expect(summaries[0].errorCount).toBe(1);
    expect(summaries[0].uniqueCallers).toBe(2);
  });

  it('returns module detail with caller breakdown', () => {
    const collector = new UsageCollector();
    collector.record('mod.a', 'caller1', 50, true);
    collector.record('mod.a', 'caller1', 100, true);
    collector.record('mod.a', 'caller2', 200, false);

    const detail = collector.getModule('mod.a', '24h');
    expect(detail.callCount).toBe(3);
    expect(detail.callers).toHaveLength(2);
    expect(detail.callers[0].callerId).toBe('caller1');
    expect(detail.callers[0].callCount).toBe(2);
  });

  it('returns latencies for a module', () => {
    const collector = new UsageCollector();
    collector.record('mod.a', 'c1', 50, true);
    collector.record('mod.a', 'c1', 100, true);

    const latencies = collector.getLatencies('mod.a', '24h');
    expect(latencies).toEqual([50, 100]);
  });

  it('returns empty for unknown module', () => {
    const collector = new UsageCollector();
    expect(collector.getSummary()).toEqual([]);
    expect(collector.getLatencies('unknown')).toEqual([]);
  });

  it('computes trend correctly', () => {
    const collector = new UsageCollector();
    // Only current period records
    collector.record('mod.a', 'c1', 50, true);
    const summaries = collector.getSummary('24h');
    expect(summaries[0].trend).toBe('new');
  });

  it('cleans up expired buckets', () => {
    const collector = new UsageCollector(1); // 1 hour retention
    // Record with a timestamp 2 hours ago
    const twoHoursAgo = new Date(Date.now() - 2 * 3600_000).toISOString();
    collector.record('mod.a', 'c1', 50, true, twoHoursAgo);
    // Record now to trigger cleanup
    collector.record('mod.a', 'c1', 50, true);
    const latencies = collector.getLatencies('mod.a', '24h');
    // Old record should have been evicted
    expect(latencies).toHaveLength(1);
  });

  it('computes rising trend when current > 1.2 * previous', () => {
    const collector = new UsageCollector();
    const now = Date.now();
    // Previous period: 5 records between 48h-24h ago
    for (let i = 0; i < 5; i++) {
      collector.record('mod.a', 'c1', 50, true, new Date(now - 36 * 3600_000 + i * 1000).toISOString());
    }
    // Current period: 10 records in last 24h
    for (let i = 0; i < 10; i++) {
      collector.record('mod.a', 'c1', 50, true, new Date(now - i * 1000).toISOString());
    }
    const summaries = collector.getSummary('24h');
    expect(summaries[0].trend).toBe('rising');
  });

  it('computes declining trend when current < 0.8 * previous', () => {
    const collector = new UsageCollector();
    const now = Date.now();
    // Previous period: 10 records between 48h-24h ago
    for (let i = 0; i < 10; i++) {
      collector.record('mod.a', 'c1', 50, true, new Date(now - 36 * 3600_000 + i * 1000).toISOString());
    }
    // Current period: 3 records in last 24h
    for (let i = 0; i < 3; i++) {
      collector.record('mod.a', 'c1', 50, true, new Date(now - i * 1000).toISOString());
    }
    const summaries = collector.getSummary('24h');
    expect(summaries[0].trend).toBe('declining');
  });

  it('computes stable trend when ratio is between 0.8 and 1.2', () => {
    const collector = new UsageCollector();
    const now = Date.now();
    // Previous period: 10 records
    for (let i = 0; i < 10; i++) {
      collector.record('mod.a', 'c1', 50, true, new Date(now - 36 * 3600_000 + i * 1000).toISOString());
    }
    // Current period: 10 records
    for (let i = 0; i < 10; i++) {
      collector.record('mod.a', 'c1', 50, true, new Date(now - i * 1000).toISOString());
    }
    const summaries = collector.getSummary('24h');
    expect(summaries[0].trend).toBe('stable');
  });

  it('supports day period format', () => {
    const collector = new UsageCollector();
    collector.record('mod.a', 'c1', 50, true);
    const summaries = collector.getSummary('1d');
    expect(summaries).toHaveLength(1);
    expect(summaries[0].callCount).toBe(1);
  });

  it('throws on invalid period format', () => {
    const collector = new UsageCollector();
    expect(() => collector.getSummary('invalid')).toThrow('Invalid period format');
    expect(() => collector.getSummary('h')).toThrow('Invalid period format');
    expect(() => collector.getSummary('-1h')).toThrow('Invalid period format');
    expect(() => collector.getSummary('abch')).toThrow('Invalid period format');
    expect(() => collector.getSummary('12abch')).toThrow('Invalid period format');
  });
});

describe('UsageMiddleware', () => {
  it('records successful call in after hook', () => {
    const collector = new UsageCollector();
    const mw = new UsageMiddleware(collector);
    const ctx = Context.create();

    mw.before('mod.a', {}, ctx);
    mw.after('mod.a', {}, { result: 'ok' }, ctx);

    const summaries = collector.getSummary('24h');
    expect(summaries).toHaveLength(1);
    expect(summaries[0].callCount).toBe(1);
    expect(summaries[0].errorCount).toBe(0);
  });

  it('records failed call in onError hook', () => {
    const collector = new UsageCollector();
    const mw = new UsageMiddleware(collector);
    const ctx = Context.create();

    mw.before('mod.a', {}, ctx);
    mw.onError('mod.a', {}, new Error('boom'), ctx);

    const summaries = collector.getSummary('24h');
    expect(summaries).toHaveLength(1);
    expect(summaries[0].errorCount).toBe(1);
  });

  it('handles missing start time gracefully', () => {
    const collector = new UsageCollector();
    const mw = new UsageMiddleware(collector);
    const ctx = Context.create();

    // Call after without before - should not throw
    mw.after('mod.a', {}, {}, ctx);
    const summaries = collector.getSummary('24h');
    expect(summaries).toHaveLength(1);
  });
});
