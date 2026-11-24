/**
 * UsageExporter — push interface for delivering UsageCollector summaries
 * to external sinks on a schedule (Issue #45 §3).
 *
 * The collector pulls; the exporter pushes. Keep them decoupled so an
 * application can swap in (for example) a Prometheus, OTLP, or HTTP
 * exporter without changing how the collector is wired into the pipeline.
 *
 * Cross-language alignment: mirrors apcore-python's
 * `apcore.observability.usage.UsageExporter` / `PeriodicUsageExporter` and
 * the Rust trait in `apcore::observability::usage::UsageExporter`.
 */

import type { UsageCollector } from './usage.js';

/**
 * Contract for a usage-summary sink.
 *
 * `export` is invoked by {@link PeriodicUsageExporter} (or any caller) with
 * the latest summary snapshot. Implementations may be sync or async; the
 * scheduler awaits whichever is returned.
 *
 * `shutdown` is invoked once during graceful teardown so an exporter can
 * flush buffers, close sockets, etc.
 */
export interface UsageExporter {
  export(summary: Record<string, unknown>): Promise<void> | void;
  shutdown(): Promise<void> | void;
}

/**
 * No-op exporter — the safe default. Useful as a placeholder when an
 * application has registered a {@link PeriodicUsageExporter} but does not
 * yet have a real backend wired up.
 */
export class NoopUsageExporter implements UsageExporter {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  export(_summary: Record<string, unknown>): void {
    // no-op
  }

  shutdown(): void {
    // no-op
  }
}

/**
 * Periodically pushes a {@link UsageCollector} summary into a
 * {@link UsageExporter}.
 *
 * Default interval is 1 hour (3 600 000 ms) which matches apcore-python's
 * `PeriodicUsageExporter` default. The first push happens one interval
 * after `start()`; nothing is exported synchronously on start.
 */
export class PeriodicUsageExporter {
  private _timer: ReturnType<typeof setInterval> | null = null;
  private _running = false;

  constructor(
    private readonly collector: UsageCollector,
    private readonly exporter: UsageExporter,
    private readonly intervalMs: number = 3_600_000,
  ) {}

  /** Begin periodic export. Idempotent — calling twice is a no-op. */
  start(): void {
    if (this._running) return;
    this._running = true;
    this._timer = setInterval(() => {
      this._tick();
    }, this.intervalMs);
  }

  /**
   * Stop the timer and await `exporter.shutdown()`.
   *
   * Idempotent (usage_exporter.json →
   * `periodic_usage_exporter_stop_is_idempotent_and_drains`): `shutdown()` is
   * invoked at most ONCE per `start()`, so calling `stop()` twice yields
   * `shutdown_call_count == 1`. `stop()` before `start()` is a pure no-op and
   * does NOT call `shutdown()` — the exporter was never taken over, mirroring
   * apcore-python's `PeriodicUsageExporter.stop()` (returns early when no task
   * is running) and BatchSpanProcessor semantics.
   */
  async stop(): Promise<void> {
    if (!this._running) return;
    this._running = false;
    if (this._timer !== null) {
      clearInterval(this._timer);
      this._timer = null;
    }
    await this.exporter.shutdown();
  }

  private _tick(): void {
    let summary: Record<string, unknown>;
    try {
      summary = { modules: this.collector.getSummary() };
    } catch (e) {
      console.warn('[apcore:usage-exporter] Failed to build summary:', e);
      return;
    }
    try {
      const result = this.exporter.export(summary);
      if (result instanceof Promise) {
        result.catch((e: unknown) => {
          console.warn('[apcore:usage-exporter] export() rejected:', e);
        });
      }
    } catch (e) {
      console.warn('[apcore:usage-exporter] export() threw:', e);
    }
  }
}
