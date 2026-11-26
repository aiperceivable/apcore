/**
 * Integration tests for the approval system through the full executor pipeline.
 */

import { describe, expect, it, vi } from 'vitest';
import { Type } from '@sinclair/typebox';
import { ACL } from '../src/acl.js';
import type { ACLRule } from '../src/acl.js';
import {
  AlwaysDenyHandler,
  AutoApproveHandler,
  CallbackApprovalHandler,
  createApprovalResult,
} from '../src/approval.js';
import type { ApprovalHandler, ApprovalRequest, ApprovalResult } from '../src/approval.js';
import { Context, createIdentity } from '../src/context.js';
import { FunctionModule } from '../src/decorator.js';
import {
  ACLDeniedError,
  ApprovalDeniedError,
  ApprovalPendingError,
} from '../src/errors.js';
import { Executor } from '../src/executor.js';
import { ExtensionManager } from '../src/extensions.js';
import { Middleware } from '../src/middleware/index.js';
import { Registry } from '../src/registry/registry.js';
import { STREAMING_MARKER } from '../src/streaming.js';

// ---------------------------------------------------------------------------
// Module implementations
// ---------------------------------------------------------------------------

const PermissiveInput = Type.Object({}, { additionalProperties: true });
const PermissiveOutput = Type.Object({}, { additionalProperties: true });

function createDestructiveModule(): FunctionModule {
  return new FunctionModule({
    moduleId: 'admin.delete_user',
    inputSchema: PermissiveInput,
    outputSchema: PermissiveOutput,
    description: 'Delete user data',
    tags: ['admin', 'destructive'],
    annotations: {
      readonly: false,
      destructive: true,
      idempotent: false,
      requiresApproval: true,
      openWorld: true,
      streaming: false,
      cacheable: false,
      cacheTtl: 0,
      cacheKeyFields: null,
      paginated: false,
      paginationStyle: 'cursor' as const, extra: {},
    },
    execute: (inputs) => ({ deleted: true, user_id: inputs['user_id'] }),
  });
}

function createSafeModule(): FunctionModule {
  return new FunctionModule({
    moduleId: 'data.read',
    inputSchema: PermissiveInput,
    outputSchema: PermissiveOutput,
    description: 'Read data',
    annotations: {
      readonly: true,
      destructive: false,
      idempotent: false,
      requiresApproval: false,
      openWorld: true,
      streaming: false,
      cacheable: false,
      cacheTtl: 0,
      cacheKeyFields: null,
      paginated: false,
      paginationStyle: 'cursor' as const, extra: {},
    },
    execute: () => ({ data: 'safe' }),
  });
}

class RecordingMiddleware extends Middleware {
  beforeCalls: string[] = [];
  afterCalls: string[] = [];
  errorCalls: string[] = [];

  override before(moduleId: string, _inputs: Record<string, unknown>, _context: Context): Record<string, unknown> | null {
    this.beforeCalls.push(moduleId);
    return null;
  }

  override after(moduleId: string, _inputs: Record<string, unknown>, _output: Record<string, unknown>, _context: Context): Record<string, unknown> | null {
    this.afterCalls.push(moduleId);
    return null;
  }

  override onError(moduleId: string, _inputs: Record<string, unknown>, _error: Error, _context: Context): Record<string, unknown> | null {
    this.errorCalls.push(moduleId);
    return null;
  }
}

function createTestRegistry(): Registry {
  const reg = new Registry();
  reg.register('admin.delete_user', createDestructiveModule());
  reg.register('data.read', createSafeModule());
  return reg;
}

// ---------------------------------------------------------------------------
// ACL + Approval interaction
// ---------------------------------------------------------------------------

