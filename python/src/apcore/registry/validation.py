"""Module validation for the registry system."""

from __future__ import annotations

import inspect
from typing import Any

from pydantic import BaseModel

__all__ = ["validate_module"]


def validate_module(module_or_class: type | Any, *, programmatic: bool = False) -> list[str]:
    """Validate that a module class implements the required module interface.

    Discovery validates the class. Programmatic registration validates the
    supplied instance and permits explicitly permissive and JSON schemas.
    Returns a list of validation error strings. Empty list means valid.
    """
    subject = module_or_class if programmatic or inspect.isclass(module_or_class) else type(module_or_class)
    errors: list[str] = []

    missing = object()
    for attr in ("input_schema", "output_schema"):
        schema = getattr(subject, attr, missing)
        if inspect.isclass(schema) and issubclass(schema, BaseModel):
            continue
        if programmatic and (
            schema is None
            or isinstance(schema, dict)
            or (
                callable(getattr(schema, "model_json_schema", None))
                and callable(getattr(schema, "model_validate", None))
            )
        ):
            continue
        requirement = "a JSON Schema object or a BaseModel subclass" if programmatic else "a BaseModel subclass"
        errors.append(f"Missing or invalid {attr}: must be {requirement}")

    # Check description
    description = getattr(subject, "description", None)
    if not programmatic and (not description or not isinstance(description, str)):
        errors.append("Missing or empty description")

    # Check execute
    execute = getattr(subject, "execute", None)
    if execute is None or not callable(execute):
        errors.append("Missing execute method")

    return errors
