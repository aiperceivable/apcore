/**
 * Unit tests for approval data types, error classes, and built-in handlers.
 */

import { describe, expect, it } from 'vitest';
import {
  AlwaysDenyHandler,
  AutoApproveHandler,
  CallbackApprovalHandler,
  createApprovalRequest,
  createApprovalResult,
} from '../src/approval.js';
import type { ApprovalHandler, ApprovalRequest, ApprovalResult } from '../src/approval.js';
import { Context } from '../src/context.js';
import {
  ApprovalDeniedError,
  ApprovalError,
  ApprovalPendingError,
  ApprovalTimeoutError,
  ErrorCodes,
} from '../src/errors.js';
import type { ModuleAnnotations } from '../src/module.js';

// ---------------------------------------------------------------------------
// ApprovalRequest
// ---------------------------------------------------------------------------

describe('ApprovalRequest', () => {
  it('carries all fields', () => {
    const ctx = Context.create();
    const ann: ModuleAnnotations = {
      readonly: false,
      destructive: false,
      idempotent: false,
      requiresApproval: true,
      openWorld: true,
      streaming: false,
      cacheable: false,
      cacheTtl: 0,
      cacheKeyFields: null,
      paginated: false,
      paginationStyle: 'cursor', extra: {},
    };
    const req = createApprovalRequest({
      moduleId: 'test.module',
      callerId: 'test.caller',
      action: 'test.module',
      arguments: { key: 'value' },
      context: ctx,
      annotations: ann,
      description: 'Test module',
      tags: ['admin'],
    });
    expect(req.moduleId).toBe('test.module');
    expect(req.callerId).toBe('test.caller');
    expect(req.action).toBe('test.module');
    expect(req.arguments).toEqual({ key: 'value' });
    expect(req.context).toBe(ctx);
    expect(req.annotations).toBe(ann);
    expect(req.description).toBe('Test module');
    expect(req.tags).toEqual(['admin']);
  });

  it('has correct defaults', () => {
    const ctx = Context.create();
    const ann: ModuleAnnotations = {
      readonly: false,
      destructive: false,
      idempotent: false,
      requiresApproval: true,
      openWorld: true,
      streaming: false,
      cacheable: false,
      cacheTtl: 0,
      cacheKeyFields: null,
      paginated: false,
      paginationStyle: 'cursor', extra: {},
    };
    const req = createApprovalRequest({
      moduleId: 'm',
      arguments: {},
      context: ctx,
      annotations: ann,
    });
    expect(req.description).toBeNull();
    expect(req.tags).toEqual([]);
    // D-03: an omitted callerId defaults to null (a top-level call) and an
    // omitted action defaults to moduleId, the populating rule BuiltinApprovalGate
    // uses (`action = module_id`) — so a caller predating both fields still
    // gets the right value rather than an empty one.
    expect(req.callerId).toBeNull();
    expect(req.action).toBe('m');
  });

  it('is frozen', () => {
    const ctx = Context.create();
    const ann: ModuleAnnotations = {
      readonly: false,
      destructive: false,
      idempotent: false,
      requiresApproval: true,
      openWorld: true,
      streaming: false,
      cacheable: false,
      cacheTtl: 0,
      cacheKeyFields: null,
      paginated: false,
      paginationStyle: 'cursor', extra: {},
    };
    const req = createApprovalRequest({
      moduleId: 'm',
      arguments: {},
      context: ctx,
      annotations: ann,
    });
    expect(() => {
      (req as unknown as Record<string, unknown>)['moduleId'] = 'other';
    }).toThrow();
  });
});

// ---------------------------------------------------------------------------
// ApprovalResult
// ---------------------------------------------------------------------------

describe('ApprovalResult', () => {
  it('creates approved result', () => {
    const result = createApprovalResult({ status: 'approved', approvedBy: 'admin' });
    expect(result.status).toBe('approved');
    expect(result.approvedBy).toBe('admin');
    expect(result.reason).toBeNull();
    expect(result.approvalId).toBeNull();
    expect(result.metadata).toBeNull();
  });

  it('creates rejected result with reason', () => {
    const result = createApprovalResult({ status: 'rejected', reason: 'Not authorized' });
    expect(result.status).toBe('rejected');
    expect(result.reason).toBe('Not authorized');
  });

  it('creates pending result with id', () => {
    const result = createApprovalResult({ status: 'pending', approvalId: 'tok-123' });
    expect(result.status).toBe('pending');
    expect(result.approvalId).toBe('tok-123');
  });

  it('is frozen', () => {
    const result = createApprovalResult({ status: 'approved' });
    expect(() => {
      (result as unknown as Record<string, unknown>)['status'] = 'rejected';
    }).toThrow();
  });
});

// ---------------------------------------------------------------------------
// Error classes
// ---------------------------------------------------------------------------

