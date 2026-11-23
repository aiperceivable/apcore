/**
 * Unit tests for the Module interface's onSuspend / onResume lifecycle hooks.
 *
 * These tests exercise the hook contract at the Module level, independent of
 * the ReloadModule control path (covered in test-control.test.ts).  They
 * verify:
 *   - A module that implements onSuspend returns state correctly.
 *   - A module that implements onResume receives the previously captured state.
 *   - The round-trip (suspend → resume) restores state faithfully.
 *   - A module whose onSuspend returns null/undefined signals "no state".
 *   - A module without the optional hooks satisfies the Module interface and
 *     can be registered / executed without error.
 */

import { describe, it, expect, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Registry } from '../src/registry/registry.js';
import type { Module } from '../src/module.js';
import type { Context } from '../src/context.js';

// ---------------------------------------------------------------------------
// Minimal shared schemas / context stub
// ---------------------------------------------------------------------------

const NoOpInputSchema = Type.Object({});
const NoOpOutputSchema = Type.Object({ ok: Type.Boolean() });

/** Minimal context stub — lifecycle hooks do not receive a Context. */
const stubContext = {} as Context;

/** Factory for a plain module that satisfies the Module interface. */
function makeBaseModule(): Module {
  return {
    inputSchema: NoOpInputSchema,
    outputSchema: NoOpOutputSchema,
    description: 'No-op module for lifecycle tests',
    execute: (_inputs, _ctx) => ({ ok: true }),
  };
}

// ---------------------------------------------------------------------------
// onSuspend
// ---------------------------------------------------------------------------

describe('Module.onSuspend()', () => {
  it('returns state when onSuspend is implemented', () => {
    const state: Record<string, unknown> = { counter: 7, label: 'hello' };
    const mod: Module = {
      ...makeBaseModule(),
      onSuspend: () => state,
    };

    const captured = mod.onSuspend!();
    expect(captured).toEqual({ counter: 7, label: 'hello' });
  });

  it('returns null when onSuspend explicitly signals no state', () => {
    const mod: Module = {
      ...makeBaseModule(),
      onSuspend: () => null,
    };

    expect(mod.onSuspend!()).toBeNull();
  });

  it('state object may be arbitrarily nested', () => {
    const deepState: Record<string, unknown> = {
      cache: { hits: 100, misses: 3 },
      config: { retries: 5 },
      tags: ['a', 'b'],
    };
    const mod: Module = {
      ...makeBaseModule(),
      onSuspend: () => deepState,
    };

    const captured = mod.onSuspend!();
    expect(captured).toEqual(deepState);
  });

  it('onSuspend is invoked each time it is called (not a one-shot)', () => {
    let callCount = 0;
    const mod: Module = {
      ...makeBaseModule(),
      onSuspend: () => { callCount++; return { callCount }; },
    };

    mod.onSuspend!();
    mod.onSuspend!();
    expect(callCount).toBe(2);
    expect(mod.onSuspend!()).toEqual({ callCount: 3 });
  });
});

// ---------------------------------------------------------------------------
// onResume
// ---------------------------------------------------------------------------

describe('Module.onResume()', () => {
  it('receives the exact state object passed to it', () => {
    const receivedStates: Array<Record<string, unknown>> = [];
    const mod: Module = {
      ...makeBaseModule(),
      onResume: (state) => { receivedStates.push(state); },
    };

    const state = { counter: 42 };
    mod.onResume!(state);
    expect(receivedStates).toHaveLength(1);
    expect(receivedStates[0]).toBe(state);
  });

  it('may be called multiple times with different state objects', () => {
    const log: Array<Record<string, unknown>> = [];
    const mod: Module = {
      ...makeBaseModule(),
      onResume: (state) => { log.push(state); },
    };

    mod.onResume!({ version: 1 });
    mod.onResume!({ version: 2 });
    expect(log).toHaveLength(2);
    expect(log[0]).toEqual({ version: 1 });
    expect(log[1]).toEqual({ version: 2 });
  });

  it('onResume can mutate internal module state', () => {
    let internalCounter = 0;
    const mod: Module = {
      ...makeBaseModule(),
      onResume: (state) => { internalCounter = state['counter'] as number; },
      execute: (_inputs, _ctx) => ({ ok: true, counter: internalCounter }),
    };

    mod.onResume!({ counter: 99 });
    const output = mod.execute({}, stubContext) as Record<string, unknown>;
    expect(output['counter']).toBe(99);
  });
});

// ---------------------------------------------------------------------------
// Round-trip: suspend → resume
// ---------------------------------------------------------------------------

