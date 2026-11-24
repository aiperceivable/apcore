/**
 * Time-windowed usage tracking with per-module and per-caller analytics.
 */

import { Middleware } from '../middleware/base.js';
import type { Context } from '../context.js';
import {
  InMemoryStorageBackend,
  STORAGE_NAMESPACE_USAGE,
  type StorageBackend,
} from './storage.js';

export interface UsageRecord {
  readonly timestamp: string;
  readonly callerId: string;
  readonly latencyMs: number;
  readonly success: boolean;
}

export interface CallerUsageSummary {
  readonly callerId: string;
  readonly callCount: number;
  readonly errorCount: number;
  readonly avgLatencyMs: number;
}

export interface HourlyBucket {
  readonly hour: string;
  readonly callCount: number;
  readonly errorCount: number;
}

export interface ModuleUsageSummary {
  readonly moduleId: string;
  readonly callCount: number;
  readonly errorCount: number;
  readonly avgLatencyMs: number;
  readonly uniqueCallers: number;
  readonly trend: string;
}

export interface ModuleUsageDetail extends ModuleUsageSummary {
  readonly callers: CallerUsageSummary[];
  readonly hourlyDistribution: HourlyBucket[];
}

function parsePeriod(period: string): number {
  const match = period.match(/^(\d+)([hd])$/);
  if (!match) throw new Error(`Invalid period format: ${period}`);
  const n = parseInt(match[1], 10);
  if (n <= 0) throw new Error(`Invalid period format: ${period}`);
  return match[2] === 'h' ? n * 3600_000 : n * 86400_000;
}

export function bucketKey(date: Date): string {
  const y = date.getUTCFullYear();
  const m = String(date.getUTCMonth() + 1).padStart(2, '0');
  const d = String(date.getUTCDate()).padStart(2, '0');
  const h = String(date.getUTCHours()).padStart(2, '0');
  return `${y}-${m}-${d}T${h}`;
}

function computeTrend(currentCount: number, previousCount: number): string {
  if (currentCount === 0 && previousCount === 0) return 'stable';
  if (currentCount === 0) return 'inactive';
  if (previousCount === 0) return 'new';
  const ratio = currentCount / previousCount;
  if (ratio > 1.2) return 'rising';
  if (ratio < 0.8) return 'declining';
  return 'stable';
}

export interface UsageCollectorOptions {
  retentionHours?: number;
  maxRecordsPerBucket?: number;
  /** Pluggable key/value storage backend (Issue #43 §1). Optional. */
  storage?: StorageBackend;
}

/**
 * In-memory usage tracker with hourly buckets and configurable retention.
 */
export class UsageCollector {
  readonly retentionHours: number;
  private readonly _maxRecordsPerBucket: number;
  private readonly _storage: StorageBackend;
  // moduleId -> bucketKey -> UsageRecord[]
  private readonly _data: Map<string, Map<string, UsageRecord[]>> = new Map();

  /**
   * Construct a `UsageCollector`.
   *
   * Two construction shapes are supported for backwards compatibility:
   * - Positional: `new UsageCollector(retentionHours, maxRecordsPerBucket)`.
   * - Options:    `new UsageCollector({ retentionHours, maxRecordsPerBucket, storage })`.
   */
  constructor(
    optionsOrRetentionHours?: UsageCollectorOptions | number,
    maxRecordsPerBucket: number = 10000,
  ) {
    if (typeof optionsOrRetentionHours === 'object' && optionsOrRetentionHours !== null) {
      const options = optionsOrRetentionHours;
      this.retentionHours = options.retentionHours ?? 168;
      this._maxRecordsPerBucket = options.maxRecordsPerBucket ?? 10000;
      this._storage = options.storage ?? new InMemoryStorageBackend();
    } else {
      this.retentionHours = optionsOrRetentionHours ?? 168;
      this._maxRecordsPerBucket = maxRecordsPerBucket;
      this._storage = new InMemoryStorageBackend();
    }
  }

