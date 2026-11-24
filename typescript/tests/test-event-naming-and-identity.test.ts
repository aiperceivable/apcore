/**
 * Tests for Issue #36 (canonical event prefixes) and Issue #45.2
 * (contextual auditing — auto-extract caller_id / identity).
 *
 * Issue #36: 4 events that previously lacked the canonical
 *   `apcore.<subsystem>.<event>` prefix are now emitted only under their
 *   canonical names. v0.22.0 removed the legacy aliases and the
 *   `emitWithLegacy()` helper.
 *
 * Issue #45.2: control modules must extract `caller_id` (default
 *   `"@external"`) and `identity` from the execution Context and include
 *   them in their audit event payloads.
 */

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { EventEmitter } from '../src/events/emitter.js';
import type { ApCoreEvent } from '../src/events/emitter.js';
import { FilterSubscriber } from '../src/events/subscribers.js';
import { PlatformNotifyMiddleware } from '../src/middleware/platform-notify.js';
import { MetricsCollector } from '../src/observability/metrics.js';
import { Context, createIdentity } from '../src/context.js';
import { Config } from '../src/config.js';
import { Registry } from '../src/registry/registry.js';
import { UpdateConfigModule, ReloadModule } from '../src/sys-modules/control.js';
import { ToggleFeatureModule } from '../src/sys-modules/toggle.js';
import { Executor } from '../src/executor.js';
import { registerSysModules } from '../src/sys-modules/registration.js';

// ---------------------------------------------------------------------------
// Issue #36 — canonical + legacy event names
// ---------------------------------------------------------------------------

describe('Issue #36 — canonical event prefixes (legacy aliases removed in v0.22.0)', () => {
  describe('PlatformNotifyMiddleware emits canonical events only', () => {
    it('emits apcore.health.error_threshold_exceeded (no legacy alias)', () => {
      const emitter = new EventEmitter();
      const events: ApCoreEvent[] = [];
      emitter.subscribe({ onEvent: (e) => { events.push(e); } });

      const metrics = new MetricsCollector();
      for (let i = 0; i < 8; i++) metrics.incrementCalls('mod.a', 'success');
      for (let i = 0; i < 2; i++) metrics.incrementCalls('mod.a', 'error');

      const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1);
      mw.onError('mod.a', {}, new Error('boom'), Context.create());

      const canonical = events.find(e => e.eventType === 'apcore.health.error_threshold_exceeded');
      const legacy = events.find(e => e.eventType === 'error_threshold_exceeded');
      expect(canonical).toBeDefined();
      expect(canonical!.severity).toBe('error');
      expect(canonical!.data['error_rate']).toBe(0.2);
      expect(canonical!.data['threshold']).toBe(0.1);
      expect(legacy).toBeUndefined();
    });

    it('emits apcore.health.latency_threshold_exceeded (no legacy alias)', () => {
      const emitter = new EventEmitter();
      const events: ApCoreEvent[] = [];
      emitter.subscribe({ onEvent: (e) => { events.push(e); } });

      const metrics = new MetricsCollector();
      for (let i = 0; i < 10; i++) metrics.observeDuration('mod.a', 6.0);

      const mw = new PlatformNotifyMiddleware(emitter, metrics, 0.1, 5000);
      mw.after('mod.a', {}, {}, Context.create());

      const canonical = events.find(e => e.eventType === 'apcore.health.latency_threshold_exceeded');
      const legacy = events.find(e => e.eventType === 'latency_threshold_exceeded');
      expect(canonical).toBeDefined();
      expect(canonical!.severity).toBe('warn');
      expect(canonical!.data['threshold']).toBe(5000);
      expect(legacy).toBeUndefined();
    });
  });

  describe('Registry register/unregister bridge emits canonical events only', () => {
    function buildClient(): { registry: Registry; emitter: EventEmitter; events: ApCoreEvent[] } {
      const config = new Config({ sys_modules: { enabled: true, events: { enabled: true } } });
      const registry = new Registry();
      const executor = new Executor({ registry });
      const ctx = registerSysModules(registry, executor, config);
      const emitter = ctx.eventEmitter!;
      const events: ApCoreEvent[] = [];
      emitter.subscribe({ onEvent: (e) => { events.push(e); } });
      return { registry, emitter, events };
    }

    it('emits apcore.registry.module_registered (no legacy alias)', () => {
      const { registry, events } = buildClient();
      registry.registerInternal('mod.test', { description: 'x', execute: () => ({}) });

      const canonical = events.find(e => e.eventType === 'apcore.registry.module_registered');
      const legacy = events.find(e => e.eventType === 'module_registered');
      expect(canonical).toBeDefined();
      expect(canonical!.moduleId).toBe('mod.test');
      expect(legacy).toBeUndefined();
    });

    it('emits apcore.registry.module_unregistered (no legacy alias)', async () => {
      const { registry, events } = buildClient();
      registry.registerInternal('mod.test', { description: 'x', execute: () => ({}) });
      // clear prior events
      events.length = 0;
      await registry.safeUnregister('mod.test');

      const canonical = events.find(e => e.eventType === 'apcore.registry.module_unregistered');
      const legacy = events.find(e => e.eventType === 'module_unregistered');
      expect(canonical).toBeDefined();
      expect(canonical!.moduleId).toBe('mod.test');
      expect(legacy).toBeUndefined();
    });

    it('FilterSubscriber with apcore.registry.* glob matches the canonical event', () => {
      const { registry, emitter } = buildClient();
      const matched: ApCoreEvent[] = [];
      const filter = new FilterSubscriber(
        { onEvent: (e) => { matched.push(e); } },
        ['apcore.registry.*'],
      );
      emitter.subscribe(filter);

      registry.registerInternal('mod.glob', { description: 'x', execute: () => ({}) });

      expect(matched.some(e => e.eventType === 'apcore.registry.module_registered')).toBe(true);
      expect(matched.every(e => e.eventType.startsWith('apcore.registry.'))).toBe(true);
    });
  });
});

