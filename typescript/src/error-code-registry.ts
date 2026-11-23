/**
 * ErrorCodeRegistry for custom module error codes with collision detection (Algorithm A17).
 */

import { ErrorCodes, ModuleError } from './errors.js';
import type { ErrorOptions } from './errors.js';

/** Reserved framework error code prefixes. */
export const FRAMEWORK_ERROR_CODE_PREFIXES: ReadonlySet<string> = new Set([
  'MODULE_',
  'SCHEMA_',
  'ACL_',
  'GENERAL_',
  'CONFIG_',
  'CIRCULAR_',
  'DEPENDENCY_',
  'CALL_',
  'FUNC_',
  'BINDING_',
  'MIDDLEWARE_',
  'APPROVAL_',
  'VERSION_',
  'ERROR_CODE_',
]);

function collectFrameworkCodes(): ReadonlySet<string> {
  return new Set(Object.values(ErrorCodes));
}

const FRAMEWORK_CODES = collectFrameworkCodes();

export class ErrorCodeCollisionError extends ModuleError {
  static override readonly DEFAULT_RETRYABLE: boolean | null = false;

  constructor(code: string, moduleId: string, conflictSource: string, options?: ErrorOptions) {
    super(
      'ERROR_CODE_COLLISION',
      `Error code '${code}' from module '${moduleId}' collides with ${conflictSource}`,
      { errorCode: code, moduleId, conflictSource },
      options?.cause,
      options?.traceId,
      options?.retryable,
      options?.aiGuidance,
      options?.userFixable,
      options?.suggestion,
    );
    this.name = 'ErrorCodeCollisionError';
  }
}

/**
 * Registry for custom module error codes with collision detection.
 *
 * Detects conflicts between module custom error codes and framework reserved
 * codes, as well as between modules.
 */
export class ErrorCodeRegistry {
  private _moduleCodes: Map<string, ReadonlySet<string>> = new Map();
  private _allCodes: Set<string> = new Set(FRAMEWORK_CODES);

  get allCodes(): ReadonlySet<string> {
    return new Set(this._allCodes);
  }

  register(moduleId: string, codes: Set<string>): void {
    if (codes.size === 0) return;

    for (const code of codes) {
      // Check collision with framework reserved codes
      if (FRAMEWORK_CODES.has(code)) {
        throw new ErrorCodeCollisionError(code, moduleId, 'framework');
      }
      // Check collision with other modules
      if (this._allCodes.has(code)) {
        const owner = this._findOwner(code);
        if (owner !== moduleId) {
          throw new ErrorCodeCollisionError(code, moduleId, owner ?? 'unknown');
        }
      }
    }

    // Check prefix reservation
    for (const code of codes) {
      for (const prefix of FRAMEWORK_ERROR_CODE_PREFIXES) {
        if (code.startsWith(prefix)) {
          throw new ErrorCodeCollisionError(code, moduleId, `reserved prefix '${prefix}'`);
        }
      }
    }

    this._moduleCodes.set(moduleId, new Set(codes));
    for (const code of codes) {
      this._allCodes.add(code);
    }
  }

  unregister(moduleId: string): void {
    const codes = this._moduleCodes.get(moduleId);
    if (codes) {
      for (const code of codes) {
        this._allCodes.delete(code);
      }
      this._moduleCodes.delete(moduleId);
    }
  }

  private _findOwner(code: string): string | null {
    for (const [mid, codes] of this._moduleCodes) {
      if (codes.has(code)) return mid;
    }
    return null;
  }
}
