import { describe, it, expect } from 'vitest';
import { Middleware } from '../src/middleware/base.js';
import { BeforeMiddleware, AfterMiddleware } from '../src/middleware/adapters.js';
import { Context, createIdentity } from '../src/context.js';

function makeContext(): Context {
  return Context.create(createIdentity('test-user'));
}

describe('Middleware base class', () => {
  it('before() returns null by default', () => {
    const mw = new Middleware();
    const ctx = makeContext();
    expect(mw.before('mod.a', { x: 1 }, ctx)).toBeNull();
  });

  it('after() returns null by default', () => {
    const mw = new Middleware();
    const ctx = makeContext();
    expect(mw.after('mod.a', { x: 1 }, { y: 2 }, ctx)).toBeNull();
  });

  it('onError() returns null by default', () => {
    const mw = new Middleware();
    const ctx = makeContext();
    expect(mw.onError('mod.a', { x: 1 }, new Error('boom'), ctx)).toBeNull();
  });
});

describe('BeforeMiddleware', () => {
  it('wraps a callback and delegates to before()', () => {
    const mw = new BeforeMiddleware((moduleId, inputs) => {
      return { ...inputs, injected: moduleId };
    });
    const ctx = makeContext();
    const result = mw.before('mod.x', { a: 1 }, ctx);
    expect(result).toEqual({ a: 1, injected: 'mod.x' });
  });

  it('after() still returns null', () => {
    const mw = new BeforeMiddleware(() => ({ replaced: true }));
    const ctx = makeContext();
    expect(mw.after('mod.x', {}, {}, ctx)).toBeNull();
  });

  it('onError() still returns null', () => {
    const mw = new BeforeMiddleware(() => ({ replaced: true }));
    const ctx = makeContext();
    expect(mw.onError('mod.x', {}, new Error('fail'), ctx)).toBeNull();
  });

  it('can return null from callback', () => {
    const mw = new BeforeMiddleware(() => null);
    const ctx = makeContext();
    expect(mw.before('mod.x', { a: 1 }, ctx)).toBeNull();
  });
});

describe('AfterMiddleware', () => {
  it('wraps a callback and delegates to after()', () => {
    const mw = new AfterMiddleware((moduleId, _inputs, output) => {
      return { ...output, processedBy: moduleId };
    });
    const ctx = makeContext();
    const result = mw.after('mod.y', { a: 1 }, { out: 42 }, ctx);
    expect(result).toEqual({ out: 42, processedBy: 'mod.y' });
  });

  it('before() still returns null', () => {
    const mw = new AfterMiddleware(() => ({ replaced: true }));
    const ctx = makeContext();
    expect(mw.before('mod.y', {}, ctx)).toBeNull();
  });

  it('onError() still returns null', () => {
    const mw = new AfterMiddleware(() => ({ replaced: true }));
    const ctx = makeContext();
    expect(mw.onError('mod.y', {}, new Error('fail'), ctx)).toBeNull();
  });

  it('can return null from callback', () => {
    const mw = new AfterMiddleware(() => null);
    const ctx = makeContext();
    expect(mw.after('mod.y', {}, { out: 1 }, ctx)).toBeNull();
  });
});
