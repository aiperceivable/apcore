import { describe, it, expect } from 'vitest';
import { Registry } from '../../src/registry/registry.js';
import { EventEmitter } from '../../src/events/emitter.js';
import { DuplicateModuleIdError, InvalidInputError } from '../../src/errors.js';

describe('Registry on_load ordering (#65)', () => {
  it('module with sync onLoad is visible immediately after register()', async () => {
    const registry = new Registry();
    let loaded = false;
    const mod = {
      id: 'test.sync',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      onLoad() { loaded = true; },
      async execute() { return {}; },
    };
    await registry.register('test.sync', mod);
    expect(loaded).toBe(true);
    expect(registry.get('test.sync')).toBe(mod);
  });

  // A-D-013: the in-flight slot must be reserved for sync onLoad too, so the
  // gate (conflict detection + has()/get() hiding) is effective uniformly —
  // Python/Rust reserve for sync and async alike. Previously the sync-onLoad
  // path published without reserving _inFlight, leaving the gate dead for it.
  // We observe the reservation two ways: (1) the id is present in _inFlight
  // during the sync onLoad and gone after publish; (2) a re-entrant same-ID
  // register() issued from inside onLoad is rejected as a duplicate (the slot
  // is held), whereas without the reservation it would not be detected.
  it('reserves the in-flight slot during sync onLoad and releases it after', async () => {
    const registry = new Registry();
    const inFlight = (registry as unknown as { _inFlight: Map<string, unknown> })._inFlight;

    let inFlightDuringLoad = false;
    let reentrantConflict = false;
    const mod = {
      id: 'test.syncgate',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      onLoad() {
        inFlightDuringLoad = inFlight.has('test.syncgate');
        // The held slot must be visible to conflict detection: a re-entrant
        // registration of the same ID during onLoad is a duplicate.
        try {
          registry.register('test.syncgate', { ...mod, onLoad: undefined });
        } catch (e) {
          reentrantConflict = e instanceof DuplicateModuleIdError;
        }
      },
      async execute() { return {}; },
    };

    await registry.register('test.syncgate', mod);

    expect(inFlightDuringLoad).toBe(true);
    expect(reentrantConflict).toBe(true);
    // Slot released and module published once register() resolves.
    expect(inFlight.has('test.syncgate')).toBe(false);
    expect(registry.has('test.syncgate')).toBe(true);
    expect(registry.get('test.syncgate')).toBe(mod);
  });

  it('module with async onLoad is NOT visible until onLoad resolves', async () => {
    const registry = new Registry();
    let resolveLoad!: () => void;
    const loadPromise = new Promise<void>((r) => { resolveLoad = r; });

    const mod = {
      id: 'test.async',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      async onLoad() { await loadPromise; },
      async execute() { return {}; },
    };

    // Start registration but do NOT await it yet
    const registerPromise = registry.register('test.async', mod);

    // Module should NOT be visible yet — get() returns null for an in-flight id
    // (A-D-002: cross-SDK canonical = null, matching getDefinition() parity and
    // the well-formed-unregistered → null contract).
    expect(registry.get('test.async')).toBeNull();
    expect(registry.getDefinition('test.async')).toBeNull();
    expect(registry.has('test.async')).toBe(false);

    // Resolve onLoad
    resolveLoad();
    await registerPromise;

    // Now the module should be visible
    expect(registry.get('test.async')).toBe(mod);
    expect(registry.has('test.async')).toBe(true);
  });

  it('concurrent registrations of same ID during onLoad throw DUPLICATE error', async () => {
    const registry = new Registry();
    let resolveLoad!: () => void;
    const loadPromise = new Promise<void>((r) => { resolveLoad = r; });

    const mod1 = {
      id: 'test.concurrent',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      async onLoad() { await loadPromise; },
      async execute() { return {}; },
    };
    const mod2 = {
      id: 'test.concurrent',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      async execute() { return {}; },
    };

    // Start first registration (in-flight, onLoad not done)
    const r1 = registry.register('test.concurrent', mod1);

    // Second registration of same ID while first is in-flight should throw DUPLICATE_MODULE_ID
    // (throws synchronously because conflict detection runs before async onLoad)
    expect(() => registry.register('test.concurrent', mod2)).toThrow(DuplicateModuleIdError);

    resolveLoad();
    await r1;
    expect(registry.get('test.concurrent')).toBe(mod1);
  });

  it('async onLoad failure rolls back registration', async () => {
    const registry = new Registry();
    const mod = {
      id: 'test.failload',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      async onLoad() { throw new Error('onLoad failed async'); },
      async execute() { return {}; },
    };

    await expect(registry.register('test.failload', mod)).rejects.toThrow('onLoad failed async');
    expect(registry.get('test.failload')).toBeNull();
    expect(registry.has('test.failload')).toBe(false);
  });

  it('onLoad failure emits apcore.registry.module_load_failed with required payload keys', async () => {
    const emitter = new EventEmitter();
    const registry = new Registry();
    registry.setEventEmitter(emitter);

    const captured: Array<{ eventType: string; data: Record<string, unknown> }> = [];
    emitter.subscribe({ onEvent: (ev) => { captured.push({ eventType: ev.eventType, data: ev.data }); } });

    const mod = {
      id: 'test.failemit',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      async onLoad() { throw new Error('init failed'); },
      async execute() { return {}; },
    };

    await expect(registry.register('test.failemit', mod)).rejects.toThrow('init failed');

    const dlqEvents = captured.filter((e) => e.eventType === 'apcore.registry.module_load_failed');
    expect(dlqEvents).toHaveLength(1);
    const payload = dlqEvents[0].data;
    expect(payload['module_id']).toBe('test.failemit');
    expect(payload['callback_name']).toBe('module.onLoad');
    expect(payload['error_type']).toBe('Error');
    expect(payload['error_message']).toBe('init failed');
    expect(typeof payload['timestamp']).toBe('string');
  });

  it('sync onLoad failure emits apcore.registry.module_load_failed (A-D-REG-001)', async () => {
    // Regression for A-D-REG-001: prior to v0.22.0 the sync-onLoad branch of
    // _registerWithOnLoad propagated the throw without emitting the
    // module_load_failed event, violating Issue #65's strong-guarantee
    // invariant that observers always see either a fully-published module
    // or the failure event.
    const emitter = new EventEmitter();
    const registry = new Registry();
    registry.setEventEmitter(emitter);

    const captured: Array<{ eventType: string; data: Record<string, unknown> }> = [];
    emitter.subscribe({ onEvent: (ev) => { captured.push({ eventType: ev.eventType, data: ev.data }); } });

    const mod = {
      id: 'test.syncfail',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      onLoad() { throw new Error('sync init failed'); },
      async execute() { return {}; },
    };

    await expect(registry.register('test.syncfail', mod)).rejects.toThrow('sync init failed');

    const dlqEvents = captured.filter((e) => e.eventType === 'apcore.registry.module_load_failed');
    expect(dlqEvents).toHaveLength(1);
    expect(dlqEvents[0].data['module_id']).toBe('test.syncfail');
    expect(dlqEvents[0].data['error_message']).toBe('sync init failed');

    // Module must not be visible after rollback
    expect(registry.get('test.syncfail')).toBeNull();
    expect(registry.has('test.syncfail')).toBe(false);
  });

  it('registerInternal sync onLoad failure emits module_load_failed and rolls back (A-D-REG-004)', () => {
    // Spec REG-003 / A-D-REG-004: the strong-guarantee invariant applies to
    // registerInternal too. On sync onLoad failure the module must NOT appear
    // in the visible map and module_load_failed must be emitted.
    const emitter = new EventEmitter();
    const registry = new Registry();
    registry.setEventEmitter(emitter);

    const captured: Array<{ eventType: string; data: Record<string, unknown> }> = [];
    emitter.subscribe({ onEvent: (ev) => { captured.push({ eventType: ev.eventType, data: ev.data }); } });

    const mod = {
      id: 'sys.failload',
      description: 'sys mod whose onLoad throws',
      onLoad() { throw new Error('boom-internal'); },
      execute() { return {}; },
    };

    expect(() => registry.registerInternal('sys.failload', mod)).toThrow('boom-internal');

    const events = captured.filter((e) => e.eventType === 'apcore.registry.module_load_failed');
    expect(events).toHaveLength(1);
    expect(events[0].data['module_id']).toBe('sys.failload');

    expect(registry.get('sys.failload')).toBeNull();
    expect(registry.has('sys.failload')).toBe(false);
  });

  it('discover()-path sync onLoad failure does not publish module (A-D-REG-003)', async () => {
    // Spec REG-003 / A-D-REG-003: deferred-publish applies to the discover
    // path as well. Before the fix, _registerImpl inserted into _modules
    // *before* calling onLoad, so a concurrent get() could observe a module
    // that later fails. After the fix the insert happens only on onLoad
    // success. The discover loop logs the failure and continues, but the
    // module_load_failed event MUST fire and the module MUST NOT appear in
    // the visible map.
    const emitter = new EventEmitter();
    const registry = new Registry();
    registry.setEventEmitter(emitter);

    const captured: Array<{ eventType: string; data: Record<string, unknown> }> = [];
    emitter.subscribe({ onEvent: (ev) => { captured.push({ eventType: ev.eventType, data: ev.data }); } });

    const failingMod = {
      id: 'discover.failload',
      description: 'discover mod whose onLoad throws',
      onLoad() { throw new Error('discover-boom'); },
      execute() { return {}; },
    };

    registry.setDiscoverer({
      async discover() {
        return [{ moduleId: 'discover.failload', module: failingMod }];
      },
    });

    const count = await registry.discover();
    expect(count).toBe(0); // failing module is not counted as registered

    expect(registry.get('discover.failload')).toBeNull();
    expect(registry.has('discover.failload')).toBe(false);

    const events = captured.filter((e) => e.eventType === 'apcore.registry.module_load_failed');
    expect(events).toHaveLength(1);
    expect(events[0].data['module_id']).toBe('discover.failload');
  });

  it('register() without async onLoad works synchronously when not awaited', () => {
    // Existing sync callers: no await, no async onLoad — should still work
    const registry = new Registry();
    const mod = {
      id: 'test.compat',
      description: 'test',
      inputSchema: {},
      outputSchema: {},
      async execute() { return {}; },
    };
    // Not awaiting — sync usage should be backward-compatible for modules without onLoad
    void registry.register('test.compat', mod);
    // Sync modules visible immediately (no async onLoad)
    expect(registry.get('test.compat')).toBe(mod);
  });
});
