# Schema System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §4 Schema Specification.


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

=== "Python"
    ```python
    from apcore import SchemaStrategy

    class SchemaStrategy(str, Enum):
        YAML_FIRST = "yaml_first"
        NATIVE_FIRST = "native_first"
        YAML_ONLY = "yaml_only"
    ```
=== "TypeScript"
    ```typescript
    import { SchemaStrategy } from "apcore-js/schema";

    // "yaml_first" | "native_first" | "yaml_only"
    const strategy: SchemaStrategy = "yaml_first";
    ```
=== "Rust"
    ```rust
    use apcore::schema::SchemaStrategy;

    let strategy = SchemaStrategy::YamlFirst;
    // SchemaStrategy::NativeFirst
    // SchemaStrategy::YamlOnly
    ```

#### ExportProfile Enum

The `ExportProfile` enum specifies which export format to use:

=== "Python"
    ```python
    from apcore import ExportProfile

    class ExportProfile(str, Enum):
        MCP = "mcp"
        OPENAI = "openai"
        ANTHROPIC = "anthropic"
        GENERIC = "generic"
    ```
=== "TypeScript"
    ```typescript
    import { ExportProfile } from "apcore-js/schema";

    // "mcp" | "openai" | "anthropic" | "generic"
    const profile: ExportProfile = "mcp";
    ```
