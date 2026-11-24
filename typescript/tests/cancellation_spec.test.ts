/**
 * Spec-traced contract tests for the cancellation feature.
 *
 * Mirrors the canonical Python suite:
 *   apcore-python/tests/test_cancellation_spec.py
 *
 * Generated from: apcore/docs/features/cancellation.md
 * Feature spec declares 2 '## Contract:' blocks:
 *   - CancelToken.cancel
 *   - CancelToken.raise_if_cancelled
 *
 * Each test carries a verbatim clause id of the form
 * 'cancellation.<method>.<kind>.<detail>' so cross-language diffs line up by
 * exact clause id.
 *
 * NOTE: the contract block names the second method
 * 'CancelToken.raise_if_cancelled'. This SDK ships it as `raiseIfCancelled()`
 * (idiomatic casing), delegating to the pre-existing `check()`. The clauses
 * under that contract used to be skipped as a missing symbol; the symbol
 * landed with spec v1.49.0 - v1.54.0 and the skips did not.
 */

import { describe, it, expect } from 'vitest';
import { CancelToken, ExecutionCancelledError } from '../src/cancel.js';
import { ModuleError } from '../src/errors.js';

// ---------------------------------------------------------------------------
// Contract: CancelToken.cancel
// ---------------------------------------------------------------------------

describe('Contract: CancelToken.cancel', () => {
  it('cancellation.cancel.property.thread_safe: >=8 concurrent cancel() on distinct tokens converge consistently', async () => {
    const tokens = Array.from({ length: 16 }, () => new CancelToken());

    const doCancel = async (tok: CancelToken): Promise<void> => {
      // Yield control so calls genuinely interleave on the event loop.
      await Promise.resolve();
      tok.cancel();
    };

    await Promise.all(tokens.map((t) => doCancel(t)));

    // Final state must be consistent: all tokens cancelled, none raised.
    expect(tokens.every((t) => t.isCancelled === true)).toBe(true);
  });

  it('cancellation.cancel.property.thread_safe: concurrent cancel() of a shared token converges to one cancelled state', async () => {
    const shared = new CancelToken();

    const doCancel = async (): Promise<void> => {
      await Promise.resolve();
      shared.cancel();
    };

    await Promise.all(Array.from({ length: 16 }, () => doCancel()));

    expect(shared.isCancelled).toBe(true);
  });

  it('cancellation.cancel.property.idempotent: calling cancel() twice is a safe no-op with identical state', () => {
    const token = new CancelToken();

    token.cancel();
    const firstState = token.isCancelled;
    token.cancel(); // Second call must be a safe no-op.
    const secondState = token.isCancelled;

    expect(firstState).toBe(true);
    expect(secondState).toBe(true);
    expect(firstState).toBe(secondState);
    // check() must behave identically after the repeated cancel.
    expect(() => token.check()).toThrow(ExecutionCancelledError);
  });
});

// ---------------------------------------------------------------------------
// Contract: CancelToken.raise_if_cancelled
//
// These three clauses were `it.skip`ped with the reason "missing symbol
// CancelToken.raiseIfCancelled (contract gap)". That was true when they were
// written and was made false by the spec v1.49.0 - v1.54.0 implementation,
// which added `raiseIfCancelled()` as the canonical spec-named method
// (src/cancel.ts). Nothing turned red when the gap closed: a disabled test
// explaining why something cannot be tested keeps explaining it after it can,
// and the clause reads as a documented gap while going untested. These mirror
// apcore-python's test_cancellation_spec.py, which was updated at the time.
// ---------------------------------------------------------------------------

describe('Contract: CancelToken.raise_if_cancelled', () => {
  it('cancellation.raise_if_cancelled.error.EXECUTION_CANCELLED', () => {
    const token = new CancelToken();
    token.cancel();

    let caught: unknown;
    try {
      token.raiseIfCancelled();
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ExecutionCancelledError);
    expect(caught).toBeInstanceOf(ModuleError);
    expect((caught as ModuleError).code).toBe('EXECUTION_CANCELLED');
  });

  it('cancellation.raise_if_cancelled.property.thread_safe', async () => {
    // Concurrent reads of a token being cancelled elsewhere must not raise
    // anything other than ExecutionCancelledError, and must converge.
    const shared = new CancelToken();

    const doCheck = async (): Promise<void> => {
      await Promise.resolve();
      try {
        shared.raiseIfCancelled();
      } catch (e) {
        if (!(e instanceof ExecutionCancelledError)) throw e;
      }
    };
    const doCancel = async (): Promise<void> => {
      await Promise.resolve();
      shared.cancel();
    };

    await Promise.all([doCancel(), ...Array.from({ length: 16 }, doCheck)]);
    expect(shared.isCancelled).toBe(true);
  });

  it('cancellation.raise_if_cancelled.property.pure', () => {
    // It only reads the flag: calling it repeatedly must not change the
    // token's observable state.
    const token = new CancelToken();
    token.cancel();

    for (let i = 0; i < 3; i += 1) {
      expect(() => token.raiseIfCancelled()).toThrow(ExecutionCancelledError);
      expect(token.isCancelled).toBe(true);
    }
  });

  it('control: an uncancelled token raises nothing', () => {
    // Without this, "it throws" would also be satisfied by a method that
    // throws unconditionally.
    expect(() => new CancelToken().raiseIfCancelled()).not.toThrow();
  });
});

// ---------------------------------------------------------------------------
// Sanity guard: ensure the declared error type/code referenced by the
// raise_if_cancelled contract actually exists with the spec'd code, so the
// gap above is purely a method-name mismatch (not a missing error type).
// ---------------------------------------------------------------------------

describe('Contract: CancelToken.raise_if_cancelled (error-type guard)', () => {
  it('cancellation.raise_if_cancelled.error.EXECUTION_CANCELLED: ExecutionCancelledError is a ModuleError with code EXECUTION_CANCELLED via the live check() path', () => {
    const token = new CancelToken();
    token.cancel();

    let caught: unknown;
    try {
      token.check();
    } catch (e) {
      caught = e;
    }

    expect(caught).toBeInstanceOf(ExecutionCancelledError);
    expect(caught).toBeInstanceOf(ModuleError);
    expect((caught as ModuleError).code).toBe('EXECUTION_CANCELLED');
  });
});
