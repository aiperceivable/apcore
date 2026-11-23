"""Tests for RefResolver $ref resolution."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest
import yaml

from apcore.errors import (
    SchemaCircularRefError,
    SchemaMaxDepthExceededError,
    SchemaNotFoundError,
    SchemaParseError,
)
from apcore.schema.ref_resolver import RefResolver


def write_yaml(path: Path, data: dict[str, Any] | str) -> Path:
    """Write a dict as YAML or raw string to a file, creating parent dirs."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(data, str):
        path.write_text(data)
    else:
        path.write_text(yaml.dump(data, default_flow_style=False))
    return path


# === Local $ref resolution ===


class TestLocalRef:
    def test_resolve_local_ref(self, tmp_path: Path) -> None:
        schema = {
            "type": "object",
            "properties": {
                "addr": {"$ref": "#/definitions/Address"},
            },
            "definitions": {
                "Address": {
                    "type": "object",
                    "properties": {"street": {"type": "string"}},
                },
            },
        }
        resolver = RefResolver(tmp_path)
        result = resolver.resolve(schema)
        assert result["properties"]["addr"] == {
            "type": "object",
            "properties": {"street": {"type": "string"}},
        }

    def test_resolve_local_ref_nested_pointer(self, tmp_path: Path) -> None:
        schema = {
            "type": "object",
            "properties": {
                "bar": {"$ref": "#/definitions/Foo/properties/bar"},
            },
            "definitions": {
                "Foo": {"properties": {"bar": {"type": "integer"}}},
            },
        }
        resolver = RefResolver(tmp_path)
        result = resolver.resolve(schema)
        assert result["properties"]["bar"] == {"type": "integer"}

    def test_local_ref_not_found(self, tmp_path: Path) -> None:
        schema = {"properties": {"x": {"$ref": "#/definitions/NonExistent"}}}
        resolver = RefResolver(tmp_path)
        with pytest.raises(SchemaNotFoundError):
            resolver.resolve(schema)


# === Relative file $ref resolution ===


class TestRelativeFileRef:
    def test_resolve_relative_file_with_pointer(self, tmp_path: Path) -> None:
        common_dir = tmp_path / "common"
        write_yaml(
            common_dir / "error.schema.yaml",
            {
                "definitions": {
                    "ErrorDetail": {
                        "type": "object",
                        "properties": {"code": {"type": "string"}},
                    }
                }
            },
        )
        main_file = write_yaml(
            tmp_path / "main.schema.yaml",
            {"properties": {"err": {"$ref": "./common/error.schema.yaml#/definitions/ErrorDetail"}}},
        )
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(main_file.read_text())
        result = resolver.resolve(schema, current_file=main_file)
        assert result["properties"]["err"] == {
            "type": "object",
            "properties": {"code": {"type": "string"}},
        }

    def test_resolve_relative_file_no_pointer(self, tmp_path: Path) -> None:
        write_yaml(tmp_path / "other.schema.yaml", {"type": "object", "description": "Other"})
        main_file = write_yaml(
            tmp_path / "main.schema.yaml",
            {"properties": {"x": {"$ref": "./other.schema.yaml"}}},
        )
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(main_file.read_text())
        result = resolver.resolve(schema, current_file=main_file)
        assert result["properties"]["x"] == {"type": "object", "description": "Other"}

    def test_relative_file_not_found(self, tmp_path: Path) -> None:
        main_file = write_yaml(
            tmp_path / "main.schema.yaml",
            {"properties": {"x": {"$ref": "./missing.schema.yaml"}}},
        )
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(main_file.read_text())
        with pytest.raises(SchemaNotFoundError):
            resolver.resolve(schema, current_file=main_file)


# === Canonical $ref resolution ===


