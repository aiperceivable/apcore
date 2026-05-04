# Module Interface Definition

<!-- preamble-tier-doc -->
> **Type:** API reference. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §5.6 Module Interface Protocol.


> **Canonical Definition** - This document is the authoritative definition of the Module interface

> Module is the core interface of apcore, and all modules must implement this interface.

!!! info "Scope: Python SDK"
    Code examples on this page use the **Python SDK** (`apcore-python`) — types like `pydantic.BaseModel`, `ClassVar[Type[...]]`, and `Protocol` are Python idioms. Each language SDK provides equivalent constructs (TypeScript: Zod schemas + interfaces; Rust: serde structs + traits). For cross-language type mapping, see [Type Mapping](../spec/type-mapping.md).

## 1. Interface Overview

```python
from typing import Any, ClassVar, Protocol, Type, runtime_checkable
from pydantic import BaseModel


@runtime_checkable
class Module(Protocol):
    """apcore module interface (structural typing / duck typing).

    Modules MUST NOT inherit from an ABC. Any class that provides the
    required attributes and methods below is a valid apcore module.
    The framework checks conformance via structural (duck-type) matching.
    """

    # ============ Required Definitions ============

    # Input Schema (required)
    input_schema: ClassVar[Type[BaseModel]]

    # Output Schema (required)
    output_schema: ClassVar[Type[BaseModel]]

    # Short description (required, ≤200 characters)
    # From docstring or explicit definition
    description: ClassVar[str]

    # Detailed documentation (optional, ≤5000 characters, supports Markdown)
    documentation: ClassVar[str | None] = None

    def execute(self, inputs: dict[str, Any], context: "Context") -> dict[str, Any]:
        """
        Execute module logic (must be implemented)

        Args:
            inputs: Input parameters (validated against input_schema)
            context: Execution context

        Returns:
            Output result (validated against output_schema)

        Raises:
            ModuleError: Module execution error
        """
        ...

    # ============ Optional Definitions ============

    # Module name (optional, generated from class name by default)
    name: ClassVar[str | None] = None

    # Module tags (optional)
    tags: ClassVar[list[str]] = []

    # Module version (optional)
    version: ClassVar[str] = "1.0.0"

    # Behavior annotations (optional, helps AI make invocation decisions)
    annotations: ClassVar["ModuleAnnotations | None"] = None

    # Usage examples (optional, helps AI understand complex modules)
    examples: ClassVar[list["ModuleExample"]] = []

    # Extended metadata (optional, free-form dict)
    metadata: ClassVar[dict[str, Any]] = {}

    # ============ Lifecycle Hooks (optional) ============

    def on_load(self) -> None:
        """Called when module is loaded"""
        pass

    def on_unload(self) -> None:
        """Called when module is unloaded"""
        pass

    def on_suspend(self) -> dict | None:
        """Called before hot-reload. Return state to preserve, or None."""
        return None

    def on_resume(self, state: dict) -> None:
        """Called after hot-reload. Restore state from on_suspend()."""
        pass

    # ============ Optional Methods ============

    def preflight(self, inputs: dict[str, Any], context: "Context") -> list[str]:
        """Return domain-specific warnings before execution (advisory only).
        
        Called during validate() as the final check. Warnings are included
        in PreflightCheckResult but do not block execution.
        Default: returns empty list.
        """
        return []

    def describe(self) -> dict[str, Any]:
        """Return module metadata for introspection.
        
        Used by Registry.describe() and system.manifest modules.
        Default: returns dict with description, input_schema, output_schema, annotations.
        """
        ...

    def validate(self, inputs: dict[str, Any]) -> "ValidationResult":
        """
        Validate input only, without execution (optional implementation)

        Args:
            inputs: Input parameters

        Returns:
            Validation result
        """
        ...

    # Note: Modules only need to define one execute() method, using either def or async def.
    # The framework automatically detects and selects the appropriate invocation method (sync or async), no need to define separately.

    # ============ Optional: Streaming ============

    async def stream(self, inputs: dict[str, Any], context: "Context") -> AsyncIterator[dict[str, Any]]:
        """
        Stream module output chunk by chunk (optional implementation)

        When defined, Executor.stream() will call this method instead of execute().
        Each yielded dict is a partial result chunk. The complete result is the
        shallow merge of all yielded chunks.

        Modules implementing stream() SHOULD also set annotations.streaming = True.
        """
        ...
```

