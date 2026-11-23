/**
 * Error history with SHA-256 fingerprinting, min-heap O(log N) eviction,
 * and pluggable observability storage.
 */

import { createHash } from 'node:crypto';
import { ModuleError } from '../errors.js';
import { InMemoryObservabilityStore, type ObservabilityStore } from './store.js';
import {
  InMemoryStorageBackend,
  STORAGE_NAMESPACE_ERROR_HISTORY,
  type StorageBackend,
} from './storage.js';

export interface ErrorEntry {
  readonly moduleId: string;
  readonly code: string;
  readonly message: string;
  readonly aiGuidance: string | null;
  readonly timestamp: string;
  readonly fingerprint: string;
  count: number;
  firstOccurred: string;
  lastOccurred: string;
}

// ---------------------------------------------------------------------------
// Message normalization and fingerprint computation
// ---------------------------------------------------------------------------

/** Replace ephemeral values with placeholders before fingerprint hashing. */
export function normalizeMessage(msg: string): string {
  // Step 1: UUID patterns (8-4-4-4-12 hex)
  msg = msg.replace(
    /[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/g,
    '<UUID>',
  );
  // Step 2: ISO 8601 timestamps (before integers to protect 4-digit years)
  msg = msg.replace(
    /\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?/g,
    '<TIMESTAMP>',
  );
  // Step 3: integers >= 4 digits
  msg = msg.replace(/\b\d{4,}\b/g, '<ID>');
  return msg.trim().toLowerCase();
}

/** Compute SHA-256(error_code:module_id:normalized_message) as 64-char hex. */
export function computeFingerprint(errorCode: string, moduleId: string, message: string): string {
  const normalized = normalizeMessage(message);
  const raw = `${errorCode}:${moduleId}:${normalized}`;
  return createHash('sha256').update(raw, 'utf8').digest('hex');
}

// ---------------------------------------------------------------------------
// Min-heap keyed on (timestamp, seq)
// ---------------------------------------------------------------------------

type HeapItem = [string, number, ErrorEntry];

function heapCompare(a: HeapItem, b: HeapItem): number {
  if (a[0] < b[0]) return -1;
  if (a[0] > b[0]) return 1;
  return a[1] - b[1];
}

class MinHeap {
  private _data: HeapItem[] = [];

  get size(): number {
    return this._data.length;
  }

  push(item: HeapItem): void {
    this._data.push(item);
    this._siftUp(this._data.length - 1);
  }

  pop(): HeapItem | undefined {
    if (this._data.length === 0) return undefined;
    const top = this._data[0];
    const last = this._data.pop()!;
    if (this._data.length > 0) {
      this._data[0] = last;
      this._siftDown(0);
    }
    return top;
  }

  private _siftUp(idx: number): void {
    while (idx > 0) {
      const parent = Math.floor((idx - 1) / 2);
      if (heapCompare(this._data[idx], this._data[parent]) < 0) {
        [this._data[idx], this._data[parent]] = [this._data[parent], this._data[idx]];
        idx = parent;
      } else break;
    }
  }

  private _siftDown(idx: number): void {
    const n = this._data.length;
    while (true) {
      let smallest = idx;
      const left = 2 * idx + 1;
      const right = 2 * idx + 2;
      if (left < n && heapCompare(this._data[left], this._data[smallest]) < 0) {
        smallest = left;
      }
      if (right < n && heapCompare(this._data[right], this._data[smallest]) < 0) {
        smallest = right;
      }
      if (smallest === idx) break;
      [this._data[idx], this._data[smallest]] = [this._data[smallest], this._data[idx]];
      idx = smallest;
    }
  }
}

// ---------------------------------------------------------------------------
// ErrorHistory
// ---------------------------------------------------------------------------

export interface ErrorHistoryOptions {
  maxEntriesPerModule?: number;
  maxTotalEntries?: number;
  store?: ObservabilityStore;
  /** Pluggable key/value storage backend (Issue #43 §1). Optional. */
  storage?: StorageBackend;
}

/**
 * Thread-safe error tracker with min-heap O(log N) eviction and SHA-256 deduplication.
 *
 * Data structures:
 *   _fpIndex: fingerprint → ErrorEntry          O(1) dedup lookup
 *   _moduleIndex: module_id → ErrorEntry[]      O(1) module lookup
 *   _heap: min-heap keyed on (lastOccurred, seq) O(log N) eviction of oldest
 *
 * Lazy deletion: stale heap items (from dedup timestamp refreshes) are skipped on pop.
 */
export class ErrorHistory {
  private readonly _maxEntriesPerModule: number;
  private readonly _maxTotalEntries: number;
  private readonly _store: ObservabilityStore;
  private readonly _storage: StorageBackend;
  private readonly _fpIndex: Map<string, ErrorEntry> = new Map();
  private readonly _moduleIndex: Map<string, ErrorEntry[]> = new Map();
  private readonly _heap: MinHeap = new MinHeap();
  private _seq = 0;

  constructor(options: ErrorHistoryOptions = {}) {
    this._maxEntriesPerModule = options.maxEntriesPerModule ?? 50;
    this._maxTotalEntries = options.maxTotalEntries ?? 1000;
    this._store = options.store ?? new InMemoryObservabilityStore();
    this._storage = options.storage ?? new InMemoryStorageBackend();
  }

  get store(): ObservabilityStore {
    return this._store;
  }

  /** The pluggable storage backend (Issue #43 §1). */
  get storage(): StorageBackend {
    return this._storage;
  }

  record(moduleId: string, error: ModuleError): void {
    const now = new Date().toISOString();
    const fp = computeFingerprint(error.code, moduleId, error.message);

    const existing = this._fpIndex.get(fp);
    if (existing !== undefined) {
      existing.count++;
      existing.lastOccurred = now;
      this._seq++;
      this._heap.push([now, this._seq, existing]);
      this._store.recordError(existing);
      return;
    }

    const entry: ErrorEntry = {
      moduleId,
      code: error.code,
      message: error.message,
      aiGuidance: error.aiGuidance,
      timestamp: now,
      fingerprint: fp,
      count: 1,
      firstOccurred: now,
      lastOccurred: now,
    };

    this._fpIndex.set(fp, entry);
    const moduleEntries = this._moduleIndex.get(moduleId);
    if (moduleEntries !== undefined) {
      moduleEntries.push(entry);
    } else {
      this._moduleIndex.set(moduleId, [entry]);
    }

    this._seq++;
    this._heap.push([now, this._seq, entry]);
    this._evictModule(moduleId);
    this._evictTotal();
    this._store.recordError(entry);

    // D-113: persist under the canonical `error_history` namespace. The
    // `storage` option was accepted here and read by nothing — one of the five
    // collector/SDK combinations of nine where the MUST reached no mechanism.
    // `save` is async and `record` is not, so the result is deliberately not
    // awaited; a backend failure must not break the in-memory path, which is
    // the authoritative one for `get`/`getAll`.
    void this._storage.save(STORAGE_NAMESPACE_ERROR_HISTORY, fp, {
      module_id: entry.moduleId,
      code: entry.code,
      message: entry.message,
      timestamp: entry.timestamp,
      count: entry.count,
      first_occurred: entry.firstOccurred,
      last_occurred: entry.lastOccurred,
      fingerprint: entry.fingerprint,
    });
  }

  get(moduleId: string, limit?: number): ErrorEntry[] {
    const moduleEntries = this._moduleIndex.get(moduleId) ?? [];
    const result = [...moduleEntries].reverse();
    return limit !== undefined ? result.slice(0, limit) : result;
  }

  getAll(limit?: number): ErrorEntry[] {
    const all = [...this._fpIndex.values()];
    all.sort((a, b) => (a.lastOccurred > b.lastOccurred ? -1 : 1));
    return limit !== undefined ? all.slice(0, limit) : all;
  }

  clear(): void {
    this._fpIndex.clear();
    this._moduleIndex.clear();
    this._store.clear();
  }

  clearModule(moduleId: string): void {
    const entries = this._moduleIndex.get(moduleId);
    if (entries !== undefined) {
      for (const entry of entries) {
        this._fpIndex.delete(entry.fingerprint);
      }
      this._moduleIndex.delete(moduleId);
    }
  }

  private _evictModule(moduleId: string): void {
    const entries = this._moduleIndex.get(moduleId);
    if (entries === undefined) return;
    while (entries.length > this._maxEntriesPerModule) {
      const evicted = entries.shift()!;
      this._fpIndex.delete(evicted.fingerprint);
    }
    if (entries.length === 0) {
      this._moduleIndex.delete(moduleId);
    }
  }

  private _evictTotal(): void {
    while (this._fpIndex.size > this._maxTotalEntries) {
      this._popOldest();
    }
  }

  private _popOldest(): void {
    while (this._heap.size > 0) {
      const item = this._heap.pop()!;
      const [ts, , entry] = item;
      // Lazy deletion: skip if already evicted OR if entry was updated by dedup
      if (this._fpIndex.has(entry.fingerprint) && entry.lastOccurred === ts) {
        this._fpIndex.delete(entry.fingerprint);
        const moduleEntries = this._moduleIndex.get(entry.moduleId);
        if (moduleEntries !== undefined) {
          const idx = moduleEntries.indexOf(entry);
          if (idx >= 0) moduleEntries.splice(idx, 1);
          if (moduleEntries.length === 0) {
            this._moduleIndex.delete(entry.moduleId);
          }
        }
        return;
      }
    }
  }
}
