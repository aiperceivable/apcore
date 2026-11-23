/**
 * System control modules -- runtime config update and hot-reload.
 */

import * as fs from 'node:fs';
import { v4 as uuidv4 } from 'uuid';
import * as yaml from 'js-yaml';
import {
  ConfigError,
  ErrorCodes,
  InvalidInputError,
  ModuleError,
  ModuleNotFoundError,
  ReloadFailedError,
  ModuleReloadConflictError,
} from '../errors.js';
import type { Registry } from '../registry/registry.js';
import { resolveDependencies } from '../registry/dependencies.js';
import { parseDependencies } from '../registry/metadata-pure.js';
import type { DependencyInfo } from '../registry/types.js';
import type { EventEmitter } from '../events/emitter.js';
import { createEvent } from '../events/emitter.js';
import type { Config } from '../config.js';
import { CONSTRAINTS } from '../config.js';
import type { Context } from '../context.js';
import type { AuditStore } from './audit.js';
import { buildAuditEntry, extractAuditIdentity } from './audit.js';
import { matchGlob } from '../utils/pattern.js';
import { REDACTED_VALUE } from '../executor.js';
import type { OverridesStore } from './overrides.js';

const RESTRICTED_KEYS = new Set(['sys_modules.enabled']);

const SENSITIVE_SEGMENTS = ['token', 'secret', 'key', 'password', 'auth', 'credential'] as const;

/** Match exact segments or underscore-compound segments (api_key, auth_token). */
function isSensitiveKey(key: string): boolean {
  return key.toLowerCase().split('.').some((seg) =>
    SENSITIVE_SEGMENTS.some((s) => seg === s || seg.endsWith(`_${s}`) || seg.startsWith(`${s}_`)),
  );
}

export interface UpdateConfigOptions {
  auditStore?: AuditStore;
  overridesPath?: string;
  /** Pluggable persistent override store (Issue #45.1). */
  overridesStore?: OverridesStore;
}

