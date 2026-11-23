"""Spec-traced contract tests for the Call Chain Guard feature.

Source spec: apcore/docs/features/call-chain-guard.md
Contract: guard_call_chain

Each test embeds its verbatim clause id in the form
``call_chain_guard.guard_call_chain.<kind>.<detail>`` so cross-language
diffs line up row-for-row. Generated tests only — production source is
never modified here.

NOTE ON SIGNATURE DIVERGENCE:
The contract ### Inputs block lists a ``context`` (Context) parameter and
names the limit params ``max_depth`` / ``max_repeat``. The actual
``apcore-python`` ``guard_call_chain`` signature takes the call chain
directly (``module_id``, ``call_chain``) and uses keyword-only limit
params ``max_call_depth`` / ``max_module_repeat``. Tests assert against the
REAL signature; the contract-only ``context`` input is recorded as a
documented skip (contract gap).
"""

from __future__ import annotations

import asyncio
import inspect

import pytest

from apcore.errors import (
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
    InvalidInputError,
)
from apcore.utils.call_chain import (
    DEFAULT_MAX_CALL_DEPTH,
    DEFAULT_MAX_MODULE_REPEAT,
    guard_call_chain,
)


# ---------------------------------------------------------------------------
# INPUT VALIDATION CLAUSES
# ---------------------------------------------------------------------------


def test_input_max_depth_below_one_rejected() -> None:
    """call_chain_guard.guard_call_chain.input.max_depth.below_one

    The depth limit must be >= 1. A value below the floor is rejected with
    ``InvalidInputError(code=GENERAL_INVALID_INPUT)`` (D-84; the contract's
    ``max_depth`` input maps to the real keyword-only ``max_call_depth``).
    """
    with pytest.raises(InvalidInputError) as exc_info:
        guard_call_chain("a", ["a"], max_call_depth=0)
    assert "max_call_depth" in str(exc_info.value)
    assert exc_info.value.code == "GENERAL_INVALID_INPUT"


def test_input_max_repeat_below_one_rejected() -> None:
    """call_chain_guard.guard_call_chain.input.max_repeat.below_one

    The repeat limit must be >= 1. A value below the floor is rejected with
    ``InvalidInputError(code=GENERAL_INVALID_INPUT)`` (D-84; the contract's
    ``max_repeat`` input maps to the real keyword-only ``max_module_repeat``).
    """
    with pytest.raises(InvalidInputError) as exc_info:
        guard_call_chain("a", ["a"], max_module_repeat=0)
    assert "max_module_repeat" in str(exc_info.value)
    assert exc_info.value.code == "GENERAL_INVALID_INPUT"


def test_input_module_id_required() -> None:
    """call_chain_guard.guard_call_chain.input.module_id.required

    ``module_id`` is a required positional parameter; omitting it is a
    TypeError raised by the binding itself (no default provided).
    """
    sig = inspect.signature(guard_call_chain)
    module_id_param = sig.parameters["module_id"]
    assert module_id_param.default is inspect.Parameter.empty
    with pytest.raises(TypeError):
        guard_call_chain()  # type: ignore[call-arg]


# ---------------------------------------------------------------------------
# ERROR CLAUSES
# ---------------------------------------------------------------------------


def test_error_call_depth_exceeded() -> None:
    """call_chain_guard.guard_call_chain.error.CALL_DEPTH_EXCEEDED

    Chain longer than max_call_depth raises CallDepthExceededError with the
    exact code CALL_DEPTH_EXCEEDED and the offending depth/limit recorded.
    """
    chain = [f"mod.{i}" for i in range(6)]  # length 6, all unique
    with pytest.raises(CallDepthExceededError) as exc_info:
        guard_call_chain("mod.5", chain, max_call_depth=5)
    err = exc_info.value
    assert err.code == "CALL_DEPTH_EXCEEDED"
    assert err.details["depth"] == 6
    assert err.details["max_depth"] == 5
    assert err.current_depth == 6
    assert err.max_depth == 5


def test_error_circular_call() -> None:
    """call_chain_guard.guard_call_chain.error.CIRCULAR_CALL

    A strict cycle of length >= 2 (A->B->A) raises CircularCallError with the
    exact code CIRCULAR_CALL and the offending module_id.
    """
    chain = ["a", "b", "a"]
    with pytest.raises(CircularCallError) as exc_info:
        guard_call_chain("a", chain)
    err = exc_info.value
    assert err.code == "CIRCULAR_CALL"
    assert err.module_id == "a"
    assert err.details["call_chain"] == ["a", "b", "a"]


def test_error_call_frequency_exceeded() -> None:
    """call_chain_guard.guard_call_chain.error.CALL_FREQUENCY_EXCEEDED

    A module appearing more than max_module_repeat times (self-calls, which
    do not form a cycle) raises CallFrequencyExceededError with the exact
    code CALL_FREQUENCY_EXCEEDED.
    """
    chain = ["a", "a", "a", "a"]
    with pytest.raises(CallFrequencyExceededError) as exc_info:
        guard_call_chain("a", chain, max_module_repeat=3)
    err = exc_info.value
    assert err.code == "CALL_FREQUENCY_EXCEEDED"
    assert err.module_id == "a"
    assert err.count == 4
    assert err.max_repeat == 3


