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

from apcore import APCore
from apcore.config import Config
from apcore.errors import SchemaValidationError
from apcore.schema.loader import SchemaLoader
from conformance.canonical_fixtures import load_fixture

# Read the canonical fixture in the spec repo, not a vendored snapshot, so a
# spec-side edit reaches this driver on the next run — the same contract
# apcore-typescript and apcore-rust already honour.
_load = load_fixture


class _Echo:
    description = "Return the supplied object"

    def __init__(self, input_schema: dict[str, Any], output_schema: dict[str, Any]) -> None:
        self.input_schema = input_schema
        self.output_schema = output_schema
        self.calls = 0

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        self.calls += 1
        return inputs


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
        assert len(ids) == len(set(ids)), (
            f"duplicate case ids in fixture: {sorted({i for i in ids if ids.count(i) > 1})}"
        )

    @pytest.mark.parametrize(
        "tc",
        [case for case in _fixture["test_cases"] if case.get("exercise_module_boundary")],
        ids=lambda tc: tc["id"],
    )
    def test_raw_json_schema_through_public_module_boundary(self, tc: dict[str, Any]) -> None:
        client = APCore()
        schema = tc["schema"]
        data = tc["input"]
        open_schema = {"type": "object"}
        input_module = _Echo(schema, open_schema)
        output_module = _Echo(open_schema, schema)
        client.register("parity.input", input_module)
        client.register("parity.output", output_module)

        for direction, module, invalid_call_count in (("input", input_module, 0), ("output", output_module, 1)):
            if tc["expected_valid"]:
                assert client.call(f"parity.{direction}", data) == data
            else:
                with pytest.raises(SchemaValidationError) as failure:
                    client.call(f"parity.{direction}", data)
                if expected_keyword := tc.get("expected_keyword"):
                    assert expected_keyword in {entry["keyword"] for entry in failure.value.details["errors"]}
                assert module.calls == invalid_call_count
