# Middleware System

## Overview

Composable middleware pipeline using the onion execution model with before/after/on_error phases. Each middleware can inspect and modify inputs before module execution, transform outputs after execution, and participate in error recovery when failures occur. The pipeline supports both full subclass-based middleware and lightweight function adapters for simple use cases.

## Requirements

- Provide a base `Middleware` class with no-op defaults for all three lifecycle phases (before, after, on_error), allowing subclasses to override only the methods they need.
- Implement onion-model execution: before hooks run in registration order, after hooks run in reverse registration order, and on_error hooks run in reverse order over only the middlewares that executed before the failure. When middleware has explicit priority values (0-1000), higher priority executes first per PROTOCOL_SPEC. When priorities are equal, registration order applies.
- Support input modification in `before()` (return a new dict to replace inputs, or None to pass through unchanged) and output modification in `after()` (same contract).
- Support error recovery: `on_error()` handlers are called in reverse order; the first handler to return a non-None dict provides recovery output, short-circuiting the remaining handlers.
- Provide `BeforeMiddleware` and `AfterMiddleware` adapters that wrap plain callback functions as middleware instances, reducing boilerplate for single-phase hooks.
- Include a `LoggingMiddleware` with structured logging, security-aware redaction of inputs via `context.redacted_inputs`, and per-call duration tracking stored in `context.data`.
- Wrap before-phase failures in `MiddlewareChainError` carrying both the original exception and the list of executed middlewares, enabling targeted error recovery.
- Ensure all mutations to the middleware list are thread-safe.

## Technical Design

### Architecture

The middleware system follows a classic onion (layered) execution model. The `MiddlewareManager` holds an ordered list of `Middleware` instances and provides three execution methods corresponding to the module call lifecycle:

1. **`execute_before() -> tuple[dict, list[Middleware]]`** -- Iterates middlewares in registration order. Each middleware's `before()` receives the current inputs and may return a replacement dict. Returns both the (possibly modified) inputs and the list of executed middlewares (for error rollback in `execute_on_error`). If a middleware raises, a `MiddlewareChainError` is raised with the list of already-executed middlewares attached.

2. **`execute_after()`** -- Iterates middlewares in reverse registration order. Each middleware's `after()` receives both original inputs and the current output, and may return a replacement output dict.

3. **`execute_on_error(module_id, inputs, error, context, executed_middlewares)`** -- Iterates the `executed_middlewares` list (from the before phase) in reverse order. The first handler to return a non-None dict becomes the recovery output. If a handler itself raises, the exception is logged and iteration continues.

### Snapshot Pattern

The `MiddlewareManager` uses a lock-protected snapshot pattern for thread safety. Before each execution pass, `snapshot()` acquires the lock, copies the middleware list, and releases the lock. The execution then iterates over the snapshot without holding the lock, so concurrent `add()`/`remove()` calls do not interfere with in-flight pipelines.

### Components

