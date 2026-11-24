"""Conformance tests for OpenAI strict-mode compatibility detection.

Drives the canonical ``apcore/conformance/fixtures/openai_strict_compat.json``
(resolved by ``conformance.canonical_fixtures``), so a spec-side edit reaches
this driver on the next run rather than leaving Python on a stale snapshot.

DRIVER CONTRACT: this file MUST drive
``apcore.schema.openai_strict.detect_openai_strict_incompatibilities()`` — the
detector that backs ``BindingStrictSchemaIncompatibleError`` on the
``auto_schema: strict`` binding path (protocol-spec §5.12.5).
The detector is the shared deterministic surface across the three SDKs; the
binding wrapper around it differs per SDK (Rust performs no runtime type
inference). Feature lists are compared for exact equality INCLUDING ORDER:
byte-identical output across SDKs is the point of the fixture.
"""

from __future__ import annotations

import copy
import json
from typing import Any

import pytest

from apcore.schema.openai_strict import detect_openai_strict_incompatibilities
from conformance.canonical_fixtures import load_fixture

# Read the canonical fixture in the spec repo, not a vendored snapshot, so a
# spec-side edit reaches this driver on the next run — the same contract
# apcore-typescript and apcore-rust already honour.
_load = load_fixture


class TestOpenAiStrictCompatFixture:
    """openai_strict_compat.json — the shared strict-compat detector."""

    _fixture = _load("openai_strict_compat.json")

    @pytest.mark.parametrize("tc", _fixture["test_cases"], ids=lambda tc: tc["id"])
    def test_detected_features_match(self, tc: dict[str, Any]) -> None:
        schema = tc["schema"]
        expected: list[str] = tc["expected_features"]
        before = copy.deepcopy(schema)

        got = detect_openai_strict_incompatibilities(schema)

        assert got == expected, (
            f"[{tc['id']}] feature-list mismatch.\n"
            f"  description: {tc.get('description', '(none)')}\n"
            f"  schema: {json.dumps(schema, sort_keys=True)}\n"
            f"  expected: {expected}\n"
            f"  got:      {got}"
        )
        # The detector reports; it must never rewrite (no oneOf -> anyOf downgrade).
        assert schema == before, f"[{tc['id']}] detector mutated the input schema"

    def test_fixture_case_ids_are_unique(self) -> None:
        ids = [tc["id"] for tc in self._fixture["test_cases"]]
        assert len(ids) == len(
            set(ids)
        ), f"duplicate case ids in fixture: {sorted({i for i in ids if ids.count(i) > 1})}"

    def test_fixture_records_keyword_set_provenance(self) -> None:
        # The supported set moves; the fixture must always say what it was pinned to.
        source = self._fixture["keyword_set_source"]
        assert "structured-outputs" in source["url"]
        assert source["verified"]
