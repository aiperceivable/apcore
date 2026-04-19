# Context Object

> **Canonical Definition** - This document is the authoritative definition of Context and Identity

> Context is the context object during module execution, carrying trace information, call chain, caller identity, and shared pipeline state.

## 1. Design Principles

**Only fields that the framework execution engine depends on are independent fields; everything else goes into `data`.**

Referring to industry practices: Go `context.Context` (3 independent fields + Value bag), OpenTelemetry Context (pure KV bag), AutoGen (`context_variables` shared dict). The apcore Context has 5 independent fields, each with a framework-level reason.

| Category | Field | Reason |
|------|------|------|
| Framework engine dependency | `trace_id`, `caller_id`, `call_chain`, `executor` | Removing any one would break the framework |
| Commonly needed | `identity` | ACL is a first-class framework citizen, needs standardized "who" |
| Generic bag | `data` | span_id, locale, pipeline intermediate state, and all mutable data |

---

## 2. Interface Definition

```python
from dataclasses import dataclass, field
from typing import Any, TYPE_CHECKING

if TYPE_CHECKING:
    from apcore import Executor


@dataclass(frozen=True)
class Identity:
    """Caller identity (human/service/AI generic)"""

    # Unique identifier (required)
    id: str

    # Identity type
    type: str = "user"  # user | service | agent | api_key | system

    # Role list (ACL engine dependency)
    # Implementations SHOULD use immutable collection types (e.g. Python tuple, TypeScript readonly array)
    roles: tuple[str, ...] = field(default_factory=tuple)

    # Extended attributes (business fields like tenant_id, email)
    attrs: dict[str, Any] = field(default_factory=dict)


@dataclass
class Context:
    """Module execution context"""

    # ====== Framework engine dependency (breaks without it) ======

    # Trace ID (required)
    trace_id: str

    # Caller module ID (optional, None for top-level calls)
    caller_id: str | None = None

    # Call chain (list of module IDs from root to current)
    call_chain: list[str] = field(default_factory=list)

    # Executor reference (for calling other modules)
    executor: "Executor | None" = None

    # ====== Almost all users need ======

    # Caller identity (ACL engine dependency)
    identity: Identity | None = None

    # ── SHOULD-level members (optional for implementations) ──────────

    # Context-aware logger (automatically injects trace_id, module_id, caller_id)
    # Level: MUST. Implementations provide this via the Context itself or a standalone ContextLogger class.
    @property
    def logger(self) -> "ContextLogger":
        """Returns a logger that automatically injects context information"""
        ...

    # Redacted data automatically provided by Executor (x-sensitive fields replaced with "***REDACTED***")
    redacted_inputs: dict[str, Any] | None = None

    # ====== Optional extension fields (MAY be provided by implementations) ======

    # Cooperative cancellation token for long-running operations
    cancel_token: CancelToken | None = None

    # Dependency injection container for sharing services across the call chain
    services: T | None = None

    # ====== Internal fields (set by Executor, not for module use) ======

    # Epoch timestamp for execution timeout (set by Executor based on global_timeout_ms config)
    # Modules should not set this directly.
    _global_deadline: float | None = None

    # ====== Everything else ======

    # Shared pipeline state (passed by reference, readable/writable along the call chain)
    data: dict[str, Any] = field(default_factory=dict)
```

#### `data` Key Conventions

`context.data` is a shared mutable dictionary. To prevent key collisions between framework internals and module code, a single reserved prefix convention applies:

| Prefix | Reserved For | Example |
|------|------|------|
| `_apcore.` | All framework internals | See hierarchy below |
| (no prefix) | Application / Module use | `user_session_id`, `pipeline_result` |

**Hierarchy under `_apcore.`:**

| Key Pattern | Subsystem | Example |
|------|------|------|
| `_apcore.mw.{middleware}.{field}` | Middleware private state | `_apcore.mw.logging.start_time` |
| `_apcore.mw.{middleware}.{field}.{id}` | Per-module middleware state | `_apcore.mw.retry.count.mod.a` |
| `_apcore.executor.{field}` | Executor internals | `_apcore.executor.global_deadline` |