> **Important**: Modules only need to implement `execute()` and provide the required schema
> attributes (`input_schema`, `output_schema`, `description`). No ABC inheritance is
> required (PROTOCOL_SPEC §5.6). The framework validates module conformance structurally
> at registration time.

---

## 2. Required Attributes

!!! note "Protocol conformance, not ABC inheritance"
    Examples below use `class MyModule(Module):` syntax. Since `Module` is a `Protocol` (not an ABC), this is a **static typing hint** — it enables IDE autocompletion and type checking but does NOT require inheritance. Any class with the required attributes and methods is a valid module via duck typing. You can equally write `class MyModule:` without inheriting from `Module`.

### 2.1 input_schema

**Defines the structure of module input parameters.**

```python
from pydantic import BaseModel, Field

class SendEmailInput(BaseModel):
    """Input parameters for sending email"""

    to: str = Field(
        ...,
        description="Recipient email address",
        pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$",
        examples=["user@example.com"]
    )

    subject: str = Field(
        ...,
        description="Email subject",
        max_length=200
    )

    body: str = Field(
        ...,
        description="Email body"
    )

    cc: list[str] = Field(
        default=[],
        description="CC list"
    )


class SendEmailModule(Module):
    input_schema = SendEmailInput  # Must be defined
    ...
```

**input_schema Requirements:**

| Requirement | Description |
|------|------|
| Type | Must be a subclass of `pydantic.BaseModel` |
| Field descriptions | Every field must have a `description` |
| Constraint definitions | Recommended to define `pattern`, `min`/`max` constraints |
| Default values | Optional fields must have default values |

---

### 2.2 output_schema

**Defines the structure of module output.**

```python
class SendEmailOutput(BaseModel):
    """Output result for sending email"""

    success: bool = Field(
        ...,
        description="Whether the email was sent successfully"
    )

    message_id: str | None = Field(
        None,
        description="Message ID (returned when successfully sent)"
    )

    error: str | None = Field(
        None,
        description="Error message (returned when sending failed)"
    )


class SendEmailModule(Module):
    output_schema = SendEmailOutput  # Must be defined
    ...
```

**output_schema Requirements:**

| Requirement | Description |
|------|------|
| Type | Must be a subclass of `pydantic.BaseModel` |
| Field descriptions | Every field must have a `description` |
| Consistency | execute() return value must conform to this Schema |

---

### 2.3 description and documentation

**Module functionality description, used by AI/LLM to understand the module.**

apcore uses two fields to organize module documentation:

| Field | Required | Length Limit | Markdown | Purpose |
|------|--------|----------|----------|------|
| `description` | **Required** | ≤200 characters | No | Short description of module functionality for AI quick matching and understanding |
| `documentation` | Optional | ≤5000 characters | Yes | Detailed documentation including usage scenarios, constraints, configuration requirements |

#### description Field (Required)

```python
# Method 1: Via docstring (recommended)
class SendEmailModule(Module):
    """Send email to specified recipient. Uses SMTP protocol, non-idempotent operation, requires email server configuration."""
    ...

# Method 2: Explicit definition
class SendEmailModule(Module):
    description = "Send email to specified recipient. Uses SMTP protocol, non-idempotent operation, requires email server configuration."
    ...
```

**description Requirements:**

| Requirement | Description |
|------|------|
| Must exist | Either docstring or explicit definition |
| Length limit | ≤200 characters (approximately 100 Chinese characters) |
| Content requirements | Describe "what it does" + "when to use" + "key features" |
| Clear and precise | AI/LLM needs to understand module functionality |

#### documentation Field (Optional)

```python
class SendEmailModule(Module):
    description = "Send email to specified recipient. Uses SMTP protocol, non-idempotent operation, requires email server configuration."

    documentation = """
# Features
Send email via SMTP protocol, supports plain text and HTML format.

## Configuration Requirements
- SMTP server information must be configured in apcore.yaml
- Valid SMTP authentication credentials must be provided

## Usage Scenarios
- Send notification emails, verification codes, reports

## Limitations
- Gmail: 500 emails/day
- Attachment size: ≤25MB
"""
```