// ---------------------------------------------------------------------------
// Issue #45.2 — contextual audit identity in event payload
// ---------------------------------------------------------------------------

describe('Issue #45.2 — control modules include caller_id / identity in audit events', () => {
  describe('UpdateConfigModule', () => {
    let config: Config;
    let emitter: EventEmitter;
    let mod: UpdateConfigModule;
    let events: ApCoreEvent[];

    beforeEach(() => {
      config = new Config({ some: { name: 'value' } });
      emitter = new EventEmitter();
      mod = new UpdateConfigModule(config, emitter);
      events = [];
      emitter.subscribe({ onEvent: (e) => { events.push(e); } });
    });

    it('defaults caller_id to "@external" and OMITS identity when context is null', () => {
      mod.execute({ key: 'some.name', value: 'v', reason: 'r' }, null);

      const evt = events.find(e => e.eventType === 'apcore.config.updated');
      expect(evt).toBeDefined();
      expect(evt!.data['caller_id']).toBe('@external');
      expect('identity' in evt!.data).toBe(false);
    });

    it('extracts caller_id and identity from a Context', () => {
      const identity = createIdentity('alice', 'user', ['admin'], { dept: 'eng' });
      const ctx = new Context('a'.repeat(32), 'caller.module', [], null, identity);

      mod.execute({ key: 'some.name', value: 'v', reason: 'r' }, ctx);

      const evt = events.find(e => e.eventType === 'apcore.config.updated');
      expect(evt).toBeDefined();
      expect(evt!.data['caller_id']).toBe('caller.module');
      const ident = evt!.data['identity'] as Record<string, unknown>;
      expect(ident).toMatchObject({ id: 'alice', type: 'user' });
      expect(ident['roles']).toEqual(['admin']);
    });

    it('defaults caller_id to "@external" when Context has null callerId', () => {
      const ctx = new Context('b'.repeat(32), null, [], null, null);

      mod.execute({ key: 'some.name', value: 'v', reason: 'r' }, ctx);

      const evt = events.find(e => e.eventType === 'apcore.config.updated');
      expect(evt!.data['caller_id']).toBe('@external');
      expect('identity' in evt!.data).toBe(false);
    });
  });

  describe('ToggleFeatureModule', () => {
    it('includes caller_id and identity in apcore.module.toggled', () => {
      const registry = new Registry();
      registry.registerInternal('test.mod', { description: 'x', execute: () => ({}) });
      const emitter = new EventEmitter();
      const mod = new ToggleFeatureModule(registry, emitter);
      const events: ApCoreEvent[] = [];
      emitter.subscribe({ onEvent: (e) => { events.push(e); } });

      const identity = createIdentity('bob', 'service');
      const ctx = new Context('c'.repeat(32), 'orch.module', [], null, identity);
      mod.execute({ module_id: 'test.mod', enabled: false, reason: 'r' }, ctx);

      const evt = events.find(e => e.eventType === 'apcore.module.toggled');
      expect(evt).toBeDefined();
      expect(evt!.data['enabled']).toBe(false);
      expect(evt!.data['caller_id']).toBe('orch.module');
      expect((evt!.data['identity'] as Record<string, unknown>)['id']).toBe('bob');
    });

    it('defaults caller_id to "@external" when no context', () => {
      const registry = new Registry();
      registry.registerInternal('test.mod', { description: 'x', execute: () => ({}) });
      const emitter = new EventEmitter();
      const mod = new ToggleFeatureModule(registry, emitter);
      const events: ApCoreEvent[] = [];
      emitter.subscribe({ onEvent: (e) => { events.push(e); } });

      mod.execute({ module_id: 'test.mod', enabled: true, reason: 'r' }, null);

      const evt = events.find(e => e.eventType === 'apcore.module.toggled');
      expect(evt!.data['caller_id']).toBe('@external');
      expect('identity' in evt!.data).toBe(false);
    });
  });

  describe('ReloadModule', () => {
    it('includes caller_id and identity in apcore.module.reloaded', async () => {
      const registry = new Registry();
      const dummy = { description: 'x', version: '1.0.0', execute: () => ({}) };
      registry.registerInternal('test.mod', dummy);
      const emitter = new EventEmitter();
      const mod = new ReloadModule(registry, emitter);
      const events: ApCoreEvent[] = [];
      emitter.subscribe({ onEvent: (e) => { events.push(e); } });

      const replacement = { description: 'x', version: '2.0.0', execute: () => ({}) };
      vi.spyOn(registry, 'discover').mockImplementation(async () => {
        registry.registerInternal('test.mod', replacement);
        return 1;
      });

      const identity = createIdentity('carol', 'user');
      const ctx = new Context('d'.repeat(32), 'orch.reload', [], null, identity);
      await mod.execute({ module_id: 'test.mod', reason: 'r' }, ctx);

      const evt = events.find(e => e.eventType === 'apcore.module.reloaded');
      expect(evt).toBeDefined();
      expect(evt!.data['caller_id']).toBe('orch.reload');
      expect((evt!.data['identity'] as Record<string, unknown>)['id']).toBe('carol');
      expect(evt!.data['previous_version']).toBe('1.0.0');
      expect(evt!.data['new_version']).toBe('2.0.0');
    });
  });
});
