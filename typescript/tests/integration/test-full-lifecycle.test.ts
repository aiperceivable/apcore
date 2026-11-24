/**
 * Comprehensive lifecycle integration tests verifying the complete execution pipeline
 * with ALL features enabled simultaneously.
 */

import { describe, it, expect } from 'vitest';
import { Type } from '@sinclair/typebox';
import { Context, createIdentity } from '../../src/context.js';
import { Executor } from '../../src/executor.js';
import { FunctionModule } from '../../src/decorator.js';
import { Registry } from '../../src/registry/registry.js';
import { ACL } from '../../src/acl.js';
import type { ACLRule } from '../../src/acl.js';
import {
  CallbackApprovalHandler,
  AutoApproveHandler,
  AlwaysDenyHandler,
  createApprovalResult,
} from '../../src/approval.js';
import type { ApprovalRequest } from '../../src/approval.js';
import { Middleware } from '../../src/middleware/base.js';
import {
  ACLDeniedError,
  ApprovalDeniedError,
  SchemaValidationError,
} from '../../src/errors.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const PermissiveInput = Type.Object({}, { additionalProperties: true });
const PermissiveOutput = Type.Object({}, { additionalProperties: true });

class RecordingMiddleware extends Middleware {
  beforeCalls: string[] = [];
  afterCalls: string[] = [];
  errorCalls: string[] = [];
  order: string[] = [];

  override before(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _context: Context,
  ): Record<string, unknown> | null {
    this.beforeCalls.push(moduleId);
    this.order.push(`before:${moduleId}`);
    return null;
  }

  override after(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _output: Record<string, unknown>,
    _context: Context,
  ): Record<string, unknown> | null {
    this.afterCalls.push(moduleId);
    this.order.push(`after:${moduleId}`);
    return null;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    _error: Error,
    _context: Context,
  ): Record<string, unknown> | null {
    this.errorCalls.push(moduleId);
    this.order.push(`onError:${moduleId}`);
    return null;
  }
}

// ---------------------------------------------------------------------------
// Test 1: full pipeline with all gates enabled
// ---------------------------------------------------------------------------

