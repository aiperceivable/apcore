---
description: "Cooperative cancellation via a thread-safe CancelToken on Context: check()/cancel()/reset(), child propagation, executor timeout-triggered cancel with grace before ModuleTimeoutError."
---

# Cancellation System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5 Module Specification (no dedicated section yet).


## Overview

The Cancellation System provides cooperative cancellation for long-running module executions. It is built around a lightweight `CancelToken` that is attached to the execution `Context` and can be checked periodically by module code. When the executor's timeout fires, it cancels the token and waits a grace period before forcibly raising `ModuleTimeoutError`, giving modules an opportunity to clean up resources gracefully.

## Requirements

- Provide a `CancelToken` class with a simple boolean cancellation flag.
- The token **MUST** be thread-safe for setting the cancellation flag.
- Modules **MUST** be able to check the token at any point during execution via `check()`, which raises `ExecutionCancelledError` if the token has been cancelled.
- The token **MUST** be attachable to a `Context` object and propagated to child contexts for nested calls.
- The executor **MUST** cancel the token on timeout and wait a configurable grace period (default 5 seconds) before raising `ModuleTimeoutError`.
- The token **MUST** support `reset()` for reuse in testing scenarios.

## Technical Design

### CancelToken

=== "Python"
    ```python
    from apcore.cancel import CancelToken

    token = CancelToken()

    # Check cancellation status
    assert not token.is_cancelled

    # Request cancellation
    token.cancel()
    assert token.is_cancelled

    # Check raises if cancelled
    try:
        token.check()
    except ExecutionCancelledError:
        print("Cancelled!")

    # Reset for reuse
    token.reset()
    assert not token.is_cancelled
    ```
=== "TypeScript"
    ```typescript
    import { CancelToken, ExecutionCancelledError } from "apcore-js";

    const token = new CancelToken();

    // Check cancellation status
    console.log(token.isCancelled); // false

    // Request cancellation
    token.cancel();
    console.log(token.isCancelled); // true

    // Check raises if cancelled
    try {
        token.check();
    } catch (e) {
        if (e instanceof ExecutionCancelledError) {
            console.log("Cancelled!");
        }
    }

    // Reset for reuse
    token.reset();
    console.log(token.isCancelled); // false
    ```
=== "Rust"
    ```rust
    use apcore::cancel::CancelToken;

    let token = CancelToken::new();

    // Check cancellation status
    assert!(!token.is_cancelled());

    // Request cancellation
    token.cancel();
    assert!(token.is_cancelled());

    // Check returns Err if cancelled
    match token.check() {
        Ok(()) => unreachable!(),
        Err(e) => println!("Cancelled: {}", e),
    }

    // Reset for reuse
    token.reset();
    assert!(!token.is_cancelled());
    ```

### API

| Method | Description |
|--------|-------------|
| `is_cancelled` (property/getter) | Returns `true` if cancellation has been requested |
| `cancel()` | Sets the cancellation flag to `true` |
| `check()` | Raises `ExecutionCancelledError` if cancelled; no-op otherwise |
| `reset()` | Resets the flag to `false` (for testing/reuse) |

### ExecutionCancelledError

A `ModuleError` subclass with code `EXECUTION_CANCELLED`. Raised by `CancelToken.check()` when the token has been cancelled.

```python
from apcore import ExecutionCancelledError

class ExecutionCancelledError(ModuleError):
    def __init__(self, message: str = "Execution was cancelled") -> None:
        super().__init__(code="EXECUTION_CANCELLED", message=message)
```

### Integration with Context

The `CancelToken` is an optional field on the `Context` object. When a parent context creates a child context via `Context.child()`, the cancel token is propagated to the child, ensuring that cancellation cascades through nested module calls.

=== "Python"
    ```python
    from apcore.context import Context
    from apcore.cancel import CancelToken

    token = CancelToken()
    ctx = Context.create(cancel_token=token)

    # Token is propagated to child contexts
    child = ctx.child("target.module")
    assert child.cancel_token is token
    ```
=== "TypeScript"
    ```typescript
    import { Context, CancelToken } from "apcore-js";
    import { randomBytes } from "crypto";

    const token = new CancelToken();
    // Generate a W3C-compatible 32-char hex trace_id
    const traceId = randomBytes(16).toString("hex");
    // CancelToken is passed via the Context constructor (not Context.create)
    const ctx = new Context(
        traceId,        // traceId (32-char lowercase hex)
        null,           // callerId
        [],             // callChain
        null,           // executor
        null,           // identity
        null,           // redactedInputs
        {},             // data
        token,          // cancelToken
    );

    // Token is propagated to child contexts
    const child = ctx.child("target.module");
    // child.cancelToken === token
    ```
