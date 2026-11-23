/**
 * Error propagation (Algorithm A11).
 */

import type { Context } from '../context.js';
import { ModuleError, ModuleExecuteError } from '../errors.js';

/**
 * Wrap a raw error into a standardized ModuleError (Algorithm A11).
 *
 * If the error is already a ModuleError, enriches it with trace context.
 * Otherwise wraps it as a ModuleExecuteError.
 *
 * @param error - The raw error.
 * @param moduleId - Module ID where the error occurred.
 * @param context - Current execution context.
 * @returns A ModuleError with traceId, moduleId, and callChain attached.
 */
export function propagateError(error: Error, moduleId: string, context: Context): ModuleError {
  if (error instanceof ModuleError) {
    // Already a ModuleError -- enrich with context if missing
    if (error.traceId === undefined) {
      (error as { traceId: string }).traceId = context.traceId;
    }
    if (!('module_id' in error.details)) {
      error.details['module_id'] = moduleId;
    }
    if (!('call_chain' in error.details)) {
      error.details['call_chain'] = [...context.callChain];
    }
    return error;
  }

  // Wrap raw error as ModuleExecuteError
  const wrapped = new ModuleExecuteError(
    moduleId,
    `Module '${moduleId}' raised ${error.constructor.name}: ${error.message}`,
    { cause: error },
  );
  wrapped.details['call_chain'] = [...context.callChain];
  return wrapped;
}
