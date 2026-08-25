---
description: "The canonical, normative apcore protocol specification (RFC 2119, v1.15.0): module, schema, naming, ACL, approval, error, config, and observability requirements for all conforming SDKs."
---

# apcore — AI-Perceivable Core Standard Specification

> **Canonical Specification** - This document is the authoritative specification for the apcore protocol

> Version: 1.15.0
> Status: Draft Specification (RFC 2119 Conformant)
> Stability: Specification content is stable, pending reference implementation verification
> Last Updated: 2026-08-25

---

### Table of Contents

- [1. Overview](#1-overview)
- [2. Naming Specification](#2-naming-specification)
- [3. Directory Specification](#3-directory-specification)
- [4. Schema Specification](#4-schema-specification)
- [5. Module Specification](#5-module-specification)
- [6. ACL Specification](#6-acl-specification)
  - [6.6 System Module Permissions](#66-system-module-permissions)
- [7. Approval System](#7-approval-system)
- [8. Error Handling Specification](#8-error-handling-specification)
- [9. Configuration Specification](#9-configuration-specification)
  - [9.4 Config Bus Architecture](#94-config-bus-architecture)
  - [9.5 Namespace Registration](#95-namespace-registration)
  - [9.6 Unified Configuration File](#96-unified-configuration-file)
  - [9.7 Mount Mechanism](#97-mount-mechanism)
  - [9.8 Environment Variable Override (Namespace Mode)](#98-environment-variable-override-namespace-mode)
  - [9.9 Namespace-Aware Access API](#99-namespace-aware-access-api)
  - [9.10 Validation Algorithm (Namespace-Aware A12-NS)](#910-validation-algorithm-namespace-aware-a12-ns)
  - [9.11 Hot-Reload (Namespace Mode)](#911-hot-reload-namespace-mode)
  - [9.12 Cross-Language Implementation Requirements](#912-cross-language-implementation-requirements)
  - [9.13 Ecosystem Integration Patterns](#913-ecosystem-integration-patterns)
  - [9.14 Config Discovery (Optional)](#914-config-discovery-optional)
- [10. Observability Specification](#10-observability-specification)
- [11. Extension Mechanism](#11-extension-mechanism)
- [12. SDK Implementation Guide](#12-sdk-implementation-guide)
- [13. Versioning](#13-versioning)
- [14. Appendix](#14-appendix)
- [Revision History](#revision-history)

---

## 1. Overview

### 1.1 Project Positioning

apcore (AI-Perceivable Core) is a **governed, protocol-neutral runtime and module standard for agent-callable application capabilities**.

**One-sentence definition**:
> apcore defines and enforces the schemas, behavioral metadata, access rules, approval gates, and execution lifecycle of a capability before adapters expose it to MCP, A2A, CLI, HTTP, or direct code.

**Positioning**:
- **Governed capability runtime**: apcore validates and governs application capabilities at the execution boundary
- **Protocol-neutral definition**: a capability is defined once and projected through independently versioned adapters
- **Complementary to MCP/A2A**: transport protocols define how peers communicate; apcore defines what is executed and which runtime rules are enforced
- **Cross-language contract**: Python, TypeScript, and Rust SDKs implement the same normative specification

### 1.2 Core Principles

| Principle | Description |
|------|------|
| **Schema-driven** | All modules enforce definition of `input_schema` / `output_schema` / `description` |
| **Three-layer Metadata** | Core (enforced Schema) + Annotations (behavior Annotations) + Extensions (free metadata) |
| **Directory as ID** | Directory path automatically maps to module ID, zero manual configuration |
| **Agent-readable** | Schema + annotations give agents structured capability metadata without promising model comprehension |
| **Protocol-neutral** | Adapters can project the same governed capability to code, MCP, A2A, CLI, or HTTP surfaces |

### 1.3 Design Goals

- **Protocol neutrality**: Modules can be exposed through code, MCP, A2A, CLI, HTTP, and future adapters
- **Agent readability**: Enforced schemas make capability contracts machine-readable and independently validatable
- **Developer Experience**: Directory as ID, zero configuration, automatic discovery
- **Cross-language**: Specification supports implementation in any programming language, Python as reference implementation
- **Extensibility**: ACL, middleware, observability

### 1.4 Relationship with MCP/A2A

apcore is a **module construction specification**, MCP/A2A is a **communication protocol**. They are complementary:

```
┌─────────────────────────────────────────────────────────────┐
│              apcore — AI-Perceivable Core                    │
│                                                             │
│  Solves: How to develop AI-Perceivable modules              │
│  - Directory as ID / Schema-driven / ACL / Observability / Middleware │
└─────────────────────────────────────────────────────────────┘
                           ↓ Modules can be exposed as
┌─────────────┬─────────────┬─────────────┬─────────────────┐
│ MCP Server  │  HTTP API   │  CLI Tool   │  Direct Code    │
│ (Clients)   │  (Adapters) │  (Terminal) │  (Native SDK)   │
└─────────────┴─────────────┴─────────────┴─────────────────┘
```

**apcore focuses on "how to build AI-perceivable modules", MCP focuses on "how to call tools".**

### 1.5 Specification Keywords

Keywords in this document are interpreted according to [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) definitions.

| English | Chinese | Meaning |
|------|------|------|
| **MUST** / **REQUIRED** / **SHALL** | Must | Absolute requirement, non-compliance means non-conformance |
| **MUST NOT** / **SHALL NOT** | Forbidden | Absolutely prohibited |
| **SHOULD** / **RECOMMENDED** | Should | Strongly recommended, can deviate only with sufficient reason |
| **SHOULD NOT** / **NOT RECOMMENDED** | Should Not | Strongly discouraged, unless there's sufficient reason |
| **MAY** / **OPTIONAL** | May | Completely optional, implementer decides |

When this document uses bolded forms of the above keywords, their semantics strictly follow RFC 2119 definitions. Normal font "must", "should", etc. do not have normative meaning.

### 1.6 Term Definitions

Core terms used in this specification are defined as follows:

| Term | English | Definition |
|------|------|------|
| Module | Module | Basic execution unit of apcore, encapsulates a function, **MUST** define `input_schema`, `output_schema`, and `description` (≤200 characters). Can optionally define `documentation` (≤5000 characters) to provide detailed documentation |
| Schema | Schema | Data structure definition based on JSON Schema Draft 2020-12, used for validating input/output and for AI/LLM understanding |
| Canonical ID | Canonical ID | Globally unique identifier for a module, automatically generated from directory path, format is dot-separated snake_case (e.g., `executor.email.send_email`) |
| Registry | Registry | Core component responsible for module discovery, registration, loading, and management |
| Executor | Executor | Core component responsible for module invocation execution, handles Schema validation, ACL checking, middleware dispatching |
| Context | Context | Runtime context object during module execution, carries trace_id, call chain, identity information, and shared state |
| Access Control List | ACL | Set of rules defining inter-module invocation permissions, based on caller_id/target_id pattern matching |
| Middleware | Middleware | Interceptor running before and after module execution, executes in onion model, can modify input/output |
| Extension Point | Extension Point | Replaceable component interface provided by framework (e.g., SchemaLoader, ModuleLoader), allows custom implementation |
| Annotations | Annotations | Module-level behavior metadata (readonly, destructive, etc.), helps AI/LLM make invocation decisions |
| Metadata | Metadata | Completely open key-value dictionary for storing extension information, framework does not validate its content |
| Entry Point | Entry Point | Code entry location of module, format is `filename:ClassName`, can be auto-inferred or manually configured |
| Call Chain | Call Chain | Complete list of module ID paths from root invocation to current invocation, used for loop detection and depth limiting |
| Trace ID | Trace ID | Identifier uniquely identifying a complete invocation chain, **MUST** be 32-char lowercase hex (W3C Trace Context format) |
| Identity | Identity | Structured expression of caller_id identity (user/service/Agent/API Key/system), ACL engine depends on it |

### 1.7 API Naming Conventions

apcore public API uses concise universal names (e.g., `Module`, `Context`, `Registry`),
relying on language-native namespace mechanisms to avoid conflicts:

| Language | Namespace Isolation Method | Example |
|------|----------------|------|
| Python | Package import | `from apcore import Module` / `import apcore` |
| Go | Package name qualification | `apcore.Module(...)` |
| Rust | Module path | `apcore::Module` |
| TypeScript | Module import | `import { Module } from 'apcore-js'` |
| Java | Package path | `import com.apcore.Module` |

Implementations **MUST** follow these naming rules:
- In languages with namespace mechanisms, **MUST NOT** add redundant prefixes to public APIs
- In languages without namespace mechanisms, **MUST** use `apcore_` prefix
- Error types **SHOULD** be prefixed with their domain (e.g., `ModuleError`, `SchemaValidationError`)

---

## 2. Naming Specification

### 2.1 Directory as ID (Core Rule)

**Directory path is the single source of truth for module IDs**. IDs are automatically generated from directory paths, zero configuration.

Implementations **MUST** convert directory paths to Canonical IDs according to the following algorithm:

```
Algorithm: directory_to_canonical_id(file_path, extensions_root)

Input:
  file_path      — Complete relative path of module file (e.g., "extensions/executor/validator/db_params.py")
  extensions_root — Extension root directory name (default "extensions")

Output:
  canonical_id   — Dot-separated module ID (e.g., "executor.validator.db_params")

Preconditions:
  - file_path must start with extensions_root + "/"
  - file_path must contain file extension

Steps:
  1. relative_path ← Remove extensions_root + "/" prefix from file_path
  2. relative_path ← Remove file extension (last "." and everything after it)
  3. segments ← Split relative_path by "/"
  4. For each segment, perform validation:
     a. If segment is empty string → Throw INVALID_PATH error
     b. If segment doesn't match /^[a-z][a-z0-9_]*$/ → Throw INVALID_SEGMENT error
  5. canonical_id ← Join all segments with "."
  6. If len(canonical_id) > 192 → Throw ID_TOO_LONG error
  7. Return canonical_id

Complexity: O(n), where n is the number of characters in the path
```

```yaml
directory_to_id:
  # Rule: Remove extensions/ prefix and file extension, convert path separator to dot
  rule: "extensions/{path}/{name}.{ext} → {path}.{name}"

  # Examples
  examples:
    - file: "extensions/executor/validator/db_params.py"
      id: "executor.validator.db_params"

    - file: "extensions/api/handler/task_submit.py"
      id: "api.handler.task_submit"

    - file: "extensions/orchestrator/engine/task_flow.py"
      id: "orchestrator.engine.task_flow"

  # ID format constraints
  format:
    pattern: "^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)*$"
    separator: "."
    case: "snake_case"
    max_length: 192
```

#### 2.1.1 Multi-Class Discovery (Opt-In)

Implementations **MAY** support multi-class discovery mode, which allows multiple module classes to be defined in a single file. Multi-class discovery **MUST** be explicitly enabled — the default (single-class) behavior **MUST** remain unchanged.

When multi-class discovery is enabled for a file:

1. The scanner **MUST** enumerate all exported classes in the file.
2. For each class that implements the Module interface, derive its ID:
   - `base_id` ← `directory_to_canonical_id(file_path, extensions_root)`   [Algorithm A01]
   - `class_segment` ← `snake_case(ClassName)`
   - `module_id` ← `base_id + "." + class_segment`
3. The resulting `module_id` **MUST** conform to the Canonical ID grammar (§2.7).
4. If two classes in the same file produce the same `class_segment`, implementations **MUST** raise `MODULE_ID_CONFLICT`.
5. A file with exactly one Module class **MUST** produce the same ID whether multi-class mode is on or off (backward compatibility guarantee).

`snake_case` conversion: replace non-alphanumeric characters with `_`, lowercase all, collapse consecutive `_` to one, strip leading/trailing `_`.

Examples:
```
class Addition   → "addition"
class MathOps    → "math_ops"
class HTTPSender → "http_sender"
```

#### Module ID Format Constraint

All module IDs **MUST** conform to the following regular expression:

```
^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$
```

**Allowed characters:**
- Lowercase ASCII letters (`a-z`)
- Digits (`0-9`)
- Underscores (`_`)
- Dots (`.`) as namespace separators only

**Explicitly prohibited:**
- Hyphens (`-`) — reserved for MCP/OpenAI tool name normalization (dot→hyphen conversion must be bijective)
- Uppercase letters
- Spaces, special characters

All SDK implementations **MUST** validate module IDs against this pattern during `register()`. Invalid IDs **MUST** be rejected with `INVALID_MODULE_ID` error (the dedicated module-ID validation code defined in the error registry below and mandated by `docs/features/error-system.md` §Error Code Constants; see also the `register()` and `Executor.call` contracts in `docs/features/registry-system.md` and `docs/features/core-executor.md`).

### 2.2 ID Map (Cross-language Conversion)

**ID Map** module handles cross-language ID conversion, supporting automatic recognition and manual configuration. Implementations **MUST** support canonical conversion from various language native formats to Canonical ID.

```
Algorithm: normalize_to_canonical_id(local_id, language)

Input:
  local_id  — Language-native format ID (e.g., Rust's "executor::validator::db_params")
  language  — Source language identifier (python | rust | go | java | typescript)

Output:
  canonical_id — Dot-separated snake_case format Canonical ID

Steps:
  1. Determine separator sep based on language:
     - python: "."  |  rust: "::"  |  go: "."  |  java: "."  |  typescript: "."
  2. segments ← Split local_id by sep
  3. For each segment, perform case normalization:
     - If segment is PascalCase → Convert to snake_case
     - If segment is camelCase → Convert to snake_case
     - If segment is already snake_case → Keep unchanged
  4. canonical_id ← Join all segments with "."
  5. Validate canonical_id conforms to ID EBNF syntax (see §2.7)
  6. Return canonical_id
```

```yaml
# apcore.yaml

id_map:
  # Auto-detection: Determine language by file extension
  auto_detect: true

  # Built-in language conversion rules
  languages:
    python:
      extensions: [".py"]
      separator: "."
      file_case: "snake_case"
      class_case: "PascalCase"
      example:
        id: "executor.validator.db_params"
        file: "executor/validator/db_params.py"
        class: "DbParamsValidator"

    rust:
      extensions: [".rs"]
      separator: "::"
      file_case: "snake_case"
      struct_case: "PascalCase"
      example:
        id: "executor.validator.db_params"
        file: "executor/validator/db_params.rs"
        local_id: "executor::validator::db_params"
        struct: "DbParamsValidator"

    go:
      extensions: [".go"]
      separator: "."
      file_case: "snake_case"
      struct_case: "PascalCase"
      example:
        id: "executor.validator.db_params"
        file: "executor/validator/db_params.go"
        struct: "DbParamsValidator"
        package: "validator"

    java:
      extensions: [".java"]
      separator: "."
      file_case: "PascalCase"
      class_case: "PascalCase"
      example:
        id: "executor.validator.db_params"
        file: "executor/validator/DbParams.java"
        class: "DbParamsValidator"
        package: "com.example.extensions.executor.validator"

    typescript:
      extensions: [".ts", ".tsx"]
      separator: "."
      file_case: "camelCase"
      class_case: "PascalCase"
      example:
        id: "executor.validator.db_params"
        file: "executor/validator/dbParams.ts"
        class: "DbParamsValidator"

  # Special mappings (override auto rules)
  overrides:
    # When automatic conversion doesn't meet requirements, manually specify
    "executor.validator.db_params":
      java:
        class: "com.mycompany.DbParamsValidator"
        package: "com.mycompany.validators"
```

### 2.3 Special Word Handling

Rules for handling abbreviations when converting class names:

```yaml
abbreviations:
  # Common abbreviations
  words: [http, api, db, id, url, sql, json, xml, html, css, tcp, udp, ip]

  # Rule: Treat abbreviations as normal words, capitalize only first letter
  # Reason: HttpJsonParser is more readable than HTTPJSONParser

  # Examples
  examples:
    id: "api.handler.http_json_parser"
    class: "HttpJsonParser"      # Not HTTPJSONParser
```

### 2.4 Version Number Handling

```yaml
versioning:
  # Filename format
  pattern: "{name}_v{major}.{ext}"

  # Examples
  examples:
    - file: "db_params.py"       # No version = default version
      id: "executor.validator.db_params"

    - file: "db_params_v2.py"
      id: "executor.validator.db_params_v2"

  # Version resolution
  resolution:
    default: "latest"            # Use latest version by default
    explicit: true               # Allow explicit specification: module_id@v2
```

### 2.5 Reserved Words

```yaml
reserved_words:
  # Framework reserved
  framework: [system, internal, core, apcore, plugin, schema, acl, ephemeral]

  # Programming language keywords
  keywords: [class, def, import, return, if, else, for, while, true, false, null, none]

  # Disallowed patterns
  patterns:
    - "^_.*"         # Starting with underscore
    - "^[0-9].*"     # Starting with digit
    - ".*__.*"       # Double underscore
```

**Reserved namespace semantics:**

| Namespace | Purpose | Registration mechanism |
|---|---|---|
| `system.*` | Built-in framework modules (health, manifest, usage, control) | Framework-internal only (e.g., `Registry.register_internal()`); user code MUST NOT register `system.*` IDs |
| `internal.*` | Reserved for SDK-private modules; not intended for user code | Implementation-defined |
| `core.*` | Reserved for future spec promotion of metadata extension keys | Reserved — no current use |
| `apcore.*`, `plugin.*`, `schema.*`, `acl.*` | Reserved for framework subsystem extensions | Reserved — no current use |
| `ephemeral.*` | Programmatically-generated runtime modules (Agent-synthesized tools, on-the-fly composition) | Standard `Registry.register()` only; framework-internal `register_internal()` MUST reject `ephemeral.*` IDs. See `./rfc-ephemeral-modules.md` for the full namespace contract. |

### 2.6 ID Conflict Detection

Implementations **MUST** perform conflict detection during module scanning, module registration, and dynamic loading.

```
Algorithm: detect_id_conflicts(new_id, existing_ids, reserved_words)

Input:
  new_id         — Canonical ID to be registered
  existing_ids   — Set of already registered IDs
  reserved_words — Set of reserved words (§2.5)

Output:
  conflict_result — { type: string, severity: "error" | "warning", message: string } | null

Steps:
  1. Exact duplication detection:
     If new_id ∈ existing_ids → Return { type: "duplicate_id", severity: "error" }
  2. Reserved word detection:
     For each segment of new_id (split by "."):
       If segment ∈ reserved_words → Return { type: "reserved_word", severity: "error" }
  3. Case collision detection:
     normalized_new ← lowercase(new_id)
     For each existing_id ∈ existing_ids:
       normalized_existing ← lowercase(existing_id)
       If normalized_new == normalized_existing and new_id ≠ existing_id:
         → Return { type: "case_collision", severity: "warning" }
  4. Return null (no conflict)

Complexity: O(n), where n is the number of registered IDs
```

```yaml
conflict_detection:
  when: [Module scanning, Module registration, Dynamic loading]

  types:
    duplicate_id:
      description: "Same ID already exists"
      action: "error"

    reserved_word:
      description: "Uses reserved word"
      action: "error"

    case_collision:
      description: "Only differs in case (cross-language conflict risk)"
      action: "warning"
```

### 2.7 ID Formal Grammar

The formal definition of Canonical ID uses EBNF notation. All implementations **MUST** reject IDs that do not conform to this grammar.

```ebnf
(* apcore Canonical ID EBNF *)

canonical_id    = segment , { "." , segment } ;
segment         = lower_alpha , { lower_alpha | digit | "_" } ;
lower_alpha     = "a" | "b" | "c" | "d" | "e" | "f" | "g" | "h" | "i"
                | "j" | "k" | "l" | "m" | "n" | "o" | "p" | "q" | "r"
                | "s" | "t" | "u" | "v" | "w" | "x" | "y" | "z" ;
digit           = "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" ;

(* Constraints *)
(* 1. canonical_id total length MUST NOT exceed 192 characters *)
(* 2. segment MUST NOT be a reserved word (see §2.5) *)
(* 3. segment MUST NOT start with a digit (guaranteed by production) *)
(* 4. segment MUST NOT contain consecutive double underscores "__" *)
```

```ebnf
(* In multi-class discovery mode, module_id = base_id "." class_segment where *)
(* class_segment is the snake_case-converted class name. The full ID must still *)
(* conform to the canonical_id production above. *)
```

Equivalent regular expression: `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`

---

## 3. Directory Specification

### 3.1 Standard Directory Structure

Implementations **MUST** follow the directory structure below. The nesting depth under `extensions/` directory (not including `extensions/` itself) **MUST NOT** exceed 8 levels.

```
{project_root}/
├── extensions/                   # Extensions directory (max depth: 8 levels)
│   ├── api/                      # Level 1 grouping: API layer
│   │   ├── validator/            # Level 2 grouping: Validators
│   │   │   ├── http_json_schema.{ext}
│   │   │   ├── http_json_schema_meta.yaml
│   │   │   └── ...
│   │   ├── parser/               # Level 2 grouping: Parsers
│   │   └── handler/              # Level 2 grouping: Handlers
│   │
│   ├── orchestrator/             # Level 1 grouping: Orchestration engine layer
│   │   ├── engine/               # Level 2 grouping: Engine
│   │   ├── validator/
│   │   └── parser/
│   │
│   └── executor/                 # Level 1 grouping: Executor layer
│       ├── handler/
│       └── validator/
│
├── schemas/                      # Schema definition directory (YAML)
│   └── {canonical_id}.schema.yaml
│
├── acl/                          # Access control configuration directory
│   └── {scope}_acl.yaml
│
└── apcore.yaml                   # Framework configuration file
```

### 3.2 Layer Definitions

| Level 1 Grouping | Responsibility | Typical Level 2 Groupings |
|----------|------|-------------|
| `api` | API entry layer, handles external requests | validator, parser, handler |
| `orchestrator` | Orchestration engine layer, task scheduling and process control | engine, validator, parser, scheduler |
| `executor` | Executor layer, actual business execution | handler, validator, connector |
| `common` | Common components | util, middleware, decorator |

### 3.3 ID Generation Rules

```yaml
id_generation:
  # Directory path → Canonical ID
  source: "directory_path"

  # Exclusions
  exclude:
    - extensions_root_dir    # Exclude "extensions/"
    - file_extension      # Exclude ".py", ".rs", etc.

  # Example
  example:
    input: "extensions/executor/validator/db_params.py"
    output: "executor.validator.db_params"
```

### 3.4 Symbolic Link Handling

Implementations **MUST NOT** follow symbolic links (symlinks) in default mode. If symbolic link support is needed, it **MUST** be enabled through explicit configuration:

```yaml
# apcore.yaml
extensions:
  follow_symlinks: false  # Default value, MUST NOT follow
```

- When symbolic link following is enabled, implementations **MUST** detect symbolic link loops
- The resolved path of symbolic links **MUST** still be within the `extensions_root` scope

### 3.5 Hidden File Handling

Implementations **MUST** ignore the following files and directories during module scanning:

| Pattern | Example | Description |
|------|------|------|
| Starts with `.` | `.git/`, `.env` | Hidden files/directories |
| Starts with `_` | `_internal/`, `_test.py` | Internal files/directories |
| `__pycache__/` | — | Python cache |
| `node_modules/` | — | Node.js dependencies |
| `*.pyc` | — | Python compiled files |

Implementations **may** extend the ignore pattern list through configuration.

### 3.6 Scanning Algorithm

Implementations **MUST** scan the extensions directory according to the following algorithm:

```
Algorithm: scan_extensions(extensions_root, config)

Input:
  extensions_root — Extensions root directory path
  config          — Scan configuration (follow_symlinks, ignore_patterns, max_depth)

Output:
  modules — List of [(file_path, canonical_id)]

Steps:
  1. If extensions_root doesn't exist or is not a directory → Throw CONFIG_ERROR
  2. modules ← []
  3. Recursively traverse extensions_root:
     For each entry (file or directory):
       a. If entry name matches ignore_patterns (§3.5) → Skip
       b. If entry is a symbolic link and config.follow_symlinks == false → Skip
       c. If entry is a directory:
          - If current depth >= config.max_depth (default 8) → Skip and issue warning
          - Otherwise → Recurse into it
       d. If entry is a file and extension belongs to supported_extensions:
          - canonical_id ← directory_to_canonical_id(entry.path, extensions_root)
          - If canonical_id passes validation → Append (entry.path, canonical_id) to modules
          - If validation fails → Log warning
  4. Perform detect_id_conflicts batch detection on modules
  5. Return modules

Complexity: O(n), where n is the number of filesystem entries
```

---

## 4. Schema Specification

### 4.1 Overview

All modules **MUST** define `input_schema` and `output_schema` to support:
- LLM tool calling (MCP compatible)
- Runtime data validation
- Automatic API documentation generation
- Cross-language interoperability

### 4.2 Schema Format

**MUST** be based on **JSON Schema Draft 2020-12** ([RFC unpublished draft](https://json-schema.org/draft/2020-12/json-schema-core)), extending with LLM-friendly fields.

**Compliance Requirements:**

| Requirement | Level | Description |
|------|------|------|
| Draft 2020-12 core vocabulary | **MUST** | type, properties, required, $ref, etc. |
| Draft 2020-12 validation vocabulary | **MUST** | minimum, maximum, pattern, enum, etc. |
| `$schema` declaration | **SHOULD** | Schema files **SHOULD** declare `$schema` field |
| `x-` extension prefix | **MUST** | Custom extension fields **MUST** be named with `x-` prefix |
| `additionalProperties` | **SHOULD** | input_schema **SHOULD** explicitly declare `additionalProperties: false` |

```yaml
# schemas/executor/validator/db_params.schema.yaml

$schema: "https://apcore.dev/schema/v1"
version: "1.0.0"
module_id: "executor.validator.db_params"

# Module description (LLM readable)
description: |
  Validates database operation parameters, checks table name format and SQL syntax safety.
  Suitable for pre-validation before executing SQL.

# Input Schema
input_schema:
  type: object
  properties:
    table:
      type: string
      pattern: "^[a-z][a-z0-9_]*$"
      description: "Target database table name"
      x-llm-description: "Database table name, only lowercase letters, numbers, and underscores allowed, must start with a letter"
      x-examples: ["user_info", "order_detail", "product_catalog"]

    sql:
      type: string
      description: "SQL statement"
      x-llm-description: "SQL statement to execute, will undergo safety checks"
      x-constraints: "Dangerous operations like DROP, TRUNCATE are not allowed"

    timeout:
      type: integer
      default: 30
      minimum: 1
      maximum: 300
      description: "Timeout in seconds"

  required: [table, sql]
  additionalProperties: false

# Output Schema
output_schema:
  type: object
  properties:
    valid:
      type: boolean
      description: "Whether validation passed"

    message:
      type: string
      description: "Validation result message"

    errors:
      type: array
      items:
        type: object
        properties:
          field:
            type: string
          code:
            type: string
          message:
            type: string
      description: "Error details list"

    warnings:
      type: array
      items:
        type: string
      description: "Warning messages"

  required: [valid]

# Error Schema (optional)
error_schema:
  type: object
  properties:
    code:
      type: string
      enum: [INVALID_TABLE, DANGEROUS_SQL, TIMEOUT, INTERNAL_ERROR]
    message:
      type: string
    details:
      type: object
  required: [code, message]
```

### 4.3 LLM Extension Fields

| Field | Type | Description |
|------|------|------|
| `x-llm-description` | string | Dedicated description for LLM (see usage guide below) |
| `x-examples` | array | Example values to help LLM understand value range |
| `x-constraints` | string | Business constraint description |
| `x-sensitive` | boolean | Mark sensitive fields (e.g., passwords), LLM should not log |

#### Relationship between `description` and `x-llm-description`

| Field | Audience | Purpose |
|------|------|------|
| `description` | All consumers (humans, AI, doc generators) | Universal field description, exported to AI protocols as field description |
| `x-llm-description` | AI/LLM only | Use only when AI description needs to be **significantly different** from human description |

**Use Cases:**

- Security warnings: AI needs additional security constraint hints (e.g., "only SELECT statements allowed")
- Usage guidance: Special behavior constraints when AI calls (e.g., "don't hard-code this value")
- Semantic supplementation: description is concise for humans, but AI needs more context to fill correctly

**Export Rules:**

- Adapters **SHOULD** prefer `x-llm-description`: If field has both `description` and `x-llm-description`, replace `description` with `x-llm-description` when exporting to AI protocols
- If field has only `description`, export as-is
- Strict mode export (§4.16) strips all `x-*` fields, always using `description`

**Anti-pattern:** Don't add `x-llm-description` to every field. For most fields, `description` is sufficient for both humans and AI, use `x-llm-description` only when there's a clear difference in needs.

#### `x-constraints` Usage Guidance

`x-constraints` is for **business rules that JSON Schema keywords cannot express**. Do not use it to duplicate what JSON Schema already provides natively:

| Constraint Type | Correct Approach | Incorrect Approach |
|------|------|------|
| Number range | `"minimum": 0, "maximum": 100` | `"x-constraints": "Value must be 0-100"` |
| String pattern | `"pattern": "^[A-Z]{3}$"` | `"x-constraints": "Must be 3 uppercase letters"` |
| Fixed options | `"enum": ["a", "b", "c"]` | `"x-constraints": "One of a, b, or c"` |
| Cross-field rule | `"x-constraints": "end_date must be after start_date"` | ✅ Correct use |
| Domain rule | `"x-constraints": "User must have verified email"` | ✅ Correct use |

When a constraint can be expressed as a JSON Schema keyword (`minimum`, `maximum`, `pattern`, `enum`, `minLength`, `maxLength`, `multipleOf`), always prefer the keyword. `x-constraints` is the **last resort** for constraints that are inherently natural-language.

### 4.4 Module Behavior Annotations (Annotations)

**Annotations are module-level behavior metadata** that help AI/LLM make invocation decisions.

```yaml
# Module behavior annotations specification
annotations:
  type: object
  properties:
    readonly:
      type: boolean
      default: false
      description: "Whether module is read-only (no side effects). true means no state modification."

    destructive:
      type: boolean
      default: false
      description: "Whether module has destructive operations. true means may delete/overwrite data."

    idempotent:
      type: boolean
      default: false
      description: "Whether module is idempotent. true means repeated calls with same parameters won't produce additional side effects."

    requires_approval:
      type: boolean
      default: false
      description: "Whether requires human approval before execution. When an ApprovalHandler is configured, the Executor enforces this at runtime (see §7 Approval System)."

    discoverable:
      type: boolean
      default: true
      description: "Whether the module appears in enumeration surfaces (Registry.list(), Registry.find(), manifest export, MCP tools/list). false hides the module from discovery while keeping it callable by ID — caller must already know the module ID. Default true preserves backward compatibility. ephemeral.* modules SHOULD set discoverable: false."

    open_world:
      type: boolean
      default: true
      description: "Whether involves external systems. true means connects to external APIs/services/network."

    streaming:
      type: boolean
      default: false
      description: "Whether module supports streaming output. true means module can emit partial results progressively."

    cacheable:
      type: boolean
      default: false
      description: "Whether the module output can be cached. true means identical inputs produce identical outputs within cache_ttl."

    cache_ttl:
      type: integer
      default: 0
      description: "Suggested cache duration in seconds. 0 means no caching. Only meaningful when cacheable=true."

    cache_key_fields:
      type: array
      items:
        type: string
      default: null
      description: "Input fields that determine cache key. null means all input fields are used. Only meaningful when cacheable=true."

    paginated:
      type: boolean
      default: false
      description: "Whether the module returns paginated results. true means the module accepts pagination parameters (cursor/offset) and returns partial result sets."

    pagination_style:
      type: string
      default: cursor
      description: "Pagination strategy. Well-known values: 'cursor' (opaque continuation token), 'offset' (numeric offset+limit), 'page' (page-number-based). Custom strategies are allowed. Only meaningful when paginated=true."
      examples: [cursor, offset, page]

    extra:
      type: object
      additionalProperties: true
      default: {}
      description: "Open extension map for ecosystem-specific or vendor annotation metadata. Keys SHOULD use a `<namespace>.<name>` form (e.g. `mcp.category`, `cli.approval_message`, `a2a.skill_id`). The `core.*` namespace is RESERVED for future spec promotion. Wire-format rules in §4.4.1 are normative."
```

**Annotations Design Principles:**

| Principle | Description |
|------|------|
| All optional | Use default values when not defined |
| Hints + Enforcement | Most annotations are hints for AI; `requires_approval` can be enforced at runtime via the Approval System (§7) |
| Aligned with MCP | Field design compatible with MCP ToolAnnotations |
| Extensible | Can add new fields without breaking compatibility |

**AI usage of Annotations decision examples:**

```
readonly=true         → AI can call safely, no confirmation needed
destructive=true      → AI should warn user before calling
idempotent=true       → AI can safely retry failed calls
requires_approval=true → AI must seek user consent; Executor enforces via ApprovalHandler (§7)
discoverable=false    → AI / orchestrator hides the module from enumeration surfaces (still callable by exact ID)
open_world=true       → AI knows this call involves external systems, may be slow
streaming=true        → AI knows this module emits partial results progressively
cacheable=true        → AI knows it can reuse previous results within cache_ttl
paginated=true        → AI knows to pass pagination params and expect partial results
```

#### 4.4.1 Annotations Extension Field (`extra`) — Wire Format

`ModuleAnnotations` carries an open extension map under the field `extra`, used by ecosystem packages and vendor integrations to attach metadata that is not part of the core annotation set. The on-the-wire JSON shape is normative across all SDK implementations.

**Producer rules:**

1. Implementations **MUST** serialize annotation extension data as a nested JSON object under the key `extra`.
2. Implementations **MUST NOT** flatten extension keys onto the annotations root object.
3. Producers **MUST NOT** emit both a nested `extra.k` and a top-level `k` for the same key in the same payload.
4. When `extra` is empty, producers **SHOULD** emit `"extra": {}` rather than omitting the field, to keep the wire shape stable across SDKs.

**Consumer rules:**

5. Consumers **MUST** accept the canonical nested form `{"extra": {...}}`.
6. Consumers **MAY** accept top-level overflow keys (i.e. unknown keys at the annotations root) as a backward-compatibility shim for one MINOR cycle following v0.18.0. Such keys **MUST** be normalized into the nested `extra` object on deserialize and re-serialized in nested form.
7. When a deserialization input contains BOTH a nested `extra.k` and a top-level `k` with the same key, **the nested value MUST win**. (This intentionally inverts the legacy Python/TypeScript "overflow wins" precedence — a one-time correction during the v0.18.0 normalization.)
8. When `extra` is absent or `null`, consumers **MUST** treat it as an empty object.

**Key naming:**

9. `extra` keys **SHOULD** use the form `<namespace>.<name>`, where `<namespace>` identifies the consuming subsystem or vendor (e.g. `mcp.category`, `cli.approval_message`, `a2a.skill_id`, `vendor.acme.priority`).
10. The `core.*` namespace is **RESERVED** for future promotion of `extra` keys into standard `Annotations` fields.
11. Dots in `extra` keys are part of the literal key string. Implementations **MUST NOT** interpret `a.b.c` as a nested path on either serialize or deserialize.
12. Extension keys **MUST NOT** collide with any canonical field name listed in §4.4. If a collision is observed during deserialization, the canonical field **MUST** win and the extension key **MUST** be discarded with a warning.

**Promotion to standard fields:**

13. When a community-adopted `extra` key is promoted to a standard `Annotations` field in a future spec version, that change **MUST** go through one MINOR deprecation cycle in which both forms (the new standard field AND the `extra.<old_key>` entry) are accepted by consumers, before the `extra` form is removed.

**Conformance:** Cross-language behavior is locked by `../../conformance/fixtures/annotations_extra_round_trip.json`.

> **Version note:** Introduced in protocol v0.18.0. SDKs at or below v0.17.1 emitting the flattened form (notably `apcore-rust ≤ 0.17.1`) are non-conformant and **MUST** migrate.

**Example (canonical wire form):**

```json
{
  "readonly": true,
  "destructive": false,
  "idempotent": true,
  "requires_approval": false,
  "open_world": true,
  "streaming": false,
  "cacheable": false,
  "cache_ttl": 0,
  "cache_key_fields": null,
  "paginated": false,
  "pagination_style": "cursor",
  "extra": {
    "mcp.category": "tools",
    "cli.approval_message": "Are you sure?"
  }
}
```

### 4.5 Module Usage Examples (Examples)

**Examples provide concrete input/output examples** to help AI/LLM understand complex modules more accurately.

```yaml
# Module examples specification
examples:
  type: array
  items:
    type: object
    properties:
      title:
        type: string
        description: "Example title"

      description:
        type: string
        description: "Example description (optional)"

      inputs:
        type: object
        description: "Example input (must conform to input_schema)"

      output:
        type: object
        description: "Example output (optional, conforms to output_schema)"
    required: [title, inputs]
```

**Example:**

```yaml
examples:
  - title: "Send plain text email"
    description: "Send plain text email via SMTP"
    inputs:
      to: "user@example.com"
      subject: "Hello"
      body: "World"
    output:
      success: true
      message_id: "msg_123"

  - title: "Send HTML email"
    description: "Send HTML format email to multiple recipients via SMTP"
    inputs:
      to: ["user1@example.com", "user2@example.com"]
      subject: "Notification"
      html: "<h1>Hello</h1>"
      smtp_host: "smtp.example.com"
      smtp_port: 587
    output:
      success: true
      message_id: "msg_456"
```

**Examples Design Principles:**

| Principle | Description |
|------|------|
| All optional | Not providing examples doesn't affect module functionality |
| Inputs must be valid | inputs must conform to input_schema |
| Multi-scenario coverage | Recommend covering typical usage and edge cases |
| Aligned with Anthropic | Similar to Anthropic input_examples, improves LLM understanding |

### 4.6 Module Extension Metadata (Metadata)

**Metadata is a completely open dict** for custom extensions beyond the framework protocol.

```yaml
# Module metadata specification
metadata:
  type: object
  additionalProperties: true
  description: "Free extension metadata, framework doesn't validate content"
```

**Common usage examples:**

```yaml
metadata:
  # Performance hints
  cost_per_call: 0.001
  avg_latency_ms: 500
  max_latency_ms: 5000

  # Data sensitivity
  data_sensitivity: ["PII"]

  # Operations info
  owner: "email-team"
  sla: "99.9%"
  documentation_url: "https://docs.example.com/send-email"

  # Custom business fields
  billing_category: "communication"
```

**Recommended AI Metadata Conventions:**

When modules are consumed by AI agents, the following metadata keys help agents understand when, how, and at what cost to use a module. These are conventions, not enforced by the framework.

**Intent Hints** — Help agents decide whether to use this module:

```yaml
metadata:
  # AI intent hints (all optional, free-form strings)
  x-when-to-use: "Use when the user wants to send a transactional email (order confirmation, password reset, etc.)"
  x-when-not-to-use: "Do not use for marketing/bulk emails; use the bulk-email module instead"
  x-common-mistakes: "Forgetting to set reply_to; passing HTML in plain_text field"
  x-workflow-hints: "Call validate-email module first to verify recipient address exists"
```

**Planning Hints** — Help agents build multi-step plans:

```yaml
metadata:
  # Preconditions: what must be true before calling this module
  x-preconditions:
    - "User must be authenticated"
    - "module.validate_card must have succeeded"

  # Postconditions: what will be true after calling this module
  x-postconditions:
    - "Payment record created in database"
    - "Confirmation email queued"

  # Side effects: external state changes caused by this module
  x-side-effects:
    - "Charges credit card via Stripe API"
    - "Writes to orders table"
```

**Performance & Cost Hints** — Help agents make cost-aware decisions:

```yaml
metadata:
  # Cost estimation (units are implementation-defined, e.g., USD)
  x-cost-per-call: 0.001
  x-avg-latency-ms: 500
  x-max-latency-ms: 5000

  # SLA targets (for monitoring; enforcement is an ecosystem concern)
  x-sla:
    availability: 0.999
    p95_latency_ms: 500
```

**Trust & Verification Hints** — Help agents assess output reliability:

```yaml
metadata:
  # Output source: where does the data come from?
  x-output-source: "database"    # database | api | generated | cached | computed
  # Verification hint: how to cross-check the output
  x-verification-hint: "Cross-check amounts with accounting.get_balance"
```

**Routing & Verification Hints** — Help orchestrators choose the right model, inject required state, and preflight destructive calls:

```yaml
metadata:
  # Reasoning demand: hint to upstream model routers for tier selection
  x-reasoning-demand: medium     # one of: low | medium | high

  # Context keys this module reads from context.data
  # (does NOT include framework-owned Context fields like trace_id, caller_id)
  x-required-context-keys:
    - user_preferences
    - billing_account_id

  # Dry-run capability: module-level signal that Executor.validate() is meaningful
  x-supports-dry-run: true
```

| Key | Category | Purpose |
|-----|----------|---------|
| `x-when-to-use` | Intent | Positive guidance: scenarios where this module is the right choice |
| `x-when-not-to-use` | Intent | Negative guidance: scenarios where a different module should be used |
| `x-common-mistakes` | Intent | Known pitfalls that AI agents (and humans) frequently encounter |
| `x-workflow-hints` | Intent | Suggested pre/post steps or related modules in a typical workflow |
| `x-preconditions` | Planning | What must be true before calling this module |
| `x-postconditions` | Planning | What will be true after successful execution |
| `x-side-effects` | Planning | External state changes caused by this module |
| `x-cost-per-call` | Performance | Estimated cost per invocation |
| `x-avg-latency-ms` | Performance | Average execution latency in milliseconds |
| `x-max-latency-ms` | Performance | Maximum expected latency in milliseconds |
| `x-sla` | Performance | SLA targets (availability, latency percentiles) |
| `x-output-source` | Trust | Data provenance: database, api, generated, cached, computed |
| `x-verification-hint` | Trust | How to cross-check the output for correctness |
| `x-reasoning-demand` | Routing | One of `low`/`medium`/`high`. Hint of the minimum reasoning capability the calling agent needs; consumed by upstream model routers for tier selection. |
| `x-required-context-keys` | Planning | Array of `context.data` key names the module reads. Does **not** include framework-owned Context fields (`trace_id`, `caller_id`, etc.). |
| `x-supports-dry-run` | Verification | Boolean. Module-level signal that `Executor.validate()` (see §12.2) is meaningful for this module — i.e., the module overrides `preflight()` and/or has no destructive side-effects pre-execute. |

**Metadata Design Principles:**

| Principle | Description |
|------|------|
| Completely free | Framework doesn't validate or interpret metadata content |
| Can be used by middleware | Middleware can read metadata for custom logic |
| Doesn't affect execution | metadata doesn't participate in module execution flow |
| Open extension | When custom needs arise, metadata is the first landing spot |

### 4.7 Three-layer Metadata Summary

```
┌───────────────────────────────────────────────────────────┐
│                  Module Metadata Three-layer Design        │
├───────────────────────────────────────────────────────────┤
│                                                           │
│  ┌─────────────────────────────────────────────────┐     │
│  │  Core Layer (Required)                          │     │
│  │  input_schema / output_schema / description     │     │
│  │  → AI understands "what" the module does        │     │
│  └─────────────────────────────────────────────────┘     │
│                          ↓                                │
│  ┌─────────────────────────────────────────────────┐     │
│  │  Annotation Layer (Optional, type-safe)         │     │
│  │  annotations / examples / name / tags / version │     │
│  │  → AI understands "how to use" the module       │     │
│  └─────────────────────────────────────────────────┘     │
│                          ↓                                │
│  ┌─────────────────────────────────────────────────┐     │
│  │  Extension Layer (Optional, free dict)          │     │
│  │  metadata: dict[str, Any]                       │     │
│  │  → Custom extension needs (framework unconstrained)  │
│  └─────────────────────────────────────────────────┘     │
│                                                           │
└───────────────────────────────────────────────────────────┘
```

### 4.8 Description and Documentation Field Specification

#### 4.8.1 Design Principles

apcore adopts the **Progressive Disclosure** design pattern, referencing the [Claude Agent Skills standard](https://github.com/anthropics/skills), splitting module information into brief description and detailed documentation:

| Field | Required | Length Limit | Markdown | Purpose |
|------|--------|----------|----------|------|
| `description` | **Required** | ≤200 characters | No | Brief module function description for AI quick matching and understanding |
| `documentation` | Optional | ≤5000 characters | Yes | Detailed documentation including usage scenarios, constraints, configuration requirements |

**Core Philosophy:**

```
AI Module Discovery Flow:
├─ Phase 1: Module Discovery
│   └─ Read all modules' description (≤200 chars)
│   └─ Quickly determine candidate modules
│
├─ Phase 2: Invocation Decision
│   └─ Read candidate modules' Schema + Annotations
│   └─ Optionally read documentation (detailed info)
│   └─ Confirm parameters and behavior characteristics
│
└─ Phase 3: Execution Preparation
    └─ Read detailed documentation (documentation/docstring)
    └─ Learn detailed usage and examples
```

**Standard Correspondence:**

| apcore | Claude Skill | OpenAPI | JSON Schema |
|--------|-------------|---------|-------------|
| `description` | `description` | `summary` | `title` |
| `documentation` | `instructions` | `description` | `description` |

#### 4.8.2 Description Field

**MUST:**
- Length limit: ≤200 characters (approx. 100 Chinese characters)
- Content requirements: Explain "what it does" + "when to use" + "key characteristics"
- Avoid redundancy: Parameter info already defined in Schema need not be repeated

**SHOULD:**
- Highlight function verbs and usage scenarios
- Explain key behavior characteristics (e.g., idempotency, side effects)
- Single line or brief paragraph (≤3 lines)

**SHOULD NOT:**
- Include detailed usage examples (put in documentation or docstring)
- Repeat parameter lists from Schema
- Exceed 200 characters

**Recommended Format:**

```python
# Format 1: Single-line concise description (recommended)
description = "Send email to specified recipients. Uses SMTP protocol, non-idempotent operation, requires mail server configuration."

# Format 2: Structured brief description (suitable for complex modules)
description = """Send email to specified recipients.
Sends text/HTML emails via SMTP, non-idempotent operation. Use cases: notifications, verification codes, reports."""
```

**YAML format:**

```yaml
module_id: "executor.email.send_email"
description: "Send email to specified recipients. Uses SMTP protocol, non-idempotent operation, requires mail server configuration."
```

#### 4.8.3 Documentation Field

The `documentation` field is **optional**, used to provide detailed module documentation. Simple modules can use only the `description` field.

**SHOULD use documentation scenarios:**
- Module has complex configuration requirements
- Need to explain important usage constraints or limitations
- Have multiple usage scenarios to explain
- Need to provide detailed error handling explanation

**Format Requirements:**
- Supports Markdown format
- Length limit: ≤5000 characters
- Recommend using structured format (headings, lists, etc.)

**Recommended Content Structure:**

```markdown
## Functionality
Brief explanation of module's detailed functionality

## Configuration Requirements
- Required configuration items
- Dependent external services

## Usage Scenarios
- Scenario 1: ...
- Scenario 2: ...

## Limitations and Constraints
- Performance limits (e.g., rate limits)
- Data size limits
- Other constraints

## Notes
Important usage considerations
```

**Example:**

```python
class SendEmailModule(Module):
    description = "Send email to specified recipients. Uses SMTP protocol, non-idempotent operation, requires mail server configuration."

    documentation = """
# Functionality
Sends emails via SMTP protocol, supports plain text and HTML formats.

## Configuration Requirements
- Must configure SMTP server info in apcore.yaml
- Need to provide valid SMTP authentication credentials

## Usage Scenarios
- Send notification emails
- Send verification codes
- Send reports

## Limitations
- Gmail: 500 emails/day
- Attachment size: ≤25MB
- Timeout: 30 seconds

## Error Handling
Email send failures throw EmailSendError exception with detailed error info.
"""
```

**YAML format:**

```yaml
module_id: "executor.email.send_email"
description: "Send email to specified recipients. Uses SMTP protocol, non-idempotent operation, requires mail server configuration."
documentation: |
  # Functionality
  Sends emails via SMTP protocol, supports plain text and HTML formats.

  ## Configuration Requirements
  - Must configure SMTP server info in apcore.yaml
  - Need to provide valid SMTP authentication credentials

  ## Usage Scenarios
  - Send notification emails, verification codes, reports

  ## Limitations
  - Gmail: 500 emails/day
  - Attachment size: ≤25MB
```

#### 4.8.4 Relationship with Docstring

apcore modules can use three forms of documentation simultaneously:

| Location | Purpose | Audience | Format |
|------|------|------|------|
| `description` | Quick module understanding | AI module discovery phase | Plain text, ≤200 chars |
| `documentation` | Detailed usage documentation | AI invocation decision phase | Markdown, ≤5000 chars |
| Python docstring | Code-level documentation | Developers, IDE | reStructuredText/Markdown |

**Priority and Usage Recommendations:**

1. **description** (required): All modules must define
2. **documentation** (recommended): Complex modules should define
3. **docstring** (optional): For developer reading, IDE can display

**Example:**

```python
class SendEmailModule(Module):
    """Email sending module (Python docstring, for developers to read)

    This is Python docstring for developers to view in IDE.
    Can include more detailed technical implementation notes, source code comments, etc.
    """

    # Used in AI module discovery phase
    description = "Send email to specified recipients. Uses SMTP protocol, non-idempotent operation, requires mail server configuration."

    # Used in AI invocation decision phase (optional)
    documentation = """
    # Functionality
    Sends emails via SMTP protocol, supports plain text and HTML formats.

    ## Configuration Requirements
    ...
    """
```

#### 4.8.5 Backward Compatibility

To maintain backward compatibility, the framework will automatically handle old format modules:

**Automatic Migration Rules:**

```python
# Existing module (only description, ≤200 chars)
if description exists and len(description) <= 200:
    # Keep unchanged, fully compatible
    description = description
    documentation = None

# description exceeds 200 chars (not recommended, but will warn)
elif description exists and len(description) > 200:
    # Issue warning, suggest migration
    warn("description exceeds 200 chars, consider moving details to 'documentation' field")
    # Keep as-is, but may affect AI performance
```

**Migration Recommendations:**

For existing long descriptions (>200 chars):
1. Extract first 200 chars as new description
2. Move complete content to documentation field
3. Or manually rewrite as brief description + detailed documentation

#### 4.8.6 AI Usage Flow Example

```
User request: "Send a welcome email to new user"

Step 1: Module Discovery (read all descriptions)
  → Scan 100 modules' descriptions (total ~20,000 characters)
  → Identify candidate module: executor.email.send_email
  → description: "Send email to specified recipients. Uses SMTP protocol, non-idempotent operation..."

Step 2: Invocation Decision (read Schema + Annotations + documentation)
  → input_schema: {to, subject, body}
  → output_schema: {success, message_id}
  → annotations: {requires_approval: true, open_world: true}
  → documentation: "# Functionality\nSends emails via SMTP protocol..."
  → Decision: User confirmation needed

Step 3: Execution Preparation
  → Learn configuration requirements: Need SMTP server configuration
  → Learn limitations: Gmail 500 emails/day
  → Prepare parameters: {"to": "new_user@example.com", ...}
```

#### 4.8.7 Performance Impact

**Token consumption comparison:**

| Approach | Module Discovery Phase | Invocation Decision Phase | Total |
|------|-------------|-------------|------|
| **Progressive Disclosure** (recommended) | ~20K tokens<br/>(100 modules × 200 chars) | ~5-10K tokens<br/>(1-2 candidate modules × 5000 chars) | ~25-30K |
| Load all full docs | ~500K tokens<br/>(100 modules × 5000 chars) | 0 | ~500K |

**Advantages:**
- Reduce 94% of initial token consumption
- Speed up module discovery (load only necessary info)
- Load detailed info on-demand (load only candidate modules' documentation)
- Better AI performance and response speed

---

### 4.9 Schema Loading Strategy

```yaml
# apcore.yaml
schema:
  # Loading strategy
  strategy: "yaml_first"  # yaml_first | native_first | yaml_only

  # yaml_first: Load from YAML first, native implementation can override
  # native_first: Prefer native implementation, YAML as fallback
  # yaml_only: Only use YAML (pure cross-language scenario)

  # Directory the YAML schema files are resolved against
  root: "./schemas"

  # Maximum $ref resolution depth
  max_ref_depth: 32
```

These three keys are the whole of the `schema` namespace: `apcore/schemas/defaults.schema.json`
declares it `additionalProperties: false`, so any other key is a configuration error.

Strictness and type coercion are **not** configurable here. Whether an undeclared
property is rejected is a property of the contract (`additionalProperties`, §4.15),
not of the host's configuration — a module's input contract must mean the same
thing regardless of who loads it.

### 4.10 Language-specific Schema Implementations

Each language SDK **MUST** provide a native schema implementation that supports JSON Schema Draft 2020-12 validation and YAML schema loading. The specific library choices are left to SDK implementers.

### 4.11 Schema References ($ref)

Implementations **MUST** support `$ref` references and **MUST** resolve according to the following algorithm. Implementations **MUST** reject circular references — a `$ref` → `$ref` chain that never reaches a schema body — and **MUST** preserve self-references as lazy `$ref` nodes rather than inlining or rejecting them. §4.15 defines the distinction; it is the sole authority on which re-entry is which.

```
Algorithm: resolve_ref(ref_string, current_file, schemas_dir, visited_refs,
                       depth, from_ref_chain)

Input:
  ref_string     — $ref value (e.g., "./common/error.schema.yaml#/definitions/ErrorDetail")
  current_file   — Current Schema file path
  schemas_dir    — schemas root directory
  visited_refs   — Set of refs already on the resolution stack. Seeded by the
                   caller with the aliases of the document being resolved
                   ("#", "#/", and the document's own $id), so a reference
                   naming that document is lazy from the FIRST encounter
   depth         — Number of $ref hops taken so far (starts at 0)
  from_ref_chain — True when this call was reached because the previous node
                   was itself a bare $ref, i.e. no schema body was traversed
                   between the two. False on any structural descent
                   (properties / items / additionalProperties / a combinator
                   branch)

Output:
  resolved_schema — Resolved Schema object, or the $ref node unchanged when
                    the reference is a self-reference (see §4.15)

Preconditions:
  - ref_string is not empty

Steps:
  1. If ref_string ∈ visited_refs:
     a. If from_ref_chain → Throw SCHEMA_CIRCULAR_REF error
        (a $ref → $ref chain consumes no instance and cannot terminate)
     b. Otherwise → Return { "$ref": ref_string } unchanged, preserving any
        sibling keys. This is a legal self-reference; the schema-to-native
        converter MUST bind it lazily against the document root at
        validation time (§4.15)
  2. If depth >= schema.max_ref_depth → Throw SCHEMA_MAX_DEPTH_EXCEEDED error
  3. visited_refs ← visited_refs ∪ {ref_string}
  4. Parse ref_string into (file_part, json_pointer):
     a. If starts with "#" → file_part = current_file, json_pointer = ref_string[1:]
     b. If contains "#" → Split by "#" into file_part and json_pointer
     c. If starts with "apcore://" → Convert to file path under schemas_dir
     d. Otherwise → file_part is path relative to current_file directory
  5. schema_doc ← Load and parse YAML/JSON file for file_part
  6. resolved ← Locate target_id node in schema_doc using json_pointer
  7. If resolved is itself a bare $ref → Recursively call resolve_ref(...) with
     depth + 1 and from_ref_chain = True
  8. Walk resolved's children. For each nested $ref reached by structural
     descent, call resolve_ref(...) with depth + 1 and from_ref_chain = False
  9. Return resolved

Complexity: O(d), where d is reference depth (bounded by schema.max_ref_depth,
default 32)
```

**Depth is consumed by `$ref` hops only.** Steps 7 and 8 both increment `depth`
because both follow a reference; ordinary structural descent into `properties`
or `items` does not. Exhausting the cap is a distinct condition from an actual
cycle and **MUST** raise `SCHEMA_MAX_DEPTH_EXCEEDED`, not `SCHEMA_CIRCULAR_REF`.

Supports cross-file references for Schema reuse:

```yaml
schema_reference:
  # Reference formats
  formats:
    # Local reference (same file)
    local: "#/definitions/ErrorDetail"

    # Cross-file reference (relative path)
    relative: "./common/error.schema.yaml#/definitions/ErrorDetail"

    # Cross-file reference (Canonical ID)
    canonical: "apcore://common.types.error/ErrorDetail"

  # Reference resolution rules
  resolution:
    - "Local references look up in current file first"
    - "Relative paths are relative to current schema file directory"
    - "Canonical ID references look in schemas/ directory"

  # Example
  example:
    # schemas/executor/validator/db_params.schema.yaml
    input_schema:
      type: object
      properties:
        table:
          type: string
        options:
          $ref: "./common/db_options.schema.yaml#/definitions/DBOptions"

    # Common definitions
    definitions:
      ErrorDetail:
        type: object
        properties:
          field: { type: string }
          code: { type: string }
          message: { type: string }
```

### 4.12 Schema Version Evolution

```yaml
schema_evolution:
  # Compatibility rules
  backward_compatible:
    allowed:
      - "Add optional field (with default value)"
      - "Relax field constraints (e.g., reduce minLength)"
      - "Add enum options"
    forbidden:
      - "Remove required field"
      - "Change field type"
      - "Tighten field constraints"
      - "Remove enum options"

  # Version declaration
  versioning:
    in_schema: true
    format: "semver"
    example:
      version: "1.1.0"
      previous_version: "1.0.0"
      changes:
        - type: "added"
          field: "options.retry_count"
          description: "Added retry count configuration"

  # Deprecated field marking
  deprecation:
    marker: "x-deprecated"
    format:
      x-deprecated:
        since: "1.1.0"
        removal: "2.0.0"
        replacement: "options.max_retries"
        message: "Please use options.max_retries instead"
```

### 4.13 Annotation Conflict Rules

When both YAML metadata file (`*_meta.yaml`) and code define Annotations, conflicts **MUST** be resolved by the following priority:

1. **YAML metadata file** (highest priority) — Operations teams can override behavior annotations without modifying code
2. **Explicit definition in code** (secondary priority) — Developer defines on module class
3. **Default values** (lowest priority) — Default values provided by framework

Implementations **MUST** merge rather than replace when loading: If YAML only defines `readonly: true`, other fields **MUST** retain values from code or defaults.

### 4.14 Schema Validation Error Format

When Schema validation fails, implementations **MUST** return structured error information:

```yaml
schema_validation_error:
  type: object
  required: [code, message, errors]
  properties:
    code:
      type: string
      const: "SCHEMA_VALIDATION_ERROR"
    message:
      type: string
      description: "Human-readable error summary"
    errors:
      type: array
      items:
        type: object
        required: [path, message]
        properties:
          path:
            type: string
            description: "JSON Pointer to error field (e.g., '/table' or '/options/retry_count')"
          message:
            type: string
            description: "Detailed description of validation failure"
          constraint:
            type: string
            description: "Name of violated constraint (e.g., 'pattern', 'minimum', 'required')"
          expected:
            description: "Expected value or constraint"
          actual:
            description: "Actual value"
```

### 4.15 Edge Case Handling

Implementations **MUST** handle Schema edge cases according to the following table:

| Scenario | Behavior | Level |
|------|------|------|
| `$ref` depth exceeds `schema.max_ref_depth` | Throw `SCHEMA_MAX_DEPTH_EXCEEDED` | **MUST** |
| `$ref` target_id path doesn't exist (404) | Throw `SCHEMA_NOT_FOUND` | **MUST** |
| Empty Schema `{}` | Treat as `type: object`, allow any properties | **MUST** |
| YAML/JSON syntax error | Throw `SCHEMA_PARSE_ERROR` | **MUST** |
| Unknown JSON Schema keyword (e.g., `x-custom`) | Ignore (forward compatible) | **MUST** |
| **Self-reference** — a `$ref` re-entered after descending through `properties` / `items` / a combinator | Preserve the `$ref` as a lazy reference; **MUST NOT** throw | **MUST** |
| **Circular reference** — a `$ref` → `$ref` chain that re-enters itself without reaching a schema body (A → B → A) | Throw `SCHEMA_CIRCULAR_REF` | **MUST** |
| `required` field is empty array `[]` | Treat as no required fields | **MUST** |
| `enum` value contains `null` | Allow, `null` is valid enum value | **MUST** |
| Cross-file `$ref` loading timeout | Throw `SCHEMA_PARSE_ERROR` (cause: timeout) | **SHOULD** |
| A value does not satisfy a **recognised** `format` | Accept the value. Raising `SCHEMA_VALIDATION_ERROR` is forbidden; emitting a warning is **SHOULD** | **MUST** |
| `format` names a term the implementation does not recognise | Collect as an annotation and pass silently | **MUST** |

**`format` is an annotation, and [type-mapping §11.1](./type-mapping.md#111-format-keyword) is its
sole authority.** The two rows above restate its consequence for edge-case handling so that a
reader arriving at this table is not left to infer that an unsatisfied `format` behaves like an
unsatisfied `pattern`. §11.1 carries the recognised-format list, the warning requirement, and the
rule that a binding is expressed with `pattern` or `enum` rather than with `format`.

#### Self-reference vs. circular reference

A `$ref` that re-enters a reference already on the resolution stack is **not**
automatically an error. JSON Schema 2020-12 §8.2.3 uses exactly that shape to
express recursive data structures — tree nodes, nested comment threads, linked
lists — and the Schema System hardening requirement "Recursive Schema Support"
([features/schema-system.md](../features/schema-system.md#2-recursive-schema-support))
requires implementations to support them. The two cases **MUST** be
distinguished by *how* the reference was re-entered:

- **Self-reference (legal).** The reference was re-entered after descending
  through a schema body — a `properties` entry, `items`, `additionalProperties`,
  or a combinator branch. Every hop consumes one level of the *instance*, so
  resolution terminates as soon as the data does. The resolver **MUST** leave the
  `$ref` node in place rather than inlining the target again, and the
  schema-to-native converter **MUST** bind it lazily (resolving it at validation
  time against the document root). Implementations **MUST NOT** throw
  `SCHEMA_CIRCULAR_REF` for this case.
- **Circular reference (illegal).** The reference was re-entered along a chain in
  which every hop is itself a bare `$ref`, so resolution never reaches a schema
  body and consumes no instance. Such a chain cannot terminate and asserts
  nothing. Implementations **MUST** throw `SCHEMA_CIRCULAR_REF`.

A reference naming the document being resolved — `#`, `#/`, or the document's own
`$id` — is a self-reference by definition and **MUST** be treated as lazy from the
first encounter, so a recursive schema is never inlined even once.

**Example — legal self-reference (lazy `$ref`, no error):**

```yaml
$id: TreeNode
type: object
required: [value]
properties:
  value: { type: string }
  children:
    type: array
    items:
      $ref: "#"      # re-entered through properties → items: a recursive structure
```

Resolution returns the schema unchanged: `items` still holds `{$ref: "#"}`, which
the converter binds to the root at validation time.

**Example — illegal circular reference (`SCHEMA_CIRCULAR_REF`):**

```yaml
# schemas/user.schema.yaml
$ref: "./team.schema.yaml#/definitions/team"

# schemas/team.schema.yaml
definitions:
  team:
    $ref: "./user.schema.yaml"  # $ref → $ref, no schema body in between
```

**Behavior**: maintain a reference path stack in `resolve_ref()`. On a duplicate
entry, throw `SCHEMA_CIRCULAR_REF` when the duplicate was reached along a
`$ref` → `$ref` chain, and emit a lazy `$ref` otherwise.

### 4.16 Strict Mode Export

OpenAI and Anthropic's `strict: true` mode requires JSON Schema to satisfy additional constraints. apcore defines `to_strict_schema()` conversion to transform standard apcore Schema to Strict Mode compatible format.

**Strict Mode Requirements:**

| Requirement | Description |
|------|------|
| `additionalProperties: false` | All nested `object` schemas **MUST** set this. A node counts as an `object` schema when it carries `properties` and either has no `type` keyword or has a `type` declaring `"object"` — see the object-detection rule below |
| All fields `required` | All fields in `properties` **MUST** appear in `required` array |
| Optional fields expressed with nullable | Originally optional fields become `required` + `type: ["original_type", "null"]`; a field with no `type` keyword is wrapped as `{anyOf: [<original>, {type: "null"}]}` instead |
| No `x-*` extension fields | All `x-*` fields **MUST** be stripped |
| No `default` values | `default` fields **MUST** be removed |

**Object detection.** `properties` alone identifies an object schema; a missing `type` keyword does not make the node any less of one. A node **MUST** be hardened when it carries `properties` **and** either has no `type` keyword at all, or has a `type` declaring `"object"` in the string form (`"object"`) or the array form (`["object", "null"]`). `properties` sitting beside a **non-object** `type` is inert (TYPE_MAPPING §17.1 R2) and **MUST NOT** be hardened. Leaving a type-less `{"properties": {…}}` node unhardened produces a schema OpenAI structured outputs rejects under `strict: true`, which is the whole reason this conversion exists.

**`to_strict_schema()` Conversion Rules:**

```
Input: apcore_schema (standard JSON Schema + x-* extensions)
Output: strict_schema (Strict Mode compatible JSON Schema)

Rules:
  1. Recursively traverse all object-schema nodes (see "Object detection" above —
     `properties` present, and `type` either absent or declaring "object"):
     a. Set additionalProperties: false
     b. Replace required with every name in properties, sorted by Unicode code
        point (not insertion order — the output must be byte-identical across
        SDKs whose object types iterate differently)
     c. For newly added required fields, change their type to [original_type, "null"];
        a field with no `type` keyword (pure $ref, allOf/oneOf/anyOf, or a bare
        `properties` object) is wrapped as {anyOf: [<original>, {type: "null"}]}
  2. Remove all fields starting with "x-" (x-llm-description, x-examples, x-sensitive, x-constraints, etc.)
  3. Remove all default fields
  4. Recursively process nested schemas: `properties` values, `items`, every
     `prefixItems` entry, every oneOf/anyOf/allOf branch, and every
     `definitions` / `$defs` entry
```

The normative statement of these rules is ALGORITHMS A23; `conformance/fixtures/schema_strict_conversion.json` pins the exact output all three SDKs must emit.

**Example — Before/After Conversion:**

```yaml
# Before conversion (apcore standard Schema)
type: object
properties:
  to:
    type: string
    description: "Recipient email"
    x-examples: ["user@example.com"]
  cc:
    type: array
    items: { type: string }
    description: "CC list"
    default: []
required: [to]

# After conversion (Strict Mode)
type: object
properties:
  to:
    type: string
    description: "Recipient email"
  cc:
    type: ["array", "null"]
    items: { type: string }
    description: "CC list"
required: [cc, to]        # every property, sorted (see ALGORITHMS A23 step 2f)
additionalProperties: false
```

**Registry `export_schema()` Integration:**

`export_schema()` accepts optional `strict` parameter. When `strict=true`, automatically applies `to_strict_schema()` conversion before export.

```python
# Standard export
schema = registry.export_schema("executor.email.send_email")

# Strict Mode export (for OpenAI / Anthropic)
strict_schema = registry.export_schema("executor.email.send_email", strict=True)
```

For detailed algorithm pseudocode, see [algorithms.md A23](./algorithms.md#a23-to_strict_schema-strict-mode-schema-conversion).

### 4.17 Export Profiles

apcore defines standard export Profiles for adapter developers to follow. Profiles are **adapter layer configuration**, not core implementation—apcore framework itself doesn't implement adapters but provides unified conversion specifications.

| Profile | Characteristics | Typical Users |
|---------|------|-----------|
| `mcp` | Preserve `x-*` fields; map annotations → hints; contains inputSchema + outputSchema | MCP Server adapters |
| `openai` | Strip `x-*`; `strict: true`; replace `.` with `_` in id; parameters only | OpenAI Function Calling adapters |
| `anthropic` | Strip `x-*`; map examples → input_examples; contains input_schema | Anthropic Claude Tool adapters |
| `generic` | Full JSON Schema + all extensions (default) | Generic scenarios, debugging |

**Conversion Details for Each Profile:**

**`mcp` Profile:**
- Schema: Preserve as-is (with `x-*` extension fields)
- ID: Use as-is (`executor.email.send_email`)
- Annotations: `readonly` → `readOnlyHint`, `destructive` → `destructiveHint`, `idempotent` → `idempotentHint`, `open_world` → `openWorldHint`
- See Appendix D.1 MCP Mapping

**`openai` Profile:**
- Schema: Apply `to_strict_schema()` conversion (§4.16)
- ID: Replace `.` with `_` (e.g., `executor_email_send_email`)
- Replace `description` with `x-llm-description` then strip
- See Appendix D.3 OpenAI Mapping

**`anthropic` Profile:**
- Schema: Strip `x-*` fields
- ID: Replace `.` with `_`
- Replace `description` with `x-llm-description` then strip
- Examples: `module.examples[*].inputs` → `input_examples`
- See Appendix D.4 Anthropic Mapping

**`generic` Profile:**
- Schema: Full output, no conversions
- For apcore internal use, debugging, documentation generation

---

## 5. Module Specification

### 5.1 Module File Structure

Each module consists of the following parts:

```
extensions/{layer}/{type}/{module_name}.{ext}      # Module implementation
extensions/{layer}/{type}/{module_name}_meta.yaml  # Module metadata (optional)
schemas/{canonical_id}.schema.yaml              # Schema definition
```

### 5.2 Metadata File and Entry Point Resolution

Implementations **MUST** resolve module entry points according to the following algorithm:

```
Algorithm: resolve_entry_point(meta_yaml, file_path, language)

Input:
  meta_yaml  — Metadata file content (may be null)
  file_path  — Module file path
  language   — File language (determined by extension)

Output:
  entry_point — { file: string, class_name: string }

Steps:
  1. If meta_yaml exists and contains entry_point field:
     a. Parse format "filename:ClassName"
     b. Return { file: filename, class_name: ClassName }
  2. Otherwise, auto-infer:
     a. file ← filename from file_path (without extension)
     b. class_name ← Convert file from snake_case to PascalCase
     c. If language == "python": Look for class inheriting Module in file
     d. If unique match found → Return that class
     e. If multiple matches found → Throw AMBIGUOUS_ENTRY_POINT error
     f. If no match found → Throw NO_MODULE_CLASS error
  3. Return entry_point
```

```yaml
# extensions/executor/validator/db_params_meta.yaml

# Note: module_id and group are auto-generated from directory, no manual specification needed

# Module description
description: "Database parameter validator"

# Entry point (optional, defaults to inference from filename)
entry_point: "db_params:DbParamsValidator"

# Allowed callers to this module (ACL)
allowed_callers:
  - "orchestrator.engine.*"        # Wildcard
  - "api.handler.task_submit"      # Exact match

# Dependencies on other modules (see 5.3 Dependency Management)
dependencies:
  - module_id: "common.util.sql_parser"
    version: ">=1.0.0"             # Version constraint (optional)
    optional: false                 # Whether optional dependency

# Tags (for categorization and search)
tags:
  - database
  - validation
  - security

# Version (optional)
version: "1.0.0"

# Behavior annotations (optional, help AI make invocation decisions)
annotations:
  readonly: false
  destructive: false
  idempotent: true
  requires_approval: false
  open_world: false

# Usage examples (optional, help AI understand complex modules)
examples:
  - title: "Validate SELECT statement"
    inputs:
      table: "user_info"
      sql: "SELECT * FROM user_info WHERE id = 1"
    output:
      valid: true
      message: "Validation passed"

  - title: "Detect dangerous SQL"
    inputs:
      table: "user_info"
      sql: "DROP TABLE user_info"
    output:
      valid: false
      errors:
        - field: "sql"
          code: "DANGEROUS_SQL"
          message: "SQL contains dangerous keyword: DROP"

# Extension metadata (optional, free dict)
metadata:
  owner: "database-team"
  avg_latency_ms: 5

# Deprecation marking (optional)
deprecated: false
deprecated_message: null
replacement: null
sunset_date: null                    # ISO 8601 date after which module MAY be removed (e.g., "2026-06-01")

# Resource limits (optional, for sandbox isolation)
resources:
  timeout: 30000                   # Execution timeout (ms)
  memory_limit: null               # Memory limit (bytes), null=no limit
```

### 5.3 Dependency Management

Implementations **MUST** use topological sorting to resolve dependency order and **MUST** detect circular dependencies.

```
Algorithm: resolve_dependencies(modules)

Input:
  modules — Module set, each module contains { id, dependencies[] }

Output:
  load_order — List of module IDs sorted by dependency order

Steps:
  1. Build dependency graph: Map<module_id, Set<dependency_id>>
  2. Calculate in-degree: Map<module_id, int>
  3. queue ← All modules with in-degree 0
  4. load_order ← []
  5. While queue is not empty:
     a. current ← queue.dequeue()
     b. load_order.append(current)
     c. For each dependent of current:
        - in_degree[dependent] -= 1
        - If in_degree[dependent] == 0 → queue.enqueue(dependent)
  6. If len(load_order) < len(modules):
     - remaining ← Modules not added to load_order
     - Throw CIRCULAR_DEPENDENCY error with circular path
  7. Return load_order

Complexity: O(V + E), where V is number of modules, E is number of dependencies
```

```yaml
dependency_management:
  # Dependency declaration format
  declaration:
    simple: "common.util.sql_parser"           # Simple format: any version
    with_version:
      module_id: "common.util.sql_parser"
      version: ">=1.0.0,<2.0.0"                # Version constraint
      optional: false                           # Required dependency

  # Version constraint syntax (similar to npm/pip)
  version_constraints:
    - "1.0.0"        # Exact version
    - ">=1.0.0"      # Greater than or equal
    - "<2.0.0"       # Less than
    - ">=1.0.0,<2.0.0"  # Range
    - "^1.0.0"       # Compatible version (1.x.x)
    - "~1.0.0"       # Approximate version (1.0.x)

  # Dependency resolution rules
  resolution:
    strategy: "highest"            # highest | lowest | locked
    conflict_resolution: "error"   # error | use_highest | use_lowest

  # Circular dependencies
  circular_dependency: "error"     # Error when circular dependency detected

  # Optional dependencies
  optional_dependency:
    behavior: "skip_if_missing"    # Skip if missing, don't error
    check_method: "has_module(module_id) -> bool"
```

### 5.4 Multi-version Coexistence

```yaml
multi_version:
  # Whether to allow multiple versions of same module
  enabled: true

  # Version identification method
  identification:
    method: "file_suffix"          # Filename suffix
    pattern: "{name}_v{major}"     # e.g., db_params_v2.py

  # Version selection
  selection:
    default: "latest"              # Use latest version by default
    explicit: "module_id@version"  # Explicit specification: executor.validator.db_params@2

  # Examples
  examples:
    - file: "db_params.py"
      id: "executor.validator.db_params"
      version: "1.0.0"

    - file: "db_params_v2.py"
      id: "executor.validator.db_params_v2"
      version: "2.0.0"
      alias: "executor.validator.db_params@2"
```

### 5.5 Module Isolation (Optional)

```yaml
# [Implementation Phase: Phase 2]
module_isolation:
  # Isolation levels
  levels:
    none: "No isolation, shared process space"
    process: "Separate process"
    container: "Container isolation"

  # Resource limits
  resource_limits:
    timeout:
      type: integer
      unit: "ms"
      default: 30000
    memory:
      type: integer
      unit: "bytes"
      default: null              # null = no limit
    cpu:
      type: number
      unit: "cores"
      default: null

  # Sandbox configuration
  sandbox:
    network: true                # Allow network access
    filesystem: "readonly"       # readonly | readwrite | none
    allowed_paths: []            # Allowed access paths
```

### 5.6 Module Interface Protocol

All modules **MUST** provide the following interface. Modules can be defined via a **decorator** (primary approach), a **class-based pattern** (no ABC inheritance required), or a **function call**. Implementations **MUST NOT** require modules to inherit from an abstract base class.

**Primary approach: Decorator**

The `@module` decorator (or `module()` function call) is the recommended way to define modules. It wraps a callable and auto-generates Schema from type annotations:

```python
from apcore import module

@module(id="email.send", tags=["email"])
def send_email(to: str, subject: str, body: str) -> dict:
    """Send email to specified recipient"""
    return {"success": True, "message_id": "msg_123"}
```

**Alternative: Class-based modules**

Class-based modules provide an `execute()` method and `input_schema` / `output_schema` / `description` attributes. No ABC inheritance is required:

```python
class SendEmailModule:
    input_schema = SendEmailInput
    output_schema = SendEmailOutput
    description = "Send email to specified recipient"

    def execute(self, inputs: dict, context: Context) -> dict:
        return {"success": True, "message_id": "msg_123"}
```

**Module Interface Contract (language-agnostic pseudocode):**

```
Interface: Module

  Required implementations:
    execute(inputs: Map<String, Any>, context: Context) → Map<String, Any>
      Precondition: inputs has passed input_schema validation
      Postcondition: Return value must conform to output_schema
      Exception: ModuleError

  Optional implementations:
    validate(inputs: Map<String, Any>) → ValidationResult
    preflight(inputs: Map<String, Any>, context: Context) → List<String>
      # Advisory pre-execution warnings. Returning warnings does NOT block execution.
      # If preflight() raises, the exception is caught and reported as a warning.
    preview(inputs: Map<String, Any>, context: Context) → PreviewResult?
      # Structured prediction of state changes the call would produce.
      # Returns null if prediction is unavailable (e.g., target not found).
      # Called by Executor.validate() after preflight(); result is folded into
      # PreflightResult.predicted_changes. If preview() raises, treated as
      # advisory warning (mirrors preflight semantics).
      # See §12.8 for PreviewResult / Change schema.
    on_load() → void
    on_unload() → void
    on_suspend() → Map<String, Any>?
      # Called before hot-reload. Returns serializable state to preserve.
      # Returns null if no state needs preservation.
    on_resume(state: Map<String, Any>) → void
      # Called after hot-reload. Restores state from on_suspend().
      # Only called if on_suspend() returned non-null state.

  Required definitions:
    input_schema: SchemaDefinition    // Input Schema
    output_schema: SchemaDefinition   // Output Schema
    description: String               // Module description

  Optional definitions:
    name: String?                     // Human-readable name
    tags: List<String>                // Tag list
    version: String                   // Semantic version
    annotations: ModuleAnnotations?   // Behavior annotations
    examples: List<ModuleExample>     // Usage examples
    metadata: Map<String, Any>        // Extension metadata
```

**Cross-language Implementation Mapping:**

| Pseudocode Interface | Python | Rust | Go | Java | TypeScript |
|-----------|--------|------|----|------|------------|
| Module definition | `@module` decorator or class with `execute()` | `trait Module` or struct | `type Module interface` or struct | `interface Module` or class | `@module` decorator or class with `execute()` |
| `execute()` | `def execute(self, inputs, context)` | `fn execute(&self, inputs, context)` | `func (m) Execute(inputs, ctx)` | `Map execute(Map, Context)` | `execute(inputs, context)` |
| `input_schema` | `ClassVar[Type[BaseModel]]` | `type InputSchema: Serialize` | `InputSchema struct` | `Class<? extends Schema>` | `static inputSchema: ZodSchema` |
| `Map<String, Any>` | `dict[str, Any]` | `HashMap<String, Value>` | `map[string]any` | `Map<String, Object>` | `Record<string, unknown>` |

All modules **MUST** implement the following interface:

```yaml
module_interface:
  # Required methods
  required_methods:
    - name: "execute"
      description: "Execute module main logic"
      input: "Defined by input_schema"
      output: "Defined by output_schema"
  # Required attributes (mirrors the "Required definitions" block in the
  # pseudocode interface above; every module MUST expose these)
  required_attributes:
    - name: "input_schema"
      type: "SchemaDefinition"
      description: "JSON Schema (or language-native equivalent) for the inputs accepted by execute()"

    - name: "output_schema"
      type: "SchemaDefinition"
      description: "JSON Schema (or language-native equivalent) for the value returned by execute()"

    - name: "description"
      type: "string"
      description: "Human/AI-readable summary of what the module does (≤200 chars recommended)"

  # Optional attributes
  optional_attributes:
    - name: "name"
      type: "string"
      description: "Human-readable module name"
      default: "Auto-generated from class name"

    - name: "tags"
      type: "list[string]"
      description: "Tags for categorization and search"
      default: "[]"

    - name: "version"
      type: "string"
      description: "Module version"
      default: "1.0.0"

    - name: "annotations"
      type: "ModuleAnnotations"
      description: "Behavior annotations, help AI make invocation decisions"
      default: "Default values (readonly=false, destructive=false, ...)"

    - name: "examples"
      type: "list[ModuleExample]"
      description: "Usage examples, help AI understand complex modules"
      default: "[]"

    - name: "metadata"
      type: "dict[str, Any]"
      description: "Free extension metadata"
      default: "{}"

  # Optional implementations
  optional_methods:
    - name: "validate"
      description: "Validate input only, don't execute"
      input: "Defined by input_schema"
      output: "{ valid: bool, errors: array }"

    - name: "describe"
      description: "Return module description (for LLM)"
      input: "None"
      output: "{ description: string, input_schema: object, output_schema: object, annotations: object, examples: array }"

    - name: "preflight"
      description: "Domain-specific pre-execution warnings (called by Executor.validate() Check 7)"
      input: "(inputs: dict, context: Context)"
      output: "list[str] — warning messages, or empty list if no warnings"
      note: "Advisory only — returning warnings does NOT block execution. If preflight() raises, the exception is caught and reported as a warning."

    - name: "preview"
      description: "Structured prediction of state changes the call would produce (called by Executor.validate() after preflight)"
      input: "(inputs: dict, context: Context)"
      output: "PreviewResult | null — { changes: List<Change> } or null if prediction unavailable"
      note: "Optional. Modules that don't implement it leave PreflightResult.predicted_changes empty. If preview() raises (sync throw or async reject), treated as advisory warning via a `module_preview` check entry — does NOT fail validation. See §12.8 for the PreviewResult / Change schema."

  # Lifecycle hooks
  lifecycle_hooks:
    - name: "on_load"
      description: "Called when module loads"
    - name: "on_unload"
      description: "Called when module unloads"
    - name: "on_suspend"
      description: "Called before hot-reload to export serializable state"
      return: "dict | null — serializable state to preserve, or null"
    - name: "on_resume"
      description: "Called after hot-reload to restore previously exported state"
      input: "state: dict — state returned by on_suspend() of the previous instance"
```

### 5.7 Context Parameter Specification

Each module invocation passes a `context` parameter containing runtime context information. In cross-process scenarios, Context **MUST** support serialization for transport.

**Design Principle**: Only fields that the framework execution engine depends on are independent fields, everything else goes in `data` (referencing Go `context.Context`, OpenTelemetry Context design philosophy).

```yaml
context_schema:
  type: object
  properties:
    # ====== Framework engine dependencies (breaks if removed) ======

    trace_id:
      type: string
      pattern: "^[0-9a-f]{32}$"
      description: "Request trace ID, 32-char lowercase hex (W3C Trace Context compatible)"
      required: true
      example: "4bf92f3577b34da6a3ce929d0e0e4736"

    caller_id:
      type: string
      nullable: true
      description: "Caller's Canonical ID (null for top-level calls)"
      example: "orchestrator.engine.task_flow"

    call_chain:
      type: array
      items:
        type: string
      description: "Complete call chain (for loop detection, depth limiting, ACL)"
      example: ["api.handler.task_submit", "orchestrator.engine.task_flow"]

    executor:
      description: "Executor reference (for calling other modules, runtime injected)"

    # ====== Almost all users need, semantically stable ======

    identity:
      type: object
      nullable: true
      description: "Caller identity (ACL engine depends on)"
      properties:
        id:
          type: string
          description: "Unique identifier"
        type:
          type: string
          examples: [user, service, agent, api_key, system, ai]
          default: "user"
          description: "Identity type (free-form string; well-known values shown in examples)"
        roles:
          type: array
          items:
            type: string
          description: "Role list (ACL depends on)"
        attrs:
          type: object
          description: "Extension attributes (tenant_id, email, etc. business fields)"

    # ====== Everything else ======

    data:
      type: object
      description: "Shared pipeline state (reference passing, readable/writable along call chain)"

    # ====== Optional extension fields (MAY be provided by implementations) ======

    cancel_token:
      nullable: true
      description: "Cooperative cancellation token for long-running operations (MAY)"

    services:
      type: object
      nullable: true
      description: "Dependency injection container for sharing services across the call chain (MAY)"

    redacted_inputs:
      type: object
      nullable: true
      description: "Copy of inputs with x-sensitive fields replaced by REDACTED_VALUE, for safe logging (MAY)"
```

**Field Classification Rationale:**

| Field | Classification | Reason |
|------|------|------|
| `trace_id` | Framework engine dependency | Executor generates, middleware/logging depends on across chain |
| `caller_id` | Framework engine dependency | ACL engine determines "who is calling whom" |
| `call_chain` | Framework engine dependency | Loop detection, depth limiting, ACL path matching |
| `executor` | Framework engine dependency | Only channel for inter-module calls |
| `identity` | Widely needed | ACL is framework first-class citizen, needs standardized "who" |
| `data` | Universal bag | span_id, locale, pipeline intermediate state, etc. all mutable data |
| `cancel_token` | Optional extension | Cooperative cancellation for timeout enforcement |
| `services` | Optional extension | DI container for framework integrations |
| `redacted_inputs` | Optional extension | Safe logging of sensitive inputs |

**Note on external correlation IDs:**

The `trace_id` field is owned by the apcore framework and follows the strict 32-char lowercase hex format for W3C Trace Context interoperability. Existing projects that already emit their own request or correlation identifiers (e.g., `X-Request-ID`, `X-Correlation-ID`, ULID, AWS X-Ray trace headers) **SHOULD** preserve those values in `context.data["x-correlation-id"]` and **MUST NOT** overwrite `trace_id` with them. Framework integrations **SHOULD** populate both at the context boundary so distributed tracing and legacy business logs remain correlatable side-by-side. See the [Integrating into Existing Projects](../guides/integrating-existing-projects.md) guide for examples.

**Context Serialization Specification (Cross-process Scenarios):**

In cross-process/cross-network invocation scenarios, Context **MUST** be serializable to JSON format:

```yaml
context_serialization:
  format: "JSON"
  rules:
    - "trace_id: MUST serialize as string"
    - "caller_id: MUST serialize as string | null"
    - "call_chain: MUST serialize as string[]"
    - "executor: MUST NOT serialize (runtime injected)"
    - "identity: MUST serialize as JSON object"
    - "data: SHOULD serialize, but MUST exclude non-serializable values"
    - "When data contains functions/connections etc. non-serializable values, MUST silently skip and log warning"
    - "cancel_token: MUST NOT serialize (runtime object)"
    - "services: MUST NOT serialize (runtime injected)"
    - "redacted_inputs: MAY serialize as JSON object"
```

### 5.8 Async Module Specification

Async module state transitions **MUST** follow this state machine:

```
                    ┌──────────────────────────────────────────┐
                    │                                          │
                    ▼                                          │
  ┌──────┐    ┌─────────┐    ┌─────────┐    ┌───────────┐    │
  │ idle │───▶│ pending │───▶│ running │───▶│ completed │    │
  └──────┘    └─────────┘    └────┬────┘    └───────────┘    │
                                  │                            │
                                  ├───▶ ┌────────┐             │
                                  │     │ failed │             │
                                  │     └────────┘             │
                                  │                            │
                                  └───▶ ┌───────────┐          │
                                        │ cancelled │──────────┘
                                        └───────────┘
                                        (can resubmit)

  State transition rules:
    idle → pending       : When submit() is called on AsyncTaskManager
    pending → running    : When executor starts processing
    running → completed  : When execution succeeds
    running → failed     : When execution throws exception
    running → cancelled  : When cancel() is called
    cancelled → pending  : When resubmitted (MAY support)
    failed → pending     : When retrying (MAY support)

  Forbidden transitions:
    completed → *        : Completed tasks MUST NOT transition to other states
    idle → running       : MUST NOT skip pending state
```

For long-running modules, async execution mode is supported:

```yaml
async_module:
  # Async execution methods
  methods:
    # Start async task (framework invokes execute() asynchronously via AsyncTaskManager.submit())
    submit:
      input: "Defined by input_schema"
      output:
        type: object
        properties:
          task_id:
            type: string
            description: "Async task ID"
          status:
            type: string
            enum: [pending, running]

    # Query task status
    get_status:
      input:
        task_id: string
      output:
        type: object
        properties:
          status:
            type: string
            enum: [pending, running, completed, failed, cancelled]
          progress:
            type: number
            minimum: 0
            maximum: 100
          result:
            description: "Result when task completes (per output_schema)"
          error:
            description: "Error info when task fails"

    # Cancel task
    cancel:
      input:
        task_id: string
      output:
        type: object
        properties:
          success: boolean

  # Callback mechanism (optional)
  callbacks:
    on_progress:
      description: "Progress update callback"
    on_complete:
      description: "Task completion callback"
    on_error:
      description: "Task failure callback"
```

### 5.9 Inter-module Communication

Modules call other modules through `Executor`:

```yaml
module_communication:
  # Invocation methods
  methods:
    # Sync call
    sync_call:
      signature: "executor.call(module_id, inputs, context)"
      returns: "output or raises ModuleError"

    # Async call
    async_call:
      signature: "executor.call_async(module_id, inputs, context)"
      returns: "Future[output]"

  # Invocation constraints
  constraints:
    - "Must go through Executor, direct instantiation not allowed"
    - "Invocations subject to ACL rules"
    - "Call chain automatically recorded to context.call_chain"
    - "Avoid circular calls (framework detects and errors)"

  # Example (Python)
  example: |
    class MyModule(Module):
        def execute(self, inputs: dict, context: Context) -> dict:
            # Call other module
            result = context.executor.call(
                module_id="common.util.sql_parser",
                inputs={"sql": inputs["sql"]},
                context=context
            )
            return {"parsed": result}
```

### 5.10 Module Implementation Example (Python)

```python
# extensions/executor/validator/db_params.py

from apcore import Module
from pydantic import BaseModel, Field
from typing import Optional

class DBParamsInput(BaseModel):
    """Input Schema - Keep consistent with YAML"""
    table: str = Field(..., pattern=r"^[a-z][a-z0-9_]*$")
    sql: str
    timeout: int = Field(default=30, ge=1, le=300)

class DBParamsOutput(BaseModel):
    """Output Schema"""
    valid: bool
    message: Optional[str] = None
    errors: list[dict] = []
    warnings: list[str] = []

class DbParamsValidator(Module):
    """Database parameter validator"""

    # Schema declaration (optional if using YAML)
    input_schema = DBParamsInput
    output_schema = DBParamsOutput

    # Behavior annotations: AI knows this is readonly check, idempotent, no side effects
    annotations = ModuleAnnotations(
        readonly=True,
        destructive=False,
        idempotent=True,
        requires_approval=False,
        open_world=False
    )

    # Usage examples
    examples = [
        ModuleExample(
            title="Validate safe SQL",
            inputs={"table": "user_info", "sql": "SELECT * FROM user_info"},
            output={"valid": True, "message": "Validation passed", "errors": [], "warnings": []}
        )
    ]

    def execute(self, inputs: dict, context: Context) -> dict:
        """Execute validation"""
        # Input already validated by Schema
        params = DBParamsInput(**inputs)

        errors = []
        warnings = []

        # Business logic: Check dangerous SQL
        dangerous_keywords = ['DROP', 'TRUNCATE', 'DELETE']
        sql_upper = params.sql.upper()
        for kw in dangerous_keywords:
            if kw in sql_upper:
                errors.append({
                    "field": "sql",
                    "code": "DANGEROUS_SQL",
                    "message": f"SQL contains dangerous keyword: {kw}"
                })

        return DBParamsOutput(
            valid=len(errors) == 0,
            message="Validation passed" if not errors else "Validation failed",
            errors=errors,
            warnings=warnings
        ).model_dump()
```

### 5.11 Function-based Module Definition (Function-based Module Definition)

> Section name uses "Function-based Module Definition" rather than "Decorator Module Definition" because Decorator is language-specific syntax, while the core requirement is **semantic capability**—wrapping existing callables as standard modules.

#### 5.11.1 Normative Statement

Implementations **MUST** provide `module()` mechanism to wrap existing callables (functions or methods) as standard modules, auto-generating Schema from type information.

`module()` is the only core concept, available in both Decorator and function call forms:

- In languages supporting Decorator syntax (Python, TypeScript, Java annotations), **SHOULD** provide as Decorator form
- In languages not supporting Decorator (Go, C), **MUST** provide as function call form
- Both forms share the same parameter set, producing modules that **MUST** be completely equivalent in Registry/Executor/Schema behavior

#### 5.11.2 Cross-language Syntax Reference

| Language | Decorator Form | Function Call Form (Register Existing Code) |
|------|---------------|--------------------------|
| Python | `@module(id="email.send") def send(...)` | `module(service.send, id="email.send")` |
| TypeScript | `@module({id: "email.send"}) function send(...)` | `module(service.send, {id: "email.send"})` |
| Java | `@Module(id="email.send") public Map send(...)` | `Apcore.module(service::send, "email.send")` |
| Rust | `#[module(id = "email.send")] fn send(...)` | `apcore::module("email.send", \|inputs\| { ... })` |
| Go | — None | `apcore.Module("email.send", sendEmail)` / `apcore.Module("email.send", service.Send)` |
| C | — None | `apcore_module("email.send", &send_email)` |

#### 5.11.3 `module()` Parameter Signature

Both forms share the same parameter set (language-agnostic pseudocode):

```
module(callable?, options?) → Module

options:
  id: String?              // Module ID (optional, auto-generate if absent)
  description: String?     // Module description (optional, extract from docstring/comment if absent)
  annotations: Annotations? // Behavior annotations (optional)
  tags: List<String>?      // Tags (optional)
  version: String?         // Version (optional, default "1.0.0")
  metadata: Map?           // Extension metadata (optional)
```

- Decorator form: `callable` implicitly provided by decorator target, `options` as decorator parameters
- Function call form: `callable` explicitly passed (first parameter), `options` as subsequent parameters

#### 5.11.4 Type Inference and Schema Auto-generation

Implementations **MUST** auto-generate JSON Schema from function signatures according to the following algorithm:

```
Algorithm: generate_schema_from_function(callable)

Input:
  callable — Target function or method

Output:
  schema — { input_schema: JSONSchema, output_schema: JSONSchema, description: String }

Steps:
  1. Extract function parameter list params (exclude self/cls and context: Context)
  2. For each param:
     a. Get type annotation type_hint
     b. If type_hint missing → Throw FUNC_MISSING_TYPE_HINT error
     c. Map type_hint to JSON Schema type (see §5.11.5)
     d. Extract constraints from Annotated metadata, default values, etc.
     e. Extract description from docstring parameter comments
  3. Construct input_schema:
     a. type: "object"
     b. properties: Schema mapping of all parameters
     c. required: List of parameters without defaults
  4. Extract return type annotation return_type
     a. If return_type missing → Throw FUNC_MISSING_RETURN_TYPE error
     b. Map return_type to output_schema
  5. Extract description:
     a. docstring/comment first line → description
     b. Parameter comments → Each field's description
  6. Return { input_schema, output_schema, description }

Complexity: O(n), where n is number of parameters
```

#### 5.11.5 Language Type → JSON Schema Mapping Table

| Language Type | JSON Schema | Description |
|---------|-------------|------|
| `str` / `String` | `{ "type": "string" }` | String |
| `int` / `Integer` / `i32` | `{ "type": "integer" }` | Integer |
| `float` / `Double` / `f64` | `{ "type": "number" }` | Float |
| `bool` / `Boolean` | `{ "type": "boolean" }` | Boolean |
| `list[T]` / `Vec<T>` / `[]T` | `{ "type": "array", "items": <T> }` | Array |
| `dict[str, T]` / `Map<String, T>` | `{ "type": "object", "additionalProperties": <T> }` | Map |
| `Optional[T]` / `T \| None` | `<T>` + `"nullable": true` | Nullable type |
| `BaseModel` / `struct` / `@dataclass` | `{ "type": "object", "properties": {...} }` | Struct/object |
| `Literal["a", "b"]` / `enum` | `{ "type": "string", "enum": ["a", "b"] }` | Enum |
| `Annotated[T, Field(...)]` | `<T>` + constraint fields | Constrained type |

#### 5.11.6 Module ID Generation Rules

When `module()` doesn't specify `id` parameter, **MUST** auto-generate from function full path:

```
Rule: generate_module_id(callable)

Steps:
  1. Get callable's module path (e.g., "myapp.services.email")
  2. Get callable's name (e.g., "send_email")
  3. Combine as "{module_path}.{name}"
  4. Normalize to Canonical ID format (§2.7)
  5. If ID already exists → Throw duplicate_id error (§2.6)
```

#### 5.11.7 Description Extraction Rules

Implementations **MUST** extract module description by the following priority:

1. `module()`'s `description` parameter (highest priority)
2. Function docstring / comment first line
3. If neither above → Generate default description from function name

Parameter-level description:
- Extract from docstring's Args/Parameters section
- Extract from `Annotated[T, Field(description="...")]`
- Extract from language comments

#### 5.11.8 Sync/Async Function Support

| Function Type | Mapping |
|---------|------|
| `def func(...)` | `execute()` |
| `async def func(...)` | `execute()` |

Implementations **SHOULD** support both sync and async functions. The framework auto-detects whether a function is sync or async and handles invocation accordingly. Both map to the module's `execute()` method.

#### 5.11.9 Context Injection

When function parameters include `context: Context` type annotation, framework **MUST** auto-inject Context object, and this parameter **MUST NOT** appear in generated `input_schema`.

```python
# context parameter auto-injected, doesn't appear in Schema
@module(id="email.send")
def send_email(to: str, subject: str, body: str, context: Context) -> dict:
    print(f"trace_id: {context.trace_id}")
    return {"success": True}

# Generated input_schema only contains to, subject, body
```

#### 5.11.10 Equivalence Guarantee

Function-defined modules and class-defined modules **MUST** be completely equivalent in:

- Registration and discovery behavior in Registry
- Executor's invocation flow (Schema validation → ACL → Middleware → Execution)
- Schema format and validation logic
- Error handling and error codes
- Observability (tracing, logging, metrics)

#### 5.11.11 Error Codes

| Error Code | Description | Trigger Condition |
|--------|------|---------|
| `FUNC_MISSING_TYPE_HINT` | Function parameter missing type annotation | Parameter has no type annotation and can't be inferred |
| `FUNC_MISSING_RETURN_TYPE` | Function missing return type annotation | Return type has no annotation and can't be inferred |

#### 5.11.12 Examples

**Python — Decorator Form:**

```python
from apcore import module, Context
from typing import Annotated
from pydantic import Field

@module(id="email.send", tags=["email", "notification"])
def send_email(
    to: Annotated[str, Field(description="Recipient email")],
    subject: Annotated[str, Field(description="Email subject", max_length=200)],
    body: Annotated[str, Field(description="Email body")],
    context: Context
) -> dict:
    """Email sending module"""
    # Original business logic
    return {"success": True, "message_id": "msg_123"}
```

**Python — Function Call Form (Register Existing Code):**

```python
from apcore import module

class EmailService:
    def send(self, to: str, subject: str, body: str) -> dict:
        """Send email"""
        return {"success": True}

service = EmailService()
module(service.send, id="email.send")
```

**Go — Function Call Form:**

```go
package main

import "github.com/apcore/apcore-go"

func sendEmail(to string, subject string, body string) map[string]any {
    return map[string]any{"success": true}
}

func main() {
    apcore.Module("email.send", sendEmail)

    // Register existing struct method
    service := &EmailService{}
    apcore.Module("email.send_template", service.SendTemplate)
}
```

---

### 5.12 External Schema Binding (External Schema Binding)

#### 5.12.1 Normative Statement

Implementations **MUST** support external Schema binding files, allowing existing functions to be mapped as apcore modules through YAML configuration, achieving zero-code-modification integration.

#### 5.12.2 Binding File Format

Binding files **MUST** be in YAML format, containing a `bindings` array:

```yaml
# bindings/email.binding.yaml
bindings:
  - module_id: "email.send"
    target_id: "myapp.services.email:send_email"
    description: "Send email"
    input_schema:
      type: object
      properties:
        to:
          type: string
          description: "Recipient email"
        subject:
          type: string
          description: "Email subject"
        body:
          type: string
          description: "Email body"
      required: [to, subject, body]
    output_schema:
      type: object
      properties:
        success:
          type: boolean
        message_id:
          type: string

  - module_id: "email.send_template"
    target_id: "myapp.services.email:EmailService.send_template"
    description: "Send email using template"
    auto_schema: true  # Auto-generate Schema from type annotations
```

**Binding Item Field Definitions:**

| Field | Type | Required | Description |
|------|------|------|------|
| `module_id` | string | **MUST** | Module Canonical ID |
| `target_id` | string | **MUST** | Target callable (format: `module.path:callable_name`) |
| `description` | string | **SHOULD** | Module description |
| `input_schema` | object | Conditional | Input Schema (choose one with `auto_schema`) |
| `output_schema` | object | Conditional | Output Schema (choose one with `auto_schema`) |
| `auto_schema` | boolean | Conditional | Auto-generate Schema from type annotations (choose one with explicit Schema) |
| `schema_ref` | string | **MAY** | Reference external Schema file path |
| `annotations` | object | **MAY** | Behavior annotations |
| `tags` | array | **MAY** | Tags |
| `version` | string | **MAY** | Version |
| `metadata` | object | **MAY** | Extension metadata |

#### 5.12.3 Target Resolution Algorithm

Implementations **MUST** resolve `target_id` field according to the following algorithm:

```
Algorithm: resolve_target(target_string)

Input:
  target_string — Target callable path (e.g., "myapp.services.email:send_email")

Output:
  callable — Callable object

Preconditions:
  - target_string conforms to "module.path:callable_name" format

Steps:
  1. Split target_string by ":" into (module_path, callable_name)
     If no ":" → Throw BINDING_INVALID_TARGET error
  2. import module_path
     If import fails → Throw BINDING_MODULE_NOT_FOUND error
  3. If callable_name contains ".":
     a. Split by "." into (class_name, method_name)
     b. Find class_name in module
     c. Find method_name on class instance
     d. If any step fails → Throw BINDING_CALLABLE_NOT_FOUND error
  4. Otherwise:
     a. Find callable_name in module
     b. If not found → Throw BINDING_CALLABLE_NOT_FOUND error
  5. Validate result is callable
     If not callable → Throw BINDING_NOT_CALLABLE error
  6. Return callable

Complexity: O(1) (not counting module loading time)
```

#### 5.12.4 Schema Reference Support

Binding items **may** reference external Schema files via `schema_ref`, avoiding inlining complete Schema in binding file:

```yaml
bindings:
  - module_id: "email.send"
    target_id: "myapp.services.email:send_email"
    schema_ref: "../schemas/email.send.schema.yaml"
```

#### 5.12.5 `auto_schema` Mode

When `auto_schema: true`, implementations **MUST** reuse the `generate_schema_from_function` algorithm from §5.11.4 to auto-generate Schema from target_id callable's type annotations.

If target_id callable lacks sufficient type information, **MUST** throw `BINDING_SCHEMA_INFERENCE_FAILED` error. (`BINDING_SCHEMA_MISSING` is the deprecated 0.19.0 alias, retained only for decoding older serialized payloads.)

#### 5.12.6 Discovery Mechanism

Binding file discovery is controlled via `apcore.yaml` configuration:

```yaml
# apcore.yaml
bindings:
  dir: "./bindings"          # Default scan directory
  files:                      # Or specify file list
    - "./bindings/email.binding.yaml"
    - "./bindings/payment.binding.yaml"
  pattern: "*.binding.yaml"  # File matching pattern (default)
```

- If `bindings.dir` is configured, implementations **MUST** scan files matching `pattern` in that directory
- If `bindings.files` is configured, implementations **MUST** load specified file list
- If neither configured, implementations **SHOULD** default to scanning `bindings/` directory

#### 5.12.7 Validation Rules

Implementations **MUST** perform the following validations when loading binding files:

1. `module_id` conforms to Canonical ID format (§2.7)
2. `target_id` can be resolved to valid callable
3. Schema is valid (explicitly defined or auto_schema can generate)
4. `module_id` doesn't conflict with registered modules (§2.6)
5. Binding file itself conforms to `binding.schema.json` (see `schemas/binding.schema.json`)

#### 5.12.8 Error Codes

| Error Code | Description | Trigger Condition |
|--------|------|---------|
| `BINDING_INVALID_TARGET` | target_id format invalid | target_id doesn't conform to `module.path:callable_name` format |
| `BINDING_MODULE_NOT_FOUND` | Module path can't be imported | import module_path fails |
| `BINDING_CALLABLE_NOT_FOUND` | Can't find target_id callable | Can't find specified function/method in module |
| `BINDING_NOT_CALLABLE` | Target not callable | Resolved object is not callable |
| `BINDING_SCHEMA_INFERENCE_FAILED` | Schema inference failed | No explicit Schema and auto_schema can't generate (deprecated alias: `BINDING_SCHEMA_MISSING`) |

### 5.13 Display Overlay (Surface-Facing Presentation)

#### 5.13.1 Normative Statement

Implementations **MUST** support an optional `display` section in binding entries, allowing users to override how modules are presented across different surfaces (CLI, MCP, A2A) without changing the canonical `module_id` or registry behavior.

The `display` overlay is a **sparse override** mechanism: binding files need only declare overrides for a subset of scanned modules. Modules without a `display` entry **MUST** use the scanner-provided values unchanged.

#### 5.13.2 Motivation

Scanner-generated module IDs and descriptions are derived from source code artifacts (operationId, function names, docstrings) that are often too verbose, too technical, or inconsistent for AI/LLM consumption across CLI commands, MCP tool names, and A2A skill names. Different surfaces have different naming constraints:

| Surface | Name Constraints | Example |
|---------|-----------------|---------|
| CLI | Shell-friendly, ≤40 chars recommended, no spaces | `pay-status` |
| MCP | Tool name, ≤64 chars, no spaces, alphanumeric + `_-` | `check_payment_status` |
| A2A | Skill name, natural language allowed, no hard limit | `Payment Status Checker` |

Without `display`, users must choose one `module_id` format that compromises across all surfaces. With `display`, each surface gets its optimal presentation while the canonical `module_id` remains stable for programmatic use.

#### 5.13.3 Display Section Schema

```yaml
bindings:
  - module_id: "credit_purchase.get_purchase_status_by_payment_intent.get"
    target_id: "myapp.purchase:get_purchase_status"
    description: "Auto-generated from docstring"

    # Display overlay — does NOT change module_id or registry behavior
    display:
      # Default overrides (apply to all surfaces unless surface-specific override exists)
      alias: "purchase-status"
      description: "Check purchase payment status by Stripe PaymentIntent ID"
      documentation: |
        Query Stripe PaymentIntent to get purchase status.
        Returns payment state, amount, and creation timestamp.
      guidance: |
        Use when the user asks about payment status or purchase confirmation.
        Do NOT use for refunds — use payment.refund instead.
        Always pass the Stripe PaymentIntent ID (pi_xxx), not the charge ID (ch_xxx).
        Returns null if the PaymentIntent has no associated purchase record.
      tags: ["billing", "payment"]

      # Surface-specific overrides (optional, take precedence over defaults above)
      cli:
        alias: "pay-status"
        description: "Check Stripe payment"
      mcp:
        alias: "check_payment_status"
        description: "Look up purchase payment status by Stripe PaymentIntent ID"
      a2a:
        alias: "Payment Status Checker"
        description: "I can check the status of any purchase payment"
```

**Display Section Field Definitions:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `display` | object | **MAY** | Display overlay container |
| `display.alias` | string | **MAY** | Default display name for all surfaces |
| `display.description` | string | **MAY** | Override description for all surfaces |
| `display.documentation` | string | **MAY** | Override extended documentation |
| `display.guidance` | string | **MAY** | Usage guidance for humans and AI — when to use, when NOT to use, common mistakes, parameter tips. Read by both `describe` command and AI tool context injection. No length limit. |
| `display.tags` | array | **MAY** | Override tags |
| `display.cli` | object | **MAY** | CLI-specific overrides |
| `display.cli.alias` | string | **MAY** | CLI command name. Shell-safe (`^[a-z][a-z0-9_-]*$`). Recommended ≤40 chars, not enforced. |
| `display.cli.description` | string | **MAY** | CLI help text |
| `display.cli.guidance` | string | **MAY** | CLI-specific guidance, overrides `display.guidance` for CLI surface |
| `display.mcp` | object | **MAY** | MCP-specific overrides |
| `display.mcp.alias` | string | **MAY** | MCP tool name. **Hard limit: 64 chars** (OpenAI function name spec). Pattern: `^[a-zA-Z_][a-zA-Z0-9_-]*$` |
| `display.mcp.description` | string | **MAY** | MCP tool description |
| `display.mcp.guidance` | string | **MAY** | MCP-specific guidance injected into tool context for AI tool selection |
| `display.a2a` | object | **MAY** | A2A-specific overrides |
| `display.a2a.alias` | string | **MAY** | A2A skill name. Natural language allowed, no hard length limit. |
| `display.a2a.description` | string | **MAY** | A2A skill description |
| `display.a2a.guidance` | string | **MAY** | A2A-specific guidance for agent routing decisions |

#### 5.13.4 Sparse Overlay Semantics

The `display` section is a **sparse overlay**, not a full declaration:

1. **Module-level sparsity**: If a binding file covers 2 out of 10 scanned modules, only those 2 get display overrides; the remaining 8 use scanner-provided values.

2. **Field-level sparsity**: Within a `display` section, only specified fields override. Unspecified fields inherit from the next level in the resolve chain.

3. **Surface-level sparsity**: Surface-specific sections (`cli`, `mcp`, `a2a`) only need to declare fields that differ from `display.*` defaults. Missing surface sections inherit the `display.*` defaults.

```yaml
bindings:
  # Only override alias, everything else uses scanner values
  - module_id: "order.create_order.post"
    display:
      alias: "create-order"

  # Only override CLI description, MCP and A2A use display.description
  - module_id: "user.get_profile.get"
    display:
      description: "Retrieve user profile by ID"
      cli:
        description: "Get user profile"  # shorter for CLI help text
```

#### 5.13.5 Resolve Priority Chain

Implementations **MUST** resolve display fields using the following priority chain (highest to lowest):

```
Algorithm: resolve_display(module_id, surface, binding_map, scanned_module)

For alias:
  1. binding_map[module_id].display.{surface}.alias    (surface-specific)
  2. binding_map[module_id].display.alias               (cross-surface default)
  3. scanned_module.metadata.suggested_alias             (scanner auto-alias)
  4. scanned_module.module_id                            (canonical ID)

For description:
  1. binding_map[module_id].display.{surface}.description
  2. binding_map[module_id].display.description
  3. binding_map[module_id].description                  (binding-level description)
  4. scanned_module.description                          (scanner-provided)

For documentation:
  1. binding_map[module_id].display.documentation
  2. binding_map[module_id].documentation                    (binding-level)
  3. scanned_module.documentation                            (scanner-provided)

For guidance:
  1. binding_map[module_id].display.{surface}.guidance
  2. binding_map[module_id].display.guidance
  3. null                                                (no scanner fallback — guidance is user-authored)

For tags:
  1. binding_map[module_id].display.tags
  2. binding_map[module_id].tags                         (binding-level)
  3. scanned_module.tags                                 (scanner-provided)
```

If `binding_map[module_id]` does not exist (no binding entry for this module), all fields resolve to the `scanned_module` values directly. `guidance` resolves to null.

#### 5.13.6 Surface Alias Naming Constraints

Implementations **MUST** enforce the MCP alias constraint and **SHOULD** warn on CLI pattern violations:

| Surface | Pattern | Max Length | Level |
|---------|---------|-----------|-------|
| CLI | `^[a-z][a-z0-9_-]*$` | Recommended ≤40, not enforced | **SHOULD** warn |
| MCP | `^[a-zA-Z_][a-zA-Z0-9_-]*$` | **64 (hard limit, OpenAI spec)** | **MUST** enforce |
| A2A | Free-form UTF-8 | No hard limit | No constraint |
| Default (`display.alias`) | `^[a-z][a-z0-9_-]*$` | 64 | **SHOULD** warn |

If a surface-specific alias violates a **MUST** constraint, implementations **MUST** reject it with a validation error. For **SHOULD** constraints, implementations **SHOULD** log a warning and continue.

#### 5.13.7 Scanner `suggested_alias` Field

Framework scanners (e.g., `OpenAPIScanner`, NestJS decorator scanner) **MAY** produce a `suggested_alias` in `ScannedModule.metadata` as a hint for the display resolver. This replaces the previous `simplify_ids` approach of directly modifying `module_id`.

```
Scanner behavior when simplify_ids=True:
  BEFORE (deprecated): module_id = simplified_name
  AFTER  (preferred): module_id = canonical_name
                      metadata.suggested_alias = simplified_name
```

This ensures the canonical `module_id` is always stable and predictable, while the simplified name is available as a fallback alias when no explicit `display.alias` is configured.

#### 5.13.8 Architectural Layering

The display overlay system is structured as three layers. Each layer has a clear responsibility and **MUST NOT** leak concerns to adjacent layers.

**Integration with the existing registry flow:**

```
Scanner
  │ ScannedModule[]
  ▼
DisplayResolver (apcore-toolkit)          ← NEW: runs BEFORE RegistryWriter
  │ ResolvedModule[]
  ├──→ RegistryWriter
  │       │ registers FunctionModule into Registry
  │       │ stores display fields in FunctionModule.metadata["display"]
  │       ▼
  │    Registry (module_id → FunctionModule)
  │
  └──→ Surface (CLI / MCP / A2A)
          │ registry.get_definition(module_id)
          │   → reads display fields from metadata["display"]
          ▼
        Surface-specific formatting
```

`DisplayResolver` runs once, before `RegistryWriter`. The resolved display fields travel alongside the module through the registry as `metadata["display"]`, and surfaces read them from there. This avoids surfaces needing direct access to the binding files at runtime.

**Layer responsibilities:**

```
┌─────────────────────────────────────────────────────────┐
│  Layer 1: Scanner (framework-specific)                  │
│  fastapi-apcore / nestjs-apcore / axum-apcore / ...    │
│                                                         │
│  Produces: ScannedModule with canonical module_id       │
│  Optionally: metadata.suggested_alias                   │
│  MUST NOT: modify module_id for display purposes        │
└─────────────────────┬───────────────────────────────────┘
                      │ ScannedModule[]
                      ▼
┌─────────────────────────────────────────────────────────┐
│  Layer 2: Display Resolver (apcore-toolkit)             │
│  Implemented once per language SDK                      │
│                                                         │
│  Inputs: ScannedModule[] + binding.yaml (optional)      │
│  Process: Parse display section, apply resolve chain    │
│  Produces: ResolvedModule[] (ScannedModule + display.*) │
│                                                         │
│  Pure data transformation — no framework dependency     │
│  Called by: RegistryWriter, YAMLWriter, surface init   │
└─────────────────────┬───────────────────────────────────┘
                      │ ResolvedModule[]
                      ▼
┌─────────────────────────────────────────────────────────┐
│  Layer 3a: RegistryWriter (apcore-toolkit)              │
│                                                         │
│  Stores display fields in FunctionModule.metadata       │
│  Key: "display" → serialized ResolvedDisplay struct     │
│  Registry key remains: canonical module_id              │
└─────────────────────┬───────────────────────────────────┘
                      │ Registry lookup: get_definition(module_id)
                      ▼
┌─────────────────────────────────────────────────────────┐
│  Layer 3b: Surface Formatter (per surface)              │
│  apcore-cli / apcore-mcp / apcore-a2a                  │
│                                                         │
│  Reads: descriptor.metadata["display"]                  │
│  CLI:  display.cli.alias → command name                 │
│  MCP:  display.mcp.alias → tool name                   │
│  A2A:  display.a2a.alias → skill name                  │
│                                                         │
│  Applies surface-specific formatting only               │
│  MUST NOT: re-resolve display fields                    │
└─────────────────────────────────────────────────────────┘
```

#### 5.13.9 `ResolvedModule` Type Definition

Implementations **MUST** define a `ResolvedModule` type (or equivalent) that carries both the canonical module data and resolved display fields:

```
ResolvedModule:
  # Canonical fields (from ScannedModule, unchanged)
  module_id: string                           # Canonical ID, used for registry key
  target_id: string                              # Callable reference
  input_schema: object                        # JSON Schema
  output_schema: object                       # JSON Schema
  version: string
  annotations: ModuleAnnotations | null

  # Resolved display fields (per-surface)
  display:
    alias: string                             # Resolved default alias
    description: string                       # Resolved default description
    documentation: string | null              # Resolved default documentation
    guidance: string | null                   # Resolved guidance (null if not authored)
    tags: string[]                            # Resolved tags

    cli:
      alias: string                           # Resolved CLI alias
      description: string                     # Resolved CLI description
      guidance: string | null                 # Resolved CLI guidance
    mcp:
      alias: string                           # Resolved MCP alias
      description: string                     # Resolved MCP description
      guidance: string | null                 # Resolved MCP guidance (injected into tool context)
    a2a:
      alias: string                           # Resolved A2A alias
      description: string                     # Resolved A2A description
      guidance: string | null                 # Resolved A2A guidance
```

Fields with a resolve chain fallback to scanner data are **always non-null** after resolution: `alias`, `description`, `tags`. Fields that are user-authored with no scanner fallback may be null: `documentation`, `guidance`.

#### 5.13.10 Implementation Responsibility Matrix

**Who implements what:**

| Component | Repo | Responsibility |
|-----------|------|---------------|
| `DisplayResolver` | `apcore-toolkit-{lang}` | Parse binding.yaml display section; apply resolve priority chain (§5.13.5); produce `ResolvedModule[]`; validate aliases (§5.13.6) |
| `RegistryWriter` update | `apcore-toolkit-{lang}` | Accept `ResolvedModule[]`; store `display.*` in `FunctionModule.metadata["display"]` |
| `YAMLWriter` update | `apcore-toolkit-{lang}` | Run `DisplayResolver` before writing `.binding.yaml`; embed resolved display fields |
| `Scanner` update | `fastapi-apcore`, `nestjs-apcore`, `axum-apcore`, etc. | Deprecate `simplify_ids`; emit `metadata.suggested_alias` instead of modifying `module_id` |
| CLI surface | `apcore-cli-{lang}` | Read `descriptor.metadata["display"].cli.alias` for command name; `display.cli.description` for help text; `display.cli.guidance` for describe output |
| MCP surface | `apcore-mcp-{lang}` | Read `display.mcp.alias` for tool name; `display.mcp.description`; inject `display.mcp.guidance` into tool description context |
| A2A surface | `apcore-a2a-{lang}` | Read `display.a2a.alias` for skill name; `display.a2a.description`; `display.a2a.guidance` for agent card |
| apcore core | `apcore-{lang}` | **No changes required.** Registry, Executor, FunctionModule unchanged. `metadata` field already supports arbitrary keys. |

**`apcore-toolkit` implementation checklist (per language):**

1. Define `ResolvedModule` type (§5.13.9)
2. Implement `DisplayResolver`:
   a. Parse `display` section from binding YAML entries
   b. Build `binding_map: dict[module_id → DisplayConfig]`
   c. For each `ScannedModule`, apply resolve priority chain (§5.13.5)
   d. `alias`, `description`, `tags` always non-null; `documentation`, `guidance` may be null
   e. Validate surface aliases per §5.13.6; MUST error on MCP >64 chars, SHOULD warn on CLI pattern violation
3. Update `RegistryWriter` to accept `ResolvedModule[]` and store `display` in `metadata["display"]`
4. Update `YAMLWriter` to embed resolved display fields in output binding files

**Surface checklist (CLI / MCP / A2A, per language):**

- Read display fields from `descriptor.metadata["display"]` (populated by RegistryWriter)
- Fall back to `descriptor.module_id` / `descriptor.description` if `metadata["display"]` is absent (backward compatibility with modules registered without DisplayResolver)
- **Never call DisplayResolver at surface time** — display resolution is a one-time operation at registration

**Conformance tests** (`../../conformance/fixtures/display_resolve.json`, already created):

| Test ID | Scenario |
|---------|----------|
| 001 | No binding entry → all fields use scanner values |
| 002 | `display.alias` only → all surfaces use it |
| 003 | Surface-specific override takes precedence |
| 004 | Field-level sparsity |
| 005 | `suggested_alias` fallback |
| 006 | Full chain, surface-specific wins |
| 007 | Binding-level description fallback |
| 008 | `display.alias` overrides `suggested_alias` |
| 009 | `display.tags` override |
| 010 | 10 modules, only 2 have binding entries |
| 011 | `guidance` resolve chain |
| 012 | `guidance` is null when not authored |
| 013 | MCP alias >64 chars → MUST error |
| 014 | CLI alias with spaces → SHOULD warn, fallback |

**Per-language implementation locations:**

| Language | DisplayResolver | ResolvedModule type |
|----------|----------------|---------------------|
| Python | `apcore_toolkit/display/resolver.py` | `apcore_toolkit/display/types.py` |
| TypeScript | `src/display/resolver.ts` | `src/display/types.ts` |
| Rust | `src/display/resolver.rs` | `src/display/types.rs` |
| Go (future) | `display/resolver.go` | `display/types.go` |
| Java (future) | `display/DisplayResolver.java` | `display/ResolvedModule.java` |

#### 5.13.11 Migration from `simplify_ids`

The `simplify_ids` parameter on framework scanners is **DEPRECATED** in favor of the display overlay system. Migration path:

| Before (deprecated) | After (preferred) |
|---------------------|-------------------|
| `OpenAPIScanner(simplify_ids=True)` modifies `module_id` | `OpenAPIScanner()` produces canonical ID + `metadata.suggested_alias` |
| `create_cli(simplify_ids=True)` | `create_cli()` + binding.yaml `display.cli.alias` |
| `create_mcp_server(simplify_ids=True)` | `create_mcp_server()` + binding.yaml `display.mcp.alias` |

Implementations **SHOULD** support `simplify_ids` as a convenience parameter during a transition period, mapping it to `metadata.suggested_alias` internally. Implementations **MUST** log a deprecation warning when `simplify_ids=True` is used.

### 5.14 Convention Module Discovery (Zero-Decorator Modules)

#### 5.14.1 Normative Statement

Implementations **MAY** support a convention-based module discovery mechanism that allows plain functions (without decorators or imports) to be automatically registered as apcore modules. This provides the lowest possible barrier for users who want to add custom CLI commands, MCP tools, or A2A skills.

Convention Module Discovery is an **optional** capability. Implementations that do not support it **MUST** still accept modules registered via `@module` decorators, YAML bindings, or class-based definitions.

#### 5.14.2 Motivation

Adding a custom command to an apcore-based CLI today requires learning the `@module` decorator API or writing YAML binding files. For users who just want to add a simple deploy script or utility command, this is unnecessary friction. Convention Module Discovery lets users drop a plain function file into a designated directory and have it auto-discovered as a module — with schema inference from type annotations and description extraction from docstrings.

#### 5.14.3 Commands Directory Convention

Implementations that support Convention Module Discovery **MUST** scan a designated directory (default: `commands/`) for source files containing plain functions.

```
commands/                    ← convention directory
  deploy.py                  ← one file = one or more modules
  backup.py
  monitoring/                ← subdirectories become group prefixes
    health.py
    metrics.py
```

#### 5.14.4 Function Discovery Rules

Implementations **MUST** apply the following rules when scanning a convention source file:

| Rule | Description |
|------|-------------|
| **Public functions only** | Functions starting with `_` are ignored. |
| **Top-level only** | Nested functions and class methods are not discovered. |
| **Type annotations required** | Functions without parameter type annotations **SHOULD** be skipped with a warning. |
| **Docstring → description** | The first line of the function's docstring becomes the module `description`. Functions without docstrings **SHOULD** use `"(no description)"`. |
| **Return type** | If the function has a return type annotation, it is used to generate `output_schema`. |

#### 5.14.5 Module ID Generation

The module ID for a convention-discovered function is generated as:

```
module_id = "{prefix}.{function_name}"
```

Where:
- `prefix` is derived from the file path relative to the commands directory: `commands/deploy.py` → `deploy`, `commands/monitoring/health.py` → `monitoring.health`
- `function_name` is the Python/TypeScript/Rust function name

If the source file contains a module-level `MODULE_PREFIX` constant, it **MUST** override the file-path-derived prefix:
```python
MODULE_PREFIX = "ops"  # overrides file-path prefix
```

If the source file contains a module-level `CLI_GROUP` constant, it is stored in `metadata["display"]["cli"]["group"]` for grouped CLI commands:
```python
CLI_GROUP = "ops"  # sets display.cli.group for all functions in this file
```

#### 5.14.6 Schema Inference

Implementations **MUST** generate `input_schema` from the function's parameter type annotations using the same type-to-JSON-Schema mapping defined in §5.11.5.

```python
# commands/deploy.py
def deploy(env: str, tag: str = "latest", replicas: int = 3) -> dict:
    """Deploy application to target_id environment."""
    ...
```

Inferred `input_schema`:
```json
{
  "type": "object",
  "properties": {
    "env": {"type": "string"},
    "tag": {"type": "string", "default": "latest"},
    "replicas": {"type": "integer", "default": 3}
  },
  "required": ["env"]
}
```

Parameters with default values are **not** included in `required`. The `self` and `ctx` parameters (if present) are always excluded.

#### 5.14.7 Metadata Conventions

Convention source files **MAY** define module-level constants to provide additional metadata:

| Constant | Type | Maps to | Example |
|----------|------|---------|---------|
| `MODULE_PREFIX` | `str` | Module ID prefix (overrides file path) | `MODULE_PREFIX = "ops"` |
| `CLI_GROUP` | `str` | `metadata["display"]["cli"]["group"]` | `CLI_GROUP = "ops"` |
| `TAGS` | `list[str]` | Module `tags` field | `TAGS = ["devops", "deploy"]` |

Function-level constants are not supported; use `@module` decorator for per-function metadata control.

#### 5.14.8 Cross-language Syntax Reference

| Language | File Extension | Function Pattern | Type System |
|----------|---------------|-----------------|-------------|
| Python | `.py` | `def func(param: type) -> type:` | PEP 484 type hints |
| TypeScript | `.ts` | `export function func(param: type): type` | TypeScript types |
| Rust | `.rs` | `pub fn func(param: Type) -> Type` | Rust types + `#[derive(JsonSchema)]` |

#### 5.14.9 Integration with Display Overlay (§5.13)

Convention-discovered modules are subject to the same display overlay system as any other module. A `binding.yaml` file **MAY** reference convention-discovered modules by their generated `module_id` to apply display overrides:

```yaml
bindings:
  - module_id: deploy.deploy
    display:
      alias: deploy
      cli:
        group: ops
        alias: deploy
        description: Deploy app to production
```

This allows users to start with zero-config convention discovery, then progressively add display customizations without changing the function code.

### 5.15 Edge Case Handling

Implementations **MUST** handle module edge cases according to the following table:

#### 5.15.1 execute() Return Value Edges

| Scenario | Behavior | Level |
|------|------|------|
| `execute()` returns `None` | Throw `MODULE_EXECUTE_ERROR` ("Return value cannot be None") | **MUST** |
| `execute()` returns non-Map/dict type | Throw `MODULE_EXECUTE_ERROR` ("Return value must be Map") | **MUST** |
| Return value doesn't match `output_schema` | Throw `SCHEMA_VALIDATION_ERROR` | **MUST** |
| `execute()` throws non-`ModuleError` exception | Wrap as `MODULE_EXECUTE_ERROR` (cause points to original exception) | **MUST** |
| `execute()` returns object with non-serializable objects | **SHOULD** log warning but don't enforce check | **SHOULD** |

#### 5.15.2 Module Dependency Loading Failures

| Scenario | Behavior | Level |
|------|------|------|
| Module in `dependencies.requires` doesn't exist | Throw `DEPENDENCY_NOT_FOUND`, refuse loading | **MUST** |
| Module in `dependencies.optional` doesn't exist | Log INFO, continue loading | **MUST** |
| Module in `dependencies.requires` fails to load | Throw `MODULE_LOAD_ERROR`, refuse loading | **MUST** |
| Module in `dependencies.optional` fails to load | Log WARN, continue loading | **MUST** |
| Module in `dependencies.requires` exists but its registered version does not satisfy the declared `version` constraint | Throw `DEPENDENCY_VERSION_MISMATCH`, refuse loading | **MUST** |
| Module in `dependencies.optional` exists but its registered version does not satisfy the declared `version` constraint | Log WARN, skip the dependency edge, continue loading | **MUST** |
| Reverse dependency (A depends on B, B also depends on A) | Throw `CIRCULAR_DEPENDENCY` | **MUST** |
| Indirect circular dependency (A → B → C → A) | Throw `CIRCULAR_DEPENDENCY` | **MUST** |

#### 5.15.3 Module Lifecycle Edges

| Scenario | Behavior | Level |
|------|------|------|
| `on_load()` hook throws exception | Throw `MODULE_LOAD_ERROR` (cause points to original exception), terminate loading | **MUST** |
| `on_unload()` hook throws exception | Log ERROR, continue unload process | **MUST** |
| Module called before load completion | Wait for load completion or throw `MODULE_NOT_FOUND` | **SHOULD** |
| Module called after unload | Throw `MODULE_NOT_FOUND` | **MUST** |
| Repeated `discover()` of same module | If `metadata.yaml` unchanged, skip (idempotent) | **MUST** |
| Hot reload while module executing | See §12.7.4 Hot Reload Race Conditions | **MUST** |

**Note**:
- Dependency topological sorting uses algorithm A07 (§5.3)
- Circular dependency detection should complete in `discover()` phase, avoiding runtime failures

### 5.16 Pipeline Control Flow Requirements

Implementations **MUST** enforce the following control flow invariants on the execution pipeline (see §5.6 and `docs/features/core-executor.md`).

1. **Fail-fast on step error.** When a pipeline step raises an error, implementations **MUST** stop pipeline execution immediately and propagate the error wrapped in a `PipelineStepError` carrying the failing step name and the original error. Implementations **MUST NOT** continue to the next step unless that step is explicitly configured with `ignore_errors: true`.

2. **O(1) step name resolution.** Implementations **MUST** use a hash map (dictionary) keyed by step name for all step lookups during execution. Implementations **MUST NOT** perform linear scans over a step list to locate a step by name. This requirement **MUST** be enforced at code review time.

3. **Replace semantic for step configuration.** When `configure_step` (or the equivalent declarative `configure:` directive) targets a step name that already exists in the current pipeline strategy, implementations **MUST** replace the existing step definition entirely. Implementations **MUST NOT** create a duplicate step entry or append a second handler under the same name. The replaced step **MUST** retain its original position in the execution order.

4. **`run_until` termination predicate.** Implementations **MUST** support a `run_until` call option that accepts a predicate receiving the current `PipelineState` (step name, accumulated outputs, context) and returning a boolean. When the predicate returns `true` after step N completes, implementations **MUST** skip all remaining steps and return the accumulated result from steps 1 through N. If the predicate never returns `true`, the pipeline runs to completion normally.

5. **Step-level middleware ordering.** Implementations **SHOULD** support middleware scoped to individual pipeline steps. When both global middleware and step-level middleware are registered, global middleware **MUST** execute before step-level middleware in the before-phase, and after step-level middleware in the after-phase.

---

## 6. ACL Specification

### 6.1 ACL Files

```yaml
# acl/global_acl.yaml

$schema: "https://apcore.dev/acl/v1"
version: "1.0.0"

# Global rules (evaluated in order, first-match-wins)
rules:
  # Rule 1: System internal modules unrestricted
  - callers: ["@system"]
    targets: ["*"]
    effect: allow
    description: "System calls are always allowed"

  # Rule 2: API layer can only call orchestration layer
  - callers: ["api.*"]
    targets: ["orchestrator.*"]
    effect: allow
    description: "API layer calls orchestration layer"

  # Rule 3: Orchestration layer can call executor layer
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow
    description: "Orchestration layer calls executor layer"

  # Rule 4: Forbid executor layer calling API layer
  - callers: ["executor.*"]
    targets: ["api.*"]
    effect: deny
    description: "Block reverse calls from executor to API"

  # Rule 5: Everyone can call common modules
  - callers: ["*"]
    targets: ["common.*"]
    effect: allow
    description: "Common modules accessible to all"

  # Rule 6: Conditional access to payment modules
  - callers: ["api.*"]
    targets: ["executor.payment.*"]
    effect: allow
    description: "Payment access restricted to admin/finance"
    conditions:
      identity_types: ["user"]
      roles: ["admin", "finance"]
      max_call_depth: 5

# Default policy
default_effect: deny

# Audit configuration
audit:
  enabled: true
  log_level: info
  include_denied: true
```

**ACL Rule Fields:**

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `callers` | **MUST** | `list[string]` | Caller patterns (OR logic: any match is sufficient) |
| `targets` | **MUST** | `list[string]` | Target patterns (OR logic: any match is sufficient) |
| `effect` | **MUST** | `"allow" \| "deny"` | Access decision |
| `description` | **SHOULD** | `string` | Human-readable rule description |
| `conditions` | **MAY** | `object` | Additional conditions (all must pass, AND logic) |

**Conditions sub-fields:**

| Field | Type | Description |
|-------|------|-------------|
| `identity_types` | `list[string]` | Identity type must be in list |
| `roles` | `list[string]` | At least one role must overlap |
| `max_call_depth` | `integer` | Call chain length must not exceed threshold |
| `$or` | `list[object]` | Compound: passes if **any** sub-condition object passes (each sub-object's keys are AND-ed internally). Sub-objects **MAY** contain further compound operators. |
| `$not` | `object` | Compound: passes if the wrapped condition object **fails**. An empty object **MUST** evaluate to false (fail-closed). |

**Compound operators and async sub-conditions.** Implementations **MUST** evaluate `$or` / `$not` sub-conditions using the same evaluator mode (sync or async) as the enclosing call. An async-only sub-condition under a sync evaluator **MUST** fail closed and **SHOULD** emit a warning. Handlers **SHOULD** therefore be registered for both sync and async paths.

**Special patterns:**

| Pattern | Description |
|---------|-------------|
| `@external` | Matches calls with no caller_id (external entry points) |
| `@system` | Matches calls where identity type is `system` |
| `*` | Wildcard, matches all module IDs |

**Reserved for future use:** `id`, `actions`, `priority` fields are reserved for future specification versions and **SHOULD NOT** be used by implementations.

### 6.2 Rule Matching

Implementations **MUST** perform pattern matching according to the following algorithm:

```
Algorithm: match_pattern(pattern, module_id)

Input:
  pattern   — ACL pattern (e.g., "api.*", "*.validator.*", "executor.email.send_email")
  module_id — Module Canonical ID to match

Output:
  matched — boolean

Steps:
  1. If pattern == "*" → Return true
  2. If pattern doesn't contain "*":
     → Return pattern == module_id (exact match)
  3. Split pattern by "*" into segments
  4. Use greedy matching algorithm:
     a. pos ← 0
     b. For each segment (non-empty):
        - Find segment in module_id[pos:]
        - If not found → Return false
        - pos ← Found position + len(segment)
     c. If pattern doesn't end with "*" → module_id must end with last segment
  5. Return true

Complexity: O(m × n), where m is pattern length, n is module_id length
```

```yaml
rule_matching:
  # Wildcard support
  wildcards:
    - "*"           # Match any
    - "api.*"       # Match all under api
    - "*.validator.*"  # Match validator in any layer

  # Evaluation order: first-match-wins
  order: "Rules are evaluated in definition order; the first matching rule determines the decision"
```

### 6.2.1 Compound Operators in Pattern Arrays

The `callers` and `targets` pattern arrays **MAY** use the compound operators `$or` and `$not` as the **first element** to alter the default OR-of-patterns semantics.

| Form                          | Semantics                                                                                                  |
|-------------------------------|------------------------------------------------------------------------------------------------------------|
| `["$or", p1, p2, ...]`        | **MUST** match the module ID if any of `p1, p2, …` matches. Observably equivalent to a flat list (which is also OR-ed) but documents intent explicitly. |
| `["$not", p]`                 | **MUST** match the module ID if `p` does **not** match.                                                    |
| `["$not"]` (no pattern)       | **MUST** evaluate to false (fail-closed).                                                                  |
| `["$not", p1, p2, ...]`       | Implementation-defined: SDKs **MUST** consult `p1` and **MAY** ignore subsequent patterns. Authors **SHOULD NOT** rely on this form. |

When `$or` or `$not` appear at any position other than index 0 of a pattern array, implementations **MUST** treat them as literal pattern strings (no special semantics). Implementations **MUST NOT** match a literal module ID equal to `"$or"` or `"$not"` under default-deny semantics — these tokens are reserved for compound-operator use.

### 6.3 Rule Evaluation Algorithm

Implementations **MUST** evaluate ACL rules using a **first-match-wins** strategy. Rules are evaluated in definition order (not sorted by priority). The first rule whose patterns match the caller_id and target_id determines the access decision.

**Naming Convention:**
- **Wire Format (JSON)**: All data transfer structures (e.g., Audit Logs, Context sync) **MUST** use `snake_case` (e.g., `caller_id`, `target_id`).
- **SDK Surface**: Implementations **SHOULD** use idiomatic naming (e.g., `callerId` in TypeScript, `caller_id` in Python).

```
Algorithm: evaluate_acl(caller, target, rules, default_effect, context)

Input:
  caller_id         — Caller module ID (null means external call, treated as "@external")
  target_id         — Called module ID
  rules          — Rule list (evaluated in definition order)
  default_effect — Default policy ("allow" | "deny")
  context        — Execution context (optional, used for condition evaluation)

Output:
  decision — { effect: "allow" | "deny", matched_rule: Rule | null }

Steps:
  1. effective_caller_id ← caller_id ?? "@external"
  2. For each rule ∈ rules (in definition order):
     a. caller_matched ← false
        For each pattern ∈ rule.callers:
          If pattern is "@external" and caller_id is null → caller_matched ← true; break
          If pattern is "@system" and context.identity.type == "system" → caller_matched ← true; break
          If match_pattern(pattern, effective_caller) → caller_matched ← true; break
     b. target_matched ← false
        For each pattern ∈ rule.targets:
          If match_pattern(pattern, target) → target_matched ← true; break
     c. If caller_matched and target_matched:
        If rule.conditions is not empty:
          If not evaluate_conditions(rule.conditions, context) → continue
        → Return { effect: rule.effect, matched_rule: rule }
  3. Return { effect: default_effect, matched_rule: null }

Complexity: O(R × P), where R is number of rules, P is average patterns per rule
```

### 6.4 Pattern Specificity Scoring

When further distinguishing rules within same priority is needed, implementations **SHOULD** calculate pattern specificity score:

```
Algorithm: calculate_specificity(pattern)

Input:
  pattern — ACL pattern string

Output:
  score — Specificity score (integer, higher = more specific)

Steps:
  1. If pattern == "*" → Return 0
  2. segments ← Split pattern by "."
  3. score ← 0
  4. For each segment:
     a. If segment == "*" → score += 0
     b. If segment contains "*" (partial wildcard) → score += 1
     c. If segment doesn't contain "*" (exact match) → score += 2
  5. Return score
```

### 6.5 Edge Case Handling

| Scenario | Behavior | Level |
|------|------|------|
| `caller_id` is null | Treat as `@external` | **MUST** |
| `rules` is empty | Use `default_effect` | **MUST** |
| `callers` or `targets` in rule is empty array | Rule never matches | **MUST** |
| Conditions present but no context provided | Rule does not match | **MUST** |
| Module calls itself | Perform ACL check normally | **MUST** |

### 6.6 System Module Permissions

The `system.*` namespace is a **framework-reserved prefix** (see §2.5). All built-in system modules (health, manifest, usage, control) are registered under this prefix. This section defines the permission semantics and defense-in-depth model for system modules.

#### 6.6.1 Registration Restriction

System modules **MUST** only be registered via `register_internal()` (or equivalent privileged API). The standard `register()` method **MUST** reject any `module_id` starting with `system.` due to the reserved word `system` in §2.5.

This guarantees that user-defined modules **cannot** impersonate system modules.

#### 6.6.2 Module Classification by Prefix

Adapters (MCP servers, HTTP gateways, Explorer UIs) **SHOULD** use `module_id` prefix to classify modules:

| Prefix | Classification | Nature |
|--------|---------------|--------|
| `system.health.*` | Observability | Read-only, no side effects |
| `system.usage.*` | Observability | Read-only, no side effects |
| `system.manifest.*` | Introspection | Read-only, no side effects |
| `system.control.*` | Administration | Write operations — may reload, disable, or reconfigure modules |

Adapters **SHOULD NOT** invent their own classification mechanisms (such as reserved tags or environment variables) — the `module_id` prefix is the canonical and unforgeable identifier.

#### 6.6.3 Defense-in-Depth Model

System module access is governed by three independent layers. Each layer operates regardless of the others — and each is **inactive by absence**: a layer that was never configured does not fail closed, it does not run. Nothing in this section is a default-deny. That is deliberate, and stating it is the point of §6.6.3.1.

```
Layer 1: Activation (Config) — two stages, not one
  sys_modules.enabled = false (default)
    → 0 modules registered. Nothing to call, nothing to list.
  sys_modules.enabled = true, sys_modules.events.enabled = false (default)
    → 6 read modules: system.health.*, system.usage.*, system.manifest.*
    → NO system.control.* — the write modules require the EventEmitter that
      carries their audit events (§6.7 cross-cutting requirement 3), so they
      are registered inside the events branch and not otherwise.
  both = true
    → 9 modules: the 6 above plus the 3 system.control.* write modules.

Layer 2: Authorization (ACL)
  When system modules ARE registered, ACL rules control who can call them.
  → Recommended default: deny external callers access to system.*
  → Example:
      rules:
        - callers: ["@external"]
          targets: ["system.*"]
          effect: deny
          description: "Block external access to system modules"

Layer 3: Approval (requires_approval annotation)
  Destructive system operations (e.g., system.control.reload_module)
  SHOULD set requires_approval = true for human-in-the-loop enforcement.
```

The 0 / 6 / 9 distinction matters to anything that reasons about exposure: a registry holding six read-only modules and no ACL is an **information-disclosure** question, not a control-plane one. Treating "system modules are registered" as one state produces a warning that fires on a configuration with no write surface at all.

Adapters and UI layers **SHOULD NOT** introduce additional independent permission switches. The three layers above are sufficient — adding adapter-level switches creates shadow permission systems that can diverge from the actual ACL state.

##### 6.6.3.1 Layers 2 and 3 are inactive by absence

**Layer 2 — a missing `acl/` path attaches nothing.** When `acl.root` resolves to a path that does not exist, implementations **MUST** attach no ACL and **MUST NOT** synthesize an empty `default_effect: deny` ACL in its place. An empty default-deny ACL denies every inter-module call, so synthesizing one would break every project that has no `acl/` directory — which is every project that has not opted in. `default_effect: deny` is the default *within a ruleset that exists*; it is not a default *for the absence of one*. Pinned by `../../conformance/fixtures/acl_root_discovery.json`.

**Layer 3 — a missing `ApprovalHandler` skips the gate.** With no handler configured, Step 5 is skipped and a module declaring `requires_approval` executes anyway, with a warning (§7.4, §7.9.4). Failing closed is opt-in through `ExecutionPolicy(strict = true)` (§7.9), which converts the skip into `ApprovalDeniedError`. Implementations **MUST NOT** change this default.

##### 6.6.3.2 A configured layer is not necessarily an enforced one

Layers 2 and 3 are pipeline **steps** — `acl_check` and `approval_gate` (§12.3). An `ExecutionStrategy` that does not contain the step does not run the layer, **even when the ACL object or the `ApprovalHandler` is attached to the executor**.

This is not a hypothetical about exotic custom strategies. Three of the four presets this specification defines remove one or both:

| Preset | `acl_check` | `approval_gate` |
|---|---|---|
| `standard` | present | present |
| `internal` | **removed** | **removed** |
| `testing` | **removed** | **removed** |
| `minimal` | **removed** | **removed** |

So `executor.set_acl(acl)` followed by selecting the `internal` strategy leaves an ACL attached and never consulted. Implementations **SHOULD** warn at the moment the mismatch is created — when an ACL or handler is attached to an executor whose current strategy has no corresponding built-in step — and **MUST** expose the condition for reading through §6.6.5, so an adapter or a health endpoint can observe it rather than infer it from the presence of the object.

Consequently, for any consumer asking "what is gating this registry?", **`acl != null` is not the answer.** The two questions — *is an object configured* and *is the gate wired into the running pipeline* — are independent, and collapsing them into one boolean reports "protected" in precisely the configuration a conformant implementation is already warning about.

#### 6.6.4 UI Adapter Guidelines

Explorer-style UIs that display module listings **SHOULD**:

1. **Classify by prefix**: Use `module_id.startswith("system.")` to separate system modules from user modules in the UI.
2. **Reflect backend state**: If no `system.*` modules appear in the listing, hide management UI elements. If they appear, show them.
3. **Not duplicate authorization**: The UI should faithfully reflect what the backend exposes. If ACL blocks a call, the Executor returns `ACL_DENIED` — the UI handles this error gracefully, rather than pre-filtering modules with its own logic.

This "backend-driven visibility" approach ensures the UI always matches the actual permission state without maintaining a parallel authorization model.

#### 6.6.5 Governance State Query

> **Added in v1.15.0.** Governance: [apcore#97](https://github.com/aiperceivable/apcore/issues/97).

§6.6.3.2 establishes that *configured* and *enforced* are independent. This section defines the read-only accessor that lets a consumer observe both without guessing, and without each adapter re-deriving it from whatever the executor happens to expose.

Implementations **MUST** expose a public, read-only accessor returning the governance state of an executor.

**Canonical signature (pseudocode):**

```
executor.governance_state()
→ GovernanceState    # a value object of booleans; MUST NOT expose the ACL or handler itself
```

Method naming follows language idiom (`governance_state()` in Python and Rust, `governanceState()` in TypeScript). Field names are **normative and identical across languages** in their language-idiomatic casing — an adapter written against one SDK must read the same facts from the other two.

##### 6.6.5.1 Fields

Seven observations, each a plain fact about the executor's current state, plus one derived flag.

| Field | Meaning |
|---|---|
| `control_modules_registered` | At least one `system.control.*` module is in the registry. This is a fact about the **registry**, not about configuration: the two `sys_modules` flags of §6.6.3 are the usual cause, but internal or manual registration can produce one without them, and the accessor **MUST** report what is registered either way. |
| `read_modules_registered` | At least one read-only `system.*` module (`system.health.*`, `system.usage.*`, `system.manifest.*`) is in the registry. |
| `acl_configured` | An ACL object is attached to this executor. |
| `builtin_acl_gate_wired` | The **running** strategy contains a step the executor recognises as the built-in ACL gate. See §6.6.5.2 — this is a type/capability test, never a name test. |
| `approval_handler_configured` | An `ApprovalHandler` is attached. |
| `builtin_approval_gate_wired` | The running strategy contains a step recognised as the built-in approval gate. |
| `policy_strict` | An `ExecutionPolicy` with `strict = true` (§7.9) is attached, which makes the approval gate fail closed with no handler. |
| `unprotected_control_surface` | Derived; defined below. |

`unprotected_control_surface` **MUST** be computed exactly as:

```
unprotected_control_surface =
      control_modules_registered
   && !(acl_configured && builtin_acl_gate_wired)
   && !(builtin_approval_gate_wired && (approval_handler_configured || policy_strict))
```

Read it as: *no built-in gate that this runtime knows how to recognise is standing in front of `system.control.*`.*

Two things it **does not** claim, and implementations **MUST NOT** document it as claiming:

1. **That protection exists when it is `false`.** A configured, wired ACL may permit every call. The flag reports the **absence of a gate**, never the presence of protection. Implementations **MUST NOT** name it `is_secure`, expose an inverse spelled as safety, or otherwise present it as a security verdict.
2. **That nothing will stop the call when it is `true`.** A deployment may enforce through a custom pipeline step, custom middleware, or an upstream gateway. Those are invisible to this accessor by construction and are not negated by the flag.

The read modules are reported separately for the reason given in §6.6.3: six read-only modules with no ACL is an information-disclosure question, and folding them into the control-surface flag makes it fire on a configuration with no write surface at all.

##### 6.6.5.2 Gate detection MUST be by type, not by name

`StrategyInfo` (design-execution-pipeline §2.6) carries `name`, `step_count`, `step_names` and `description` — names only. A custom step named `acl_check` that never consults an ACL satisfies a name test.

The `*_gate_wired` fields **MUST** therefore be determined by the step's type or an explicit capability marker, matching the test the executor already performs when it wires a gate: attaching an ACL locates the step by *both* its name and its built-in type before injecting, and warns when no such step exists.

The direction of failure is the reason this is a MUST. A name test on a look-alike step produces `builtin_acl_gate_wired = true`, hence `unprotected_control_surface = false` — the accessor reporting a gate that is not there. That is the one direction this flag must never fail in; a false `true` on the derived flag is merely conservative.

This is also why the accessor belongs on `Executor`, where the step objects are reachable, and **MUST NOT** be derived by adapters from `describe_pipeline()` or `list_strategies()` output. Extending `StrategyInfo` with a capability marker is a conforming alternative implementation, but it is a public-API change and is not required here.

##### 6.6.5.3 Constraints

1. **Pure read.** `governance_state()` **MUST NOT** enforce, warn, throw, or mutate any executor state. What to do about an unprotected control surface belongs to the caller — a serve-time adapter may warn or refuse, a test may assert, a health endpoint may report. Putting the reaction inside the accessor makes it unavoidable and untestable.
2. **Booleans only.** The returned value **MUST NOT** contain the ACL object, the `ApprovalHandler`, the `ExecutionPolicy`, or any rule content. An SDK that already exposes those (for example as public struct fields) **MAY** keep doing so; this accessor answers a different question and does not replace them.
3. **No default changes.** Adding this accessor changes no behaviour. In particular the two invariants of §6.6.3.1 stand: a missing `acl/` path still attaches nothing, and a missing `ApprovalHandler` still warns and continues under a non-strict policy.
4. **Live, not cached.** The returned value **MUST** reflect the executor's state at the moment of the call. Swapping the strategy, attaching an ACL, or registering a control module after a previous call **MUST** be visible in the next one.

##### 6.6.5.4 Conformance

Pinned by `../../conformance/fixtures/governance_state.json`. Every field, the derived flag included, **MUST** be identical across the three SDKs for every case, which **MUST** include at minimum:

- no system modules registered;
- read modules only;
- control modules with an ACL attached but no `acl_check` step in the running strategy (the `internal` / `testing` / `minimal` presets of §6.6.3.2);
- control modules with an ACL attached and the built-in step present;
- control modules with an `ApprovalHandler` but no `approval_gate` step;
- control modules with `ExecutionPolicy(strict = true)` and no handler;
- control modules with none of the three;
- **control modules with a custom step named `acl_check` that is not the built-in gate** — `builtin_acl_gate_wired` MUST be `false`.

The last case is the one that decides whether an implementation satisfies §6.6.5.2 or merely appears to.

### 6.7 Canonical System Module Catalogue

This section documents the canonical `system.*` module catalogue. Conformant SDKs at the indicated level **MUST** ship modules with these exact Canonical IDs, equivalent semantics, and equivalent input/output schemas (verified by `../../conformance/fixtures/system_modules_hardening.json`). The full JSON Schema definitions live in each SDK's reference source — see `apcore-python/src/apcore/sys_modules/`, `apcore-typescript/src/sys_modules/`, `apcore-rust/src/sys_modules/`. This catalogue is the contract surface; the SDK source is the schema source of truth.

| Canonical ID | Layer | Read/Write | Required at | Description |
|---|---|---|---|---|
| `system.health.summary`        | Observability  | Read  | Level 1 | Aggregated health overview (counts by status, per-module entries) |
| `system.health.module`         | Observability  | Read  | Level 1 | Detailed health for a single module (latency p50/p99, error count, error rate) |
| `system.manifest.module`       | Introspection  | Read  | Level 1 | Full manifest for one module (input_schema, output_schema, annotations, tags, dependencies, source path) |
| `system.manifest.full`         | Introspection  | Read  | Level 1 | Full manifest for all registered modules |
| `system.usage.summary`         | Observability  | Read  | Level 1 | Aggregated usage statistics (calls, errors, latency) |
| `system.usage.module`          | Observability  | Read  | Level 1 | Per-module usage statistics |
| `system.control.update_config` | Administration | Write | Level 2 | Update a runtime configuration value by dot-path key (audit-logged) |
| `system.control.reload_module` | Administration | Write | Level 2 | Hot-reload a module by `module_id` or `path_filter` glob (mutually exclusive); MAY cascade to dependents |
| `system.control.toggle_feature`| Administration | Write | Level 2 | Enable/disable a registered module via `ToggleState`; persists via `OverridesStore` if configured |

**Cross-cutting requirements:**

1. **Registration MUST use `register_internal()`** (or equivalent privileged API) per §6.6.1 — the public `register()` MUST reject any `module_id` starting with `system.`.
2. **Read modules** (`system.health.*`, `system.usage.*`, `system.manifest.*`) **MUST NOT** mutate framework state. Calling them with any inputs MUST be safe to repeat.
3. **Write modules** (`system.control.*`) **MUST** emit audit events through the framework `EventEmitter` and **SHOULD** declare `annotations.requires_approval = true` to surface the Approval Gate (§7) for destructive operations.
4. **`system.control.reload_module`** input **MUST** accept exactly one of `module_id` (exact match) or `path_filter` (glob); supplying both or neither MUST raise a validation error.
5. **`system.control.update_config`** **MUST** redact sensitive keys (per §10 `obs.redaction.sensitive_keys`) in both `old_value` and `new_value` fields of its output and audit event.
6. **`system.control.toggle_feature`** state **MUST** persist via the configured `OverridesStore` so toggle state survives process restart; without persistence, toggle decisions revert to the registered defaults on reload.

**Conformance note:** SDKs declaring Level 1 conformance (§ ./conformance.md §3) MUST register the 6 read modules. SDKs declaring Level 2 MUST additionally register the 3 control modules. Implementations MAY register additional modules under `system.<vendor>.*` namespaces — these are NOT covered by this canonical catalogue and MUST NOT collide with the names above.

#### 6.7.1 Usage Module Output Contract

> **Added in v1.14.0.** Governance: [apcore#96](https://github.com/aiperceivable/apcore/issues/96).

§6.7 requires `system.usage.summary` and `system.usage.module` to ship with "equivalent input/output schemas" and defers the field contract to each SDK's source. That deferral is why three implementations of it diverged in four ways without any of them becoming non-conformant: the catalogue named the modules, and nothing said what their fields mean. This section states the parts an SDK cannot infer, and `../../schemas/sys-usage-summary.schema.json` and `../../schemas/sys-usage-module.schema.json` are the canonical shape.

Two of the requirements below are **value** semantics that no JSON Schema can assert — a wrong `p99_latency_ms` and a full-history `call_count` are both a `number` in the right place. Those are pinned by fixture (§6.7.1.6), not by schema.

##### 6.7.1.1 `period` is a filter, not an echo

Both modules accept a `period` input, default `"24h"`.

**Grammar.** `period` **MUST** match `^[1-9][0-9]*[hd]$` — a positive integer followed by `h` (hours) or `d` (days). Implementations **MUST** declare this `pattern` in the module's `input_schema` so a malformed value is rejected by input validation (§12.3 Step 7) with `SCHEMA_VALIDATION_ERROR`, uniformly, rather than by an implementation-private parser that raises a language-native error in one SDK and silently accepts in another.

The leading `[1-9]` is normative: `"0h"` is **MUST**-reject. A zero-width window is not a meaningful query, and accepting it produces an all-zero report that reads as "no traffic" rather than as "bad input". Signs (`"+3h"`, `"-5d"`), fractions (`"1.5h"`) and uppercase units (`"24H"`) are likewise rejected.

**Semantics.** The window is `[now − period, now]` in UTC. **Every** statistic in the output **MUST** be computed over that window:

| Module | Fields that MUST honour `period` |
|---|---|
| `system.usage.summary` | `total_calls`, `total_errors`, and every entry of `modules[]` — `call_count`, `error_count`, `avg_latency_ms`, `unique_callers`, `trend` |
| `system.usage.module` | `call_count`, `error_count`, `avg_latency_ms`, `p99_latency_ms`, `trend`, every entry of `callers[]`, and `hourly_distribution` |

An implementation **MUST NOT** echo `period` in its output while computing any statistic over the full retained history. That failure mode is silent by construction — the response names the window it did not apply — and it is the shape all three SDKs must be checked against rather than assumed clear of.

`trend` compares the requested window against the immediately preceding window of equal length (`[now − 2·period, now − period]`), per §6.7.1.5.

##### 6.7.1.2 `hourly_distribution`

Emitted by `system.usage.module` only.

1. **Key format.** `hour` **MUST** be the UTC hourly bucket key `YYYY-MM-DDTHH` — e.g. `2026-03-08T14`. This is the key `UsageCollector` already produces in all three SDKs; a module layer that reformats it (to `2026-03-08T14:00:00Z`, or to anything else) **MUST NOT** do so. One serialization, one place.
2. **Cardinality.** The array **MUST** contain exactly 24 entries, covering the 24 hourly buckets ending at the current hour (`now − 23h .. now`). Gaps **MUST** be zero-filled (`call_count: 0`, `error_count: 0`) rather than omitted, so a consumer can index the array positionally without reconciling missing keys.
3. **Order.** Entries **MUST** be sorted ascending by `hour`.
4. **Relationship to `period`.** The 24-entry window is fixed and is **not** widened or narrowed by `period`; only the counts inside each bucket are filtered by it. A `period` shorter than 24h therefore yields leading zero buckets, and a longer one does not add entries. This is stated because it is the one place where "every statistic honours `period`" (§6.7.1.1) would otherwise read as licence to change the array length.

##### 6.7.1.3 `p99_latency_ms`

`p99_latency_ms` **MUST** be the **nearest-rank** 99th percentile of the latency samples in the window, computed as:

```
Algorithm: p99(latencies)

Input:  latencies — list of latency samples in ms, in any order
Output: p99       — number

Steps:
  1. If latencies is empty → Return 0
  2. sorted ← ascending sort of latencies
  3. N      ← length(sorted)
  4. rank   ← ceil(0.99 × N)          // 1-based rank
  5. index  ← min(rank, N) − 1        // clamp, then convert to 0-based
  6. Return sorted[index]
```

Implementations **MUST NOT** interpolate between adjacent samples, and **MUST NOT** return the element after `index`.

**Worked example, normative.** For `latencies = [1, 2, …, 100]` (N = 100): `rank = ceil(99.0) = 99`, `index = 98`, so `p99 = 99`. An implementation returning `100` has read one element past the rank it computed. For N = 1 the result is the single sample; for N = 0 it is `0`.

The empty-input result is `0` and not `null`: the field is `required` and typed `number` in `sys-usage-module.schema.json`, so a module with no samples in the window still emits a well-formed report.

##### 6.7.1.4 `caller_id` in `callers[]`

A call recorded with no caller identity **MUST** be attributed to the literal string `"unknown"`. Implementations **MUST NOT** omit the entry, use `null`, or substitute `@external` — `@external` is an ACL matching token (§6.2) and reusing it here would make an unattributed call indistinguishable from an externally-attributed one in a usage report.

##### 6.7.1.5 `trend`

`trend` **MUST** be one of `stable`, `rising`, `declining`, `new`, `inactive`, decided by comparing the call count in the requested window (`current`) against the count in the immediately preceding window of equal length (`previous`):

| Condition | `trend` |
|---|---|
| `current == 0` and `previous == 0` | `stable` |
| `current == 0` and `previous > 0` | `inactive` |
| `previous == 0` and `current > 0` | `new` |
| `current / previous > 1.2` | `rising` |
| `current / previous < 0.8` | `declining` |
| otherwise | `stable` |

The thresholds are normative so that the same traffic does not read as `rising` in one SDK and `stable` in another. The order of the rows is normative too — the zero cases are decided before the ratio, which is what keeps the ratio from dividing by zero.

##### 6.7.1.6 `output_schema` and conformance

1. Both modules' `output_schema()` **MUST** declare the full field contract — `type`, `properties` and `required` — matching the canonical schemas. A bare `{"type": "object"}` is **MUST**-reject: it satisfies §6.7's "equivalent output schemas" only in the sense that any two such declarations are equivalent to each other, which is precisely the divergence this section closes.
2. Output from every conformant SDK **MUST** validate against `sys-usage-summary.schema.json` / `sys-usage-module.schema.json`. Both declare `additionalProperties: false`: a field one SDK emits and the others do not is a parity gap, and failing loudly is the intended behaviour.
3. **Schemas cannot assert §6.7.1.1 or §6.7.1.3.** A full-history `call_count` and an off-by-one `p99_latency_ms` are both well-typed values in the right field. Those two, and only those two, are pinned by `../../conformance/fixtures/usage_contract.json` with fixed inputs and expected outputs.

---

## 7. Approval System

### 7.1 Overview

The Approval System provides **runtime enforcement** of the `requires_approval` annotation. While annotations are generally hints for AI/LLM clients, `requires_approval` is unique: when an `ApprovalHandler` is configured, the Executor **blocks execution** of modules marked `requires_approval=true` until explicit approval is granted.

This mechanism is the bridge between annotation-level metadata and runtime governance — making apcore the only framework that **enforces** Human-in-the-Loop approval rather than merely hinting at it.

**Relationship to ACL:**

| Concern | ACL (§6) | Approval System (§7) |
|---------|----------|----------------------|
| Question answered | "Is this caller_id **allowed** to invoke this module?" | "Does this **invocation** need human sign-off?" |
| Mechanism | Pattern-based rule matching | Pluggable handler with external interaction |
| Timing | Step 4 in Executor pipeline | Step 5 in Executor pipeline (after ACL) |
| Interaction | None (deterministic rule evaluation) | May involve user dialog, webhook, or agent confirmation |

A caller_id may pass ACL (they have the role to call `deploy.prod`) but still require approval for each invocation (because the module is destructive).

### 7.2 ApprovalHandler Protocol

Implementations **MUST** define an `ApprovalHandler` protocol (or interface) with the following contract:

```
Interface: ApprovalHandler
  /**
   * Request approval for a module execution.
   * Implementation decides whether to block synchronously or return pending.
   *
   * @param request — ApprovalRequest containing module_id, arguments, context, annotations
   * @return result — ApprovalResult with status: approved|rejected|timeout|pending
   */
  request_approval(request: ApprovalRequest) → ApprovalResult

  /**
   * Check status of a previously pending approval (Phase B).
   * Default implementation SHOULD return rejected.
   *
   * @param approval_id — Identifier from a prior pending result
   * @return result     — ApprovalResult with current status
   */
  check_approval(approval_id: String) → ApprovalResult
```

Both methods **MUST** be asynchronous (async/await) in implementations that support it.

### 7.3 Data Types

#### 7.3.1 ApprovalRequest

```yaml
ApprovalRequest:
  type: object
  required: [module_id, arguments, context, annotations]
  properties:
    module_id:
      type: string
      description: "Target module's canonical ID"
    arguments:
      type: object
      description: "The arguments that will be passed to the module"
    context:
      type: object
      description: "Execution context (trace_id, identity, call_chain) — see §5.7 Context Parameter Specification"
    annotations:
      type: object
      description: "Module's ModuleAnnotations (requires_approval is guaranteed true) — see §4.4 Module Behavior Annotations"
    description:
      type: string
      nullable: true
      description: "Module's human-readable description"
    tags:
      type: array
      items: { type: string }
      description: "Module's tags"
```

#### 7.3.2 ApprovalResult

```yaml
ApprovalResult:
  type: object
  required: [status]
  properties:
    status:
      type: string
      enum: [approved, rejected, timeout, pending]
      description: "Approval decision"
    approved_by:
      type: string
      nullable: true
      description: "Identifier of the approver (human, agent, policy)"
    reason:
      type: string
      nullable: true
      description: "Reason for rejection or additional context"
    approval_id:
      type: string
      nullable: true
      description: "Identifier for async approval tracking (Phase B)"
    metadata:
      type: object
      nullable: true
      description: "Additional metadata from the approval process"
```

### 7.4 Executor Integration (Step 5)

The Approval Gate is Step 5 in the Executor's pipeline, between ACL Enforcement and Middleware Before Chain:

```
Executor Pipeline:
  Step  1: Context Creation
  Step  2: Call Chain Guard
  Step  3: Module Lookup
  Step  4: ACL Enforcement
  Step  5: Approval Gate
  Step  6: Middleware Before Chain
  Step  7: Input Validation
  Step  8: Module Execution
  Step  9: Output Validation
  Step 10: Middleware After Chain
  Step 11: Result Return
```

**Step 11 Contract:**

1. `Executor.call()` **MUST** return the module's output dict unchanged (no framework envelope). Output Validation (Step 9) **MUST** have already passed before Step 11 returns.
2. `Executor.stream()` **MUST** complete the async iterator after the last chunk; the final accumulated dict (recursive deep-merge of all yielded chunks per §5 streaming semantics) is what Step 9 validates.
3. `Executor.validate()` **MUST** return a `PreflightResult` (§12.2) carrying per-check status, a `requires_approval` flag, and the duck-type-compatible `valid` / `errors` properties.
4. Trace metadata (`trace_id`, `caller_id`, `call_chain`, `executor`) **MUST** be carried on the `Context` object, **MUST NOT** be attached to the return value.
5. Side-channel emissions (events, OTel spans, audit log entries) **MAY** continue after Step 11 returns; the return value **MUST NOT** depend on side-channel completion.

**Step 5 Algorithm:**

```
Algorithm: approval_gate(module, arguments, context, approval_handler)

Input:
  module         — Resolved module instance
  arguments      — Call arguments
  context        — Execution context (with identity)
  approval_handler — Configured ApprovalHandler or null

Behavior:
  1. IF approval_handler is null → SKIP (no enforcement)
  2. LET annotations = module.annotations
  3. IF annotations is null OR annotations.requires_approval is false → SKIP
  4. IF arguments contains "_approval_token":
       a. LET token = arguments.pop("_approval_token")
       b. LET result = approval_handler.check_approval(token)
     ELSE:
       a. LET request = ApprovalRequest(module_id, arguments, context, annotations, ...)
       b. LET result = approval_handler.request_approval(request)
  5. SWITCH result.status:
       "approved" → CONTINUE to Step 6
       "rejected" → THROW ApprovalDeniedError(result)
       "timeout"  → THROW ApprovalTimeoutError(result)
       "pending"  → THROW ApprovalPendingError(result)
```

**Key behaviors:**
- When no `ApprovalHandler` is configured, Step 5 is **skipped** for backward compatibility — but per the fail-loud principle (§7.9.4) a module that needs approval **MUST** produce a warning on skip, and an `ExecutionPolicy` with `strict` (§7.9) turns this skip into a fail-closed `ApprovalDeniedError` instead.
- When an `ExecutionPolicy` (§7.9) is attached, the gate consults it first: the policy may force approval on a module that does not declare `requires_approval`, or (with `gate_destructive`) gate a `destructive` module.
- The `_approval_token` mechanism (Phase B) allows clients to retry after external approval without re-triggering the approval flow.
- The `_approval_token` key **MUST** be removed from arguments before passing to subsequent steps.

**Resume semantics.** When a caller retries an `APPROVAL_PENDING` call by injecting `_approval_token` into `arguments`, the executor **MUST** re-enter the pipeline from Step 1. Implementations **MUST NOT** preserve any intermediate `PipelineContext` state across the suspend/resume boundary — the pipeline is stateless across the approval gate, and resumption is a fresh top-to-bottom traversal of the 11 steps with `_approval_token` present in `arguments`. Modules **MUST NOT** assume that side-effects performed in pre-approval steps (for example, logging or tracing in `Middleware.before`) are skipped on resume — they re-execute. Middleware authors who require at-most-once semantics across an approval gate **SHOULD** inspect `_approval_token` in their own logic and short-circuit accordingly. This contract enables external retry/replay layers to drive long-running pause/resume cycles by persisting `approval_id` plus the original inputs, without any mid-pipeline checkpointing in apcore. See [`./design-durability-boundary.md`](./design-durability-boundary.md) §2.2 for downstream integration patterns.

### 7.5 Error Types

Implementations **MUST** define the following error types under `ModuleError`:

```yaml
approval_error_codes:
  APPROVAL_DENIED:
    description: "Approval was explicitly rejected"
    http_status: 403
  APPROVAL_TIMEOUT:
    description: "Approval request timed out without response"
    http_status: 408
  APPROVAL_PENDING:
    description: "Approval is pending — retry with _approval_token after approval"
    http_status: 202
```

Error hierarchy addition:

```
ModuleError
├── ...existing errors...
├── ApprovalError              # Base class for all approval errors
│   ├── ApprovalDeniedError    # APPROVAL_DENIED — Explicitly rejected
│   ├── ApprovalTimeoutError   # APPROVAL_TIMEOUT — No response within timeout
│   └── ApprovalPendingError   # APPROVAL_PENDING — Awaiting external approval
```

Each approval error **MUST** carry a `result` field containing the full `ApprovalResult`.

### 7.6 Built-in Handlers

Implementations **SHOULD** provide these built-in handlers:

| Handler | Behavior | Use Case |
|---------|----------|----------|
| `AlwaysDenyHandler` | Always returns `rejected` | Default safe behavior when no handler configured but enforcement desired |
| `AutoApproveHandler` | Always returns `approved` | Testing and development |
| `CallbackApprovalHandler` | Delegates to a user-provided async callback | Custom approval logic |

### 7.7 Protocol Bridge Handlers

Protocol bridges (such as apcore-mcp, apcore-a2a, apcore-cli) **SHOULD** provide handlers that leverage their protocol's interaction capabilities:

| Bridge | Handler | Mechanism |
|--------|---------|-----------|
| apcore-mcp | `ElicitationApprovalHandler` | Uses MCP Elicitation protocol to show confirmation dialog in MCP clients |
| apcore-a2a | `A2AApprovalHandler` | Uses A2A protocol interaction to request confirmation from calling agent |
| apcore-cli | `CliApprovalHandler` | Uses interactive terminal prompt to request confirmation from user |

These handlers are **not** part of the apcore core specification — they are provided by the respective bridge packages.

### 7.8 Phased Implementation

#### Phase A: Synchronous Approval (Required)

- `request_approval()` blocks until a decision is reached or timeout occurs.
- Suitable for interactive scenarios (MCP client dialogs, agent-to-agent confirmation).
- **All conformant implementations MUST support Phase A.**

#### Phase B: Asynchronous Approval (Optional)

- `request_approval()` may return `status: "pending"` with an `approval_id`.
- Client retries the tool call with `_approval_token` in arguments.
- `check_approval(approval_id)` returns the current status.
- Suitable for long-running approval workflows (Slack, email, dashboard).
- **Phase B is optional but recommended for production deployments.**

### 7.9 Execution Policy (v1.9.0, #76)

> **Status:** Normative as of v1.9.0. Implemented in all three SDKs at 0.26.0 — apcore-python, apcore-typescript (`ExecutionPolicy`, `PolicyRule`) and apcore-rust (`ExecutionPolicy`, `PolicyDecision`, `PolicyRule`, `Executor::set_policy`).

An **Execution Policy** is a declarative, execution-time governance layer that overrides a module's governance annotations (`requires_approval`, `destructive`) **independent of how the module was registered**. It exists because "governed capabilities" is a platform-level promise: an operator must be able to gate already-registered modules without editing their code or re-registering them.

#### 7.9.1 Attach Point and Precedence

1. A policy **MUST** attach at the **Executor** and be consulted by the Approval Gate (Step 5, §7.4).
2. A policy is a set of **pattern rules**. Each rule matches a module ID using the ACL wildcard semantics (Algorithm A08, §6) and carries optional overrides for `requires_approval` and `destructive` (each `null`/absent = "do not override").
3. Rule selection **MUST** use ACL specificity scoring (Algorithm A10, §6): the most specific matching rule wins. On a specificity tie, the more **restrictive** rule wins (a rule that forces `requires_approval = true` outranks one that clears it).
4. A matched rule's non-null override **MUST** take precedence over the module's own declared or scanned annotation. This is deliberate: external governance is the platform's word over the module author's.
5. A policy decision **MUST** be recorded in the audit trail. When a policy changes a module's effective governance, the Executor **MUST** emit `apcore.policy.override` (§9.16.2) when an event emitter is configured.

#### 7.9.2 `destructive` → approval resolution

The approval gate keys on the **effective** `requires_approval`. Because HTTP-method inference and scanners can set `destructive = true` without setting `requires_approval` (a governance footgun — e.g. an inferred `DELETE`), a policy **MAY** enable `gate_destructive`:

- When `gate_destructive` is true, a module whose **effective** `destructive` is true **MUST** be treated as needing approval even when `requires_approval` is false.
- `gate_destructive` is **opt-in**; with no policy, `destructive` and `requires_approval` remain orthogonal (no behavior change). Independently, an implementation **SHOULD** warn when a `destructive` module is exposed with no approval/ACL covering it (see §7.9.4).

#### 7.9.3 Effective annotations to the handler

When the gate invokes the handler because a policy forced approval, the `ApprovalRequest.annotations` handed to the handler **MUST** carry the **effective** governance values (`requires_approval = true`, `destructive` = the effective value), not the module's raw declaration. This preserves the §7.3 contract that "`requires_approval` is guaranteed true" in a handler-visible `ApprovalRequest`.

#### 7.9.4 Fail loud, not silent (security principle)

A misconfigured or unreachable governance control **MUST NOT** silently allow. Concretely:

1. When a module needs approval but **no** `ApprovalHandler` is configured, the implementation **MUST** either (default) keep the §7.4 skip behavior **and** emit a warning (at least once per module), or — when the policy sets `strict` — **fail closed** by raising `ApprovalDeniedError` (`APPROVAL_DENIED`).
2. A `destructive` module that no approval gate covers **SHOULD** produce a warning (at least once per module).
3. A strict-fail-closed rejection **MUST** emit `apcore.approval.decision` (status `rejected`) like any other adjudication.
4. Strict parsing: an implementation that loads a policy from a document (YAML/JSON) **MUST** reject unknown keys and a missing rule `pattern`, so a typo cannot silently disable a control.

#### 7.9.5 Preflight

`Executor.validate()` (§12.2) **MUST** report the policy-effective `requires_approval` — i.e. the same verdict the gate will enforce, including a `gate_destructive`-driven or rule-forced approval. The `apcore.acl.denied` and governance decision events **MUST NOT** be emitted during a dry-run `validate()`.

### 7.10 Conformance

| Level | Requirement |
|-------|-------------|
| **Level 1 (Basic)** | `ApprovalHandler` protocol defined; Executor skips gate when handler is null |
| **Level 2 (Standard)** | Step 5 implemented in `call()`, `call_async()`, and `stream()` paths; `AlwaysDenyHandler` and `AutoApproveHandler` provided |
| **Level 3 (Full)** | Phase B support (`check_approval`, `_approval_token`); `CallbackApprovalHandler` provided; approval audit events emitted |
| **Level 4 (Governance)** | Execution Policy (§7.9): external override + specificity precedence, `gate_destructive`, `strict` fail-closed, effective-annotations contract, and the `apcore.approval.decision` / `apcore.policy.override` / `apcore.acl.denied` events |

---

## 8. Error Handling Specification

### 8.1 Unified Error Format

All errors must follow unified format:

```yaml
error_format:
  type: object
  required: [code, message]
  properties:
    code:
      type: string
      description: "Error code (format: CATEGORY_SPECIFIC_ERROR)"
    message:
      type: string
      description: "Human-readable error message"
    details:
      type: object
      description: "Error details (optional)"
    cause:
      type: object
      description: "Original error (optional, for error chains)"
    trace_id:
      type: string
      description: "Trace ID"
    timestamp:
      type: string
      format: datetime
    retryable:
      type: boolean
      nullable: true
      description: "Whether the error is retryable (see §8.6 for defaults per error code)"
    ai_guidance:
      type: string
      nullable: true
      description: "Machine-readable hint for AI agents on how to handle this error"
    user_fixable:
      type: boolean
      nullable: true
      description: "Whether the end-user can fix the root cause without developer intervention"
    suggestion:
      type: string
      nullable: true
      description: "Actionable suggestion for resolving the error"
```

#### 8.1.1 AI Error Guidance Fields

The four optional fields (`retryable`, `ai_guidance`, `user_fixable`, `suggestion`) enable AI agents to programmatically understand and respond to errors without parsing human-readable messages.

These fields are the foundation of apcore's **Self-Healing** mechanism, which serves two higher-level goals:

- **Self-Repair**: The Agent autonomously corrects errors and retries within a single interaction.
- **Self-Evolution**: The system continuously adapts through health monitoring, event-driven feedback loops, and runtime reconfiguration (see §9.11 Hot-Reload, §10 Observability).

**Field semantics:**

| Field | Type | Purpose |
|-------|------|---------|
| `retryable` | `boolean \| null` | Whether retrying the same call may succeed. Each error code has a default value (see §8.6). Callers may override. `null` means "depends on context". |
| `ai_guidance` | `string \| null` | Machine-readable guidance for AI agents, e.g. `"validate input schema before retry"`, `"check module registry for available alternatives"`. |
| `user_fixable` | `boolean \| null` | Whether the end-user (non-developer) can resolve the issue, e.g. fixing a typo in input vs. a server misconfiguration. |
| `suggestion` | `string \| null` | Human-readable actionable suggestion, e.g. `"Check that the table name contains only lowercase letters and underscores"`. |

**Serialization rules:**

- Implementations **MUST** use sparse serialization: fields with `null` values **SHOULD** be omitted from the serialized output.
- `retryable` defaults to the value specified in §8.6 for each error code. Callers may explicitly override this default.
- `ai_guidance`, `user_fixable`, and `suggestion` default to `null` (omitted).

**Example with AI guidance fields:**

```json
{
  "code": "SCHEMA_VALIDATION_ERROR",
  "message": "Invalid table name format",
  "details": { "field": "table", "value": "User-Info" },
  "retryable": false,
  "user_fixable": true,
  "suggestion": "Table names must use only lowercase letters and underscores. Change 'User-Info' to 'user_info'.",
  "ai_guidance": "validate input against schema before retry; this error will recur with the same input"
}
```

### 8.2 Framework Error Codes

!!! info "`http_status` is descriptive, not a required SDK surface"

    Each entry below carries an `http_status`. It is a **suggested** mapping for
    an integration that places an executor behind HTTP — a gateway, an
    `apcore-a2a-*` server — and implementations are **NOT** required to expose it.
    The MUST in §7.5 and here is *define the following error types*; the status is
    metadata about each type, in the same way `description` is.

    Read as a requirement it would be 47 unimplemented MUSTs: no SDK exposes the
    mapping, and none is expected to. Stated because a conformance fixture had
    declared `expected.http_status` that no driver could ever assert, and the
    ambiguity is what put it there (apcore#94). `499` is an nginx extension and
    `508` is WebDAV — deliberately practical choices for a gateway author, and
    another reason this is guidance rather than a contract three SDKs must
    reproduce.

```yaml
error_codes:
  # Configuration-related (CONFIG_*)
  CONFIG_NOT_FOUND:
    description: "Configuration file not found"
    http_status: 500
  CONFIG_INVALID:
    description: "Invalid configuration file"
    http_status: 500

  # Module-related (MODULE_*)
  MODULE_NOT_FOUND:
    description: "Module doesn't exist"
    http_status: 404
  MODULE_LOAD_ERROR:
    description: "Module load failed"
    http_status: 500
  MODULE_EXECUTE_ERROR:
    description: "Module execution error"
    http_status: 500
  MODULE_TIMEOUT:
    description: "Module execution timeout"
    http_status: 504
  MODULE_DISABLED:
    description: "Module is disabled via system.control.toggle_feature"
    http_status: 403
  INVALID_MODULE_ID:
    description: "module_id is empty or fails the Module ID Format Constraint (§2.1)"
    http_status: 400

  # Execution-related (EXECUTION_*)
  EXECUTION_CANCELLED:
    description: "Module execution cancelled via CancelToken"
    http_status: 499

  # Reload-related (RELOAD_*)
  RELOAD_FAILED:
    description: "Module hot-reload failed"
    http_status: 500

  # Schema-related (SCHEMA_*)
  SCHEMA_NOT_FOUND:
    description: "Schema doesn't exist"
    http_status: 404
  SCHEMA_VALIDATION_ERROR:
    description: "Schema validation failed"
    http_status: 400
  SCHEMA_PARSE_ERROR:
    description: "Schema parse error"
    http_status: 500
  SCHEMA_CIRCULAR_REF:
    description: "Schema circular reference detected — a $ref → $ref chain that never reaches a schema body (§4.15)"
    http_status: 500
  SCHEMA_MAX_DEPTH_EXCEEDED:
    description: "$ref resolution exceeded schema.max_ref_depth. Distinct from SCHEMA_CIRCULAR_REF: the chain is well-formed but too deep (§4.11, §4.15)"
    http_status: 500
  SCHEMA_UNION_NO_MATCH:
    description: "Value matched no branch of a oneOf/anyOf union"
    http_status: 400
  SCHEMA_UNION_AMBIGUOUS:
    description: "Value matched more than one branch of a oneOf union, which MUST be exclusive"
    http_status: 400

  # Permission-related (ACL_*)
  ACL_DENIED:
    description: "Permission denied"
    http_status: 403
  ACL_RULE_ERROR:
    description: "ACL rule error"
    http_status: 500

  # Function module-related (FUNC_*)
  FUNC_MISSING_TYPE_HINT:
    description: "Function parameter missing type annotation"
    http_status: 500
  FUNC_MISSING_RETURN_TYPE:
    description: "Function missing return type annotation"
    http_status: 500

  # Binding-related (BINDING_*)
  BINDING_INVALID_TARGET:
    description: "Binding target_id format invalid"
    http_status: 500
  BINDING_MODULE_NOT_FOUND:
    description: "Binding target_id module path can't be imported"
    http_status: 500
  BINDING_CALLABLE_NOT_FOUND:
    description: "Binding target_id callable not found"
    http_status: 500
  BINDING_NOT_CALLABLE:
    description: "Binding target_id not callable"
    http_status: 500
  BINDING_SCHEMA_INFERENCE_FAILED:
    description: "Auto-schema inference failed: callable lacks usable type hints"
    http_status: 500
    # Deprecated alias (renamed in 0.19.0): BINDING_SCHEMA_MISSING — old serialized payloads remain decodable.
  BINDING_FILE_INVALID:
    description: "Binding file parse error"
    http_status: 500

  # Middleware-related (MIDDLEWARE_*)
  MIDDLEWARE_CHAIN_ERROR:
    description: "Middleware chain execution failed"
    http_status: 500

  # Version-related (VERSION_*)
  VERSION_INCOMPATIBLE:
    description: "SDK/config version incompatible"
    http_status: 500

  # Error code registry (ERROR_CODE_*)
  ERROR_CODE_COLLISION:
    description: "Custom error code collides with framework or other module code"
    http_status: 500

  # Dependency-related (CIRCULAR_*, DEPENDENCY_*)
  CIRCULAR_DEPENDENCY:
    description: "Module dependency cycle detected"
    http_status: 500
  DEPENDENCY_NOT_FOUND:
    description: "Dependent module doesn't exist"
    http_status: 500
  DEPENDENCY_VERSION_MISMATCH:
    description: "Dependent module exists but its registered version does not satisfy the declared version constraint"
    http_status: 500

  # General errors (GENERAL_*)
  GENERAL_INVALID_INPUT:
    description: "Invalid input"
    http_status: 400
  GENERAL_INTERNAL_ERROR:
    description: "Internal error"
    http_status: 500
  GENERAL_NOT_IMPLEMENTED:
    description: "Feature not implemented"
    http_status: 501

  # Call chain-related (CALL_*)
  CALL_DEPTH_EXCEEDED:
    description: "Call chain depth exceeded limit"
    http_status: 508
  CIRCULAR_CALL:
    description: "Circular call detected"
    http_status: 508
  CALL_FREQUENCY_EXCEEDED:
    description: "Same module call frequency exceeded limit"
    http_status: 508

  # Approval-related (APPROVAL_*)
  APPROVAL_DENIED:
    description: "Approval was explicitly rejected"
    http_status: 403
  APPROVAL_TIMEOUT:
    description: "Approval request timed out without response"
    http_status: 408
  APPROVAL_PENDING:
    description: "Approval is pending, retry with _approval_token"
    http_status: 202
```

### 8.3 Error Propagation

Implementations **MUST** propagate errors according to the following algorithm:

```
Algorithm: propagate_error(error, module_id, context)

Input:
  error     — Original exception/error object
  module_id — Module ID where error occurred
  context   — Current execution context

Output:
  module_error — Standardized ModuleError object

Steps:
  1. If error is already ModuleError type:
     a. Keep original error.code
     b. Append current module_id to error.chain
     c. Return error
  2. Construct module_error:
     a. code ← Map based on error type:
        - SchemaValidationError → "SCHEMA_VALIDATION_ERROR"
        - ACLDeniedError → "ACL_DENIED"
        - TimeoutError → "MODULE_TIMEOUT"
        - Other → "MODULE_EXECUTE_ERROR"
     b. message ← Human-readable message from error
     c. details ← Extract structured details from error
     d. cause ← error (keep original error)
     e. trace_id ← context.trace_id
     f. module_id ← module_id
     g. call_chain ← Copy of context.call_chain
     h. timestamp ← Current UTC time (ISO 8601)
  3. Return module_error
```

```yaml
error_propagation:
  # Module errors
  module_error:
    - "Module-thrown errors **MUST** be wrapped as ModuleError"
    - "Original error **MUST** be saved in cause field"
    - "Error code prefixed with MODULE_"

  # Error context
  context:
    - "Error **MUST** contain trace_id"
    - "Error **SHOULD** contain occurrence location (module_id)"
    - "**MUST** support error chain tracing"
```

### 8.4 Custom Error Codes

Modules can define their own error codes:

```yaml
custom_error_codes:
  # Naming convention
  naming:
    pattern: "{MODULE_PREFIX}_{ERROR_NAME}"
    module_prefix: "Last part of module ID in uppercase"
    examples:
      - module_id: "executor.validator.db_params"
        prefix: "DB_PARAMS"
        error_code: "DB_PARAMS_INVALID_TABLE"
        error_code: "DB_PARAMS_SQL_INJECTION"

  # Declare in Schema
  declaration:
    location: "error_schema.codes"
    example:
      error_schema:
        codes:
          DB_PARAMS_INVALID_TABLE:
            description: "Invalid table name"
            http_status: 400
          DB_PARAMS_SQL_INJECTION:
            description: "SQL injection detected"
            http_status: 400

  # Error code registration
  registration:
    - "Framework startup **MUST** collect all module error codes"
    - "**MUST** detect error code conflicts"
    - "**SHOULD** generate error code documentation"

  # Framework error code priority
  priority:
    - "Framework error codes (MODULE_/SCHEMA_/ACL_/GENERAL_) **MUST** be reserved, modules **MUST NOT** use them"
    - "Module custom error codes **MUST NOT** conflict with framework error codes"

  # Collision detection algorithm
  collision_detection: |
    Algorithm: detect_error_code_collisions(framework_codes, module_codes_map)

    Steps:
      1. all_codes ← Copy of framework_codes
      2. For each (module_id, codes) ∈ module_codes_map:
         For each code ∈ codes:
           a. If code ∈ framework_codes → Throw error: Module can't use framework reserved codes
           b. If code ∈ all_codes → Throw error: Error code already registered by other module
           c. all_codes ← all_codes ∪ {code}
      3. Return all_codes (complete error code registry)
```

### 8.5 Error Response Format

```yaml
# Standard error response
error_response:
  code: "DB_PARAMS_INVALID_TABLE"
  message: "Invalid table name format"
  details:
    field: "table"
    value: "User-Info"
    expected: "Only lowercase letters and underscores allowed"
  trace_id: "4bf92f3577b34da6a3ce929d0e0e4736"
  timestamp: "2026-02-05T10:30:00Z"
  cause: null  # Or nested error object
  retryable: false
  user_fixable: true
  suggestion: "Table names must use only lowercase letters and underscores. Change 'User-Info' to 'user_info'."
  # ai_guidance: omitted (null) — sparse serialization
```

### 8.6 Retry Semantics

Implementations **MUST NOT** default retry failed module invocations. Retry behavior **MUST** be explicitly controlled by caller_id or middleware.

**Retryability Classification:**

| Error Code | Retryable | Description |
|--------|--------|------|
| `MODULE_TIMEOUT` | **Yes** | Timeout may be temporary |
| `GENERAL_INTERNAL_ERROR` | **Yes** | Internal error may be transient |
| `APPROVAL_TIMEOUT` | **Yes** | Approval handler may respond on retry |
| `MODULE_EXECUTE_ERROR` | **Depends** | Depends on module's `annotations.idempotent` |
| `CONFIG_NOT_FOUND` | **No** | Configuration file missing, needs deployment fix |
| `CONFIG_INVALID` | **No** | Configuration content invalid, needs manual fix |
| `ACL_RULE_ERROR` | **No** | ACL rule definition error, needs config fix |
| `ACL_DENIED` | **No** | Permission insufficiency won't change with retry |
| `APPROVAL_DENIED` | **No** | Explicit denial, retry won't change decision |
| `APPROVAL_PENDING` | **No** | Async approval in progress, use polling instead |
| `MODULE_NOT_FOUND` | **No** | Module non-existence won't change with retry |
| `MODULE_DISABLED` | **No** | Module explicitly disabled, needs re-enabling |
| `MODULE_LOAD_ERROR` | **No** | Load errors typically need code fixes |
| `EXECUTION_CANCELLED` | **Yes** | Cancellation may be temporary, retry with new CancelToken |
| `RELOAD_FAILED` | **Yes** | Reload may succeed after transient issue resolves |
| `SCHEMA_VALIDATION_ERROR` | **No** | Input error won't change with retry |
| `SCHEMA_NOT_FOUND` | **No** | Schema reference missing, needs config fix |
| `SCHEMA_PARSE_ERROR` | **No** | Schema syntax error, needs manual fix |
| `SCHEMA_CIRCULAR_REF` | **No** | Circular reference in schema, needs manual fix |
| `SCHEMA_MAX_DEPTH_EXCEEDED` | **No** | Reference chain too deep, needs schema or config fix |
| `SCHEMA_UNION_NO_MATCH` | **No** | Input error won't change with retry |
| `SCHEMA_UNION_AMBIGUOUS` | **No** | Input error won't change with retry |
| `CALL_DEPTH_EXCEEDED` | **No** | Call chain structure issue, retry won't change |
| `CIRCULAR_CALL` | **No** | Call chain structure issue, retry won't change |
| `CALL_FREQUENCY_EXCEEDED` | **No** | Call chain structure issue, retry won't change |
| `GENERAL_INVALID_INPUT` | **No** | Invalid input, caller_id must fix before retry |
| `FUNC_MISSING_TYPE_HINT` | **No** | Code-level issue, needs developer fix |
| `FUNC_MISSING_RETURN_TYPE` | **No** | Code-level issue, needs developer fix |
| `BINDING_INVALID_TARGET` | **No** | Binding format error, needs config fix |
| `BINDING_MODULE_NOT_FOUND` | **No** | Binding target_id module missing, needs config fix |
| `BINDING_CALLABLE_NOT_FOUND` | **No** | Binding target_id callable missing, needs code fix |
| `BINDING_NOT_CALLABLE` | **No** | Binding target_id not callable, needs code fix |
| `BINDING_SCHEMA_MISSING` | **No** | Schema missing for binding, needs code fix |
| `BINDING_FILE_INVALID` | **No** | Binding file parse error, needs config fix |
| `CIRCULAR_DEPENDENCY` | **No** | Module dependency cycle, needs architecture fix |
| `MIDDLEWARE_CHAIN_ERROR` | **No** | Middleware failed, needs code fix |
| `VERSION_INCOMPATIBLE` | **No** | Version mismatch, needs upgrade or config fix |
| `ERROR_CODE_COLLISION` | **No** | Error code conflict, needs code fix |

Implementations **SHOULD** use this table as the default `retryable` value for each error subclass. Callers may override the default on a per-instance basis.

> **Note:** `GENERAL_NOT_IMPLEMENTED` and `DEPENDENCY_NOT_FOUND` are included in the hierarchy above. Both are non-retryable by default.

Retry middleware (if implemented) **SHOULD**:
- Only retry errors marked as retryable
- Only auto-retry modules with `annotations.idempotent == true`
- Use exponential backoff strategy
- Set max retry count limit (**SHOULD** not exceed 5 times)

### 8.7 Error Hierarchy

All framework errors **MUST** extend from a single `ModuleError` base class using a flat hierarchy. Implementations use a flat hierarchy under `ModuleError` for simplicity.

```
ModuleError (base error for all framework errors)
├── ConfigError                    # CONFIG_INVALID — Invalid configuration file
├── ConfigNotFoundError            # CONFIG_NOT_FOUND — Configuration file not found
├── ModuleNotFoundError            # MODULE_NOT_FOUND — Module doesn't exist
├── ModuleLoadError                # MODULE_LOAD_ERROR — Module load failed
├── ModuleExecuteError             # MODULE_EXECUTE_ERROR — Module execution error
├── ModuleTimeoutError             # MODULE_TIMEOUT — Module execution timeout
├── ModuleDisabledError            # MODULE_DISABLED — Module is disabled
├── ExecutionCancelledError        # EXECUTION_CANCELLED — Execution cancelled via CancelToken
├── ReloadFailedError              # RELOAD_FAILED — Module hot-reload failed
├── SchemaNotFoundError            # SCHEMA_NOT_FOUND — Schema doesn't exist
├── SchemaValidationError          # SCHEMA_VALIDATION_ERROR — Schema validation failed
├── SchemaParseError               # SCHEMA_PARSE_ERROR — Schema parse error
├── SchemaCircularRefError         # SCHEMA_CIRCULAR_REF — Schema circular reference
├── SchemaMaxDepthExceededError    # SCHEMA_MAX_DEPTH_EXCEEDED — $ref chain exceeded max_ref_depth
├── ACLDeniedError                 # ACL_DENIED — Permission denied
├── ACLRuleError                   # ACL_RULE_ERROR — ACL rule error
├── FuncMissingTypeHintError       # FUNC_MISSING_TYPE_HINT — Function parameter missing type annotation
├── FuncMissingReturnTypeError     # FUNC_MISSING_RETURN_TYPE — Function missing return type annotation
├── BindingInvalidTargetError      # BINDING_INVALID_TARGET — target_id format invalid
├── BindingModuleNotFoundError     # BINDING_MODULE_NOT_FOUND — Module path can't be imported
├── BindingCallableNotFoundError   # BINDING_CALLABLE_NOT_FOUND — Can't find target_id callable
├── BindingNotCallableError        # BINDING_NOT_CALLABLE — Target not callable
├── BindingSchemaMissingError      # BINDING_SCHEMA_MISSING — Schema missing
├── BindingFileInvalidError        # BINDING_FILE_INVALID — Binding file parse error
├── CircularDependencyError        # CIRCULAR_DEPENDENCY — Circular dependency
├── DependencyNotFoundError        # DEPENDENCY_NOT_FOUND — Dependent module doesn't exist
├── CallDepthExceededError         # CALL_DEPTH_EXCEEDED — Call depth exceeded limit
├── CircularCallError              # CIRCULAR_CALL — Circular call
├── CallFrequencyExceededError     # CALL_FREQUENCY_EXCEEDED — Call frequency exceeded limit
├── MiddlewareChainError           # MIDDLEWARE_CHAIN_ERROR — Middleware chain execution failed
├── ApprovalError                  # Base class for approval errors (§7)
│   ├── ApprovalDeniedError        # APPROVAL_DENIED — Approval explicitly rejected
│   ├── ApprovalTimeoutError       # APPROVAL_TIMEOUT — Approval timed out
│   └── ApprovalPendingError       # APPROVAL_PENDING — Approval pending (Phase B)
├── InvalidInputError              # GENERAL_INVALID_INPUT — Invalid input
├── InternalError                  # GENERAL_INTERNAL_ERROR — Internal error
├── NotImplementedError            # GENERAL_NOT_IMPLEMENTED — Feature not implemented
├── VersionIncompatibleError       # VERSION_INCOMPATIBLE — SDK/config version incompatible
└── ErrorCodeCollisionError        # ERROR_CODE_COLLISION — Error code collision detected
```

Each error class carries a `code` attribute set to the corresponding error code string (e.g., `MODULE_NOT_FOUND`). Implementations **MUST** ensure all framework-thrown errors are instances of `ModuleError`. Module custom errors **SHOULD** also extend `ModuleError` directly.

### 8.8 Error Formatter Registry

apcore-mcp and apcore-a2a each independently implement an `ErrorMapper` that translates `ModuleError` into their protocol-specific format (MCP camelCase JSON, A2A JSON-RPC codes). The **Error Formatter Registry** provides a shared registration point so this translation contract is visible to the framework, without requiring apcore to know protocol details.

> **Scope:** apcore provides the interface and fallback only. Protocol-specific formatters are owned by each adapter package. Adoption is **SHOULD**-level for ecosystem adapters.

#### 8.8.1 `ErrorFormatter` Protocol

```
protocol ErrorFormatter:
    format(error: ModuleError, context: Context) → dict
    # Returns a protocol-specific error representation.
    # MUST NOT raise — return best-effort dict on internal failure.
```

#### 8.8.2 Registration API

`ErrorFormatterRegistry` is a class exported from the `apcore` package:

```python
from apcore import ErrorFormatterRegistry
```

```
ErrorFormatterRegistry.register(
    adapter_name: string,        # MUST — unique adapter id (e.g., "mcp", "a2a", "cli")
    formatter:    ErrorFormatter  # MUST — implements ErrorFormatter protocol
)
```

**Registration rules:**
1. `adapter_name` **MUST** be unique. Registering the same name twice **MUST** raise `ERROR_FORMATTER_DUPLICATE`.
2. Registration **SHOULD** happen at adapter initialization, before any module calls are processed.
3. The registry is global and shared across all Executor instances.

#### 8.8.3 Lookup Algorithm

```
Algorithm: format_error(error, adapter_name, context)

Steps:
  1. formatter ← ErrorFormatterRegistry.get(adapter_name)
  2. If formatter is nil:
       → Serialize via error.to_dict() (§8.5 standard format)
  3. Return formatter.format(error, context)
```

The framework guarantees a result — callers **MUST NOT** handle exceptions from `format_error`.

#### 8.8.4 Ecosystem Adoption

Ecosystem adapters **SHOULD** register their formatter at initialization:

| Adapter | `adapter_name` | Responsibility |
|---------|---------------|----------------|
| apcore-mcp | `mcp` | snake_case → camelCase, ACL masking, AI guidance mapping |
| apcore-a2a | `a2a` | JSON-RPC code mapping (`-32601`/`-32602`/`-32603`), truncation |
| apcore-cli | `cli` | Terminal-friendly formatting, exit code derivation |

apcore itself does not ship these formatters. Each adapter package owns its implementation.

#### 8.8.5 New Error Code

| Error Code | Trigger | New? |
|------------|---------|------|
| `ERROR_FORMATTER_DUPLICATE` | `register()` called twice for the same `adapter_name` | New |

---

## 9. Configuration Specification

### 9.1 Framework Configuration

apcore.yaml is the core configuration file of the framework. Implementations **MUST** validate configuration files according to the following JSON Schema.

**What is required, and why so little is.** A key is required **only when it has no canonical default** — that is, only when omitting it leaves a value the framework cannot supply. By that rule exactly two keys are required: `version` and `project.name`. Every other key in this section either carries a default in `schemas/defaults.schema.json` (`extensions.*`, `schema.*`, `acl.*`, `executor.*`, `sys_modules.*`, `observability.*`, `stream.*`) or is optional outright, so requiring it would reject a configuration the framework can resolve perfectly well.

The `MUST` markers in the example below therefore mean "this key is normative and its default is fixed", **not** "the file is invalid without it". Only `version` and `project.name` make a file invalid by their absence — `schemas/apcore-config.schema.json` declares exactly those two in its `required` array.

Requiredness is evaluated against the **declared** document, before defaults are merged. An implementation that merges its default table into the parsed document and then checks for required fields can never fail the check — the merge has already supplied every key — which is how a required-field list becomes dead code that looks like validation.

**What "declared" means.** The declared document is everything supplied by *someone* — the configuration file, environment-variable overrides (§9.2), runtime `set()`, and `mount()` — and excludes exactly one thing: the framework's own default table. Implementations **MUST** expose this view (`Config.get_declared()` / `getDeclared()` / `Config.declared`) and **MUST** use it for the required-field check.

Environment overrides count as declaration. `APCORE_PROJECT_NAME=my-app` against a file that omits `project.name` **MUST** satisfy the requirement: the operator has supplied the value, and a container deployment that configures entirely through the environment is a first-class case, not a degraded one. Only the default table is excluded, because a default is the framework answering its own question.

**apcore.yaml Complete JSON Schema Definition:**

```yaml
# apcore.yaml — Complete configuration structure and constraints

$schema: "https://apcore.dev/config/v1"
version: "1.0.0"                    # REQUIRED — no default exists

# Project information
project:
  name: "my-ai-project"             # REQUIRED — no default exists (pattern: ^[a-z][a-z0-9_-]*$)
  version: "0.1.0"                   # SHOULD, project version (semver)

# Extension configuration
extensions:
  root: "./extensions"               # MUST (default: "./extensions")
  auto_discover: true                # SHOULD, auto-discovery (default: true)
  lazy_load: true                    # MAY, lazy loading (default: true)
  follow_symlinks: false             # MUST NOT default true (default: false)
  max_depth: 8                       # SHOULD, max scan depth (default: 8, max: 16)
  ignore_patterns:                   # MAY, additional ignore patterns
    - "*.test.*"
    - "*.spec.*"

# Schema configuration — these three keys are the whole namespace (§4.9);
# `defaults.schema.json` declares it additionalProperties: false. Strictness and
# type coercion are NOT configurable: they are properties of the contract, not
# of the host (§4.9, TYPE_MAPPING §17.3).
schema:
  root: "./schemas"                  # MUST (default: "./schemas")
  strategy: "yaml_first"             # SHOULD, loading strategy (yaml_first|native_first|yaml_only)
  max_ref_depth: 32                  # MAY, $ref max recursion depth (default: 32)

# ACL configuration
acl:
  root: "./acl"                      # MUST (default: "./acl")
  default_effect: "deny"             # MUST (default: deny) — deny|allow
  audit:
    enabled: true                    # SHOULD, audit logging (default: true)
    log_level: "info"                # MAY, audit log level
    include_denied: true             # SHOULD, log denied calls

# Logging configuration
logging:
  level: "info"                      # SHOULD (trace|debug|info|warn|error|fatal)
  format: "json"                     # SHOULD (json|text, default: json)

# Observability configuration
observability:
  tracing:
    enabled: true                    # SHOULD (default: false — opt in)
    sampling_rate: 1.0               # MAY (0.0-1.0, default: 1.0)
    exporter: "stdout"               # MAY (stdout|otlp|jaeger)
  metrics:
    enabled: true                    # MAY (default: false — opt in)
    exporter: "stdout"               # MAY (stdout|prometheus|otlp)

# Middleware configuration
middleware:
  disabled: []                       # MAY, list of disabled built-in middleware

# Binding configuration
bindings:
  dir: "./bindings"                  # SHOULD, binding file directory (default: "./bindings")
  files: []                          # MAY, specified binding file list
  pattern: "*.binding.yaml"          # SHOULD, file matching pattern (default: "*.binding.yaml")

# ID Map configuration
id_map:
  auto_detect: true                  # SHOULD (default: true)
  overrides: {}                      # MAY, manual ID mapping overrides
```

#### 9.1.1 Default Values Summary

Implementations **MUST** follow these default value conventions:

| Configuration Item | Default Value | Valid Range | Description |
|--------|--------|---------|------|
| `extensions.root` | `"./extensions"` | Valid directory path | Extension root directory |
| `extensions.max_depth` | `8` | `1..16` | Scan depth limit |
| `schema.root` | `"./schemas"` | Valid directory path | Schema root directory |
| `schema.max_ref_depth` | `32` | `1..100` | `$ref` resolution depth limit |
| `acl.root` | `"./acl"` | Valid directory path | ACL file root directory |
| `acl.default_effect` | `"deny"` | `allow`/`deny` | Default behavior when no rule matches |
| `executor.default_timeout` | `30000` (ms) | `0..600000` | Per-module execution timeout (0 means no limit) |
| `executor.global_timeout` | `60000` (ms) | `0..600000` | Global execution timeout across entire call chain (0 means no limit) |
| `executor.max_call_depth` | `32` | `1..1000` | Call chain max depth |
| `executor.max_module_repeat` | `3` | `1..100` | Max occurrences of same module in call chain |
| `observability.tracing.enabled` | `false` | `true`/`false` | Distributed tracing switch |
| `observability.tracing.sampling_rate` | `1.0` | `0.0..1.0` | Trace sampling rate |
| `observability.tracing.exporter` | `"stdout"` | `stdout`/`otlp`/`jaeger` | Tracing exporter |
| `observability.metrics.enabled` | `false` | `true`/`false` | Metrics collection switch |
| `observability.metrics.exporter` | `"stdout"` | `stdout`/`prometheus`/`otlp` | Metrics exporter |
| `bindings.dir` | `"./bindings"` | Valid directory path | Binding file directory |
| `bindings.pattern` | `"*.binding.yaml"` | glob pattern | Binding file matching pattern |
| `id_map.auto_detect` | `true` | `true`/`false` | Auto ID mapping detection |

**Note**:
- Configuration values exceeding ranges **MUST** be rejected in `validate_config()` (algorithm A12)
- Implementations use a **dual-timeout model**: `default_timeout` applies to each individual module execution, while `global_timeout` applies to the entire call chain from root invocation. If either timeout is exceeded, a `MODULE_TIMEOUT` error is raised.
- `timeout = 0` means disable that timeout, implementations **SHOULD** log WARN
- `max_call_depth` and `max_module_repeat` used for call chain safety checks (algorithm A20)

### 9.2 Environment Variable Override

Implementations **MUST** support overriding configuration file values through environment variables.

**Override Rules:**

| Priority | Source | Example |
|--------|------|------|
| 1 (Highest) | Environment variable | `APCORE_EXTENSIONS_ROOT=./ext` |
| 2 | Configuration file | `extensions.root: "./extensions"` |
| 3 (Lowest) | Default value | `"./extensions"` |

**Environment Variable Naming Convention:**

```
APCORE_{SECTION}_{KEY}

Rules:
  1. Prefix APCORE_ (uppercase)
  2. Nested levels separated by _
  3. All letters uppercase
  4. Hyphens converted to underscores

Examples:
  extensions.root        → APCORE_EXTENSIONS_ROOT
  schema.max_ref_depth   → APCORE_SCHEMA_MAX_REF_DEPTH
  acl.default_effect     → APCORE_ACL_DEFAULT_EFFECT
  logging.level          → APCORE_LOGGING_LEVEL
  observability.tracing.enabled → APCORE_OBSERVABILITY_TRACING_ENABLED
```

### 9.3 Configuration Validation Algorithm

Implementations **MUST** validate configuration at startup:

```
Algorithm: validate_config(config)

Input:
  config — Merged configuration object (env + file + defaults)

Output:
  validated_config — Validated configuration, or throw CONFIG_INVALID error

Steps:
  1. For each required field — `version` and `project.name`, the only two keys
     with no canonical default (§9.1):
     If missing from the DECLARED document (before the default table is merged)
       → Throw CONFIG_INVALID with the missing field path
     A key that carries a default in defaults.schema.json is NEVER required;
     checking it after merging defaults is a no-op and MUST NOT be relied on.
  2. Type validation:
     For each field, validate value type conforms to Schema definition
  3. Constraint validation:
     - extensions.root MUST be valid directory path
     - schema.root MUST be valid directory path
     - acl.default_effect MUST be "allow" or "deny"
     - observability.tracing.sampling_rate MUST be in [0.0, 1.0] range
     - extensions.max_depth MUST be in [1, 16] range
  4. Semantic validation:
     - If extensions.auto_discover == true and extensions.root doesn't exist → Warning
     - If schema.strategy == "yaml_only" and schema.root doesn't exist → Error
  5. Return validated_config
```

### 9.4 Config Bus Architecture

> **Added in v1.6.0-draft**

The apcore configuration system serves as a **Config Bus** — shared infrastructure that any package in the apcore ecosystem (or external packages) can register with, without being forced to adopt it.

#### 9.4.1 Design Principles

| Principle | Description |
|-----------|-------------|
| **Bus, not center** | apcore.Config does not own or mandate configuration; it provides registration, loading, validation, and access infrastructure. Packages that never call `register_namespace` are unaffected. |
| **Zero-cost adoption** | Existing `apcore.yaml` files continue to work without modification. Namespace mode activates only when the file contains an `apcore:` top-level key. |
| **Gradual integration** | Projects choose their integration depth: (1) apcore-only, (2) apcore + ecosystem packages, (3) apcore + third-party mount, (4) full unified file. Each level is independently valid. |
| **Cross-language consistency** | All SDK implementations (Python, TypeScript, Rust, Go, Java) **MUST** implement the same namespace registration, loading, and validation semantics defined in this section. |
| **Strict and flexible coexistence** | Strict mode enforces that every namespace is registered; flexible mode (default) passes through unknown namespaces with a warning. Both modes coexist within a single deployment. |

#### 9.4.2 Terminology

| Term | Definition |
|------|------------|
| **Namespace** | A top-level key in the unified configuration file (e.g., `apcore`, `apflow`, `apcore-mcp`). Each namespace is owned by exactly one package. |
| **Schema** | A JSON Schema (Draft 2020-12) document that describes the structure, types, defaults, and constraints for a namespace's configuration. |
| **Config Bus** | The `Config` class acting as shared infrastructure: it stores registered namespaces, loads configuration files, applies environment overrides per namespace, validates registered namespaces, and provides unified access. |
| **Mount** | The act of attaching an external configuration source (file or dict) to a namespace in the Config Bus, without requiring a unified configuration file. |
| **Legacy mode** | Backward-compatible behavior where the entire YAML file is treated as the `apcore` namespace. Activated when no `apcore:` top-level key is detected. |
| **Namespace mode** | The file is partitioned by top-level keys, each representing a registered (or unregistered) namespace. Activated when an `apcore:` top-level key is present. |

### 9.5 Namespace Registration

#### 9.5.1 Registration API

All SDK implementations **MUST** expose a namespace registration method on the `Config` class (or its language-idiomatic equivalent).

**Canonical signature (pseudocode):**

```
Config.register_namespace(
    name:        string,                            # MUST — namespace identifier
    schema:      JSONSchema | path | nil,           # MAY  — validation schema
    env_prefix:  string | nil,                      # MAY  — env var prefix (nil = auto-derive from name)
    defaults:    map | nil,                          # MAY  — default values for this namespace
    env_style:   "nested" | "flat" | "auto" | nil,  # MAY  — env var key conversion strategy
    max_depth:   int | nil,                          # MAY  — max nesting depth for env conversion
    env_map:     map<string, string> | nil,          # MAY  — bare env var → config key mapping
)
```

**Parameters:**

| Parameter | Required | Description |
|-----------|----------|-------------|
| `name` | MUST | Namespace identifier. Pattern: `^[a-z][a-z0-9]*(-[a-z0-9]+)*$` (lowercase, hyphens allowed). Examples: `apcore`, `apflow`, `apcore-mcp`, `my-billing`. |
| `schema` | MAY | JSON Schema document (inline object or file path). When provided, the namespace section is validated against this schema during `Config.validate()`. When `nil`, the namespace is registered for isolation and env override only — no structural validation is performed. |
| `env_prefix` | MAY | Uppercase prefix for environment variable overrides (e.g., `APFLOW`). When `nil`, auto-derived from `name` via `name.upper().replace("-", "_")` (e.g., `name="apcore-mcp"` → `env_prefix="APCORE_MCP"`). When an explicit string, used as-is. Must match pattern: `^[A-Z][A-Z0-9]*(_[A-Z0-9]+)*$`. |
| `defaults` | MAY | Default configuration values for this namespace. Merged before file data (lowest priority). |
| `env_style` | MAY | Controls how environment variable suffixes are converted to config keys. `"auto"` (default): matches the suffix against the `defaults` tree structure to determine the correct interpretation — flat keys match flat, nested paths match nested. When `defaults` is `nil`, falls back to `"nested"` behavior. `"nested"`: single `_` → `.` (section separator), double `__` → literal `_` — suitable for purely hierarchical config structures. `"flat"`: no conversion, suffix is lowercased as-is — suitable for purely flat snake_case config keys where `_` is part of the key name, not a hierarchy separator. When `nil`, defaults to `"auto"`. |
| `max_depth` | MAY | Maximum nesting depth for environment variable key conversion. Applies to `"nested"` and `"auto"` styles; ignored for `"flat"`. After reaching `max_depth` levels, remaining `_` characters are preserved as literal underscores instead of being converted to `.` separators. Default: `5`. Example: `A_B_C_D_E_F_G` with `max_depth=5` → `a.b.c.d.e_f_g` (5 segments). |
| `env_map` | MAY | Explicit mapping of bare (unprefixed) environment variable names to config keys within this namespace. Each key is an exact env var name (e.g., `"REDIS_URL"`), each value is the target_id config key (e.g., `"cache_url"`). Only explicitly listed env vars are captured. Same priority as `env_prefix` overrides. An env var name **MUST NOT** appear in more than one `env_map` (global or namespace) — duplicates raise `CONFIG_ENV_MAP_CONFLICT`. When `nil`, no bare env var mapping is applied. |

**Registration rules:**

1. `register_namespace` **MUST** be callable before `Config.load()`. Implementations **may** also allow registration after load (see §9.5.3).
2. Registering the same namespace name twice **MUST** raise `CONFIG_NAMESPACE_DUPLICATE`.
3. The namespace `apcore` is implicitly registered by the framework itself. Attempting to register `apcore` externally **MUST** raise `CONFIG_NAMESPACE_RESERVED`.
4. The reserved namespace `_config` **MUST NOT** be registerable. It is used for Config Bus meta-configuration (see §9.6.3). Attempting to register it **MUST** raise `CONFIG_NAMESPACE_RESERVED`.
5. Namespace registration is permanent for the process lifetime. There is no `unregister_namespace` API in this version of the specification. This simplifies thread safety and avoids invalidation cascades across Config instances.

#### 9.5.2 Cross-Language Registration Examples

**Python:**

```python
from apcore import Config

# Nested style (default) — hierarchical config with _ → . conversion
Config.register_namespace(
    "apflow",
    schema="schemas/apflow.schema.json",
    env_prefix="APFLOW",
    defaults={"api": {"timeout": 30.0}},
)

# Flat style — flat snake_case config, no _ → . conversion
Config.register_namespace(
    "reach",
    env_prefix="REACHFORGE",
    env_style="flat",
    defaults={"devto_api_key": "", "llm_model": "gemini-pro"},
)

# Auto style — mixed flat keys + nested sections, resolved via defaults
Config.register_namespace(
    "myapp",
    env_prefix="MYAPP",
    env_style="auto",
    defaults={"devto_api_key": "", "publish": {"delay": 5, "retry": 3}},
)
```

**TypeScript:**

```typescript
import { Config } from 'apcore-js';

// Nested style (default)
Config.registerNamespace({
  name: 'apflow',
  schema: 'schemas/apflow.schema.json',
  envPrefix: 'APFLOW',
  defaults: { api: { timeout: 30.0 } },
});

// Flat style
Config.registerNamespace({
  name: 'reach',
  envPrefix: 'REACHFORGE',
  envStyle: 'flat',
  defaults: { devto_api_key: '', llm_model: 'gemini-pro' },
});

// Auto style
Config.registerNamespace({
  name: 'myapp',
  envPrefix: 'MYAPP',
  envStyle: 'auto',
  defaults: { devto_api_key: '', publish: { delay: 5, retry: 3 } },
});
```

**Rust:**

```rust
use apcore::Config;

// Nested style (default)
Config::register_namespace(NamespaceRegistration {
    name: "apflow",
    schema: Some("schemas/apflow.schema.json".into()),
    env_prefix: Some("APFLOW"),
    defaults: Some(serde_json::json!({"api": {"timeout": 30.0}})),
    env_style: EnvStyle::Nested,
    max_depth: 5,
})?;

// Flat style
Config::register_namespace(NamespaceRegistration {
    name: "reach",
    env_prefix: Some("REACHFORGE"),
    env_style: EnvStyle::Flat,
    defaults: Some(serde_json::json!({"devto_api_key": "", "llm_model": "gemini-pro"})),
    schema: None,
    max_depth: 5,
})?;

// Auto style
Config::register_namespace(NamespaceRegistration {
    name: "myapp",
    env_prefix: Some("MYAPP"),
    env_style: EnvStyle::Auto,
    defaults: Some(serde_json::json!({"devto_api_key": "", "publish": {"delay": 5, "retry": 3}})),
    schema: None,
    max_depth: 5,
})?;
```

**Java:**

```java
import dev.apcore.Config;

Config.registerNamespace(NamespaceRegistration.builder()
    .name("apflow")
    .schema("schemas/apflow.schema.json")
    .envPrefix("APFLOW")
    .defaults(Map.of("api", Map.of("timeout", 30.0)))
    .build());
```

#### 9.5.3 Registration Lifecycle

**Namespace registration is global (class-level), not per-instance.** The registry of namespaces is shared across all `Config` instances within a process. This matches the real-world pattern: a package registers its namespace once at import time, and any `Config` instance can then load, validate, and serve that namespace's data.

```
┌──────────────────────────────────────────────────────────────────────────┐
│                      Config Bus Lifecycle                                │
│                                                                          │
│  Phase 1: Registration (global)   Phase 2: Loading       Phase 3: Usage │
│  ──────────────────────────────   ───────────────────    ─────────────── │
│                                                                          │
│  Config.register_namespace(...)   Config.load(path)      config.get(...) │
│  Config.register_namespace(...)   ├─ read global registry config.set(...)│
│  Config.register_namespace(...)   ├─ detect mode         config.mount()  │
│  ...                              ├─ parse YAML          config.reload() │
│                                   ├─ merge defaults      config.bind()   │
│                                   ├─ apply env overrides                 │
│                                   ├─ validate (A12-NS)                   │
│                                   └─ return Config instance              │
└──────────────────────────────────────────────────────────────────────────┘
```

**Config instances are independent.** Each `Config.load()` call returns a new instance with its own data tree and lock (consistent with the existing implementation in §9.1). Multiple Config instances may coexist (e.g., in tests), but they all share the same global namespace registry.

**Late registration** (calling `register_namespace` after one or more `Config.load()` calls) is permitted with these constraints:

1. The namespace is added to the global registry immediately.
2. **Already-loaded Config instances are not retroactively modified.** The new namespace takes effect on the next `Config.load()` or `config.reload()` call. This avoids surprising mutations to instances that callers may already be using.
3. If the caller_id needs the new namespace to apply immediately to an existing instance, they **MUST** call `config.reload()` explicitly. The reload will pick up the newly registered namespace, apply its defaults and env overrides, and validate.
4. Late registration **MUST NOT** invalidate previously loaded data in other namespaces.

### 9.6 Unified Configuration File

#### 9.6.1 Mode Detection Algorithm

When `Config.load(path)` is called, implementations **MUST** detect the file mode:

```
Algorithm: detect_config_mode(parsed_yaml)

Input:
  parsed_yaml — Top-level mapping from the YAML file

Output:
  mode — "legacy" or "namespace"

Steps:
  1. If parsed_yaml contains a top-level key "apcore":
       → Return "namespace"
  2. Else:
       → Return "legacy"
```

> **Note — Detection rationale:** This algorithm relies on the fact that `apcore` is not a valid top-level key in the legacy §9.1 schema (the legacy schema uses `version`, `extensions`, `schema`, `acl`, `project`, etc. — never `apcore` as a wrapper). A file that contains `apcore:` as a top-level key is unambiguously a namespace-mode file.
>
> **Consequence:** A namespace-mode file that omits the `apcore:` section (e.g., contains only `apflow:` and `apcore-mcp:`) will be misdetected as legacy mode and fail validation. This is by design — the `apcore:` namespace section is **required** in namespace mode because it contains framework-critical fields (`version`, `extensions.root`, etc.). Users who want namespace mode without apcore core configuration should use `apcore:` with minimal required fields, or use the mount mechanism (§9.7) instead.

**Legacy mode** — the entire file is the `apcore` namespace (backward compatible with §9.1):

```yaml
# apcore.yaml — legacy mode (no "apcore:" key)
version: "0.14.0"
extensions:
  root: ./extensions
executor:
  default_timeout: 5000
```

**Namespace mode** — each top-level key is a namespace:

```yaml
# project.yaml — namespace mode ("apcore:" key present)
apcore:
  version: "0.14.0"
  extensions:
    root: ./extensions

apflow:
  api:
    server_url: http://localhost:8000

apcore-mcp:
  transport: streamable-http
  port: 8000
```

#### 9.6.2 Merge Priority (Namespace Mode)

For each registered namespace, the merge priority (highest wins) is:

| Priority | Source | Description |
|----------|--------|-------------|
| 1 (Highest) | Environment variables | `{ENV_PREFIX}_{SECTION}_{KEY}` |
| 2 | Configuration file | Namespace section from YAML |
| 3 | Mount data | Data supplied via `config.mount()` |
| 4 (Lowest) | Registered defaults | Defaults from `register_namespace()` |

For the `apcore` namespace, legacy merge rules (§9.2) apply unchanged, using the `APCORE_` prefix.

#### 9.6.3 Config Bus Meta-Configuration

The reserved `_config` namespace controls Config Bus behavior itself. It **MUST NOT** be registerable by external packages.

```yaml
_config:
  strict: false          # SHOULD, default: false
  allow_unknown: true    # SHOULD, default: true (only relevant when strict: false)
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `strict` | boolean | `false` | When `true`: (a) every top-level key (except `_config`) **MUST** correspond to a registered namespace — unknown namespaces cause a `ConfigError`; and (b) every key inside a framework section of the `apcore` namespace **MUST** be declared by `schemas/apcore-config.schema.json`, which is `additionalProperties: false` for all of them. See the `reject_unknown_framework_keys` sub-algorithm in §9.10. Clause (b) applies in legacy mode too, where the whole file *is* the `apcore` namespace. |
| `allow_unknown` | boolean | `true` | When `strict` is `false` and `allow_unknown` is `true`, unknown namespace data is stored and accessible via `get()` but not validated. When `allow_unknown` is `false`, unknown namespaces are silently ignored (not stored). |

**Behavior matrix:**

| `strict` | `allow_unknown` | Unknown namespace in YAML | Result |
|-----------|-----------------|---------------------------|--------|
| `true`    | *(ignored)*     | `billing: {db: ...}`      | `ConfigError`: namespace `billing` not registered |
| `false`   | `true`          | `billing: {db: ...}`      | Stored, accessible, WARN logged, not validated |
| `false`   | `false`         | `billing: {db: ...}`      | Silently ignored, not stored |

#### 9.6.4 File Format Examples

**Scenario 1 — Pure apcore (legacy, zero migration):**

```yaml
# apcore.yaml — unchanged from pre-9.4 era
version: "0.14.0"
extensions:
  root: ./extensions
schema:
  root: ./schemas
acl:
  root: ./acl
  default_effect: deny
project:
  name: my-project
```

**Scenario 2 — apcore + ecosystem packages:**

```yaml
# project.yaml
apcore:
  version: "0.14.0"
  extensions:
    root: ./extensions
  schema:
    root: ./schemas
  acl:
    root: ./acl
    default_effect: deny
  project:
    name: my-project

apflow:
  api:
    server_url: http://localhost:8000
    timeout: 30.0
  governance:
    default_policy: auto-downgrade
  durability:
    max_attempts: 3
    backoff_strategy: exponential

apcore-mcp:
  transport: streamable-http
  port: 8000
  auth:
    enabled: true

apcore-a2a:
  name: "My Agent"
  url: http://localhost:9000
  skills:
    - name: data-analysis
      module: executor.analyze

apcore-cli:
  theme: minimal
  output_format: json
```

**Scenario 3 — Third-party project with mount (separate files):**

```yaml
# apcore.yaml — only apcore's own config
apcore:
  version: "0.14.0"
  extensions:
    root: ./extensions
  project:
    name: saas-platform
```

```python
# Third-party project mounts its own config files
config = Config.load("apcore.yaml")
config.mount("billing", from_file="config/billing.yaml")
config.mount("notifications", from_file="config/notifications.yaml")
```

**Scenario 4 — Strict mode for new projects:**

```yaml
# project.yaml
_config:
  strict: true

apcore:
  version: "0.14.0"
  extensions:
    root: ./extensions
  project:
    name: greenfield-app

apflow:
  api:
    server_url: http://localhost:8000

apcore-mcp:
  transport: streamable-http
```

**Scenario 5 — Framework integration (Django, FastAPI):**

```yaml
# project.yaml — framework integration registers its own namespace automatically
apcore:
  version: "0.14.0"
  extensions:
    root: ./extensions
  project:
    name: django-app

django-apcore:
  auto_register_modules: true
  url_prefix: /api/apcore
  middleware:
    enabled: true

fastapi-apcore:
  auto_register_modules: true
  route_prefix: /apcore
  openapi:
    include: true
```

### 9.7 Mount Mechanism

The mount mechanism allows attaching external configuration sources to the Config Bus without requiring a unified configuration file. This is the primary integration path for third-party projects with existing configuration systems.

#### 9.7.1 Mount API

**Canonical signature (pseudocode):**

```
config.mount(
    namespace:  string,           # MUST — target_id namespace
    from_file:  path | nil,       # MAY  — load from file
    from_dict:  map  | nil,       # MAY  — load from in-memory dict
)
```

Exactly one of `from_file` or `from_dict` **MUST** be provided.

**Mount rules:**

1. The target_id namespace **may** or **may not** be previously registered via `register_namespace`.
   - If registered: mount data is merged (file/dict < env overrides), then validated against the registered schema.
   - If not registered: mount data is stored as-is, accessible via `get()`, but not validated. A WARN **SHOULD** be logged.
2. Mounting to a namespace that already has data (from the unified file or a prior mount) **MUST** deep-merge the mount data into the existing data. Mount data has lower priority than file data (see §9.6.2). This means the unified file is the authoritative source when both exist. If the caller_id intends the mounted file to be the authoritative source for a namespace, the namespace section **SHOULD NOT** appear in the unified file.
3. Mounting to the `apcore` namespace is permitted but **SHOULD** log a WARN (it overrides framework configuration).
4. Mounting to `_config` **MUST** raise a `CONFIG_MOUNT_ERROR`.

#### 9.7.2 Cross-Language Mount Examples

**Python:**

```python
config = Config.load("apcore.yaml")

# Mount from file
config.mount("billing", from_file="config/billing.yaml")

# Mount from dict (e.g., loaded by a third-party library)
config.mount("notifications", from_dict={"provider": "ses", "region": "us-east-1"})
```

**TypeScript:**

```typescript
const config = await Config.load('apcore.yaml');

config.mount('billing', { fromFile: 'config/billing.yaml' });
config.mount('notifications', { fromDict: { provider: 'ses', region: 'us-east-1' } });
```

**Rust:**

```rust
let mut config = Config::load("apcore.yaml")?;

config.mount("billing", MountSource::File("config/billing.yaml".into()))?;
config.mount("notifications", MountSource::Dict(serde_json::json!({
    "provider": "ses", "region": "us-east-1"
})))?;
```

**Go:**

```go
cfg, _ := config.Load("apcore.yaml")

cfg.Mount("billing", config.FromFile("config/billing.yaml"))
cfg.Mount("notifications", config.FromDict(map[string]any{
    "provider": "ses", "region": "us-east-1",
}))
```

**Java:**

```java
Config config = Config.load("apcore.yaml");

config.mount("billing", MountSource.fromFile("config/billing.yaml"));
config.mount("notifications", MountSource.fromDict(Map.of(
    "provider", "ses", "region", "us-east-1"
)));
```

#### 9.7.3 Mount vs Unified File Decision Guide

| Criterion | Unified File | Mount |
|-----------|--------------|-------|
| New project, full apcore adoption | Recommended | — |
| Existing project adding apcore | — | Recommended |
| Config managed by external tool (Ansible, Terraform) | — | Recommended |
| Single-file deployment simplicity | Recommended | — |
| Team owns all config schemas | Recommended | — |
| Third-party config with unknown schema | — | Recommended |

Both approaches produce the same namespace tree at runtime. The choice is purely organizational.

### 9.8 Environment Variable Override (Namespace Mode)

#### 9.8.1 Per-Namespace Env Prefix

In namespace mode, each registered namespace with an `env_prefix` has its own environment variable scope.

**Naming convention:**

The env variable convention depends on the `env_style` setting of the namespace registration (see §9.5.1). The default style is `"nested"`.

**Nested style** (`env_style = "nested"`, default) — follows the same rules defined in §9.2:

```
{ENV_PREFIX}_{SECTION}_{KEY}

Rules:
  1. Prefix is the registered env_prefix (uppercase), followed by _
  2. Single _ → . (section separator), up to max_depth levels
  3. Double __ → literal _ (within key names)
  4. All letters uppercase
  5. After max_depth segments, remaining _ are preserved as literal _

Examples (namespace "apflow", env_prefix "APFLOW", env_style "nested"):
  APFLOW_API_SERVER__URL=http://...    → apflow.api.server_url
  APFLOW_API_TIMEOUT=60                → apflow.api.timeout
  APFLOW_GOVERNANCE_DEFAULT__POLICY=x  → apflow.governance.default_policy

Examples (namespace "apcore-mcp", env_prefix "APCORE_MCP", env_style "nested"):
  APCORE_MCP_TRANSPORT=stdio          → apcore-mcp.transport
  APCORE_MCP_PORT=9000                → apcore-mcp.port

Examples (namespace "apcore", env_prefix "APCORE" — unchanged from §9.2):
  APCORE_EXECUTOR_DEFAULT__TIMEOUT=5000 → apcore.executor.default_timeout
```

**Flat style** (`env_style = "flat"`) — the suffix after the prefix is lowercased without any separator conversion. Underscores in the suffix are preserved as literal underscores in the config key. This is designed for namespaces whose config keys are flat snake_case identifiers (e.g., `devto_api_key`, `llm_model`) rather than hierarchical paths.

```
{ENV_PREFIX}_{KEY}

Rules:
  1. Prefix is the registered env_prefix (uppercase), followed by _
  2. Suffix is lowercased as-is (no _ → . conversion, no __ escaping needed)
  3. All letters uppercase in the env var

Examples (namespace "reach", env_prefix "REACHFORGE", env_style "flat"):
  REACHFORGE_DEVTO_API_KEY=abc123     → reach.devto_api_key
  REACHFORGE_LLM_MODEL=gemini-pro     → reach.llm_model
  REACHFORGE_PUBLISH_DELAY=5          → reach.publish_delay

Examples (namespace "myapp", env_prefix "MYAPP", env_style "flat"):
  MYAPP_DATABASE_URL=postgres://...    → myapp.database_url
  MYAPP_MAX_RETRIES=3                  → myapp.max_retries
```

**Auto style** (`env_style = "auto"`) — resolves each env var suffix by matching against the registered `defaults` tree structure. This handles namespaces that mix flat snake_case keys with nested sub-sections, without requiring the user to escape underscores.

```
Algorithm: auto_resolve(suffix, defaults_tree, depth, max_depth)

  1. Try full suffix (lowercased) as a flat key in the current tree level.
     If found → return suffix (flat match).

  2. If depth >= max_depth → return suffix as flat key (depth limit).

  3. For each underscore position in suffix (left to right):
     Split into (prefix, remainder).
     If prefix exists in tree AND is a dict/map:
       Recurse: sub_key ← auto_resolve(remainder, tree[prefix], depth+1, max_depth)
       If sub_key is not nil → return prefix + "." + sub_key

  4. No match found → fall back to nested conversion (with max_depth).

Examples (namespace "reach", env_prefix "REACHFORGE", env_style "auto",
          defaults {"devto_api_key": "", "publish": {"delay": 5, "retry": 3}}):

  REACHFORGE_DEVTO_API_KEY=abc  → "devto_api_key" in defaults? Yes → reach.devto_api_key
  REACHFORGE_PUBLISH_DELAY=5    → "publish_delay" in defaults? No
                                → split "publish" + "delay"
                                → "publish" is dict? Yes → "delay" in it? Yes
                                → reach.publish.delay
  REACHFORGE_PUBLISH_RETRY=3    → same logic → reach.publish.retry
  REACHFORGE_NEW_UNKNOWN=x      → no match in defaults → nested fallback → reach.new.unknown
```

> **When to use each style:** Use `"nested"` (default) when your config is purely hierarchical (e.g., `api.server.url`, `executor.default_timeout`). Use `"flat"` when your config is a flat set of snake_case keys without any nesting (e.g., `devto_api_key`, `llm_model`). Use `"auto"` when your config mixes flat snake_case keys with nested sub-sections — this is the recommended style for most real-world applications. `"auto"` requires `defaults` to be provided for accurate resolution; keys not found in `defaults` fall back to `"nested"` behavior.

**max_depth** (default: 5) — limits the nesting depth for `"nested"` and `"auto"` styles. After `max_depth` segments are produced, remaining `_` characters are preserved as literal underscores. This prevents excessively deep nesting from long env var names. Ignored for `"flat"` style.

```
Examples (max_depth=5):
  A_B_C_D_E=1       → a.b.c.d.e          (5 segments — within limit)
  A_B_C_D_E_F_G=1   → a.b.c.d.e_f_g      (5 segments — F_G kept as literal)
  A_B_C_D_E_F_G_H=1 → a.b.c.d.e_f_g_h    (5 segments — F_G_H kept as literal)
```

**Type coercion** follows the same rules as §9.2: `"true"`/`"false"` → boolean, numeric strings → int/float, otherwise string.

> **Note — apflow env var compatibility:** The apflow project currently uses a simpler convention where dots become single underscores without the double-underscore escape (e.g., `api.server_url` → `APFLOW_API_SERVER_URL`). This works because apflow's config keys do not contain literal underscores. When migrating to the Config Bus, apflow **SHOULD** adopt the §9.2 convention (`APFLOW_API_SERVER__URL`) for consistency, but implementations **may** accept both forms during a transition period by attempting double-underscore parsing first and falling back to the simpler form.

#### 9.8.2 Env Prefix Conflict Prevention

Env prefix conflicts arise when one registered prefix is a string prefix of another, making it ambiguous which namespace owns a given env var. The following rules prevent this:

1. Each `env_prefix` **MUST** be unique across all registered namespaces. Attempting to register a duplicate `env_prefix` **MUST** raise `CONFIG_ENV_PREFIX_CONFLICT`.
2. Any `env_prefix` that starts with `APCORE_` (i.e., matches `^APCORE_[A-Z0-9]`) **MUST** raise `CONFIG_ENV_PREFIX_CONFLICT`. This prevents collision with the `apcore` namespace's `APCORE_` prefix. The double-underscore form (`^APCORE_[A-Z]`, e.g., `APCORE_MCP`) is explicitly permitted and dispatched via longest-prefix-match (see dispatch algorithm below).
3. The prefix `APCORE` is reserved for the `apcore` namespace. Attempting to register it for another namespace **MUST** raise `CONFIG_NAMESPACE_RESERVED`.

**Resolving the `APCORE` / `APCORE_MCP` ambiguity:**

A naive prefix scheme would make `APCORE_MCP` (for apcore-mcp) collide with `APCORE_` (for apcore, key path `mcp.*`). The ecosystem convention resolves this by requiring apcore ecosystem packages to use a double-underscore separator between `APCORE` and the sub-package name in their env prefix:

| Package | Namespace | Env Prefix | Why safe |
|---------|-----------|------------|----------|
| apcore | `apcore` | `APCORE` | Base prefix |
| apcore-mcp | `apcore-mcp` | `APCORE_MCP` | `APCORE_MCP_` is not a valid `APCORE_` key (double `__` creates invalid path) |
| apcore-a2a | `apcore-a2a` | `APCORE_A2A` | Same reasoning |
| apcore-cli | `apcore-cli` | `APCORE_CLI` | Same reasoning |
| apflow | `apflow` | `APFLOW` | Completely disjoint prefix |
| django-apcore | `django-apcore` | `DJANGO_APCORE` | No `DJANGO` namespace registered — no prefix collision |

This works because the `APCORE_` prefix matcher stops at the first `_` boundary. An env var like `APCORE_MCP_TRANSPORT` starts with `APCORE_` (double underscore), which the `APCORE_` prefix handler would interpret as key path `apcore._mcp.transport` — not a valid apcore config path. The `APCORE_MCP_` prefix handler correctly claims it.

However, this convention introduces complexity. Implementations **MUST** use **longest-prefix-match** when dispatching env vars to namespaces:

```
Algorithm: dispatch_env_var(env_key, registered_prefixes)

Input:
  env_key             — Environment variable name (e.g., "APCORE_MCP_TRANSPORT")
  registered_prefixes — List of (env_prefix + "_", namespace_name) tuples,
                        sorted by prefix length descending

Output:
  (namespace_name, suffix) or nil

Steps:
  1. For each (prefix, ns_name) in registered_prefixes (longest first):
       If env_key starts with prefix:
         → Return (ns_name, env_key[len(prefix):])
  2. Return nil (env var does not match any namespace)
```

#### 9.8.3 Env Override Application Algorithm

```
Algorithm: apply_namespace_env_overrides(config_data, registered_namespaces)

Input:
  config_data           — Merged configuration tree (all namespaces)
  registered_namespaces — Map of namespace name → registration info

Output:
  config_data with env overrides applied per namespace

Steps:
  0. Build prefix table (with auto-derive):
       registered_prefixes ← []
       For each (name, registration) in registered_namespaces:
         prefix ← registration.env_prefix
         If prefix is nil:
           prefix ← name.upper().replace("-", "_")     # auto-derive
         registered_prefixes.append((prefix + "_", name, registration))
       Sort registered_prefixes by prefix length descending (longest first)

  1. Build env_map lookup (global + per-namespace):
       global_env_map ← Config._global_env_map           # from Config.env_map()
       ns_env_maps ← {}                                   # env_var → (ns_name, config_key)
       For each (name, registration) in registered_namespaces:
         If registration.env_map is not nil:
           For each (env_var, config_key) in registration.env_map:
             ns_env_maps[env_var] ← (name, config_key)

  2. For each (env_key, env_value) in environment variables:
       coerced ← coerce_env_value(env_value)

       # 2a. Check global env_map (bare env var → top-level key)
       If env_key in global_env_map:
         config_data[global_env_map[env_key]] ← coerced
         continue

       # 2b. Check namespace env_map (bare env var → namespace key)
       If env_key in ns_env_maps:
         (ns_name, config_key) ← ns_env_maps[env_key]
         set config_data[ns_name][config_key] ← coerced
         continue

       # 2c. Prefix-based dispatch (existing logic)
       match ← dispatch_env_var(env_key, registered_prefixes)
       If match is nil → skip
       (ns_name, suffix, registration) ← match
       max_depth ← registration.max_depth or 5
       If registration.env_style == "flat":
         key ← lowercase(suffix)
         set config_data[ns_name][key] ← coerced
       Else if registration.env_style == "auto":
         key ← auto_resolve(suffix, registration.defaults, 0, max_depth)
         set config_data[ns_name] via key (flat or nested depending on resolution)
       Else:  # "nested"
         dot_path ← convert suffix (single _ → ., double __ → _), lowercase,
                     stopping at max_depth segments
         set config_data[ns_name][dot_path] ← coerced (nested via set_nested)

  3. Return config_data
```

### 9.9 Namespace-Aware Access API

#### 9.9.1 Unified Access

In namespace mode, `get()` and `set()` use dot-paths where the first segment is the namespace:

```
config.get("apcore.executor.default_timeout")   → 30000
config.get("apflow.api.timeout")                 → 30.0
config.get("apcore-mcp.transport")               → "streamable-http"
config.get("billing.db.host")                    → "localhost"
```

In legacy mode, `get()` behaves as before (no namespace prefix):

```
config.get("executor.default_timeout")           → 30000
```

**Implementations MUST NOT break legacy mode access patterns.** When the config is in legacy mode, `get("executor.default_timeout")` **MUST** continue to work without requiring an `apcore.` prefix.

**Dot-path namespace resolution algorithm:**

Because namespace names may contain hyphens (e.g., `apcore-mcp`), implementations **MUST NOT** naively split on the first `.` to extract the namespace. Instead:

```
Algorithm: resolve_namespace_path(dot_path, mode, known_namespaces)

Input:
  dot_path         — Full dot-path string (e.g., "apcore-mcp.transport")
  mode             — "legacy" or "namespace"
  known_namespaces — Set of registered + loaded namespace names

Output:
  (namespace, remainder) or (nil, dot_path) for legacy mode

Steps:
  1. If mode == "legacy":
       → Return (nil, dot_path)   // entire path is within apcore namespace

  2. Extract candidate ← substring before the first "."
     remainder ← substring after the first "."
     // candidate = "apcore-mcp", remainder = "transport"
     // Hyphens are legal in namespace names but NOT in config key segments,
     // so this split is unambiguous.

  3. If candidate is in known_namespaces:
       → Return (candidate, remainder)

  4. Else:
       → Return (candidate, remainder)
       // Unknown namespace — behavior depends on strict/allow_unknown settings.
       // The get() method should look up candidate as a top-level key in the
       // config data tree; if absent, return the provided default.
```

> **Why this is unambiguous:** Namespace names allow hyphens (`^[a-z][a-z0-9]*(-[a-z0-9]+)*$`) but the §9.1 config keys within a namespace use underscores, not hyphens (e.g., `default_timeout`, `max_depth`). The first `.` is always the boundary between namespace and config path. A dot-path like `apcore-mcp.transport` can only mean namespace `apcore-mcp`, key `transport` — never namespace `apcore`, key `mcp.transport` (because `apcore` in namespace mode would require `apcore.mcp.transport`).

#### 9.9.2 Namespace Accessor

Implementations **SHOULD** provide a convenience method to retrieve the entire configuration subtree for a namespace:

```
config.namespace("apflow")
→ {"api": {"server_url": "...", "timeout": 30.0}, "governance": {...}}
```

This returns a deep copy. Mutations to the returned object **MUST NOT** affect the Config Bus.

#### 9.9.3 Typed Access

Implementations **SHOULD** provide typed access methods appropriate to the language:

**Statically typed languages (Rust, Go, Java, TypeScript):**

```
config.bind<T>("apflow")  → T    // Deserialize namespace into typed struct
```

**Python:**

```python
# Option A: runtime type check
timeout: float = config.get_typed("apflow.api.timeout", float)

# Option B: Pydantic / dataclass binding (Pydantic as optional dependency)
settings: ApflowSettings = config.bind("apflow", ApflowSettings)
```

**Binding rules:**

1. `bind()` deserializes the namespace subtree into the target_id type.
2. If the namespace has a registered JSON Schema, validation **SHOULD** have already occurred at load time. `bind()` performs structural deserialization only — it **SHOULD NOT** re-validate.
3. If deserialization fails (missing fields, type mismatch), `bind()` **MUST** raise a `ConfigError` with a clear message indicating the namespace and the failing field.
4. The model type used in `bind()` is owned by the downstream package, not by apcore. apcore provides the mechanism, not the types.

#### 9.9.4 Registered Namespace Introspection

Implementations **SHOULD** expose a method to list registered namespaces:

```
Config.registered_namespaces()
→ [
    {name: "apcore",     env_prefix: "APCORE",      has_schema: true},
    {name: "apflow",     env_prefix: "APFLOW",      has_schema: true},
    {name: "apcore-mcp", env_prefix: "APCORE_MCP", has_schema: true},
    {name: "billing",    env_prefix: "BILLING",      has_schema: false},
  ]
```

This is useful for diagnostic tools, CLI introspection, and IDE plugins.

#### 9.9.5 Reserved Namespace Query

> **Added in v1.9.0**

§9.5.1 (rules 3 and 4) defines a set of namespace names that are reserved by the framework and **MUST NOT** be registered by external callers (`apcore` is owned by the framework itself; `_config` is used for Config Bus meta-configuration per §9.6.3).

Implementations **MUST** expose a public, read-only query API returning the set of reserved top-level namespace names.

**Canonical signature (pseudocode):**

```
Config.reserved_namespaces()
→ frozen_set<string>   # MUST be immutable from the caller's perspective
```

**Requirements:**

1. The returned set **MUST** include at minimum `apcore` and `_config`.
2. The returned set **MUST** be the same set used by `register_namespace` to enforce `CONFIG_NAMESPACE_RESERVED` (single source of truth — implementations **MUST NOT** maintain two separate lists).
3. The returned set **MUST** be immutable from the caller's perspective. Languages without immutable set types **MUST** return a defensive copy or a read-only view.
4. The API **MUST** be callable without instantiating `Config` (class-level or module-level access) — diagnostic tools may query reserved names before any config file is loaded.
5. The method name follows language idiom (snake_case for Python/Rust, camelCase getter or method for TypeScript). Concrete names are non-normative; semantics are normative.

**Cross-language examples:**

=== "Python"
    ```python
    from apcore import Config

    # Class-level query — no Config instance needed
    reserved = Config.reserved_namespaces()
    assert "apcore" in reserved
    assert "_config" in reserved

    # Pre-validate user input before register_namespace
    def register_user_namespace(name: str) -> None:
        if name in Config.reserved_namespaces():
            raise ValueError(
                f"Namespace '{name}' is reserved by the apcore framework. "
                f"Pick a different name (e.g., 'my-{name}')."
            )
        Config.register_namespace(name)
    ```

=== "TypeScript"
    ```typescript
    import { Config } from 'apcore-js';

    // Static getter — no Config instance needed
    const reserved: ReadonlySet<string> = Config.reservedNamespaces;
    console.assert(reserved.has('apcore'));
    console.assert(reserved.has('_config'));

    // Pre-validate user input before registerNamespace
    function registerUserNamespace(name: string): void {
      if (Config.reservedNamespaces.has(name)) {
        throw new Error(
          `Namespace '${name}' is reserved by the apcore framework. ` +
          `Pick a different name (e.g., 'my-${name}').`,
        );
      }
      Config.registerNamespace(name);
    }
    ```

=== "Rust"
    ```rust
    use apcore::Config;

    // Module-level query — no Config instance needed.
    // Returned as `&'static [&'static str]` (idiomatic Rust for a small,
    // compile-time-known reserved set; inherently immutable, zero runtime
    // initialisation cost).
    let reserved: &'static [&'static str] = Config::reserved_namespaces();
    assert!(reserved.contains(&"apcore"));
    assert!(reserved.contains(&"_config"));

    // Pre-validate user input before register_namespace.
    fn register_user_namespace(name: &str) -> Result<(), String> {
        if Config::reserved_namespaces().iter().any(|&s| s == name) {
            return Err(format!(
                "Namespace '{}' is reserved by the apcore framework. \
                 Pick a different name (e.g., 'my-{}').",
                name, name,
            ));
        }
        // Real call site: Config::register_namespace takes a NamespaceRegistration
        // struct and returns Result<(), ModuleError>. Omitted here for brevity;
        // see §9.5.1.
        Ok(())
    }
    ```

**Intended audience:** This API is for third-party consumers (custom CLIs, framework integrations, application code) that accept user-supplied namespace names. Official downstream packages in the apcore ecosystem (`apcore-cli-*`, `apcore-mcp-*`, `apcore-toolkit-*`) do not call `Config.set()` or `register_namespace()` with user input and therefore do not need to consume this API.

### 9.10 Validation Algorithm (Namespace-Aware A12-NS)

The original Algorithm A12 (§9.3) is extended for namespace mode:

```
Algorithm: validate_config_ns(config_data, mode, registered_namespaces, meta_config)

Input:
  config_data           — Full configuration tree
  mode                  — "legacy" or "namespace"
  registered_namespaces — Map of namespace name → registration info
  meta_config           — Parsed _config section (strict, allow_unknown)

Output:
  validated config, or throw CONFIG_INVALID error

Steps:
  1. If mode == "legacy":
       → Run original A12 algorithm on config_data (unchanged behavior)
       → Run reject_unknown_framework_keys(config_data, meta_config)
       → Return

  2. Validate the "apcore" namespace:
       apcore_data ← config_data["apcore"]
       Run original A12 algorithm on apcore_data
       Run reject_unknown_framework_keys(apcore_data, meta_config)

  3. For each top-level key (ns_name) in config_data:
       If ns_name == "_config" → skip (meta-configuration)
       If ns_name == "apcore" → skip (already validated in step 2)

       a. If ns_name is in registered_namespaces:
            registration ← registered_namespaces[ns_name]
            If registration.schema is not nil:
              Validate config_data[ns_name] against registration.schema
              Collect errors with namespace prefix: "{ns_name}: {error}"

       b. Else (unknown namespace):
            If meta_config.strict == true:
              Error: "Unknown namespace '{ns_name}' (strict mode enabled)"
            Else if meta_config.allow_unknown == true:
              Log WARN: "Namespace '{ns_name}' is not registered; data stored but not validated"
            Else:
              Remove config_data[ns_name] from the tree (silently ignore)

  4. If any errors collected → throw CONFIG_INVALID with all errors
  5. Return validated config_data
```

```
Sub-algorithm: reject_unknown_framework_keys(apcore_data, meta_config)

Steps:
  1. If meta_config.strict != true:
       → Return (the key is retained and readable; see below)

  2. declared ← union of the canonical config schemas (see below)
     For each framework section in declared:
       For each key present in apcore_data[section]:
         If declared[section] does not contain the key:
           Collect error: "Unknown key '{section}.{key}' (strict mode enabled)"

  3. If any errors collected → throw CONFIG_INVALID with ALL of them
```

**Unknown keys inside the `apcore` namespace.** Every framework section in
`schemas/apcore-config.schema.json` is `additionalProperties: false`. That
closedness is now enforced — but **only** under `_config.strict: true`.

- **Default (`strict: false`).** An unknown key inside a framework section
  **MUST** be retained and readable through `get()`. Implementations **MUST NOT**
  silently discard it. This is a cross-language requirement: an SDK that models a
  section as a typed record still has to keep what the record does not model,
  because "the operator wrote it and it vanished" is indistinguishable from "the
  operator never wrote it".
- **`strict: true`.** The key **MUST** cause `CONFIG_INVALID`, and the error
  **MUST** enumerate every offending key rather than failing on the first, so one
  restart is enough to see the whole problem.

`allow_unknown` does **not** apply here. It is defined in §9.6.3 for unknown
top-level *namespaces*, and stretching one field across two granularities would
make its meaning depend on where it is read.

**The declared surface is the union of the canonical schemas, not one file.**
`schemas/apcore-config.schema.json` is the root, but sections that own a separate
file declare their keys there — `$defs/SysModulesConfig` stops at `enabled` while
`schemas/sys-modules.schema.json` declares `control.*`, `error_history.*` and
`events.*` under §9.15.3. Implementations **MUST** derive the surface from the
same set `conformance/generate_config_key_governance.py` reads, which is the
authoritative list and is pinned by `conformance/fixtures/config_key_governance.json`.

Taking the root file alone would reject `sys_modules.events.enabled` — documented,
and validated by every SDK's constraint table. This is stated because apcore-python
and apcore-typescript each arrived at the union independently while implementing
this rule: two implementations converging on an unwritten convention is how the
next one diverges.

!!! note "Why `strict` and not a warning"
    `strict` already means "every top-level key must correspond to a registered
    namespace" (§9.6.3). Extending it to the keys *inside* the framework namespace
    is the same promise one level down, for operators who have already asked for it.
    A key with no declaration is either a typo — in which case the operator is
    looking at a default they believe they overrode — or dead configuration. Both
    are worth a startup failure to someone who opted into strictness, and neither
    is worth one to someone who did not.

### 9.11 Hot-Reload (Namespace Mode)

`config.reload()` **MUST** support namespace mode:

1. Re-read the YAML file.
2. Re-detect mode (legacy vs namespace).
3. Re-apply all registered namespace defaults.
4. Re-apply all environment variable overrides per registered namespace.
5. Re-validate per A12-NS — **but only if the originating `load()` validated.** An SDK whose `load()` accepts a validation opt-out (`validate=False` / `{validate: false}`) **MUST** carry that choice forward to `reload()`. Silently re-imposing validation on a config the caller deliberately loaded without it is a behaviour change disguised as a refresh; conversely an SDK with no such opt-out **MUST** always re-validate here. Reload is a refresh of the same config, not a stricter reload of it.
6. Atomically replace the config data tree.
7. Re-apply mount data for namespaces that were mounted (mount sources **SHOULD** be remembered).

If a mount source was `from_file`, the file **SHOULD** also be re-read during reload. If the mount source was `from_dict`, the original dict is re-applied (not re-read).

### 9.12 Cross-Language Implementation Requirements

#### 9.12.1 Required API Surface

All SDK implementations claiming Config Bus conformance **MUST** implement:

| Method | Requirement Level | Description |
|--------|-------------------|-------------|
| `Config.register_namespace()` | MUST | Static/class method for namespace registration |
| `Config.env_map()` | MUST | Static/class method to register global bare env var → top-level config key mappings |
| `Config.load()` | MUST | Load with mode detection (legacy/namespace) |
| `Config.from_defaults()` | MUST | Create from defaults (apcore namespace only, legacy compatible) |
| `config.get(dot_path)` | MUST | Namespace-aware dot-path access |
| `config.set(dot_path, value)` | MUST | Namespace-aware dot-path mutation |
| `config.mount(ns, source)` | MUST | Attach external config source |
| `config.validate()` | MUST | A12-NS validation |
| `config.reload()` | MUST | Hot-reload with namespace support |
| `config.namespace(name)` | SHOULD | Retrieve full namespace subtree |
| `config.bind(ns, type)` | SHOULD | Typed deserialization |
| `config.get_typed(path, type)` | SHOULD | Single-value typed access |
| `Config.registered_namespaces()` | SHOULD | Introspection |

#### 9.12.2 Language-Idiomatic Naming

| Concept | Python | TypeScript | Rust | Go | Java |
|---------|--------|------------|------|----|------|
| Register | `register_namespace()` | `registerNamespace()` | `register_namespace()` | `RegisterNamespace()` | `registerNamespace()` |
| Load | `Config.load()` | `Config.load()` | `Config::load()` | `config.Load()` | `Config.load()` |
| Get | `config.get()` | `config.get()` | `config.get()` | `cfg.Get()` | `config.get()` |
| Mount | `config.mount()` | `config.mount()` | `config.mount()` | `cfg.Mount()` | `config.mount()` |
| Bind | `config.bind()` | `config.bind<T>()` | `config.bind::<T>()` | `config.Bind()` | `config.bind()` |
| Namespace | `config.namespace()` | `config.namespace()` | `config.namespace()` | `cfg.Namespace()` | `config.namespace()` |

> **Parameter passing style:** Each language should use its idiomatic parameter passing convention. Python uses keyword arguments (`mount("ns", from_file="path")`), TypeScript uses an options object (`mount('ns', { fromFile: 'path' })`), Rust uses enum variants (`mount("ns", MountSource::File(...))`), Go uses functional options or helper constructors, and Java uses builder or overloaded methods. The semantic contract is identical; only the surface syntax varies.

#### 9.12.3 Thread Safety

All Config Bus methods **MUST** be thread-safe / concurrency-safe:

- `register_namespace()` **MUST** use a global lock or atomic registry (registrations can happen from multiple threads during framework startup).
- `get()`, `set()`, `mount()`, `reload()` **MUST** be synchronized per Config instance (same requirement as §12.7).
- `bind()` reads a snapshot — the returned object is independent and needs no synchronization.

#### 9.12.4 Error Types

Implementations **MUST** use the following error codes (extensions to §8). All config errors inherit from the base error type (`ModuleError` in Python, equivalent in other languages) and are non-retryable (`retryable = false`), consistent with the existing `CONFIG_NOT_FOUND` and `CONFIG_INVALID` codes defined in §8.2:

| Error Code | Trigger | Existing? |
|------------|---------|-----------|
| `CONFIG_NOT_FOUND` | Configuration file not found | §8.2 (unchanged) |
| `CONFIG_INVALID` | Validation failure, extended to include namespace-level schema validation errors and strict-mode unknown namespace errors | §8.2 (extended) |
| `CONFIG_NAMESPACE_DUPLICATE` | `register_namespace` called twice for the same namespace name | New |
| `CONFIG_NAMESPACE_RESERVED` | Attempt to register `apcore` or `_config` | New |
| `CONFIG_ENV_PREFIX_CONFLICT` | Duplicate `env_prefix`, or `env_prefix` matches `^APCORE_[A-Z0-9]` (collides with the `apcore` namespace's `APCORE_` prefix) | New |
| `CONFIG_MOUNT_ERROR` | Mount source file not found, invalid YAML in mount file, or mount to `_config` | New |
| `CONFIG_BIND_ERROR` | Typed deserialization failure in `bind()` — missing fields or type mismatch between namespace data and target_id type | New |
| `CONFIG_ENV_MAP_CONFLICT` | An env var name in `env_map` is already claimed by another `env_map` (global or namespace) | New |

### 9.13 Ecosystem Integration Patterns

#### 9.13.1 apcore Ecosystem Packages

Packages in the apcore ecosystem (apcore-mcp, apcore-a2a, apcore-cli, apflow, framework integrations) **SHOULD** follow this pattern:

```python
# In package __init__.py or equivalent entry point

from apcore import Config

# Register at import time — before Config.load() is called by the application
Config.register_namespace(
    "apcore-mcp",
    schema=_resolve_schema_path("apcore-mcp.schema.json"),
    env_prefix="APCORE_MCP",   # double underscore to avoid APCORE_ prefix collision
    defaults={"transport": "streamable-http", "port": 8000},
)
```

**Convention for apcore ecosystem packages:**

| Package | Namespace | Env Prefix | Conflict? | Schema |
|---------|-----------|------------|-----------|--------|
| apcore (core) | `apcore` | `APCORE` | — | `apcore-config.schema.json` |
| apcore-mcp | `apcore-mcp` | `APCORE_MCP` | Yes — `APCORE_` is a prefix of `APCORE_MCP_`; use `APCORE_MCP` to disambiguate | `apcore-mcp.schema.json` |
| apcore-a2a | `apcore-a2a` | `APCORE_A2A` | Same as above | `apcore-a2a.schema.json` |
| apcore-cli | `apcore-cli` | `APCORE_CLI` | Same as above | `apcore-cli.schema.json` |
| apflow | `apflow` | `APFLOW` | No — disjoint from `APCORE_` | `apflow.schema.json` |
| django-apcore | `django-apcore` | `DJANGO_APCORE` | No — no `DJANGO` namespace registered | `django-apcore.schema.json` |
| fastapi-apcore | `fastapi-apcore` | `FASTAPI_APCORE` | No — no `FASTAPI` namespace registered | `fastapi-apcore.schema.json` |
| flask-apcore | `flask-apcore` | `FLASK_APCORE` | No — no `FLASK` namespace registered | `flask-apcore.schema.json` |
| nestjs-apcore | `nestjs-apcore` | `NESTJS_APCORE` | No — no `NESTJS` namespace registered | `nestjs-apcore.schema.json` |
| axum-apcore | `axum-apcore` | `AXUM_APCORE` | No — no `AXUM` namespace registered | `axum-apcore.schema.json` |

> **Why `APCORE_MCP` and not `APCORE_MCP`?** The prefix `APCORE_` (for the `apcore` namespace) is a string prefix of `APCORE_MCP_`. An env var like `APCORE_MCP_TRANSPORT` is ambiguous: does it set `apcore → mcp.transport` or `apcore-mcp → transport`? The double-underscore convention (`APCORE_MCP_`) breaks the ambiguity because `APCORE_` never matches `APCORE_MCP_TRANSPORT` as an apcore key (the double underscore creates an invalid key path `_mcp.transport`). Framework integrations like `DJANGO_APCORE` do not have this problem because no `DJANGO` namespace is registered, so `DJANGO_APCORE_*` is unambiguous.

#### 9.13.2 Third-Party Package Integration

Third-party packages that want to participate in the Config Bus **SHOULD** follow this pattern:

```python
# In third-party package setup
from apcore import Config

# Schema is optional — register for namespace isolation and env override only
Config.register_namespace(
    "my-billing",
    env_prefix="BILLING",
    # No schema — existing config structure is not constrained by apcore
)
```

Third-party packages **MUST NOT** assume that apcore.Config is the only configuration source. The registration is additive — if the application does not use the Config Bus, the package must fall back to its own configuration mechanism:

```python
# Defensive integration pattern for third-party packages
def get_billing_config(apcore_config=None):
    """Return billing config from Config Bus if available, else standalone."""
    if apcore_config is not None:
        try:
            return apcore_config.namespace("my-billing")
        except KeyError:
            pass

    # Fallback: use own config mechanism
    return load_billing_config_standalone()
```

#### 9.13.3 Framework Integration Auto-Registration

Framework integrations (django-apcore, fastapi-apcore, etc.) **SHOULD** auto-register their namespace when the framework loads them:

**Django:**

```python
# django_apcore/apps.py
from django.apps import AppConfig as DjangoAppConfig

class DjangoApcoreConfig(DjangoAppConfig):
    name = "django_apcore"

    def ready(self):
        from apcore import Config
        Config.register_namespace(
            "django-apcore",
            schema=self._resolve_schema(),
            env_prefix="DJANGO_APCORE",
            defaults={"auto_register_modules": True, "url_prefix": "/api/apcore"},
        )
```

**FastAPI:**

```python
# fastapi_apcore/__init__.py
from apcore import Config

Config.register_namespace(
    "fastapi-apcore",
    schema=_resolve_schema(),
    env_prefix="FASTAPI_APCORE",
    defaults={"auto_register_modules": True, "route_prefix": "/apcore"},
)
```

**NestJS:**

**NestJS** (no `NESTJS` namespace exists — single underscore is safe):

```typescript
// nestjs-apcore/src/index.ts
import { Config } from 'apcore-js';

Config.registerNamespace({
  name: 'nestjs-apcore',
  schema: resolveSchema(),
  envPrefix: 'NESTJS_APCORE',
  defaults: { autoRegisterModules: true, routePrefix: '/apcore' },
});
```

### 9.14 Config Discovery (Optional)

Implementations **may** support automatic configuration file discovery. When `Config.load()` is called without a path argument, the following search order **SHOULD** be used:

```
Algorithm: discover_config_file()

Search order (first match wins):
  1. $APCORE_CONFIG_FILE          — explicit override via env var
  2. ./project.yaml               — project root
  3. ./project.yml
  4. ./apcore.yaml                — apcore-specific
  5. ./apcore.yml
  6. ~/.config/apcore/config.yaml — user-level (XDG on Linux, ~/Library/Application Support on macOS)
  7. ~/.apcore/config.yaml        — legacy user-level

If no file is found:
  → Use Config.from_defaults() (apcore namespace only, no error)
```

This is a **MAY**-level feature. Implementations that do not support discovery **MUST** require an explicit path in `Config.load()`.

> **Note:** The file name does not influence mode detection. A file named `project.yaml` may contain legacy-mode content, and a file named `apcore.yaml` may contain namespace-mode content. Mode is always determined by the presence of the `apcore:` top-level key (see §9.6.1).

### 9.15 apcore Built-in Namespace Registrations

The framework pre-registers two namespaces for its own subsystems at startup, before any application `Config.load()` call. This applies the Config Bus pattern (§9.4) to apcore's own internal configuration — the same mechanism apcore exposes to third parties is now used by apcore itself.

Both namespaces promote existing flat keys that already live inside the `apcore` namespace into dedicated, independently-configurable namespaces. The migration is strictly additive: legacy mode files continue to work unchanged.

#### 9.15.1 Bootstrap Order

```
┌─────────────────────────────────────────────────────────────┐
│                Framework Bootstrap Order                    │
│                                                             │
│  1. Config.register_namespace("observability", ...)         │
│  2. Config.register_namespace("sys_modules", ...)           │
│  3. [Application / ecosystem packages register their own]   │
│  4. Config.load(path)                                       │
│  5. register_sys_modules(config)                            │
└─────────────────────────────────────────────────────────────┘
```

Third-party packages **SHOULD** register their namespaces after step 2 and before step 4 (at import time).

#### 9.15.2 `observability` Namespace

Extracts the existing `observability.*` flat keys from the `apcore` namespace into a dedicated Config Bus namespace. This makes observability configuration independently addressable and allows ecosystem adapters to read a single authoritative source.

```python
Config.register_namespace(
    "observability",
    schema="schemas/observability.schema.json",
    env_prefix="APCORE_OBSERVABILITY",
    defaults={
        "tracing": {
            "enabled": False,
            "strategy": "full",        # "full" | "proportional" | "error_first" | "off"
            "sampling_rate": 1.0,
            "exporter": "stdout",      # "stdout" | "otlp" | "in_memory"
            "otlp_endpoint": None,
        },
        "metrics": {
            "enabled": False,
            "exporter": "stdout",      # "stdout" | "prometheus" | "in_memory"
        },
        "logging": {
            "enabled": True,
            "level": "info",           # "trace"|"debug"|"info"|"warn"|"error"|"fatal"
            "format": "json",          # "json" | "text"
            "redact_sensitive": True,
        },
        "error_history": {
            "max_entries_per_module": 50,
            "max_total_entries": 1000,
        },
        "platform_notify": {
            "enabled": False,
            "error_rate_threshold": 0.1,
            "latency_p99_threshold_ms": 5000.0,
        },
    },
)
```

**Migration:** Existing `apcore.observability.*` flat keys map 1:1 to `observability.*` namespace keys. No changes required to existing configuration files.

**Environment variable examples (`env_prefix = APCORE_OBSERVABILITY`):**

```
APCORE_OBSERVABILITY_TRACING_STRATEGY=error_first
APCORE_OBSERVABILITY_LOGGING_LEVEL=debug
APCORE_OBSERVABILITY_METRICS_EXPORTER=prometheus
```

**Ecosystem adoption** — adapter packages **SHOULD** read from this namespace rather than maintaining their own observability defaults:

| Package | Keys to read |
|---------|-------------|
| apcore (core) | Full namespace — owns all keys |
| apcore-mcp | `tracing.strategy`, `logging.level` |
| apcore-a2a | `tracing.strategy`, `logging.level` |
| apcore-cli | `logging.level`, `logging.format` |
| Third-party | `logging.level` (MAY) — read-only |

#### 9.15.3 `sys_modules` Namespace

Promotes the existing `sys_modules.*` flat keys — currently read directly by `register_sys_modules()` — into a dedicated namespace, making system module configuration independently overridable without touching the `apcore` namespace.

```python
Config.register_namespace(
    "sys_modules",
    schema="schemas/sys-modules.schema.json",
    env_prefix="APCORE_SYS",
    defaults={
        "enabled": True,
        "health":   {"enabled": True},
        "manifest": {"enabled": True},
        "usage": {
            "enabled": True,
            "retention_hours": 168,
            "bucketing_strategy": "hourly",
        },
        "control":  {"enabled": True},
        "events": {
            "enabled": True,
            "thresholds": {
                "error_rate": 0.1,
                "latency_p99_ms": 5000.0,
            },
        },
    },
)
```

**Migration:** `register_sys_modules()` **MUST** prefer `config.namespace("sys_modules")` in namespace mode, falling back to `config.get("sys_modules.*")` in legacy mode. No breaking change.

**Environment variable examples (`env_prefix = APCORE_SYS`):**

```
APCORE_SYS_ENABLED=true
APCORE_SYS_USAGE_RETENTION__HOURS=336
APCORE_SYS_EVENTS_ENABLED=false
```

---

### 9.16 Event Type Naming Convention and Canonical Definitions

apcore-python currently emits event types as hardcoded strings scattered across multiple files, with two confirmed collisions:

- `"module_health_changed"` used in `control.py` (toggle on/off) and `platform_notify.py` (error rate recovery) with different payloads
- `"config_changed"` used for both key-value updates and module reload notifications

This section defines the canonical event type names and payload contracts, resolving collisions and establishing the naming convention for the ecosystem.

#### 9.16.1 Naming Convention

Event type names **MUST** use dot-namespaced format. The prefix identifies ownership:

| Prefix | Owner | Examples |
|--------|-------|---------|
| `apcore.*` | Core framework | `apcore.registry.module_registered`, `apcore.config.updated` |
| `apcore-mcp.*` | apcore-mcp | `apcore-mcp.tool_called` |
| `apcore-a2a.*` | apcore-a2a | `apcore-a2a.task_submitted` |
| `apcore-cli.*` | apcore-cli | `apcore-cli.command_invoked` |
| `apflow.*` | apflow | `apflow.step_completed` |
| Custom | Third-party | `billing.invoice_generated` |

The `apcore.*` prefix is reserved. Ecosystem packages **MUST NOT** emit events with this prefix.

#### 9.16.2 Canonical Core Event Types

The following are the canonical event type names, payload keys, and severity for all events emitted by apcore SDKs. Implementations **MUST** use these names. Two distinct rename cohorts are reflected in the legacy column:

- **Cohort A (removed in v0.18.0):** the unprefixed short-form names `module_health_changed` and `config_changed` were emitted as transitional aliases up to v0.17.x and were **REMOVED in v0.18.0**.
- **Cohort B (renamed in v0.22.0, see [event-system.md](../features/event-system.md#deprecation-legacy-event-names)):** four early names that violated the `apcore.<subsystem>.<event>` convention (`module_registered`, `module_unregistered`, `apcore.error.threshold_exceeded`, `apcore.latency.threshold_exceeded` — the latter two used `error`/`latency` as the subsystem segment, which are categories, not subsystems) were **renamed in v0.22.0**. Dual-emission through v0.21.x has ended; implementations **MUST** emit only the canonical names below.

| Canonical Name | Alias (legacy) | Severity | Emitted by | Payload Keys |
|----------------|---------------|----------|------------|--------------|
| `apcore.registry.module_registered` | `module_registered` (v0.22.0 rename), `apcore.module.registered` (early draft) | `info` | Registry bridge | `module_id` |
| `apcore.registry.module_unregistered` | `module_unregistered` (v0.22.0 rename), `apcore.module.unregistered` (early draft) | `info` | Registry bridge | `module_id` |
| `apcore.module.toggled` | *(new — was collision)* | `info`/`warn` | `system.control.toggle_feature` | `module_id`, `enabled` |
| `apcore.module.reloaded` | `config_changed` (partial, v0.18.0 removal) | `info` | `system.control.reload_module` | `module_id`, `previous_version`, `new_version` |
| `apcore.config.updated` | `config_changed` (partial, v0.18.0 removal) | `info` | `system.control.update_config` | `key`, `old_value`, `new_value` |
| `apcore.health.error_threshold_exceeded` | `apcore.error.threshold_exceeded` (v0.22.0 rename) | `error` | `PlatformNotifyMiddleware` | `module_id`, `error_rate`, `threshold` |
| `apcore.health.latency_threshold_exceeded` | `apcore.latency.threshold_exceeded` (v0.22.0 rename) | `warn` | `PlatformNotifyMiddleware` | `module_id`, `p99_latency_ms`, `threshold` |
| `apcore.health.recovered` | *(new — was collision)* | `info` | `PlatformNotifyMiddleware` | `module_id`, `error_rate` |
| `apcore.approval.decision` | *(new — v1.9.0, #77)* | `info` (approved/pending) / `warn` (rejected/timeout) | Approval Gate (§7) | `module_id`, `status`, `approved_by`, `reason`, `approval_id`, `trace_id` |
| `apcore.policy.override` | *(new — v1.9.0, #77)* | `info` | Approval Gate (§7) | `module_id`, `pattern`, `requires_approval`, `destructive`, `needs_approval`, `reason`, `trace_id` |
| `apcore.acl.denied` | *(new — v1.9.0, #77)* | `warn` | ACL Check (§6) | `module_id`, `caller_id`, `reason`, `trace_id` |
| `apcore.stream.post_validation_failed` | *(new — v1.9.0, documents existing emit)* | `error` | Executor (streaming Phase 3) | `error_type`, `message`, `trace_id` |
| `apcore.registry.module_load_failed` | *(new — v1.9.0, documents existing emit)* | `error` | Registry | `module_id`, `callback_name`, `error_type`, `error_message` |
| `apcore.circuit.opened` | *(new — v1.9.0, documents existing emit)* | `warn` | `CircuitBreakerMiddleware` | `module_id`, `caller_id`, `error_rate` |
| `apcore.circuit.closed` | *(new — v1.9.0, documents existing emit)* | `info` | `CircuitBreakerMiddleware` | `module_id`, `caller_id`, `error_rate` |
| `apcore.subscriber.circuit_opened` | *(new — v1.9.0, documents existing emit)* | `warn` | Event delivery (per-subscriber breaker) | `subscriber_id`, `subscriber_type`, `consecutive_failures` |
| `apcore.subscriber.circuit_closed` | *(new — v1.9.0, documents existing emit)* | `info` | Event delivery (per-subscriber breaker) | `subscriber_id`, `subscriber_type` |
| `apcore.event.delivery_failed` | *(new — v1.9.0, documents existing DLQ emit; see §9.16 dead-letter)* | `error` | Event bus (dead-letter path) | `event_type`, `reason`, `subscriber_id` |

> **Governance events (v1.9.0, #76/#77).** `apcore.approval.decision`, `apcore.policy.override`, and `apcore.acl.denied` make the governance chain (ACL → policy → approval) observable on the event bus. They are emitted **only** when an event emitter is configured, are best-effort side channels (execution outcome **MUST NOT** depend on their delivery), and follow the skip contract: the approval gate emits `apcore.approval.decision` only when it actually adjudicates (never on a skipped gate), and `apcore.acl.denied` is **NOT** emitted during a dry-run `validate()` preflight. See §7.9 (Execution Policy) for the policy layer that drives `apcore.policy.override`.
>
> **Collision resolution (v0.18.0):** `"module_health_changed"` was retired. Its two usages are replaced by `apcore.module.toggled` (enable/disable) and `apcore.health.recovered` (error rate recovery). `"config_changed"` was retired and split into `apcore.module.reloaded` and `apcore.config.updated`. These two legacy names were emitted as transitional aliases up to v0.17.x and were **REMOVED in v0.18.0**; implementations **MUST NOT** emit them.
>
> **Subsystem-segment correction (v0.22.0):** the registry events moved from `apcore.module.*` to `apcore.registry.*` (subsystem is the emitting module, not the affected entity), and the threshold events moved from `apcore.error.*` / `apcore.latency.*` to `apcore.health.*` (`error` and `latency` are categories, not subsystems; the emitting subsystem is the health-monitoring `PlatformNotifyMiddleware`). See [event-system.md §Legacy Aliases](../features/event-system.md#deprecation-legacy-event-names) for the full rename table.

---

## 10. Observability Specification

### 10.1 Tracing

Based on OpenTelemetry specification:

```yaml
tracing:
  # Trace context
  context:
    trace_id:
      format: "32-char lowercase hex (W3C Trace Context compatible)"
      propagation: "Must propagate in call chain"
    span_id:
      format: "16 character hexadecimal"
    parent_span_id:
      format: "16 character hexadecimal"

  # Span creation rules
  spans:
    - name: "module.execute"
      attributes: [module_id, method, duration_ms, success]

    - name: "module.validate"
      attributes: [module_id, valid, error_count]

  # Propagation methods
  propagation:
    - "Auto-propagate through context parameter"
    - "Use W3C Trace Context for HTTP calls"
```

### 10.2 Logging

```yaml
logging:
  levels: [trace, debug, info, warn, error, fatal]

  # Structured logging
  format:
    timestamp: "ISO 8601"
    level: string
    message: string
    trace_id: string
    module_id: string
    extra: object

  # Sensitive data
  sensitive_data:
    - "x-sensitive fields auto-redacted"
    - "Passwords, tokens not logged"
```

### 10.3 Metrics

```yaml
metrics:
  - name: "apcore_module_calls_total"
    type: counter
    labels: [module_id, status]

  - name: "apcore_module_duration_seconds"
    type: histogram
    labels: [module_id]

  - name: "apcore_module_errors_total"
    type: counter
    labels: [module_id, error_code]
```

### 10.4 Usage Tracking

Implementations **SHOULD** provide a `UsageCollector` that tracks per-module call statistics for the `system.usage.*` system modules:

```yaml
usage:
  storage: "in-memory"              # In-memory bucketed storage
  bucket_duration: "1h"             # Hourly buckets for trend data
  per_module:
    - call_count: counter           # Total calls
    - error_count: counter          # Total errors
    - latency_ms: histogram         # Call duration histogram
    - last_called_at: timestamp     # Last call time
  aggregate:
    - total_calls: counter
    - total_errors: counter
    - avg_latency_ms: gauge
```

A `UsageMiddleware` **SHOULD** automatically record call data into the `UsageCollector` during Step 10 (Middleware After). The collected data is consumed by `system.usage.summary` and `system.usage.module` system modules.

### 10.5 Trace ID Format

trace_id **MUST** be a 32-character lowercase hexadecimal string, aligned with the W3C Trace Context `trace-id` field for direct interoperability with distributed tracing backends (Jaeger, Tempo, Honeycomb, Datadog, OTLP) and OpenTelemetry SDKs.

```yaml
trace_id_spec:
  format: "32-char lowercase hex"      # MUST
  pattern: "^[0-9a-f]{32}$"
  example: "4bf92f3577b34da6a3ce929d0e0e4736"

  distributed:
    w3c_trace_context: "REQUIRED"
    traceparent_header: "traceparent: 00-{trace_id}-{span_id}-{flags}"

  generation:
    - "Top-level calls MUST generate a random 128-bit trace_id when no valid trace_parent is supplied"
    - "Child calls MUST inherit the parent call's trace_id unchanged"
    - "All-zero (00000000000000000000000000000000) and all-f (ffffffffffffffffffffffffffffffff) are INVALID per W3C and MUST be rejected"

  external_trace_parent_handling:
    # Applies only when Context.create receives an explicit trace_parent argument.
    - "Input matching ^[0-9a-f]{32}$ and not equal to the W3C-invalid values MUST be accepted as-is"
    - "All other inputs MUST cause the SDK to generate a fresh trace_id and SHOULD log a warning at WARN level"
    - "Implementations MUST NOT raise or reject the inbound request on invalid trace_parent — execution MUST continue with a regenerated trace_id"
    - "Implementations MUST NOT perform dashed-UUID stripping, case folding, or similar normalization at Context.create — such normalization, if needed, is the responsibility of the TraceParent header parser or the user's ContextFactory"
```

> Note: This format reflects W3C Trace Context Level 2 as of the current protocol version. Future protocol versions MAY introduce alternative formats, structured trace IDs, or additional trace-related context fields (for example, for multi-agent or non-linear trace topologies). Implementations SHOULD treat the trace_id format as a versioned contract rather than a permanent guarantee.

### 10.6 Sensitive Data Redaction

Implementations **MUST** redact fields marked as `x-sensitive` in logs and trace outputs.

```
Algorithm: redact_sensitive(data, schema)

Input:
  data   — Data object to redact
  schema — Corresponding JSON Schema (contains x-sensitive marking)

Output:
  redacted_data — Redacted data copy

Steps:
  1. redacted ← deep_copy(data)
  2. For each (field_name, field_schema) in schema.properties:
     a. If field_schema["x-sensitive"] == true:
        - If redacted[field_name] exists and is not null:
          redacted[field_name] ← "***REDACTED***"
     b. If field_schema.type == "object" and has properties:
        - Recurse: redacted[field_name] ← redact_sensitive(redacted[field_name], field_schema)
     c. If field_schema.type == "array" and items has x-sensitive:
        - Redact each element in array
  3. Return redacted

Complexity: O(n), where n is number of data fields
```

### 10.7 Sampling Strategy

Implementations **SHOULD** support the following sampling strategies:

| Strategy | Configuration Value | Description |
|------|--------|------|
| Full sampling | `sampling_rate: 1.0` | Record all calls (development environment **recommended**) |
| Proportional sampling | `sampling_rate: 0.1` | Record 10% of calls |
| Error-first | `sampling_strategy: "error_first"` | Always record error calls, successful calls by proportion |
| Off | `sampling_rate: 0.0` | Don't record trace info |

Sampling decision **MUST** be made at call chain root node, child calls **MUST** inherit parent call's sampling decision.

### 10.8 Span Naming Convention

Implementations **SHOULD** follow these Span naming conventions:

```yaml
span_naming:
  pattern: "apcore.{component}.{operation}"
  examples:
    module_execute: "apcore.module.execute"
    module_validate: "apcore.module.validate"
    acl_check: "apcore.acl.check"
    middleware_before: "apcore.middleware.before"
    middleware_after: "apcore.middleware.after"
    schema_validate: "apcore.schema.validate"
    registry_discover: "apcore.registry.discover"

  attributes:
    - "module_id"        # MUST
    - "method"           # MUST (execute|validate|describe)
    - "duration_ms"      # MUST
    - "success"          # MUST (boolean)
    - "error_code"       # SHOULD (when failed)
    - "caller_id"     # SHOULD
```

---

## 11. Extension Mechanism

### 11.1 Middleware/Interceptors

```yaml
middleware:
  order: "Onion model (first in, last out)"

  hooks:
    - name: "before"
      params: [module_id, inputs, context]
      can_modify: [inputs, context]
      can_abort: true

    - name: "after"
      params: [module_id, inputs, output, context]
      can_modify: [output]

    - name: "on_error"
      params: [module_id, inputs, error, context]
      can_retry: true
```

### 11.2 Middleware Registration and Priority

```yaml
middleware_registration:
  # Registration methods
  methods:
    # 1. Configuration file registration
    config:
      location: "apcore.yaml"
      example:
        middleware:
          - id: "logging"
            class: "apcore.middleware.LoggingMiddleware"
            priority: 100
            config:
              level: "info"

          - id: "tracing"
            class: "apcore.middleware.TracingMiddleware"
            priority: 90

          - id: "custom"
            class: "my_project.middleware.CustomMiddleware"
            priority: 50

    # 2. Code registration (runtime)
    code:
      example: |
        registry.add_middleware(
            id="custom",
            middleware=CustomMiddleware(),
            priority=50
        )

  # Priority rules
  priority:
    range: "0-1000"
    higher_first: true           # Higher number executes first
    default: 100

    # Recommended values
    recommended:
      framework: "900-1000"      # Framework built-in
      security: "800-899"        # Security-related
      logging: "700-799"         # Logging/tracing
      validation: "600-699"      # Additional validation
      custom: "0-599"            # User-defined

  # Execution order example
  execution_order:
    request: "[1000] → [900] → [800] → [100] → Module"
    response: "Module → [100] → [800] → [900] → [1000]"
```

### 11.3 Custom Extension Points

```yaml
extension_points:
  # Extension point definitions
  points:
    schema_loader:
      description: "Custom Schema loading method"
      interface: "load(module_id: str) -> Schema"
      use_case: "Load Schema from remote service/database; Binding file Schema loading"
      default: "YAMLSchemaLoader"

    id_converter:
      description: "Custom ID conversion rules"
      interface: "to_canonical(local_id, lang) -> str"
      use_case: "Support new programming language"
      default: "DefaultIDConverter"

    module_loader:
      description: "Custom module loading method"
      interface: "load(module_id: str) -> Module"
      use_case: "Remote loading, dynamic compilation; Function-based module loading; Binding file target_id resolution and module loading"
      default: "DirectoryModuleLoader"

    executor:
      description: "Custom execution method"
      interface: "execute(module, method, input, context) -> output"
      use_case: "Distributed execution, sandbox isolation"
      default: "LocalExecutor"

    acl_checker:
      description: "Custom permission checking"
      interface: "check(caller, target, action, context) -> bool"
      use_case: "Integrate external permission system"
      default: "YAMLACLChecker"

  # Extension point registration
  registration:
    config:
      location: "apcore.yaml"
      example:
        extensions:
          schema_loader: "my_project.loaders.RemoteSchemaLoader"
          executor: "my_project.executors.DistributedExecutor"

    code:
      example: |
        registry.set_extension(
            point="schema_loader",
            implementation=RemoteSchemaLoader(url="https://...")
        )

  # Extension point chaining (multiple implementations)
  chaining:
    enabled: true
    strategy: "first_success"    # first_success | all | fallback
    example:
      schema_loader:
        - "CacheSchemaLoader"    # Check cache first
        - "YAMLSchemaLoader"     # Load from file if cache miss
```

> **NOTE — Implementation Extension Point Names:**
> The theoretical extension point names above (`schema_loader`, `id_converter`, `module_loader`, `executor`, `acl_checker`) reflect the original design-time taxonomy. Current SDK implementations (Python and TypeScript) use a different set of five built-in extension point names that map to runtime needs:
>
> | Spec (Theoretical) | Implementation (Actual) | Rationale |
> |---------------------|--------------------------|-----------|
> | `schema_loader`     | `discoverer`             | Unified discovery replaces separate schema/module loading |
> | `module_loader`     | `module_validator`       | Validation is the primary customization need at load time |
> | `acl_checker`       | `acl`                    | Shortened for ergonomic API use |
> | *(no equivalent)*   | `middleware`             | First-class middleware extension point added for runtime pipeline customization |
> | *(no equivalent)*   | `span_exporter`          | Observability export as a dedicated extension point |
> | `id_converter`      | *(not yet implemented)*  | Deferred; not a common runtime customization need |
> | `executor`          | *(not yet implemented)*  | Deferred; local execution covers current use cases |
>
> Implementations declaring Level 2 conformance use the actual names (`discoverer`, `middleware`, `acl`, `span_exporter`, `module_validator`, `approval_handler`) in `ExtensionManager`.

### 11.4 Framework Built-in Middleware

```yaml
builtin_middleware:
  # Must enable (cannot disable)
  required:
    - id: "schema_validation"
      description: "Input/output Schema validation"
      priority: 1000

    - id: "acl_check"
      description: "Permission check"
      priority: 999

  # Default enabled (can disable)
  default_enabled:
    - id: "tracing"
      description: "Trace context propagation"
      priority: 950

    - id: "logging"
      description: "Call logging"
      priority: 900

    - id: "metrics"
      description: "Metrics collection"
      priority: 890

    - id: "error_wrapper"
      description: "Error wrapping and formatting"
      priority: 800

  # Disable built-in middleware
  disable:
    config:
      middleware:
        disabled:
          - "metrics"            # Disable metrics collection
```

### 11.5 Middleware Execution State Machine

Middleware chain execution **MUST** follow this state machine:

```
                                     Error branch
  ┌──────┐    ┌────────┐    ┌─────────┐    ┌──────┐    ┌──────┐
  │ init │───▶│ before │───▶│ execute │───▶│ after│───▶│ done │
  └──────┘    └───┬────┘    └────┬────┘    └──┬───┘    └──────┘
                  │              │             │
                  │ abort        │ error       │ error
                  ▼              ▼             ▼
              ┌──────┐    ┌──────────┐    ┌──────┐
              │ done │    │ on_error │───▶│ done │
              └──────┘    └──────────┘    └──────┘

State descriptions:
  init     — Initialize middleware chain, prepare execution context
  before   — Execute all middleware before() in order
  execute  — Execute module's execute() method
  after    — Execute all middleware after() in reverse order
  on_error — Execute all middleware on_error() in reverse order
  done     — Execution complete, return result or error

Rules:
  - before phase: If middleware returns non-None → Use as replacement input
  - before phase: If middleware throws exception → Skip subsequent before and execute, enter on_error
  - after phase: If middleware returns non-None → Use as replacement output
  - on_error phase: If middleware returns non-None → Use as fallback result, stop error propagation
```

### 11.6 Extension Point Interface Formalization

Implementations **SHOULD** support the following extension points. Each supported extension point **MUST** define a clear interface contract:

```
Extension Point: SchemaLoader
  load(module_id: String) → Schema
  supports(module_id: String) → Boolean
  priority: Integer

Extension Point: ModuleLoader
  load(module_id: String) → Module
  supports(file_path: String) → Boolean
  priority: Integer

Extension Point: IDConverter
  to_canonical(local_id: String, language: String) → String
  from_canonical(canonical_id: String, language: String) → String

Extension Point: ACLChecker
  check(caller_id: String, target_id: String, context: Context) → Boolean

Extension Point: Executor
  execute(module: Module, method: String, inputs: Map, context: Context) → Map
```

> **NOTE:** The interface contracts above use the original theoretical names. See the mapping table in §11.3 for the actual extension point names used in SDK implementations (`discoverer`, `middleware`, `acl`, `span_exporter`, `module_validator`, `approval_handler`).

### 11.7 Extension Loading Order

Implementations **MUST** load extensions according to the following algorithm:

```
Algorithm: load_extensions(config, extension_points)

Steps:
  1. Sort each extension point's implementations by priority descending
  2. For each extension point:
     a. If strategy == "first_success": Try in order, first success takes effect
     b. If strategy == "all": Execute all implementations, merge results
     c. If strategy == "fallback": Try in order, try next on failure
  3. If extension point has no available implementation → Use framework default implementation
```

### 11.8 Edge Case Handling

Implementations **MUST** handle middleware edge cases according to the following table:

#### 11.8.1 on_error Cascade

| Scenario | Behavior | Level |
|------|------|------|
| `on_error()` itself throws exception | Log ERROR, continue to next `on_error()` in chain | **MUST** |
| `on_error()` returns non-`None` value | Stop propagation, use return value as module final output | **MUST** |
| `on_error()` returns `None` | Continue propagating error downward | **MUST** |
| All `on_error()` return `None` | Throw original error to caller_id | **MUST** |

#### 11.8.2 before() Edges

| Scenario | Behavior | Level |
|------|------|------|
| `before()` returns `None` | Keep `inputs` unchanged, continue chain | **MUST** |
| `before()` returns partial field dict | Replace `inputs` entirely | **MUST** |
| `before()` returns non-dict type | Throw `GENERAL_INTERNAL_ERROR` | **MUST** |
| `before()` throws `ModuleError` | Trigger `on_error()` chain, skip module execution | **MUST** |
| `before()` modifies `context.data` | Allowed, modifications visible to subsequent middleware and module | **MUST** |

#### 11.8.3 after() Edges

| Scenario | Behavior | Level |
|------|------|------|
| `after()` returns `None` | Keep `result` unchanged, continue chain | **MUST** |
| `after()` returns partial field dict | Replace `result` entirely | **MUST** |
| `after()` throws `ModuleError` | Trigger `on_error()` chain, replace original result | **MUST** |
| `after()` returns value not matching `output_schema` | Trigger `SCHEMA_VALIDATION_ERROR` | **MUST** |

#### 11.8.4 Timeout Related

| Scenario | Behavior | Level |
|------|------|------|
| Timeout occurs in `before()` phase | Throw `MODULE_TIMEOUT`, trigger `on_error()` chain | **MUST** |
| Timeout occurs in `execute()` phase | Throw `MODULE_TIMEOUT`, trigger `on_error()` chain | **MUST** |
| Timeout occurs in `after()` phase | Throw `MODULE_TIMEOUT`, trigger `on_error()` chain | **MUST** |
| `on_error()` handling timeout itself times out | Log ERROR, stop `on_error()` chain, throw original `MODULE_TIMEOUT` | **MUST** |

**Note**:
- Timeout timer should start at first `before()` call
- Timeout enforcement algorithm see §12.7.5 and algorithms.md A22

---

## 12. SDK Implementation Guide

### 12.1 Required Core Components

| Component | Responsibility | Phase |
|------|------|------|
| `IDConverter` | Canonical ID ↔ Local ID conversion (class or utility function) | Phase 1 |
| `SchemaLoader` | Load YAML Schema | Phase 1 |
| `Registry` | Module discovery, registration, loading | Phase 1 |
| `Executor` | Module invocation, Schema validation | Phase 1 |
| `ACLChecker` | Permission checking | Phase 2 |
| `MiddlewareManager` | Middleware management | Phase 2 |
| `TracingProvider` | Trace context | Phase 2 |
| `MetricsCollector` | Metrics collection | Phase 2 |

### 12.2 Core Component Interface Contracts

Following are formalized interface definitions for each core component (language-agnostic pseudocode). All SDK implementations **MUST** provide equivalent implementations of these interfaces.

```
Interface: IDConverter
  /**
   * Convert language-native ID to Canonical ID
   * @param local_id  — Native format ID (e.g., "executor::validator::db_params")
   * @param language  — Source language identifier (python|rust|go|java|typescript)
   * @return canonical_id — Dot-separated snake_case format
   * @throws INVALID_ID — If local_id doesn't conform to language naming convention
   */
  to_canonical(local_id: String, language: String) → String

  /**
   * Convert Canonical ID to language-native ID
   * @param canonical_id — Dot-separated snake_case format
   * @param language     — Target language identifier
   * @return local_id    — Language-native format
   */
  from_canonical(canonical_id: String, language: String) → String

  /**
   * Implementation Note: SDKs MAY implement IDConverter as a standalone
   * utility function (e.g., normalize_to_canonical_id()) rather than a
   * class, provided the same conversion semantics are satisfied.
   */

Interface: SchemaLoader
  /**
   * Load specified module's Schema definition
   * @param module_id — Canonical ID
   * @return schema   — Parsed Schema object (contains input_schema, output_schema, description)
   * @throws SCHEMA_NOT_FOUND — If Schema file doesn't exist
   * @throws SCHEMA_INVALID   — If Schema format is invalid
   */
  load(module_id: String) → Schema

  /**
   * Validate Schema itself for validity
   * @param schema — Schema object to validate
   * @return errors — Error list, empty list means valid
   */
  validate(schema: Schema) → List<ValidationError>

Interface: Registry
  /**
   * Register a module under a canonical ID.
   *
   * `version` and `metadata` are OPTIONAL parameters an implementation MAY
   * accept; §5.4 governs multi-version coexistence, and resolving BY version
   * remains optional. Accepting the parameters and resolving by version are
   * separate capabilities — see the SDK status table in
   * features/registry-system.md.
   *
   * When `metadata` is accepted, a `dependencies` entry — a list of
   * {module_id, version?, optional?} objects — MUST reach the registered
   * module's descriptor, so that `get_definition(module_id).dependencies`
   * returns what the caller declared. Reload ordering reads that accessor: an
   * implementation that parses `dependencies` for load-time sorting but drops
   * it from the descriptor degrades a dependency-ordered reload to its sort's
   * seed order, which is usually alphabetical and therefore plausible enough
   * to go unreported.
   *
   * The ordered side effects, the in-flight reservation and the visibility
   * rule are specified in features/registry-system.md
   * § "Contract: Registry.register".
   *
   * @param module_id — Canonical ID
   * @param module    — Module instance
   * @param version   — Optional declared version (§5.4)
   * @param metadata  — Optional metadata map; `dependencies` MUST survive
   * @throws INVALID_ID          — If module_id fails validation
   * @throws DUPLICATE_MODULE_ID — If module_id is already registered
   */
  register(module_id: String, module: Module, version: String?, metadata: Map?) → void

  /**
   * Scan extension directory, discover and register all modules
   * @param config — Framework configuration
   * @throws EXTENSION_ROOT_NOT_FOUND — If extension root directory doesn't exist
   */
  discover(config: Config) → void

  /**
   * Get specified module
   * @param module_id — Canonical ID
   * @return module   — Module instance
   * @throws MODULE_NOT_FOUND — If module not registered
   */
  get(module_id: String) → Module

  /**
   * List all registered module IDs
   * @return ids — Canonical ID list
   */
  list() → List<String>

  /**
   * Get module description info (for AI/LLM use)
   * @param module_id — Canonical ID
   * @return description — Complete description including Schema, Annotations, Examples
   */
  describe(module_id: String) → ModuleDescription

Interface: Executor
  /**
   * Call a module through the execution pipeline.
   *
   * Note: In implementations, this is exposed as `call()` (sync) and
   * `call_async()` (async). The separate `execute(module_id, method, ...)`
   * signature is folded into `call()` — the executor always runs the
   * module's `execute()` method through the full pipeline.
   *
   * `context` MUST be bound to at most one Executor. When `context.executor`
   * is non-null and refers to a DIFFERENT Executor instance, the call MUST
   * raise `CONTEXT_BINDING_ERROR`. Accepting the rebind silently was permitted
   * as a documented deviation through v1.10.0; no SDK took it — apcore-python
   * `context.py:152`, apcore-typescript `context.ts:187` and apcore-rust
   * `context.rs:765` all raise — and the alternative made the behaviour
   * unassertable, since a conformance case cannot state two legal outcomes
   * without each driver deciding which one applies to it (apcore#92).
   *
   * @param module_id — Canonical ID
   * @param inputs    — Input parameters (conform to input_schema)
   * @param context   — Execution context, bound to at most one Executor
   * @return output   — Output result (conform to output_schema)
   * @throws INPUT_VALIDATION_FAILED  — Input validation failed
   * @throws OUTPUT_VALIDATION_FAILED — Output validation failed
   * @throws ACL_DENIED               — Permission denied
   * @throws MODULE_EXECUTION_ERROR   — Module execution exception
   */
  call(module_id: String, inputs: Map, context: Context) → Map

  /**
   * [SHOULD] Non-destructive preflight check that runs Steps 1–5 and Step 7
   * of the execution pipeline (skipping Step 6 Middleware Before Chain),
   * plus an optional module-level preflight (Check 7), without invoking
   * module code or middleware. See §12.8 for the language-specific guide.
   *
   * Runs: context creation (Step 1), call chain guard (Step 2), module
   * lookup (Step 3), ACL enforcement (Step 4), approval detection (Step 5,
   * report only — MUST NOT invoke ApprovalHandler), input schema validation
   * (Step 7), and — only when the ACL permitted the call — module.preflight()
   * and module.preview() for advisory warnings and predicted changes
   * (§12.8.5.1).
   *
   * MUST NOT: execute module code, run middleware, or modify external state.
   *
   * All check failures are collected into the result rather than thrown,
   * so the caller_id can see every problem in a single round-trip.
   *
   * @param module_id — Canonical ID
   * @param inputs    — Input parameters to validate
   * @param context   — Optional execution context (for call-chain checks)
   * @return result   — PreflightResult with per-check status
   */
  validate(module_id: String, inputs: Map, context: Context?) → PreflightResult

/**
 * Result of Executor.validate() preflight check.
 *
 * PreflightResult SHOULD be duck-type compatible with ValidationResult
 * (i.e., it has `valid: Boolean` and `errors: List`), so that existing
 * consumers of validate() continue to work after the enhancement.
 */
Type: PreflightCheckResult
  check: String              // "module_id" | "module_lookup" | "call_chain" | "acl" | "approval" | "schema" | "module_preflight" | "module_preview"
  passed: Boolean
  error: Map?                // Error details when passed=false; null when passed=true
  warnings: List<String>     // Non-fatal advisory messages (default: empty list)

Type: Change
  action: String             // Free-form verb describing the kind of change (e.g. "write", "delete", "send", "charge")
  target: String             // Free-form identifier of what is changed (e.g. "users.42", "stripe:charge:ch_abc")
  summary: String            // Human-readable single-line summary (REQUIRED — floor for destructive modules)
  before: Any?               // Optional snapshot of prior state; OMIT for unobservable side effects
  after: Any?                // Optional predicted new state; OMIT when unknown (e.g. server-assigned IDs)
  // x-* extension fields permitted (consistent with §4.6 conventions). See ./rfc-preview-method.md
  // for cross-SDK schema-encoding guidance (pydantic / serde-flatten / TypeBox Type.Unsafe).

Type: PreviewResult
  changes: List<Change>      // Module's prediction of what would change if the call were executed

Type: PreflightResult
  valid: Boolean             // True only if ALL checks passed
  checks: List<PreflightCheckResult>
  requires_approval: Boolean // True if module has requires_approval annotation
  predicted_changes: List<Change>  // Populated when module implements preview() and Executor.validate() ran in dry_run mode (default: empty list).
                                   // ALWAYS empty when the acl check failed — §12.8.5.1

Interface: ACLChecker
  /**
   * Check invocation permission
   * @param caller_id — Caller module ID or identity
   * @param target_id — Target module ID
   * @param context   — Execution context
   * @return allowed  — Whether allowed
   */
  check(caller_id: String, target_id: String, context: Context) → Boolean

Interface: MiddlewareManager
  /**
   * Register middleware
   * @param middleware  — Middleware instance
   *
   * Note: Priority is explicit (0-1000, higher executes first), as defined
   * in §11.2. Registration order is used only as a tiebreaker when two
   * middleware have equal priority. Execution follows the onion model.
   */
  add(middleware: Middleware) → void

  /**
   * Execute middleware chain in priority order.
   *
   * SDKs MAY implement this as a single run_chain() method or as
   * separate phase methods (execute_before, execute_after, execute_on_error)
   * called by the Executor at the appropriate pipeline steps. Both
   * approaches are conformant provided the onion-model execution order
   * is preserved.
   */
  run_chain(module_id: String, inputs: Map, context: Context, next: Function) → Map

Interface: TracingProvider
  /**
   * Create new Span
   * @param name       — Span name (follows §10.8 naming convention)
   * @param context    — Execution context (contains trace_id, parent_span_id)
   * @return span      — Span object
   */
  start_span(name: String, context: Context) → Span

  /**
   * End Span and record result
   * @param span       — Span to end
   * @param attributes — Additional attributes
   */
  end_span(span: Span, attributes: Map) → void

Interface: MetricsCollector
  /**
   * Record counter metric
   * @param name   — Metric name
   * @param labels — Label key-value pairs
   * @param value  — Increment value (default 1)
   */
  increment(name: String, labels: Map, value: Integer) → void

  /**
   * Record histogram metric
   * @param name   — Metric name
   * @param labels — Label key-value pairs
   * @param value  — Observed value
   */
  observe(name: String, labels: Map, value: Float) → void
```

#### Cross-Language Naming Conventions

The protocol specification uses `snake_case` for canonical definitions. Each language SDK **MUST** translate to its native naming convention:

| Protocol (canonical) | TypeScript | Python | Go | Rust |
|---------------------|------------|--------|-----|------|
| `module_id` | `moduleId` | `module_id` | `ModuleId` | `module_id` |
| `input_schema` | `inputSchema` | `input_schema` | `InputSchema` | `input_schema` |
| `output_schema` | `outputSchema` | `output_schema` | `OutputSchema` | `output_schema` |
| `get_definition()` | `getDefinition()` | `get_definition()` | `GetDefinition()` | `get_definition()` |
| `call_async()` | `callAsync()` | `call_async()` | `CallAsync()` | `call_async()` |
| `requires_approval` | `requiresApproval` | `requires_approval` | `RequiresApproval` | `requires_approval` |
| `open_world` | `openWorld` | `open_world` | `OpenWorld` | `open_world` |

**Rule:** Bridge/adapter packages (e.g., apcore-mcp-typescript) **MUST** use the same naming conventions as their language's core SDK. A TypeScript MCP bridge **MUST** use camelCase to match apcore-typescript, not snake_case from the protocol spec.

#### Standard Registry Event Names

Registry implementations **MUST** support exactly two standard events:

| Event Name | Triggered | Callback Signature |
|-----------|-----------|-------------------|
| `"register"` | After module successfully registered | `(module_id, module) -> None` |
| `"unregister"` | Before module is removed | `(module_id, module) -> None` |

All SDKs **MUST** export these event names as named constants (e.g., TypeScript: `REGISTRY_EVENTS.REGISTER`, Python: `REGISTRY_EVENTS["REGISTER"]`). Consumers **MUST NOT** hardcode event name strings.

#### Error Code Constants Export Requirement

All SDKs **MUST** export the framework error codes defined in Section 8 as enumerated constants. This prevents magic string dependencies and enables IDE autocomplete.

Example (TypeScript):
```typescript
export const ErrorCodes = {
  MODULE_NOT_FOUND: "MODULE_NOT_FOUND",
  SCHEMA_VALIDATION_ERROR: "SCHEMA_VALIDATION_ERROR",
  // ... all codes from Section 8
} as const;
```

#### Context Factory Protocol

For web framework integrations (Django, Flask, FastAPI, NestJS, Express), SDKs **SHOULD** provide a `ContextFactory` protocol:

```
Protocol ContextFactory:
    create_context(request: Any) -> Context
```

This enables framework-specific context creation (e.g., extracting Identity from Django `request.user`, JWT tokens, or API keys) without coupling apcore core to any web framework.

The lifecycle is: request arrives → ContextFactory.create_context(request) → Executor.call(module_id, inputs, context) → response.

#### Streaming Execution Protocol

Modules MAY support incremental output by implementing a `stream` method alongside the required `execute` method:

```
stream(inputs, context) → AsyncIterable<Record>
```

**Semantics:**

- Each yielded record is a partial result chunk; the framework does not prescribe chunk structure.
- The complete result is the **recursive deep merge** of all yielded chunks. Implementations **MUST** recurse into nested objects and **MUST** replace (not concatenate) arrays at matching keys. Recursion depth **MUST** be capped to prevent stack exhaustion via adversarial chunk shapes; the canonical default is 32. See `./algorithms.md` §A24 `deep_merge_chunks` for pseudocode and `../../conformance/fixtures/stream_aggregation.json` for the 9 cross-language test cases.
- `execute()` MUST remain implemented as the non-streaming fallback.
- Module descriptors SHOULD declare `annotations.streaming = true` when `stream()` is provided.

**Executor.stream() pipeline:**

1. Steps 1–7 identical to `call()`: context creation, call chain guard, module lookup, ACL, approval gate, before-middleware, input validation.
2. If module lacks `stream()`: fall back to `call()`, yield single chunk, return.
3. Iterate `module.stream(inputs, context)`, yield each chunk to caller.
4. After all chunks: validate accumulated output against `output_schema`, run after-middleware on accumulated result.

**Cross-language signatures:**

| Language   | Executor method signature                                                          |
|------------|------------------------------------------------------------------------------------|
| TypeScript | `async *stream(moduleId, inputs?, context?): AsyncGenerator<Record<string, unknown>>` |
| Python     | `async def stream(module_id, inputs?, context?) -> AsyncIterator[dict[str, Any]]`  |

**MCP bridge behavior:**

When bridging `Executor.stream()` to MCP, implementations SHOULD use the standard `notifications/progress` mechanism:

1. Client includes `_meta.progressToken` in the `tools/call` request to opt into streaming.
2. Server calls `Executor.stream()` and for each yielded chunk, sends `notifications/progress` with `message` containing the JSON-serialized chunk.
3. The final `CallToolResult` contains the complete accumulated result.
4. If the client does not provide `progressToken`, the bridge accumulates internally and returns an atomic result.

### 12.3 Cross-language Implementation Requirements

All SDK implementations **MUST** satisfy the following requirements regardless of language:

| Requirement | Level |
|------|--------|
| JSON Schema Draft 2020-12 validation | MUST |
| YAML parsing | MUST |
| Directory as ID | MUST |
| ID Map cross-language conversion | MUST |
| ACL engine | MUST |
| Middleware onion model | MUST |
| Structured logging | MUST |
| Error code specification | MUST |
| OpenTelemetry integration | SHOULD |
| Executor.validate() preflight | SHOULD |

### 12.4 Consistency Testing Requirements

Each SDK implementation **MUST** pass the following consistency test suite to ensure cross-language behavior consistency:

```
Consistency Test Suite:

1. ID Conversion Tests:
   - Directory path → Canonical ID (10+ cases, including edge cases)
   - Cross-language ID conversion (5+ cases per language)
   - Invalid ID detection (format errors, too long, reserved words)

2. Schema Validation Tests:
   - Valid input passes validation
   - Invalid input rejected (type error, missing field, extra field)
   - x-sensitive field marking recognition
   - $ref reference resolution (including circular reference detection)

3. ACL Tests:
   - Wildcard matching (*, **)
   - deny takes precedence over allow
   - Default policy takes effect
   - Identity type matching (user, service, agent, api_key, system)

4. Middleware Tests:
   - Onion model execution order
   - before phase modifying input
   - after phase modifying output
   - on_error phase fallback handling
   - Priority sorting correctness

5. Executor Tests:
   - Normal execution flow (input validation → ACL → middleware → execute → output validation)
   - Error propagation and error codes
   - Context propagation (trace_id, call_chain)
   - Circular call detection
   - Call depth limiting

6. Observability Tests:
   - trace_id is valid 32-char lowercase hex
   - Sensitive data redaction
   - Structured log format

7. Preflight (validate) Tests:
   - Valid module + valid inputs → PreflightResult.valid=true, all checks passed
   - Invalid module_id format → module_id check failed, early return
   - Unknown module → module_lookup check failed, early return
   - ACL denial → acl check failed, valid=false
   - ACL denial → NO module_preflight / module_preview check, predicted_changes empty (§12.8.5.1)
   - Module with requires_approval → requiresApproval=true, approval check still passed
   - Schema validation failure → schema check failed with error details
   - PreflightResult.errors matches filtered failed checks (duck-type ValidationResult)
   - validate() MUST NOT execute module code or run middleware
```

### 12.5 Implementation Roadmap

```
Phase 1: Core MVP
├── IDMap (Directory as ID + cross-language conversion)
├── SchemaLoader (YAML → language-native Schema)
├── Registry (directory scanning, registration)
└── Executor (invocation, validation)

Phase 2: Core Complete
├── ACLChecker (permission checking)
├── MiddlewareManager (middleware)
├── ErrorHandler (error handling)
└── ObservabilityProvider (tracing, logging, metrics)

Phase 3: CLI & DX
├── CLI tools (init, create, run)
├── Schema validation tools
└── Developer documentation

Phase 4: Advanced
├── Module hot reloading
├── OpenTelemetry integration
└── Performance optimization
```

### 12.6 Language-specific Guidelines

Each SDK implementation **SHOULD** use the idiomatic schema validation, async model, and package management conventions of its target_id language. Specific library choices are documented in each SDK's own repository.

### 12.7 Concurrency Model Specification

This section defines apcore's concurrency model and thread safety requirements, ensuring SDK implementers correctly implement the framework in multi-threaded/coroutine environments.

#### 12.7.1 Module Instance Lifecycle

**Singleton Model (MUST)**:

- Each `module_id` **MUST** correspond to unique module instance (singleton)
- Instance created at `discover()` or first invocation, destroyed at `unregister()` or app shutdown
- `on_load()` hook **MUST** be called only once during instance lifecycle

**Reentrancy (MUST)**:

- `execute()` method **MUST** support concurrent reentrant calls (thread-safe)
- Module internal state (if any) **SHOULD** use thread-safe mechanisms (locks, atomic variables, etc.) for protection
- Implementations **MUST NOT** assume `execute()` calls are serial

**Example — Thread-safe counter module:**

```python
# Python example (similar for other languages)
import threading

class CounterModule:
    def __init__(self):
        self._count = 0
        self._lock = threading.Lock()

    def on_load(self, context):
        # Called only once
        print("Counter module loaded")

    def execute(self, inputs, context):
        # Multi-thread safe
        with self._lock:
            self._count += 1
            return {"count": self._count}
```

**Lifecycle Guarantees:**

| Phase | Call Count | Concurrency | Description |
|------|---------|--------|------|
| `__init__()` | 1 time | Single-thread | Instance creation |
| `on_load()` | 1 time | Single-thread | Module initialization |
| `on_resume(state)` | 0..1 time | Single-thread | Restore state from previous instance (hot-reload only) |
| `execute()` | 0..N times | **Multi-thread** | Business logic |
| `on_suspend()` | 0..1 time | Single-thread | Export state before hot-reload |
| `on_unload()` | 0..1 time | Single-thread | Resource cleanup |

> **Note:** During hot-reload, `on_suspend()` and `on_unload()` run on the **old** instance;
> `on_resume(state)` runs on the **new** instance after its `on_load()`.
> Full sequence: `old.on_suspend() → old.on_unload() → new.__init__() → new.on_load() → new.on_resume(state)`.

#### 12.7.2 Context.data Sharing Semantics

**Reference Sharing (MUST)**:

- `context.data` **MUST** be the same dict/Map object across entire call chain (reference sharing)
- Parent module modifications to `context.data` visible to child modules, vice versa
- When `child()` creates new Context, `data` field **MUST** copy reference (not deep copy)

**Isolation (MUST)**:

- Different top-level `call()` invocations **MUST** use independent `context.data` instances
- Concurrently executing call chains **MUST NOT** share `context.data` (avoid race conditions)

**Example — Context.data Sharing:**

```python
# Top-level calls
context1 = Context(data={})
executor.call("module_a", {}, context1)  # context1.data independent

context2 = Context(data={})
executor.call("module_b", {}, context2)  # context2.data independent

# Sharing within call chain
# module_a.execute():
context.data["key"] = "value"  # Write
sub_context = context.child("module_c")
executor.call("module_c", {}, sub_context)

# module_c.execute():
print(context.data["key"])  # Reads "value" (reference sharing)
```

**Concurrent Access Protection (SHOULD)**:

- If `context.data` might be accessed by multiple threads (e.g., async middleware), **SHOULD** use thread-safe Map implementation
- Python's `dict` is partially thread-safe in CPython (GIL protected), but **SHOULD** avoid relying on implementation details

#### 12.7.3 Hot Reload with State Migration

**State Migration Hooks (MAY)**:

Modules that hold in-memory state (counters, caches, connection pools) can implement `on_suspend()` / `on_resume()` to preserve state across hot-reload cycles:

```
Hot-Reload Sequence:
  1. old_instance.on_suspend() → state (dict or null)
  2. old_instance.on_unload()
  3. (reload module code from disk)
  4. new_instance.__init__()
  5. new_instance.on_load()
  6. If state is not null: new_instance.on_resume(state)
```

**Constraints:**
- `on_suspend()` return value **MUST** be JSON-serializable (no functions, connections, file handles)
- `on_resume()` **MUST** tolerate missing or extra keys (new version may have different state shape)
- If `on_suspend()` raises, log ERROR and proceed with unload (state is lost)
- If `on_resume()` raises, log ERROR and continue (module starts with fresh state)
- Framework **MUST NOT** call `on_resume()` if `on_suspend()` returned null or was not implemented

```python
# Example: Counter module with state preservation
class CounterModule(Module):
    def on_load(self):
        self._count = 0
        self._cache = {}

    def on_suspend(self) -> dict | None:
        """Export state before hot-reload"""
        return {"count": self._count, "cache": self._cache}

    def on_resume(self, state: dict) -> None:
        """Restore state after hot-reload"""
        self._count = state.get("count", 0)
        self._cache = state.get("cache", {})

    def execute(self, inputs, context):
        self._count += 1
        return {"count": self._count}
```

#### 12.7.4 Hot Reload Race Conditions

**Problem**: During `unregister()`, module might be executing in other threads.

**Safe Unload Algorithm (MUST)**:

```
Algorithm: safe_unregister(module_id, registry)

Steps:
  1. Mark module as "unloading" state
  2. Remove from registry (new call() will throw MODULE_NOT_FOUND)
  3. Wait for all executing calls to complete:
     - Maintain reference count (number of executing calls)
     - Block until count reaches zero or timeout (default 5 seconds)
  4. Call on_suspend() hook (if implemented) → save returned state
  5. Call on_unload() hook
  6. Release module instance

Return:
  - If successfully unloaded → true, state (dict or null)
  - If timeout → Log ERROR, force unload, return false, null
```

**For detailed algorithm see algorithms.md A21 — safe_unregister()**

**Operation Concurrency Table:**

| Operation A | Operation B | Behavior | Description |
|--------|--------|------|------|
| `call()` | `unregister()` | If A already started execution, continue to completion; if not started, throw `MODULE_NOT_FOUND` | **MUST** |
| `register()` | `register()` same ID | Latter throws `GENERAL_INVALID_INPUT` | **MUST** |
| `unregister()` | `unregister()` same ID | Idempotent, succeed silently | **MUST** |
| `get()` | `unregister()` | If get executes first, return instance; if unregister executes first, throw `MODULE_NOT_FOUND` | **SHOULD** |

#### 12.7.5 Timeout Enforcement

**Cooperative Cancellation (SHOULD)**:

- SDK **SHOULD** prefer cooperative cancellation mechanism (e.g., Python `asyncio.CancelledError`, Go `context.Context`)
- Module **SHOULD** check cancellation signal and actively exit

**Forced Termination (MAY)**:

- If module doesn't respond to cancellation signal, SDK **may** forcibly terminate execution thread/coroutine
- After forced termination **MUST** log ERROR including `module_id` and timeout duration

**Timeout Enforcement Algorithm (MUST)**:

```
Algorithm: enforce_timeout(module_id, inputs, context, timeout_ms)

Steps:
  1. Start timer (from first before() middleware)
  2. Concurrent execution:
     a. Main task: execute_with_middleware(module_id, inputs, context)
     b. Timeout monitor: sleep(timeout_ms)
  3. If main task completes first → Cancel timer, return result
  4. If timeout triggers first:
     - Send cancellation signal (cooperative)
     - Wait maximum grace_period (default 5 seconds)
     - If still hasn't exited → Forcibly terminate (if supported)
     - Throw MODULE_TIMEOUT error

Return:
  - Success → Module output
  - Failure → MODULE_TIMEOUT error
```

**For detailed algorithm see algorithms.md A22 — enforce_timeout()**

**Timeout Levels:**

| Level | Scope | Default | Description |
|------|------|--------|------|
| Per-module timeout | Individual module execution | 30000ms | Configuration item `executor.default_timeout` |
| Global timeout | before + execute + after (entire call chain) | 60000ms | Configuration item `executor.global_timeout` |
| ACL check timeout | ACL rule evaluation | 1000ms | Separate timing |
| Schema validation timeout | Input/output validation | Included in global timeout | Not separately timed |

#### 12.7.6 Middleware Chain Atomicity

**Call-level Isolation (MUST)**:

- Each `call()`'s middleware chain execution **MUST** not interleave with other `call()`'s chain execution
- Middleware chain's before → execute → after sequence **MUST** complete atomically (not interrupted by other calls)

**Instance Sharing (MUST)**:

- Middleware instances shared at application level (similar to module singleton)
- Middleware **MUST** be thread-safe, support concurrent calls

**Example — Middleware Concurrent Execution:**

```
Thread 1: before1 → before2 → execute(A) → after2 → after1
Thread 2:                before1 → before2 → execute(B) → after2 → after1
                       ↑ Interleaving allowed (different calls)

# Forbidden:
Thread 1: before1 → before2 → [interrupted by Thread 2]
Thread 2:                      before1 → ...
```

**State Isolation (SHOULD)**:

- Middleware **SHOULD** store call-level state through `context.data` (e.g., request ID, timer)
- **MUST NOT** use middleware instance variables to store call-level state (causes race conditions)

#### 12.7.7 Sync/Async Mixing

**Bridging Strategy (MUST)**:

Implementations **MUST** support mixed calls of sync and async modules, bridging according to these rules:

| Caller | Called Module | Bridging Strategy | Description |
|--------|-----------|---------|------|
| Sync | Sync | Direct call | No overhead |
| Sync | Async | Block and wait (await) | Sync caller_id blocks until async completes |
| Async | Sync | Thread pool offload | Avoid blocking event loop |
| Async | Async | Direct await | No overhead |

**Language Mapping:**

| Language | Sync Model | Async Model | Bridging Mechanism |
|------|---------|---------|---------|
| Python | Regular function | `async def` | `asyncio.run()` / `run_in_executor()` |
| JavaScript | Blocking code (rare) | Promise / async/await | Direct await (JS default async) |
| Rust | Regular function | `async fn` | `block_on()` / `spawn_blocking()` |
| Go | goroutine | goroutine + channel | Naturally supported (goroutines lightweight) |
| Java | Thread | CompletableFuture | `join()` / `supplyAsync()` |

**Performance Considerations:**

- Sync→Async bridging blocks caller_id thread, **SHOULD** avoid frequent use in async contexts
- Async→Sync bridging needs thread pool, **SHOULD** configure reasonable thread pool size (default CPU cores × 2)

#### 12.7.8 Resource Cleanup Guarantees

Implementations **MUST** guarantee resource cleanup according to this table:

| Exit Scenario | `on_unload()` Called | `finally` Block Executed | File/Network Closed | Memory Released | Level |
|---------|-------------------|-----------------|--------------|---------|------|
| Normal completion | ✅ Called | ✅ Executed | ✅ Guaranteed | ✅ GC reclaimed | **MUST** |
| Timeout (cooperative cancel) | ✅ Called | ✅ Executed | ✅ Guaranteed | ✅ GC reclaimed | **MUST** |
| Timeout (forced termination) | ⚠️ May not call | ⚠️ May not execute | ⚠️ May leak | ✅ GC reclaimed (eventually) | **MAY** |
| Process crash/kill | ❌ Not called | ❌ Not executed | ❌ Leak | ❌ Lost | N/A |

**Best Practices:**

- Modules **SHOULD** use RAII (Resource Acquisition Is Initialization) pattern to manage resources
- **SHOULD** acquire resources in `on_load()`, release in `on_unload()`
- **SHOULD** avoid opening long-term resources in `execute()` (e.g., database connections), use connection pools instead

**Example — Resource Cleanup:**

```python
class DatabaseModule:
    def on_load(self, context):
        self.pool = create_connection_pool()  # Long-term resource

    def execute(self, inputs, context):
        with self.pool.get_connection() as conn:  # Short-term resource, auto-released
            return conn.query(inputs["sql"])

    def on_unload(self):
        self.pool.close()  # Cleanup long-term resource
```

### 12.8 Executor.validate() Cross-Language Implementation Guide

The `validate()` preflight method (§12.2, SHOULD level) runs Steps 1–5 and Step 7 of the Executor pipeline
(plus optional module-level preflight Check 7) without executing module code or middleware. This section provides language-specific guidance for
SDK implementers.

#### 12.8.1 Design Principles

1. **Collect, don't throw.** All check failures are appended to a `checks` list. The caller_id sees every problem in one call.
2. **Early return only when subsequent checks are meaningless.** module_id format failure or module-not-found justifies early return because later checks require a valid module reference.
3. **Reuse existing internals.** validate() calls the same helper functions used by the `call()` pipeline (regex check, registry lookup, ACL check, schema validation). No new capabilities are required.
4. **Duck-type backward compatibility.** PreflightResult SHOULD expose `.valid` (Boolean) and `.errors` (List) so existing consumers of the old ValidationResult continue to work.
5. **Authorization gates disclosure.** A failed `acl` check does not stop the checks the Executor computes on its own, but it **MUST** stop module-level introspection — see §12.8.5.1. `validate()` is a preflight, not a way around the ACL.

#### 12.8.2 Error Handling Mapping

Each check in validate() calls the same helper functions used by the `call()` pipeline. Failures are appended to the `checks` list rather than thrown/returned immediately.

| Error Model | Pattern |
|-------------|---------|
| **Exception-based** (try/catch) | `try { helper(); push(passed) } catch(e) { push(failed, e) }` |
| **Error-return** (Go, Rust) | `if err := helper(); err != nil { push(failed, err) } else { push(passed) }` |

> **Note:** Error-return patterns are more natural than try/catch for this "collect all errors" flow.

#### 12.8.3 PreflightResult Type

`PreflightResult` **MUST** contain the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `valid` | boolean | `true` if all checks passed |
| `checks` | list of `PreflightCheckResult` | Ordered list of check results |
| `requires_approval` | boolean | Whether the module requires approval |
| `errors` (computed) | list of error objects | Filtered view: only checks where `passed` is `false` |

#### 12.8.4 PreflightCheckResult Type

`PreflightCheckResult` **MUST** contain the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `check` | string | Check name (e.g., `"module_id_format"`, `"acl"`, `"schema"`, `"module_preflight"`) |
| `passed` | boolean | Whether the check passed |
| `error` | error object or null | Error details when `passed` is `false` |
| `warnings` | list of strings | Non-fatal advisory messages (default: empty list) |

#### 12.8.5 Schema Validation

validate() Check 6 (schema) reuses the same JSON Schema validation that `call()` Step 6 already performs.
No additional schema library is required beyond what the SDK already uses.

#### 12.8.5.1 Module-Level Preflight (Check 7)

After schema validation, validate() **MAY** invoke the module's optional `preflight(inputs, context)` method (§5.6). This check is advisory:

- If `preflight()` returns a non-empty list of strings, they are stored as `warnings` on a `module_preflight` check result with `passed: true`.
- If `preflight()` returns an empty list, a `module_preflight` check result with `passed: true` and no warnings is added.
- If the module does not define `preflight()`, no `module_preflight` check is added.
- If `preflight()` raises an exception, the exception is caught and reported as a warning (not a failure).

##### Authorization gates module-level introspection

`preflight()` and `preview()` (§5.6) are **module-authored code**, and what they return describes what
calling the module would do — the target a side effect would land on, the argv a command-wrapping module
would execute, the records a query would touch. When the `acl` check has **failed**, `validate()`
**MUST NOT**:

- invoke the module's `preflight()` or `preview()` method,
- emit a `module_preflight` or `module_preview` check result, or
- populate `predicted_changes` — it **MUST** be empty.

Everything else keeps the collect-don't-throw behaviour of §12.8.1. The failed `acl` check itself
**MUST** still be reported, so the caller learns *why* the preflight came back invalid, and the checks
the Executor computes on its own (`schema`, `call_chain`, `module_lookup`) are unaffected. What is
withheld is exactly the part a denied caller has no authority to learn.

The rule is about **authorization**, not about validity in general. A failed `schema` check does **not**
suppress module-level introspection: a caller the ACL permits is a caller entitled to hear what the
module would do, even when its inputs are malformed. `requires_approval` likewise does not suppress it —
an approval gate means the caller is authorized and awaiting a human decision, which is precisely the
decision `predicted_changes` exists to inform.

#### 12.8.6 Naming Convention

Follow each language's idiomatic casing for PreflightCheckResult and PreflightResult fields:

| Field (Protocol) | snake_case languages | camelCase languages | PascalCase languages |
|------------------|---------------------|--------------------|--------------------|
| `check` | `check` | `check` | `Check` |
| `passed` | `passed` | `passed` | `Passed` |
| `error` | `error` | `error` | `Error` |
| `valid` | `valid` | `valid` | `Valid` |
| `checks` | `checks` | `checks` | `Checks` |
| `requires_approval` | `requires_approval` | `requiresApproval` | `RequiresApproval` |
| `errors` (computed) | `errors` | `errors` | `Errors()` |

> **Convention:** snake_case (Python, Rust), camelCase (TypeScript, Java methods), PascalCase (Go exported fields). Other languages follow their idiomatic convention.

---

## 13. Versioning

### 13.1 Version Number Specification

```
{major}.{minor}.{patch}[-{prerelease}]

major: Incompatible API changes
minor: Backward-compatible feature additions
patch: Backward-compatible bug fixes
prerelease: draft, alpha, beta, rc
```

### 13.2 Compatibility Promise

!!! note "Normative corrections in v1.9.0 — no deprecation cycle is owed"
    v1.9.0 changes three requirements that shipped in v1.8.x. **No implementation ever
    provided the v1.8.x behaviour**, so there is nobody to deprecate *for*: these are
    corrections to text that was wrong, not a migration users have to make. Verified
    against all three SDKs rather than assumed.

    | v1.8.x text | v1.9.0 text | Did any SDK ship the v1.8.x behaviour? |
    |---|---|---|
    | `$ref` circular-reference detection **MUST** reject `A → B → A` in all forms (§4.11) | Only a `$ref` → `$ref` chain that never reaches a schema body is rejected; a **self-reference** reached by descending through `properties` / `items` / a combinator **MUST** be preserved as a lazy reference (§4.15) | **No.** All three ship the `from_ref_chain` discriminator that implements the v1.9.0 rule. v1.8.x also required Recursive Schema Support, so the blanket rule was never simultaneously satisfiable — implementers followed the requirement that worked. |
    | `$ref` depth exhaustion **MUST** throw `SCHEMA_CIRCULAR_REF` (§4.15 edge-case table) | It **MUST** throw `SCHEMA_MAX_DEPTH_EXCEEDED` | **No.** All three already emit `SCHEMA_MAX_DEPTH_EXCEEDED`; no SDK emits `SCHEMA_CIRCULAR_REF` for depth. The code was missing from the §8 registry, which is what this change repairs. |
    | The §9.1 example marked `extensions.root` / `schema.root` and peers as **MUST**, and `apcore-config.schema.json` required `version`, `project`, `extensions`, `schema`, `acl` | Those keys are normative with **fixed defaults**; only `version` and `project.name` make a file invalid by their absence | **No.** All three invented a `version: "0.16.0"` default rather than enforcing the requirement — evidence that hard-required was never what implementers understood. |

    This is a **deliberate relaxation** in the third row and a contradiction-resolution in
    the first two. All three are judged on their own merits in the linked governance issue;
    none carries a dual-accept window, because there is no prior behaviour to accept.

    Added in v1.9.0 (new requirements, not corrections): the A23 object-detection rule
    (§4.16), and the declared-configuration view `get_declared()` / `getDeclared()` /
    `Config.declared` with the rule that environment overrides count as declaration (§9.1).

- **Within major version**: Protocol backward compatible
- **Schema evolution**: Support version declaration, old version Schema readable by new SDK
- **Deprecation policy**: Keep at least 2 minor versions for deprecation period

### 13.3 Version Negotiation Algorithm

When SDK loads configuration or Schema, **MUST** perform version negotiation:

```
Algorithm: negotiate_version(declared_version, sdk_version)

Input:
  declared_version — Version declared in configuration/Schema (e.g., "1.2.0")
  sdk_version      — Current SDK's supported highest version (e.g., "1.3.0")

Output:
  effective_version — Effective version number, or throw VERSION_INCOMPATIBLE error

Steps:
  1. Parse declared_version as (major_d, minor_d, patch_d)
  2. Parse sdk_version as (major_s, minor_s, patch_s)
  3. If major_d ≠ major_s:
     → Throw VERSION_INCOMPATIBLE ("Major version incompatible")
  4. If minor_d > minor_s:
     → Throw VERSION_INCOMPATIBLE ("SDK version too low, please upgrade")
  5. If minor_d < minor_s:
     → Issue DEPRECATION_WARNING (if minor_s - minor_d > 2)
     → effective_version ← declared_version (backward compatibility mode)
  6. If minor_d == minor_s:
     → effective_version ← max(declared_version, sdk_version)
  7. Return effective_version
```

### 13.4 Schema Migration

When Schema version changes, implementations **SHOULD** support automatic migration:

```
Algorithm: migrate_schema(schema, from_version, to_version)

Input:
  schema       — Original Schema object
  from_version — Original version number
  to_version   — Target version number

Output:
  migrated_schema — Migrated Schema, or throw MIGRATION_FAILED error

Steps:
  1. If from_version == to_version → Return schema (no migration needed)
  2. migration_path ← Find migration path from from_version to to_version
     (e.g., 1.0 → 1.1 → 1.2, each step has corresponding migration function)
  3. If migration_path empty → Throw MIGRATION_FAILED ("No available migration path")
  4. current_schema ← deep_copy(schema)
  5. For each (step_from, step_to, migrate_fn) in migration_path:
     a. current_schema ← migrate_fn(current_schema)
     b. Validate current_schema conforms to step_to version's Schema specification
     c. If validation fails → Throw MIGRATION_FAILED with step info
  6. Return current_schema

Migration types:
  - add_field:     Add field (provide default value)
  - rename_field:  Rename field (keep old name as alias)
  - remove_field:  Remove field (mark as deprecated for at least 2 minor versions)
  - change_type:   Type change (only allowed in major version changes)
```

### 13.5 Backward/Forward Compatibility Matrix

| Change Type | Backward Compatible | Forward Compatible | Version Impact | Description |
|----------|----------|----------|----------|------|
| Add optional field | Yes | Yes | patch/minor | New SDK ignores unknown fields, old SDK uses defaults |
| Add required field | No | No | major | Old SDK can't provide new required field |
| Remove optional field | Yes | No | minor | Old SDK passes removed field, new SDK ignores |
| Remove required field | Yes | No | minor | Old SDK still passes field, new SDK ignores |
| Expand field type (e.g., string → string\|number) | Yes | No | minor | Old SDK only sends string, new SDK accepts more types |
| Narrow field type (e.g., string\|number → string) | No | Yes | major | Old SDK might send number, new SDK rejects |
| Add error code | Yes | No | minor | Old SDK might not recognize new error code |
| Remove error code | No | Yes | major | Old SDK's dependent error code disappears |
| Add extension point | Yes | Yes | minor | Doesn't affect existing extension points |
| Modify extension point interface | No | No | major | All extension point implementations need adaptation |
| Add middleware Hook | Yes | Yes | minor | Old middleware doesn't implement new Hook |
| Modify middleware Hook signature | No | No | major | All middleware needs adaptation |

**Compatibility Rules:**

1. **SDK MUST** ignore unknown configuration fields and Schema properties (forward compatibility foundation)
2. **SDK MUST** provide reasonable defaults for all new fields (backward compatibility foundation)
3. **SDK SHOULD** gracefully handle unknown error codes
4. **SDK MUST NOT** remove published public APIs in minor/patch versions

---

## 14. Appendix

### A. Complete Example Project Structure

```
my-ai-project/
├── apcore.yaml              # Framework configuration
├── extensions/                   # Extension directory
│   ├── api/
│   │   └── handler/
│   │       ├── task_submit.py
│   │       └── task_submit_meta.yaml
│   ├── orchestrator/
│   │   └── engine/
│   │       ├── task_flow.py
│   │       └── task_flow_meta.yaml
│   └── executor/
│       ├── validator/
│       │   ├── db_params.py
│       │   └── db_params_meta.yaml
│       └── handler/
│           ├── db_task.py
│           └── db_task_meta.yaml
├── schemas/                      # Schema definitions
│   ├── api.handler.task_submit.schema.yaml
│   ├── orchestrator.engine.task_flow.schema.yaml
│   ├── executor.validator.db_params.schema.yaml
│   └── executor.handler.db_task.schema.yaml
├── acl/                          # Permission configuration
│   └── global_acl.yaml
└── tests/
```

### B. JSON Schema Validation Files

See `.schema.json` files in `schemas/` directory.

### C. Reference Implementations

- **apcore (this project)**: Python reference implementation
- **apflow**: Task orchestration application example based on apcore

### D. Module Exposure Methods Reference (Non-core, Reference Only)

apcore modules can be exposed in multiple forms for external invocation. Following are common AI protocol mapping references, **apcore doesn't provide adapter implementations**.

#### D.1 MCP (Model Context Protocol) Mapping

```yaml
# Module → MCP Tool mapping
mcp_mapping:
  tool:
    name: "{module.id}"
    description: "{module.description}"
    inputSchema: "{module.input_schema}"
    outputSchema: "{module.output_schema}"

    # annotations mapping
    annotations:
      readOnlyHint: "{module.annotations.readonly}"
      destructiveHint: "{module.annotations.destructive}"
      idempotentHint: "{module.annotations.idempotent}"
      openWorldHint: "{module.annotations.open_world}"

  # Example
  example:
    # apcore Module
    module:
      id: "executor.email.send_email"
      description: "Send emails via SMTP"
      input_schema: { ... }
      annotations:
        readonly: false
        destructive: false
        idempotent: false
        open_world: true

    # Map to MCP Tool
    mcp_tool:
      name: "executor.email.send_email"
      description: "Send emails via SMTP"
      inputSchema: { ... }
      outputSchema: { ... }
      annotations:
        readOnlyHint: false
        destructiveHint: false
        idempotentHint: false
        openWorldHint: true
```

#### D.2 A2A (Agent-to-Agent) Mapping

```yaml
# Module → A2A Skill mapping
a2a_mapping:
  skill:
    id: "{module.id}"
    name: "{module.name}"
    description: "{module.description}"
    tags: "{module.tags}"
    examples: "{module.examples[*].title}"
    inputSchema: "{module.input_schema}"
    outputSchema: "{module.output_schema}"
    inputModes: ["application/json"]
    outputModes: ["application/json"]
```

#### D.3 OpenAI Function Calling Mapping

```yaml
# Module → OpenAI Function mapping
openai_mapping:
  function:
    name: "{module.id.replace('.', '_')}"  # OpenAI doesn't support dots
    description: "{module.description}"
    parameters: "{module.input_schema}"
    strict: true

  # annotations mapping (OpenAI Agents SDK)
  agents_sdk:
    needs_approval: "{module.annotations.requires_approval}"
```

#### D.4 Anthropic Claude Tool Mapping

```yaml
# Module → Anthropic Tool mapping
anthropic_mapping:
  tool:
    name: "{module.id.replace('.', '_')}"
    description: "{module.description}"
    input_schema: "{module.input_schema}"
    input_examples: "{module.examples[*].inputs}"
```

#### D.5 LangChain Tool Mapping

```python
# Module → LangChain Tool
from langchain.tools import StructuredTool

def module_to_langchain_tool(module):
    return StructuredTool.from_function(
        func=module.execute,
        name=module.id,
        description=module.description,
        args_schema=module.input_schema,  # Pydantic model
        tags=module.tags,
        metadata=module.metadata,
    )
```

### E. Module Definition Methods Comparison

apcore supports three module definition methods to meet different scenario needs:

| Dimension | Class-based (Class Definition) | Function-based (Functional) | External Binding (External Binding) |
|------|---------------------|------------------------|---------------------------|
| **Definition Method** | Inherit Module base class | `@module` / `module()` | YAML binding file |
| **Schema Source** | Native model / YAML | Type annotation auto-generation | YAML explicit definition / auto_schema |
| **Code Invasiveness** | High (need inherit base class) | Low (add decorator or function call) | Zero (no source code modification) |
| **Applicable Scenarios** | New module development | Existing function/method wrapping | Existing application zero-modification integration |
| **Lifecycle Hooks** | on_load / on_unload | Not supported | Not supported |
| **Advanced Features** | Full support | Partial support (annotations, tags, etc.) | Full support (via YAML configuration) |
| **Cross-language** | Each language implements base class | Each language implements module() | Universal YAML format |

**Cross-language Syntax Reference:**

Each language SDK **SHOULD** provide idiomatic module definition syntax. The following illustrates the general patterns:

| Pattern | Class-based | Decorator/Attribute | Function Call | External Binding |
|------|------------|--------------------------|-------------------------|-----------------|
| Description | Inherit/implement Module interface | Language-native annotation | Wrap existing callable | YAML binding file |
| Example (Python) | `class M(Module)` | `@module(id=...)` | `module(fn, id=...)` | YAML |
| Example (TypeScript) | `class M extends Module` | `@module({id: ...})` | `module(fn, {id: ...})` | YAML |

---

## Revision History

> **Note**: The specification document uses its own version track (`1.x.0-draft`), independent of the SDK/ecosystem release version (`0.x.0`). The mapping between specification versions and release versions is recorded in `CHANGELOG.md`.

| Version | Date | Change Description |
|------|------|----------|
| 1.0.0-draft | 2026-02-05 | Initial draft |
| 1.1.0-draft | 2026-02-07 | Added §5.11 Function-based Module Definition, §5.12 External Schema Binding, Appendix E Module Definition Methods Comparison |
| 1.2.0-draft | 2026-02-09 | Revised §4.3 supplemented x-llm-description usage guide; Added §4.16 Strict Mode Export, §4.17 Export Profile |
| 1.3.0-draft | 2026-03-01 | Added §7 Approval System (ApprovalHandler protocol, Executor Step 4.5, error types, built-in and protocol bridge handlers, phased implementation, conformance levels); Updated §4.4 requires_approval annotation to reference runtime enforcement; Added APPROVAL_DENIED/TIMEOUT/PENDING error codes to §8; Renumbered §7–§13 → §8–§14 |
| 1.4.0-draft | 2026-03-06 | Refined Executor pipeline — Approval Gate is now Step 5, subsequent steps shifted; Added Executor.validate() [SHOULD] to §12.2 with PreflightResult/PreflightCheckResult types for non-destructive preflight checks through Steps 1–6; Updated §7.4, §7.9, streaming protocol references to match new numbering; Added §12.8 Executor.validate() Cross-Language Implementation Guide (error handling mapping, type mapping for Python/TypeScript/Go/Rust/Java/C/C++, schema library requirements, naming conventions); Added C/C++ and TypeScript to §12.6; Added validate() preflight to §12.3 requirements table; Added Preflight Tests to §12.4 consistency test suite |
| 1.5.0-draft | 2026-03-20 | Added §5.13 Display Overlay — sparse binding.yaml `display` section for surface-facing presentation (CLI/MCP/A2A alias, description, documentation overrides); Defined resolve priority chain algorithm; Added `ResolvedModule` type; Added `SurfaceOverride` and `DisplayOverlay` to `binding.schema.json`; Added `suggested_alias` scanner metadata convention; Deprecated `simplify_ids` in favor of display overlay; Cross-language implementation guide for Python/TypeScript/Rust/Go/Java/Ruby/PHP; Renumbered §5.13 Edge Case Handling → §5.14 → §5.15 |
| 1.6.0-draft | 2026-03-29 | Added §9.4–9.14 Config Bus Architecture — namespace registration, unified configuration file with legacy/namespace mode detection, mount mechanism for third-party integration, per-namespace environment variable overrides, namespace-aware access API (get/set/bind/namespace), extended validation algorithm A12-NS, hot-reload with namespace support, cross-language implementation requirements (Python/TypeScript/Rust/Go/Java), ecosystem integration patterns (apcore packages, third-party packages, framework auto-registration), optional config discovery; Added error codes CONFIG_NAMESPACE_DUPLICATE, CONFIG_NAMESPACE_RESERVED, CONFIG_ENV_PREFIX_CONFLICT, CONFIG_MOUNT_ERROR, CONFIG_BIND_ERROR; Added `_config` reserved namespace for strict/allow_unknown meta-configuration |
| 1.6.0-draft | 2026-04-08 | §2.7 EBNF constraint #1 — `canonical_id` maximum length raised from 128 to 192 characters to accommodate deep-namespace languages (Java/.NET/Spring FQN-derived IDs). 192 is filesystem-safe (`192 + ".binding.yaml" = 205 bytes < 255-byte filename limit on ext4/xfs/NTFS/APFS/btrfs`) and remains within `VARCHAR(255)` for typical persistence. Schemas updated: `binding.schema.json`, `module-schema.schema.json`, `module-meta.schema.json`, `acl-config.schema.json` (callers/targets pattern strings, kept symmetric with module_id). Algorithm A01 (`directory_to_canonical_id`) Step 7 threshold updated. Conformance test T01-006 boundary updated. Forward-compatible relaxation: implementations conforming to this revision MUST accept IDs up to 192; older 128-only implementations cannot load IDs in the 129–192 range from newer SDKs. |
| 1.7.0-draft | 2026-05-04 | **§6.1** — formalised compound operators `$or` (list[object]) and `$not` (object) as conditions sub-fields, with required cross-mode (sync/async) evaluator semantics and fail-closed rules for empty `$not` (resolves issue #46 / `planning/acl-compound-operators-spec-patch`). **§6.2.1** (new) — formalised compound operators `$or` and `$not` as the first element of `callers`/`targets` pattern arrays, with the four allowed forms tabulated and a reservation rule preventing literal-token matching (resolves issue #46). All four behaviour shapes are already implemented uniformly in apcore-python, apcore-typescript, and apcore-rust at v0.20.0 and verified by `../../conformance/fixtures/acl_evaluation.json`; no SDK behaviour change. **§5.1** — `Executor.validate()` description aligned with §12.8: "Steps 1–5 and Step 7" (skipping Step 6 Middleware Before Chain) plus optional module-level preflight, replacing the stale "Steps 1–6" wording that pre-dated the v0.18 step swap (resolves issue #47 / `planning/validate-step-count-spec-patch`). No SDK behaviour change. |
| 1.8.0-draft | 2026-05-04 | **§5 streaming semantics** — corrected "shallow merge" to "**recursive deep merge** with depth cap" matching all three SDK implementations (`apcore-python/src/apcore/executor.py:_deep_merge`, `apcore-typescript/src/executor.ts:deepMergeChunk`, `apcore-rust/src/executor.rs:deep_merge_chunks`) and `../../conformance/fixtures/stream_aggregation.json` (9 cases). Added algorithm `A24 deep_merge_chunks` to `./algorithms.md` formalising the merge with the canonical 32-depth cap (resolves issue #49). **§7.4 Step 11 Contract** (new block after the pipeline diagram) — five-point normative contract specifying: `call()` returns module output unchanged (no envelope), `stream()` final accumulated dict is what Step 9 validates, `validate()` returns `PreflightResult`, trace metadata lives on `Context` (not the return value), side-channel emissions are independent (resolves issue #50). **§6.7 Canonical System Module Catalogue** (new) — enumerates the 9 canonical `system.*` modules with read/write classification, conformance level (Level 1 for the 6 read modules; Level 2 for the 3 control modules), and 6 cross-cutting requirements (registration via `register_internal()`, audit events for write modules, `system.control.reload_module` mutually-exclusive `module_id`/`path_filter`, sensitive-key redaction in `update_config` output, persistence requirement for `toggle_feature`). Authoritative JSON Schemas remain in SDK source to avoid drift; this section is the contract surface (resolves issue #51). All three patches: zero SDK behaviour change. |
| 1.9.0 | 2026-05-18 | **§9.9.5 Reserved Namespace Query** (new) — formalised public API requirement that all SDKs MUST expose a read-only query API returning the set of reserved top-level namespace names (`apcore`, `_config` at minimum). Returned set MUST be the single source of truth used by `register_namespace` to enforce `CONFIG_NAMESPACE_RESERVED` (single source of truth invariant). Class-level / module-level access (callable without instantiating Config). Cross-language examples for Python (`Config.reserved_namespaces()`), TypeScript (`Config.reservedNamespaces`), Rust (`Config::reserved_namespaces()`). Intended for third-party consumers (custom CLIs, framework integrations) needing fail-fast pre-validation of user-supplied namespace names. Resolves issue #60. |
| 1.9.0 | 2026-08-12 | **Finalised — first non-draft release of the specification.** Normative corrections from the cross-language consistency sweep: **§4.11/§4.15/A05** self-reference and circular reference separated (a `$ref` re-entered through a schema body is a recursive data structure and MUST be preserved as a lazy reference; only a `$ref` → `$ref` chain MUST raise), resolving a contradiction with the Recursive Schema Support requirement in the same document; **§4.15/§8** `$ref` depth exhaustion MUST raise `SCHEMA_MAX_DEPTH_EXCEEDED`, not `SCHEMA_CIRCULAR_REF`, and the code is registered; **§4.16/A23** object detection widened to `properties` present AND (`type` absent OR declaring `object`); **§9.1** `required` narrowed to `version` and `project` — a key with a canonical default is normative but not required — and the declared-configuration view (`get_declared()`) added, with environment overrides counting as declaration; **§11 type-mapping** `format` is an annotation and MUST NOT fail validation, and the module boundary MUST NOT coerce types. No implementation had provided the superseded v1.8.x behaviour, so no deprecation cycle was owed. Governance: apcore#79. |
| 1.10.0 | 2026-08-13 | **§12.2 `Interface: Registry` gains `register` (#90).** The normative component interface declared `discover`, `get`, `list` and `describe` — not `register`, the most-used entry point on the component and the one every SDK exposes with a four-argument signature. `register(module_id, module, version?, metadata?)` is now stated, and when an implementation accepts `metadata`, a `dependencies` entry (a list of `{module_id, version?, optional?}`) **MUST** reach the registered module's descriptor so `get_definition(module_id).dependencies` returns what the caller declared. That requirement existed nowhere: all three SDKs lost it independently and all three fixed it independently (apcore-python `ad2998d`, apcore-typescript#35, apcore-rust `71295e1`), because discovery-time sorting reads its own parse and keeps working — `resolve_dependencies` looks healthy while the accessor is empty and reload order degrades to its sort's seed order, which is alphabetical and therefore plausible. `version` is stated as an OPTIONAL parameter only: all three SDKs accept it, only apcore-python resolves by it, and §5.4 continues to govern multi-version coexistence as optional — making resolution normative would put a requirement into the spec that two of three implementations do not provide, which is the shape 1.9.0 spent a release removing. `get(module_id)` keeps its single-argument normative form for the same reason. No SDK behaviour change: all three already satisfy the requirement. Governance: apcore#90. |
| 1.11.0 | 2026-08-14 | **§12.2 `Interface: Executor` — cross-executor rebind is a MUST (#92).** `features/core-executor.md` stated it as *SHOULD raise `ContextBindingError`*, with *"SDKs that choose to accept silently instead MUST document the deviation prominently"* as the escape hatch. All three SDKs raise, so the deviation was permitted for nobody — and the alternative had a cost: `conformance/fixtures/context_create.json` had to express it as `expected_one_of: [raise, silent_accept]`, which no driver can assert without deciding its own branch. All three hardcoded `raise` and read the alternation only in a comment, so mutating the entire expectation left every suite green. The rule is now stated normatively with the wire code `CONTEXT_BINDING_ERROR`, the fixture carries a single `expected`, and the feature page records the withdrawal. **No SDK behaviour change.** Governance: apcore#92. |
| 1.12.0 | 2026-08-14 | **§11 type-mapping — the library-level coercion knob's behaviour is normative when the knob exists (#95).** Offering the switch stays a MAY; an SDK that offers one **MUST** coerce exactly `string→integer`, `string→number`, and `string→boolean` limited to `"true"` / `"false"` case-sensitive, and **MUST NOT** coerce anything else. Previously the paragraph constrained only *where* the knob could be used — not on the module path, not from configuration, default off — and said nothing about what it does, so apcore-rust and apcore-typescript shipped a twelve-spelling case-insensitive dialect (`"yes"`, `"on"`, `"y"`, `"t"`, `"1"`, `"0"` and negatives) while apcore-python coerced no string to a boolean at all, and both were conforming. `"0"` → `false` sat directly against R5, which makes the number `0` a MUST-reject for `boolean`. `conformance/fixtures/schema_validation.json` had pinned the coercing mode cross-SDK in exactly one case, on `integer` — the one axis where all three agreed — which is why it never fired. Governance: apcore#95. |
| 1.13.0 | 2026-08-17 | **§12.8.5.1 — a failed `acl` check withholds module-level introspection (#96).** `validate()` looked the module up at Step 3 and ran `preflight()` and `preview()` at Check 7 on the strength of that lookup alone, so a caller the ACL had just denied still made module-authored code run and still received what it returned. For a command-wrapping module that is the resolved binary and its argv; for a writer it is the target of the side effect. All three SDKs did this — apcore-python `executor.py`, apcore-typescript `executor.ts`, apcore-rust `executor.rs` — each guarding only on "module lookup succeeded", and `apcore-mcp-rust` had already grown a string-matched disclosure filter over the top of it (`async_task_bridge.rs`), which is the evidence the gap was reachable in a shipped product rather than theoretical. `validate()` **MUST NOT** now invoke either hook, emit a `module_preflight` / `module_preview` check, or populate `predicted_changes` when `acl` failed; the failed `acl` check itself is still reported and no other check is suppressed, because the rule is about authorization and not about validity — a malformed input from a permitted caller still gets the module's own account of what would happen. §12.8.1 gains principle 5, §12.4 gains the test, and `conformance/fixtures/preflight_disclosure.json` pins it. **§4.15** additionally gains the two `format` rows its edge-case table never carried, so that `schema_hardening_formats.json`'s long-standing §4.15 citation resolves to something; [type-mapping §11.1](./type-mapping.md#111-format-keyword) remains the authority. Governance: maintainer approval per GOVERNANCE.md § Decision Making; **no tracking issue was opened.** The `apcore#96` cited when this row was written was a reserved number that issue #96 has since been assigned to for an unrelated change (`system.usage.*` schemas) — the citation is withdrawn rather than repointed, because inventing a link is worse than recording that there is none. |
| 1.14.0 | 2026-08-25 | **§6.7.1 Usage Module Output Contract (new) — the `system.usage.*` field contract is stated, not deferred (#96).** §6.7 required "equivalent input/output schemas" and pointed at each SDK's source as the schema source of truth. That deferral is why three implementations diverged in five ways without any becoming non-conformant. Now normative: `period` **MUST** match `^[1-9][0-9]*[hd]$`, declared as a `pattern` in `input_schema` so a malformed value fails uniformly with `SCHEMA_VALIDATION_ERROR` rather than through an implementation-private parser (apcore-python accepted `"0h"`, `"-5d"` and `"+3h"`; apcore-typescript rejected all three; apcore-rust parsed no period at all), and **every** statistic in both outputs MUST be computed over `[now − period, now]` — apcore-rust echoed `period` back while `get_all_summaries()` / `get_module_summary()` covered the full retained history. `hourly_distribution[].hour` **MUST** be the collector's own key `YYYY-MM-DDTHH`; apcore-rust reformatted it to `%Y-%m-%dT%H:00:00Z` behind a constant whose comment claimed the two matched, and **this specification's own example in `features/system-modules.md` showed the reformatted spelling**, so the divergent implementation was the one following the docs. Exactly 24 entries, ascending, zero-filled. `p99_latency_ms` **MUST** be nearest-rank `sorted[min(ceil(0.99·N), N) − 1]` with no interpolation — apcore-python computed that index and then returned `sorted[rank]`, one element higher, contradicting its own comment; for 100 samples it answered 100 where the other two answered 99. Unattributed calls are the literal `caller_id` `"unknown"`. `output_schema()` **MUST** declare `properties` and `required`; apcore-rust returned a bare `{"type": "object"}` for both modules. `schemas/sys-usage-summary.schema.json` and `schemas/sys-usage-module.schema.json` are added as the canonical shape — both with `additionalProperties: false`, and the `hour` pattern deliberately rejects the current apcore-rust output. **This is an SDK behaviour change in apcore-rust (all six points) and apcore-python (p99, period grammar).** Governance: apcore#96. |
| 1.15.0 | 2026-08-25 | **§6.6.5 Governance State Query (new), §6.6.3 rewritten — *configured* and *enforced* are separate facts (#97).** Nothing exposed what is actually gating a registry: apcore-rust leaked `acl` / `approval_handler` / `policy` as public struct fields with no defined semantics, apcore-typescript and apcore-python exposed nothing, and none of the three answered the useful question — because `acl != null` means an ACL is attached, not that ACL evaluation runs. The gates are pipeline **steps**, and three of the four strategies this specification itself defines (`internal`, `testing`, `minimal`) remove `acl_check`, so an adapter reading `acl.is_some()` reports "protected" in precisely the configuration `set_acl()` already warns about. §6.6.5 requires a read-only `governance_state()` returning seven observations plus one derived flag, with normative field names across the three SDKs; `builtin_acl_gate_wired` / `builtin_approval_gate_wired` **MUST** be determined by step type or capability, **never** by step name, because `StrategyInfo` carries names only and a custom step named `acl_check` would otherwise report a gate that is not there — the one direction the flag must never fail in. `unprotected_control_surface` is defined exactly, and is explicitly **not** a security verdict: it reports the absence of a recognised gate, never the presence of protection, and an `is_secure`-shaped field is forbidden. §6.6.3 additionally states what was previously only implied: Layer 1 registers **0 / 6 / 9** modules across two config flags, not one; and Layers 2 and 3 are **inactive by absence** — a missing `acl/` path attaches nothing and **MUST NOT** synthesize an empty default-deny ACL, a missing `ApprovalHandler` warns and continues unless `ExecutionPolicy(strict)` is set. **No default changes and no behaviour changes**; the accessor is purely additive, and apcore-rust's existing public fields are untouched. Governance: apcore#97. |
