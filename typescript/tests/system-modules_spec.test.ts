/**
 * Spec-traced contract tests for the apcore System Modules feature (TypeScript SDK).
 *
 * Source spec: apcore/docs/features/system-modules.md
 * Canonical suite mirrored: apcore-python/tests/test_system_modules_spec.py
 *
 * Contracts under test (the six `## Contract:` blocks):
 *   1. system.control.update_config   (UpdateConfigModule.execute)
 *   2. system.control.reload_module   (ReloadModule.execute)
 *   3. system.control.toggle_feature  (ToggleFeatureModule.execute)
 *   4. checkModuleDisabled / check_module_disabled
 *   5. isModuleDisabled / is_module_disabled
 *   6. registerSysModules / register_sys_modules
 *
 * Each `it(...)` name carries the verbatim clause id formatted
 * `system_modules.<method>.<kind>.<detail>` so cross-language diffs line up
 * row-for-row with the canonical Python suite. These tests are READ-ONLY
 * contract verification — they never modify production source.
 *
 * Notable TS/Python divergences (asserted as the ACTUAL TS behavior, not faked):
 *   - update_config sensitive redaction: TS uses the literal sentinel `'***'`.
 *   - reload_module: TS `ReloadModule.execute` is ASYNC (returns a Promise),
 *     unlike the synchronous Python implementation.
 *   - annotations: TS uses `requiresApproval` (camelCase), not `requires_approval`.
 *   - check/isModuleDisabled: single-arg (moduleId) using the global
 *     DEFAULT_TOGGLE_STATE — same single-arg divergence as Python; the spec's
 *     two-arg `(module_id, registry)` signature does not exist. Skipped.
 *   - registerSysModules: positional args (registry, executor, config,
 *     metricsCollector?, options?) and a sync function. fail_on_error lives in
 *     the options object as `failOnError`.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';

import { Config } from '../src/config.js';
import {
  ConfigError,
  InvalidInputError,
  ModuleDisabledError,
  ModuleError,
  ModuleNotFoundError,
  ModuleReloadConflictError,
} from '../src/errors.js';
import { EventEmitter, createEvent } from '../src/events/emitter.js';
import type { ApCoreEvent } from '../src/events/emitter.js';
import { Executor } from '../src/executor.js';
import { Registry } from '../src/registry/registry.js';
import {
  ToggleFeatureModule,
  ToggleState,
  DEFAULT_TOGGLE_STATE,
  checkModuleDisabled,
  isModuleDisabled,
} from '../src/sys-modules/toggle.js';
import { UpdateConfigModule, ReloadModule } from '../src/sys-modules/control.js';
import { registerSysModules } from '../src/sys-modules/registration.js';

// ---------------------------------------------------------------------------
// Helpers / fixtures
// ---------------------------------------------------------------------------

/** EventEmitter subclass that records every emitted event. */
class RecordingEmitter extends EventEmitter {
  readonly events: ApCoreEvent[] = [];

  override emit(event: ApCoreEvent): void {
    this.events.push(event);
    super.emit(event);
  }
}

/** A minimal registrable module for toggle/reload tests. */
class DummyModule {
  readonly inputSchema = { type: 'object' as const };
  readonly outputSchema = { type: 'object' as const };
  readonly description = 'dummy';
  readonly version = '1.0.0';
  execute(_inputs: Record<string, unknown>, _context: unknown): Record<string, unknown> {
    return {};
  }
}

function makeConfig(overrides: Record<string, unknown> = {}): Config {
  const config = Config.fromDefaults();
  for (const [key, value] of Object.entries(overrides)) {
    config.set(key, value);
  }
  return config;
}

function updateModule(config?: Config, emitter?: EventEmitter): UpdateConfigModule {
  return new UpdateConfigModule(config ?? Config.fromDefaults(), emitter ?? new EventEmitter());
}

function toggleModule(
  registry: Registry,
  emitter?: EventEmitter,
  toggleState?: ToggleState,
): ToggleFeatureModule {
  return new ToggleFeatureModule(registry, emitter ?? new EventEmitter(), toggleState ?? new ToggleState());
}

