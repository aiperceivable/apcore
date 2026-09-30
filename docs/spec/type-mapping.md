---
description: "Canonical mapping from JSON Schema Draft 2020-12 to Python, TypeScript (TypeBox) and Rust types, with format, null and optional-field handling, and the validation-keyword requirements every SDK enforces."
---

# apcore — Cross-language Type Mapping Specification

> This document is the canonical mapping from JSON Schema types to the native types of the three apcore SDKs, and states what each SDK enforces at the validation boundary. Guides link here instead of keeping their own tables. [protocol-spec.md](./protocol-spec.md) is normative and wins on any conflict.

## 1. Overview

### 1.1 Purpose

apcore uses JSON Schema Draft 2020-12 for module `input_schema` / `output_schema` (see [protocol-spec §4](./protocol-spec.md#4-schema-specification)). Every SDK **MUST** map JSON Schema types to native types consistently, so that:

- **Data Consistency**: The same JSON data has the same semantics in every SDK
- **Type Safety**: Each language's type system catches errors as early as possible
- **AI Awareness**: Schema-driven types let an LLM read field constraints accurately
- **Interoperability**: Modules in different languages exchange data through one JSON format

### 1.2 Scope

This specification covers the three SDKs: **Python** (apcore-python), **TypeScript** (apcore-js, schemas built with TypeBox) and **Rust** (apcore-rust).

### 1.3 Terminology

- **JSON Schema Type**: `type` values defined in JSON Schema Draft 2020-12
- **Native Type**: The type a module author writes in each language
- **Serialization Format**: Representation of a value in JSON transmission
- **Round-trip Fidelity**: Whether data is unchanged after serialization → transmission → deserialization

### 1.4 How the SDKs use these mappings

A module declares its contract in one of two ways, and the tables below serve both:

| | Python | TypeScript | Rust |
|---|---|---|---|
| **Schema first** — the schema is written as JSON Schema (YAML, a binding file, or a dict) | Validated inputs arrive as a `dict` of JSON values | Validated inputs arrive as a plain object | Validated inputs arrive as `serde_json::Value` |
| **Code first** — the schema is derived from native types | `@module` / `module()` infers it from type annotations through pydantic | Schemas are TypeBox objects; the same object is the JSON Schema and the static type (`Static<typeof S>`) | `typed_handler::<I, O>()` derives it with `schemars` from `#[derive(JsonSchema)]` types |

Either way, the module-invocation boundary validates the JSON value against the JSON Schema without type coercion (§17.3). Native types are what an author writes; they never widen what the schema accepts.

---

## 2. Basic Type Mappings

### 2.1 String Type (`string`)

**JSON Schema Definition:**

```yaml
type: string
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `str` | Unicode string |
| TypeScript | `Type.String()` → `string` | UTF-16 internal encoding |
| Rust | `String` | UTF-8 heap-allocated string |

**Notes:**

- All implementations **MUST** support the complete Unicode character set
- JSON transmission **MUST** use UTF-8 encoding
- TypeScript uses UTF-16 internally; take care with surrogate pairs when computing character lengths

### 2.2 Integer Type (`integer`)

**JSON Schema Definition:**

```yaml
type: integer
```

**Cross-language Mappings:**

| Language | Native Type | Range | Notes |
|------|---------|------|------|
| Python | `int` | Arbitrary precision | Python natively supports big integers |
| TypeScript | `Type.Integer()` → `number` | -(2^53-1) ~ 2^53-1 safe range | IEEE 754 double precision, see §14.1 |
| Rust | `i64` | -2^63 ~ 2^63-1 | `u64` / `i128` when the schema's range needs it |

**Supported Constraints:** `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`. All SDK implementations **MUST** enforce these constraints during validation.

A number with a zero fractional part is an `integer` (JSON Schema 2020-12 §6.1.1): `4.0` satisfies `{"type": "integer"}`, `4.5` does not (§17.3).

### 2.3 Number Type (`number`)

**JSON Schema Definition:**

```yaml
type: number
```

**Cross-language Mappings:**

| Language | Native Type | Precision | Notes |
|------|---------|------|------|
| Python | `float` | IEEE 754 double precision | Use a `string` field and `Decimal` for exact decimals (§14.2) |
| TypeScript | `Type.Number()` → `number` | IEEE 754 double precision | — |
| Rust | `f64` | IEEE 754 double precision | — |

**Constraint mappings** are the same as integer type (`minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`).

### 2.4 Boolean Type (`boolean`)

**JSON Schema Definition:**

```yaml
type: boolean
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `bool` | — |
| TypeScript | `Type.Boolean()` → `boolean` | — |
| Rust | `bool` | — |

**Notes:**

- JSON **MUST** use `true` / `false`; `0` / `1` / `"true"` / `"false"` are not booleans
- At the module-invocation boundary `0`, `1`, `"true"` and `"false"` **MUST** be rejected for a `boolean`, and no host configuration may relax it (§17.3)

### 2.5 Null Type (`null`)

**JSON Schema Definition:**

```yaml
type: "null"
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `None` | — |
| TypeScript | `Type.Null()` → `null` | Distinct from `undefined` |
| Rust | `Option::None` (`()` alone is rarely useful) | Usually combined with `Option<T>` |

A bare `null` type is rare; `null` normally appears as one branch of a nullable type (§4).

---

## 3. Collection Type Mappings

### 3.1 Object Type (`object` with `properties`)

**JSON Schema Definition:**

```yaml
type: object
properties:
  name:
    type: string
  age:
    type: integer
required: [name]
```

**Cross-language Mappings:**

| Language | Native Type | Mapping Method |
|------|---------|---------|
| Python | `class User(BaseModel)` | pydantic model with typed fields |
| TypeScript | `Type.Object({ name: Type.String(), age: Type.Optional(Type.Integer()) })` | `Static<typeof User>` gives the interface |
| Rust | `#[derive(Serialize, Deserialize, JsonSchema)] struct User { ... }` | serde struct; `schemars` derives the schema |

**Supported Constraints:** `minProperties`, `maxProperties` (§6.5.1–§6.5.2), `required` (§6.5.3), `dependentRequired` (§6.5.4), `patternProperties`, `additionalProperties`, `propertyNames` (§10.3.2), `dependentSchemas` (§10.2.2.4) and `unevaluatedProperties` (§11.3). All SDK implementations **MUST** enforce these during validation — see §17 for the per-keyword requirement.

`minProperties` / `maxProperties` count the keys the *instance* carried, undeclared ones included: every `additionalProperties` form except `false` keeps unknown keys, and they count.

### 3.2 Array Type (`array` with `items`)

**JSON Schema Definition:**

```yaml
type: array
items:
  type: string
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `list[str]` | — |
| TypeScript | `Type.Array(Type.String())` → `string[]` | — |
| Rust | `Vec<String>` | — |

**Supported Constraints:** `minItems`, `maxItems`, `uniqueItems`, `contains` / `minContains` / `maxContains`, `prefixItems` and `unevaluatedItems`. All SDK implementations **MUST** enforce these constraints during validation — see §17.

**Tuple form:** when `prefixItems` is present, `items` describes only the positions *past* the prefix (JSON Schema 2020-12 §10.3.1.2). Applying `items` to the whole array is a conformance defect: it rejects a valid tuple head.

```yaml
type: array
prefixItems:
  - type: string    # position 0
  - type: integer   # position 1
items:
  type: boolean     # positions 2 and beyond
```

**Nested Array Example (Array of Objects):**

```yaml
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
```

| Language | Native Type |
|------|---------|
| Python | `list[ErrorDetail]` (where `ErrorDetail` is a pydantic model) |
| TypeScript | `Type.Array(ErrorDetail)` → `Static<typeof ErrorDetail>[]` |
| Rust | `Vec<ErrorDetail>` |

---

## 4. Nullable Type Mappings

### 4.1 Nullable Type (`T | null`)

A nullable value is written with the Draft 2020-12 type array or an `anyOf` with a `null` branch:

```yaml
type: [string, "null"]
```

```yaml
anyOf:
  - type: string
  - type: "null"
```

**Cross-language Mappings:**

| Language | Native Type | Schema the SDK infers from code |
|------|---------|------|
| Python | `str \| None` | `anyOf: [{type: string}, {type: "null"}]` (pydantic) |
| TypeScript | `Type.Union([Type.String(), Type.Null()])` → `string \| null` | `anyOf` of the two branches |
| Rust | `Option<String>` | `type: [string, "null"]` (schemars) |

JSON Schema has no `nullable` keyword; `"nullable": true` is OpenAPI 3.0 syntax and Draft 2020-12 validators ignore it.

**Serialization Rules:**

- When value is `null`, JSON **MUST** output `null` (not omit the field)
- Null and absent are different (§6.1)

---

## 5. Enum Type Mappings

### 5.1 String Enum

**JSON Schema Definition:**

```yaml
type: string
enum: [pending, running, completed, failed, cancelled]
```

**Cross-language Mappings:**

| Language | Native Type | Example |
|------|---------|------|
| Python | `Literal["pending", "running", ...]` or `StrEnum` | `class Status(StrEnum): PENDING = "pending"` |
| TypeScript | `Type.Union([Type.Literal("pending"), ...])` → `"pending" \| "running" \| ...` | Emits `anyOf` of `const`, which validates the same values |
| Rust | `#[serde(rename_all = "snake_case")] enum Status { Pending, Running, ... }` | Derive `JsonSchema` to emit `enum` |

A value outside the list **MUST** fail with `SCHEMA_VALIDATION_ERROR` (§14.5).

### 5.2 Integer Enum

**JSON Schema Definition:**

```yaml
type: integer
enum: [0, 1, 2, 3]
```

**Cross-language Mappings:**

| Language | Native Type | Example |
|------|---------|------|
| Python | `Literal[0, 1, 2, 3]` or `IntEnum` | `class Priority(IntEnum): LOW = 0` |
| TypeScript | `Type.Union([Type.Literal(0), Type.Literal(1), ...])` | — |
| Rust | `i64` (the validator enforces `enum`) | A fieldless enum serializes by name, not number, under plain serde |

---

## 6. Optional Fields and `required` Mapping

### 6.1 Optional Fields (not in `required` array)

**JSON Schema Definition:**

```yaml
type: object
properties:
  name:
    type: string
  note:
    type: string
    default: ""
required: [name]
# note is not in required, so it is optional
```

**Cross-language Mappings:**

| Language | Required Field (`name`) | Optional Field (`note`) | Notes |
|------|-------------------|-------------------|------|
| Python | `name: str` | `note: str = ""` | A parameter with a default is left out of `required` |
| TypeScript | `name: Type.String()` | `note: Type.Optional(Type.String())` | Static type `note?: string` |
| Rust | `name: String` | `#[serde(default)] note: String` or `note: Option<String>` | A missing `Option` field deserializes to `None` |

**Optional and nullable are independent.** `note: str | None = None` (Python), `Type.Optional(Type.Union([Type.String(), Type.Null()]))` (TypeScript) and `Option<String>` (Rust) are both optional and nullable.

**Important Distinctions:**

| Semantics | JSON Representation | Description |
|------|----------|------|
| Field missing (optional field not provided) | Key does not exist | No SDK fills a schema `default` into the inputs; a Python code-first parameter's own default applies |
| Field is null (explicit null value) | `"field": null` | Requires a nullable declaration (§4) |
| Field is empty string | `"field": ""` | Has a value, but empty |

---

## 7. Date and Time Type Mappings

`date-time`, `date` and `time` are `string` values with a `format`. **No SDK converts them to a native date or time type**: the module receives the JSON string and parses it if it needs to.

### 7.1 `date-time` Format

**JSON Schema Definition:**

```yaml
type: string
format: date-time
```

**Cross-language Mappings:**

| Language | Declared Type | Parse inside the module with | Example |
|------|---------|----------|------|
| Python | `str` | `datetime.fromisoformat()` | `"2026-02-07T10:30:00Z"` |
| TypeScript | `Type.String({ format: "date-time" })` → `string` | `new Date()` | `"2026-02-07T10:30:00Z"` |
| Rust | `String` | `chrono::DateTime::parse_from_rfc3339` | `"2026-02-07T10:30:00Z"` |

!!! warning "Python: do not annotate a code-first parameter as `datetime`, `date`, `time` or `UUID`"
    pydantic infers the right `format` from those annotations, but the module-invocation boundary validates in strict mode and inputs arrive as JSON strings, so such a parameter rejects every call with `SCHEMA_VALIDATION_ERROR`. Declare `str` and parse inside the module.

**Notes:**

- Serialization **MUST** produce ISO 8601 / RFC 3339
- Implementations **SHOULD** use UTC (`Z` suffix) unless the business explicitly requires an offset
- Parsing **MUST** accept timezone offsets (e.g., `+08:00`)

### 7.2 `date` Format

**JSON Schema Definition:**

```yaml
type: string
format: date
```

| Language | Declared Type | Parse inside the module with | Example |
|------|---------|----------|------|
| Python | `str` | `date.fromisoformat()` | `"2026-02-07"` |
| TypeScript | `Type.String({ format: "date" })` → `string` | — | `"2026-02-07"` |
| Rust | `String` | `chrono::NaiveDate::parse_from_str` | `"2026-02-07"` |

### 7.3 `time` Format

**JSON Schema Definition:**

```yaml
type: string
format: time
```

| Language | Declared Type | Parse inside the module with | Example |
|------|---------|----------|------|
| Python | `str` | `time.fromisoformat()` | `"10:30:00"` |
| TypeScript | `Type.String({ format: "time" })` → `string` | — | `"10:30:00"` |
| Rust | `String` | `chrono::NaiveTime::parse_from_str` | `"10:30:00"` |

---

## 8. Nested Object Mapping

### 8.1 Nested Object Properties

**JSON Schema Definition:**

```yaml
type: object
properties:
  user:
    type: object
    properties:
      name:
        type: string
      address:
        type: object
        properties:
          city:
            type: string
          zip_code:
            type: string
        required: [city]
    required: [name]
required: [user]
```

**Cross-language Mapping Strategies:**

| Language | Strategy | Example |
|------|------|------|
| Python | Nested pydantic models | `class Address(BaseModel)` + `class User(BaseModel)` |
| TypeScript | Nested `Type.Object` | `const Address = Type.Object({...})`; `const User = Type.Object({ address: Address })` |
| Rust | Nested structs | `struct Address { ... }` + `struct User { ... }` |

**Naming Convention:**

- Nested objects **SHOULD** be extracted as independent named types
- Type names **SHOULD** be generated from property paths (e.g., `user.address` → `UserAddress`)
- When a schema uses `$ref` (see [protocol-spec §4.11](./protocol-spec.md#411-schema-references-ref)), all languages **MUST** map it to the same shared type

---

## 9. Union Type Mappings

### 9.1 `oneOf` Type

**JSON Schema Definition:**

```yaml
oneOf:
  - type: object
    properties:
      type:
        const: "email"
      address:
        type: string
    required: [type, address]
  - type: object
    properties:
      type:
        const: "sms"
      phone:
        type: string
    required: [type, phone]
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `Annotated[Email \| Sms, Field(discriminator="type")]` | Discriminated union |
| TypeScript | `Type.Union([Email, Sms])` | TypeBox has no `oneOf` builder; `Type.Union` emits `anyOf` |
| Rust | `#[serde(tag = "type")] enum Notification { Email { .. }, Sms { .. } }` | Internally tagged enum |

`oneOf` in a JSON Schema document is enforced as **exactly one** branch by every SDK (§17.2), whatever the native type an author chose.

### 9.2 `anyOf` Type

**JSON Schema Definition:**

```yaml
anyOf:
  - type: string
  - type: integer
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `str \| int` | — |
| TypeScript | `Type.Union([Type.String(), Type.Integer()])` → `string \| number` | — |
| Rust | `#[serde(untagged)] enum StringOrInt { Str(String), Int(i64) }` | Untagged enum |

**Implementation Recommendations:**

- An `anyOf` whose only other branch is `null` is a nullable type (§4.1)
- When every branch is an object type, prefer a discriminated union
- Rust needs an untagged enum or `serde_json::Value` with runtime dispatch

---

## 10. `additionalProperties` (Arbitrary Key-Value Mapping)

### 10.1 Arbitrary Key-Value Map

**JSON Schema Definition:**

```yaml
type: object
additionalProperties:
  type: string
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `dict[str, str]` | — |
| TypeScript | `Type.Record(Type.String(), Type.String())` → `Record<string, string>` | — |
| Rust | `HashMap<String, String>` | `BTreeMap` for ordered output |

### 10.2 Mixed Mode (Fixed Properties + Additional Properties)

**JSON Schema Definition:**

```yaml
type: object
properties:
  name:
    type: string
required: [name]
additionalProperties:
  type: integer
```

**Cross-language Mappings:**

| Language | Strategy | Notes |
|------|------|------|
| Python | pydantic model with `extra="allow"` | Extra fields are validated against `additionalProperties` |
| TypeScript | `Type.Object({ name: Type.String() }, { additionalProperties: Type.Integer() })` | — |
| Rust | Fixed fields + `#[serde(flatten)] extra: HashMap<String, i64>` | — |

### 10.3 `additionalProperties: false`

When `input_schema` declares `additionalProperties: false` (protocol-spec §4.2 **SHOULD**), implementations **MUST** reject inputs containing unknown fields.

A field is "unknown" only if neither `properties` nor `patternProperties` claimed it (JSON Schema 2020-12 §10.3.2.3). A key matched by a `patternProperties` entry **MUST NOT** be rejected by `additionalProperties: false`.

To close an object *after* the branches of `allOf` / `anyOf` / `oneOf` / `if`-`then`-`else` have contributed their own properties, use `unevaluatedProperties: false` instead — `additionalProperties` cannot see those branches. See §17.2.

---

## 11. Format Constraint Mappings

### 11.1 `format` Keyword

`format` carries a semantic hint about a `string` value. It is an **annotation, not an assertion**: under the default format-annotation vocabulary of JSON Schema 2020-12 §7.2.1, a value that does not satisfy its declared `format` **MUST NOT** fail validation. Implementations **SHOULD** recognise the formats below and emit a warning when a value does not conform, and **MUST** accept the value regardless. A `format` the implementation does not recognise is collected as an annotation and **MUST** pass silently — a contract is free to declare `format: "path"` or any other vocabulary term without becoming uncallable.

Recognised formats (the same eight in all three SDKs):

| `format` Value | Meaning | Regex/Rule | Example |
|-------------|------|----------|------|
| `email` | Email address | RFC 5322 | `"user@example.com"` |
| `uri` | URI | RFC 3986 | `"https://apcore.dev/docs"` |
| `uuid` | UUID | RFC 4122 | `"550e8400-e29b-41d4-a716-446655440000"` |
| `ipv4` | IPv4 address | RFC 2673 | `"192.168.1.1"` |
| `ipv6` | IPv6 address | RFC 4291 | `"::1"` |
| `date-time` | Date-time | ISO 8601 | `"2026-02-07T10:30:00Z"` |
| `date` | Date | ISO 8601 | `"2026-02-07"` |
| `time` | Time | ISO 8601 | `"10:30:00"` |

**No SDK converts a formatted value.** It stays a JSON string on the way into and out of the module; §7 shows how to parse the date and time formats.

### 11.2 Format Validation Requirements

All SDK implementations **SHOULD** check a value against its declared `format` when the format appears in §11.1, and **SHOULD** report a non-conforming value as a warning. The check **MUST NOT** produce `SCHEMA_VALIDATION_ERROR` — see §11.1.

To make a format binding, express it as an assertion the vocabulary already carries: `pattern` for a syntactic shape, or `enum` for a closed set. `format` alone never rejects.

**Where each SDK emits the warning:**

| SDK | On module invocation | Standalone validator |
|---|---|---|
| apcore-python | Yes — input and output validation steps | Yes |
| apcore-typescript | Yes — `SchemaValidator` on the invocation path | Yes |
| apcore-rust | **No** — the executor's `validate_against_schema` does not call the format check | Yes — `SchemaValidator` / `format_warnings()` |

The conformance fixture `schema_hardening_formats.json` asserts the annotation semantics (validation passes) on all three SDKs; its `warn_logged` expectations go through the standalone warning path, so they do not cover the Rust invocation gap.

**TypeScript and TypeBox.** TypeBox's own `Value.Check` rejects a string whose `format` is not in its global `FormatRegistry`. apcore's `SchemaValidator` treats every format as an annotation for the duration of its check, so the rule above holds on the apcore path; calling `Value.Check` directly on such a schema does not.

---

## 12. Complete Type Mapping Table

The following table summarizes all JSON Schema type to language mappings:

| JSON Schema | Python | TypeScript (TypeBox → static type) | Rust |
|-------------|--------|------------|------|
| `string` | `str` | `Type.String()` → `string` | `String` |
| `integer` | `int` | `Type.Integer()` → `number` | `i64` |
| `number` | `float` | `Type.Number()` → `number` | `f64` |
| `boolean` | `bool` | `Type.Boolean()` → `boolean` | `bool` |
| `null` | `None` | `Type.Null()` → `null` | `Option::None` |
| `object` (with properties) | pydantic `BaseModel` | `Type.Object({...})` | `struct` |
| `array` (with items) | `list[T]` | `Type.Array(T)` → `T[]` | `Vec<T>` |
| `T \| null` | `T \| None` | `Type.Union([T, Type.Null()])` → `T \| null` | `Option<T>` |
| optional property | parameter with a default | `Type.Optional(T)` → `prop?: T` | `Option<T>` or `#[serde(default)]` |
| string `enum` | `Literal[...]` / `StrEnum` | `Type.Union([Type.Literal(...)])` | `enum` (serde) |
| integer `enum` | `Literal[...]` / `IntEnum` | `Type.Union([Type.Literal(...)])` | `i64` |
| `oneOf` | discriminated union | `Type.Union([...])` (emits `anyOf`) | `enum` (tagged) |
| `anyOf` | `A \| B` | `Type.Union([...])` | `enum` (untagged) |
| `additionalProperties` | `dict[str, V]` | `Type.Record(Type.String(), V)` → `Record<string, V>` | `HashMap<String, V>` |
| `string` + any `format` | `str` | `Type.String({ format })` → `string` | `String` |

---

## 13. Serialization Round-trip Fidelity

### 13.1 Fidelity Guarantees

Serialization round-trip fidelity refers to whether data semantics remain consistent after serializing a language's native object to JSON and then deserializing to another language's native object.

Implementations **MUST** guarantee perfect round-trips for the following types:

| Type | Fidelity Requirement | Description |
|------|-----------|------|
| `string` | **MUST** perfect round-trip | UTF-8 encoding lossless |
| `boolean` | **MUST** perfect round-trip | `true`/`false` |
| `null` | **MUST** perfect round-trip | — |
| `integer` (within safe range) | **MUST** perfect round-trip | Absolute value ≤ 2^53 - 1 (cross-language safe boundary) |
| `number` (IEEE 754 representable) | **SHOULD** perfect round-trip | Floating-point precision limits |
| `object` | **MUST** perfect round-trip | Field order **MAY** differ |
| `array` | **MUST** perfect round-trip | Element order **MUST** be preserved |

### 13.2 Serialization Specifications

| Rule | Level | Description |
|------|------|------|
| Output **MUST** be valid JSON | **MUST** | RFC 8259 |
| Character encoding **MUST** be UTF-8 | **MUST** | — |
| Integers **MUST NOT** serialize as floats | **MUST** | `42` not `42.0` |
| Floats **MUST** preserve decimal part | **MUST** | `3.14` not `3` |
| `null` fields **SHOULD** be explicitly output | **SHOULD** | `{"field": null}` |
| Object key order **MAY** not be guaranteed | **MAY** | But **SHOULD** maintain stable output |

---

## 14. Boundary Cases and Known Issues

### 14.1 Large Integer Precision Loss

**Problem Description:**

JavaScript (the TypeScript runtime) represents every number as an IEEE 754 double, with a safe integer range of `-(2^53 - 1)` to `2^53 - 1`. Integers outside this range lose precision.

**Impact Range:**

| Language | Integer Range | Is Affected |
|------|---------|-----------|
| Python | Arbitrary precision | No |
| TypeScript | number (safe range 2^53-1) | Yes |
| Rust | i64 (-2^63 ~ 2^63-1) | Only beyond i64 |

**Cross-language Safe Boundary:**

apcore defines **2^53 - 1** (`9007199254740991`, JavaScript `Number.MAX_SAFE_INTEGER`) as the cross-language integer safe boundary, set by the weakest consumer (JavaScript/TypeScript).

**Specification Requirements:**

| Rule | Level | Description |
|------|------|------|
| Integers with absolute value ≤ 2^53 - 1 | **MUST** use `type: integer` | All languages can handle losslessly |
| Integers with absolute value > 2^53 - 1 | **MUST** use `type: string` + `format` | Avoid JavaScript precision loss |
| Schema **SHOULD** explicitly declare `minimum` / `maximum` | **SHOULD** | Help languages choose appropriate native types |

**Large Number `format` Specification:**

Values exceeding the safe boundary **MUST** be transmitted as `type: string`, with `format` naming the semantics. These formats are not in §11.1's recognised set, so they are annotations only: add a `pattern` to reject malformed values, and convert inside the module.

| `format` Value | Meaning | Range | Convert to |
|-------------|------|------|-----------|
| `int64` | 64-bit signed integer | -2^63 ~ 2^63-1 | Python `int`, TypeScript `BigInt`, Rust `i64` |
| `bigint` | Arbitrary precision integer | Unlimited | Python `int`, TypeScript `BigInt`, Rust `num_bigint::BigInt` |
| `decimal` | High precision decimal | Unlimited | Python `Decimal`, TypeScript `decimal.js`, Rust `rust_decimal::Decimal` |

**Schema Example:**

```yaml
properties:
  # Within safe range — use integer directly
  user_id:
    type: integer
    minimum: 0
    maximum: 9007199254740991

  # Exceeds safe range — use string + format
  snowflake_id:
    type: string
    format: int64
    description: "Twitter Snowflake ID, exceeds JS safe integer range"
    pattern: "^-?\\d+$"

  # Arbitrary precision integer
  blockchain_nonce:
    type: string
    format: bigint
    description: "Blockchain nonce, may exceed int64 range"

  # High precision amount
  amount:
    type: string
    format: decimal
    description: "Amount, precise to cents"
    pattern: "^-?\\d+\\.\\d{2}$"
```

### 14.2 Floating-Point Precision Issues

**Problem Description:**

IEEE 754 double-precision floating-point numbers cannot precisely represent all decimal fractions, for example `0.1 + 0.2 !== 0.3`.

**Solution Strategy:**

1. For financial or other exact calculations, transmit the value as a `string` (`format: decimal`, §14.1) and convert it to the language's decimal type inside the module
2. Use the `x-precision` extension field in the schema to annotate precision requirements

### 14.3 Date Timezone Handling

**Problem Description:**

Different languages handle timezones differently, which may cause date-time offsets during conversion.

**Specification Requirements:**

- Serialization **MUST** include timezone information (`Z` or `+HH:MM`)
- Deserialization **MUST** correctly parse timezone offsets
- If input lacks timezone information, implementations **SHOULD** treat as UTC

### 14.4 Empty Object vs Empty Map Distinction

**Problem Description:**

In JSON, `{}` can represent both an empty object (object with no properties) and an empty Map (additionalProperties object with no key-value pairs); the two cannot be distinguished at the JSON level.

**Solution Strategy:**

- Distinguish based on Schema definition: with `properties` is structured object, with `additionalProperties` is Map
- When both are present, fields in `properties` are handled as structured, other fields as Map

### 14.5 Enum Values vs String Distinction

**Problem Description:**

Enum values and plain strings are identical on the wire (e.g., `"pending"`); only the schema distinguishes them.

**Specification Requirements:**

- Validation **MUST** check the schema's `enum` constraint
- If the input value is not in the `enum` list, validation **MUST** fail with `SCHEMA_VALIDATION_ERROR`

---

## 15. Reserved Keyword Adaptations

Some spec-defined method names conflict with language reserved keywords. Implementations **MUST** provide the equivalent functionality under a language-idiomatic alternative name:

| Spec Method | Python | TypeScript | Rust | Reason |
|---|---|---|---|---|
| `use(middleware)` | `use(middleware)` | `use(middleware)` | `use_middleware(middleware)` | `use` is a Rust reserved keyword |

**Rules:**

- When a spec method name is a reserved keyword in a target language, the SDK **MUST** choose a name that preserves the verb and adds a noun suffix describing the argument (e.g., `use` → `use_middleware`).
- The adapted name **MUST** be documented in the SDK's README API Overview section.
- Implementations **MUST NOT** rely on language-specific escape mechanisms (e.g., Rust raw identifiers `r#keyword`) as the primary API surface. The adapted name **MUST** be a natural identifier in the target language.
- Cross-language sync checks **MUST** treat the adapted name as equivalent to the spec name and **MUST NOT** flag it as a divergence.

**Maintenance:** SDK implementers **SHOULD** check for keyword conflicts when adding new public methods and update this table accordingly.

---

## 16. Per-SDK Validation Library Notes

### 16.1 TypeScript SDK — TypeBox

The TypeScript SDK uses **`@sinclair/typebox`** (^0.34) for JSON Schema–shaped validation rather than a standalone Draft 2020-12 validator (e.g., `ajv`). TypeBox is a schema builder/validator hybrid that provides static TypeScript types from the same schema object.

**Keywords TypeBox validates natively** (through `Value.Check` / `TypeCompiler`):
`type`, `properties`, `required`, `enum`, `const`, `items`, `minItems`, `maxItems`, `uniqueItems`, `contains`, `minContains`, `maxContains`, `minProperties`, `maxProperties`, `additionalProperties`, `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`, `minLength`, `maxLength`, `pattern`, `format` (carried as an annotation, see §11), `$ref` (within the same schema — not cross-file).

**Keywords with no TypeBox node**, supplied by the SDK's own evaluator and registered as a custom TypeBox kind so `Value.Check` reaches them: `prefixItems`, `patternProperties`, `propertyNames`, `dependentRequired`, `dependentSchemas`, `if` / `then` / `else`, `unevaluatedItems`, `unevaluatedProperties`. Sub-schemas of these keywords are handed back to the converter, so there is still exactly one validation engine. The requirement level for each is §17 — the absence of a library node is **not** a licence to drop the keyword.

**Remaining limitation vs. full Draft 2020-12:** `$ref` to external URIs / files is not supported; use `apcore`'s schema registry for cross-module references.

### 16.2 Python and Rust

**Python SDK** converts schemas to pydantic models and uses `jsonschema` (>=4.21, fully Draft 2020-12 conformant) for keywords pydantic cannot express, delegated as a sub-schema assertion.

**Rust SDK** uses `jsonschema` 0.28 and hands it the raw schema, so every keyword in §17 is enforced by the library directly. Typed handlers derive their schemas with `schemars` 0.8.

---

## 17. Validation Keyword Conformance

§2–§11 describe how *types* map. This section states, keyword by keyword, what an SDK is required to do at the validation boundary — the path a module invocation actually takes (schema-to-native conversion followed by the SDK's validator), not a side-channel raw-schema check. A caller cannot reason about a contract whose constraints are enforced in one runtime and ignored in another, so "partial support" is not an option.

### 17.1 General rules

- **R1 — No silent drop.** An SDK **MUST NOT** discard a keyword listed as MUST below. If a keyword cannot be enforced, the SDK **MUST** reject the schema at load time with `SCHEMA_PARSE_ERROR` rather than accept it and validate less than the contract declares.
- **R2 — Inertness.** Every keyword in the table applies only to instances of the type it describes and **MUST** pass every other instance type (JSON Schema 2020-12 §6, §10.3). `{"minimum": 3}` rejects `1` and accepts `"x"`, `[1]`, `true` and `null`; `{"prefixItems": [...]}` accepts every non-array; `{"patternProperties": {...}}` accepts every non-object. A conversion that narrows a type-less schema to the constrained type violates this.
- **R3 — Adjacency.** A keyword sitting next to `type` is an independent assertion; **both** must hold (§10.2). Converting only the `type` half is a violation.
- **R4 — Fixture.** Conformance to this section is asserted by `conformance/fixtures/schema_keyword_parity.json`, which every SDK **MUST** drive through its conversion + validation pair. Cases are verified against a Draft 2020-12 reference validator before being added.

### 17.2 Requirement table

| Keyword(s) | JSON Schema 2020-12 | Requirement | Rationale |
|---|---|---|---|
| `type`, `enum`, `const` | §6.1 | **MUST** enforce | The core of the contract. A `type` array is a union of *all* its members. |
| `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf` | §6.2 | **MUST** enforce | Also stated in §2.2. |
| `minLength`, `maxLength`, `pattern` | §6.3 | **MUST** enforce | `pattern` is an ECMA-262 regex matched anywhere in the string, not anchored. |
| `minItems`, `maxItems`, `uniqueItems` | §6.4.1–§6.4.3 | **MUST** enforce | Also stated in §3.2. |
| `contains`, `minContains`, `maxContains` | §6.4.4–§6.4.5, §10.3.1.3 | **MUST** enforce | `minContains` / `maxContains` assert nothing without `contains`; the three travel together. |
| `minProperties`, `maxProperties` | §6.5.1–§6.5.2 | **MUST** enforce | Counts the keys the *instance* carried, undeclared ones included — every `additionalProperties` form but `false` keeps them. |
| `required` | §6.5.3 | **MUST** enforce | A name listed here without a `properties` entry is still required: `{"required": ["b"]}` is a complete schema and is the usual shape of an `if` / `then` / `dependentSchemas` sub-schema. |
| `dependentRequired` | §6.5.4 | **MUST** enforce | Expresses "flag A requires flag B", which CLI- and form-shaped contracts rely on. Pure key-presence logic, cheap in every language. |
| `allOf`, `anyOf`, `oneOf`, `not` | §10.2.1 | **MUST** enforce | `oneOf` **MUST** be exclusive (exactly one branch), not first-match. |
| `if`, `then`, `else` | §10.2.2.1–§10.2.2.3 | **MUST** enforce | `if` never fails an instance on its own; it selects `then` or `else`, and a missing branch asserts nothing. Rejecting the schema outright is **NOT** conformant. |
| `dependentSchemas` | §10.2.2.4 | **MUST** enforce | The schema-valued counterpart of `dependentRequired`. |
| `prefixItems`, `items` | §10.3.1.1–§10.3.1.2 | **MUST** enforce | With `prefixItems` present, `items` applies **only** past the prefix. Applying `items` to the tuple head is a defect, not a simplification. |
| `properties`, `patternProperties`, `additionalProperties` | §10.3.2.1–§10.3.2.3 | **MUST** enforce | `additionalProperties` targets only the keys `properties` and `patternProperties` did not claim; a pattern-matched key **MUST NOT** be rejected by `additionalProperties: false`. |
| `propertyNames` | §10.3.2.4 | **MUST** enforce | The sub-schema applies to each key *string*, not to its value. |
| `unevaluatedItems`, `unevaluatedProperties` | §11.2–§11.3 | **MUST** enforce | The only way to write "this object is closed, whatever the `allOf` / `if` branches added". Enforcing it requires collecting annotations from the sibling applicators — only sub-schemas that **succeeded** contribute. |
| `format` | §7 | **MUST** treat as an annotation; **SHOULD** warn | Draft 2020-12 §7.2.1 makes `format` non-assertive by default. An SDK **MUST NOT** fail validation on an unsatisfied `format`; it **SHOULD** emit a warning (see §11.2). |
| `$ref` to an external URI or file | §8.2.3 | **MAY** decline | Resolution policy is the schema registry's, not the validator's; `apcore` resolves cross-module references before conversion. An SDK that declines **MUST** say so, as §16.1 does. |
| `$defs`, `title`, `description`, `default`, `examples`, `deprecated`, `readOnly`, `writeOnly` | §8.2.4, §9 | **MAY** ignore for validation | Annotations. They **SHOULD** be preserved through conversion so schema export round-trips. |
| `contentMediaType`, `contentEncoding`, `contentSchema` | §8.4–§8.5 | **MAY** ignore | Annotation-only in Draft 2020-12, and apcore transports decoded JSON, so there is no encoded string to inspect. |

### 17.3 No type coercion at the module boundary

**R5 — No coercion.** The module-invocation boundary **MUST NOT** perform type coercion. Every keyword in §17.2, `type` included, is asserted against the instance as it arrived. `{"type": "integer"}` **MUST** reject `"42"`; `{"type": "boolean"}` **MUST** reject `1`, `0`, `"true"` and `"false"`; `{"type": "string"}` **MUST** reject `42`. This holds for inputs and outputs alike, at every depth, and inside `items` / `additionalProperties` / union branches exactly as at the top level.

**This is not host-configurable.** A module's input contract has to mean the same thing regardless of which host loaded it. If a host could switch coercion on, the same module would accept `{"count": "3"}` in one deployment and reject it in another. There is therefore no configuration key for it: the `schema` namespace is `root` / `strategy` / `max_ref_depth` and nothing else, and `defaults.schema.json` declares it `additionalProperties: false` (protocol-spec §4.9).

**What "no coercion" does *not* mean.** The rule is about instance *types*, not renderings. JSON Schema 2020-12 §6.1.1 defines `integer` as any number with a zero fractional part, so `4.0` **MUST** satisfy `{"type": "integer"}` while `4.5` **MUST NOT**; `42` **MUST** satisfy `{"type": "number"}`. An SDK whose native integer type cannot hold `4.0` has to narrow it — that is honouring the type definition, not relaxing it. pydantic's strict mode rejects `4.0` for `int`, so apcore-python normalises the zero-fraction case before the check.

**Library-level knob.** An SDK **MAY** keep a coercion switch on its standalone validator API — apcore-python `SchemaValidator(coerce_types=…)`, apcore-typescript `new SchemaValidator(…)`, apcore-rust `SchemaValidator::with_coerce_types(…)` — for callers validating their *own* untyped input (a CLI parsing argv, a form handler). Such a knob **MUST NOT** reach the module-invocation path, **MUST NOT** be readable from a configuration file, and its default **SHOULD** be no-coercion so the two paths cannot silently disagree.

**What the knob coerces, when it exists.** Offering the switch is a **MAY**; an SDK with no coercing mode at all is conforming. An SDK that offers one **MUST** coerce exactly this set, and **MUST NOT** coerce anything else:

| from | to | accepted |
|---|---|---|
| string | `integer` | surrounding whitespace is trimmed, then the whole remainder must parse as a number with a zero fractional part — `"42"`, `"-7"`, `" 42 "`, `"42.0"`. `"3.14"` **MUST NOT** be accepted, nor may a trailing remainder be ignored (`"42abc"`). |
| string | `number` | surrounding whitespace is trimmed, then the whole remainder must parse as a finite number — `"1.5"`, `"42"`, `"-0.5"`, `" 1.5 "`. |
| string | `boolean` | exactly `"true"` and `"false"`, **case-sensitive**. |

`"42.0"` coercing to `integer` follows from R5's note above: `4.0` satisfies `{"type": "integer"}`, and the string form follows the number form.

Coercion is **from a string only**, and only toward a type the schema declares. A number is never coerced to a boolean, a boolean never to a number, and nothing is coerced toward `string`.

The boolean row is deliberately narrow. `"true"` and `"false"` are JSON's own spelling of a boolean. `"yes"`, `"on"`, `"y"`, `"t"`, `"1"`, `"0"` and their negatives are shell and INI conventions that belong to whatever parses `argv`, not to a JSON Schema validator. `"0"` → `false` is the sharpest case: R5 makes the *number* `0` a MUST-reject for `boolean`, so accepting the string `"0"` would put two paths of the same SDK on opposite sides of one value.

**Not to be confused with §9.2 environment-override coercion.** `APCORE_*` variables arrive as strings and are coerced into config values by a *different* rule, which accepts `true` / `false` **case-insensitively** — apcore-python `_coerce_env_value`, apcore-typescript `coerceEnvValue`, apcore-rust `coerce_env_value`. That is the protocol-spec §9.2 contract and this table does not govern it: case-sensitive here, because a JSON Schema instance is JSON; case-insensitive there, because `APCORE_DEBUG=True` is what an operator types.

**Keyword slicing.** An SDK that delegates part of a schema to a strict Draft 2020-12 engine **SHOULD** delegate the applicator keywords alone rather than the whole schema, so a `type` its own conversion already enforced is not re-asserted twice over a differently-shaped value. The one exception is `unevaluatedItems` / `unevaluatedProperties`, which are defined against the annotations of every sibling keyword and therefore cannot be evaluated from a slice.

**Fixture.** `conformance/fixtures/schema_keyword_parity.json` asserts R5 at the boundary; the opt-in library-level coercing mode is covered separately by `conformance/fixtures/schema_validation.json` (`expected_valid_strict` / `expected_valid_coerce`).

---

## 18. References

- [protocol-spec §4 — Schema Specification](./protocol-spec.md#4-schema-specification)
- [protocol-spec §4.10 — Language-specific Schema Implementations](./protocol-spec.md#410-language-specific-schema-implementations)
- [protocol-spec §4.11 — Schema References ($ref)](./protocol-spec.md#411-schema-references-ref)
- [protocol-spec §5.11.5 — Language Type → JSON Schema Mapping Table](./protocol-spec.md#5115-language-type-json-schema-mapping-table)
- [protocol-spec §12.3 — Cross-language Implementation Requirements](./protocol-spec.md#123-cross-language-implementation-requirements)
- [JSON Schema Draft 2020-12](https://json-schema.org/draft/2020-12/json-schema-core)
- [RFC 8259 — The JavaScript Object Notation (JSON) Data Interchange Format](https://www.rfc-editor.org/rfc/rfc8259)
