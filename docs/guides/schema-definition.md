---
description: "Defining apcore module input/output schemas — Pydantic models, YAML JSON Schema Draft 2020-12, x-* LLM extension fields, runtime validation, and cross-language schema sharing."
---

# Schema Definition Explained

> Schema is the core of apcore, defining the input and output structure of modules.

## 1. Why is Schema Mandatory?

```
Traditional Module:
    def process(data):  # What is data? Unknown
        return result   # What is result? Unknown

apcore Module:
    input_schema = ProcessInput    # Input structure is clear
    output_schema = ProcessOutput  # Output structure is clear

    def execute(inputs, context):
        # inputs are validated, structure is known
        return {...}  # Output will be validated
```

**Purpose of Schema:**

| Purpose | Description |
|------|------|
| **AI Understanding** | LLM knows how to call the module through Schema |
| **Auto Validation** | Framework automatically validates input and output |
| **Documentation Generation** | Automatically generate API documentation |
| **Type Safety** | Type hints available during development |
| **Cross-Language** | YAML Schema can be shared across languages |

---

## 2. Schema Definition Methods

### 2.1 Native Schema Construction (Recommended)

**The preferred method when defining schemas directly in code.** Each SDK uses the idiomatic schema construction approach for its language: Pydantic for Python, TypeBox for TypeScript, and `serde_json::json!` for Rust.

=== "Python"

    ```python
    from pydantic import BaseModel, Field
    from typing import Literal


    class Address(BaseModel):
        """Address information"""
        province: str = Field(..., description="Province")
        city: str = Field(..., description="City")
        district: str = Field(..., description="District")
        detail: str = Field(..., description="Detailed address")
        postal_code: str = Field(..., description="Postal code", pattern=r"^\d{6}$")


    class OrderInput(BaseModel):
        """Order creation input"""

        # Required fields
        product_id: str = Field(
            ...,                          # ... means required
            description="Product ID",
            min_length=1,
            max_length=50,
        )

        quantity: int = Field(
            ...,
            description="Purchase quantity",
            ge=1,                         # >= 1
            le=100,                       # <= 100
        )

        # Optional fields (must have default value)
        note: str | None = Field(
            None,                         # Default value
            description="Order note",
            max_length=500,
        )

        # Enum type
        payment_method: Literal["alipay", "wechat", "card"] = Field(
            "alipay",
            description="Payment method",
        )

        # Nested object
        shipping_address: Address = Field(
            ...,
            description="Shipping address",
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // Address sub-schema
    const Address = Type.Object({
      province: Type.String({ description: 'Province' }),
      city: Type.String({ description: 'City' }),
      district: Type.String({ description: 'District' }),
      detail: Type.String({ description: 'Detailed address' }),
      postal_code: Type.String({
        description: 'Postal code',
        pattern: '^\\d{6}$',
      }),
    });

    // Order creation input
    const OrderInput = Type.Object({
      // Required fields
      product_id: Type.String({
        description: 'Product ID',
        minLength: 1,
        maxLength: 50,
      }),
      quantity: Type.Integer({
        description: 'Purchase quantity',
        minimum: 1,
        maximum: 100,
      }),

      // Optional fields (must have default value)
      note: Type.Optional(
        Type.Union([Type.String({ maxLength: 500 }), Type.Null()], {
          description: 'Order note',
          default: null,
        }),
      ),

      // Enum type
      payment_method: Type.Union(
        [Type.Literal('alipay'), Type.Literal('wechat'), Type.Literal('card')],
        { description: 'Payment method', default: 'alipay' },
      ),

      // Nested object
      shipping_address: Type.Composite([Address], {
        description: 'Shipping address',
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // Address sub-schema
    fn address_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "province":    {"type": "string", "description": "Province"},
                "city":        {"type": "string", "description": "City"},
                "district":    {"type": "string", "description": "District"},
                "detail":      {"type": "string", "description": "Detailed address"},
                "postal_code": {
                    "type": "string",
                    "description": "Postal code",
                    "pattern": "^\\d{6}$"
                }
            },
            "required": ["province", "city", "district", "detail", "postal_code"]
        })
    }

    // Order creation input
    fn order_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // Required fields
                "product_id": {
                    "type": "string",
                    "description": "Product ID",
                    "minLength": 1,
                    "maxLength": 50
                },
                "quantity": {
                    "type": "integer",
                    "description": "Purchase quantity",
                    "minimum": 1,
                    "maximum": 100
                },
                // Optional fields (with default)
                "note": {
                    "type": ["string", "null"],
                    "description": "Order note",
                    "maxLength": 500,
                    "default": null
                },
                // Enum type
                "payment_method": {
                    "type": "string",
                    "description": "Payment method",
                    "enum": ["alipay", "wechat", "card"],
                    "default": "alipay"
                },
                // Nested object
                "shipping_address": address_schema()
            },
            "required": ["product_id", "quantity", "shipping_address"]
        })
    }
    ```

### 2.2 YAML Schema (Cross-Language)

**Used for sharing Schema definitions across languages.**

```yaml
# schemas/executor/order/create_order.schema.yaml

$schema: "https://apcore.dev/schema/v1"
version: "1.0.0"
module_id: "executor.order.create_order"

description: |
  Order creation module
  Supports multiple payment methods and shipping address configuration.

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
      type: string
      description: "Order note"
      maxLength: 500
      default: null

    payment_method:
      type: string
      description: "Payment method"
      enum: ["alipay", "wechat", "card"]
      default: "alipay"

    shipping_address:
      $ref: "#/definitions/Address"
      description: "Shipping address"

  required: [product_id, quantity, shipping_address]

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

definitions:
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
        description: "Detailed address"
      postal_code:
        type: string
        description: "Postal code"
        pattern: "^\\d{6}$"
    required: [province, city, district, detail, postal_code]
```

---

## 3. Field Types

### 3.1 Basic Types

=== "Python"

    ```python
    from pydantic import BaseModel, Field
    from typing import Any

    class BasicTypes(BaseModel):
        # String
        name: str = Field(..., description="Name")

        # Integer
        age: int = Field(..., description="Age")

        # Float
        price: float = Field(..., description="Price")

        # Boolean
        active: bool = Field(..., description="Is active")

        # Any type (avoid when possible)
        data: Any = Field(..., description="Any data")
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const BasicTypes = Type.Object({
      // String
      name: Type.String({ description: 'Name' }),

      // Integer
      age: Type.Integer({ description: 'Age' }),

      // Float
      price: Type.Number({ description: 'Price' }),

      // Boolean
      active: Type.Boolean({ description: 'Is active' }),

      // Any type (avoid when possible)
      data: Type.Unknown({ description: 'Any data' }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn basic_types_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // String
                "name":   {"type": "string",  "description": "Name"},
                // Integer
                "age":    {"type": "integer", "description": "Age"},
                // Float
                "price":  {"type": "number",  "description": "Price"},
                // Boolean
                "active": {"type": "boolean", "description": "Is active"},
                // Any type (avoid when possible)
                "data":   {"description": "Any data"}
            },
            "required": ["name", "age", "price", "active", "data"]
        })
    }
    ```