=== "Rust"
    ```rust
    use apcore::schema::ExportProfile;

    let profile = ExportProfile::Mcp;
    // ExportProfile::OpenAi
    // ExportProfile::Anthropic
    // ExportProfile::Generic
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

## Usage

=== "Python"
    ```python
    from apcore.schema import SchemaLoader, SchemaExporter, SchemaValidator, SchemaStrategy, ExportProfile

    # Load a schema from YAML
    loader = SchemaLoader(strategy=SchemaStrategy.YAML_FIRST)
    schema = loader.load("schemas/email_send.yaml")

    # Validate data
    validator = SchemaValidator()
    errors = validator.validate(schema, {"to": "alice@example.com", "subject": "Hello"})
    if errors:
        print(f"Validation failed: {errors}")

    # Export to MCP tool format
    exporter = SchemaExporter()
    mcp_tool = exporter.export(schema, profile=ExportProfile.MCP)
    print(mcp_tool)  # {"name": "...", "description": "...", "inputSchema": {...}}
    ```
=== "TypeScript"
    ```typescript
    import { SchemaLoader, SchemaExporter, SchemaValidator } from "apcore-js/schema";
    import type { SchemaStrategy, ExportProfile } from "apcore-js/schema";

    // Load a schema from YAML
    const loader = new SchemaLoader({ strategy: "yaml_first" });
    const schema = await loader.load("schemas/email_send.yaml");

    // Validate data
    const validator = new SchemaValidator();
    const errors = validator.validate(schema, { to: "alice@example.com", subject: "Hello" });
    if (errors.length > 0) {
        console.error("Validation failed:", errors);
    }

    // Export to OpenAI function format
    const exporter = new SchemaExporter();
    const openaiTool = exporter.export(schema, { profile: "openai" });
    console.log(openaiTool);
    ```
=== "Rust"
    ```rust
    use apcore::schema::{SchemaLoader, SchemaExporter, SchemaValidator, SchemaStrategy, ExportProfile};

    // Load a schema from YAML
    let loader = SchemaLoader::new(SchemaStrategy::YamlFirst);
    let schema = loader.load("schemas/email_send.yaml")?;

    // Validate data
    let validator = SchemaValidator::new();
    let errors = validator.validate(&schema, &serde_json::json!({
        "to": "alice@example.com",
        "subject": "Hello"
    }))?;
    if !errors.is_empty() {
        eprintln!("Validation failed: {:?}", errors);
    }

    // Export to Anthropic tool format
    let exporter = SchemaExporter::new();
    let anthropic_tool = exporter.export(&schema, ExportProfile::Anthropic)?;
    println!("{}", anthropic_tool);
    ```

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

## Contract: Schema.validate

### Inputs
- `data` (dict/object/Value, required) — data to validate
- `schema` (dict/object/Value, required) — JSON Schema Draft 2020-12 schema object

### Errors
- None — `Schema.validate` does not raise. Failure is reported via the returned result object (D10-012). For a raise-on-failure variant, use `Schema.validate_input` / `Schema.validate_output` (Python+TS+Rust) or `validate_or_error` (Rust).

### Returns
- On success: `SchemaValidationResult { valid: true, errors: [] }` (Python+TS) or `ValidationResult { valid: true, errors: [] }` (Rust)
- On failure: same result-object shape with `valid: false` and an `errors` array carrying per-failure detail objects.

### Properties
- async: false
- thread_safe: true
- pure: true (no side effects; deterministic given same data and schema)

### Cross-language note

All three SDKs return a result object rather than raising. Spec versions ≤ 0.20.0 incorrectly declared "void/None/() — raises on failure"; that text was implementation-incorrect across all SDKs and has been amended (D10-012). For the raise-on-failure semantic, use the dedicated `validate_input` / `validate_output` (or Rust `validate_or_error`) entry points which raise `SchemaValidationError(code=SCHEMA_VALIDATION_FAILED)` on a non-empty `errors` array.
- idempotent: true

## Contract: RefResolver — `$ref` resolution

> **Spec amendment (D10-015).** Earlier drafts attributed `resolve_refs`
> to the `Schema` class with a `(schema, base_uri)` signature. No SDK
> exposes that surface. The actual `$ref` resolution lives on
> `RefResolver` in each SDK with language-idiomatic shapes.

### SDK surfaces

| SDK | Class / location | Method | Inputs |
|---|---|---|---|
| Python | `apcore.schema.ref_resolver.RefResolver` (`apcore-python/src/apcore/schema/ref_resolver.py:46`) | `resolve_ref(ref, schema_root, ...)` | single `$ref` string + the schema document holding it |
| TypeScript | `apcore-js/schema/ref-resolver.RefResolver` (`apcore-typescript/src/schema/ref-resolver.ts:39`) | `resolveRef(ref, schemaRoot, ...)` | same shape; camelCase |
| Rust | `apcore::schema::ref_resolver::RefResolver` (`apcore-rust/src/schema/ref_resolver.rs:50`) | `resolve(schema)` | the full schema (no separate base_uri argument; bases are derived from `$id` claims encountered during traversal) |

### Errors

- `SchemaCircularRefError(code=SCHEMA_CIRCULAR_REF)` — a `$ref` cycle was detected
- `SchemaRefNotFoundError(code=SCHEMA_REF_NOT_FOUND)` — a referenced schema cannot be resolved

### Returns

- On success: a schema with the requested `$ref` resolved inline. Python+TypeScript variants resolve a single `$ref` per call (caller iterates); Rust resolves all `$ref` entries in one traversal.

### Properties

- async: false
- thread_safe: true
- pure: true
- idempotent: true

---

## Schema System Hardening (Issue #44)

This section documents five normative hardening requirements introduced in Issue #44. Each requirement addresses a known behavioral gap across the Python, TypeScript, and Rust SDKs.

### 1. Union Type Standardization

**Problem:** Python and TypeScript currently short-circuit union evaluation — they test only the first branch of `anyOf`/`oneOf` and return success if it matches, without evaluating remaining branches. This causes `oneOf` to behave identically to `anyOf`, masking ambiguous schemas where multiple branches match.

**Normative requirements:**

Implementations MUST evaluate ALL branches of `anyOf`/`oneOf` before returning a result. An input MUST be accepted for `anyOf` if it matches at least one branch. An input MUST be accepted for `oneOf` if it matches exactly one branch. Implementations MUST NOT return success after testing only the first branch.

For `oneOf`: if more than one branch matches, implementations MUST treat this as a validation error.

!!! warning "Breaking change for existing modules using `oneOf`"
    Schemas that relied on short-circuit `oneOf` evaluation — where multiple branches could match the same input — will begin failing validation after this change is applied. Authors MUST audit `oneOf` schemas to ensure branches are mutually exclusive.

=== "Python"
    ```python
    from typing import Annotated, Union
    from pydantic import BaseModel, Field, model_validator

    class CircleShape(BaseModel):
        kind: str = "circle"
        radius: float

    class RectShape(BaseModel):
        kind: str = "rect"
        width: float
        height: float

    # Pydantic discriminated union — evaluated exhaustively at model_validate time
    Shape = Annotated[
        Union[CircleShape, RectShape],
        Field(discriminator="kind"),
    ]

    class DrawCommand(BaseModel):
        shape: Shape

    # anyOf: succeeds if any branch matches
    result = DrawCommand.model_validate({"shape": {"kind": "circle", "radius": 5.0}})

    # oneOf: Pydantic discriminated union enforces mutual exclusivity by key
    # For non-discriminated oneOf, use a custom model_validator to assert exactly one branch matched
    from pydantic import model_validator as mv

    def _try_validate(model, data):
        try:
            model.model_validate(data)
            return True
        except Exception:
            return False

    class StrictOneOf(BaseModel):
        value: Union[CircleShape, RectShape]

        @mv(mode="before")
        @classmethod
        def enforce_one_of(cls, data: dict) -> dict:
            matched = sum(
                _try_validate(m, data.get("value", {}))
                for m in (CircleShape, RectShape)
            )
            if matched != 1:
                raise ValueError(f"oneOf: expected exactly 1 match, got {matched}")
            return data
    ```

!!! info "Pydantic discriminated unions"
    When a `discriminator` field is available, prefer `Field(discriminator=...)` — Pydantic validates only the correct branch and raises clearly if the discriminator value is missing or unrecognized. For schemas without a discriminator, implement the exhaustive check shown above.

=== "TypeScript"
    ```typescript
    import { Type, Static, TUnion } from "@sinclair/typebox";
    import { Value } from "@sinclair/typebox/value";

    const CircleShape = Type.Object({ kind: Type.Literal("circle"), radius: Type.Number() });
    const RectShape = Type.Object({ kind: Type.Literal("rect"), width: Type.Number(), height: Type.Number() });

    // anyOf: Value.Check returns true if any branch matches
    const AnyOfShape = Type.Union([CircleShape, RectShape]);
    const anyOfResult = Value.Check(AnyOfShape, { kind: "circle", radius: 5.0 }); // true

    // oneOf: evaluate all branches and assert exactly one matches
    function validateOneOf<T>(schemas: TUnion["anyOf"], data: unknown): T {
        const matches = schemas.filter((s) => Value.Check(s, data));
        if (matches.length !== 1) {
            throw new Error(`oneOf: expected exactly 1 match, got ${matches.length}`);
        }
        return data as T;
    }

    const oneOfResult = validateOneOf([CircleShape, RectShape], { kind: "rect", width: 10, height: 20 });
    ```

=== "Rust"
    ```rust
    use jsonschema::{JSONSchema, Draft};
    use serde_json::{json, Value};

    fn validate_any_of(branches: &[Value], data: &Value) -> bool {
        branches.iter().any(|branch| {
            let compiled = JSONSchema::options()
                .with_draft(Draft::Draft202012)
                .compile(branch)
                .expect("invalid branch schema");
            compiled.is_valid(data)
        })
    }

    fn validate_one_of(branches: &[Value], data: &Value) -> Result<(), String> {
        let matched: usize = branches
            .iter()
            .filter(|branch| {
                let compiled = JSONSchema::options()
                    .with_draft(Draft::Draft202012)
                    .compile(branch)
                    .expect("invalid branch schema");
                compiled.is_valid(data)
            })
            .count();
        match matched {
            1 => Ok(()),
            n => Err(format!("oneOf: expected exactly 1 match, got {n}")),
        }
    }

    fn main() {
        let circle_schema = json!({
            "type": "object",
            "properties": { "kind": { "const": "circle" }, "radius": { "type": "number" } },
            "required": ["kind", "radius"]
        });
        let rect_schema = json!({
            "type": "object",
            "properties": { "kind": { "const": "rect" }, "width": { "type": "number" }, "height": { "type": "number" } },
            "required": ["kind", "width", "height"]
        });
        let branches = vec![circle_schema, rect_schema];
        let data = json!({ "kind": "circle", "radius": 5.0 });

        assert!(validate_any_of(&branches, &data));
        assert!(validate_one_of(&branches, &data).is_ok());
    }
    ```

---

### 2. Recursive Schema Support

**Problem:** Schemas that reference themselves (e.g., tree nodes, nested comment threads) cause infinite loops in the current `$ref` resolver because it eagerly inlines every `$ref` it encounters, including self-references, until stack overflow.

**Normative requirements:**

Implementations MUST support self-referencing schemas via lazy resolution. When a `$ref` resolves to the schema's own `$id`, implementations MUST replace the reference with a lazy (deferred) reference rather than inlining the schema body again. Implementations MUST NOT eagerly inline a `$ref` that would re-enter the currently-resolving schema.

The canonical recursive schema example used across all SDK conformance tests is:

```json
{
  "$id": "TreeNode",
  "type": "object",
  "properties": {
    "value": { "type": "string" },
    "children": {
      "type": "array",
      "items": { "$ref": "TreeNode" }
    }
  },
  "required": ["value"]
}
```

=== "Python"
    ```python
    from __future__ import annotations
    from typing import Optional
    from pydantic import BaseModel

    class TreeNode(BaseModel):
        value: str
        children: Optional[list[TreeNode]] = None

    # model_rebuild() resolves the forward reference introduced by `from __future__ import annotations`
    TreeNode.model_rebuild()

    root = TreeNode(
        value="root",
        children=[
            TreeNode(value="child1", children=[TreeNode(value="grandchild")]),
            TreeNode(value="child2"),
        ],
    )
    assert root.children[0].children[0].value == "grandchild"
    ```

!!! info "Why `model_rebuild()` is required"
    Pydantic defers resolution of forward references until `model_rebuild()` is called. Without it, the `TreeNode` type annotation inside `list[TreeNode]` is an unresolved string at class-creation time, and validation will fail with a `PydanticUserError`.

=== "TypeScript"
    ```typescript
    import { Type, Static } from "@sinclair/typebox";
    import { Value } from "@sinclair/typebox/value";

    // TypeBox Recursive() wraps the schema in a self-referential $ref
    const TreeNode = Type.Recursive((self) =>
        Type.Object({
            value: Type.String(),
            children: Type.Optional(Type.Array(self)),
        }),
        { $id: "TreeNode" }
    );

    type TreeNode = Static<typeof TreeNode>;

    const root: TreeNode = {
        value: "root",
        children: [
            { value: "child1", children: [{ value: "grandchild" }] },
            { value: "child2" },
        ],
    };

    const valid = Value.Check(TreeNode, root);
    console.assert(valid === true);
    ```

=== "Rust"
    ```rust
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct TreeNode {
        value: String,
        // Box<T> breaks the infinite-size cycle; Option makes the field optional
        #[serde(skip_serializing_if = "Option::is_none")]
        children: Option<Vec<Box<TreeNode>>>,
    }

    fn main() {
        let root = TreeNode {
            value: "root".into(),
            children: Some(vec![
                Box::new(TreeNode {
                    value: "child1".into(),
                    children: Some(vec![Box::new(TreeNode {
                        value: "grandchild".into(),
                        children: None,
                    })]),
                }),
                Box::new(TreeNode { value: "child2".into(), children: None }),
            ]),
        };

        let json = serde_json::to_string(&root).unwrap();
        let parsed: TreeNode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.children.as_ref().unwrap()[0].value, "child1");
    }
    ```

---

### 3. Rust Validator Enhancement

**Problem:** The Rust validator performs only basic type checking. It does not enforce composition keywords (`allOf`, `anyOf`, `oneOf`, `not`) or numerical and string constraints (`minimum`, `maximum`, `minLength`, `maxLength`, `pattern`). This creates a cross-language behavioral gap where inputs rejected by Python/TypeScript validators are accepted by the Rust validator.

**Normative requirements:**

The Rust validator MUST support all constraint types that the Python and TypeScript validators support. The Rust validator MUST reject data that violates `minimum`, `maximum`, `minLength`, `maxLength`, or `pattern` constraints.

**Recommended approach:** Replace the hand-written validator with the `jsonschema` crate (formerly `jsonschema-rs`), which supports JSON Schema Draft 2020-12 natively.

**Alternative:** Incrementally extend the existing hand-written logic. The table below compares both approaches:

| Dimension | `jsonschema` crate | Incremental hand-written |
|---|---|---|
| Implementation cost | Low — swap validation call site | High — reimplement each keyword |
| Draft 2020-12 coverage | Complete | Partial (only what is written) |
| Maintenance burden | Low — upstream tracks spec changes | High — every new keyword requires a PR |
| Performance | Comparable; crate is optimized | Potentially faster for simple schemas |
| `allOf`/`anyOf`/`oneOf`/`not` | Supported out of the box | Must be hand-written |
| Numerical/string constraints | Supported out of the box | Must be hand-written |

!!! info "Recommended migration path"
    Add `jsonschema = "0.22"` (or latest) to `Cargo.toml` and route all `SchemaValidator::validate` calls through `JSONSchema::compile` + `compiled.validate(data)`. The existing hand-written type-check logic can be removed once the crate is wired in and all conformance fixtures pass.

=== "Python"
    ```python
    from pydantic import BaseModel, Field, ValidationError

    class RangedValue(BaseModel):
        count: int = Field(ge=1, le=100)
        label: str = Field(min_length=1, max_length=50, pattern=r"^[a-z_]+$")

    try:
        RangedValue(count=200, label="INVALID LABEL!")
    except ValidationError as exc:
        print(exc)
        # count: Input should be less than or equal to 100
        # label: String should match pattern '^[a-z_]+'
    ```

=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { Value } from "@sinclair/typebox/value";

    const RangedValue = Type.Object({
        count: Type.Integer({ minimum: 1, maximum: 100 }),
        label: Type.String({ minLength: 1, maxLength: 50, pattern: "^[a-z_]+$" }),
    });

    const errors = [...Value.Errors(RangedValue, { count: 200, label: "INVALID LABEL!" })];
    console.log(errors);
    // [ { path: '/count', message: 'Expected integer to be less than or equal to 100' }, ... ]
    ```

