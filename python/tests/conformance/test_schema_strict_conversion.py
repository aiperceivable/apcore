"""Conformance tests for Algorithm A23 ``to_strict_schema()``.

Drives the canonical ``apcore/conformance/fixtures/schema_strict_conversion.json``
(resolved by ``conformance.canonical_fixtures``), so a spec-side edit reaches
this driver on the next run rather than leaving Python on a stale snapshot.

DRIVER CONTRACT: this file MUST drive ``apcore.schema.to_strict_schema()`` —
the A23 entry point (PROTOCOL_SPEC §4.16 / ALGORITHMS A23) — not the exporter
and not the binding wrapper. A23 is the shared deterministic surface; the three
SDKs must emit the same strict schema for the same input.
"""

from __future__ import annotations

import copy
import json
from typing import Any

import pytest
from jsonschema import Draft202012Validator

from apcore.schema import to_strict_schema
from conformance.canonical_fixtures import load_fixture

# Read the canonical fixture in the spec repo, not a vendored snapshot, so a
# spec-side edit reaches this driver on the next run — the same contract
# apcore-typescript and apcore-rust already honour.
_load = load_fixture


class TestStrictConversionFixture:
    """schema_strict_conversion.json — A23 output parity across SDKs."""

    _fixture = _load("schema_strict_conversion.json")

    @pytest.mark.parametrize("tc", _fixture["test_cases"], ids=lambda tc: tc["id"])
    def test_strict_output_matches(self, tc: dict[str, Any]) -> None:
        schema = tc["schema"]
        before = copy.deepcopy(schema)

        got = to_strict_schema(schema)

        assert got == tc["expected"], (
            f"[{tc['id']}] strict-schema mismatch.\n"
            f"  description: {tc.get('description', '(none)')}\n"
            f"  input:    {json.dumps(schema, sort_keys=True)}\n"
            f"  expected: {json.dumps(tc['expected'], sort_keys=True)}\n"
            f"  got:      {json.dumps(got, sort_keys=True)}"
        )
        if "validation_cases" in tc:
            Draft202012Validator.check_schema(got)
            validator = Draft202012Validator(got)
            for example in tc["validation_cases"]:
                assert (
                    validator.is_valid(example["data"]) == example["valid"]
                ), f"[{tc['id']}] unexpected validation result for {example['data']!r}"
        # A23 MUST deep-copy — the caller's schema is never mutated.
        assert schema == before, f"[{tc['id']}] to_strict_schema mutated its input"

    def test_fixture_case_ids_are_unique(self) -> None:
        ids = [tc["id"] for tc in self._fixture["test_cases"]]
        assert len(ids) == len(
            set(ids)
        ), f"duplicate case ids in fixture: {sorted({i for i in ids if ids.count(i) > 1})}"