**Corresponding JSON Schema:**

```yaml
properties:
  name:
    type: string
  age:
    type: integer
  price:
    type: number
  active:
    type: boolean
  data: {}  # Any type
```

### 3.2 Optional Types

=== "Python"

    ```python
    from pydantic import BaseModel, Field
    from typing import Optional

    class OptionalTypes(BaseModel):
        # Method 1: | None
        email: str | None = Field(None, description="Email")

        # Method 2: Optional (equivalent to above)
        phone: Optional[str] = Field(None, description="Phone")

        # Non-None field with default value
        count: int = Field(default=0, description="Count")
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const OptionalTypes = Type.Object({
      // Nullable optional
      email: Type.Optional(
        Type.Union([Type.String(), Type.Null()], { description: 'Email', default: null }),
      ),

      // Same shape, alternate spelling
      phone: Type.Optional(
        Type.Union([Type.String(), Type.Null()], { description: 'Phone', default: null }),
      ),

      // Non-null field with default value
      count: Type.Integer({ description: 'Count', default: 0 }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn optional_types_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // Nullable optional
                "email": {"type": ["string", "null"], "description": "Email", "default": null},
                // Same shape, alternate spelling
                "phone": {"type": ["string", "null"], "description": "Phone", "default": null},
                // Non-null field with default value
                "count": {"type": "integer", "description": "Count", "default": 0}
            }
            // No "required" array — all fields are optional
        })
    }
    ```

### 3.3 Lists and Dictionaries

=== "Python"

    ```python
    from pydantic import BaseModel, Field
    from typing import Any


    class OrderItem(BaseModel):
        product_id: str = Field(..., description="Product ID")
        quantity: int = Field(..., description="Quantity")
        price: float = Field(..., description="Unit price")


    class CollectionTypes(BaseModel):
        # String list
        tags: list[str] = Field(default=[], description="Tag list")

        # Object list
        items: list[OrderItem] = Field(..., description="Order items")

        # Dictionary
        metadata: dict[str, Any] = Field(default={}, description="Metadata")

        # Dictionary with specified value type
        scores: dict[str, int] = Field(default={}, description="Score mapping")
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const OrderItem = Type.Object({
      product_id: Type.String({ description: 'Product ID' }),
      quantity: Type.Integer({ description: 'Quantity' }),
      price: Type.Number({ description: 'Unit price' }),
    });

    const CollectionTypes = Type.Object({
      // String list
      tags: Type.Array(Type.String(), { description: 'Tag list', default: [] }),

      // Object list
      items: Type.Array(OrderItem, { description: 'Order items' }),

      // Dictionary
      metadata: Type.Record(Type.String(), Type.Unknown(), {
        description: 'Metadata',
        default: {},
      }),

      // Dictionary with specified value type
      scores: Type.Record(Type.String(), Type.Integer(), {
        description: 'Score mapping',
        default: {},
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn order_item_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "product_id": {"type": "string",  "description": "Product ID"},
                "quantity":   {"type": "integer", "description": "Quantity"},
                "price":      {"type": "number",  "description": "Unit price"}
            },
            "required": ["product_id", "quantity", "price"]
        })
    }

    fn collection_types_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // String list
                "tags": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Tag list",
                    "default": []
                },
                // Object list
                "items": {
                    "type": "array",
                    "items": order_item_schema(),
                    "description": "Order items"
                },
                // Dictionary
                "metadata": {
                    "type": "object",
                    "additionalProperties": true,
                    "description": "Metadata",
                    "default": {}
                },
                // Dictionary with specified value type
                "scores": {
                    "type": "object",
                    "additionalProperties": {"type": "integer"},
                    "description": "Score mapping",
                    "default": {}
                }
            },
            "required": ["items"]
        })
    }
    ```

**Corresponding JSON Schema:**

```yaml
properties:
  tags:
    type: array
    items:
      type: string
    default: []

  items:
    type: array
    items:
      $ref: "#/definitions/OrderItem"

  metadata:
    type: object
    additionalProperties: true
    default: {}

  scores:
    type: object
    additionalProperties:
      type: integer
    default: {}
```

### 3.4 Enum Types

=== "Python"

    ```python
    from pydantic import BaseModel, Field
    from typing import Literal
    from enum import Enum

    # Method 1: Literal (recommended)
    class Order1(BaseModel):
        status: Literal["pending", "paid", "shipped", "done"] = Field(
            ...,
            description="Order status",
        )

        priority: Literal[1, 2, 3] = Field(
            default=2,
            description="Priority: 1-high 2-medium 3-low",
        )

    # Method 2: Enum
    class OrderStatus(str, Enum):
        PENDING = "pending"
        PAID = "paid"
        SHIPPED = "shipped"
        DONE = "done"

    class Order2(BaseModel):
        status: OrderStatus = Field(..., description="Order status")
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // Method 1: Union of literal values (recommended)
    const Order1 = Type.Object({
      status: Type.Union(
        [
          Type.Literal('pending'),
          Type.Literal('paid'),
          Type.Literal('shipped'),
          Type.Literal('done'),
        ],
        { description: 'Order status' },
      ),

      priority: Type.Union(
        [Type.Literal(1), Type.Literal(2), Type.Literal(3)],
        { description: 'Priority: 1-high 2-medium 3-low', default: 2 },
      ),
    });

    // Method 2: Reused enum constant
    const OrderStatus = Type.Union(
      [
        Type.Literal('pending'),
        Type.Literal('paid'),
        Type.Literal('shipped'),
        Type.Literal('done'),
      ],
      { $id: 'OrderStatus' },
    );

    const Order2 = Type.Object({
      status: Type.Composite([OrderStatus], { description: 'Order status' }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // Method 1: enum keyword on a string field (recommended)
    fn order1_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "description": "Order status",
                    "enum": ["pending", "paid", "shipped", "done"]
                },
                "priority": {
                    "type": "integer",
                    "description": "Priority: 1-high 2-medium 3-low",
                    "enum": [1, 2, 3],
                    "default": 2
                }
            },
            "required": ["status"]
        })
    }

    // Method 2: reused enum sub-schema
    fn order_status_schema() -> Value {
        json!({
            "type": "string",
            "enum": ["pending", "paid", "shipped", "done"]
        })
    }

    fn order2_schema() -> Value {
        let mut status = order_status_schema();
        status["description"] = json!("Order status");
        json!({
            "type": "object",
            "properties": { "status": status },
            "required": ["status"]
        })
    }
    ```

### 3.5 Date and Time

