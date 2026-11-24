/**
 * Middleware that records ModuleError details into ErrorHistory.
 */

import { ModuleError } from '../errors.js';
import type { Context } from '../context.js';
import { Middleware } from './base.js';
import type { ErrorHistory } from '../observability/error-history.js';

/**
 * Records ModuleError instances into ErrorHistory on every onError() call.
 * Generic exceptions are ignored. This middleware never recovers from errors.
 */
export class ErrorHistoryMiddleware extends Middleware {
  private readonly _errorHistory: ErrorHistory;

  constructor(errorHistory: ErrorHistory) {
    super();
    this._errorHistory = errorHistory;
  }

  override onError(
    moduleId: string,
    _inputs: Record<string, unknown>,
    error: Error,
    _context: Context,
  ): Record<string, unknown> | null {
    if (error instanceof ModuleError) {
      this._errorHistory.record(moduleId, error);
    }
    return null;
  }
}
