"""Local-reference sensitive redaction regressions."""

from __future__ import annotations

import copy
from typing import Any
from unittest.mock import patch

import pytest

from apcore.utils.redaction import REDACTED_VALUE, redact_sensitive


@pytest.mark.parametrize("reference", ["#/$defs/Payload", "#payload", "https://example.test/schema#payload"])
def test_redact_sensitive_follows_local_object_reference_and_preserves_inputs(reference: str) -> None:
    schema = {
        "$id": "https://example.test/schema",
        "type": "object",
        "$defs": {
            "Payload": {
                "$anchor": "payload",
                "type": "object",
                "properties": {"value": {"type": "string", "x-sensitive": True}},
            }
        },
        "properties": {"payload": {"$ref": reference}},
    }
    data = {"payload": {"value": "private-canary", "public": 1}}
    original_data, original_schema = copy.deepcopy(data), copy.deepcopy(schema)
    assert redact_sensitive(data, schema, sensitive_keys=[]) == {"payload": {"value": REDACTED_VALUE, "public": 1}}
    assert data == original_data and schema == original_schema


@pytest.mark.parametrize("keyword", ["anyOf", "oneOf", "allOf"])
def test_redact_sensitive_follows_combinator_reference_union(keyword: str) -> None:
    schema = {
        "$defs": {"Sensitive": {"properties": {"value": {"x-sensitive": True}}}},
        "properties": {"payload": {keyword: [{"$ref": "#/$defs/Sensitive"}, {"type": "null"}]}},
    }
    assert redact_sensitive({"payload": {"value": "private-canary"}}, schema, sensitive_keys=[]) == {
        "payload": {"value": REDACTED_VALUE}
    }


def test_redact_sensitive_follows_scalar_and_nested_array_item_references() -> None:
    schema = {
        "$defs": {
            "Value": {"type": "string", "x-sensitive": True},
            "Rows": {"type": "array", "items": {"$ref": "#/$defs/Value"}},
        },
        "properties": {
            "value": {"$ref": "#/$defs/Value"},
            "rows": {"type": "array", "items": {"$ref": "#/$defs/Rows"}},
        },
    }
    assert redact_sensitive(
        {"value": "private-canary", "rows": [["private-canary", None]]}, schema, sensitive_keys=[]
    ) == {"value": REDACTED_VALUE, "rows": [[REDACTED_VALUE, None]]}


def test_redact_sensitive_recurses_over_data_without_expanding_recursive_schema() -> None:
    schema = {
        "$defs": {
            "Node": {"type": "object", "properties": {"value": {"x-sensitive": True}, "next": {"$ref": "#/$defs/Node"}}}
        },
        "properties": {"head": {"$ref": "#/$defs/Node"}},
    }
    data = {"head": {"value": "private-canary", "next": {"value": "private-canary", "next": {"value": None}}}}
    assert redact_sensitive(data, schema, sensitive_keys=[]) == {
        "head": {"value": REDACTED_VALUE, "next": {"value": REDACTED_VALUE, "next": {"value": None}}}
    }


def test_redact_sensitive_reference_cycle_keeps_sibling_markers_and_terminates() -> None:
    schema = {
        "$defs": {
            "A": {"$ref": "#/$defs/B"},
            "B": {"$ref": "#/$defs/A", "properties": {"value": {"x-sensitive": True}}},
        },
        "properties": {"payload": {"$ref": "#/$defs/A"}, "whole": {"$ref": "#/$defs/A", "x-sensitive": True}},
    }
    assert redact_sensitive(
        {"payload": {"value": "private-canary"}, "whole": {"value": "private-canary"}}, schema, sensitive_keys=[]
    ) == {"payload": {"value": REDACTED_VALUE}, "whole": REDACTED_VALUE}


def test_redact_sensitive_never_retrieves_external_or_file_references() -> None:
    schema = {
        "properties": {
            "payload": {"$ref": "https://example.test/schema"},
            "value": {"$ref": "file:///tmp/private-schema", "x-sensitive": True},
        }
    }
    with patch("urllib.request.urlopen") as fetch:
        assert redact_sensitive({"payload": {"public": 1}, "value": "private-canary"}, schema, sensitive_keys=[]) == {
            "payload": {"public": 1},
            "value": REDACTED_VALUE,
        }
    fetch.assert_not_called()


