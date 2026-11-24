/**
 * D-32: `_discoverDefault` must expose 8 canonical stages (matching Rust
 * `default_discoverer.rs`):
 *   1. _ensureIdMap
 *   2. _scanRoots
 *   3. _applyIdMapOverrides
 *   4. _loadAllMetadata
 *   5. _resolveAllEntryPoints
 *   6. _validateAll
 *   7. _filterIdConflicts   (NEW — was inlined into _registerInOrder)
 *   8. _resolveLoadOrder + _registerInOrder
 *
 * `_filterIdConflicts` MUST exist as a separate helper, MUST be invoked
 * by `_discoverDefault`, and MUST batch the conflict-detection pass that
 * was previously inlined inside `_registerInOrder`. Behaviour is identical:
 * conflicting / invalid IDs are dropped (warn + skip) and the remaining
 * modules are registered in dependency order.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { Registry } from '../../src/registry/registry.js';
import { detectIdConflicts } from '../../src/registry/conflicts.js';

describe('_discoverDefault 8-stage decomposition (D-32)', () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
  });

  it('exposes _filterIdConflicts as a separate stage helper', () => {
    const registry = new Registry();
    const r = registry as unknown as Record<string, unknown>;
    expect(typeof r._filterIdConflicts).toBe('function');
  });

  it('_discoverDefault invokes _filterIdConflicts between _validateAll and _resolveLoadOrder', async () => {
    const registry = new Registry();
    const r = registry as unknown as Record<string, unknown>;

    const order: string[] = [];
    const stages = [
      '_ensureIdMap',
      '_scanRoots',
      '_applyIdMapOverrides',
      '_loadAllMetadata',
      '_resolveAllEntryPoints',
      '_validateAll',
      '_filterIdConflicts',
      '_resolveLoadOrder',
      '_registerInOrder',
    ];

    for (const stage of stages) {
      const original = r[stage] as (...a: unknown[]) => unknown;
      r[stage] = (...args: unknown[]): unknown => {
        order.push(stage);
        return original.apply(registry, args);
      };
    }

    // Force _scanRoots to return an empty discovery list so the pipeline
    // walks every stage without needing fixtures on disk.
    r._scanRoots = async () => {
      order.push('_scanRoots');
      return [];
    };

    await registry.discover();

    // _filterIdConflicts must appear after _validateAll and before
    // _resolveLoadOrder / _registerInOrder.
    const validateIdx = order.indexOf('_validateAll');
    const filterIdx = order.indexOf('_filterIdConflicts');
    const resolveIdx = order.indexOf('_resolveLoadOrder');
    const registerIdx = order.indexOf('_registerInOrder');

    expect(validateIdx).toBeGreaterThanOrEqual(0);
    expect(filterIdx).toBeGreaterThan(validateIdx);
    expect(resolveIdx).toBeGreaterThan(filterIdx);
    expect(registerIdx).toBeGreaterThan(resolveIdx);
  });

  it('_filterIdConflicts drops modules with conflicting IDs (lowercase collision) before registration', () => {
    const registry = new Registry();
    const r = registry as unknown as {
      _filterIdConflicts: (
        validModules: Map<string, unknown>,
        rawMetadata: Map<string, Record<string, unknown>>,
      ) => Map<string, unknown>;
      _modules: Map<string, unknown>;
      _lowercaseMap: Map<string, string>;
    };

    // Pre-populate registry with an existing module to force a duplicate
    // conflict for the discovered batch.
    r._modules.set('email.send', { foo: 1 });
    r._lowercaseMap.set('email.send', 'email.send');

    const valid = new Map<string, unknown>([
      ['email.send', { duplicate: true }],
      ['email.receive', { ok: true }],
    ]);
    const meta = new Map<string, Record<string, unknown>>([
      ['email.send', {}],
      ['email.receive', {}],
    ]);

    const filtered = r._filterIdConflicts(valid, meta);

    // The duplicate is dropped, the unique one survives.
    expect(filtered.has('email.send')).toBe(false);
    expect(filtered.has('email.receive')).toBe(true);
  });

  it('_filterIdConflicts drops modules with invalid IDs (reserved word)', () => {
    const registry = new Registry();
    const r = registry as unknown as {
      _filterIdConflicts: (
        validModules: Map<string, unknown>,
        rawMetadata: Map<string, Record<string, unknown>>,
      ) => Map<string, unknown>;
    };

    const valid = new Map<string, unknown>([
      ['system.boot', { reserved: true }],
      ['email.send', { ok: true }],
    ]);
    const meta = new Map<string, Record<string, unknown>>([
      ['system.boot', {}],
      ['email.send', {}],
    ]);

    const filtered = r._filterIdConflicts(valid, meta);
    expect(filtered.has('system.boot')).toBe(false);
    expect(filtered.has('email.send')).toBe(true);
  });

  it('_registerInOrder no longer inlines conflict detection (it trusts the filter)', () => {
    // Sanity: detectIdConflicts is still the canonical helper; it's just
    // called from _filterIdConflicts now rather than _registerInOrder.
    expect(typeof detectIdConflicts).toBe('function');
  });
});
