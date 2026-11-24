/**
 * [D5-001] BatchSpanProcessor unit coverage — parity with
 * apcore-python/tests/observability/test_batch_span_processor.py.
 *
 * Covered behaviors (mirroring the Python suite where the TS impl has the feature):
 *  - re-export from the observability barrel (module surface),
 *  - onSpan enqueues without exporting eagerly,
 *  - queue-full backpressure: spans are dropped and spansDropped increments,
 *  - timed/interval background flush exports buffered spans,
 *  - forceFlush drains the queue synchronously (from the caller's point of view),
 *  - shutdown flushes remaining spans,
 *  - shutdown is idempotent.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  BatchSpanProcessor,
  InMemoryExporter,
  createSpan,
  type Span,
} from '../../src/observability/index.js';
import { BatchSpanProcessor as DirectImport } from '../../src/observability/batch-span-processor.js';

function makeSpan(name = 's'): Span {
  return createSpan({ traceId: 't1', name, startTime: Date.now() });
}

describe('BatchSpanProcessor module surface', () => {
  it('is the same class via the observability barrel and the direct module', () => {
    expect(BatchSpanProcessor).toBe(DirectImport);
  });
});

describe('BatchSpanProcessor enqueue', () => {
  it('enqueues spans via onSpan without exporting eagerly', async () => {
    const exporter = new InMemoryExporter();
    const proc = new BatchSpanProcessor({
      exporter,
      maxQueueSize: 10,
      scheduleDelayMs: 60_000,
    });
    try {
      proc.onSpan(makeSpan());
      proc.onSpan(makeSpan());
      expect(proc.queueSize).toBe(2);
      expect(exporter.getSpans()).toEqual([]);
    } finally {
      await proc.shutdown();
    }
  });
});

describe('BatchSpanProcessor queue full', () => {
  it('drops spans when the queue is full and increments spansDropped', async () => {
    const exporter = new InMemoryExporter();
    const proc = new BatchSpanProcessor({
      exporter,
      maxQueueSize: 2,
      scheduleDelayMs: 60_000,
    });
    try {
      proc.onSpan(makeSpan());
      proc.onSpan(makeSpan());
      expect(proc.spansDropped).toBe(0);

      proc.onSpan(makeSpan()); // third — must drop
      proc.onSpan(makeSpan()); // fourth — must drop
      expect(proc.queueSize).toBe(2);
      expect(proc.spansDropped).toBe(2);
    } finally {
      await proc.shutdown();
    }
  });
});

describe('BatchSpanProcessor forceFlush', () => {
  it('drains the queue synchronously (from the caller\'s point of view)', async () => {
    const exporter = new InMemoryExporter();
    const proc = new BatchSpanProcessor({
      exporter,
      maxQueueSize: 10,
      scheduleDelayMs: 60_000,
    });
    try {
      for (let i = 0; i < 3; i++) {
        proc.onSpan(makeSpan());
      }
      expect(proc.queueSize).toBe(3);
      const drained = await proc.forceFlush(2000);
      expect(drained).toBe(true);
      expect(proc.queueSize).toBe(0);
      expect(exporter.getSpans()).toHaveLength(3);
    } finally {
      await proc.shutdown();
    }
  });

  it('is safe to call on an already-shut-down processor', async () => {
    const exporter = new InMemoryExporter();
    const proc = new BatchSpanProcessor({ exporter, scheduleDelayMs: 60_000 });
    await proc.shutdown();
    await expect(proc.forceFlush(1000)).resolves.toBe(true);
  });
});

describe('BatchSpanProcessor background flush', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('exports buffered spans on the timed interval flush', async () => {
    const exporter = new InMemoryExporter();
    const proc = new BatchSpanProcessor({
      exporter,
      maxQueueSize: 10,
      scheduleDelayMs: 50,
    });
    try {
      for (let i = 0; i < 3; i++) {
        proc.onSpan(makeSpan());
      }
      expect(exporter.getSpans()).toEqual([]);

      // Advance past one schedule delay so the interval flush fires.
      await vi.advanceTimersByTimeAsync(60);

      expect(exporter.getSpans()).toHaveLength(3);
      expect(proc.queueSize).toBe(0);
    } finally {
      await proc.shutdown();
    }
  });
});

describe('BatchSpanProcessor shutdown', () => {
  it('flushes remaining spans on shutdown', async () => {
    const exporter = new InMemoryExporter();
    const proc = new BatchSpanProcessor({
      exporter,
      maxQueueSize: 10,
      scheduleDelayMs: 60_000,
    });
    proc.onSpan(makeSpan());
    proc.onSpan(makeSpan());
    await proc.shutdown();
    expect(exporter.getSpans()).toHaveLength(2);
  });

  it('is idempotent — a second shutdown does not throw', async () => {
    const exporter = new InMemoryExporter();
    const proc = new BatchSpanProcessor({ exporter, scheduleDelayMs: 60_000 });
    await proc.shutdown();
    await expect(proc.shutdown()).resolves.toBeUndefined();
  });
});
