# APCore Declarative Configuration Specification

**Spec version**: `1.0`
**Applies to SDKs**: `apcore-python >= 0.19.0`, `apcore-typescript >= 0.19.0`, `apcore-rust >= 0.19.0`
**Status**: Draft for review — not yet implemented
**Canonical location**: `apcore/docs/spec/DECLARATIVE_CONFIG_SPEC.md`
**Authoritative schemas**: `apcore/schemas/{binding,apcore-config,module-meta}.schema.json`
**Related specs**: `apcore/protocol-spec.md` (module ID format, protocol wire format)
**Last updated**: 2026-04-16

---

## 1. Overview

APCore supports three declarative YAML surfaces for integrating modules and customizing pipelines without modifying source code:

| Surface | File pattern | Canonical JSON Schema | Purpose |
|---|---|---|---|
| **Bindings** | `*.binding.yaml` | `apcore/schemas/binding.schema.json` | Register existing functions/classes as apcore modules |
| **Pipeline config** | `pipeline:` section of `apcore.yaml` | `apcore/schemas/apcore-config.schema.json` (extended in 0.19.0) | Customize the execution pipeline |
| **Entry point meta** | `entry_point:` key in module manifest | `apcore/schemas/module-meta.schema.json` | Override auto-discovery |

This document defines:
- **YAML syntax** — identical across all three SDKs
- **Per-field behavior** — must be defined for every SDK (never silently dropped)
- **Default behavior** — auto-processing where the language permits; explicit error otherwise
- **Error model** — when to fail (parse-time vs resolve-time); exact message templates for cross-SDK alignment
- **Configurable policy limits** — soft limits live in `apcore.yaml` for deployment flexibility
- **Versioning** — how the spec evolves

**Core principles**:

1. **YAML syntax is 100% consistent across SDKs.** Only the underlying resolution mechanism differs (e.g., Rust handler-map vs Python `importlib`).
2. **Never silently drop a field.** Unknown fields, type mismatches, and feature-not-supported conditions are parse-time errors with actionable messages.
3. **Auto-processing is the default**, not an opt-in. Users only declare schema explicitly when overriding.
4. **JSON Schema enforces structure; runtime config enforces policy.** UI / UX limits (description length, tags pattern, etc.) live in `apcore.yaml` so deployments can adjust without forking the spec.
5. **The canonical JSON Schema files in `apcore/schemas/` are the source of truth.** This document is the human-readable interpretation. If they disagree, the schema wins and this document is updated.

---

## 2. Conventions

### 2.1 Field naming

- **YAML / JSON field names**: `snake_case` universally.
- **SDK runtime objects**: each SDK uses its own idiomatic case (`snake_case` in Python/Rust, `camelCase` in TypeScript). SDKs MUST accept `snake_case` on the wire and convert to their runtime idiom during parsing.

### 2.2 String target syntax

References to executable code (`target`, `handler`, `entry_point`) use the universal format:

```
"<module_path>:<symbol>"
```

For class methods:

```
"<module_path>:<ClassName>.<method_name>"
```

The `target` field's canonical regex (relaxed in 1.0):

```
^[@./a-zA-Z_][-@./a-zA-Z0-9_]*:[a-zA-Z_][a-zA-Z0-9_.]*$
```

This pattern accepts:
- Python dotted paths: `my_app.utils:format_date`, `my_app.utils:Service.method`
- TypeScript ESM specifiers: `./format-date:formatDate`, `@myorg/pkg:formatDate`
- Rust handler-map keys: `format_date:format_date_string`, `format_date:Service.format`

SDKs MUST add language-specific security validation on top (e.g., reject `..` segments to prevent path traversal in TypeScript). Such validation produces `BindingInvalidTargetError` at parse time.

### 2.3 Error reporting phases

- **Parse-time errors**: raised when `load_bindings()` / `build_strategy_from_config()` is called and the YAML is structurally invalid, contains unsupported fields, or references unsupported features.
- **Resolve-time errors**: raised when the referenced code is actually invoked and fails.

Unknown fields, type mismatches, mode conflicts, and feature-not-supported conditions MUST be parse-time errors. Each SDK MUST emit a message that conforms to the templates in §7.2.

### 2.4 Spec versioning

Each YAML file MAY include a top-level `spec_version` field:

