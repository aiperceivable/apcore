"""Tests for calculate_specificity (Algorithm A10)."""

from __future__ import annotations

import pytest

from apcore.utils.pattern import calculate_specificity


class TestCalculateSpecificity:
    """Spec-defined scoring: exact segment → +2, partial wildcard → +1, pure wildcard → 0."""

    @pytest.mark.parametrize(
        ("pattern", "expected"),
        [
            ("*", 0),
            ("api.*", 2),
            ("api.handler.*", 4),
            ("api.handler.task_submit", 6),
        ],
    )
    def test_spec_examples(self, pattern: str, expected: int) -> None:
        assert calculate_specificity(pattern) == expected

    def test_single_exact_segment(self) -> None:
        assert calculate_specificity("api") == 2

    def test_partial_wildcard_segment(self) -> None:
        # "api*" is one segment containing "*"
        assert calculate_specificity("api*") == 1

    def test_mixed_partial_and_exact(self) -> None:
        # "api.handler*.submit" → exact(2) + partial(1) + exact(2) = 5
        assert calculate_specificity("api.handler*.submit") == 5

    def test_all_wildcards(self) -> None:
        # "*.*.*" → 0 + 0 + 0 = 0
        assert calculate_specificity("*.*.*") == 0

    def test_deeply_nested_exact(self) -> None:
        # "a.b.c.d.e" → 5 * 2 = 10
        assert calculate_specificity("a.b.c.d.e") == 10

    def test_empty_string(self) -> None:
        # Edge case: empty string has one segment "" which is exact (no "*")
        assert calculate_specificity("") == 2

    def test_monotonically_increasing_with_exact_segments(self) -> None:
        """More exact segments → higher specificity."""
        s1 = calculate_specificity("api.*")
        s2 = calculate_specificity("api.handler.*")
        s3 = calculate_specificity("api.handler.task")
        assert s1 < s2 < s3
