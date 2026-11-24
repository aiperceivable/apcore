/**
 * D-107 (spec v1.50.0) — per-class markers are the only multi-class opt-in.
 *
 * This repo shipped two doors with opposite defaults. `Registry.discoverMultiClass`
 * honours the per-class `ClassDescriptor.multiClass` flag
 * (`classes.some(c => c.implementsModule && c.multiClass === true)`), while the
 * publicly exported free function `discoverMultiClass` ignored that field
 * entirely and gated on its own `multiClassEnabled` BOOLEAN, defaulting to off.
 *
 * multi-module-discovery.md's own TypeScript example calls the free function
 * with three arguments and marks both classes `multiClass: true`, documenting
 * two registered IDs — and silently got one.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { discoverMultiClass } from '../../src/registry/multi-class.js';
import type { ClassDescriptor } from '../../src/registry/multi-class.js';
import { Registry } from '../../src/registry/registry.js';

afterEach(() => {
  vi.restoreAllMocks();
});

const MARKED: ClassDescriptor[] = [
  { name: 'Addition', implementsModule: true, multiClass: true },
  { name: 'Subtraction', implementsModule: true, multiClass: true },
];

const UNMARKED: ClassDescriptor[] = [
  { name: 'Addition', implementsModule: true },
  { name: 'Subtraction', implementsModule: true },
];

describe('the exported free function honours the per-class marker (D-107)', () => {
  it("multi-module-discovery.md's own TypeScript example yields two IDs", () => {
    const entries = discoverMultiClass('extensions/math/math_ops.ts', MARKED, 'extensions');
    expect(entries.map((e) => e.moduleId).sort()).toEqual([
      'math.math_ops.addition',
      'math.math_ops.subtraction',
    ]);
  });

  it('an unmarked file still collapses to whole-file mode', () => {
    const entries = discoverMultiClass('extensions/math/math_ops.ts', UNMARKED, 'extensions');
    expect(entries).toHaveLength(1);
    expect(entries[0].moduleId).toBe('math.math_ops');
  });

  it('one marked class among several opts the file in, and only it is registered (D-147)', () => {
    const mixed: ClassDescriptor[] = [
      { name: 'Addition', implementsModule: true, multiClass: true },
      { name: 'Subtraction', implementsModule: true },
    ];
    const entries = discoverMultiClass('extensions/math/math_ops.ts', mixed, 'extensions');
    // Two Module classes, so rule 5 does not apply: the marked one keeps its
    // segment, and the unmarked one receives no ID.
    expect(entries.map((e) => e.moduleId)).toEqual(['math.math_ops.addition']);
  });

  it('a marker on a non-qualifying class does not opt the file in', () => {
    const helperOnly: ClassDescriptor[] = [
      { name: 'Addition', implementsModule: true },
      { name: 'Helper', implementsModule: false, multiClass: true },
    ];
    const entries = discoverMultiClass('extensions/math/math_ops.ts', helperOnly, 'extensions');
    expect(entries).toHaveLength(1);
    expect(entries[0].moduleId).toBe('math.math_ops');
  });

  it('the two doors now agree', () => {
    const registry = new Registry();
    const viaMethod = registry.discoverMultiClass('extensions/math/math_ops.ts', MARKED, 'extensions');
    const viaFunction = discoverMultiClass('extensions/math/math_ops.ts', MARKED, 'extensions');
    expect(viaFunction).toEqual(viaMethod);

    const viaMethodOff = registry.discoverMultiClass('extensions/math/math_ops.ts', UNMARKED, 'extensions');
    const viaFunctionOff = discoverMultiClass('extensions/math/math_ops.ts', UNMARKED, 'extensions');
    expect(viaFunctionOff).toEqual(viaMethodOff);
  });

  it('the legacy boolean is ignored, and passing it warns once', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    // `false` used to suppress multi-class derivation outright.
    const entries = discoverMultiClass('extensions/math/math_ops.ts', MARKED, 'extensions', false);
    expect(entries).toHaveLength(2);
    expect(warn).toHaveBeenCalled();
  });

  it('the legacy boolean cannot turn multi-class ON for unmarked classes', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const entries = discoverMultiClass('extensions/math/math_ops.ts', UNMARKED, 'extensions', true);
    expect(entries).toHaveLength(1);
    expect(entries[0].moduleId).toBe('math.math_ops');
  });
});