```yaml
spec_version: "1.0"
```

If absent, the SDK assumes `"1.0"` and emits a **deprecation warning** (will become a parse-time error in spec 1.1). Breaking changes increment major; additive changes increment minor. SDKs MUST accept newer minor versions but MAY warn on unknown fields they don't recognize.

### 2.5 Configurable policy limits

Soft limits (UI/UX guardrails like description length, tag format) are NOT hardcoded into the canonical JSON Schemas. They live in `apcore.yaml` under `apcore.validation.*` so deployments can tighten or relax without forking the spec. See §9 for the full policy catalog.

Hard limits (filesystem-safety constraints, OpenAI function-name spec compliance, regex security) remain hardcoded in JSON Schema — they are technical constraints, not policy.

---

## 3. Bindings YAML (`*.binding.yaml`)

**Source of truth**: `apcore/schemas/binding.schema.json`.

### 3.1 File structure

```yaml
spec_version: "1.0"
bindings:
  - module_id: "utils.format_date"
    target: "my_app.utils.format_date:format_date"
    description: "Format a date string"
    documentation: |
      Long-form documentation for AI/LLM consumption.
    tags: ["utility", "date"]
    version: "1.0.0"
    annotations:
      readonly: true
      idempotent: true
    display:
      alias: "format_date"
      guidance: "Use when reformatting between known date formats."
      mcp:
        alias: "format_date"
      cli:
        alias: "format-date"
    metadata:
      owner_team: "utilities"
```

A binding entry with no `auto_schema` / `input_schema`+`output_schema` / `schema_ref` fields uses the **default auto-processing behavior** (see §3.4).

### 3.2 Top-level keys

| Key | Type | Required | Description |
|---|---|---|---|
| `spec_version` | string | recommended | Defaults to `"1.0"` with deprecation warning if omitted (mandatory in 1.1). |
| `bindings` | list | **yes** | List of binding entries; `minItems: 1`. |

Any other top-level key is parse-time error (`additionalProperties: false`).

### 3.3 Binding entry fields

| Field | Type | Required | Purpose |
|---|---|---|---|
| `module_id` | string | **yes** | apcore module ID. Pattern + maxLength governed by `PROTOCOL_SPEC §2.7`. |
| `target` | string | **yes** | Reference to the callable. See §2.2. |
| `description` | string | no | Short description for AI/LLM. Length governed by `apcore.validation.binding.description_max_length` (default 500, configurable, null = unlimited). |
| `documentation` | string | no | Extended long-form docs. Length governed by `apcore.validation.binding.documentation_max_length` (default null = unlimited). |
| `tags` | list of strings | no | Item format governed by `apcore.validation.binding.tags_pattern` (default `^[a-z][a-z0-9_]*$`, configurable, null = no constraint). |
| `version` | string | no | SemVer (default `"1.0.0"`). Pattern: `^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$`. |
| `auto_schema` | boolean OR `"true"` \| `"strict"` \| `"permissive"` | no | See §3.4 and §6. |
| `input_schema` | JSON Schema object | no | Explicit input schema. |
| `output_schema` | JSON Schema object | no | Explicit output schema. |
| `schema_ref` | string (path) | no | External schema YAML file path, relative to the binding file's directory. |
| `annotations` | object | no | Behavior hints for AI (see §3.5). |
| `display` | object | no | Surface-facing presentation overlay (see §3.6). |
| `metadata` | object | no | Free-form extension (`additionalProperties: true`). |

Unknown fields at the entry level produce parse-time error.

### 3.4 Schema mode resolution

Schema source determined at parse time, in this order:

1. If `input_schema` AND `output_schema` are present → **explicit** mode.
2. Else if `schema_ref` is present → **external file** mode.
3. Else if `auto_schema` is present (any value) → explicit auto mode (see §6.2 for `true`/`strict`/`permissive`).
4. Else → **implicit auto mode**, equivalent to `auto_schema: true`.

Specifying multiple modes in the same entry (e.g., `auto_schema` + `input_schema`) is a parse-time error: `BindingSchemaModeConflictError`.

If auto mode (explicit or implicit) cannot infer a schema (e.g., target has no detectable type info), parse-time error: `BindingSchemaInferenceFailedError`. The error message names the mode chosen and the inference attempts made.

### 3.5 `annotations` field

