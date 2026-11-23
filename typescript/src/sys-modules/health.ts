/**
 * System health modules -- summary and single-module health.
 */

import type { Registry } from '../registry/registry.js';
import type { MetricsCollector } from '../observability/metrics.js';
import { computeModuleErrorRate, estimateP99FromHistogram } from '../observability/metrics-utils.js';
import type { ErrorHistory } from '../observability/error-history.js';
import type { Config } from '../config.js';
import { InvalidInputError, ModuleNotFoundError } from '../errors.js';

const DEFAULT_HEALTHY_THRESHOLD = 0.01;
const DEFAULT_DEGRADED_THRESHOLD = 0.10;

export function classifyHealthStatus(
  errorRate: number,
  totalCalls: number,
  healthyThreshold: number = DEFAULT_HEALTHY_THRESHOLD,
  degradedThreshold: number = DEFAULT_DEGRADED_THRESHOLD,
): string {
  if (totalCalls === 0) return 'unknown';
  if (errorRate < healthyThreshold) return 'healthy';
  if (errorRate < degradedThreshold) return 'degraded';
  return 'error';
}

/** @internal */
export class HealthSummaryModule {
  readonly description = 'Aggregated health overview of all registered modules';
  readonly annotations = { readonly: true, destructive: false, idempotent: true, requiresApproval: false, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      error_rate_threshold: { type: 'number' as const, description: 'Error rate threshold for healthy status', default: 0.01 },
      include_healthy: { type: 'boolean' as const, description: 'Whether to include healthy modules in output', default: true },
    },
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      project: { type: 'object' as const, description: 'Project information' },
      summary: { type: 'object' as const, description: 'Aggregated health counts by status' },
      modules: { type: 'array' as const, description: 'Per-module health entries' },
    },
    required: ['project', 'summary', 'modules'],
  };

  private readonly _registry: Registry;
  private readonly _metrics: MetricsCollector | null;
  private readonly _errorHistory: ErrorHistory;
  private readonly _config: Config | null;

  constructor(
    registry: Registry,
    metrics: MetricsCollector | null,
    errorHistory: ErrorHistory,
    config: Config | null = null,
  ) {
    this._registry = registry;
    this._metrics = metrics;
    this._errorHistory = errorHistory;
    this._config = config;
  }

  execute(inputs: Record<string, unknown>, _context: unknown): Record<string, unknown> {
    const healthyThreshold = Number(inputs['error_rate_threshold'] ?? DEFAULT_HEALTHY_THRESHOLD);
    // D-109: `error_rate_threshold` moves the FIRST boundary only. The
    // degraded/error boundary is the classification table's fixed 0.10.
    //
    // This used to be `healthyThreshold * 10`, which silently gave the caller a
    // second knob the table does not define: with `error_rate_threshold: 0.001`
    // a module erroring 5% of the time was classified `error` here and
    // `degraded` by apcore-rust, which reads the table. The consequence is
    // recorded in the contract because it surprises — at 0.001, `degraded`
    // spans 0.1%-10% — and a caller wanting a stricter ERROR boundary is asking
    // for a knob that deliberately does not exist.
    const degradedThreshold = DEFAULT_DEGRADED_THRESHOLD;
    const includeHealthy = inputs['include_healthy'] !== false && inputs['include_healthy'] !== 'false';

    const moduleIds = this._registry.list();
    const counts = { healthy: 0, degraded: 0, error: 0, unknown: 0 };
    const modules: Record<string, unknown>[] = [];

    for (const mid of moduleIds) {
      const { totalCalls, errorRate } = this._getModuleMetrics(mid);
      const status = classifyHealthStatus(errorRate, totalCalls, healthyThreshold, degradedThreshold);
      counts[status as keyof typeof counts]++;
      if (!includeHealthy && status === 'healthy') continue;

      const topError = this._getTopError(mid);
      modules.push({ module_id: mid, status, error_rate: errorRate, top_error: topError });
    }

    const projectName = this._config?.get('project.name', 'apcore') ?? 'apcore';
    return {
      project: { name: projectName },
      summary: { total_modules: moduleIds.length, ...counts },
      modules,
    };
  }

  private _getModuleMetrics(moduleId: string): { totalCalls: number; errorRate: number } {
    if (!this._metrics) return { totalCalls: 0, errorRate: 0 };
    const { totalCalls, errorRate } = computeModuleErrorRate(this._metrics, moduleId);
    return { totalCalls, errorRate };
  }

  private _getTopError(moduleId: string): Record<string, unknown> | null {
    const entries = this._errorHistory.get(moduleId);
    if (entries.length === 0) return null;
    const top = entries.reduce((a, b) => (a.count >= b.count ? a : b));
    return { code: top.code, message: top.message, ai_guidance: top.aiGuidance, count: top.count };
  }
}

