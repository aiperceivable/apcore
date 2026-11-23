"""apcore schema system -- public API.

Re-exports all public classes, functions, and types from schema submodules.

Example usage::

    from apcore.schema import SchemaLoader, SchemaValidator, ExportProfile
    from apcore.schema import to_strict_schema, merge_annotations
"""

from __future__ import annotations

from apcore.schema.annotations import (
    governance_union,
    merge_annotations,
    merge_examples,
    merge_metadata,
)
from apcore.schema.exporter import SchemaExporter
from apcore.schema.loader import SchemaLoader
from apcore.schema.openai_strict import (
    assert_openai_strict_compatible,
    detect_openai_strict_incompatibilities,
)
from apcore.schema.ref_resolver import RefResolver
from apcore.schema.strict import to_strict_schema
from apcore.schema.types import (
    ExportProfile,
    LLMExtensions,
    ResolvedSchema,
    SchemaDefinition,
    SchemaStrategy,
    SchemaValidationErrorDetail,
    SchemaValidationResult,
)
from apcore.schema.validator import SchemaValidator

__all__ = [
    "SchemaStrategy",
    "ExportProfile",
    "SchemaDefinition",
    "ResolvedSchema",
    "SchemaValidationResult",
    "SchemaValidationErrorDetail",
    "LLMExtensions",
    "RefResolver",
    "SchemaLoader",
    "SchemaValidator",
    "SchemaExporter",
    "to_strict_schema",
    "assert_openai_strict_compatible",
    "detect_openai_strict_incompatibilities",
    "governance_union",
    "merge_annotations",
    "merge_examples",
    "merge_metadata",
]
