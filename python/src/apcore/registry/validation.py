"""Module validation for the registry system."""

from __future__ import annotations

import inspect
from typing import Any

from pydantic import BaseModel

__all__ = ["validate_module"]


def validate_module(module_or_class: type | Any) -> list[str]:
    """Validate that a module class implements the required module interface.

    Accepts a class or an instance. If passed an instance, validates type(instance).
    Returns a list of validation error strings. Empty list means valid.
    """
    cls = module_or_class if inspect.isclass(module_or_class) else type(module_or_class)
    errors: list[str] = []

    # Check input_schema
    input_schema = getattr(cls, "input_schema", None)
    if input_schema is None:
        errors.append("Missing or invalid input_schema: must be a BaseModel subclass")
    elif not inspect.isclass(input_schema) or not issubclass(input_schema, BaseModel):
        errors.append("Missing or invalid input_schema: must be a BaseModel subclass")

    # Check output_schema
    output_schema = getattr(cls, "output_schema", None)
    if output_schema is None:
        errors.append("Missing or invalid output_schema: must be a BaseModel subclass")
    elif not inspect.isclass(output_schema) or not issubclass(output_schema, BaseModel):
        errors.append("Missing or invalid output_schema: must be a BaseModel subclass")

    # Check description
    description = getattr(cls, "description", None)
    if not description or not isinstance(description, str):
        errors.append("Missing or empty description")

    # Check execute
    execute = getattr(cls, "execute", None)
    if execute is None or not callable(execute):
        errors.append("Missing execute method")

    return errors
