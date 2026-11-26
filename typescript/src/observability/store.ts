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
  private readonly _errors = new Map<string | symbol, unknown>();
  private _metrics: MetricPoint[] = [];

  recordError(entry: unknown): void {
    this._errors.set(Symbol(), entry);
  }

  /** @internal Retain the latest snapshot of a collector-owned fingerprint. */
  retainError(fingerprint: string, entry: unknown): void {
    this._errors.set(fingerprint, entry);
  }

  /** @internal Remove the snapshot when the owning collector evicts it. */
  evictError(fingerprint: string): void {
    this._errors.delete(fingerprint);
  }

  getErrors(moduleId?: string | null, limit?: number | null): unknown[] {
    let entries = [...this._errors.values()];
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
    this._errors.clear();
    this._metrics = [];
  }
}