=== "Python"

    ```python
    from datetime import datetime, date, time
    from pydantic import BaseModel, Field

    class DateTimeTypes(BaseModel):
        # Datetime
        created_at: datetime = Field(..., description="Creation time")

        # Date only
        birth_date: date = Field(..., description="Birth date")

        # Time only
        alarm_time: time = Field(..., description="Alarm time")

        # Date string format (needs to be parsed manually)
        date_str: str = Field(
            ...,
            description="Date string",
            pattern=r"^\d{4}-\d{2}-\d{2}$",  # YYYY-MM-DD
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const DateTimeTypes = Type.Object({
      // Datetime (ISO 8601)
      created_at: Type.String({
        description: 'Creation time',
        format: 'date-time',
      }),

      // Date only
      birth_date: Type.String({
        description: 'Birth date',
        format: 'date',
      }),

      // Time only
      alarm_time: Type.String({
        description: 'Alarm time',
        format: 'time',
      }),

      // Date string format (custom pattern)
      date_str: Type.String({
        description: 'Date string',
        pattern: '^\\d{4}-\\d{2}-\\d{2}$', // YYYY-MM-DD
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn datetime_types_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // Datetime (ISO 8601)
                "created_at": {
                    "type": "string",
                    "description": "Creation time",
                    "format": "date-time"
                },
                // Date only
                "birth_date": {
                    "type": "string",
                    "description": "Birth date",
                    "format": "date"
                },
                // Time only
                "alarm_time": {
                    "type": "string",
                    "description": "Alarm time",
                    "format": "time"
                },
                // Date string format (custom pattern)
                "date_str": {
                    "type": "string",
                    "description": "Date string",
                    "pattern": "^\\d{4}-\\d{2}-\\d{2}$"
                }
            },
            "required": ["created_at", "birth_date", "alarm_time", "date_str"]
        })
    }
    ```

### 3.6 Cross-Language Type Mapping Quick Reference

