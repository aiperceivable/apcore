"""Conformance tests for JSON Schema keyword parity (PROTOCOL_SPEC §4.15).

Drives the canonical ``apcore/conformance/fixtures/schema_keyword_parity.json``
(resolved by ``conformance.canonical_fixtures``).

Unlike ``test_schema_hardening.py`` — which feeds raw JSON Schema to the
``jsonschema`` library via ``validate_schema_dict()`` — this file drives the
code path a real module invocation goes through: ``SchemaLoader.generate_model()``
converts the JSON Schema to a Pydantic model, and ``model_validate(strict=True)``
performs the check (see ``BuiltinInputValidation`` in
``src/apcore/builtin_steps.py``). ``strict=True`` is part of that path, not an
option: the module-invocation boundary performs no type coercion
(TYPE_MAPPING §17.3), so a driver that omitted it would accept inputs a real
module call rejects.

That distinction is the entire point of the fixture. The fixture's
``driver_contract`` records why: while conformance only exercised the raw
validator, a whole class of converter defects — dropped ``type``-array members,
discarded combinator siblings, vanished array constraints — stayed invisible
across several releases. Every case here must run through the conversion.
"""

from __future__ import annotations

import json
from typing import Any

import pydantic
import pytest

from apcore.config import Config
from apcore.schema.loader import SchemaLoader
from conformance.canonical_fixtures import load_fixture

# Read the canonical fixture in the spec repo, not a vendored snapshot, so a
# spec-side edit reaches this driver on the next run — the same contract
# apcore-typescript and apcore-rust already honour.
_load = load_fixture


class TestKeywordParityFixture:
    """schema_keyword_parity.json — converter + validator, not the raw validator."""

    _fixture = _load("schema_keyword_parity.json")

    @pytest.mark.parametrize(
        "tc",
        _fixture["test_cases"],
        ids=lambda tc: tc["id"],
    )
    def test_generated_model_validates(self, tc: dict[str, Any]) -> None:
        schema: dict[str, Any] = tc["schema"]
        data = tc["input"]
        expected: bool = tc["expected_valid"]

        loader = SchemaLoader(Config({}))
        model = loader.generate_model(schema, f"KeywordParity_{tc['id']}")

        error: pydantic.ValidationError | None = None
        try:
            model.model_validate(data, strict=True)
            got = True
        except pydantic.ValidationError as exc:
            got = False
            error = exc

        assert got is expected, (
            f"[{tc['id']}] validity mismatch: got valid={got}, expected valid={expected}.\n"
            f"  description: {tc.get('description', '(none)')}\n"
            f"  schema: {json.dumps(schema, sort_keys=True)}\n"
            f"  input: {json.dumps(data, sort_keys=True)}\n"
            f"  errors: {error.errors() if error is not None else '(none — validation passed)'}"
        )

    def test_fixture_case_ids_are_unique(self) -> None:
        ids = [tc["id"] for tc in self._fixture["test_cases"]]
        assert len(ids) == len(
            set(ids)
        ), f"duplicate case ids in fixture: {sorted({i for i in ids if ids.count(i) > 1})}"