class TestCanonicalRef:
    def test_resolve_canonical_ref(self, tmp_path: Path) -> None:
        write_yaml(
            tmp_path / "common" / "types" / "error.schema.yaml",
            {
                "ErrorDetail": {
                    "type": "object",
                    "properties": {"msg": {"type": "string"}},
                }
            },
        )
        schema = {"properties": {"err": {"$ref": "apcore://common.types.error/ErrorDetail"}}}
        resolver = RefResolver(tmp_path)
        result = resolver.resolve(schema)
        assert result["properties"]["err"] == {
            "type": "object",
            "properties": {"msg": {"type": "string"}},
        }

    def test_canonical_dots_to_path(self, tmp_path: Path) -> None:
        write_yaml(
            tmp_path / "a" / "b" / "c" / "d.schema.yaml",
            {"Foo": {"type": "string"}},
        )
        schema = {"x": {"$ref": "apcore://a.b.c.d/Foo"}}
        resolver = RefResolver(tmp_path)
        result = resolver.resolve(schema)
        assert result["x"] == {"type": "string"}


# === Nested $ref resolution ===


class TestNestedRef:
    def test_three_level_chain(self, tmp_path: Path) -> None:
        write_yaml(tmp_path / "c.schema.yaml", {"Final": {"type": "boolean"}})
        write_yaml(tmp_path / "b.schema.yaml", {"Mid": {"$ref": "./c.schema.yaml#/Final"}})
        file_a = write_yaml(
            tmp_path / "a.schema.yaml",
            {"properties": {"val": {"$ref": "./b.schema.yaml#/Mid"}}},
        )
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(file_a.read_text())
        result = resolver.resolve(schema, current_file=file_a)
        assert result["properties"]["val"] == {"type": "boolean"}

    def test_recursive_nested_ref(self, tmp_path: Path) -> None:
        write_yaml(
            tmp_path / "shared.schema.yaml",
            {"Inner": {"type": "integer"}},
        )
        schema = {
            "definitions": {
                "Wrapper": {"$ref": "./shared.schema.yaml#/Inner"},
            },
            "properties": {"w": {"$ref": "#/definitions/Wrapper"}},
        }
        file_main = write_yaml(tmp_path / "main.schema.yaml", schema)
        resolver = RefResolver(tmp_path)
        loaded = yaml.safe_load(file_main.read_text())
        result = resolver.resolve(loaded, current_file=file_main)
        assert result["properties"]["w"] == {"type": "integer"}


# === Circular reference detection ===


class TestCircularRef:
    def test_direct_circular(self, tmp_path: Path) -> None:
        write_yaml(tmp_path / "a.schema.yaml", {"X": {"$ref": "./b.schema.yaml#/Y"}})
        write_yaml(tmp_path / "b.schema.yaml", {"Y": {"$ref": "./a.schema.yaml#/X"}})
        file_a = tmp_path / "a.schema.yaml"
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(file_a.read_text())
        with pytest.raises(SchemaCircularRefError):
            resolver.resolve(schema, current_file=file_a)

    def test_self_ref(self, tmp_path: Path) -> None:
        schema = {
            "definitions": {"Self": {"$ref": "#/definitions/Self"}},
            "x": {"$ref": "#/definitions/Self"},
        }
        resolver = RefResolver(tmp_path)
        with pytest.raises(SchemaCircularRefError):
            resolver.resolve(schema)

    def test_three_way_circular(self, tmp_path: Path) -> None:
        write_yaml(tmp_path / "a.schema.yaml", {"X": {"$ref": "./b.schema.yaml#/Y"}})
        write_yaml(tmp_path / "b.schema.yaml", {"Y": {"$ref": "./c.schema.yaml#/Z"}})
        write_yaml(tmp_path / "c.schema.yaml", {"Z": {"$ref": "./a.schema.yaml#/X"}})
        file_a = tmp_path / "a.schema.yaml"
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(file_a.read_text())
        with pytest.raises(SchemaCircularRefError):
            resolver.resolve(schema, current_file=file_a)

    def test_max_depth_exceeded(self, tmp_path: Path) -> None:
        # Chain of 5 levels with max_depth=3.
        # Depth-cap exhaustion (A-D-038) emits SCHEMA_MAX_DEPTH_EXCEEDED,
        # distinct from a genuine cycle (SCHEMA_CIRCULAR_REF).
        write_yaml(tmp_path / "d.schema.yaml", {"V": {"$ref": "./e.schema.yaml#/W"}})
        write_yaml(tmp_path / "e.schema.yaml", {"W": {"type": "string"}})
        write_yaml(tmp_path / "c.schema.yaml", {"U": {"$ref": "./d.schema.yaml#/V"}})
        write_yaml(tmp_path / "b.schema.yaml", {"T": {"$ref": "./c.schema.yaml#/U"}})
        write_yaml(tmp_path / "a.schema.yaml", {"S": {"$ref": "./b.schema.yaml#/T"}})
        file_a = tmp_path / "a.schema.yaml"
        resolver = RefResolver(tmp_path, max_depth=3)
        schema = yaml.safe_load(file_a.read_text())
        with pytest.raises(SchemaMaxDepthExceededError) as excinfo:
            resolver.resolve(schema, current_file=file_a)
        assert excinfo.value.code == "SCHEMA_MAX_DEPTH_EXCEEDED"
        # A genuine cycle must NOT be misreported as a depth-cap error.
        assert not isinstance(excinfo.value, SchemaCircularRefError)

    def test_self_ref_cycle_keeps_circular_code(self, tmp_path: Path) -> None:
        # A genuine A->B->A cycle still raises SchemaCircularRefError
        # (SCHEMA_CIRCULAR_REF), not the depth-cap code.
        write_yaml(tmp_path / "a.schema.yaml", {"X": {"$ref": "./b.schema.yaml#/Y"}})
        write_yaml(tmp_path / "b.schema.yaml", {"Y": {"$ref": "./a.schema.yaml#/X"}})
        file_a = tmp_path / "a.schema.yaml"
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(file_a.read_text())
        with pytest.raises(SchemaCircularRefError) as excinfo:
            resolver.resolve(schema, current_file=file_a)
        assert excinfo.value.code == "SCHEMA_CIRCULAR_REF"


