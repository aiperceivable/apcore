/**
 * Sync finding D-08 — `RetryConfig.computeDelayMs` is the canonical
 * cross-language name. The legacy `computeDelay` alias must continue
 * to work but emit a one-shot deprecation warning.
 */
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { RetryConfig } from '../src/async-task.js';

describe('RetryConfig.computeDelayMs (D-08)', () => {
  let warnSpy: MockInstance<typeof console.warn>;

  beforeEach(() => {
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
  });

  it('exposes computeDelayMs and computes exponential backoff capped by maxRetryDelayMs', () => {
    const cfg = new RetryConfig({
      retryDelayMs: 1000,
      backoffMultiplier: 2.0,
      maxRetryDelayMs: 60000,
    });
    expect(typeof cfg.computeDelayMs).toBe('function');
    expect(cfg.computeDelayMs(0)).toBe(1000);
    expect(cfg.computeDelayMs(1)).toBe(2000);
    expect(cfg.computeDelayMs(2)).toBe(4000);
    // Cap kicks in
    expect(cfg.computeDelayMs(20)).toBe(60000);
  });

  it('legacy computeDelay still works and returns the same value', () => {
    const cfg = new RetryConfig({
      retryDelayMs: 500,
      backoffMultiplier: 3.0,
      maxRetryDelayMs: 10000,
    });
    // Legacy method must exist
    const legacy = (cfg as unknown as { computeDelay: (n: number) => number }).computeDelay.bind(
      cfg,
    );
    expect(typeof legacy).toBe('function');
    expect(legacy(0)).toBe(cfg.computeDelayMs(0));
    expect(legacy(1)).toBe(cfg.computeDelayMs(1));
  });

  it('legacy computeDelay emits a deprecation warning (one-shot per process)', async () => {
    // Use vi.resetModules() + dynamic import to get a fresh module state so
    // the one-shot bookkeeping is reset for this test.
    vi.resetModules();
    const mod = await import('../src/async-task.js');
    const cfg = new mod.RetryConfig({ retryDelayMs: 100 }) as unknown as {
      computeDelay: (n: number) => number;
    };
    cfg.computeDelay(0);
    cfg.computeDelay(1);
    cfg.computeDelay(2);

    const deprecationCalls = warnSpy.mock.calls.filter((call) =>
      String(call[0] ?? '').includes('computeDelay is deprecated'),
    );
    expect(deprecationCalls.length).toBe(1);
    expect(String(deprecationCalls[0][0])).toContain('computeDelayMs');
  });
});
