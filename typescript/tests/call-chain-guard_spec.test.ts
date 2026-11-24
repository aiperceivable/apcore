/**
 * Spec-traced contract tests for the Call Chain Guard feature (TypeScript SDK).
 *
 * Source spec: apcore/docs/features/call-chain-guard.md
 * Contract: guard_call_chain  (TS: guardCallChain)
 *
 * Each test embeds the verbatim clause id from the canonical Python suite
 * (apcore-python/tests/test_call_chain_guard_spec.py) in the form
 * ``call_chain_guard.guard_call_chain.<kind>.<detail>`` so cross-language
 * diffs line up row-for-row. Tests only — production source is never modified.
 *
 * SIGNATURE / BEHAVIOUR NOTES (TS vs Python canonical intent):
 *  - The real TS `guardCallChain` takes the call chain directly
 *    (`moduleId`, `callChain`) with positional (not keyword-only) limit params
 *    `maxCallDepth` / `maxModuleRepeat`, matching the corrected Contract
 *    `### Inputs` (module_id, call_chain) in call-chain-guard.md.
 *  - Input-floor violations throw a plain `Error` (not a typed/ValueError),
 *    with the camelCase param name in the message.
 *  - Error `details` keys are snake_case for cross-language parity with the
 *    spec, Python, and Rust (sync finding A-D-019): `depth`, `max_depth`,
 *    `count`, `max_repeat`, `module_id`, `call_chain`. The public TS getters
 *    (`maxDepth`, `moduleId`, `maxRepeat`) keep their camelCase names.
 */

import { describe, it, expect } from 'vitest';
import {
  guardCallChain,
  DEFAULT_MAX_CALL_DEPTH,
  DEFAULT_MAX_MODULE_REPEAT,
} from '../src/utils/call-chain.js';
import {
  CallDepthExceededError,
  CallFrequencyExceededError,
  CircularCallError,
} from '../src/errors.js';

// ---------------------------------------------------------------------------
// INPUT VALIDATION CLAUSES
// ---------------------------------------------------------------------------

