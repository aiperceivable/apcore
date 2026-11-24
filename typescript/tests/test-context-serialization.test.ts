import { describe, it, expect, vi } from 'vitest';
import { Context, createIdentity } from '../src/context.js';

function makeCtx(): Context {
  return Context.create(
    createIdentity('user-1', 'user', ['admin'], { org: 'acme' }),
  );
}

describe('Context.serialize()', () => {
  it('AC-003: includes _context_version: 1', () => {
    const result = makeCtx().serialize();
    expect(result._context_version).toBe(1);
  });

  it('includes required fields', () => {
    const result = makeCtx().serialize();
    expect(result).toHaveProperty('trace_id');
    expect(result).toHaveProperty('caller_id');
    expect(result).toHaveProperty('call_chain');
    expect(result).toHaveProperty('identity');
  });

  it('serializes identity with correct structure', () => {
    const result = makeCtx().serialize();
    const identity = result.identity as Record<string, unknown>;
    expect(identity.id).toBe('user-1');
    expect(identity.type).toBe('user');
    expect(identity.roles).toEqual(['admin']);
    expect(identity.attrs).toEqual({ org: 'acme' });
  });

  it('AC-004: excludes executor, services, cancelToken, globalDeadline', () => {
    const result = makeCtx().serialize();
    expect(result).not.toHaveProperty('executor');
    expect(result).not.toHaveProperty('services');
    expect(result).not.toHaveProperty('cancelToken');
    expect(result).not.toHaveProperty('cancel_token');
    expect(result).not.toHaveProperty('globalDeadline');
    expect(result).not.toHaveProperty('global_deadline');
  });

  it('AC-005: filters _-prefixed keys from data', () => {
    const ctx = makeCtx();
    ctx.data['_apcore.internal'] = 'hidden';
    ctx.data['_secret_key'] = 'hidden';
    ctx.data['public.counter'] = 42;
    ctx.data['app.name'] = 'test';
    const result = ctx.serialize();
    const data = result.data as Record<string, unknown>;
    expect(data).not.toHaveProperty('_apcore.internal');
    expect(data).not.toHaveProperty('_secret_key');
    expect(data['public.counter']).toBe(42);
    expect(data['app.name']).toBe('test');
  });

  it('produces empty data when only _-prefixed keys exist', () => {
    const ctx = makeCtx();
    ctx.data['_private'] = 'hidden';
    const result = ctx.serialize();
    expect(result.data).toEqual({});
  });

  it('uses snake_case keys in output', () => {
    const result = makeCtx().serialize();
    expect(result).toHaveProperty('trace_id');
    expect(result).toHaveProperty('caller_id');
    expect(result).toHaveProperty('call_chain');
    expect(result).not.toHaveProperty('traceId');
    expect(result).not.toHaveProperty('callerId');
    expect(result).not.toHaveProperty('callChain');
  });
});

describe('Context.deserialize()', () => {
  it('roundtrip preserves fields', () => {
    const ctx = makeCtx();
    ctx.data['app.counter'] = 42;
    const serialized = ctx.serialize();
    const restored = Context.deserialize(serialized);
    expect(restored.traceId).toBe(ctx.traceId);
    expect(restored.callerId).toBe(ctx.callerId);
    expect(restored.data['app.counter']).toBe(42);
    expect(restored.identity).not.toBeNull();
    expect(restored.identity!.id).toBe('user-1');
  });

  it('deserialized context has null executor', () => {
    const serialized = makeCtx().serialize();
    const restored = Context.deserialize(serialized);
    expect(restored.executor).toBeNull();
  });

  it('deserialized context has null services', () => {
    const serialized = makeCtx().serialize();
    const restored = Context.deserialize(serialized);
    expect(restored.services).toBeNull();
  });

  it('deserialized context has null cancelToken', () => {
    const serialized = makeCtx().serialize();
    const restored = Context.deserialize(serialized);
    expect(restored.cancelToken).toBeNull();
  });

  it('deserialized context has null globalDeadline', () => {
    const serialized = makeCtx().serialize();
    const restored = Context.deserialize(serialized);
    expect(restored.globalDeadline).toBeNull();
  });

  it('future _context_version > 1 logs warning but succeeds', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const data = {
      _context_version: 99,
      trace_id: 'abc-123',
      caller_id: 'test',
      call_chain: [] as string[],
      data: {},
    };
    const restored = Context.deserialize(data);
    expect(restored.traceId).toBe('abc-123');
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('_context_version'),
    );
    warnSpy.mockRestore();
  });

  it('round-trips redactedOutput through toJSON/fromJSON (A-D-009)', () => {
    const ctx = makeCtx();
    ctx.redactedOutput = { token: '***', value: 1 };
    const serialized = ctx.toJSON();
    expect(serialized.redacted_output).toEqual({ token: '***', value: 1 });
    const restored = Context.fromJSON(serialized);
    expect(restored.redactedOutput).toEqual({ token: '***', value: 1 });
  });

  it('does not crash on unknown top-level fields', () => {
    const data = {
      _context_version: 1,
      trace_id: 'abc-123',
      caller_id: 'test',
      call_chain: [] as string[],
      data: { custom: 'value' },
      future_field: 'should_not_crash',
    };
    const restored = Context.deserialize(data);
    expect(restored.traceId).toBe('abc-123');
    expect(restored.data['custom']).toBe('value');
  });
});