# === $ref sibling handling ===


class TestSiblingKeys:
    def test_sibling_description_merged(self, tmp_path: Path) -> None:
        schema = {
            "definitions": {"Foo": {"type": "string"}},
            "properties": {"x": {"$ref": "#/definitions/Foo", "description": "Override"}},
        }
        resolver = RefResolver(tmp_path)
        result = resolver.resolve(schema)
        assert result["properties"]["x"]["type"] == "string"
        assert result["properties"]["x"]["description"] == "Override"

    def test_sibling_overrides_target(self, tmp_path: Path) -> None:
        schema = {
            "definitions": {"Foo": {"type": "string", "description": "Original"}},
            "properties": {"x": {"$ref": "#/definitions/Foo", "description": "Override"}},
        }
        resolver = RefResolver(tmp_path)
        result = resolver.resolve(schema)
        assert result["properties"]["x"]["description"] == "Override"
        assert result["properties"]["x"]["type"] == "string"


# === File caching ===


class TestFileCache:
    def test_file_cached(self, tmp_path: Path) -> None:
        write_yaml(tmp_path / "shared.schema.yaml", {"Foo": {"type": "string"}})
        file_main = write_yaml(
            tmp_path / "main.schema.yaml",
            {
                "properties": {
                    "a": {"$ref": "./shared.schema.yaml#/Foo"},
                    "b": {"$ref": "./shared.schema.yaml#/Foo"},
                },
            },
        )
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(file_main.read_text())
        resolver.resolve(schema, current_file=file_main)
        shared_path = (tmp_path / "shared.schema.yaml").resolve()
        assert shared_path in resolver._file_cache


# === JSON Pointer RFC 6901 ===


