"""Tests for the SchemaExporter."""

from __future__ import annotations

from typing import Any

from apcore.module import ModuleAnnotations, ModuleExample
from apcore.schema.exporter import SchemaExporter
from apcore.schema.types import ExportProfile, SchemaDefinition


def _make_schema_def(
    module_id: str = "executor.email.send",
    description: str = "Send an email",
    input_schema: dict[str, Any] | None = None,
    output_schema: dict[str, Any] | None = None,
    definitions: dict[str, Any] | None = None,
) -> SchemaDefinition:
    return SchemaDefinition(
        module_id=module_id,
        description=description,
        input_schema=input_schema
        or {
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient",
                    "x-llm-description": "Full recipient email",
                },
                "cc": {"type": "array", "items": {"type": "string"}, "default": []},
            },
            "required": ["to"],
        },
        output_schema=output_schema or {"type": "object", "properties": {"status": {"type": "string"}}},
        definitions=definitions or {},
    )


def _make_annotations(**kwargs: Any) -> ModuleAnnotations:
    return ModuleAnnotations(**kwargs)


def _make_example(title: str = "Example 1", inputs: dict[str, Any] | None = None) -> ModuleExample:
    return ModuleExample(title=title, inputs=inputs or {"to": "user@example.com"})


# ===== export_generic() =====


class TestExportGeneric:
    def test_full_schema(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_generic(sd)
        assert result["module_id"] == "executor.email.send"
        assert result["description"] == "Send an email"
        assert result["input_schema"] == sd.input_schema
        assert result["output_schema"] == sd.output_schema
        assert result["definitions"] == sd.definitions

    def test_all_keys_present(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_generic(sd)
        assert set(result.keys()) == {
            "module_id",
            "description",
            "input_schema",
            "output_schema",
            "definitions",
        }


# ===== export_mcp() =====


class TestExportMcp:
    def test_preserves_x_fields(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_mcp(sd)
        assert "x-llm-description" in result["inputSchema"]["properties"]["to"]

    def test_id_uses_dots(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_mcp(sd)
        assert result["name"] == "executor.email.send"

    def test_annotations_mapped(self) -> None:
        sd = _make_schema_def()
        ann = _make_annotations(readonly=True, destructive=True, idempotent=True, open_world=False)
        exporter = SchemaExporter()
        result = exporter.export_mcp(sd, annotations=ann)
        assert result["annotations"]["readOnlyHint"] is True
        assert result["annotations"]["destructiveHint"] is True
        assert result["annotations"]["idempotentHint"] is True
        assert result["annotations"]["openWorldHint"] is False

    def test_default_annotations(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_mcp(sd)
        assert result["annotations"]["readOnlyHint"] is False
        assert result["annotations"]["destructiveHint"] is False
        assert result["annotations"]["idempotentHint"] is False
        assert result["annotations"]["openWorldHint"] is True
        assert "streaming" not in result["annotations"]
        assert "streaming" not in result["_meta"]

    def test_streaming_annotation(self) -> None:
        sd = _make_schema_def()
        ann = _make_annotations(streaming=True)
        exporter = SchemaExporter()
        result = exporter.export_mcp(sd, annotations=ann)
        assert result["_meta"]["streaming"] is True
        assert "streaming" not in result["annotations"]

    def test_custom_name(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_mcp(sd, name="custom_tool")
        assert result["name"] == "custom_tool"

    def test_meta_defaults(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_mcp(sd)
        meta = result["_meta"]
        assert meta["cacheable"] is False
        assert meta["cacheTtl"] == 0
        assert meta["cacheKeyFields"] is None
        assert meta["paginated"] is False
        assert meta["paginationStyle"] == "cursor"

    def test_meta_with_annotations(self) -> None:
        sd = _make_schema_def()
        ann = _make_annotations(
            cacheable=True,
            cache_ttl=600,
            cache_key_fields=["id", "name"],
            paginated=True,
            pagination_style="offset",
        )
        exporter = SchemaExporter()
        result = exporter.export_mcp(sd, annotations=ann)
        meta = result["_meta"]
        assert meta["cacheable"] is True
        assert meta["cacheTtl"] == 600
        assert meta["cacheKeyFields"] == ("id", "name")
        assert meta["paginated"] is True
        assert meta["paginationStyle"] == "offset"


# ===== export_openai() =====


class TestExportOpenai:
    def test_strict_mode_applied(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_openai(sd)
        assert result["function"]["parameters"]["additionalProperties"] is False

    def test_llm_description_replaces(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_openai(sd)
        assert result["function"]["parameters"]["properties"]["to"]["description"] == "Full recipient email"

    def test_x_fields_stripped(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_openai(sd)
        assert "x-llm-description" not in result["function"]["parameters"]["properties"]["to"]

    def test_defaults_stripped(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_openai(sd)
        assert "default" not in result["function"]["parameters"]["properties"].get("cc", {})

    def test_id_dots_to_underscores(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_openai(sd)
        assert result["function"]["name"] == "executor_email_send"

    def test_strict_true(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_openai(sd)
        assert result["function"]["strict"] is True

    def test_function_wrapper(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_openai(sd)
        assert result["type"] == "function"
        assert "function" in result


# ===== export_anthropic() =====


class TestExportAnthropic:
    def test_x_fields_stripped_no_strict(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_anthropic(sd)
        props = result["input_schema"]["properties"]
        assert "x-llm-description" not in props["to"]
        # Strict mode NOT applied — additionalProperties not forced
        assert result["input_schema"].get("additionalProperties") is None

    def test_llm_description_replaces(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_anthropic(sd)
        assert result["input_schema"]["properties"]["to"]["description"] == "Full recipient email"

    def test_id_dots_to_underscores(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_anthropic(sd)
        assert result["name"] == "executor_email_send"

    def test_examples_mapped(self) -> None:
        sd = _make_schema_def()
        ex1 = _make_example("Ex1", {"to": "a@b.com"})
        ex2 = _make_example("Ex2", {"to": "c@d.com"})
        exporter = SchemaExporter()
        result = exporter.export_anthropic(sd, examples=[ex1, ex2])
        assert result["input_examples"] == [{"to": "a@b.com"}, {"to": "c@d.com"}]

    def test_no_examples(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_anthropic(sd)
        assert "input_examples" not in result

    def test_uses_input_schema_key(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_anthropic(sd)
        assert "input_schema" in result
        assert "inputSchema" not in result

    def test_preserves_default_values(self) -> None:
        """export_anthropic() preserves default values in the output schema."""
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export_anthropic(sd)
        # The default schema has cc with default=[]
        assert result["input_schema"]["properties"]["cc"]["default"] == []


# ===== export() dispatch =====


class TestExportDispatch:
    def test_generic(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export(sd, ExportProfile.GENERIC)
        assert "module_id" in result

    def test_mcp(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export(sd, ExportProfile.MCP)
        assert "inputSchema" in result

    def test_openai(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export(sd, ExportProfile.OPENAI)
        assert result["type"] == "function"

    def test_anthropic(self) -> None:
        sd = _make_schema_def()
        exporter = SchemaExporter()
        result = exporter.export(sd, ExportProfile.ANTHROPIC)
        assert "input_schema" in result