**When to use documentation:**

| Scenario | Need documentation |
|------|----------------------|
| Simple modules (e.g., validation functions) | No - only description needed |
| Complex configuration requirements | Yes |
| Important usage constraints | Yes |
| Multiple usage scenarios | Yes |

#### Relationship with docstring

| Location | Purpose | Audience | Format |
|------|------|------|------|
| `description` | Quick understanding of module functionality | AI module discovery phase | Plain text, ≤200 characters |
| `documentation` | Detailed usage documentation | AI invocation decision phase | Markdown, ≤5000 characters |
| Python docstring | Code-level documentation | Developers, IDE | reStructuredText/Markdown |

---

### 2.4 execute()

**The core execution logic of the module.**

```python
class SendEmailModule(Module):
    input_schema = SendEmailInput
    output_schema = SendEmailOutput

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        """
        Execute email sending

        Args:
            inputs: Input parameters, validated against input_schema
            context: Execution context, includes trace_id, caller_id, etc.

        Returns:
            Output result, must conform to output_schema
        """
        # 1. Can directly use inputs (already validated)
        to = inputs["to"]
        subject = inputs["subject"]
        body = inputs["body"]

        # 2. Or convert to Pydantic object
        params = self.input_schema(**inputs)

        # 3. Execute business logic
        try:
            message_id = self._send_email(params.to, params.subject, params.body)
            return {"success": True, "message_id": message_id, "error": None}
        except Exception as e:
            return {"success": False, "message_id": None, "error": str(e)}

    def _send_email(self, to: str, subject: str, body: str) -> str:
        """Internal method: actually send email"""
        # ... implementation ...
        return "msg_123"
```

**execute() Specifications:**

| Specification | Description |
|------|------|
| Parameters | `inputs: dict` and `context: Context` |
| Return value | Must be `dict`, conforming to `output_schema` |
| Input validated | Framework has validated against `input_schema`, can use directly |
| Output validated | Return value validated against `output_schema` |
| Exception handling | Can throw `ModuleError`, framework handles uniformly |

### 2.5 Interface Contract Summary

| Interface | Level | Contract |
|------|------|------|
| `input_schema` | **MUST** | Must be defined, must be a valid Schema type |
| `output_schema` | **MUST** | Must be defined, must be a valid Schema type |
| `description` | **MUST** | Must be provided via docstring or explicit attribute, ≤200 characters |
| `documentation` | **MAY** | Optional; ≤5000 characters, supports Markdown |
| `execute()` | **MUST** | Must be implemented (`def` or `async def`), framework auto-detects sync/async |
| `validate()` | **MAY** | Optional implementation; should have no side effects when called |
| `preflight()` | **MAY** | Optional; returns advisory warnings, does not block execution |
| `describe()` | **MAY** | Optional; returns module metadata dict for introspection |
| `on_load()` / `on_unload()` | **MAY** | Optional implementation; exceptions should not block other module loading |
| `on_suspend()` / `on_resume()` | **MAY** | Optional; preserve and restore state across hot-reload cycles |
| `name` | **MAY** | Optional; generated from class name by default |
| `tags` | **MAY** | Optional; empty list by default |
| `version` | **MAY** | Optional; defaults to "1.0.0", must conform to semver |
| `annotations` | **MAY** | Optional; all values use defaults by default |
| `examples` | **MAY** | Optional; inputs must conform to input_schema |
| `metadata` | **MAY** | Optional; empty dict by default |

**Timeout Semantics:**
- Module execution **should** complete within configured timeout (`resources.timeout` default 30000ms; global `executor.global_timeout` default 60000ms)
- After timeout, framework **must** throw `MODULE_TIMEOUT` error
- Module **should** support graceful cancellation (by checking cancellation signal in context)

**Thread Safety Specifications:**
- Module instances **must** support concurrent calls to `execute()` by multiple threads/coroutines
- Modules **must not** modify instance-level state (ClassVar attributes) in `execute()`
- If shared state is needed, **must** use thread-safe data structures

