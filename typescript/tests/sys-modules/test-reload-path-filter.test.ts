/**
 * Issue #45.4 — Granular reload via path_filter.
 *
 * ReloadModule MUST support a `path_filter` glob input that scopes the
 * re-discovery to matching module IDs. Unaffected modules stay loaded.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { InvalidInputError, ModuleReloadConflictError, ReloadFailedError } from '../../src/errors.js';
import { EventEmitter } from '../../src/events/emitter.js';
import { Registry } from '../../src/registry/registry.js';
import { InMemoryAuditStore } from '../../src/sys-modules/audit.js';
import { ReloadModule } from '../../src/sys-modules/control.js';

describe('ReloadModule path_filter (Issue #45.4)', () => {
  let registry: Registry;
  let emitter: EventEmitter;
  let mod: ReloadModule;

  function createDummyModule(version: string = '1.0.0'): Record<string, unknown> {
    return {
      description: 'A dummy module',
      version,
      execute: () => ({ result: 'ok' }),
    };
  }

  beforeEach(() => {
    registry = new Registry();
    emitter = new EventEmitter();
    mod = new ReloadModule(registry, emitter);
  });

  it('returns reloaded_modules array for path_filter mode', async () => {
    registry.registerInternal('app.email.send', createDummyModule());
    registry.registerInternal('app.email.fetch', createDummyModule());
    registry.registerInternal('app.calendar.list', createDummyModule());

    // Re-discovery brings back what was unregistered.
    vi.spyOn(registry, 'discover').mockImplementation(async () => {
      registry.registerInternal('app.email.send', createDummyModule('2.0.0'));
      registry.registerInternal('app.email.fetch', createDummyModule('2.0.0'));
      return 2;
    });

    const result = await mod.execute({ path_filter: 'app.email.*', reason: 'rotate' }, null);

    expect(result.success).toBe(true);
    expect(result.reloaded_modules).toEqual(['app.email.fetch', 'app.email.send']);
  });

  it('fails with RELOAD_FAILED when a module is missing after re-discovery (D-112 rule 4)', async () => {
    const send = createDummyModule();
    const fetch = createDummyModule();
    registry.registerInternal('app.email.send', send);
    registry.registerInternal('app.email.fetch', fetch);
    const store = new InMemoryAuditStore();
    mod = new ReloadModule(registry, emitter, store);

    // Only one of the two matched modules comes back.
    vi.spyOn(registry, 'discover').mockImplementation(async () => {
      registry.registerInternal('app.email.fetch', createDummyModule('2.0.0'));
      return 1;
    });

    const err = await mod
      .execute({ path_filter: 'app.email.*', reason: 'rotate' }, null)
      .then(() => null, (e: unknown) => e);

    expect(err).toBeInstanceOf(ReloadFailedError);
    expect(String((err as Error).message)).toContain('app.email.send');
    // The missing module is restored; the one that reloaded is not rolled back.
    expect(registry.get('app.email.send')).toBe(send);
    expect(registry.get('app.email.fetch')).not.toBe(fetch);
    // A failed operation is not audited as a completed reload.
    expect(store.query()).toEqual([]);
  });

  it('rejects when both module_id and path_filter are supplied', async () => {
    await expect(
      mod.execute({ module_id: 'm.x', path_filter: 'app.*', reason: 'r' }, null),
    ).rejects.toThrow(ModuleReloadConflictError);
  });

  it('rejects when neither module_id nor path_filter is supplied', async () => {
    await expect(mod.execute({ reason: 'r' }, null)).rejects.toThrow(InvalidInputError);
  });

  it('does not unregister modules outside the path_filter glob', async () => {
    registry.registerInternal('app.email.send', createDummyModule());
    registry.registerInternal('app.calendar.list', createDummyModule());

    // discover() is a no-op for the test — but we want to ensure
    // app.calendar.list is left untouched throughout the call.
    const safeUnregisterSpy = vi.spyOn(registry, 'safeUnregister');
    vi.spyOn(registry, 'discover').mockImplementation(async () => {
      registry.registerInternal('app.email.send', createDummyModule());
      return 1;
    });

    await mod.execute({ path_filter: 'app.email.*', reason: 'rotate' }, null);

    const unregisteredIds = safeUnregisterSpy.mock.calls.map((c) => c[0]);
    expect(unregisteredIds).toContain('app.email.send');
    expect(unregisteredIds).not.toContain('app.calendar.list');
  });

  it('rejects empty-string path_filter as invalid', async () => {
    await expect(mod.execute({ path_filter: '', reason: 'r' }, null)).rejects.toThrow(
      InvalidInputError,
    );
  });

  // -------------------------------------------------------------------------
  // Issue #35 — reload order is topological, not lexicographic.
  //
  // The canonical fixture (`system_modules_hardening.json`,
  // `reload_with_path_filter`) declares `reload_order: "topological"` but its
  // three modules declare no dependencies on each other, so every permutation
  // is a valid linearization and an assertion against it passes whatever the
  // SDK does. These cases use module ids whose lexicographic order is the
  // WRONG one — `executor.alpha` depends on `executor.zulu` — so `.sort()`
  // and a real topological sort cannot both pass. A case of this shape is now
  // canonical — `reload_order_is_topological_not_alphabetical` in the same
  // fixture, driven from tests/conformance.test.ts. The cases below stay as the
  // repo-local edge coverage the canonical case does not carry (code-declared
  // vs metadata-declared, non-list values, cycles).
  // -------------------------------------------------------------------------

  /** Re-discovery of an unchanged source tree re-registers what was unregistered. */
  function stubRediscovery(
    modules: Record<string, Record<string, unknown>>,
    metadata: Record<string, Record<string, unknown>> = {},
  ): void {
    vi.spyOn(registry, 'discover').mockImplementation(async () => {
      let n = 0;
      for (const [id, module] of Object.entries(modules)) {
        if (!registry.has(id)) {
          const meta = metadata[id];
          if (meta !== undefined) {
            await registry.register(id, module, null, meta);
          } else {
            registry.registerInternal(id, module);
          }
          n += 1;
        }
      }
      return n;
    });
  }

  it('reloads a code-declared dependency before the module that declares it', async () => {
    const modules: Record<string, Record<string, unknown>> = {
      'executor.alpha': {
        ...createDummyModule(),
        dependencies: [{ module_id: 'executor.zulu' }],
      },
      'executor.zulu': createDummyModule(),
    };
    for (const [id, module] of Object.entries(modules)) {
      registry.registerInternal(id, module);
    }
    const safeUnregisterSpy = vi.spyOn(registry, 'safeUnregister');
    stubRediscovery(modules);

    const result = await mod.execute({ path_filter: 'executor.*', reason: 'topo test' }, null);
    const order = result.reloaded_modules as string[];

    // Every matched module reloaded exactly once...
    expect([...order].sort()).toEqual(['executor.alpha', 'executor.zulu']);
    // ...and the dependency strictly before its dependent. Lexicographic
    // ordering puts 'executor.alpha' first and fails right here.
    expect(order.indexOf('executor.zulu')).toBeLessThan(order.indexOf('executor.alpha'));
    // The reported order is the order the work happened in, not a relabelling
    // of it: unregistration walks the same sequence.
    expect(safeUnregisterSpy.mock.calls.map((c) => c[0])).toEqual([
      'executor.zulu',
      'executor.alpha',
    ]);
  });

  it('honours a dependency declared through registration metadata', async () => {
    // The YAML side of the merge rule — `register(..., { dependencies })` —
    // must reach the reload path exactly as a code-declared list does.
    const modules: Record<string, Record<string, unknown>> = {
      'executor.alpha': createDummyModule(),
      'executor.zulu': createDummyModule(),
    };
    const metadata = { 'executor.alpha': { dependencies: [{ module_id: 'executor.zulu' }] } };
    await registry.register('executor.alpha', modules['executor.alpha'], null, metadata['executor.alpha']);
    registry.registerInternal('executor.zulu', modules['executor.zulu']);
    stubRediscovery(modules, metadata);

    const result = await mod.execute({ path_filter: 'executor.*', reason: 'topo test' }, null);

    expect(result.reloaded_modules).toEqual(['executor.zulu', 'executor.alpha']);
  });

  it('falls back to alphabetical order when no dependencies are declared', async () => {
    // Kahn's sort seeds from a sorted zero-in-degree queue, so a
    // dependency-free set has exactly one valid output — the old behaviour
    // survives as the degenerate case.
    const modules: Record<string, Record<string, unknown>> = {
      'executor.zulu': createDummyModule(),
      'executor.alpha': createDummyModule(),
      'executor.mike': createDummyModule(),
    };
    for (const [id, module] of Object.entries(modules)) {
      registry.registerInternal(id, module);
    }
    stubRediscovery(modules);

    const result = await mod.execute({ path_filter: 'executor.*', reason: 'topo test' }, null);

    expect(result.reloaded_modules).toEqual([
      'executor.alpha',
      'executor.mike',
      'executor.zulu',
    ]);
  });

  it('ignores a non-list dependencies value rather than failing the reload', async () => {
    // No schema describes `dependencies`, so a scalar can reach the merge from
    // YAML and land in the stored metadata unaltered.
    const modules: Record<string, Record<string, unknown>> = {
      'executor.alpha': createDummyModule(),
      'executor.zulu': createDummyModule(),
    };
    const metadata = { 'executor.alpha': { dependencies: 'executor.zulu' } };
    await registry.register('executor.alpha', modules['executor.alpha'], null, metadata['executor.alpha']);
    registry.registerInternal('executor.zulu', modules['executor.zulu']);
    stubRediscovery(modules, metadata);

    const result = await mod.execute({ path_filter: 'executor.*', reason: 'topo test' }, null);

    expect(result.reloaded_modules).toEqual(['executor.alpha', 'executor.zulu']);
  });

  it('falls back to alphabetical order when the declared dependencies form a cycle', async () => {
    const modules: Record<string, Record<string, unknown>> = {
      'executor.alpha': {
        ...createDummyModule(),
        dependencies: [{ module_id: 'executor.zulu' }],
      },
      'executor.zulu': {
        ...createDummyModule(),
        dependencies: [{ module_id: 'executor.alpha' }],
      },
    };
    for (const [id, module] of Object.entries(modules)) {
      registry.registerInternal(id, module);
    }
    stubRediscovery(modules);
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const result = await mod.execute({ path_filter: 'executor.*', reason: 'topo test' }, null);

    // A cycle is not a reason to refuse the reload; it degrades to the
    // deterministic order and warns.
    expect(result.reloaded_modules).toEqual(['executor.alpha', 'executor.zulu']);
    expect(warnSpy.mock.calls.some((c) => String(c[0]).includes('Topo sort failed'))).toBe(true);
    warnSpy.mockRestore();
  });
});
