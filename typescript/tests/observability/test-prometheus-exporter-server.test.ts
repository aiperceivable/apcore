/**
 * PrometheusExporter HTTP server — covers the start()/stop() lifecycle and
 * the /metrics, /healthz, /readyz, and 404 routes.
 *
 * Uses port 0 so the OS picks a free ephemeral port; tests don't compete
 * for a fixed listener.
 */
import { afterEach, describe, it, expect } from 'vitest';
import {
  MetricsCollector,
  PrometheusExporter,
  UsageCollector,
} from '../../src/observability/index.js';
import type { AddressInfo } from 'node:net';

interface ExporterInternals {
  _server: { address(): AddressInfo | string | null } | null;
}

async function fetchText(port: number, path: string): Promise<{ status: number; body: string }> {
  // Each case closes its ephemeral listener. Do not let the shared fetch pool
  // reuse an idle socket after that listener closes and its port is reassigned.
  const res = await fetch(`http://127.0.0.1:${port}${path}`, { headers: { Connection: 'close' } });
  const body = await res.text();
  return { status: res.status, body };
}

function getPort(exporter: PrometheusExporter): number {
  const internals = exporter as unknown as ExporterInternals;
  const addr = internals._server?.address();
  if (addr === null || addr === undefined || typeof addr === 'string') {
    throw new Error('exporter is not listening');
  }
  return addr.port;
}

describe('PrometheusExporter HTTP server', () => {
  let exporter: PrometheusExporter | null = null;

  afterEach(async () => {
    if (exporter !== null) {
      await exporter.stop();
      exporter = null;
    }
  });

  it('serves /metrics with Prometheus content type and metrics body', async () => {
    const collector = new MetricsCollector();
    collector.increment('apcore_module_calls_total', { module_id: 'mod.x', status: 'success' });
    exporter = new PrometheusExporter({ collector });
    exporter.start({ port: 0 });
    const port = getPort(exporter);

    const res = await fetch(`http://127.0.0.1:${port}/metrics`, { headers: { Connection: 'close' } });
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toContain('text/plain');
    const body = await res.text();
    expect(body).toContain('apcore');
  });

  it('serves /metrics on the configured custom path', async () => {
    const collector = new MetricsCollector();
    exporter = new PrometheusExporter({ collector });
    exporter.start({ port: 0, path: '/p' });
    const port = getPort(exporter);

    const ok = await fetchText(port, '/p');
    expect(ok.status).toBe(200);

    const missing = await fetchText(port, '/metrics');
    expect(missing.status).toBe(404);
  });

  it('/healthz returns 200 OK', async () => {
    exporter = new PrometheusExporter({ collector: new MetricsCollector() });
    exporter.start({ port: 0 });
    const port = getPort(exporter);

    const r = await fetchText(port, '/healthz');
    expect(r.status).toBe(200);
    expect(r.body).toBe('OK');
  });

  it('/readyz returns 503 before markReady() and 200 after', async () => {
    exporter = new PrometheusExporter({ collector: new MetricsCollector() });
    exporter.start({ port: 0 });
    const port = getPort(exporter);

    const before = await fetchText(port, '/readyz');
    expect(before.status).toBe(503);
    expect(before.body).toBe('Not ready');

    exporter.markReady();
    const after = await fetchText(port, '/readyz');
    expect(after.status).toBe(200);
    expect(after.body).toBe('OK');
  });

  it('returns 404 for unknown paths', async () => {
    exporter = new PrometheusExporter({ collector: new MetricsCollector() });
    exporter.start({ port: 0 });
    const port = getPort(exporter);

    const r = await fetchText(port, '/nope');
    expect(r.status).toBe(404);
  });

  it('start() throws when called twice without stop()', () => {
    exporter = new PrometheusExporter({ collector: new MetricsCollector() });
    exporter.start({ port: 0 });
    expect(() => exporter!.start({ port: 0 })).toThrow(/already running/i);
  });

  it('stop() resolves even when start() was never called', async () => {
    const e = new PrometheusExporter({ collector: new MetricsCollector() });
    await expect(e.stop()).resolves.toBeUndefined();
  });

  it('export() includes usage metrics when a UsageCollector is attached', async () => {
    const collector = new MetricsCollector();
    const usageCollector = new UsageCollector();
    usageCollector.record('mod.usage', 'caller', 25, true);
    usageCollector.record('mod.usage', 'caller', 30, false);
    exporter = new PrometheusExporter({ collector, usageCollector });
    exporter.start({ port: 0 });
    const port = getPort(exporter);

    const r = await fetchText(port, '/metrics');
    expect(r.body).toContain('apcore_usage_calls_total');
    expect(r.body).toContain('apcore_usage_p50_latency_ms');
  });

  it('export() omits usage block when no usage data has been recorded', () => {
    const collector = new MetricsCollector();
    const usageCollector = new UsageCollector();
    const e = new PrometheusExporter({ collector, usageCollector });
    const text = e.export();
    expect(text).not.toContain('apcore_usage_calls_total');
  });
});
