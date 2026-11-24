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
  private _draining: Promise<void> | null = null;
  private _stopping: Promise<void> | null = null;

  constructor(options: BatchSpanProcessorOptions) {
    this._exporter = options.exporter;
    this._maxQueueSize = options.maxQueueSize ?? 2048;
    this._scheduleDelayMs = options.scheduleDelayMs ?? 5000;
    this._maxExportBatchSize = options.maxExportBatchSize ?? 512;
    if (!Number.isInteger(this._maxExportBatchSize) || this._maxExportBatchSize < 1) {
      throw new RangeError('maxExportBatchSize must be a positive integer');
    }
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
    if (this._stopping !== null) return;
    if (this._queue.length >= this._maxQueueSize) {
      this._spansDropped++;
      return;
    }
    this._queue.push(span);
  }

  async shutdown(): Promise<void> {
    if (this._stopping !== null) return this._stopping;
    if (this._timer !== null) {
      clearInterval(this._timer);
      this._timer = null;
    }
    this._stopping = this.forceFlush(this._exportTimeoutMs).then(() => {});
    return this._stopping;
  }

  /**
   * Synchronously (from the caller's point of view) drain the queue,
   * exporting all currently buffered spans.
   *
   * Unlike `shutdown()`, the processor stays alive after this call; new
   * spans can continue to be enqueued. Resolves `true` once the queue is
   * empty and in-flight exports settle, or `false` if `timeoutMs` elapses
   * first. A later flush can continue waiting for the same drain.
   *
   * Mirrors apcore-python's `BatchSpanProcessor.force_flush`.
   */
  async forceFlush(timeoutMs = 30000): Promise<boolean> {
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    const timeoutPromise = new Promise<boolean>((resolve) => {
      timeoutId = setTimeout(() => resolve(false), timeoutMs);
    });

    try {
      return await Promise.race([this._flush().then(() => true), timeoutPromise]);
    } finally {
      if (timeoutId !== null) clearTimeout(timeoutId);
    }
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

  private _flush(): Promise<void> {
    if (this._draining === null) {
      this._draining = Promise.resolve().then(() => this._drain()).finally(() => {
        this._draining = null;
      });
    }
    return this._draining;
  }

  private async _drain(): Promise<void> {
    while (this._queue.length > 0) {
      const batch = this._queue.splice(0, this._maxExportBatchSize);
      await Promise.all(batch.map(async span => {
        try {
          await this._exporter.export(span);
        } catch (error) {
          console.warn('[apcore:tracing] Span export failed:', error);
        }
      }));
    }
  }
}
