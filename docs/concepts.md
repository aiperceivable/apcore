# apcore — AI-Perceivable Core Concepts

> Understanding apcore's design philosophy, technical architecture, and the AI Collaboration Lifecycle.

## 1. Design Philosophy

### 1.1 The Concept: Cognitive Interface

Traditional software provides **UI** for humans and **API** for programs. apcore defines the **Cognitive Interface** for AI Agents.
- **Intent-Oriented**: AI thinks in terms of "What to do" (Intents), not "Which endpoint to call".
- **Strict Contracts**: Mandatory schemas ensure AI uses your tools correctly every time.
- **Behavioral Personality**: Annotations like `readonly`, `destructive`, and `requires_approval` give Agents the guardrails they need to make safe decisions.
- **Self-Healing**: Error guidance (`ai_guidance`) tells the Agent exactly how to fix issues and retry. Self-Healing serves two higher-level goals:
  - **Self-Repair**: The Agent autonomously corrects errors and retries within a single interaction.
  - **Self-Evolution**: The system continuously adapts through health monitoring, event-driven feedback loops, and runtime reconfiguration.

### 1.2 The AI Collaboration Lifecycle

apcore organizes module metadata into a coherent lifecycle that guides an Agent through every stage of a task:

1.  **Discovery (Identity) — `description`**: Helps the Agent find the right tool for its intent.
2.  **Strategy (Wisdom) — `metadata`**: Teaches the Agent *when* and *how* to use the tool correctly (e.g., `x-when-to-use`, `x-common-mistakes`).
3.  **Governance (Safety) — `requires_approval`**: Sets the safety boundary for sensitive operations.
4.  **Recovery (Resilience) — error guidance fields**: Every `ModuleError` carries optional fields that enable AI agents to recover autonomously: `retryable` (can the call be retried?), `ai_guidance` (machine-readable recovery hint), `user_fixable` (can the end-user fix it?), and `suggestion` (actionable fix). See PROTOCOL_SPEC §8.1.1.

Together with health sensing (`system.health.*`) and the control plane (`system.control.*`), the Recovery stage completes the Self-Healing loop — powering both **Self-Repair** and **Self-Evolution** (see §1.1).

---

### 1.3 Why Do We Need apcore?

The growing number of fragmented MCP implementations across the ecosystem proves the demand is real. apcore is the only solution that provides a **complete SDK** with a **unified standard** — enforced schema, behavioral annotations, access control, audit trails, and cross-language consistency. It doesn't replace any project's AI capabilities; it brings them all under one standard.

**Problems with traditional module development:**

```python
# Traditional way: code can be called, but AI can't understand
def send_email(to, subject, body, cc=None):
    """Send email"""
    # ... implementation ...
```

- AI/LLM cannot know parameter constraints (to's email address, subject's content)
- Cannot automatically validate inputs/outputs
- Cannot auto-discover and register

**apcore's solution:**

```python
# apcore way: code can be called, and AI can understand
class SendEmailModule(Module):
    """Send email - AI understands functionality through docstring"""

    input_schema = SendEmailInput    # AI understands inputs through Schema
    output_schema = SendEmailOutput  # AI understands outputs through Schema

    def execute(self, inputs, context):
        # ... implementation ...
```

**Bridging existing code with `module()`:**

For existing functions or methods, no need to rewrite as Class-based Module, use `module()` to wrap as standard module:

```python
from apcore import module

# Method 1: @module decorator (low intrusion)
@module(id="email.send")
def send_email(to: str, subject: str, body: str) -> dict:
    """Send email"""
    return {"success": True}

# Method 2: function call (zero intrusion, no source code change)
module(existing_service.send, id="email.send")
```

