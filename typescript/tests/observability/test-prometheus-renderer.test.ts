import { describe, expect, it } from 'vitest';
import {
  MetricsCollector,
  UsageCollector,
  renderPrometheusMetrics,
} from '../../src/observability/portable.js';

describe('renderPrometheusMetrics', () => {
  it('preserves the collected text when there is no usage data', () => {
    const bytes = 'sample_counter{label="\u6e2c\u8a66"} 1\n';
    const metrics = { exportPrometheus: () => bytes };
    expect(renderPrometheusMetrics(metrics)).toBe(bytes);
    expect(renderPrometheusMetrics(metrics, new UsageCollector())).toBe(bytes);
  });

  it('preserves all usage metric families and the final newline', () => {
    const collector = new MetricsCollector();
    collector.increment('sample_counter', {});
    const usageCollector = new UsageCollector();
    usageCollector.record('sample.module', 'caller', 25, true);
    usageCollector.record('sample.module', 'caller', 30, false);
    const usage = [
      '# HELP apcore_usage_calls_24h Module calls in the last 24 hours by status',
      '# TYPE apcore_usage_calls_24h gauge',
      'apcore_usage_calls_24h{module_id="sample.module",status="success"} 1',
      'apcore_usage_calls_24h{module_id="sample.module",status="error"} 1',
      '# HELP apcore_usage_calls_total Deprecated alias of apcore_usage_calls_24h',
      '# TYPE apcore_usage_calls_total gauge',
      'apcore_usage_calls_total{module_id="sample.module",status="success"} 1',
      'apcore_usage_calls_total{module_id="sample.module",status="error"} 1',
      '# HELP apcore_usage_error_rate Module error rate (0.0-1.0)',
      '# TYPE apcore_usage_error_rate gauge',
      'apcore_usage_error_rate{module_id="sample.module"} 0.5',
      '# HELP apcore_usage_p50_latency_ms Module p50 latency in milliseconds',
      '# TYPE apcore_usage_p50_latency_ms gauge',
      'apcore_usage_p50_latency_ms{module_id="sample.module"} 25',
      '# HELP apcore_usage_p95_latency_ms Module p95 latency in milliseconds',
      '# TYPE apcore_usage_p95_latency_ms gauge',
      'apcore_usage_p95_latency_ms{module_id="sample.module"} 30',
      '# HELP apcore_usage_p99_latency_ms Module p99 latency in milliseconds',
      '# TYPE apcore_usage_p99_latency_ms gauge',
      'apcore_usage_p99_latency_ms{module_id="sample.module"} 30',
      '',
    ].join('\n');
    const rendered = renderPrometheusMetrics(collector, usageCollector);
    expect(rendered).toBe(collector.exportPrometheus() + usage);
  });
});