- **`Middleware`** (base class) -- Plain class (not ABC) with three methods returning None by default. Subclasses override only what they need.
- **`MiddlewareManager`** -- Manages the ordered list and orchestrates the three execution phases. Uses a lock with the snapshot pattern for thread safety.
- **`BeforeMiddleware` / `AfterMiddleware`** -- Lightweight adapters wrapping a single callback function as a full `Middleware` subclass. Non-overridden phases remain no-ops.
- **`LoggingMiddleware`** -- Structured logging middleware that records start time in `context.data["_apcore.mw.logging.start_time"]` during `before()`, computes duration in `after()`, and uses `context.redacted_inputs` to avoid leaking sensitive data. Configurable via `log_inputs`, `log_outputs`, and `log_errors` flags.
- **`RetryMiddleware`** -- Built-in middleware that retries failed module calls with configurable backoff strategies (exponential or fixed). Only retries errors marked `retryable=True`. Supports `max_retries`, `base_delay_ms`, `max_delay_ms`, and jitter. See [Middleware Guide](../guides/middleware.md#54-retrymiddleware-built-in) for configuration details.
- **`MiddlewareChainError`** -- Exception subclass carrying `original` (the root cause) and `executed_middlewares` (the list of middlewares whose `before()` was called, for targeted error recovery).

### Data Flow

```
Inputs --> [MW1.before] --> [MW2.before] --> [MW3.before] --> Module.execute()
                                                                  |
Output <-- [MW1.after]  <-- [MW2.after]  <-- [MW3.after]  <------+

On Error (if MW3.before fails):
         [MW2.on_error] <-- [MW3.on_error]
         (MW1.on_error is not called because MW3 is where before failed,
          and recovery walks backwards through executed middlewares)
```

## Usage

=== "Python"
    ```python
    from apcore import APCore
    from apcore.middleware import Middleware, BeforeMiddleware, AfterMiddleware

    client = APCore()

    # Subclass-based middleware
    class AuditMiddleware(Middleware):
        def before(self, module_id, inputs, context):
            print(f"[AUDIT] calling {module_id}")

        def after(self, module_id, inputs, output, context):
            print(f"[AUDIT] {module_id} returned {output}")

    # Register middleware
    client.use(AuditMiddleware())

    # Lightweight function adapters
    client.use_before(lambda module_id, inputs, ctx: print(f"Before: {module_id}"))
    client.use_after(lambda module_id, inputs, out, ctx: print(f"After: {module_id}"))

    @client.module(id="greet", description="Say hello")
    def greet(name: str) -> dict:
        return {"message": f"Hello, {name}!"}

    result = client.call("greet", {"name": "World"})
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Middleware, BeforeMiddleware, AfterMiddleware } from "apcore-js";

    const client = new APCore();

    // Subclass-based middleware
    class AuditMiddleware extends Middleware {
        before(moduleId: string, inputs: Record<string, unknown>, context: unknown) {
            console.log(`[AUDIT] calling ${moduleId}`);
        }

        after(moduleId: string, inputs: Record<string, unknown>, output: Record<string, unknown>, context: unknown) {
            console.log(`[AUDIT] ${moduleId} returned`, output);
        }
    }

    // Register middleware
    client.use(new AuditMiddleware());

    // Lightweight function adapters
    client.useBefore((moduleId, inputs, ctx) => { console.log(`Before: ${moduleId}`); return null; });
    client.useAfter((moduleId, inputs, out, ctx) => { console.log(`After: ${moduleId}`); return null; });

    client.module({
        id: "greet",
        description: "Say hello",
        inputSchema: { type: "object", properties: { name: { type: "string" } } },
        outputSchema: { type: "object", properties: { message: { type: "string" } } },
        execute: ({ name }: { name: string }) => ({ message: `Hello, ${name}!` }),
    });

    const result = await client.call("greet", { name: "World" });
    ```
=== "Rust"
    ```rust
    use apcore::APCore;
    use apcore::middleware::{Middleware, MiddlewareContext};
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::Value;

    struct AuditMiddleware;

    #[async_trait]
    impl Middleware for AuditMiddleware {
        async fn before(
            &self,
            module_id: &str,
            inputs: &Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            println!("[AUDIT] calling {}", module_id);
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: &Value,
            output: &Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            println!("[AUDIT] {} returned {:?}", module_id, output);
            Ok(None)
        }
    }

    let mut client = APCore::new();
    client.use_middleware(Box::new(AuditMiddleware));
    ```

## Dependencies

- `apcore.context.Context` -- Execution context passed to all middleware methods, provides `trace_id`, `caller_id`, `redacted_inputs`, and `data` dict for per-call state storage.

??? info "Python SDK reference"
    The following tables are **not protocol requirements** — they document the Python SDK's source layout and runtime dependencies for implementers/users of `apcore-python`.

    **Source files:**

    | File | Lines | Purpose |
    |------|-------|---------|
    | `src/apcore/middleware/__init__.py` | 16 | Package re-exports for convenient imports |
    | `src/apcore/middleware/base.py` | 36 | `Middleware` base class with no-op defaults |
    | `src/apcore/middleware/manager.py` | 129 | `MiddlewareManager` and `MiddlewareChainError` |
    | `src/apcore/middleware/logging.py` | 94 | `LoggingMiddleware` with structured logging and redaction |
    | `src/apcore/middleware/adapters.py` | 43 | `BeforeMiddleware` and `AfterMiddleware` function adapters |
    | `src/apcore/middleware/retry.py` | ~190 | `RetryMiddleware` with configurable backoff strategies (exponential/fixed) |

    **Runtime dependencies:**

    - `threading` (stdlib) -- Lock for thread-safe middleware list management.
    - `logging` (stdlib) -- Standard library logging used by `LoggingMiddleware` and manager error reporting.
    - `time` (stdlib) -- Wall-clock timing for duration measurements in `LoggingMiddleware`.

## Testing Strategy

Tests are split across two files targeting different abstraction levels:

### Unit Tests (`tests/test_middleware.py`)
- **Middleware base class**: Verifies it is not an ABC, can be instantiated directly, all methods return None by default, and subclasses can selectively override methods.
- **BeforeMiddleware adapter**: Confirms it is a `Middleware` subclass, delegates `before()` to the callback, and leaves `after()`/`on_error()` as no-ops. Validates correct argument forwarding.
- **AfterMiddleware adapter**: Same structure as `BeforeMiddleware` tests but for the `after()` phase.

### Manager Tests (`tests/test_middleware_manager.py`)
- **add/remove**: Verifies append ordering, identity-based removal, and return values.
- **execute_before**: Tests registration-order execution, input replacement via returned dicts, None passthrough, `MiddlewareChainError` on failure with correct `executed_middlewares` tracking, and empty-list passthrough.
- **execute_after**: Tests reverse-order execution, output replacement, None passthrough, exception propagation, and empty-list passthrough.
- **execute_on_error**: Tests reverse iteration over executed middlewares, first-dict-wins recovery, None continuation, exception-in-handler logging and continuation, and empty-list returns None.
- **Thread safety**: Concurrent `add()` with no lost middlewares (10 threads x 50 adds), snapshot consistency after mutations, and concurrent `add()` + `snapshot()` with no exceptions (5 writer + 5 reader threads).

### Integration Tests (`tests/integration/test_middleware_chain.py`)
- Full pipeline tests exercising middleware through the `Executor.call()` path.

## Contract: Middleware.before

### Inputs
- `module_id` (str/string/&str, required) — ID of the module about to execute
- `inputs` (dict/object/Value, required) — module inputs (may be modified and returned)
- `context` (Context, required) — current execution context

### Errors
- Any error raised by the middleware propagates and aborts the execution pipeline (downstream middlewares' `before` hooks are skipped; `on_error` hooks of already-executed middlewares are invoked)

### Returns
- On success: `dict`/`Record<string, unknown>`/`Value` or None/null/() — modified inputs (or None to pass inputs unchanged)

### Properties
- async: language-dependent (Python allows sync or async; TypeScript and Rust MUST be async)
- thread_safe: true (called under executor lock on shared mutable state)
- pure: false (may mutate context or inputs)

## Contract: Middleware.after

### Errors
- Any error raised by the middleware: **behavior is SDK-defined**. Python and Rust propagate the first error immediately; TypeScript catches per-hook and rethrows the first error after all hooks have run. See PROTOCOL_SPEC.md §Middleware for the normative MUST once aligned.

### Inputs
- `module_id` (str/string/&str, required)
- `inputs` (dict/object/Value, required)
- `output` (dict/object/Value, required) — module output
- `context` (Context, required)

### Returns
- On success: `dict`/`Record<string, unknown>`/`Value` or None/null/() — modified output (or None to pass unchanged)

### Properties
- async: language-dependent
- thread_safe: true

## Contract: Middleware.on_error

### Inputs
- `module_id` (str/string/&str, required)
- `inputs` (dict/object/Value, required)
- `error` (ModuleError, required) — the error that terminated execution
- `context` (Context, required)

### Errors
- No errors raised (on_error MUST NOT raise)

### Returns
- On success with recovery: `dict`/`Record<string, unknown>`/`Value` — replacement output; **Python and TypeScript return immediately on first recovery value; Rust continues calling all hooks but keeps first recovery**. See note under `Middleware.after`.
- On pass-through: None/null/None — signals no recovery; error continues propagating

### Properties
- async: language-dependent
- thread_safe: true
