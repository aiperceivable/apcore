---
description: "Schema loading, validation, $ref resolution bridging YAML to runtime schema objects; strategies yaml_first/native_first/yaml_only, x-* extensions, strict-mode export, export to MCP/OpenAI/Anthropic formats."
---

# Schema System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §4 Schema Specification.


## Overview

The Schema System loads, validates, resolves (`$ref`) and exports the input/output schemas of apcore modules. It bridges human-authored YAML schema files and the runtime schema objects the executor validates against, and it exports schemas to LLM tool-calling formats.

## Requirements

- Load module schemas from YAML files and turn them into validated runtime representations.
- Resolve `$ref` references, including nested and cross-file references. A self-reference (a recursive data structure) is preserved as a lazy reference; only a `$ref` → `$ref` chain that reaches no schema body is rejected as circular (PROTOCOL_SPEC §4.15).
- Support the JSON Schema composition keywords (`oneOf`, `anyOf`, `allOf`, `not`). Each SDK uses its idiomatic representation: Pydantic models in Python, TypeBox schemas in TypeScript, `serde_json::Value` validated by the `jsonschema` crate (Draft 2020-12) in Rust.
- Validate data against a schema and report failures as structured errors (PROTOCOL_SPEC §4.14).
- Export schemas to the MCP, OpenAI, Anthropic and generic formats, including strict-mode export (§4.16).
- Recognise the LLM extension fields (`x-*`) defined in §4.3.
- Provide the three schema resolution strategies (§4.9).
- Cache resolved schemas by content hash so identical schemas are compiled once.

## Components

### SchemaLoader

`SchemaLoader` is the main entry point. It is constructed from a `Config` (reading `schema.root`, `schema.strategy` and `schema.max_ref_depth`), loads `<schema.root>/<module/id/path>.schema.yaml` for a module ID, resolves its `$ref`s and builds the runtime schema objects. The strategy decides how YAML and code-defined (native) schemas interact:

- **`yaml_first`** (default) — use the YAML file; fall back to the native schema if there is no YAML file.
- **`native_first`** — use the native schema; fall back to YAML if none is registered.
- **`yaml_only`** — use only YAML; raise `SCHEMA_NOT_FOUND` if the file is missing.

=== "Python"
    ```python
    from apcore.schema import SchemaStrategy

    strategy = SchemaStrategy.YAML_FIRST
    # SchemaStrategy.NATIVE_FIRST, SchemaStrategy.YAML_ONLY
    print(strategy.value)  # "yaml_first"
    ```
=== "TypeScript"
    ```typescript
    import { SchemaStrategy } from "apcore-js";

    const strategy: SchemaStrategy = SchemaStrategy.YAML_FIRST;
    // SchemaStrategy.NATIVE_FIRST, SchemaStrategy.YAML_ONLY
    console.log(strategy); // "yaml_first"
    ```
=== "Rust"
    ```rust
    use apcore::schema::SchemaStrategy;

    fn main() {
        let strategy = SchemaStrategy::YamlFirst;
        // SchemaStrategy::NativeFirst, SchemaStrategy::YamlOnly
        println!("{strategy:?}");
    }
    ```

In practice the strategy is set with the `schema.strategy` config key rather than in code.

### RefResolver

`RefResolver` resolves `$ref` references:

- Local references (`#/definitions/Foo`, `#/$defs/Foo`). A local reference is resolved against the schema file's root first, then against the schema node being resolved (D-104).
- Relative-file references (`other.schema.yaml#/definitions/Bar`) and canonical references (`apcore://...`), resolved against `schema.root`.
- Cycle classification: re-entering a reference along a `$ref` → `$ref` chain raises `SCHEMA_CIRCULAR_REF`; re-entering one after descending through a schema body is a self-reference and stays a lazy `$ref`.
- A depth cap (`schema.max_ref_depth`, default `32`); exceeding it raises `SCHEMA_MAX_DEPTH_EXCEEDED`.

Resolved fragments are inlined into a copy of the schema; the input document is never mutated.

### SchemaValidator

