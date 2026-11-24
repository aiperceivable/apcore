/**
 * Span processors: SimpleSpanProcessor (synchronous) and BatchSpanProcessor (async, non-blocking).
 */

import type { Span, SpanExporter } from './tracing.js';

export interface SpanProcessor {
  onSpan(span: Span): void;
  shutdown(): Promise<void>;
}

/** Synchronous processor — exports each span immediately. For development and testing. */
export class SimpleSpanProcessor implements SpanProcessor {
  private readonly _exporter: SpanExporter;

  constructor(options: { exporter: SpanExporter }) {
    this._exporter = options.exporter;
  }

  onSpan(span: Span): void {
    this._exporter.export(span);
  }

  async shutdown(): Promise<void> {}
}

export interface BatchSpanProcessorOptions {
  exporter: SpanExporter;
  maxQueueSize?: number;
  scheduleDelayMs?: number;
  maxExportBatchSize?: number;
  exportTimeoutMs?: number;
}

/**
 * Non-blocking batch processor — buffers spans and exports in background.
 *
 * When the queue is full, new spans are dropped and spansDropped is incremented.
 * On shutdown, remaining spans are flushed within exportTimeoutMs.
 */
export class BatchSpanProcessor implements SpanProcessor {
  private readonly _exporter: SpanExporter;
  private readonly _maxQueueSize: number;
  private readonly _scheduleDelayMs: number;
  private readonly _maxExportBatchSize: number;
  private readonly _exportTimeoutMs: number;
  private _queue: Span[] = [];
  private _spansDropped = 0;
  private _timer: ReturnType<typeof setInterval> | null = null;

  constructor(options: BatchSpanProcessorOptions) {
    this._exporter = options.exporter;
    this._maxQueueSize = options.maxQueueSize ?? 2048;
    this._scheduleDelayMs = options.scheduleDelayMs ?? 5000;
    this._maxExportBatchSize = options.maxExportBatchSize ?? 512;
    this._exportTimeoutMs = options.exportTimeoutMs ?? 30000;
    this._startTimer();
  }

  get queueSize(): number {
    return this._queue.length;
  }

  get spansDropped(): number {
    return this._spansDropped;
  }

  onSpan(span: Span): void {
    if (this._queue.length >= this._maxQueueSize) {
      this._spansDropped++;
      return;
    }
    this._queue.push(span);
  }

  async shutdown(): Promise<void> {
    if (this._timer !== null) {
      clearInterval(this._timer);
      this._timer = null;
    }

    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    const timeoutPromise = new Promise<void>((_, reject) => {
      timeoutId = setTimeout(() => reject(new Error('Export timeout')), this._exportTimeoutMs);
    });

    try {
      await Promise.race([this._flush(), timeoutPromise]);
    } catch {
      this._queue = [];
    } finally {
      if (timeoutId !== null) clearTimeout(timeoutId);
    }
  }

  /**
   * Synchronously (from the caller's point of view) drain the queue,
   * exporting all currently buffered spans.
   *
   * Unlike `shutdown()`, the processor stays alive after this call; new
   * spans can continue to be enqueued. Resolves `true` once the queue is
   * empty, or `false` if `timeoutMs` elapses first. Safe to call on an
   * already-shut-down processor — `_flush()` only touches the queue and the
   * exporter, neither of which `shutdown()` tears down.
   *
   * Mirrors apcore-python's `BatchSpanProcessor.force_flush`.
   */
  async forceFlush(timeoutMs = 30000): Promise<boolean> {
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    const timeoutPromise = new Promise<void>((resolve) => {
      timeoutId = setTimeout(resolve, timeoutMs);
    });

    try {
      await Promise.race([this._flush(), timeoutPromise]);
    } finally {
      if (timeoutId !== null) clearTimeout(timeoutId);
    }
    return this._queue.length === 0;
  }

  private _startTimer(): void {
    this._timer = setInterval(() => {
      void this._flush();
    }, this._scheduleDelayMs);
    // Prevent the timer from keeping the Node.js event loop alive in tests
    if (typeof (this._timer as unknown as { unref?: () => void }).unref === 'function') {
      (this._timer as unknown as { unref: () => void }).unref();
    }
  }

  private async _flush(): Promise<void> {
    while (this._queue.length > 0) {
      const batch = this._queue.splice(0, this._maxExportBatchSize);
      for (const span of batch) {
        this._exporter.export(span);
      }
    }
  }
}