=== "Rust"
    ```rust
    use jsonschema::{JSONSchema, Draft};
    use serde_json::json;

    fn main() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "count": { "type": "integer", "minimum": 1, "maximum": 100 },
                "label": { "type": "string", "minLength": 1, "maxLength": 50, "pattern": "^[a-z_]+$" }
            },
            "required": ["count", "label"]
        });

        let compiled = JSONSchema::options()
            .with_draft(Draft::Draft202012)
            .compile(&schema)
            .expect("invalid schema");

        let data = json!({ "count": 200, "label": "INVALID LABEL!" });
        let result = compiled.validate(&data);

        if let Err(errors) = result {
            for error in errors {
                eprintln!("Validation error: {} at {}", error, error.instance_path);
            }
        }
    }
    ```

---

### 4. Semantic Format Mapping

**Problem:** JSON Schema `format` keywords (`date-time`, `email`, `uri`, etc.) are currently treated as annotations — they are passed through without enforcement. This means a field declared `format: date-time` accepts any string, even one that is not a valid ISO 8601 timestamp.

**Normative requirements:**

Implementations SHOULD map `format: date-time` to the language-native datetime type: `datetime.datetime` in Python, `Date` in TypeScript, `chrono::DateTime<Utc>` in Rust. Implementations MAY map other `format` values to native types. Unmapped `format` values SHOULD be preserved as `string` without raising an error.

