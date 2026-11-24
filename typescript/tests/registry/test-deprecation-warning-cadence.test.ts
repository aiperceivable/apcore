/**
 * D-89 (spec v1.49.0) — at most one `x-deprecation` warning per
 * `(moduleId, version)` per registry instance.
 *
 * `getDefinition` is a read hosts call in loops, so warning on every read is
 * log spam proportional to traffic — which is how operators learn to filter the
 * advisory out. This SDK was the decision's AUTHORITY and had no test for the
 * module-deprecation warning at all: the one test labelled "D-89 cadence" lives
 * in test-version-hint-deprecation.test.ts and covers a DIFFERENT warning (the
 * D-126 version-hint deprecation, which reuses the same dedupe set). The
 * mechanism the decision is about was unpinned.
 *
 * What makes these RED: removing the `_deprecationWarned` guard in
 * `_warnModuleDeprecated` (the first test), or narrowing its key — dropping the
 * version makes the version test red, keying it on something process-global
 * makes the per-instance test red.
 *
 * Two further dimensions were left open by v1.49.0 and adjudicated at v1.59.0:
 * the warning fires on the READ, never on registration, and the dedupe key
 * includes the `x-deprecation` BLOCK and is never cleared on `unregister`.
 * Both are asserted below. This SDK already fired on the read; it keyed on
 * `<moduleId>@<version>` alone, so a re-registered module carrying a new or
 * changed notice was silently deduped against the one it replaced.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { Registry } from '../../src/registry/registry.js';

const DEPRECATION = {
  deprecated_since: '1.0.0',
  sunset_version: '3.0.0',
  migration_guide: 'Use mod.new instead.',
};

function deprecatedModule() {
  return {
    inputSchema: { type: 'object' },
    outputSchema: { type: 'object' },
    description: 'a deprecated module',
    execute: async () => ({}),
  };
}

async function register(
  reg: Registry,
  moduleId: string,
  version = '1.0.0',
  deprecation: Record<string, string> | null = null,
): Promise<void> {
  await reg.register(moduleId, deprecatedModule(), version, {
    'x-deprecation': { ...(deprecation ?? DEPRECATION) },
  });
}

/** Count `x-deprecation` warnings emitted while `f` runs. */
function countWarnings(f: () => void): number {
  const spy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  try {
    f();
    return spy.mock.calls.filter((c) => String(c[0]).includes('is deprecated')).length;
  } finally {
    spy.mockRestore();
  }
}

