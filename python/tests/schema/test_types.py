"""Tests for schema type definitions."""

from __future__ import annotations

import pytest
from pydantic import BaseModel

from apcore.errors import SchemaValidationError
from apcore.schema.types import (
    ExportProfile,
    LLMExtensions,
    ResolvedSchema,
    SchemaDefinition,
    SchemaStrategy,
    SchemaValidationErrorDetail,
    SchemaValidationResult,
)


class TestSchemaStrategy:
    def test_yaml_first_value(self) -> None:
        assert SchemaStrategy.YAML_FIRST == "yaml_first"

    def test_native_first_value(self) -> None:
        assert SchemaStrategy.NATIVE_FIRST == "native_first"

    def test_yaml_only_value(self) -> None:
        assert SchemaStrategy.YAML_ONLY == "yaml_only"

    def test_string_comparison(self) -> None:
        assert SchemaStrategy.YAML_FIRST == "yaml_first"
        assert isinstance(SchemaStrategy.YAML_FIRST, str)


class TestExportProfile:
    def test_mcp_value(self) -> None:
        assert ExportProfile.MCP == "mcp"

    def test_openai_value(self) -> None:
        assert ExportProfile.OPENAI == "openai"

    def test_anthropic_value(self) -> None:
        assert ExportProfile.ANTHROPIC == "anthropic"

    def test_generic_value(self) -> None:
        assert ExportProfile.GENERIC == "generic"

    def test_string_comparison(self) -> None:
        assert ExportProfile.MCP == "mcp"
        assert isinstance(ExportProfile.MCP, str)


class TestSchemaDefinition:
    def test_required_fields(self) -> None:
        sd = SchemaDefinition(
            module_id="test.module",
            description="A test",
            input_schema={"type": "object"},
            output_schema={"type": "object"},
        )
        assert sd.module_id == "test.module"
        assert sd.description == "A test"
        assert sd.input_schema == {"type": "object"}
        assert sd.output_schema == {"type": "object"}

    def test_default_values(self) -> None:
        sd = SchemaDefinition(
            module_id="test.module",
            description="A test",
            input_schema={"type": "object"},
            output_schema={"type": "object"},
        )
        assert sd.error_schema is None
        assert sd.definitions == {}
        assert sd.version == "1.0.0"
        assert sd.documentation is None
        assert sd.schema_url is None

    def test_all_fields(self) -> None:
        sd = SchemaDefinition(
            module_id="db.query",
            description="Query DB",
            input_schema={
                "type": "object",
                "properties": {"table": {"type": "string"}},
            },
            output_schema={"type": "object"},
            error_schema={"type": "object"},
            definitions={"Address": {"type": "object"}},
            version="2.0.0",
            documentation="Extended docs here.",
            schema_url="https://json-schema.org/draft/2020-12/schema",
        )
        assert sd.module_id == "db.query"
        assert sd.version == "2.0.0"
        assert sd.documentation == "Extended docs here."
        assert sd.schema_url == "https://json-schema.org/draft/2020-12/schema"
        assert "Address" in sd.definitions
        assert sd.error_schema == {"type": "object"}


class TestResolvedSchema:
    def test_creation(self) -> None:
        class DummyModel(BaseModel):
            pass

        rs = ResolvedSchema(
            json_schema={"type": "object"},
            model=DummyModel,
            module_id="test.mod",
            direction="input",
        )
        assert rs.json_schema == {"type": "object"}
        assert rs.model is DummyModel
        assert rs.module_id == "test.mod"
        assert rs.direction == "input"


class TestSchemaValidationErrorDetail:
    def test_all_fields(self) -> None:
        detail = SchemaValidationErrorDetail(
            path="/table",
            message="wrong type",
            constraint="type",
            expected="string",
            actual=42,
        )
        assert detail.path == "/table"
        assert detail.message == "wrong type"
        assert detail.constraint == "type"
        assert detail.expected == "string"
        assert detail.actual == 42

    def test_default_none_for_optional(self) -> None:
        detail = SchemaValidationErrorDetail(path="/x", message="error")
        assert detail.constraint is None
        assert detail.expected is None
        assert detail.actual is None


class TestSchemaValidationResult:
    def test_to_error_invalid_result(self) -> None:
        detail = SchemaValidationErrorDetail(
            path="/name",
            message="required field",
            constraint="required",
            expected="string",
            actual=None,
        )
        result = SchemaValidationResult(valid=False, errors=[detail])
        err = result.to_error()

        assert isinstance(err, SchemaValidationError)
        assert err.code == "SCHEMA_VALIDATION_ERROR"
        assert len(err.details["errors"]) == 1
        err_dict = err.details["errors"][0]
        assert err_dict["path"] == "/name"
        assert err_dict["message"] == "required field"
        assert err_dict["keyword"] == "required"
        assert set(err_dict) == {"path", "keyword", "message"}

    def test_to_error_raises_on_valid(self) -> None:
        result = SchemaValidationResult(valid=True)
        with pytest.raises(ValueError, match="Cannot convert valid result to error"):
            result.to_error()

    def test_valid_result_empty_errors(self) -> None:
        result = SchemaValidationResult(valid=True)
        assert result.valid is True
        assert result.errors == []

    def test_to_error_multiple_errors(self) -> None:
        details = [
            SchemaValidationErrorDetail(
                path="/name",
                message="required",
                constraint="required",
                expected="string",
                actual=None,
            ),
            SchemaValidationErrorDetail(
                path="/age",
                message="wrong type",
                constraint="type",
                expected="integer",
                actual="abc",
            ),
        ]
        result = SchemaValidationResult(valid=False, errors=details)
        err = result.to_error()
        assert len(err.details["errors"]) == 2
        assert err.details["errors"][0]["path"] == "/name"
        assert err.details["errors"][1]["path"] == "/age"

    def test_to_error_invalid_with_empty_errors(self) -> None:
        result = SchemaValidationResult(valid=False, errors=[])
        err = result.to_error()
        assert isinstance(err, SchemaValidationError)
        assert err.details["errors"] == []


class TestLLMExtensions:
    def test_default_values(self) -> None:
        ext = LLMExtensions()
        assert ext.llm_description is None
        assert ext.examples is None
        assert ext.sensitive is False
        assert ext.constraints is None
        assert ext.deprecated is None

    def test_all_fields(self) -> None:
        ext = LLMExtensions(
            llm_description="AI-friendly description",
            examples=[{"table": "users"}],
            sensitive=True,
            constraints="Must be a valid table name",
            deprecated={"since": "2.0", "replacement": "new.module"},
        )
        assert ext.llm_description == "AI-friendly description"
        assert ext.examples == [{"table": "users"}]
        assert ext.sensitive is True
        assert ext.constraints == "Must be a valid table name"
        assert ext.deprecated == {"since": "2.0", "replacement": "new.module"}
