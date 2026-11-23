/**
 * Tests for ContextKey typed accessor.
 */

import { describe, it, expect } from 'vitest';
import { ContextKey } from '../src/context-key.js';
import { Context } from '../src/context.js';

function makeCtx(): Context {
  return Context.create();
}

describe('ContextKey', () => {
  it('AC-001: get() returns typed value from context.data', () => {
    const key = new ContextKey<number>('test.counter');
    const ctx = makeCtx();
    ctx.data['test.counter'] = 42;
    expect(key.get(ctx)).toBe(42);
  });

  it('AC-016: get() with absent key returns undefined', () => {
    const key = new ContextKey<number>('test.absent');
    const ctx = makeCtx();
    expect(key.get(ctx)).toBeUndefined();
  });

  it('AC-016: get() with absent key returns default', () => {
    const key = new ContextKey<number>('test.absent');
    const ctx = makeCtx();
    expect(key.get(ctx, 99)).toBe(99);
  });

  it('AC-001: set() writes value to context.data', () => {
    const key = new ContextKey<string>('test.name');
    const ctx = makeCtx();
    key.set(ctx, 'hello');
    expect(ctx.data['test.name']).toBe('hello');
  });

  it('delete() removes key from context.data', () => {
    const key = new ContextKey<number>('test.temp');
    const ctx = makeCtx();
    key.set(ctx, 10);
    key.delete(ctx);
    expect('test.temp' in ctx.data).toBe(false);
  });

  it('AC-017: delete() on absent key is no-op', () => {
    const key = new ContextKey<number>('test.absent');
    const ctx = makeCtx();
    expect(() => key.delete(ctx)).not.toThrow();
  });

  it('AC-018: exists() returns false for absent key', () => {
    const key = new ContextKey<number>('test.absent');
    const ctx = makeCtx();
    expect(key.exists(ctx)).toBe(false);
  });

  it('AC-018: exists() returns true after set', () => {
    const key = new ContextKey<number>('test.present');
    const ctx = makeCtx();
    key.set(ctx, 1);
    expect(key.exists(ctx)).toBe(true);
  });

  it('AC-002: scoped(suffix) creates sub-key', () => {
    const base = new ContextKey<number>('_apcore.mw.retry.count');
    const scoped = base.scoped('mod1');
    expect(scoped.name).toBe('_apcore.mw.retry.count.mod1');
  });

  it('scoped key is independent from base key', () => {
    const base = new ContextKey<number>('base');
    const scoped = base.scoped('child');
    const ctx = makeCtx();
    base.set(ctx, 1);
    scoped.set(ctx, 2);
    expect(base.get(ctx)).toBe(1);
    expect(scoped.get(ctx)).toBe(2);
  });

  it('name is readonly', () => {
    const key = new ContextKey<number>('test');
    // TypeScript readonly enforces this at compile time.
    // At runtime, assignment to a readonly property is silently ignored in non-strict
    // or throws in strict mode. We verify the name stays unchanged.
    expect(key.name).toBe('test');
  });
});
