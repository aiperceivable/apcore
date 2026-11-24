"""Conformance tests for Schema System Hardening (Issue #44, PROTOCOL_SPEC §4.15).

Exercises five fixture files:
  schema_hardening_union.json       — anyOf/oneOf/allOf exhaustive evaluation
  schema_hardening_recursive.json   — recursive $ref (#) via TreeNode
  schema_hardening_constraints.json — numeric + string constraint enforcement
  schema_hardening_formats.json     — format-level warnings (SHOULD, not hard error)
  schema_hardening_cache.json       — SHA-256 content-addressable deduplication

DRIVER CONTRACT (union + recursive): those two fixtures MUST be driven through
the code path a module invocation takes — ``SchemaLoader.resolve()``, i.e.
``RefResolver`` followed by ``generate_model()``, then ``SchemaValidator``. They
used to call ``validate_schema_dict()``, which hands the raw JSON Schema straight
to the ``jsonschema`` library. That is not what a module call goes through, and it
is exactly where the recursive-schema defect lived: ``jsonschema`` resolved
``{"$ref": "#"}`` happily while ``RefResolver`` in front of it raised
``SCHEMA_CIRCULAR_REF``, so conformance stayed green on a schema no module could
register. The remaining three fixtures assert properties of
``hardening``/``content_hash`` themselves and keep calling them directly.
"""

from __future__ import annotations

import logging
from typing import Any

import pytest
from pydantic import BaseModel

from apcore.config import Config
from apcore.schema.hardening import content_hash, validate_schema_dict
from apcore.schema.loader import SchemaLoader
from apcore.schema.types import SchemaDefinition, SchemaValidationResult
from apcore.schema.validator import SchemaValidator
from conformance.canonical_fixtures import load_fixture

# Read the canonical fixture in the spec repo, not a vendored snapshot, so a
# spec-side edit reaches this driver on the next run — the same contract
# apcore-typescript and apcore-rust already honour.
_load = load_fixture


def _module_model(schema: dict[str, Any]) -> type[BaseModel]:
    """Compile *schema* exactly as registering a module would.

    ``SchemaLoader.resolve()`` runs ``RefResolver`` and then ``generate_model()``
    — the pair ``BuiltinInputValidation`` ultimately validates against.
    """
    loader = SchemaLoader(Config({}))
    definition = SchemaDefinition(
        module_id="conformance.schema_hardening",
        description="conformance fixture",
        input_schema=schema,
        output_schema={},
    )
    return loader.resolve(definition)[0].model


def _validate_via_module_path(data: Any, schema: dict[str, Any]) -> SchemaValidationResult:
    return SchemaValidator(coerce_types=False).validate(data, _module_model(schema))


# ---------------------------------------------------------------------------
# Fixture 1: Union type evaluation (anyOf / oneOf / allOf)
# ---------------------------------------------------------------------------


class TestUnionFixture:
    """schema_hardening_union.json — PROTOCOL_SPEC §4.15.1."""

    _fixture = _load("schema_hardening_union.json")

    @pytest.mark.parametrize(
        "tc",
        [tc for tc in _fixture["test_cases"] if "schema" in tc],
        ids=lambda tc: tc["id"],
    )
    def test_validate_union(self, tc: dict[str, Any]) -> None:
        schema: dict[str, Any] = tc["schema"]
        data = tc["input"]
        expected = tc["expected"]
        result = _validate_via_module_path(data, schema)
        assert result.valid == expected["valid"], (
            f"[{tc['id']}] valid mismatch: got {result.valid}, expected {expected['valid']}. " f"errors={result.errors}"
        )
        if expected.get("error_code"):
            assert result.error_code == expected["error_code"], (
                f"[{tc['id']}] error_code mismatch: got {result.error_code!r}, " f"expected {expected['error_code']!r}"
            )


# ---------------------------------------------------------------------------
# Fixture 2: Recursive schema (TreeNode with $ref: "#")
# ---------------------------------------------------------------------------