- All framework components (middleware, executor, registry) **MUST** use `_apcore.` prefixed keys.
- Module developers **MUST NOT** use `_apcore.` prefixed keys.

### 2.1 Field Constraint Table

| Field | Type | Required | Max Length/Depth | Thread Safe | Serializable |
|------|------|------|--------------|---------|---------|
| `trace_id` | string (32-char hex) | **MUST** | 32 chars | Read-only, safe | **MUST** |
| `caller_id` | string \| null | **MUST** | 128 chars | Read-only, safe | **MUST** |
| `call_chain` | list[string] | **MUST** | Max depth 32 | Read-only, safe | **MUST** |
| `executor` | Executor | **MUST** | — | Thread-safe | **MUST NOT** |
| `identity` | Identity \| null | **SHOULD** | — | Read-only, safe | **MUST** |
| `logger` | ContextLogger | **SHOULD** | — | Thread-safe | **MUST NOT** |
| `redacted_inputs` | dict \| null | **SHOULD** | — | Read-only, safe | **MAY** |
| `cancel_token` | CancelToken \| null | **MAY** | — | Thread-safe | **MUST NOT** |
| `services` | T \| null | **MAY** | — | Read-only, safe | **MUST NOT** |
| `data` | dict[str, Any] | **MUST** | — | Not thread-safe | **SHOULD** |

**call_chain depth limit:** Implementations **MUST** reject new module calls when `call_chain` length reaches 32, throwing a `CALL_DEPTH_EXCEEDED` error.

**call_chain cycle detection:** Implementations **MUST** reject calls when the target module ID already exists in `call_chain`, throwing a `CIRCULAR_CALL` error.

**call_chain frequency limit:** Implementations **MUST** reject new calls when the same module ID appears in `call_chain` more than `max_module_repeat` times (default 3), throwing a `CALL_FREQUENCY_EXCEEDED` error. This detection prevents non-strict cycle loop patterns in AI orchestration scenarios (like A→B→C→B→C→B→C...), intercepting before depth limit is triggered. `max_module_repeat` can be configured via `apcore.yaml` at `executor.max_module_repeat`.

**Concurrency semantics:**
- `trace_id`, `caller_id`, `identity` **MUST NOT** be modified after creation (immutable)
- `call_chain` is automatically managed by Executor, module code **MUST NOT** modify it directly
- `data` is shared by reference, callers **SHOULD** manage synchronization themselves in concurrent scenarios
- `executor` reference **MUST** be thread-safe