Source: `#/$defs/Annotations` in `binding.schema.json`. **MUST be kept structurally identical to** `#/$defs/Annotations` in `module-meta.schema.json` (same module-annotation contract).

**Core behavior hints**:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `readonly` | boolean | `false` | No side effects. |
| `destructive` | boolean | `false` | Performs destructive operations. |
| `idempotent` | boolean | `false` | Safe to retry. |
| `requires_approval` | boolean | `false` | Human approval required. |
| `open_world` | boolean | `true` | Interacts with external systems. |

**Capabilities**:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `streaming` | boolean | `false` | Supports streaming output. |
| `cacheable` | boolean | `false` | Output cacheable. |
| `cache_ttl` | integer | `0` | Cache duration (seconds); only meaningful when `cacheable: true`. |
| `cache_key_fields` | array of strings \| null | `null` | Input fields determining cache key (`null` = all inputs). |
| `paginated` | boolean | `false` | Returns paginated results. |
| `pagination_style` | string | `"cursor"` | `cursor` / `offset` / `page` / custom. |

**Open extension**:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `extra` | object | `{}` | Vendor / ecosystem metadata. Keys SHOULD use `<namespace>.<name>` (e.g., `mcp.category`). `core.*` namespace reserved. Wire format per `PROTOCOL_SPEC §4.4.1`. |

All three SDKs MUST round-trip every field onto the registered module's `annotations`.

### 3.6 `display` field (surface overlay)

Source: `#/$defs/DisplayOverlay` in `binding.schema.json`.

Top-level display keys: `alias`, `description`, `documentation`, `guidance`, `tags`.

Per-surface overrides — the schema uses **`patternProperties`** with key pattern `^[a-z][a-z0-9_]*$`, so any new surface name is allowed without schema bump. Each surface override conforms to `SurfaceOverride` (`alias`, `description`, `guidance`).

Surfaces with hard technical constraints (1.0 ships with):
- `display.cli` — alias must be shell-safe (`^[a-z][a-z0-9_-]*$`).
- `display.mcp` — alias maxLength 64 (OpenAI function name spec).
- `display.a2a` — alias accepts natural language.

Future surfaces (e.g., `display.graphql`) added via `patternProperties` without schema changes; SDKs that don't integrate with a surface MUST still preserve its `display.<surface>` field round-trip without error.

Per-surface override resolution (most specific wins on a given surface):

```
display.<surface>.<field>  >  display.<field>  >  top-level binding field
```

### 3.7 Target resolution per SDK

| SDK | Mechanism | Format examples | Method binding |
|---|---|---|---|
| Python | `importlib.import_module()` + `getattr` | `"pkg.module:func"`, `"pkg.module:Class.method"` | ✅ instantiates class no-args |
| TypeScript | `await import()` (ESM, async) | `"./relative:func"`, `"@scope/pkg:Class.method"` | ✅ `new cls()` no-args |
| Rust | Handler-map lookup (user pre-registers) | `"key:func"`, `"key:Class.method"` (string is opaque) | ✅ user closure captures instance/method binding as needed |

**Rust caveat**: handler-map keys are opaque strings; no compile-time linking. The user supplies a `HashMap<String, BindingHandler>` with handlers keyed by the `target` string:

```rust
let handlers = HashMap::from([
    ("format_date:format_date_string".to_string(),
     Box::new(|input, ctx| { /* ... */ }) as BindingHandler),
    ("format_date:Service.format".to_string(),
     Box::new(|input, ctx| {
         let svc = Service::new();
         svc.format(input)
     }) as BindingHandler),
]);
BindingLoader::new().load_from_yaml("bindings.yaml", &registry, handlers)?;
```

The YAML file is identical across all three SDKs.

---

## 4. Pipeline Config YAML

Lives inside `apcore.yaml` under `pipeline:`. Source of truth: `apcore/schemas/apcore-config.schema.json` (pipeline section added in 0.19.0).

### 4.1 File structure

```yaml
spec_version: "1.0"
pipeline:
  remove:
    - "audit"
  configure:
    acl_check:
      ignore_errors: false
      timeout_ms: 5000
  steps:
    - name: "rate_limit"
      type: "rate_limit"
      config:
        max_per_minute: 60
      match_modules: ["api.*"]
      pure: false
      timeout_ms: 2000
      after: "acl_check"
    - name: "custom_audit"
      handler: "my_app.steps:CustomAuditStep"
      config:
        log_level: "INFO"
      before: "call"
```

