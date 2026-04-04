# Executor API

> **Canonical Definition** - This document is the authoritative definition of the Executor interface

> Executor is responsible for module execution, Context management, middleware scheduling, and observability.

## 1. Interface Overview

```python
from typing import Any
from apcore import Registry, Context, Middleware


class Executor:
    """Module executor"""

    def __init__(
        self,
        registry: Registry,
        middlewares: list[Middleware] | None = None,
        acl: "ACL | None" = None,
        approval_handler: "ApprovalHandler | None" = None,
        config: "Config | None" = None
    ) -> None:
        """
        Initialize Executor

        Args:
            registry: Module registry
            middlewares: Middleware list (executed in order)
            acl: Access Control List
            approval_handler: Approval handler for modules with requires_approval=true
            config: Framework configuration (optional). When provided,
                executor settings (timeouts, max call depth, etc.) are
                read from config.
        """
        ...

    # ============ Factory Methods ============

    @classmethod
    def from_registry(
        cls,
        registry: Registry,
        middlewares: list[Middleware] | None = None,
        acl: "ACL | None" = None,
        config: "Config | None" = None,
        approval_handler: "ApprovalHandler | None" = None,
    ) -> "Executor":
        """
        Create an Executor from a Registry instance

        Convenience factory that constructs a fully configured Executor.
        Equivalent to calling the constructor directly but provides a
        clearer intent when composing from existing components.
        """
        ...

    # ============ Execution Methods ============

    def call(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        """
        Synchronously call module

        Args:
            module_id: Module ID
            inputs: Input parameters
            context: Call context (optional, auto-created)
            version_hint: Version constraint for module resolution (e.g., ">=1.0.0")

        Returns:
            Module output

        Raises:
            ModuleNotFoundError: Module does not exist
            ModuleDisabledError: Module is disabled
            ModuleTimeoutError: Module execution timed out
            ValidationError: Input/output validation failed
            ACLDeniedError: Insufficient permissions
            ApprovalDeniedError: Approval explicitly rejected
            ApprovalTimeoutError: Approval timed out
            ApprovalPendingError: Approval pending, retry with _approval_token (Phase B)
            CallDepthExceededError: Call chain depth exceeded
            CircularCallError: Circular call detected
            CallFrequencyExceededError: Same module call frequency exceeded
            ModuleError: Module execution error
        """
        ...

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        """
        Asynchronously call module

        Note: Modules only need to define one execute() method (def or async def),
        framework auto-detects and selects appropriate calling method. call_async is used
        to call modules in async context.
        """
        ...

    async def stream(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> AsyncIterator[dict[str, Any]]:
        """
        Stream module output chunk by chunk

        Yields partial results as they become available.
        Calls the module's stream() method if defined,
        otherwise falls back to standard execute().
        Streaming chunks are accumulated via recursive deep merge (depth cap 32).
        """
        ...

    def validate(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
    ) -> "PreflightResult":
        """
        Non-destructive preflight check (Steps 1-7)

        Runs up to 7 checks without executing module code or middleware:
        1. Module ID format validation
        2. Module lookup
        3. Call chain safety (if context provided)
        4. ACL check
        5. Approval detection (reports requires_approval flag)
        6. Input schema validation
        7. Module preflight (MAY) — invokes module.preflight() for advisory warnings

        Check 7 is optional (MAY-level per spec §12.8.5.1). If the module
        implements preflight(), warnings are collected but never block validation.

        Returns:
            PreflightResult with per-check results, requires_approval flag,
            and duck-type compatible .valid and .errors properties
        """
        ...

    # ============ Middleware Management ============

    def use(self, middleware: Middleware) -> "Executor":
        """Add middleware (Class-based)"""
        ...

    def use_before(self, callback: "Callable") -> "Executor":
        """Add before function middleware (Function-first API)"""
        ...

    def use_after(self, callback: "Callable") -> "Executor":
        """Add after function middleware (Function-first API)"""
        ...

    def remove(self, middleware: Middleware) -> bool:
        """Remove middleware"""
        ...

    # ============ Configuration ============

    def set_acl(self, acl: "ACL") -> None:
        """Set or replace the Access Control List after construction"""
        ...

    def set_approval_handler(self, handler: "ApprovalHandler") -> None:
        """Set or replace the approval handler after construction"""
        ...

    # ============ Properties ============

    @property
    def registry(self) -> Registry:
        """Get Registry"""
        ...

    @property
    def middlewares(self) -> list[Middleware]:
        """Get middleware list"""
        ...
```

