/**
 * v0.21.0 Stage 3 — `ephemeral.*` namespace + `discoverable` annotation.
 *
 * Mirrors apcore-python PR #26 / iter-11 alignment behavior. Cross-refs:
 *   - PROTOCOL_SPEC §2.5 (reserved words / ephemeral namespace semantics)
 *   - PROTOCOL_SPEC §4.4 (`discoverable` annotation)
 *   - protocol-spec §2.5.1 (the `ephemeral.*` namespace)
 */

import { describe, it, expect, beforeEach, afterEach, vi, type MockInstance } from 'vitest';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Type } from '@sinclair/typebox';
import {
  Registry,
  EPHEMERAL_NAMESPACE_PREFIX,
  isEphemeralModuleId,
} from '../../src/registry/registry.js';
import { FunctionModule } from '../../src/decorator.js';
import { InvalidInputError } from '../../src/errors.js';
import { createAnnotations } from '../../src/module.js';
import { EventEmitter, type ApCoreEvent } from '../../src/events/emitter.js';
import { Context, Identity } from '../../src/context.js';

function createMod(
  id: string,
  opts?: { discoverable?: boolean; requiresApproval?: boolean },
): FunctionModule {
  return new FunctionModule({
    execute: () => ({ ok: true }),
    moduleId: id,
    inputSchema: Type.Object({}),
    outputSchema: Type.Object({ ok: Type.Boolean() }),
    description: `Module ${id}`,
    annotations:
      opts === undefined
        ? undefined
        : createAnnotations({
            ...(opts.discoverable !== undefined ? { discoverable: opts.discoverable } : {}),
            ...(opts.requiresApproval !== undefined ? { requiresApproval: opts.requiresApproval } : {}),
          }),
  });
}

describe('v0.21.0 — EPHEMERAL_NAMESPACE_PREFIX constant', () => {
  it('is exported with the expected literal value', () => {
    expect(EPHEMERAL_NAMESPACE_PREFIX).toBe('ephemeral.');
  });

  it('isEphemeralModuleId classifies the bare segment and prefixed IDs', () => {
    expect(isEphemeralModuleId('ephemeral')).toBe(true);
    expect(isEphemeralModuleId('ephemeral.tool_v1')).toBe(true);
    expect(isEphemeralModuleId('ephemeral.a.b.c')).toBe(true);
    // Trailing-dot guard: only matches when the prefix terminates the
    // first segment.
    expect(isEphemeralModuleId('ephemerals.foo')).toBe(false);
    expect(isEphemeralModuleId('something.ephemeral.foo')).toBe(false);
    expect(isEphemeralModuleId('plain.module')).toBe(false);
  });
});

describe('v0.21.0 — Registry.register accepts ephemeral.* IDs', () => {
  let warnSpy: MockInstance<typeof console.warn>;
  beforeEach(() => {
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });
  afterEach(() => {
    warnSpy.mockRestore();
  });

  it('register() succeeds for an ephemeral.* ID with requiresApproval=true', () => {
    const registry = new Registry();
    const mod = createMod('ephemeral.tool_v1', {
      requiresApproval: true,
      discoverable: false,
    });
    registry.register('ephemeral.tool_v1', mod);
    expect(registry.has('ephemeral.tool_v1')).toBe(true);
    // No soft-warning when requiresApproval is true.
    const warned = warnSpy.mock.calls.some((c) =>
      String(c[0] ?? '').includes('without requiresApproval=true'),
    );
    expect(warned).toBe(false);
  });

  it('register() emits a SHOULD-warning when ephemeral.* lacks requiresApproval=true', () => {
    const registry = new Registry();
    const mod = createMod('ephemeral.no_approval', { discoverable: false });
    registry.register('ephemeral.no_approval', mod);
    expect(registry.has('ephemeral.no_approval')).toBe(true);
    const warned = warnSpy.mock.calls.some((c) =>
      String(c[0] ?? '').includes(
        "ephemeral.* module 'ephemeral.no_approval' registered without requiresApproval=true",
      ),
    );
    expect(warned).toBe(true);
  });

  it('register() does not soft-warn for non-ephemeral modules without requiresApproval', () => {
    const registry = new Registry();
    registry.register('app.regular', createMod('app.regular'));
    const warned = warnSpy.mock.calls.some((c) =>
      String(c[0] ?? '').includes('without requiresApproval=true'),
    );
    expect(warned).toBe(false);
  });
});

describe('v0.21.0 — registerInternal rejects ephemeral.* IDs', () => {
  it('throws InvalidInputError pointing the caller to Registry.register()', () => {
    const registry = new Registry();
    const mod = createMod('ephemeral.illegal');
    expect(() => registry.registerInternal('ephemeral.illegal', mod)).toThrow(
      InvalidInputError,
    );
    try {
      registry.registerInternal('ephemeral.illegal', mod);
    } catch (e) {
      const msg = (e as Error).message;
      expect(msg).toContain('Registry.register()');
      expect(msg).toContain("'ephemeral.illegal'");
    }
    expect(registry.has('ephemeral.illegal')).toBe(false);
  });

  it('still accepts system.* IDs (sanity check — registerInternal continues to work)', () => {
    const registry = new Registry();
    registry.registerInternal('system.health.test', createMod('system.health.test'));
    expect(registry.has('system.health.test')).toBe(true);
  });
});