describe('D-89: the deprecation warning fires once per (moduleId, version)', () => {
  afterEach(() => vi.restoreAllMocks());

  it('ten reads produce one warning', async () => {
    const reg = new Registry();
    await register(reg, 'cadence.once');

    expect(
      countWarnings(() => {
        for (let i = 0; i < 10; i++) reg.getDefinition('cadence.once');
      }),
    ).toBe(1);
  });

  it('control: a module with no x-deprecation never warns', () => {
    // Without this, "exactly one warning" would also hold for an SDK that warns
    // once per registry for any module at all.
    const reg = new Registry();
    void reg.register('cadence.plain', deprecatedModule(), '1.0.0');

    expect(
      countWarnings(() => {
        for (let i = 0; i < 10; i++) reg.getDefinition('cadence.plain');
      }),
    ).toBe(0);
  });

  it('distinct modules warn separately', async () => {
    const reg = new Registry();
    await register(reg, 'cadence.first');
    await register(reg, 'cadence.second');

    expect(
      countWarnings(() => {
        for (let i = 0; i < 3; i++) {
          reg.getDefinition('cadence.first');
          reg.getDefinition('cadence.second');
        }
      }),
    ).toBe(2);
  });

  it('the dedupe is per registry INSTANCE, not process-global', async () => {
    // A process-global set would silence the second registry's advisory for a
    // module its operator has never been told about.
    const a = new Registry();
    const b = new Registry();
    await register(a, 'cadence.instance');
    await register(b, 'cadence.instance');

    expect(
      countWarnings(() => {
        a.getDefinition('cadence.instance');
        b.getDefinition('cadence.instance');
      }),
    ).toBe(2);
  });

  it('the dedupe key includes the version', async () => {
    // A newly registered version is a new deprecation notice, with its own
    // sunset. Keying on the module id alone silences it.
    const reg = new Registry();
    await register(reg, 'cadence.versions', '1.0.0');

    const first = countWarnings(() => {
      reg.getDefinition('cadence.versions');
      reg.getDefinition('cadence.versions');
    });
    expect(first).toBe(1);

    await reg.unregister('cadence.versions');
    await register(reg, 'cadence.versions', '2.0.0');

    expect(
      countWarnings(() => {
        reg.getDefinition('cadence.versions');
        reg.getDefinition('cadence.versions');
      }),
    ).toBe(1);
  });

  it('the warning fires on the READ, not on registration', async () => {
    // D-89 / spec v1.59.0. Registration runs at startup, frequently before the
    // host has installed its log handling; a registration-time warning is then
    // lost with no later chance to re-emit, because the dedupe entry has
    // already been written. apcore-rust emitted at registration and its reads
    // never warned at all.
    const reg = new Registry();

    let atRegistration = 0;
    const spy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await reg.register('cadence.read_path', deprecatedModule(), '1.0.0', {
        'x-deprecation': { ...DEPRECATION },
      });
      atRegistration = spy.mock.calls.filter((c) =>
        String(c[0]).includes('is deprecated'),
      ).length;
    } finally {
      spy.mockRestore();
    }
    expect(atRegistration).toBe(0);

    expect(countWarnings(() => reg.getDefinition('cadence.read_path'))).toBe(1);
  });

  it('a re-registration carrying the SAME notice stays silent', async () => {
    // D-89 / spec v1.59.0. `watch()` re-runs discovery as an unregister +
    // re-register, so clearing the dedupe on unregister re-warns for every
    // deprecated module on every hot reload — the traffic-proportional spam
    // D-89 exists to prevent, through the door its wording did not close.
    const reg = new Registry();
    await register(reg, 'cadence.same_notice');

    expect(countWarnings(() => reg.getDefinition('cadence.same_notice'))).toBe(1);

    await reg.unregister('cadence.same_notice');
    await register(reg, 'cadence.same_notice');

    expect(countWarnings(() => reg.getDefinition('cadence.same_notice'))).toBe(0);
  });

  it('a re-registration carrying a CHANGED notice warns again', async () => {
    // The other half, and the control for the test above: without it, "stays
    // silent" is equally satisfied by an implementation that never warns for a
    // re-registered module at all — which is what this SDK did, swallowing a
    // genuinely new notice.
    const reg = new Registry();
    await register(reg, 'cadence.changed_notice');

    expect(countWarnings(() => reg.getDefinition('cadence.changed_notice'))).toBe(1);

    await reg.unregister('cadence.changed_notice');
    await register(reg, 'cadence.changed_notice', '1.0.0', {
      deprecated_since: '1.0.0',
      sunset_version: '2.0.0', // brought forward
      migration_guide: 'Use mod.new instead.',
    });

    const spy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      reg.getDefinition('cadence.changed_notice');
      const messages = spy.mock.calls
        .map((c) => String(c[0]))
        .filter((m) => m.includes('is deprecated'));
      expect(messages).toHaveLength(1);
      expect(messages[0]).toContain('sunset in 2.0.0');
    } finally {
      spy.mockRestore();
    }
  });

  it('a notice ADDED on re-registration warns', async () => {
    const reg = new Registry();
    await reg.register('cadence.added_notice', deprecatedModule(), '1.0.0');

    expect(countWarnings(() => reg.getDefinition('cadence.added_notice'))).toBe(0);

    await reg.unregister('cadence.added_notice');
    await register(reg, 'cadence.added_notice');

    expect(countWarnings(() => reg.getDefinition('cadence.added_notice'))).toBe(1);
  });
});