---

## 2. Initialization

### 2.1 Basic Initialization

```python
from apcore import Registry, Executor

# Create Registry
registry = Registry(extensions_dir="./extensions")
registry.discover()

# Create Executor
executor = Executor(registry)
```

### 2.2 With Middleware

```python
from apcore import Registry, Executor
from apcore import LoggingMiddleware, MetricsMiddleware

executor = Executor(
    registry=registry,
    middlewares=[
        LoggingMiddleware(),
        MetricsMiddleware()
    ]
)
```

### 2.3 With ACL

```python
from apcore import Registry, Executor, ACL

acl = ACL.load("./acl/global_acl.yaml")
executor = Executor(registry=registry, acl=acl)
```

### 2.4 With ApprovalHandler

```python
from apcore import Registry, Executor
from apcore.approval import AutoApproveHandler, CallbackApprovalHandler, ApprovalResult

# Auto-approve (testing/development)
executor = Executor(registry=registry, approval_handler=AutoApproveHandler())

# Custom callback
async def my_approval_logic(request):
    if request.module_id.startswith("dangerous."):
        return ApprovalResult(status="rejected", reason="Dangerous modules blocked")
    return ApprovalResult(status="approved")

executor = Executor(
    registry=registry,
    approval_handler=CallbackApprovalHandler(my_approval_logic)
)
```

When an `approval_handler` is set, modules declaring `requires_approval=true` in their annotations will trigger the handler at Step 5 of the execution pipeline. See [Approval System](../features/approval-system.md).

---

## 3. Calling Modules

### 3.1 Basic Call

```python
result = executor.call(
    module_id="executor.email.send_email",
    inputs={
        "to": "user@example.com",
        "subject": "Hello",
        "body": "World"
    }
)

print(result)
# {"success": True, "message_id": "msg_123", "error": None}
```

### 3.2 With Context

```python
from apcore import Context

# Create custom Context
context = Context(
    trace_id="custom-trace-123",
    identity=Identity(id="user_456", type="user", roles=["admin"]),
    data={"locale": "zh-CN"}
)

result = executor.call(
    module_id="executor.email.send_email",
    inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"},
    context=context
)
```

### 3.3 Async Call

```python
import asyncio

async def main():
    result = await executor.call_async(
        module_id="executor.email.send_email",
        inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"}
    )
    print(result)

asyncio.run(main())
```

### 3.4 Batch Concurrent Calls

```python
import asyncio

async def send_batch_emails(emails: list[dict]):
    tasks = [
        executor.call_async(
            module_id="executor.email.send_email",
            inputs=email
        )
        for email in emails
    ]
    results = await asyncio.gather(*tasks, return_exceptions=True)
    return results

# Usage
emails = [
    {"to": "user1@example.com", "subject": "Hi", "body": "Hello 1"},
    {"to": "user2@example.com", "subject": "Hi", "body": "Hello 2"},
    {"to": "user3@example.com", "subject": "Hi", "body": "Hello 3"},
]
results = asyncio.run(send_batch_emails(emails))
```

---

## 4. Input Validation

### 4.1 Auto Validation

```python
# Inputs are automatically validated against input_schema
try:
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={
            "to": "invalid-email",  # Format error
            "subject": "Hi"
            # Missing required field body
        }
    )
except ValidationError as e:
    print(f"Validation failed: {e.errors}")
    # [
    #   {"field": "to", "message": "Invalid email format"},
    #   {"field": "body", "message": "Field required"}
    # ]
```

### 4.2 Pre-validation (Preflight)

```python
# Non-destructive preflight check (Steps 1-6, no execution)
preflight = executor.validate(
    module_id="executor.email.send_email",
    inputs={"to": "user@example.com", "subject": "Hi"}
)

if preflight.valid:
    print("All 6 checks passed")
else:
    print(f"Errors: {preflight.errors}")

# Inspect individual checks
for check in preflight.checks:
    status = "PASS" if check.passed else f"FAIL: {check.error}"
    print(f"  {check.check}: {status}")

# Check if module requires approval
if preflight.requires_approval:
    print("This module requires approval before execution")
```

`PreflightResult` runs 6 checks: module ID format, module lookup, call chain safety, ACL, approval detection, and schema validation. It is duck-type compatible with the legacy `ValidationResult` — `.valid` and `.errors` work identically.