!!! note "The `format:` rows are aspirational"
    No SDK maps a `format` to a native type today — apcore-python annotates a `format: date-time` field as `str`, not `datetime`. `format` is an **annotation, not an assertion** ([TYPE_MAPPING §11](../spec/type-mapping.md#111-format-keyword)): a non-conforming value emits a warning and **still validates**. To make a format binding, use `pattern` or `enum` instead.

| JSON Schema Type | Python | Rust | Go | Java | TypeScript |
|-----------------|--------|------|----|------|------------|
| `string` | `str` | `String` | `string` | `String` | `string` |
| `integer` | `int` | `i64` | `int64` | `long` / `Long` | `number` |
| `number` | `float` | `f64` | `float64` | `double` / `Double` | `number` |
| `boolean` | `bool` | `bool` | `bool` | `boolean` / `Boolean` | `boolean` |
| `null` | `None` | `()` | `nil` | `null` | `null` |
| `object` (with `properties`) | `BaseModel` subclass | `struct` | `struct` | `class` | `z.object({})` |
| `additionalProperties` | `dict[str, V]` | `HashMap<String, V>` | `map[string]V` | `Map<String, V>` | `Record<string, V>` |
| `array` | `list[T]` | `Vec<T>` | `[]T` | `List<T>` | `T[]` |
| `string` + `format: date-time` | `datetime` | `DateTime<Utc>` | `time.Time` | `OffsetDateTime` | `Date` / `string` |
| `string` + `format: date` | `date` | `NaiveDate` | `civil.Date` | `LocalDate` | `string` |
| `string` + `format: time` | `time` | `NaiveTime` | Custom | `LocalTime` | `string` |
| `string` + `format: email` | `str` + warning | `String` + warning | `string` + warning | `String` + warning | `string` + warning |
| `string` + `format: uri` | `str` + warning | `String` + warning | `string` + warning | `String` + warning | `string` + warning |
| `string` + `format: uuid` | `UUID` | `Uuid` | `uuid.UUID` | `UUID` | `string` + warning |
| `string enum` | `Literal[...]` / `StrEnum` | `enum` | `type T string` + const | `enum` | union type |
| `integer enum` | `IntEnum` | `enum` | `type T int64` + iota | `enum` | union type |
| `oneOf` | Discriminated Union | `enum` (tagged) | `interface{}` | Sealed Class | discriminated union |
| `anyOf` | Union type | `enum` (untagged) | `interface{}` | `Object` | union type |
| `T \| null` | `T \| None` | `Option<T>` | `*T` | `@Nullable T` | `T \| null` |

> This table is a quick reference. For complete type mapping specifications (including serialization fidelity and edge cases), see [docs/spec/type-mapping.md](../spec/type-mapping.md).

---

## 4. Field Constraints

### 4.1 String Constraints

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    class StringConstraints(BaseModel):
        # Length constraints
        username: str = Field(
            ...,
            description="Username",
            min_length=3,          # Minimum length
            max_length=20,         # Maximum length
        )

        # Regular expression
        email: str = Field(
            ...,
            description="Email",
            pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$",
        )

        phone: str = Field(
            ...,
            description="Phone number",
            pattern=r"^1[3-9]\d{9}$",
        )

        # Format (JSON Schema standard format)
        website: str = Field(
            ...,
            description="Website",
            json_schema_extra={"format": "uri"},
            # Common formats: email, uri, date, time, date-time, uuid
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const StringConstraints = Type.Object({
      // Length constraints
      username: Type.String({
        description: 'Username',
        minLength: 3,
        maxLength: 20,
      }),

      // Regular expression
      email: Type.String({
        description: 'Email',
        pattern: '^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$',
      }),

      phone: Type.String({
        description: 'Phone number',
        pattern: '^1[3-9]\\d{9}$',
      }),

      // Format (JSON Schema standard format)
      website: Type.String({
        description: 'Website',
        format: 'uri',
        // Common formats: email, uri, date, time, date-time, uuid
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn string_constraints_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // Length constraints
                "username": {
                    "type": "string",
                    "description": "Username",
                    "minLength": 3,
                    "maxLength": 20
                },
                // Regular expression
                "email": {
                    "type": "string",
                    "description": "Email",
                    "pattern": "^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$"
                },
                "phone": {
                    "type": "string",
                    "description": "Phone number",
                    "pattern": "^1[3-9]\\d{9}$"
                },
                // Format (JSON Schema standard format)
                "website": {
                    "type": "string",
                    "description": "Website",
                    "format": "uri"
                    // Common formats: email, uri, date, time, date-time, uuid
                }
            },
            "required": ["username", "email", "phone", "website"]
        })
    }
    ```

### 4.2 Numeric Constraints

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    class NumberConstraints(BaseModel):
        # Range constraints
        age: int = Field(
            ...,
            description="Age",
            ge=0,       # >= 0
            le=150,     # <= 150
        )

        price: float = Field(
            ...,
            description="Price",
            gt=0,           # > 0
            lt=1_000_000,   # < 1000000
        )

        # Multiple constraint
        quantity: int = Field(
            ...,
            description="Quantity (must be multiple of 10)",
            multiple_of=10,
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const NumberConstraints = Type.Object({
      // Range constraints (inclusive)
      age: Type.Integer({
        description: 'Age',
        minimum: 0,    // >= 0
        maximum: 150,  // <= 150
      }),

      // Range constraints (exclusive)
      price: Type.Number({
        description: 'Price',
        exclusiveMinimum: 0,        // > 0
        exclusiveMaximum: 1000000,  // < 1000000
      }),

      // Multiple constraint
      quantity: Type.Integer({
        description: 'Quantity (must be multiple of 10)',
        multipleOf: 10,
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn number_constraints_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // Range constraints (inclusive)
                "age": {
                    "type": "integer",
                    "description": "Age",
                    "minimum": 0,
                    "maximum": 150
                },
                // Range constraints (exclusive)
                "price": {
                    "type": "number",
                    "description": "Price",
                    "exclusiveMinimum": 0,
                    "exclusiveMaximum": 1000000
                },
                // Multiple constraint
                "quantity": {
                    "type": "integer",
                    "description": "Quantity (must be multiple of 10)",
                    "multipleOf": 10
                }
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
        # Length constraints
        tags: list[str] = Field(
            ...,
            description="Tags",
            min_length=1,    # At least 1
            max_length=10,   # At most 10
        )

        # Unique elements (requires custom validation in Pydantic)
        unique_ids: list[str] = Field(
            ...,
            description="Unique ID list",
        )

        @field_validator("unique_ids")
        @classmethod
        def check_unique(cls, v):
            if len(v) != len(set(v)):
                raise ValueError('List elements must be unique')
            return v
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const ListConstraints = Type.Object({
      // Length constraints
      tags: Type.Array(Type.String(), {
        description: 'Tags',
        minItems: 1,    // At least 1
        maxItems: 10,   // At most 10
      }),

      // Unique elements — JSON Schema `uniqueItems` keyword
      unique_ids: Type.Array(Type.String(), {
        description: 'Unique ID list',
        uniqueItems: true,
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn list_constraints_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // Length constraints
                "tags": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Tags",
                    "minItems": 1,
                    "maxItems": 10
                },
                // Unique elements — JSON Schema `uniqueItems` keyword
                "unique_ids": {
                    "type": "array",
                    "items": {"type": "string"},
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

**These fields help AI/LLM better understand Schema.**

### 5.1 x-llm-description

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    class LLMFriendlyInput(BaseModel):
        sql: str = Field(
            ...,
            description="SQL statement",
            json_schema_extra={
                "x-llm-description": (
                    "SQL query statement to execute.\n"
                    "- Only SELECT statements allowed\n"
                    "- DROP, DELETE, UPDATE and other modification operations are not allowed\n"
                    "- Table names must use schema.table format"
                ),
            },
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const LLMFriendlyInput = Type.Object({
      sql: Type.String({
        description: 'SQL statement',
        'x-llm-description': [
          'SQL query statement to execute.',
          '- Only SELECT statements allowed',
          '- DROP, DELETE, UPDATE and other modification operations are not allowed',
          '- Table names must use schema.table format',
        ].join('\n'),
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn llm_friendly_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "SQL statement",
                    "x-llm-description": concat!(
                        "SQL query statement to execute.\n",
                        "- Only SELECT statements allowed\n",
                        "- DROP, DELETE, UPDATE and other modification operations are not allowed\n",
                        "- Table names must use schema.table format"
                    )
                }
            },
            "required": ["sql"]
        })
    }
    ```

**Corresponding YAML:**

```yaml
sql:
  type: string
  description: "SQL statement"
  x-llm-description: |
    SQL query statement to execute.
    - Only SELECT statements allowed
    - DROP, DELETE, UPDATE and other modification operations are not allowed
    - Table names must use schema.table format
```

### 5.2 x-examples

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    class WithExamples(BaseModel):
        email: str = Field(
            ...,
            description="Email address",
            json_schema_extra={
                "x-examples": [
                    "user@example.com",
                    "admin@company.org",
                    "test.user@domain.co.jp",
                ],
            },
        )

        phone: str = Field(
            ...,
            description="China mainland mobile number",
            pattern=r"^1[3-9]\d{9}$",
            json_schema_extra={
                "x-examples": ["13800138000", "15912345678"],
            },
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const WithExamples = Type.Object({
      email: Type.String({
        description: 'Email address',
        'x-examples': [
          'user@example.com',
          'admin@company.org',
          'test.user@domain.co.jp',
        ],
      }),

      phone: Type.String({
        description: 'China mainland mobile number',
        pattern: '^1[3-9]\\d{9}$',
        'x-examples': ['13800138000', '15912345678'],
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn with_examples_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "description": "Email address",
                    "x-examples": [
                        "user@example.com",
                        "admin@company.org",
                        "test.user@domain.co.jp"
                    ]
                },
                "phone": {
                    "type": "string",
                    "description": "China mainland mobile number",
                    "pattern": "^1[3-9]\\d{9}$",
                    "x-examples": ["13800138000", "15912345678"]
                }
            },
            "required": ["email", "phone"]
        })
    }
    ```

### 5.3 x-sensitive

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    class WithSensitive(BaseModel):
        username: str = Field(..., description="Username")

        password: str = Field(
            ...,
            description="Password",
            json_schema_extra={
                "x-sensitive": True,  # Mark as sensitive field
            },
        )

        api_key: str = Field(
            ...,
            description="API key",
            json_schema_extra={
                "x-sensitive": True,
            },
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const WithSensitive = Type.Object({
      username: Type.String({ description: 'Username' }),

      password: Type.String({
        description: 'Password',
        'x-sensitive': true, // Mark as sensitive field
      }),

      api_key: Type.String({
        description: 'API key',
        'x-sensitive': true,
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn with_sensitive_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "username": {"type": "string", "description": "Username"},
                "password": {
                    "type": "string",
                    "description": "Password",
                    "x-sensitive": true
                },
                "api_key": {
                    "type": "string",
                    "description": "API key",
                    "x-sensitive": true
                }
            },
            "required": ["username", "password", "api_key"]
        })
    }
    ```

**Handling of sensitive fields:**
- Automatically masked in logs
- Not recorded in trace data
- AI/LLM is advised not to store

---

## 6. Nesting and References

### 6.1 Nested Objects

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    class Address(BaseModel):
        """Address information"""
        city: str = Field(..., description="City")
        street: str = Field(..., description="Street")
        postal_code: str = Field(..., description="Postal code")


    class Company(BaseModel):
        """Company information"""
        name: str = Field(..., description="Company name")
        address: Address = Field(..., description="Company address")  # Nested


    class Employee(BaseModel):
        """Employee information"""
        name: str = Field(..., description="Name")
        company: Company = Field(..., description="Company")  # Multi-level nesting
        home_address: Address = Field(..., description="Home address")  # Reuse
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // Address information — reusable
    const Address = Type.Object({
      city: Type.String({ description: 'City' }),
      street: Type.String({ description: 'Street' }),
      postal_code: Type.String({ description: 'Postal code' }),
    });

    // Company information
    const Company = Type.Object({
      name: Type.String({ description: 'Company name' }),
      address: Type.Composite([Address], { description: 'Company address' }), // Nested
    });

    // Employee information
    const Employee = Type.Object({
      name: Type.String({ description: 'Name' }),
      company: Type.Composite([Company], { description: 'Company' }),       // Multi-level nesting
      home_address: Type.Composite([Address], { description: 'Home address' }), // Reuse
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // Address information — reusable
    fn address_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "city":        {"type": "string", "description": "City"},
                "street":      {"type": "string", "description": "Street"},
                "postal_code": {"type": "string", "description": "Postal code"}
            },
            "required": ["city", "street", "postal_code"]
        })
    }

    // Company information
    fn company_schema() -> Value {
        let mut address = address_schema();
        address["description"] = json!("Company address");
        json!({
            "type": "object",
            "properties": {
                "name":    {"type": "string", "description": "Company name"},
                "address": address
            },
            "required": ["name", "address"]
        })
    }

    // Employee information
    fn employee_schema() -> Value {
        let mut company = company_schema();
        company["description"] = json!("Company");
        let mut home = address_schema();
        home["description"] = json!("Home address");
        json!({
            "type": "object",
            "properties": {
                "name":         {"type": "string", "description": "Name"},
                "company":      company,
                "home_address": home
            },
            "required": ["name", "company", "home_address"]
        })
    }
    ```

### 6.2 References in YAML

```yaml
# Using $ref references
input_schema:
  type: object
  properties:
    shipping_address:
      $ref: "#/definitions/Address"
    billing_address:
      $ref: "#/definitions/Address"

definitions:
  Address:
    type: object
    properties:
      city:
        type: string
      street:
        type: string
```

### 6.3 Cross-File References

```yaml
# schemas/order.schema.yaml
input_schema:
  type: object
  properties:
    customer:
      $ref: "./common/customer.schema.yaml#/definitions/Customer"
    items:
      type: array
      items:
        $ref: "./common/product.schema.yaml#/definitions/OrderItem"
```

---

## 7. Custom Validation

### 7.1 Field Validators

For validation rules that go beyond what JSON Schema can express (cross-field checks, normalization, business rules), use the language's native validation hooks. In Python this is Pydantic's `field_validator`; in TypeScript and Rust the convention is to run validation inside `execute()` and raise/return a structured error before processing.

=== "Python"

    ```python
    from pydantic import BaseModel, Field, field_validator

    class UserInput(BaseModel):
        username: str = Field(..., description="Username")
        email: str = Field(..., description="Email")
        password: str = Field(..., description="Password")
        confirm_password: str = Field(..., description="Confirm password")

        @field_validator('username')
        @classmethod
        def username_alphanumeric(cls, v: str) -> str:
            if not v.isalnum():
                raise ValueError('Username can only contain letters and numbers')
            return v

        @field_validator('email')
        @classmethod
        def email_valid(cls, v: str) -> str:
            if '@' not in v:
                raise ValueError('Email format is incorrect')
            return v.lower()  # Convert to lowercase
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // Schema declares structure and basic constraints (alphanumeric via pattern,
    // email via format). Cross-field rules go in execute().
    const UserInput = Type.Object({
      username: Type.String({
        description: 'Username',
        pattern: '^[A-Za-z0-9]+$', // alphanumeric only
      }),
      email: Type.String({ description: 'Email', format: 'email' }),
      password: Type.String({ description: 'Password' }),
      confirm_password: Type.String({ description: 'Confirm password' }),
    });

    // Custom normalization / cross-field validation runs in execute().
    function normalizeUserInput(inputs: {
      username: string;
      email: string;
      password: string;
      confirm_password: string;
    }) {
      if (!/^[A-Za-z0-9]+$/.test(inputs.username)) {
        throw new Error('Username can only contain letters and numbers');
      }
      if (!inputs.email.includes('@')) {
        throw new Error('Email format is incorrect');
      }
      return { ...inputs, email: inputs.email.toLowerCase() };
    }
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // Schema declares structure and basic constraints. Cross-field rules
    // run inside execute().
    fn user_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "username": {
                    "type": "string",
                    "description": "Username",
                    "pattern": "^[A-Za-z0-9]+$"
                },
                "email": {
                    "type": "string",
                    "description": "Email",
                    "format": "email"
                },
                "password":         {"type": "string", "description": "Password"},
                "confirm_password": {"type": "string", "description": "Confirm password"}
            },
            "required": ["username", "email", "password", "confirm_password"]
        })
    }

    fn normalize_user_input(mut inputs: Value) -> Result<Value, String> {
        let username = inputs["username"].as_str().unwrap_or("");
        if !username.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Username can only contain letters and numbers".into());
        }
        let email = inputs["email"].as_str().unwrap_or("");
        if !email.contains('@') {
            return Err("Email format is incorrect".into());
        }
        inputs["email"] = json!(email.to_lowercase());
        Ok(inputs)
    }
    ```

### 7.2 Model Validators

Model-level validation enforces rules that span multiple fields. Python uses Pydantic's `model_validator`; TypeScript and Rust express the same logic as a plain function invoked from `execute()`.

=== "Python"

    ```python
    from pydantic import BaseModel, Field, model_validator

    class PasswordInput(BaseModel):
        password: str = Field(..., description="Password")
        confirm_password: str = Field(..., description="Confirm password")

        @model_validator(mode='after')
        def passwords_match(self) -> 'PasswordInput':
            if self.password != self.confirm_password:
                raise ValueError('Password entries do not match')
            return self
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const PasswordInput = Type.Object({
      password: Type.String({ description: 'Password' }),
      confirm_password: Type.String({ description: 'Confirm password' }),
    });

    function validatePasswordInput(
      inputs: { password: string; confirm_password: string },
    ): void {
      if (inputs.password !== inputs.confirm_password) {
        throw new Error('Password entries do not match');
      }
    }
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn password_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "password":         {"type": "string", "description": "Password"},
                "confirm_password": {"type": "string", "description": "Confirm password"}
            },
            "required": ["password", "confirm_password"]
        })
    }

    fn validate_password_input(inputs: &Value) -> Result<(), String> {
        if inputs["password"] != inputs["confirm_password"] {
            return Err("Password entries do not match".into());
        }
        Ok(())
    }
    ```

### 7.3 Complex Business Validation

=== "Python"

    ```python
    from pydantic import BaseModel, Field, model_validator


    class OrderItem(BaseModel):
        product_id: str = Field(..., description="Product ID")
        quantity: int = Field(..., description="Quantity")
        price: float = Field(..., description="Unit price")


    class OrderInput(BaseModel):
        items: list[OrderItem] = Field(..., description="Order items")
        coupon_code: str | None = Field(None, description="Coupon")
        total_amount: float = Field(..., description="Total amount")

        @model_validator(mode='after')
        def validate_order(self) -> 'OrderInput':
            # Calculate total product price
            calculated_total = sum(item.price * item.quantity for item in self.items)

            # Validate total amount
            if abs(self.total_amount - calculated_total) > 0.01:
                raise ValueError(f'Total amount is incorrect, should be {calculated_total}')

            # Validate coupon
            if self.coupon_code and not self._is_valid_coupon(self.coupon_code):
                raise ValueError('Coupon is invalid or expired')

            return self

        def _is_valid_coupon(self, code: str) -> bool:
            # Coupon validation logic
            return True
    ```

=== "TypeScript"

    ```typescript
    import { Type, type Static } from '@sinclair/typebox';

    const OrderItem = Type.Object({
      product_id: Type.String({ description: 'Product ID' }),
      quantity: Type.Integer({ description: 'Quantity' }),
      price: Type.Number({ description: 'Unit price' }),
    });

    const OrderInput = Type.Object({
      items: Type.Array(OrderItem, { description: 'Order items' }),
      coupon_code: Type.Optional(
        Type.Union([Type.String(), Type.Null()], {
          description: 'Coupon',
          default: null,
        }),
      ),
      total_amount: Type.Number({ description: 'Total amount' }),
    });

    type OrderInputT = Static<typeof OrderInput>;

    function isValidCoupon(_code: string): boolean {
      // Coupon validation logic
      return true;
    }

    function validateOrder(input: OrderInputT): void {
      const calculatedTotal = input.items.reduce(
        (sum, it) => sum + it.price * it.quantity,
        0,
      );
      if (Math.abs(input.total_amount - calculatedTotal) > 0.01) {
        throw new Error(`Total amount is incorrect, should be ${calculatedTotal}`);
      }
      if (input.coupon_code && !isValidCoupon(input.coupon_code)) {
        throw new Error('Coupon is invalid or expired');
      }
    }
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn order_item_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "product_id": {"type": "string",  "description": "Product ID"},
                "quantity":   {"type": "integer", "description": "Quantity"},
                "price":      {"type": "number",  "description": "Unit price"}
            },
            "required": ["product_id", "quantity", "price"]
        })
    }

    fn order_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": order_item_schema(),
                    "description": "Order items"
                },
                "coupon_code": {
                    "type": ["string", "null"],
                    "description": "Coupon",
                    "default": null
                },
                "total_amount": {"type": "number", "description": "Total amount"}
            },
            "required": ["items", "total_amount"]
        })
    }

    fn is_valid_coupon(_code: &str) -> bool {
        // Coupon validation logic
        true
    }

    fn validate_order(input: &Value) -> Result<(), String> {
        let items = input["items"].as_array().ok_or("items must be an array")?;
        let calculated: f64 = items
            .iter()
            .map(|it| {
                it["price"].as_f64().unwrap_or(0.0)
                    * it["quantity"].as_f64().unwrap_or(0.0)
            })
            .sum();

        let total = input["total_amount"].as_f64().unwrap_or(0.0);
        if (total - calculated).abs() > 0.01 {
            return Err(format!("Total amount is incorrect, should be {}", calculated));
        }

        if let Some(code) = input["coupon_code"].as_str() {
            if !is_valid_coupon(code) {
                return Err("Coupon is invalid or expired".into());
            }
        }
        Ok(())
    }
    ```

---

## 8. Schema Loading Strategy

**apcore supports multiple Schema loading methods:**

```yaml
# apcore.yaml
schema:
  # Loading strategy
  strategy: "yaml_first"  # yaml_first | native_first | yaml_only

  # yaml_first: Load from YAML first, native implementation can override
  # native_first: Prefer Python class definition, YAML as fallback
  # yaml_only: YAML only (pure cross-language scenarios)

  # Directory the YAML schema files are resolved against
  root: "./schemas"

  # Maximum $ref resolution depth
  max_ref_depth: 32