describe('v0.21.0 — discoverable annotation honored by enumeration surfaces', () => {
  it('list() excludes discoverable=false modules by default', () => {
    const registry = new Registry();
    registry.register('app.visible', createMod('app.visible', { discoverable: true }));
    registry.register('app.hidden', createMod('app.hidden', { discoverable: false }));
    const ids = registry.list();
    expect(ids).toContain('app.visible');
    expect(ids).not.toContain('app.hidden');
  });

  it('list({ includeHidden: true }) returns hidden modules', () => {
    const registry = new Registry();
    registry.register('app.visible', createMod('app.visible', { discoverable: true }));
    registry.register('app.hidden', createMod('app.hidden', { discoverable: false }));
    const ids = registry.list({ includeHidden: true });
    expect(ids).toContain('app.visible');
    expect(ids).toContain('app.hidden');
  });

  it('moduleIds excludes discoverable=false modules', () => {
    const registry = new Registry();
    registry.register('app.visible', createMod('app.visible'));
    registry.register('app.hidden', createMod('app.hidden', { discoverable: false }));
    expect(registry.moduleIds).toEqual(['app.visible']);
  });

  it('iter() excludes hidden modules by default; includeHidden:true returns all', () => {
    const registry = new Registry();
    registry.register('app.visible', createMod('app.visible'));
    registry.register('app.hidden', createMod('app.hidden', { discoverable: false }));
    const visible = [...registry.iter()].map(([id]) => id).sort();
    expect(visible).toEqual(['app.visible']);
    const all = [...registry.iter({ includeHidden: true })].map(([id]) => id).sort();
    expect(all).toEqual(['app.hidden', 'app.visible']);
  });

  it('count includes hidden modules (matches apcore-python parity)', () => {
    const registry = new Registry();
    registry.register('app.visible', createMod('app.visible'));
    registry.register('app.hidden', createMod('app.hidden', { discoverable: false }));
    expect(registry.count).toBe(2);
  });

  it('get/has work for hidden modules — discoverability is enumeration-only', () => {
    const registry = new Registry();
    const hidden = createMod('app.hidden', { discoverable: false });
    registry.register('app.hidden', hidden);
    expect(registry.has('app.hidden')).toBe(true);
    expect(registry.get('app.hidden')).toBe(hidden);
  });
});

describe('v0.21.0 — Audit-event single-emit rule', () => {
  it('emits ONE apcore.registry.module_registered event for ephemeral.* with full payload', () => {
    const registry = new Registry();
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });
    registry.setEventEmitter(emitter);

    const mod = createMod('ephemeral.audit_one', {
      requiresApproval: true,
      discoverable: false,
    });
    registry.register('ephemeral.audit_one', mod);

    const registrations = events.filter(
      (e) => e.eventType === 'apcore.registry.module_registered',
    );
    expect(registrations).toHaveLength(1);
    expect(registrations[0].moduleId).toBe('ephemeral.audit_one');
    expect(registrations[0].data.namespace_class).toBe('ephemeral');
    // Default caller_id when no context is supplied.
    expect(registrations[0].data.caller_id).toBe('@external');
  });

  it('forwards Context.callerId / identity into the audit payload', () => {
    const registry = new Registry();
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });
    registry.setEventEmitter(emitter);

    const ctx = new Context(
      'trace-1',
      'agent-99',
      [],
      null,
      new Identity('agent-99', 'agent', ['executor']),
    );
    registry.register(
      'ephemeral.audit_ctx',
      createMod('ephemeral.audit_ctx', { requiresApproval: true, discoverable: false }),
      undefined,
      undefined,
      { context: ctx },
    );

    const registrations = events.filter(
      (e) => e.eventType === 'apcore.registry.module_registered',
    );
    expect(registrations).toHaveLength(1);
    expect(registrations[0].data.caller_id).toBe('agent-99');
    const identity = registrations[0].data.identity as Record<string, unknown>;
    expect(identity).not.toBeNull();
    expect(identity.id).toBe('agent-99');
    expect(identity.type).toBe('agent');
  });

  it('emits ONE apcore.registry.module_unregistered event for ephemeral.* on unregister', () => {
    const registry = new Registry();
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });
    registry.setEventEmitter(emitter);

    registry.register(
      'ephemeral.audit_unreg',
      createMod('ephemeral.audit_unreg', { requiresApproval: true, discoverable: false }),
    );
    registry.unregister('ephemeral.audit_unreg');

    const unregistrations = events.filter(
      (e) => e.eventType === 'apcore.registry.module_unregistered',
    );
    expect(unregistrations).toHaveLength(1);
    expect(unregistrations[0].moduleId).toBe('ephemeral.audit_unreg');
    expect(unregistrations[0].data.namespace_class).toBe('ephemeral');
  });

  it('does NOT emit registry-side audit for non-ephemeral modules', () => {
    const registry = new Registry();
    const emitter = new EventEmitter();
    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });
    registry.setEventEmitter(emitter);

    registry.register('app.regular', createMod('app.regular'));
    // The registry's `setEventEmitter` only emits for ephemerals; the bridge
    // in sys-modules/registration.ts handles non-ephemerals (not wired here).
    expect(events.filter((e) => e.eventType.startsWith('apcore.registry.'))).toEqual([]);
  });

  it('warn-logs the audit event when no EventEmitter is wired', () => {
    const registry = new Registry();
    const infoSpy = vi.spyOn(console, 'info').mockImplementation(() => {});
    try {
      registry.register(
        'ephemeral.no_emitter',
        createMod('ephemeral.no_emitter', { requiresApproval: true, discoverable: false }),
      );
      const logged = infoSpy.mock.calls.some((c) =>
        String(c[0] ?? '').includes('ephemeral audit event apcore.registry.module_registered'),
      );
      expect(logged).toBe(true);
    } finally {
      infoSpy.mockRestore();
    }
  });
});

