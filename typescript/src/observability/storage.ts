/**
 * Pluggable observability storage backend (Issue #43 §1).
 *
 * Provides a language-agnostic key/value abstraction so SDK consumers can
 * persist error history, usage records, and metric snapshots to redis,
 * postgres, etc. without ErrorHistory / UsageCollector / MetricsCollector
 * needing to know the underlying technology.
 *
 * The default `InMemoryStorageBackend` keeps everything in a Map and is the
 * implicit fallback when no backend is supplied.
 */

/**
 * Pluggable key/value storage backend.
 *
 * All operations may return synchronously or asynchronously — implementations
 * targeting redis/postgres typically return Promises, while in-memory
 * implementations return values directly. Callers should `await` the result.
 *
 * Namespaces partition the key space (e.g. "errors", "usage", "metrics") so
 * a single backend instance can be shared by multiple collectors safely.
 */
/**
 * The namespace each bundled collector writes under (D-113).
 *
 * Named here, identically in every SDK (docs/features/metrics-and-usage.md
 * "Pluggable storage backends"), because a namespace IS the key a caller
 * queries by: an unnamed one is an argument that cannot be read back.
 */
export const STORAGE_NAMESPACE_METRICS = 'metrics';
export const STORAGE_NAMESPACE_USAGE = 'usage';
export const STORAGE_NAMESPACE_ERROR_HISTORY = 'error_history';

export interface StorageBackend {
  save(
    namespace: string,
    key: string,
    value: Record<string, unknown>,
  ): Promise<void> | void;

  get(
    namespace: string,
    key: string,
  ): Promise<Record<string, unknown> | null> | Record<string, unknown> | null;

  list(
    namespace: string,
    prefix?: string,
  ):
    | Promise<Array<[string, Record<string, unknown>]>>
    | Array<[string, Record<string, unknown>]>;

  delete(namespace: string, key: string): Promise<void> | void;
}

/**
 * In-memory default backend.
 *
 * Each namespace is its own `Map<string, Record<string, unknown>>`. Values are
 * stored by reference — callers that need snapshot semantics should clone
 * before save. This is the implicit default when no backend is supplied to
 * `ErrorHistory`, `UsageCollector`, or `MetricsCollector`.
 */
export class InMemoryStorageBackend implements StorageBackend {
  private readonly _namespaces: Map<string, Map<string, Record<string, unknown>>> = new Map();

  save(namespace: string, key: string, value: Record<string, unknown>): void {
    let ns = this._namespaces.get(namespace);
    if (ns === undefined) {
      ns = new Map();
      this._namespaces.set(namespace, ns);
    }
    ns.set(key, value);
  }

  get(namespace: string, key: string): Record<string, unknown> | null {
    const ns = this._namespaces.get(namespace);
    if (ns === undefined) return null;
    return ns.get(key) ?? null;
  }

  list(
    namespace: string,
    prefix?: string,
  ): Array<[string, Record<string, unknown>]> {
    const ns = this._namespaces.get(namespace);
    if (ns === undefined) return [];
    const entries: Array<[string, Record<string, unknown>]> = [];
    for (const [k, v] of ns.entries()) {
      if (prefix === undefined || k.startsWith(prefix)) {
        entries.push([k, v]);
      }
    }
    return entries;
  }

  delete(namespace: string, key: string): void {
    const ns = this._namespaces.get(namespace);
    if (ns === undefined) return;
    ns.delete(key);
    if (ns.size === 0) {
      this._namespaces.delete(namespace);
    }
  }
}