---

## 5. Error Handling

### 5.1 Error Types

```python
from apcore import (
    ModuleNotFoundError,
    ModuleDisabledError,
    ModuleTimeoutError,
    ValidationError,
    ACLDeniedError,
    ApprovalDeniedError,
    ApprovalTimeoutError,
    ApprovalPendingError,
    CallDepthExceededError,
    CircularCallError,
    CallFrequencyExceededError,
    ReloadFailedError,
    FeatureNotImplementedError,
    DependencyNotFoundError,
    ModuleError
)

try:
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"}
    )
except ModuleNotFoundError as e:
    # Module does not exist
    print(f"Module not found: {e.module_id}")

except ModuleDisabledError as e:
    # Module is disabled via system.control.toggle_feature
    print(f"Module disabled: {e.module_id}")

except ModuleTimeoutError as e:
    # Module execution timed out (per-module or global deadline)
    print(f"Timeout: {e.module_id}")

except ValidationError as e:
    # Input/output validation failed
    print(f"Validation failed: {e.errors}")

except ACLDeniedError as e:
    # Insufficient permissions
    print(f"Access denied: {e.message}")
    print(f"Required: {e.required_permission}")
    print(f"Caller: {e.caller_id}")

except ApprovalDeniedError as e:
    # Approval explicitly rejected (requires_approval=true module)
    print(f"Approval denied: {e.reason}")

except ApprovalTimeoutError as e:
    # Approval timed out waiting for response
    print(f"Approval timed out: {e.reason}")

except CallDepthExceededError as e:
    # Call chain depth exceeded
    print(f"Call depth exceeded: {e.current_depth}/{e.max_depth}")
    print(f"Call chain: {e.call_chain}")

except CircularCallError as e:
    # Circular call (strict cycle: A→B→A)
    print(f"Circular call detected: {e.module_id}")
    print(f"Call chain: {e.call_chain}")

except CallFrequencyExceededError as e:
    # Same module call frequency exceeded (A→B→C→B→C→B→C...)
    print(f"Call frequency exceeded: {e.module_id} called {e.count}/{e.max_repeat} times")
    print(f"Call chain: {e.call_chain}")

except ModuleError as e:
    # Module execution error
    print(f"Module error: {e.message}")
    print(f"Module ID: {e.module_id}")
    print(f"Trace ID: {e.trace_id}")
```

### 5.2 Error Context

```python
try:
    result = executor.call(...)
except ModuleError as e:
    # Get complete error context
    print(f"Error: {e.message}")
    print(f"Module: {e.module_id}")
    print(f"Trace ID: {e.trace_id}")
    print(f"Call Chain: {e.call_chain}")
    print(f"Inputs: {e.inputs}")

    # Log to logging system
    logger.error(
        f"Module execution failed",
        extra={
            "trace_id": e.trace_id,
            "module_id": e.module_id,
            "call_chain": e.call_chain
        }
    )
```

### 5.3 AI Error Guidance Fields

All `ModuleError` instances carry four optional guidance fields (see PROTOCOL_SPEC §8.1.1) that enable AI agents to programmatically understand and respond to errors without parsing human-readable messages:

| Field | Type | Purpose |
|-------|------|---------|
| `retryable` | `bool \| None` | Whether retrying the same call may succeed. Each error code has a default (see §8.6). `None` means "depends on context". |
| `ai_guidance` | `str \| None` | Machine-readable hint for AI agents, e.g. `"validate input schema before retry"`. |
| `user_fixable` | `bool \| None` | Whether the end-user (not developer) can fix the issue. |
| `suggestion` | `str \| None` | Actionable suggestion for resolving the error. |

Fields with `None` values are omitted from serialized output (sparse serialization).

```python
except SchemaValidationError as e:
    print(e.retryable)      # False (default for this error code)
    print(e.user_fixable)   # True — user can fix their input
    print(e.suggestion)     # "Table names must use only lowercase letters and underscores"
    print(e.ai_guidance)    # "validate input against schema before retry"
```

---

## 6. Execution Flow

### 6.1 Complete Flow

