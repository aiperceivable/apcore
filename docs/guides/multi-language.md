---
description: "Developing apcore modules across Python, Rust, Go, Java, and TypeScript using YAML Schema as a shared cross-language contract for consistent, interoperable module interfaces."
---

# apcore — Multi-Language Development Guide

> Use YAML Schema as a shared contract to develop apcore modules in Python, Rust, Go, Java, TypeScript, and other languages.

## 1. Overview

One of the core designs of apcore is **cross-language interoperability**. Using YAML Schema as a shared contract, SDKs in different languages can load the same Schema definitions to achieve consistent module interfaces.

**Cross-Language Architecture:**

```
                    YAML Schema (Shared Contract)
                          │
          ┌───────┬───────┼───────┬───────┐
          ▼       ▼       ▼       ▼       ▼
       Python   Rust     Go     Java    TypeScript
        SDK      SDK     SDK     SDK      SDK
          │       │       │       │       │
          └───────┴───────┴───────┴───────┘
                          │
                    Canonical ID
                (Unified Addressing System)
```

**Core Principles:**

| Principle | Description |
|------|------|
| **Schema is the Source of Truth** | YAML Schema files define the input/output structure of modules, with each language SDK generating or loading types from them |
| **Canonical ID is the Unified Address** | All languages use the same dot-separated snake_case ID to reference modules |
| **Type Mapping has Standards** | Clear mapping table from JSON Schema types to language-specific types |
| **Behavior Through Annotations** | Module behavior annotations (readonly, destructive, etc.) are universal across languages |

---

## 2. Cross-Language Development Model

### 2.1 How apcore Supports Multiple Languages

apcore achieves cross-language support through the following mechanisms:

1. **YAML Schema Sharing**: Module `input_schema` and `output_schema` are defined in YAML files, read by all language SDKs
2. **Canonical ID**: Module IDs use language-agnostic dot-separated snake_case format (e.g., `executor.validator.db_params`)
3. **ID Map**: Converts local naming conventions of each language (PascalCase, camelCase, etc.) to Canonical ID
4. **JSON Schema Draft 2020-12**: Schema is based on international standards, with mature validation libraries available in all languages

```
Development Workflow:
  1. Define YAML Schema → schemas/executor/validator/db_params.schema.yaml
  2. Each language SDK loads the Schema
  3. Implement Module interface (execute method)
  4. Framework automatically handles Schema validation, ACL checks, etc.
```

### 2.2 YAML Schema as a Shared Contract

```yaml
# schemas/executor/email/send_email.schema.yaml
# This file is shared by SDKs in all languages

$schema: "https://apcore.dev/schema/v1"
version: "1.0.0"
module_id: "executor.email.send_email"

description: |
  Email sending module.
  Sends emails via SMTP protocol.

input_schema:
  type: object
  properties:
    to:
      oneOf:
        - type: string
          format: email
        - type: array
          items:
            type: string
            format: email
      description: "Recipient(s)"
    subject:
      type: string
      maxLength: 200
      description: "Email subject"
    body:
      type: string
      description: "Email body (plain text)"
    html:
      type: string
      description: "Email body (HTML)"
    smtp_host:
      type: string
      description: "SMTP server address"
    smtp_port:
      type: integer
      description: "SMTP server port (default 587)"
      default: 587
  required: [to, subject]
  additionalProperties: false

output_schema:
  type: object
  properties:
    success:
      type: boolean
      description: "Whether the send was successful"
    message_id:
      type: string
      description: "Message ID"
  required: [success]
```

### 2.3 Canonical ID as Unified Addressing

Canonical ID is the globally unique identifier for a module, using a language-agnostic format:

```
Format: dot-separated snake_case
Regex: ^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$
Max length: 192 characters

Examples:
  executor.validator.db_params
  orchestrator.engine.task_flow
  api.handler.task_submit
  common.util.sql_parser
```

Each language uses the same Canonical ID to reference modules:

=== "Python"

    ```python
    result = executor.call("executor.email.send_email", inputs, context)
    ```

