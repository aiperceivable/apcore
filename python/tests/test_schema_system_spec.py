"""Spec-traced contract tests for the apcore Schema System.

Source spec: apcore/docs/features/schema-system.md
Contracts under test (5 ``## Contract:`` blocks):
  - ``Schema.validate``           -> apcore.schema.hardening.validate_schema_dict
  - ``RefResolver -- $ref``       -> apcore.schema.ref_resolver.RefResolver
  - ``Schema.validate_union``     -> validate_schema_dict (anyOf/oneOf path)
  - ``Schema.validate_recursive`` -> validate_schema_dict (recursive $ref path)
  - ``Schema.content_hash``       -> apcore.schema.hardening.content_hash

Each test docstring carries the verbatim clause id formatted
``schema_system.<method>.<kind>.<detail>`` (kind in
input|error|property|side_effect|return) so cross-language diffs line up
row-for-row with the TypeScript and Rust suites. This Python suite is the
CANONICAL clause source.

API mapping note:
  The spec phrases these as ``Schema.validate`` / ``Schema.content_hash`` etc.
  The apcore-python SDK exposes these operations as module-level functions in
  ``apcore.schema.hardening`` (``validate_schema_dict``, ``content_hash``).
  ``validate_schema_dict(data, schema)`` takes a raw JSON Schema dict and
  returns a ``SchemaValidationResult`` (never raises) -- this is the exact
  surface the ``Schema.validate`` contract describes. The union/recursive
  contracts are the same function exercised over union- and recursive-$ref
  schemas; their declared raise-on-failure semantic is delivered as a
  result object carrying an ``error_code`` (see the divergence flags below).

These tests are READ-ONLY contract verification -- they never modify src/.
"""

from __future__ import annotations

import asyncio
import inspect

import pytest

from apcore.errors import (
    SchemaCircularRefError,
    SchemaMaxDepthExceededError,
    SchemaNotFoundError,
)
from apcore.schema.hardening import content_hash, validate_schema_dict
from apcore.schema.ref_resolver import RefResolver
from apcore.schema.types import SchemaValidationResult

# ---------------------------------------------------------------------------
# Shared fixture schemas (canonical conformance shapes from the spec)
# ---------------------------------------------------------------------------

_CONSTRAINT_SCHEMA = {
    "type": "object",
    "properties": {
        "count": {"type": "integer", "minimum": 1, "maximum": 100},
        "label": {"type": "string", "minLength": 1, "maxLength": 50, "pattern": "^[a-z_]+$"},
    },
    "required": ["count", "label"],
}

_ONEOF_SCHEMA = {
    "oneOf": [
        {"type": "object", "properties": {"kind": {"const": "a"}}, "required": ["kind"]},
        {"type": "object", "properties": {"kind": {"const": "b"}}, "required": ["kind"]},
    ]
}

_ANYOF_SCHEMA = {
    "anyOf": [
        {"type": "object", "properties": {"kind": {"const": "a"}}, "required": ["kind"]},
        {"type": "object", "properties": {"kind": {"const": "b"}}, "required": ["kind"]},
    ]
}

# A oneOf schema where a single input matches BOTH branches (ambiguous).
_ONEOF_AMBIGUOUS_SCHEMA = {
    "oneOf": [
        {"type": "object"},
        {"type": "object", "properties": {"kind": {"type": "string"}}},
    ]
}

_TREE_NODE_SCHEMA = {
    "$id": "TreeNode",
    "type": "object",
    "properties": {
        "value": {"type": "string"},
        "children": {"type": "array", "items": {"$ref": "TreeNode"}},
    },
    "required": ["value"],
}


# ===========================================================================
# Contract: Schema.validate  ->  validate_schema_dict(data, schema)
# ===========================================================================


def test_validate_input_accepts_data_and_schema() -> None:
    """schema_system.validate.input.data_and_schema

    Inputs: `data` (required) and `schema` (JSON Schema Draft 2020-12 object,
    required). The function MUST accept both positional inputs.
    """
    sig = inspect.signature(validate_schema_dict)
    params = list(sig.parameters)
    assert params[0] == "data"
    assert params[1] == "schema"
    result = validate_schema_dict({"count": 50, "label": "hello_world"}, _CONSTRAINT_SCHEMA)
    assert isinstance(result, SchemaValidationResult)