class TestJsonPointer:
    def test_escaped_tilde_and_slash(self, tmp_path: Path) -> None:
        schema = {
            "definitions": {"a/b": {"type": "string"}, "c~d": {"type": "integer"}},
            "properties": {
                "x": {"$ref": "#/definitions/a~1b"},
                "y": {"$ref": "#/definitions/c~0d"},
            },
        }
        resolver = RefResolver(tmp_path)
        result = resolver.resolve(schema)
        assert result["properties"]["x"] == {"type": "string"}
        assert result["properties"]["y"] == {"type": "integer"}

    def test_empty_pointer_returns_document(self, tmp_path: Path) -> None:
        write_yaml(tmp_path / "doc.schema.yaml", {"type": "object", "description": "Whole doc"})
        file_main = write_yaml(
            tmp_path / "main.schema.yaml",
            {"x": {"$ref": "./doc.schema.yaml"}},
        )
        resolver = RefResolver(tmp_path)
        schema = yaml.safe_load(file_main.read_text())
        result = resolver.resolve(schema, current_file=file_main)
        assert result["x"] == {"type": "object", "description": "Whole doc"}

    def test_pointer_not_found(self, tmp_path: Path) -> None:
        schema = {
            "definitions": {"Foo": {"type": "string"}},
            "x": {"$ref": "#/definitions/Bar"},
        }
        resolver = RefResolver(tmp_path)
        with pytest.raises(SchemaNotFoundError):
            resolver.resolve(schema)


# === Edge cases ===


class TestEdgeCases:
    def test_empty_file_returns_empty_dict(self, tmp_path: Path) -> None:
        empty_file = tmp_path / "empty.schema.yaml"
        empty_file.write_text("")
        resolver = RefResolver(tmp_path)
        result = resolver._load_file(empty_file)
        assert result == {}

    def test_comment_only_file_returns_empty_dict(self, tmp_path: Path) -> None:
        comment_file = tmp_path / "comments.schema.yaml"
        comment_file.write_text("# This is a comment\n")
        resolver = RefResolver(tmp_path)
        result = resolver._load_file(comment_file)
        assert result == {}

    def test_invalid_yaml_raises_parse_error(self, tmp_path: Path) -> None:
        bad_file = tmp_path / "bad.schema.yaml"
        bad_file.write_text("key: [unclosed")
        resolver = RefResolver(tmp_path)
        with pytest.raises(SchemaParseError):
            resolver._load_file(bad_file)

    def test_original_schema_not_modified(self, tmp_path: Path) -> None:
        original = {
            "definitions": {"Foo": {"type": "string"}},
            "properties": {"x": {"$ref": "#/definitions/Foo"}},
        }
        import copy

        snapshot = copy.deepcopy(original)
        resolver = RefResolver(tmp_path)
        resolver.resolve(original)
        assert original == snapshot


# === Path-traversal containment (security) ===


class TestPathTraversalContainment:
    """A $ref must never resolve to a file outside the schemas directory.

    Mirrors apcore-typescript RefResolver._assertWithinSchemasDir, which
    rejects relative-file and file#pointer $refs escaping the schemas dir
    (e.g. ../../etc/passwd) by raising SchemaNotFoundError.
    """

    def test_plain_file_ref_escaping_schemas_dir_is_rejected(self, tmp_path: Path) -> None:
        schemas_dir = tmp_path / "schemas"
        # A real "secret" file outside the schemas directory that must not be read.
        secret = tmp_path / "secret.yaml"
        write_yaml(secret, {"password": "hunter2"})

        main_file = write_yaml(
            schemas_dir / "main.schema.yaml",
            {"properties": {"x": {"$ref": "../secret.yaml"}}},
        )
        resolver = RefResolver(schemas_dir)
        schema = yaml.safe_load(main_file.read_text())
        with pytest.raises(SchemaNotFoundError, match="outside schemas directory"):
            resolver.resolve(schema, current_file=main_file)

    def test_file_pointer_ref_escaping_schemas_dir_is_rejected(self, tmp_path: Path) -> None:
        schemas_dir = tmp_path / "schemas"
        secret = tmp_path / "outside.yaml"
        write_yaml(secret, {"foo": {"type": "string"}})

        main_file = write_yaml(
            schemas_dir / "main.schema.yaml",
            {"properties": {"x": {"$ref": "../outside.yaml#/foo"}}},
        )
        resolver = RefResolver(schemas_dir)
        schema = yaml.safe_load(main_file.read_text())
        with pytest.raises(SchemaNotFoundError, match="outside schemas directory"):
            resolver.resolve(schema, current_file=main_file)

    def test_deep_traversal_ref_is_rejected_without_reading_target(self, tmp_path: Path) -> None:
        schemas_dir = tmp_path / "schemas"
        main_file = write_yaml(
            schemas_dir / "main.schema.yaml",
            {"properties": {"x": {"$ref": "../../../etc/passwd"}}},
        )
        resolver = RefResolver(schemas_dir)
        schema = yaml.safe_load(main_file.read_text())
        # The guard must fire before any attempt to read the outside file:
        # _load_file would otherwise raise SchemaNotFoundError on the resolved
        # path, so we assert the containment message specifically.
        with pytest.raises(SchemaNotFoundError, match="outside schemas directory"):
            resolver.resolve(schema, current_file=main_file)

    def test_legitimate_relative_ref_inside_schemas_dir_still_works(self, tmp_path: Path) -> None:
        schemas_dir = tmp_path / "schemas"
        write_yaml(
            schemas_dir / "common" / "error.schema.yaml",
            {"definitions": {"E": {"type": "object"}}},
        )
        main_file = write_yaml(
            schemas_dir / "main.schema.yaml",
            {"properties": {"e": {"$ref": "./common/error.schema.yaml#/definitions/E"}}},
        )
        resolver = RefResolver(schemas_dir)
        schema = yaml.safe_load(main_file.read_text())
        result = resolver.resolve(schema, current_file=main_file)
        assert result["properties"]["e"] == {"type": "object"}


