import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import {
  ToggleState,
  DEFAULT_TOGGLE_STATE,
  isModuleDisabled,
  checkModuleDisabled,
  ToggleFeatureModule,
} from '../src/sys-modules/toggle.js';
import { InvalidInputError, ModuleNotFoundError, ModuleDisabledError } from '../src/errors.js';
import { Registry } from '../src/registry/registry.js';
import { EventEmitter } from '../src/events/emitter.js';
import { Executor } from '../src/executor.js';
import type { ApCoreEvent } from '../src/events/emitter.js';

describe('ToggleState', () => {
  let state: ToggleState;

  beforeEach(() => {
    state = new ToggleState();
  });

  it('reports module as not disabled by default', () => {
    expect(state.isDisabled('some.module')).toBe(false);
  });

  it('disables a module', () => {
    state.disable('my.module');
    expect(state.isDisabled('my.module')).toBe(true);
  });

  it('enables a previously disabled module', () => {
    state.disable('my.module');
    state.enable('my.module');
    expect(state.isDisabled('my.module')).toBe(false);
  });

  it('enable is a no-op for a module that is not disabled', () => {
    state.enable('unknown.module');
    expect(state.isDisabled('unknown.module')).toBe(false);
  });

  it('clears all disabled modules', () => {
    state.disable('a');
    state.disable('b');
    state.clear();
    expect(state.isDisabled('a')).toBe(false);
    expect(state.isDisabled('b')).toBe(false);
  });

  it('tracks multiple modules independently', () => {
    state.disable('a');
    state.disable('b');
    expect(state.isDisabled('a')).toBe(true);
    expect(state.isDisabled('b')).toBe(true);
    state.enable('a');
    expect(state.isDisabled('a')).toBe(false);
    expect(state.isDisabled('b')).toBe(true);
  });
});

describe('DEFAULT_TOGGLE_STATE', () => {
  afterEach(() => {
    DEFAULT_TOGGLE_STATE.clear();
  });

  it('is an instance of ToggleState', () => {
    expect(DEFAULT_TOGGLE_STATE).toBeInstanceOf(ToggleState);
  });
});

describe('isModuleDisabled', () => {
  afterEach(() => {
    DEFAULT_TOGGLE_STATE.clear();
  });

  it('returns false when module is not disabled', () => {
    expect(isModuleDisabled('some.module')).toBe(false);
  });

  it('delegates to DEFAULT_TOGGLE_STATE', () => {
    DEFAULT_TOGGLE_STATE.disable('some.module');
    expect(isModuleDisabled('some.module')).toBe(true);
  });

  it('returns false after module is re-enabled', () => {
    DEFAULT_TOGGLE_STATE.disable('some.module');
    DEFAULT_TOGGLE_STATE.enable('some.module');
    expect(isModuleDisabled('some.module')).toBe(false);
  });
});

describe('checkModuleDisabled', () => {
  afterEach(() => {
    DEFAULT_TOGGLE_STATE.clear();
  });

  it('does nothing when module is not disabled', () => {
    expect(() => checkModuleDisabled('ok.module')).not.toThrow();
  });

  it('throws ModuleDisabledError when module is disabled', () => {
    DEFAULT_TOGGLE_STATE.disable('bad.module');
    expect(() => checkModuleDisabled('bad.module')).toThrow(ModuleDisabledError);
  });
});