`SchemaValidator` validates data against a schema and returns a result object (`valid`, `errors`, plus an error code on failure) rather than raising. `validate_input` / `validate_output` are the raising variants the executor uses. See [Contract: SchemaValidator.validate](#contract-schemavalidatorvalidate).

At the module-invocation boundary validation is strict: no type coercion is applied (TYPE_MAPPING §17.3). The validator's optional `coerce_types` flag is a library-level knob for callers validating their own untyped input, not something the executor enables.

### SchemaExporter and ExportProfile

`SchemaExporter.export(schema_def, profile, ...)` converts a schema definition into a target format selected by `ExportProfile` (§4.17):

- **`mcp`** — MCP tool definition; keeps `x-*` fields and maps annotations to hints.
- **`openai`** — OpenAI function-calling definition; strips `x-*`, applies strict mode.
- **`anthropic`** — Anthropic tool-use definition.
- **`generic`** — provider-neutral representation.

=== "Python"
    ```python
    from apcore.schema import ExportProfile

    profile = ExportProfile.MCP
    # ExportProfile.OPENAI, ExportProfile.ANTHROPIC, ExportProfile.GENERIC
    print(profile.value)  # "mcp"
    ```
=== "TypeScript"
    ```typescript
    import { ExportProfile } from "apcore-js";

    const profile: ExportProfile = ExportProfile.MCP;
    // ExportProfile.OPENAI, ExportProfile.ANTHROPIC, ExportProfile.GENERIC
    console.log(profile); // "mcp"
    ```
=== "Rust"
    ```rust
    use apcore::schema::ExportProfile;

    fn main() {
        let profile = ExportProfile::Mcp;
        // ExportProfile::OpenAi, ExportProfile::Anthropic, ExportProfile::Generic
        println!("{profile:?}");
    }
    ```

`Registry.export_schema(module_id, strict)` exports a registered module's input and output schemas in the generic shape; pass `strict=true` to apply strict-mode conversion.

### LLM extension fields

Schemas may carry the `x-*` fields defined in PROTOCOL_SPEC §4.3:

| Field | Purpose |
|---|---|
| `x-llm-description` | Description for LLM consumers; replaces `description` on export to AI formats |
| `x-examples` | Example values |
| `x-constraints` | Business rules JSON Schema keywords cannot express |
| `x-sensitive` | Marks a field as sensitive; its value is redacted in logs, traces and `context.redacted_inputs` |

Unknown `x-*` keywords are ignored during validation (forward compatibility).

### Strict-mode export

`to_strict_schema()` (Python, Rust) / `toStrictSchema()` (TypeScript) converts a schema to the form OpenAI and Anthropic require under `strict: true` (PROTOCOL_SPEC §4.16, Algorithm A23): every object schema gets `additionalProperties: false`, every property becomes required (optional ones become nullable), and `x-*` fields and `default` values are removed. It is an export transform — it does not change how the executor validates input.

=== "Python"
    ```python
    from apcore.schema import to_strict_schema

    schema = {
        "type": "object",
        "properties": {
            "to": {"type": "string", "x-examples": ["user@example.com"]},
            "cc": {"type": "array", "items": {"type": "string"}, "default": []},
        },
        "required": ["to"],
    }
    strict = to_strict_schema(schema)
    assert strict["additionalProperties"] is False
    assert strict["required"] == ["cc", "to"]
    assert strict["properties"]["cc"]["type"] == ["array", "null"]
    ```
=== "TypeScript"
    ```typescript
    import { toStrictSchema } from "apcore-js";

    const schema = {
      type: "object",
      properties: {
        to: { type: "string", "x-examples": ["user@example.com"] },
        cc: { type: "array", items: { type: "string" }, default: [] },
      },
      required: ["to"],
    };
    const strict = toStrictSchema(schema);
    console.log(strict.additionalProperties); // false
    console.log(strict.required); // ["cc", "to"]
    ```
=== "Rust"
    ```rust
    use apcore::to_strict_schema;
    use serde_json::json;

    fn main() {
        let schema = json!({
            "type": "object",
            "properties": {
                "to": { "type": "string", "x-examples": ["user@example.com"] },
                "cc": { "type": "array", "items": { "type": "string" }, "default": [] }
            },
            "required": ["to"]
        });
        let strict = to_strict_schema(&schema);
        assert_eq!(strict["additionalProperties"], json!(false));
        assert_eq!(strict["required"], json!(["cc", "to"]));
    }
    ```

## Data flow

1. `SchemaLoader.load(module_id)` reads `<schema.root>/<module/id/path>.schema.yaml` into a `SchemaDefinition`.
2. `RefResolver` walks the definition, inlining `$ref` targets (self-references stay lazy) and rejecting `$ref` → `$ref` cycles.
3. The resolved schema is converted to the SDK's runtime representation (Pydantic model / TypeBox schema / compiled `jsonschema` validator).
4. The result is cached by content hash and handed to the executor for validation or to the exporter.

## Validation semantics

### Union keywords

All branches of `anyOf` / `oneOf` are evaluated:

- `anyOf` accepts the input when at least one branch matches; otherwise `SCHEMA_UNION_NO_MATCH`.
- `oneOf` accepts the input when exactly one branch matches; zero matches is `SCHEMA_UNION_NO_MATCH`, more than one is `SCHEMA_UNION_AMBIGUOUS`.

These rules apply wherever the keyword appears — at the root, inside `properties`, `items`, `$defs`, or another combinator's branch — and on every validation path, including the executor's input and output validation steps. A host union type (`typing.Union`, TypeBox `Type.Union`) has `anyOf` semantics, so the SDKs enforce `oneOf` exclusivity themselves rather than relying on it. Authors must keep `oneOf` branches mutually exclusive.

### Recursive schema support {#2-recursive-schema-support}

A schema may refer to itself. When a `$ref` points to the schema's own `$id`, to the document root (`#`, `#/`), or to a location already on the resolution path that was reached through a schema body, the `$ref` is preserved rather than inlined again, and it does **not** raise `SCHEMA_CIRCULAR_REF` — that code is reserved for a `$ref` → `$ref` chain that never reaches a schema body (PROTOCOL_SPEC §4.15).

The preserved reference is bound lazily against the document root at validation time, so recursive positions are validated, not widened to "accept anything". These three forms are all self-references and behave identically:

| Form | Meaning |
|------|---------|
| `{"$ref": "#"}` | the document being resolved |
| `{"$ref": "<root $id>"}` | the document, named by its own identifier |
| `{"$ref": "#/$defs/Node"}` re-entered from inside `$defs/Node` | a recursive `$defs` entry |

The recursive schema used across the conformance fixtures:

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

### Constraint keywords

All three SDKs enforce the numeric and string constraints (`minimum`, `maximum`, `exclusiveMinimum`, `minLength`, `maxLength`, `pattern`), the array and object keywords (`minItems`, `maxItems`, `uniqueItems`, `prefixItems`, `minProperties`, `additionalProperties`), `const` / `enum`, and the composition keywords (`allOf`, `anyOf`, `oneOf`, `not`). Cross-SDK keyword behaviour is pinned by `schema_hardening_constraints.json` and `schema_keyword_parity.json` in `conformance/fixtures/`.

### `format` is an annotation

Under JSON Schema 2020-12's default format-annotation vocabulary, a value that does not satisfy its declared `format` does **not** fail validation. A field declared `format: date-time` accepts any string. See [TYPE_MAPPING §11](../spec/type-mapping.md#111-format-keyword), which is normative for this keyword:

- Implementations recognise the formats listed in TYPE_MAPPING §11.1 and emit a warning when a value does not conform (SHOULD).
- The value is accepted regardless; a format check never produces `SCHEMA_VALIDATION_ERROR`.
- An unrecognised `format` passes silently.

To make a format binding, express it as an assertion: `pattern` for a syntactic shape, `enum` for a closed set.

### Content-hash cache

Resolved schemas are cached in two levels: module ID → content hash, and content hash → compiled schema. The hash is the lowercase SHA-256 of the canonical JSON form of the resolved schema (object keys sorted), so two schemas with the same content — even under different module IDs or with different key order — share one compiled object. See [Contract: content_hash](#contract-content_hash).

## Usage

=== "Python"
    ```python
    from apcore import Config
    from apcore.schema import ExportProfile, SchemaExporter, SchemaLoader, SchemaValidator

    # The loader reads schema.root / schema.strategy from the Config;
    # load() takes a MODULE ID, not a file path.
    loader = SchemaLoader(Config.load("apcore.yaml"))
    schema_def = loader.load("executor.email.send_email")

    # resolve() returns (input, output) ResolvedSchema pairs carrying the Pydantic model.
    input_schema, _output_schema = loader.resolve(schema_def)

    validator = SchemaValidator()
    result = validator.validate({"to": "alice@example.com", "subject": "Hello"}, input_schema.model)
    if not result.valid:
        print(f"Validation failed ({result.error_code}): {result.errors}")

    exporter = SchemaExporter()
    mcp_tool = exporter.export(schema_def, ExportProfile.MCP)
    print(mcp_tool)  # {"name": "...", "description": "...", "inputSchema": {...}, ...}
    ```
=== "TypeScript"
    ```typescript
    import { Config, ExportProfile, SchemaExporter, SchemaLoader, SchemaValidator } from "apcore-js";

    // The loader reads schema.root / schema.strategy from the Config;
    // load() takes a MODULE ID and is synchronous.
    const loader = new SchemaLoader(Config.load("apcore.yaml"));
    const schemaDef = loader.load("executor.email.send_email");

    // resolve() returns [input, output] ResolvedSchema pairs carrying the TypeBox schema.
    const [inputSchema] = loader.resolve(schemaDef);

    const validator = new SchemaValidator();
    const result = validator.validate({ to: "alice@example.com", subject: "Hello" }, inputSchema.schema);
    if (!result.valid) {
      console.error(`Validation failed (${result.errorCode}):`, result.errors);
    }

    const exporter = new SchemaExporter();
    const openaiTool = exporter.export(schemaDef, ExportProfile.OPENAI);
    console.log(openaiTool);
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::schema::{ExportProfile, SchemaExporter, SchemaLoader, SchemaValidator};
    use apcore::Config;
    use serde_json::json;

    fn main() -> Result<(), ModuleError> {
        // with_config() reads schema.root / schema.strategy; load() takes a MODULE ID.
        let config = Config::from_defaults();
        let mut loader = SchemaLoader::with_config(&config, None);
        let schema_def = loader.load("executor.email.send_email")?;

        // validate(value, schema) returns a ValidationResult; it does not raise.
        let validator = SchemaValidator::new();
        let result = validator.validate(
            &json!({ "to": "alice@example.com", "subject": "Hello" }),
            &schema_def.input_schema,
        );
        if !result.valid {
            eprintln!("Validation failed: {:?}", result.errors);
        }

        // export() takes the schema Value, a profile, and optional ExportOptions.
        let exporter = SchemaExporter::new();
        let anthropic_tool = exporter.export(&schema_def.input_schema, ExportProfile::Anthropic, None)?;
        println!("{anthropic_tool}");
        Ok(())
    }
    ```

## Dependencies

- The **Executor** validates inputs at pipeline step 7 (`input_validation`) and outputs at step 9 (`output_validation`) using the Schema System.
- The **Registry** uses it to load module schemas during discovery and to build `ModuleDescriptor`s.

??? info "Python SDK reference"
    Not a protocol requirement — the source layout of `apcore-python`'s schema package.

    | File | Purpose |
    |------|---------|
    | `schema/loader.py` | `SchemaLoader`: YAML loading, strategies, Pydantic model generation, content-hash cache |
    | `schema/ref_resolver.py` | `RefResolver`: `$ref` resolution, self-reference vs. circular classification |
    | `schema/validator.py` | `SchemaValidator`: result-object validation and the raising `validate_input` / `validate_output` |
    | `schema/hardening.py` | `content_hash`, exhaustive union validation, `format` warnings |
    | `schema/exporter.py` | `SchemaExporter`: MCP / OpenAI / Anthropic / generic export |
    | `schema/strict.py` | `to_strict_schema` (Algorithm A23) |
    | `schema/openai_strict.py` | OpenAI strict-mode compatibility detection |
    | `schema/annotations.py` | Merging YAML and code annotations, examples and metadata |
    | `schema/types.py` | `SchemaDefinition`, `ResolvedSchema`, `SchemaValidationResult`, enums |

    Runtime dependencies: `pydantic` (model generation), `pyyaml` (YAML parsing), `jsonschema` (exhaustive union validation).

## Testing strategy

- **Loader** — YAML parsing, the three strategies, cache reuse.
- **RefResolver** — local, cross-file and nested references; a `$ref` re-entered through `properties` / `items` survives as a lazy reference; a `$ref` → `$ref` cycle raises `SCHEMA_CIRCULAR_REF`; chains reaching `schema.max_ref_depth` raise `SCHEMA_MAX_DEPTH_EXCEEDED`.
- **Validator** — every supported type and keyword, both union error codes, `format` warnings that do not fail validation.
- **Exporter** — each profile's output and its handling of `x-*` fields; strict-mode conversion output.
- **Model generation** — constraints (required, types, patterns, enums) are enforced and `x-sensitive` reaches redaction.

Cross-language fixtures in `conformance/fixtures/`: `schema_validation.json`, `schema_keyword_parity.json`, `schema_hardening_union.json`, `schema_hardening_recursive.json`, `schema_hardening_constraints.json`, `schema_hardening_formats.json`, `schema_hardening_cache.json`, `schema_content_hash.json`, `schema_strict_conversion.json`, `schema_export_envelope.json`.

## Contract: SchemaValidator.validate

### Inputs
- `data` (dict/object/Value, required) — the data to validate
- `schema` (required) — the runtime schema: a Pydantic model class (Python), a TypeBox `TSchema` (TypeScript), a JSON Schema `Value` (Rust)

### Errors
- None — `validate` does not raise; failure is reported in the returned result object. For the raising form use `validate_input` / `validate_output` (all SDKs) or `validate_or_error` (Rust), which raise `SchemaValidationError` (`SCHEMA_VALIDATION_ERROR`, or a `SCHEMA_UNION_*` code).

### Returns
- Success: `SchemaValidationResult { valid: true, errors: [] }` (Python, TypeScript) / `ValidationResult { valid: true, errors: [] }` (Rust)
- Failure: the same shape with `valid: false`, an `errors` array of §4.14 detail objects (`path`, `message`, `constraint`, `expected`, `actual`), and an error code (`error_code` / `errorCode`) in Python and TypeScript.

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true

## Contract: RefResolver — `$ref` resolution

### SDK surfaces

| SDK | Construction | Whole-document resolution | Single reference |
|---|---|---|---|
| Python | `RefResolver(schemas_dir, max_depth=32)` | `resolve(schema, current_file=None)` | `resolve_ref(ref_string, current_file, ...)` |
| TypeScript | `new RefResolver(schemasDir, maxDepth = 32)` | `resolve(schema, currentFile?)` | `resolveRef(refString, currentFile, ...)` |
| Rust | `RefResolver::new()` / `RefResolver::with_max_depth(n)`, then `.with_schemas_dir(dir)` / `.with_current_file(file)` | `resolve(&schema)` | — |

`SchemaLoader` builds its resolver from `schema.root` and `schema.max_ref_depth`; most callers never construct one directly.

### Inputs
- `schema` (object/map/`Value`, required) — the schema document; every `$ref` in it is resolved in one traversal. The input is not mutated.
- `current_file` / `currentFile` (path, optional) — the file the document was loaded from; relative references resolve against it and local `#/…` references resolve against its root first.
- `schemas_dir` / `schemasDir` / `with_schemas_dir` — root for relative-file and `apcore://` references.
- `max_depth` / `maxDepth` (integer, default `32`) — reference-depth cap.

### Errors
- `SchemaCircularRefError` (`SCHEMA_CIRCULAR_REF`) — a `$ref` → `$ref` cycle. A self-reference is not an error.
- `SchemaNotFoundError` (`SCHEMA_NOT_FOUND`) — a referenced schema or pointer cannot be resolved.
- `SchemaMaxDepthExceededError` (`SCHEMA_MAX_DEPTH_EXCEEDED`) — the reference depth exceeded `max_depth`.
- `SchemaParseError` (`SCHEMA_PARSE_ERROR`) — a referenced file is not valid YAML/JSON.

### Returns
- A copy of the schema with every `$ref` inlined, except self-references, which remain lazy `$ref` nodes.

### Properties
- async: false
- thread_safe: true
- pure: true (reads referenced files; caches them)
- idempotent: true

## Contract: content_hash

`content_hash` (Python `apcore.schema.hardening`, Rust `apcore::schema`) / `contentHash` (TypeScript; `contentHashAsync` in the browser build).

### Inputs
- `schema` (dict/object/Value, required) — a resolved JSON Schema

### Errors
- None.

### Returns
- `str` / `string` / `String` — the lowercase hexadecimal SHA-256 (64 characters) of the canonical JSON serialization of `schema` (object keys sorted, no insignificant whitespace). The three SDKs produce byte-identical hashes; `conformance/fixtures/schema_content_hash.json` pins the canonical form for floats, non-ASCII text and large integers.

### Properties
- async: false (`contentHashAsync` is async)
- thread_safe: true
- pure: true
- idempotent: true
