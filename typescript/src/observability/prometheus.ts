/** Render metrics without opening a listener or depending on Node. */

import type { MetricsCollector } from './metrics.js';
import type { UsageCollector } from './usage.js';

type PrometheusUsageCollector = Pick<UsageCollector, 'getSummary' | 'getLatencies'>;

function computePercentiles(values: number[], ps: readonly number[]): number[] {
  if (values.length === 0) return ps.map(() => 0);
  const sorted = [...values].sort((a, b) => a - b);
  return ps.map((p) => {
    const idx = Math.ceil((p / 100) * sorted.length) - 1;
    return sorted[Math.max(0, idx)];
  });
}

function renderUsageMetrics(usageCollector: PrometheusUsageCollector): string {
  const summaries = usageCollector.getSummary();
  if (summaries.length === 0) return '';

  // Build per-metric-family lines so HELP/TYPE immediately precede each family's data.
  const callFamilies = [
    { name: 'apcore_usage_calls_24h', help: 'Module calls in the last 24 hours by status' },
    { name: 'apcore_usage_calls_total', help: 'Deprecated alias of apcore_usage_calls_24h' },
  ].map(({ name, help }) => ({ name, lines: [`# HELP ${name} ${help}`, `# TYPE ${name} gauge`] }));
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

    for (const { name, lines } of callFamilies) {
      lines.push(`${name}{module_id="${moduleId}",status="success"} ${successCount}`);
      lines.push(`${name}{module_id="${moduleId}",status="error"} ${errorCount}`);
    }
    errorRateLines.push(`apcore_usage_error_rate{module_id="${moduleId}"} ${errorRate}`);

    const latencies = usageCollector.getLatencies(moduleId);
    const [p50, p95, p99] = computePercentiles(latencies, [50, 95, 99]);
    p50Lines.push(`apcore_usage_p50_latency_ms{module_id="${moduleId}"} ${p50}`);
    p95Lines.push(`apcore_usage_p95_latency_ms{module_id="${moduleId}"} ${p95}`);
    p99Lines.push(`apcore_usage_p99_latency_ms{module_id="${moduleId}"} ${p99}`);
  }

  return [...callFamilies.flatMap(({ lines }) => lines), ...errorRateLines, ...p50Lines, ...p95Lines, ...p99Lines]
    .join('\n') + '\n';
}

/** Render the current metrics and optional usage metrics in Prometheus text format. */
export function renderPrometheusMetrics(
  collector: Pick<MetricsCollector, 'exportPrometheus'>,
  usageCollector?: PrometheusUsageCollector,
): string {
  const metrics = collector.exportPrometheus();
  return usageCollector === undefined ? metrics : metrics + renderUsageMetrics(usageCollector);
}