describe('ToggleFeatureModule', () => {
  let registry: Registry;
  let emitter: EventEmitter;
  let toggleState: ToggleState;
  let toggle: ToggleFeatureModule;
  let capturedEvents: ApCoreEvent[];

  beforeEach(() => {
    registry = new Registry();
    emitter = new EventEmitter();
    toggleState = new ToggleState();
    toggle = new ToggleFeatureModule(registry, emitter, toggleState);
    capturedEvents = [];

    emitter.subscribe({
      onEvent(event: ApCoreEvent) {
        capturedEvents.push(event);
      },
    });

    // Register a dummy module so toggle can find it
    registry.registerInternal('test.dummy', { description: 'dummy module for testing' });
  });

  afterEach(() => {
    DEFAULT_TOGGLE_STATE.clear();
  });

  describe('input validation', () => {
    it('throws InvalidInputError when module_id is missing', () => {
      expect(() => toggle.execute({ enabled: true, reason: 'test' }, null)).toThrow(InvalidInputError);
    });

    it('throws InvalidInputError when module_id is empty string', () => {
      expect(() => toggle.execute({ module_id: '', enabled: true, reason: 'test' }, null)).toThrow(InvalidInputError);
    });

    it('throws InvalidInputError when module_id is not a string', () => {
      expect(() => toggle.execute({ module_id: 123, enabled: true, reason: 'test' }, null)).toThrow(InvalidInputError);
    });

    it('throws InvalidInputError when enabled is missing', () => {
      expect(() => toggle.execute({ module_id: 'test.dummy', reason: 'test' }, null)).toThrow(InvalidInputError);
    });

    it('throws InvalidInputError when enabled is not a boolean', () => {
      expect(() => toggle.execute({ module_id: 'test.dummy', enabled: 'true', reason: 'test' }, null)).toThrow(InvalidInputError);
    });

    it('throws InvalidInputError when reason is missing', () => {
      expect(() => toggle.execute({ module_id: 'test.dummy', enabled: true }, null)).toThrow(InvalidInputError);
    });

    it('throws InvalidInputError when reason is empty string', () => {
      expect(() => toggle.execute({ module_id: 'test.dummy', enabled: true, reason: '' }, null)).toThrow(InvalidInputError);
    });

    it('throws InvalidInputError when reason is not a string', () => {
      expect(() => toggle.execute({ module_id: 'test.dummy', enabled: true, reason: 42 }, null)).toThrow(InvalidInputError);
    });
  });

  it('throws ModuleNotFoundError for an unregistered module', () => {
    expect(() =>
      toggle.execute({ module_id: 'nonexistent.module', enabled: false, reason: 'test' }, null),
    ).toThrow(ModuleNotFoundError);
  });

  it('disables a module', () => {
    toggle.execute({ module_id: 'test.dummy', enabled: false, reason: 'maintenance' }, null);
    expect(toggleState.isDisabled('test.dummy')).toBe(true);
  });

  it('enables a previously disabled module', () => {
    toggleState.disable('test.dummy');
    toggle.execute({ module_id: 'test.dummy', enabled: true, reason: 'back online' }, null);
    expect(toggleState.isDisabled('test.dummy')).toBe(false);
  });

  it('emits apcore.module.toggled event with enabled and identity context', () => {
    // W-12: event payload carries `enabled` (Python parity).
    // Issue #45.2: payload also carries `caller_id` (defaults to "@external")
    //              and `identity` (null when no Context is supplied).
    // `reason` stays in the module return value only.
    toggle.execute({ module_id: 'test.dummy', enabled: false, reason: 'shutting down' }, null);

    expect(capturedEvents).toHaveLength(1);
    const canonicalEvent = capturedEvents[0];
    expect(canonicalEvent.eventType).toBe('apcore.module.toggled');
    expect(canonicalEvent.moduleId).toBe('test.dummy');
    expect(canonicalEvent.severity).toBe('info');
    // Per docs/features/system-modules.md §"Contextual auditing", the
    // payload includes module_id and OMITS `identity` when the context
    // identity is null (do not emit `null`).
    expect(canonicalEvent.data).toEqual({
      module_id: 'test.dummy',
      enabled: false,
      caller_id: '@external',
    });
  });

  it('returns success result with module_id and enabled (no reason per spec)', () => {
    const result = toggle.execute(
      { module_id: 'test.dummy', enabled: false, reason: 'disabled for test' },
      null,
    );
    expect(result).toEqual({
      success: true,
      module_id: 'test.dummy',
      enabled: false,
    });
  });

  it('returns correct result when enabling a module', () => {
    toggleState.disable('test.dummy');
    const result = toggle.execute(
      { module_id: 'test.dummy', enabled: true, reason: 're-enabled' },
      null,
    );
    expect(result).toEqual({
      success: true,
      module_id: 'test.dummy',
      enabled: true,
    });
  });
});

describe('ToggleGate enforcement via Executor.call', () => {
  let reg: Registry;
  let executor: Executor;

  beforeEach(() => {
    reg = new Registry();
    reg.registerInternal('test.toggled', { execute: () => ({ ok: true }) });
    executor = new Executor({ registry: reg });
    DEFAULT_TOGGLE_STATE.clear();
  });

  afterEach(() => {
    DEFAULT_TOGGLE_STATE.clear();
  });

  it('throws ModuleDisabledError when a disabled module is called via the standard strategy', async () => {
    DEFAULT_TOGGLE_STATE.disable('test.toggled');
    await expect(executor.call('test.toggled', {})).rejects.toBeInstanceOf(ModuleDisabledError);
  });

  it('calls succeed when module is enabled', async () => {
    await expect(executor.call('test.toggled', {})).resolves.toMatchObject({ ok: true });
  });

  it('calls succeed after a module is re-enabled', async () => {
    DEFAULT_TOGGLE_STATE.disable('test.toggled');
    DEFAULT_TOGGLE_STATE.enable('test.toggled');
    await expect(executor.call('test.toggled', {})).resolves.toMatchObject({ ok: true });
  });
});
