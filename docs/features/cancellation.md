---
description: "Cooperative cancellation via a thread-safe CancelToken on Context: check()/cancel()/reset(), child propagation, pipeline cancel checks, and how the executor enforces timeouts."
---

# Cancellation System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.7 Context Object (`cancel_token`), §12.7.5 Timeout Enforcement. Pipeline check points: [core-executor.md](./core-executor.md#cancel-token-mid-pipeline-check).


## Overview

The Cancellation System provides caller-driven cancellation for module executions. It is built around a lightweight `CancelToken` that the caller attaches to the execution `Context`; the pipeline checks it before running the module, and module code checks it periodically during long-running work. Timeouts are enforced separately by the executor (see [Timeouts](#integration-with-executor-timeout)) and do not use the token.

## Requirements

- Provide a `CancelToken` class with a simple boolean cancellation flag.
- The token **MUST** be thread-safe for setting the cancellation flag.
- Modules **MUST** be able to check the token at any point during execution via `check()` (alias `raise_if_cancelled()`), which raises `ExecutionCancelledError` if the token has been cancelled.
- The token **MUST** be attachable to a `Context` object and propagated to child contexts for nested calls.
- The pipeline **MUST** check the token at Step 2 (call-chain guard) and again at Step 8 (immediately before invoking the module) (D-21).
- `ExecutionCancelledError` **MUST** bypass the `on_error` middleware chain (D-20).
- The token **MUST** support `reset()` for reuse in testing scenarios.

## Technical Design

### CancelToken

=== "Python"
    ```python
    from apcore import CancelToken, ExecutionCancelledError

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
    use apcore::CancelToken;

    fn main() {
        let token = CancelToken::new();

        // Check cancellation status
        assert!(!token.is_cancelled());

        // Request cancellation (takes &self; clones share the same flag)
        token.cancel();
        assert!(token.is_cancelled());

        // check() returns Err(ExecutionCancelledError) if cancelled
        match token.check() {
            Ok(()) => unreachable!(),
            Err(e) => println!("Cancelled: {e}"),
        }

        // Reset for reuse
        token.reset();
        assert!(!token.is_cancelled());
    }
    ```

### API

| Method | Python | TypeScript | Rust | Description |
|--------|--------|------------|------|-------------|
| is cancelled | `is_cancelled` (property) | `isCancelled` (getter) | `is_cancelled()` | `true` if cancellation has been requested |
| cancel | `cancel()` | `cancel()` | `cancel()` | Sets the cancellation flag |
| check | `check()` / `raise_if_cancelled()` | `check()` / `raiseIfCancelled()` | `check()` / `raise_if_cancelled()` | Raises `ExecutionCancelledError` if cancelled; no-op otherwise. The two names are identical. |
| reset | `reset()` | `reset()` | `reset()` | Clears the flag (for testing/reuse) |
| abort signal | — | `signal` (getter) | — | The token's `AbortSignal` (TypeScript only, see below) |

Rust additionally has `check_for(module_id)`, which returns the same error with `module_id` populated.

### ExecutionCancelledError

Error code `EXECUTION_CANCELLED`, raised by `check()` when the token has been cancelled. In Python and TypeScript it is a `ModuleError` subclass. In Rust it is a separate struct (`module_id: Option<String>`, `message: String`) that converts into `ModuleError` via `From`, so `token.check()?` works inside a function returning `Result<_, ModuleError>`.

### TypeScript: `AbortSignal`

In TypeScript, `cancel()` also aborts an `AbortController` owned by the token (D-18). The signal is available as `token.signal` and as `context.signal` (a never-aborted signal when no token is bound). Pass it to Web-API I/O (`fetch`, `AbortSignal.any`, Web Streams) so the I/O is aborted when the call is cancelled. The executor also races the module's promise against this signal, so a cancelled call rejects with `ExecutionCancelledError` immediately, even while the module is suspended at an await point that is not a Web API.

An `AbortSignal` cannot be un-aborted, so after `cancel()` the signal stays aborted even if `reset()` clears the flag; construct a new `CancelToken` for signal-based work instead of reusing a cancelled one (D-90). Python and Rust have no signal channel: in a direct call a module observes cancellation only through `check()` / `is_cancelled`. (`AsyncTaskManager.cancel()` additionally interrupts its own background tasks in every SDK — see [Async Tasks](./async-tasks.md#cancellation).)

### Integration with Context

The `CancelToken` is an optional field on the `Context` object. When a parent context creates a child context via `Context.child()`, the cancel token is propagated to the child, ensuring that cancellation cascades through nested module calls.

=== "Python"
    ```python
    from apcore import CancelToken, Context

    token = CancelToken()
    ctx = Context.create(cancel_token=token)

    # Token is propagated to child contexts
    child = ctx.child("target.module")
    assert child.cancel_token is token
    ```
=== "TypeScript"
    ```typescript
    import { CancelToken, Context } from "apcore-js";

    const token = new CancelToken();
    // Context.create(identity, traceParent, cancelToken, data, services, globalDeadline)
    const ctx = Context.create(null, null, token);

    // Token is propagated to child contexts
    const child = ctx.child("target.module");
    console.log(child.cancelToken === token); // true
    ```
=== "Rust"
    ```rust
    use apcore::{CancelToken, Context};
    use serde_json::Value;

    fn main() {
        let token = CancelToken::new();
        // Context::create(identity, trace_parent, cancel_token, data, services, global_deadline)
        let ctx: Context<Value> =
            Context::create(None, None, Some(token.clone()), None, Value::Null, None);

        // Token is propagated to child contexts; clones share one flag
        let child = ctx.child("target.module");
        token.cancel();
        assert!(child.cancel_token.as_ref().unwrap().is_cancelled());
    }
    ```

To cancel a call, create the context with a token, pass it to `call()`, and invoke `token.cancel()` from elsewhere (another task, a request-abort handler). The pipeline checks the token at Step 2 and again at Step 8 before invoking the module; see [Cancel Token Mid-Pipeline Check](./core-executor.md#cancel-token-mid-pipeline-check). `cancel_token` is runtime-only and never serialized; cancelling across process boundaries needs an out-of-band channel ([Distributed Cancellation Semantics](./core-executor.md#distributed-cancellation-semantics)).

### Integration with Executor Timeout

The executor enforces timeouts at Step 8 of the pipeline independently of the `CancelToken`:

1. The effective timeout is the module's declared `resources.timeout` (milliseconds), else `executor.default_timeout`, clamped to whatever remains of the call tree's `global_deadline` (from `executor.global_timeout`). `0` means no per-module limit.
2. If the deadline has already passed when Step 8 is reached, `ModuleTimeoutError` (`MODULE_TIMEOUT`) is raised without invoking the module.
3. Otherwise the module runs under the timer. When the timer fires first, the executor raises `ModuleTimeoutError` immediately. It does **not** call `cancel()` on the context's `CancelToken`, and there is no grace period. What happens to the still-running module differs by language:

| SDK | Mechanism | Effect on the running module |
|-----|-----------|------------------------------|
| Python | `asyncio.wait_for` | An `async` module's coroutine is cancelled (`asyncio.CancelledError` at its current `await`). A sync module runs in a thread pool and cannot be interrupted; its thread runs to completion and the result is discarded. |
| TypeScript | `Promise.race` against a timer (and the token's abort signal) | The module's promise is not interrupted; it keeps running and its result is discarded. |
| Rust | `tokio::time::timeout` | The module future is dropped, so it stops at its current `.await`. |

[protocol-spec §12.7.5](../spec/protocol-spec.md) specifies a grace period before forced termination; no SDK implements it yet.

### Usage in Module Code

Modules performing long-running work **SHOULD** check the cancel token between units of work:

=== "Python"
    ```python
    import asyncio

    from apcore import Context, module


    async def process_item(item: str) -> str:
        await asyncio.sleep(0.1)  # stand-in for real work
        return item.upper()


    @module(id="data.process_batch", description="Process large data batch")
    async def process_batch(items: list[str], context: Context) -> dict:
        results = []
        for item in items:
            # Check cancellation before each unit of work
            if context.cancel_token is not None:
                context.cancel_token.check()
            results.append(await process_item(item))
        return {"processed": len(results), "results": results}
    ```
=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { APCore } from "apcore-js";
    import type { Context } from "apcore-js";

    async function processItem(item: string, signal: AbortSignal): Promise<string> {
        // Web-API I/O takes the signal and is aborted on cancel
        const res = await fetch(`https://example.com/items/${item}`, { signal });
        return res.text();
    }

    const client = new APCore();

    client.module({
        id: "data.process_batch",
        description: "Process large data batch",
        inputSchema: Type.Object({ items: Type.Array(Type.String()) }),
        outputSchema: Type.Object({
            processed: Type.Number(),
            results: Type.Array(Type.String()),
        }),
        execute: async (inputs, context: Context) => {
            const results: string[] = [];
            for (const item of inputs.items as string[]) {
                // Check cancellation before each unit of work
                context.cancelToken?.check();
                results.push(await processItem(item, context.signal));
            }
            return { processed: results.length, results };
        },
    });
    ```
=== "Rust"
    ```rust
    use apcore::{Context, Module, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct ProcessBatch;

    async fn process_item(item: &Value) -> Value {
        // stand-in for real work
        item.clone()
    }

    #[async_trait]
    impl Module for ProcessBatch {
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"items": {"type": "array"}}})
        }

        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"processed": {"type": "integer"}}})
        }

        fn description(&self) -> &str {
            "Process large data batch"
        }

        async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let items = inputs["items"].as_array().cloned().unwrap_or_default();
            let mut results = Vec::new();
            for item in &items {
                // Check cancellation before each unit of work
                if let Some(token) = &ctx.cancel_token {
                    token.check()?; // ExecutionCancelledError -> ModuleError via From
                }
                results.push(process_item(item).await);
            }
            Ok(json!({"processed": results.len(), "results": results}))
        }
    }
    ```

## Dependencies

- **Context** — Carries the `CancelToken` through the execution pipeline.
- **Core Executor** — Checks the token at Steps 2 and 8, and enforces timeouts at Step 8.
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
- **Pipeline check tests** verify that a token cancelled before the call raises `ExecutionCancelledError` at Step 2, and one cancelled during Steps 3–7 raises it at Step 8 without invoking the module.
- **Executor timeout tests** verify that an expired timer raises `ModuleTimeoutError` (`MODULE_TIMEOUT`).
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

Also exposed as `raise_if_cancelled()` (`raiseIfCancelled()` in TypeScript); both names have identical behaviour.

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

`reset()` clears the flag on the existing token; it **MUST NOT** substitute a fresh underlying primitive, because a consumer holding the old handle would never see a later `cancel()`. Where the platform cannot un-signal a handle (a TypeScript `AbortSignal`), the single handle is kept, the cooperative flag is authoritative, and the caller is warned once that a cancelled token is not reusable for signal-based work (D-90).

!!! note "Intended for testing/reuse, not for cancellation cleanup mid-call"
    `reset()` clears the cancelled flag on the existing token instance. Nothing observes the transition: a module that already raised `ExecutionCancelledError` from `check()` has already unwound, and the Executor does not re-poll a token after acting on cancellation. Resetting an in-flight token to "uncancel" a call in progress is not a supported pattern; `reset()` exists so a single `CancelToken` instance can be reused across independent test cases or task runs.