export class UpdateConfigModule {
  readonly description = 'Update a runtime configuration value by dot-path key';
  // SYS-19: `idempotent` is declared PER MODULE. All three `system.control.*`
  // modules used to claim `true`, so middleware that retries or dedupes on the
  // annotation would retry a config WRITE. A config update is a state
  // transition with an audit entry and an `apcore.config.updated` event per
  // call — replaying it is not free.
  readonly annotations = { readonly: false, destructive: false, idempotent: false, requiresApproval: true, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      key: { type: 'string' as const, description: 'Dot-path config key' },
      value: { description: 'New value' },
      reason: { type: 'string' as const, description: 'Audit reason' },
    },
    required: ['key', 'value', 'reason'],
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      success: { type: 'boolean' as const, description: 'Whether the update succeeded' },
      key: { type: 'string' as const, description: 'Updated config key' },
      old_value: { description: 'Previous value (redacted for sensitive keys)' },
      new_value: { description: 'New value (redacted for sensitive keys)' },
    },
    required: ['success', 'key', 'old_value', 'new_value'],
  };

  private readonly _config: Config;
  private readonly _emitter: EventEmitter;
  private readonly _auditStore: AuditStore | null;
  private readonly _overridesPath: string | null;
  private readonly _overridesStore: OverridesStore | null;

  constructor(config: Config, eventEmitter: EventEmitter, options?: UpdateConfigOptions) {
    this._config = config;
    this._emitter = eventEmitter;
    this._auditStore = options?.auditStore ?? null;
    this._overridesPath = options?.overridesPath ?? null;
    this._overridesStore = options?.overridesStore ?? null;
  }

  execute(inputs: Record<string, unknown>, context: unknown): Record<string, unknown> {
    const { key, value, reason } = this._validateInputs(inputs);
    const ctx = context as Context | null;

    this._checkRestricted(key);

    const oldValue = this._config.get(key);
    this._config.set(key, value);

    this._validatePostSet(key, value, oldValue);

    // SYS-13: mask with the framework's canonical redaction constant, which is
    // what apcore-python and apcore-rust write. A bespoke `'***'` here meant an
    // operator grepping an audit trail for the redaction marker found the
    // config-write events on two SDKs of three, and a consumer matching on the
    // constant silently classified a redacted value as a real one.
    const isSensitive = isSensitiveKey(key);
    const safeOld = isSensitive ? REDACTED_VALUE : oldValue;
    const safeNew = isSensitive ? REDACTED_VALUE : value;

    // Store-XOR-path precedence (Python/Rust parity, matches toggle.ts): the
    // pluggable store takes precedence over the legacy YAML path; never both.
    if (this._overridesStore !== null) {
      this._persistOverrideToStore(key, value);
    } else if (this._overridesPath !== null) {
      this._persistOverride(key, value);
    }

    const entry = buildAuditEntry('update_config', 'system.control.update_config', ctx, { before: safeOld, after: safeNew });
    if (this._auditStore !== null) {
      this._auditStore.append(entry);
    } else {
      console.warn(`[apcore:audit] update_config key=${key} actor=${entry.actorId} reason=${reason}`);
    }

    // Issue #45.2: Audit events carry requester identity so subscribers can
    // attribute the change. caller_id defaults to "@external" when absent.
    // When identity is null the field is OMITTED from the payload (not
    // emitted as null).
    const { caller_id, identity } = extractAuditIdentity(ctx);
    const updatedPayload: Record<string, unknown> = {
      key, old_value: safeOld, new_value: safeNew, caller_id,
    };
    if (identity !== null) updatedPayload['identity'] = identity;
    this._emitter.emit(createEvent('apcore.config.updated', 'system.control.update_config', 'info', updatedPayload));

    console.warn(`[apcore:control] Config updated: key=${key} old_value=${safeOld} new_value=${safeNew} reason=${reason}`);

    return { success: true, key, old_value: safeOld, new_value: safeNew };
  }

  private _persistOverrideToStore(key: string, value: unknown): void {
    try {
      const store = this._overridesStore!;
      const loaded = store.load();
      if (loaded !== null && typeof (loaded as { then?: unknown }).then === 'function') {
        // Async store — fire-and-forget the read-modify-write so execute() stays sync.
        (loaded as Promise<Record<string, unknown>>)
          .then((existing) => {
            existing[key] = value;
            return store.save(existing);
          })
          .catch((err: unknown) => {
            console.warn(`[apcore:control] Failed to persist override for key '${key}' via OverridesStore:`, err);
          });
        return;
      }
      const existing = loaded as Record<string, unknown>;
      existing[key] = value;
      const saveResult = store.save(existing);
      if (saveResult !== undefined && typeof (saveResult as { then?: unknown }).then === 'function') {
        (saveResult as Promise<void>).catch((err: unknown) => {
          console.warn(`[apcore:control] Failed to persist override for key '${key}' via OverridesStore:`, err);
        });
      }
    } catch (err) {
      console.warn(`[apcore:control] Failed to persist override for key '${key}' via OverridesStore:`, err);
    }
  }

  private _persistOverride(key: string, value: unknown): void {
    let overrides: Record<string, unknown> = {};
    if (fs.existsSync(this._overridesPath!)) {
      try {
        const content = fs.readFileSync(this._overridesPath!, 'utf-8');
        const parsed = yaml.load(content);
        if (typeof parsed === 'object' && parsed !== null) {
          overrides = parsed as Record<string, unknown>;
        }
      } catch {
        // Start fresh if parse fails
      }
    }
    overrides[key] = value;
    try {
      fs.writeFileSync(this._overridesPath!, yaml.dump(overrides), 'utf-8');
    } catch (err) {
      console.warn(`[apcore:control] Failed to persist override for key '${key}':`, err);
    }
  }

  private _validateInputs(inputs: Record<string, unknown>): { key: string; value: unknown; reason: string } {
    const key = inputs['key'];
    if (typeof key !== 'string' || !key) {
      throw new InvalidInputError("'key' is required and must not be empty");
    }
    const reason = inputs['reason'];
    if (typeof reason !== 'string' || !reason) {
      throw new InvalidInputError("'reason' is required and must not be empty");
    }
    const value = inputs['value'];
    return { key, value, reason };
  }

  /** Raise if `key` is in the restricted keys list. Mirrors Python `_check_restricted`. */
  private _checkRestricted(key: string): void {
    if (RESTRICTED_KEYS.has(key)) {
      throw new ModuleError(
        ErrorCodes.CONFIG_KEY_RESTRICTED,
        `Configuration key '${key}' cannot be changed at runtime`,
        { key },
      );
    }
  }

  /**
   * Check the registered constraint after setting. Roll back to `oldValue` and
   * raise ConfigError on failure. Mirrors Python `_validate_post_set`.
   */
  private _validatePostSet(key: string, value: unknown, oldValue: unknown): void {
    const constraint = CONSTRAINTS[key];
    if (constraint === undefined) {
      return;
    }
    const [checkFn, errMsg] = constraint;
    if (!checkFn(value)) {
      this._config.set(key, oldValue);
      throw new ConfigError(`Invalid value for '${key}': ${errMsg} (got ${JSON.stringify(value)})`, {
        details: { key, value },
      });
    }
  }
}