See [PROTOCOL_SPEC §12.7.2 Context.data Sharing Semantics](../../PROTOCOL_SPEC.md#1272-contextdata-sharing-semantics) for the complete concurrency model specification.

---

## 3. Identity Details

**Identity unifies all types of caller identities.**

Module callers are not only human users, but also services, AI Agents, API Keys, etc. `Identity` expresses this in a structured way, giving the ACL engine a standard place to get `roles`.

### 3.1 type Values

| type | Scenario | Example |
|------|------|------|
| `user` | Human user via Web/CLI | `Identity(id="u_123", type="user", roles=["admin"])` |
| `service` | Microservice-to-service calls | `Identity(id="svc_order", type="service", roles=["internal"])` |
| `agent` | AI Agent autonomous module calls | `Identity(id="agent_gpt4", type="agent", roles=["readonly"])` |
| `api_key` | Third-party integrations | `Identity(id="key_abc", type="api_key", roles=["limited"])` |
| `system` | Scheduled tasks, internal triggers | `Identity(id="system", type="system", roles=["system"])` |

### 3.2 attrs Extensions

Business-specific fields go in `attrs`, not polluting the core structure:

=== "Python"

    ```python
    from apcore import Identity

    identity = Identity(
        id="u_123",
        type="user",
        roles=["user", "admin"],
        attrs={
            "name": "John Doe",
            "email": "john@example.com",
            "tenant_id": "tenant_456",
            "department": "engineering"
        }
    )
    ```

=== "TypeScript"

    ```typescript
    import { createIdentity } from 'apcore-js';

    const identity = createIdentity({
        id: "u_123",
        type: "user",
        roles: ["user", "admin"],
        attrs: {
            name: "John Doe",
            email: "john@example.com",
            tenantId: "tenant_456",
            department: "engineering"
        }
    });
    ```

=== "Rust"

    ```rust
    use apcore::Identity;
    use std::collections::HashMap;

    let mut attrs = HashMap::new();
    attrs.insert("name".into(), "John Doe".into());
    attrs.insert("email".into(), "john@example.com".into());
    attrs.insert("tenant_id".into(), "tenant_456".into());
    attrs.insert("department".into(), "engineering".into());

    let identity = Identity::new("u_123")
        .with_type("user")
        .with_roles(vec!["user", "admin"])
        .with_attrs(attrs);
    ```

### 3.3 Using in Modules

```python
class DeleteUserModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        # Check identity
        if not context.identity:
            return {"success": False, "error": "Authentication required"}

        # Check roles (ACL will also auto-validate, this is business-level double-check)
        if "admin" not in context.identity.roles:
            return {"success": False, "error": "Admin permission required"}

        # Record operator
        self._delete_user(inputs["user_id"])
        return {
            "success": True,
            "operated_by": context.identity.id
        }
```

---

## 4. Independent Field Details

### 4.1 trace_id

**Uniquely identifies a complete call chain.**

```python
def execute(self, inputs: dict, context: Context) -> dict:
    # For logging and tracing
    print(f"[{context.trace_id}] Starting execution...")

    # All subcalls share the same trace_id
    result = context.executor.call(
        module_id="executor.email.send_email",
        inputs={...},
        context=context
    )

    return result
```

| Feature | Description |
|------|------|
| Uniqueness | Each top-level call generates a new trace_id |
| Propagation | Subcalls inherit parent's trace_id |
| Format | 32-char lowercase hex (W3C Trace Context compatible); MUST NOT be all-zeros or all-f |

---

### 4.2 caller_id

**The module ID that called the current module.**

```python
def execute(self, inputs: dict, context: Context) -> dict:
    if context.caller_id:
        print(f"Called by: {context.caller_id}")
    else:
        print("Top-level call (no caller)")

    return {...}
```

| Scenario | caller_id Value |
|------|--------------|
| Top-level call (user/external) | `None` |
| Module A calls Module B | `"executor.module_a"` |
| Module B calls Module C | `"executor.module_b"` |

---

### 4.3 call_chain

**The complete path from root call to current call.**

```python
def execute(self, inputs: dict, context: Context) -> dict:
    # Print call chain
    print(f"Call chain: {' -> '.join(context.call_chain)}")

    # Detect circular calls
    current_module = "executor.email.send_email"
    if current_module in context.call_chain:
        raise ModuleError("Circular call detected!")

    return {...}
```

**Uses:**
- Debugging and logging
- Circular call detection (strict cycle)
- Call frequency detection (repeated call patterns)
- Call depth limiting
- ACL permission validation

---

### 4.4 executor

**Executor reference for calling other modules.**

```python
class UserRegisterModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        user_id = self._create_user(inputs)

        # Call other modules via executor
        email_result = context.executor.call(
            module_id="executor.email.send_email",
            inputs={
                "to": inputs["email"],
                "subject": "Hello",
                "body": "World"
            },
            context=context
        )

        return {
            "user_id": user_id,
            "email_sent": email_result["success"]
        }
```

---

## 5. Logging and Redaction

### 5.1 context.logger

**`context.logger` is a context-aware logger** that automatically injects `trace_id`, `module_id`, `caller_id` into every log entry.

```python
class SendEmailModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        # Use context.logger, automatically carries trace info
        context.logger.info(f"Sending email to {inputs['to']}")
        # Output: [abc-123] [executor.email.send_email] Sending email to user@example.com

        context.logger.warning("SMTP connection slow")
        # Output: [abc-123] [executor.email.send_email] SMTP connection slow

        return {"success": True, "message_id": "msg_123"}
```

> **Design note**: `context.logger` replaces manually passing `trace_id` to logs, ensuring all log entries automatically associate with the call chain.

### 5.2 Redacted Data

**`context.redacted_inputs` is automatically generated redacted input data by Executor**, where field values marked with `x-sensitive: true` are replaced with `"***REDACTED***"`.

```python
# Assuming password is marked x-sensitive: true in input_schema
# Original inputs: {"username": "john", "password": "secret123"}
# context.redacted_inputs: {"username": "john", "password": "***REDACTED***"}

class LoggingMiddleware(Middleware):
    def before(self, module_id: str, inputs: dict, context: Context) -> None:
        # ✅ Safe: use redacted data
        log.info(f"Calling {module_id}", extra={"inputs": context.redacted_inputs})

        # ❌ Dangerous: directly logging raw inputs may leak passwords
        # log.info(f"Calling {module_id}", extra={"inputs": inputs})
```

### 5.3 `_secret_` Prefix Convention

For sensitive data stored in `context.data`, use the `_secret_` prefix to mark it. Logging systems and middleware **SHOULD** automatically filter keys with this prefix:

```python
# Write
context.data["_secret_api_token"] = "sk-abc123..."

# Logging system automatically filters _secret_ prefix keys
# SafeLogFormatter output: {"data": {"task_info": {...}}}  # _secret_api_token omitted
```

---

## 6. data (Shared Pipeline State)

**`data` is a reference-shared dict, readable/writable along the call chain, used for pipeline state flow.**

### 6.1 Difference from inputs

| | inputs | data |
|--|--------|------|
| **Semantics** | Explicit input for this call | Shared pipeline state |
| **Schema** | Validated by input_schema | No Schema, free read/write |
| **Source** | Explicitly passed by caller | Accumulated along call chain |
| **Lifecycle** | Independent per call | Shared across entire call chain |
| **Passing method** | Value passing | Reference passing |

### 6.2 AI Orchestration Scenario (Core Use Case)

```python
# AI Agent orchestrates multi-step calls, data carries pipeline state
context = Context.create(executor=executor, identity=user_identity)
context.data["task_info"] = {"type": "report", "date": "2024-01"}

# Step 1: Fetch data
result_a = executor.call("module_fetch", inputs={...}, context=context)
# module_fetch writes context.data["raw_records"] = [...]

# Step 2: Analyze (reads Step 1 output)
result_b = executor.call("module_analyze", inputs={...}, context=context)
# module_analyze reads context.data["raw_records"]
# module_analyze writes context.data["analysis"] = {...}

# Step 3: Generate report (reads Step 1 + Step 2 outputs)
result_c = executor.call("module_report", inputs={...}, context=context)
```

### 6.3 Reading/Writing data in Modules

```python
class AnalyzeModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        # First try from inputs (explicit, testable)
        records = inputs.get("records")

        # Fallback to shared state (pipeline scenario)
        if records is None:
            records = context.data.get("raw_records")

        if records is None:
            raise ModuleError("VALIDATION_ERROR", "records required")

        analysis = self._analyze(records)

        # Write to shared state (for downstream use)
        context.data["analysis"] = analysis

        return {"score": analysis["score"]}
```

### 6.4 Common data Uses

| Use | key Example | Description |
|------|---------|------|
| Pipeline intermediate state | `raw_records`, `analysis` | Data flow in AI orchestration multi-step calls |
| Observability | `span_id`, `parent_span_id` | Written by middleware, used by TracingMiddleware |
| Internationalization | `locale`, `timezone` | Set at top level, read by modules as needed |
| Feature flags | `feature_flags` | Set at top level, read by modules as needed |
| Request metadata | `source`, `client_ip`, `session_id` | Written at entry layer |

---

## 7. Context Creation and Propagation

### 7.1 Creating at Top-Level Calls

=== "Python"

    ```python
    from apcore import Executor, Context, Identity
    import uuid

    executor = Executor(registry)

    # Method 1: Auto-create Context (recommended)
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"}
    )
    # Framework automatically generates trace_id, others are default values

    # Method 2: Manually create Context
    context = Context(
        trace_id=str(uuid.uuid4()),
        identity=Identity(
            id="u_123",
            type="user",
            roles=["admin"],
            attrs={"tenant_id": "t_456"}
        ),
        data={"locale": "zh-CN", "feature_flags": {"new_ui": True}}
    )
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"},
        context=context
    )
    ```

=== "TypeScript"

    ```typescript
    import { Executor, Context, createIdentity } from 'apcore-js';

    const executor = new Executor({ registry });

    // Method 1: Auto-create Context (recommended)
    const result1 = await executor.call(
        "executor.email.send_email",
        { to: "user@example.com", subject: "Hi", body: "Hello" }
    );
    // Framework automatically generates trace_id, others are default values

    // Method 2: Manually create Context
    const identity = createIdentity({
        id: "u_123",
        type: "user",
        roles: ["admin"],
        attrs: { tenantId: "t_456" }
    });
    const context = Context.create(undefined, identity);
    context.data.locale = "zh-CN";
    context.data.featureFlags = { newUi: true };

    const result2 = await executor.call(
        "executor.email.send_email",
        { to: "user@example.com", subject: "Hi", body: "Hello" },
        context
    );
    ```

=== "Rust"

    ```rust
    use apcore::{Executor, Context, Identity};

    let executor = Executor::new(registry);

    // Method 1: Auto-create Context (recommended)
    let result = executor.call(
        "executor.email.send_email",
        serde_json::json!({"to": "user@example.com", "subject": "Hi", "body": "Hello"}),
        None,
    ).await?;
    // Framework automatically generates trace_id, others are default values

    // Method 2: Manually create Context
    let identity = Identity::new("u_123")
        .with_type("user")
        .with_roles(vec!["admin"]);

    let mut ctx = Context::new(identity);
    ctx.data_mut().insert("locale".into(), "zh-CN".into());

    let result = executor.call(
        "executor.email.send_email",
        serde_json::json!({"to": "user@example.com", "subject": "Hi", "body": "Hello"}),
        Some(ctx),
    ).await?;
    ```

### 7.2 Passing Between Modules

```python
class ModuleA(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        # Pass context when calling other modules
        # Framework automatically updates caller_id and call_chain
        result = context.executor.call(
            module_id="executor.module_b",
            inputs={...},
            context=context
        )

        return result
```

### 7.3 Automatic Framework Handling

When passing Context, the framework automatically:

1. **Keeps trace_id unchanged**
2. **Updates caller_id** to current module ID
3. **Appends call_chain** with current module ID
4. **Keeps identity unchanged**
5. **Shares data by reference** (same dict instance)

```text
Top-level call:
  trace_id: "abc-123"
  caller_id: None
  call_chain: []
  data: {"locale": "zh-CN"}           # ← Same dict

  ↓ Calls orchestrator.user_register

orchestrator.user_register:
  trace_id: "abc-123"                  # Kept
  caller_id: None                      # Updated to parent (top-level)
  call_chain: ["orchestrator.user_register"]
  data: {"locale": "zh-CN"}           # ← Same dict (reference shared)

  ↓ Calls executor.email.send_email

executor.email.send_email:
  trace_id: "abc-123"                  # Kept
  caller_id: "orchestrator.user_register"
  call_chain: ["orchestrator.user_register", "executor.email.send_email"]
  data: {"locale": "zh-CN"}           # ← Same dict (reference shared)
```

---

## 8. Usage Patterns

### 8.1 Logging

```python
class SendEmailModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        # Recommended: use context.logger (automatically injects trace_id, module_id, caller_id)
        context.logger.info(f"Sending email to {inputs['to']}")

        # Can also manually use standard logger (need to manually pass context info)
        # import logging
        # logger = logging.getLogger(__name__)
        # logger.info(f"[{context.trace_id}] Sending email to {inputs['to']}")

        return {"success": True, "message_id": "msg_123"}
```

### 8.2 Permission Checking

```python
class DeleteUserModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        if not context.identity:
            return {"success": False, "error": "Authentication required"}

        if "admin" not in context.identity.roles:
            return {"success": False, "error": "Admin permission required"}

        self._delete_user(inputs["user_id"])
        return {"success": True}
```

### 8.3 Call Depth Limiting

```python
class RecursiveModule(Module):
    MAX_DEPTH = 10

    def execute(self, inputs: dict, context: Context) -> dict:
        if len(context.call_chain) > self.MAX_DEPTH:
            return {
                "success": False,
                "error": f"Call depth exceeded (max: {self.MAX_DEPTH})"
            }

        return {"success": True}
```

### 8.4 Circular Call Detection

> **Note:** The following detection is already implemented uniformly at the Executor layer (see [Executor API §6.1](./executor-api.md#61-complete-flow)), module code typically doesn't need manual detection. The following example illustrates the principle or custom detection scenarios.

**Strict cycle detection** (A→B→A, automatically detected by Executor):

```python
class SafeModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        current_id = "executor.safe_module"

        if current_id in context.call_chain:
            return {
                "success": False,
                "error": "Circular call detected",
                "call_chain": context.call_chain
            }

        return {"success": True}
```

### 8.5 Call Frequency Detection

**Frequency detection** (A→B→C→B→C→B→C..., automatically detected by Executor):

```python
class FrequencySafeModule(Module):
    MAX_REPEAT = 3  # Max appearances of same module

    def execute(self, inputs: dict, context: Context) -> dict:
        current_id = "executor.frequency_safe_module"

        count = context.call_chain.count(current_id)
        if count >= self.MAX_REPEAT:
            return {
                "success": False,
                "error": f"Call frequency exceeded: '{current_id}' appeared {count} times",
                "call_chain": context.call_chain
            }

        return {"success": True}
```

**Unified Executor-level detection (recommended):** No need to manually write frequency detection in each module, Executor automatically performs three-layer protection before calling any module:

```text
executor.call(module_id, inputs, context)
    ├─ Depth check: len(call_chain) >= 32             → CALL_DEPTH_EXCEEDED
    ├─ Cycle detection: module_id in call_chain            → CIRCULAR_CALL
    └─ Frequency detection: call_chain.count(module_id) >= 3   → CALL_FREQUENCY_EXCEEDED
```

---

## 9. Working with Middleware

```python
class LoggingMiddleware:
    """Logging middleware using Context information"""

    def before(self, module_id: str, inputs: dict, context: Context) -> dict:
        print(f"[{context.trace_id}] Before: {module_id}")
        print(f"  Caller: {context.caller_id}")
        identity_id = context.identity.id if context.identity else "anonymous"
        print(f"  Identity: {identity_id}")
        return inputs

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        print(f"[{context.trace_id}] After: {module_id}")
        print(f"  Success: {output.get('success')}")

    def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
        print(f"[{context.trace_id}] Error in {module_id}: {error}")


class TracingMiddleware:
    """Tracing middleware, writes span info to data"""

    def before(self, module_id: str, inputs: dict, context: Context) -> None:
        import uuid
        context.data["span_id"] = str(uuid.uuid4())[:16]
        context.data["span_start"] = time.time()

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        duration = time.time() - context.data.get("span_start", 0)
        context.data["span_duration_ms"] = round(duration * 1000)
```

---

## 10. Complete Reference Implementation

```python
from dataclasses import dataclass, field
from typing import Any
from datetime import datetime
import uuid


@dataclass(frozen=True)
class Identity:
    """Caller identity"""
    id: str
    type: str = "user"
    roles: tuple[str, ...] = field(default_factory=tuple)
    attrs: dict[str, Any] = field(default_factory=dict)


@dataclass
class Context:
    """Module execution context"""

    trace_id: str
    caller_id: str | None = None
    call_chain: list[str] = field(default_factory=list)
    executor: Any = None
    identity: Identity | None = None
    data: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def create(
        cls,
        executor: Any,
        identity: Identity | None = None,
        data: dict[str, Any] | None = None,
        trace_parent: "TraceParent | None" = None,
        services: "T | None" = None
    ) -> "Context":
        """Create new top-level Context

        Args:
            executor: Executor reference for inter-module calls
            identity: Caller identity for ACL enforcement
            data: Initial shared pipeline state
            trace_parent: W3C Trace Context parent for distributed tracing
            services: Dependency injection container for sharing services across the call chain
        """
        return cls(
            trace_id=trace_parent.trace_id if trace_parent else str(uuid.uuid4()),
            caller_id=None,
            call_chain=[],
            executor=executor,
            identity=identity,
            data=data or {},
            services=services
        )

    def child(self, target_module_id: str) -> "Context":
        """Create child Context (used by Executor when calling target module)"""
        return Context(
            trace_id=self.trace_id,                                     # Keep
            caller_id=self.call_chain[-1] if self.call_chain else None, # Previous module
            call_chain=self.call_chain + [target_module_id],            # Append target module
            executor=self.executor,                                     # Keep
            identity=self.identity,                                     # Keep
            data=self.data                                              # Reference shared
        )
```

---

## 10.1 Serialization

Context supports serialization for cross-process transfer (e.g., distributed execution, task queues).

```python
    def serialize(self) -> dict[str, Any]:
        """Serialize Context to a JSON-compatible dict for cross-process transfer.
        
        Includes `_context_version: 1` for forward compatibility.
        Excludes non-serializable fields: executor, cancel_token, services.
        """
        ...

    @classmethod
    def deserialize(cls, data: dict[str, Any]) -> "Context":
        """Reconstruct Context from a serialized dict.
        
        Validates `_context_version` field. Executor must be re-injected after deserialization.
        """
        ...
```

**Serialized fields:** `trace_id`, `caller_id`, `call_chain`, `identity`, `data`, `redacted_inputs`

**Skipped fields (runtime-only):** `executor`, `cancel_token`, `services`

!!! note
    After deserializing a Context, the `executor` reference must be re-injected before the Context can be used for inter-module calls. The `_context_version` field enables future schema evolution without breaking existing serialized data.

---

## 11. Edge Case Handling

Implementations **MUST** handle Context edge cases per the following table:

| Scenario | Behavior | Level |
|------|------|------|
| `context.data` exceeds memory limit | Behavior depends on language runtime (OOM or exception), **SHOULD** log WARN | **SHOULD** |
| Non-serializable value stored in `context.data` | Allow (in-memory passing), but fail when passing across processes | **MUST** |
| `call_chain` reaches `max_call_depth` | Throw `CALL_DEPTH_EXCEEDED` | **MUST** |
| `trace_id` is invalid 32-hex format | Log WARN and regenerate a 32-char hex trace_id | **SHOULD** |
| `caller_id` exceeds 128 chars | Log WARN, allow continued execution | **SHOULD** |
| `context.data` key conflict (parent/child same key) | Later write overwrites earlier value (dict semantics) | **MUST** |
| Concurrent modification of `context.data` (multi-threaded) | Behavior undefined (race condition), **SHOULD** use locks for protection | **SHOULD** |

**Best practices:**
- Avoid storing large objects (> 1MB) in `context.data`, use external cache instead
- Use namespace prefixes to avoid key conflicts (e.g., `my_module:result`)
- See [PROTOCOL_SPEC §12.7.2 Context.data Sharing Semantics](../../PROTOCOL_SPEC.md#1272-contextdata-sharing-semantics) for details

---

## Next Steps

- [Module Interface Definition](./module-interface.md) - How modules use Context
- [Executor API](./executor-api.md) - How executor manages Context
- [Core Executor Feature](../features/core-executor.md) - Executor pipeline and context propagation
- [Middleware Guide](../guides/middleware.md) - How middleware accesses Context
- [Observability](../features/observability.md) - Tracing, metrics, and context logger
