---
description: "Specifies mapping rules from JSON Schema Draft 2020-12 types to native Python, Rust, and TypeScript types (with Go and Java reference) to ensure cross-language data consistency and type safety."
---

# apcore — Cross-language Type Mapping Specification

> This document defines standard mapping rules from JSON Schema types to native types in various languages for the apcore framework, ensuring behavioral consistency across language implementations.

## 1. Overview

### 1.1 Purpose

apcore adopts JSON Schema Draft 2020-12 as the standard description format for module `input_schema` / `output_schema` (see [PROTOCOL_SPEC §4](./protocol-spec.md#4-schema-specification)). When implementing SDKs in various languages, implementations **MUST** accurately map JSON Schema types to corresponding language native types to ensure:

- **Data Consistency**: The same JSON data has the same semantics when deserialized in different languages
- **Type Safety**: Fully utilize each language's type system to detect errors as early as possible at compile-time or runtime
- **AI Awareness**: Schema-driven type mapping enables LLMs to accurately understand field constraints
- **Interoperability**: Modules implemented in different languages can exchange data through unified JSON format

### 1.2 Scope

This specification covers type mappings for the following languages: **Python**, **Rust**, and **TypeScript** (which have official SDK implementations). Type mappings for **Go** and **Java** are provided as reference for future implementers but do not have official SDKs at this time. Specific library choices for schema validation are left to each SDK implementation.

### 1.3 Terminology

- **JSON Schema Type**: `type` values defined in JSON Schema Draft 2020-12
- **Native Type**: Corresponding built-in or standard library types in each programming language
- **Serialization Format**: Representation format of types in JSON transmission
- **Round-trip Fidelity**: Whether data remains unchanged after serialization → transmission → deserialization

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
| Rust | `String` | UTF-8 heap-allocated string |
| Go | `string` | UTF-8 immutable string |
| Java | `String` | UTF-16 internal encoding |
| TypeScript | `string` | UTF-16 internal encoding |

**Notes:**

- All implementations **MUST** support the complete Unicode character set
- JSON transmission **MUST** use UTF-8 encoding
- Java/TypeScript use UTF-16 internally; special attention needed for character length calculation when dealing with surrogate pairs

### 2.2 Integer Type (`integer`)

**JSON Schema Definition:**

```yaml
type: integer
```

**Cross-language Mappings:**

| Language | Native Type | Range | Notes |
|------|---------|------|------|
| Python | `int` | Arbitrary precision | Python natively supports big integers |
| Rust | `i64` | -2^63 ~ 2^63-1 | Can use `i128` or `BigInt` for larger range |
| Go | `int64` | -2^63 ~ 2^63-1 | — |
| Java | `long` | -2^63 ~ 2^63-1 | Can use `BigInteger` for larger range |
| TypeScript | `number` | -(2^53-1) ~ 2^53-1 safe range | IEEE 754 double precision, see boundary cases |

**Supported Constraints:** `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`. All SDK implementations **MUST** enforce these constraints during validation.

### 2.3 Number Type (`number`)

**JSON Schema Definition:**

```yaml
type: number
```

**Cross-language Mappings:**

| Language | Native Type | Precision | Notes |
|------|---------|------|------|
| Python | `float` | IEEE 754 double precision | Can also use `Decimal` for high precision |
| Rust | `f64` | IEEE 754 double precision | — |
| Go | `float64` | IEEE 754 double precision | — |
| Java | `double` | IEEE 754 double precision | Can use `BigDecimal` for high precision |
| TypeScript | `number` | IEEE 754 double precision | — |

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
| Rust | `bool` | — |
| Go | `bool` | — |
| Java | `boolean` / `Boolean` | Primitive type / wrapper class |
| TypeScript | `boolean` | — |

**Notes:**

- JSON **MUST** use `true` / `false`, does not accept variants like `0` / `1` / `"true"` / `"false"`
- This holds at the module-invocation boundary unconditionally: `0`, `1`, `"true"` and `"false"` **MUST** be rejected for a `boolean`, and no host configuration may relax it (§17.3)

### 2.5 Null Type (`null`)

**JSON Schema Definition:**

```yaml
type: "null"
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `None` | — |
| Rust | `()` or `Option::None` | Usually not used alone, combined with `Option<T>` |
| Go | `nil` | — |
| Java | `null` | — |
| TypeScript | `null` | Distinct from `undefined` |

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
| Python | `class MyModel` | Class with typed fields |
| Rust | `struct MyStruct { ... }` | Struct with typed fields |
| Go | `type MyStruct struct { ... }` | Struct with JSON tags |
| Java | `class MyClass { ... }` | Class with typed fields |
| TypeScript | `interface MyType { ... }` | Interface or object schema |

**Supported Constraints:** `minProperties`, `maxProperties` (§6.5.1–§6.5.2), `required` (§6.5.3), `dependentRequired` (§6.5.4), `patternProperties`, `additionalProperties`, `propertyNames` (§10.3.2), `dependentSchemas` (§10.2.2.4) and `unevaluatedProperties` (§11.3). All SDK implementations **MUST** enforce these during validation — see §17 for the per-keyword requirement and its rationale.

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
| Python | `list[str]` | Generic list |
| Rust | `Vec<String>` | — |
| Go | `[]string` | Slice type |
| Java | `List<String>` | Usually uses `ArrayList` |
| TypeScript | `string[]` or `Array<string>` | — |

**Supported Constraints:** `minItems`, `maxItems`, `uniqueItems`, `contains` / `minContains` / `maxContains`, `prefixItems` and `unevaluatedItems`. All SDK implementations **MUST** enforce these constraints during validation — see §17 for the per-keyword requirement and its rationale.

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
| Python | `list[ErrorDetail]` (where `ErrorDetail` is a typed class) |
| Rust | `Vec<ErrorDetail>` |
| Go | `[]ErrorDetail` |
| Java | `List<ErrorDetail>` |
| TypeScript | `ErrorDetail[]` |

---

## 4. Nullable Type Mappings

### 4.1 Nullable Type (`T | null`)

**JSON Schema Definition (using `oneOf`):**

```yaml
oneOf:
  - type: string
  - type: "null"
```

**Or using Draft 2020-12 array type syntax:**

```yaml
type: [string, "null"]
```

**Cross-language Mappings:**

| Language | Native Type | Notes |
|------|---------|------|
| Python | `str \| None` or `Optional[str]` | `str \| None` syntax preferred |
| Rust | `Option<String>` | Rust type system natively supports |
| Go | `*string` | Pointer type represents nullable |
| Java | `@Nullable String` or `Optional<String>` | Needs annotation marking |
| TypeScript | `string \| null` | — |

**Serialization Rules:**

- When value is `null`, JSON **MUST** output as `null` (not omit the field)
- This has different semantics from "optional field omission" (see §6.1)

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
| Rust | `enum Status { Pending, Running, ... }` | Map to snake_case for JSON serialization |
| Go | `type Status string` + constants | `const StatusPending Status = "pending"` |
| Java | `enum Status { PENDING("pending"), ... }` | Map to snake_case for JSON serialization |
| TypeScript | `z.enum(["pending", "running", ...])` | Or `type Status = "pending" \| "running" \| ...` |

### 5.2 Integer Enum

**JSON Schema Definition:**

```yaml
type: integer
enum: [0, 1, 2, 3]
```

**Cross-language Mappings:**

| Language | Native Type | Example |
|------|---------|------|
| Python | `IntEnum` | `class Priority(IntEnum): LOW = 0` |
| Rust | `#[repr(i64)] enum Priority { Low = 0, ... }` | — |
| Go | `type Priority int64` + `iota` constants | — |
| Java | `enum Priority { LOW(0), ... }` | — |
| TypeScript | `z.union([z.literal(0), z.literal(1), ...])` | Or `const enum` |

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
# note not in required, is optional field
```

**Cross-language Mappings:**

| Language | Required Field (`name`) | Optional Field (`note`) | Notes |
|------|-------------------|-------------------|------|
| Python | `name: str` | `note: str = ""` or `note: str \| None = None` | Use default value if has default, otherwise use `None` |
| Rust | `name: String` | `note: Option<String>` with default | — |
| Go | `Name string \`json:"name"\`` | `Note *string \`json:"note,omitempty"\`` | Pointer + omitempty |
| Java | `@NotNull String name` | `String note` (can be null) | — |
| TypeScript | `name: string` | `note?: string` | Optional property syntax |

**Important Distinctions:**

| Semantics | JSON Representation | Description |
|------|----------|------|
| Field missing (optional field not provided) | Key does not exist | Uses Schema's `default` value or language zero value |
| Field is null (explicit null value) | `"field": null` | Requires nullable declaration |
| Field is empty string | `"field": ""` | Has value, but empty |

---

## 7. Date and Time Type Mappings

### 7.1 `date-time` Format

**JSON Schema Definition:**

```yaml
type: string
format: date-time
```

**Cross-language Mappings:**

| Language | Native Type | JSON Format | Example |
|------|---------|----------|------|
| Python | `datetime` | ISO 8601 | `"2026-02-07T10:30:00Z"` |
| Rust | `chrono::DateTime<Utc>` | ISO 8601 | `"2026-02-07T10:30:00Z"` |
| Go | `time.Time` | RFC 3339 | `"2026-02-07T10:30:00Z"` |
| Java | `OffsetDateTime` / `Instant` | ISO 8601 | `"2026-02-07T10:30:00Z"` |
| TypeScript | `Date` or `string` | ISO 8601 | `"2026-02-07T10:30:00Z"` |

**Notes:**

- All implementations **MUST** output ISO 8601 / RFC 3339 format when serializing
- Implementations **SHOULD** use UTC timezone (`Z` suffix) unless business explicitly requires timezone offset
- Deserialization **MUST** accept formats with timezone offsets (e.g., `+08:00`)

### 7.2 `date` Format

**JSON Schema Definition:**

```yaml
type: string
format: date
```

**Cross-language Mappings:**

| Language | Native Type | JSON Format | Example |
|------|---------|----------|------|
| Python | `date` | ISO 8601 | `"2026-02-07"` |
| Rust | `chrono::NaiveDate` | ISO 8601 | `"2026-02-07"` |
| Go | `civil.Date` (or custom type) | ISO 8601 | `"2026-02-07"` |
| Java | `LocalDate` | ISO 8601 | `"2026-02-07"` |
| TypeScript | `string` | ISO 8601 | `"2026-02-07"` |

### 7.3 `time` Format

**JSON Schema Definition:**

```yaml
type: string
format: time
```

**Cross-language Mappings:**

| Language | Native Type | JSON Format | Example |
|------|---------|----------|------|
| Python | `time` | ISO 8601 | `"10:30:00"` |
| Rust | `chrono::NaiveTime` | ISO 8601 | `"10:30:00"` |
| Go | Custom type | ISO 8601 | `"10:30:00"` |
| Java | `LocalTime` | ISO 8601 | `"10:30:00"` |
| TypeScript | `string` | ISO 8601 | `"10:30:00"` |

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
| Python | Nested class | `class Address` + `class User` |
| Rust | Nested struct | `struct Address { ... }` + `struct User { ... }` |
| Go | Nested struct (can inline) | `type Address struct { ... }` + `type User struct { ... }` |
| Java | Nested class or separate class | `class Address { ... }` + `class User { ... }` |
| TypeScript | Nested interface or schema | Nested type definitions |

**Naming Convention:**

- Nested objects **SHOULD** be extracted as independent named types
- Type names **SHOULD** be generated based on property paths (e.g., `user.address` corresponds to `UserAddress`)
- When Schema uses `$ref` references (see PROTOCOL_SPEC §4.10), all languages **MUST** map to the same shared type

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
| Python | `EmailNotification \| SmsNotification` (Discriminated Union) | Uses `discriminator` field |
| Rust | `enum Notification { Email(EmailData), Sms(SmsData) }` | Tagged union |
| Go | `interface{}` + runtime judgment | Go lacks native union types |
| Java | Sealed Class or `@JsonSubTypes` | Java 17+ recommends sealed classes |
| TypeScript | `z.discriminatedUnion("type", [...])` | Type guard |

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
| Rust | `enum StringOrInt { Str(String), Int(i64) }` | Untagged union |
| Go | `interface{}` | Runtime type assertion |
| Java | `Object` + runtime type check | Or use custom wrapper class |
| TypeScript | `string \| number` | — |

**Implementation Recommendations:**

- If `anyOf` contains `null`, equivalent to nullable (see §4.1)
- For cases where all branches in `anyOf` are object types, **recommend** using discriminated union
- Static type languages (Rust, Go, Java) may need additional runtime dispatch logic when handling `anyOf`

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
| Rust | `HashMap<String, String>` | — |
| Go | `map[string]string` | — |
| Java | `Map<String, String>` | Usually uses `HashMap` |
| TypeScript | `Record<string, string>` | — |

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
| Python | Fixed fields + extra fields allowed via config | — |
| Rust | Fixed fields + flattened extra `HashMap<String, i64>` | — |
| Go | Fixed fields + `Extra map[string]int64` with custom JSON codec | — |
| Java | Fixed fields + `@JsonAnySetter Map<String, Integer>` | — |
| TypeScript | Object schema with catchall type | — |

### 10.3 `additionalProperties: false`

When `input_schema` declares `additionalProperties: false` (PROTOCOL_SPEC §4.2 **SHOULD**), implementations **MUST** reject inputs containing unknown fields.

A field is "unknown" only if neither `properties` nor `patternProperties` claimed it (JSON Schema 2020-12 §10.3.2.3). A key matched by a `patternProperties` entry **MUST NOT** be rejected by `additionalProperties: false`.

To close an object *after* the branches of `allOf` / `anyOf` / `oneOf` / `if`-`then`-`else` have contributed their own properties, use `unevaluatedProperties: false` instead — `additionalProperties` cannot see those branches. See §17.2.

---

## 11. Format Constraint Mappings

### 11.1 `format` Keyword

`format` carries a semantic hint about a `string` value. It is an **annotation, not an assertion**: under the default format-annotation vocabulary of JSON Schema 2020-12 §7.2.1, a value that does not satisfy its declared `format` **MUST NOT** fail validation. Implementations **SHOULD** recognise the formats below and emit a warning when a value does not conform, and **MUST** accept the value regardless. A `format` the implementation does not recognise is collected as an annotation and **MUST** pass silently — a contract is free to declare `format: "path"` or any other vocabulary term without becoming uncallable.

Recognised formats:

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

### 11.2 Format Validation Requirements

All SDK implementations **SHOULD** check a value against its declared `format` when the format appears in §11.1, and **SHOULD** report a non-conforming value as a warning. The check **MUST NOT** produce `SCHEMA_VALIDATION_ERROR` — see §11.1. The specific validation libraries and methods are left to each SDK implementation.

To make a format binding, express it as an assertion the vocabulary already carries: `pattern` for a syntactic shape, or `enum` for a closed set. `format` alone never rejects.

**Where the warning is emitted is not yet uniform.** apcore-typescript and apcore-python emit it on the module-invocation path. apcore-rust computes format warnings in `SchemaValidator::validate_detailed_raw`, but its module-invocation path does not go through `SchemaValidator` at all — the executor's `validate_against_schema` builds its own `jsonschema` validator and returns on `is_valid`, so a Rust module call emits no format warning. Making it uniform means calling the warning path from `validate_against_schema`, not rerouting `SchemaValidator`. The conformance fixture `schema_hardening_formats.json` asserts the annotation semantics (validation passes) on all three SDKs; its `warn_logged` expectations are satisfied through a direct call to the warning path, not through a module invocation, so they do not pin this difference.

---

## 12. Complete Type Mapping Table

The following table summarizes all JSON Schema type to language mappings:

| JSON Schema | Python | Rust | Go | Java | TypeScript |
|-------------|--------|------|----|------|------------|
| `string` | `str` | `String` | `string` | `String` | `string` |
| `integer` | `int` | `i64` | `int64` | `long` / `Long` | `number` |
| `number` | `float` | `f64` | `float64` | `double` / `Double` | `number` |
| `boolean` | `bool` | `bool` | `bool` | `boolean` / `Boolean` | `boolean` |
| `null` | `None` | `()` | `nil` | `null` | `null` |
| `object` (with properties) | class | `struct` | `struct` | `class` | interface / object |
| `array` (with items) | `list[T]` | `Vec<T>` | `[]T` | `List<T>` | `T[]` |
| `T \| null` | `T \| None` | `Option<T>` | `*T` | `@Nullable T` | `T \| null` |
| `string enum` | `Literal[...]` / `StrEnum` | `enum` | `type T string` + const | `enum` | union type |
| `integer enum` | `IntEnum` | `enum` | `type T int64` + iota | `enum` | union type |
| `oneOf` | Discriminated Union | `enum` (tagged) | `interface{}` | Sealed Class | discriminated union |
| `anyOf` | Union type | `enum` (untagged) | `interface{}` | `Object` | union type |
| `additionalProperties` | `dict[str, V]` | `HashMap<String, V>` | `map[string]V` | `Map<String, V>` | `Record<string, V>` |
| `string` + `format: date-time` | `datetime` | `DateTime<Utc>` | `time.Time` | `OffsetDateTime` | `Date` / `string` |
| `string` + `format: date` | `date` | `NaiveDate` | `civil.Date` | `LocalDate` | `string` |
| `string` + `format: time` | `time` | `NaiveTime` | Custom | `LocalTime` | `string` |
| `string` + `format: email` | `str` + validation | `String` + validation | `string` + validation | `String` + validation | `string` + validation |
| `string` + `format: uri` | `str` + validation | `String` + validation | `string` + validation | `String` + validation | `string` + validation |
| `string` + `format: uuid` | `UUID` | `Uuid` | `uuid.UUID` | `UUID` | `string` + validation |

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
| `object` | **MUST** perfect round-trip | Field order **may** differ |
| `array` | **MUST** perfect round-trip | Element order **MUST** be preserved |

### 13.2 Serialization Specifications

| Rule | Level | Description |
|------|------|------|
| Output **MUST** be valid JSON | **MUST** | RFC 8259 |
| Character encoding **MUST** be UTF-8 | **MUST** | — |
| Integers **MUST NOT** serialize as floats | **MUST** | `42` not `42.0` |
| Floats **MUST** preserve decimal part | **MUST** | `3.14` not `3` |
| `null` fields **SHOULD** be explicitly output | **SHOULD** | `{"field": null}` |
| Object key order **may** not be guaranteed | **MAY** | But **SHOULD** maintain stable output |

---

## 14. Boundary Cases and Known Issues

### 14.1 Large Integer Precision Loss

**Problem Description:**

JavaScript (TypeScript runtime) uses IEEE 754 double-precision floating-point numbers to represent all numbers, with safe integer range of `-(2^53 - 1)` to `2^53 - 1`. Integers outside this range will lose precision.

**Impact Range:**

| Language | Integer Range | Is Affected |
|------|---------|-----------|
| Python | Arbitrary precision | No |
| Rust | i64 (-2^63 ~ 2^63-1) | Partial (when exceeding i64 range) |
| Go | int64 (-2^63 ~ 2^63-1) | Partial |
| Java | long (-2^63 ~ 2^63-1) | Partial |
| TypeScript | number (safe range 2^53-1) | Yes |

**Cross-language Safe Boundary:**

apcore defines **2^53 - 1** (i.e., `9007199254740991`, JavaScript `Number.MAX_SAFE_INTEGER`) as the cross-language integer safe boundary. This boundary is determined by the weakest consumer (JavaScript/TypeScript) in JSON specification.

**Specification Requirements:**

| Rule | Level | Description |
|------|------|------|
| Integers with absolute value ≤ 2^53 - 1 | **MUST** use `type: integer` | All languages can handle losslessly |
| Integers with absolute value > 2^53 - 1 | **MUST** use `type: string` + `format` | Avoid JavaScript precision loss |
| Schema **SHOULD** explicitly declare `minimum` / `maximum` | **SHOULD** | Help languages choose appropriate native types |

**Large Number `format` Specification:**

Values exceeding the safe boundary **MUST** be transmitted using `type: string`, with `format` indicating semantics:

| `format` Value | Meaning | Range | Language Mappings |
|-------------|------|------|-----------|
| `int64` | 64-bit signed integer | -2^63 ~ 2^63-1 | Python `int`, Rust `i64`, Go `int64`, Java `long`, TS `BigInt` |
| `bigint` | Arbitrary precision integer | Unlimited | Python `int`, Rust `num_bigint::BigInt`, Go `math/big.Int`, Java `BigInteger`, TS `BigInt` |
| `decimal` | High precision decimal | Unlimited | Python `Decimal`, Rust `rust_decimal::Decimal`, Go `shopspring/decimal`, Java `BigDecimal`, TS `decimal.js` |

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

1. For high-precision scenarios like financial calculations, **recommend** using `string` type for transmission, with each language converting to its native high-precision decimal type
2. Use `x-precision` extension field in Schema to annotate precision requirements

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

Enum values and plain strings are completely identical in transmission format (e.g., `"pending"`); need Schema information to distinguish.

**Specification Requirements:**

- Deserialization **MUST** refer to Schema's `enum` constraint for validation
- If input value is not in `enum` list, **MUST** return `SCHEMA_VALIDATION_ERROR`

---

## 15. Reserved Keyword Adaptations

Some spec-defined method names conflict with language reserved keywords. Implementations **MUST** provide the equivalent functionality under a language-idiomatic alternative name. The following table documents all known keyword conflicts and their canonical adaptations:

| Spec Method | Python | TypeScript | Rust | Reason |
|---|---|---|---|---|
| `use(middleware)` | `use(middleware)` | `use(middleware)` | `use_middleware(middleware)` | `use` is a Rust reserved keyword |

**Rules:**

- When a spec method name is a reserved keyword in a target language, the SDK **MUST** choose a name that preserves the verb and adds a noun suffix describing the argument (e.g., `use` → `use_middleware`).
- The adapted name **MUST** be documented in the SDK's README API Overview section.
- Implementations **MUST NOT** rely on language-specific escape mechanisms (e.g., Rust raw identifiers `r#keyword`, Kotlin backtick-quoted identifiers `` `keyword` ``) as the primary API surface. The adapted name **MUST** be a natural identifier in the target language.
- Cross-language sync checks **MUST** treat the adapted name as equivalent to the spec name and **MUST NOT** flag it as a divergence.

**Maintenance:** SDK implementers **SHOULD** check for keyword conflicts when adding new public methods and update this table accordingly. The table above is maintained as conflicts are discovered — it is not an exhaustive pre-analysis of all possible keyword collisions.

---

## 16. Per-SDK Validation Library Notes

### 16.1 TypeScript SDK — TypeBox

The TypeScript SDK uses **`@sinclair/typebox`** (^0.34) for JSON Schema–shaped validation rather than a standalone Draft 2020-12 validator (e.g., `ajv`). TypeBox is a schema builder/validator hybrid that provides static TypeScript types from the same schema object.

**Keywords TypeBox validates natively** (through `Value.Check` / `TypeCompiler`):
`type`, `properties`, `required`, `enum`, `const`, `items`, `minItems`, `maxItems`, `uniqueItems`, `contains`, `minContains`, `maxContains`, `minProperties`, `maxProperties`, `additionalProperties`, `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`, `minLength`, `maxLength`, `pattern`, `format` (carried as an annotation, see §11), `$ref` (within the same schema — not cross-file).

**Keywords with no TypeBox node**, supplied by the SDK's own evaluator and registered as a custom TypeBox kind so `Value.Check` reaches them: `prefixItems`, `patternProperties`, `propertyNames`, `dependentRequired`, `dependentSchemas`, `if` / `then` / `else`, `unevaluatedItems`, `unevaluatedProperties`. Sub-schemas of these keywords are handed back to the converter, so there is still exactly one validation engine. The requirement level for each is §17 — the absence of a library node is **not** a licence to drop the keyword.

**Remaining limitation vs. full Draft 2020-12:** `$ref` to external URIs / files is not supported; use `apcore`'s schema registry for cross-module references.

**Python SDK** uses `jsonschema ^4.21` (fully Draft 2020-12 conformant); keywords Pydantic cannot express are delegated to it as a sub-schema assertion.
**Rust SDK** uses `jsonschema ^0.28` (pre-1.0; conformance grows with each minor; check release notes) and hands it the raw schema, so every keyword in §17 is enforced by the library directly.

---

## 17. Validation Keyword Conformance

§2–§11 describe how *types* map. This section states, keyword by keyword, what an SDK is required to do at the validation boundary — the path a module invocation actually takes (schema-to-native conversion followed by the SDK's validator), not a side-channel raw-schema check.

It exists because "partial support" is not a specification. While this document said `if/then/else`, `contains` and `prefixItems` were "partially supported" and said nothing at all about `patternProperties`, `propertyNames`, `dependentRequired` or `minProperties`/`maxProperties`, the three SDKs drifted: apcore-rust rejected inputs that apcore-typescript and apcore-python accepted, from the identical contract. A caller cannot reason about a contract whose constraints are enforced in one runtime and ignored in another.

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
| `if`, `then`, `else` | §10.2.2.1–§10.2.2.3 | **MUST** enforce | `if` never fails an instance on its own; it selects `then` or `else`, and a missing branch asserts nothing. Rejecting the schema outright (as apcore-python once did) is **NOT** conformant. |
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

**This is not host-configurable.** A module's input contract has to mean the same thing regardless of which host loaded it. If a host could switch coercion on, the same module would accept `{"count": "3"}` in one deployment and reject it in another — the contract would no longer be a contract, and a caller could not reason about it without also knowing the deployment's configuration. That is why there is no `schema.validation.coerce_types` key: the `schema` namespace is `root` / `strategy` / `max_ref_depth` and nothing else, and `defaults.schema.json` declares it `additionalProperties: false` (PROTOCOL_SPEC §4.9). Earlier revisions of this section referenced such a key; no SDK ever read it.

**What "no coercion" does *not* mean.** The rule is about instance *types*, not renderings. JSON Schema 2020-12 §6.1.1 defines `integer` as any number with a zero fractional part, so `4.0` **MUST** satisfy `{"type": "integer"}` while `4.5` **MUST NOT**; `42` **MUST** satisfy `{"type": "number"}`. An SDK whose native integer type cannot hold `4.0` has to narrow it — that is honouring the type definition, not relaxing it. (This is a real trap: pydantic's strict mode rejects `4.0` for `int`, so apcore-python normalises the zero-fraction case before the check.)

**Library-level knob.** An SDK **MAY** keep a coercion switch on its standalone validator API — apcore-python `SchemaValidator(coerce_types=…)`, apcore-typescript `new SchemaValidator(…)`, apcore-rust `SchemaValidator::with_coerce_types(…)` — for callers validating their *own* untyped input (a CLI parsing argv, a form handler). Such a knob **MUST NOT** reach the module-invocation path, **MUST NOT** be readable from a configuration file, and its default **SHOULD** be no-coercion so the two paths cannot silently disagree. apcore-rust shipped a validator whose default coerced while its executor path did not, and the two answered differently for the same schema and input; that is the failure mode this paragraph exists to prevent.

**Keyword slicing.** An SDK that delegates part of a schema to a strict Draft 2020-12 engine **SHOULD** delegate the applicator keywords alone rather than the whole schema, so a `type` its own conversion already enforced is not re-asserted twice over a differently-shaped value. The one documented exception is `unevaluatedItems` / `unevaluatedProperties`, which are defined against the annotations of every sibling keyword and therefore cannot be evaluated from a slice.

**Fixture.** `conformance/fixtures/schema_keyword_parity.json` asserts R5 at the boundary; the opt-in library-level coercing mode is covered separately by `conformance/fixtures/schema_validation.json` (`expected_valid_strict` / `expected_valid_coerce`).

---

## 18. References

- [PROTOCOL_SPEC §4 — Schema Specification](./protocol-spec.md#4-schema-specification)
- [PROTOCOL_SPEC §4.10 — Language-specific Schema Implementations](./protocol-spec.md#410-language-specific-schema-implementations)
- [PROTOCOL_SPEC §4.11 — Schema References ($ref)](./protocol-spec.md#411-schema-references-ref)
- [PROTOCOL_SPEC §12.3 — Cross-language Implementation Requirements](./protocol-spec.md#123-cross-language-implementation-requirements)
- [JSON Schema Draft 2020-12](https://json-schema.org/draft/2020-12/json-schema-core)
- [RFC 8259 — The JavaScript Object Notation (JSON) Data Interchange Format](https://www.rfc-editor.org/rfc/rfc8259)