!!! warning "Format enforcement is opt-in (SHOULD, not MUST)"
    Enforcing `format` as a type constraint is a breaking change for any module that stores non-conformant strings in a `format`-annotated field. Enable format enforcement incrementally and validate existing module inputs before deploying.

**Canonical format mapping table:**

| JSON Schema `format` | Python | TypeScript | Rust |
|---|---|---|---|
| `date-time` | `datetime.datetime` | `Date` | `chrono::DateTime<Utc>` |
| `date` | `datetime.date` | `string` (ISO 8601 date) | `chrono::NaiveDate` |
| `time` | `datetime.time` | `string` (ISO 8601 time) | `chrono::NaiveTime` |
| `email` | `pydantic.EmailStr` | `string` (format-validated) | `String` (regex-validated) |
| `uri` | `pydantic.AnyUrl` | `URL` | `url::Url` |
| `uuid` | `uuid.UUID` | `string` (UUID regex) | `uuid::Uuid` |
| `ipv4` | `IPv4Address` | `string` (format-validated) | `std::net::Ipv4Addr` |
| `ipv6` | `IPv6Address` | `string` (format-validated) | `std::net::Ipv6Addr` |

=== "Python"
    ```python
    from datetime import datetime
    from ipaddress import IPv4Address, IPv6Address
    from uuid import UUID
    from pydantic import BaseModel, AnyUrl, EmailStr

    class EventRecord(BaseModel):
        event_id: UUID
        occurred_at: datetime       # format: date-time
        source_ip: IPv4Address      # format: ipv4
        callback_url: AnyUrl        # format: uri
        contact: EmailStr           # format: email

    record = EventRecord(
        event_id="550e8400-e29b-41d4-a716-446655440000",
        occurred_at="2024-01-15T09:30:00Z",
        source_ip="192.168.1.1",
        callback_url="https://example.com/hook",
        contact="user@example.com",
    )
    print(record.occurred_at)  # 2024-01-15 09:30:00+00:00 (datetime object)
    ```