# ---------------------------------------------------------------------------
# ORDERING (Side-Effect-like ordered checks: depth -> circular -> frequency)
# ---------------------------------------------------------------------------


def test_order_depth_before_circular() -> None:
    """call_chain_guard.guard_call_chain.side_effect.1.depth_before_circular

    A chain that violates BOTH depth and circular must surface the depth
    error first (check order: depth -> circular -> frequency).
    """
    # ["a","b","a"] is circular AND exceeds max_call_depth=2 (length 3).
    chain = ["a", "b", "a"]
    with pytest.raises(CallDepthExceededError) as exc_info:
        guard_call_chain("a", chain, max_call_depth=2)
    assert exc_info.value.code == "CALL_DEPTH_EXCEEDED"


def test_order_circular_before_frequency() -> None:
    """call_chain_guard.guard_call_chain.side_effect.2.circular_before_frequency

    A chain that is both circular and over the frequency limit must surface
    the circular error first (circular is checked before frequency).
    """
    # A->B->A->B->A: circular (B between repeats) and "a" repeats 3x.
    chain = ["a", "b", "a", "b", "a"]
    with pytest.raises(CircularCallError) as exc_info:
        guard_call_chain("a", chain, max_module_repeat=2)
    assert exc_info.value.code == "CIRCULAR_CALL"


# ---------------------------------------------------------------------------
# PROPERTY CLAUSES
# ---------------------------------------------------------------------------


def test_property_async_is_false() -> None:
    """call_chain_guard.guard_call_chain.property.async

    Contract declares async: false. The function must be a plain synchronous
    callable (not a coroutine function) and must return None on success.
    """
    assert not inspect.iscoroutinefunction(guard_call_chain)
    result = guard_call_chain("c", ["a", "b", "c"])
    assert result is None
    assert not inspect.isawaitable(result)


async def test_property_thread_safe_concurrent_calls() -> None:
    """call_chain_guard.guard_call_chain.property.thread_safe

    Contract declares thread_safe: true. Launch >=8 concurrent guard calls
    with distinct inputs; none must raise for valid chains and the results
    must be consistent (all None), proving no shared-state corruption.
    """

    async def run(idx: int) -> None:
        # Distinct, non-circular, within-limit chain per task.
        chain = [f"mod.{idx}.{j}" for j in range(3)]
        return await asyncio.to_thread(guard_call_chain, f"mod.{idx}.2", chain)

    results = await asyncio.gather(*(run(i) for i in range(16)))
    assert len(results) == 16
    assert all(r is None for r in results)


def test_property_pure_no_input_mutation() -> None:
    """call_chain_guard.guard_call_chain.property.pure

    Contract declares pure: true (reads, does not mutate). Calling the guard
    must not mutate the caller-supplied call_chain list, and calling twice on
    the same state must yield identical (no-raise) outcomes.
    """
    chain = ["a", "b", "c"]
    snapshot = list(chain)

    guard_call_chain("c", chain)
    assert chain == snapshot, "guard mutated the input call_chain"

    # Second call on identical state -> identical observable outcome.
    guard_call_chain("c", chain)
    assert chain == snapshot


def test_property_idempotent_repeated_violation() -> None:
    """call_chain_guard.guard_call_chain.property.idempotent

    Calling the guard twice with identical inputs yields an identical outcome
    (same error type + code) and leaves the input chain unchanged.
    """
    chain = ["a", "b", "a"]
    snapshot = list(chain)

    codes: list[str] = []
    for _ in range(2):
        with pytest.raises(CircularCallError) as exc_info:
            guard_call_chain("a", chain)
        codes.append(exc_info.value.code)

    assert codes == ["CIRCULAR_CALL", "CIRCULAR_CALL"]
    assert chain == snapshot


# ---------------------------------------------------------------------------
# DEFAULTS (Configuration clause)
# ---------------------------------------------------------------------------


def test_input_max_depth_default_matches_constant() -> None:
    """call_chain_guard.guard_call_chain.input.max_depth.default

    The default depth limit is DEFAULT_MAX_CALL_DEPTH (32): a chain of exactly
    32 passes, 33 fails with CALL_DEPTH_EXCEEDED.
    """
    assert DEFAULT_MAX_CALL_DEPTH == 32
    ok_chain = [f"mod.{i}" for i in range(32)]
    guard_call_chain("mod.31", ok_chain)  # exactly at limit -> no raise

    over_chain = [f"mod.{i}" for i in range(33)]
    with pytest.raises(CallDepthExceededError) as exc_info:
        guard_call_chain("mod.32", over_chain)
    assert exc_info.value.code == "CALL_DEPTH_EXCEEDED"


def test_input_max_repeat_default_matches_constant() -> None:
    """call_chain_guard.guard_call_chain.input.max_repeat.default

    The default repeat limit is DEFAULT_MAX_MODULE_REPEAT (3): a module
    appearing exactly 3 times passes, 4 times fails.
    """
    assert DEFAULT_MAX_MODULE_REPEAT == 3
    # "a" appears 3 times via self-calls (no cycle) -> at limit, no raise.
    guard_call_chain("a", ["a", "a", "a"])

    with pytest.raises(CallFrequencyExceededError) as exc_info:
        guard_call_chain("a", ["a", "a", "a", "a"])
    assert exc_info.value.code == "CALL_FREQUENCY_EXCEEDED"
