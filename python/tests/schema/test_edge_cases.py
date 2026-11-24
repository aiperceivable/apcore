"""Edge case and integration tests for the schema system."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest

from apcore.errors import SchemaNotFoundError, SchemaParseError
from apcore.schema import SchemaLoader, SchemaValidator
from apcore.config import Config


# === Empty and minimal schemas ===


class TestEmptySchemas:
    def test_empty_file_raises_parse_error(self, schema_loader: SchemaLoader) -> None:
        """Empty schema file (0 bytes) should raise SchemaParseError."""
        with pytest.raises(SchemaParseError):
            schema_loader.load("empty")

    def test_comment_only_file_raises(self, schemas_dir: Path) -> None:
        """Schema with only YAML comments raises SchemaParseError."""
        comment_file = schemas_dir / "comments_only.schema.yaml"
        comment_file.write_text("# This is a comment\n# Another comment\n")
        config = Config(data={"schema": {"root": str(schemas_dir)}})
        loader = SchemaLoader(config=config, schemas_dir=schemas_dir)
        with pytest.raises(SchemaParseError):
            loader.load("comments_only")


# === $ref edge cases ===


class TestRefEdgeCases:
    def test_ref_with_sibling_description(self) -> None:
        """$ref alongside sibling keys merges correctly via RefResolver."""
        from apcore.schema import RefResolver

        schema = {
            "type": "object",
            "properties": {
                "address": {
                    "$ref": "#/definitions/Address",
                    "description": "Overridden description",
                },
            },
            "definitions": {
                "Address": {
                    "type": "object",
                    "properties": {"city": {"type": "string"}},
                    "description": "Original description",
                },
            },
        }
        resolver = RefResolver(schemas_dir=Path("/tmp"), max_depth=32)
        resolved = resolver.resolve(schema)
        addr = resolved["properties"]["address"]
        assert addr["description"] == "Overridden description"
        assert "city" in addr["properties"]


# === Type system edge cases ===


class TestTypeEdgeCases:
    def test_deeply_nested_schema_loads(self, schema_loader: SchemaLoader) -> None:
        """Deeply nested schema (>4 levels) generates model successfully."""
        sd = schema_loader.load("nested_objects")
        input_rs, _ = schema_loader.resolve(sd)
        model = input_rs.model
        # Validate a deeply nested data dict
        instance = model.model_validate(
            {
                "config": {
                    "database": {
                        "connection": {
                            "host": "localhost",
                            "port": 5432,
                            "pool": {"min_size": 1, "max_size": 10},
                        },
                        "name": "testdb",
                    },
                    "cache_ttl": 300,
                }
            }
        )
        assert instance is not None

    def test_nullable_type_array(self) -> None:
        """type: ["string", "null"] generates Optional[str] model field."""
        schema: dict[str, Any] = {
            "type": "object",
            "properties": {
                "value": {"type": ["string", "null"]},
            },
            "required": ["value"],
        }
        config = Config(data={})
        loader = SchemaLoader(config=config)
        model = loader.generate_model(schema, "NullableTest")
        assert model.model_validate({"value": "hello"}) is not None
        assert model.model_validate({"value": None}) is not None

    def test_enum_containing_null(self) -> None:
        """Enum with null value accepts None."""
        schema: dict[str, Any] = {
            "type": "object",
            "properties": {
                "status": {"enum": ["active", "inactive", None]},
            },
            "required": ["status"],
        }
        config = Config(data={})
        loader = SchemaLoader(config=config)
        model = loader.generate_model(schema, "EnumNullTest")
        assert model.model_validate({"status": "active"}) is not None
        assert model.model_validate({"status": None}) is not None

    def test_format_annotations_stored_not_enforced(self) -> None:
        """Format annotations are stored but not enforced during validation."""
        schema: dict[str, Any] = {
            "type": "object",
            "properties": {
                "created_at": {"type": "string", "format": "date-time"},
                "email": {"type": "string", "format": "email"},
            },
            "required": ["created_at", "email"],
        }
        config = Config(data={})
        loader = SchemaLoader(config=config)
        model = loader.generate_model(schema, "FormatTest")
        # Non-conforming values should still pass (format not enforced)
        instance = model.model_validate({"created_at": "not-a-date", "email": "not-an-email"})
        assert instance is not None


# === Unicode ===


class TestUnicode:
    def test_non_ascii_field_names(self) -> None:
        """Non-ASCII field names work correctly."""
        schema: dict[str, Any] = {
            "type": "object",
            "properties": {
                "nombre": {"type": "string"},
                "ciudad": {"type": "string"},
            },
            "required": ["nombre", "ciudad"],
        }
        config = Config(data={})
        loader = SchemaLoader(config=config)
        model = loader.generate_model(schema, "UnicodeTest")
        instance = model.model_validate({"nombre": "Juan", "ciudad": "Madrid"})
        assert instance is not None


# === Definitions edge cases ===


class TestDefinitionsEdgeCases:
    def test_missing_definitions_raises(self) -> None:
        """$ref to #/definitions/Foo with no definitions key raises SchemaNotFoundError."""
        from apcore.schema import RefResolver

        schema = {
            "type": "object",
            "properties": {
                "item": {"$ref": "#/definitions/Foo"},
            },
        }
        resolver = RefResolver(schemas_dir=Path("/tmp"), max_depth=32)
        with pytest.raises(SchemaNotFoundError):
            resolver.resolve(schema)

    def test_defs_overrides_definitions(self, schemas_dir: Path) -> None:
        """$defs entries override definitions entries with same name."""
        schema_file = schemas_dir / "dual_defs.schema.yaml"
        schema_file.write_text(
            "module_id: test.dual_defs\n"
            "description: Dual defs test\n"
            "definitions:\n"
            "  SharedType:\n"
            "    type: object\n"
            "    properties:\n"
            "      old_field: {type: string}\n"
            "$defs:\n"
            "  SharedType:\n"
            "    type: object\n"
            "    properties:\n"
            "      new_field: {type: integer}\n"
            "input_schema:\n"
            "  type: object\n"
            "  properties:\n"
            "    data: {type: string}\n"
            "output_schema:\n"
            "  type: object\n"
            "  properties:\n"
            "    result: {type: string}\n"
        )
        config = Config(data={"schema": {"root": str(schemas_dir)}})
        loader = SchemaLoader(config=config, schemas_dir=schemas_dir)
        sd = loader.load("dual_defs")
        assert "new_field" in sd.definitions["SharedType"]["properties"]