def test_validate_error_does_not_raise_on_failure() -> None:
    """schema_system.validate.error.no_raise

    `Schema.validate` does not raise (D10-012). Validation failure is reported
    via the returned result object, NOT via an exception.
    """
    # An input violating the minimum constraint must NOT raise.
    result = validate_schema_dict({"count": 0, "label": "hello"}, _CONSTRAINT_SCHEMA)
    assert result.valid is False
    assert len(result.errors) >= 1


def test_validate_return_success_shape() -> None:
    """schema_system.validate.return.success_shape

    On success: a result object with `valid == True` and an empty `errors`
    array.
    """
    result = validate_schema_dict({"count": 50, "label": "hello_world"}, _CONSTRAINT_SCHEMA)
    assert result.valid is True
    assert result.errors == []


def test_validate_return_failure_shape() -> None:
    """schema_system.validate.return.failure_shape

    On failure: same result-object shape with `valid == False` and a non-empty
    `errors` array carrying per-failure detail objects.
    """
    result = validate_schema_dict({"count": 200, "label": "INVALID LABEL!"}, _CONSTRAINT_SCHEMA)
    assert result.valid is False
    assert isinstance(result.errors, list)
    assert len(result.errors) >= 1
    # Each entry is a structured detail object exposing a path + message.
    detail = result.errors[0]
    assert hasattr(detail, "path")
    assert hasattr(detail, "message")


def test_validate_property_async_false() -> None:
    """schema_system.validate.property.async_false

    Property: async == false. The function MUST be an ordinary (non-coroutine)
    callable.
    """
    assert not inspect.iscoroutinefunction(validate_schema_dict)
    assert not inspect.isasyncgenfunction(validate_schema_dict)


async def test_validate_property_thread_safe() -> None:
    """schema_system.validate.property.thread_safe

    Property: thread_safe == true. Concurrent validation of >= 8 independent
    calls (via asyncio.gather over to_thread) MUST all succeed without
    cross-talk.
    """
    payloads = [({"count": i + 1, "label": "ok"}, True) for i in range(8)] + [
        ({"count": 0, "label": "ok"}, False) for _ in range(4)
    ]

    async def run(data: dict, expected_valid: bool) -> bool:
        result = await asyncio.to_thread(validate_schema_dict, data, _CONSTRAINT_SCHEMA)
        return result.valid == expected_valid

    outcomes = await asyncio.gather(*(run(d, e) for d, e in payloads))
    assert all(outcomes)
    assert len(outcomes) >= 8


def test_validate_property_pure_and_idempotent() -> None:
    """schema_system.validate.property.pure_idempotent

    Property: pure == true and idempotent == true. Same data + same schema MUST
    yield equal results across repeated calls, and the inputs MUST NOT be
    mutated.
    """
    data = {"count": 50, "label": "hello_world"}
    schema_copy = dict(_CONSTRAINT_SCHEMA)
    first = validate_schema_dict(data, _CONSTRAINT_SCHEMA)
    second = validate_schema_dict(data, _CONSTRAINT_SCHEMA)
    assert first.valid == second.valid
    assert [e.path for e in first.errors] == [e.path for e in second.errors]
    # Inputs unchanged (no side effects).
    assert data == {"count": 50, "label": "hello_world"}
    assert _CONSTRAINT_SCHEMA == schema_copy


# ===========================================================================
# Contract: RefResolver -- $ref resolution
#   Python surface: RefResolver(schemas_dir, max_depth=32)
#                   .resolve_ref(ref, current_file, ...)  /  .resolve(schema)
# ===========================================================================


def test_refresolver_input_construction_signature() -> None:
    """schema_system.resolve_ref.input.construction

    Construction: `schemas_dir` (required) and `max_depth` (optional,
    default 32).
    """
    sig = inspect.signature(RefResolver.__init__)
    params = sig.parameters
    assert "schemas_dir" in params
    assert "max_depth" in params
    assert params["max_depth"].default == 32


