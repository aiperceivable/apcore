import { describe, it, expect } from 'vitest';
import { Context } from '../src/context.js';

describe('Context.globalDeadline', () => {
  it('AC-020: globalDeadline is accessible on Context', () => {
    const ctx = Context.create(undefined, undefined, undefined, undefined, undefined, 1234.5);
    expect(ctx.globalDeadline).toBe(1234.5);
  });

  it('globalDeadline defaults to null', () => {
    const ctx = Context.create();
    expect(ctx.globalDeadline).toBeNull();
  });

  it('globalDeadline accepts null explicitly', () => {
    const ctx = Context.create(undefined, undefined, undefined, undefined, undefined, null);
    expect(ctx.globalDeadline).toBeNull();
  });

  it('globalDeadline is number type (epoch seconds)', () => {
    const deadline = Date.now() / 1000 + 30; // 30 seconds from now
    const ctx = Context.create(undefined, undefined, undefined, undefined, undefined, deadline);
    expect(typeof ctx.globalDeadline).toBe('number');
    expect(ctx.globalDeadline).toBeGreaterThan(0);
  });

  it('child inherits globalDeadline from parent', () => {
    const parent = Context.create(undefined, undefined, undefined, undefined, undefined, 9999.9);
    const child = parent.child('child-module');
    expect(child.globalDeadline).toBe(9999.9);
  });

  it('child inherits null globalDeadline from parent', () => {
    const parent = Context.create();
    const child = parent.child('child-module');
    expect(child.globalDeadline).toBeNull();
  });

  it('globalDeadline is NOT included in toJSON output', () => {
    const ctx = Context.create(undefined, undefined, undefined, undefined, undefined, 1234.5);
    const json = ctx.toJSON();
    expect(json).not.toHaveProperty('globalDeadline');
    expect(json).not.toHaveProperty('global_deadline');
  });

  it('fromJSON sets globalDeadline to null', () => {
    const ctx = Context.create(undefined, undefined, undefined, undefined, undefined, 1234.5);
    const json = ctx.toJSON();
    const restored = Context.fromJSON(json);
    expect(restored.globalDeadline).toBeNull();
  });
});
