/**
 * StreamingModule interface and utilities for apcore.
 */

import type { Module } from './module.js';
import type { Context } from './context.js';

export const STREAMING_MARKER = Symbol.for('apcore.streaming');

export interface StreamingModule extends Module {
  readonly [STREAMING_MARKER]: true;
  stream(
    inputs: Record<string, unknown>,
    context: Context,
  ): AsyncGenerator<Record<string, unknown>>;
}

const _warned = new WeakSet<object>();

type MaybeModule = Record<string, unknown> & Record<symbol, unknown>;

/**
 * Returns true when the module implements the StreamingModule interface.
 *
 * Detection order:
 *   1. Module has STREAMING_MARKER and stream() → proper implementation.
 *   2. Module has stream() only → transitional duck-typing fallback; warns once
 *      per instance and will be removed in the next major version.
 *   3. Neither → false.
 */
export function isStreamingModule(m: Module): m is StreamingModule {
  const mm = m as unknown as MaybeModule;
  if (mm[STREAMING_MARKER] === true && typeof mm['stream'] === 'function') {
    return true;
  }
  if (typeof mm['stream'] === 'function') {
    if (!_warned.has(m)) {
      _warned.add(m);
      console.warn(
        `[apcore:streaming] Module ${(m as { constructor?: { name?: string } }).constructor?.name ?? '<anonymous>'} ` +
        `exposes stream() without [STREAMING_MARKER]. ` +
        `This duck-typing fallback is deprecated and will be removed in the next major. ` +
        `Add 'readonly [STREAMING_MARKER]: true' to the class.`,
      );
    }
    return true;
  }
  return false;
}
