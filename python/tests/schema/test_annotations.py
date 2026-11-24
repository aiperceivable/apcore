"""Tests for annotation conflict resolution merge functions."""

from __future__ import annotations

from typing import Any

from apcore.module import ModuleAnnotations, ModuleExample
from apcore.schema.annotations import merge_annotations, merge_examples, merge_metadata


# === merge_annotations() ===


class TestMergeAnnotations:
    def test_both_none_returns_defaults(self) -> None:
        result = merge_annotations(None, None)
        assert result == ModuleAnnotations()
        assert result.readonly is False
        assert result.destructive is False
        assert result.idempotent is False
        assert result.requires_approval is False
        assert result.open_world is True

    def test_only_code(self) -> None:
        code = ModuleAnnotations(readonly=True, destructive=True)
        result = merge_annotations(None, code)
        assert result.readonly is True
        assert result.destructive is True
        assert result.idempotent is False

    def test_only_yaml(self) -> None:
        result = merge_annotations({"readonly": True}, None)
        assert result.readonly is True
        assert result.destructive is False

    def test_both_no_conflict(self) -> None:
        code = ModuleAnnotations(idempotent=True)
        yaml: dict[str, Any] = {"readonly": True}
        result = merge_annotations(yaml, code)
        assert result.readonly is True
        assert result.idempotent is True

    def test_both_yaml_wins(self) -> None:
        code = ModuleAnnotations(readonly=False, destructive=True)
        yaml: dict[str, Any] = {"readonly": True, "destructive": False}
        result = merge_annotations(yaml, code)
        assert result.readonly is True
        assert result.destructive is False

    def test_yaml_partial_code_preserved(self) -> None:
        code = ModuleAnnotations(destructive=True, idempotent=True)
        yaml: dict[str, Any] = {"readonly": True}
        result = merge_annotations(yaml, code)
        assert result.readonly is True
        assert result.destructive is True
        assert result.idempotent is True
        assert result.requires_approval is False
        assert result.open_world is True

    def test_new_annotation_fields_defaults(self) -> None:
        result = merge_annotations(None, None)
        assert result.cacheable is False
        assert result.cache_ttl == 0
        assert result.cache_key_fields is None
        assert result.paginated is False
        assert result.pagination_style == "cursor"

    def test_new_annotation_fields_from_code(self) -> None:
        code = ModuleAnnotations(
            cacheable=True, cache_ttl=300, cache_key_fields=["id"], paginated=True, pagination_style="offset"
        )
        result = merge_annotations(None, code)
        assert result.cacheable is True
        assert result.cache_ttl == 300
        assert result.cache_key_fields == ("id",)
        assert result.paginated is True
        assert result.pagination_style == "offset"

    def test_new_annotation_fields_yaml_overrides_code(self) -> None:
        code = ModuleAnnotations(cacheable=True, cache_ttl=300)
        yaml: dict[str, Any] = {"cacheable": False, "cache_ttl": 0, "paginated": True}
        result = merge_annotations(yaml, code)
        assert result.cacheable is False
        assert result.cache_ttl == 0
        assert result.paginated is True

    def test_unknown_yaml_key_ignored(self) -> None:
        yaml: dict[str, Any] = {"readonly": True, "unknown_field": 42}
        result = merge_annotations(yaml, None)
        assert result.readonly is True


# === merge_examples() ===


class TestMergeExamples:
    def test_yaml_overrides_code(self) -> None:
        yaml = [{"title": "Ex1", "inputs": {"x": 1}}]
        code = [ModuleExample(title="Code", inputs={"y": 2})]
        result = merge_examples(yaml, code)
        assert len(result) == 1
        assert result[0].title == "Ex1"
        assert result[0].inputs == {"x": 1}

    def test_yaml_empty_list_overrides_code(self) -> None:
        code = [ModuleExample(title="Code", inputs={"y": 2})]
        result = merge_examples([], code)
        assert result == []

    def test_yaml_none_uses_code(self) -> None:
        code = [ModuleExample(title="Code", inputs={"y": 2})]
        result = merge_examples(None, code)
        assert result == code

    def test_both_none_returns_empty(self) -> None:
        result = merge_examples(None, None)
        assert result == []

    def test_yaml_dict_converted_to_module_example(self) -> None:
        yaml = [
            {
                "title": "Full",
                "inputs": {"a": 1},
                "output": {"b": 2},
                "description": "desc",
            }
        ]
        result = merge_examples(yaml, None)
        assert isinstance(result[0], ModuleExample)
        assert result[0].title == "Full"
        assert result[0].inputs == {"a": 1}
        assert result[0].output == {"b": 2}
        assert result[0].description == "desc"


# === merge_metadata() ===


class TestMergeMetadata:
    def test_both_yaml_overrides(self) -> None:
        code: dict[str, Any] = {"version": "1.0", "author": "Alice"}
        yaml: dict[str, Any] = {"version": "2.0", "license": "MIT"}
        result = merge_metadata(yaml, code)
        assert result == {"version": "2.0", "author": "Alice", "license": "MIT"}

    def test_only_code(self) -> None:
        result = merge_metadata(None, {"key": "val"})
        assert result == {"key": "val"}

    def test_only_yaml(self) -> None:
        result = merge_metadata({"key": "val"}, None)
        assert result == {"key": "val"}

    def test_both_none(self) -> None:
        result = merge_metadata(None, None)
        assert result == {}
