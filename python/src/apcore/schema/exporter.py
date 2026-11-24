"""SchemaExporter — converts schemas to platform-specific export formats."""

from __future__ import annotations

import copy
from typing import Any

from apcore.module import ModuleAnnotations, ModuleExample
from apcore.schema.strict import (
    _apply_llm_descriptions,
    _strip_extensions,
    to_strict_schema,
)
from apcore.schema.types import ExportProfile, SchemaDefinition

__all__ = ["SchemaExporter"]


class SchemaExporter:
    """Stateless transformer that exports schemas in platform-specific formats."""

    def export(
        self,
        schema_def: SchemaDefinition,
        profile: ExportProfile,
        annotations: ModuleAnnotations | None = None,
        examples: list[ModuleExample] | None = None,
        name: str | None = None,
    ) -> dict[str, Any]:
        """Export schema in the specified profile format."""
        if profile == ExportProfile.MCP:
            return self.export_mcp(schema_def, annotations=annotations, name=name)
        if profile == ExportProfile.OPENAI:
            return self.export_openai(schema_def)
        if profile == ExportProfile.ANTHROPIC:
            return self.export_anthropic(schema_def, examples=examples)
        return self.export_generic(schema_def)

    def export_mcp(
        self,
        schema_def: SchemaDefinition,
        annotations: ModuleAnnotations | None = None,
        name: str | None = None,
    ) -> dict[str, Any]:
        """Export in MCP format. Preserves x-* fields and uses dot-notation IDs."""
        return {
            "name": name if name is not None else schema_def.module_id,
            "description": schema_def.description,
            "inputSchema": schema_def.input_schema,
            "annotations": {
                "readOnlyHint": annotations.readonly if annotations else False,
                "destructiveHint": annotations.destructive if annotations else False,
                "idempotentHint": annotations.idempotent if annotations else False,
                "openWorldHint": annotations.open_world if annotations else True,
            },
            "_meta": {
                "cacheable": annotations.cacheable if annotations else False,
                "cacheTtl": annotations.cache_ttl if annotations else 0,
                "cacheKeyFields": annotations.cache_key_fields if annotations else None,
                "paginated": annotations.paginated if annotations else False,
                "paginationStyle": annotations.pagination_style if annotations else "cursor",
                **({"requiresApproval": True} if annotations and annotations.requires_approval else {}),
                **({"streaming": True} if annotations and annotations.streaming else {}),
            },
        }

    def export_openai(self, schema_def: SchemaDefinition) -> dict[str, Any]:
        """Export in OpenAI function calling format with strict mode."""
        schema = copy.deepcopy(schema_def.input_schema)
        _apply_llm_descriptions(schema)
        strict_schema = to_strict_schema(schema)
        return {
            "type": "function",
            "function": {
                "name": schema_def.module_id.replace(".", "_"),
                "description": schema_def.description,
                "parameters": strict_schema,
                "strict": True,
            },
        }

    def export_anthropic(
        self,
        schema_def: SchemaDefinition,
        examples: list[ModuleExample] | None = None,
    ) -> dict[str, Any]:
        """Export in Anthropic tool use format. Strips extensions but no strict mode."""
        schema = copy.deepcopy(schema_def.input_schema)
        _apply_llm_descriptions(schema)
        _strip_extensions(schema, strip_defaults=False)
        result: dict[str, Any] = {
            "name": schema_def.module_id.replace(".", "_"),
            "description": schema_def.description,
            "input_schema": schema,
        }
        if examples:
            result["input_examples"] = [ex.inputs for ex in examples]
        return result

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
