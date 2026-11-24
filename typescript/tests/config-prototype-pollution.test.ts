/**
 * Regression tests for D-92 (CFG-1): a configuration dot-path must address DATA,
 * never the host object graph.
 *
 * `setNested` guarded its descent with `part in current`. `'__proto__' in {}` is
 * true (it is an inherited accessor) and `typeof Object.prototype === 'object'`,
 * so the guard passed and the walk left `data` entirely and assigned to
 * `Object.prototype` — polluting every object in the process while `data`'s own
 * keys stayed unchanged.
 *
 * Reachable without any module call: the `APCORE_` env-override loader maps
 * `APCORE_____PROTO_____POLLUTED` to the dot-path `__proto__.polluted` (`__` is
 * the escape for a literal `_`), so having that variable set at `Config.load()`
 * was enough. apcore-python and apcore-rust store `__proto__` as an ordinary
 * nested key; this suite pins that behaviour for TypeScript.
 */

import { describe, it, expect, afterEach } from 'vitest';
import { Config } from '../src/config.js';

const PROBE = 'apcorePollutionProbe';

function prototypeIsClean(): boolean {
  return !(PROBE in {});
}

afterEach(() => {
  // Never leave a polluted prototype behind for another suite, even on failure.
  delete (Object.prototype as Record<string, unknown>)[PROBE];
});

describe('config dot-path cannot reach the host object graph', () => {
  it('config_bus.set.security.proto_segment_does_not_pollute_object_prototype', () => {
    const cfg = new Config({ data: {} });
    cfg.set(`__proto__.${PROBE}`, 'PWNED');

    expect(prototypeIsClean()).toBe(true);
    expect(({} as Record<string, unknown>)[PROBE]).toBeUndefined();
  });

  it('config_bus.set.security.constructor_and_prototype_segments_are_refused', () => {
    const cfg = new Config({ data: {} });
    cfg.set(`constructor.${PROBE}`, 'x');
    cfg.set(`prototype.${PROBE}`, 'x');

    expect(prototypeIsClean()).toBe(true);
    // The refusal is total: no partial key is written at the root either.
    expect(cfg.get(PROBE)).toBeUndefined();
  });

  it('config_bus.set.security.refuses_the_whole_path_rather_than_sanitising_it', () => {
    const cfg = new Config({ data: {} });
    cfg.set(`__proto__.${PROBE}`, 'PWNED');

    // A caller who wrote `__proto__.x` must not silently get `x` at the root.
    expect(cfg.get(PROBE)).toBeUndefined();
    expect(cfg.get(`__proto__.${PROBE}`)).toBeUndefined();
  });

  it('config_bus.get.security.does_not_read_inherited_properties', () => {
    const cfg = new Config({ data: { executor: {} } });

    // `in` would find these on the prototype chain; `Object.hasOwn` does not.
    expect(cfg.get('executor.toString')).toBeUndefined();
    expect(cfg.get('executor.constructor')).toBeUndefined();
    expect(cfg.get('toString')).toBeUndefined();
  });

  it('config_bus.load.security.env_override_cannot_pollute_object_prototype', () => {
    const saved = process.env['APCORE_____PROTO_____POLLUTED'];
    // `__` is the escape for a literal `_`, so this suffix is the dot-path
    // `__proto__.polluted` — the no-module-call door.
    process.env['APCORE_____PROTO_____POLLUTED'] = 'PWNED';
    try {
      Config.fromDefaults();
      expect('polluted' in {}).toBe(false);
      expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
    } finally {
      if (saved === undefined) delete process.env['APCORE_____PROTO_____POLLUTED'];
      else process.env['APCORE_____PROTO_____POLLUTED'] = saved;
      delete (Object.prototype as Record<string, unknown>)['polluted'];
    }
  });

  it('config_bus.set.data.ordinary_nested_paths_still_work', () => {
    const cfg = new Config({ data: {} });
    cfg.set('executor.timeout', 30000);
    cfg.set('a.b.c', 'deep');

    expect(cfg.get('executor.timeout')).toBe(30000);
    expect(cfg.get('a.b.c')).toBe('deep');
  });
});