  /** The pluggable storage backend (Issue #43 §1). */
  get storage(): StorageBackend {
    return this._storage;
  }

  record(
    moduleId: string,
    callerId: string,
    latencyMs: number,
    success: boolean,
    timestamp?: string,
  ): void {
    const now = timestamp ? new Date(timestamp) : new Date();
    const ts = timestamp ?? now.toISOString();
    const bk = bucketKey(now);
    const rec: UsageRecord = { timestamp: ts, callerId, latencyMs, success };

    let mod = this._data.get(moduleId);
    if (!mod) {
      mod = new Map();
      this._data.set(moduleId, mod);
    }
    let bucket = mod.get(bk);
    if (!bucket) {
      bucket = [];
      mod.set(bk, bucket);
    }
    if (bucket.length < this._maxRecordsPerBucket) {
      bucket.push(rec);
    } else {
      console.warn(`[apcore:usage] Record buffer full for module '${moduleId}' bucket '${bk}' (max ${this._maxRecordsPerBucket}) — record dropped`);
    }
    this._cleanupExpired(moduleId);

    // D-113: persist under the canonical `usage` namespace. The backend was
    // constructed here and written to by nothing. Not awaited — `record` is
    // synchronous and a backend failure must not break the in-memory
    // aggregation, which is what `getSummary` reads.
    void this._storage.save(STORAGE_NAMESPACE_USAGE, `${moduleId}:${ts}`, {
      module_id: moduleId,
      timestamp: ts,
      caller_id: callerId,
      latency_ms: latencyMs,
      success,
    });
  }

  getSummary(period: string = '24h'): ModuleUsageSummary[] {
    const deltaMs = parsePeriod(period);
    const now = new Date();
    const cutoff = new Date(now.getTime() - deltaMs);
    const prevCutoff = new Date(cutoff.getTime() - deltaMs);
    const result: ModuleUsageSummary[] = [];
    for (const moduleId of this._data.keys()) {
      result.push(this._buildSummary(moduleId, cutoff, prevCutoff, now));
    }
    return result;
  }

  getModule(moduleId: string, period: string = '24h'): ModuleUsageDetail {
    const deltaMs = parsePeriod(period);
    const now = new Date();
    const cutoff = new Date(now.getTime() - deltaMs);
    const prevCutoff = new Date(cutoff.getTime() - deltaMs);
    return this._buildDetail(moduleId, cutoff, prevCutoff, now);
  }

  getLatencies(moduleId: string, period: string = '24h'): number[] {
    const deltaMs = parsePeriod(period);
    const now = new Date();
    const cutoff = new Date(now.getTime() - deltaMs);
    const records = this._collectRecords(moduleId, cutoff, now);
    return records.map((r) => r.latencyMs);
  }

  private _collectRecords(moduleId: string, start: Date, end: Date): UsageRecord[] {
    const mod = this._data.get(moduleId);
    if (!mod) return [];

    // Bucket keys are ISO hour strings (e.g. "2026-04-18T10") — lexicographic
    // order matches chronological order, so we can skip buckets outside range.
    const startKey = bucketKey(start);
    const endKey = bucketKey(end);

    const records: UsageRecord[] = [];
    for (const [bk, recs] of mod.entries()) {
      if (bk < startKey || bk > endKey) continue;
      for (const r of recs) {
        const ts = new Date(r.timestamp);
        if (ts >= start && ts <= end) {
          records.push(r);
        }
      }
    }
    return records;
  }

  private _buildSummary(
    moduleId: string,
    cutoff: Date,
    prevCutoff: Date,
    now: Date,
  ): ModuleUsageSummary {
    const current = this._collectRecords(moduleId, cutoff, now);
    const previous = this._collectRecords(moduleId, prevCutoff, cutoff);
    const callCount = current.length;
    const errorCount = current.filter((r) => !r.success).length;
    const avgLatencyMs = callCount > 0
      ? current.reduce((sum, r) => sum + r.latencyMs, 0) / callCount
      : 0;
    const callers = new Set(current.map((r) => r.callerId));
    const trend = computeTrend(callCount, previous.length);
    return { moduleId, callCount, errorCount, avgLatencyMs, uniqueCallers: callers.size, trend };
  }

