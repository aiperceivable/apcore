import { describe, it, expect } from 'vitest';
import { Context, createIdentity } from '../src/context.js';
import type { Identity } from '../src/context.js';

describe('createIdentity', () => {
  it('creates identity with defaults', () => {
    const id = createIdentity('user1');
    expect(id.id).toBe('user1');
    expect(id.type).toBe('user');
    expect(id.roles).toEqual([]);
    expect(id.attrs).toEqual({});
  });

  it('creates identity with all fields', () => {
    const id = createIdentity('admin1', 'admin', ['superuser'], { org: 'acme' });
    expect(id.id).toBe('admin1');
    expect(id.type).toBe('admin');
    expect(id.roles).toEqual(['superuser']);
    expect(id.attrs).toEqual({ org: 'acme' });
  });

  it('returns frozen object', () => {
    const id = createIdentity('u1');
    expect(Object.isFrozen(id)).toBe(true);
    expect(Object.isFrozen(id.roles)).toBe(true);
    expect(Object.isFrozen(id.attrs)).toBe(true);
  });
});

describe('Context.create()', () => {
  it('creates context with unique traceId', () => {
    const ctx1 = Context.create();
    const ctx2 = Context.create();
    expect(ctx1.traceId).toBeDefined();
    expect(ctx2.traceId).toBeDefined();
    expect(ctx1.traceId).not.toBe(ctx2.traceId);
  });

  it('has null callerId by default', () => {
    const ctx = Context.create();
    expect(ctx.callerId).toBeNull();
  });

  it('has empty callChain by default', () => {
    const ctx = Context.create();
    expect(ctx.callChain).toEqual([]);
  });

  it('accepts identity (executor is bound on executor.call(), not here)', () => {
    const identity = createIdentity('u1', 'admin');
    const ctx = Context.create(identity);
    expect(ctx.identity).toBe(identity);
    // Issue #66: executor is NOT a Context.create() parameter.
    expect(ctx.executor).toBeNull();
  });

  it('defaults identity to null', () => {
    const ctx = Context.create();
    expect(ctx.identity).toBeNull();
  });

  it('defaults executor to null', () => {
    const ctx = Context.create();
    expect(ctx.executor).toBeNull();
  });

  it('defaults data to empty object', () => {
    const ctx = Context.create();
    expect(ctx.data).toEqual({});
  });

  it('accepts custom data', () => {
    const ctx = Context.create(null, null, null, { key: 'value' });
    expect(ctx.data).toEqual({ key: 'value' });
  });

  it('has null redactedInputs by default', () => {
    const ctx = Context.create();
    expect(ctx.redactedInputs).toBeNull();
  });
});

describe('Context.child()', () => {
  it('preserves traceId from parent', () => {
    const parent = Context.create();
    const child = parent.child('module.a');
    expect(child.traceId).toBe(parent.traceId);
  });

  it('sets callerId to null when parent callChain is empty', () => {
    const parent = Context.create();
    const child = parent.child('module.a');
    expect(child.callerId).toBeNull();
  });

  it('sets callerId to last element of parent callChain', () => {
    const parent = Context.create();
    const child1 = parent.child('module.a');
    expect(child1.callChain).toEqual(['module.a']);

    const child2 = child1.child('module.b');
    expect(child2.callerId).toBe('module.a');
    expect(child2.callChain).toEqual(['module.a', 'module.b']);
  });

  it('builds up callChain through multiple levels', () => {
    const root = Context.create();
    const c1 = root.child('a');
    const c2 = c1.child('b');
    const c3 = c2.child('c');
    expect(c3.callChain).toEqual(['a', 'b', 'c']);
    expect(c3.callerId).toBe('b');
  });

  it('shares data reference with parent', () => {
    const parent = Context.create(null, null, null, { shared: true });
    const child = parent.child('mod');
    expect(child.data).toBe(parent.data);

    child.data['newKey'] = 'newValue';
    expect(parent.data['newKey']).toBe('newValue');
  });

  it('preserves executor from parent (when bound via _withExecutor)', () => {
    const executor = { id: 'exec' };
    // Issue #66: executor is bound via _withExecutor (internal helper),
    // not Context.create(). Child() then propagates the bound executor.
    const parent = Context.create()._withExecutor(executor);
    const child = parent.child('mod');
    expect(child.executor).toBe(executor);
  });

  it('preserves identity from parent', () => {
    const identity = createIdentity('u1', 'admin', ['role1']);
    const parent = Context.create(identity);
    const child = parent.child('mod');
    expect(child.identity).toBe(identity);
  });

  it('resets redactedInputs to null', () => {
    const parent = Context.create();
    parent.redactedInputs = { field: '***' };
    const child = parent.child('mod');
    expect(child.redactedInputs).toBeNull();
  });

  it('does not modify parent callChain', () => {
    const parent = Context.create();
    const chainBefore = [...parent.callChain];
    parent.child('mod.a');
    expect(parent.callChain).toEqual(chainBefore);
  });
});

