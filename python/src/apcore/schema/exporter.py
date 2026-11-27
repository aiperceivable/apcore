"""SchemaExporter — exports protocol-neutral module schemas."""

from __future__ import annotations

from typing import Any

from apcore.schema.types import SchemaDefinition

__all__ = ["SchemaExporter"]


class SchemaExporter:
    """Stateless exporter for protocol-neutral module schemas."""

    def export_generic(
        self,
        schema_def: SchemaDefinition,
        sunset_date: str | None = None,
    ) -> dict[str, Any]:
        """Export in generic format with full schema, no modifications."""
        result: dict[str, Any] = {
            "module_id": schema_def.module_id,
            "description": schema_def.description,
            "input_schema": schema_def.input_schema,
            "output_schema": schema_def.output_schema,
            "definitions": schema_def.definitions,
        }
        if sunset_date is not None:
            result["sunset_date"] = sunset_date
        return result