class TestRecursiveFixture:
    """schema_hardening_recursive.json — PROTOCOL_SPEC §4.15.2."""

    _fixture = _load("schema_hardening_recursive.json")

    @pytest.mark.parametrize(
        "tc",
        _fixture["test_cases"],
        ids=lambda tc: tc["id"],
    )
    def test_validate_recursive(self, tc: dict[str, Any]) -> None:
        schema: dict[str, Any] = self._fixture["schema"]
        data = tc["input"]
        expected = tc["expected"]
        result = _validate_via_module_path(data, schema)
        assert result.valid == expected["valid"], (
            f"[{tc['id']}] valid mismatch: got {result.valid}, expected {expected['valid']}. " f"errors={result.errors}"
        )
        if expected.get("error_code"):
            assert result.error_code == expected["error_code"], (
                f"[{tc['id']}] error_code mismatch: got {result.error_code!r}, " f"expected {expected['error_code']!r}"
            )


# ---------------------------------------------------------------------------
# Fixture 3: Numeric and string constraints
# ---------------------------------------------------------------------------


class TestConstraintsFixture:
    """schema_hardening_constraints.json — PROTOCOL_SPEC §4.15.3."""

    _fixture = _load("schema_hardening_constraints.json")

    @pytest.mark.parametrize(
        "tc",
        _fixture["test_cases"],
        ids=lambda tc: tc["id"],
    )
    def test_validate_constraints(self, tc: dict[str, Any]) -> None:
        schema: dict[str, Any] = tc["schema"]
        data = tc["input"]
        expected = tc["expected"]
        result = validate_schema_dict(data, schema)
        assert result.valid == expected["valid"], (
            f"[{tc['id']}] valid mismatch: got {result.valid}, expected {expected['valid']}. " f"errors={result.errors}"
        )
        if expected.get("error_code"):
            assert result.error_code == expected["error_code"], (
                f"[{tc['id']}] error_code mismatch: got {result.error_code!r}, " f"expected {expected['error_code']!r}"
            )


# ---------------------------------------------------------------------------
# Fixture 4: Semantic format mapping (warn-on-invalid, not hard error)
# ---------------------------------------------------------------------------


class TestFormatsFixture:
    """schema_hardening_formats.json — PROTOCOL_SPEC §4.15.4."""

    _fixture = _load("schema_hardening_formats.json")

    @pytest.mark.parametrize(
        "tc",
        _fixture["test_cases"],
        ids=lambda tc: tc["id"],
    )
    def test_validate_formats(self, tc: dict[str, Any], caplog: pytest.LogCaptureFixture) -> None:
        schema: dict[str, Any] = tc["schema"]
        data = tc["input"]
        expected = tc["expected"]

        with caplog.at_level(logging.WARNING, logger="apcore.schema.hardening"):
            result = validate_schema_dict(data, schema)

        assert result.valid == expected["valid"], (
            f"[{tc['id']}] valid mismatch: got {result.valid}, expected {expected['valid']}. " f"errors={result.errors}"
        )

        if expected.get("error_code"):
            assert result.error_code == expected["error_code"], (
                f"[{tc['id']}] error_code mismatch: got {result.error_code!r}, " f"expected {expected['error_code']!r}"
            )

        expected_warn = expected.get("warn_logged", False)
        got_warn = any("Format violation" in r.message for r in caplog.records)
        assert got_warn == expected_warn, (
            f"[{tc['id']}] warn_logged mismatch: got {got_warn}, expected {expected_warn}. "
            f"log records: {[r.message for r in caplog.records]}"
        )

    # Regression: sync finding A-D-032 — format checks must recurse into
    # nested objects and arrays. Previously the Python walker only iterated
    # top-level `properties` while apcore-typescript and apcore-rust both
    # recursed; same input + schema produced different warning sets.
    def test_format_violation_in_nested_object_emits_warning(self, caplog: pytest.LogCaptureFixture) -> None:
        schema = {
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "created_at": {"type": "string", "format": "date-time"},
                    },
                },
            },
        }
        data = {"user": {"created_at": "not-a-real-date-time"}}
        with caplog.at_level(logging.WARNING, logger="apcore.schema.hardening"):
            validate_schema_dict(data, schema)
        messages = [r.message for r in caplog.records]
        assert any(
            "Format violation" in m and "date-time" in m and "not-a-real-date-time" in m for m in messages
        ), f"expected nested format violation warning, got {messages}"

    def test_format_violation_in_array_items_emits_warning(self, caplog: pytest.LogCaptureFixture) -> None:
        schema = {
            "type": "object",
            "properties": {
                "ids": {
                    "type": "array",
                    "items": {"type": "string", "format": "uuid"},
                },
            },
        }
        data = {"ids": ["not-a-uuid", "also-not-a-uuid"]}
        with caplog.at_level(logging.WARNING, logger="apcore.schema.hardening"):
            validate_schema_dict(data, schema)
        warnings = [r.message for r in caplog.records if "Format violation" in r.message]
        # Both array items should emit a warning.
        assert len(warnings) == 2, f"expected 2 array-item format violations (one per non-uuid string), got {warnings}"


