import { ModuleNotFoundError } from './errors.js';
import type { PipelineContext } from './pipeline.js';

/** @internal The declaration and registration generation owned by a typed handle. */
export interface BoundDeclaration {
  readonly module: object;
  readonly isCurrent: () => boolean;
}

type BoundInvocation = {
  readonly id: string;
  readonly declaration: BoundDeclaration;
  dispatched: boolean;
};

const invocations = new WeakMap<PipelineContext, BoundInvocation>();

/** @internal Keep the expected registration outside mutable pipeline state. */
export function beginDeclarationInvocation(ctx: PipelineContext, declaration?: BoundDeclaration): void {
  if (declaration) invocations.set(ctx, { id: ctx.moduleId, declaration, dispatched: false });
}

/** @internal Check the registration chosen by lookup or recorded for dispatch. */
export function assertDeclarationInvocation(ctx: PipelineContext, module: unknown = ctx.module): void {
  const binding = invocations.get(ctx);
  if (binding && (ctx.moduleId !== binding.id || module !== binding.declaration.module || !binding.declaration.isCurrent())) {
    throw new ModuleNotFoundError(binding.id);
  }
}

/** @internal Record dispatch after the stock execution lease was acquired. */
export function recordDeclarationDispatch(ctx: PipelineContext): void {
  const binding = invocations.get(ctx);
  if (binding) binding.dispatched = true;
}

/** @internal A typed call cannot return output from a traversal that skipped dispatch. */
export function assertDeclarationDispatch(ctx: PipelineContext): void {
  const binding = invocations.get(ctx);
  if (binding && !binding.dispatched) throw new ModuleNotFoundError(binding.id);
}

/** @internal Reject recovery of unavailable registrations before module execution. */
export function isDeclarationAdmissionError(ctx: PipelineContext, error: unknown): boolean {
  const binding = invocations.get(ctx);
  return binding !== undefined && !binding.dispatched && error instanceof ModuleNotFoundError;
}