**Return Value Constraints:**
- `execute()` **must** return `dict` (or language-equivalent Map type)
- Return value **must** pass `output_schema` validation
- Return value **must not** include non-serializable objects (functions, connections, etc.)

---

## 3. Optional Attributes

### 3.1 name

```python
class SendEmailModule(Module):
    name = "Send Email"  # Human-readable name

    # If not defined, generated from class name by default:
    # SendEmailModule → "Send Email Module"
```

### 3.2 tags

```python
class SendEmailModule(Module):
    tags = ["email", "notification", "communication"]

    # Used for categorization and search
    # registry.list(tags=["email"]) can filter
```

### 3.3 version

```python
class SendEmailModule(Module):
    version = "2.0.0"

    # Used for version management
    # Defaults to "1.0.0"
```

### 3.4 annotations

> **Canonical Definition** - This section is the authoritative definition of ModuleAnnotations

**Behavior annotations, help AI/LLM make invocation decisions.**

```python
from dataclasses import dataclass


@dataclass(frozen=True)
class ModuleAnnotations:
    """Module behavior annotations"""
    readonly: bool = False          # Read-only, no side effects
    destructive: bool = False       # Has destructive operations
    idempotent: bool = False        # Idempotent, safe to call repeatedly
    requires_approval: bool = False # Requires human confirmation
    open_world: bool = True         # Involves external systems
    streaming: bool = False         # Supports streaming output
    cacheable: bool = False         # Output can be cached
    cache_ttl: int = 0              # Cache duration in seconds (0 = no cache)
    cache_key_fields: tuple[str, ...] | None = None  # Input fields for cache key (None = all; lists are auto-converted to tuples)
    paginated: bool = False         # Returns paginated results
    pagination_style: str = "cursor"  # "cursor", "offset", or "page"
    extra: dict[str, Any] = field(default_factory=dict)  # Extension metadata for ecosystem packages
```

!!! note "cache_key_fields type"
    `cache_key_fields` is a `tuple[str, ...]` to ensure immutability. Lists passed in are automatically converted to tuples.

!!! note "extra field"
    `extra` captures any additional key-value pairs not covered by the standard annotation fields, enabling ecosystem packages to attach custom metadata without modifying the core `ModuleAnnotations` definition.

| Field | Default | Meaning | AI Behavior |
|------|--------|------|---------|
| `readonly` | `False` | Does not modify any state | `True` → Safe to call |
| `destructive` | `False` | May delete/overwrite data | `True` → Warn before calling |
| `idempotent` | `False` | Repeated calls have no additional side effects | `True` → Safe to retry |
| `requires_approval` | `False` | Requires human confirmation before execution. When an `ApprovalHandler` is configured on the Executor, this is enforced at runtime (Step 5). See [Approval System](../features/approval-system.md). | `True` → Seek consent |
| `open_world` | `True` | Connects to external systems | `True` → May be slow |
| `streaming` | `False` | Supports streaming chunk-by-chunk output | `True` → Read output incrementally |
| `cacheable` | `False` | Output can be cached for identical inputs | `True` → Reuse previous results |
| `cache_ttl` | `0` | Cache duration in seconds (0 = no cache) | Determines how long cached results remain valid |
| `cache_key_fields` | `None` | Input fields for cache key (None = all fields) | Determines cache hit criteria |
| `paginated` | `False` | Returns paginated results | `True` → Pass cursor/offset, expect partial results |
| `pagination_style` | `"cursor"` | `"cursor"`, `"offset"`, or `"page"` pagination | Determines pagination parameter format |
| `extra` | `{}` | Extension metadata for ecosystem packages | Custom key-value pairs for non-standard annotations |

```python
# Query module - read-only, safe
class GetUserModule(Module):
    """Query user information"""
    annotations = ModuleAnnotations(readonly=True, idempotent=True, open_world=False)

# Send module - has side effects, connects to external system
class SendEmailModule(Module):
    """Send email"""
    annotations = ModuleAnnotations(open_world=True)

# Delete module - destructive, requires confirmation
class DeleteUserModule(Module):
    """Delete user"""
    annotations = ModuleAnnotations(destructive=True, requires_approval=True, open_world=False)

# Cacheable query with 5-minute TTL
class GetProductModule(Module):
    """Get product details"""
    annotations = ModuleAnnotations(readonly=True, cacheable=True, cache_ttl=300, open_world=False)

# Paginated list endpoint
class ListOrdersModule(Module):
    """List user orders"""
    annotations = ModuleAnnotations(readonly=True, paginated=True, pagination_style="cursor")
```