describe('v0.21.0 — filesystem discovery rejects ephemeral.* IDs', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'apcore-ephemeral-test-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it('_discoverDefault throws when scanner produces an ephemeral.* canonical_id', async () => {
    // Create a directory layout that the scanner would normally walk; we
    // then mock _scanRoots to inject an ephemeral.* canonical_id so we can
    // exercise the rejection path without relying on scanner internals.
    const registry = new Registry({ extensionsDir: tmpDir });
    const r = registry as unknown as Record<string, unknown>;
    r._scanRoots = async () => {
      return [
        {
          canonicalId: 'ephemeral.from_disk',
          filePath: '/fake/path.ts',
          metaPath: null,
          namespace: undefined,
        },
      ];
    };
    await expect(registry.discover()).rejects.toThrow(/ephemeral\.\*/);
  });

  it('custom discoverer entries with ephemeral.* IDs are skipped with a warning', async () => {
    const registry = new Registry({ extensionsDir: tmpDir });
    registry.setDiscoverer({
      discover: () => [
        { moduleId: 'ephemeral.from_custom', module: createMod('ephemeral.from_custom') },
        { moduleId: 'app.legit', module: createMod('app.legit') },
      ],
    });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const count = await registry.discover();
      expect(count).toBe(1);
      expect(registry.has('app.legit')).toBe(true);
      expect(registry.has('ephemeral.from_custom')).toBe(false);
      const warned = warnSpy.mock.calls.some((c) =>
        String(c[0] ?? '').includes("Skipping custom-discovered module 'ephemeral.from_custom'"),
      );
      expect(warned).toBe(true);
    } finally {
      warnSpy.mockRestore();
    }
  });
});

describe('v0.21.0 — registerSysModules wires the registry EventEmitter and short-circuits the bridge', () => {
  it('non-ephemeral registrations flow through the bridge once; ephemeral.* go via the registry-side emit', async () => {
    // Lazy-require to avoid a circular import at file load time.
    const { Executor } = await import('../../src/executor.js');
    const { Config } = await import('../../src/config.js');
    const { registerSysModules } = await import('../../src/sys-modules/registration.js');

    const registry = new Registry();
    const config = new Config();
    config.set('sys_modules.enabled', true);
    config.set('sys_modules.events.enabled', true);
    const executor = new Executor({ registry });

    const ctx = registerSysModules(registry, executor, config);
    const emitter = ctx.eventEmitter;
    expect(emitter).toBeDefined();
    if (!emitter) return;

    const events: ApCoreEvent[] = [];
    emitter.subscribe({ onEvent: (e) => { events.push(e); } });

    // Non-ephemeral: bridge fires the canonical event (and a legacy alias).
    registry.register('app.bridged', createMod('app.bridged'));
    const canonicalNonEph = events.filter(
      (e) =>
        e.eventType === 'apcore.registry.module_registered' &&
        e.moduleId === 'app.bridged',
    );
    expect(canonicalNonEph).toHaveLength(1);

    // Ephemeral: registry-side direct emit fires; bridge SHORT-CIRCUITS.
    events.length = 0;
    registry.register(
      'ephemeral.bridge_skip',
      createMod('ephemeral.bridge_skip', { requiresApproval: true, discoverable: false }),
    );
    const ephRegistered = events.filter(
      (e) =>
        e.eventType === 'apcore.registry.module_registered' &&
        e.moduleId === 'ephemeral.bridge_skip',
    );
    // Exactly ONE — the bridge does not double-emit.
    expect(ephRegistered).toHaveLength(1);
    expect(ephRegistered[0].data.namespace_class).toBe('ephemeral');
    // The legacy alias is also suppressed for ephemeral.* — single-emit rule.
    const legacyAlias = events.filter(
      (e) => e.eventType === 'module_registered' && e.moduleId === 'ephemeral.bridge_skip',
    );
    expect(legacyAlias).toHaveLength(0);
  });
});
