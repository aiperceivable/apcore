"""Schema query and export functions for the registry system."""

from __future__ import annotations

import copy
import dataclasses
import json
from typing import TYPE_CHECKING, Any

import yaml

from apcore.errors import ModuleNotFoundError
from apcore.module import ModuleAnnotations, ModuleExample
from apcore.schema.exporter import SchemaExporter
from apcore.schema.strict import _strip_extensions, to_strict_schema
from apcore.schema.types import ExportProfile, SchemaDefinition

if TYPE_CHECKING:
    from apcore.registry.registry import Registry

__all__ = ["get_schema", "export_schema", "get_all_schemas", "export_all_schemas"]


def get_schema(registry: Registry, module_id: str) -> dict[str, Any] | None:
    """Build a structured schema dict from a registered module's attributes.

    Returns None if the module does not exist in the registry.
    """
    module = registry.get(module_id)
    if module is None:
        return None

    input_schema_dict = module.input_schema.model_json_schema()
    output_schema_dict = module.output_schema.model_json_schema()

    annotations = getattr(module, "annotations", None)
    annotations_dict = dataclasses.asdict(annotations) if isinstance(annotations, ModuleAnnotations) else None

    examples_raw = getattr(module, "examples", []) or []
    examples_list = [dataclasses.asdict(ex) for ex in examples_raw if isinstance(ex, ModuleExample)]

    return {
        "module_id": module_id,
        "name": getattr(module, "name", None),
        "description": getattr(module, "description", ""),
        "version": getattr(module, "version", "1.0.0"),
        "tags": list(getattr(module, "tags", []) or []),
        "input_schema": input_schema_dict,
        "output_schema": output_schema_dict,
        "annotations": annotations_dict,
        "examples": examples_list,
    }


def export_schema(
    registry: Registry,
    module_id: str,
    format: str = "json",
    strict: bool = False,
    compact: bool = False,
    profile: str | None = None,
) -> str:
    """Export a single module's schema as a JSON or YAML string.

    Raises ModuleNotFoundError if the module_id is not registered.
    """
    schema_dict = get_schema(registry, module_id)
    if schema_dict is None:
        raise ModuleNotFoundError(module_id=module_id)

    if profile is not None:
        return _export_with_profile(registry, module_id, schema_dict, profile, format)

    result = copy.deepcopy(schema_dict)

    if strict:
        result["input_schema"] = to_strict_schema(result["input_schema"])
        result["output_schema"] = to_strict_schema(result["output_schema"])
    elif compact:
        _apply_compact(result)

    return _serialize(result, format)


def get_all_schemas(registry: Registry) -> dict[str, dict[str, Any]]:
    """Collect schema dicts for all registered modules."""
    result: dict[str, dict[str, Any]] = {}
    for module_id in registry.module_ids:
        schema = get_schema(registry, module_id)
        if schema is not None:
            result[module_id] = schema
    return result


def export_all_schemas(
    registry: Registry,
    format: str = "json",
    strict: bool = False,
    compact: bool = False,
    profile: str | None = None,
) -> str:
    """Export all module schemas as a combined JSON or YAML string."""
    all_schemas = get_all_schemas(registry)

    if strict or compact:
        for module_id, schema in all_schemas.items():
            result = copy.deepcopy(schema)
            if strict:
                result["input_schema"] = to_strict_schema(result["input_schema"])
                result["output_schema"] = to_strict_schema(result["output_schema"])
            elif compact:
                _apply_compact(result)
            all_schemas[module_id] = result

    return _serialize(all_schemas, format)


# ----- Helpers -----


def _export_with_profile(
    registry: Registry,
    module_id: str,
    schema_dict: dict[str, Any],
    profile: str,
    format: str,
) -> str:
    """Export using SchemaExporter with a specific profile."""
    schema_def = SchemaDefinition(
        module_id=module_id,
        description=schema_dict["description"],
        input_schema=schema_dict["input_schema"],
        output_schema=schema_dict["output_schema"],
        definitions={},
    )
    module = registry.get(module_id)
    annotations = getattr(module, "annotations", None) if module else None
    examples = getattr(module, "examples", []) if module else []
    name = getattr(module, "name", None) if module else None

    exported = SchemaExporter().export(
        schema_def,
        profile=ExportProfile(profile),
        annotations=annotations,
        examples=examples,
        name=name,
    )
    return _serialize(exported, format)


def _apply_compact(schema_dict: dict[str, Any]) -> None:
    """Apply compact mode transformations in place."""
    # Truncate description
    desc = schema_dict.get("description", "")
    if desc:
        schema_dict["description"] = _truncate_description(desc)

    # Strip x-* keys from schemas
    _strip_extensions(schema_dict.get("input_schema", {}))
    _strip_extensions(schema_dict.get("output_schema", {}))

    # Remove documentation and examples
    schema_dict.pop("documentation", None)
    schema_dict.pop("examples", None)


def _truncate_description(description: str) -> str:
    """Truncate description to the first sentence."""
    dot_space = description.find(". ")
    newline = description.find("\n")

    candidates = []
    if dot_space >= 0:
        candidates.append(dot_space + 1)  # include the period
    if newline >= 0:
        candidates.append(newline)

    if candidates:
        cut = min(candidates)
        return description[:cut].rstrip()

    return description


def _serialize(data: Any, format: str) -> str:
    """Serialize data to JSON or YAML string."""
    if format == "yaml":
        return yaml.dump(data, default_flow_style=False)
    return json.dumps(data, indent=2)
