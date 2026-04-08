# Schema System

## Overview

The Schema System provides complete schema loading, validation, `$ref` resolution, and export capabilities for structured module interfaces in apcore. It serves as the bridge between human-authored YAML schema definitions and the runtime model classes used by the executor for input/output validation. The system also supports exporting schemas to multiple LLM provider formats, enabling modules to describe their interfaces to external AI systems.

## Requirements

- Load module interface schemas from YAML files and convert them into validated, usable runtime representations.
- Resolve `$ref` references within schemas, including nested and cross-file references, with circular reference detection to prevent infinite loops.
- Dynamically generate runtime model classes from JSON Schema definitions, supporting the full range of JSON Schema composition keywords (`oneOf`, `anyOf`, `allOf`). Each SDK MAY use its idiomatic validation library (e.g., Pydantic for Python, Zod for TypeScript, serde for Rust).
- Validate arbitrary data against loaded schemas, providing clear and actionable error messages on failure.
- Export schemas to multiple target formats: MCP, OpenAI, Anthropic, and a generic format, enabling integration with various LLM tool-calling interfaces.
- Support LLM-specific extension fields (`x-*` fields) for annotating schemas with metadata such as sensitivity markers, display hints, and provider-specific instructions.
- Provide configurable schema resolution strategies to control how YAML-defined and native (code-defined) schemas interact.
- Cache loaded and generated schemas to avoid redundant parsing and model generation.

## Technical Design

### Components

#### SchemaLoader (Primary Entry Point)

The `SchemaLoader` is the main interface for loading schemas. It reads YAML schema files, resolves all `$ref` references, and generates runtime model classes from the resulting JSON Schema. It supports three resolution strategies:

- **yaml_first** (default): Attempts to load from YAML; falls back to native schema if no YAML file exists.
- **native_first**: Prefers the code-defined schema; falls back to YAML if no native schema is registered.
- **yaml_only**: Only loads from YAML; raises an error if no YAML file is found.

The loader maintains an internal cache keyed by schema path and strategy, so repeated loads of the same schema return the cached result without re-parsing.

These strategies are defined as the `SchemaStrategy` enum:

```python
from apcore import SchemaStrategy

class SchemaStrategy(str, Enum):
    YAML_FIRST = "yaml_first"
    NATIVE_FIRST = "native_first"
    YAML_ONLY = "yaml_only"
```

#### ExportProfile Enum

The `ExportProfile` enum specifies which export format to use:

```python
from apcore import ExportProfile

class ExportProfile(str, Enum):
    MCP = "mcp"
    OPENAI = "openai"
    ANTHROPIC = "anthropic"
    GENERIC = "generic"
```

Pass an `ExportProfile` value to `SchemaExporter.export()` or `Registry.export_schema(profile=...)` to control the output format.

#### RefResolver

The `RefResolver` handles `$ref` resolution within JSON Schema documents. It supports:

- Local references (`#/definitions/Foo`).
- Cross-file references (`other_schema.yaml#/definitions/Bar`).
- Recursive resolution with circular reference detection: a visited-set tracks resolution paths, and `max_depth=32` provides a hard limit to prevent runaway resolution.

When a `$ref` is resolved, the referenced schema fragment is inlined into the parent schema, producing a fully self-contained document suitable for runtime model generation.

#### SchemaValidator

The `SchemaValidator` validates data dictionaries against loaded schemas. It wraps the underlying model validation with additional handling for apcore-specific extensions (such as `x-sensitive` field detection). Validation errors are collected and returned as structured objects rather than raising exceptions, enabling batch validation reporting.

#### SchemaExporter

The `SchemaExporter` converts loaded schemas into target-specific formats:

- **MCP format**: Produces tool definitions compatible with the Model Context Protocol.
- **OpenAI format**: Produces function-calling tool definitions for OpenAI's API.
- **Anthropic format**: Produces tool definitions for Anthropic's tool-use API.
- **Generic format**: A provider-agnostic representation suitable for custom integrations.