### 4.2 `pipeline` section keys

| Key | Type | Required | Description |
|---|---|---|---|
| `remove` | list of strings | no | Built-in step names to remove. |
| `configure` | map (string → object) | no | Field overrides for existing steps. |
| `steps` | list | no | Custom step insertions. |

### 4.3 Step entry fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | **yes** | Unique step instance name. Length governed by `apcore.validation.pipeline.step_name_max_length` (default 64). |
| `type` | string | one of `type`/`handler` | Key into the step-type registry. |
| `handler` | string | one of `type`/`handler` | `"module:Class"` reference for dynamic import. |
| `config` | object | no | Constructor arguments for the step factory. |
| `match_modules` | list of glob strings | no | Restrict step to matching modules. |
| `ignore_errors` | boolean | no | Default `false`. |
| `pure` | boolean | no | Default `false`. |
| `timeout_ms` | integer | no | Default `0` (no timeout). Max governed by `apcore.validation.pipeline.timeout_ms_max` (default 300000). |
| `after` | string | one of `after`/`before` | Insert after named step. |
| `before` | string | one of `after`/`before` | Insert before named step. |

Specifying both `type` and `handler`, or both `after` and `before`, is parse-time error.

### 4.4 Resolution semantics per SDK

| SDK | `type:` (registry) | `handler:` (dynamic) | Metadata fields |
|---|---|---|---|
| Python | ✅ `register_step_type()` | ✅ `importlib.import_module` | ✅ honored |
| TypeScript | ✅ `registerStepType()` | ✅ `await import()` (**0.19.0 NEW** — was incorrectly rejected) | ✅ honored |
| Rust | ✅ `register_step_type()` | ❌ parse-time `PipelineHandlerNotSupportedError` (compiled language; cannot dynamically load) | ✅ honored (**0.19.0 NEW** — previously silently dropped) |

---

## 5. Entry Point Meta

Source: `apcore/schemas/module-meta.schema.json`, `entry_point` field.

### 5.1 Syntax

Within a module's manifest:

```yaml
entry_point: "<file_stem>:<ClassName>"
```

Canonical pattern:

```
^[a-z][a-z0-9_]*:[A-Z][a-zA-Z0-9]*$
```

### 5.2 Semantics per SDK

| SDK | File loading | Class detection | When meta is used |
|---|---|---|---|
| Python | `importlib.util.spec_from_file_location` + `exec_module` (sync) | Duck-type: `input_schema`, `output_schema` (Pydantic models), callable `execute` | Runtime `discover_default()` |
| TypeScript | `await import(filePath)` (async) | Duck-type: `inputSchema`, `outputSchema`, callable `execute`; default export preferred | Runtime `discoverDefault()` |
| Rust | Naming-convention parse only; **no runtime file loading** (build-time codegen / `build.rs` consumes `entry_point`) | N/A | Build time |

**TypeScript duck-type field-name note**: the runtime check looks for `inputSchema` / `outputSchema` (camelCase, JS-native) on the loaded class. The YAML / manifest `entry_point` string itself remains snake_case. Not an inconsistency — different layers.

**Rust runtime constraint**: Rust SDK currently exposes no API path that would attempt runtime file loading. The constraint is permanent (compiled language, no in-process module reload). However, any future Rust SDK feature that opts users into runtime loading — for example `libloading`-based plugin discovery, or a deliberate `Registry::register_from_meta_runtime()` API — MUST raise `EntryPointRuntimeUnsupportedError` (§7.1) when the operation is unsupported by the build target. The error name is reserved in the canonical hierarchy now to ensure cross-SDK error catalog symmetry and prevent ad-hoc renaming when the feature is added.

---

## 6. `auto_schema` — Cross-SDK Semantics

### 6.1 Universal principle

`auto_schema` means "derive the JSON Schema for the target's input/output from the target's type declarations; do not require the user to duplicate the schema in YAML." It is the **default behavior** when no schema mode is specified (§3.4).

### 6.2 Three values