Schema is auto-generated from type annotations, existing code logic remains unchanged. You can also use YAML binding files for completely zero-code-modification integration (see [Creating Modules Guide](./guides/creating-modules.md#external-schema-binding-yaml)).

### 1.4 Core Design Principles

| Principle | Description |
|------|------|
| **Schema Mandatory** | All modules must define input_schema / output_schema / description |
| **Directory as ID** | File paths automatically become module IDs, zero-config |
| **Convention over Configuration** | Works by following conventions, no tedious configuration |
| **AI-Perceivable** | Designed from the start to be AI-perceivable and understandable |
| **Universal Standard** | Not just AI framework, but a universal module standard that is naturally AI-Perceivable |

---

## 2. Core Concepts

### 2.1 Module

> 📖 **Reference** - Complete definition in [Module Interface](./api/module-interface.md)

**Module is the basic unit of apcore.** Each Module represents a callable functional unit.

```python
class MyModule(Module):
    """Module description - must provide, AI uses it to understand functionality"""

    input_schema = MyInput      # Input Schema - required
    output_schema = MyOutput    # Output Schema - required

    def execute(self, inputs: dict, context: Context) -> dict:
        """Execute module logic"""
        # inputs already validated by Schema
        # return value will be validated by output_schema
        return {"result": "..."}
```

**Module required components:**

| Component | Type | Description |
|------|------|------|
| `description` | str | Module functionality description (from docstring or explicit definition) |
| `input_schema` | Schema | Input parameter Schema definition |
| `output_schema` | Schema | Output result Schema definition |
| `execute()` | method | Execution logic |

**Module optional components:**

| Component | Type | Description |
|------|------|------|
| `name` | str | Human-readable name (defaults from class name) |
| `tags` | list[str] | Tags for categorization and search |
| `version` | str | Module version |
| `documentation` | str | Detailed documentation (≤5000 chars, supports Markdown) |
| `annotations` | ModuleAnnotations | Behavior annotations to help AI make call decisions |
| `examples` | list[ModuleExample] | Usage examples to help AI understand complex modules |
| `metadata` | dict[str, Any] | Free extension metadata |
| `on_load()` | method | Load hook |
| `on_unload()` | method | Unload hook |
| `on_suspend()` | method | Hot-reload state export hook |
| `on_resume()` | method | Hot-reload state restore hook |

---

### 2.2 Schema

**Schema defines data structure and constraints.** apcore uses Schema to achieve:

- Input parameter validation
- Output result validation
- AI/LLM understanding of module inputs/outputs

**Schema definition methods:**

> 💡 Complete examples in [Schema Definition Guide](./guides/schema-definition.md)

```python
# Method 1: Pydantic Model (recommended, Python)
from pydantic import BaseModel, Field

class SendEmailInput(BaseModel):
    to: str = Field(..., description="Recipient email")
    subject: str = Field(..., description="Email subject")
    body: str = Field(..., description="Email body")

class SendEmailOutput(BaseModel):
    success: bool = Field(..., description="Whether successful")
    message_id: str | None = Field(None, description="Email ID")
```

```yaml
# Method 2: YAML Schema (cross-language sharing)
# schemas/executor/email/send_email.schema.yaml

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
```

**Schema LLM extension fields:**

| Field | Description |
|------|------|
| `description` | Field description (standard JSON Schema) |
| `x-llm-description` | LLM-oriented detailed description |
| `x-examples` | Example values to help LLM understand |
| `x-sensitive` | Sensitive field marker (passwords, etc.) |

---

### 2.3 Annotations (Behavior Annotations)

> 📖 **Reference** - Complete definition in [ModuleAnnotations API](./api/module-interface.md#34-annotations)

**Annotations are module-level behavior metadata**, helping AI understand module characteristics.

**Design philosophy**:
- This is a "hint" for AI, not mandatory constraint
- Helps AI decide if human confirmation needed
- Aligns with MCP ToolAnnotations

**Usage example**:

```python
# Read-only query - AI can call safely
annotations = ModuleAnnotations(readonly=True)

# Delete operation - AI needs confirmation
annotations = ModuleAnnotations(destructive=True, requires_approval=True)
```

See [API documentation](./api/module-interface.md#34-annotations) for detailed field descriptions.

**How AI uses Annotations:**

| Annotation | AI Behavior |
|------|---------|
| `readonly=True` | Safe call, no confirmation needed |
| `destructive=True` | Warn user before calling |
| `idempotent=True` | Safe to retry on failure |
| `requires_approval=True` | Must get user consent |
| `open_world=True` | Knows this call involves external systems, may be slow |

**Real usage examples:**

```python
class SendEmailModule(Module):
    """Send email"""
    input_schema = SendEmailInputSchema
    output_schema = SendEmailOutputSchema

    # AI knows: has side effects, not idempotent, connects to external system
    annotations = ModuleAnnotations(
        readonly=False,
        destructive=False,
        idempotent=False,
        requires_approval=False,
        open_world=True
    )


class DeleteUserModule(Module):
    """Delete user account"""
    input_schema = DeleteUserInput
    output_schema = DeleteUserOutput

    # AI knows: destructive operation, needs human confirmation
    annotations = ModuleAnnotations(
        readonly=False,
        destructive=True,
        idempotent=True,
        requires_approval=True,
        open_world=False
    )


class GetUserInfoModule(Module):
    """Query user info"""
    input_schema = GetUserInput
    output_schema = GetUserOutput

    # AI knows: read-only query, safe to call
    annotations = ModuleAnnotations(
        readonly=True,
        idempotent=True,
        open_world=False
    )
```

**Correspondence with MCP ToolAnnotations:**

| apcore | MCP | Description |
|--------|-----|------|
| `readonly` | `readOnlyHint` | Whether read-only |
| `destructive` | `destructiveHint` | Whether destructive |
| `idempotent` | `idempotentHint` | Whether idempotent |
| `open_world` | `openWorldHint` | Whether involves external systems |
| `requires_approval` | — | apcore unique, OpenAI SDK also has similar field |

---

### 2.4 Examples (Usage Examples)

> 📖 **Reference** - Complete definition in [ModuleExample API](./api/module-interface.md#35-examples)

**Examples provide concrete input/output examples**, helping AI more accurately understand complex modules.

**Usage example:**

```python
class SendEmailModule(Module):
    """Send email"""
    input_schema = SendEmailInputSchema
    output_schema = SendEmailOutputSchema

    examples = [
        ModuleExample(
            title="Send plain text email",
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
            },
            output={
                "success": True,
                "message_id": "msg_456"
            }
        )
    ]
```

**Value of Examples:**

| Scenario | Without Examples | With Examples |
|------|-------------|------------|
| Complex Union types | LLM might pass wrong type | LLM sees examples and knows how to pass |
| Multi-Provider modules | LLM doesn't know which params different providers need | One example per provider |
| Export as Anthropic Tool | No input_examples | Auto-mapped to input_examples |
| Documentation generation | Only abstract description | Has concrete use cases |

---

### 2.5 Metadata (Extension Metadata)

**Metadata is completely open dict**, used to store extension information not belonging to core or annotation layers.

```python
class SendEmailModule(Module):
    """Send email"""
    input_schema = SendEmailInputSchema
    output_schema = SendEmailOutputSchema

    metadata = {
        # AI intent hints
        "x-when-to-use": "Use for transactional emails (order confirmation, password reset)",
        "x-when-not-to-use": "Do not use for bulk/marketing emails",

        # AI planning hints
        "x-preconditions": ["SMTP server must be configured", "Recipient must be validated"],
        "x-postconditions": ["Email queued for delivery"],
        "x-side-effects": ["Sends email via external SMTP server"],

        # Performance & cost hints
        "x-cost-per-call": 0.001,
        "x-avg-latency-ms": 500,

        # Trust hints
        "x-output-source": "api",

        # Operations info
        "data_sensitivity": ["PII"],
        "owner": "email-team",
    }
```

**Three-layer metadata summary:**

```
┌──────────────────────────────────────────────────────┐
│  Core Layer (Required)                               │
│  input_schema / output_schema / description          │
│  → AI understands module "what it does"              │
│                                                      │
│  + documentation (optional, detailed docs)           │
│  → AI understands "detailed use cases and constraints"│
├──────────────────────────────────────────────────────┤
│  Annotation Layer (optional, type-safe)              │
│  annotations / examples / name / tags / version      │
│  cacheable / paginated / streaming                   │
│  → AI understands module "how to use"                │
├──────────────────────────────────────────────────────┤
│  Extension Layer (optional, free dict)               │
│  metadata: dict[str, Any]                            │
│  → AI intent (x-when-to-use), planning              │
│    (x-preconditions), cost (x-cost-per-call),        │
│    trust (x-output-source), and custom fields        │
└──────────────────────────────────────────────────────┘
```

**Design significance:**

| Layer | Characteristics | Change Frequency |
|----|------|---------|
| Core Layer | Mandatory, all modules must have | Rarely changes |
| Annotation Layer | Optional but type-safe, framework understands | Occasionally add fields |
| Extension Layer | Completely free, framework doesn't interpret | Can extend anytime |

**Recommended AI Metadata Conventions** (PROTOCOL_SPEC §4.6):

The following `x-*` keys are conventions for guiding AI agents. They are not enforced by the framework but provide tactical wisdom in the Extension Layer:

| Key | Purpose | Example |
|-----|---------|---------|
| `x-when-to-use` | Scenarios where this module is the right choice | `"Use when the user wants to send transactional email"` |
| `x-when-not-to-use` | Scenarios where a different module should be used | `"Do not use for marketing/bulk emails"` |
| `x-common-mistakes` | Known pitfalls AI agents frequently encounter | `"Forgetting to set reply_to; passing HTML in plain text field"` |
| `x-workflow-hints` | Suggested pre/post steps or related modules | `"Call validate-email first to verify recipient exists"` |
| `x-preconditions` | What must be true before calling this module | `"User must be authenticated"` |
| `x-postconditions` | What will be true after successful execution | `"Email is queued for delivery"` |
| `x-side-effects` | External state changes caused by this module | `"Sends email, logs to audit trail"` |
| `x-cost-per-call` | Estimated cost per invocation | `0.001` |
| `x-avg-latency-ms` | Average execution latency in milliseconds | `500` |
| `x-max-latency-ms` | Maximum expected latency in milliseconds | `5000` |
| `x-sla` | SLA targets (availability, latency percentiles) | `{"availability": "99.9%", "p99_latency_ms": 5000}` |
| `x-output-source` | Data provenance | `"database"` |
| `x-verification-hint` | How to cross-check the output for correctness | `"Cross-check amounts with accounting.get_balance"` |

```yaml
# In module metadata or schema x-* fields
metadata:
  x-when-to-use: "Use when the user wants to send transactional email"
  x-when-not-to-use: "Do not use for marketing/bulk emails"
  x-common-mistakes: "Forgetting to set reply_to; passing HTML in plain text field"
  x-workflow-hints: "Call validate-email first to verify recipient exists"
```

---

### 2.6 Module ID

**Module ID is automatically generated from directory path.** This is one of apcore's core designs.

```
File Path                                   Module ID
─────────────────────────────────────────────────────────
extensions/executor/email/send_email.py    →   executor.email.send_email
extensions/api/handler/user_create.py      →   api.handler.user_create
extensions/common/util/validator.py        →   common.util.validator
```

**ID generation rules:**

1. Remove `extensions/` prefix
2. Remove file extension
3. Path separator `/` becomes `.`

**ID format constraints:**

```yaml
format: "^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)*$"
separator: "."
case: "snake_case"
max_length: 192
```

**Valid examples:**
- `executor.email.send_email`
- `api.handler.user_create`
- `common.util.string_validator`

**Invalid examples:**
- `Executor.Email.SendEmail` ❌ No uppercase
- `executor-email-send_email` ❌ No hyphens
- `executor/email/send_email` ❌ No slashes

---

### 2.7 ID Map (Cross-Language ID Mapping)

**ID Map handles cross-language ID conversion.** Different languages have different naming conventions, ID Map automatically handles conversion.

```yaml
# Module ID (unified)
executor.email.send_email

# Local representation in each language
Python:     executor/email/send_email.py      class SendEmailModule
Rust:       executor/email/send_email.rs       struct SendEmailModule
Go:         executor/email/send_email.go       type SendEmailModule
Java:       executor/email/SendEmail.java     class SendEmailModule
TypeScript: executor/email/sendEmail.ts        class SendEmailModule
```

**ID Map configuration (special case overrides):**

```yaml
# apcore.yaml
id_map:
  auto_detect: true  # Auto-detect language by file extension

  # Special mappings (override auto rules)
  overrides:
    "executor.email.send_email":
      java:
        class: "com.mycompany.email.SendEmailModule"
```

---

### 2.8 Registry

> 📖 **Reference** - Complete definition in [Registry API](./api/registry-api.md)

**Registry is responsible for module discovery, registration, and management.**

```python
from apcore import Registry

# Create Registry (single root directory, backward compatible)
registry = Registry(extensions_dir="./extensions")

# Or: multi-root directory mode (namespace isolation)
# registry = Registry(extensions_dirs=["./extensions", "./plugins"])

# Or: no bound directory, manual registration only
# registry = Registry(extensions_dir=None)

# Auto-discover all modules
registry.discover()

# Get module
module = registry.get("executor.email.send_email")

# Get module definition descriptor (cross-language compatible, replaces get_class)
definition = registry.get_definition("executor.email.send_email")

# Get structured Schema (dict) for programmatic processing
schema = registry.get_schema("executor.email.send_email")

# Export serialized Schema (str) for transmission/storage
schema_json = registry.export_schema("executor.email.send_email", format="json")

# List all modules
all_modules = registry.list()

# Filter by tags
email_modules = registry.list(tags=["email"])
```

**Registry responsibilities:**

| Responsibility | Description |
|------|------|
| **Discovery** | Scan extensions directory, discover all modules |
| **Registration** | Register modules to internal mapping table |
| **Loading** | Lazy load modules (load on first use) |
| **Query** | Query modules by ID, tags, etc. |
| **Validation** | Verify modules conform to spec |
| **Events** | Support register/unregister event callbacks (`on("register", ...)` / `on("unregister", ...)`) |

---

### 2.9 Executor

**Executor is responsible for module invocation and execution.**

```python
from apcore import Executor

executor = Executor(registry)

# Call module
result = executor.call(
    module_id="executor.email.send_email",
    inputs={
        "to": "user@example.com",
        "subject": "Hello",
        "body": "World"
    },
    context=context  # Context object (optional, auto-created)
)
```

**Executor responsibilities:**

| Responsibility | Description |
|------|------|
| **Lookup module** | Get module from Registry, error if not exists |
| **Permission check** | Check call permissions per ACL |
| **Input validation** | Validate input per input_schema |
| **Execution** | Call module's execute() method (framework auto-detects sync/async) |
| **Output validation** | Validate output per output_schema |
| **Redaction** | Automatically provide `context.redacted_inputs` for middleware and logging |
| **Tracing** | Record call tracing info |
| **Error handling** | Unified error handling and wrapping |

> **Sync/async unification**: Modules only need to define one `execute()` method (`def` or `async def`), framework auto-detects and selects appropriate call method. No need to define both `execute()` and `execute_async()`.

**Execution flow:**

```
Input → Lookup module → ACL check → Input validation → Middleware(before) → execute() → Output validation → Middleware(after) → Output
          ↓          ↓          ↓            ↓              ↓            ↓             ↓
       Not found  Denied    Failed       Can modify     Business       Failed      Can modify
          ↓          ↓          ↓            ↓              ↓            ↓             ↓
       Exception  Exception  Exception    Continue      Return        Exception    Continue
```

---

### 2.10 Context

> 📖 **Reference** - Complete definition in [Context Object](./api/context-object.md)

**Context carries context information for each call.**

**Design principle**: Only fields that framework execution engine depends on are independent fields, everything else goes into `data`.

**Core fields**:

- **Framework engine dependencies**: `trace_id`, `caller_id`, `call_chain`, `executor`
- **Identity and security**: `identity`, `redacted_inputs`, `logger` (property)
- **Shared state**: `data` (reference-passed pipeline state)

**Field classification**:

| Category | Fields | Rationale |
|------|------|------|
| Framework engine dependencies | `trace_id`, `caller_id`, `call_chain`, `executor` | Framework can't work without any one |
| Universally needed | `identity` | ACL is framework first-class citizen, needs standard "who" |
| Universal bag | `data` | span_id, locale, pipeline intermediate state and other mutable data |

**Context propagation:**

```python
class ModuleA(Module):
    def execute(self, inputs, context):
        # context auto-propagates, data reference-shared
        result = context.executor.call(
            "module.b",
            inputs={"...": "..."},
            context=context
        )
        # call_chain auto-updates, data read/write along chain
```

**Pipeline usage of data (AI orchestration scenario):**

```python
# AI orchestrator initiates multi-step call
context.data["task_info"] = {"type": "report", "date": "2024-01"}

result_a = executor.call("module_a", inputs={...}, context=context)
# module_a writes context.data["raw_records"] = [...]

result_b = executor.call("module_b", inputs={...}, context=context)
# module_b reads context.data["raw_records"], writes context.data["analysis"]
```

---

### 2.11 ACL (Access Control)

**ACL defines call permissions between modules.**

```yaml
# acl/global_acl.yaml

rules:
  # API layer can only call Orchestrator layer
  - callers: ["api.*"]
    targets: ["orchestrator.*"]
    effect: allow

  # Orchestrator layer can call Executor layer
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow

  # Prohibit Executor layer calling API layer
  - callers: ["executor.*"]
    targets: ["api.*"]
    effect: deny

# Default policy
default_effect: deny
```

**ACL rule matching:**

| Pattern | Description |
|------|------|
| `api.*` | Matches all modules under api |
| `executor.email.*` | Matches all modules under executor.email |
| `*.validator.*` | Matches validator in any layer |
| `executor.email.send_email` | Exact match |

---

### 2.12 Middleware

**Middleware can intercept and process module calls.**

```python
from apcore import Middleware

class LoggingMiddleware(Middleware):
    """Logging middleware"""

    def before(self, module_id: str, inputs: dict, context: Context):
        """Before execution"""
        print(f"[{context.trace_id}] Calling {module_id}")
        return inputs  # Can modify inputs

    def after(self, module_id: str, inputs: dict, output: dict, context: Context):
        """After execution"""
        print(f"[{context.trace_id}] {module_id} returned")
        return output  # Can modify output

    def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context):
        """On error"""
        print(f"[{context.trace_id}] {module_id} failed: {error}")
        # Return None to continue error propagation
        # Return a dict to stop propagation and use it as recovery output
        return None
```

**Middleware registration:**

```yaml
# apcore.yaml
middleware:
  - id: "logging"
    class: "myapp.middleware.LoggingMiddleware"
    priority: 100

  - id: "tracing"
    class: "apcore.middleware.TracingMiddleware"
    priority: 90
```

**Execution order (onion model):**

```
Request → [Middleware A before] → [Middleware B before] → execute()
                                                          ↓
Response ← [Middleware A after]  ← [Middleware B after]  ← Result
```

---

### 2.13 Observability

**apcore has built-in observability support: tracing, logging, metrics.**

#### Tracing

```python
# Auto-generated tracing info
{
    "trace_id": "550e8400-e29b-41d4-a716-446655440000",
    "span_id": "1234567890abcdef",
    "parent_span_id": "0987654321fedcba",
    "module_id": "executor.email.send_email",
    "method": "execute",
    "duration_ms": 123,
    "status": "success"
}
```

#### Logging

```python
# Structured logging
{
    "timestamp": "2026-02-05T10:30:00Z",
    "level": "info",
    "message": "Module executed",
    "trace_id": "550e8400-e29b-41d4-a716-446655440000",
    "module_id": "executor.email.send_email",
    "duration_ms": 123
}
```

#### Metrics

```yaml
# Auto-collected metrics
apcore_module_calls_total{module_id="executor.email.send_email", status="success"} 100
apcore_module_duration_seconds{module_id="executor.email.send_email"} 0.123
apcore_module_errors_total{module_id="executor.email.send_email", error_code="VALIDATION_ERROR"} 5
```

---

## 3. Concept Relationship Diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    apcore — AI-Perceivable Core                          │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   ┌─────────────┐      ┌─────────────┐      ┌─────────────┐            │
│   │   Module    │      │   Schema    │      │   ID Map    │            │
│   │             │◄────►│             │      │             │            │
│   │ - execute() │      │ - input     │      │ - Cross-    │            │
│   │ - schema    │      │ - output    │      │   language  │            │
│   └──────┬──────┘      └─────────────┘      └─────────────┘            │
│          │                                                              │
│          │ Register                                                     │
│          ▼                                                              │
│   ┌─────────────┐      ┌─────────────┐      ┌─────────────┐            │
│   │  Registry   │─────►│  Executor   │─────►│   Context   │            │
│   │             │      │             │      │             │            │
│   │ - discover  │      │ - call      │      │ - trace_id  │            │
│   │ - get       │      │ - validate  │      │ - caller_id │            │
│   └─────────────┘      └──────┬──────┘      └─────────────┘            │
│                               │                                         │
│                               │ Intercept                               │
│                               ▼                                         │
│   ┌─────────────┐      ┌─────────────┐      ┌─────────────┐            │
│   │     ACL     │      │ Middleware  │      │Observability│            │
│   │             │      │             │      │             │            │
│   │ - Permission│      │ - before    │      │ - tracing   │            │
│   │ - Rule match│      │ - after     │      │ - logging   │            │
│   └─────────────┘      └─────────────┘      │ - metrics   │            │
│                                             └─────────────┘            │
└─────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Formal Concept Definitions

| Concept | Formal Definition | Invariant |
|------|-----------|--------|
| Module | `M = (id, input_schema, output_schema, description, execute)` | `∀ inputs: validate(inputs, input_schema) ⟹ validate(execute(inputs, ctx), output_schema)` |
| Schema | `S = JSON Schema Draft 2020-12 + x-* extensions` | `∀ data: validate(data, S) → {true, false}` |
| Canonical ID | `ID = segment ("." segment)*` where `segment = [a-z][a-z0-9_]*` | `len(ID) ≤ 192 ∧ ∀ segment ∉ reserved_words` |
| Registry | `R = Map<CanonicalID, Module>` | `∀ id ∈ R: is_valid_canonical_id(id) ∧ validate_module(R[id])` |
| Executor | `E = (R, ACL, [Middleware])` | `E.call(id, inputs, ctx) ⟹ ACL.check(ctx.caller, id) ∧ Schema.validate(inputs)` |
| Context | `C = (trace_id, caller_id, call_chain, executor, identity, data)` | `trace_id ≠ null ∧ len(call_chain) ≤ 32` |
| ACL | `ACL = ([Rule], default_effect)` | `evaluate(caller, target) → {allow, deny}` |

### 3.2 Concept Relationship Matrix

| | Module | Schema | Registry | Executor | Context | ACL | Middleware |
|---|--------|--------|----------|----------|---------|-----|-----------|
| **Module** | — | Defines | Registered in | Called by | Receives | Controlled by | Intercepted by |
| **Schema** | Belongs to | — | Loaded by | Used for validation | — | — | — |
| **Registry** | Manages | Loads | — | Provides modules | — | — | — |
| **Executor** | Calls | Validates | Queries | — | Creates/passes | Checks | Schedules |
| **Context** | Passed to | — | — | References | — | Carries identity | Readable/writable |
| **ACL** | Protects | — | — | Queried by | Reads identity | — | — |
| **Middleware** | Intercepts | — | — | Managed by | Can modify data | — | Chain calls |

### 3.3 System Invariant Declarations

These invariants **must** always hold during system runtime:

1. **ID Uniqueness**: `∀ m1, m2 ∈ Registry: m1.id ≠ m2.id`
2. **Schema Integrity**: `∀ m ∈ Registry: m.input_schema ≠ null ∧ m.output_schema ≠ null`
3. **Call Chain Acyclic**: `∀ ctx: ¬∃ cycle in ctx.call_chain`
4. **Call Depth Bounded**: `∀ ctx: len(ctx.call_chain) ≤ MAX_DEPTH (32)`
5. **trace_id Propagation**: `∀ child_ctx derived from parent_ctx: child_ctx.trace_id == parent_ctx.trace_id`
6. **ACL Consistency**: `∀ call: ACL.check(caller, target) result is deterministic under same ruleset`
7. **Schema Validation Idempotent**: `∀ data, schema: validate(data, schema) multiple calls yield consistent result`

---

## 4. Typical Usage Flow

```
1. Define Schema
   └── Define input/output Schema (SDK model class or YAML)

2. Create Module
   └── Inherit Module class, define execute() method

3. Place file
   └── Place file according to directory structure, ID auto-generated

4. Configure ACL (optional)
   └── Define call permissions between modules

5. Add Middleware (optional)
   └── Add logging, tracing, etc. middleware

6. Call execution
   └── Call module through Executor
```

---

## Next Steps

- [Creating Modules Guide](./guides/creating-modules.md) - Detailed module creation tutorial
- [Schema Definition Deep Dive](./guides/schema-definition.md) - Complete Schema usage
- [Module Interface Definition](./api/module-interface.md) - API reference
