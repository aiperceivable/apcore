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

### 2.1 Pydantic Model (Recommended)

**The preferred method for Python development.**

```python
from pydantic import BaseModel, Field
from typing import Literal
from datetime import datetime

class OrderInput(BaseModel):
    """Order creation input"""

    # Required fields
    product_id: str = Field(
        ...,                          # ... means required
        description="Product ID",
        min_length=1,
        max_length=50
    )

    quantity: int = Field(
        ...,
        description="Purchase quantity",
        ge=1,                         # >= 1
        le=100                        # <= 100
    )

    # Optional fields (must have default value)
    note: str | None = Field(
        None,                         # Default value
        description="Order note",
        max_length=500
    )

    # Enum type
    payment_method: Literal["alipay", "wechat", "card"] = Field(
        "alipay",
        description="Payment method"
    )

    # Nested object
    shipping_address: "Address" = Field(
        ...,
        description="Shipping address"
    )


class Address(BaseModel):
    """Address information"""
    province: str = Field(..., description="Province")
    city: str = Field(..., description="City")
    district: str = Field(..., description="District")
    detail: str = Field(..., description="Detailed address")
    postal_code: str = Field(..., description="Postal code", pattern=r"^\d{6}$")
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

```python
from typing import Optional

class OptionalTypes(BaseModel):
    # Method 1: | None
    email: str | None = Field(None, description="Email")

    # Method 2: Optional (equivalent to above)
    phone: Optional[str] = Field(None, description="Phone")

    # Non-None field with default value
    count: int = Field(default=0, description="Count")
```

### 3.3 Lists and Dictionaries

```python
from typing import List, Dict

class CollectionTypes(BaseModel):
    # String list
    tags: list[str] = Field(default=[], description="Tag list")

    # Object list
    items: list["OrderItem"] = Field(..., description="Order items")

    # Dictionary
    metadata: dict[str, Any] = Field(default={}, description="Metadata")

    # Dictionary with specified value type
    scores: dict[str, int] = Field(default={}, description="Score mapping")


class OrderItem(BaseModel):
    product_id: str = Field(..., description="Product ID")
    quantity: int = Field(..., description="Quantity")
    price: float = Field(..., description="Unit price")
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

```python
from typing import Literal
from enum import Enum

# Method 1: Literal (recommended)
class Order1(BaseModel):
    status: Literal["pending", "paid", "shipped", "done"] = Field(
        ...,
        description="Order status"
    )

    priority: Literal[1, 2, 3] = Field(
        default=2,
        description="Priority: 1-high 2-medium 3-low"
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

### 3.5 Date and Time

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
        pattern=r"^\d{4}-\d{2}-\d{2}$"  # YYYY-MM-DD
    )
```

### 3.6 Cross-Language Type Mapping Quick Reference

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
| `string` + `format: email` | `str` + validation | `String` + validation | `string` + validation | `String` + validation | `string` + validation |
| `string` + `format: uri` | `str` + validation | `String` + validation | `string` + validation | `String` + validation | `string` + validation |
| `string` + `format: uuid` | `UUID` | `Uuid` | `uuid.UUID` | `UUID` | `string` + validation |
| `string enum` | `Literal[...]` / `StrEnum` | `enum` | `type T string` + const | `enum` | union type |
| `integer enum` | `IntEnum` | `enum` | `type T int64` + iota | `enum` | union type |
| `oneOf` | Discriminated Union | `enum` (tagged) | `interface{}` | Sealed Class | discriminated union |
| `anyOf` | Union type | `enum` (untagged) | `interface{}` | `Object` | union type |
| `T \| null` | `T \| None` | `Option<T>` | `*T` | `@Nullable T` | `T \| null` |

> This table is a quick reference. For complete type mapping specifications (including serialization fidelity and edge cases), see [docs/spec/type-mapping.md](../spec/type-mapping.md).

---

## 4. Field Constraints

### 4.1 String Constraints

```python
class StringConstraints(BaseModel):
    # Length constraints
    username: str = Field(
        ...,
        description="Username",
        min_length=3,          # Minimum length
        max_length=20          # Maximum length
    )

    # Regular expression
    email: str = Field(
        ...,
        description="Email",
        pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$"
    )

    phone: str = Field(
        ...,
        description="Phone number",
        pattern=r"^1[3-9]\d{9}$"
    )

    # Format (JSON Schema standard format)
    website: str = Field(
        ...,
        description="Website",
        # Common formats: email, uri, date, time, date-time, uuid
    )
```

### 4.2 Numeric Constraints