describe('Context.toJSON() / Context.fromJSON()', () => {
  it('round-trips context with identity', () => {
    const identity = createIdentity('user-42', 'admin', ['superuser', 'editor'], { org: 'acme' });
    const executor = { name: 'test-executor' };
    const original = new Context(
      'trace-abc',
      'caller-1',
      ['mod.a', 'mod.b'],
      executor,
      identity,
      { password: '***' },
      { transient: 'value' },
    );

    const serialized = original.toJSON();
    const restored = Context.fromJSON(serialized);

    expect(restored.traceId).toBe(original.traceId);
    expect(restored.callerId).toBe(original.callerId);
    expect(restored.callChain).toEqual(['mod.a', 'mod.b']);
    expect(restored.identity).not.toBeNull();
    expect(restored.identity!.id).toBe('user-42');
    expect(restored.identity!.type).toBe('admin');
    expect([...restored.identity!.roles]).toEqual(['superuser', 'editor']);
    expect({ ...restored.identity!.attrs }).toEqual({ org: 'acme' });
    expect(restored.redactedInputs).toEqual({ password: '***' });
    expect(restored.data).toEqual({ transient: 'value' });
  });

  it('round-trips context without identity', () => {
    const original = new Context('trace-xyz', null, [], null, null, null);
    const serialized = original.toJSON();
    const restored = Context.fromJSON(serialized);

    expect(restored.traceId).toBe('trace-xyz');
    expect(restored.callerId).toBeNull();
    expect(restored.callChain).toEqual([]);
    expect(restored.identity).toBeNull();
    expect(restored.redactedInputs).toBeNull();
  });

  it('excludes executor from toJSON output but includes data', () => {
    const ctx = new Context('trace-1', null, [], 'my-executor', null, null, { key: 'included' });
    const serialized = ctx.toJSON();

    expect(serialized).not.toHaveProperty('executor');
    expect(serialized).toHaveProperty('data');
    expect(serialized.data).toEqual({ key: 'included' });
  });

  it('executor is null after fromJSON (non-serializable)', () => {
    const ctx = new Context('trace-2', null, [], 'original-exec');
    const serialized = ctx.toJSON();
    const restored = Context.fromJSON(serialized);

    expect(restored.executor).toBeNull();
  });

  it('defaults executor to null when not provided to fromJSON', () => {
    const serialized = { traceId: 't1', callerId: null, callChain: [], identity: null, redactedInputs: null };
    const restored = Context.fromJSON(serialized);
    expect(restored.executor).toBeNull();
  });

  it('toJSON returns copies, not references', () => {
    const identity = createIdentity('u1', 'user', ['r1'], { k: 'v' });
    const ctx = new Context('t1', null, ['a', 'b'], null, identity, { field: 'val' }, { shared: true });
    const serialized = ctx.toJSON();

    // Mutate serialized copies (serialize() outputs snake_case keys)
    (serialized.call_chain as string[]).push('mutated');
    (serialized.identity as Record<string, unknown>).id = 'mutated';
    (serialized.redacted_inputs as Record<string, unknown>).extra = true;
    (serialized.data as Record<string, unknown>).extra = true;

    // Originals unchanged
    expect(ctx.callChain).toEqual(['a', 'b']);
    expect(ctx.identity!.id).toBe('u1');
    expect(ctx.redactedInputs).toEqual({ field: 'val' });
    expect(ctx.data).toEqual({ shared: true });
  });

  it('toJSON excludes internal keys starting with _', () => {
    const ctx = Context.create();
    ctx.data['visible'] = 'yes';
    ctx.data['_apcore.mw.tracing.spans'] = [1, 2, 3];
    ctx.data['_internal'] = 42;
    const serialized = ctx.toJSON();
    const data = serialized.data as Record<string, unknown>;
    expect(data['visible']).toBe('yes');
    expect(data['_apcore.mw.tracing.spans']).toBeUndefined();
    expect(data['_internal']).toBeUndefined();
  });

  it('fromJSON handles null roles and attrs gracefully', () => {
    const json = {
      traceId: 'test-id',
      callerId: null,
      callChain: [],
      identity: {
        id: 'user-1',
        type: 'user',
        roles: null,
        attrs: null,
      },
      data: {},
    };
    const ctx = Context.fromJSON(json);
    expect(ctx.identity).toBeDefined();
    expect(ctx.identity!.roles).toEqual([]);
    expect(ctx.identity!.attrs).toEqual({});
  });
});