def test_refresolver_input_resolve_ref_signature(tmp_path) -> None:
    """schema_system.resolve_ref.input.resolve_ref_params

    `resolve_ref(ref_string, current_file, ...)` accepts a single `$ref`
    string and the file it was found in (`None` for inline schemas).
    """
    sig = inspect.signature(RefResolver.resolve_ref)
    params = list(sig.parameters)
    assert "ref_string" in params
    assert "current_file" in params
    resolver = RefResolver(schemas_dir=str(tmp_path))
    # Inline local $ref resolves against the in-memory document supplied via resolve().
    schema = {
        "type": "object",
        "properties": {"addr": {"$ref": "#/definitions/Address"}},
        "definitions": {"Address": {"type": "string"}},
    }
    out = resolver.resolve(schema)
    assert out["properties"]["addr"] == {"type": "string"}


def test_refresolver_return_resolves_ref_inline(tmp_path) -> None:
    """schema_system.resolve.return.inline_resolved

    On success: a schema with the requested `$ref` resolved inline. Local
    (`#/definitions/...`) references MUST be inlined into the parent document.
    """
    resolver = RefResolver(schemas_dir=str(tmp_path))
    schema = {
        "type": "object",
        "properties": {"addr": {"$ref": "#/definitions/Address"}},
        "definitions": {"Address": {"type": "string", "minLength": 2}},
    }
    out = resolver.resolve(schema)
    assert out["properties"]["addr"]["type"] == "string"
    assert out["properties"]["addr"]["minLength"] == 2


def test_refresolver_side_effect_input_not_mutated(tmp_path) -> None:
    """schema_system.resolve.side_effect.input_not_mutated

    The input document is never mutated (a resolved copy is returned).
    """
    resolver = RefResolver(schemas_dir=str(tmp_path))
    schema = {
        "type": "object",
        "properties": {"addr": {"$ref": "#/definitions/Address"}},
        "definitions": {"Address": {"type": "string"}},
    }
    out = resolver.resolve(schema)
    assert schema["properties"]["addr"] == {"$ref": "#/definitions/Address"}
    assert out is not schema


def test_refresolver_error_circular_ref(tmp_path) -> None:
    """schema_system.resolve.error.circular_ref

    `SchemaCircularRefError(code=SCHEMA_CIRCULAR_REF)` MUST be raised when a
    `$ref` cycle is detected.
    """
    resolver = RefResolver(schemas_dir=str(tmp_path))
    schema = {
        "$ref": "#/definitions/A",
        "definitions": {
            "A": {"$ref": "#/definitions/B"},
            "B": {"$ref": "#/definitions/A"},
        },
    }
    with pytest.raises(SchemaCircularRefError) as exc_info:
        resolver.resolve(schema)
    assert exc_info.value.code == "SCHEMA_CIRCULAR_REF"


def test_refresolver_error_ref_not_found_actual_code(tmp_path) -> None:
    """schema_system.resolve.error.ref_not_found

    A referenced schema that cannot be resolved MUST raise
    ``SchemaNotFoundError(code=SCHEMA_NOT_FOUND)`` — the symbol named by the
    schema-system Contract ### Errors block, implemented identically across
    all three SDKs.
    """
    resolver = RefResolver(schemas_dir=str(tmp_path))
    with pytest.raises(SchemaNotFoundError) as exc_info:
        resolver.resolve_ref("#/definitions/DoesNotExist", None)
    assert exc_info.value.code == "SCHEMA_NOT_FOUND"


def test_resolve_ref_property_async_false() -> None:
    """schema_system.resolve_ref.property.async_false

    Property: async == false for both `resolve` and `resolve_ref`.
    """
    assert not inspect.iscoroutinefunction(RefResolver.resolve)
    assert not inspect.iscoroutinefunction(RefResolver.resolve_ref)