describe('call-chain-guard: input validation', () => {
  it('call_chain_guard.guard_call_chain.input.max_depth.below_one: depth limit below floor rejected', () => {
    let caught: unknown;
    try {
      guardCallChain('a', ['a'], 0);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(Error);
    // TS throws a plain Error referencing the camelCase param name.
    expect((caught as Error).message).toContain('maxCallDepth');
  });

  it('call_chain_guard.guard_call_chain.input.max_repeat.below_one: repeat limit below floor rejected', () => {
    let caught: unknown;
    try {
      guardCallChain('a', ['a'], DEFAULT_MAX_CALL_DEPTH, 0);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(Error);
    expect((caught as Error).message).toContain('maxModuleRepeat');
  });

  it.skip('call_chain_guard.guard_call_chain.input.module_id.required: no runtime arity enforcement (contract gap)', () => {
    // TS has no runtime "missing required positional" error; omitting moduleId
    // is a compile-time type error, not a runtime TypeError. No runtime symbol
    // mirrors Python's binding-level TypeError, so this clause is a gap.
  });
});

// ---------------------------------------------------------------------------
// ERROR CLAUSES
// ---------------------------------------------------------------------------

describe('call-chain-guard: errors', () => {
  it('call_chain_guard.guard_call_chain.error.CALL_DEPTH_EXCEEDED: over-depth chain raises typed error', () => {
    const chain = Array.from({ length: 6 }, (_, i) => `mod.${i}`); // length 6, unique
    let caught: unknown;
    try {
      guardCallChain('mod.5', chain, 5);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(CallDepthExceededError);
    const err = caught as CallDepthExceededError;
    expect(err.code).toBe('CALL_DEPTH_EXCEEDED');
    // Details keys are snake_case for cross-language parity (sync finding
    // A-D-019): depth / max_depth / call_chain, matching Python and Rust.
    expect(err.details['depth']).toBe(6);
    expect(err.details['max_depth']).toBe(5);
    expect(err.details['call_chain']).toEqual(chain);
    // Public getters keep their camelCase names (stable API).
    expect(err.currentDepth).toBe(6);
    expect(err.maxDepth).toBe(5);
  });

  it('call_chain_guard.guard_call_chain.error.CIRCULAR_CALL: A->B->A cycle raises typed error', () => {
    const chain = ['a', 'b', 'a'];
    let caught: unknown;
    try {
      guardCallChain('a', chain);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(CircularCallError);
    const err = caught as CircularCallError;
    expect(err.code).toBe('CIRCULAR_CALL');
    expect(err.moduleId).toBe('a');
    expect(err.details['module_id']).toBe('a');
    expect(err.details['call_chain']).toEqual(['a', 'b', 'a']);
  });

  it('call_chain_guard.guard_call_chain.error.CALL_FREQUENCY_EXCEEDED: over-frequency self-calls raise typed error', () => {
    const chain = ['a', 'a', 'a', 'a'];
    let caught: unknown;
    try {
      guardCallChain('a', chain, DEFAULT_MAX_CALL_DEPTH, 3);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(CallFrequencyExceededError);
    const err = caught as CallFrequencyExceededError;
    expect(err.code).toBe('CALL_FREQUENCY_EXCEEDED');
    expect(err.moduleId).toBe('a');
    expect(err.count).toBe(4);
    expect(err.maxRepeat).toBe(3);
    // snake_case detail keys (sync finding A-D-019).
    expect(err.details['module_id']).toBe('a');
    expect(err.details['count']).toBe(4);
    expect(err.details['max_repeat']).toBe(3);
    expect(err.details['call_chain']).toEqual(chain);
  });
});

// ---------------------------------------------------------------------------
// ORDERING (Side-Effect-like ordered checks: depth -> circular -> frequency)
// ---------------------------------------------------------------------------

describe('call-chain-guard: check ordering', () => {
  it('call_chain_guard.guard_call_chain.side_effect.1.depth_before_circular: depth checked before circular', () => {
    // ["a","b","a"] is circular AND exceeds maxCallDepth=2 (length 3).
    const chain = ['a', 'b', 'a'];
    let caught: unknown;
    try {
      guardCallChain('a', chain, 2);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(CallDepthExceededError);
    expect((caught as CallDepthExceededError).code).toBe('CALL_DEPTH_EXCEEDED');
  });

  it('call_chain_guard.guard_call_chain.side_effect.2.circular_before_frequency: circular checked before frequency', () => {
    // A->B->A->B->A: circular AND "a" repeats 3x.
    const chain = ['a', 'b', 'a', 'b', 'a'];
    let caught: unknown;
    try {
      guardCallChain('a', chain, DEFAULT_MAX_CALL_DEPTH, 2);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(CircularCallError);
    expect((caught as CircularCallError).code).toBe('CIRCULAR_CALL');
  });
});

// ---------------------------------------------------------------------------
// PROPERTY CLAUSES
// ---------------------------------------------------------------------------

describe('call-chain-guard: properties', () => {
  it('call_chain_guard.guard_call_chain.property.async: synchronous, returns undefined on success', () => {
    const result = guardCallChain('c', ['a', 'b', 'c']);
    // Contract: async=false. TS guard returns void (undefined), not a Promise.
    expect(result).toBeUndefined();
    expect(
      result !== null &&
        typeof (result as unknown as { then?: unknown })?.then === 'function',
    ).toBe(false);
  });

  it('call_chain_guard.guard_call_chain.property.thread_safe: >=8 concurrent calls all succeed', async () => {
    const run = async (idx: number): Promise<void> => {
      const chain = [`mod.${idx}.0`, `mod.${idx}.1`, `mod.${idx}.2`];
      return guardCallChain(`mod.${idx}.2`, chain);
    };
    const results = await Promise.all(
      Array.from({ length: 16 }, (_, i) => run(i)),
    );
    expect(results).toHaveLength(16);
    expect(results.every((r) => r === undefined)).toBe(true);
  });

  it('call_chain_guard.guard_call_chain.property.pure: does not mutate input call chain', () => {
    const chain = ['a', 'b', 'c'];
    const snapshot = [...chain];

    guardCallChain('c', chain);
    expect(chain).toEqual(snapshot);

    // Second call on identical state -> identical (no-throw) outcome.
    guardCallChain('c', chain);
    expect(chain).toEqual(snapshot);
  });

  it('call_chain_guard.guard_call_chain.property.idempotent: repeated identical violation yields identical outcome', () => {
    const chain = ['a', 'b', 'a'];
    const snapshot = [...chain];

    const codes: string[] = [];
    for (let i = 0; i < 2; i++) {
      try {
        guardCallChain('a', chain);
      } catch (err) {
        expect(err).toBeInstanceOf(CircularCallError);
        codes.push((err as CircularCallError).code);
      }
    }
    expect(codes).toEqual(['CIRCULAR_CALL', 'CIRCULAR_CALL']);
    expect(chain).toEqual(snapshot);
  });
});

// ---------------------------------------------------------------------------
// DEFAULTS (Configuration clauses)
// ---------------------------------------------------------------------------

describe('call-chain-guard: defaults', () => {
  it('call_chain_guard.guard_call_chain.input.max_depth.default: default depth is 32 (at-limit passes, over fails)', () => {
    expect(DEFAULT_MAX_CALL_DEPTH).toBe(32);

    const okChain = Array.from({ length: 32 }, (_, i) => `mod.${i}`);
    expect(() => guardCallChain('mod.31', okChain)).not.toThrow();

    const overChain = Array.from({ length: 33 }, (_, i) => `mod.${i}`);
    let caught: unknown;
    try {
      guardCallChain('mod.32', overChain);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(CallDepthExceededError);
    expect((caught as CallDepthExceededError).code).toBe('CALL_DEPTH_EXCEEDED');
  });

  it('call_chain_guard.guard_call_chain.input.max_repeat.default: default repeat is 3 (at-limit passes, over fails)', () => {
    expect(DEFAULT_MAX_MODULE_REPEAT).toBe(3);

    // "a" appears 3 times via self-calls (no cycle) -> at limit, no throw.
    expect(() => guardCallChain('a', ['a', 'a', 'a'])).not.toThrow();

    let caught: unknown;
    try {
      guardCallChain('a', ['a', 'a', 'a', 'a']);
    } catch (err) {
      caught = err;
    }
    expect(caught).toBeInstanceOf(CallFrequencyExceededError);
    expect((caught as CallFrequencyExceededError).code).toBe(
      'CALL_FREQUENCY_EXCEEDED',
    );
  });
});
