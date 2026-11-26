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
  private readonly _data: HeapItem[] = [];
  private readonly _positions = new Map<ErrorEntry, number>();

  get size(): number {
    return this._data.length;
  }

  update(item: HeapItem): void {
    const entry = item[2];
    const position = this._positions.get(entry);
    if (position === undefined) {
      this._positions.set(entry, this._data.length);
      this._data.push(item);
      this._siftUp(this._data.length - 1);
    } else {
      const previous = this._data[position];
      this._data[position] = item;
      if (heapCompare(item, previous) < 0) this._siftUp(position);
      else this._siftDown(position);
    }
  }

  pop(): ErrorEntry | undefined {
    const top = this._data[0];
    if (top === undefined) return undefined;
    this.remove(top[2]);
    return top[2];
  }

  remove(entry: ErrorEntry): void {
    const idx = this._positions.get(entry);
    if (idx === undefined) return;
    this._positions.delete(entry);
    const last = this._data.pop()!;
    if (idx < this._data.length) {
      this._data[idx] = last;
      this._positions.set(last[2], idx);
      const parent = Math.floor((idx - 1) / 2);
      if (idx > 0 && heapCompare(last, this._data[parent]) < 0) this._siftUp(idx);
      else this._siftDown(idx);
    }
  }

  sequence(entry: ErrorEntry): number {
    return this._data[this._positions.get(entry)!][1];
  }

  clear(): void {
    this._data.length = 0;
    this._positions.clear();
  }

  private _swap(a: number, b: number): void {
    [this._data[a], this._data[b]] = [this._data[b], this._data[a]];
    this._positions.set(this._data[a][2], a);
    this._positions.set(this._data[b][2], b);
  }

  private _siftUp(idx: number): void {
    while (idx > 0) {
      const parent = Math.floor((idx - 1) / 2);
      if (heapCompare(this._data[idx], this._data[parent]) < 0) {
        this._swap(idx, parent);
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
      this._swap(idx, smallest);
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
 * The heap keeps one node per live entry and updates it in place on repeats.
 */
export class ErrorHistory {
  private readonly _maxEntriesPerModule: number;
  private readonly _maxTotalEntries: number;
  private readonly _store: ObservabilityStore;
  private readonly _defaultStore: InMemoryObservabilityStore | null;
  private readonly _storage: StorageBackend;
  private readonly _ownsStorage: boolean;
  private readonly _fpIndex: Map<string, ErrorEntry> = new Map();
  private readonly _moduleIndex: Map<string, ErrorEntry[]> = new Map();
  private readonly _heap: MinHeap = new MinHeap();
  private readonly _pendingWrites = new Map<string, Record<string, unknown> | undefined>();
  private _seq = 0;

  constructor(options: ErrorHistoryOptions = {}) {
    this._maxEntriesPerModule = options.maxEntriesPerModule ?? 50;
    this._maxTotalEntries = options.maxTotalEntries ?? 1000;
    if (options.store == null) {
      this._defaultStore = new InMemoryObservabilityStore();
      this._store = this._defaultStore;
    } else {
      this._defaultStore = null;
      this._store = options.store;
    }
    this._ownsStorage = options.storage == null;
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
      this._heap.update([now, this._seq, existing]);
      this._recordEntry(existing);
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
    this._heap.update([now, this._seq, entry]);
    this._evictModule(moduleId);
    this._evictTotal();
    this._recordEntry(entry);
  }

  private _recordEntry(entry: ErrorEntry): void {
    const key = entry.fingerprint;
    const retained = this._fpIndex.has(key);
    if (this._defaultStore !== null) {
      if (retained) this._defaultStore.retainError(key, entry);
    } else {
      try {
        this._store.recordError(entry);
      } catch (error) {
        console.warn('[apcore:error-history] Store notification failed', error);
      }
    }
    if (this._ownsStorage && !retained) return;
    const snapshot = {
      module_id: entry.moduleId,
      code: entry.code,
      message: entry.message,
      timestamp: entry.timestamp,
      count: entry.count,
      first_occurred: entry.firstOccurred,
      last_occurred: entry.lastOccurred,
      fingerprint: entry.fingerprint,
    };
    if (this._pendingWrites.has(key)) {
      // Only the latest snapshot needs to be written after the active write.
      this._pendingWrites.set(key, snapshot);
      return;
    }
    this._save(key, snapshot);
  }

  private _save(key: string, snapshot: Record<string, unknown>): void {
    let write: Promise<void> | void;
    try {
      write = this._storage.save(STORAGE_NAMESPACE_ERROR_HISTORY, key, snapshot);
    } catch (error) {
      console.warn('[apcore:error-history] Storage write failed', error);
      return;
    }
    if (write === undefined) return;
    this._pendingWrites.set(key, undefined);
    void Promise.resolve(write).catch(error => {
      console.warn('[apcore:error-history] Storage write failed', error);
    }).then(() => {
      const pending = this._pendingWrites.get(key);
      this._pendingWrites.delete(key);
      if (pending !== undefined) this._save(key, pending);
    });
  }

  get(moduleId: string, limit?: number): ErrorEntry[] {
    const moduleEntries = this._moduleIndex.get(moduleId) ?? [];
    const result = [...moduleEntries].reverse();
    return limit !== undefined ? result.slice(0, limit) : result;
  }

  getAll(limit?: number): ErrorEntry[] {
    const all = [...this._fpIndex.values()];
    all.sort((a, b) => b.lastOccurred.localeCompare(a.lastOccurred) || this._heap.sequence(b) - this._heap.sequence(a));
    return limit !== undefined ? all.slice(0, limit) : all;
  }

  clear(): void {
    for (const fingerprint of this._fpIndex.keys()) this._evictDefaultBacking(fingerprint);
    this._fpIndex.clear();
    this._moduleIndex.clear();
    this._heap.clear();
    this._store.clear();
  }

  clearModule(moduleId: string): void {
    const entries = this._moduleIndex.get(moduleId);
    if (entries !== undefined) {
      for (const entry of entries) {
        this._fpIndex.delete(entry.fingerprint);
        this._heap.remove(entry);
        this._evictDefaultBacking(entry.fingerprint);
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
      this._heap.remove(evicted);
      this._evictDefaultBacking(evicted.fingerprint);
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
    const entry = this._heap.pop()!;
    this._fpIndex.delete(entry.fingerprint);
    this._evictDefaultBacking(entry.fingerprint);
    const moduleEntries = this._moduleIndex.get(entry.moduleId)!;
    moduleEntries.splice(moduleEntries.indexOf(entry), 1);
    if (moduleEntries.length === 0) {
      this._moduleIndex.delete(entry.moduleId);
    }
  }

  private _evictDefaultBacking(fingerprint: string): void {
    this._defaultStore?.evictError(fingerprint);
    if (this._ownsStorage) this._storage.delete(STORAGE_NAMESPACE_ERROR_HISTORY, fingerprint);
  }
}