| Value | Meaning |
|---|---|
| `auto_schema: true` | **Permissive** mode. Schema reflects native types as-is; nullable fields, optional properties, `additionalProperties: true` allowed. |
| `auto_schema: permissive` | Synonym of `true`; explicit form preferred for clarity. |
| `auto_schema: strict` | OpenAI / Anthropic strict-schema-compatible: `additionalProperties: false`, all properties marked `required`, restricted type set (string, number, boolean, object, array, null). Unsupported native types (datetime, decimal, etc.) parse-time error: `BindingStrictSchemaIncompatibleError`. |

Implicit (no field) defaults to `true` (permissive).

### 6.3 Per-SDK implementation

| SDK | Source of types | Mechanism |
|---|---|---|
| Python | Function type hints | `inspect.signature()` → Pydantic model → `model_json_schema()` |
| TypeScript | TypeBox / zod / class-validator DTO / typia | Multi-adapter chain (§6.4) |
| Rust | Structs with `#[derive(JsonSchema, BindingSchema)]` | Inventory-backed lookup (§6.5) |

### 6.4 TypeScript adapter chain

`auto_schema` triggers an ordered detection chain:

1. **TypeBox adapter** — input/output is a TypeBox schema object → use directly.
2. **Zod adapter** — input/output is a zod schema → convert via `zod-to-json-schema`.
3. **DTO adapter** — input is a class decorated with `class-validator` + `@ApiProperty` → extract via reflect-metadata.
4. **typia adapter** (opt-in via `auto_schema: typia` or registry priority) — true type-driven inference; requires `ts-patch` build setup.
5. **No match** → `BindingSchemaInferenceFailedError` with message listing attempted adapters.

Custom adapters via `SchemaExtractorRegistry.register(adapter, priority?)`. The `nestjs-apcore` package bundles a class-validator adapter that integrates with `@nestjs/swagger`.

### 6.5 Rust auto_schema

Implemented through the new `apcore-macros` crate (introduced in 0.19.0):

```rust
use apcore::{BindingSchema, module};
use schemars::JsonSchema;

#[derive(JsonSchema, BindingSchema)]
struct FormatDateInput { date_string: String, output_format: String }

#[derive(JsonSchema, BindingSchema)]
struct FormatDateOutput { formatted: String }

// Plain function + YAML binding (auto_schema implicit)
fn format_date(input: FormatDateInput) -> Result<FormatDateOutput, ModuleError> { /* ... */ }

// OR: attribute macro declares the module directly (no YAML required)
#[module(id = "utils.format_date", auto_schema)]
fn format_date(input: FormatDateInput) -> Result<FormatDateOutput, ModuleError> { /* ... */ }
```

`#[derive(BindingSchema)]` registers the type's schema into a compile-time `inventory`-backed lookup table. `auto_schema` resolves via this table at YAML load time.

### 6.6 Failure modes

| Condition | Error |
|---|---|
| Auto mode but target has no usable type info | parse-time `BindingSchemaInferenceFailedError` |
| Auto mode combined with `input_schema` / `output_schema` / `schema_ref` | parse-time `BindingSchemaModeConflictError` |
| `auto_schema: strict` and inferred schema contains incompatible features | parse-time `BindingStrictSchemaIncompatibleError` |

---

## 7. Error Model

### 7.1 Canonical error names

All three SDKs expose these errors with name parity (allowing language-appropriate casing — e.g., Rust enum variant `BindingFileInvalid`):

**Bindings**:

| Error | Phase | Trigger |
|---|---|---|
| `BindingFileInvalidError` | parse | YAML parse failure, missing required top-level key, malformed structure |
| `BindingInvalidTargetError` | parse | `target` violates pattern, missing `:`, contains traversal segments |
| `BindingModuleNotFoundError` | parse | `target` module cannot be resolved |
| `BindingCallableNotFoundError` | parse | Symbol does not exist in module |
| `BindingNotCallableError` | parse | Symbol is not callable |
| `BindingSchemaModeConflictError` | parse | Multiple schema modes specified (1.0 NEW) |
| `BindingSchemaInferenceFailedError` | parse | Auto mode failed to infer (1.0 NEW) |
| `BindingStrictSchemaIncompatibleError` | parse | `auto_schema: strict` and incompatible feature found (1.0 NEW) |

**Pipeline config**:

