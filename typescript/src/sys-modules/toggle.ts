/**
 * Module toggle system -- disable/enable modules without unloading.
 *
 * Implements system.control.toggle_feature as defined in PROTOCOL_SPEC.md:
 * - Error code: MODULE_DISABLED (HTTP 403)
 * - Canonical event: apcore.module.toggled
 * - See spec sections: "Error Codes" and "Canonical Event Types"
 */

import { InvalidInputError, ModuleNotFoundError, ModuleDisabledError } from '../errors.js';
import type { Registry } from '../registry/registry.js';
import type { EventEmitter } from '../events/emitter.js';
import { createEvent } from '../events/emitter.js';
import type { Context } from '../context.js';
import type { AuditStore } from './audit.js';
import { buildAuditEntry, extractAuditIdentity } from './audit.js';
import type { OverridesStore } from './overrides.js';

/**
 * Toggle state container. Tracks which modules are disabled.
 * State survives module reload since it lives outside the Registry.
 */
export class ToggleState {
  private readonly _disabled: Set<string> = new Set();

  isDisabled(moduleId: string): boolean {
    return this._disabled.has(moduleId);
  }

  disable(moduleId: string): void {
    this._disabled.add(moduleId);
  }

  enable(moduleId: string): void {
    this._disabled.delete(moduleId);
  }

  clear(): void {
    this._disabled.clear();
  }
}

/** Default global toggle state. */
export const DEFAULT_TOGGLE_STATE = new ToggleState();

/** Check if a module is disabled using the default toggle state. */
export function isModuleDisabled(moduleId: string): boolean {
  return DEFAULT_TOGGLE_STATE.isDisabled(moduleId);
}

/** Throw ModuleDisabledError if the module is disabled. */
export function checkModuleDisabled(moduleId: string): void {
  if (isModuleDisabled(moduleId)) {
    throw new ModuleDisabledError(moduleId);
  }
}

/**
 * Disable or enable a module without unloading it from the Registry.
 * A disabled module remains registered but calls return MODULE_DISABLED error.
 * @internal
 */
export class ToggleFeatureModule {
  readonly description = 'Disable or enable a module without unloading it';
  // SYS-19: toggle_feature IS idempotent — setting a module to the state it
  // is already in changes nothing. Its two `system.control.*` siblings are
  // not, and now say so.
  readonly annotations = { readonly: false, destructive: false, idempotent: true, requiresApproval: true, openWorld: false, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const };
  readonly inputSchema = {
    type: 'object' as const,
    properties: {
      module_id: { type: 'string' as const, description: 'ID of the module to toggle' },
      enabled: { type: 'boolean' as const, description: 'True to enable, false to disable' },
      reason: { type: 'string' as const, description: 'Audit reason for the toggle' },
    },
    required: ['module_id', 'enabled', 'reason'],
  };
  readonly outputSchema = {
    type: 'object' as const,
    properties: {
      success: { type: 'boolean' as const, description: 'Whether the toggle succeeded' },
      module_id: { type: 'string' as const, description: 'ID of the toggled module' },
      enabled: { type: 'boolean' as const, description: 'Current enabled state' },
    },
    required: ['success', 'module_id', 'enabled'],
  };

  private readonly _registry: Registry;
  private readonly _emitter: EventEmitter;
  private readonly _toggleState: ToggleState;
  private readonly _auditStore: AuditStore | null;
  private readonly _overridesStore: OverridesStore | null;

  constructor(
    registry: Registry,
    eventEmitter: EventEmitter,
    toggleState?: ToggleState,
    auditStore?: AuditStore,
    overridesStore?: OverridesStore,
  ) {
    this._registry = registry;
    this._emitter = eventEmitter;
    this._toggleState = toggleState ?? DEFAULT_TOGGLE_STATE;
    this._auditStore = auditStore ?? null;
    this._overridesStore = overridesStore ?? null;
  }

  execute(inputs: Record<string, unknown>, context: unknown): Record<string, unknown> {
    const { moduleId, enabled, reason } = this._validateInputs(inputs);
    const ctx = context as Context | null;

    if (!this._registry.has(moduleId)) {
      throw new ModuleNotFoundError(moduleId);
    }

    const before = !this._toggleState.isDisabled(moduleId);
    if (enabled) {
      this._toggleState.enable(moduleId);
    } else {
      this._toggleState.disable(moduleId);
    }

    const entry = buildAuditEntry('toggle_feature', moduleId, ctx, { before, after: enabled });
    if (this._auditStore !== null) {
      this._auditStore.append(entry);
    } else {
      console.warn(`[apcore:audit] toggle_feature ${moduleId} actor=${entry.actorId} before=${before} after=${enabled} reason=${reason}`);
    }

    // W-12: Event payload carries `enabled` (Python parity) plus requester
    // identity per Issue #45.2 so subscribers can attribute the toggle. The
    // `reason` field is returned in the module output but not in the event.
    const { caller_id, identity } = extractAuditIdentity(ctx);
    const toggledPayload: Record<string, unknown> = {
      module_id: moduleId,
      enabled,
      caller_id,
    };
    if (identity !== null) toggledPayload['identity'] = identity;
    this._emitter.emit(createEvent('apcore.module.toggled', moduleId, 'info', toggledPayload));
    console.warn(`[apcore:control] Feature toggled: module_id=${moduleId} enabled=${enabled} reason=${reason}`);

    if (this._overridesStore !== null) {
      this._persistToggleOverride(moduleId, enabled);
    }

    return { success: true, module_id: moduleId, enabled };
  }

  private _persistToggleOverride(moduleId: string, enabled: boolean): void {
    const store = this._overridesStore!;
    const overrideKey = `toggle.${moduleId}`;
    try {
      const loaded = store.load();
      if (loaded !== null && typeof (loaded as { then?: unknown }).then === 'function') {
        (loaded as Promise<Record<string, unknown>>)
          .then((existing) => {
            existing[overrideKey] = enabled;
            return store.save(existing);
          })
          .catch((err: unknown) => {
            console.warn(`[apcore:control] Failed to persist toggle override for '${moduleId}':`, err);
          });
        return;
      }
      const existing = loaded as Record<string, unknown>;
      existing[overrideKey] = enabled;
      const saveResult = store.save(existing);
      if (saveResult !== undefined && typeof (saveResult as { then?: unknown }).then === 'function') {
        (saveResult as Promise<void>).catch((err: unknown) => {
          console.warn(`[apcore:control] Failed to persist toggle override for '${moduleId}':`, err);
        });
      }
    } catch (err) {
      console.warn(`[apcore:control] Failed to persist toggle override for '${moduleId}':`, err);
    }
  }

  private _validateInputs(inputs: Record<string, unknown>): { moduleId: string; enabled: boolean; reason: string } {
    const moduleId = inputs['module_id'];
    if (typeof moduleId !== 'string' || !moduleId) {
      throw new InvalidInputError("'module_id' is required and must be a non-empty string");
    }
    const enabled = inputs['enabled'];
    if (typeof enabled !== 'boolean') {
      throw new InvalidInputError("'enabled' is required and must be a boolean");
    }
    const reason = inputs['reason'];
    if (typeof reason !== 'string' || !reason) {
      throw new InvalidInputError("'reason' is required and must be a non-empty string");
    }
    return { moduleId, enabled, reason };
  }
}
