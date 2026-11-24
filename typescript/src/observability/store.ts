/**
 * Pluggable observability storage backend interface and in-memory default.
 */

export interface MetricPoint {
  readonly name: string;
  readonly value: number;
  readonly moduleId: string | null;
  readonly labels: Record<string, string>;
  readonly timestamp: string;
}

export interface ObservabilityStore {
  recordError(entry: unknown): void;
  getErrors(moduleId?: string | null, limit?: number | null): unknown[];
  recordMetric(metric: MetricPoint): void;
  getMetrics(moduleId?: string | null, metricName?: string | null): MetricPoint[];
  flush(): void;
  clear(): void;
}

export class InMemoryObservabilityStore implements ObservabilityStore {
  private _errors: unknown[] = [];
  private _metrics: MetricPoint[] = [];

  recordError(entry: unknown): void {
    this._errors.push(entry);
  }

  getErrors(moduleId?: string | null, limit?: number | null): unknown[] {
    let entries = [...this._errors];
    if (moduleId != null) {
      entries = entries.filter((e) => (e as Record<string, unknown>)['moduleId'] === moduleId);
    }
    return limit != null ? entries.slice(0, limit) : entries;
  }

  recordMetric(metric: MetricPoint): void {
    this._metrics.push(metric);
  }

  getMetrics(moduleId?: string | null, metricName?: string | null): MetricPoint[] {
    let metrics = [...this._metrics];
    if (moduleId != null) {
      metrics = metrics.filter((m) => m.moduleId === moduleId);
    }
    if (metricName != null) {
      metrics = metrics.filter((m) => m.name === metricName);
    }
    return metrics;
  }

  flush(): void {}

  clear(): void {
    this._errors = [];
    this._metrics = [];
  }
}
