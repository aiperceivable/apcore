/**
 * Context namespace utilities for apcore middleware hardening (Issue #42).
 *
 * Namespace rules:
 *   - _apcore.* — reserved for framework use only
 *   - ext.*     — reserved for user extensions
 *   - All other keys are allowed (legacy compatibility)
 */

export type ContextKeyWriter = 'framework' | 'user';

export interface ContextKeyValidation {
  readonly valid: boolean;
  readonly warning: boolean;
}

const APCORE_NS_PREFIX = '_apcore.';
const EXT_NS_PREFIX = 'ext.';

/**
 * Validate whether a context key write is allowed for the given writer role.
 *
 * Rules (docs/features/middleware-system.md "Context Namespacing"):
 *   - User middleware MUST NOT write `_apcore.*` keys.
 *   - Framework MUST NOT write `ext.*` keys.
 */
export function validateContextKey(
  writer: ContextKeyWriter,
  key: string,
): ContextKeyValidation {
  if (writer === 'user' && key.startsWith(APCORE_NS_PREFIX)) {
    return { valid: false, warning: true };
  }
  if (writer === 'framework' && key.startsWith(EXT_NS_PREFIX)) {
    return { valid: false, warning: true };
  }
  return { valid: true, warning: false };
}

/**
 * Detect whether a function is an async function at call time using
 * `handler.constructor.name === 'AsyncFunction'`.
 *
 * This is the TypeScript-safe equivalent of Python's `inspect.iscoroutinefunction`.
 * Do NOT use `handler() instanceof Promise` — that calls the function and is too late.
 */
export function isAsyncHandler(handler: (...args: never[]) => unknown): boolean {
  return handler.constructor.name === 'AsyncFunction';
}