```
executor.call(module_id, inputs, context)
    │
    ├─ 1. Create/validate Context
    │      └─ If context is None, auto-create
    │
    ├─ 2. Call chain safety checks
    │      ├─ 2a. Depth check: len(call_chain) >= 32 → throw CallDepthExceededError
    │      ├─ 2b. Cycle detection: module_id in call_chain → throw CircularCallError
    │      └─ 2c. Frequency detection: call_chain.count(module_id) >= max_repeat → throw CallFrequencyExceededError
    │
    ├─ 3. Lookup module
    │      └─ registry.get(module_id)
    │      └─ If not exists, throw ModuleNotFoundError
    │
    ├─ 4. ACL check
    │      └─ acl.check(caller_id, module_id, context)
    │      └─ If denied, throw ACLDeniedError
    │
    ├─ 5. Approval gate (if approval_handler configured)
    │      └─ Only for modules with requires_approval=true
    │      └─ approval_handler.request_approval(request)
    │      └─ If rejected, throw ApprovalDeniedError
    │      └─ If timeout, throw ApprovalTimeoutError
    │      └─ If pending, throw ApprovalPendingError (Phase B)
    │
    ├─ 6. Input validation
    │      └─ Validate against input_schema
    │      └─ If failed, throw ValidationError
    │
    ├─ 7. Middleware before
    │      └─ middleware.before(module_id, inputs, context)
    │
    ├─ 8. Execute module
    │      └─ module.execute(inputs, context)
    │      └─ If failed, call middleware on_error
    │
    ├─ 9. Output validation
    │      └─ Validate against output_schema
    │
    ├─ 10. Middleware after
    │      └─ middleware.after(module_id, inputs, output, context)
    │
    └─ 11. Return result
```

### 6.2 Automatic Context Handling

When modules internally call other modules, Executor automatically handles Context:

```python
# Internal module call
result = context.executor.call(
    module_id="executor.other_module",
    inputs={...},
    context=context  # Pass current context
)

# Executor automatically:
# 1. Keeps trace_id unchanged
# 2. Updates caller_id to current module
# 3. Appends call_chain
# 4. Call chain safety checks (depth + cycle + frequency)
```

### 6.3 Execution State Machine

```
  ┌─────────┐
  │  idle   │
  └────┬────┘
       │ call()
       ▼
  ┌──────────┐  depth/cycle/freq  ┌──────────────────────────┐
  │call_chain│───────────────────▶│ error: DEPTH_EXCEEDED    │
  │  guard   │                    │      / CIRCULAR_CALL     │
  └────┬─────┘                    │      / FREQUENCY_EXCEEDED│
       │ check passed             └──────────────────────────┘
       ▼
  ┌─────────┐    module not exist ┌──────────────────┐
  │ resolve │───────────────────▶│ error: NOT_FOUND │
  └────┬────┘                    └──────────────────┘
       │ module found
       ▼
  ┌─────────┐    permission denied ┌──────────────────┐
  │  acl    │────────────────────▶│ error: ACL_DENIED│
  └────┬────┘                     └──────────────────┘
       │ permission passed
       ▼
  ┌──────────┐  rejected/timeout ┌──────────────────────────┐
  │ approval │──────────────────▶│ error: APPROVAL_DENIED   │
  │   gate   │                   │      / APPROVAL_TIMEOUT  │
  └────┬─────┘                   │      / APPROVAL_PENDING  │
       │                         └──────────────────────────┘
       │ approved (or skipped)
       ▼
  ┌──────────┐   validation failed ┌──────────────────────┐
  │ validate │────────────────────▶│ error: VALIDATION    │
  │  input   │                     └──────────────────────┘
  └────┬─────┘
       │ validation passed
       ▼
  ┌──────────┐
  │ before   │──── middleware error ──▶ on_error chain
  │middleware│
  └────┬─────┘
       │
       ▼
  ┌──────────┐   execution error  ┌──────────────────────┐
  │ execute  │────────────────────▶│ on_error middleware  │
  │  module  │                     └──────────────────────┘
  └────┬─────┘
       │ success
       ▼
  ┌──────────┐   validation failed ┌──────────────────────┐
  │ validate │────────────────────▶│ error: VALIDATION    │
  │  output  │                     └──────────────────────┘
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │  after   │
  │middleware│
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │ return   │
  │ result   │
  └──────────┘
```

### 6.4 Timeout Specification (Dual-Timeout Model)

| Config | Default | Description |
|------|--------|------|
| Module execution timeout | 30000ms | Can be overridden via `resources.timeout` |
| Global timeout | 60000ms | Total time including middleware and validation |
| ACL check timeout | 1000ms | Max time for ACL rule evaluation |

