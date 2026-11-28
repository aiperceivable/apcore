import { awaitExecution, CancelToken, ExecutionCancelledError } from './cancel.js';
import { Context, type Identity } from './context.js';
import { PolicyEvaluationError } from './errors.js';
import { authorityKeyOf, detachIdentity } from './identity.js';
import type { ApprovalHandler } from './approval.js';
import type { ModuleAnnotations } from './module.js';
import type { PipelineContext } from './pipeline.js';
import { sha256Hex } from './utils/sha256.js';

export interface PolicyEvaluationRequest {
  readonly moduleId: string;
  readonly moduleVersion: string | null;
  readonly callerId: string | null;
  readonly identity: Identity | null;
  readonly arguments: Readonly<Record<string, unknown>>;
  readonly sensitiveFields: readonly string[];
  readonly signal: AbortSignal;
}

export interface PolicyVerdict {
  readonly decision: 'allow' | 'deny' | 'require_approval';
  readonly ruleId?: string;
}

/** A host-supplied policy on the validated inputs of an invocation. */
export interface PolicyEvaluator {
  evaluate(request: PolicyEvaluationRequest): Promise<PolicyVerdict>;
}

type ValidationSnapshot = {
  readonly arguments: Record<string, unknown>;
  readonly canonical: readonly string[];
  sensitiveFields: readonly string[];
};

interface PolicyInvocation {
  readonly id: string;
  readonly preflight: boolean;
  readonly stream: boolean;
  readonly parent: Context;
  context?: Context;
  callerId?: string | null;
  identity?: Identity | null;
  principalId?: string;
  principalType?: string;
  authorityKey?: string | null;
  selected?: { module: unknown; version: string | null; governance: ModuleAnnotations | null };
  aclApprovalRequired?: boolean;
  earlyApproval?: { needed: boolean; annotations: ModuleAnnotations; handler: ApprovalHandler | null };
  validated?: ValidationSnapshot;
  evaluating?: boolean;
  approvedForDispatch?: boolean;
  requiresApproval?: boolean;
  dispatched?: boolean;
  producer?: AsyncGenerator<Record<string, unknown>>;
}

// Policy state is private to this traversal. Step hooks can inspect the
// public PipelineContext but cannot supply their own authorization proof.
const invocations = new WeakMap<PipelineContext, PolicyInvocation>();

/** @internal */
export function beginPolicyInvocation(ctx: PipelineContext): void {
  invocations.set(ctx, {
    id: ctx.moduleId, preflight: ctx.dryRun === true, stream: ctx.stream === true, parent: ctx.context,
  });
}

/** @internal */
export function policyInvocationOf(ctx: PipelineContext): PolicyInvocation | undefined {
  return invocations.get(ctx);
}

/** @internal Release the validated snapshot when its traversal ends. */
export function endPolicyInvocation(ctx: PipelineContext): void {
  invocations.delete(ctx);
}

/** @internal */
export function assertPolicyBinding(ctx: PipelineContext, invocation: PolicyInvocation): void {
  const { context, selected } = invocation;
  if (ctx.moduleId !== invocation.id || ctx.context !== (context ?? invocation.parent) ||
      (context && (context.callerId !== invocation.callerId || context.identity !== invocation.identity ||
        context.identity?.id !== invocation.principalId || context.identity?.type !== invocation.principalType ||
        authorityKeyOf(context.identity) !== invocation.authorityKey)) ||
      (selected && ctx.module !== selected.module)) {
    throw new PolicyEvaluationError(invocation.id);
  }
}

function ownData(descriptors: { [key: string]: PropertyDescriptor }, key: string): unknown {
  const descriptor = Object.hasOwn(descriptors, key) ? descriptors[key] : undefined;
  if (!descriptor?.enumerable || !Object.hasOwn(descriptor, 'value')) {
    throw new Error('Policy inputs must use enumerable data properties.');
  }
  return descriptor.value;
}

function quote(value: string): string {
  // In Unicode mode a surrogate pair is a single scalar, so this matches
  // only surrogate code points that have no matching code unit.
  if (/[\uD800-\uDFFF]/u.test(value)) throw new Error('Policy inputs must contain Unicode scalar values.');
  return JSON.stringify(value);
}

function encodeNumber(value: number): string {
  if (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value))) {
    throw new Error('Policy inputs must contain finite, lossless numbers.');
  }
  if (Number.isInteger(value) && !Object.is(value, -0)) return `i${value};`;
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, value);
  return `f${view.getBigUint64(0).toString(16).padStart(16, '0')};`;
}

function copyArgument(value: unknown, parts: string[], active: Set<object>): unknown {
  if (typeof value === 'string') parts.push(quote(value));
  else if (typeof value === 'number') parts.push(encodeNumber(value));
  else if (value === null || typeof value === 'boolean') parts.push(String(value));
  else if (typeof value === 'object' && !active.has(value)) {
    active.add(value);
    try {
      if (Array.isArray(value) && Object.getPrototypeOf(value) === Array.prototype) {
        return copyArray(value, parts, active);
      }
      if (Object.getPrototypeOf(value) === Object.prototype) return copyObject(value, parts, active);
      throw new Error('Policy inputs must use plain objects and arrays.');
    } finally {
      active.delete(value);
    }
  } else throw new Error('Policy inputs must contain acyclic JSON values.');
  return value;
}

