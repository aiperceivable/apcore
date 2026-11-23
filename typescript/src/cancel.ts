/**
 * Cancellation support for apcore module execution.
 *
 * Spec D-18 (apcore v0.22.0): `cancel()` MUST be a real interrupt — not
 * merely a cooperative flag. In TypeScript this is realised by backing
 * `CancelToken` with an `AbortController`. The `signal` is exposed so it
 * can be composed (`AbortSignal.any([...])`) with timeout signals inside
 * `BuiltinExecute`, and so user modules performing standard Web-API I/O
 * (`fetch`, `setTimeout`, Web Streams) participate in real abort. The
 * cooperative `check()` and `isCancelled` accessors remain for modules
 * whose pause points are not Web APIs.
 */

import { drainExecutions, linkExecutionParent } from './execution-lifetime.js';
import { ModuleError } from './errors.js';

export class ExecutionCancelledError extends ModuleError {
  // Retryable with a fresh CancelToken (protocol-spec §8.6, D-135): the
  // cancelled token is spent, not the operation.
  static override readonly DEFAULT_RETRYABLE: boolean | null = true;

  constructor(message: string = "Execution was cancelled") {
    super("EXECUTION_CANCELLED", message);
    this.name = "ExecutionCancelledError";
  }
}

export class CancelToken {
  /**
   * Authoritative cancellation state, mirroring apcore-python's `_cancelled`
   * and apcore-rust's `cancelled` atomic. The `AbortSignal` is a derived
   * *channel*, not the state: a signal can never return to the un-aborted
   * state, so using it as the state made `reset()` unimplementable without
   * swapping the controller — which is exactly the defect fixed below.
   */
  private _cancelled: boolean = false;

  /**
   * ONE controller for the token's lifetime. It is never replaced, so a
   * consumer that captured `signal` keeps observing this token for as long
   * as it holds the token — the property Python and Rust get for free by
   * resetting their flag in place.
   */
  private readonly _controller: AbortController = new AbortController();
  private readonly _signal: AbortSignal;
  private _disposeParentLink: (() => void) | null = null;

  /** Derive a cancellation scope, including runtimes without AbortSignal.any. */
  constructor(private readonly _parent: CancelToken | null = null) {
    linkExecutionParent(this, _parent);
    if (_parent !== null && typeof AbortSignal.any === 'function') {
      this._signal = AbortSignal.any([this._controller.signal, _parent.signal]);
    } else {
      this._signal = this._controller.signal;
      if (_parent !== null) {
        const onAbort = () => { this._controller.abort(); this.dispose(); };
        this._disposeParentLink = () => _parent.signal.removeEventListener('abort', onAbort);
        if (_parent.signal.aborted) onAbort();
        else _parent.signal.addEventListener('abort', onAbort, { once: true });
      }
    }
  }

  /** @internal Release a completed execution scope's fallback parent listener. */
  dispose(): void {
    this._disposeParentLink?.();
    this._disposeParentLink = null;
  }

  /** One-shot guard for the reset-after-cancel notice below. */
  private _resetAfterCancelWarned: boolean = false;

  /**
   * The underlying `AbortSignal`. Modules using Web APIs (`fetch`,
   * `setTimeout` via `AbortSignal.timeout`, Web Streams) should attach
   * this signal to participate in real abort (D-18).
   *
   * Stable for the life of the token: the same `AbortSignal` instance is
   * returned before and after {@link reset}.
   */
  get signal(): AbortSignal {
    return this._signal;
  }

  get isCancelled(): boolean {
    return this._cancelled || (this._parent?.isCancelled ?? false);
  }

  cancel(): void {
    this._cancelled = true;
    if (!this._controller.signal.aborted) {
      this._controller.abort();
    }
    this.dispose();
  }

  /**
   * Wait for owned module bodies and their asynchronous finalizers to settle.
   * Cancellation remains prompt; lifecycle owners call drain after stopping new
   * work. Do not await this from inside an execution owned by the same token.
   * A body that ignores cancellation can keep drain pending indefinitely.
   */
  async drain(): Promise<void> {
    await drainExecutions(this);
  }

  check(): void {
    if (this.isCancelled) {
      throw new ExecutionCancelledError();
    }
  }

  /**
   * Spec `docs/features/cancellation.md` "Contract: CancelToken.raise_if_cancelled"
   * names this method (idiomatic TS casing: `raiseIfCancelled`). Identical
   * behavior to {@link check} — this is the canonical spec name, `check()` is
   * kept as-is (not deprecated) since it is already used internally
   * (`src/builtin-steps.ts`) and may be used by external callers.
   */
  raiseIfCancelled(): void {
    this.check();
  }

  /**
   * Reset the token for reuse, clearing the cancellation flag in place —
   * the same semantics as apcore-python `CancelToken.reset` and apcore-rust
   * `CancelToken::reset`.
   *
   * The controller is deliberately NOT replaced. Installing a fresh one
   * detached every consumer holding the pre-reset `signal`: a module that
   * had composed `ctx.cancelToken.signal` into an in-flight `fetch` saw
   * `aborted === false` forever after, so a `reset()` followed by a
   * `cancel()` let that I/O run to completion while `isCancelled` reported
   * true. `signal` is the D-18 real-abort channel that
   * `apcore/docs/features/async-tasks.md` makes normative for TypeScript
   * specifically, so silently detaching it was a real-abort failure, not a
   * cosmetic difference — and one invisible to cooperative checkers.
   *
   * The cost of keeping one controller is the half of peer parity that the
   * Web platform does not offer: **an `AbortSignal` cannot be un-aborted**.
   * After a cancel, the signal channel stays aborted for good. Cooperative
   * state ({@link isCancelled}, {@link check}, {@link raiseIfCancelled})
   * resets exactly as the peers do; signal-based consumers must construct a
   * new `CancelToken` rather than reuse a cancelled one, and are told so
   * once per token. Failing closed this way can only cancel work early —
   * the previous behaviour failed open and could not cancel it at all.
   */
  reset(): void {
    this._cancelled = false;
    if (this._controller.signal.aborted && !this._resetAfterCancelWarned) {
      this._resetAfterCancelWarned = true;
      console.warn(
        '[apcore:cancel] CancelToken.reset() cleared the cancellation flag, but ' +
        'its AbortSignal stays aborted — an AbortSignal cannot be un-aborted. ' +
        'Cooperative checks (isCancelled / check() / raiseIfCancelled()) observe ' +
        'the reset; anything composing `signal` does not, and will abort ' +
        'immediately. Construct a new CancelToken instead of reusing a cancelled ' +
        'one for signal-based work.',
      );
    }
  }
}