### 3.5 examples

**Usage examples, help AI understand complex modules.**

> **SHOULD recommendation**: When a module's input_schema contains any of the following features, **SHOULD** provide at least one `ModuleExample`:
> - Uses `oneOf` / `anyOf` (Union types)
> - More than 5 required fields (`required` array length > 5)
> - Contains nested `object` (2+ levels)
>
> These features significantly increase the difficulty for AI to correctly construct inputs, examples can greatly improve AI invocation accuracy.

```python
from dataclasses import dataclass
from typing import Any


@dataclass
class ModuleExample:
    """Module usage example"""
    title: str                                # Example title
    inputs: dict[str, Any] = field(default_factory=dict)   # Example input
    output: dict[str, Any] = field(default_factory=dict)   # Example output
    description: str | None = None            # Description (optional)
```

```python
class SendEmailModule(Module):
    """Send email"""

    examples = [
        ModuleExample(
            title="Send plain text email",
            description="Send plain text email via SMTP",
            inputs={
                "to": "user@example.com",
                "subject": "Hello",
                "body": "World"
            },
            output={
                "success": True,
                "message_id": "msg_123"
            }
        ),
        ModuleExample(
            title="Send HTML email",
            description="Send HTML format email to multiple recipients via SMTP",
            inputs={
                "to": ["user1@example.com", "user2@example.com"],
                "subject": "Notification",
                "html": "<h1>Hello</h1>",
                "smtp_host": "smtp.example.com",
                "smtp_port": 587
            }
        )
    ]
```

**Mapping to AI Protocols:**

| Protocol | Mapped Field |
|------|---------|
| Anthropic | `input_examples` |
| A2A | `AgentSkill.examples` |
| MCP | Not directly supported, can be placed in `_meta` |

### 3.6 metadata

**Free-form extended metadata, framework does not validate content.**

```python
class SendEmailModule(Module):
    """Send email"""

    metadata = {
        # Performance & cost hints
        "x-cost-per-call": 0.001,
        "x-avg-latency-ms": 500,

        # Planning hints
        "x-preconditions": ["SMTP server must be configured"],
        "x-postconditions": ["Email queued for delivery"],
        "x-side-effects": ["Sends email via external SMTP server"],

        # Trust hints
        "x-output-source": "api",

        # Data sensitivity
        "data_sensitivity": ["PII"],

        # Operations info
        "owner": "email-team",
        "documentation_url": "https://docs.example.com/send-email"
    }
```

**Usage Scenarios:**

| Purpose | Example |
|------|------|
| Middleware reading | Rate limiting middleware reads `metadata["rate_limit"]` |
| Monitoring dashboard | Display `metadata["owner"]` |
| Documentation generation | Link to `metadata["documentation_url"]` |
| Other needs | When new standards emerge, test in metadata first |

---

## 4. Lifecycle Hooks

### 4.1 on_load()

```python
class DatabaseModule(Module):
    def on_load(self) -> None:
        """Called when module is loaded, used for resource initialization"""
        self.connection = create_db_connection()
        self.pool = create_connection_pool()

    def on_unload(self) -> None:
        """Called when module is unloaded, used for resource cleanup"""
        self.pool.close()
        self.connection.close()
```

### 4.2 on_suspend() / on_resume()

Optional hooks for preserving module state across hot-reload cycles:

```python
class CounterModule(Module):
    def on_load(self) -> None:
        self._count = 0
        self._cache = {}

    def on_suspend(self) -> dict | None:
        """Called before hot-reload. Return serializable state to preserve."""
        return {"count": self._count, "cache": self._cache}

    def on_resume(self, state: dict) -> None:
        """Called after hot-reload. Restore state from on_suspend()."""
        self._count = state.get("count", 0)
        self._cache = state.get("cache", {})

    def on_unload(self) -> None:
        pass
```

