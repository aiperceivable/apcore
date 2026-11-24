"""Spec-traced contract tests for the cancellation feature.

Generated from: apcore/docs/features/cancellation.md
Feature spec declares 2 '## Contract:' blocks:
  - CancelToken.cancel
  - CancelToken.raise_if_cancelled

Each test below carries a verbatim clause id of the form
'cancellation.<method>.<kind>.<detail>' so cross-language diffs line up.

`CancelToken.raise_if_cancelled` was added as the canonical spec-named method,
identical in behavior to `check()` — which remains supported and is not
deprecated (`src/apcore/cancel.py`). The clauses below now exercise
`raise_if_cancelled` directly instead of skipping on the naming gap.
"""

from __future__ import annotations

import asyncio

import pytest

from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.errors import ModuleError


# ---------------------------------------------------------------------------
# Contract: CancelToken.cancel
# ---------------------------------------------------------------------------


async def test_cancellation_cancel_property_thread_safe() -> None:
    """cancellation.cancel.property.thread_safe

    Launch >=8 concurrent cancel() calls on distinct tokens via
    asyncio.gather; assert no exception is raised and every token ends in a
    consistent cancelled state.
    """
    tokens = [CancelToken() for _ in range(16)]

    async def do_cancel(tok: CancelToken) -> None:
        # Yield control so calls genuinely interleave on the event loop.
        await asyncio.sleep(0)
        tok.cancel()

    await asyncio.gather(*(do_cancel(t) for t in tokens))

    # Final state must be consistent: all tokens cancelled, none raised.
    assert all(t.is_cancelled is True for t in tokens)


async def test_cancellation_cancel_property_thread_safe_shared_token() -> None:
    """cancellation.cancel.property.thread_safe

    Concurrent cancel() of a single shared token from many tasks must
    converge to exactly one consistent cancelled state with no exception.
    """
    shared = CancelToken()

    async def do_cancel() -> None:
        await asyncio.sleep(0)
        shared.cancel()

    await asyncio.gather(*(do_cancel() for _ in range(16)))

    assert shared.is_cancelled is True


async def test_cancellation_cancel_property_idempotent() -> None:
    """cancellation.cancel.property.idempotent

    Call cancel() twice with identical (no) inputs; assert identical
    observable outcome and state (is_cancelled stays True, no raise).
    """
    token = CancelToken()

    token.cancel()
    first_state = token.is_cancelled
    token.cancel()  # Second call must be a safe no-op.
    second_state = token.is_cancelled

    assert first_state is True
    assert second_state is True
    assert first_state == second_state
    # check() must behave identically after the repeated cancel.
    with pytest.raises(ExecutionCancelledError):
        token.check()


# ---------------------------------------------------------------------------
# Contract: CancelToken.raise_if_cancelled
# ---------------------------------------------------------------------------


def test_cancellation_raise_if_cancelled_error_execution_cancelled() -> None:
    """cancellation.raise_if_cancelled.error.EXECUTION_CANCELLED"""
    token = CancelToken()
    token.cancel()
    with pytest.raises(ExecutionCancelledError) as exc_info:
        token.raise_if_cancelled()

    err = exc_info.value
    assert isinstance(err, ModuleError)
    assert err.code == "EXECUTION_CANCELLED"


async def test_cancellation_raise_if_cancelled_property_thread_safe() -> None:
    """cancellation.raise_if_cancelled.property.thread_safe

    Mirrors test_cancellation_cancel_property_thread_safe_shared_token above:
    concurrent raise_if_cancelled() reads of a token being cancelled elsewhere
    must not raise a spurious exception and must converge to a consistent
    cancelled state.
    """
    shared = CancelToken()

    async def do_check() -> None:
        await asyncio.sleep(0)
        # Not cancelled yet on most interleavings; a raise here would still be
        # correct, so only ExecutionCancelledError is tolerated.
        try:
            shared.raise_if_cancelled()
        except ExecutionCancelledError:
            pass

    async def do_cancel() -> None:
        await asyncio.sleep(0)
        shared.cancel()

    await asyncio.gather(do_cancel(), *(do_check() for _ in range(16)))
    assert shared.is_cancelled is True


def test_cancellation_raise_if_cancelled_property_pure() -> None:
    """cancellation.raise_if_cancelled.property.pure

    raise_if_cancelled() only reads is_cancelled and never mutates it — calling
    it repeatedly (raising each time once cancelled) must not change the
    token's observable state.
    """
    token = CancelToken()
    token.cancel()

    for _ in range(3):
        with pytest.raises(ExecutionCancelledError):
            token.raise_if_cancelled()
        assert token.is_cancelled is True


# ---------------------------------------------------------------------------
# Sanity guard: raise_if_cancelled() and check() must agree on every call —
# neither is a separate flag, and the addition of one name must not create a
# second source of truth for cancellation state.
# ---------------------------------------------------------------------------


def test_cancellation_execution_cancelled_error_code_matches_spec() -> None:
    """cancellation.raise_if_cancelled.error.EXECUTION_CANCELLED (error-type guard)

    The contract requires ExecutionCancelledError(code=EXECUTION_CANCELLED).
    Verify the error TYPE and CODE field match exactly via both check() and
    raise_if_cancelled(), confirming the two names share one implementation.
    """
    token = CancelToken()
    token.cancel()
    with pytest.raises(ExecutionCancelledError) as exc_info:
        token.check()

    err = exc_info.value
    assert isinstance(err, ModuleError)
    assert err.code == "EXECUTION_CANCELLED"

    with pytest.raises(ExecutionCancelledError) as exc_info2:
        token.raise_if_cancelled()

    err2 = exc_info2.value
    assert isinstance(err2, ModuleError)
    assert err2.code == "EXECUTION_CANCELLED"
