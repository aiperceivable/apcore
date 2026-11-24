/**
 * Spec-traced contract tests for the Approval System (TypeScript mirror).
 *
 * Source spec: apcore/docs/features/approval-system.md
 * Contract: ApprovalHandler.requestApproval
 *
 * Each test carries a verbatim clause id of the form
 * `approval_system.request_approval.<kind>.<detail>` so cross-language diffs
 * line up row by row. These mirror the canonical Python suite
 * (apcore-python/tests/test_approval_system_spec.py). Tests only — production
 * source is never modified here.
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
} from '../src/errors.js';
import type { ModuleAnnotations } from '../src/module.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function _annotations(): ModuleAnnotations {
  return {
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
    paginationStyle: 'cursor',
    extra: {},
  };
}

function _makeRequest(
  moduleId = 'test.mod',
  extraArgs: Record<string, unknown> = {},
): ApprovalRequest {
  return createApprovalRequest({
    moduleId,
    arguments: { ...extraArgs },
    context: Context.create(),
    annotations: _annotations(),
  });
}

function _statusHandler(status: ApprovalResult['status']): CallbackApprovalHandler {
  return new CallbackApprovalHandler(async (_request: ApprovalRequest) =>
    createApprovalResult({
      status,
      approvalId: status === 'pending' ? 'tok-1' : null,
    }),
  );
}

// ---------------------------------------------------------------------------
// input.<param>.<condition>
// ---------------------------------------------------------------------------

describe('TestInputs', () => {
  it('approval_system.request_approval.input.request.module_id_required: request must carry moduleId', () => {
    // The contract requires `request` to carry moduleId. In TS, moduleId is a
    // required property on the createApprovalRequest options object. Omitting it
    // is a compile-time type error; there is no runtime throw on construction
    // (unlike Python's dataclass TypeError). We assert the positive contract:
    // a request built with moduleId actually exposes it.
    const req = _makeRequest('present.id');
    expect(req.moduleId).toBe('present.id');
  });

  it('approval_system.request_approval.input.request.caller_id_action_required: request carries callerId and action', () => {
    // Was skipped as a contract gap: the Inputs clause required `caller_id` and
    // `action` and the interface exposed neither, so caller identity lived on
    // request.context alone. Spec decision D-03 (spec v1.32.0 §7.3.1) closed
    // that gap and this SDK carries both fields — `callerId` is null on a
    // top-level call (never the "@external" sentinel, which is ACL-internal)
    // and `action` always names the module being invoked.
    const topLevel = _makeRequest('test.mod');
    expect(topLevel.callerId).toBeNull();
    expect(topLevel.action).toBe('test.mod');

    const nested = createApprovalRequest({
      moduleId: 'test.mod',
      arguments: {},
      context: Context.create().child('test.caller'),
      annotations: _annotations(),
      callerId: 'test.caller',
      action: 'test.mod',
    });
    expect(nested.callerId).toBe('test.caller');
    expect(nested.action).toBe('test.mod');
  });
});

// ---------------------------------------------------------------------------
// error.<CODE>
// ---------------------------------------------------------------------------

describe('TestErrors', () => {
  it('approval_system.request_approval.error.APPROVAL_DENIED: rejected maps to ApprovalDeniedError', async () => {
    const handler = new AlwaysDenyHandler();
    const result = await handler.requestApproval(_makeRequest());
    expect(result.status).toBe('rejected');

    const err = new ApprovalDeniedError(result, 'test.mod');
    expect(err).toBeInstanceOf(ApprovalDeniedError);
    expect(err.code).toBe('APPROVAL_DENIED');
  });

  it('approval_system.request_approval.error.APPROVAL_TIMEOUT: timeout maps to ApprovalTimeoutError', async () => {
    const handler = _statusHandler('timeout');
    const result = await handler.requestApproval(_makeRequest());
    expect(result.status).toBe('timeout');

    const err = new ApprovalTimeoutError(result, 'test.mod');
    expect(err).toBeInstanceOf(ApprovalTimeoutError);
    expect(err.code).toBe('APPROVAL_TIMEOUT');
  });

  it('approval_system.request_approval.error.APPROVAL_PENDING: pending maps to ApprovalPendingError', async () => {
    const handler = _statusHandler('pending');
    const result = await handler.requestApproval(_makeRequest());
    expect(result.status).toBe('pending');

    const err = new ApprovalPendingError(result, 'test.mod');
    expect(err).toBeInstanceOf(ApprovalPendingError);
    expect(err.code).toBe('APPROVAL_PENDING');
    expect(err.approvalId).toBe('tok-1');
  });
});

// ---------------------------------------------------------------------------
// property.<name>
// ---------------------------------------------------------------------------

describe('TestProperties', () => {
  it('approval_system.request_approval.property.async: returns a Promise resolving to ApprovalResult', async () => {
    const handler = new AutoApproveHandler();
    const promise = handler.requestApproval(_makeRequest());
    expect(promise).toBeInstanceOf(Promise);
    const result = await promise;
    expect(result.status).toBe('approved');
  });

  it('approval_system.request_approval.property.thread_safe: N concurrent calls each return own input', async () => {
    const n = 12;
    const handler = new CallbackApprovalHandler(async (request: ApprovalRequest) => {
      // Yield to interleave concurrent calls.
      await Promise.resolve();
      return createApprovalResult({ status: 'approved', metadata: { mod: request.moduleId } });
    });

    const requests = Array.from({ length: n }, (_, i) => _makeRequest(`mod.${i}`, { idx: i }));
    const results = await Promise.all(requests.map((req) => handler.requestApproval(req)));

    expect(results).toHaveLength(n);
    expect(results.every((r) => r.status === 'approved')).toBe(true);
    const seen = new Set(results.map((r) => (r.metadata as Record<string, unknown>)['mod']));
    expect(seen).toEqual(new Set(Array.from({ length: n }, (_, i) => `mod.${i}`)));
  });

  it('approval_system.request_approval.property.idempotent: false — same input may yield different outcomes', async () => {
    const states: ApprovalResult['status'][] = ['approved', 'rejected'];
    let callIndex = 0;
    const handler = new CallbackApprovalHandler(async (_request: ApprovalRequest) =>
      createApprovalResult({ status: states[callIndex++] }),
    );

    const request = _makeRequest();
    const first = await handler.requestApproval(request);
    const second = await handler.requestApproval(request);
    expect(first.status).toBe('approved');
    expect(second.status).toBe('rejected');
    expect(first.status).not.toBe(second.status);
  });

  it('approval_system.request_approval.property.pure: false — handler may mutate observable state', async () => {
    class RecordingHandler implements ApprovalHandler {
      readonly calls: string[] = [];

      async requestApproval(request: ApprovalRequest): Promise<ApprovalResult> {
        this.calls.push(request.moduleId);
        return createApprovalResult({ status: 'approved' });
      }

      async checkApproval(_approvalId: string): Promise<ApprovalResult> {
        return createApprovalResult({ status: 'rejected' });
      }
    }

    const handler = new RecordingHandler();
    expect(handler.calls).toEqual([]);
    await handler.requestApproval(_makeRequest('audit.me'));
    // Observable state changed: the call was recorded (impure / side effect).
    expect(handler.calls).toEqual(['audit.me']);
  });
});

// ---------------------------------------------------------------------------
// Protocol conformance (requestApproval is part of the ApprovalHandler interface)
// ---------------------------------------------------------------------------

describe('TestProtocolConformance', () => {
  it('approval_system.request_approval.property.protocol_conformance: built-in handlers satisfy ApprovalHandler', () => {
    const handlers: ApprovalHandler[] = [
      new AlwaysDenyHandler(),
      new AutoApproveHandler(),
      new CallbackApprovalHandler(async () => createApprovalResult({ status: 'approved' })),
    ];
    for (const handler of handlers) {
      expect(typeof handler.requestApproval).toBe('function');
      expect(typeof handler.checkApproval).toBe('function');
    }
  });
});

// Keep ApprovalError imported/referenced for the instanceof hierarchy contract.
describe('TestErrorHierarchy', () => {
  it('approval errors extend the ApprovalError base', () => {
    const err = new ApprovalDeniedError(createApprovalResult({ status: 'rejected' }), 'm');
    expect(err).toBeInstanceOf(ApprovalError);
  });
});