| Error | Phase | Trigger |
|---|---|---|
| `PipelineConfigInvalidError` | parse | Malformed `pipeline:` section |
| `PipelineStepNotFoundError` | parse | Unregistered `type`, or `after`/`before` names a non-existent step |
| `PipelineHandlerNotSupportedError` | parse | `handler:` used on Rust |
| `PipelineStepInsertionAmbiguousError` | parse | Neither `after` nor `before` specified |

**Entry point**:

| Error | Phase | Trigger |
|---|---|---|
| `EntryPointNotFoundError` | parse | Class named in `entry_point` not found in file |
| `EntryPointAmbiguousError` | parse | Auto-inference found multiple candidates and no override |
| `EntryPointRuntimeUnsupportedError` | resolve | Rust-only; raised by any future opt-in runtime file-loading API (e.g., plugin discovery via `libloading`). Reserved in 1.0; no current API path raises it. |

### 7.2 Exact error message templates

SDKs MUST produce these exact strings (with placeholder substitution). Cross-SDK conformance tests in `apcore/conformance/` assert byte-for-byte match.

```
BindingFileInvalidError:
  template: "{file_path}: {reason}"
  example:  "bindings.yaml: missing required top-level key 'bindings'"

BindingInvalidTargetError:
  template: "{file_path}:{line}: target '{value}' is invalid: {reason}. See DECLARATIVE_CONFIG_SPEC.md §2.2"
  example:  "bindings.yaml:5: target '../etc/passwd:fn' is invalid: path traversal not allowed. See DECLARATIVE_CONFIG_SPEC.md §2.2"

BindingModuleNotFoundError:
  template: "{file_path}:{line}: cannot resolve module '{module_path}' for binding '{module_id}'"

BindingCallableNotFoundError:
  template: "{file_path}:{line}: callable '{symbol}' not found in module '{module_path}' for binding '{module_id}'"

BindingNotCallableError:
  template: "{file_path}:{line}: target '{value}' resolved to a non-callable for binding '{module_id}'"

BindingSchemaModeConflictError:
  template: "{file_path}:{line}: binding '{module_id}' specifies multiple schema modes ({modes_listed}). Choose one. See DECLARATIVE_CONFIG_SPEC.md §3.4"

BindingSchemaInferenceFailedError:
  template: "{file_path}:{line}: binding '{module_id}' auto schema inference failed for target '{target}'. {language_specific_remediation}. See DECLARATIVE_CONFIG_SPEC.md §6"

BindingStrictSchemaIncompatibleError:
  template: "{file_path}:{line}: binding '{module_id}' uses auto_schema: strict but inferred schema contains incompatible features: {features_listed}. See DECLARATIVE_CONFIG_SPEC.md §6.2"

PipelineHandlerNotSupportedError:
  template: "{file_path}:{line}: pipeline step '{step_name}' uses 'handler:' which is not supported in {sdk_name}. Use 'type:' with {register_function_name}(). See DECLARATIVE_CONFIG_SPEC.md §4.4"

PipelineStepInsertionAmbiguousError:
  template: "{file_path}:{line}: pipeline step '{step_name}' specifies neither 'after' nor 'before'. See DECLARATIVE_CONFIG_SPEC.md §4.3"

EntryPointRuntimeUnsupportedError:
  template: "runtime entry-point loading is not supported in apcore-rust on this build target ({reason}). Register the module type explicitly via Registry, or build with the feature enabling runtime loading. See DECLARATIVE_CONFIG_SPEC.md §5.2"
```

`{language_specific_remediation}` template values per SDK are listed in Appendix C.

### 7.3 Error class hierarchy

All binding-related errors inherit from a `BindingError` base; pipeline errors from `PipelineConfigError`; entry-point errors from `EntryPointError`. SDKs may add language-specific intermediate base classes but MUST preserve direct catchability of the canonical names.

---

## 8. SDK Implementation Matrix