function registryWithModule(moduleId = 'math.add'): Registry {
  const reg = new Registry();
  reg.registerInternal(moduleId, new DummyModule());
  return reg;
}

// Keep the module-global default toggle state clean between tests.
beforeEach(() => DEFAULT_TOGGLE_STATE.clear());
afterEach(() => DEFAULT_TOGGLE_STATE.clear());

// ===========================================================================
// Contract 1: system.control.update_config  (UpdateConfigModule.execute)
// ===========================================================================

describe('Contract: system.control.update_config', () => {
  it('system_modules.update_config.input.key_required: missing/empty key -> InvalidInputError', () => {
    const mod = updateModule();
    expect(() => mod.execute({ key: '', value: 1, reason: 'r' }, null)).toThrow(InvalidInputError);
  });

  it('system_modules.update_config.input.reason_required: missing/empty reason -> InvalidInputError', () => {
    const mod = updateModule();
    expect(() =>
      mod.execute({ key: 'executor.default_timeout', value: 1, reason: '' }, null),
    ).toThrow(InvalidInputError);
  });

  it('system_modules.update_config.input.value_any_accepted: value accepts any JSON value (no input-time validation)', () => {
    const config = makeConfig();
    const mod = updateModule(config);
    const result = mod.execute(
      { key: 'some.arbitrary.field', value: { nested: [1, 2, 3] }, reason: 'r' },
      null,
    );
    expect(result['new_value']).toEqual({ nested: [1, 2, 3] });
  });

  it('system_modules.update_config.error.config_key_restricted: updating a restricted key raises ModuleError(CONFIG_KEY_RESTRICTED)', () => {
    // Aligned with apcore-python: restricted keys raise ModuleError with code
    // CONFIG_KEY_RESTRICTED (not ConfigError/CONFIG_INVALID).
    const mod = updateModule();
    let caught: unknown;
    try {
      mod.execute({ key: 'sys_modules.enabled', value: false, reason: 'r' }, null);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(ModuleError);
    expect((caught as ModuleError).code).toBe('CONFIG_KEY_RESTRICTED');
  });

  it('system_modules.update_config.error.config_constraint: constraint violation raises ConfigError', () => {
    // Aligned with apcore-python: a post-set constraint violation raises
    // ConfigError after rolling Config back to the old value.
    const config = makeConfig();
    const mod = updateModule(config);
    let caught: unknown;
    try {
      mod.execute({ key: 'executor.default_timeout', value: -5, reason: 'r' }, null);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(ConfigError);
    expect((caught as ModuleError).code).toBe('CONFIG_INVALID');
  });

  it('system_modules.update_config.side_effect.rollback_on_constraint: Config is rolled back to old_value on constraint violation', () => {
    // Aligned with apcore-python: on a constraint violation the post-set check
    // rolls Config back to old_value before raising.
    const config = makeConfig();
    const mod = updateModule(config);
    const oldValue = config.get('executor.default_timeout');
    expect(() =>
      mod.execute({ key: 'executor.default_timeout', value: -5, reason: 'r' }, null),
    ).toThrow(ConfigError);
    expect(config.get('executor.default_timeout')).toBe(oldValue);
  });

  it('system_modules.update_config.side_effect.set_and_emit_event: on success Config is mutated and apcore.config.updated is emitted', () => {
    const config = makeConfig();
    const emitter = new RecordingEmitter();
    const mod = updateModule(config, emitter);
    mod.execute({ key: 'executor.default_timeout', value: 60000, reason: 'r' }, null);
    expect(config.get('executor.default_timeout')).toBe(60000);
    const types = emitter.events.map((e) => e.eventType);
    expect(types).toContain('apcore.config.updated');
  });

  it('system_modules.update_config.return.success_shape: returns {success, key, old_value, new_value}', () => {
    const config = makeConfig();
    const mod = updateModule(config);
    const result = mod.execute({ key: 'executor.default_timeout', value: 60000, reason: 'r' }, null);
    expect(result['success']).toBe(true);
    expect(result['key']).toBe('executor.default_timeout');
    expect(result['old_value']).toBe(30000);
    expect(result['new_value']).toBe(60000);
  });

  it('system_modules.update_config.return.redacts_sensitive_segments: sensitive key segments redact old_value/new_value', () => {
    const config = makeConfig();
    const mod = updateModule(config);
    const result = mod.execute({ key: 'platform.api_token', value: 'supersecret', reason: 'r' }, null);
    expect(result['new_value']).not.toBe('supersecret');
    expect(result['old_value']).not.toBe('supersecret');
  });

  it('system_modules.update_config.property.idempotent_false: repeated calls with different values produce different state', () => {
    const config = makeConfig();
    const mod = updateModule(config);
    mod.execute({ key: 'executor.default_timeout', value: 1000, reason: 'r' }, null);
    mod.execute({ key: 'executor.default_timeout', value: 2000, reason: 'r' }, null);
    expect(config.get('executor.default_timeout')).toBe(2000);
  });

  it('system_modules.update_config.property.pure_false: mutates Config state (not pure)', () => {
    const config = makeConfig();
    const before = config.get('executor.default_timeout');
    const mod = updateModule(config);
    mod.execute({ key: 'executor.default_timeout', value: 4444, reason: 'r' }, null);
    expect(config.get('executor.default_timeout')).not.toBe(before);
  });

  it('system_modules.update_config.property.async_false: execute is synchronous (does not return a Promise)', () => {
    const config = makeConfig();
    const mod = updateModule(config);
    const result = mod.execute({ key: 'executor.default_timeout', value: 5000, reason: 'r' }, null);
    expect(result).not.toBeInstanceOf(Promise);
    expect(typeof (result as { then?: unknown }).then).not.toBe('function');
  });
});

// ===========================================================================
// Contract 2: system.control.reload_module  (ReloadModule.execute)
// ===========================================================================

describe('Contract: system.control.reload_module', () => {
  function module(registry: Registry, emitter?: EventEmitter): ReloadModule {
    return new ReloadModule(registry, emitter ?? new EventEmitter());
  }

  it('system_modules.reload_module.input.module_id_required: missing module_id and path_filter -> InvalidInputError', async () => {
    const mod = module(new Registry());
    await expect(mod.execute({ reason: 'r' }, null)).rejects.toBeInstanceOf(InvalidInputError);
  });

  it('system_modules.reload_module.input.reason_required: missing/empty reason -> InvalidInputError', async () => {
    const mod = module(registryWithModule('math.add'));
    await expect(mod.execute({ module_id: 'math.add', reason: '' }, null)).rejects.toBeInstanceOf(
      InvalidInputError,
    );
  });

  it('system_modules.reload_module.error.module_not_found: absent module_id -> ModuleNotFoundError', async () => {
    const mod = module(new Registry());
    await expect(
      mod.execute({ module_id: 'missing.module', reason: 'r' }, null),
    ).rejects.toBeInstanceOf(ModuleNotFoundError);
  });

  it('system_modules.reload_module.error.reload_conflict: both module_id and path_filter -> ModuleReloadConflictError (code MODULE_RELOAD_CONFLICT)', async () => {
    const mod = module(registryWithModule('math.add'));
    let caught: unknown;
    try {
      await mod.execute({ module_id: 'math.add', path_filter: 'math.*', reason: 'r' }, null);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(ModuleReloadConflictError);
    expect((caught as ModuleError).code).toBe('MODULE_RELOAD_CONFLICT');
  });

  it('system_modules.reload_module.property.async_false: TS execute IS async (returns a Promise) — DIVERGENCE from sync Python', () => {
    // DIVERGENCE: Python's ReloadModule.execute is synchronous; the TS
    // implementation is async (performs awaited safeUnregister/discover).
    const mod = module(registryWithModule('math.add'));
    const result = mod.execute({ module_id: 'math.add', path_filter: 'x.*', reason: 'r' }, null);
    expect(typeof (result as { then?: unknown }).then).toBe('function');
    // Swallow the rejection (conflict) to avoid an unhandled promise rejection.
    void (result as Promise<unknown>).catch(() => undefined);
  });

  it('system_modules.reload_module.property.requires_approval: annotation requiresApproval=true (control module)', () => {
    const mod = module(registryWithModule('math.add'));
    expect(mod.annotations.requiresApproval).toBe(true);
  });
});

// ===========================================================================
// Contract 3: system.control.toggle_feature  (ToggleFeatureModule.execute)
// ===========================================================================

describe('Contract: system.control.toggle_feature', () => {
  it('system_modules.toggle_feature.input.module_id_required: missing/empty module_id -> InvalidInputError', () => {
    const mod = toggleModule(new Registry());
    expect(() => mod.execute({ module_id: '', enabled: false, reason: 'r' }, null)).toThrow(
      InvalidInputError,
    );
  });

  it('system_modules.toggle_feature.input.enabled_must_be_bool: non-bool enabled (string) -> InvalidInputError', () => {
    const mod = toggleModule(registryWithModule('math.add'));
    expect(() => mod.execute({ module_id: 'math.add', enabled: 'false', reason: 'r' }, null)).toThrow(
      InvalidInputError,
    );
  });

  it('system_modules.toggle_feature.input.enabled_int_rejected: integer 1 is not a bool -> InvalidInputError', () => {
    const mod = toggleModule(registryWithModule('math.add'));
    expect(() => mod.execute({ module_id: 'math.add', enabled: 1, reason: 'r' }, null)).toThrow(
      InvalidInputError,
    );
  });

  it('system_modules.toggle_feature.input.reason_required: missing/empty reason -> InvalidInputError', () => {
    const mod = toggleModule(registryWithModule('math.add'));
    expect(() => mod.execute({ module_id: 'math.add', enabled: false, reason: '' }, null)).toThrow(
      InvalidInputError,
    );
  });

  it('system_modules.toggle_feature.error.module_not_found: unregistered module_id -> ModuleNotFoundError', () => {
    const mod = toggleModule(new Registry());
    expect(() => mod.execute({ module_id: 'ghost.module', enabled: false, reason: 'r' }, null)).toThrow(
      ModuleNotFoundError,
    );
  });

  it('system_modules.toggle_feature.side_effect.disable_sets_state: enabled=false marks module disabled in ToggleState', () => {
    const reg = registryWithModule('math.add');
    const state = new ToggleState();
    const mod = toggleModule(reg, undefined, state);
    mod.execute({ module_id: 'math.add', enabled: false, reason: 'r' }, null);
    expect(state.isDisabled('math.add')).toBe(true);
  });

  it('system_modules.toggle_feature.side_effect.enable_clears_state: enabled=true clears disabled state', () => {
    const reg = registryWithModule('math.add');
    const state = new ToggleState();
    state.disable('math.add');
    const mod = toggleModule(reg, undefined, state);
    mod.execute({ module_id: 'math.add', enabled: true, reason: 'r' }, null);
    expect(state.isDisabled('math.add')).toBe(false);
  });

  it('system_modules.toggle_feature.side_effect.emits_toggled_event: emits apcore.module.toggled', () => {
    const reg = registryWithModule('math.add');
    const emitter = new RecordingEmitter();
    const mod = toggleModule(reg, emitter);
    mod.execute({ module_id: 'math.add', enabled: false, reason: 'r' }, null);
    const types = emitter.events.map((e) => e.eventType);
    expect(types).toContain('apcore.module.toggled');
  });

  it('system_modules.toggle_feature.return.success_shape: returns {success, module_id, enabled}', () => {
    const reg = registryWithModule('math.add');
    const mod = toggleModule(reg);
    const result = mod.execute({ module_id: 'math.add', enabled: false, reason: 'r' }, null);
    expect(result).toEqual({ success: true, module_id: 'math.add', enabled: false });
  });

  it('system_modules.toggle_feature.property.idempotent_true: toggling to the current state twice yields the same outcome', () => {
    const reg = registryWithModule('math.add');
    const state = new ToggleState();
    const mod = toggleModule(reg, undefined, state);
    const r1 = mod.execute({ module_id: 'math.add', enabled: false, reason: 'r' }, null);
    const r2 = mod.execute({ module_id: 'math.add', enabled: false, reason: 'r' }, null);
    expect(r1).toEqual(r2);
    expect(state.isDisabled('math.add')).toBe(true);
  });

  it('system_modules.toggle_feature.property.thread_safe: concurrent (>=8) toggles via Promise.all all land disabled', async () => {
    const moduleIds = Array.from({ length: 8 }, (_, i) => `mod.m${i}`);
    const reg = new Registry();
    for (const mid of moduleIds) {
      reg.registerInternal(mid, new DummyModule());
    }
    const state = new ToggleState();
    const mod = toggleModule(reg, undefined, state);

    const results = await Promise.all(
      moduleIds.map((mid) =>
        Promise.resolve().then(() =>
          mod.execute({ module_id: mid, enabled: false, reason: 'r' }, null),
        ),
      ),
    );
    expect(results).toHaveLength(8);
    for (const mid of moduleIds) {
      expect(state.isDisabled(mid)).toBe(true);
    }
  });

  it('system_modules.toggle_feature.property.async_false: execute is synchronous (does not return a Promise)', () => {
    const reg = registryWithModule('math.add');
    const mod = toggleModule(reg);
    const result = mod.execute({ module_id: 'math.add', enabled: false, reason: 'r' }, null);
    expect(typeof (result as { then?: unknown }).then).not.toBe('function');
  });

  it('system_modules.toggle_feature.property.requires_approval: annotation requiresApproval=true (control module)', () => {
    const reg = registryWithModule('math.add');
    const mod = toggleModule(reg);
    expect(mod.annotations.requiresApproval).toBe(true);
  });
});

// ===========================================================================
// Contract 4: checkModuleDisabled / check_module_disabled
// ===========================================================================

describe('Contract: checkModuleDisabled', () => {
  it('system_modules.check_module_disabled.error.module_disabled: raises ModuleDisabledError (code MODULE_DISABLED) when disabled', () => {
    DEFAULT_TOGGLE_STATE.disable('risky.module');
    let caught: unknown;
    try {
      checkModuleDisabled('risky.module');
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(ModuleDisabledError);
    expect((caught as ModuleError).code).toBe('MODULE_DISABLED');
  });

  it('system_modules.check_module_disabled.return.none_when_enabled: returns undefined (does not throw) when enabled', () => {
    expect(checkModuleDisabled('enabled.module')).toBeUndefined();
  });

  it('system_modules.check_module_disabled.property.pure_read_only: reads toggle state only; does not mutate disabled state', () => {
    DEFAULT_TOGGLE_STATE.disable('x.mod');
    expect(() => checkModuleDisabled('x.mod')).toThrow(ModuleDisabledError);
    // State unchanged after the read-only check.
    expect(isModuleDisabled('x.mod')).toBe(true);
  });

  it.skip('system_modules.check_module_disabled.input.registry_param: missing symbol — TS checkModuleDisabled is single-arg (moduleId), no registry parameter', () => {
    // DIVERGENCE: spec declares a two-arg signature (module_id, registry); the
    // TS implementation is single-arg using the global DEFAULT_TOGGLE_STATE.
    // No `registry` parameter exists. Skipped (matches Python's skip).
    expect(checkModuleDisabled.length).toBe(2);
  });
});

// ===========================================================================
// Contract 5: isModuleDisabled / is_module_disabled
// ===========================================================================

describe('Contract: isModuleDisabled', () => {
  it('system_modules.is_module_disabled.return.true_when_disabled: returns true for a disabled module', () => {
    DEFAULT_TOGGLE_STATE.disable('dm');
    expect(isModuleDisabled('dm')).toBe(true);
  });

  it('system_modules.is_module_disabled.return.false_when_enabled: returns false for an enabled module', () => {
    expect(isModuleDisabled('em')).toBe(false);
  });

  it('system_modules.is_module_disabled.error.never_raises_unknown: never throws; returns false for unknown module IDs', () => {
    expect(isModuleDisabled('totally.unknown.module.id')).toBe(false);
  });

  it('system_modules.is_module_disabled.property.pure_read_only: pure read; repeated calls are stable and side-effect free', () => {
    DEFAULT_TOGGLE_STATE.disable('p.mod');
    expect(isModuleDisabled('p.mod')).toBe(true);
    expect(isModuleDisabled('p.mod')).toBe(true);
  });

  it.skip('system_modules.is_module_disabled.input.registry_param: missing symbol — TS isModuleDisabled is single-arg (moduleId), no registry parameter', () => {
    // DIVERGENCE: spec declares a two-arg signature (module_id, registry); the
    // TS implementation is single-arg using the global DEFAULT_TOGGLE_STATE.
    // No `registry` parameter exists. Skipped (matches Python's skip).
    expect(isModuleDisabled.length).toBe(2);
  });
});

// ===========================================================================
// Contract 6: registerSysModules / register_sys_modules
// ===========================================================================

describe('Contract: registerSysModules', () => {
  function makeExecutor(registry: Registry): Executor {
    return new Executor({ registry });
  }

  it('system_modules.register_sys_modules.side_effect.disabled_returns_empty: when sys_modules.enabled is false, returns an empty context (exit early)', () => {
    const registry = new Registry();
    const executor = makeExecutor(registry);
    const config = makeConfig(); // sys_modules.enabled defaults to false
    const result = registerSysModules(registry, executor, config);
    expect(result).toEqual({});
  });

  it('system_modules.register_sys_modules.return.context_components: when enabled, returns context with errorHistory, errorHistoryMiddleware, usageCollector, usageMiddleware', () => {
    const registry = new Registry();
    const executor = makeExecutor(registry);
    const config = makeConfig({ 'sys_modules.enabled': true });
    const result = registerSysModules(registry, executor, config);
    for (const key of ['errorHistory', 'errorHistoryMiddleware', 'usageCollector', 'usageMiddleware'] as const) {
      expect(result[key], `missing context key: ${key}`).toBeDefined();
    }
  });

  it('system_modules.register_sys_modules.side_effect.registers_health_modules: when enabled, health/manifest system modules are registered', () => {
    const registry = new Registry();
    const executor = makeExecutor(registry);
    const config = makeConfig({ 'sys_modules.enabled': true });
    registerSysModules(registry, executor, config);
    expect(registry.has('system.health.summary')).toBe(true);
    expect(registry.has('system.manifest.full')).toBe(true);
  });

  it('system_modules.register_sys_modules.input.fail_on_error_default: accepts failOnError (default false) per the 1.5 hardening rule', () => {
    // TS exposes fail_on_error as `failOnError` inside the options object.
    // Default-false behavior is verified: a duplicate registration with the
    // default options does NOT throw (errors logged and swallowed).
    const registry = new Registry();
    const executor = makeExecutor(registry);
    const config = makeConfig({ 'sys_modules.enabled': true });
    registerSysModules(registry, executor, config);
    expect(() => registerSysModules(registry, executor, config)).not.toThrow();
  });

  it('system_modules.register_sys_modules.property.idempotent_false: registering twice with failOnError throws (modules already registered)', () => {
    const registry = new Registry();
    const executor = makeExecutor(registry);
    const config = makeConfig({ 'sys_modules.enabled': true });
    registerSysModules(registry, executor, config);
    expect(() => registerSysModules(registry, executor, config, null, { failOnError: true })).toThrow();
  });

  it('system_modules.register_sys_modules.property.async_false: registerSysModules is synchronous (does not return a Promise)', () => {
    const registry = new Registry();
    const executor = makeExecutor(registry);
    const config = makeConfig();
    const result = registerSysModules(registry, executor, config);
    expect(typeof (result as { then?: unknown }).then).not.toBe('function');
  });
});