describe('ApprovalWithACL', () => {
  it('ACL deny fires before approval gate', async () => {
    const registry = createTestRegistry();
    const acl = new ACL([{ callers: ['*'], targets: ['admin.*'], effect: 'deny', description: '', conditions: null } as ACLRule]);

    let approvalCalled = false;
    const handler = new CallbackApprovalHandler(async () => {
      approvalCalled = true;
      return createApprovalResult({ status: 'approved' });
    });
    const executor = new Executor({ registry, acl, approvalHandler: handler });

    await expect(executor.call('admin.delete_user', { user_id: '123' })).rejects.toThrow(ACLDeniedError);
    expect(approvalCalled).toBe(false);
  });

  it('ACL allows but approval denies', async () => {
    const registry = createTestRegistry();
    const acl = new ACL([{ callers: ['*'], targets: ['admin.*'], effect: 'allow', description: '', conditions: null } as ACLRule]);

    const executor = new Executor({ registry, acl, approvalHandler: new AlwaysDenyHandler() });
    await expect(executor.call('admin.delete_user', { user_id: '123' })).rejects.toThrow(ApprovalDeniedError);
  });

  it('ACL allows and approval approves', async () => {
    const registry = createTestRegistry();
    const acl = new ACL([{ callers: ['*'], targets: ['*'], effect: 'allow', description: '', conditions: null } as ACLRule]);

    const executor = new Executor({ registry, acl, approvalHandler: new AutoApproveHandler() });
    const result = await executor.call('admin.delete_user', { user_id: '123' });
    expect(result['deleted']).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Approval + Middleware interaction
// ---------------------------------------------------------------------------

describe('ApprovalWithMiddleware', () => {
  it('middleware runs after approval passes', async () => {
    const registry = createTestRegistry();
    const mw = new RecordingMiddleware();
    const executor = new Executor({
      registry,
      middlewares: [mw],
      approvalHandler: new AutoApproveHandler(),
    });
    const result = await executor.call('admin.delete_user', { user_id: '123' });
    expect(result['deleted']).toBe(true);
    expect(mw.beforeCalls).toContain('admin.delete_user');
    expect(mw.afterCalls).toContain('admin.delete_user');
  });

  it('middleware not reached when approval denied', async () => {
    const registry = createTestRegistry();
    const mw = new RecordingMiddleware();
    const executor = new Executor({
      registry,
      middlewares: [mw],
      approvalHandler: new AlwaysDenyHandler(),
    });
    await expect(executor.call('admin.delete_user', { user_id: '123' })).rejects.toThrow(ApprovalDeniedError);
    expect(mw.beforeCalls).toHaveLength(0);
    expect(mw.afterCalls).toHaveLength(0);
  });

  it('safe module with middleware and handler still works normally', async () => {
    const registry = createTestRegistry();
    const mw = new RecordingMiddleware();
    const executor = new Executor({
      registry,
      middlewares: [mw],
      approvalHandler: new AlwaysDenyHandler(),
    });
    const result = await executor.call('data.read');
    expect(result['data']).toBe('safe');
    expect(mw.beforeCalls).toContain('data.read');
    expect(mw.afterCalls).toContain('data.read');
  });
});

// ---------------------------------------------------------------------------
// Callback with identity
// ---------------------------------------------------------------------------

describe('ApprovalCallback', () => {
  it('callback receives identity from context', async () => {
    const registry = createTestRegistry();
    const captured: ApprovalRequest[] = [];
    const handler = new CallbackApprovalHandler(async (request: ApprovalRequest) => {
      captured.push(request);
      return createApprovalResult({ status: 'approved', approvedBy: 'callback' });
    });
    const executor = new Executor({ registry, approvalHandler: handler });
    const identity = createIdentity('user-42', 'user', ['admin']);
    const ctx = Context.create(identity);

    await executor.call('admin.delete_user', { user_id: '123' }, ctx);

    expect(captured).toHaveLength(1);
    expect(captured[0].context.identity).not.toBeNull();
    expect(captured[0].context.identity!.id).toBe('user-42');
    expect(captured[0].context.identity!.roles).toContain('admin');
  });

  it('callback can make conditional decisions', async () => {
    const registry = createTestRegistry();
    const handler = new CallbackApprovalHandler(async (request: ApprovalRequest) => {
      if (request.context.identity && request.context.identity.roles.includes('admin')) {
        return createApprovalResult({ status: 'approved', approvedBy: 'policy' });
      }
      return createApprovalResult({ status: 'rejected', reason: 'Admin role required' });
    });
    const executor = new Executor({ registry, approvalHandler: handler });

    // Admin user → approved
    const adminCtx = Context.create(createIdentity('admin-1', 'user', ['admin']));
    const result = await executor.call('admin.delete_user', { user_id: '123' }, adminCtx);
    expect(result['deleted']).toBe(true);

    // Regular user → denied
    const userCtx = Context.create(createIdentity('user-1', 'user', ['viewer']));
    try {
      await executor.call('admin.delete_user', { user_id: '123' }, userCtx);
      expect.unreachable('Should have thrown');
    } catch (e) {
      expect(e).toBeInstanceOf(ApprovalDeniedError);
      expect((e as ApprovalDeniedError).message).toContain('Admin role required');
    }
  });
});

// ---------------------------------------------------------------------------
// Phase B flow
// ---------------------------------------------------------------------------

describe('ApprovalPhaseB', () => {
  it('validates the resume request through the scoped handler', async () => {
    const registry = createTestRegistry();
    let pendingArgs: Record<string, unknown> | null = null;
    const handler: ApprovalHandler = {
      async requestApproval(request: ApprovalRequest) {
        pendingArgs = { ...request.arguments };
        return createApprovalResult({ status: 'pending', approvalId: 'test-resume-credential' });
      },
      async checkApproval() {
        throw new Error('The status query cannot authorize execution');
      },
      async checkApprovalForCall(approvalId: string, request: ApprovalRequest) {
        expect(request.moduleId).toBe('admin.delete_user');
        expect(request.context.identity?.id).toBe('operator');
        expect(request.arguments).toEqual(pendingArgs);
        expect(approvalId).toBe('test-resume-credential');
        pendingArgs = null;
        return createApprovalResult({ status: 'approved', approvedBy: 'reviewer' });
      },
    };

    const executor = new Executor({ registry, approvalHandler: handler });
    const context = Context.create(createIdentity('operator', 'user'));
    await expect(executor.call('admin.delete_user', { user_id: '123' }, context)).rejects.toMatchObject({
      approvalId: 'test-resume-credential',
    });
    const result = await executor.call('admin.delete_user', {
      user_id: '123',
      _approval_token: 'test-resume-credential',
    }, context);
    expect(result['deleted']).toBe(true);
    expect(pendingArgs).toBeNull();
  });

  it('keeps synchronous approval working for a status-only handler', async () => {
    const executor = new Executor({ registry: createTestRegistry(), approvalHandler: new AutoApproveHandler() });
    await expect(executor.call('admin.delete_user', { user_id: '123' })).resolves.toMatchObject({ deleted: true });
    await expect(executor.call('admin.delete_user', {
      user_id: '123', _approval_token: 'test-resume-credential',
    })).rejects.toThrow(ApprovalDeniedError);
  });
});

// ---------------------------------------------------------------------------
// ExtensionManager wiring
// ---------------------------------------------------------------------------

describe('ApprovalExtensionManager', () => {
  it('wires approval handler via ExtensionManager', async () => {
    const registry = createTestRegistry();
    const em = new ExtensionManager();
    em.register('approval_handler', new AutoApproveHandler());

    const executor = new Executor({ registry });
    em.apply(registry, executor);

    const result = await executor.call('admin.delete_user', { user_id: '123' });
    expect(result['deleted']).toBe(true);
  });

  it('ExtensionManager with deny handler blocks execution', async () => {
    const registry = createTestRegistry();
    const em = new ExtensionManager();
    em.register('approval_handler', new AlwaysDenyHandler());

    const executor = new Executor({ registry });
    em.apply(registry, executor);

    await expect(executor.call('admin.delete_user', { user_id: '123' })).rejects.toThrow(ApprovalDeniedError);
  });
});

// ---------------------------------------------------------------------------
// Public API imports
// ---------------------------------------------------------------------------

describe('ApprovalImports', () => {
  it('all types importable from apcore package', async () => {
    const apcore = await import('../src/index.js');
    expect(apcore.AlwaysDenyHandler).toBeDefined();
    expect(apcore.AutoApproveHandler).toBeDefined();
    expect(apcore.CallbackApprovalHandler).toBeDefined();
    expect(apcore.createApprovalRequest).toBeDefined();
    expect(apcore.createApprovalResult).toBeDefined();
    expect(apcore.ApprovalError).toBeDefined();
    expect(apcore.ApprovalDeniedError).toBeDefined();
    expect(apcore.ApprovalTimeoutError).toBeDefined();
    expect(apcore.ApprovalPendingError).toBeDefined();
  });
});

// ---------------------------------------------------------------------------
// `_approval_token` is a protocol key, not part of a module's input contract
// ---------------------------------------------------------------------------

describe('_approval_token versus additionalProperties: false', () => {
  const StrictInput = Type.Object({ a: Type.String() }, { additionalProperties: false });
  const AnyOutput = Type.Object({}, { additionalProperties: true });

  function gatedAnnotations(requiresApproval: boolean) {
    return {
      readonly: false,
      destructive: false,
      idempotent: false,
      requiresApproval,
      openWorld: true,
      streaming: false,
      cacheable: false,
      cacheTtl: 0,
      cacheKeyFields: null,
      paginated: false,
      paginationStyle: 'cursor' as const,
      extra: {},
    };
  }

  async function buildRegistry(requiresApproval: boolean, seen: Record<string, unknown>[]) {
    const registry = new Registry();
    await registry.register(
      'probe.strict',
      new FunctionModule({
        moduleId: 'probe.strict',
        inputSchema: StrictInput,
        outputSchema: AnyOutput,
        annotations: gatedAnnotations(requiresApproval),
        execute: (inputs) => {
          seen.push(inputs);
          return { ok: inputs['a'] };
        },
      }),
    );
    return registry;
  }

  it('is stripped when the module needs no approval', async () => {
    const seen: Record<string, unknown>[] = [];
    const executor = new Executor({
      registry: await buildRegistry(false, seen),
      approvalHandler: new AutoApproveHandler(),
    });
    const result = await executor.call('probe.strict', { a: 'x', _approval_token: 'tok' });
    expect(result['ok']).toBe('x');
    expect(seen[0]).not.toHaveProperty('_approval_token');
  });

  it('is stripped when approval is required but no handler is configured', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const seen: Record<string, unknown>[] = [];
    const executor = new Executor({ registry: await buildRegistry(true, seen) });
    const result = await executor.call('probe.strict', { a: 'x', _approval_token: 'tok' });
    warnSpy.mockRestore();
    expect(result['ok']).toBe('x');
    expect(seen[0]).not.toHaveProperty('_approval_token');
  });

  it('is stripped on the approved path', async () => {
    const seen: Record<string, unknown>[] = [];
    class ProbeHandler extends AutoApproveHandler {
      async checkApprovalForCall(credential: string, request: ApprovalRequest) {
        expect(credential).toBe('tok');
        expect(request.arguments).toEqual({ a: 'x' });
        return createApprovalResult({ status: 'approved' });
      }
    }
    const executor = new Executor({
      registry: await buildRegistry(true, seen),
      approvalHandler: new ProbeHandler(),
    });
    const result = await executor.call('probe.strict', { a: 'x', _approval_token: 'tok' });
    expect(result['ok']).toBe('x');
    expect(seen[0]).not.toHaveProperty('_approval_token');
  });

  it('leaves the caller-supplied inputs object untouched', async () => {
    const seen: Record<string, unknown>[] = [];
    const executor = new Executor({
      registry: await buildRegistry(false, seen),
      approvalHandler: new AutoApproveHandler(),
    });
    const inputs: Record<string, unknown> = { a: 'x', _approval_token: 'tok' };
    await executor.call('probe.strict', inputs);
    expect(inputs['_approval_token']).toBe('tok');
  });

  it('keeps the credential out of unary and native stream inputs when the gate is absent', async () => {
    const registry = new Registry();
    await registry.register('probe.streaming', {
      [STREAMING_MARKER]: true,
      description: 'Echo the application input',
      inputSchema: StrictInput,
      outputSchema: AnyOutput,
      execute: (inputs: Record<string, unknown>) => inputs,
      async *stream(inputs: Record<string, unknown>) {
        yield inputs;
      },
    });
    const executor = new Executor({ registry });
    executor.currentStrategy.remove('approval_gate');
    const inspectedInputs = vi.fn();
    executor.addStepMiddleware({ beforeStep: (_, state) => inspectedInputs({ ...state.context.inputs }) });
    const inputs = { a: 'x', _approval_token: 'sample-credential' };

    await expect(executor.callAsync('probe.streaming', inputs)).resolves.toEqual({ a: 'x' });
    const stream = executor.streamWithCompletion('probe.streaming', inputs);
    expect(await stream.chunks.next()).toEqual({ value: { a: 'x' }, done: false });
    expect(await stream.chunks.next()).toMatchObject({ done: true });
    await expect(stream.completion).resolves.toMatchObject({ reason: 'completed', output: { a: 'x' } });

    expect(inspectedInputs).toHaveBeenCalled();
    for (const [argumentsSeen] of inspectedInputs.mock.calls) expect(argumentsSeen).toEqual({ a: 'x' });
    expect(inputs._approval_token).toBe('sample-credential');
  });

  it('rejects malformed credentials on real calls with no gate', async () => {
    const executor = new Executor({ registry: await buildRegistry(false, []) });
    executor.currentStrategy.remove('approval_gate');
    await expect(executor.call('probe.strict', { a: 'x', _approval_token: 42 })).rejects.toMatchObject({
      code: 'GENERAL_INVALID_INPUT',
      message: expect.stringContaining('_approval_token must be a string'),
    });
  });

  it('ignores credentials in preflight and keeps them out of module hooks', async () => {
    const preflight = vi.fn(() => []);
    const preview = vi.fn(() => null);
    const handler = new CallbackApprovalHandler(async () => {
      throw new Error('Preflight must never request approval');
    });
    const registry = new Registry();
    await registry.register('probe.preflight', {
      description: 'Report application inputs',
      inputSchema: StrictInput,
      outputSchema: AnyOutput,
      annotations: gatedAnnotations(true),
      execute: () => ({}),
      preflight,
      preview,
    });
    const executor = new Executor({ registry, approvalHandler: handler });
    const result = await executor.validate('probe.preflight', { a: 'x', _approval_token: 42 });
    expect(result).toMatchObject({ valid: true, requiresApproval: true });
    expect(preflight).toHaveBeenCalledWith({ a: 'x' }, expect.any(Context));
    expect(preview).toHaveBeenCalledWith({ a: 'x' }, expect.any(Context));
  });

  it('gives approval a detached snapshot before trusted middleware rewrites nested inputs', async () => {
    const requestApproval = vi.fn(async (_request: ApprovalRequest) => createApprovalResult({ status: 'approved' }));
    const registry = new Registry();
    await registry.register('probe.snapshot', {
      description: 'Echo a nested value',
      inputSchema: Type.Object({ message: Type.Object({ text: Type.String() }) }),
      outputSchema: AnyOutput,
      annotations: gatedAnnotations(true),
      execute: (inputs: Record<string, unknown>) => ({ ...inputs }),
    });
    const executor = new Executor({
      registry,
      approvalHandler: new CallbackApprovalHandler(requestApproval),
    });
    executor.useBefore((_moduleId, inputs) => {
      (inputs.message as { text: string }).text = 'normalized';
      return null;
    });

    await expect(executor.call('probe.snapshot', { message: { text: 'caller value' } })).resolves.toEqual({
      message: { text: 'normalized' },
    });
    expect(requestApproval).toHaveBeenCalledOnce();
    expect(requestApproval.mock.calls[0][0].arguments).toEqual({ message: { text: 'caller value' } });
  });
});