```

`root`, `strategy` and `max_ref_depth` are the whole `schema` namespace —
`defaults.schema.json` declares it `additionalProperties: false`, so any other
key is a configuration error. In particular there is no `schema.validation`
block: whether an undeclared property is rejected, and whether `"42"` counts as
an integer, are properties of the *contract*, not of the host that loaded it
(PROTOCOL_SPEC §4.9, TYPE_MAPPING §17.3).

---

## 9. Best Practices

### 9.1 Every Field Should Have a Description

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    # ✅ Good
    class GoodSchema(BaseModel):
        name: str = Field(..., description="User name, 2-50 characters")
        age: int = Field(..., description="User age, 0-150 years")

    # ❌ Bad
    class BadSchema(BaseModel):
        name: str
        age: int
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // ✅ Good
    const GoodSchema = Type.Object({
      name: Type.String({ description: 'User name, 2-50 characters' }),
      age: Type.Integer({ description: 'User age, 0-150 years' }),
    });

    // ❌ Bad — no descriptions
    const BadSchema = Type.Object({
      name: Type.String(),
      age: Type.Integer(),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // ✅ Good
    fn good_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string",  "description": "User name, 2-50 characters"},
                "age":  {"type": "integer", "description": "User age, 0-150 years"}
            },
            "required": ["name", "age"]
        })
    }

    // ❌ Bad — no descriptions
    fn bad_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age":  {"type": "integer"}
            },
            "required": ["name", "age"]
        })
    }
    ```