# ---------------------------------------------------------------------------
# Fixture 5: Content-addressable cache (SHA-256 deduplication)
# ---------------------------------------------------------------------------


class TestCacheFixture:
    """schema_hardening_cache.json — PROTOCOL_SPEC §4.15.5."""

    _fixture = _load("schema_hardening_cache.json")

    @pytest.mark.parametrize(
        "tc",
        _fixture["test_cases"],
        ids=lambda tc: tc["id"],
    )
    def test_content_hash(self, tc: dict[str, Any]) -> None:
        schemas: list[dict[str, Any]] = tc["schemas"]
        expected = tc["expected"]
        assert len(schemas) == 2, "Each cache test case must have exactly 2 schemas"

        hash_a = content_hash(schemas[0])
        hash_b = content_hash(schemas[1])
        same_hash = hash_a == hash_b

        assert same_hash == expected["same_hash"], (
            f"[{tc['id']}] same_hash mismatch: got {same_hash}, expected {expected['same_hash']}. "
            f"hash_a={hash_a[:12]}…, hash_b={hash_b[:12]}…"
        )
        assert len(hash_a) == 64, "SHA-256 hex digest must be 64 characters"
        assert len(hash_b) == 64, "SHA-256 hex digest must be 64 characters"


# ---------------------------------------------------------------------------
# Fixture 6: Cross-repo content-hash parity for tricky canonicalization cases
# (A-D-037). No `expected` hash is recorded — each SDK computes its own; the
# digests below are reported so cross-repo byte-for-byte agreement can be
# checked manually/by the harness. See schema_content_hash.json.
# ---------------------------------------------------------------------------


# Reference digests computed by this SDK (apcore-python, SHA-256 over the
# canonical JSON form: json.dumps(sort_keys=True, separators=(",", ":"))).
# Recorded for cross-repo parity comparison; update only if canonicalization
# intentionally changes.
PYTHON_CONTENT_HASHES: dict[str, str] = {
    "float_one_point_zero": "3213ddad2217bdd811e24bd32fa5cb7e055d8183468fdccfe38924165742f203",
    "non_ascii_unicode_key_and_value": "6f59ca33f434a234ec46ad8a2bee0e5ae2d4253f375288d8aacfe81464feca28",
    "large_integer": "82ef34e554fd0838c22ca2e871d4eec23623ad58b9db621d510056b32043a82a",
    "nested_unsorted_keys": "6a94e1c77af06f49b5507c607ae73ec8429eac1d070beba0a2ff7f5a0ae8208c",
    "baseline_simple_object": "7d2a526161b8fd53a2661ffad166def49e7167ddbbac3497737c155021dfd09f",
}


class TestContentHashParity:
    """schema_content_hash.json — A-D-037 cross-repo canonicalization parity."""

    _fixture = _load("schema_content_hash.json")

    @pytest.mark.parametrize(
        "tc",
        _fixture["test_cases"],
        ids=lambda tc: tc["id"],
    )
    def test_content_hash_is_stable_and_matches_reference(self, tc: dict[str, Any]) -> None:
        schema: dict[str, Any] = tc["schema"]
        digest = content_hash(schema)

        # 64-char SHA-256 hex and idempotent across two computations.
        assert len(digest) == 64
        assert digest == content_hash(schema), f"[{tc['id']}] content_hash is not idempotent"

        # Pin against the recorded reference so an accidental canonicalization
        # change (e.g. float rendering or key-sort regression) is caught.
        assert digest == PYTHON_CONTENT_HASHES[tc["id"]], (
            f"[{tc['id']}] content_hash drifted: got {digest}, " f"reference {PYTHON_CONTENT_HASHES[tc['id']]}"
        )

    def test_report_all_digests(self) -> None:
        """Print all computed digests for cross-repo parity inspection."""
        lines = [f"{tc['id']:35s} {content_hash(tc['schema'])}" for tc in self._fixture["test_cases"]]
        print("\n[A-D-037] apcore-python content_hash digests:\n" + "\n".join(lines))
        # Every case is covered by the reference table.
        assert {tc["id"] for tc in self._fixture["test_cases"]} == set(PYTHON_CONTENT_HASHES)