The executor enforces a **dual-timeout model**: both a per-module timeout and a global deadline are tracked. The shorter of the two is applied, preventing nested call chains from exceeding the global budget. The global deadline is set on the root call and propagated to child contexts via `Context._global_deadline`.

- After timeout **MUST** throw `MODULE_TIMEOUT` error
- Timeout counting **MUST** start from the first `before()` middleware
- Middleware execution time **SHOULD** count toward total timeout

**Cooperative cancellation:** On module timeout, the executor sends `CancelToken.cancel()` and waits a 5-second grace period before raising `ModuleTimeoutError`. Modules that check `cancel_token` in their execution loop can clean up gracefully:

```python
@client.module(id="long.task", description="Long-running task")
async def long_task(inputs: dict, context: Context) -> dict:
    for item in items:
        if context.cancel_token.is_cancelled:
            return {"partial": True, "processed": count}
        await process(item)
    return {"partial": False, "processed": len(items)}
```

### 6.5 Concurrent Execution Semantics

- Same Executor instance **MUST** support concurrent calls by multiple threads/coroutines
- Each `call()` **MUST** use independent Context copy (call_chain and caller_id)
- `context.data` is shared by reference, concurrent calls **SHOULD** use different Context instances
- Batch `call_async()` **MAY** execute concurrently, order not guaranteed

### 6.6 Edge Case Handling

Implementations **MUST** handle Executor edge cases per the following table:

| Scenario | Behavior | Level |
|------|------|------|
| `timeout = 0` | Disable timeout limit, log WARN | **MUST** |
| `timeout` is negative | Throw `GENERAL_INVALID_INPUT` | **MUST** |
| `module_id` is empty string `""` | Throw `MODULE_NOT_FOUND` | **MUST** |
| `inputs = null` | Treat as empty dict `{}`, continue validation | **MUST** |
| `context = null` | Create new Context (empty call_chain) | **MUST** |
| Concurrent calls sharing same Context instance | Behavior undefined (race condition), **SHOULD** log WARN | **SHOULD** |
| `call()` during module `unregister()` | If execution started, continue; if not started, throw `MODULE_NOT_FOUND` | **MUST** |
| Call chain depth exceeds `max_call_depth` | Throw `CALL_DEPTH_EXCEEDED` | **MUST** |

