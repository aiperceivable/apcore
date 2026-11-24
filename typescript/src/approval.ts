/**
 * Approval system: interfaces, data types, built-in handlers.
 *
 * Provides a pluggable gate at Executor Step 4.5, between ACL enforcement
 * and input validation. When a module declares requiresApproval=true and
 * an ApprovalHandler is configured, the handler is invoked before execution.
 */

import type { Context } from './context.js';
import type { ModuleAnnotations } from './module.js';

/**
 * Carries invocation context to the approval handler.
 *
 * `callerId` and `action` (spec v2026-05 D-03) let a handler — a Slack
 * approver, an audit log — read who is calling and what is being invoked off
 * this one flat record, without traversing `context` or treating `moduleId`
 * as double duty for "the thing being approved". `callerId` is read straight
 * off `Context.callerId` with no substitution: it is `null` for a top-level
 * call, exactly as `Context.callerId` is, never the `"@external"` sentinel
 * ACL evaluation substitutes internally. `action` always equals the invoked
 * module's ID.
 */
export interface ApprovalRequest {
  readonly moduleId: string;
  readonly callerId: string | null;
  readonly action: string;
  readonly arguments: Record<string, unknown>;
  readonly context: Context;
  readonly annotations: ModuleAnnotations;
  readonly description: string | null;
  readonly tags: readonly string[];
}

/**
 * Create a frozen ApprovalRequest.
 *
 * `callerId` defaults to `null` (a top-level call) and `action` to
 * `moduleId` (D-03's populating rule, `action = module_id`) so existing
 * callers that predate both fields keep compiling and get the right value
 * for free. `BuiltinApprovalGate` (`src/builtin-steps.ts`) is the one
 * production call site and passes both explicitly from the call's own
 * `Context` and module ID rather than relying on the default.
 */
export function createApprovalRequest(options: {
  moduleId: string;
  arguments: Record<string, unknown>;
  context: Context;
  annotations: ModuleAnnotations;
  description?: string | null;
  tags?: string[];
  callerId?: string | null;
  action?: string;
}): ApprovalRequest {
  return Object.freeze({
    moduleId: options.moduleId,
    callerId: options.callerId ?? null,
    action: options.action ?? options.moduleId,
    arguments: options.arguments,
    context: options.context,
    annotations: options.annotations,
    description: options.description ?? null,
    tags: Object.freeze([...(options.tags ?? [])]),
  });
}

/**
 * Carries the approval handler's decision.
 */
export interface ApprovalResult {
  readonly status: 'approved' | 'rejected' | 'timeout' | 'pending';
  readonly approvedBy: string | null;
  readonly reason: string | null;
  readonly approvalId: string | null;
  readonly metadata: Record<string, unknown> | null;
}

/**
 * Create a frozen ApprovalResult.
 */
export function createApprovalResult(options: {
  status: 'approved' | 'rejected' | 'timeout' | 'pending';
  approvedBy?: string | null;
  reason?: string | null;
  approvalId?: string | null;
  metadata?: Record<string, unknown> | null;
}): ApprovalResult {
  return Object.freeze({
    status: options.status,
    approvedBy: options.approvedBy ?? null,
    reason: options.reason ?? null,
    approvalId: options.approvalId ?? null,
    metadata: options.metadata ?? null,
  });
}

/**
 * Protocol for pluggable approval handlers.
 *
 * Implementations receive an ApprovalRequest and return an ApprovalResult.
 * Both methods are asynchronous.
 */
export interface ApprovalHandler {
  requestApproval(request: ApprovalRequest): Promise<ApprovalResult>;
  checkApproval(approvalId: string): Promise<ApprovalResult>;
}

/**
 * Built-in handler that always rejects. Safe default for enforcement.
 */
export class AlwaysDenyHandler implements ApprovalHandler {
  async requestApproval(_request: ApprovalRequest): Promise<ApprovalResult> {
    return createApprovalResult({ status: 'rejected', reason: 'Always denied' });
  }

  async checkApproval(_approvalId: string): Promise<ApprovalResult> {
    return createApprovalResult({ status: 'rejected', reason: 'Always denied' });
  }
}

/**
 * Built-in handler that always approves. For testing and development.
 */
export class AutoApproveHandler implements ApprovalHandler {
  async requestApproval(_request: ApprovalRequest): Promise<ApprovalResult> {
    return createApprovalResult({ status: 'approved', approvedBy: 'auto' });
  }

  async checkApproval(_approvalId: string): Promise<ApprovalResult> {
    return createApprovalResult({ status: 'approved', approvedBy: 'auto' });
  }
}

/**
 * Built-in handler that delegates to a user-provided async callback.
 */
export class CallbackApprovalHandler implements ApprovalHandler {
  private _callback: (request: ApprovalRequest) => Promise<ApprovalResult>;

  constructor(callback: (request: ApprovalRequest) => Promise<ApprovalResult>) {
    this._callback = callback;
  }

  async requestApproval(request: ApprovalRequest): Promise<ApprovalResult> {
    return this._callback(request);
  }

  async checkApproval(_approvalId: string): Promise<ApprovalResult> {
    return createApprovalResult({
      status: 'rejected',
      reason: 'Phase B not supported by callback handler',
    });
  }
}
