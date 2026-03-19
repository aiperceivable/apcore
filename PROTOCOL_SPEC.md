# apcore — AI-Perceivable Core Standard Specification

> **Canonical Specification** - This document is the authoritative specification for the apcore protocol

> Version: 1.5.0-draft
> Status: Draft Specification (RFC 2119 Conformant)
> Stability: Specification content is stable, pending reference implementation verification
> Last Updated: 2026-03-10

---

### Table of Contents

- [1. Overview](#1-overview)
- [2. Naming Specification](#2-naming-specification-naming-specification)
- [3. Directory Specification](#3-directory-specification-directory-specification)
- [4. Schema Specification](#4-schema-specification-schema-specification)
- [5. Module Specification](#5-module-specification-module-specification)
- [6. ACL Specification](#6-acl-specification-acl-specification)
  - [6.6 System Module Permissions](#66-system-module-permissions)
- [7. Approval System](#7-approval-system-approval-system)
- [8. Error Handling Specification](#8-error-handling-specification-error-handling-specification)
- [9. Configuration Specification](#9-configuration-specification-configuration-specification)
- [10. Observability Specification](#10-observability-specification-observability-specification)
- [11. Extension Mechanism](#11-extension-mechanism-extension-mechanism)
- [12. SDK Implementation Guide](#12-sdk-implementation-guide-sdk-implementation-guide)
- [13. Versioning](#13-versioning-versioning)
- [14. Appendix](#14-appendix)
- [Revision History](#revision-history)

---

## 1. Overview

### 1.1 Project Positioning

apcore (AI-Perceivable Core) is a **schema-enforced module standard for the AI-Perceivable era**.

**One-sentence definition**:
> apcore is an AI-Perceivable module standard that makes every interface naturally perceivable and understandable by AI through enforced Schema definitions and behavioral annotations.

**Positioning**:
- **AI-Perceivable Module Standard**: Not just an AI framework, but a universal module standard that is naturally AI-Perceivable
- **Enforced AI-Perceivable**: Schema is mandatory, making modules naturally perceivable and understandable by AI
- **Complementary to MCP/A2A**: MCP/A2A define communication protocols, apcore defines module construction specifications
- Foundation for other projects (such as apflow)

### 1.2 Core Principles

| Principle | Description |
|------|------|
| **Schema-driven** | All modules enforce definition of `input_schema` / `output_schema` / `description` |
| **Three-layer Metadata** | Core (enforced Schema) + Annotations (behavior Annotations) + Extensions (free metadata) |
| **Directory as ID** | Directory path automatically maps to module ID, zero manual configuration |
| **AI-Perceivable** | Schema + Annotations enable AI/LLM to perceive and understand modules, this is a design requirement |
| **Universal Standard** | Modules can be called by code/AI/HTTP/CLI in any manner |

### 1.3 Design Goals

- **Universality**: Modules can be called by code, AI, HTTP, CLI, etc. in any manner
- **AI Perceptibility**: Enforced Schema ensures LLM can perceive and understand modules
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
│ MCP Server  │  HTTP API   │  CLI Tool   │  gRPC Service   │
│ (Claude)    │  (REST)     │  (Terminal) │  (Microservice) │
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
| Module | Module | Basic execution unit of apcore, encapsulates a function, **must** define `input_schema`, `output_schema`, and `description` (≤200 characters). Can optionally define `documentation` (≤5000 characters) to provide detailed documentation |
| Schema | Schema | Data structure definition based on JSON Schema Draft 2020-12, used for validating input/output and for AI/LLM understanding |
| Canonical ID | Canonical ID | Globally unique identifier for a module, automatically generated from directory path, format is dot-separated snake_case (e.g., `executor.email.send_email`) |
| Registry | Registry | Core component responsible for module discovery, registration, loading, and management |
| Executor | Executor | Core component responsible for module invocation execution, handles Schema validation, ACL checking, middleware dispatching |
| Context | Context | Runtime context object during module execution, carries trace_id, call chain, identity information, and shared state |
| Access Control List | ACL | Set of rules defining inter-module invocation permissions, based on caller/target pattern matching |
| Middleware | Middleware | Interceptor running before and after module execution, executes in onion model, can modify input/output |
| Extension Point | Extension Point | Replaceable component interface provided by framework (e.g., SchemaLoader, ModuleLoader), allows custom implementation |
| Annotations | Annotations | Module-level behavior metadata (readonly, destructive, etc.), helps AI/LLM make invocation decisions |
| Metadata | Metadata | Completely open key-value dictionary for storing extension information, framework does not validate its content |
| Entry Point | Entry Point | Code entry location of module, format is `filename:ClassName`, can be auto-inferred or manually configured |
| Call Chain | Call Chain | Complete list of module ID paths from root invocation to current invocation, used for loop detection and depth limiting |
| Trace ID | Trace ID | Identifier uniquely identifying a complete invocation chain, **must** be UUID v4 format |
| Identity | Identity | Structured expression of caller identity (user/service/Agent/API Key/system), ACL engine depends on it |

### 1.7 API Naming Conventions

apcore public API uses concise universal names (e.g., `Module`, `Context`, `Registry`),
relying on language-native namespace mechanisms to avoid conflicts:

| Language | Namespace Isolation Method | Example |
|------|----------------|------|
| Python | Package import | `from apcore import Module` / `import apcore` |
| Go | Package name qualification | `apcore.Module(...)` |
| Rust | Module path | `apcore::Module` |
| TypeScript | Module import | `import { Module } from 'apcore'` |
| Java | Package path | `import com.apcore.Module` |

Implementations **MUST** follow these naming rules:
- In languages with namespace mechanisms, **MUST NOT** add redundant prefixes to public APIs
- In languages without namespace mechanisms, **MUST** use `apcore_` prefix
- Error types **SHOULD** be prefixed with their domain (e.g., `ModuleError`, `SchemaValidationError`)

---

## 2. Naming Specification (Naming Specification)

### 2.1 Directory as ID (Core Rule)

**Directory path is the single source of truth for module IDs**. IDs are automatically generated from directory paths, zero configuration.

Implementations **must** convert directory paths to Canonical IDs according to the following algorithm:

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
  6. If len(canonical_id) > 128 → Throw ID_TOO_LONG error
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
    max_length: 128
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

All SDK implementations **MUST** validate module IDs against this pattern during `register()`. Invalid IDs **MUST** be rejected with `GENERAL_INVALID_INPUT` error.

### 2.2 ID Map (Cross-language Conversion)

**ID Map** module handles cross-language ID conversion, supporting automatic recognition and manual configuration. Implementations **must** support canonical conversion from various language native formats to Canonical ID.

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
  framework: [system, internal, core, apcore, plugin, schema, acl]

  # Programming language keywords
  keywords: [class, def, import, return, if, else, for, while, true, false, null, none]

  # Disallowed patterns
  patterns:
    - "^_.*"         # Starting with underscore
    - "^[0-9].*"     # Starting with digit
    - ".*__.*"       # Double underscore
```

### 2.6 ID Conflict Detection

Implementations **must** perform conflict detection during module scanning, module registration, and dynamic loading.

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

The formal definition of Canonical ID uses EBNF notation. All implementations **must** reject IDs that do not conform to this grammar.

```ebnf
(* apcore Canonical ID EBNF *)

canonical_id    = segment , { "." , segment } ;
segment         = lower_alpha , { lower_alpha | digit | "_" } ;
lower_alpha     = "a" | "b" | "c" | "d" | "e" | "f" | "g" | "h" | "i"
                | "j" | "k" | "l" | "m" | "n" | "o" | "p" | "q" | "r"
                | "s" | "t" | "u" | "v" | "w" | "x" | "y" | "z" ;
digit           = "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" ;

(* Constraints *)
(* 1. canonical_id total length MUST NOT exceed 128 characters *)
(* 2. segment MUST NOT be a reserved word (see §2.5) *)
(* 3. segment MUST NOT start with a digit (guaranteed by production) *)
(* 4. segment MUST NOT contain consecutive double underscores "__" *)
```

Equivalent regular expression: `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`

---

## 3. Directory Specification (Directory Specification)

### 3.1 Standard Directory Structure

Implementations **must** follow the directory structure below. The nesting depth under `extensions/` directory (not including `extensions/` itself) **must not** exceed 8 levels.

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

Implementations **must not** follow symbolic links (symlinks) in default mode. If symbolic link support is needed, it **must** be enabled through explicit configuration:

```yaml
# apcore.yaml
extensions:
  follow_symlinks: false  # Default value, MUST NOT follow
```

- When symbolic link following is enabled, implementations **must** detect symbolic link loops
- The resolved path of symbolic links **must** still be within the `extensions_root` scope

### 3.5 Hidden File Handling

Implementations **must** ignore the following files and directories during module scanning:

| Pattern | Example | Description |
|------|------|------|
| Starts with `.` | `.git/`, `.env` | Hidden files/directories |
| Starts with `_` | `_internal/`, `_test.py` | Internal files/directories |
| `__pycache__/` | — | Python cache |
| `node_modules/` | — | Node.js dependencies |
| `*.pyc` | — | Python compiled files |

Implementations **may** extend the ignore pattern list through configuration.

### 3.6 Scanning Algorithm

Implementations **must** scan the extensions directory according to the following algorithm:

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

## 4. Schema Specification (Schema Specification)

### 4.1 Overview

All modules **must** define `input_schema` and `output_schema` to support:
- LLM tool calling (MCP compatible)
- Runtime data validation
- Automatic API documentation generation
- Cross-language interoperability

### 4.2 Schema Format

**Must** be based on **JSON Schema Draft 2020-12** ([RFC unpublished draft](https://json-schema.org/draft/2020-12/json-schema-core)), extending with LLM-friendly fields.

**Compliance Requirements:**

| Requirement | Level | Description |
|------|------|------|
| Draft 2020-12 core vocabulary | **MUST** | type, properties, required, $ref, etc. |
| Draft 2020-12 validation vocabulary | **MUST** | minimum, maximum, pattern, enum, etc. |
| `$schema` declaration | **SHOULD** | Schema files **should** declare `$schema` field |
| `x-` extension prefix | **MUST** | Custom extension fields **must** be named with `x-` prefix |
| `additionalProperties` | **SHOULD** | input_schema **should** explicitly declare `additionalProperties: false` |

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
      enum: [cursor, offset, page]
      default: cursor
      description: "Pagination strategy. 'cursor' = opaque continuation token; 'offset' = numeric offset+limit; 'page' = page-number-based pagination. Only meaningful when paginated=true."
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
open_world=true       → AI knows this call involves external systems, may be slow
streaming=true        → AI knows this module emits partial results progressively
cacheable=true        → AI knows it can reuse previous results within cache_ttl
paginated=true        → AI knows to pass pagination params and expect partial results
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

  paths:
    yaml_schemas: "./schemas"

  # Validation options
  validation:
    strict: true                # Strict mode: disallow extra fields
    coerce_types: true          # Type coercion
```

### 4.10 Language-specific Schema Implementations

Each language SDK **must** provide a native schema implementation that supports JSON Schema Draft 2020-12 validation and YAML schema loading. The specific library choices are left to SDK implementers.

### 4.11 Schema References ($ref)

Implementations **must** support `$ref` references and **must** resolve according to the following algorithm. Implementations **must** detect and reject circular references.

```
Algorithm: resolve_ref(ref_string, current_file, schemas_dir, visited_refs)

Input:
  ref_string   — $ref value (e.g., "./common/error.schema.yaml#/definitions/ErrorDetail")
  current_file — Current Schema file path
  schemas_dir  — schemas root directory
  visited_refs — Set of visited refs (for loop detection)

Output:
  resolved_schema — Resolved Schema object

Preconditions:
  - ref_string is not empty

Steps:
  1. If ref_string ∈ visited_refs → Throw SCHEMA_CIRCULAR_REF error
  2. visited_refs ← visited_refs ∪ {ref_string}
  3. Parse ref_string into (file_part, json_pointer):
     a. If starts with "#" → file_part = current_file, json_pointer = ref_string[1:]
     b. If contains "#" → Split by "#" into file_part and json_pointer
     c. If starts with "apcore://" → Convert to file path under schemas_dir
     d. Otherwise → file_part is path relative to current_file directory
  4. schema_doc ← Load and parse YAML/JSON file for file_part
  5. resolved ← Locate target node in schema_doc using json_pointer
  6. If resolved still contains $ref → Recursively call resolve_ref(...)
  7. Return resolved

Complexity: O(d), where d is reference depth (implementations SHOULD limit max depth to 32)
```

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

When both YAML metadata file (`*_meta.yaml`) and code define Annotations, conflicts **must** be resolved by the following priority:

1. **YAML metadata file** (highest priority) — Operations teams can override behavior annotations without modifying code
2. **Explicit definition in code** (secondary priority) — Developer defines on module class
3. **Default values** (lowest priority) — Default values provided by framework

Implementations **must** merge rather than replace when loading: If YAML only defines `readonly: true`, other fields **must** retain values from code or defaults.

### 4.14 Schema Validation Error Format

When Schema validation fails, implementations **must** return structured error information:

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

Implementations **must** handle Schema edge cases according to the following table:

| Scenario | Behavior | Level |
|------|------|------|
| `$ref` depth exceeds `schema.max_ref_depth` | Throw `SCHEMA_CIRCULAR_REF` | **MUST** |
| `$ref` target path doesn't exist (404) | Throw `SCHEMA_NOT_FOUND` | **MUST** |
| Empty Schema `{}` | Treat as `type: object`, allow any properties | **MUST** |
| YAML/JSON syntax error | Throw `SCHEMA_PARSE_ERROR` | **MUST** |
| Unknown JSON Schema keyword (e.g., `x-custom`) | Ignore (forward compatible) | **MUST** |
| Circular reference detection (A → B → A) | Throw `SCHEMA_CIRCULAR_REF` | **MUST** |
| `required` field is empty array `[]` | Treat as no required fields | **MUST** |
| `enum` value contains `null` | Allow, `null` is valid enum value | **MUST** |
| Cross-file `$ref` loading timeout | Throw `SCHEMA_PARSE_ERROR` (cause: timeout) | **SHOULD** |

**Example — Circular Reference Detection:**

```yaml
# schemas/user.schema.yaml
$ref: "./team.schema.yaml#/definitions/team"

# schemas/team.schema.yaml
definitions:
  team:
    $ref: "./user.schema.yaml"  # Circular reference
```

**Behavior**: Maintain reference path stack in `resolve_ref()`, throw `SCHEMA_CIRCULAR_REF` when duplicate path is detected.

### 4.16 Strict Mode Export

OpenAI and Anthropic's `strict: true` mode requires JSON Schema to satisfy additional constraints. apcore defines `to_strict_schema()` conversion to transform standard apcore Schema to Strict Mode compatible format.

**Strict Mode Requirements:**

| Requirement | Description |
|------|------|
| `additionalProperties: false` | All nested `object` types **must** set this |
| All fields `required` | All fields in `properties` **must** appear in `required` array |
| Optional fields expressed with nullable | Originally optional fields become `required` + `type: ["original_type", "null"]` |
| No `x-*` extension fields | All `x-*` fields **must** be stripped |
| No `default` values | `default` fields **must** be removed |

**`to_strict_schema()` Conversion Rules:**

```
Input: apcore_schema (standard JSON Schema + x-* extensions)
Output: strict_schema (Strict Mode compatible JSON Schema)

Rules:
  1. Recursively traverse all type: "object" nodes:
     a. Set additionalProperties: false
     b. Add properties not in required to required
     c. For newly added required fields, change their type to [original_type, "null"]
  2. Remove all fields starting with "x-" (x-llm-description, x-examples, x-sensitive, x-constraints, etc.)
  3. Remove all default fields
  4. Recursively process nested objects (including array items, oneOf/anyOf/allOf sub-schemas)
```

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
required: [to, cc]
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

For detailed algorithm pseudocode, see [algorithms.md A23](docs/spec/algorithms.md#a23-to_strict_schema--strict-mode-conversion).

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

## 5. Module Specification (Module Specification)

### 5.1 Module File Structure

Each module consists of the following parts:

```
extensions/{layer}/{type}/{module_name}.{ext}      # Module implementation
extensions/{layer}/{type}/{module_name}_meta.yaml  # Module metadata (optional)
schemas/{canonical_id}.schema.yaml              # Schema definition
```

### 5.2 Metadata File and Entry Point Resolution

Implementations **must** resolve module entry points according to the following algorithm:

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

Implementations **must** use topological sorting to resolve dependency order and **must** detect circular dependencies.

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

All modules **must** provide the following interface. Modules can be defined via a **decorator** (primary approach), a **class-based pattern** (no ABC inheritance required), or a **function call**. Implementations **MUST NOT** require modules to inherit from an abstract base class.

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
    execute_async(inputs: Map<String, Any>, context: Context) → Future<Map<String, Any>>
      # Deprecated as a separate method. Implementations typically expose a
      # single execute() method. The framework auto-detects sync vs async
      # (e.g., via inspect.iscoroutinefunction) and handles invocation
      # accordingly. execute_async is retained here for reference only.
    validate(inputs: Map<String, Any>) → ValidationResult
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

All modules **must** implement the following interface:

```yaml
module_interface:
  # Required methods
  required_methods:
    - name: "execute"
      description: "Execute module main logic"
      input: "Defined by input_schema"
      output: "Defined by output_schema"
      async_variant: "execute_async"  # Deprecated: framework auto-detects sync/async

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

Each module invocation passes a `context` parameter containing runtime context information. In cross-process scenarios, Context **must** support serialization for transport.

**Design Principle**: Only fields that the framework execution engine depends on are independent fields, everything else goes in `data` (referencing Go `context.Context`, OpenTelemetry Context design philosophy).

```yaml
context_schema:
  type: object
  properties:
    # ====== Framework engine dependencies (breaks if removed) ======

    trace_id:
      type: string
      format: uuid
      description: "Request trace ID (unique across full chain)"
      required: true

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
          enum: [user, service, agent, api_key, system]
          default: "user"
          description: "Identity type"
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

**Context Serialization Specification (Cross-process Scenarios):**

In cross-process/cross-network invocation scenarios, Context **must** be serializable to JSON format:

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

Async module state transitions **must** follow this state machine:

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
    idle → pending       : When execute_async() is called
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
    # Start async task
    execute_async:
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
- Both forms share the same parameter set, producing modules that **must** be completely equivalent in Registry/Executor/Schema behavior

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

Implementations **must** auto-generate JSON Schema from function signatures according to the following algorithm:

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

When `module()` doesn't specify `id` parameter, **must** auto-generate from function full path:

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

Implementations **must** extract module description by the following priority:

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

Implementations **should** support both sync and async functions. The framework auto-detects whether a function is sync or async and handles invocation accordingly. Both map to the module's `execute()` method; the separate `execute_async()` method is deprecated.

#### 5.11.9 Context Injection

When function parameters include `context: Context` type annotation, framework **must** auto-inject Context object, and this parameter **must not** appear in generated `input_schema`.

```python
# context parameter auto-injected, doesn't appear in Schema
@module(id="email.send")
def send_email(to: str, subject: str, body: str, context: Context) -> dict:
    print(f"trace_id: {context.trace_id}")
    return {"success": True}

# Generated input_schema only contains to, subject, body
```

#### 5.11.10 Equivalence Guarantee

Function-defined modules and class-defined modules **must** be completely equivalent in:

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

Binding files **must** be in YAML format, containing a `bindings` array:

```yaml
# bindings/email.binding.yaml
bindings:
  - module_id: "email.send"
    target: "myapp.services.email:send_email"
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
    target: "myapp.services.email:EmailService.send_template"
    description: "Send email using template"
    auto_schema: true  # Auto-generate Schema from type annotations
```

**Binding Item Field Definitions:**

| Field | Type | Required | Description |
|------|------|------|------|
| `module_id` | string | **MUST** | Module Canonical ID |
| `target` | string | **MUST** | Target callable (format: `module.path:callable_name`) |
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

Implementations **must** resolve `target` field according to the following algorithm:

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
    target: "myapp.services.email:send_email"
    schema_ref: "../schemas/email.send.schema.yaml"
```

#### 5.12.5 `auto_schema` Mode

When `auto_schema: true`, implementations **must** reuse the `generate_schema_from_function` algorithm from §5.11.4 to auto-generate Schema from target callable's type annotations.

If target callable lacks sufficient type information, **must** throw `BINDING_SCHEMA_MISSING` error.

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

- If `bindings.dir` is configured, implementations **must** scan files matching `pattern` in that directory
- If `bindings.files` is configured, implementations **must** load specified file list
- If neither configured, implementations **should** default to scanning `bindings/` directory

#### 5.12.7 Validation Rules

Implementations **must** perform the following validations when loading binding files:

1. `module_id` conforms to Canonical ID format (§2.7)
2. `target` can be resolved to valid callable
3. Schema is valid (explicitly defined or auto_schema can generate)
4. `module_id` doesn't conflict with registered modules (§2.6)
5. Binding file itself conforms to `binding.schema.json` (see `schemas/binding.schema.json`)

#### 5.12.8 Error Codes

| Error Code | Description | Trigger Condition |
|--------|------|---------|
| `BINDING_INVALID_TARGET` | target format invalid | target doesn't conform to `module.path:callable_name` format |
| `BINDING_MODULE_NOT_FOUND` | Module path can't be imported | import module_path fails |
| `BINDING_CALLABLE_NOT_FOUND` | Can't find target callable | Can't find specified function/method in module |
| `BINDING_NOT_CALLABLE` | Target not callable | Resolved object is not callable |
| `BINDING_SCHEMA_MISSING` | Schema missing | No explicit Schema and auto_schema can't generate |

### 5.13 Edge Case Handling

Implementations **must** handle module edge cases according to the following table:

#### 5.13.1 execute() Return Value Edges

| Scenario | Behavior | Level |
|------|------|------|
| `execute()` returns `None` | Throw `MODULE_EXECUTE_ERROR` ("Return value cannot be None") | **MUST** |
| `execute()` returns non-Map/dict type | Throw `MODULE_EXECUTE_ERROR` ("Return value must be Map") | **MUST** |
| Return value doesn't match `output_schema` | Throw `SCHEMA_VALIDATION_ERROR` | **MUST** |
| `execute()` throws non-`ModuleError` exception | Wrap as `MODULE_EXECUTE_ERROR` (cause points to original exception) | **MUST** |
| `execute()` returns object with non-serializable objects | **Should** log warning but don't enforce check | **SHOULD** |

#### 5.13.2 Module Dependency Loading Failures

| Scenario | Behavior | Level |
|------|------|------|
| Module in `dependencies.requires` doesn't exist | Throw `DEPENDENCY_NOT_FOUND`, refuse loading | **MUST** |
| Module in `dependencies.optional` doesn't exist | Log INFO, continue loading | **MUST** |
| Module in `dependencies.requires` fails to load | Throw `MODULE_LOAD_ERROR`, refuse loading | **MUST** |
| Module in `dependencies.optional` fails to load | Log WARN, continue loading | **MUST** |
| Reverse dependency (A depends on B, B also depends on A) | Throw `CIRCULAR_DEPENDENCY` | **MUST** |
| Indirect circular dependency (A → B → C → A) | Throw `CIRCULAR_DEPENDENCY` | **MUST** |

#### 5.13.3 Module Lifecycle Edges

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

---

## 6. ACL Specification (ACL Specification)

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

**Special patterns:**

| Pattern | Description |
|---------|-------------|
| `@external` | Matches calls with no caller (external entry points) |
| `@system` | Matches calls where identity type is `system` |
| `*` | Wildcard, matches all module IDs |

**Reserved for future use:** `id`, `actions`, `priority` fields are reserved for future specification versions and **SHOULD NOT** be used by implementations.

### 6.2 Rule Matching

Implementations **must** perform pattern matching according to the following algorithm:

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

### 6.3 Rule Evaluation Algorithm

Implementations **must** evaluate ACL rules using a **first-match-wins** strategy. Rules are evaluated in definition order (not sorted by priority). The first rule whose patterns match the caller and target determines the access decision.

```
Algorithm: evaluate_acl(caller_id, target_id, rules, default_effect, context)

Input:
  caller_id      — Caller module ID (null means external call, treated as "@external")
  target_id      — Called module ID
  rules          — Rule list (evaluated in definition order)
  default_effect — Default policy ("allow" | "deny")
  context        — Execution context (optional, used for condition evaluation)

Output:
  decision — { effect: "allow" | "deny", matched_rule: Rule | null }

Steps:
  1. effective_caller ← caller_id ?? "@external"
  2. For each rule ∈ rules (in definition order):
     a. caller_matched ← false
        For each pattern ∈ rule.callers:
          If pattern is "@external" and caller_id is null → caller_matched ← true; break
          If pattern is "@system" and context.identity.type == "system" → caller_matched ← true; break
          If match_pattern(pattern, effective_caller) → caller_matched ← true; break
     b. target_matched ← false
        For each pattern ∈ rule.targets:
          If match_pattern(pattern, target_id) → target_matched ← true; break
     c. If caller_matched and target_matched:
        If rule.conditions is not empty:
          If not evaluate_conditions(rule.conditions, context) → continue
        → Return { effect: rule.effect, matched_rule: rule }
  3. Return { effect: default_effect, matched_rule: null }

Complexity: O(R × P), where R is number of rules, P is average patterns per rule
```

### 6.4 Pattern Specificity Scoring

When further distinguishing rules within same priority is needed, implementations **should** calculate pattern specificity score:

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

System module access is governed by three independent layers. Each layer operates regardless of the others:

```
Layer 1: Activation (Config)
  sys_modules.enabled = false (default)
  → system.* modules are NOT registered
  → No system module exists in the registry — nothing to call or list

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

Adapters and UI layers **SHOULD NOT** introduce additional independent permission switches. The three layers above are sufficient — adding adapter-level switches creates shadow permission systems that can diverge from the actual ACL state.

#### 6.6.4 UI Adapter Guidelines

Explorer-style UIs that display module listings **SHOULD**:

1. **Classify by prefix**: Use `module_id.startswith("system.")` to separate system modules from user modules in the UI.
2. **Reflect backend state**: If no `system.*` modules appear in the listing, hide management UI elements. If they appear, show them.
3. **Not duplicate authorization**: The UI should faithfully reflect what the backend exposes. If ACL blocks a call, the Executor returns `ACL_DENIED` — the UI handles this error gracefully, rather than pre-filtering modules with its own logic.

This "backend-driven visibility" approach ensures the UI always matches the actual permission state without maintaining a parallel authorization model.

---

## 7. Approval System (Approval System)

### 7.1 Overview

The Approval System provides **runtime enforcement** of the `requires_approval` annotation. While annotations are generally hints for AI/LLM clients, `requires_approval` is unique: when an `ApprovalHandler` is configured, the Executor **blocks execution** of modules marked `requires_approval=true` until explicit approval is granted.

This mechanism is the bridge between annotation-level metadata and runtime governance — making apcore the only framework that **enforces** Human-in-the-Loop approval rather than merely hinting at it.

**Relationship to ACL:**

| Concern | ACL (§6) | Approval System (§7) |
|---------|----------|----------------------|
| Question answered | "Is this caller **allowed** to invoke this module?" | "Does this **invocation** need human sign-off?" |
| Mechanism | Pattern-based rule matching | Pluggable handler with external interaction |
| Timing | Step 4 in Executor pipeline | Step 5 in Executor pipeline (after ACL) |
| Interaction | None (deterministic rule evaluation) | May involve user dialog, webhook, or agent confirmation |

A caller may pass ACL (they have the role to call `deploy.prod`) but still require approval for each invocation (because the module is destructive).

### 7.2 ApprovalHandler Protocol

Implementations **must** define an `ApprovalHandler` protocol (or interface) with the following contract:

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

Both methods **must** be asynchronous (async/await) in implementations that support it.

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

The Approval Gate is Step 5 in the Executor's pipeline, between ACL Enforcement and Input Validation:

```
Executor Pipeline:
  Step  1: Context Creation
  Step  2: Safety Checks
  Step  3: Module Lookup
  Step  4: ACL Enforcement
  Step  5: Approval Gate
  Step  6: Input Validation
  Step  7: Middleware Before Chain
  Step  8: Module Execution
  Step  9: Output Validation
  Step 10: Middleware After Chain
  Step 11: Result Return
```

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
- When no `ApprovalHandler` is configured, Step 5 is **completely skipped** — backward compatible with existing code.
- The `_approval_token` mechanism (Phase B) allows clients to retry after external approval without re-triggering the approval flow.
- The `_approval_token` key **must** be removed from arguments before passing to subsequent steps.

### 7.5 Error Types

Implementations **must** define the following error types under `ModuleError`:

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

Each approval error **must** carry a `result` field containing the full `ApprovalResult`.

### 7.6 Built-in Handlers

Implementations **should** provide these built-in handlers:

| Handler | Behavior | Use Case |
|---------|----------|----------|
| `AlwaysDenyHandler` | Always returns `rejected` | Default safe behavior when no handler configured but enforcement desired |
| `AutoApproveHandler` | Always returns `approved` | Testing and development |
| `CallbackApprovalHandler` | Delegates to a user-provided async callback | Custom approval logic |

### 7.7 Protocol Bridge Handlers

Protocol bridges (such as apcore-mcp, apcore-a2a) **should** provide handlers that leverage their protocol's interaction capabilities:

| Bridge | Handler | Mechanism |
|--------|---------|-----------|
| apcore-mcp | `ElicitationApprovalHandler` | Uses MCP Elicitation protocol to show confirmation dialog in MCP clients |
| apcore-a2a | `A2AApprovalHandler` | Uses A2A protocol interaction to request confirmation from calling agent |

These handlers are **not** part of the apcore core specification — they are provided by the respective bridge packages.

### 7.8 Phased Implementation

#### Phase A: Synchronous Approval (Required)

- `request_approval()` blocks until a decision is reached or timeout occurs.
- Suitable for interactive scenarios (MCP client dialogs, agent-to-agent confirmation).
- **All conformant implementations must support Phase A.**

#### Phase B: Asynchronous Approval (Optional)

- `request_approval()` may return `status: "pending"` with an `approval_id`.
- Client retries the tool call with `_approval_token` in arguments.
- `check_approval(approval_id)` returns the current status.
- Suitable for long-running approval workflows (Slack, email, dashboard).
- **Phase B is optional but recommended for production deployments.**

### 7.9 Conformance

| Level | Requirement |
|-------|-------------|
| **Level 1 (Basic)** | `ApprovalHandler` protocol defined; Executor skips gate when handler is null |
| **Level 2 (Standard)** | Step 5 implemented in `call()`, `call_async()`, and `stream()` paths; `AlwaysDenyHandler` and `AutoApproveHandler` provided |
| **Level 3 (Full)** | Phase B support (`check_approval`, `_approval_token`); `CallbackApprovalHandler` provided; approval audit events emitted |

---

## 8. Error Handling Specification (Error Handling Specification)

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
- **Self-Evolution**: The system continuously adapts through health monitoring, event-driven feedback loops, and runtime reconfiguration (see §6.6, §10).

**Field semantics:**

| Field | Type | Purpose |
|-------|------|---------|
| `retryable` | `boolean \| null` | Whether retrying the same call may succeed. Each error code has a default value (see §8.6). Callers may override. `null` means "depends on context". |
| `ai_guidance` | `string \| null` | Machine-readable guidance for AI agents, e.g. `"validate input schema before retry"`, `"check module registry for available alternatives"`. |
| `user_fixable` | `boolean \| null` | Whether the end-user (non-developer) can resolve the issue, e.g. fixing a typo in input vs. a server misconfiguration. |
| `suggestion` | `string \| null` | Human-readable actionable suggestion, e.g. `"Check that the table name contains only lowercase letters and underscores"`. |

**Serialization rules:**

- Implementations **must** use sparse serialization: fields with `null` values **should** be omitted from the serialized output.
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
    description: "Schema circular reference detected"
    http_status: 500

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
    description: "Binding target format invalid"
    http_status: 500
  BINDING_MODULE_NOT_FOUND:
    description: "Binding target module path can't be imported"
    http_status: 500
  BINDING_CALLABLE_NOT_FOUND:
    description: "Binding target callable not found"
    http_status: 500
  BINDING_NOT_CALLABLE:
    description: "Binding target not callable"
    http_status: 500
  BINDING_SCHEMA_MISSING:
    description: "Binding Schema missing"
    http_status: 500
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

Implementations **must** propagate errors according to the following algorithm:

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
    - "Module-thrown errors **must** be wrapped as ModuleError"
    - "Original error **must** be saved in cause field"
    - "Error code prefixed with MODULE_"

  # Error context
  context:
    - "Error **must** contain trace_id"
    - "Error **should** contain occurrence location (module_id)"
    - "**Must** support error chain tracing"
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
    - "Framework startup **must** collect all module error codes"
    - "**Must** detect error code conflicts"
    - "**Should** generate error code documentation"

  # Framework error code priority
  priority:
    - "Framework error codes (MODULE_/SCHEMA_/ACL_/GENERAL_) **must** be reserved, modules **must not** use them"
    - "Module custom error codes **must not** conflict with framework error codes"

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
  trace_id: "550e8400-e29b-41d4-a716-446655440000"
  timestamp: "2026-02-05T10:30:00Z"
  cause: null  # Or nested error object
  retryable: false
  user_fixable: true
  suggestion: "Table names must use only lowercase letters and underscores. Change 'User-Info' to 'user_info'."
  # ai_guidance: omitted (null) — sparse serialization
```

### 8.6 Retry Semantics

Implementations **must not** default retry failed module invocations. Retry behavior **must** be explicitly controlled by caller or middleware.

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
| `CALL_DEPTH_EXCEEDED` | **No** | Call chain structure issue, retry won't change |
| `CIRCULAR_CALL` | **No** | Call chain structure issue, retry won't change |
| `CALL_FREQUENCY_EXCEEDED` | **No** | Call chain structure issue, retry won't change |
| `GENERAL_INVALID_INPUT` | **No** | Invalid input, caller must fix before retry |
| `FUNC_MISSING_TYPE_HINT` | **No** | Code-level issue, needs developer fix |
| `FUNC_MISSING_RETURN_TYPE` | **No** | Code-level issue, needs developer fix |
| `BINDING_INVALID_TARGET` | **No** | Binding format error, needs config fix |
| `BINDING_MODULE_NOT_FOUND` | **No** | Binding target module missing, needs config fix |
| `BINDING_CALLABLE_NOT_FOUND` | **No** | Binding target callable missing, needs code fix |
| `BINDING_NOT_CALLABLE` | **No** | Binding target not callable, needs code fix |
| `BINDING_SCHEMA_MISSING` | **No** | Schema missing for binding, needs code fix |
| `BINDING_FILE_INVALID` | **No** | Binding file parse error, needs config fix |
| `CIRCULAR_DEPENDENCY` | **No** | Module dependency cycle, needs architecture fix |
| `MIDDLEWARE_CHAIN_ERROR` | **No** | Middleware failed, needs code fix |
| `VERSION_INCOMPATIBLE` | **No** | Version mismatch, needs upgrade or config fix |
| `ERROR_CODE_COLLISION` | **No** | Error code conflict, needs code fix |

Implementations **should** use this table as the default `retryable` value for each error subclass. Callers may override the default on a per-instance basis.

> **Note:** `GENERAL_NOT_IMPLEMENTED` and `DEPENDENCY_NOT_FOUND` are included in the hierarchy above. Both are non-retryable by default.

Retry middleware (if implemented) **should**:
- Only retry errors marked as retryable
- Only auto-retry modules with `annotations.idempotent == true`
- Use exponential backoff strategy
- Set max retry count limit (**should** not exceed 5 times)

### 8.7 Error Hierarchy

All framework errors **must** extend from a single `ModuleError` base class using a flat hierarchy. Implementations use a flat hierarchy under `ModuleError` for simplicity.

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
├── ACLDeniedError                 # ACL_DENIED — Permission denied
├── ACLRuleError                   # ACL_RULE_ERROR — ACL rule error
├── FuncMissingTypeHintError       # FUNC_MISSING_TYPE_HINT — Function parameter missing type annotation
├── FuncMissingReturnTypeError     # FUNC_MISSING_RETURN_TYPE — Function missing return type annotation
├── BindingInvalidTargetError      # BINDING_INVALID_TARGET — target format invalid
├── BindingModuleNotFoundError     # BINDING_MODULE_NOT_FOUND — Module path can't be imported
├── BindingCallableNotFoundError   # BINDING_CALLABLE_NOT_FOUND — Can't find target callable
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

Each error class carries a `code` attribute set to the corresponding error code string (e.g., `MODULE_NOT_FOUND`). Implementations **must** ensure all framework-thrown errors are instances of `ModuleError`. Module custom errors **should** also extend `ModuleError` directly.

---

## 9. Configuration Specification (Configuration Specification)

### 9.1 Framework Configuration

apcore.yaml is the core configuration file of the framework. Implementations **must** validate configuration files according to the following JSON Schema.

**apcore.yaml Complete JSON Schema Definition:**

```yaml
# apcore.yaml — Complete configuration structure and constraints

$schema: "https://apcore.dev/config/v1"
version: "1.0.0"                    # MUST, configuration version

# Project information
project:
  name: "my-ai-project"             # MUST, project name (pattern: ^[a-z][a-z0-9_-]*$)
  version: "0.1.0"                   # SHOULD, project version (semver)

# Extension configuration
extensions:
  root: "./extensions"               # MUST, extension root directory
  auto_discover: true                # SHOULD, auto-discovery (default: true)
  lazy_load: true                    # MAY, lazy loading (default: true)
  follow_symlinks: false             # MUST NOT default true (default: false)
  max_depth: 8                       # SHOULD, max scan depth (default: 8, max: 16)
  ignore_patterns:                   # MAY, additional ignore patterns
    - "*.test.*"
    - "*.spec.*"

# Schema configuration
schema:
  root: "./schemas"                  # MUST, Schema directory
  strategy: "yaml_first"             # SHOULD, loading strategy (yaml_first|native_first|yaml_only)
  validation:
    strict: true                     # SHOULD, strict mode (default: true)
    coerce_types: true               # MAY, type coercion (default: true)
  max_ref_depth: 32                  # MAY, $ref max recursion depth (default: 32)

# ACL configuration
acl:
  root: "./acl"                      # MUST, ACL configuration directory
  default_effect: "deny"             # MUST, default policy (deny|allow, default: deny)
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
    enabled: true                    # SHOULD (default: true)
    sampling_rate: 1.0               # MAY (0.0-1.0, default: 1.0)
    exporter: "stdout"               # MAY (stdout|otlp|jaeger)
  metrics:
    enabled: true                    # MAY (default: true)
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

#### 8.1.1 Default Values Summary

Implementations **must** follow these default value conventions:

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
| `observability.enabled` | `true` | `true`/`false` | Observability master switch |
| `observability.exporter` | `"stdout"` | `stdout`/`prometheus`/`otlp` | Default exporter |
| `bindings.dir` | `"./bindings"` | Valid directory path | Binding file directory |
| `bindings.pattern` | `"*.binding.yaml"` | glob pattern | Binding file matching pattern |
| `id_map.auto_detect` | `true` | `true`/`false` | Auto ID mapping detection |

**Note**:
- Configuration values exceeding ranges **must** be rejected in `validate_config()` (algorithm A12)
- Implementations use a **dual-timeout model**: `default_timeout` applies to each individual module execution, while `global_timeout` applies to the entire call chain from root invocation. If either timeout is exceeded, a `MODULE_TIMEOUT` error is raised.
- `timeout = 0` means disable that timeout, implementations **should** log WARN
- `max_call_depth` and `max_module_repeat` used for call chain safety checks (algorithm A20)

### 9.2 Environment Variable Override

Implementations **must** support overriding configuration file values through environment variables.

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
  schema.validation.strict → APCORE_SCHEMA_VALIDATION_STRICT
  acl.default_effect     → APCORE_ACL_DEFAULT_EFFECT
  logging.level          → APCORE_LOGGING_LEVEL
  observability.tracing.enabled → APCORE_OBSERVABILITY_TRACING_ENABLED
```

### 9.3 Configuration Validation Algorithm

Implementations **must** validate configuration at startup:

```
Algorithm: validate_config(config)

Input:
  config — Merged configuration object (env + file + defaults)

Output:
  validated_config — Validated configuration, or throw CONFIG_INVALID error

Steps:
  1. For each required field (MUST):
     If missing → Throw CONFIG_INVALID with missing field path
  2. Type validation:
     For each field, validate value type conforms to Schema definition
  3. Constraint validation:
     - extensions.root must be valid directory path
     - schema.root must be valid directory path
     - acl.default_effect must be "allow" or "deny"
     - observability.tracing.sampling_rate must be in [0.0, 1.0] range
     - extensions.max_depth must be in [1, 16] range
  4. Semantic validation:
     - If extensions.auto_discover == true and extensions.root doesn't exist → Warning
     - If schema.strategy == "yaml_only" and schema.root doesn't exist → Error
  5. Return validated_config
```

---

## 10. Observability Specification (Observability Specification)

### 10.1 Tracing

Based on OpenTelemetry specification:

```yaml
tracing:
  # Trace context
  context:
    trace_id:
      format: "uuid-v4 or w3c trace-id"
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

Implementations **should** provide a `UsageCollector` that tracks per-module call statistics for the `system.usage.*` system modules:

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

A `UsageMiddleware` **should** automatically record call data into the `UsageCollector` during Step 10 (Middleware After). The collected data is consumed by `system.usage.summary` and `system.usage.module` system modules.

### 10.5 Trace ID Format

trace_id **must** use UUID v4 format. In distributed scenarios, **recommended** to be compatible with W3C Trace Context standard.

```yaml
trace_id_spec:
  format: "uuid-v4"                    # MUST
  example: "550e8400-e29b-41d4-a716-446655440000"

  distributed:
    w3c_trace_context: "RECOMMENDED"   # Recommended for distributed scenarios
    traceparent_header: "traceparent: 00-{trace_id_hex}-{span_id_hex}-{flags}"

  generation:
    - "Auto-generated by Executor at top-level calls"
    - "Child calls inherit parent call's trace_id"
    - "MUST NOT allow externally provided unvalidated trace_id"
```

### 10.5 Sensitive Data Redaction

Implementations **must** redact fields marked as `x-sensitive` in logs and trace outputs.

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

### 10.6 Sampling Strategy

Implementations **should** support the following sampling strategies:

| Strategy | Configuration Value | Description |
|------|--------|------|
| Full sampling | `sampling_rate: 1.0` | Record all calls (development environment **recommended**) |
| Proportional sampling | `sampling_rate: 0.1` | Record 10% of calls |
| Error-first | `sampling_strategy: "error_first"` | Always record error calls, successful calls by proportion |
| Off | `sampling_rate: 0.0` | Don't record trace info |

Sampling decision **must** be made at call chain root node, child calls **must** inherit parent call's sampling decision.

### 10.7 Span Naming Convention

Implementations **should** follow these Span naming conventions:

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
    - "caller_id"        # SHOULD
```

---

## 11. Extension Mechanism (Extension Mechanism)

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
      params: [module_id, error, context]
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
      use_case: "Remote loading, dynamic compilation; Function-based module loading; Binding file target resolution and module loading"
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
> Implementations declaring Level 2 conformance use the actual names (`discoverer`, `middleware`, `acl`, `span_exporter`, `module_validator`) in `ExtensionManager`.

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

Middleware chain execution **must** follow this state machine:

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

Implementations **should** support the following extension points, each **must** define clear interface contract:

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

> **NOTE:** The interface contracts above use the original theoretical names. See the mapping table in §11.3 for the actual extension point names used in SDK implementations (`discoverer`, `middleware`, `acl`, `span_exporter`, `module_validator`).

### 11.7 Extension Loading Order

Implementations **must** load extensions according to the following algorithm:

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

Implementations **must** handle middleware edge cases according to the following table:

#### 10.8.1 on_error Cascade

| Scenario | Behavior | Level |
|------|------|------|
| `on_error()` itself throws exception | Log ERROR, continue to next `on_error()` in chain | **MUST** |
| `on_error()` returns non-`None` value | Stop propagation, use return value as module final output | **MUST** |
| `on_error()` returns `None` | Continue propagating error downward | **MUST** |
| All `on_error()` return `None` | Throw original error to caller | **MUST** |

#### 10.8.2 before() Edges

| Scenario | Behavior | Level |
|------|------|------|
| `before()` returns `None` | Keep `inputs` unchanged, continue chain | **MUST** |
| `before()` returns partial field dict | Replace `inputs` entirely | **MUST** |
| `before()` returns non-dict type | Throw `GENERAL_INTERNAL_ERROR` | **MUST** |
| `before()` throws `ModuleError` | Trigger `on_error()` chain, skip module execution | **MUST** |
| `before()` modifies `context.data` | Allowed, modifications visible to subsequent middleware and module | **MUST** |

#### 10.8.3 after() Edges

| Scenario | Behavior | Level |
|------|------|------|
| `after()` returns `None` | Keep `result` unchanged, continue chain | **MUST** |
| `after()` returns partial field dict | Replace `result` entirely | **MUST** |
| `after()` throws `ModuleError` | Trigger `on_error()` chain, replace original result | **MUST** |
| `after()` returns value not matching `output_schema` | Trigger `SCHEMA_VALIDATION_ERROR` | **MUST** |

#### 10.8.4 Timeout Related

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

## 12. SDK Implementation Guide (SDK Implementation Guide)

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

Following are formalized interface definitions for each core component (language-agnostic pseudocode). All SDK implementations **must** provide equivalent implementations of these interfaces.

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
   * @param module_id — Canonical ID
   * @param inputs    — Input parameters (conform to input_schema)
   * @param context   — Execution context
   * @return output   — Output result (conform to output_schema)
   * @throws INPUT_VALIDATION_FAILED  — Input validation failed
   * @throws OUTPUT_VALIDATION_FAILED — Output validation failed
   * @throws ACL_DENIED               — Permission denied
   * @throws MODULE_EXECUTION_ERROR   — Module execution exception
   */
  call(module_id: String, inputs: Map, context: Context) → Map

  /**
   * [SHOULD] Non-destructive preflight check through Steps 1–6 of the
   * execution pipeline without invoking module code or middleware.
   *
   * Runs: context creation, safety checks, module lookup, ACL enforcement,
   * approval detection (report only, MUST NOT invoke ApprovalHandler),
   * and input schema validation.
   *
   * MUST NOT: execute module code, run middleware, or modify external state.
   *
   * All check failures are collected into the result rather than thrown,
   * so the caller can see every problem in a single round-trip.
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
  check: String              // "module_id" | "module_lookup" | "call_chain" | "acl" | "approval" | "schema"
  passed: Boolean
  error: Map?                // Error details when passed=false; null when passed=true

Type: PreflightResult
  valid: Boolean             // True only if ALL checks passed
  checks: List<PreflightCheckResult>
  requires_approval: Boolean // True if module has requires_approval annotation

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
   * Note: Priority is determined by registration order, not by an explicit
   * priority parameter. Middleware registered first executes first (before)
   * and last (after), following the onion model.
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
   * @param name       — Span name (follows §10.7 naming convention)
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
- The complete result is the shallow merge of all yielded chunks (left-to-right object spread).
- `execute()` MUST remain implemented as the non-streaming fallback.
- Module descriptors SHOULD declare `annotations.streaming = true` when `stream()` is provided.

**Executor.stream() pipeline:**

1. Steps 1–7 identical to `call()`: context creation, safety checks, module lookup, ACL, approval gate, input validation, before-middleware.
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

All SDK implementations **must** satisfy the following requirements regardless of language:

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

Each SDK implementation **must** pass the following consistency test suite to ensure cross-language behavior consistency:

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
   - trace_id is valid UUID v4
   - Sensitive data redaction
   - Structured log format

7. Preflight (validate) Tests:
   - Valid module + valid inputs → PreflightResult.valid=true, all checks passed
   - Invalid module_id format → module_id check failed, early return
   - Unknown module → module_lookup check failed, early return
   - ACL denial → acl check failed, valid=false
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

Each SDK implementation **should** use the idiomatic schema validation, async model, and package management conventions of its target language. Specific library choices are documented in each SDK's own repository.

### 12.7 Concurrency Model Specification

This section defines apcore's concurrency model and thread safety requirements, ensuring SDK implementers correctly implement the framework in multi-threaded/coroutine environments.

#### 12.7.1 Module Instance Lifecycle

**Singleton Model (MUST)**:

- Each `module_id` **must** correspond to unique module instance (singleton)
- Instance created at `discover()` or first invocation, destroyed at `unregister()` or app shutdown
- `on_load()` hook **must** be called only once during instance lifecycle

**Reentrancy (MUST)**:

- `execute()` method **must** support concurrent reentrant calls (thread-safe)
- Module internal state (if any) **should** use thread-safe mechanisms (locks, atomic variables, etc.) for protection
- Implementations **must not** assume `execute()` calls are serial

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

- `context.data` **must** be the same dict/Map object across entire call chain (reference sharing)
- Parent module modifications to `context.data` visible to child modules, vice versa
- When `child()` creates new Context, `data` field **must** copy reference (not deep copy)

**Isolation (MUST)**:

- Different top-level `call()` invocations **must** use independent `context.data` instances
- Concurrently executing call chains **must not** share `context.data` (avoid race conditions)

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

- If `context.data` might be accessed by multiple threads (e.g., async middleware), **should** use thread-safe Map implementation
- Python's `dict` is partially thread-safe in CPython (GIL protected), but **should** avoid relying on implementation details

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
- `on_suspend()` return value **must** be JSON-serializable (no functions, connections, file handles)
- `on_resume()` **must** tolerate missing or extra keys (new version may have different state shape)
- If `on_suspend()` raises, log ERROR and proceed with unload (state is lost)
- If `on_resume()` raises, log ERROR and continue (module starts with fresh state)
- Framework **must not** call `on_resume()` if `on_suspend()` returned null or was not implemented

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

- SDK **should** prefer cooperative cancellation mechanism (e.g., Python `asyncio.CancelledError`, Go `context.Context`)
- Module **should** check cancellation signal and actively exit

**Forced Termination (MAY)**:

- If module doesn't respond to cancellation signal, SDK **may** forcibly terminate execution thread/coroutine
- After forced termination **must** log ERROR including `module_id` and timeout duration

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

- Each `call()`'s middleware chain execution **must** not interleave with other `call()`'s chain execution
- Middleware chain's before → execute → after sequence **must** complete atomically (not interrupted by other calls)

**Instance Sharing (MUST)**:

- Middleware instances shared at application level (similar to module singleton)
- Middleware **must** be thread-safe, support concurrent calls

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

- Middleware **should** store call-level state through `context.data` (e.g., request ID, timer)
- **Must not** use middleware instance variables to store call-level state (causes race conditions)

#### 12.7.7 Sync/Async Mixing

**Bridging Strategy (MUST)**:

Implementations **must** support mixed calls of sync and async modules, bridging according to these rules:

| Caller | Called Module | Bridging Strategy | Description |
|--------|-----------|---------|------|
| Sync | Sync | Direct call | No overhead |
| Sync | Async | Block and wait (await) | Sync caller blocks until async completes |
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

- Sync→Async bridging blocks caller thread, **should** avoid frequent use in async contexts
- Async→Sync bridging needs thread pool, **should** configure reasonable thread pool size (default CPU cores × 2)

#### 12.7.8 Resource Cleanup Guarantees

Implementations **must** guarantee resource cleanup according to this table:

| Exit Scenario | `on_unload()` Called | `finally` Block Executed | File/Network Closed | Memory Released | Level |
|---------|-------------------|-----------------|--------------|---------|------|
| Normal completion | ✅ Called | ✅ Executed | ✅ Guaranteed | ✅ GC reclaimed | **MUST** |
| Timeout (cooperative cancel) | ✅ Called | ✅ Executed | ✅ Guaranteed | ✅ GC reclaimed | **MUST** |
| Timeout (forced termination) | ⚠️ May not call | ⚠️ May not execute | ⚠️ May leak | ✅ GC reclaimed (eventually) | **MAY** |
| Process crash/kill | ❌ Not called | ❌ Not executed | ❌ Leak | ❌ Lost | N/A |

**Best Practices:**

- Modules **should** use RAII (Resource Acquisition Is Initialization) pattern to manage resources
- **Should** acquire resources in `on_load()`, release in `on_unload()`
- **Should** avoid opening long-term resources in `execute()` (e.g., database connections), use connection pools instead

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

The `validate()` preflight method (§12.2, SHOULD level) runs Steps 1–6 of the Executor pipeline
(plus optional module-level preflight Check 7) without executing module code or middleware. This section provides language-specific guidance for
SDK implementers.

#### 12.8.1 Design Principles

1. **Collect, don't throw.** All check failures are appended to a `checks` list. The caller sees every problem in one call.
2. **Early return only when subsequent checks are meaningless.** module_id format failure or module-not-found justifies early return because later checks require a valid module reference.
3. **Reuse existing internals.** validate() calls the same helper functions used by the `call()` pipeline (regex check, registry lookup, ACL check, schema validation). No new capabilities are required.
4. **Duck-type backward compatibility.** PreflightResult SHOULD expose `.valid` (Boolean) and `.errors` (List) so existing consumers of the old ValidationResult continue to work.

#### 12.8.2 Error Handling Mapping

Each check in validate() calls the same helper functions used by the `call()` pipeline. Failures are appended to the `checks` list rather than thrown/returned immediately.

| Error Model | Pattern |
|-------------|---------|
| **Exception-based** (try/catch) | `try { helper(); push(passed) } catch(e) { push(failed, e) }` |
| **Error-return** (Go, Rust) | `if err := helper(); err != nil { push(failed, err) } else { push(passed) }` |

> **Note:** Error-return patterns are more natural than try/catch for this "collect all errors" flow.

#### 12.8.3 PreflightResult Type

`PreflightResult` **must** contain the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `valid` | boolean | `true` if all checks passed |
| `checks` | list of `PreflightCheckResult` | Ordered list of check results |
| `requires_approval` | boolean | Whether the module requires approval |
| `errors` (computed) | list of error objects | Filtered view: only checks where `passed` is `false` |

#### 12.8.4 PreflightCheckResult Type

`PreflightCheckResult` **must** contain the following fields:

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
- If `preflight()` returns an empty list or is not defined, no `module_preflight` check is added.
- If `preflight()` raises an exception, the exception is caught and reported as a warning (not a failure).

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

## 13. Versioning (Versioning)

### 13.1 Version Number Specification

```
{major}.{minor}.{patch}[-{prerelease}]

major: Incompatible API changes
minor: Backward-compatible feature additions
patch: Backward-compatible bug fixes
prerelease: draft, alpha, beta, rc
```

### 13.2 Compatibility Promise

- **Within major version**: Protocol backward compatible
- **Schema evolution**: Support version declaration, old version Schema readable by new SDK
- **Deprecation policy**: Keep at least 2 minor versions for deprecation period

### 13.3 Version Negotiation Algorithm

When SDK loads configuration or Schema, **must** perform version negotiation:

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

When Schema version changes, implementations **should** support automatic migration:

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

1. **SDK must** ignore unknown configuration fields and Schema properties (forward compatibility foundation)
2. **SDK must** provide reasonable defaults for all new fields (backward compatibility foundation)
3. **SDK should** gracefully handle unknown error codes
4. **SDK must not** remove published public APIs in minor/patch versions

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

Each language SDK **should** provide idiomatic module definition syntax. The following illustrates the general patterns:

| Pattern | Class-based | Decorator/Attribute | Function Call | External Binding |
|------|------------|--------------------------|-------------------------|-----------------|
| Description | Inherit/implement Module interface | Language-native annotation | Wrap existing callable | YAML binding file |
| Example (Python) | `class M(Module)` | `@module(id=...)` | `module(fn, id=...)` | YAML |
| Example (TypeScript) | `class M extends Module` | `@module({id: ...})` | `module(fn, {id: ...})` | YAML |

---

## Revision History

| Version | Date | Change Description |
|------|------|----------|
| 1.0.0-draft | 2026-02-05 | Initial draft |
| 1.1.0-draft | 2026-02-07 | Added §5.11 Function-based Module Definition, §5.12 External Schema Binding, Appendix E Module Definition Methods Comparison |
| 1.2.0-draft | 2026-02-09 | Revised §4.3 supplemented x-llm-description usage guide; Added §4.16 Strict Mode Export, §4.17 Export Profile |
| 1.3.0-draft | 2026-03-01 | Added §7 Approval System (ApprovalHandler protocol, Executor Step 4.5, error types, built-in and protocol bridge handlers, phased implementation, conformance levels); Updated §4.4 requires_approval annotation to reference runtime enforcement; Added APPROVAL_DENIED/TIMEOUT/PENDING error codes to §8; Renumbered §7–§13 → §8–§14 |
| 1.4.0-draft | 2026-03-06 | Refined Executor pipeline — Approval Gate is now Step 5, subsequent steps shifted; Added Executor.validate() [SHOULD] to §12.2 with PreflightResult/PreflightCheckResult types for non-destructive preflight checks through Steps 1–6; Updated §7.4, §7.9, streaming protocol references to match new numbering; Added §12.8 Executor.validate() Cross-Language Implementation Guide (error handling mapping, type mapping for Python/TypeScript/Go/Rust/Java/C/C++, schema library requirements, naming conventions); Added C/C++ and TypeScript to §12.6; Added validate() preflight to §12.3 requirements table; Added Preflight Tests to §12.4 consistency test suite |