### 9.2 Use Explicit Types

=== "Python"

    ```python
    from pydantic import BaseModel
    from typing import Any, Literal

    # ✅ Good: Clear types
    class GoodSchema(BaseModel):
        status: Literal["active", "inactive", "pending"]
        count: int
        price: float

    # ❌ Bad: Vague types
    class BadSchema(BaseModel):
        status: str  # Can be any string
        count: Any   # Unknown type
        price: Any
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // ✅ Good: Clear types
    const GoodSchema = Type.Object({
      status: Type.Union([
        Type.Literal('active'),
        Type.Literal('inactive'),
        Type.Literal('pending'),
      ]),
      count: Type.Integer(),
      price: Type.Number(),
    });

    // ❌ Bad: Vague types
    const BadSchema = Type.Object({
      status: Type.String(),  // Can be any string
      count: Type.Unknown(),  // Unknown type
      price: Type.Unknown(),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // ✅ Good: Clear types
    fn good_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["active", "inactive", "pending"]},
                "count":  {"type": "integer"},
                "price":  {"type": "number"}
            },
            "required": ["status", "count", "price"]
        })
    }

    // ❌ Bad: Vague types
    fn bad_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": {"type": "string"}, // Can be any string
                "count":  {},                  // Unknown type
                "price":  {}
            },
            "required": ["status", "count", "price"]
        })
    }
    ```

