"""Tests for the SchemaExporter."""

from __future__ import annotations

from typing import Any

from apcore.schema.exporter import SchemaExporter
from apcore.schema.types import SchemaDefinition


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