function copyArray(value: unknown[], parts: string[], active: Set<object>): unknown[] {
  const descriptors = Object.getOwnPropertyDescriptors(value);
  if (Reflect.ownKeys(descriptors).length !== value.length + 1) {
    throw new Error('Policy input arrays must be dense and have no additional properties.');
  }
  parts.push('[');
  const copy: unknown[] = [];
  for (let index = 0; index < value.length; index++) {
    if (index > 0) parts.push(',');
    copy.push(copyArgument(ownData(descriptors, String(index)), parts, active));
  }
  parts.push(']');
  return copy;
}

function copyObject(value: object, parts: string[], active: Set<object>): Record<string, unknown> {
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const keys = Object.keys(descriptors);
  if (Reflect.ownKeys(descriptors).length !== keys.length) {
    throw new Error('Policy input objects must have string keys.');
  }
  // Canonicalize ordering by unsigned UTF-16 as required by the wire contract.
  const copies = new Map<string, unknown>();
  parts.push('{');
  for (const key of keys.slice().sort()) {
    if (copies.size > 0) parts.push(',');
    parts.push(quote(key), ':');
    copies.set(key, copyArgument(ownData(descriptors, key), parts, active));
  }
  parts.push('}');
  // Preserve the accepted business object's property order for execution.
  return Object.fromEntries(keys.map((key) => [key, copies.get(key)]));
}

/** @internal Detach a wire object using the invocation's canonical JSON constraints. */
export function snapshotJsonObject(value: unknown): ValidationSnapshot {
  const parts: string[] = [];
  const args = copyArgument(value, parts, new Set());
  if (args === null || typeof args !== 'object' || Array.isArray(args)) {
    throw new Error('Expected a JSON object.');
  }
  return { arguments: args as Record<string, unknown>, canonical: parts, sensitiveFields: [] };
}

/** @internal Detach the inputs and capture their canonical encoding. */
export function copyPolicyInputs(moduleId: string, inputs: Record<string, unknown>): ValidationSnapshot {
  try {
    const snapshot = snapshotJsonObject(inputs);
    if (Object.hasOwn(snapshot.arguments, '_approval_token')) {
      throw new Error('Policy requires an object of business arguments.');
    }
    return snapshot;
  } catch {
    throw new PolicyEvaluationError(moduleId);
  }
}

/** @internal Reject unsafe top-level properties before the existing ACL and token extraction read them. */
export function policyIngressInputs(inputs: Record<string, unknown>, moduleId: string): Record<string, unknown> {
  try {
    if (Object.getPrototypeOf(inputs) !== Object.prototype) throw new Error('Expected a plain input object.');
    const descriptors = Object.getOwnPropertyDescriptors(inputs);
    const entries = Object.keys(descriptors).map((key) => [key, ownData(descriptors, key)]);
    if (Reflect.ownKeys(descriptors).length !== entries.length) throw new Error('Expected string keys.');
    return Object.fromEntries(entries);
  } catch {
    throw new PolicyEvaluationError(moduleId);
  }
}

/** @internal */
export async function policyArgumentsDigest(moduleId: string, snapshot: ValidationSnapshot): Promise<string> {
  try {
    return await sha256Hex(snapshot.canonical.join(''));
  } catch {
    throw new PolicyEvaluationError(moduleId);
  }
}

/** @internal Isolate the scope handed to an asynchronous approval handler. */
export function policyApprovalContext(context: Context): Context {
  const snapshot = new Context(
    context.traceId, context.callerId, [...context.callChain], context.executor,
    detachIdentity(context.identity), context.redactedInputs, context.data, context.cancelToken,
    context.services, context.globalDeadline, context.redactedOutput,
  );
  Object.defineProperties(snapshot, { identity: { writable: false }, callerId: { writable: false } });
  return snapshot;
}

/** @internal Bound evaluator work and sanitize provider errors. */
export async function evaluatePolicy(
  evaluator: PolicyEvaluator,
  ctx: PipelineContext,
  invocation: PolicyInvocation,
  snapshot: ValidationSnapshot,
): Promise<PolicyVerdict> {
  const { context, moduleId } = ctx;
  const parent = context.cancelToken;
  const token = new CancelToken(parent);
  const deadline = context.globalDeadline;
  const timeout = Math.min(1_000, deadline === null ? 1_000 : deadline * 1_000 - Date.now());
  const started = performance.now();
  invocation.evaluating = true;
  try {
    token.check();
    if (timeout <= 0) throw new PolicyEvaluationError(moduleId);
    const request: PolicyEvaluationRequest = Object.freeze({
      moduleId,
      moduleVersion: invocation.selected?.version ?? null,
      callerId: context.callerId,
      identity: detachIdentity(context.identity),
      arguments: structuredClone(snapshot.arguments),
      sensitiveFields: Object.freeze([...snapshot.sensitiveFields]),
      signal: token.signal,
    });
    const evaluation = Promise.resolve().then(() => evaluator.evaluate(request));
    const verdict = await awaitExecution(evaluation, token, moduleId, timeout, true);
    if (performance.now() - started >= timeout) {
      token.cancel();
      throw new PolicyEvaluationError(moduleId);
    }
    const decision = verdict?.decision;
    if (decision !== 'allow' && decision !== 'deny' && decision !== 'require_approval') {
      throw new PolicyEvaluationError(moduleId);
    }
    const ruleId = verdict?.ruleId;
    return {
      decision,
      ...(typeof ruleId === 'string' && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/.test(ruleId) ? { ruleId } : {}),
    };
  } catch {
    if (parent?.isCancelled || parent?.signal.aborted) throw new ExecutionCancelledError();
    throw new PolicyEvaluationError(moduleId);
  } finally {
    invocation.evaluating = false;
    token.dispose();
  }
}
