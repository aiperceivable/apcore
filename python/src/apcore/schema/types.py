"""Schema type definitions and data structures for the apcore schema system."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any

from pydantic import BaseModel

from apcore.errors import SchemaValidationError

__all__ = [
    "SchemaStrategy",
    "ExportProfile",
    "SchemaDefinition",
    "ResolvedSchema",
    "SchemaValidationErrorDetail",
    "SchemaValidationResult",
    "LLMExtensions",
]


class SchemaStrategy(str, Enum):
    """Controls how SchemaLoader resolves schemas."""

    YAML_FIRST = "yaml_first"
    NATIVE_FIRST = "native_first"
    YAML_ONLY = "yaml_only"


class ExportProfile(str, Enum):
    """Determines which export format to use."""

    MCP = "mcp"
    OPENAI = "openai"
    ANTHROPIC = "anthropic"
    GENERIC = "generic"


@dataclass
class SchemaDefinition:
    """Represents a parsed *.schema.yaml file before $ref resolution."""

    module_id: str
    description: str
    input_schema: dict[str, Any]
    output_schema: dict[str, Any]
    error_schema: dict[str, Any] | None = None
    definitions: dict[str, Any] = field(default_factory=dict)
    version: str = "1.0.0"
    documentation: str | None = None
    schema_url: str | None = None
    #: The ``*.schema.yaml`` file this definition was read from, when it came
    #: from one. Algorithm A05 step 4a (D-104) resolves a local ``#/…``
    #: reference against the FILE ROOT first, so the resolver has to be told
    #: which file that is. ``None`` for a definition built in memory, which
    #: keeps a local reference resolving within the schema node alone.
    source_path: Path | None = None


@dataclass
class ResolvedSchema:
    """A schema after all $ref references have been inlined, paired with its generated Pydantic model."""

    json_schema: dict[str, Any]
    model: type[BaseModel]
    module_id: str
    direction: str


@dataclass
class SchemaValidationErrorDetail:
    """One validation error in the PROTOCOL_SPEC section 4.14 format."""

    path: str
    message: str
    constraint: str | None = None
    expected: Any = None
    actual: Any = None


@dataclass
class SchemaValidationResult:
    """Aggregation of validation errors."""

    valid: bool
    errors: list[SchemaValidationErrorDetail] = field(default_factory=list)
    error_code: str | None = None

    def to_error(self) -> SchemaValidationError:
        """Convert this validation result into a SchemaValidationError exception."""
        if self.valid:
            raise ValueError("Cannot convert valid result to error")
        error_dicts = [
            {
                "path": e.path,
                "message": e.message,
                "keyword": e.constraint or "type",
            }
            for e in self.errors
        ]
        return SchemaValidationError(message="Schema validation failed", errors=error_dicts)


@dataclass
class LLMExtensions:
    """Extracted x-* extension fields from a schema property."""

    llm_description: str | None = None
    examples: list[Any] | None = None
    sensitive: bool = False
    constraints: str | None = None
    deprecated: dict[str, Any] | None = None
