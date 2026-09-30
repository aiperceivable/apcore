---
description: "Defining apcore module input/output schemas with Pydantic, TypeBox, or serde_json and in shared YAML files: field types, constraints, x-* LLM extension fields, references, and AI-friendly design."
---

# Schema Definition Guide

> Every apcore module declares an input schema and an output schema. This guide shows how to write them well in each SDK.

## 1. Why Schemas Are Required

```text
A plain function:
    def process(data):      # What is data? Unknown
        return result       # What is result? Unknown

An apcore module:
    input_schema  = ProcessInput     # input structure is declared
    output_schema = ProcessOutput    # output structure is declared

    def execute(inputs, context):
        # inputs were validated before this runs
        return {...}                 # the output is validated after it returns
```

| Purpose | What the schema gives you |
|------|------|
| **AI understanding** | An LLM learns how to call the module from the schema and its descriptions |
| **Validation** | The executor validates inputs before `execute()` and outputs after it |
| **Documentation** | Schemas are exported for tool listings and API docs |
| **Type safety** | Native types (Pydantic, TypeBox `Static<>`, serde structs) during development |
| **Cross-language contracts** | A YAML schema file can be loaded by all three SDKs |

All schemas are JSON Schema Draft 2020-12 plus `x-` extension fields ([protocol-spec §4.2](../spec/protocol-spec.md#42-schema-format)).

---

## 2. Two Ways to Write a Schema

### 2.1 In Code (Recommended)

Each SDK uses its language's natural tool: **Pydantic** in Python, **TypeBox** in TypeScript, and `serde_json::json!` in Rust.

=== "Python"

    ```python
    from typing import Literal

    from pydantic import BaseModel, Field


    class Address(BaseModel):
        """Shipping address."""

        province: str = Field(..., description="Province")
        city: str = Field(..., description="City")
        district: str = Field(..., description="District")
        detail: str = Field(..., description="Street address")
        postal_code: str = Field(..., description="Postal code", pattern=r"^\d{6}$")


    class OrderInput(BaseModel):
        """Order creation input."""

        # Required fields (`...` means required)
        product_id: str = Field(..., description="Product ID", min_length=1, max_length=50)
        quantity: int = Field(..., description="Purchase quantity", ge=1, le=100)

        # Optional fields carry a default
        note: str | None = Field(None, description="Order note", max_length=500)
        payment_method: Literal["alipay", "wechat", "card"] = Field(
            "alipay", description="Payment method"
        )

        # Nested object
        shipping_address: Address = Field(..., description="Shipping address")
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const Address = Type.Object({
      province: Type.String({ description: 'Province' }),
      city: Type.String({ description: 'City' }),
      district: Type.String({ description: 'District' }),
      detail: Type.String({ description: 'Street address' }),
      postal_code: Type.String({ description: 'Postal code', pattern: '^\\d{6}$' }),
    });

    export const OrderInput = Type.Object({
      // Required fields
      product_id: Type.String({ description: 'Product ID', minLength: 1, maxLength: 50 }),
      quantity: Type.Integer({ description: 'Purchase quantity', minimum: 1, maximum: 100 }),

      // Optional fields
      note: Type.Optional(
        Type.Union([Type.String({ maxLength: 500 }), Type.Null()], {
          description: 'Order note',
          default: null,
        }),
      ),
      payment_method: Type.Union(
        [Type.Literal('alipay'), Type.Literal('wechat'), Type.Literal('card')],
        { description: 'Payment method', default: 'alipay' },
      ),

      // Nested object (Composite copies Address and adds a description)
      shipping_address: Type.Composite([Address], { description: 'Shipping address' }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn address_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "province":    { "type": "string", "description": "Province" },
                "city":        { "type": "string", "description": "City" },
                "district":    { "type": "string", "description": "District" },
                "detail":      { "type": "string", "description": "Street address" },
                "postal_code": { "type": "string", "description": "Postal code", "pattern": "^\\d{6}$" }
            },
            "required": ["province", "city", "district", "detail", "postal_code"]
        })
    }

    pub fn order_input_schema() -> Value {
        let mut shipping_address = address_schema();
        shipping_address["description"] = json!("Shipping address");
        json!({
            "type": "object",
            "properties": {
                // Required fields
                "product_id": { "type": "string", "description": "Product ID", "minLength": 1, "maxLength": 50 },
                "quantity":   { "type": "integer", "description": "Purchase quantity", "minimum": 1, "maximum": 100 },
                // Optional fields
                "note": {
                    "type": ["string", "null"],
                    "description": "Order note",
                    "maxLength": 500,
                    "default": null
                },
                "payment_method": {
                    "type": "string",
                    "description": "Payment method",
                    "enum": ["alipay", "wechat", "card"],
                    "default": "alipay"
                },
                // Nested object
                "shipping_address": shipping_address
            },
            "required": ["product_id", "quantity", "shipping_address"]
        })
    }
    ```

!!! note "Python validates strictly"
    The Python executor validates inputs with Pydantic in strict mode and passes the inputs **as sent** to `execute()`. Two consequences: a `datetime`, `date`, `time`, or `Enum` field rejects the plain JSON string a caller sends, so declare those fields as `str` (with a `format`) or `Literal[...]` and parse them inside `execute()`; and a validator can reject a value but cannot rewrite what `execute()` receives. See [§3.4](#34-enum-types), [§3.5](#35-date-and-time), and [§7](#7-custom-validation).

### 2.2 In a YAML Schema File (Cross-Language)

A `*.schema.yaml` file under `schema.root` holds a module's schemas so that every SDK can load the same contract at runtime (see [Multi-Language § Sharing a YAML Schema](./multi-language.md#3-sharing-a-yaml-schema)):

```yaml
# schemas/executor/order/create_order.schema.yaml
$schema: "https://apcore.dev/schema/v1"
version: "1.0.0"
module_id: "executor.order.create_order"
description: "Create an order for one product and ship it to an address"

input_schema:
  type: object
  properties:
    product_id:
      type: string
      description: "Product ID"
      minLength: 1
      maxLength: 50
    quantity:
      type: integer
      description: "Purchase quantity"
      minimum: 1
      maximum: 100
    note:
      type: ["string", "null"]
      description: "Order note"
      maxLength: 500
      default: null
    payment_method:
      type: string
      description: "Payment method"
      enum: ["alipay", "wechat", "card"]
      default: "alipay"
    shipping_address:
      $ref: "#/$defs/Address"
      description: "Shipping address"
  required: [product_id, quantity, shipping_address]
  additionalProperties: false

output_schema:
  type: object
  properties:
    order_id:
      type: string
      description: "Order ID"
    status:
      type: string
      description: "Order status"
      enum: ["created", "pending", "paid", "failed"]
    total_amount:
      type: number
      description: "Total order amount"
    created_at:
      type: string
      format: date-time
      description: "Creation time"
  required: [order_id, status, total_amount, created_at]

$defs:
  Address:
    type: object
    properties:
      province:
        type: string
        description: "Province"
      city:
        type: string
        description: "City"
      district:
        type: string
        description: "District"
      detail:
        type: string
        description: "Street address"
      postal_code:
        type: string
        description: "Postal code"
        pattern: "^\\d{6}$"
    required: [province, city, district, detail, postal_code]
```

`description`, `input_schema`, and `output_schema` are required; `description` is at most 200 characters. A local `#/$defs/...` reference resolves against the file root, and keys written beside a `$ref` (such as `description` above) are kept.

---

## 3. Field Types

How each JSON Schema type maps to a native type in each SDK is specified once, in the [Type Mapping Specification](../spec/type-mapping.md). The examples below show how to declare each kind of field.

### 3.1 Basic Types

=== "Python"

    ```python
    from typing import Any

    from pydantic import BaseModel, Field


    class BasicTypes(BaseModel):
        name: str = Field(..., description="Name")               # string
        age: int = Field(..., description="Age")                 # integer
        price: float = Field(..., description="Price")           # number
        active: bool = Field(..., description="Is active")       # boolean
        data: Any = Field(..., description="Any data")           # any value (avoid when possible)
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const BasicTypes = Type.Object({
      name: Type.String({ description: 'Name' }),          // string
      age: Type.Integer({ description: 'Age' }),           // integer
      price: Type.Number({ description: 'Price' }),        // number
      active: Type.Boolean({ description: 'Is active' }),  // boolean
      data: Type.Unknown({ description: 'Any data' }),     // any value (avoid when possible)
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn basic_types_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name":   { "type": "string",  "description": "Name" },
                "age":    { "type": "integer", "description": "Age" },
                "price":  { "type": "number",  "description": "Price" },
                "active": { "type": "boolean", "description": "Is active" },
                // any value (avoid when possible)
                "data":   { "description": "Any data" }
            },
            "required": ["name", "age", "price", "active", "data"]
        })
    }
    ```

### 3.2 Optional and Nullable Fields

A field is **optional** when it is not listed in `required`; it is **nullable** when its `type` includes `"null"`. JSON Schema has no `nullable` keyword — write `type: [T, "null"]`.

=== "Python"

    ```python
    from pydantic import BaseModel, Field


    class OptionalTypes(BaseModel):
        # Optional and nullable: may be absent or null
        email: str | None = Field(None, description="Email")

        # Optional, never null: absent means the default
        count: int = Field(0, description="Count")
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const OptionalTypes = Type.Object({
      // Optional and nullable: may be absent or null
      email: Type.Optional(
        Type.Union([Type.String(), Type.Null()], { description: 'Email', default: null }),
      ),

      // Optional, never null: absent means the default
      count: Type.Optional(Type.Integer({ description: 'Count', default: 0 })),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn optional_types_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // Optional and nullable: may be absent or null
                "email": { "type": ["string", "null"], "description": "Email", "default": null },
                // Optional, never null: absent means the default
                "count": { "type": "integer", "description": "Count", "default": 0 }
            }
            // No "required" array: both fields are optional
        })
    }
    ```

### 3.3 Lists and Maps

=== "Python"

    ```python
    from typing import Any

    from pydantic import BaseModel, Field


    class OrderItem(BaseModel):
        product_id: str = Field(..., description="Product ID")
        quantity: int = Field(..., description="Quantity")
        price: float = Field(..., description="Unit price")


    class CollectionTypes(BaseModel):
        tags: list[str] = Field(default_factory=list, description="Tag list")
        items: list[OrderItem] = Field(..., description="Order items")
        metadata: dict[str, Any] = Field(default_factory=dict, description="Metadata")
        scores: dict[str, int] = Field(default_factory=dict, description="Score per user")
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const OrderItem = Type.Object({
      product_id: Type.String({ description: 'Product ID' }),
      quantity: Type.Integer({ description: 'Quantity' }),
      price: Type.Number({ description: 'Unit price' }),
    });

    export const CollectionTypes = Type.Object({
      tags: Type.Optional(Type.Array(Type.String(), { description: 'Tag list', default: [] })),
      items: Type.Array(OrderItem, { description: 'Order items' }),
      metadata: Type.Optional(
        Type.Record(Type.String(), Type.Unknown(), { description: 'Metadata', default: {} }),
      ),
      scores: Type.Optional(
        Type.Record(Type.String(), Type.Integer(), { description: 'Score per user', default: {} }),
      ),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn collection_types_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tag list",
                    "default": []
                },
                "items": {
                    "type": "array",
                    "description": "Order items",
                    "items": {
                        "type": "object",
                        "properties": {
                            "product_id": { "type": "string",  "description": "Product ID" },
                            "quantity":   { "type": "integer", "description": "Quantity" },
                            "price":      { "type": "number",  "description": "Unit price" }
                        },
                        "required": ["product_id", "quantity", "price"]
                    }
                },
                "metadata": {
                    "type": "object",
                    "additionalProperties": true,
                    "description": "Metadata",
                    "default": {}
                },
                "scores": {
                    "type": "object",
                    "additionalProperties": { "type": "integer" },
                    "description": "Score per user",
                    "default": {}
                }
            },
            "required": ["items"]
        })
    }
    ```

A map is an `object` whose values are described by `additionalProperties`; a list is an `array` whose elements are described by `items`.

### 3.4 Enum Types

=== "Python"

    ```python
    from typing import Literal

    from pydantic import BaseModel, Field

    OrderStatus = Literal["pending", "paid", "shipped", "done"]  # reusable alias


    class Order(BaseModel):
        status: OrderStatus = Field(..., description="Order status")
        priority: Literal[1, 2, 3] = Field(2, description="Priority: 1 high, 2 medium, 3 low")
    ```

    Use `Literal[...]` rather than an `Enum` class: under strict validation an `Enum` field rejects the plain string a caller sends.

=== "TypeScript"

    ```typescript
    import { Type, type TSchema } from '@sinclair/typebox';

    // Reusable enum: a factory lets each use site add its own description.
    const orderStatus = (options: { description: string }): TSchema =>
      Type.Union(
        [Type.Literal('pending'), Type.Literal('paid'), Type.Literal('shipped'), Type.Literal('done')],
        options,
      );

    export const Order = Type.Object({
      status: orderStatus({ description: 'Order status' }),
      priority: Type.Union([Type.Literal(1), Type.Literal(2), Type.Literal(3)], {
        description: 'Priority: 1 high, 2 medium, 3 low',
        default: 2,
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // Reusable enum: a function lets each use site add its own description.
    fn order_status(description: &str) -> Value {
        json!({
            "type": "string",
            "enum": ["pending", "paid", "shipped", "done"],
            "description": description
        })
    }

    pub fn order_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": order_status("Order status"),
                "priority": {
                    "type": "integer",
                    "enum": [1, 2, 3],
                    "description": "Priority: 1 high, 2 medium, 3 low",
                    "default": 2
                }
            },
            "required": ["status"]
        })
    }
    ```

### 3.5 Date and Time

Dates and times travel as ISO 8601 strings. `format` documents the expected shape but does not reject a non-conforming value (see [§4.1](#41-string-constraints)); add a `pattern` when the shape must be enforced.

=== "Python"

    ```python
    from datetime import datetime

    from pydantic import BaseModel, Field


    class DateTimeTypes(BaseModel):
        created_at: str = Field(..., description="Creation time (ISO 8601)", json_schema_extra={"format": "date-time"})
        birth_date: str = Field(..., description="Birth date", json_schema_extra={"format": "date"})
        alarm_time: str = Field(..., description="Alarm time", json_schema_extra={"format": "time"})
        date_str: str = Field(..., description="Date, YYYY-MM-DD", pattern=r"^\d{4}-\d{2}-\d{2}$")


    def parse_created_at(inputs: dict) -> datetime:
        """Parse inside execute(); the module receives the string the caller sent."""
        return datetime.fromisoformat(inputs["created_at"])
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const DateTimeTypes = Type.Object({
      created_at: Type.String({ description: 'Creation time (ISO 8601)', format: 'date-time' }),
      birth_date: Type.String({ description: 'Birth date', format: 'date' }),
      alarm_time: Type.String({ description: 'Alarm time', format: 'time' }),
      date_str: Type.String({ description: 'Date, YYYY-MM-DD', pattern: '^\\d{4}-\\d{2}-\\d{2}$' }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn datetime_types_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "created_at": { "type": "string", "description": "Creation time (ISO 8601)", "format": "date-time" },
                "birth_date": { "type": "string", "description": "Birth date", "format": "date" },
                "alarm_time": { "type": "string", "description": "Alarm time", "format": "time" },
                "date_str":   { "type": "string", "description": "Date, YYYY-MM-DD", "pattern": "^\\d{4}-\\d{2}-\\d{2}$" }
            },
            "required": ["created_at", "birth_date", "alarm_time", "date_str"]
        })
    }
    ```

---

## 4. Field Constraints

### 4.1 String Constraints

=== "Python"

    ```python
    from pydantic import BaseModel, Field


    class StringConstraints(BaseModel):
        username: str = Field(..., description="Username", min_length=3, max_length=20)
        email: str = Field(..., description="Email", pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$")
        phone: str = Field(..., description="Mobile number", pattern=r"^1[3-9]\d{9}$")
        # Annotation only: documents the shape, does not reject other strings
        website: str = Field(..., description="Website", json_schema_extra={"format": "uri"})
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const StringConstraints = Type.Object({
      username: Type.String({ description: 'Username', minLength: 3, maxLength: 20 }),
      email: Type.String({ description: 'Email', pattern: '^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$' }),
      phone: Type.String({ description: 'Mobile number', pattern: '^1[3-9]\\d{9}$' }),
      // Annotation only: documents the shape, does not reject other strings
      website: Type.String({ description: 'Website', format: 'uri' }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn string_constraints_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "username": { "type": "string", "description": "Username", "minLength": 3, "maxLength": 20 },
                "email":    { "type": "string", "description": "Email", "pattern": "^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$" },
                "phone":    { "type": "string", "description": "Mobile number", "pattern": "^1[3-9]\\d{9}$" },
                // Annotation only: documents the shape, does not reject other strings
                "website":  { "type": "string", "description": "Website", "format": "uri" }
            },
            "required": ["username", "email", "phone", "website"]
        })
    }
    ```

!!! note "`format` is an annotation"
    A value that does not match a recognised `format` (`email`, `uri`, `date-time`, `uuid`, …) is accepted, with at most a warning. When a shape must be enforced, use `pattern` or `enum` ([type-mapping §11.1](../spec/type-mapping.md#111-format-keyword)).

### 4.2 Numeric Constraints

=== "Python"

    ```python
    from pydantic import BaseModel, Field


    class NumberConstraints(BaseModel):
        age: int = Field(..., description="Age", ge=0, le=150)                      # 0 <= age <= 150
        price: float = Field(..., description="Price", gt=0, lt=1_000_000)          # 0 < price < 1000000
        quantity: int = Field(..., description="Quantity, multiple of 10", multiple_of=10)
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const NumberConstraints = Type.Object({
      age: Type.Integer({ description: 'Age', minimum: 0, maximum: 150 }),
      price: Type.Number({ description: 'Price', exclusiveMinimum: 0, exclusiveMaximum: 1_000_000 }),
      quantity: Type.Integer({ description: 'Quantity, multiple of 10', multipleOf: 10 }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn number_constraints_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "age":      { "type": "integer", "description": "Age", "minimum": 0, "maximum": 150 },
                "price":    { "type": "number", "description": "Price", "exclusiveMinimum": 0, "exclusiveMaximum": 1000000 },
                "quantity": { "type": "integer", "description": "Quantity, multiple of 10", "multipleOf": 10 }
            },
            "required": ["age", "price", "quantity"]
        })
    }
    ```

### 4.3 List Constraints

=== "Python"

    ```python
    from pydantic import BaseModel, Field, field_validator


    class ListConstraints(BaseModel):
        tags: list[str] = Field(..., description="Tags", min_length=1, max_length=10)
        unique_ids: list[str] = Field(..., description="Unique ID list")

        # Pydantic has no uniqueItems constraint for lists; check it in a validator.
        @field_validator("unique_ids")
        @classmethod
        def check_unique(cls, v: list[str]) -> list[str]:
            if len(v) != len(set(v)):
                raise ValueError("List elements must be unique")
            return v
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const ListConstraints = Type.Object({
      tags: Type.Array(Type.String(), { description: 'Tags', minItems: 1, maxItems: 10 }),
      unique_ids: Type.Array(Type.String(), { description: 'Unique ID list', uniqueItems: true }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn list_constraints_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags",
                    "minItems": 1,
                    "maxItems": 10
                },
                "unique_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Unique ID list",
                    "uniqueItems": true
                }
            },
            "required": ["tags", "unique_ids"]
        })
    }
    ```

---

## 5. LLM Extension Fields

These `x-` fields help an AI agent use a field correctly. They are defined in [protocol-spec §4.3](../spec/protocol-spec.md#43-llm-extension-fields).

| Field | Purpose |
|------|------|
| `x-llm-description` | Longer guidance written for the model, beside the short human `description` |
| `x-examples` | Example values that show the expected range and shape |
| `x-constraints` | A business rule the schema cannot express |
| `x-sensitive` | Marks a value (password, token) that must not appear in logs or traces |

### 5.1 `x-llm-description`

=== "Python"

    ```python
    from pydantic import BaseModel, Field


    class QueryInput(BaseModel):
        sql: str = Field(
            ...,
            description="SQL statement",
            json_schema_extra={
                "x-llm-description": (
                    "SQL query to execute.\n"
                    "- Only SELECT statements are allowed\n"
                    "- DROP, DELETE, UPDATE and other modifications are rejected\n"
                    "- Table names use schema.table form"
                ),
            },
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const QueryInput = Type.Object({
      sql: Type.String({
        description: 'SQL statement',
        'x-llm-description': [
          'SQL query to execute.',
          '- Only SELECT statements are allowed',
          '- DROP, DELETE, UPDATE and other modifications are rejected',
          '- Table names use schema.table form',
        ].join('\n'),
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn query_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "SQL statement",
                    "x-llm-description": concat!(
                        "SQL query to execute.\n",
                        "- Only SELECT statements are allowed\n",
                        "- DROP, DELETE, UPDATE and other modifications are rejected\n",
                        "- Table names use schema.table form"
                    )
                }
            },
            "required": ["sql"]
        })
    }
    ```

In YAML:

```yaml
sql:
  type: string
  description: "SQL statement"
  x-llm-description: |
    SQL query to execute.
    - Only SELECT statements are allowed
    - DROP, DELETE, UPDATE and other modifications are rejected
    - Table names use schema.table form
```

### 5.2 `x-examples`

=== "Python"

    ```python
    from pydantic import BaseModel, Field


    class ContactInput(BaseModel):
        email: str = Field(
            ...,
            description="Email address",
            json_schema_extra={"x-examples": ["user@example.com", "admin@company.org"]},
        )
        phone: str = Field(
            ...,
            description="Mainland China mobile number",
            pattern=r"^1[3-9]\d{9}$",
            json_schema_extra={"x-examples": ["13800138000", "15912345678"]},
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const ContactInput = Type.Object({
      email: Type.String({
        description: 'Email address',
        'x-examples': ['user@example.com', 'admin@company.org'],
      }),
      phone: Type.String({
        description: 'Mainland China mobile number',
        pattern: '^1[3-9]\\d{9}$',
        'x-examples': ['13800138000', '15912345678'],
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn contact_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "description": "Email address",
                    "x-examples": ["user@example.com", "admin@company.org"]
                },
                "phone": {
                    "type": "string",
                    "description": "Mainland China mobile number",
                    "pattern": "^1[3-9]\\d{9}$",
                    "x-examples": ["13800138000", "15912345678"]
                }
            },
            "required": ["email", "phone"]
        })
    }
    ```

### 5.3 `x-sensitive`

=== "Python"

    ```python
    from pydantic import BaseModel, Field


    class LoginInput(BaseModel):
        username: str = Field(..., description="Username")
        password: str = Field(..., description="Password", json_schema_extra={"x-sensitive": True})
        api_key: str = Field(..., description="API key", json_schema_extra={"x-sensitive": True})
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const LoginInput = Type.Object({
      username: Type.String({ description: 'Username' }),
      password: Type.String({ description: 'Password', 'x-sensitive': true }),
      api_key: Type.String({ description: 'API key', 'x-sensitive': true }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn login_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "username": { "type": "string", "description": "Username" },
                "password": { "type": "string", "description": "Password", "x-sensitive": true },
                "api_key":  { "type": "string", "description": "API key", "x-sensitive": true }
            },
            "required": ["username", "password", "api_key"]
        })
    }
    ```

**What `x-sensitive` does:** after input validation the executor keeps a redacted copy of the inputs in `context.redacted_inputs`, with every `x-sensitive` value (including values nested in objects and arrays) replaced by `***REDACTED***`. Logging, tracing, and audit output read that copy. The module itself still receives the real value in `execute()`. Redaction rules, including the configurable `obs.redaction.*` keys, are specified in [protocol-spec §10.6](../spec/protocol-spec.md#106-sensitive-data-redaction); the [Observability cookbook](./cookbook-observability.md) shows them end to end.

---

## 6. Nesting and References

### 6.1 Nested Objects

=== "Python"

    ```python
    from pydantic import BaseModel, Field


    class Address(BaseModel):
        city: str = Field(..., description="City")
        street: str = Field(..., description="Street")
        postal_code: str = Field(..., description="Postal code")


    class Company(BaseModel):
        name: str = Field(..., description="Company name")
        address: Address = Field(..., description="Company address")


    class Employee(BaseModel):
        name: str = Field(..., description="Name")
        company: Company = Field(..., description="Employer")          # two levels deep
        home_address: Address = Field(..., description="Home address")  # reused model
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const Address = Type.Object({
      city: Type.String({ description: 'City' }),
      street: Type.String({ description: 'Street' }),
      postal_code: Type.String({ description: 'Postal code' }),
    });

    const Company = Type.Object({
      name: Type.String({ description: 'Company name' }),
      address: Type.Composite([Address], { description: 'Company address' }),
    });

    export const Employee = Type.Object({
      name: Type.String({ description: 'Name' }),
      company: Type.Composite([Company], { description: 'Employer' }),            // two levels deep
      home_address: Type.Composite([Address], { description: 'Home address' }), // reused schema
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn address_schema(description: &str) -> Value {
        json!({
            "type": "object",
            "description": description,
            "properties": {
                "city":        { "type": "string", "description": "City" },
                "street":      { "type": "string", "description": "Street" },
                "postal_code": { "type": "string", "description": "Postal code" }
            },
            "required": ["city", "street", "postal_code"]
        })
    }

    fn company_schema(description: &str) -> Value {
        json!({
            "type": "object",
            "description": description,
            "properties": {
                "name":    { "type": "string", "description": "Company name" },
                "address": address_schema("Company address")
            },
            "required": ["name", "address"]
        })
    }

    pub fn employee_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name":         { "type": "string", "description": "Name" },
                "company":      company_schema("Employer"),         // two levels deep
                "home_address": address_schema("Home address")      // reused schema
            },
            "required": ["name", "company", "home_address"]
        })
    }
    ```

### 6.2 References in YAML

```yaml
input_schema:
  type: object
  properties:
    shipping_address:
      $ref: "#/$defs/Address"
      description: "Where to ship"
    billing_address:
      $ref: "#/$defs/Address"
      description: "Where to send the invoice"

$defs:
  Address:
    type: object
    properties:
      city:
        type: string
        description: "City"
      street:
        type: string
        description: "Street"
    required: [city, street]
```

### 6.3 Cross-File References

A relative path in `$ref` is resolved against the file that contains it:

```yaml
# schemas/executor/order/create_order.schema.yaml
input_schema:
  type: object
  properties:
    customer:
      $ref: "../../common/customer.schema.yaml#/$defs/Customer"
    items:
      type: array
      items:
        $ref: "../../common/product.schema.yaml#/$defs/OrderItem"
```

Reference chains are limited by `schema.max_ref_depth` (see [§8](#8-schema-loading-strategy)). The full resolution algorithm is in [protocol-spec §4.11](../spec/protocol-spec.md#411-schema-references-ref).

---

## 7. Custom Validation

JSON Schema covers structure and simple constraints. Cross-field checks and business rules go in code:

- **Python**: Pydantic `field_validator` / `model_validator` run during input validation. A validator can **reject** a value (the call fails with `SCHEMA_VALIDATION_ERROR`), but `execute()` receives the inputs as sent, so do any normalization (lower-casing, trimming) inside `execute()`.
- **TypeScript and Rust**: put the check in a plain function and call it at the start of `execute()`, returning a structured error when it fails.

### 7.1 Field Checks

=== "Python"

    ```python
    from pydantic import BaseModel, Field, field_validator


    class UserInput(BaseModel):
        username: str = Field(..., description="Username, letters and digits only")
        email: str = Field(..., description="Email")

        @field_validator("username")
        @classmethod
        def username_alphanumeric(cls, v: str) -> str:
            if not v.isalnum():
                raise ValueError("Username can only contain letters and numbers")
            return v

        @field_validator("email")
        @classmethod
        def email_has_at(cls, v: str) -> str:
            if "@" not in v:
                raise ValueError("Email format is incorrect")
            return v


    def normalize(inputs: dict) -> dict:
        """Call from execute(): validators reject, they do not rewrite inputs."""
        return {**inputs, "email": inputs["email"].lower()}
    ```

=== "TypeScript"

    ```typescript
    import { Type, type Static } from '@sinclair/typebox';

    export const UserInput = Type.Object({
      // pattern is enforced by the schema itself
      username: Type.String({ description: 'Username, letters and digits only', pattern: '^[A-Za-z0-9]+$' }),
      email: Type.String({ description: 'Email' }),
    });

    // Call from execute(): checks and normalization the schema cannot express.
    export function normalizeUserInput(inputs: Static<typeof UserInput>): Static<typeof UserInput> {
      if (!inputs.email.includes('@')) {
        throw new Error('Email format is incorrect');
      }
      return { ...inputs, email: inputs.email.toLowerCase() };
    }
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn user_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // pattern is enforced by the schema itself
                "username": { "type": "string", "description": "Username, letters and digits only", "pattern": "^[A-Za-z0-9]+$" },
                "email":    { "type": "string", "description": "Email" }
            },
            "required": ["username", "email"]
        })
    }

    // Call from execute(): checks and normalization the schema cannot express.
    pub fn normalize_user_input(mut inputs: Value) -> Result<Value, String> {
        let email = inputs["email"].as_str().unwrap_or_default().to_string();
        if !email.contains('@') {
            return Err("Email format is incorrect".into());
        }
        inputs["email"] = json!(email.to_lowercase());
        Ok(inputs)
    }
    ```

### 7.2 Cross-Field Checks

=== "Python"

    ```python
    from pydantic import BaseModel, Field, model_validator


    class OrderItem(BaseModel):
        product_id: str = Field(..., description="Product ID")
        quantity: int = Field(..., description="Quantity")
        price: float = Field(..., description="Unit price")


    class OrderInput(BaseModel):
        items: list[OrderItem] = Field(..., description="Order items")
        coupon_code: str | None = Field(None, description="Coupon code")
        total_amount: float = Field(..., description="Total amount")

        @model_validator(mode="after")
        def check_total(self) -> "OrderInput":
            calculated = sum(item.price * item.quantity for item in self.items)
            if abs(self.total_amount - calculated) > 0.01:
                raise ValueError(f"Total amount is incorrect, should be {calculated}")
            return self
    ```

=== "TypeScript"

    ```typescript
    import { Type, type Static } from '@sinclair/typebox';

    const OrderItem = Type.Object({
      product_id: Type.String({ description: 'Product ID' }),
      quantity: Type.Integer({ description: 'Quantity' }),
      price: Type.Number({ description: 'Unit price' }),
    });

    export const OrderInput = Type.Object({
      items: Type.Array(OrderItem, { description: 'Order items' }),
      coupon_code: Type.Optional(
        Type.Union([Type.String(), Type.Null()], { description: 'Coupon code', default: null }),
      ),
      total_amount: Type.Number({ description: 'Total amount' }),
    });

    // Call from execute().
    export function checkTotal(input: Static<typeof OrderInput>): void {
      const calculated = input.items.reduce((sum, it) => sum + it.price * it.quantity, 0);
      if (Math.abs(input.total_amount - calculated) > 0.01) {
        throw new Error(`Total amount is incorrect, should be ${calculated}`);
      }
    }
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn order_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "description": "Order items",
                    "items": {
                        "type": "object",
                        "properties": {
                            "product_id": { "type": "string",  "description": "Product ID" },
                            "quantity":   { "type": "integer", "description": "Quantity" },
                            "price":      { "type": "number",  "description": "Unit price" }
                        },
                        "required": ["product_id", "quantity", "price"]
                    }
                },
                "coupon_code":  { "type": ["string", "null"], "description": "Coupon code", "default": null },
                "total_amount": { "type": "number", "description": "Total amount" }
            },
            "required": ["items", "total_amount"]
        })
    }

    // Call from execute().
    pub fn check_total(input: &Value) -> Result<(), String> {
        let items = input["items"].as_array().ok_or("items must be an array")?;
        let calculated: f64 = items
            .iter()
            .map(|it| it["price"].as_f64().unwrap_or(0.0) * it["quantity"].as_f64().unwrap_or(0.0))
            .sum();
        let total = input["total_amount"].as_f64().unwrap_or(0.0);
        if (total - calculated).abs() > 0.01 {
            return Err(format!("Total amount is incorrect, should be {calculated}"));
        }
        Ok(())
    }
    ```

To report a failed check from `execute()` in a way an AI agent can act on, raise a `ModuleError` with `ai_guidance` — see [Creating Modules § Error Handling](./creating-modules.md#error-handling).

---

## 8. Schema Loading Strategy

YAML schema files are read by `SchemaLoader`, which takes its settings from the `schema` section of `apcore.yaml`:

```yaml
# apcore.yaml
schema:
  root: "./schemas"        # directory schema files are resolved against
  strategy: "yaml_first"   # yaml_first | native_first | yaml_only
  max_ref_depth: 32        # maximum $ref chain length, 1-100
```

| Strategy | Behaviour |
|------|------|
| `yaml_first` (default) | Use the YAML file when one exists; fall back to the schema written in code |
| `native_first` | Use the schema written in code when there is one; fall back to the YAML file |
| `yaml_only` | Use only YAML files (for purely cross-language contracts) |

`root`, `strategy`, and `max_ref_depth` are the whole `schema` section; any other key is a configuration error. Whether undeclared properties are rejected, or whether `"42"` counts as an integer, is decided by the schema itself (`additionalProperties`, `type`), never by configuration ([protocol-spec §4.9](../spec/protocol-spec.md#49-schema-loading-strategy), [type-mapping §17.3](../spec/type-mapping.md#173-no-type-coercion-at-the-module-boundary)).

---

## 9. Best Practices

### 9.1 Every Field Has a Description

=== "Python"

    ```python
    from pydantic import BaseModel, Field


    class Good(BaseModel):
        name: str = Field(..., description="User name, 2-50 characters")
        age: int = Field(..., description="User age in years")


    class Bad(BaseModel):  # the model has to guess what these mean
        name: str
        age: int
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const Good = Type.Object({
      name: Type.String({ description: 'User name, 2-50 characters' }),
      age: Type.Integer({ description: 'User age in years' }),
    });

    // The model has to guess what these mean.
    export const Bad = Type.Object({ name: Type.String(), age: Type.Integer() });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn good() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string",  "description": "User name, 2-50 characters" },
                "age":  { "type": "integer", "description": "User age in years" }
            },
            "required": ["name", "age"]
        })
    }

    // The model has to guess what these mean.
    pub fn bad() -> Value {
        json!({
            "type": "object",
            "properties": { "name": { "type": "string" }, "age": { "type": "integer" } },
            "required": ["name", "age"]
        })
    }
    ```

### 9.2 Use Precise Types and Constraints

=== "Python"

    ```python
    from typing import Any, Literal

    from pydantic import BaseModel, Field


    class Good(BaseModel):
        status: Literal["active", "inactive", "pending"] = Field(..., description="Account status")
        username: str = Field(..., description="Username", min_length=3, max_length=20)
        age: int = Field(..., description="Age", ge=0, le=150)


    class Bad(BaseModel):
        status: str      # any string at all
        username: str    # empty or megabytes long
        age: Any         # not even a number
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    export const Good = Type.Object({
      status: Type.Union([Type.Literal('active'), Type.Literal('inactive'), Type.Literal('pending')], {
        description: 'Account status',
      }),
      username: Type.String({ description: 'Username', minLength: 3, maxLength: 20 }),
      age: Type.Integer({ description: 'Age', minimum: 0, maximum: 150 }),
    });

    export const Bad = Type.Object({
      status: Type.String(),   // any string at all
      username: Type.String(), // empty or megabytes long
      age: Type.Unknown(),     // not even a number
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    pub fn good() -> Value {
        json!({
            "type": "object",
            "properties": {
                "status":   { "type": "string", "enum": ["active", "inactive", "pending"], "description": "Account status" },
                "username": { "type": "string", "minLength": 3, "maxLength": 20, "description": "Username" },
                "age":      { "type": "integer", "minimum": 0, "maximum": 150, "description": "Age" }
            },
            "required": ["status", "username", "age"]
        })
    }

    pub fn bad() -> Value {
        json!({
            "type": "object",
            "properties": {
                "status":   { "type": "string" }, // any string at all
                "username": { "type": "string" }, // empty or megabytes long
                "age":      {}                    // not even a number
            },
            "required": ["status", "username", "age"]
        })
    }
    ```

### 9.3 Reuse Sub-Schemas

Define a shared shape once (a Pydantic model, a TypeBox constant, a Rust function, or a YAML `$defs` entry) and reference it, instead of flattening copies into prefixed fields such as `shipping_city` / `billing_city`. See [§6](#6-nesting-and-references).

---

## 10. Edge Cases

The normative rules live in [protocol-spec §4.15](../spec/protocol-spec.md#415-edge-case-handling) and the [Type Mapping Specification](../spec/type-mapping.md). In short:

- **`null`, empty, and missing are different.** `{"name": null}`, `{"name": ""}`, and `{}` are three inputs. `required` controls whether a field must be present; including `"null"` in `type` controls whether `null` is allowed. There is no `nullable` keyword.
- **Large integers.** Integers above `2^53 - 1` lose precision in JavaScript. Send them as strings with a digits-only `pattern` ([type-mapping §14.1](../spec/type-mapping.md#141-large-integer-precision-loss)). Exact decimals such as money are also best sent as strings or as integer minor units.
- **`$ref` depth.** A reference chain longer than `schema.max_ref_depth` (default 32, configurable from 1 to 100) fails with `SCHEMA_MAX_DEPTH_EXCEEDED`; a `$ref` → `$ref` loop fails with `SCHEMA_CIRCULAR_REF`. A recursive schema whose `$ref` sits under `properties` or `items` is legal.
- **`format` never rejects.** A value that does not match its `format` is accepted, with at most a warning.
- **Unknown `x-` keywords are ignored**, so extensions are forward compatible.

```yaml
properties:
  user_id:
    type: integer               # safely below 2^53
    minimum: 0
    description: "User ID"
  order_no:
    type: string                # 64-bit snowflake ID, sent as digits
    pattern: "^\\d+$"
    description: "Snowflake order number"
  amount:
    type: string                # exact decimal
    pattern: "^-?\\d+\\.\\d{2}$"
    description: "Amount, exact to the cent"
```

---

## 11. AI-Friendly Schema Design

Tool-calling LLM APIs consume JSON Schema, so the same schema that validates a call also teaches the model how to make it. These guidelines make that easier.

### 11.1 Flat Is Better Than Nested

Models fill deeply nested structures less accurately. Keep an AI-facing `input_schema` to two or three levels.

```yaml
# Bad: four levels for two values
input_schema:
  type: object
  properties:
    config:
      type: object
      properties:
        database:
          type: object
          properties:
            connection:
              type: object
              properties:
                host: { type: string }
                port: { type: integer }

# Good: flat and self-describing
input_schema:
  type: object
  properties:
    db_host:
      type: string
      description: "Database server hostname or IP address"
      x-examples: ["localhost", "db.example.com"]
    db_port:
      type: integer
      description: "Database port number"
      default: 5432
```

### 11.2 Descriptions Explain Meaning, Not Type

A `description` should answer "what does the model need to know to fill this field correctly?"

| Do | Example |
|------|------|
| Explain semantics, not the type | Not "a string"; the schema already says so |
| State the accepted values | "Supported formats: png, jpg, gif" |
| State the default behaviour | "If omitted, sends to all subscribers" |
| State related constraints | "When format is html, template_id is required" |

```yaml
# Bad: repeats the type, gives no guidance
body:
  type: string
  description: "Email body, string type"

# Good: explains meaning and usage
body:
  type: string
  description: "Email body, plain text or HTML. Set html=true when sending HTML."
```

### 11.3 Token Awareness

Tool-calling APIs put the schema into the prompt, so every word costs context.

| Recommendation | Why |
|------|------|
| Don't repeat type information in `description` | It is already in `type` |
| Add `x-llm-description` when an enum has more than about five values | The model should not guess what each value means |
| Export compact schemas during discovery | See the export profiles in [Schema System](../features/schema-system.md) |
| Send full schemas only after a module is selected | Progressive disclosure keeps early prompts small |

```yaml
status:
  type: string
  enum: ["draft", "pending_review", "in_review", "approved", "rejected",
         "published", "archived", "suspended", "deleted"]
  description: "Article status"
  x-llm-description: |
    Use draft when creating, pending_review to submit for review, and
    published to go live. archived, suspended, and deleted are for
    administrators and are not used when creating an article.
```

### 11.4 Required Fields First

List required properties before optional ones so the essential parameters are read first:

```yaml
input_schema:
  type: object
  properties:
    # Required
    to:
      type: string
      description: "Recipient email address"
    subject:
      type: string
      description: "Email subject"
    body:
      type: string
      description: "Email body"
    # Optional
    cc:
      type: array
      items: { type: string }
      description: "CC list"
      default: []
    html:
      type: boolean
      description: "Send the body as HTML"
      default: false
  required: [to, subject, body]
```

### 11.5 Checklist

Before exposing a module to AI callers:

- [ ] `input_schema` is at most three levels deep
- [ ] Every field has a `description` that does not just restate its type
- [ ] Required fields are listed before optional ones
- [ ] Enums with more than about five values have an `x-llm-description`
- [ ] Complex inputs (`oneOf`/`anyOf`, five or more required fields) have `x-examples` or module examples
- [ ] Secrets are marked `x-sensitive`
- [ ] Numeric fields declare `minimum`/`maximum` and a `default` where one makes sense
- [ ] Fields with a fixed set of values use `enum` (`Literal[...]` in Python)

---

## 12. Compatibility with Other Tool-Calling APIs

apcore schemas are JSON Schema Draft 2020-12. When a module is exported to an external LLM or agent API, the receiving side may support only part of JSON Schema. Staying within a widely supported subset avoids surprises.

### 12.1 Widely Supported Keywords

| Keyword | Purpose |
|--------|------|
| `type` | Type declaration |
| `properties` / `required` | Object shape |
| `enum` / `const` | Fixed values |
| `description` / `default` | Documentation and defaults |
| `minimum` / `maximum` | Numeric range |
| `minLength` / `maxLength` / `pattern` | String constraints |
| `items` | Array element schema |
| `$ref` (local) | Reference within the same document |
| `oneOf` / `anyOf` / `allOf` | Combinations |
| `additionalProperties` | Extra keys |
| `format` | Annotation such as `date-time` or `email` |

### 12.2 Use With Care

| Keyword | Introduced in | Risk |
|--------|------|------|
| `if` / `then` / `else` | Draft 7 | Often ignored by tool-calling APIs |
| `$anchor` | 2019-09 | Not understood by Draft 7 consumers, which use `$id` |
| `dependentRequired` / `dependentSchemas` | 2019-09 | Draft 7 consumers expect `dependencies` |
| `prefixItems` | 2020-12 | Draft 7 consumers expect the array form of `items` |
| `$dynamicRef` / `$dynamicAnchor` | 2020-12 | Rarely supported outside full validators |

### 12.3 Recommendations

- For modules that will be exported, stay within §12.1.
- Modules used only inside apcore can use all of Draft 2020-12.
- With `oneOf`/`anyOf`, add examples so the model can tell the branches apart.
- Inline cross-file `$ref`s before export; most consumers cannot fetch external references. The [strict-mode export](../spec/protocol-spec.md#416-strict-mode-export) produces a schema that OpenAI and Anthropic strict mode accept.

---

## Next Steps

- [Creating Modules Guide](./creating-modules.md) — Module creation tutorial
- [Multi-Language Guide](./multi-language.md) — Sharing schemas across SDKs
- [Schema System](../features/schema-system.md) — Loader, validator, and exporter reference
- [Type Mapping Specification](../spec/type-mapping.md) — Canonical type mapping