**Hot-Reload Lifecycle:**

```text
old_instance.on_suspend() → state     ← Export state
old_instance.on_unload()              ← Cleanup resources
  (reload module code from disk)
new_instance.__init__()
new_instance.on_load()                ← Initialize resources
new_instance.on_resume(state)         ← Restore state (only if state is not None)
```

**Constraints:**
- `on_suspend()` return value **must** be JSON-serializable
- `on_resume()` **must** tolerate missing or extra keys (version may differ)
- If either hook raises, the error is logged but does not block the reload process

**Normal Shutdown Lifecycle:**

```text
Registry.discover()
    ↓
Module class loading
    ↓
Module instantiation
    ↓
on_load() called         ← Initialize resources
    ↓
[Module available]
    ↓
on_unload() called       ← Cleanup resources (on application exit)
```

---

## 5. Optional Methods

### 5.1 validate()

**Validate input only, without execution.**

```python
class SendEmailModule(Module):
    def validate(self, inputs: dict[str, Any]) -> ValidationResult:
        """
        Validate input parameters

        Returns:
            ValidationResult: Validation result
        """
        errors = []

        # Custom validation logic
        if inputs.get("to", "").endswith("@blocked.com"):
            errors.append({
                "field": "to",
                "code": "BLOCKED_DOMAIN",
                "message": "This domain is blocked"
            })

        return ValidationResult(
            valid=len(errors) == 0,
            errors=errors
        )
```

**Purpose:**
- Pre-validate before execution
- Provide more friendly error messages
- Perform custom business validation

---

### 5.2 Async Execution

**Modules only need to define one `execute()` method, either `def` or `async def`.** The framework automatically detects the function type and selects the appropriate invocation method.

```python
# Sync module
class SyncModule(Module):
    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        """Synchronous execution"""
        result = self._do_work(inputs)
        return {"result": result}

# Async module
class AsyncModule(Module):
    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        """Asynchronous execution — framework auto-detects async def and uses async invocation"""
        async with aiohttp.ClientSession() as session:
            resp = await session.post("https://api.example.com", json=inputs)
            data = await resp.json()
            return {"result": data}
```

**Rules:**
- Modules **do not need** to define a separate `execute_async()` method
- Framework auto-detects via `inspect.iscoroutinefunction()` or language-equivalent mechanism
- Both `Executor.call()` and `Executor.call_async()` can correctly handle both types of modules

---

## 6. Complete Example

