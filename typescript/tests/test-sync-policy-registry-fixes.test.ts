/**
 * Cross-language sync regressions for governance policy, registry ordering and
 * the strategy presets.
 *
 * W3 — `ExecutionPolicy.fromObject` must reject a non-boolean
 *      `gate_destructive` / `strict` instead of coercing with JS truthiness.
 * W4 — `Registry.register` must validate module structure BEFORE duplicate
 *      detection (registry-system.md "Side Effects (ordered)" steps 2 and 3).
 * W5 — the discovery register pass must reserve the ID in the in-flight set
 *      while an async `onLoad` runs, so a concurrent `register()` of the same
 *      ID is rejected instead of silently overwriting.
 * W9 — the `minimal` preset must receive the per-instance `ToggleState`.
 */

import { describe, it, expect } from 'vitest';
import { ExecutionPolicy } from '../src/policy.js';
import { Registry } from '../src/registry/registry.js';
import {
  DuplicateModuleIdError,
  ModuleDisabledError,
  StreamingInterfaceError,
} from '../src/errors.js';
import { ToggleState } from '../src/sys-modules/toggle.js';
import { MiddlewareManager } from '../src/middleware/manager.js';
import { Executor } from '../src/executor.js';
import { buildMinimalStrategy } from '../src/builtin-steps.js';

// ---------------------------------------------------------------------------
// W3 — strict boolean parsing for governance switches
// ---------------------------------------------------------------------------