=== "TypeScript"

    ```typescript
    const result = await executor.call("executor.email.send_email", inputs, context);
    ```

=== "Rust"

    ```rust
    let result = executor.call("executor.email.send_email", &inputs, &context)?;
    ```

=== "Go"

    ```go
    result, err := executor.Call("executor.email.send_email", inputs, ctx)
    ```

=== "Java"

    ```java
    Map<String, Object> result = executor.call("executor.email.send_email", inputs, context);
    ```

---

## 3. YAML Schema Sharing

### 3.1 Schema Files as the Source of Truth

In multi-language projects, YAML Schema files are the **Single Source of Truth**. Each language SDK should generate or load type definitions from Schema files, rather than writing them manually.

```
schemas/
├── executor/
│   ├── email/
│   │   └── send_email.schema.yaml
│   └── validator/
│       └── db_params.schema.yaml
├── orchestrator/
│   └── engine/
│       └── task_flow.schema.yaml
└── common/
    ├── error.schema.yaml
    └── pagination.schema.yaml
```

### 3.2 How Each Language SDK Consumes Schema

SDKs can consume YAML Schema files via two strategies:

| Strategy | Best For | Description |
|------|----------|------|
| **Code Generation** | Compiled languages (Rust, Go, Java, etc.) | Generate native types from YAML Schema at build time. Type-safe, zero runtime overhead. |
| **Runtime Loading** | Dynamic languages (Python, TypeScript, etc.) | Load and validate YAML Schema at runtime. Flexible, no build step needed. |

Each SDK **should** document its schema consumption approach in its own repository.

---

## 4. SDK Implementation

Each language SDK **should** implement the Module interface following the language's idiomatic patterns. SDK-specific examples, patterns, and library choices are documented in each SDK's own repository.

**Key requirements for all SDKs:**

| Requirement | Description |
|------|------|
| Module interface | Implement `execute()`, `description`, `annotations`, `input_schema`, `output_schema` |
| Schema validation | Validate inputs against `input_schema` and outputs against `output_schema` |
| Canonical ID | Support directory-based ID generation and cross-language ID mapping |
| Error handling | Return structured errors with standard error codes |

---

## 5. ID Map Configuration

### 5.1 When ID Map is Needed

ID Map is used to convert local naming conventions of each language to Canonical ID.

**Scenarios requiring ID Map:**

| Scenario | Description |
|------|------|
| Rust modules | Rust uses `::` separator and PascalCase struct names |
| Java modules | Java uses `.` separator and PascalCase class names, with package names |
| Mixed-language projects | Modules in different languages need to call each other |
| Custom mapping | When automatic conversion rules don't meet requirements |

**Scenarios NOT requiring ID Map:**

| Scenario | Description |
|------|------|
| Pure Python projects | Python already uses snake_case and `.` separator |
| Pure Go projects | Go also uses `.` separator |
| All modules follow standard naming | Auto-detection is sufficient |

### 5.2 Configuration Example

```yaml
# apcore.yaml

id_map:
  # Auto-detect: determine language by file extension
  auto_detect: true

  # Language rules (defaults are usually sufficient)
  languages:
    python:
      extensions: [".py"]
      separator: "."
      file_case: "snake_case"
      class_case: "PascalCase"

    rust:
      extensions: [".rs"]
      separator: "::"
      file_case: "snake_case"
      struct_case: "PascalCase"

    go:
      extensions: [".go"]
      separator: "."
      file_case: "snake_case"
      struct_case: "PascalCase"

    java:
      extensions: [".java"]
      separator: "."
      file_case: "PascalCase"
      class_case: "PascalCase"

    typescript:
      extensions: [".ts", ".tsx"]
      separator: "."
      file_case: "camelCase"
      class_case: "PascalCase"

  # Special mappings (override auto rules)
  overrides:
    "executor.validator.db_params":
      java:
        class: "com.mycompany.DbParamsValidator"
        package: "com.mycompany.validators"
      rust:
        module: "executor::validator::db_params"
        struct: "DbParamsValidator"
```