```python
from typing import Any, ClassVar, Type
from pydantic import BaseModel, Field
from apcore import Module, Context, ModuleError, ModuleAnnotations, ModuleExample


# ============ Schema Definitions ============

class SendEmailInput(BaseModel):
    """Send email input"""
    to: str = Field(..., description="Recipient email address", pattern=r"^[\w\.-]+@[\w\.-]+\.\w+$")
    subject: str = Field(..., description="Email subject", max_length=200)
    body: str = Field(..., description="Email body")
    cc: list[str] = Field(default=[], description="CC list")
    html: bool = Field(default=False, description="Whether HTML format")


class SendEmailOutput(BaseModel):
    """Send email output"""
    success: bool = Field(..., description="Whether sending was successful")
    message_id: str | None = Field(None, description="Message ID")
    error: str | None = Field(None, description="Error message")


# ============ Module Definition ============

class SendEmailModule(Module):
    """Send email module (Python docstring, for developers to read)

    This is the Python docstring, for developers to view in IDE.
    Can include more detailed technical implementation notes, source code comments, etc.
    """

    # Required definitions
    input_schema: ClassVar[Type[BaseModel]] = SendEmailInput
    output_schema: ClassVar[Type[BaseModel]] = SendEmailOutput
    description: ClassVar[str] = "Send email to specified recipient. Uses SMTP protocol, non-idempotent operation, requires email server configuration."

    # Optional: detailed documentation
    documentation: ClassVar[str] = """
# Features
Send email via SMTP protocol, supports plain text and HTML format.

## Configuration Requirements
- SMTP server information must be configured in apcore.yaml
- Valid SMTP authentication credentials must be provided

## Usage Scenarios
- Send notification emails, verification codes, reports

## Limitations
- Gmail: 500 emails/day
- Attachment size: ≤25MB
"""

    # Optional definitions
    name: ClassVar[str] = "Send Email"
    tags: ClassVar[list[str]] = ["email", "notification"]
    version: ClassVar[str] = "1.0.0"

    # Behavior annotations
    annotations = ModuleAnnotations(
        readonly=False,       # Has side effects (sends email)
        destructive=False,    # Non-destructive
        idempotent=False,     # Non-idempotent (each call sends one email)
        requires_approval=False,
        open_world=True       # Connects to external SMTP/API
    )

    # Usage examples
    examples = [
        ModuleExample(
            title="Send plain text email",
            inputs={
                "to": "user@example.com",
                "subject": "Hello",
                "body": "World"
            },
            output={"success": True, "message_id": "msg_123", "error": None}
        ),
        ModuleExample(
            title="Send HTML email",
            inputs={
                "to": "user@example.com",
                "subject": "Welcome",
                "body": "<h1>Hello</h1>",
                "html": True
            },
            output={"success": True, "message_id": "msg_456", "error": None}
        )
    ]

    # Extended metadata
    metadata = {
        "cost_per_call": 0.001,
        "avg_latency_ms": 500,
        "owner": "email-team"
    }

    # Instance attributes
    _smtp_client: Any = None

    def on_load(self) -> None:
        """Initialize SMTP client"""
        self._smtp_client = self._create_smtp_client()

    def on_unload(self) -> None:
        """Close SMTP client"""
        if self._smtp_client:
            self._smtp_client.close()

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        """Execute email sending"""
        params = SendEmailInput(**inputs)

        try:
            message_id = self._send(params)
            return {
                "success": True,
                "message_id": message_id,
                "error": None
            }
        except Exception as e:
            # Log error (using trace_id from context)
            self._log_error(context.trace_id, e)
            return {
                "success": False,
                "message_id": None,
                "error": str(e)
            }

    def _send(self, params: SendEmailInput) -> str:
        """Internal method: actual sending logic"""
        # ... implementation ...
        return "msg_123"

    def _create_smtp_client(self) -> Any:
        """Create SMTP client"""
        # ... implementation ...
        pass

    def _log_error(self, trace_id: str, error: Exception) -> None:
        """Log error"""
        # ... implementation ...
        pass
```

---

## 7. Interface Validation

apcore validates interface implementation correctness when loading modules:

```python
# Framework internal validation logic
def validate_module(module_class: Type[Module]) -> list[str]:
    errors = []

    # ============ Core Layer Validation (Required) ============

    # Check input_schema
    if not hasattr(module_class, 'input_schema'):
        errors.append("Missing input_schema")
    elif not issubclass(module_class.input_schema, BaseModel):
        errors.append("input_schema must be a subclass of BaseModel")

    # Check output_schema
    if not hasattr(module_class, 'output_schema'):
        errors.append("Missing output_schema")
    elif not issubclass(module_class.output_schema, BaseModel):
        errors.append("output_schema must be a subclass of BaseModel")

    # Check description
    description = getattr(module_class, 'description', None) or module_class.__doc__
    if not description:
        errors.append("Missing description (docstring or explicit definition)")
    elif len(description) > 200:
        errors.append("description exceeds 200 character limit")

    # Check documentation (optional)
    documentation = getattr(module_class, 'documentation', None)
    if documentation is not None and len(documentation) > 5000:
        errors.append("documentation exceeds 5000 character limit")

    # Check execute method
    if not hasattr(module_class, 'execute'):
        errors.append("Missing execute method")

    # ============ Annotation Layer Validation (Optional, type checking) ============

    annotations = getattr(module_class, 'annotations', None)
    if annotations is not None and not isinstance(annotations, ModuleAnnotations):
        errors.append("annotations must be of type ModuleAnnotations")

    examples = getattr(module_class, 'examples', [])
    if examples:
        for i, example in enumerate(examples):
            if not isinstance(example, ModuleExample):
                errors.append(f"examples[{i}] must be of type ModuleExample")
            elif not example.title or not example.inputs:
                errors.append(f"examples[{i}] must have title and inputs")

    metadata = getattr(module_class, 'metadata', {})
    if not isinstance(metadata, dict):
        errors.append("metadata must be of type dict")

    return errors
```