  private _buildDetail(
    moduleId: string,
    cutoff: Date,
    prevCutoff: Date,
    now: Date,
  ): ModuleUsageDetail {
    const summary = this._buildSummary(moduleId, cutoff, prevCutoff, now);
    const current = this._collectRecords(moduleId, cutoff, now);
    const callers = this._perCallerBreakdown(current);
    const hourlyDistribution = this._hourlyDistribution(current);
    return { ...summary, callers, hourlyDistribution };
  }

  private _perCallerBreakdown(records: UsageRecord[]): CallerUsageSummary[] {
    const byCaller = new Map<string, UsageRecord[]>();
    for (const r of records) {
      let arr = byCaller.get(r.callerId);
      if (!arr) {
        arr = [];
        byCaller.set(r.callerId, arr);
      }
      arr.push(r);
    }
    const result: CallerUsageSummary[] = [];
    for (const [cid, recs] of [...byCaller.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
      const cc = recs.length;
      const ec = recs.filter((r) => !r.success).length;
      const avg = recs.reduce((s, r) => s + r.latencyMs, 0) / cc;
      result.push({ callerId: cid, callCount: cc, errorCount: ec, avgLatencyMs: avg });
    }
    return result;
  }

  private _hourlyDistribution(records: UsageRecord[]): HourlyBucket[] {
    const byHour = new Map<string, UsageRecord[]>();
    for (const r of records) {
      const hk = bucketKey(new Date(r.timestamp));
      let arr = byHour.get(hk);
      if (!arr) {
        arr = [];
        byHour.set(hk, arr);
      }
      arr.push(r);
    }
    const result: HourlyBucket[] = [];
    for (const key of [...byHour.keys()].sort()) {
      const recs = byHour.get(key)!;
      result.push({
        hour: key,
        callCount: recs.length,
        errorCount: recs.filter((r) => !r.success).length,
      });
    }
    return result;
  }

  private _cleanupExpired(moduleId: string): void {
    const cutoff = new Date(Date.now() - this.retentionHours * 3600_000);
    const cutoffKey = bucketKey(cutoff);
    const mod = this._data.get(moduleId);
    if (!mod) return;
    for (const bk of [...mod.keys()]) {
      if (bk < cutoffKey) {
        mod.delete(bk);
      }
    }
  }
}

const CTX_USAGE_STARTS = '_apcore.mw.usage.starts';

/**
 * Middleware that records module call usage via UsageCollector.
 */
export class UsageMiddleware extends Middleware {
  private readonly _collector: UsageCollector;

  constructor(collector: UsageCollector) {
    super();
    this._collector = collector;
  }

  override before(
    _moduleId: string,
    _inputs: Record<string, unknown>,
    context: Context,
  ): Record<string, unknown> | null {
    const starts = (context.data[CTX_USAGE_STARTS] as number[] | undefined) ?? [];
    starts.push(Date.now());
    context.data[CTX_USAGE_STARTS] = starts;
    return null;
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    context: Context,
  ): Record<string, unknown> | null {
    const latencyMs = this._popElapsedMs(context);
    const callerId = context.callerId ?? 'unknown';
    this._collector.record(moduleId, callerId, latencyMs, true);
    return null;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _error: Error,
    context: Context,
  ): Record<string, unknown> | null {
    const latencyMs = this._popElapsedMs(context);
    const callerId = context.callerId ?? 'unknown';
    this._collector.record(moduleId, callerId, latencyMs, false);
    return null;
  }

  private _popElapsedMs(context: Context): number {
    const starts = context.data[CTX_USAGE_STARTS] as number[] | undefined;
    if (!starts || starts.length === 0) return 0;
    const startTime = starts.pop()!;
    return Date.now() - startTime;
  }
}