```python
class NumberConstraints(BaseModel):
    # Range constraints
    age: int = Field(
        ...,
        description="Age",
        ge=0,       # >= 0
        le=150      # <= 150
    )

    price: float = Field(
        ...,
        description="Price",
        gt=0,       # > 0
        lt=1000000  # < 1000000
    )

    # Multiple constraint
    quantity: int = Field(
        ...,
        description="Quantity (must be multiple of 10)",
        multiple_of=10
    )
```

### 4.3 List Constraints

```python
from pydantic import BaseModel, Field, field_validator

class ListConstraints(BaseModel):
    # Length constraints
    tags: list[str] = Field(
        ...,
        description="Tags",
        min_length=1,    # At least 1
        max_length=10    # At most 10
    )

    # Unique elements (requires custom validation)
    unique_ids: list[str] = Field(
        ...,
        description="Unique ID list"
    )

    @field_validator("unique_ids")
    @classmethod
    def check_unique(cls, v):
        if len(v) != len(set(v)):
            raise ValueError('List elements must be unique')
        return v
```

---

## 5. LLM Extension Fields

**These fields help AI/LLM better understand Schema.**

### 5.1 x-llm-description

```python
class LLMFriendlyInput(BaseModel):
    sql: str = Field(
        ...,
        description="SQL statement",
        json_schema_extra={
            "x-llm-description": """
            SQL query statement to execute.
            - Only SELECT statements allowed
            - DROP, DELETE, UPDATE and other modification operations are not allowed
            - Table names must use schema.table format
            """
        }
    )
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

```python
class WithExamples(BaseModel):
    email: str = Field(
        ...,
        description="Email address",
        json_schema_extra={
            "x-examples": [
                "user@example.com",
                "admin@company.org",
                "test.user@domain.co.jp"
            ]
        }
    )

    phone: str = Field(
        ...,
        description="China mainland mobile number",
        pattern=r"^1[3-9]\d{9}$",
        json_schema_extra={
            "x-examples": ["13800138000", "15912345678"]
        }
    )
```

### 5.3 x-sensitive

```python
class WithSensitive(BaseModel):
    username: str = Field(..., description="Username")

    password: str = Field(
        ...,
        description="Password",
        json_schema_extra={
            "x-sensitive": True  # Mark as sensitive field
        }
    )

    api_key: str = Field(
        ...,
        description="API key",
        json_schema_extra={
            "x-sensitive": True
        }
    )
```

**Handling of sensitive fields:**
- Automatically masked in logs
- Not recorded in trace data
- AI/LLM is advised not to store

---

## 6. Nesting and References

### 6.1 Nested Objects

```python
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

### 7.2 Model Validators

```python
from pydantic import BaseModel, model_validator

class PasswordInput(BaseModel):
    password: str = Field(..., description="Password")
    confirm_password: str = Field(..., description="Confirm password")

    @model_validator(mode='after')
    def passwords_match(self) -> 'PasswordInput':
        if self.password != self.confirm_password:
            raise ValueError('Password entries do not match')
        return self
```

### 7.3 Complex Business Validation

```python
class OrderInput(BaseModel):
    items: list["OrderItem"] = Field(..., description="Order items")
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

  paths:
    yaml_schemas: "./schemas"

  validation:
    strict: true        # Strict mode: extra fields not allowed
    coerce_types: true  # Type coercion
```

---

## 9. Best Practices

### 9.1 Every Field Should Have a Description

```python
# ✅ Good
class GoodSchema(BaseModel):
    name: str = Field(..., description="User name, 2-50 characters")
    age: int = Field(..., description="User age, 0-150 years")

# ❌ Bad
class BadSchema(BaseModel):
    name: str
    age: int
```

### 9.2 Use Explicit Types

```python
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

### 9.3 Set Reasonable Constraints

```python
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

### 9.4 Separate Complex Schemas

```python
# ✅ Good: Reusable and clear
class Address(BaseModel):
    """Address - reusable"""
    city: str = Field(..., description="City")
    street: str = Field(..., description="Street")

class OrderInput(BaseModel):
    shipping_address: Address
    billing_address: Address

# ❌ Bad: Duplicate definitions
class OrderInput(BaseModel):
    shipping_city: str
    shipping_street: str
    billing_city: str
    billing_street: str
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
        json_schema_extra={"format": "int64"}
    )

    # High precision amount
    amount: str = Field(
        ...,
        description="Payment amount (accurate to cent)",
        pattern=r"^-?\d+\.\d{2}$",
        json_schema_extra={"format": "decimal"}
    )
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

```python
# ❌ Bad: Repeats type information, no actual guidance
body: str = Field(
    ...,
    description="Email body, string type"
)

# ✅ Good: Explains semantics and usage
body: str = Field(
    ...,
    description="Email body content, supports plain text or HTML. HTML format requires html=true"
)
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