describe('ApprovalErrors', () => {
  it('creates ApprovalDeniedError with correct code and message', () => {
    const result = createApprovalResult({ status: 'rejected', reason: 'Policy violation' });
    const err = new ApprovalDeniedError(result, 'test.mod');
    expect(err.code).toBe('APPROVAL_DENIED');
    expect(err.message).toContain('test.mod');
    expect(err.message).toContain('Policy violation');
    expect(err.result).toBe(result);
    expect(err.moduleId).toBe('test.mod');
    expect(err).toBeInstanceOf(ApprovalError);
  });

  it('creates ApprovalTimeoutError', () => {
    const result = createApprovalResult({ status: 'timeout' });
    const err = new ApprovalTimeoutError(result, 'test.mod');
    expect(err.code).toBe('APPROVAL_TIMEOUT');
    expect(err.message).toContain('test.mod');
    expect(err.result).toBe(result);
  });

  it('creates ApprovalPendingError with approvalId', () => {
    const result = createApprovalResult({ status: 'pending', approvalId: 'abc-123' });
    const err = new ApprovalPendingError(result, 'test.mod');
    expect(err.code).toBe('APPROVAL_PENDING');
    expect(err.approvalId).toBe('abc-123');
    expect(err.result).toBe(result);
  });

  it('has correct error codes', () => {
    expect(ErrorCodes.APPROVAL_DENIED).toBe('APPROVAL_DENIED');
    expect(ErrorCodes.APPROVAL_TIMEOUT).toBe('APPROVAL_TIMEOUT');
    expect(ErrorCodes.APPROVAL_PENDING).toBe('APPROVAL_PENDING');
  });

  it('inherits from ModuleError via ApprovalError', () => {
    const result = createApprovalResult({ status: 'rejected' });
    const err = new ApprovalDeniedError(result);
    expect(err).toBeInstanceOf(ApprovalError);
    expect(err.timestamp).toBeDefined();
    expect(err.code).toBeDefined();
  });

  it('reason property returns result reason', () => {
    const result = createApprovalResult({ status: 'rejected', reason: 'Policy violation' });
    const err = new ApprovalDeniedError(result, 'test.mod');
    expect(err.reason).toBe('Policy violation');
  });

  it('reason property returns null when no reason', () => {
    const result = createApprovalResult({ status: 'timeout' });
    const err = new ApprovalTimeoutError(result, 'test.mod');
    expect(err.reason).toBeNull();
  });

  it('reason property works on pending error', () => {
    const result = createApprovalResult({ status: 'pending', reason: 'Awaiting manager', approvalId: 'tok-1' });
    const err = new ApprovalPendingError(result, 'test.mod');
    expect(err.reason).toBe('Awaiting manager');
  });
});

// ---------------------------------------------------------------------------
// Built-in handlers
// ---------------------------------------------------------------------------

describe('AlwaysDenyHandler', () => {
  it('rejects on requestApproval', async () => {
    const handler = new AlwaysDenyHandler();
    const ctx = Context.create();
    const request = createApprovalRequest({
      moduleId: 'test.mod',
      arguments: {},
      context: ctx,
      annotations: { readonly: false, destructive: false, idempotent: false, requiresApproval: true, openWorld: true, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const, extra: {} },
    });
    const result = await handler.requestApproval(request);
    expect(result.status).toBe('rejected');
    expect(result.reason).toBe('Always denied');
  });

  it('rejects on checkApproval', async () => {
    const handler = new AlwaysDenyHandler();
    const result = await handler.checkApproval('some-id');
    expect(result.status).toBe('rejected');
  });

  it('satisfies ApprovalHandler interface', () => {
    const handler: ApprovalHandler = new AlwaysDenyHandler();
    expect(typeof handler.requestApproval).toBe('function');
    expect(typeof handler.checkApproval).toBe('function');
  });
});

describe('AutoApproveHandler', () => {
  it('approves on requestApproval', async () => {
    const handler = new AutoApproveHandler();
    const ctx = Context.create();
    const request = createApprovalRequest({
      moduleId: 'test.mod',
      arguments: {},
      context: ctx,
      annotations: { readonly: false, destructive: false, idempotent: false, requiresApproval: true, openWorld: true, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const, extra: {} },
    });
    const result = await handler.requestApproval(request);
    expect(result.status).toBe('approved');
    expect(result.approvedBy).toBe('auto');
  });

  it('approves on checkApproval', async () => {
    const handler = new AutoApproveHandler();
    const result = await handler.checkApproval('some-id');
    expect(result.status).toBe('approved');
  });

  it('satisfies ApprovalHandler interface', () => {
    const handler: ApprovalHandler = new AutoApproveHandler();
    expect(typeof handler.requestApproval).toBe('function');
    expect(typeof handler.checkApproval).toBe('function');
  });
});

describe('CallbackApprovalHandler', () => {
  it('delegates to callback', async () => {
    const handler = new CallbackApprovalHandler(async (request: ApprovalRequest): Promise<ApprovalResult> => {
      return createApprovalResult({
        status: 'approved',
        approvedBy: 'callback',
        metadata: { module: request.moduleId },
      });
    });
    const ctx = Context.create();
    const request = createApprovalRequest({
      moduleId: 'test.mod',
      arguments: { x: 1 },
      context: ctx,
      annotations: { readonly: false, destructive: false, idempotent: false, requiresApproval: true, openWorld: true, streaming: false, cacheable: false, cacheTtl: 0, cacheKeyFields: null, paginated: false, paginationStyle: 'cursor' as const, extra: {} },
    });
    const result = await handler.requestApproval(request);
    expect(result.status).toBe('approved');
    expect(result.approvedBy).toBe('callback');
    expect(result.metadata).toEqual({ module: 'test.mod' });
  });

  it('rejects on checkApproval by default', async () => {
    const handler = new CallbackApprovalHandler(async () => createApprovalResult({ status: 'approved' }));
    const result = await handler.checkApproval('some-id');
    expect(result.status).toBe('rejected');
  });

  it('satisfies ApprovalHandler interface', () => {
    const handler: ApprovalHandler = new CallbackApprovalHandler(async () => createApprovalResult({ status: 'approved' }));
    expect(typeof handler.requestApproval).toBe('function');
    expect(typeof handler.checkApproval).toBe('function');
  });
});
