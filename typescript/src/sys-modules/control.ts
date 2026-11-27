/**
 * System control modules -- runtime config update and hot-reload.
 */

import { v4 as uuidv4 } from 'uuid';
import {
  ConfigError,
  ErrorCodes,
  InvalidInputError,
  ModuleError,
  ModuleNotFoundError,
  ReloadFailedError,
  ModuleReloadConflictError,
} from '../errors.js';
import type { RegistrationSnapshot, Registry } from '../registry/registry.js';
import { resolveDependencies } from '../registry/dependencies.js';
import { parseDependencies } from '../registry/metadata-pure.js';
import type { DependencyInfo } from '../registry/types.js';
import type { EventEmitter } from '../events/emitter.js';
import { createEvent } from '../events/emitter.js';
import { Config, CONSTRAINTS, isConfigurationDataPath } from '../config.js';
import { FRAMEWORK_CONFIG_KEYS } from '../config-key-surface.js';
import type { Context } from '../context.js';
import { LifecycleProgress, lifecycleReportToWire, validateLifecycleDuration, type ModuleLifecycleProgress } from '../lifecycle.js';
import type { Module } from '../module.js';
import type { ManagedDrainPolicy } from '../registry/managed-drain.js';
import { RedactionConfig } from '../observability/context-logger.js';
import type { AuditStore } from './audit.js';
import { buildAuditEntry, extractAuditIdentity } from './audit.js';
import { matchGlob } from '../utils/pattern.js';
import { REDACTED_VALUE } from '../executor.js';
import { FileOverridesStore, type OverridesStore } from './overrides.js';
import { persistOverride, TOGGLE_OVERRIDE_NAMESPACE, TOGGLE_OVERRIDE_PREFIX } from './override-update.js';

const RESTRICTED_KEYS = new Set(['sys_modules.enabled']);
const RELOAD_MODULE_ID = 'system.control.reload_module';

const reloadLifecycleOptionsSchema = {
  type: 'object', additionalProperties: false,
  description: 'Opts in to managed reload and adds a lifecycle report to the result',
  properties: {
    drain_timeout_ms: { type: 'integer', minimum: 0, maximum: 2_147_483_647, description: 'Drain budget in milliseconds' },
    on_drain_timeout: { enum: ['abort_reload', 'continue_waiting'], description: 'Action when the drain budget expires' },
  },
};

const reloadLifecycleErrorSchema = {
  type: 'object', additionalProperties: false, required: ['code', 'phase'],
  properties: { code: { type: 'string', minLength: 1 }, phase: { type: 'string', minLength: 1 } },
};

const reloadLifecycleReportSchema = {
  type: 'object', additionalProperties: false,
  description: 'Terminal result of an accepted module reload with lifecycle options',
  required: ['operation_id', 'kind', 'status', 'event_flush', 'modules'],
  properties: {
    operation_id: { type: 'string', minLength: 1 }, kind: { const: 'reload' },
    status: { enum: ['completed', 'failed'] }, event_flush: { const: 'not_applicable' },
    error: reloadLifecycleErrorSchema,
    modules: { type: 'array', items: {
      type: 'object', additionalProperties: false, required: ['module_id', 'phase', 'outcome'],
      properties: {
        module_id: { type: 'string' },
        phase: { enum: ['pending', 'draining', 'recovering', 'unloading', 'loading', 'resuming', 'done'] },
        outcome: { enum: ['reloaded', 'retained_ready', 'retained_unavailable', 'failed', 'absent', 'already_removing', 'not_started'] },
        drain_timed_out: { type: 'boolean' }, abort_reason: { enum: ['drain_timeout', 'drain_error'] },
        compensation: { enum: ['not_required', 'restored', 'failed'] }, error: reloadLifecycleErrorSchema,
      },
    } },
  },
};

const SENSITIVE_SEGMENTS = ['token', 'secret', 'key', 'password', 'auth', 'credential'] as const;

