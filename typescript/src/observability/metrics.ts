/**
 * In-memory metrics collection with Prometheus export.
 */

import type { Context } from '../context.js';
import { ModuleError } from '../errors.js';
import { Middleware } from '../middleware/base.js';
import { InMemoryObservabilityStore, type ObservabilityStore } from './store.js';
import {
  InMemoryStorageBackend,
  STORAGE_NAMESPACE_METRICS,
  type StorageBackend,
} from './storage.js';

const DESCRIPTIONS: Record<string, string> = {
  apcore_module_calls_total: 'Total module calls',
  apcore_module_errors_total: 'Total module errors',
  apcore_module_duration_seconds: 'Module execution duration',
};

/**
 * Escape the four characters that carry structure in a composite series key:
 * `\\` (the escape itself), `,` (label separator), `=` (key/value separator)
 * and `|` (name/labels/bucket separator).
 *
 * OBS-006: the key used to be built by plain concatenation, which is not
 * injective — `{a: "x,b=y"}` and `{a: "x", b: "y"}` both flattened to
 * `a=x,b=y`, so two distinct series silently aggregated into one map entry,
 * and export parsed the string back into labels that were never passed in.
 * apcore-python keys on a sorted tuple and apcore-rust on a `BTreeMap`; this
 * is the same structural key, spelled for a `Map<string, …>`.
 *
 * A label key or value containing none of the four is left byte-for-byte
 * alone, so the ordinary series key — `apcore_module_calls_total|module_id=
 * executor.email.send,status=error` — keeps its exact spelling, which
 * `metrics-utils.ts` and the snapshot surface both depend on.
 */
function escapeKeyPart(part: string): string {
  return part.replace(/[\\,=|]/g, (c) => `\\${c}`);
}

function labelsKey(labels: Record<string, string>): string {
  return Object.entries(labels)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([k, v]) => `${escapeKeyPart(k)}=${escapeKeyPart(v)}`)
    .join(',');
}

export interface MetricsCollectorOptions {
  buckets?: number[];
  store?: ObservabilityStore;
  /** Pluggable key/value storage backend (Issue #43 §1). Optional. */
  storage?: StorageBackend;
}

export class MetricsCollector {
  static readonly DEFAULT_BUCKETS: number[] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
  ];

  private _buckets: number[];
  private readonly _store: ObservabilityStore;
  private readonly _storage: StorageBackend;
  private _counters: Map<string, number> = new Map();
  private _histogramSums: Map<string, number> = new Map();
  private _histogramCounts: Map<string, number> = new Map();
  private _histogramBuckets: Map<string, number> = new Map();
  /**
   * The label map each composite key was built from, kept verbatim.
   *
   * OBS-006: export used to reconstruct labels by splitting the composite key
   * back apart, which fabricated a label out of any value containing `,` and
   * truncated any value containing `=` (JavaScript's `split('=', 2)` DROPS the
   * remainder rather than limiting the split). Holding the original map means
   * the exposition format carries exactly what the caller passed in, and the
   * composite key is only ever an identity — never a data channel.
   */
  private _labelsByKey: Map<string, Record<string, string>> = new Map();

  constructor(optionsOrBuckets?: MetricsCollectorOptions | number[]) {
    if (Array.isArray(optionsOrBuckets)) {
      this._buckets = [...optionsOrBuckets].sort((a, b) => a - b);
      this._store = new InMemoryObservabilityStore();
      this._storage = new InMemoryStorageBackend();
    } else {
      const buckets = optionsOrBuckets?.buckets;
      this._buckets = buckets ? [...buckets].sort((a, b) => a - b) : [...MetricsCollector.DEFAULT_BUCKETS];
      this._store = optionsOrBuckets?.store ?? new InMemoryObservabilityStore();
      this._storage = optionsOrBuckets?.storage ?? new InMemoryStorageBackend();
    }
  }

  get store(): ObservabilityStore {
    return this._store;
  }

  /** The pluggable storage backend (Issue #43 §1). */
  get storage(): StorageBackend {
    return this._storage;
  }

  get buckets(): readonly number[] {
    return this._buckets;
  }

  increment(name: string, labels: Record<string, string>, amount: number = 1): void {
    const key = `${name}|${labelsKey(labels)}`;
    this._rememberLabels(key, labels);
    this._counters.set(key, (this._counters.get(key) ?? 0) + amount);
  }

  observe(name: string, labels: Record<string, string>, value: number): void {
    const lk = labelsKey(labels);
    const key = `${name}|${lk}`;
    this._rememberLabels(key, labels);

    this._histogramSums.set(key, (this._histogramSums.get(key) ?? 0) + value);
    this._histogramCounts.set(key, (this._histogramCounts.get(key) ?? 0) + 1);

    for (const b of this._buckets) {
      if (value <= b) {
        const bkey = `${name}|${lk}|${b}`;
        this._histogramBuckets.set(bkey, (this._histogramBuckets.get(bkey) ?? 0) + 1);
      }
    }
    // +Inf bucket
    const infKey = `${name}|${lk}|Inf`;
    this._histogramBuckets.set(infKey, (this._histogramBuckets.get(infKey) ?? 0) + 1);

    // D-113: persist under the canonical `metrics` namespace. The backend was
    // constructed here and written to by nothing. Not awaited — `observe` is
    // synchronous and a backend failure must not break the in-memory snapshot,
    // which is what every reader uses.
    void this._storage.save(STORAGE_NAMESPACE_METRICS, key, {
      name,
      labels: { ...labels },
      value,
    });
  }

  snapshot(): Record<string, unknown> {
    return {
      counters: Object.fromEntries(this._counters),
      histograms: {
        sums: Object.fromEntries(this._histogramSums),
        counts: Object.fromEntries(this._histogramCounts),
        buckets: Object.fromEntries(this._histogramBuckets),
      },
    };
  }

  reset(): void {
    this._counters.clear();
    this._histogramSums.clear();
    this._histogramCounts.clear();
    this._histogramBuckets.clear();
    this._labelsByKey.clear();
  }

  /** Record the labels behind a composite key the first time it is seen. */
  private _rememberLabels(key: string, labels: Record<string, string>): void {
    if (!this._labelsByKey.has(key)) {
      this._labelsByKey.set(key, { ...labels });
    }
  }

  exportPrometheus(): string {
    const lines: string[] = [];
    const counterNames = new Set<string>();
    const histNames = new Set<string>();

    // Counters
    for (const [compositeKey, value] of [...this._counters.entries()].sort()) {
      // The metric NAME is the part before the first `|`; label parts escape
      // theirs (OBS-006), so that separator is unambiguous. `split('|', 2)` is
      // avoided because JavaScript's limit argument DROPS the remainder rather
      // than limiting the split — the same trap OBS-006 names for `=`.
      const name = compositeKey.slice(0, compositeKey.indexOf('|'));
      if (!counterNames.has(name)) {
        const desc = DESCRIPTIONS[name] ?? name;
        lines.push(`# HELP ${name} ${desc}`);
        lines.push(`# TYPE ${name} counter`);
        counterNames.add(name);
      }
      const labelsStr = formatLabels(this._labelsByKey.get(compositeKey) ?? {});
      lines.push(`${name}${labelsStr} ${value}`);
    }

    // Histograms
    const histKeys = [...this._histogramSums.keys()].sort();
    for (const compositeKey of histKeys) {
      const name = compositeKey.slice(0, compositeKey.indexOf('|'));
      if (!histNames.has(name)) {
        const desc = DESCRIPTIONS[name] ?? name;
        lines.push(`# HELP ${name} ${desc}`);
        lines.push(`# TYPE ${name} histogram`);
        histNames.add(name);
      }

      const labelsDict = this._labelsByKey.get(compositeKey) ?? {};
      const labelsStr = formatLabels(labelsDict);

      for (const b of this._buckets) {
        const bkey = `${compositeKey}|${b}`;
        const count = this._histogramBuckets.get(bkey) ?? 0;
        const leStr = String(b);
        const leLabels = { ...labelsDict, le: leStr };
        lines.push(`${name}_bucket${formatLabels(leLabels)} ${count}`);
      }

      const infKey = `${compositeKey}|Inf`;
      const infCount = this._histogramBuckets.get(infKey) ?? 0;
      const infLabels = { ...labelsDict, le: '+Inf' };
      lines.push(`${name}_bucket${formatLabels(infLabels)} ${infCount}`);

      const sumVal = this._histogramSums.get(compositeKey) ?? 0;
      const countVal = this._histogramCounts.get(compositeKey) ?? 0;
      lines.push(`${name}_sum${labelsStr} ${sumVal}`);
      lines.push(`${name}_count${labelsStr} ${countVal}`);
    }

    return lines.length > 0 ? lines.join('\n') + '\n' : '';
  }

  incrementCalls(moduleId: string, status: string): void {
    this.increment('apcore_module_calls_total', { module_id: moduleId, status });
  }

  incrementErrors(moduleId: string, errorCode: string): void {
    this.increment('apcore_module_errors_total', { module_id: moduleId, error_code: errorCode });
  }

  observeDuration(moduleId: string, durationSeconds: number): void {
    this.observe('apcore_module_duration_seconds', { module_id: moduleId }, durationSeconds);
  }
}

