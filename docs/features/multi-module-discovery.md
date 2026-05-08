# Multi-Module Discovery

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §3 Directory Specification.


## Overview

Multi-module discovery is an opt-in extension to the standard apcore module scanner that allows multiple module classes to coexist in a single file. By default, apcore enforces a one-file-one-module model: the canonical module ID is derived entirely from the file path (see [PROTOCOL_SPEC §2.1](../spec/protocol-spec.md#21-directory-as-id-core-rule)). Multi-class discovery relaxes that constraint by appending the snake_case-converted class name as an additional segment to the base file ID.

This feature exists to serve two practical needs:

- **Large module files**: related operations (e.g., `Addition`, `Subtraction`, `Multiplication`) naturally belong together in one file but should each be independently addressable by the registry and ACL engine.
- **Logical grouping**: module authors may prefer to collocate tightly coupled class definitions rather than spread them across many single-class files.

Multi-class mode is always opt-in. Existing single-class files are unaffected and produce identical IDs regardless of whether the feature is enabled.

## Requirements

- Implementations **MAY** support multi-class discovery mode.
- Multi-class discovery **MUST** be explicitly enabled per file (via decorator) or globally (via configuration); it **MUST NOT** activate automatically.
- When enabled for a file, the scanner **MUST** enumerate all exported classes that implement the Module interface.
- Each qualifying class **MUST** receive a module ID of the form `base_id.class_segment`, where `base_id` is the standard file-derived ID and `class_segment` is the snake_case conversion of the class name.
- The full derived `module_id` **MUST** conform to the canonical ID grammar defined in [PROTOCOL_SPEC §2.7](../spec/protocol-spec.md#27-id-formal-grammar).
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

Multi-class discovery is **opt-in per file** via a language-idiomatic marker:

**Per-file opt-in:** Apply `@multi_class` (Python decorator), `@multiClass()` (TypeScript decorator), or `#[multi_class]` (Rust macro attribute) to the file or each participating class. Only classes in annotated files are scanned for multi-class IDs. Files that contain exactly one Module class are unaffected (backward compatibility guarantee applies).

> **Note**: An earlier draft of the spec mentioned a global config key `extensions.multi_class_discovery`. That toggle was never implemented in any SDK and was removed per [decision-log D-06](../spec/2026-05-decision-log.md#d-06-multi_class_enabled-config-plumbing). Per-class markers are the only opt-in path.

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

All three SDKs expose `discover_multi_class` as a **method on `Registry`**, matching the spec contract. The free-function form remains available as an internal helper for SDK-internal use, but new application code SHOULD prefer the method form.

> **Cross-language signature divergence (D11-004 — intentional, language-idiomatic).**
> The method input set differs across SDKs because each language's
> static-analysis story differs:
>
> - **Python** can `import` a file at runtime and reflect on its module
>   members, so the method takes only `(file_path, extensions_root)` and
>   does the reading internally.
> - **TypeScript** cannot reliably introspect class declarations at
>   runtime (ES module specifiers are immutable, AST traversal requires a
>   heavyweight dev dependency like `ts-morph`), so the method takes
>   pre-resolved `ClassDescriptor[]` from the caller's scanner output:
>   `(filePath, classes, extensionsRoot, multiClassEnabled)`.
> - **Rust** uses macro-driven registration (no runtime class reflection),
>   so the multi-class entry point lives as `discover_multi_class` on a
>   separate trait module helper rather than on `Registry` directly.
>
> This is **language-idiomatic divergence** rather than a parity bug.
> Cross-language fixtures cannot 1:1-port a single `discover_multi_class`
> call. Application code that needs to scan multi-class files SHOULD use
> each SDK's idiomatic surface (file-path-in for Python; pre-parsed
> classes for TypeScript; macro-driven trait-module helper for Rust).

| SDK | Public method | Internal helper |
|---|---|---|
| Python | `Registry.discover_multi_class(file_path, extensions_root="extensions")` | `apcore.registry.multi_class._discover_multi_class(...)` |
| TypeScript | `Registry.discoverMultiClass(filePath, classes, extensionsRoot, multiClassEnabled)` | `_discoverMultiClass(filePath, classes, ...)` (module-private) |
| Rust | `apcore::registry::multi_class::derive_module_ids(...)` (trait-module helper, not on `Registry`) | same |

### Inputs

- `file_path` (str/string/&str, required) — path to the file to scan, relative to the project root
- `classes` (`ClassDescriptor[]`, **TypeScript-only**, required) — pre-resolved class descriptors produced by the caller's scanner. Python derives this internally via `import`; Rust derives via macros at compile time.
- `extensions_root` (str/string/&str, optional, default=`"extensions"`) — root directory name used by Algorithm A01
- `multi_class_enabled` (`bool`, **TypeScript-only**, optional, default=`false`) — whether to apply the per-file opt-in described in [Enabling Multi-Class Mode](#enabling-multi-class-mode). Python infers from the `@multi_class` decorator; Rust infers from a macro attribute.
- `pre_approval_hook` (callable, **Python-only**, optional) — pre-import safety check; see [Python-only `pre_approval_hook`](#python-only-pre_approval_hook) below

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

### Cross-SDK usage

=== "Python"
    ```python
    from apcore import Registry

    registry = Registry()
    discovered = registry.discover_multi_class("extensions/math/math_ops.py")
    for module_id, cls in discovered:
        print(module_id, cls.__name__)
    ```
=== "TypeScript"
    ```typescript
    import { Registry, type ClassDescriptor } from "apcore-js";

    const registry = new Registry();
    // TypeScript callers pre-resolve ClassDescriptors via their scanner;
    // commonly produced by the build-time AST scan that wires extension
    // modules into the bundle.
    const classes: ClassDescriptor[] = await scanFile("extensions/math/math-ops.ts");
    const discovered = registry.discoverMultiClass(
        "extensions/math/math-ops.ts",
        classes,
        "extensions",
        /* multiClassEnabled */ true,
    );
    for (const entry of discovered) {
        console.log(entry.moduleId, entry.className);
    }
    ```
=== "Rust"
    ```rust
    use apcore::registry::multi_class::derive_module_ids;

    // Rust resolves classes at compile time via macros; the trait-module
    // helper takes the pre-derived candidate list rather than reading
    // a file at runtime.
    let candidates = build_macro_class_list();  // from your #[apcore::multi_class] expansion
    let discovered = derive_module_ids(
        "extensions/math/math_ops.rs",
        &candidates,
        "extensions",
    )?;
    for (module_id, class_ref) in discovered {
        println!("{} -> {:?}", module_id, class_ref);
    }
    ```

### Python-only `pre_approval_hook`

!!! note "Python-only safety hook"
    `pre_approval_hook` (`Registry.discover_multi_class(file_path, extensions_root, pre_approval_hook=...)`) protects against importing arbitrary code, since Python imports the file at scan time. TypeScript and Rust do **not** import code from disk for discovery — they parse static AST/source — so the hook is **not** present in those SDKs. Callers using TypeScript or Rust should sandbox file system access externally (e.g. constrain `extensions_root`, run discovery under an OS-level allowlist, or pre-resolve the file list themselves) (D-30).

The hook receives the absolute path of the file the registry is about to import; returning `False` (or raising) skips the file. The free-function form `_discover_multi_class(...)` accepts the same parameter.

=== "Python"
    ```python
    from pathlib import Path
    from apcore import Registry

    ALLOWED_DIRS = (Path("extensions/math").resolve(),)

    def is_safe(path: str) -> bool:
        resolved = Path(path).resolve()
        return any(resolved.is_relative_to(allowed) for allowed in ALLOWED_DIRS)

    registry = Registry()
    discovered = registry.discover_multi_class(
        "extensions/math/math_ops.py",
        extensions_root="extensions",
        pre_approval_hook=is_safe,
    )
    ```

=== "TypeScript"
    ```typescript
    // No pre_approval_hook — TS parses static AST and never imports code at scan time.
    // Sandbox file system access externally before calling discoverMultiClass.
    import { Registry } from "apcore-js";

    const registry = new Registry();
    const discovered = await registry.discoverMultiClass(
        "extensions/math/math_ops.ts",
        { extensionsRoot: "extensions" },
    );
    ```

=== "Rust"
    ```rust
    // No pre_approval_hook — Rust parses source via `syn`/proc-macros and never executes code at scan time.
    // Sandbox file system access externally (e.g., constrain `extensions_root` or use an OS allowlist).
    use apcore::Registry;

    let registry = Registry::new();
    let discovered = registry.discover_multi_class("extensions/math/math_ops.rs")?;
    ```

## File extensions and skip patterns

Multi-module discovery scans a configured directory tree for module definitions. Each SDK ships sensible language-native defaults; the table below documents them so cross-language projects can reason about what gets picked up (D-31).

| SDK | Extensions | Skip patterns |
|---|---|---|
| Python | `.py` | `__pycache__/`, `*.pyc`, files starting with `_` |
| TypeScript | `.ts`, `.js` | `*.d.ts`, `*.test.*`, `*.spec.*` |
| Rust | `.rs` (configurable via `with_extensions`) | (none — Rust scans only `.rs`) |

**Notes:**

- Python's leading-`_` skip rule preserves the long-standing convention that `_private.py`, `__init__.py`, and similar files are not auto-discovered.
- TypeScript's `*.d.ts` / `*.test.*` / `*.spec.*` skip patterns prevent declaration files and test files from being mistaken for module sources during AST parsing.
- Rust's extension list is configurable: callers may pass additional extensions to `RegistryBuilder::with_extensions(...)` (e.g. for projects that generate `.rs.in` templates), but the default is `[".rs"]` only.
- Implementations **MAY** expose extension/skip-pattern overrides as configuration knobs in the future; the table above documents the canonical defaults all 3 SDKs ship today.

## Testing Strategy

- **Single-class identity**: confirm that a file with one Module class produces `base_id` (not `base_id.class_segment`) under multi-class mode.
- **Two-class distinct IDs**: confirm that a file with two Module classes produces two distinct, correctly-suffixed IDs.
- **snake_case conversion coverage**: test `Addition` → `addition`, `MathOps` → `math_ops`, `HTTPSender` → `http_sender`, and edge cases with leading/trailing/consecutive non-alphanumeric characters.
- **Conflict detection**: verify that two classes mapping to the same segment raise `MODULE_ID_CONFLICT` and leave the registry unmodified.
- **Grammar conformance**: assert that all derived IDs satisfy `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`.
- **Disabled by default**: confirm that a file with two Module classes and no opt-in is loaded as a single-class module (first class wins or raises, per SDK policy).
- **Full ID length**: verify that a derived ID exceeding 192 characters raises `ID_TOO_LONG`.
- Test naming follows the `test_<unit>_<behavior>` convention.
