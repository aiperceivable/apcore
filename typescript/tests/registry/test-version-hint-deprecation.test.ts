/**
 * `Registry.get(moduleId, versionHint)` accepts a hint it cannot honour
 * (spec v1.55.0, D-126).
 *
 * The spec's v1.10.0 row recorded "all three SDKs accept it, only apcore-python
 * resolves by it". Half of that is wrong: apcore-rust's `get(&self, name)` takes
 * no hint at all, so a Rust caller cannot pass one — the compiler refuses. This
 * SDK accepted the argument and discarded it; `_versionHint` appeared exactly
 * once in the source, in the signature.
 *
 * The parameter is INERT rather than wrong — `register` rejects a second
 * registration of the same module_id, so only one version can ever be present —
 * but a caller writing `get(id, '1.0.0')` believes it has pinned a version and
 * has not. That is the §9.1.3 shape the spec forbids for configuration keys, a
 * declared surface that reaches no mechanism, on a method parameter.
 *
 * Deprecated rather than removed, following D-121: removal is a compile error
 * for every caller passing one, and the warning carries the same information
 * without breaking the build.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { Registry } from '../../src/registry/registry.js';

const MODULE_ID = 'executor.probe.versioned';

function mod(version: string) {
  return {
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    description: `v${version}`,
    version,
    execute: async () => ({ version }),
  };
}

describe('Registry.get version hint (D-126)', () => {
  afterEach(() => vi.restoreAllMocks());

  it('warns when a caller passes a hint it cannot honour', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry();
    registry.register(MODULE_ID, mod('1.0.0'), '1.0.0');

    registry.get(MODULE_ID, '2.0.0');

    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0]?.[0]).toMatch(/version hint is ignored/);
    expect(warn.mock.calls[0]?.[0]).toMatch(/removed at 2\.0/);
  });

  it('warns at most once per module ID (D-89 cadence)', () => {
    // `get` is a read hosts call in loops; an advisory whose volume is
    // proportional to traffic is one operators learn to filter out.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry();
    registry.register(MODULE_ID, mod('1.0.0'), '1.0.0');

    for (let i = 0; i < 20; i++) registry.get(MODULE_ID, '2.0.0');

    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('stays silent when no hint is passed', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry();
    registry.register(MODULE_ID, mod('1.0.0'), '1.0.0');

    registry.get(MODULE_ID);
    registry.get(MODULE_ID, null);
    registry.get(MODULE_ID, '');

    expect(warn).not.toHaveBeenCalled();
  });

  it('still returns the module — the hint is inert, not a filter', () => {
    // Changing that would be a behaviour change for every caller who passes a
    // hint today and gets a module back. The deprecation says so; it does not
    // start enforcing.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const registry = new Registry();
    registry.register(MODULE_ID, mod('1.0.0'), '1.0.0');

    expect(registry.get(MODULE_ID, '9.9.9')).not.toBeNull();
    void warn;
  });

  it('multi-version registration is refused, which is why there is nothing to resolve', () => {
    const registry = new Registry();
    registry.register(MODULE_ID, mod('1.0.0'), '1.0.0');

    expect(() => registry.register(MODULE_ID, mod('2.0.0'), '2.0.0')).toThrow(
      /already registered/i,
    );
  });
});