Each export format strips or transforms `x-*` extension fields as appropriate for the target.

#### SchemaAnnotations

The `SchemaAnnotations` class manages field-level metadata extracted from `x-*` extension fields in the schema. Supported annotations include:

- `x-sensitive`: Marks a field as containing sensitive data (used by the executor's redaction logic).
- `x-display`: Hints for UI rendering.
- `x-llm-description`: Instructions or context intended for LLM consumption.

### Dynamic Model Generation

The `SchemaLoader` converts JSON Schema definitions into runtime model classes (using each SDK's idiomatic validation library — e.g., Pydantic for Python, Zod for TypeScript, serde for Rust). This process handles:

- Primitive types, arrays, objects, and nested objects.
- `oneOf` / `anyOf` / `allOf` composition via union types and model inheritance.
- Required vs. optional fields, default values, and constrained types (min/max, pattern, enum).
- Custom validators injected for fields with `x-*` annotations.

### Strict Mode

The `strict` module provides a strict validation mode that rejects any fields not explicitly defined in the schema. This is useful for modules that require exact input shapes and must reject unexpected data to prevent injection or misconfiguration.

### Data Flow

1. A YAML schema file is located on disk (typically adjacent to the module definition).
2. `SchemaLoader.load()` reads the YAML, parses it into a raw dictionary.
3. `RefResolver.resolve()` walks the dictionary, inlining all `$ref` targets and detecting cycles.
4. The resolved dictionary is converted into a runtime model class.
5. The model is cached and returned for use by the executor (validation) or exporter (format conversion).

## Dependencies

- The **Executor** depends on the Schema System for input/output validation (pipeline steps 6 and 9).
- The **Registry** uses the Schema System to load module schemas during discovery and to generate `ModuleDescriptor` objects.

??? info "Python SDK reference"
    The following tables are **not protocol requirements** — they document the Python SDK's source layout and runtime dependencies for implementers/users of `apcore-python`.

    **Source files:**

    | File | Lines | Purpose |
    |------|-------|---------|
    | `schema/loader.py` | 391 | Primary schema loading, YAML parsing, Pydantic model generation |
    | `schema/ref_resolver.py` | 206 | `$ref` resolution with circular reference detection (max_depth=32) |
    | `schema/validator.py` | 109 | Data validation against loaded schemas |
    | `schema/exporter.py` | 99 | Schema export to MCP, OpenAI, Anthropic, and generic formats |
    | `schema/types.py` | 109 | Shared type definitions and schema representation classes |
    | `schema/strict.py` | 105 | Strict validation mode implementation |
    | `schema/annotations.py` | 62 | Field-level `x-*` annotation extraction and management |

    **Runtime dependencies:**

    - `pydantic>=2.0` -- Runtime model generation and data validation.
    - `pyyaml>=6.0` -- YAML schema file parsing.

## Testing Strategy

- **Loader tests** verify that YAML schemas are correctly parsed, that resolution strategies (`yaml_first`, `native_first`, `yaml_only`) behave as documented, and that caching prevents redundant work.
- **RefResolver tests** cover local references, cross-file references, deeply nested references, and circular reference detection. Edge cases include self-referencing schemas and reference chains that reach the `max_depth=32` limit.
- **Validator tests** exercise success and failure paths for all supported JSON Schema types, composition keywords (`oneOf`, `anyOf`, `allOf`), and strict mode rejection of unknown fields.
- **Exporter tests** verify that each target format (MCP, OpenAI, Anthropic, generic) produces correct output and that `x-*` fields are appropriately handled per format.
- **Model generation tests** confirm that dynamically created models enforce constraints (required fields, types, patterns, enums) and that `x-sensitive` annotations flow through to the executor's redaction logic.
- Test naming follows the `test_<unit>_<behavior>` convention.