=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { Value } from "@sinclair/typebox/value";

    const EventRecord = Type.Object({
        event_id: Type.String({ format: "uuid" }),
        occurred_at: Type.String({ format: "date-time" }),
        source_ip: Type.String({ format: "ipv4" }),
        callback_url: Type.String({ format: "uri" }),
        contact: Type.String({ format: "email" }),
    });

    // TypeBox format validation requires the `@sinclair/typebox/format` registry
    import { Format } from "@sinclair/typebox/format";
    Format.Set("date-time", (v) => !isNaN(new Date(v).getTime()));
    Format.Set("uuid", (v) => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(v));

    const valid = Value.Check(EventRecord, {
        event_id: "550e8400-e29b-41d4-a716-446655440000",
        occurred_at: "2024-01-15T09:30:00Z",
        source_ip: "192.168.1.1",
        callback_url: "https://example.com/hook",
        contact: "user@example.com",
    });
    console.assert(valid === true);
    ```

=== "Rust"
    ```rust
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Serialize};
    use std::net::Ipv4Addr;
    use url::Url;
    use uuid::Uuid;

    #[derive(Debug, Serialize, Deserialize)]
    struct EventRecord {
        event_id: Uuid,
        occurred_at: DateTime<Utc>,
        source_ip: Ipv4Addr,
        callback_url: Url,
        contact: String, // email validated separately via regex or lettre
    }

    fn main() {
        let json = r#"{
            "event_id": "550e8400-e29b-41d4-a716-446655440000",
            "occurred_at": "2024-01-15T09:30:00Z",
            "source_ip": "192.168.1.1",
            "callback_url": "https://example.com/hook",
            "contact": "user@example.com"
        }"#;

        let record: EventRecord = serde_json::from_str(json).unwrap();
        println!("{}", record.occurred_at); // 2024-01-15 09:30:00 UTC
    }
    ```

---

### 5. Global Schema Cache by Content Hash

**Problem:** The current schema cache is keyed by `(path, strategy)`. This means the same schema content loaded from two different file paths is cached twice and occupies duplicate memory. It also means that when schema file content changes without a path change (e.g., in-place edits during development), the stale cached model continues to be returned.

**Design:** Replace the single-level path cache with a two-level content-addressable cache:

1. **Path index** (`path, strategy` → `sha256_hex`): maps a load request to the content hash of the schema it resolved to.
2. **Content cache** (`sha256_hex` → `model`): stores the compiled model, keyed by the SHA-256 of the canonical JSON serialization of the resolved schema dict.

The canonical JSON form is defined as: `json.dumps(schema, sort_keys=True, separators=(',', ':'))` (Python), `JSON.stringify(sortedKeys(schema))` (TypeScript), `serde_json::to_string` with sorted keys (Rust).

**Normative requirements:**

Implementations MUST deduplicate identical schema content. When two schema paths resolve to the same content hash, implementations MUST return the same cached model object. Implementations MUST NOT generate two separate model objects for schemas that are byte-for-byte identical after canonical JSON serialization.

=== "Python"
    ```python
    import hashlib
    import json
    from typing import Any

    _path_index: dict[tuple[str, str], str] = {}    # (path, strategy) -> sha256_hex
    _content_cache: dict[str, Any] = {}             # sha256_hex -> compiled model

    def _content_hash(schema: dict) -> str:
        canonical = json.dumps(schema, sort_keys=True, separators=(",", ":"))
        return hashlib.sha256(canonical.encode()).hexdigest()

    def load_with_content_cache(path: str, strategy: str, resolve_fn, compile_fn):
        cache_key = (path, strategy)
        if cache_key in _path_index:
            return _content_cache[_path_index[cache_key]]

        raw_schema = resolve_fn(path, strategy)
        digest = _content_hash(raw_schema)

        if digest not in _content_cache:
            _content_cache[digest] = compile_fn(raw_schema)

        _path_index[cache_key] = digest
        return _content_cache[digest]
    ```

=== "TypeScript"
    ```typescript
    import { createHash } from "crypto";

    const pathIndex = new Map<string, string>();     // `${path}:${strategy}` -> sha256hex
    const contentCache = new Map<string, unknown>(); // sha256hex -> compiled model

    function sortedKeysStringify(obj: unknown): string {
        if (obj === null || typeof obj !== "object") return JSON.stringify(obj);
        if (Array.isArray(obj)) return `[${obj.map(sortedKeysStringify).join(",")}]`;
        const sorted = Object.keys(obj as object).sort();
        const pairs = sorted.map((k) => `${JSON.stringify(k)}:${sortedKeysStringify((obj as Record<string, unknown>)[k])}`);
        return `{${pairs.join(",")}}`;
    }

    function contentHash(schema: unknown): string {
        return createHash("sha256").update(sortedKeysStringify(schema)).digest("hex");
    }

    function loadWithContentCache(
        path: string,
        strategy: string,
        resolveFn: (p: string, s: string) => unknown,
        compileFn: (schema: unknown) => unknown,
    ): unknown {
        const pathKey = `${path}:${strategy}`;
        if (pathIndex.has(pathKey)) {
            return contentCache.get(pathIndex.get(pathKey)!)!;
        }
        const rawSchema = resolveFn(path, strategy);
        const digest = contentHash(rawSchema);
        if (!contentCache.has(digest)) {
            contentCache.set(digest, compileFn(rawSchema));
        }
        pathIndex.set(pathKey, digest);
        return contentCache.get(digest)!;
    }
    ```

=== "Rust"
    ```rust
    use sha2::{Digest, Sha256};
    use serde_json::Value;
    use std::collections::HashMap;

    struct SchemaCache {
        path_index: HashMap<(String, String), String>,   // (path, strategy) -> sha256_hex
        content_cache: HashMap<String, Value>,           // sha256_hex -> compiled schema/model
    }

    impl SchemaCache {
        fn new() -> Self {
            Self {
                path_index: HashMap::new(),
                content_cache: HashMap::new(),
            }
        }

        fn content_hash(schema: &Value) -> String {
            // serde_json serializes object keys in insertion order;
            // sort them for a stable canonical form
            let canonical = sort_keys_serialize(schema);
            let mut hasher = Sha256::new();
            hasher.update(canonical.as_bytes());
            format!("{:x}", hasher.finalize())
        }

        fn load(
            &mut self,
            path: &str,
            strategy: &str,
            resolve: impl Fn(&str, &str) -> Value,
            compile: impl Fn(Value) -> Value,
        ) -> &Value {
            let path_key = (path.to_string(), strategy.to_string());
            if let Some(digest) = self.path_index.get(&path_key) {
                return self.content_cache.get(digest).unwrap();
            }
            let raw_schema = resolve(path, strategy);
            let digest = Self::content_hash(&raw_schema);
            if !self.content_cache.contains_key(&digest) {
                self.content_cache.insert(digest.clone(), compile(raw_schema));
            }
            self.path_index.insert(path_key, digest.clone());
            self.content_cache.get(&digest).unwrap()
        }
    }

    fn sort_keys_serialize(value: &Value) -> String {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let pairs: Vec<String> = keys
                    .iter()
                    .map(|k| format!("\"{}\":{}", k, sort_keys_serialize(&map[*k])))
                    .collect();
                format!("{{{}}}", pairs.join(","))
            }
            _ => value.to_string(),
        }
    }
    ```

---

### Conformance Fixtures

Cross-language behavioral verification for the hardening requirements above is shipped across five fixtures in `conformance/fixtures/`:

- `schema_hardening_union.json` — `anyOf`/`oneOf` branch evaluation
- `schema_hardening_recursive.json` — `$ref` cycles up to `max_depth`
- `schema_hardening_constraints.json` — numeric and string constraint paths
- `schema_hardening_formats.json` — `format` keyword annotation semantics
- `schema_hardening_cache.json` — `content_hash` canonical serialization

The case excerpts below illustrate the canonical shape; refer to the JSON files for the full case lists.

**`union_type_all_branches_evaluated`** — validates that `anyOf` accepts a matching branch and that `oneOf` rejects inputs where multiple branches match:

```json
{
  "id": "union_type_all_branches_evaluated",
  "description": "anyOf accepts first-branch match; oneOf rejects multi-branch match",
  "schema": {
    "oneOf": [
      { "type": "object", "properties": { "kind": { "const": "a" } }, "required": ["kind"] },
      { "type": "object", "properties": { "kind": { "const": "b" } }, "required": ["kind"] }
    ]
  },
  "test_cases": [
    { "id": "one_of_single_match", "input": { "kind": "a" }, "expected": true },
    { "id": "one_of_no_match", "input": { "kind": "c" }, "expected": false },
    { "id": "any_of_first_branch", "input": { "kind": "a" }, "schema_keyword": "anyOf", "expected": true },
    { "id": "any_of_second_branch", "input": { "kind": "b" }, "schema_keyword": "anyOf", "expected": true }
  ]
}
```

**`recursive_schema_tree_node`** — validates tree node recursion up to depth 5:

```json
{
  "id": "recursive_schema_tree_node",
  "description": "Self-referencing TreeNode schema validates nested structures up to depth 5",
  "schema": {
    "$id": "TreeNode",
    "type": "object",
    "properties": {
      "value": { "type": "string" },
      "children": { "type": "array", "items": { "$ref": "TreeNode" } }
    },
    "required": ["value"]
  },
  "test_cases": [
    { "id": "depth_1", "input": { "value": "root" }, "expected": true },
    { "id": "depth_2", "input": { "value": "root", "children": [{ "value": "child" }] }, "expected": true },
    { "id": "depth_5", "input": { "value": "a", "children": [{ "value": "b", "children": [{ "value": "c", "children": [{ "value": "d", "children": [{ "value": "e" }] }] }] }] }, "expected": true },
    { "id": "missing_value", "input": { "children": [] }, "expected": false }
  ]
}
```

**`rust_validator_constraints`** — validates `minimum`, `maximum`, `minLength`, `maxLength`, and `pattern` enforcement:

```json
{
  "id": "rust_validator_constraints",
  "description": "Numeric and string constraints enforced by all three SDK validators",
  "schema": {
    "type": "object",
    "properties": {
      "count": { "type": "integer", "minimum": 1, "maximum": 100 },
      "label": { "type": "string", "minLength": 1, "maxLength": 50, "pattern": "^[a-z_]+$" }
    },
    "required": ["count", "label"]
  },
  "test_cases": [
    { "id": "valid_input", "input": { "count": 50, "label": "hello_world" }, "expected": true },
    { "id": "count_below_minimum", "input": { "count": 0, "label": "hello" }, "expected": false },
    { "id": "count_above_maximum", "input": { "count": 101, "label": "hello" }, "expected": false },
    { "id": "label_too_short", "input": { "count": 5, "label": "" }, "expected": false },
    { "id": "label_too_long", "input": { "count": 5, "label": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }, "expected": false },
    { "id": "label_pattern_mismatch", "input": { "count": 5, "label": "UPPER_CASE" }, "expected": false }
  ]
}
```

---

## Contract: Schema.validate_union

### Inputs
- `data` (dict/object/Value, required) — data to validate against the union schema
- `schema` (dict/object/Value, required) — JSON Schema Draft 2020-12 schema object containing `anyOf` or `oneOf`
- `keyword` (`"anyOf"` | `"oneOf"`, required) — which union keyword governs validation

### Errors
- `SchemaValidationError(code=SCHEMA_UNION_NO_MATCH)` — no branch matched (for `anyOf`) or zero branches matched (for `oneOf`)
- `SchemaValidationError(code=SCHEMA_UNION_AMBIGUOUS)` — more than one branch matched a `oneOf` schema

### Returns
- On success: void/None/() — validation passed; raises on failure

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true

## Contract: Schema.validate_recursive

### Inputs
- `data` (dict/object/Value, required) — potentially deeply-nested data to validate
- `schema` (dict/object/Value, required) — JSON Schema Draft 2020-12 schema that may contain a self-referencing `$ref`
- `max_depth` (int/number/usize, optional, default=32) — maximum recursion depth before raising a depth-limit error

### Errors
- `SchemaValidationError(code=SCHEMA_VALIDATION_FAILED)` — data does not conform to schema at any nesting level
- `SchemaCircularRefError(code=SCHEMA_MAX_DEPTH_EXCEEDED)` — recursion depth exceeded `max_depth`

### Returns
- On success: void/None/() — validation passed at all nesting levels

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true

## Contract: Schema.content_hash

### Inputs
- `schema` (dict/object/Value, required) — resolved JSON Schema dict (all `$ref` entries already inlined)

### Errors
- None — this operation MUST NOT raise; serialization failures MUST surface as panics in development and be caught as internal errors in production

### Returns
- On success: `str`/`string`/`String` — lowercase hexadecimal SHA-256 digest of the canonical JSON serialization of `schema` (64 characters)

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true