/** Match exact segments or underscore-compound segments (api_key, auth_token). */
function isLegacySensitiveField(fieldName: string): boolean {
  const segment = fieldName.toLowerCase();
  return SENSITIVE_SEGMENTS.some(
    (s) => segment === s || segment.endsWith(`_${s}`) || segment.startsWith(`${s}_`),
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
      value: { description: 'New value', 'x-sensitive': true },
      reason: { type: 'string' as const, description: 'Audit reason' },
    },
    required: ['key', 'value', 'reason'],
  };
  readonly outputSchema = {
    type: 'object' as const,
    additionalProperties: false,
    properties: {
      success: { type: 'boolean' as const, description: 'Whether the update succeeded' },
      key: { type: 'string' as const, description: 'Updated config key' },
      old_value: { description: 'Previous value, null if absent, or redacted for sensitive keys' },
      new_value: { description: 'New value (redacted for sensitive keys)' },
    },
    required: ['success', 'key', 'old_value', 'new_value'],
  };

  private readonly _config: Config;
  private readonly _emitter: EventEmitter;
  private readonly _auditStore: AuditStore | null;
  private readonly _overridesStore: OverridesStore | null;

  constructor(config: Config, eventEmitter: EventEmitter, options?: UpdateConfigOptions) {
    this._config = config;
    this._emitter = eventEmitter;
    this._auditStore = options?.auditStore ?? null;
    this._overridesStore = options?.overridesStore ??
      (options?.overridesPath != null ? new FileOverridesStore(options.overridesPath) : null);
  }

  execute(inputs: Record<string, unknown>, context: unknown): Record<string, unknown> {
    const { key, value, reason } = this._validateInputs(inputs);
    const ctx = context as Context | null;

    const frameworkKey = key.startsWith('apcore.') ? key.slice('apcore.'.length) : key;
    this._checkRestricted(frameworkKey);
    this._validateValue(frameworkKey, value);

    const isFrameworkKey = FRAMEWORK_CONFIG_KEYS.includes(frameworkKey);
    const settingsKey = isFrameworkKey ? frameworkKey : key;
    const oldValue = this._config.get(settingsKey) ?? null;

    // Resolve the key rules before the update, which may itself replace them.
    const redaction = RedactionConfig.forSensitiveKeys(this._config, REDACTED_VALUE, isLegacySensitiveField);
    const redact = (value: unknown): unknown => {
      const safeValue = redaction.redactConfigValue(settingsKey, value);
      return isFrameworkKey ? redaction.redactConfigValue(`apcore.${settingsKey}`, safeValue) : safeValue;
    };
    const safeOld = redact(oldValue);
    this._config.set(settingsKey, value);
    const effectiveValue = this._config.get(settingsKey) ?? null;
    const safeNew = redact(effectiveValue);

    // Store-XOR-path precedence (Python/Rust parity, matches toggle.ts): the
    // pluggable store takes precedence over the legacy YAML path; never both.
    if (this._overridesStore !== null) {
      persistOverride(this._overridesStore, settingsKey, effectiveValue);
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

  private _validateInputs(inputs: Record<string, unknown>): { key: string; value: unknown; reason: string } {
    const key = inputs['key'];
    if (typeof key !== 'string' || !key) {
      throw new InvalidInputError("'key' is required and must not be empty");
    }
    if (!isConfigurationDataPath(key)) {
      throw new InvalidInputError("Configuration key contains an invalid path segment");
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
    if (key === 'apcore' || key === TOGGLE_OVERRIDE_NAMESPACE || key.startsWith(TOGGLE_OVERRIDE_PREFIX) || [...RESTRICTED_KEYS].some(
      (restricted) => key === restricted || restricted.startsWith(`${key}.`) || key.startsWith(`${restricted}.`),
    )) {
      throw new ModuleError(
        ErrorCodes.CONFIG_KEY_RESTRICTED,
        `Configuration key '${key}' cannot be changed at runtime`,
        { key },
      );
    }
  }

  /** Check registered constraints before publishing, persisting, or auditing the update. */
  private _validateValue(key: string, value: unknown): void {
    if (value === '' && Config.pathTypedKeys().includes(key)) {
      throw new ConfigError(`Invalid value for '${key}': a filesystem path cannot be empty`, {
        details: { key },
      });
    }
    if (!Object.hasOwn(CONSTRAINTS, key)) return;
    const constraint = CONSTRAINTS[key];
    const [checkFn, errMsg] = constraint;
    if (!checkFn(value)) {
      throw new ConfigError(`Invalid value for '${key}': ${errMsg}`, {
        details: { key },
      });
    }
  }
}

/** Reload modules through the registry. */
export class ReloadModule {
  readonly description = 'Hot-reload a module by safe unregister and re-discover';
  // SYS-19: not idempotent — each reload unregisters and re-imports, running
  // `onUnload` / `onLoad` and emitting `apcore.module.reloaded` again.
  readonly annotations = { readonly: false, destructive: false, idempotent: false, requiresApproval: true, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    additionalProperties: false,
    properties: {
      module_id: { type: 'string' as const, description: 'ID of the module to reload (mutually exclusive with path_filter)' },
      path_filter: { type: 'string' as const, description: 'Glob pattern to bulk-reload matching modules (mutually exclusive with module_id)' },
      reason: { type: 'string' as const, description: 'Audit reason for the reload' },
      lifecycle: reloadLifecycleOptionsSchema,
    },
    required: ['reason'],
  };
  readonly outputSchema = {
    type: 'object' as const,
    additionalProperties: false,
    properties: {
      success: { type: 'boolean' as const, description: 'Whether the reload succeeded' },
      module_id: { type: ['string', 'null'], description: 'ID of the reloaded module, or null in bulk mode' },
      reloaded_modules: {
        type: 'array', items: { type: 'string' }, description: 'IDs of the modules reloaded in bulk mode',
      },
      previous_version: { type: ['string', 'null'], description: 'Version before reload' },
      new_version: { type: ['string', 'null'], description: 'Version after reload' },
      reload_duration_ms: { type: 'number' as const, description: 'Reload duration in milliseconds' },
      lifecycle_report: reloadLifecycleReportSchema,
    },
    required: ['success', 'module_id'],
    oneOf: [
      { properties: { module_id: { type: 'string' }, reloaded_modules: false } },
      {
        required: ['reloaded_modules'],
        properties: {
          module_id: { type: 'null' },
          reloaded_modules: { type: 'array' },
          previous_version: false,
          new_version: false,
        },
      },
    ],
  };

  private readonly _registry: Registry;
  private readonly _emitter: EventEmitter;
  private readonly _auditStore: AuditStore | null;

  constructor(registry: Registry, eventEmitter: EventEmitter, auditStore?: AuditStore) {
    this._registry = registry;
    this._emitter = eventEmitter;
    this._auditStore = auditStore ?? null;
  }

  async execute(inputs: Record<string, unknown>, context: unknown): Promise<Record<string, unknown>> {
    const { moduleId, pathFilter, reason } = this._validateInputs(inputs);
    const ctx = context as Context | null;
    const lifecycle = inputs['lifecycle'];
    if (lifecycle !== undefined) {
      return this._reloadManaged(moduleId, pathFilter, reason, ctx, this._validateReloadLifecycle(lifecycle));
    }

    if (pathFilter !== undefined) {
      return this._reloadWithPathFilter(pathFilter, reason, ctx);
    }

    return this._reloadSingleModule(moduleId!, reason, ctx);
  }

  private _validateReloadLifecycle(value: unknown): ManagedDrainPolicy {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
      throw new InvalidInputError('lifecycle must be an object');
    }
    for (const key of Object.keys(value)) {
      if (key !== 'drain_timeout_ms' && key !== 'on_drain_timeout') {
        throw new InvalidInputError(`Unrecognized reload lifecycle option '${key}'`);
      }
    }
    const options = value as Record<string, unknown>;
    const timeout = options['drain_timeout_ms'];
    const action = options['on_drain_timeout'];
    validateLifecycleDuration('drain_timeout_ms', timeout);
    if (action !== undefined && action !== 'abort_reload' && action !== 'continue_waiting') {
      throw new InvalidInputError('on_drain_timeout must be abort_reload or continue_waiting');
    }
    return { timeoutMs: (timeout as number | undefined) ?? 5_000, action: action ?? 'abort_reload' };
  }

  private async _reloadManaged(
    moduleId: string | undefined, pathFilter: string | undefined, reason: string,
    context: Context | null, policy: ManagedDrainPolicy,
  ): Promise<Record<string, unknown>> {
    const started = performance.now();
    const matchingIds = moduleId === undefined ? this._matchPathFilter(pathFilter!) : [moduleId];
    if (matchingIds.includes(RELOAD_MODULE_ID) && this._registry.has(RELOAD_MODULE_ID)) {
      throw new ReloadFailedError(moduleId ?? pathFilter!, 'The reload controller cannot reload itself');
    }
    const snapshots = matchingIds.flatMap(id => {
      const snapshot = this._registry.snapshotRegistration(id);
      if (!snapshot && moduleId !== undefined) throw new ModuleNotFoundError(id);
      if (!snapshot) return [];
      const instance = snapshot.module as Module;
      if (typeof instance.onDrain === 'function' && typeof instance.onDrainAborted !== 'function') {
        throw new InvalidInputError(`onDrainAborted is required to reload '${id}'`, { details: { module_id: id } });
      }
      return [snapshot];
    });
    const rows = snapshots.map(({ moduleId }): ModuleLifecycleProgress => ({ moduleId, phase: 'pending' }));
    const lifecycle = new LifecycleProgress('reload', rows, [this._registry], this._registry.lifecycleCallbacks);
    const versions = new Map<string, string>();
    const done = await lifecycle.run(async () => {
      for (const [index, snapshot] of snapshots.entries()) {
        const version = await this._registry.reloadManagedRegistration(snapshot, policy, lifecycle, rows[index]!, this._emitter);
        if (version === undefined) continue;
        versions.set(snapshot.moduleId, version);
        try {
          this._auditManagedReload(snapshot, version, context, lifecycle.report.operationId, reason);
        } catch {
          console.warn(`[apcore:audit] Could not record successful reload of ${snapshot.moduleId}`);
        }
      }
    }).then(() => lifecycle.operation.snapshot());

    const reloaded = done.modules.filter(row => row.outcome === 'reloaded').map(row => row.moduleId);
    const single = snapshots[0];
    return {
      success: done.status === 'completed', module_id: moduleId ?? null,
      reload_duration_ms: performance.now() - started, lifecycle_report: lifecycleReportToWire(done),
      ...(moduleId === undefined ? { reloaded_modules: reloaded } : {
        previous_version: single ? String(single.metadata['version'] ?? '1.0.0') : null,
        new_version: versions.get(moduleId) ?? null,
      }),
    };
  }

  private _auditManagedReload(snapshot: RegistrationSnapshot, after: string, context: Context | null, correlationId: string, reason: string): void {
    const { moduleId, metadata } = snapshot;
    const before = String(metadata['version'] ?? '1.0.0');
    const { caller_id, identity } = extractAuditIdentity(context);
    this._emitter.emit(createEvent('apcore.module.reloaded', moduleId, 'info', {
      module_id: moduleId, previous_version: before, new_version: after, caller_id,
      ...(identity === null ? {} : { identity }),
    }));
    const entry = buildAuditEntry('reload_module', moduleId, context, { before, after }, correlationId);
    if (this._auditStore !== null) this._auditStore.append(entry);
    else console.warn(`[apcore:audit] reload_module ${moduleId} actor=${entry.actorId} reason=${reason}`);
  }

  private async _reloadSingleModule(moduleId: string, reason: string, ctx: Context | null): Promise<Record<string, unknown>> {
    this._assertLegacyReloadAvailable([moduleId]);
    const existing = this._registry.snapshotRegistration(moduleId);
    if (existing === null) {
      throw new ModuleNotFoundError(moduleId);
    }
    if (moduleId === RELOAD_MODULE_ID) {
      throw new ReloadFailedError(moduleId, 'The reload controller cannot reload itself');
    }
    const existingObj = existing.module as Record<string, unknown>;
    const previousVersion = String(existingObj['version'] ?? '1.0.0');

    const start = performance.now();
    const states = await this._suspendAndUnload([moduleId]);
    const beforePublish = this._resumeBeforePublish(states);

    try {
      await this._registry.discover({
        reloadFiles: existing.sourceFile === undefined ? [] : [existing.sourceFile],
        beforePublish,
      });
    } catch (err) {
      await this._restoreMissingRegistrations([existing], beforePublish);
      throw new ReloadFailedError(moduleId, String(err));
    }

    const reloaded = this._registry.get(moduleId);
    if (reloaded === null) {
      await this._restoreMissingRegistrations([existing], beforePublish);
      throw new ReloadFailedError(moduleId, `Module '${moduleId}' was not found after re-discovery`);
    }

    const reloadedObj = reloaded as Record<string, unknown>;
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

  private async _suspendAndUnload(moduleIds: readonly string[]): Promise<Map<string, Record<string, unknown>>> {
    const states = new Map<string, Record<string, unknown>>();
    for (const id of moduleIds) {
      await this._registry.safeUnregister(id, undefined, async (module) => {
        const onSuspend = (module as Record<string, unknown>)['onSuspend'];
        if (typeof onSuspend !== 'function') return;
        const state = await onSuspend.call(module) as Record<string, unknown> | null | undefined;
        if (state != null) states.set(id, state);
      });
    }
    return states;
  }

  private _resumeBeforePublish(
    states: ReadonlyMap<string, Record<string, unknown>>,
  ): (moduleId: string, module: unknown) => Promise<void> {
    return async (moduleId, module) => {
      const state = states.get(moduleId);
      if (state === undefined) return;
      try {
        const onResume = (module as Record<string, unknown>)['onResume'];
        if (typeof onResume === 'function') await onResume.call(module, state);
      } catch (error) {
        console.warn(`[apcore:control] onResume failed for ${moduleId}:`, error);
      }
    };
  }

  private async _restoreMissingRegistrations(
    registrations: readonly RegistrationSnapshot[],
    beforePublish: (moduleId: string, module: unknown) => Promise<void>,
  ): Promise<void> {
    for (const registration of registrations) {
      if (this._registry.has(registration.moduleId)) continue;
      try {
        await this._registry.restoreRegistration(registration, beforePublish);
      } catch (error) {
        console.warn(`[apcore:control] Failed to restore '${registration.moduleId}' after reload failed:`, error);
      }
    }
  }

  private async _reloadWithPathFilter(pathFilter: string, reason: string, ctx: Context | null): Promise<Record<string, unknown>> {
    const started = Date.now();
    if (this._registry.has(RELOAD_MODULE_ID) && matchGlob(pathFilter, RELOAD_MODULE_ID)) {
      throw new ReloadFailedError(pathFilter, 'The reload controller cannot reload itself');
    }
    const matchingIds = this._matchPathFilter(pathFilter);
    this._assertLegacyReloadAvailable(matchingIds);
    if (matchingIds.length === 0) {
      return { success: true, module_id: null, reloaded_modules: [], reload_duration_ms: Date.now() - started };
    }

    return this._reloadMatching(matchingIds, pathFilter, reason, ctx, started);
  }

  private _matchPathFilter(pathFilter: string): string[] {
    const allIds = this._registry.list();
    // Alphabetical first so the topo sort has a deterministic input, then
    // dependency order on top (fixture `system_modules_hardening.json`
    // reload_with_path_filter: `reload_order: "topological"`).
    return this._topoSortModules(
      // PROTOCOL_SPEC 6.7 clause 4 / 9.2.3: `path_filter` is a GLOB-dialect
      // pattern (A25), not module-ID matching (A08). The two differ on `?`,
      // which A08 treats as a literal — so `executor.?mail.*` reloaded
      // nothing here while the other two SDKs reloaded two modules, and the
      // zero-match no-op rule made that silent (#117).
      allIds.filter((id) => matchGlob(pathFilter, id)).sort(),
    );
  }

  private async _reloadMatching(matchingIds: string[], pathFilter: string, reason: string, ctx: Context | null, started: number): Promise<Record<string, unknown>> {
    // Capture existing modules and versions before unregistering
    const existingModules: RegistrationSnapshot[] = [];
    const previousVersions = new Map<string, string>();
    for (const id of matchingIds) {
      const registration = this._registry.snapshotRegistration(id);
      if (registration !== null) {
        existingModules.push(registration);
        previousVersions.set(id, String((registration.module as Record<string, unknown>)['version'] ?? '1.0.0'));
      }
    }

    const states = await this._suspendAndUnload(matchingIds);
    const beforePublish = this._resumeBeforePublish(states);

    // Re-discover
    try {
      await this._registry.discover({
        reloadFiles: existingModules.flatMap(({ sourceFile }) => sourceFile === undefined ? [] : [sourceFile]),
        beforePublish,
      });
    } catch (err) {
      await this._restoreMissingRegistrations(existingModules, beforePublish);
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
      await this._restoreMissingRegistrations(existingModules, beforePublish);
      throw new ReloadFailedError(
        pathFilter,
        `${missing.length} module(s) were not found after re-discovery: ${missing.join(', ')}`,
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

  private _assertLegacyReloadAvailable(moduleIds: string[]): void {
    const reserved = moduleIds.find(id => this._registry.isManagedReloading(id));
    if (reserved !== undefined) throw new ReloadFailedError(reserved, 'A managed lifecycle operation owns this reload');
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
