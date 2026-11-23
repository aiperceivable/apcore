/**
 * D-90 (spec v1.49.0) — `reset()` must not substitute the cancellation handle.
 *
 * This SDK is the one the decision was written about. `reset()` installed a
 * fresh `AbortController`, so a consumer holding the pre-reset `signal` was
 * permanently detached: a module that had composed `ctx.cancelToken.signal`
 * into an in-flight `fetch` saw `aborted === false` for ever after, and a later
 * `cancel()` could not reach it. `signal` is the D-18 real-abort channel
 * `async-tasks.md` makes normative for TypeScript specifically, so that was a
 * real-abort failure rather than a cosmetic difference — and INVISIBLE to
 * cooperative checkers, because `isCancelled` and `check()` read the current
 * controller and reported exactly what the caller expected.
 *
 * The fix was implemented and pinned by nothing. `_controller` is `readonly`
 * today, so the substitution is a compile error, but these tests assert the
 * OBSERVABLE contract: a `readonly` that someone removes in a refactor is one
 * keystroke, and the failure it re-opens is silent.
 *
 * What makes each test RED: replacing the controller in `reset()`.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { CancelToken } from '../src/cancel.js';

describe('D-90: reset() keeps one handle for the token lifetime', () => {
  afterEach(() => vi.restoreAllMocks());

  it('returns the same AbortSignal before and after reset', () => {
    const token = new CancelToken();
    const before = token.signal;
    token.reset();
    expect(token.signal).toBe(before);
  });

  it('a consumer holding the pre-reset signal still observes a later cancel', () => {
    // The failure the decision is about, in the shape a module actually hits:
    // grab the signal, get reset out from under you, and never hear the cancel.
    const token = new CancelToken();
    const heldByAModule = token.signal;

    let aborted = false;
    heldByAModule.addEventListener('abort', () => {
      aborted = true;
    });

    token.reset();
    token.cancel();

    expect(aborted).toBe(true);
    expect(heldByAModule.aborted).toBe(true);
  });

  it('the cooperative flag resets even though the signal cannot be un-aborted', () => {
    // The half of peer parity the Web platform does not offer. Cooperative
    // state resets exactly as apcore-python and apcore-rust do; the signal
    // channel stays aborted, which fails CLOSED — it can only cancel work
    // early. The previous behaviour failed open and could not cancel at all.
    const token = new CancelToken();
    vi.spyOn(console, 'warn').mockImplementation(() => {});

    token.cancel();
    expect(token.isCancelled).toBe(true);
    expect(token.signal.aborted).toBe(true);

    token.reset();
    expect(token.isCancelled).toBe(false);
    expect(() => token.check()).not.toThrow();
    expect(token.signal.aborted).toBe(true);
  });

  it('says so once per token rather than faking a reset', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const token = new CancelToken();

    token.cancel();
    token.reset();
    token.reset();
    token.reset();

    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0]?.[0]).toMatch(/cannot be un-aborted/);
  });

  it('the notice is per token, not per process', () => {
    // A process-wide one-shot would tell the first caller and leave every
    // later one to discover it in production.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    for (const token of [new CancelToken(), new CancelToken()]) {
      token.cancel();
      token.reset();
    }

    expect(warn).toHaveBeenCalledTimes(2);
  });

  it('control: a reset with no prior cancel says nothing', () => {
    // Without this, "warns once" would also hold for an SDK that warns on
    // every reset, which is noise for the ordinary reuse the method exists for.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const token = new CancelToken();
    token.reset();
    token.reset();

    expect(warn).not.toHaveBeenCalled();
  });
});
