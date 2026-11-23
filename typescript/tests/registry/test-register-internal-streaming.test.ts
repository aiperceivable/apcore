/**
 * STR-5 — `registerInternal` skipped the streaming-annotation check.
 *
 * `register()` and the discovery path `_registerImpl` both refuse a module
 * that declares `streaming: true` without implementing the streaming
 * interface; `registerInternal` did not. Its own docstring has always said it
 * "bypasses **only** the reserved word check", and apcore-python's
 * `register_internal` runs the check (registry.py:~2478) while apcore-rust's
 * `register_core` is reached by both doors.
 *
 * A sys/internal module could therefore advertise streaming it cannot do, and
 * the failure moved from registration to the first `stream()` call.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Registry } from '../../src/registry/registry.js';
import { StreamingInterfaceError } from '../../src/errors.js';

const BASE = {
  inputSchema: Type.Object({}),
  outputSchema: Type.Object({ n: Type.Number() }),
  description: 'test module',
  execute: () => ({ n: 1 }),
};

describe('registerInternal enforces the streaming annotation (STR-5)', () => {
  it('rejects streaming: true with no stream() method', () => {
    const registry = new Registry();
    expect(() =>
      registry.registerInternal('system.fake.streamer', {
        ...BASE,
        annotations: { streaming: true },
      }),
    ).toThrow(StreamingInterfaceError);
    expect(registry.has('system.fake.streamer')).toBe(false);
  });

  it('reports the same error the public door reports', () => {
    const registry = new Registry();
    let viaInternal: unknown = null;
    let viaPublic: unknown = null;
    try {
      registry.registerInternal('system.fake.streamer', { ...BASE, annotations: { streaming: true } });
    } catch (e) {
      viaInternal = e;
    }
    try {
      registry.register('executor.fake.streamer', { ...BASE, annotations: { streaming: true } });
    } catch (e) {
      viaPublic = e;
    }
    expect((viaInternal as StreamingInterfaceError).code).toBe(
      (viaPublic as StreamingInterfaceError).code,
    );
  });

  it('accepts a module that really does implement streaming', () => {
    const registry = new Registry();
    registry.registerInternal('system.real.streamer', {
      ...BASE,
      annotations: { streaming: true },
      stream: async function* () {
        yield { n: 1 };
      },
      [Symbol.for('apcore.streaming')]: true,
    });
    expect(registry.has('system.real.streamer')).toBe(true);
  });

  it('a non-streaming internal module is unaffected', () => {
    const registry = new Registry();
    registry.registerInternal('system.plain.mod', { ...BASE, annotations: { streaming: false } });
    expect(registry.has('system.plain.mod')).toBe(true);
  });

  it('the reserved-word bypass is still the ONLY bypass', () => {
    const registry = new Registry();
    // `system.*` is reserved for `register()` and allowed here — unchanged.
    registry.registerInternal('system.allowed.mod', BASE);
    expect(registry.has('system.allowed.mod')).toBe(true);
  });
});