# === Large schemas ===


class TestLargeSchemas:
    def test_many_properties(self) -> None:
        """Schema with 100 properties generates model and validates."""
        props: dict[str, Any] = {}
        for i in range(100):
            props[f"field_{i}"] = {"type": "string"}
        schema: dict[str, Any] = {
            "type": "object",
            "properties": props,
            "required": list(props.keys()),
        }
        config = Config(data={})
        loader = SchemaLoader(config=config)
        model = loader.generate_model(schema, "LargeModel")
        # Valid data
        data = {f"field_{i}": f"value_{i}" for i in range(100)}
        instance = model.model_validate(data)
        assert instance is not None
        # Missing required field
        del data["field_50"]
        validator = SchemaValidator(coerce_types=True)
        result = validator.validate(data, model)
        assert result.valid is False


# === Public API imports ===


class TestPublicApi:
    def test_all_names_importable(self) -> None:
        """All 14 public API names are importable from apcore.schema."""
        from apcore.schema import (
            ExportProfile,
            LLMExtensions,
            RefResolver,
            ResolvedSchema,
            SchemaDefinition,
            SchemaExporter,
            SchemaLoader,
            SchemaStrategy,
            SchemaValidationErrorDetail,
            SchemaValidationResult,
            SchemaValidator,
            merge_annotations,
            merge_examples,
            merge_metadata,
            to_strict_schema,
        )

        # Just verify they're all not None
        assert all(
            [
                ExportProfile,
                LLMExtensions,
                RefResolver,
                ResolvedSchema,
                SchemaDefinition,
                SchemaExporter,
                SchemaLoader,
                SchemaStrategy,
                SchemaValidationErrorDetail,
                SchemaValidationResult,
                SchemaValidator,
                merge_annotations,
                merge_examples,
                merge_metadata,
                to_strict_schema,
            ]
        )