---

## 8. Function-based Module Interface

> In addition to the Class-based approach of inheriting the Module base class, apcore also supports function-based module definitions. See [PROTOCOL_SPEC §5.11](../../PROTOCOL_SPEC.md) for detailed specifications.

### 8.1 `module()` Unified Concept

`module()` is the single registration mechanism, supporting two forms:

- **Decorator form**: Suitable for new code or modifiable code
- **Function call form**: Suitable for unmodifiable existing code (class methods, third-party library functions, etc.)

Modules produced by both forms are completely equivalent to Class-based Modules in Registry/Executor/Schema behavior.

### 8.2 Parameter Signature

```python
# Decorator form
@module(
    id: str = None,              # Module ID (optional, auto-generated)
    description: str = None,     # Module description (optional, extracted from docstring)
    documentation: str = None,   # Detailed documentation (optional, ≤5000 characters, Markdown)
    annotations: ModuleAnnotations = None,  # Behavior annotations
    tags: list[str] = None,      # Tags
    version: str = "1.0.0",      # Version number
    metadata: dict = None        # Extended metadata
)
def my_function(...):
    ...

# Function call form
module(
    callable,                    # Target function or method
    id: str = None,
    description: str = None,
    documentation: str = None,   # Detailed documentation (optional, ≤5000 characters, Markdown)
    annotations: ModuleAnnotations = None,
    tags: list[str] = None,
    version: str = "1.0.0",
    metadata: dict = None
)
```

### 8.3 Equivalence Mapping with Class-based Module

| Class-based Attribute | Function-based Equivalent |
|------------------|-------------------|
| `input_schema` | Auto-generated from function parameter type annotations |
| `output_schema` | Auto-generated from return type annotation |
| `description` | Extracted from docstring first line, or `description` parameter |
| `documentation` | `documentation` parameter |
| `execute()` | Function (`def` or `async def`, framework auto-detects) |
| `name` | Generated from function name |
| `tags` | `tags` parameter |
| `version` | `version` parameter |
| `annotations` | `annotations` parameter |
| `metadata` | `metadata` parameter |
| `on_load()` / `on_unload()` | Not supported (function-based modules have no lifecycle hooks) |

### 8.4 Context Injection Mechanism

When a function parameter declares `context: Context`, the framework automatically injects the Context object:

```python
@module(id="email.send")
def send_email(to: str, subject: str, context: Context) -> dict:
    """Send email"""
    print(f"trace_id: {context.trace_id}")
    return {"success": True}
```

- The `context` parameter **will not** appear in the generated `input_schema`
- If the function doesn't need Context, the parameter can be omitted

### 8.5 Underlying Module Object Access

Functions registered via `module()` produce an underlying Module object, accessible via Registry:

```python
@module(id="email.send")
def send_email(to: str, subject: str) -> dict:
    """Send email"""
    return {"success": True}

# Get underlying Module object via Registry
registry = Registry()
mod = registry.get("email.send")

# Can access all standard Module attributes
print(mod.description)       # "Send email"
print(mod.input_schema)      # Auto-generated JSON Schema
print(mod.output_schema)     # Auto-generated JSON Schema
```

### 8.6 Interface Contract Summary

| Interface | Level | Contract |
|------|------|------|
| `module()` parameter type annotations | **MUST** | All parameters **must** have type annotations |
| Return type annotation | **MUST** | Function **must** have return type annotation |
| `id` parameter | **MAY** | Optional; auto-generated from function path if not provided |
| `description` | **MAY** | Optional; extracted from docstring if not provided |
| Context injection | **MAY** | Optional; auto-injected when `context: Context` parameter declared |
| Sync/async | **MUST** | Both `def` and `async def` map to `execute()`, framework auto-detects and selects invocation method |
| Schema equivalence | **MUST** | Generated Schema equivalent to Class-based definition |

---

## Next Steps

- [Creating Modules Guide](../guides/creating-modules.md) - Complete module creation tutorial
- [Context Object](./context-object.md) - Detailed explanation of Context
- [Registry API](./registry-api.md) - Module registration and discovery