async def test_resolve_property_thread_safe(tmp_path) -> None:
    """schema_system.resolve.property.thread_safe

    Property: thread_safe == true. >= 8 concurrent resolutions over independent
    resolver instances MUST each return the correctly inlined document.
    """

    def resolve_one(idx: int) -> dict:
        resolver = RefResolver(schemas_dir=str(tmp_path))
        schema = {
            "type": "object",
            "properties": {"v": {"$ref": "#/definitions/V"}},
            "definitions": {"V": {"type": "integer", "const": idx}},
        }
        return resolver.resolve(schema)

    results = await asyncio.gather(*(asyncio.to_thread(resolve_one, i) for i in range(8)))
    assert len(results) >= 8
    for i, out in enumerate(results):
        assert out["properties"]["v"]["const"] == i


def test_resolve_property_idempotent(tmp_path) -> None:
    """schema_system.resolve.property.idempotent

    Property: idempotent == true. Re-resolving the same input yields an equal
    resolved document.
    """
    resolver = RefResolver(schemas_dir=str(tmp_path))
    schema = {
        "type": "object",
        "properties": {"addr": {"$ref": "#/definitions/Address"}},
        "definitions": {"Address": {"type": "string"}},
    }
    first = resolver.resolve(schema)
    second = resolver.resolve(schema)
    assert first == second


def test_resolve_error_max_depth_exceeded(tmp_path) -> None:
    """schema_system.resolve.error.max_depth_exceeded

    `max_depth` (default 32) caps recursion; exceeding it MUST raise
    `SchemaMaxDepthExceededError(code=SCHEMA_MAX_DEPTH_EXCEEDED)`.
    """
    resolver = RefResolver(schemas_dir=str(tmp_path), max_depth=3)
    schema = {
        "$ref": "#/definitions/A",
        "definitions": {
            "A": {"$ref": "#/definitions/B"},
            "B": {"$ref": "#/definitions/C"},
            "C": {"$ref": "#/definitions/D"},
            "D": {"$ref": "#/definitions/E"},
            "E": {"type": "string"},
        },
    }
    with pytest.raises(SchemaMaxDepthExceededError) as exc_info:
        resolver.resolve(schema)
    assert exc_info.value.code == "SCHEMA_MAX_DEPTH_EXCEEDED"


# ===========================================================================
# Contract: Schema.validate_union  ->  validate_schema_dict over anyOf/oneOf
# ===========================================================================


def test_validate_union_input_keyword_anyof() -> None:
    """schema_system.validate_union.input.anyof

    Inputs: `data`, a union `schema`, and the governing `keyword`. For
    `anyOf`, an input MUST be accepted if it matches at least one branch.
    """
    assert validate_schema_dict({"kind": "a"}, _ANYOF_SCHEMA).valid is True
    assert validate_schema_dict({"kind": "b"}, _ANYOF_SCHEMA).valid is True


def test_validate_union_input_keyword_oneof() -> None:
    """schema_system.validate_union.input.oneof

    For `oneOf`, an input MUST be accepted if it matches exactly one branch.
    """
    assert validate_schema_dict({"kind": "a"}, _ONEOF_SCHEMA).valid is True


def test_validate_union_error_no_match() -> None:
    """schema_system.validate_union.error.no_match

    `SchemaValidationError(code=SCHEMA_UNION_NO_MATCH)` -- no branch matched.

    Cross-language note: the contract Returns says "raises on failure", but
    all three SDKs report failures via the result object (D10-012 amendment of
    the sibling `validate` contract). Here the failure surfaces as a result
    object whose `error_code == SCHEMA_UNION_NO_MATCH`.
    """
    result = validate_schema_dict({"kind": "c"}, _ONEOF_SCHEMA)
    assert result.valid is False
    assert result.error_code == "SCHEMA_UNION_NO_MATCH"

    any_result = validate_schema_dict({"kind": "c"}, _ANYOF_SCHEMA)
    assert any_result.valid is False
    assert any_result.error_code == "SCHEMA_UNION_NO_MATCH"


def test_validate_union_error_ambiguous() -> None:
    """schema_system.validate_union.error.ambiguous

    `SchemaValidationError(code=SCHEMA_UNION_AMBIGUOUS)` -- more than one branch
    matched a `oneOf` schema. (Reported via the result object's `error_code`.)
    """
    result = validate_schema_dict({"kind": "x"}, _ONEOF_AMBIGUOUS_SCHEMA)
    assert result.valid is False
    assert result.error_code == "SCHEMA_UNION_AMBIGUOUS"


