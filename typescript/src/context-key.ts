/**
 * Typed key for type-safe access to context.data.
 */

import type { Context } from './context.js';

/**
 * Typed accessor for a named slot within `Context.data`.
 *
 * Provides get/set/delete/exists operations with namespace isolation
 * via the `scoped()` factory.
 */
export class ContextKey<T> {
  constructor(readonly name: string) {}

  /** Return the value for this key, or `defaultValue` if absent. */
  get(ctx: Context, defaultValue?: T): T | undefined {
    const val = ctx.data[this.name];
    return val !== undefined ? (val as T) : defaultValue;
  }

  /** Store `value` under this key in context.data. */
  set(ctx: Context, value: T): void {
    ctx.data[this.name] = value;
  }

  /** Remove this key from context.data (no-op if absent). */
  delete(ctx: Context): void {
    delete ctx.data[this.name];
  }

  /** Return true if this key is present in context.data. */
  exists(ctx: Context): boolean {
    return this.name in ctx.data;
  }

  /** Create a sub-key with `{name}.{suffix}`. */
  scoped(suffix: string): ContextKey<T> {
    return new ContextKey(`${this.name}.${suffix}`);
  }
}