| Feature | Python | TypeScript | Rust |
|---|---|---|---|
| **Bindings — core** |
| `bindings:` top-level list | ✅ | ✅ | ✅ (**0.19.0**: migrated from `- name:` flat list) |
| `module_id` field | ✅ | ✅ | ✅ (**0.19.0**: renamed from `name`) |
| `target: "mod:sym"` string | ✅ | ✅ | ✅ (**0.19.0**: replaced object form) |
| `target: "mod:Class.method"` | ✅ | ✅ | ✅ (**0.19.0**: opaque handler-map keys) |
| `description` / `documentation` / `tags` / `version` | ✅ | ✅ | ✅ |
| `annotations` | ✅ | ✅ | ✅ (**0.19.0**: align field names) |
| `display` overlay | ✅ | ✅ | ✅ (**0.19.0 NEW** in Rust) |
| `display.<custom_surface>` via patternProperties | ✅ | ✅ | ✅ (**0.19.0 NEW**) |
| `metadata` free-form | ✅ | ✅ | ✅ |
| **Bindings — schema modes** |
| `input_schema` + `output_schema` | ✅ | ✅ | ✅ |
| `schema_ref` (external file) | ✅ | ✅ | ✅ (**0.19.0**: previously parsed but not loaded) |
| `auto_schema: true` (or implicit default) | ✅ via type hints | ✅ (**0.19.0 NEW**) via adapter chain | ✅ (**0.19.0 NEW**) via `apcore-macros` |
| `auto_schema: strict` | ✅ (**0.19.0 NEW**) | ✅ (**0.19.0 NEW**) | ✅ (**0.19.0 NEW**) |
| `auto_schema: permissive` | ✅ (**0.19.0 NEW**) | ✅ (**0.19.0 NEW**) | ✅ (**0.19.0 NEW**) |
| **Pipeline config** |
| `remove` / `configure` / `steps` | ✅ | ✅ | ✅ |
| `type:` (registered) | ✅ | ✅ | ✅ |
| `handler:` (dynamic import) | ✅ | ✅ (**0.19.0 NEW** — was rejected) | ❌ parse-time error (language limit) |
| `match_modules` / `ignore_errors` / `pure` / `timeout_ms` | ✅ | ✅ | ✅ (**0.19.0 NEW** — previously silently dropped) |
| **Entry point** |
| Runtime file loading | ✅ sync | ✅ async | ❌ build-time only |
| `entry_point: "file:Class"` override | ✅ | ✅ | ✅ (parse only) |
| Auto single-class inference | ✅ | ✅ (default export preferred) | N/A |
| **Configurable policy** |
| Reads `apcore.validation.*` from apcore.yaml | ✅ (**0.19.0 NEW**) | ✅ (**0.19.0 NEW**) | ✅ (**0.19.0 NEW**) |

### 8.1 Migration notes (0.18.x → 0.19.0)

**Rust users — `bindings.yaml` format change** (BREAKING):

Old:
```yaml
- name: "utils.format_date"
  target: { module_name: "utils.format_date", callable: "format_date:format_date_string" }
  metadata:
    description: "..."
    input_schema: { ... }
```

New canonical:
```yaml
bindings:
  - module_id: "utils.format_date"
    target: "format_date:format_date_string"
    description: "..."
    input_schema: { ... }
```

Migration script: `apcore-rust/scripts/migrate-binding-yaml-0.19.sh`.

**Python users — implicit auto_schema fallback** (BEHAVIOR CHANGE):

In 0.18.x and earlier, omitting all schema fields in a Python binding silently defaulted to type-hint-based inference. In 0.19.0 this behavior is now **uniform across all three SDKs** and elevated to spec-defined behavior (§3.4 step 4). The user-visible difference:

- 0.18.x: silent fallback only on Python; TS/Rust would reject
- 0.19.0: implicit auto-processing on all three SDKs; if inference fails on TS/Rust, clear `BindingSchemaInferenceFailedError` with remediation

**TS users — `handler:` in pipeline now works**: previously rejected at resolve-time. Now functional via `await import()`.

**Rust users — pipeline step metadata now honored**: review existing `apcore.yaml` for unintended values in `match_modules`, `ignore_errors`, `pure`, `timeout_ms`.

---

## 9. Configurable Policy Limits

### 9.1 Principle

Soft / UX limits live in `apcore.yaml` under `apcore.validation.*`. Hard / technical limits stay in JSON Schema.

| Limit type | Where | Example |
|---|---|---|
| Filesystem-safety | JSON Schema (hard) | `module_id` maxLength 192 |
| External-spec compliance | JSON Schema (hard) | `display.mcp.alias` maxLength 64 (OpenAI) |
| Shell-safety | JSON Schema (hard) | `display.cli.alias` pattern |
| UI / readability | `apcore.yaml` (soft) | `description` max length |
| Format normalization | `apcore.yaml` (soft) | `tags` pattern |