### 5.3 Mapping from Language Naming Conventions to Canonical ID

```
Python:
  File: extensions/executor/validator/db_params.py
  Class: DbParamsValidator
  → Canonical ID: executor.validator.db_params

Rust:
  File: extensions/executor/validator/db_params.rs
  Module path: executor::validator::db_params
  Struct: DbParamsValidator
  → Canonical ID: executor.validator.db_params

Go:
  File: extensions/executor/validator/db_params.go
  Package: validator
  Struct: DbParamsValidator
  → Canonical ID: executor.validator.db_params

Java:
  File: extensions/executor/validator/DbParams.java
  Package: com.example.extensions.executor.validator
  Class: DbParamsValidator
  → Canonical ID: executor.validator.db_params

TypeScript:
  File: extensions/executor/validator/dbParams.ts
  Class: DbParamsValidator
  → Canonical ID: executor.validator.db_params
```

---

## 6. Type Mapping Reference

### 6.1 Basic Type Mapping Quick Reference

| JSON Schema Type | Python | Rust | Go | Java | TypeScript |
|-----------------|--------|------|----|------|------------|
| `string` | `str` | `String` | `string` | `String` | `string` |
| `integer` | `int` | `i64` | `int64` | `long` / `Long` | `number` |
| `number` | `float` | `f64` | `float64` | `double` / `Double` | `number` |
| `boolean` | `bool` | `bool` | `bool` | `boolean` / `Boolean` | `boolean` |
| `null` | `None` | `Option::None` | `nil` | `null` | `null` |
| `object` | `dict[str, Any]` | `HashMap<String, Value>` | `map[string]any` | `Map<String, Object>` | `Record<string, any>` |
| `array` | `list[T]` | `Vec<T>` | `[]T` | `List<T>` | `T[]` |

### 6.2 Format Type Mapping

| JSON Schema Format | Python | Rust | Go | Java | TypeScript |
|-----------------|--------|------|----|------|------------|
| `format: date-time` | `datetime` | `chrono::DateTime<Utc>` | `time.Time` | `Instant` / `ZonedDateTime` | `Date` / `string` |
| `format: date` | `date` | `chrono::NaiveDate` | `time.Time` | `LocalDate` | `string` |
| `format: uuid` | `uuid.UUID` | `uuid::Uuid` | `uuid.UUID` | `UUID` | `string` |
| `format: email` | `str` | `String` | `string` | `String` | `string` |
| `format: uri` | `str` | `String` | `string` | `String` | `string` |

### 6.3 Composite Type Mapping

| JSON Schema | Python | Rust | Go | Java | TypeScript |
|------------|--------|------|----|------|------------|
| `enum: [...]` | `Literal[...]` | `enum` | `type T string` + const | `enum` | union type |
| `oneOf` / `anyOf` | `Union[A, B]` | `enum { A(A), B(B) }` | `interface{}` | `Object` | `A \| B` |
| `T \| null` | `Optional[T]` | `Option<T>` | `*T` | `@Nullable T` | `T \| null` |
| `additionalProperties: T` | `dict[str, T]` | `HashMap<String, T>` | `map[string]T` | `Map<String, T>` | `Record<string, T>` |

For complete type mapping reference, see [Type Mapping Specification](../spec/type-mapping.md).

---

## 7. Common Pitfalls

### 7.1 Integer Precision Issues (JavaScript/JSON 53-bit Limitation)

JavaScript's `Number` type uses IEEE 754 double-precision floating-point, with a safe integer range of `-2^53 + 1` to `2^53 - 1` (i.e., `Number.MAX_SAFE_INTEGER = 9007199254740991`).

**Problem:**

```json
// JSON returned from backend
{"order_id": 9007199254740993}

// After JavaScript parsing
JSON.parse('{"order_id": 9007199254740993}')
// → { order_id: 9007199254740992 }  Precision loss!
```

**Solution:**

```yaml
# Use string to transmit large integers in Schema
properties:
  order_id:
    type: string
    pattern: "^[0-9]+$"
    description: "Order ID (string format to avoid precision loss)"
    x-llm-description: "Large integer transmitted as string to avoid JavaScript precision loss"
```

