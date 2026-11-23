"""Protocol §§2.2.1, 2.7 bare-name fixtures through the root public API."""

from __future__ import annotations

from typing import Any

import pytest

from apcore import CanonicalNameResult, canonicalize_name

from .canonical_fixtures import load_fixture

_CASES = load_fixture("canonicalize_name.json")["test_cases"]


@pytest.mark.parametrize("case", _CASES, ids=[case["id"] for case in _CASES])
def test_canonicalize_name_fixture(case: dict[str, Any]) -> None:
    """Every string case has a complete structured result, never an exception."""
    assert set(case["input"]) == {"name"}
    assert set(case["expected"]) == {"original_name", "canonical_name", "error"}
    result = canonicalize_name(case["input"]["name"])
    assert isinstance(result, CanonicalNameResult)
    assert result.original_name == case["expected"]["original_name"]
    assert result.canonical_name == case["expected"]["canonical_name"]
    assert (result.error.value if result.error is not None else None) == case["expected"]["error"]
    assert (result.error is None) is (result.canonical_name is not None)