def test_validate_union_property_all_branches_evaluated() -> None:
    """schema_system.validate_union.property.all_branches

    Implementations MUST evaluate ALL branches (no short-circuit). A second-
    branch-only match MUST be accepted for both `anyOf` and `oneOf`.
    """
    # Matches only the second branch -> must still validate.
    assert validate_schema_dict({"kind": "b"}, _ONEOF_SCHEMA).valid is True
    assert validate_schema_dict({"kind": "b"}, _ANYOF_SCHEMA).valid is True


async def test_validate_union_property_thread_safe() -> None:
    """schema_system.validate_union.property.thread_safe

    Property: thread_safe == true. >= 8 concurrent union validations MUST each
    return the expected verdict.
    """
    cases = [
        ({"kind": "a"}, True),
        ({"kind": "b"}, True),
        ({"kind": "c"}, False),
        ({"kind": "a"}, True),
        ({"kind": "b"}, True),
        ({"kind": "c"}, False),
        ({"kind": "a"}, True),
        ({"kind": "b"}, True),
    ]

    async def run(data: dict, expected: bool) -> bool:
        result = await asyncio.to_thread(validate_schema_dict, data, _ONEOF_SCHEMA)
        return result.valid == expected

    outcomes = await asyncio.gather(*(run(d, e) for d, e in cases))
    assert len(outcomes) >= 8
    assert all(outcomes)


def test_validate_union_property_pure_idempotent() -> None:
    """schema_system.validate_union.property.pure_idempotent

    Property: pure == true and idempotent == true for union validation.
    """
    first = validate_schema_dict({"kind": "x"}, _ONEOF_AMBIGUOUS_SCHEMA)
    second = validate_schema_dict({"kind": "x"}, _ONEOF_AMBIGUOUS_SCHEMA)
    assert first.valid == second.valid
    assert first.error_code == second.error_code


# ===========================================================================
# Contract: Schema.validate_recursive  ->  validate_schema_dict over $id $ref
# ===========================================================================


def test_validate_recursive_input_nested_data() -> None:
    """schema_system.validate_recursive.input.nested_data

    Inputs: deeply-nested `data` and a self-referencing `schema`. A valid
    nested structure (up to depth 5) MUST validate true.
    """
    assert validate_schema_dict({"value": "root"}, _TREE_NODE_SCHEMA).valid is True
    assert validate_schema_dict({"value": "r", "children": [{"value": "c"}]}, _TREE_NODE_SCHEMA).valid is True
    deep = {
        "value": "a",
        "children": [
            {"value": "b", "children": [{"value": "c", "children": [{"value": "d", "children": [{"value": "e"}]}]}]}
        ],
    }
    assert validate_schema_dict(deep, _TREE_NODE_SCHEMA).valid is True


def test_validate_recursive_error_validation_error() -> None:
    """schema_system.validate_recursive.error.validation_error

    `SchemaValidationError(code=SCHEMA_VALIDATION_ERROR)` -- data does not
    conform at some nesting level. (Reported via the result object.)
    """
    # Missing required `value` at the top level.
    result = validate_schema_dict({"children": []}, _TREE_NODE_SCHEMA)
    assert result.valid is False
    assert result.error_code == "SCHEMA_VALIDATION_ERROR"


def test_validate_recursive_error_nested_validation_error() -> None:
    """schema_system.validate_recursive.error.nested_validation_error

    Validation MUST reach nested levels: a child node missing `value` MUST be
    rejected.
    """
    result = validate_schema_dict({"value": "root", "children": [{"children": []}]}, _TREE_NODE_SCHEMA)
    assert result.valid is False
    assert result.error_code == "SCHEMA_VALIDATION_ERROR"