=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::cancel::CancelToken;
    use std::sync::Arc;

    let token = Arc::new(CancelToken::new());
    let ctx = Context::create(None, None, Some(token.clone()), None, Value::Null, None);

    // Token is propagated to child contexts
    let child = ctx.child("target.module");
    // child.cancel_token is the same Arc<CancelToken>
    ```

### Integration with Executor Timeout

The executor uses the `CancelToken` in its timeout enforcement at Step 8 of the pipeline:

1. Before executing the module, the executor creates or reuses a `CancelToken` on the context.
2. A timer is set for the shorter of per-module timeout and global deadline.
3. When the timer fires:
   - `token.cancel()` is called, signaling the module to stop.
   - A 5-second grace period begins.
   - If the module completes within the grace period, its result is returned normally.
   - If the module does not complete, `ModuleTimeoutError` is raised.

### Usage in Module Code

Modules performing long-running work **SHOULD** check the cancel token periodically:

=== "Python"
    ```python
    from apcore.decorator import module
    from apcore.context import Context

    @module(id="data.process_batch", description="Process large data batch")
    async def process_batch(items: list, context: Context) -> dict:
        results = []
        for item in items:
            # Check cancellation before each unit of work
            if context.cancel_token:
                context.cancel_token.check()
            result = await process_item(item)
            results.append(result)
        return {"processed": len(results), "results": results}
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";
    import type { Context } from "apcore-js";

    const client = new APCore();

    client.module({
        id: "data.process_batch",
        description: "Process large data batch",
        inputSchema: { type: "object", properties: { items: { type: "array" } } },
        outputSchema: { type: "object", properties: { processed: { type: "number" } } },
        execute: async (inputs: { items: unknown[] }, context: Context) => {
            const results: unknown[] = [];
            for (const item of inputs.items) {
                // Check cancellation before each unit of work
                if (context.cancelToken) {
                    context.cancelToken.check();
                }
                const result = await processItem(item);
                results.push(result);
            }
            return { processed: results.length, results };
        },
    });
    ```
=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use serde_json::{json, Value};

    async fn process_batch(inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let items = inputs["items"].as_array().unwrap();
        let mut results = Vec::new();
        for item in items {
            // Check cancellation before each unit of work
            if let Some(token) = &ctx.cancel_token {
                token.check()?;
            }
            let result = process_item(item).await?;
            results.push(result);
        }
        Ok(json!({"processed": results.len(), "results": results}))
    }
    ```

## Dependencies

- **Context** — Carries the `CancelToken` through the execution pipeline.
- **Core Executor** — Sets and monitors the token during timeout enforcement (Step 8).
- **Error System** — `ExecutionCancelledError` is part of the error hierarchy.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/cancel.py` | `CancelToken`, `ExecutionCancelledError` |

## Testing Strategy

- **Basic lifecycle tests** verify the cancel → check → raise flow and the reset mechanism.
- **Context propagation tests** verify that the token propagates through `Context.child()`.
- **Executor timeout tests** verify that timeout triggers `token.cancel()`, the grace period is respected, and `ModuleTimeoutError` is raised after grace expiry.
- **Concurrent cancellation tests** verify thread-safety when `cancel()` is called from a timer thread while `check()` is called from the module thread.

## Contract: CancelToken.is_cancelled

### Inputs
- No inputs

### Errors
- No errors raised

### Returns
- On success: bool/boolean/bool — `true` if `cancel()` has been called on this token, `false` otherwise

### Properties
- async: false
- thread_safe: true
- pure: true (reads internal cancelled state; no side effects)
- idempotent: true (repeated reads are safe and do not change state)

## Contract: CancelToken.cancel

### Inputs
- No inputs

### Errors
- No errors raised

### Returns
- On success: void/None/()

### Properties
- async: false
- thread_safe: true
- idempotent: true (multiple calls to cancel are safe; subsequent calls are no-ops)

## Contract: CancelToken.check

> This contract was previously published under the name `raise_if_cancelled`. No SDK exposes that name — the Python, TypeScript, and Rust examples above all call `check()` — so the heading is corrected here to match the public API. `check()` is not a second method layered on top of `raise_if_cancelled`; it is the one method, under its one real name.

### Inputs
- No inputs

### Errors
- `ExecutionCancelledError(code=EXECUTION_CANCELLED)` — if the token has been cancelled

### Returns
- On success (not cancelled): void/None/()
- On failure: raises `ExecutionCancelledError`

### Properties
- async: false
- thread_safe: true
- pure: true (no side effects; only checks internal cancelled state)

## Contract: CancelToken.reset

### Inputs
- No inputs

### Errors
- No errors raised

### Returns
- On success: void/None/()

### Properties
- async: false
- thread_safe: true
- idempotent: true (multiple calls to reset are safe; the flag is simply set to `false` each time)

!!! note "Intended for testing/reuse, not for cancellation cleanup mid-call"
    `reset()` clears the cancelled flag on the existing token instance. Nothing observes the transition: a module that already raised `ExecutionCancelledError` from `check()` has already unwound, and the Executor does not re-poll a token after acting on cancellation. Resetting an in-flight token to "uncancel" a call in progress is not a supported pattern; `reset()` exists so a single `CancelToken` instance can be reused across independent test cases or task runs.
