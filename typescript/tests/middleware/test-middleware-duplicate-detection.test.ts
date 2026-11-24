import { describe, it, expect, vi, afterEach } from 'vitest';
import { MiddlewareManager } from '../../src/middleware/manager.js';
import { Middleware } from '../../src/middleware/base.js';
import type { Context } from '../../src/context.js';

class TestMiddleware extends Middleware {
  constructor(readonly priority: number = 0) { super(priority); }
  override async before() { return null; }
  override async after(_mId: string, _inp: Record<string, unknown>, out: Record<string, unknown>) { return out; }
  override async onError() { return null; }
}

describe('Middleware duplicate detection (#64)', () => {
  afterEach(() => vi.restoreAllMocks());

  it('single registration produces no warning', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const mgr = new MiddlewareManager();
    mgr.add(new TestMiddleware());
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it('two registrations of same class produce one console.warn', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const mgr = new MiddlewareManager();
    mgr.add(new TestMiddleware());
    mgr.add(new TestMiddleware());
    expect(warnSpy).toHaveBeenCalledTimes(1);
    expect(warnSpy.mock.calls[0][0]).toContain('Duplicate middleware');
  });

  it('allowDuplicate: true suppresses warning', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const mgr = new MiddlewareManager();
    mgr.add(new TestMiddleware());
    mgr.add(new TestMiddleware(), { allowDuplicate: true });
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it('distinct identityKey suppresses warning', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const mgr = new MiddlewareManager();
    mgr.add(new TestMiddleware(), { identityKey: 'mw-1' });
    mgr.add(new TestMiddleware(), { identityKey: 'mw-2' });
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it('both middlewares execute in before/after', async () => {
    const mgr = new MiddlewareManager();
    const calls: string[] = [];

    class MW1 extends Middleware {
      override priority = 0;
      override async before() { calls.push('before-1'); return null; }
      override async after(_m: string, _i: Record<string,unknown>, o: Record<string,unknown>) { calls.push('after-1'); return o; }
      override async onError() { return null; }
    }
    class MW2 extends Middleware {
      override priority = 0;
      override async before() { calls.push('before-2'); return null; }
      override async after(_m: string, _i: Record<string,unknown>, o: Record<string,unknown>) { calls.push('after-2'); return o; }
      override async onError() { return null; }
    }

    mgr.add(new MW1(), { identityKey: 'mw1' });
    mgr.add(new MW2(), { identityKey: 'mw2' });

    const ctx = {} as Context;
    await mgr.executeBefore('m', {}, ctx);
    await mgr.executeAfter('m', {}, {}, ctx);
    expect(calls).toEqual(['before-1', 'before-2', 'after-2', 'after-1']);
  });

  it('remove cleans up identity registry', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const mgr = new MiddlewareManager();
    const mw = new TestMiddleware();
    mgr.add(mw);
    mgr.remove(mw);
    // After removal, registering the same class again should not warn
    mgr.add(new TestMiddleware());
    expect(warnSpy).not.toHaveBeenCalled();
  });
});