### 9.3 Set Reasonable Constraints

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    # ✅ Good: Has reasonable constraints
    class GoodSchema(BaseModel):
        username: str = Field(..., min_length=3, max_length=20)
        age: int = Field(..., ge=0, le=150)
        email: str = Field(..., pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$")

    # ❌ Bad: No constraints
    class BadSchema(BaseModel):
        username: str  # Can be empty string or very long
        age: int       # Can be negative or absurd number
        email: str     # Can be any string
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // ✅ Good: Has reasonable constraints
    const GoodSchema = Type.Object({
      username: Type.String({ minLength: 3, maxLength: 20 }),
      age: Type.Integer({ minimum: 0, maximum: 150 }),
      email: Type.String({ pattern: '^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$' }),
    });

    // ❌ Bad: No constraints
    const BadSchema = Type.Object({
      username: Type.String(), // Can be empty string or very long
      age: Type.Integer(),     // Can be negative or absurd number
      email: Type.String(),    // Can be any string
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // ✅ Good: Has reasonable constraints
    fn good_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "username": {"type": "string",  "minLength": 3, "maxLength": 20},
                "age":      {"type": "integer", "minimum": 0,    "maximum": 150},
                "email":    {"type": "string",  "pattern": "^[\\w\\.-]+@[\\w\\.-]+\\.\\w+$"}
            },
            "required": ["username", "age", "email"]
        })
    }

    // ❌ Bad: No constraints
    fn bad_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "username": {"type": "string"},
                "age":      {"type": "integer"},
                "email":    {"type": "string"}
            },
            "required": ["username", "age", "email"]
        })
    }
    ```

### 9.4 Separate Complex Schemas

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    # ✅ Good: Reusable and clear
    class Address(BaseModel):
        """Address - reusable"""
        city: str = Field(..., description="City")
        street: str = Field(..., description="Street")

    class OrderInputGood(BaseModel):
        shipping_address: Address
        billing_address: Address

    # ❌ Bad: Duplicate definitions
    class OrderInputBad(BaseModel):
        shipping_city: str
        shipping_street: str
        billing_city: str
        billing_street: str
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // ✅ Good: Reusable and clear
    const Address = Type.Object({
      city: Type.String({ description: 'City' }),
      street: Type.String({ description: 'Street' }),
    });

    const OrderInputGood = Type.Object({
      shipping_address: Type.Composite([Address]),
      billing_address: Type.Composite([Address]),
    });

    // ❌ Bad: Duplicate definitions
    const OrderInputBad = Type.Object({
      shipping_city: Type.String(),
      shipping_street: Type.String(),
      billing_city: Type.String(),
      billing_street: Type.String(),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // ✅ Good: Reusable and clear
    fn address_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "city":   {"type": "string", "description": "City"},
                "street": {"type": "string", "description": "Street"}
            },
            "required": ["city", "street"]
        })
    }

    fn order_input_good_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "shipping_address": address_schema(),
                "billing_address":  address_schema()
            },
            "required": ["shipping_address", "billing_address"]
        })
    }

    // ❌ Bad: Duplicate definitions
    fn order_input_bad_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "shipping_city":   {"type": "string"},
                "shipping_street": {"type": "string"},
                "billing_city":    {"type": "string"},
                "billing_street":  {"type": "string"}
            },
            "required": ["shipping_city", "shipping_street", "billing_city", "billing_street"]
        })
    }
    ```

---

## 10. Edge Case Handling

### 10.1 null vs Empty String

| Scenario | JSON Value | Meaning |
|------|---------|------|
| Field value is `null` | `{"name": null}` | Field exists but has no value |
| Field does not exist | `{}` | Field is missing |
| Empty string | `{"name": ""}` | Field exists and value is empty string |

Implementations **must** distinguish these three cases. The `required` constraint checks if the field exists, `nullable` controls whether `null` is allowed.

### 10.2 Large Number Handling

apcore specifies **2^53 - 1** (`9007199254740991`) as the cross-language integer safe boundary (determined by JavaScript `Number.MAX_SAFE_INTEGER`).

| Scenario | Schema Definition | Description |
|------|------------|------|
| Within safe range (≤ 2^53 - 1) | `type: integer` | Use directly, lossless across all languages |
| Beyond safe range | `type: string` + `format: int64` | 64-bit integer, transmitted as string |
| Arbitrary precision integer | `type: string` + `format: bigint` | Like blockchain nonce |
| High precision decimal | `type: string` + `format: decimal` | Like currency, exchange rate |

=== "Python"

    ```python
    from pydantic import BaseModel, Field

    class PaymentInput(BaseModel):
        # Within safe range — use int directly
        user_id: int = Field(..., description="User ID", ge=0)

        # Beyond safe range — use string + format
        order_no: str = Field(
            ...,
            description="Snowflake order number",
            pattern=r"^\d+$",
            json_schema_extra={"format": "int64"},
        )

        # High precision amount
        amount: str = Field(
            ...,
            description="Payment amount (accurate to cent)",
            pattern=r"^-?\d+\.\d{2}$",
            json_schema_extra={"format": "decimal"},
        )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    const PaymentInput = Type.Object({
      // Within safe range — use integer directly
      user_id: Type.Integer({ description: 'User ID', minimum: 0 }),

      // Beyond safe range — use string + format
      order_no: Type.String({
        description: 'Snowflake order number',
        pattern: '^\\d+$',
        format: 'int64',
      }),

      // High precision amount
      amount: Type.String({
        description: 'Payment amount (accurate to cent)',
        pattern: '^-?\\d+\\.\\d{2}$',
        format: 'decimal',
      }),
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    fn payment_input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                // Within safe range — use integer directly
                "user_id": {
                    "type": "integer",
                    "description": "User ID",
                    "minimum": 0
                },
                // Beyond safe range — use string + format
                "order_no": {
                    "type": "string",
                    "description": "Snowflake order number",
                    "pattern": "^\\d+$",
                    "format": "int64"
                },
                // High precision amount
                "amount": {
                    "type": "string",
                    "description": "Payment amount (accurate to cent)",
                    "pattern": "^-?\\d+\\.\\d{2}$",
                    "format": "decimal"
                }
            },
            "required": ["user_id", "order_no", "amount"]
        })
    }
    ```