### 9.2 Catalog (1.0)

```yaml
apcore:
  validation:
    binding:
      description_max_length: 500          # null = unlimited
      documentation_max_length: null       # null = unlimited
      tags_pattern: "^[a-z][a-z0-9_]*$"   # null = no constraint
      version_require_semver: true
    pipeline:
      step_name_max_length: 64             # null = unlimited
      timeout_ms_max: 300000               # null = unlimited (5 min default cap)
```

### 9.3 Defaults

If `apcore.validation.*` is absent from `apcore.yaml`, the defaults shown above apply. Users opt out of any single limit by setting it to `null`.

### 9.4 Schema location

The `validation` section is added to `apcore/schemas/apcore-config.schema.json` under `properties.apcore.properties.validation` in 0.19.0.

### 9.5 Violation behavior

> **Status (D9-001..003).** Configurable policy enforcement is **deferred**.
> The `BindingPolicyViolationError` / `PipelineConfigPolicyViolationError`
> classes were declared but never raised in any SDK; the corresponding
> exports have been removed in 0.21.0. When policy enforcement lands, this
> section will be updated together with re-introduced error classes,
> normative parse-time enforcement steps, and conformance fixtures.
>
> Until then, fields are bounded only by the hard schema limits documented
> in PROTOCOL_SPEC §2.7 and the JSON-Schema validation produced by each
> binding's `auto_schema` / `manual_schema` mode.

---

## 10. JSON Schema Files

| Surface | File | Status |
|---|---|---|
| Bindings | `apcore/schemas/binding.schema.json` | Exists. **0.19.0 changes**: relax `target` regex (§2.2); change `display` to `patternProperties`; remove soft maxLength/pattern that move to apcore.yaml policy. |
| Pipeline | `apcore/schemas/apcore-config.schema.json` | **0.19.0 NEW**: pipeline section added; `validation` section added. |
| Entry point | `apcore/schemas/module-meta.schema.json` | Exists. No structural change. |

Each SDK's test suite MUST validate its example YAML fixtures against these schemas. Cross-SDK conformance tests under `apcore/conformance/fixtures/` share the same fixture files.

---

## 11. Changelog (of this spec)

| Version | Date | Change |
|---|---|---|
| 1.0 | 2026-04-17 | Initial unified specification. Defines cross-SDK YAML parity for bindings, pipeline config, and entry-point meta. Implicit auto_schema as default; `auto_schema: true \| strict \| permissive` values; relaxed target regex for TS/Rust paths; `display` surfaces extensible via `patternProperties`; soft limits moved to `apcore.yaml` policy; exact cross-SDK error message templates; `spec_version` field with deprecation warning toward 1.1 mandatory; TS pipeline `handler:` functional via `await import()`; Rust method-form target via opaque handler keys; Rust pipeline metadata fields honored; `EntryPointRuntimeUnsupportedError` reserved for future Rust runtime loading APIs. |

---

## Appendix A: Non-goals

- **Hot reload semantics** of binding files — handled by registry layer.
- **Module ID format** — governed by `PROTOCOL_SPEC §2.7`.
- **Wire protocol for module invocation** — governed by `PROTOCOL_SPEC`.

## Appendix B: Open questions for 1.1

- Should `bindings` entries support `depends_on` for registration ordering?
- Should `schema_ref` support remote URLs (HTTPS) with caching policy?
- Should `pipeline.steps[*]` support conditional inclusion (`when:` env interpolation)?
- Should `display.<surface>` support deeper override structures beyond `SurfaceOverride`?

## Appendix C: Per-SDK remediation message templates

`{language_specific_remediation}` substitution table for `BindingSchemaInferenceFailedError`:

**Python**:
```
target function lacks complete type hints. Add type annotations to all parameters and the return type, or specify input_schema/output_schema explicitly.
```

**TypeScript**:
```
no schema source detected. Provide one of: target input/output as TypeBox schemas, zod schemas, class-validator-decorated DTO, or use the typia adapter (auto_schema: typia). Alternatively specify input_schema/output_schema explicitly.
```

**Rust**:
```
target type does not implement BindingSchema. Add #[derive(JsonSchema, BindingSchema)] to the input and output struct types, or specify input_schema/output_schema explicitly.
```
