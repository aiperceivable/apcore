"""Tests for schema query and export functions."""

from __future__ import annotations

import json
from typing import Any

import pytest
import yaml
from pydantic import BaseModel

from apcore.errors import ModuleNotFoundError
from apcore.module import ModuleAnnotations, ModuleExample
from apcore.registry.registry import Registry
from apcore.registry.schema_export import (
    export_all_schemas,
    export_schema,
    get_all_schemas,
    get_schema,
)


# ---------------------------------------------------------------------------
# Helper schemas and modules
# ---------------------------------------------------------------------------


class _InputSchema(BaseModel):
    query: str
    limit: int = 10


class _OutputSchema(BaseModel):
    results: list[str]
    count: int


class _AnnotatedModule:
    """Module with annotations and examples."""

    input_schema = _InputSchema
    output_schema = _OutputSchema
    description = "A test module for schema export"
    name = "Test Export Module"
    version = "2.0.0"
    tags = ["search", "query"]
    annotations = ModuleAnnotations(
        readonly=True,
        destructive=False,
        idempotent=True,
        requires_approval=False,
        open_world=False,
    )
    examples = [
        ModuleExample(
            title="Basic search",
            inputs={"query": "hello", "limit": 5},
            output={"results": ["hello world"], "count": 1},
            description="A simple search example",
        )
    ]

    def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
        return {"results": [], "count": 0}


class _SimpleModule:
    """Minimal module without annotations/examples."""

    input_schema = _InputSchema
    output_schema = _OutputSchema
    description = "Simple module"

    def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
        return {"results": [], "count": 0}


def _make_registry() -> Registry:
    """Create a Registry with test modules."""
    reg = Registry()
    reg.register("test.annotated", _AnnotatedModule())
    reg.register("test.simple", _SimpleModule())
    return reg


# ---------------------------------------------------------------------------
# get_schema() tests
# ---------------------------------------------------------------------------


class TestGetSchema:
    def test_returns_structured_dict_with_all_fields(self) -> None:
        """get_schema returns dict with all expected fields."""
        reg = _make_registry()
        result = get_schema(reg, "test.annotated")
        assert result is not None
        assert result["module_id"] == "test.annotated"
        assert result["name"] == "Test Export Module"
        assert result["description"] == "A test module for schema export"
        assert result["version"] == "2.0.0"
        assert result["tags"] == ["search", "query"]
        assert "properties" in result["input_schema"]
        assert "properties" in result["output_schema"]

    def test_returns_none_for_nonexistent(self) -> None:
        """get_schema returns None for missing module."""
        reg = _make_registry()
        assert get_schema(reg, "nonexistent") is None

    def test_schemas_from_pydantic(self) -> None:
        """Input/output schemas come from model_json_schema()."""
        reg = _make_registry()
        result = get_schema(reg, "test.annotated")
        assert result is not None
        assert "query" in result["input_schema"]["properties"]
        assert "limit" in result["input_schema"]["properties"]
        assert "results" in result["output_schema"]["properties"]
        assert "count" in result["output_schema"]["properties"]

    def test_annotations_serialized_as_dict(self) -> None:
        """Annotations are serialized to a plain dict."""
        reg = _make_registry()
        result = get_schema(reg, "test.annotated")
        assert result is not None
        ann = result["annotations"]
        assert isinstance(ann, dict)
        assert ann["readonly"] is True
        assert ann["idempotent"] is True
        assert ann["open_world"] is False

    def test_examples_serialized(self) -> None:
        """Examples are serialized to list of dicts."""
        reg = _make_registry()
        result = get_schema(reg, "test.annotated")
        assert result is not None
        examples = result["examples"]
        assert len(examples) == 1
        assert examples[0]["title"] == "Basic search"
        assert examples[0]["inputs"]["query"] == "hello"

    def test_no_annotations_returns_none(self) -> None:
        """Module without annotations has None in annotations field."""
        reg = _make_registry()
        result = get_schema(reg, "test.simple")
        assert result is not None
        assert result["annotations"] is None

    def test_no_examples_returns_empty_list(self) -> None:
        """Module without examples has empty list."""
        reg = _make_registry()
        result = get_schema(reg, "test.simple")
        assert result is not None
        assert result["examples"] == []


