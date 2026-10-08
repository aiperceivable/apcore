---
description: "Opt-in per-class mode letting several module classes share one file: derives IDs as base_id.snake_case_class_segment, validates grammar, raises MODULE_ID_CONFLICT on duplicate segments."
---

# Multi-Module Discovery

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §2.1.1 Multi-Class Discovery.


## Overview

By default apcore uses a one-file-one-module model: the canonical module ID is derived entirely from the file path (see [PROTOCOL_SPEC §2.1](../spec/protocol-spec.md#21-directory-as-id-core-rule)). Multi-module discovery lets several module classes share one file by appending the snake_case form of each class name to the file's base ID.

It serves two needs:

- **Related operations in one file** — `Addition`, `Subtraction`, `Multiplication` belong together but must each be independently addressable by the registry and the ACL engine.
- **Logical grouping** — tightly coupled classes can live side by side instead of being spread across single-class files.

Multi-class discovery is an explicit call, separate from the registry's ordinary `discover()` scan: you hand one file to it and register what it returns.

## Opt-in model: per-class markers

Opt-in is **per class** — there is no file-level or configuration toggle (D-107). Each SDK carries the marker in its own idiom:

| SDK | Marker |
|---|---|
| Python | the `@multi_class` class decorator (`from apcore.registry import multi_class`) |
| TypeScript | `multiClass: true` on the `ClassDescriptor` you pass in |
| Rust | `.with_multi_class(true)` on the `MultiClassEntry` (or `DiscoveredClass`) you pass in |

A file is in multi-class mode when at least one qualifying class carries the marker. What counts as a qualifying class differs by SDK:

Once a file is in multi-class mode, only the classes that carry the marker receive IDs; a class without it is not registered, even when it implements Module (D-147). A helper class beside the modules is therefore simply left unmarked. In Python a class qualifies when it carries `@multi_class` and looks like a Module (`input_schema`, `output_schema`, callable `execute`, defined in that file); in TypeScript and Rust when its descriptor carries the marker and says it implements Module (`implementsModule` / `implements_module`).

## ID derivation

For a file in multi-class mode:

1. **Base ID** — apply Algorithm A01 (`directory_to_canonical_id`): take the path components after the `extensions_root` directory, strip the file extension, join with `.`. If `extensions_root` does not appear in the path, the base ID is the bare file stem.
2. **Single-class identity** — if the file has exactly one Module class, its ID is the base ID unchanged (no segment appended). This keeps existing single-class IDs stable. Any other marked class gets `base_id + "." + segment`, including one marked class beside an unmarked Module class, so marking a second class never renames the first (D-147).
3. **Class segment** — for each qualifying class, convert the class name with the snake_case algorithm below.
4. **Segment grammar** — the segment must match `^[a-z][a-z0-9_]*$`; otherwise `INVALID_SEGMENT`.
5. **Conflict check** — if the segment was already produced by another class in the same file, `MODULE_ID_CONFLICT`.
6. **Module ID** — `base_id + "." + segment`; it must match the canonical ID grammar `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$` (otherwise `INVALID_SEGMENT`) and be at most 192 characters (otherwise `ID_TOO_LONG`).

Any error aborts the whole file: no module from it is registered.

### snake_case algorithm

All three SDKs implement the same six steps (`class_name_to_segment` / `classNameToSegment`):

1. Insert `_` between a run of capitals and a following capitalised word — regex `([A-Z]+)([A-Z][a-z])` → `\1_\2` (`HTTPSender` → `HTTP_Sender`).
2. Insert `_` between a lowercase letter or digit and a following capital — regex `([a-z\d])([A-Z])` → `\1_\2` (`MathOps` → `Math_Ops`).
3. Replace every non-alphanumeric character with `_`.
4. Lowercase.
5. Collapse consecutive `_` to one.
6. Strip leading and trailing `_`.

| Class name | Segment |
|---|---|
| `Addition` | `addition` |
| `MathOps` | `math_ops` |
| `HTTPSender` | `http_sender` |
| `MyModule_V2` | `my_module_v2` |
| `_Internal__Helper_` | `internal_helper` |

Steps 1 and 2 are what split CamelCase; without them `MathOps` would become `mathops`.

### Conflict detection

Two classes conflict when their segments are identical, for example:

- `MyModule` and `My_Module` → both `my_module`
- `HTTPClient` and `Http_Client` → both `http_client`

The SDK logs the conflict and raises `MODULE_ID_CONFLICT`; the error details carry `file_path`, `class_names` (both classes), and `conflicting_segment`.

## API

| SDK | Entry point | Returns |
|---|---|---|
| Python | `Registry.discover_multi_class(file_path, extensions_root="extensions")` — also the free function `apcore.discover_multi_class(file_path, extensions_root="extensions", pre_approval_hook=None)` | `list[tuple[str, type]]` — `(module_id, class)` pairs; you instantiate and register |
| TypeScript | `registry.discoverMultiClass(filePath, classes, extensionsRoot = "extensions")` — also the free function `discoverMultiClass` | `MultiClassEntry[]` — `{ moduleId, className }`; you register the matching instances |
| Rust | `Registry::register_multi_class(&self, file_path, extensions_root, entries, &DiscoveryConfig)` — also the pure `derive_module_ids(file_path, extensions_root, &classes, &DiscoveryConfig)` | `Result<Vec<String>, ModuleError>` — the IDs; `register_multi_class` also registers them |

Notes:

- **Python imports the file** to enumerate its classes. `Registry.discover_multi_class` forwards the `pre_approval_hook` given to `Registry(...)`; the hook is called with the file path before import and rejects the file by raising (surfaced as `ModuleLoadError`).
- **TypeScript and Rust do not read the file.** They cannot enumerate classes at runtime, so you pass the class list: `ClassDescriptor { name, implementsModule, multiClass? }` in TypeScript, `MultiClassEntry::new(class_name, Box<dyn Module>)` in Rust. There is no pre-approval hook in either SDK because no code is loaded.
- **Rust registration is atomic.** `register_multi_class` rolls back the modules it already registered from the batch if a later registration fails (for example, a duplicate ID from another file).
- **Deprecated inputs.** TypeScript's fourth `multiClassEnabled` argument and Rust's `DiscoveryConfig::multi_class` field are ignored; the per-class marker is the only opt-in. Rust still requires a `&DiscoveryConfig` argument — pass `&DiscoveryConfig::default()`.

### Errors

| Code | Python / TypeScript class | When |
|---|---|---|
| `MODULE_ID_CONFLICT` | `ModuleIdConflictError` | Two classes in the file produce the same segment |
| `INVALID_SEGMENT` | `InvalidSegmentError` | A segment, or the full derived ID, violates the ID grammar (e.g. a class name that starts with a digit) |
| `ID_TOO_LONG` | `IdTooLongError` | The derived module ID exceeds 192 characters |
| `MODULE_LOAD_ERROR` | `ModuleLoadError` | Python only: the file cannot be imported, or the pre-approval hook rejected it |

Rust returns these as `ModuleError` with `ErrorCode::ModuleIdConflict`, `ErrorCode::InvalidSegment` or `ErrorCode::IdTooLong`.

## Usage

The file `extensions/math/math_ops.*` below contains two modules; both register under `math.math_ops.*`.

=== "Python"
    ```python
    # extensions/math/math_ops.py
    from pydantic import BaseModel, Field

    from apcore import Context, Module
    from apcore.registry import multi_class


    class MathInput(BaseModel):
        a: float = Field(..., description="First operand")
        b: float = Field(..., description="Second operand")


    class MathResult(BaseModel):
        result: float = Field(..., description="Computed result")


    @multi_class
    class Addition(Module):
        input_schema = MathInput
        output_schema = MathResult
        description = "Add two numbers and return their sum."

        def execute(self, inputs: dict, context: Context) -> dict:
            return {"result": inputs["a"] + inputs["b"]}


    @multi_class
    class Subtraction(Module):
        input_schema = MathInput
        output_schema = MathResult
        description = "Subtract b from a and return the difference."

        def execute(self, inputs: dict, context: Context) -> dict:
            return {"result": inputs["a"] - inputs["b"]}
    ```

    ```python
    # app.py
    from pathlib import Path

    from apcore import Registry

    ALLOWED = Path("extensions").resolve()


    def only_inside_extensions(path: Path) -> None:
        if not Path(path).resolve().is_relative_to(ALLOWED):
            raise PermissionError(f"refusing to import {path}")


    registry = Registry(pre_approval_hook=only_inside_extensions)

    for module_id, cls in registry.discover_multi_class("extensions/math/math_ops.py"):
        registry.register(module_id, cls())

    print(registry.list())  # ['math.math_ops.addition', 'math.math_ops.subtraction']
    ```

=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { Registry } from "apcore-js";
    import type { ClassDescriptor, Context, Module } from "apcore-js";

    const MathInput = Type.Object({
      a: Type.Number({ description: "First operand" }),
      b: Type.Number({ description: "Second operand" }),
    });
    const MathResult = Type.Object({
      result: Type.Number({ description: "Computed result" }),
    });

    // extensions/math/math_ops.ts
    class Addition implements Module {
      inputSchema = MathInput;
      outputSchema = MathResult;
      description = "Add two numbers and return their sum.";

      async execute(inputs: Record<string, unknown>, _context: Context): Promise<Record<string, unknown>> {
        return { result: (inputs.a as number) + (inputs.b as number) };
      }
    }

    class Subtraction implements Module {
      inputSchema = MathInput;
      outputSchema = MathResult;
      description = "Subtract b from a and return the difference.";

      async execute(inputs: Record<string, unknown>, _context: Context): Promise<Record<string, unknown>> {
        return { result: (inputs.a as number) - (inputs.b as number) };
      }
    }

    // Describe the file's classes; `multiClass: true` is the per-class opt-in.
    const classes: ClassDescriptor[] = [
      { name: "Addition", implementsModule: true, multiClass: true },
      { name: "Subtraction", implementsModule: true, multiClass: true },
    ];
    const instances: Record<string, Module> = {
      Addition: new Addition(),
      Subtraction: new Subtraction(),
    };

    const registry = new Registry();
    for (const entry of registry.discoverMultiClass("extensions/math/math_ops.ts", classes)) {
      await registry.register(entry.moduleId, instances[entry.className]);
    }

    console.log(registry.list()); // ['math.math_ops.addition', 'math.math_ops.subtraction']
    ```

=== "Rust"
    ```rust
    use std::path::Path;

    use apcore::{Context, DiscoveryConfig, Module, ModuleError, MultiClassEntry, Registry};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    fn math_input() -> Value {
        json!({
            "type": "object",
            "properties": {
                "a": { "type": "number", "description": "First operand" },
                "b": { "type": "number", "description": "Second operand" }
            },
            "required": ["a", "b"]
        })
    }

    fn math_result() -> Value {
        json!({
            "type": "object",
            "properties": { "result": { "type": "number", "description": "Computed result" } },
            "required": ["result"]
        })
    }

    struct Addition;

    #[async_trait]
    impl Module for Addition {
        fn description(&self) -> &str { "Add two numbers and return their sum." }
        fn input_schema(&self) -> Value { math_input() }
        fn output_schema(&self) -> Value { math_result() }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let a = inputs["a"].as_f64().unwrap_or(0.0);
            let b = inputs["b"].as_f64().unwrap_or(0.0);
            Ok(json!({ "result": a + b }))
        }
    }

    struct Subtraction;

    #[async_trait]
    impl Module for Subtraction {
        fn description(&self) -> &str { "Subtract b from a and return the difference." }
        fn input_schema(&self) -> Value { math_input() }
        fn output_schema(&self) -> Value { math_result() }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let a = inputs["a"].as_f64().unwrap_or(0.0);
            let b = inputs["b"].as_f64().unwrap_or(0.0);
            Ok(json!({ "result": a - b }))
        }
    }

    fn main() -> Result<(), ModuleError> {
        let registry = Registry::new();

        // `.with_multi_class(true)` is the per-class opt-in.
        let entries = vec![
            MultiClassEntry::new("Addition", Box::new(Addition)).with_multi_class(true),
            MultiClassEntry::new("Subtraction", Box::new(Subtraction)).with_multi_class(true),
        ];

        let ids = registry.register_multi_class(
            Path::new("extensions/math/math_ops.rs"),
            "extensions",
            entries,
            &DiscoveryConfig::default(),
        )?;

        assert_eq!(ids, vec!["math.math_ops.addition", "math.math_ops.subtraction"]);
        Ok(())
    }
    ```

## Behaviour without the marker

When no qualifying class carries the marker:

- **Python** — `discover_multi_class` returns `[]` (there are no qualifying classes). The ordinary `discover()` scan treats a file with more than one Module class as an ambiguous entry point and does not load it.
- **TypeScript / Rust** — the file is single-class: the result is one entry, the base ID, for the first qualifying class.

## Testing checklist

- **Single-class identity** — a file with one qualifying class yields the base ID, with no segment.
- **Distinct IDs** — two marked classes yield two correctly suffixed IDs.
- **snake_case coverage** — `Addition`, `MathOps`, `HTTPSender`, and names with leading, trailing or consecutive non-alphanumeric characters.
- **Conflict** — two classes mapping to the same segment raise `MODULE_ID_CONFLICT` and leave the registry unchanged.
- **Grammar and length** — derived IDs match the canonical grammar; an ID over 192 characters raises `ID_TOO_LONG`.
- **No marker** — the per-SDK behaviour in [Behaviour without the marker](#behaviour-without-the-marker).

The cross-language cases live in `conformance/fixtures/multi_module_discovery.json`.