> For complete large number type mapping specifications, see [docs/spec/type-mapping.md §14.1](../spec/type-mapping.md#141-large-integer-precision-loss).

### 10.3 Unicode Handling

- All `string` types **must** support UTF-8 encoding
- `minLength` / `maxLength` **should** count by Unicode characters (code points), not bytes
- When containing Emoji and combining characters, implementations **should** use NFC normalized form

### 10.4 Nesting Depth Limit

- Schema object nesting depth **should** not exceed 16 levels
- `$ref` recursion depth **must** not exceed 32 levels
- Implementations **should** issue warnings when detecting excessive nesting

---

## 11. AI-Friendly Schema Design

> All mainstream AI protocols (MCP, OpenAI, Anthropic, Gemini, LangChain) are based on JSON Schema. apcore's use of JSON Schema Draft 2020-12 is the correct choice. This section provides design guidelines to make Schema easier for AI/LLM to correctly understand and call.

### 11.1 Flat is Better Than Nested

AI/LLM accuracy significantly decreases with deeply nested structures. input_schema **should** keep nesting within 2-3 levels.

```yaml
# ❌ Bad: Deep nesting, hard for AI to fill accurately
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
                host:
                  type: string
                port:
                  type: integer

# ✅ Good: Flat, clear for AI
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

> **Reference**: §10.4 specifies nesting depth limit of 16 levels, but modules for AI calling **should** not exceed 3 levels.

### 11.2 Description Quality Rules

Each field's `description` should answer "**What does the AI need to know to correctly fill this field?**"

| Rule | Description |
|------|------|
| Explain semantics, not type | Schema already declares `type: string`, description doesn't need to repeat "string type" |
| Explain value range | Like "Supported formats: png, jpg, gif" |
| Explain default behavior | Like "If not filled, send to all subscribers" |
| Explain related constraints | Like "When format is html, template_id must be provided" |

=== "Python"

    ```python
    from pydantic import Field

    # ❌ Bad: Repeats type information, no actual guidance
    body_bad = Field(
        ...,
        description="Email body, string type",
    )

    # ✅ Good: Explains semantics and usage
    body_good = Field(
        ...,
        description="Email body content, supports plain text or HTML. HTML format requires html=true",
    )
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';

    // ❌ Bad: Repeats type information, no actual guidance
    const bodyBad = Type.String({
      description: 'Email body, string type',
    });

    // ✅ Good: Explains semantics and usage
    const bodyGood = Type.String({
      description:
        'Email body content, supports plain text or HTML. HTML format requires html=true',
    });
    ```

=== "Rust"

    ```rust
    use serde_json::{json, Value};

    // ❌ Bad: Repeats type information, no actual guidance
    fn body_bad() -> Value {
        json!({"type": "string", "description": "Email body, string type"})
    }

    // ✅ Good: Explains semantics and usage
    fn body_good() -> Value {
        json!({
            "type": "string",
            "description": "Email body content, supports plain text or HTML. HTML format requires html=true"
        })
    }
    ```

### 11.3 Token Awareness

AI protocols serialize Schema and inject it into the prompt, consuming context tokens. Reducing unnecessary token consumption can improve AI reasoning.

| Recommendation | Description |
|------|------|
| Avoid repeating type info in description | `type: string` already declares type, description doesn't need to mention it |
| Use x-llm-description when enum has more than 5 values | Avoid AI guessing enum meanings one by one |
| Use first sentence description in compact mode | See [Schema System](../features/schema-system.md) compact export |
| Send simplified Schema during module discovery | Load full Schema after module is selected (progressive disclosure) |

```yaml
# ❌ Bad: Many enum values without explanation
status:
  type: string
  enum: ["draft", "pending_review", "in_review", "approved", "rejected",
         "published", "archived", "suspended", "deleted"]

# ✅ Good: Use x-llm-description to explain usage scenarios
status:
  type: string
  enum: ["draft", "pending_review", "in_review", "approved", "rejected",
         "published", "archived", "suspended", "deleted"]
  description: "Article status"
  x-llm-description: |
    Article lifecycle status. Use draft when creating, pending_review for submission,
    approved after review, published for going live.
    Other statuses (archived/suspended/deleted) are for admin operations, generally not used during creation.
```

### 11.4 Required Field Priority

In the `properties` block, list required fields (`required`) first, then optional fields. This helps AI grasp core parameters faster when reading Schema.

```yaml
# ✅ Recommended: Required fields first
input_schema:
  type: object
  properties:
    # --- Required fields ---
    to:
      type: string
      description: "Recipient email address"
    subject:
      type: string
      description: "Email subject"
    body:
      type: string
      description: "Email body"
    # --- Optional fields ---
    cc:
      type: array
      items: { type: string }
      description: "CC list"
      default: []
    html:
      type: boolean
      description: "Is HTML format"
      default: false
  required: [to, subject, body]
```

### 11.5 Design Checklist

Before publishing modules for AI calling, it's recommended to check against the following checklist:

- [ ] input_schema nesting does not exceed 3 levels
- [ ] Every field has description, and does not repeat type information
- [ ] Required fields are listed before optional fields in properties
- [ ] Provide x-llm-description when enum has more than 5 values
- [ ] Complex modules (oneOf/anyOf, 5+ required fields) provide examples
- [ ] Sensitive fields marked with x-sensitive
- [ ] Numeric fields declare `minimum`/`maximum` and `default` where applicable (Pydantic: `Field(ge=, le=)`)
- [ ] String fields with fixed options use `enum` or `Literal[...]` instead of free-form text

---

## 12. Cross-Protocol Compatibility

> apcore uses JSON Schema Draft 2020-12, but some AI protocols (like MCP) are still based on Draft 7. Modules aimed at broad AI protocol compatibility **should** prioritize using the common subset of both versions.

### 12.1 Safe Features (Draft 7 + 2020-12 Common)

The following keywords can be safely used in all mainstream AI protocols:

| Keyword | Description |
|--------|------|
| `type` | Type declaration |
| `properties` | Object property definition |
| `required` | Required field list |
| `enum` | Enum values |
| `const` | Constant value |
| `description` | Field description |
| `default` | Default value |
| `minimum` / `maximum` | Numeric range |
| `minLength` / `maxLength` | String length |
| `pattern` | Regular expression constraint |
| `items` | Array element definition |
| `$ref` | Reference (local reference) |
| `oneOf` / `anyOf` / `allOf` | Combined types |
| `additionalProperties` | Allow additional properties |
| `format` | Format annotation (like `date-time`, `email`) |

### 12.2 Use with Caution (Draft 2020-12 Specific)

The following keywords are only available in Draft 2020-12, some AI protocols may not support:

| Keyword | Description | Risk |
|--------|------|------|
| `if` / `then` / `else` | Conditional Schema | MCP Draft 7 doesn't support; some AI protocols ignore |
| `$dynamicRef` / `$dynamicAnchor` | Dynamic reference | Most AI protocols don't support |
| `prefixItems` | Tuple definition (replaces old `items` array form) | Draft 7 uses `items` array form |
| `$anchor` | Named anchor | Draft 7 uses `$id` |
| `dependentRequired` / `dependentSchemas` | Dependency declaration | Draft 7 uses `dependencies` |

### 12.3 Recommendations

- Modules aimed at broad AI protocol compatibility **SHOULD** limit to the safe subset in §12.1
- Modules used only within apcore and not exported to external AI protocols can freely use all Draft 2020-12 features
- When using `oneOf`/`anyOf`, **SHOULD** provide `examples` to help AI understand branch meanings
- Cross-file `$ref` **SHOULD** be inlined when exported, as most AI protocols don't support external reference resolution

---

## Next Steps

- [Creating Modules Guide](./creating-modules.md) - Complete module creation tutorial
- [Module Interface](../features/module-interface.md) - Module Protocol contract
- [ACL Configuration Guide](./acl-configuration.md) - Access control configuration