def test_validate_recursive_property_idempotent() -> None:
    """schema_system.validate_recursive.property.idempotent

    Property: pure == true and idempotent == true for recursive validation.
    """
    data = {"value": "root", "children": [{"value": "c"}]}
    first = validate_schema_dict(data, _TREE_NODE_SCHEMA)
    second = validate_schema_dict(data, _TREE_NODE_SCHEMA)
    assert first.valid == second.valid is True
    assert data == {"value": "root", "children": [{"value": "c"}]}


async def test_validate_recursive_property_thread_safe() -> None:
    """schema_system.validate_recursive.property.thread_safe

    Property: thread_safe == true. >= 8 concurrent recursive validations MUST
    each return the expected verdict.
    """
    valid_payload = {"value": "root", "children": [{"value": "c"}]}
    invalid_payload = {"children": []}
    cases = [(valid_payload, True) if i % 2 == 0 else (invalid_payload, False) for i in range(8)]

    async def run(data: dict, expected: bool) -> bool:
        result = await asyncio.to_thread(validate_schema_dict, data, _TREE_NODE_SCHEMA)
        return result.valid == expected

    outcomes = await asyncio.gather(*(run(d, e) for d, e in cases))
    assert len(outcomes) >= 8
    assert all(outcomes)


# ===========================================================================
# Contract: Schema.content_hash  ->  content_hash(schema)
# ===========================================================================


def test_content_hash_input_schema_dict() -> None:
    """schema_system.content_hash.input.schema_dict

    Inputs: a resolved JSON Schema dict. The function MUST accept a single
    `schema` positional argument.
    """
    sig = inspect.signature(content_hash)
    params = list(sig.parameters)
    assert params[0] == "schema"
    digest = content_hash({"type": "object"})
    assert isinstance(digest, str)


def test_content_hash_return_hex_digest() -> None:
    """schema_system.content_hash.return.hex_digest

    On success: lowercase hexadecimal SHA-256 digest (64 characters).
    """
    digest = content_hash(_CONSTRAINT_SCHEMA)
    assert isinstance(digest, str)
    assert len(digest) == 64
    assert digest == digest.lower()
    assert all(c in "0123456789abcdef" for c in digest)


def test_content_hash_error_does_not_raise() -> None:
    """schema_system.content_hash.error.no_raise

    This operation MUST NOT raise for a serializable schema dict.
    """
    digest = content_hash({"a": 1, "b": [1, 2, {"c": "d"}]})
    assert len(digest) == 64


def test_content_hash_property_canonical_dedup() -> None:
    """schema_system.content_hash.property.canonical_dedup

    Property: pure/deterministic. Two schemas that are byte-for-byte identical
    after canonical (sorted-key) JSON serialization MUST hash identically;
    key ordering MUST NOT affect the digest.
    """
    a = {"b": 1, "a": 2, "z": {"y": 1, "x": 2}}
    b = {"a": 2, "z": {"x": 2, "y": 1}, "b": 1}
    assert content_hash(a) == content_hash(b)
    # Distinct content -> distinct hash.
    assert content_hash({"a": 1}) != content_hash({"a": 2})


def test_content_hash_property_idempotent() -> None:
    """schema_system.content_hash.property.idempotent

    Property: idempotent == true. Repeated calls on the same schema produce the
    same digest, and the input is not mutated.
    """
    schema = {"type": "object", "properties": {"x": {"type": "string"}}}
    snapshot = {"type": "object", "properties": {"x": {"type": "string"}}}
    first = content_hash(schema)
    second = content_hash(schema)
    assert first == second
    assert schema == snapshot


def test_content_hash_property_async_false() -> None:
    """schema_system.content_hash.property.async_false

    Property: async == false. `content_hash` MUST be a synchronous callable.
    """
    assert not inspect.iscoroutinefunction(content_hash)


async def test_content_hash_property_thread_safe() -> None:
    """schema_system.content_hash.property.thread_safe

    Property: thread_safe == true. >= 8 concurrent hashes of the same schema
    MUST all agree.
    """
    expected = content_hash(_CONSTRAINT_SCHEMA)

    async def run() -> str:
        return await asyncio.to_thread(content_hash, _CONSTRAINT_SCHEMA)

    digests = await asyncio.gather(*(run() for _ in range(8)))
    assert len(digests) >= 8
    assert all(d == expected for d in digests)