# ---------------------------------------------------------------------------
# Self-reference vs. circular reference (PROTOCOL_SPEC §4.15)
# ---------------------------------------------------------------------------

TREE_SCHEMA: dict[str, Any] = {
    "$id": "TreeNode",
    "type": "object",
    "required": ["value"],
    "properties": {
        "value": {"type": "string"},
        "children": {"type": "array", "items": {"$ref": "#"}},
    },
}


class TestSelfReference:
    """A recursive `$ref` is preserved lazily; only a `$ref` → `$ref` cycle fails."""

    def test_hash_self_reference_is_preserved_not_rejected(self, tmp_path: Path) -> None:
        # Regression: every self-referencing schema was unregisterable — the
        # resolver treated the recursion anchor as a cycle and raised
        # SCHEMA_CIRCULAR_REF, so `SchemaLoader.resolve()` failed before the model
        # generator ever saw the schema.
        resolved = RefResolver(schemas_dir=str(tmp_path)).resolve(TREE_SCHEMA)
        assert resolved["properties"]["children"]["items"] == {"$ref": "#"}

    def test_self_reference_by_document_id_is_preserved(self, tmp_path: Path) -> None:
        schema = {
            **TREE_SCHEMA,
            "properties": {
                "value": {"type": "string"},
                "children": {"type": "array", "items": {"$ref": "TreeNode"}},
            },
        }
        # Previously this took the relative-file branch and raised
        # SchemaNotFoundError looking for a file named `TreeNode` on disk.
        resolved = RefResolver(schemas_dir=str(tmp_path)).resolve(schema)
        assert resolved["properties"]["children"]["items"] == {"$ref": "TreeNode"}

    def test_self_referencing_defs_entry_is_preserved_at_re_entry(self, tmp_path: Path) -> None:
        schema = {
            "type": "object",
            "properties": {"root": {"$ref": "#/$defs/Node"}},
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {"child": {"$ref": "#/$defs/Node"}},
                }
            },
        }
        resolved = RefResolver(schemas_dir=str(tmp_path)).resolve(schema)
        root = resolved["properties"]["root"]
        assert root["type"] == "object"
        assert root["properties"]["child"] == {"$ref": "#/$defs/Node"}

    def test_ref_only_cycle_still_raises_circular_ref(self, tmp_path: Path) -> None:
        # A `$ref` → `$ref` chain reaches no schema body, consumes no instance and
        # cannot terminate: that is the circular case §4.15 still rejects.
        schema = {
            "$ref": "#/$defs/a",
            "$defs": {"a": {"$ref": "#/$defs/b"}, "b": {"$ref": "#/$defs/a"}},
        }
        with pytest.raises(SchemaCircularRefError) as exc_info:
            RefResolver(schemas_dir=str(tmp_path)).resolve(schema)
        assert exc_info.value.code == "SCHEMA_CIRCULAR_REF"
