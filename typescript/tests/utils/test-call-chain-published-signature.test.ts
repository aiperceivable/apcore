/**
 * D-83 (spec v1.49.0) — the published `guardCallChain` signature is normative.
 *
 * The decision was written about apcore-rust, whose shipped guard took a
 * `Context<T>` and a depth: the spec's own Rust tab published code that did not
 * compile, and a host whose services type was anything else could not call the
 * guard at all, so nested calls ran with no depth, cycle or frequency
 * enforcement. `DEFAULT_MAX_CALL_DEPTH` was a public constant the guard never
 * consulted.
 *
 * apcore-typescript is one of the two SDKs that already published the
 * chain-taking form, which is why the decision names the spec and reachability
 * as its authority rather than any implementation — and why this SDK had no
 * test for it.
 *
 * Two things are pinned, because they are the two the decision is about:
 *
 *   - REACHABILITY. Everything here imports from the PACKAGE ROOT and from the
 *     BROWSER entry, the two doors a host actually comes through. Importing
 *     `../../src/utils/call-chain.js` would pass while both entries omitted the
 *     symbol, which is the shape D-83 forbids.
 *   - The published PARAMETERS are consulted: the chain, the depth limit and
 *     the repeat limit each change the outcome on their own, and the documented
 *     defaults are the enforced ones.
 */

import { describe, it, expect } from 'vitest';
import {
  guardCallChain,
  DEFAULT_MAX_CALL_DEPTH,
  DEFAULT_MAX_MODULE_REPEAT,
} from '../../src/index.js';
import * as browserEntry from '../../src/browser/index.js';
import {
  CallDepthExceededError,
  CallFrequencyExceededError,
  CircularCallError,
} from '../../src/errors.js';

describe('D-83: the published guardCallChain signature', () => {
  it('is reachable from the package root and the browser entry', () => {
    expect(typeof guardCallChain).toBe('function');
    expect(DEFAULT_MAX_CALL_DEPTH).toBe(32);
    expect(typeof DEFAULT_MAX_MODULE_REPEAT).toBe('number');

    // The guard is pure logic with no Node dependency, so a browser host must
    // reach it too. This is the same import-graph question the D-127 fix hit
    // from the other side.
    expect(typeof browserEntry.guardCallChain).toBe('function');
    expect(browserEntry.DEFAULT_MAX_CALL_DEPTH).toBe(DEFAULT_MAX_CALL_DEPTH);
  });

  it('consults the callChain argument: two chains, one moduleId, two outcomes', () => {
    expect(() =>
      guardCallChain('c', ['a', 'b', 'c'], DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT),
    ).not.toThrow();

    expect(() =>
      guardCallChain('c', ['c', 'b', 'c'], DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT),
    ).toThrow(CircularCallError);
  });

  it('enforces the documented default when the limits are omitted', () => {
    // The limit a host READS must be the limit the guard APPLIES.
    // apcore-rust's DEFAULT_MAX_CALL_DEPTH was a public constant nothing
    // consulted — a declared surface reaching no mechanism.
    //
    // A20 rejects on `chain.length > maxCallDepth`, so the limit itself is
    // allowed and the first rejected length is limit + 1. Both sides are
    // asserted: a guard that was off by one, or that ignored the constant,
    // fails one of them.
    const atLimit = Array.from({ length: DEFAULT_MAX_CALL_DEPTH }, (_, i) => `mod.${i}`);
    expect(() => guardCallChain('mod.last', atLimit)).not.toThrow();

    const overLimit = Array.from({ length: DEFAULT_MAX_CALL_DEPTH + 1 }, (_, i) => `mod.${i}`);
    expect(() => guardCallChain('mod.last', overLimit)).toThrow(CallDepthExceededError);
  });

  it('the depth and repeat limits are separate parameters', () => {
    // Without this, a guard that ignored maxModuleRepeat entirely — or
    // conflated it with the depth — passes every test above.
    //
    // A20 checks depth, then circularity, then frequency, so the repeated
    // module must sit at the END of the prior chain or the circular check
    // fires first and the frequency limit is never reached.
    const repeating = ['a', 'a', 'a'];
    expect(() => guardCallChain('a', repeating, 32, 3)).not.toThrow();
    expect(() => guardCallChain('a', repeating, 32, 2)).toThrow(CallFrequencyExceededError);

    const deep = Array.from({ length: 10 }, (_, i) => `mod.${i}`);
    expect(() => guardCallChain('mod.next', deep, 32, 3)).not.toThrow();
    expect(() => guardCallChain('mod.next', deep, 5, 3)).toThrow(CallDepthExceededError);
  });
});
