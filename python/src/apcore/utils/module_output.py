"""Project supported module return values onto their JSON object representation."""

from __future__ import annotations

import logging
from collections.abc import Mapping
from datetime import date, time
from typing import Any

from pydantic import AliasChoices, AliasPath, BaseModel, RootModel
from pydantic_core import PydanticSerializationError, to_jsonable_python

_logger = logging.getLogger(__name__)


def normalize_module_output(value: Any) -> Any:
    """Normalize model and mapping outputs, leaving shape checks to the caller."""
    if isinstance(value, (BaseModel, Mapping)):
        return _project_output_value(value, set())
    return value


def _project_output_value(value: Any, ancestors: set[int]) -> Any:
    if isinstance(value, BaseModel):
        try:
            output = value.model_dump(mode="json", by_alias=True)
        except PydanticSerializationError:
            # §5.15.1 preserves non-serializable members of an output object.
            _logger.warning("Module output contains a non-JSON value")
            output = _use_output_schema_names(value, value.model_dump(by_alias=True))
            return _project_output_value(output, ancestors)
        return _use_output_schema_names(value, output)
    if isinstance(value, (date, time)):
        return to_jsonable_python(value)
    if not isinstance(value, (Mapping, list, tuple)):
        return value

    identity = id(value)
    if identity in ancestors:
        return value
    ancestors.add(identity)
    try:
        if isinstance(value, Mapping):
            projected = {key: _project_output_value(item, ancestors) for key, item in value.items()}
            if type(value) is dict and all(projected[key] is item for key, item in value.items()):
                return value
            return projected

        elements = [_project_output_value(item, ancestors) for item in value]
        if all(projected is item for projected, item in zip(elements, value, strict=True)):
            return value
        return tuple(elements) if isinstance(value, tuple) else elements
    finally:
        ancestors.remove(identity)


def _use_output_schema_names(value: Any, output: Any) -> Any:
    """Keep Pydantic's serialized values under the names its validation schema publishes."""
    if isinstance(value, RootModel):
        return _use_output_schema_names(value.root, output)
    if isinstance(output, dict):
        if isinstance(value, BaseModel):
            return _use_model_field_names(value, output)
        if isinstance(value, Mapping):
            return {key: _use_output_schema_names(value.get(key), item) for key, item in output.items()}
    if isinstance(value, (list, tuple)) and isinstance(output, (list, tuple)) and len(value) == len(output):
        items = [_use_output_schema_names(source, item) for source, item in zip(value, output, strict=True)]
        return tuple(items) if isinstance(output, tuple) else items
    return output


def _use_model_field_names(value: BaseModel, output: dict[str, Any]) -> dict[str, Any]:
    model = type(value)
    if model.model_config.get("json_schema_mode_override") == "serialization":
        return output

    projected: dict[str, Any] = {}
    fields = {
        field.serialization_alias if field.serialization_alias is not None else name: (name, field.validation_alias)
        for name, field in model.model_fields.items()
        if name in vars(value)
    }
    for key, item in output.items():
        if key not in fields:
            projected[key] = item
            continue
        name, alias = fields[key]
        wire_name = _validation_property_name(name, alias, key)
        projected[wire_name] = _use_output_schema_names(vars(value)[name], item)
    return projected


def _validation_property_name(name: str, alias: str | AliasChoices | AliasPath | None, serialized_name: str) -> str:
    if alias is None:
        return name
    if isinstance(alias, str):
        return alias
    if isinstance(alias, AliasChoices):
        for path in alias.convert_to_aliases():
            if len(path) == 1 and isinstance(path[0], str):
                return path[0]
    # A path-shaped alias cannot be reconstructed by renaming a single key.
    return serialized_name
