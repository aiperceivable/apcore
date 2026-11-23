/**
 * Prometheus text-format exporter with /metrics, /healthz, and /readyz HTTP endpoints.
 */

import { createServer, type Server } from 'node:http';
import type { MetricsCollector } from './metrics.js';
import type { UsageCollector } from './usage.js';

function computePercentiles(values: number[], ps: readonly number[]): number[] {
  if (values.length === 0) return ps.map(() => 0);
  const sorted = [...values].sort((a, b) => a - b);
  return ps.map((p) => {
    const idx = Math.ceil((p / 100) * sorted.length) - 1;
    return sorted[Math.max(0, idx)];
  });
}

export interface PrometheusExporterStartOptions {
  port?: number;
  path?: string;
}

/**
 * Serves Prometheus text metrics and K8s health endpoints over HTTP.
 *
 * Serves:
 *   GET {path}   (default /metrics) — Prometheus text exposition format
 *   GET /healthz — liveness probe, always 200 OK
 *   GET /readyz  — readiness probe, 200 OK after markReady() is called
 */
export class PrometheusExporter {
  private readonly _collector: MetricsCollector;
  private readonly _usageCollector: UsageCollector | null;
  private _server: Server | null = null;
  private _ready = false;

  constructor(options: { collector: MetricsCollector; usageCollector?: UsageCollector }) {
    this._collector = options.collector;
    this._usageCollector = options.usageCollector ?? null;
  }

  /** Return current metrics in Prometheus text exposition format. */
  export(): string {
    let output = this._collector.exportPrometheus();
    if (this._usageCollector !== null) {
      output += this._exportUsageMetrics();
    }
    return output;
  }

  private _exportUsageMetrics(): string {
    const summaries = this._usageCollector!.getSummary();
    if (summaries.length === 0) return '';

    // Build per-metric-family lines so HELP/TYPE immediately precede each family's data.
    const callsLines: string[] = [
      '# HELP apcore_usage_calls_total Total usage calls by module and status',
      '# TYPE apcore_usage_calls_total counter',
    ];
    const errorRateLines: string[] = [
      '# HELP apcore_usage_error_rate Module error rate (0.0-1.0)',
      '# TYPE apcore_usage_error_rate gauge',
    ];
    const p50Lines: string[] = [
      '# HELP apcore_usage_p50_latency_ms Module p50 latency in milliseconds',
      '# TYPE apcore_usage_p50_latency_ms gauge',
    ];
    const p95Lines: string[] = [
      '# HELP apcore_usage_p95_latency_ms Module p95 latency in milliseconds',
      '# TYPE apcore_usage_p95_latency_ms gauge',
    ];
    const p99Lines: string[] = [
      '# HELP apcore_usage_p99_latency_ms Module p99 latency in milliseconds',
      '# TYPE apcore_usage_p99_latency_ms gauge',
    ];

    for (const summary of summaries) {
      const { moduleId, callCount, errorCount } = summary;
      const successCount = callCount - errorCount;
      const errorRate = callCount > 0 ? errorCount / callCount : 0;

      callsLines.push(`apcore_usage_calls_total{module_id="${moduleId}",status="success"} ${successCount}`);
      callsLines.push(`apcore_usage_calls_total{module_id="${moduleId}",status="error"} ${errorCount}`);
      errorRateLines.push(`apcore_usage_error_rate{module_id="${moduleId}"} ${errorRate}`);

      const latencies = this._usageCollector!.getLatencies(moduleId);
      const [p50, p95, p99] = computePercentiles(latencies, [50, 95, 99]);
      p50Lines.push(`apcore_usage_p50_latency_ms{module_id="${moduleId}"} ${p50}`);
      p95Lines.push(`apcore_usage_p95_latency_ms{module_id="${moduleId}"} ${p95}`);
      p99Lines.push(`apcore_usage_p99_latency_ms{module_id="${moduleId}"} ${p99}`);
    }

    return [...callsLines, ...errorRateLines, ...p50Lines, ...p95Lines, ...p99Lines].join('\n') + '\n';
  }

  /** Signal that the application is ready to serve traffic (/readyz → 200). */
  markReady(): void {
    this._ready = true;
  }

  /** Start the HTTP server in a background listener. */
  start(options?: PrometheusExporterStartOptions): void {
    const port = options?.port ?? 9090;
    const metricsPath = options?.path ?? '/metrics';

    if (this._server !== null) {
      throw new Error('PrometheusExporter is already running. Call stop() first.');
    }

    this._server = createServer((req, res) => {
      const url = req.url ?? '/';
      if (url === metricsPath) {
        const body = Buffer.from(this.export(), 'utf-8');
        res.writeHead(200, {
          'Content-Type': 'text/plain; version=0.0.4; charset=utf-8',
          'Content-Length': String(body.length),
        });
        res.end(body);
      } else if (url === '/healthz') {
        res.writeHead(200);
        res.end('OK');
      } else if (url === '/readyz') {
        if (this._ready) {
          res.writeHead(200);
          res.end('OK');
        } else {
          res.writeHead(503);
          res.end('Not ready');
        }
      } else {
        res.writeHead(404);
        res.end();
      }
    });

    this._server.listen(port);
  }

  /** Shut down the HTTP server. */
  stop(): Promise<void> {
    return new Promise((resolve) => {
      if (this._server !== null) {
        const server = this._server;
        this._server = null;
        server.close(() => resolve());
        server.closeIdleConnections();
      } else {
        resolve();
      }
    });
  }
}
