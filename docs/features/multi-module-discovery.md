# Multi-Module Discovery

## Overview

Multi-module discovery is an opt-in extension to the standard apcore module scanner that allows multiple module classes to coexist in a single file. By default, apcore enforces a one-file-one-module model: the canonical module ID is derived entirely from the file path (see [PROTOCOL_SPEC §2.1](../../PROTOCOL_SPEC.md#21-directory-as-id-core-rule)). Multi-class discovery relaxes that constraint by appending the snake_case-converted class name as an additional segment to the base file ID.

This feature exists to serve two practical needs:

- **Large module files**: related operations (e.g., `Addition`, `Subtraction`, `Multiplication`) naturally belong together in one file but should each be independently addressable by the registry and ACL engine.
- **Logical grouping**: module authors may prefer to collocate tightly coupled class definitions rather than spread them across many single-class files.

Multi-class mode is always opt-in. Existing single-class files are unaffected and produce identical IDs regardless of whether the feature is enabled.

## Requirements

- Implementations **MAY** support multi-class discovery mode.
- Multi-class discovery **MUST** be explicitly enabled per file (via decorator) or globally (via configuration); it **MUST NOT** activate automatically.
- When enabled for a file, the scanner **MUST** enumerate all exported classes that implement the Module interface.
- Each qualifying class **MUST** receive a module ID of the form `base_id.class_segment`, where `base_id` is the standard file-derived ID and `class_segment` is the snake_case conversion of the class name.
- The full derived `module_id` **MUST** conform to the canonical ID grammar defined in [PROTOCOL_SPEC §2.7](../../PROTOCOL_SPEC.md#27-id-formal-grammar).
- If two classes in the same file produce the same `class_segment`, implementations **MUST** raise `MODULE_ID_CONFLICT` and **MUST NOT** register any module from that file.
- A file containing exactly one Module class **MUST** produce the same ID in both single-class and multi-class modes (backward compatibility guarantee).
- The `snake_case` conversion algorithm **MUST** be applied consistently across all SDKs (see [Discovery Algorithm](#discovery-algorithm) below).

## Technical Design

### Discovery Algorithm

The scanner executes the following steps when multi-class discovery is enabled for a file:

1. **Compute base ID** — apply Algorithm A01 (`directory_to_canonical_id`) to derive the standard file-path-based ID.
2. **Enumerate classes** — collect all exported classes in the file that implement the Module interface.
3. **Convert class name to segment** — for each class, apply `snake_case(ClassName)`:
   - Replace every non-alphanumeric character with `_`.
   - Lowercase the entire string.
   - Collapse consecutive `_` characters to a single `_`.
   - Strip leading and trailing `_` characters.
4. **Derive module ID** — concatenate `base_id + "." + class_segment`.
5. **Validate** — verify the full `module_id` matches the canonical ID grammar (`^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`) and does not exceed 192 characters.
6. **Conflict detection** — collect all `class_segment` values for the file; if any two are equal, raise `MODULE_ID_CONFLICT` before registering any module.
7. **Register** — each validated module ID is registered independently in the registry.

**snake_case conversion examples:**

| Class name | Segment |
|---|---|
| `Addition` | `addition` |
| `MathOps` | `math_ops` |
| `HTTPSender` | `http_sender` |
| `MyModule_V2` | `my_module_v2` |

### Enabling Multi-Class Mode

Multi-class discovery can be enabled at two granularities:

**Per-file opt-in (decorator):** Apply `@multi_class` (or the language-equivalent marker) to the file or each participating class. Only classes in annotated files are scanned for multi-class IDs.

**Global opt-in (configuration):** Set `extensions.multi_class_discovery: true` in `apcore.yaml` to enable multi-class scanning for all files. Individual files that contain exactly one Module class are unaffected (backward compatibility guarantee applies).

### Conflict Detection

Two classes produce a conflict when their snake_case-converted names are identical. Common sources of conflict:

- `MyModule` and `My_Module` both produce `my_module`.
- `HTTPClient` and `Http_Client` both produce `http_client`.

When a conflict is detected, implementations **MUST**:

1. Raise `MODULE_ID_CONFLICT` with the file path, both class names, and the conflicting segment in the error details.
2. Abort registration of the entire file — no partial registration is permitted.
3. Log the conflict at `ERROR` level with `trace_id` if a context is available.

### Backward Compatibility

The single-class guarantee ensures zero breaking changes for existing module files:

- A file with one Module class under multi-class mode produces `base_id.class_segment`, which differs from the original `base_id`.
- **Exception — the single-class identity guarantee:** if exactly one class is present, implementations **MUST** use the original `base_id` (without appending the class segment), regardless of whether multi-class mode is enabled. This preserves all existing module IDs.
- ACL rules, conformance fixtures, and external references to existing IDs remain valid without modification.

## Usage

=== "Python"
    ```python
    from apcore import Module, ModuleAnnotations, Context
    from apcore.discovery import multi_class
    from pydantic import BaseModel, Field

    class AddInput(BaseModel):
        a: float = Field(..., description="First operand")
        b: float = Field(..., description="Second operand")

    class MathResult(BaseModel):
        result: float = Field(..., description="Computed result")

    @multi_class
    class Addition(Module):
        input_schema = AddInput
        output_schema = MathResult
        description = "Add two numbers and return their sum."

        def execute(self, inputs: dict, context: Context) -> dict:
            validated = AddInput(**inputs)
            return MathResult(result=validated.a + validated.b).model_dump()

    @multi_class
    class Subtraction(Module):
        input_schema = AddInput
        output_schema = MathResult
        description = "Subtract b from a and return the difference."

        def execute(self, inputs: dict, context: Context) -> dict:
            validated = AddInput(**inputs)
            return MathResult(result=validated.a - validated.b).model_dump()

    # File: extensions/math/math_ops.py
    # Registered IDs:
    #   math.math_ops.addition
    #   math.math_ops.subtraction
    ```

=== "TypeScript"
    ```typescript
    import { Module, ModuleAnnotations, Context, multiClass } from "apcore-js";
    import { Type } from "@sinclair/typebox";

    const MathInputSchema = Type.Object({
        a: Type.Number({ description: "First operand" }),
        b: Type.Number({ description: "Second operand" }),
    });

    const MathResultSchema = Type.Object({
        result: Type.Number({ description: "Computed result" }),
    });

    @multiClass()
    export class Addition extends Module {
        inputSchema = MathInputSchema;
        outputSchema = MathResultSchema;
        description = "Add two numbers and return their sum.";

        async execute(inputs: Record<string, unknown>, context: Context): Promise<Record<string, unknown>> {
            const a = inputs.a as number;
            const b = inputs.b as number;
            return { result: a + b };
        }
    }

    @multiClass()
    export class Subtraction extends Module {
        inputSchema = MathInputSchema;
        outputSchema = MathResultSchema;
        description = "Subtract b from a and return the difference.";

        async execute(inputs: Record<string, unknown>, context: Context): Promise<Record<string, unknown>> {
            const a = inputs.a as number;
            const b = inputs.b as number;
            return { result: a - b };
        }
    }

    // File: extensions/math/math_ops.ts
    // Registered IDs:
    //   math.math_ops.addition
    //   math.math_ops.subtraction
    ```

=== "Rust"
    ```rust
    use apcore::{Module, Context, MultiClass};
    use serde_json::{json, Value};

    #[derive(MultiClass)]
    pub struct Addition;

    impl Module for Addition {
        fn description(&self) -> &str {
            "Add two numbers and return their sum."
        }

        fn execute(&self, inputs: Value, _context: &Context) -> Result<Value, apcore::Error> {
            let a = inputs["a"].as_f64().unwrap_or(0.0);
            let b = inputs["b"].as_f64().unwrap_or(0.0);
            Ok(json!({ "result": a + b }))
        }
    }

    #[derive(MultiClass)]
    pub struct Subtraction;

    impl Module for Subtraction {
        fn description(&self) -> &str {
            "Subtract b from a and return the difference."
        }

        fn execute(&self, inputs: Value, _context: &Context) -> Result<Value, apcore::Error> {
            let a = inputs["a"].as_f64().unwrap_or(0.0);
            let b = inputs["b"].as_f64().unwrap_or(0.0);
            Ok(json!({ "result": a - b }))
        }
    }

    // File: extensions/math/math_ops.rs
    // Registered IDs:
    //   math.math_ops.addition
    //   math.math_ops.subtraction
    ```

## Contract: Registry.discover_multi_class

### Inputs

- `file_path` (str/string/&str, required) — path to the file to scan, relative to the project root
- `extensions_root` (str/string/&str, optional, default=`"extensions"`) — root directory name used by Algorithm A01

### Errors

- `MODULE_ID_CONFLICT` — two or more classes in the file produce the same `class_segment` after snake_case conversion; details include `file_path`, `class_names`, and `conflicting_segment`
- `INVALID_SEGMENT` — a derived `class_segment` does not conform to the canonical ID grammar (e.g., starts with a digit after snake_case conversion)
- `ID_TOO_LONG` — the full derived `module_id` exceeds 192 characters

### Returns

- On success: list of `(module_id: str, class_ref)` pairs — one entry per qualifying Module class discovered in the file

### Properties

- async: false
- thread_safe: true
- pure: false (reads file system)
- idempotent: true (repeated calls with the same file produce the same result)

## Testing Strategy

- **Single-class identity**: confirm that a file with one Module class produces `base_id` (not `base_id.class_segment`) under multi-class mode.
- **Two-class distinct IDs**: confirm that a file with two Module classes produces two distinct, correctly-suffixed IDs.
- **snake_case conversion coverage**: test `Addition` → `addition`, `MathOps` → `math_ops`, `HTTPSender` → `http_sender`, and edge cases with leading/trailing/consecutive non-alphanumeric characters.
- **Conflict detection**: verify that two classes mapping to the same segment raise `MODULE_ID_CONFLICT` and leave the registry unmodified.
- **Grammar conformance**: assert that all derived IDs satisfy `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`.
- **Disabled by default**: confirm that a file with two Module classes and no opt-in is loaded as a single-class module (first class wins or raises, per SDK policy).
- **Full ID length**: verify that a derived ID exceeding 192 characters raises `ID_TOO_LONG`.
- Test naming follows the `test_<unit>_<behavior>` convention.