**Concurrent safety notes:**
- Executor instance **MUST** be thread-safe, supporting multi-threaded concurrent calls
- Each `call()` **SHOULD** use independent Context instance (created via `child()`)
- See [PROTOCOL_SPEC §12.7 Concurrency Model Specification](../../PROTOCOL_SPEC.md#127-concurrency-model-specification)

---

## 7. Middleware

### 7.1 Adding Middleware

```python
from apcore import LoggingMiddleware, MetricsMiddleware

# Method 1: Add during initialization (Class-based)
executor = Executor(
    registry=registry,
    middlewares=[LoggingMiddleware(), MetricsMiddleware()]
)

# Method 2: Chain adding (Class-based)
executor = Executor(registry=registry)
executor.use(LoggingMiddleware()).use(MetricsMiddleware())

# Method 3: Function-first API (lightweight scenarios)
executor.use_before(lambda module_id, inputs, ctx: print(f"Calling {module_id}"))
executor.use_after(lambda module_id, inputs, output, ctx: print(f"Done {module_id}"))
```

### 7.2 Execution Order

Middleware executes in onion model:

```
Request → MW1.before → MW2.before → Module → MW2.after → MW1.after → Response

On error:
Request → MW1.before → MW2.before → Module(error) → MW2.on_error → MW1.on_error
```

### 7.3 Removing Middleware

```python
logging_mw = LoggingMiddleware()
executor.use(logging_mw)

# Remove
executor.remove(logging_mw)
```

---

## 8. ACL Integration

### 8.1 Configure ACL

!!! warning "Default-deny is strongly recommended"
    The example below uses `default_effect: deny` as recommended by the protocol specification.
    Using `default_effect: allow` is permitted but should be accompanied by explicit deny rules
    for sensitive modules. See [ACL System](../features/acl-system.md) for security guidance.

```yaml
# acl/global_acl.yaml
default_effect: deny

rules:
  # Allow all modules to call common.* modules
  - callers: ["*"]
    targets: ["common.*"]
    effect: allow

  # Only allow orchestrator.* to call executor.*
  - callers: ["orchestrator.*"]
    targets: ["executor.*"]
    effect: allow

  # Deny direct calls to internal.*
  - callers: ["*"]
    targets: ["internal.*"]
    effect: deny
```

### 8.2 Using ACL

```python
from apcore import Executor, ACL

acl = ACL.load("./acl/global_acl.yaml")
executor = Executor(registry=registry, acl=acl)

# Permissions automatically checked during calls
try:
    result = executor.call(
        module_id="internal.secret_module",
        inputs={...},
        context=context
    )
except ACLDeniedError as e:
    print(f"Access denied: {e.message}")
```

---

## 9. Observability

### 9.1 Built-in Metrics

Executor automatically collects the following metrics:

| Metric | Type | Description |
|------|------|------|
| `apcore_module_calls_total` | Counter | Total module calls |
| `apcore_module_duration_seconds` | Histogram | Module execution time |
| `apcore_module_errors_total` | Counter | Total module errors |
| `apcore_validation_errors_total` | Counter | Total validation errors |
| `apcore_acl_denied_total` | Counter | Total ACL denials |

### 9.2 Tracing

```python
# All calls carry trace_id
result = executor.call(
    module_id="executor.email.send_email",
    inputs={...}
)

# Can get trace_id from middleware or logs
# Used to trace complete call chain in distributed systems
```

### 9.3 Logging

```python
from apcore import LoggingMiddleware
import logging

# Configure logging
logging.basicConfig(level=logging.INFO)

# Add logging middleware
executor.use(LoggingMiddleware(
    log_inputs=True,   # Whether to log inputs
    log_outputs=True,  # Whether to log outputs
    log_errors=True    # Whether to log errors
))
```

Log output example:

```
[INFO] [trace:abc-123] Before: executor.email.send_email
[INFO] [trace:abc-123] Inputs: {"to": "user@example.com", ...}
[INFO] [trace:abc-123] After: executor.email.send_email (45ms)
[INFO] [trace:abc-123] Output: {"success": true, ...}
```

> **Security tip**: Logging middleware **SHOULD** use `context.redacted_inputs` instead of raw `inputs` to avoid leaking sensitive fields marked with `x-sensitive` (like passwords, API Keys). See [Context Object — redacted_inputs](./context-object.md#52-redacted-data).

---

## 10. Complete Example

```python
from apcore import Registry, Executor, Context, ACL
from apcore import LoggingMiddleware, MetricsMiddleware

# 1. Create Registry and discover modules
registry = Registry(extensions_dir="./extensions")
registry.discover()

# 2. Load ACL
acl = ACL.load("./acl/global_acl.yaml")

# 3. Create Executor
executor = Executor(
    registry=registry,
    middlewares=[
        LoggingMiddleware(),
        MetricsMiddleware()
    ],
    acl=acl
)

# 4. Create Context (with identity info)
context = Context.create(
    executor=executor,
    identity=Identity(id="user_123", type="user", roles=["admin"]),
    data={"locale": "zh-CN"}
)

# 5. Call module
try:
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={
            "to": "user@example.com",
            "subject": "Hello",
            "body": "World"
        },
        context=context
    )
    print(f"Success: {result}")

except Exception as e:
    print(f"Error: {e}")

# 6. Async batch calls
async def send_batch():
    tasks = [
        executor.call_async(
            module_id="executor.email.send_email",
            inputs={"to": f"user{i}@example.com", "subject": "Hi", "body": "Hello"},
            context=context
        )
        for i in range(10)
    ]
    return await asyncio.gather(*tasks, return_exceptions=True)

import asyncio
results = asyncio.run(send_batch())
print(f"Sent {len([r for r in results if not isinstance(r, Exception)])} emails")
```

---

## Next Steps

- [Registry API](./registry-api.md) - Module registration and discovery
- [Context Object](./context-object.md) - Execution context
- [Core Executor Feature](../features/core-executor.md) - Detailed executor pipeline specification
- [Middleware System](../features/middleware-system.md) - Middleware architecture and built-in middleware
- [Middleware Guide](../guides/middleware.md) - Middleware development
- [ACL System](../features/acl-system.md) - Access control architecture
- [ACL Configuration Guide](../guides/acl-configuration.md) - Access control configuration
- [Approval System](../features/approval-system.md) - Human approval workflow