/**
 * Escape a Prometheus exposition-format label value per
 * https://prometheus.io/docs/instrumenting/exposition_formats/ :
 *   backslash -> \\, double-quote -> \", newline -> \n.
 * Without this, any label value containing `"`, `\` or `\n` silently breaks
 * the exposition format and breaks downstream parsers.
 */
function escapeLabelValue(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n');
}

function formatLabels(labels: Record<string, string>): string {
  const entries = Object.entries(labels);
  if (entries.length === 0) return '';
  const sorted = entries.sort(([a], [b]) => {
    if (a === 'le') return 1;
    if (b === 'le') return -1;
    return a.localeCompare(b);
  });
  const pairs = sorted.map(([k, v]) => `${k}="${escapeLabelValue(v)}"`).join(',');
  return `{${pairs}}`;
}

export class MetricsMiddleware extends Middleware {
  private _collector: MetricsCollector;

  constructor(collector: MetricsCollector) {
    super();
    this._collector = collector;
  }

  override before(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    context: Context,
  ): null {
    const starts = (context.data['_apcore.mw.metrics.starts'] as number[]) ?? [];
    starts.push(performance.now());
    context.data['_apcore.mw.metrics.starts'] = starts;
    return null;
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    context: Context,
  ): null {
    const starts = context.data['_apcore.mw.metrics.starts'] as number[] | undefined;
    if (!starts || starts.length === 0) return null;
    const startTime = starts.pop()!;
    const durationS = (performance.now() - startTime) / 1000;
    this._collector.incrementCalls(moduleId, 'success');
    this._collector.observeDuration(moduleId, durationS);
    return null;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    error: Error,
    context: Context,
  ): null {
    const starts = context.data['_apcore.mw.metrics.starts'] as number[] | undefined;
    if (!starts || starts.length === 0) return null;
    const startTime = starts.pop()!;
    const durationS = (performance.now() - startTime) / 1000;
    const errorCode = error instanceof ModuleError ? error.code : error.constructor.name;
    this._collector.incrementCalls(moduleId, 'error');
    this._collector.incrementErrors(moduleId, errorCode);
    this._collector.observeDuration(moduleId, durationS);
    return null;
  }
}
