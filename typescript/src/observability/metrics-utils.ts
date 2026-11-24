/**
 * Shared utilities for extracting module-level metrics from MetricsCollector snapshots.
 */

import type { MetricsCollector } from './metrics.js';

export const METRIC_CALLS_TOTAL = 'apcore_module_calls_total';
export const METRIC_DURATION_SECONDS = 'apcore_module_duration_seconds';


/**
 * Check if a metric key's labels contain an exact module_id match.
 * Keys have the format: "metric_name|module_id=foo,status=bar"
 * This prevents false-matching when one module ID is a suffix of another.
 */
export function matchesModuleId(key: string, moduleId: string): boolean {
  const labelMatch = `|module_id=${moduleId}`;
  return key.includes(`${labelMatch},`) || key.endsWith(labelMatch);
}

/**
 * Compute error rate for a module from a MetricsCollector snapshot.
 */
export function computeModuleErrorRate(
  metricsCollector: MetricsCollector,
  moduleId: string,
): { totalCalls: number; errorCount: number; errorRate: number } {
  const snap = metricsCollector.snapshot();
  const counters = snap['counters'] as Record<string, number> | undefined;
  if (!counters) return { totalCalls: 0, errorCount: 0, errorRate: 0 };

  let total = 0;
  let errors = 0;
  const prefix = `${METRIC_CALLS_TOTAL}|`;
  for (const [key, value] of Object.entries(counters)) {
    if (!key.startsWith(prefix)) continue;
    if (!matchesModuleId(key, moduleId)) continue;
    total += value;
    if (key.includes('status=error')) {
      errors += value;
    }
  }
  return { totalCalls: total, errorCount: errors, errorRate: total === 0 ? 0 : errors / total };
}

/**
 * Estimate p99 latency in milliseconds from histogram buckets.
 * Returns the upper bound of the first bucket that contains ≥99% of observations.
 * If all observations exceed the largest bucket, returns the last bucket upper bound.
 */
export function estimateP99FromHistogram(
  metricsCollector: MetricsCollector,
  moduleId: string,
): { avgLatencyMs: number; p99LatencyMs: number } {
  const snap = metricsCollector.snapshot();
  const histograms = snap['histograms'] as {
    sums?: Record<string, number>;
    counts?: Record<string, number>;
    buckets?: Record<string, number>;
  } | undefined;
  if (!histograms) return { avgLatencyMs: 0, p99LatencyMs: 0 };

  const durationKey = `${METRIC_DURATION_SECONDS}|module_id=${moduleId}`;
  const sumVal = histograms.sums?.[durationKey] ?? 0;
  const countVal = histograms.counts?.[durationKey] ?? 0;
  const avgLatencyMs = countVal > 0 ? (sumVal / countVal) * 1000 : 0;

  let p99LatencyMs = 0;
  if (countVal > 0 && histograms.buckets) {
    const buckets = metricsCollector.buckets;
    const target = countVal * 0.99;
    for (const b of buckets) {
      const bkey = `${durationKey}|${b}`;
      const cumCount = histograms.buckets[bkey] ?? 0;
      if (cumCount >= target) {
        p99LatencyMs = b * 1000;
        return { avgLatencyMs, p99LatencyMs };
      }
    }
    // All observations exceed the largest bucket
    p99LatencyMs = (buckets[buckets.length - 1] ?? 0) * 1000;
  }

  return { avgLatencyMs, p99LatencyMs };
}