/** @internal */
export class HealthModule {
  readonly description = 'Detailed health information for a single module';
  readonly annotations = { readonly: true, destructive: false, idempotent: true, requiresApproval: false, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      module_id: { type: 'string' as const, description: 'ID of the module to inspect' },
      error_limit: { type: 'integer' as const, description: 'Max number of recent errors to return', default: 10 },
    },
    required: ['module_id'],
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      module_id: { type: 'string' as const, description: 'Module identifier' },
      status: { type: 'string' as const, description: 'Health status: healthy, degraded, error, or unknown' },
      total_calls: { type: 'integer' as const, description: 'Total number of calls' },
      error_count: { type: 'integer' as const, description: 'Total number of errors' },
      error_rate: { type: 'number' as const, description: 'Error rate as a float (0.0-1.0)' },
      avg_latency_ms: { type: 'number' as const, description: 'Average latency in milliseconds' },
      p99_latency_ms: { type: 'number' as const, description: '99th percentile latency in milliseconds' },
      recent_errors: { type: 'array' as const, description: 'Recent error entries' },
    },
    required: ['module_id', 'status', 'total_calls', 'error_count', 'error_rate', 'avg_latency_ms', 'p99_latency_ms', 'recent_errors'],
  };

  private readonly _registry: Registry;
  private readonly _metrics: MetricsCollector | null;
  private readonly _errorHistory: ErrorHistory;

  constructor(
    registry: Registry,
    metrics: MetricsCollector | null,
    errorHistory: ErrorHistory,
  ) {
    this._registry = registry;
    this._metrics = metrics;
    this._errorHistory = errorHistory;
  }

  private _getModuleMetrics(moduleId: string): {
    totalCalls: number; errorCount: number; errorRate: number;
    avgLatencyMs: number; p99LatencyMs: number;
  } {
    if (!this._metrics) return { totalCalls: 0, errorCount: 0, errorRate: 0, avgLatencyMs: 0, p99LatencyMs: 0 };
    const { totalCalls, errorCount, errorRate } = computeModuleErrorRate(this._metrics, moduleId);
    const { avgLatencyMs, p99LatencyMs } = estimateP99FromHistogram(this._metrics, moduleId);
    return { totalCalls, errorCount, errorRate, avgLatencyMs, p99LatencyMs };
  }

  execute(inputs: Record<string, unknown>, _context: unknown): Record<string, unknown> {
    const moduleId = inputs['module_id'];
    if (typeof moduleId !== 'string' || !moduleId) {
      throw new InvalidInputError('module_id is required');
    }
    if (!this._registry.has(moduleId)) {
      throw new ModuleNotFoundError(moduleId);
    }

    const errorLimit = Number(inputs['error_limit'] ?? 10);
    const recentErrors = this._errorHistory.get(moduleId, errorLimit).map((e) => ({
      code: e.code,
      message: e.message,
      ai_guidance: e.aiGuidance,
      count: e.count,
      first_occurred: e.firstOccurred,
      last_occurred: e.lastOccurred,
    }));

    const { totalCalls, errorCount, errorRate, avgLatencyMs, p99LatencyMs } = this._getModuleMetrics(moduleId);
    return {
      module_id: moduleId,
      status: classifyHealthStatus(errorRate, totalCalls),
      total_calls: totalCalls,
      error_count: errorCount,
      error_rate: errorRate,
      avg_latency_ms: avgLatencyMs,
      p99_latency_ms: p99LatencyMs,
      recent_errors: recentErrors,
    };
  }
}
