/**
 * Persistent runtime override store (Issue #45.1).
 *
 * Mirrors the Python `_load_overrides` / `_write_overrides` flow and the
 * Rust `load_overrides` / `write_override` helpers, but exposed as a
 * pluggable interface so embeddings (tests, multi-tenant deployments,
 * remote config services) can swap the storage layer without forking
 * `registerSysModules` or `UpdateConfigModule`.
 *
 * Each override is a flat `key → value` mapping where `key` is the
 * dot-path config key (e.g. `"app.timeout"` or `"toggle.module.id"`).
 */

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as yaml from 'js-yaml';

/**
 * Pluggable persistence for runtime config & toggle overrides.
 *
 * Implementations must be safe to call from multiple call sites in sequence;
 * the `FileOverridesStore` reference implementation uses an atomic
 * tempfile + rename to prevent partial-write corruption.
 */
export interface OverridesStore {
  /**
   * Load all overrides from the backing store. Returns `{}` if empty/missing.
   *
   * Implementations may return synchronously or asynchronously — async stores
   * (e.g. remote config services) cannot be applied at startup by the
   * synchronous `registerSysModules`, so prefer sync stores there or
   * pre-load and pass results into `Config` directly.
   */
  load(): Promise<Record<string, unknown>> | Record<string, unknown>;

  /** Replace the entire override set with the given mapping. */
  save(overrides: Record<string, unknown>): Promise<void> | void;
}

/**
 * In-memory store, intended primarily for tests and embeddings that do not
 * require disk persistence.
 */
export class InMemoryOverridesStore implements OverridesStore {
  private _data: Record<string, unknown>;

  constructor(initial?: Record<string, unknown>) {
    this._data = initial !== undefined ? { ...initial } : {};
  }

  load(): Record<string, unknown> {
    return { ...this._data };
  }

  save(overrides: Record<string, unknown>): void {
    this._data = { ...overrides };
  }
}

/**
 * YAML-file-backed override store with atomic write semantics.
 *
 * Each `save()` writes to a sibling tempfile and `fs.renameSync`-es it into
 * place so an interrupted process cannot leave a half-written overrides file
 * behind (matches Python's `os.replace` and Rust's `std::fs::rename` flow).
 */
export class FileOverridesStore implements OverridesStore {
  private readonly _path: string;

  constructor(filePath: string) {
    this._path = filePath;
  }

  /** Path of the underlying YAML file. */
  get path(): string {
    return this._path;
  }

  load(): Record<string, unknown> {
    if (!fs.existsSync(this._path)) return {};
    let raw: string;
    try {
      raw = fs.readFileSync(this._path, 'utf-8');
    } catch (err) {
      console.warn(`[apcore:overrides] Failed to read overrides file '${this._path}':`, err);
      return {};
    }
    if (raw.length === 0) return {};
    try {
      const parsed = yaml.load(raw);
      if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) {
        return { ...(parsed as Record<string, unknown>) };
      }
      console.warn(`[apcore:overrides] Overrides file '${this._path}' root is not a mapping; skipping`);
      return {};
    } catch (err) {
      console.warn(`[apcore:overrides] Overrides file '${this._path}' is not valid YAML; skipping`, err);
      return {};
    }
  }

  save(overrides: Record<string, unknown>): void {
    const dir = path.dirname(this._path) || '.';
    try {
      fs.mkdirSync(dir, { recursive: true });
    } catch (err) {
      console.error(`[apcore:overrides] Failed to create parent directory '${dir}':`, err);
      return;
    }

    // Sort keys for stable on-disk layout.
    const ordered: Record<string, unknown> = {};
    for (const key of Object.keys(overrides).sort()) {
      ordered[key] = overrides[key];
    }
    const yamlText = yaml.dump(ordered, { sortKeys: true });

    const base = path.basename(this._path);
    const tmp = path.join(dir, `.${base}.${process.pid}.${Date.now()}.tmp`);
    try {
      fs.writeFileSync(tmp, yamlText, 'utf-8');
      fs.renameSync(tmp, this._path);
    } catch (err) {
      console.error(`[apcore:overrides] Failed to write overrides file '${this._path}':`, err);
      try {
        fs.unlinkSync(tmp);
      } catch {
        /* ignore */
      }
    }
  }
}
