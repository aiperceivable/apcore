/**
 * Approval system: interfaces, data types, built-in handlers.
 *
 * Provides a pluggable gate at Executor Step 4.5, between ACL enforcement
 * and input validation. When a module declares requiresApproval=true and
 * an ApprovalHandler is configured, the handler is invoked before execution.
 */

import type { Context } from './context.js';
import { InvalidInputError } from './errors.js';
import type { ModuleAnnotations } from './module.js';
import type { PipelineContext } from './pipeline.js';

// A pending credential is internal to one pipeline attempt. Step middleware
// can inspect PipelineContext and application inputs, so neither carries it.
const approvalCredentials = new WeakMap<PipelineContext, string | null>();

/** @internal Return application inputs without exposing the approval credential. */
export function stripApprovalToken(inputs: Record<string, unknown>): Record<string, unknown> {
  if (!('_approval_token' in inputs)) return inputs;
  const { _approval_token: _, ...argumentsOnly } = inputs;
  return argumentsOnly;
}

/** @internal Extract the credential before the pipeline visits any steps. */
export function prepareApprovalInputs(ctx: PipelineContext): void {
  let credential: string | null = null;
  if ('_approval_token' in ctx.inputs) {
    const { _approval_token: token, ...argumentsOnly } = ctx.inputs;
    ctx.inputs = argumentsOnly;
    if (typeof token !== 'string') throw new InvalidInputError('_approval_token must be a string');
    credential = token;
  }
  approvalCredentials.set(ctx, credential);
}

/** @internal Consume the credential at the approval gate. */
export function takeApprovalToken(ctx: PipelineContext): string | null {
  // Low-level callers can invoke the built-in step with a raw context.
  if (!approvalCredentials.has(ctx)) prepareApprovalInputs(ctx);
  const credential = approvalCredentials.get(ctx) ?? null;
  approvalCredentials.set(ctx, null);
  return credential;
}

/** @internal Release an unused credential at the end of an invocation. */
export function discardApprovalToken(ctx: PipelineContext): void {
  approvalCredentials.delete(ctx);
}

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
  /** The resolved registration version of a final-scope request. */
  readonly moduleVersion?: string | null;
  /** Present on approvals bound to the final validated invocation. */
  readonly canonicalArgsDigest?: string;
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
  moduleVersion?: string | null;
  canonicalArgsDigest?: string;
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
    ...(options.moduleVersion !== undefined ? { moduleVersion: options.moduleVersion } : {}),
    ...(options.canonicalArgsDigest !== undefined ? { canonicalArgsDigest: options.canonicalArgsDigest } : {}),
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
  /** An optional, non-secret identifier for approval telemetry. */
  readonly auditId?: string | null;
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
  auditId?: string | null;
  metadata?: Record<string, unknown> | null;
}): ApprovalResult {
  return Object.freeze({
    status: options.status,
    approvedBy: options.approvedBy ?? null,
    reason: options.reason ?? null,
    approvalId: options.approvalId ?? null,
    ...(options.auditId !== undefined ? { auditId: options.auditId } : {}),
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
  /** Opt in to issuing and consuming approvals bound to the final invocation. */
  readonly supportsFinalScope?: boolean;
  requestApproval(request: ApprovalRequest): Promise<ApprovalResult>;
  checkApproval(approvalId: string): Promise<ApprovalResult>;
  /** Validate the current call's identity, target and arguments, then consume the approval. */
  checkApprovalForCall?(approvalId: string, request: ApprovalRequest): Promise<ApprovalResult>;
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