**Recommendation:** Use `type: string` with `pattern: "^[0-9]+$"` in Schema for integers that may exceed the safe range. Each language converts to its native large integer type.

### 7.2 DateTime and Timezone Handling

**Problem:** Different languages handle timezones differently by default, which can lead to time discrepancies.

**Best Practices:**

| Rule | Description |
|------|------|
| Store in UTC | All timestamps unified in UTC for storage and transmission |
| Display in local timezone | Convert to local timezone only at UI layer |
| Always include timezone info | Avoid using "naive" time (time without timezone) |
| Use ISO 8601 format | Uniformly use `2026-02-07T10:30:00Z` format |

```yaml
# Schema recommends using ISO 8601 + UTC
properties:
  created_at:
    type: string
    format: date-time
    description: "Creation time (ISO 8601, UTC)"
    x-examples: ["2026-02-07T10:30:00Z"]
```

### 7.3 Unicode Normalization Differences

**Problem:** Different operating systems and languages have different Unicode string normalization forms.

```
"cafe\u0301" (e + combining accent) vs "caf\u00e9" (precomposed e)
These two are visually identical but have different bytes.
```

**Recommended Practice:**

- All SDK implementations **should** normalize Unicode strings to NFC form
- Use `x-constraints` in Schema to annotate Unicode handling requirements

```yaml
# Annotate Unicode handling requirements in Schema
properties:
  name:
    type: string
    description: "Username"
    x-constraints: "MUST use NFC normalization form"
```

### 7.4 Null vs Undefined vs Missing Fields

**Problem:** `null` in JSON, missing fields, and language-specific concepts (like JavaScript's `undefined`) have different semantics.

```json
// Three different situations
{"name": null}      // Field exists, value is null
{"name": ""}        // Field exists, value is empty string
{}                  // Field does not exist (missing)
```

**Handling in Schema:**

```yaml
properties:
  name:
    type: ["string", "null"]   # Allow string or null
    description: "Username"

required: ["name"]  # Field must exist (but value can be null)
```

**Best Practices:**

```yaml
# Clearly distinguish between required and optional

# Required field: must exist and not be null
required_field:
  type: string

# Optional field: can be absent, not null when present
optional_field:
  type: string
  default: "default value"

# Nullable field: must exist, but can be null
nullable_field:
  type: ["string", "null"]
```

### 7.5 Floating-Point Precision

**Problem:** IEEE 754 floating-point numbers may produce different precision results in different languages.

```python
# Python
0.1 + 0.2  # 0.30000000000000004
```

**Recommended Practice:**

- Use `integer` for monetary fields (in cents) or `string` (precise decimal)
- When precise comparison is needed, use epsilon tolerance

```yaml
# Monetary handling in Schema
properties:
  amount_cents:
    type: integer
    description: "Amount (unit: cents)"
    minimum: 0
    x-llm-description: "Amount as integer in cents, e.g., 1999 represents 19.99 dollars"

  # Or use string
  amount:
    type: string
    pattern: "^\\d+\\.\\d{2}$"
    description: "Amount (string format, precise to cents)"
    x-examples: ["19.99", "100.00"]
```

### 7.6 Enum Value Serialization Differences

**Problem:** Different languages have different serialization conventions for enums.

```yaml
# Uniformly use snake_case in Schema
properties:
  status:
    type: string
    enum: ["pending", "in_progress", "completed", "failed"]
```

**Recommendation:** Enum values in JSON transmission **must** use the exact string values defined in Schema (typically `snake_case`). Each language SDK handles the mapping between its local naming convention and the Schema-defined values.

---

## Next Steps

- [Schema Definition Guide](./schema-definition.md) - Deep dive into Schema definitions
- [Module Testing Guide](./testing-modules.md) - Cross-language testing strategies
- [Creating Modules Guide](./creating-modules.md) - Complete module creation tutorial
- [Architecture Design](../architecture.md) - Overall system architecture
