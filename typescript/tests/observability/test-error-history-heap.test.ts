/**
 * Heap-eviction performance contract for ErrorHistory.
 *
 * Records far more entries than the configured `maxTotalEntries` across many
 * modules. The min-heap path must keep the live total ≤ cap and the resulting
 * surviving entries' fingerprints must match the live per-module index
 * (lazy-deletion correctness).
 */

import { describe, it, expect, vi } from 'vitest';
import { ErrorHistory } from '../../src/observability/error-history.js';
import { ModuleError } from '../../src/errors.js';

describe('ErrorHistory min-heap eviction', () => {
  it('keeps one heap entry per fingerprint and evicts by latest occurrence within the same tick', () => {
    vi.useFakeTimers();
    try {
      const history = new ErrorHistory({ maxTotalEntries: 2 });
      const repeated = new ModuleError('CODE', 'repeated');
      history.record('mod.one', repeated);
      history.record('mod.two', new ModuleError('CODE', 'second'));
      for (let i = 0; i < 1000; i++) history.record('mod.one', repeated);
      const heap = (history as unknown as { _heap: { size: number } })._heap;
      expect(heap.size).toBe(2);
      history.record('mod.three', new ModuleError('CODE', 'third'));
      expect(history.getAll().map(entry => entry.moduleId)).toEqual(['mod.three', 'mod.one']);
      history.clearModule('mod.one');
      expect(heap.size).toBe(1);
      history.clear();
      expect(heap.size).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps total within cap across many modules', () => {
    const cap = 100;
    const history = new ErrorHistory({
      maxEntriesPerModule: 1000,
      maxTotalEntries: cap,
    });

    // 50 modules × 20 unique fingerprints = 1000 records, cap is 100
    for (let m = 0; m < 50; m++) {
      for (let i = 0; i < 20; i++) {
        history.record(`mod.${m}`, new ModuleError('CODE', `module ${m} error variant ${i}`));
      }
    }

    const all = history.getAll();
    expect(all.length).toBeLessThanOrEqual(cap);
    expect(all.length).toBeGreaterThan(0);
  });

  it('lazy-deletion keeps consistency between heap and live fingerprint index', () => {
    const cap = 50;
    const history = new ErrorHistory({
      maxEntriesPerModule: 1000,
      maxTotalEntries: cap,
    });

    // Mix unique entries with dedup-refreshed entries — dedup updates lastOccurred,
    // which leaves stale heap items behind. Min-heap pop must skip them.
    for (let i = 0; i < 200; i++) {
      const mod = `mod.${i % 10}`;
      history.record(mod, new ModuleError('CODE', `unique message ${i}`));
      // Re-record some earlier entries to bump lastOccurred (creates stale heap items)
      if (i % 5 === 0 && i > 0) {
        history.record(`mod.${(i - 5) % 10}`, new ModuleError('CODE', `unique message ${i - 5}`));
      }
    }

    const all = history.getAll();
    expect(all.length).toBeLessThanOrEqual(cap);

    // Every surviving entry must be reachable via the per-module index too.
    for (const entry of all) {
      const moduleEntries = history.get(entry.moduleId);
      expect(moduleEntries.some((e) => e.fingerprint === entry.fingerprint)).toBe(true);
    }
  });

  it('completes large eviction cycle without quadratic blow-up', () => {
    // Smoke test: 5000 records with cap 100 must finish quickly.
    const history = new ErrorHistory({
      maxEntriesPerModule: 5000,
      maxTotalEntries: 100,
    });

    const start = Date.now();
    for (let i = 0; i < 5000; i++) {
      history.record(`mod.${i % 200}`, new ModuleError('CODE', `msg ${i}`));
    }
    const elapsed = Date.now() - start;

    // With heap eviction this is well under a second; flag if it regresses badly.
    expect(elapsed).toBeLessThan(5000);
    expect(history.getAll().length).toBeLessThanOrEqual(100);
  });
});