describe('suspend → resume round-trip', () => {
  it('state captured by onSuspend is faithfully delivered to onResume', () => {
    const originalState: Record<string, unknown> = {
      requestCount: 17,
      lastError: 'timeout',
      config: { timeout: 5000, retries: 3 },
    };

    const oldModule: Module = {
      ...makeBaseModule(),
      onSuspend: () => ({ ...originalState }),
    };

    let resumedWith: Record<string, unknown> | null = null;
    const newModule: Module = {
      ...makeBaseModule(),
      onResume: (state) => { resumedWith = state; },
    };

    // Simulate the hand-off that ReloadModule performs:
    const suspendedState = oldModule.onSuspend!();
    if (suspendedState !== null) {
      newModule.onResume!(suspendedState);
    }

    expect(resumedWith).toEqual(originalState);
  });

  it('round-trip with complex nested state preserves all fields', () => {
    const state: Record<string, unknown> = {
      metrics: { calls: 50, errors: 2, p99: 120 },
      featureFlags: { darkMode: true, betaFeature: false },
      sessionIds: ['abc', 'def'],
    };

    const oldModule: Module = {
      ...makeBaseModule(),
      onSuspend: () => state,
    };

    let restored: Record<string, unknown> | null = null;
    const newModule: Module = {
      ...makeBaseModule(),
      onResume: (s) => { restored = s; },
    };

    const captured = oldModule.onSuspend!();
    if (captured !== null) {
      newModule.onResume!(captured);
    }

    expect(restored).toBe(state); // same reference — no copy made in the hand-off
    expect(restored!['metrics']).toEqual({ calls: 50, errors: 2, p99: 120 });
    expect(restored!['sessionIds']).toEqual(['abc', 'def']);
  });

  it('onResume is NOT called when onSuspend returns null', () => {
    const oldModule: Module = {
      ...makeBaseModule(),
      onSuspend: () => null,
    };

    const resumeFn = vi.fn();
    const newModule: Module = {
      ...makeBaseModule(),
      onResume: resumeFn,
    };

    // Mirror the guard used by ReloadModule
    const captured = oldModule.onSuspend!();
    if (captured !== null) {
      newModule.onResume!(captured);
    }

    expect(resumeFn).not.toHaveBeenCalled();
  });

  it('onResume is NOT called when old module has no onSuspend', () => {
    const oldModule: Module = makeBaseModule(); // no onSuspend

    const resumeFn = vi.fn();
    const newModule: Module = {
      ...makeBaseModule(),
      onResume: resumeFn,
    };

    // Mirror the guard: only call onSuspend when it exists
    const captured = typeof oldModule.onSuspend === 'function'
      ? oldModule.onSuspend()
      : null;
    if (captured !== null) {
      newModule.onResume!(captured);
    }

    expect(resumeFn).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// Module without lifecycle hooks (default / backward-compatible behavior)
// ---------------------------------------------------------------------------

describe('Module without onSuspend / onResume', () => {
  it('satisfies the Module interface without defining lifecycle hooks', () => {
    const mod: Module = makeBaseModule();

    expect(mod.onSuspend).toBeUndefined();
    expect(mod.onResume).toBeUndefined();
  });

  it('can be registered in the Registry without error', () => {
    const registry = new Registry();
    const mod: Module = makeBaseModule();

    expect(() => registry.register('lifecycle.no_hooks', mod)).not.toThrow();
    expect(registry.has('lifecycle.no_hooks')).toBe(true);
  });

  it('can be executed after registration', async () => {
    const registry = new Registry();
    const mod: Module = {
      ...makeBaseModule(),
      execute: (_inputs, _ctx) => ({ ok: true }),
    };
    registry.register('lifecycle.no_hooks', mod);

    const registered = registry.get('lifecycle.no_hooks') as Module;
    const result = await registered.execute({}, stubContext);
    expect(result).toEqual({ ok: true });
  });

  it('lifecycle guard (typeof check) is safe for modules without hooks', () => {
    const mod: Module = makeBaseModule();

    // Guards that callers (e.g. ReloadModule) use — must not throw
    const suspendedState = typeof mod.onSuspend === 'function'
      ? mod.onSuspend()
      : null;
    expect(suspendedState).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Registry interaction with suspend/resume hooks
// ---------------------------------------------------------------------------

describe('Registry and suspend/resume hooks', () => {
  it('registers a module that has both onSuspend and onResume', () => {
    const registry = new Registry();
    const mod: Module = {
      ...makeBaseModule(),
      onSuspend: () => ({ x: 1 }),
      onResume: (_state) => { /* no-op */ },
    };

    expect(() => registry.register('lifecycle.full_hooks', mod)).not.toThrow();
    expect(registry.has('lifecycle.full_hooks')).toBe(true);
  });

  it('retrieves a module with hooks intact after registration', () => {
    const registry = new Registry();
    const suspendFn = vi.fn(() => ({ value: 100 }));
    const resumeFn = vi.fn();

    const mod: Module = {
      ...makeBaseModule(),
      onSuspend: suspendFn,
      onResume: resumeFn,
    };

    registry.register('lifecycle.hooks_intact', mod);

    const retrieved = registry.get('lifecycle.hooks_intact') as Module;
    expect(retrieved).toBe(mod); // same object reference
    expect(typeof retrieved.onSuspend).toBe('function');
    expect(typeof retrieved.onResume).toBe('function');
  });

  it('onSuspend on retrieved module returns the expected state', () => {
    const registry = new Registry();
    const state = { pending: ['job1', 'job2'], cursor: 'abc123' };
    const mod: Module = {
      ...makeBaseModule(),
      onSuspend: () => state,
    };

    registry.register('lifecycle.state_check', mod);

    const retrieved = registry.get('lifecycle.state_check') as Module;
    const captured = retrieved.onSuspend!();
    expect(captured).toBe(state);
  });

  it('unregistering a module does not affect the hook functions', () => {
    const registry = new Registry();
    const suspendFn = vi.fn(() => ({ snapshot: true }));
    const mod: Module = {
      ...makeBaseModule(),
      onSuspend: suspendFn,
    };

    registry.register('lifecycle.unregister_test', mod);

    // Grab a reference before unregistering
    const retrieved = registry.get('lifecycle.unregister_test') as Module;
    registry.unregister('lifecycle.unregister_test');

    // The module object itself still has its hooks — unregister does not mutate
    expect(retrieved.onSuspend).toBeDefined();
    const captured = retrieved.onSuspend!();
    expect(captured).toEqual({ snapshot: true });
    expect(suspendFn).toHaveBeenCalledOnce();
  });
});