def test_redact_sensitive_local_reference_escapes_and_default_patterns_survive() -> None:
    schema = {"$defs": {"a/b~c": {"x-sensitive": True}}, "properties": {"value": {"$ref": "#/$defs/a~1b~0c"}}}
    result = redact_sensitive(
        {"value": "private-canary", "api_key": "private-canary", "trace_id": "correlation"}, schema
    )
    assert result == {"value": REDACTED_VALUE, "api_key": REDACTED_VALUE, "trace_id": "correlation"}


@pytest.mark.parametrize(
    "reference,data,expected",
    [
        ("child#secret", {"payload": {"value": "private-canary"}}, {"payload": {"value": REDACTED_VALUE}}),
        ("child#/properties/value", {"payload": "private-canary"}, {"payload": REDACTED_VALUE}),
    ],
)
def test_redact_sensitive_embedded_resource_anchors_and_pointers(
    reference: str, data: dict[str, Any], expected: dict[str, Any]
) -> None:
    schema = {
        "$id": "https://example.test/root",
        "type": "object",
        "$defs": {
            "Child": {
                "$id": "child",
                "$anchor": "secret",
                "type": "object",
                "properties": {"value": {"x-sensitive": True}},
            }
        },
        "properties": {"payload": {"$ref": reference}},
    }
    with patch("urllib.request.urlopen") as fetch:
        assert redact_sensitive(data, schema, sensitive_keys=[]) == expected
    fetch.assert_not_called()


def test_redact_sensitive_percent_encoded_local_pointer() -> None:
    schema = {"$defs": {"some value": {"x-sensitive": True}}, "properties": {"value": {"$ref": "#/$defs/some%20value"}}}
    assert redact_sensitive({"value": "private-canary"}, schema, sensitive_keys=[]) == {"value": REDACTED_VALUE}


@pytest.mark.parametrize("dialect", ["2020", "draft7"])
@pytest.mark.parametrize("sensitive_tail", [False, True])
def test_redact_sensitive_positional_references_preserve_prefix_and_tail_semantics(
    dialect: str, sensitive_tail: bool
) -> None:
    sensitive = {"$ref": "#/definitions/Sensitive"}
    public = {"type": ["string", "null"]}
    prefix, tail = (public, sensitive) if sensitive_tail else (sensitive, public)
    shape = (
        {"prefixItems": [prefix], "items": tail} if dialect == "2020" else {"items": [prefix], "additionalItems": tail}
    )
    schema = {
        "$schema": "https://json-schema.org/draft/2020-12/schema"
        if dialect == "2020"
        else "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "definitions": {"Sensitive": {"type": ["string", "null"], "x-sensitive": True}},
        "properties": {"rows": {"type": "array", "items": {"type": "array", **shape}}},
    }
    data = {
        "rows": [
            ["public" if sensitive_tail else "private-canary", "private-canary" if sensitive_tail else "public", None]
        ]
    }
    original_data, original_schema = copy.deepcopy(data), copy.deepcopy(schema)
    assert redact_sensitive(data, schema, sensitive_keys=[]) == {
        "rows": [["public" if sensitive_tail else REDACTED_VALUE, REDACTED_VALUE if sensitive_tail else "public", None]]
    }
    assert data == original_data and schema == original_schema


def test_redact_sensitive_declared_draft7_fragment_identifier_anchor() -> None:
    schema = {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "https://example.test/root",
        "definitions": {"Child": {"$id": "#secret", "properties": {"value": {"x-sensitive": True}}}},
        "properties": {"payload": {"$ref": "#secret"}},
    }
    assert redact_sensitive({"payload": {"value": "private-canary"}}, schema, sensitive_keys=[]) == {
        "payload": {"value": REDACTED_VALUE}
    }


@pytest.mark.parametrize(
    "dialect", ["http://json-schema.org/draft-07/schema#", "https://json-schema.org/draft/2019-09/schema"]
)
def test_redact_sensitive_legacy_draft_ignores_prefix_items_metadata(dialect: str) -> None:
    schema = {
        "$schema": dialect,
        "type": "object",
        "properties": {
            "rows": {
                "type": "array",
                "items": {
                    "type": "array",
                    "prefixItems": [{"type": "string"}],
                    "items": {"type": "string", "x-sensitive": True},
                },
            }
        },
    }
    assert redact_sensitive({"rows": [["private-first", "private-tail"]]}, schema, sensitive_keys=[]) == {
        "rows": [[REDACTED_VALUE, REDACTED_VALUE]]
    }