describe('Full Lifecycle Integration', () => {
  it('full pipeline with all gates enabled', async () => {
    const registry = new Registry();
    registry.register('admin.action', new FunctionModule({
      execute: (inputs) => ({ result: `executed:${inputs['name']}` }),
      moduleId: 'admin.action',
      inputSchema: PermissiveInput,
      outputSchema: PermissiveOutput,
      description: 'Admin action requiring approval',
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
    }));

    const acl = new ACL([
      { callers: ['*'], targets: ['*'], effect: 'allow', description: 'Allow all' },
    ], 'deny');

    let approvalCalled = false;
    const approvalHandler = new CallbackApprovalHandler(async (_request: ApprovalRequest) => {
      approvalCalled = true;
      return createApprovalResult({ status: 'approved', approvedBy: 'test' });
    });

    const mw = new RecordingMiddleware();

    const executor = new Executor({
      registry,
      acl,
      approvalHandler: approvalHandler,
      middlewares: [mw],
    });

    const result = await executor.call('admin.action', { name: 'test_op' });

    // All gates fired
    expect(approvalCalled).toBe(true);
    expect(mw.beforeCalls).toContain('admin.action');
    expect(mw.afterCalls).toContain('admin.action');
    expect(mw.errorCalls).toHaveLength(0);

    // Correct order: before -> after (approval fires before middleware)
    expect(mw.order).toEqual(['before:admin.action', 'after:admin.action']);

    // Correct result returned
    expect(result['result']).toBe('executed:test_op');
  });

  // ---------------------------------------------------------------------------
  // Test 2: ACL denies before approval fires
  // ---------------------------------------------------------------------------

  it('ACL denies before approval fires', async () => {
    const registry = new Registry();
    registry.register('admin.action', new FunctionModule({
      execute: () => ({ done: true }),
      moduleId: 'admin.action',
      inputSchema: PermissiveInput,
      outputSchema: PermissiveOutput,
      description: 'Admin action',
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
    }));

    const acl = new ACL([
      { callers: ['*'], targets: ['admin.*'], effect: 'deny', description: 'Deny admin', conditions: null } as ACLRule,
    ], 'deny');

    let approvalCalled = false;
    const approvalHandler = new CallbackApprovalHandler(async () => {
      approvalCalled = true;
      return createApprovalResult({ status: 'approved' });
    });

    const mw = new RecordingMiddleware();

    const executor = new Executor({
      registry,
      acl,
      approvalHandler,
      middlewares: [mw],
    });

    await expect(executor.call('admin.action', {})).rejects.toThrow(ACLDeniedError);
    expect(approvalCalled).toBe(false);
    expect(mw.beforeCalls).toHaveLength(0);
    expect(mw.afterCalls).toHaveLength(0);
  });

  // ---------------------------------------------------------------------------
  // Test 3: approval denied before middleware fires
  // ---------------------------------------------------------------------------

  it('approval denied before middleware fires', async () => {
    const registry = new Registry();
    registry.register('admin.action', new FunctionModule({
      execute: () => ({ done: true }),
      moduleId: 'admin.action',
      inputSchema: PermissiveInput,
      outputSchema: PermissiveOutput,
      description: 'Admin action',
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
    }));

    const acl = new ACL([
      { callers: ['*'], targets: ['*'], effect: 'allow', description: 'Allow all', conditions: null } as ACLRule,
    ], 'deny');

    const mw = new RecordingMiddleware();

    const executor = new Executor({
      registry,
      acl,
      approvalHandler: new AlwaysDenyHandler(),
      middlewares: [mw],
    });

    await expect(executor.call('admin.action', {})).rejects.toThrow(ApprovalDeniedError);
    expect(mw.beforeCalls).toHaveLength(0);
    expect(mw.afterCalls).toHaveLength(0);
  });

  // ---------------------------------------------------------------------------
  // Test 4: nested module call full lifecycle
  // ---------------------------------------------------------------------------

  it('nested module call full lifecycle', async () => {
    const registry = new Registry();

    registry.register('mod.b', new FunctionModule({
      execute: (inputs) => ({ answer: (inputs['x'] as number) + 1 }),
      moduleId: 'mod.b',
      inputSchema: Type.Object({ x: Type.Number() }),
      outputSchema: Type.Object({ answer: Type.Number() }),
      description: 'Module B',
    }));

    registry.register('mod.a', new FunctionModule({
      execute: async (_inputs: Record<string, unknown>, context: Context) => {
        const exec = context.executor as Executor;
        const bResult = await exec.call('mod.b', { x: 10 }, context);
        return { combined: bResult['answer'] };
      },
      moduleId: 'mod.a',
      inputSchema: PermissiveInput,
      outputSchema: PermissiveOutput,
      description: 'Module A calls B',
    }));

    const executor = new Executor({ registry });
    const ctx = Context.create(createIdentity('user1'));

    const result = await executor.call('mod.a', {}, ctx);
    expect(result['combined']).toBe(11);

    // The context child() preserves traceId and builds callChain.
    // We cannot inspect internal ctx from outside easily, but result
    // proves both modules ran.
  });

  // ---------------------------------------------------------------------------
  // Test 5: context.data shared between nested calls
  // ---------------------------------------------------------------------------

  it('context.data shared between nested calls', async () => {
    const registry = new Registry();

    registry.register('mod.b', new FunctionModule({
      execute: (_inputs: Record<string, unknown>, context: Context) => {
        // Read the value set by module A
        return { received: context.data['sharedKey'] as string };
      },
      moduleId: 'mod.b',
      inputSchema: PermissiveInput,
      outputSchema: PermissiveOutput,
      description: 'Module B reads shared data',
    }));

    registry.register('mod.a', new FunctionModule({
      execute: async (_inputs: Record<string, unknown>, context: Context) => {
        context.data['sharedKey'] = 'hello_from_a';
        const exec = context.executor as Executor;
        const bResult = await exec.call('mod.b', {}, context);
        return { fromB: bResult['received'] };
      },
      moduleId: 'mod.a',
      inputSchema: PermissiveInput,
      outputSchema: PermissiveOutput,
      description: 'Module A sets shared data',
    }));

    const executor = new Executor({ registry });
    const ctx = Context.create(createIdentity('user1'));

    const result = await executor.call('mod.a', {}, ctx);
    expect(result['fromB']).toBe('hello_from_a');
  });

  // ---------------------------------------------------------------------------
  // Test 6: error in nested call propagates correctly
  // ---------------------------------------------------------------------------

  it('error in nested call propagates correctly', async () => {
    const registry = new Registry();

    registry.register('mod.b', new FunctionModule({
      execute: () => {
        throw new Error('boom from B');
      },
      moduleId: 'mod.b',
      inputSchema: PermissiveInput,
      outputSchema: PermissiveOutput,
      description: 'Module B throws',
    }));

    registry.register('mod.a', new FunctionModule({
      execute: async (_inputs: Record<string, unknown>, context: Context) => {
        const exec = context.executor as Executor;
        return await exec.call('mod.b', {}, context);
      },
      moduleId: 'mod.a',
      inputSchema: PermissiveInput,
      outputSchema: PermissiveOutput,
      description: 'Module A calls B',
    }));

    const mw = new RecordingMiddleware();
    const executor = new Executor({ registry, middlewares: [mw] });

    await expect(executor.call('mod.a', {})).rejects.toThrow('boom from B');

    // onError should fire for mod.a since middleware was active during its execution
    // mod.b's error propagates up through mod.a
    expect(mw.errorCalls).toContain('mod.a');
  });

  // ---------------------------------------------------------------------------
  // Test 7: schema validation in full pipeline
  // ---------------------------------------------------------------------------

  it('schema validation in full pipeline', async () => {
    const registry = new Registry();
    registry.register('strict.mod', new FunctionModule({
      execute: (inputs) => ({ echo: inputs['name'] }),
      moduleId: 'strict.mod',
      inputSchema: Type.Object({
        name: Type.String(),
        age: Type.Number(),
      }),
      outputSchema: Type.Object({ echo: Type.String() }),
      description: 'Strict schema module',
    }));

    const acl = new ACL([
      { callers: ['*'], targets: ['*'], effect: 'allow', description: 'Allow all' },
    ], 'deny');

    const executor = new Executor({
      registry,
      acl,
      approvalHandler: new AutoApproveHandler(),
    });

    // Missing required field 'age', wrong type for 'name'
    await expect(
      executor.call('strict.mod', { name: 123 }),
    ).rejects.toThrow(SchemaValidationError);
  });

  // ---------------------------------------------------------------------------
  // Test 8: safe hot-reload lifecycle
  // ---------------------------------------------------------------------------

  it('safe hot-reload lifecycle', async () => {
    const registry = new Registry();

    let resolveExecution: (() => void) | null = null;
    const executionStarted = new Promise<void>((resolve) => {
      registry.register('mod.slow', new FunctionModule({
        execute: async () => {
          resolve(); // Signal that execution started
          // Wait until test allows completion
          await new Promise<void>((r) => { resolveExecution = r; });
          return { status: 'completed' };
        },
        moduleId: 'mod.slow',
        inputSchema: PermissiveInput,
        outputSchema: PermissiveOutput,
        description: 'Slow module',
      }));
    });

    const executor = new Executor({ registry });

    // Acquire the module to simulate in-flight execution
    registry.acquire('mod.slow');

    // Start a call in the background
    const callPromise = executor.call('mod.slow', {});

    // Wait for execution to actually start
    await executionStarted;

    // Begin safe unregister while call is in-flight
    const unregisterPromise = registry.safeUnregister('mod.slow', 5000);

    // Module should be draining now
    expect(registry.isDraining('mod.slow')).toBe(true);

    // Release the acquired reference (simulating in-flight completion)
    registry.release('mod.slow');

    // Let the execution complete
    resolveExecution!();

    // Wait for both the call and unregister to complete
    const result = await callPromise;
    expect(result['status']).toBe('completed');

    const cleanDrain = await unregisterPromise;
    expect(cleanDrain).toBe(true);

    // Module should be unregistered now
    expect(registry.has('mod.slow')).toBe(false);
  });
});