/** @internal */
export class ReloadModule {
  readonly description = 'Hot-reload a module by safe unregister and re-discover';
  // SYS-19: not idempotent — each reload unregisters and re-imports, running
  // `onUnload` / `onLoad` and emitting `apcore.module.reloaded` again.
  readonly annotations = { readonly: false, destructive: false, idempotent: false, requiresApproval: true, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      module_id: { type: 'string' as const, description: 'ID of the module to reload (mutually exclusive with path_filter)' },
      path_filter: { type: 'string' as const, description: 'Glob pattern to bulk-reload matching modules (mutually exclusive with module_id)' },
      reload_dependents: {
        type: 'boolean' as const,
        deprecated: true,
        description:
          'DEPRECATED (spec v1.51.0, D-121) — ignored, and removed at 2.0. Declared in all '
          + 'three SDKs and implemented by none, so no caller could ever rely on it. Use an '
          + 'explicit path_filter that also matches the dependents. At 2.0 this field is '
          + 'REMOVED and, because this schema sets additionalProperties: false, passing it '
          + 'becomes a validation error rather than a silent no-op.',
      },
      reason: { type: 'string' as const, description: 'Audit reason for the reload' },
    },
    required: ['reason'],
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      success: { type: 'boolean' as const, description: 'Whether the reload succeeded' },
      module_id: { type: 'string' as const, description: 'ID of the reloaded module (single-module mode)' },
      reloaded_modules: { type: 'array' as const, description: 'IDs of reloaded modules (path_filter mode)' },
      previous_version: { type: 'string' as const, description: 'Version before reload' },
      new_version: { type: 'string' as const, description: 'Version after reload' },
      reload_duration_ms: { type: 'number' as const, description: 'Reload duration in milliseconds' },
    },
    required: ['success'],
  };

  private readonly _registry: Registry;
  private readonly _emitter: EventEmitter;
  private readonly _auditStore: AuditStore | null;

  constructor(registry: Registry, eventEmitter: EventEmitter, auditStore?: AuditStore) {
    this._registry = registry;
    this._emitter = eventEmitter;
    this._auditStore = auditStore ?? null;
  }

  /**
   * Once per INSTANCE, not per process. `reload` is called by hot-reload loops
   * and watchers, so an advisory whose volume is proportional to traffic is one
   * operators learn to filter out — the cadence reasoning D-89 settled. Per
   * instance rather than per process for the reason D-90's notice is per token:
   * a process-wide one-shot tells the first caller and leaves every later one to
   * discover it in production.
   */
  private _reloadDependentsWarned = false;

  /**
   * Warn once that `reload_dependents` is ignored and will be removed (D-121).
   *
   * The field was declared in all three SDKs' input schemas and read by none — a
   * spec MUST that no implementation satisfied, which is the §9.1.3 "declared
   * surface reaches no mechanism" shape applied to a module input field. Three
   * independent implementations skipping it is the evidence the maintainer
   * decision rests on.
   *
   * Deprecated rather than removed now, because the input schema sets
   * `additionalProperties: false`: at 2.0 the same call stops being a silent
   * no-op and becomes a validation error, so a caller passing it today needs a
   * release in which they are told.
   */
  private _warnReloadDependents(inputs: Record<string, unknown>): void {
    if (!inputs['reload_dependents']) return;
    if (this._reloadDependentsWarned) return;
    this._reloadDependentsWarned = true;
    console.warn(
      "[apcore:sys-modules] system.control.reload: 'reload_dependents' is ignored and will be " +
        'removed at 2.0 (spec D-121). No SDK has ever implemented it. Use a path_filter that also ' +
        'matches the dependents; after removal this field becomes a validation error, not a no-op.',
    );
  }

  async execute(inputs: Record<string, unknown>, context: unknown): Promise<Record<string, unknown>> {
    this._warnReloadDependents(inputs);
    const { moduleId, pathFilter, reason } = this._validateInputs(inputs);
    const ctx = context as Context | null;

    if (pathFilter !== undefined) {
      return this._reloadWithPathFilter(pathFilter, reason, ctx);
    }

    return this._reloadSingleModule(moduleId!, reason, ctx);
  }

  private async _reloadSingleModule(moduleId: string, reason: string, ctx: Context | null): Promise<Record<string, unknown>> {
    const existing = this._registry.get(moduleId);
    if (existing === null) {
      throw new ModuleNotFoundError(moduleId);
    }
    const existingObj = existing as Record<string, unknown>;
    const previousVersion = String(existingObj['version'] ?? '1.0.0');

    let suspendedState: Record<string, unknown> | null = null;
    if (typeof existingObj['onSuspend'] === 'function') {
      try {
        suspendedState = (existingObj['onSuspend'] as () => Record<string, unknown> | null)();
      } catch (err) {
        console.warn(`[apcore:control] onSuspend failed for ${moduleId}:`, err);
      }
    }

    const start = performance.now();
    await this._registry.safeUnregister(moduleId);

    try {
      await this._registry.discover();
    } catch (err) {
      this._registry.registerInternal(moduleId, existing);
      throw new ReloadFailedError(moduleId, String(err));
    }

    const reloaded = this._registry.get(moduleId);
    if (reloaded === null) {
      this._registry.registerInternal(moduleId, existing);
      throw new ReloadFailedError(moduleId, `Module '${moduleId}' was not found after re-discovery`);
    }

    const reloadedObj = reloaded as Record<string, unknown>;
    if (suspendedState !== null && typeof reloadedObj['onResume'] === 'function') {
      try {
        (reloadedObj['onResume'] as (state: Record<string, unknown>) => void)(suspendedState);
      } catch (err) {
        console.warn(`[apcore:control] onResume failed for ${moduleId}:`, err);
      }
    }

    const elapsedMs = performance.now() - start;
    const newVersion = String(reloadedObj['version'] ?? '1.0.0');

    const entry = buildAuditEntry('reload_module', moduleId, ctx, { before: previousVersion, after: newVersion });
    if (this._auditStore !== null) {
      this._auditStore.append(entry);
    } else {
      console.warn(`[apcore:audit] reload_module ${moduleId} actor=${entry.actorId} reason=${reason}`);
    }

    // Issue #45.2: include requester identity in reload audit event payload.
    const { caller_id, identity } = extractAuditIdentity(ctx);
    const reloadedPayload: Record<string, unknown> = {
      module_id: moduleId,
      previous_version: previousVersion,
      new_version: newVersion,
      caller_id,
    };
    if (identity !== null) reloadedPayload['identity'] = identity;
    this._emitter.emit(createEvent('apcore.module.reloaded', moduleId, 'info', reloadedPayload));

    console.warn(
      `[apcore:control] Module reloaded: module_id=${moduleId} previous_version=${previousVersion} new_version=${newVersion} reason=${reason}`,
    );

    return {
      success: true,
      module_id: moduleId,
      previous_version: previousVersion,
      new_version: newVersion,
      reload_duration_ms: elapsedMs,
    };
  }

  /**
   * Order `moduleIds` so a declared dependency is reloaded before the module
   * that needs it (leaves first), matching apcore-python
   * `ReloadModule._topo_sort_modules`.
   *
   * Only edges *within* the matched set are considered — a dependency outside
   * the path_filter is not being reloaded, so it constrains nothing. Kahn's
   * sort seeds its queue from a sorted zero-in-degree list, so a set that
   * declares no dependencies at all comes back in alphabetical order: the
   * previous behaviour is the degenerate case of this one, not a separate
   * branch. A cycle or unresolvable edge falls back to alphabetical rather
   * than failing the reload.
   */
  private _topoSortModules(moduleIds: string[]): string[] {
    const matchedSet = new Set(moduleIds);
    const entries: Array<[string, DependencyInfo[]]> = [];
    for (const id of moduleIds) {
      const meta = this._registry.getModuleMetadata(id);
      // YAML-declared `dependencies` reaches the merge unvalidated (no schema
      // describes it), so a scalar can arrive here. Anything that is not a
      // list means "no declared dependencies", never a crashed reload.
      const rawValue = meta['dependencies'];
      const depsRaw = Array.isArray(rawValue) ? (rawValue as Array<Record<string, unknown>>) : [];
      const deps = depsRaw.length > 0 ? parseDependencies(depsRaw) : [];
      entries.push([id, deps.filter((d) => matchedSet.has(d.moduleId))]);
    }

    try {
      return resolveDependencies(entries, matchedSet);
    } catch (err) {
      console.warn(
        `[apcore:control] Topo sort failed for path_filter reload; falling back to alphabetical: ${String(err)}`,
      );
      return [...moduleIds].sort();
    }
  }

  private async _reloadWithPathFilter(pathFilter: string, reason: string, ctx: Context | null): Promise<Record<string, unknown>> {
    const started = Date.now();
    const allIds = this._registry.list();
    // Alphabetical first so the topo sort has a deterministic input, then
    // dependency order on top (fixture `system_modules_hardening.json`
    // reload_with_path_filter: `reload_order: "topological"`).
    const matchingIds = this._topoSortModules(
      // PROTOCOL_SPEC 6.7 clause 4 / 9.2.3: `path_filter` is a GLOB-dialect
      // pattern (A25), not module-ID matching (A08). The two differ on `?`,
      // which A08 treats as a literal — so `executor.?mail.*` reloaded
      // nothing here while the other two SDKs reloaded two modules, and the
      // zero-match no-op rule made that silent (#117).
      allIds.filter((id) => matchGlob(pathFilter, id)).sort(),
    );

    // Capture existing modules and versions before unregistering
    const existingModules = new Map<string, unknown>();
    const previousVersions = new Map<string, string>();
    for (const id of matchingIds) {
      const mod = this._registry.get(id);
      if (mod !== null) {
        existingModules.set(id, mod);
        previousVersions.set(id, String((mod as Record<string, unknown>)['version'] ?? '1.0.0'));
      }
    }

    // Unregister all matching modules
    for (const id of matchingIds) {
      await this._registry.safeUnregister(id);
    }

    // Re-discover
    try {
      await this._registry.discover();
    } catch (err) {
      for (const [id, mod] of existingModules) {
        this._registry.registerInternal(id, mod);
      }
      throw new ReloadFailedError(pathFilter, String(err));
    }

    // Collect successfully reloaded modules. A module discovery did not bring
    // back is restored and fails the operation (D-112 rule 4); modules that
    // did reload stay reloaded.
    const reloadedModules: string[] = [];
    const missing: string[] = [];
    for (const id of matchingIds) {
      const reloaded = this._registry.get(id);
      if (reloaded === null) {
        missing.push(id);
        const orig = existingModules.get(id);
        if (orig !== undefined) {
          this._registry.registerInternal(id, orig);
        }
      } else {
        reloadedModules.push(id);
        // Emit the same event the single-module path emits. The event bus is
        // the MINIMUM audit surface (system-modules.md Contextual Auditing),
        // precisely for the no-store case, so a bulk reload emits it per
        // module as apcore-python and apcore-rust do.
        const { caller_id, identity } = extractAuditIdentity(ctx);
        const payload: Record<string, unknown> = {
          module_id: id,
          previous_version: previousVersions.get(id) ?? '1.0.0',
          new_version: String((reloaded as Record<string, unknown>)['version'] ?? '1.0.0'),
          caller_id,
        };
        if (identity !== null) payload['identity'] = identity;
        this._emitter.emit(createEvent('apcore.module.reloaded', id, 'info', payload));
      }
    }

    if (missing.length > 0) {
      throw new ReloadFailedError(
        pathFilter,
        `${missing.length} module(s) were not restored by re-discovery and were put back as they were: ${missing.join(', ')}`,
      );
    }

    // D-111: one audit entry PER MODULE, and every entry from ONE bulk reload
    // shares a correlation id so "what did this deploy touch" stays a single
    // query. Generated once, here, rather than per entry: a fresh id per entry
    // groups nothing, and a constant groups every deploy together. Written only
    // once the operation has succeeded, as apcore-python and apcore-rust do.
    if (this._auditStore !== null) {
      const correlationId = uuidv4();
      for (const id of reloadedModules) {
        const after = this._registry.get(id) as Record<string, unknown> | null;
        this._auditStore.append(
          buildAuditEntry(
            'reload_module',
            id,
            ctx,
            {
              before: previousVersions.get(id) ?? '1.0.0',
              after: String(after?.['version'] ?? '1.0.0'),
            },
            correlationId,
          ),
        );
      }
    }

    console.warn(`[apcore:control] Bulk reload: path_filter=${pathFilter} reloaded=${reloadedModules.length} reason=${reason}`);

    // `module_id: null` and `reload_duration_ms` match the shape apcore-python
    // and apcore-rust return for the bulk branch; omitting them made a caller
    // parsing one response shape fail against this SDK (sync finding A-D-017).
    return {
      success: true,
      module_id: null,
      reloaded_modules: reloadedModules,
      reload_duration_ms: Date.now() - started,
    };
  }

  private _validateInputs(inputs: Record<string, unknown>): { moduleId?: string; pathFilter?: string; reason: string } {
    const moduleId = inputs['module_id'];
    const pathFilter = inputs['path_filter'];

    if (moduleId !== undefined && pathFilter !== undefined) {
      throw new ModuleReloadConflictError();
    }
    if (moduleId === undefined && pathFilter === undefined) {
      throw new InvalidInputError("either 'module_id' or 'path_filter' is required");
    }

    if (moduleId !== undefined) {
      if (typeof moduleId !== 'string' || !moduleId) {
        throw new InvalidInputError("'module_id' is required and must be a non-empty string");
      }
    }
    if (pathFilter !== undefined) {
      if (typeof pathFilter !== 'string' || !pathFilter) {
        throw new InvalidInputError("'path_filter' must be a non-empty string");
      }
    }

    const reason = inputs['reason'];
    if (typeof reason !== 'string' || !reason) {
      throw new InvalidInputError("'reason' is required and must be a non-empty string");
    }

    return {
      moduleId: moduleId as string | undefined,
      pathFilter: pathFilter as string | undefined,
      reason,
    };
  }
}