# ---------------------------------------------------------------------------
# export_schema() tests
# ---------------------------------------------------------------------------


class TestExportSchema:
    def test_export_json(self) -> None:
        """export_schema format='json' returns parseable JSON."""
        reg = _make_registry()
        result = export_schema(reg, "test.annotated", format="json")
        parsed = json.loads(result)
        assert parsed["module_id"] == "test.annotated"

    def test_export_yaml(self) -> None:
        """export_schema format='yaml' returns parseable YAML."""
        reg = _make_registry()
        result = export_schema(reg, "test.annotated", format="yaml")
        parsed = yaml.safe_load(result)
        assert parsed["module_id"] == "test.annotated"

    def test_strict_mode(self) -> None:
        """export_schema strict=True applies strict transformations."""
        reg = _make_registry()
        result = export_schema(reg, "test.annotated", format="json", strict=True)
        parsed = json.loads(result)
        input_schema = parsed["input_schema"]
        assert input_schema.get("additionalProperties") is False
        assert "required" in input_schema

    def test_compact_mode(self) -> None:
        """export_schema compact=True truncates description, removes extras."""
        reg = Registry()
        mod = _AnnotatedModule()
        mod.description = "First sentence. Second sentence with more detail."
        reg.register("test.mod", mod)
        result = export_schema(reg, "test.mod", format="json", compact=True)
        parsed = json.loads(result)
        assert parsed["description"] == "First sentence."
        assert "examples" not in parsed
        assert "documentation" not in parsed

    def test_nonexistent_raises(self) -> None:
        """export_schema raises ModuleNotFoundError for missing module."""
        reg = _make_registry()
        with pytest.raises(ModuleNotFoundError):
            export_schema(reg, "missing")

    def test_profile_export(self) -> None:
        """export_schema with profile delegates to SchemaExporter."""
        reg = _make_registry()
        result = export_schema(reg, "test.annotated", format="json", profile="mcp")
        parsed = json.loads(result)
        # MCP format has 'name' and 'inputSchema' keys
        assert "name" in parsed
        assert "inputSchema" in parsed


# ---------------------------------------------------------------------------
# get_all_schemas() / export_all_schemas() tests
# ---------------------------------------------------------------------------


class TestGetAllSchemas:
    def test_returns_dict_keyed_by_module_id(self) -> None:
        """get_all_schemas returns dict keyed by module_id."""
        reg = _make_registry()
        result = get_all_schemas(reg)
        assert "test.annotated" in result
        assert "test.simple" in result
        assert len(result) == 2

    def test_empty_registry(self) -> None:
        """get_all_schemas on empty registry returns empty dict."""
        reg = Registry()
        assert get_all_schemas(reg) == {}


class TestExportAllSchemas:
    def test_export_all_json(self) -> None:
        """export_all_schemas format='json' returns JSON with all modules."""
        reg = _make_registry()
        result = export_all_schemas(reg, format="json")
        parsed = json.loads(result)
        assert "test.annotated" in parsed
        assert "test.simple" in parsed

    def test_export_all_yaml(self) -> None:
        """export_all_schemas format='yaml' returns YAML with all modules."""
        reg = _make_registry()
        result = export_all_schemas(reg, format="yaml")
        parsed = yaml.safe_load(result)
        assert "test.annotated" in parsed
        assert "test.simple" in parsed

    def test_empty_registry(self) -> None:
        """export_all_schemas on empty registry returns serialized empty dict."""
        reg = Registry()
        result = export_all_schemas(reg, format="json")
        assert json.loads(result) == {}