describe('W3: ExecutionPolicy.fromObject rejects non-boolean governance switches', () => {
  it('accepts real booleans', () => {
    const policy = ExecutionPolicy.fromObject({ gate_destructive: true, strict: false });
    expect(policy.gateDestructive).toBe(true);
    expect(policy.strict).toBe(false);
  });

  it('defaults both switches to false when absent', () => {
    const policy = ExecutionPolicy.fromObject({});
    expect(policy.gateDestructive).toBe(false);
    expect(policy.strict).toBe(false);
  });

  it('rejects an empty array for gate_destructive (JS truthiness would say true)', () => {
    expect(() => ExecutionPolicy.fromObject({ gate_destructive: [] })).toThrow(
      /gate_destructive/,
    );
  });

  it('rejects the string "false" for gate_destructive', () => {
    expect(() => ExecutionPolicy.fromObject({ gate_destructive: 'false' })).toThrow(
      /gate_destructive/,
    );
  });

  it('rejects a non-boolean strict', () => {
    expect(() => ExecutionPolicy.fromObject({ strict: 'yes' })).toThrow(/strict/);
  });

  it('accepts an explicit null as "unset"', () => {
    const policy = ExecutionPolicy.fromObject({ gate_destructive: null, strict: null });
    expect(policy.gateDestructive).toBe(false);
    expect(policy.strict).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// W4 — structure validation precedes duplicate detection
// ---------------------------------------------------------------------------

describe('W4: register() validates module structure before duplicate detection', () => {
  it('raises StreamingInterfaceError, not DuplicateModuleIdError, for a broken re-registration', async () => {
    const registry = new Registry();
    await registry.register('svc.stream', { execute: async () => ({}) });

    const broken = {
      execute: async () => ({}),
      annotations: { streaming: true },
      // no stream() and no STREAMING_MARKER — the structure is invalid
    };

    expect(() => registry.register('svc.stream', broken)).toThrow(StreamingInterfaceError);
  });

  it('still raises DuplicateModuleIdError for a structurally valid duplicate', async () => {
    const registry = new Registry();
    await registry.register('svc.plain', { execute: async () => ({}) });
    expect(() => registry.register('svc.plain', { execute: async () => ({}) })).toThrow(
      DuplicateModuleIdError,
    );
  });
});

// ---------------------------------------------------------------------------
// W5 — in-flight reservation on the discovery register pass
// ---------------------------------------------------------------------------

describe('W5: the discovery register pass reserves the ID while onLoad runs', () => {
  it('rejects a concurrent register() of the same ID during an async onLoad', async () => {
    const registry = new Registry();

    let releaseOnLoad!: () => void;
    const gate = new Promise<void>((r) => {
      releaseOnLoad = r;
    });

    const discovered = {
      execute: async () => ({ from: 'discovered' }),
      onLoad: async () => {
        await gate;
      },
    };

    // White-box: `_registerInOrder` is the discovery-side register pass
    // (`discover()` stage 8). It is private, but it is the exact path the
    // registration-ordering invariant applies to
    // ("SDKs MUST NOT create per-path exceptions").
    const pass = (
      registry as unknown as {
        _registerInOrder(
          loadOrder: string[],
          validModules: Map<string, unknown>,
          rawMetadata: Map<string, Record<string, unknown>>,
        ): Promise<number>;
      }
    )._registerInOrder(['svc.raced'], new Map([['svc.raced', discovered]]), new Map());

    // Let the onLoad start and suspend.
    await Promise.resolve();
    await Promise.resolve();

    // The module must not be visible yet …
    expect(registry.has('svc.raced')).toBe(false);
    // … and a concurrent registration of the same ID must be rejected.
    expect(() => registry.register('svc.raced', { execute: async () => ({ from: 'racer' }) })).toThrow(
      DuplicateModuleIdError,
    );

    releaseOnLoad();
    await pass;

    expect(registry.has('svc.raced')).toBe(true);
    const mod = registry.get('svc.raced') as { execute: () => Promise<{ from: string }> };
    expect(await mod.execute()).toEqual({ from: 'discovered' });
  });

  it('releases the reservation when onLoad fails', async () => {
    const registry = new Registry();
    const failing = {
      execute: async () => ({}),
      onLoad: async () => {
        throw new Error('boom');
      },
    };
    await (
      registry as unknown as {
        _registerInOrder(
          loadOrder: string[],
          validModules: Map<string, unknown>,
          rawMetadata: Map<string, Record<string, unknown>>,
        ): Promise<number>;
      }
    )._registerInOrder(['svc.failed'], new Map([['svc.failed', failing]]), new Map());

    expect(registry.has('svc.failed')).toBe(false);
    // The slot must be free again for a later registration.
    await registry.register('svc.failed', { execute: async () => ({}) });
    expect(registry.has('svc.failed')).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// W9 — minimal preset toggle-state wiring
// ---------------------------------------------------------------------------

describe('W9: buildMinimalStrategy honours the per-instance ToggleState', () => {
  it('blocks a module disabled on the instance ToggleState', async () => {
    const registry = new Registry();
    await registry.register('svc.toggled', { execute: async () => ({ ok: true }) });

    const toggleState = new ToggleState();
    toggleState.disable('svc.toggled');

    const executor = new Executor({ registry, strategy: 'minimal', toggleState });
    await expect(executor.call('svc.toggled', {})).rejects.toThrow(ModuleDisabledError);
  });

  it('does not leak a disable across instances (module-global fallback)', async () => {
    const registry = new Registry();
    await registry.register('svc.isolated', { execute: async () => ({ ok: true }) });

    const disabled = new ToggleState();
    disabled.disable('svc.isolated');

    const a = new Executor({ registry, strategy: 'minimal', toggleState: disabled });
    const b = new Executor({ registry, strategy: 'minimal', toggleState: new ToggleState() });

    await expect(a.call('svc.isolated', {})).rejects.toThrow(ModuleDisabledError);
    await expect(b.call('svc.isolated', {})).resolves.toEqual({ ok: true });
  });

  it('wires toggleState into the minimal preset factory', () => {
    const registry = new Registry();
    const strategy = buildMinimalStrategy({
      registry,
      config: null,
      middlewareManager: new MiddlewareManager(),
      acl: null,
      approvalHandler: null,
      toggleState: new ToggleState(),
    });
    expect(strategy.stepNames()).toContain('module_lookup');
  });
});
