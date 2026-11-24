/**
 * System usage modules -- summary and single-module usage detail.
 */

import { InvalidInputError, ModuleNotFoundError } from '../errors.js';
import type { Registry } from '../registry/registry.js';
import type { UsageCollector } from '../observability/usage.js';
import { bucketKey } from '../observability/usage.js';
import type { HourlyBucket } from '../observability/usage.js';

function computeP99(latencies: number[]): number {
  if (latencies.length === 0) return 0;
  const sorted = [...latencies].sort((a, b) => a - b);
  const idx = Math.max(0, Math.ceil(0.99 * sorted.length) - 1);
  return sorted[idx];
}

function padHourlyDistribution(buckets: HourlyBucket[]): Record<string, unknown>[] {
  const now = new Date();
  const existing = new Map<string, HourlyBucket>();
  for (const b of buckets) existing.set(b.hour, b);

  const keys: string[] = [];
  for (let i = 0; i < 24; i++) {
    const d = new Date(now.getTime() - (23 - i) * 3600_000);
    keys.push(bucketKey(d));
  }
  for (const k of existing.keys()) {
    if (!keys.includes(k)) keys.push(k);
  }
  keys.sort();
  const latest = keys.slice(-24);

  return latest.map((k) => {
    const b = existing.get(k);
    return b
      ? { hour: k, call_count: b.callCount, error_count: b.errorCount }
      : { hour: k, call_count: 0, error_count: 0 };
  });
}

/** @internal */
export class UsageSummaryModule {
  readonly description = 'All modules usage overview with trend detection';
  readonly annotations = { readonly: true, destructive: false, idempotent: true, requiresApproval: false, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      period: {
        type: 'string' as const,
        description:
          "Time window: a positive integer followed by 'h' (hours) or 'd' (days), " +
          'e.g. 1h, 24h, 7d. Every statistic in the output is computed over ' +
          '[now - period, now].',
        default: '24h',
        // PROTOCOL_SPEC 6.7.1.1: declared here so a malformed value is rejected
        // by input validation with SCHEMA_VALIDATION_ERROR, uniformly across
        // SDKs, rather than by parsePeriod throwing a plain Error.
        pattern: '^[1-9][0-9]*[hd]$',
      },
    },
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      period: { type: 'string' as const, description: 'Requested time period' },
      total_calls: { type: 'integer' as const, description: 'Total calls across all modules' },
      total_errors: { type: 'integer' as const, description: 'Total errors across all modules' },
      modules: { type: 'array' as const, description: 'Per-module usage entries' },
    },
    required: ['period', 'total_calls', 'total_errors', 'modules'],
  };

  private readonly _collector: UsageCollector;

  constructor(collector: UsageCollector) {
    this._collector = collector;
  }

  execute(inputs: Record<string, unknown>, _context: unknown): Record<string, unknown> {
    const period = (inputs['period'] as string) ?? '24h';
    const summaries = this._collector.getSummary(period);
    const sorted = [...summaries].sort((a, b) => b.callCount - a.callCount);

    const totalCalls = sorted.reduce((s, e) => s + e.callCount, 0);
    const totalErrors = sorted.reduce((s, e) => s + e.errorCount, 0);

    return {
      period,
      total_calls: totalCalls,
      total_errors: totalErrors,
      modules: sorted.map((e) => ({
        module_id: e.moduleId,
        call_count: e.callCount,
        error_count: e.errorCount,
        avg_latency_ms: e.avgLatencyMs,
        unique_callers: e.uniqueCallers,
        trend: e.trend,
      })),
    };
  }
}

/** @internal */
export class UsageModule {
  readonly description = 'Detailed usage statistics for a single module';
  readonly annotations = { readonly: true, destructive: false, idempotent: true, requiresApproval: false, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      module_id: { type: 'string' as const, description: 'ID of the module to inspect' },
      period: {
        type: 'string' as const,
        description:
          "Time window: a positive integer followed by 'h' (hours) or 'd' (days), " +
          'e.g. 1h, 24h, 7d. Every statistic in the output is computed over ' +
          '[now - period, now].',
        default: '24h',
        // PROTOCOL_SPEC 6.7.1.1: declared here so a malformed value is rejected
        // by input validation with SCHEMA_VALIDATION_ERROR, uniformly across
        // SDKs, rather than by parsePeriod throwing a plain Error.
        pattern: '^[1-9][0-9]*[hd]$',
      },
    },
    required: ['module_id'],
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      module_id: { type: 'string' as const, description: 'Module identifier' },
      period: { type: 'string' as const, description: 'Requested time period' },
      call_count: { type: 'integer' as const, description: 'Total calls in the period' },
      error_count: { type: 'integer' as const, description: 'Total errors in the period' },
      avg_latency_ms: { type: 'number' as const, description: 'Average latency in milliseconds' },
      p99_latency_ms: { type: 'number' as const, description: '99th percentile latency in milliseconds' },
      trend: { type: 'string' as const, description: 'Usage trend indicator' },
      callers: { type: 'array' as const, description: 'Per-caller usage breakdown' },
      hourly_distribution: { type: 'array' as const, description: 'Hourly call/error distribution' },
    },
    required: ['module_id', 'period', 'call_count', 'error_count', 'avg_latency_ms', 'p99_latency_ms', 'trend', 'callers', 'hourly_distribution'],
  };

  private readonly _registry: Registry;
  private readonly _collector: UsageCollector;

  constructor(registry: Registry, usageCollector: UsageCollector) {
    this._registry = registry;
    this._collector = usageCollector;
  }

  execute(inputs: Record<string, unknown>, _context: unknown): Record<string, unknown> {
    const moduleId = inputs['module_id'];
    if (typeof moduleId !== 'string' || !moduleId) {
      throw new InvalidInputError('module_id is required');
    }
    if (!this._registry.has(moduleId)) {
      throw new ModuleNotFoundError(moduleId);
    }

    const period = (inputs['period'] as string) ?? '24h';
    const detail = this._collector.getModule(moduleId, period);
    const latencies = this._collector.getLatencies(moduleId, period);
    const p99 = computeP99(latencies);

    const callers = detail.callers.map((c) => ({
      caller_id: c.callerId,
      call_count: c.callCount,
      error_count: c.errorCount,
      avg_latency_ms: c.avgLatencyMs,
    }));

    const hourly = padHourlyDistribution(detail.hourlyDistribution);

    return {
      module_id: detail.moduleId,
      period,
      call_count: detail.callCount,
      error_count: detail.errorCount,
      avg_latency_ms: detail.avgLatencyMs,
      p99_latency_ms: p99,
      trend: detail.trend,
      callers,
      hourly_distribution: hourly,
    };
  }
}
