import { ACL, type AccessDecision } from './acl.js';
import type { Context, Identity } from './context.js';
import { ACLDeniedError } from './errors.js';
import { hasScopeCeiling } from './identity.js';
import type { PipelineContext } from './pipeline.js';

type ScopedBinding = {
  readonly identity: Identity;
  readonly moduleId: string;
  readonly callerId: string | null;
  readonly callChain: readonly string[];
  resolvedModule?: unknown;
  allowed: boolean;
};

const bindings = new WeakMap<PipelineContext, ScopedBinding>();

/** @internal Require the actual stock ACL semantics for a scoped call. */
export function isStockScopeAcl(acl: ACL | null): acl is ACL {
  return acl !== null && Object.getPrototypeOf(acl) === ACL.prototype &&
    acl.asyncCheckAccess === ACL.prototype.asyncCheckAccess &&
    acl.evaluateVisibility === ACL.prototype.evaluateVisibility;
}

/** @internal Bind the authority before any arbitrary middleware hook runs. */
export function beginScopedInvocation(ctx: PipelineContext): void {
  const identity = ctx.context.identity;
  if (identity !== null && hasScopeCeiling(identity)) {
    const callChain = ctx.context.callChain;
    bindings.set(ctx, {
      identity, moduleId: ctx.moduleId, callerId: callChain.at(-1) ?? null,
      callChain: [...callChain, ctx.moduleId], allowed: false,
    });
  }
}

/** @internal Record the registered module selected by the stock lookup step. */
export function recordScopedLookup(ctx: PipelineContext): void {
  const binding = bindings.get(ctx);
  if (binding) binding.resolvedModule = ctx.module;
}

function remainsBound(ctx: PipelineContext, binding: ScopedBinding): boolean {
  return ctx.context.identity === binding.identity && ctx.moduleId === binding.moduleId &&
    binding.resolvedModule !== undefined && binding.resolvedModule === ctx.module &&
    ctx.context.callerId === binding.callerId && ctx.context.callChain.length === binding.callChain.length &&
    ctx.context.callChain.every((id, index) => id === binding.callChain[index]);
}

/** @internal Check the bound authority at the ACL gate and before dispatch. */
export function assertScopedInvocation(ctx: PipelineContext, requireGrant: boolean): void {
  const binding = bindings.get(ctx);
  if (binding && (!remainsBound(ctx, binding) || (requireGrant && !binding.allowed))) {
    throw new ACLDeniedError(binding.callerId, binding.moduleId);
  }
}

/** @internal Call only from the stock gate after consulting the stock ACL. */
export function recordScopedGrant(ctx: PipelineContext, decision: AccessDecision): void {
  assertScopedInvocation(ctx, false);
  const binding = bindings.get(ctx);
  if (binding) binding.allowed = decision.access === 'allow';
}

/** @internal A denial on a scoped invocation cannot become recovery output. */
export function isScopedAdmissionError(ctx: PipelineContext, error: unknown): boolean {
  return bindings.has(ctx) && error instanceof ACLDeniedError;
}

/** @internal Fail closed before traversing a strategy without a scope gate. */
export function assertScopeGate(ctx: Context, moduleId: string, hasGate: boolean): void {
  if (hasScopeCeiling(ctx.identity) && !hasGate) throw new ACLDeniedError(ctx.callerId, moduleId);
}
