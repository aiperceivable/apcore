"""Tests for guard_call_chain (Algorithm A20)."""

from __future__ import annotations

import pytest

from apcore.errors import (
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
)
from apcore.utils.call_chain import guard_call_chain


class TestGuardCallChain:
    def test_valid_chain_passes(self) -> None:
        guard_call_chain("c", ["a", "b", "c"])  # Should not raise

    def test_depth_exceeded(self) -> None:
        # All unique IDs to avoid circular/frequency triggers
        chain = [f"mod.{i}" for i in range(10)]
        with pytest.raises(CallDepthExceededError) as exc_info:
            guard_call_chain("mod.9", chain, max_call_depth=5)
        assert exc_info.value.details["depth"] == 10

    def test_circular_detection(self) -> None:
        # A → B → A is a strict cycle of length >= 2
        chain = ["a", "b", "a"]
        with pytest.raises(CircularCallError):
            guard_call_chain("a", chain)

    def test_no_false_circular_for_same_start_end(self) -> None:
        # "a" appears at start but cycle requires length >= 2
        chain = ["a", "a"]
        # This is a direct self-call — prior_chain = ["a"], module_id = "a"
        # last_idx = 0, subsequence = [] (length 0) → NOT circular
        guard_call_chain("a", chain, max_module_repeat=10)  # Should not raise CircularCallError

    def test_frequency_exceeded(self) -> None:
        # a → a (self-calls, no cycle since subsequence length is 0 each time)
        chain = ["a", "a", "a"]
        with pytest.raises(CallFrequencyExceededError) as exc_info:
            guard_call_chain("a", chain, max_module_repeat=2)
        assert exc_info.value.details["count"] == 3

    def test_frequency_within_limit(self) -> None:
        chain = ["a", "a"]
        guard_call_chain("a", chain, max_module_repeat=3)  # count=2, limit=3

    def test_empty_chain(self) -> None:
        guard_call_chain("a", [])  # Should not raise

    def test_single_element(self) -> None:
        guard_call_chain("a", ["a"])  # Should not raise

    def test_defaults_match_spec(self) -> None:
        """Default limits: max_call_depth=32, max_module_repeat=3."""
        # 32 elements should be fine
        chain = [f"mod.{i}" for i in range(32)]
        guard_call_chain("mod.31", chain)  # Should not raise

        # 33 elements should fail
        chain = [f"mod.{i}" for i in range(33)]
        with pytest.raises(CallDepthExceededError):
            guard_call_chain("mod.32", chain)
