---
description: "Onion-model middleware: before/after/on_error phases, priority ordering, input/output mutation, error-recovery short-circuit, function adapters, thread-safe snapshot, logging/retry."
---

# Middleware System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §11.1 Middleware/Interceptors.


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
- Any error raised by the middleware propagates immediately (**fail-fast**): Python, TypeScript, and Rust all stop the `after` chain at the first error and propagate it without running the remaining hooks. See protocol-spec.md §Middleware for the normative MUST.

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

---

## Middleware Architecture Hardening (Issue #42)

### 1.1 Context Namespacing

Context keys in `context.data` are partitioned by namespace to prevent collisions between framework internals and user extensions.

**Normative rules:**

- Framework-owned keys MUST use the `_apcore.*` prefix (e.g., `_apcore.mw.logging.start_time`).
- User extensions MUST use the `ext.*` prefix (e.g., `ext.my_company.request_id`).
- Implementations MUST NOT write to keys in the other party's namespace. A framework implementation MUST NOT write `ext.*` keys; user middleware MUST NOT write `_apcore.*` keys.
- Keys that begin with neither prefix are allowed for backward compatibility but SHOULD be migrated to one of the two namespaces.

**Canonical `_apcore.*` keys:**

| Key | Set by | Value |
|-----|--------|-------|
| `_apcore.mw.logging.start_time` | `LoggingMiddleware.before()` | Wall-clock time (float, seconds since epoch) at the start of the module call |
| `_apcore.mw.tracing.span_id` | `TracingMiddleware.before()` | Active span ID string for the current module execution span |
| `_apcore.mw.circuit.state` | `CircuitBreakerMiddleware.before()` | Circuit state string: `CLOSED`, `OPEN`, or `HALF_OPEN` |

### 1.2 CircuitBreakerMiddleware

The `CircuitBreakerMiddleware` tracks per-module error rates and latencies, and opens a circuit when thresholds are exceeded, preventing calls to unhealthy modules.

**Normative rules:**

- `CircuitBreakerMiddleware` MUST track per-(`module_id`, `caller_id`) error statistics in a configurable rolling window.
- When the error rate in the window exceeds `open_threshold` (default: `0.5`), the circuit for that pair MUST transition to `OPEN`. In `OPEN` state, calls MUST be short-circuited and MUST raise `CircuitBreakerOpenError`.
- The circuit MUST transition to `HALF_OPEN` after `recovery_window_ms` (default: `30000`) has elapsed since the circuit opened. In `HALF_OPEN` state, exactly one probe call is allowed through. A successful probe transitions the circuit to `CLOSED`; a failed probe transitions it back to `OPEN`.
- `CircuitBreakerMiddleware` MUST store the current circuit state in `context.data["_apcore.mw.circuit.state"]` on every call.
- Implementations MUST emit `apcore.circuit.opened` and `apcore.circuit.closed` events via the `EventEmitter` when state transitions occur.

=== "Python"
    ```python
    from apcore import APCore
    from apcore.middleware import CircuitBreakerMiddleware

    client = APCore()

    client.use(CircuitBreakerMiddleware(
        open_threshold=0.3,        # open if >30% of calls fail
        recovery_window_ms=60000,  # probe after 60 seconds
        window_size=20,            # rolling window of 20 calls
    ))

    result = client.call("executor.payment.charge", {"amount": 100})
    ```
=== "TypeScript"
    ```typescript
    import { APCore, CircuitBreakerMiddleware } from "apcore-js";

    const client = new APCore();

    client.use(new CircuitBreakerMiddleware({
        openThreshold: 0.3,        // open if >30% of calls fail
        recoveryWindowMs: 60000,   // probe after 60 seconds
        windowSize: 20,            // rolling window of 20 calls
    }));

    const result = await client.call("executor.payment.charge", { amount: 100 });
    ```
=== "Rust"
    ```rust
    use apcore::APCore;
    use apcore::middleware::CircuitBreakerMiddleware;

    let mut client = APCore::new();

    client.use_middleware(Box::new(
        CircuitBreakerMiddleware::builder()
            .open_threshold(0.3)         // open if >30% of calls fail
            .recovery_window_ms(60_000)  // probe after 60 seconds
            .window_size(20)             // rolling window of 20 calls
            .build(),
    ));

    let result = client.call("executor.payment.charge", json!({ "amount": 100 })).await?;
    ```

### 1.3 TracingMiddleware (OpenTelemetry-Compatible)

The `TracingMiddleware` creates spans around module execution compatible with OTLP exporters.

**Normative rules:**

- `TracingMiddleware` MUST create a span in `before()` with span name equal to `module_id` and attributes `{ apcore.trace_id, apcore.caller_id, apcore.module_id }`.
- `TracingMiddleware` MUST end the span in `after()` with the execution result status (`ok` on success, `error` on failure).
- `TracingMiddleware` MUST store the span ID in `context.data["_apcore.mw.tracing.span_id"]`.
- `TracingMiddleware` SHOULD propagate W3C `traceparent` headers to outbound calls.
- `TracingMiddleware` MUST NOT raise if the OpenTelemetry SDK is not installed; in that case it MUST operate as a no-op.

=== "Python"
    ```python
    from apcore import APCore
    from apcore import TracingMiddleware

    # opentelemetry-api must be installed for active tracing;
    # if absent, TracingMiddleware silently becomes a no-op.
    client = APCore()

    client.use(TracingMiddleware(
        service_name="my-service",
        propagate_traceparent=True,
    ))

    result = client.call("executor.email.send_email", {"to": "user@example.com"})
    ```
=== "TypeScript"
    ```typescript
    import { APCore, TracingMiddleware } from "apcore-js";
    // @opentelemetry/api must be installed for active tracing;
    // if absent, TracingMiddleware silently becomes a no-op.

    const client = new APCore();

    client.use(new TracingMiddleware({
        serviceName: "my-service",
        propagateTraceparent: true,
    }));

    const result = await client.call("executor.email.send_email", { to: "user@example.com" });
    ```
=== "Rust"
    ```rust
    use apcore::APCore;
    use apcore::middleware::TracingMiddleware;
    // opentelemetry crate must be in Cargo.toml for active tracing;
    // if absent (feature-flag disabled), TracingMiddleware is a no-op.

    let mut client = APCore::new();

    client.use_middleware(Box::new(
        TracingMiddleware::builder()
            .service_name("my-service")
            .propagate_traceparent(true)
            .build(),
    ));

    let result = client.call("executor.email.send_email", json!({ "to": "user@example.com" })).await?;
    ```

### 1.4 Declarative Middleware Configuration (YAML-Driven)

Config-over-code: middleware chains SHOULD be configurable via `apcore.yaml` without writing any application code.

```yaml
middleware:
  - type: "tracing"
    match_modules: ["executor.*"]
  - type: "circuit_breaker"
    open_threshold: 0.3
    recovery_window_ms: 60000
  - type: "logging"
    log_inputs: true
    log_outputs: false
  - type: "custom"
    handler: "myapp.middleware.RateLimiter"
    config:
      requests_per_second: 100
```

**Normative rules:**

- Implementations MUST support at minimum the `tracing`, `circuit_breaker`, and `logging` built-in middleware types via YAML configuration.
- Custom middleware types MUST be resolvable via a dotted module path supplied in the `handler` field (e.g., `myapp.middleware.RateLimiter`). Implementations MUST raise a clear configuration error if the handler cannot be imported or does not implement the `Middleware` interface.
- The `match_modules` field, when present, restricts the middleware to module IDs matching the provided glob patterns. When absent, the middleware applies to all modules.

### 1.5 Async Handler Detection

Incorrect async detection causes middleware to be invoked synchronously when it should be awaited, silently swallowing results.

!!! warning
    Using `isawaitable(handler)` in Python always returns `False` for non-called functions — it tests whether an *object* is awaitable, not whether a *function* is a coroutine function. Use `inspect.iscoroutinefunction(handler)` instead.

**Normative rules:**

- **Python:** Implementations MUST use `inspect.iscoroutinefunction(handler)` to detect async handlers. Using `isawaitable(handler)` on an uncalled function is incorrect and MUST NOT be used for this purpose.
- **TypeScript:** Implementations MUST check `handler.constructor.name === 'AsyncFunction'` to detect async handlers before invocation. Checking `instanceof Promise` after invocation is too late (the function has already been called synchronously). Preferred approach: inspect the function itself via `handler.constructor.name`.
- **Rust:** Async handlers are statically typed via `async_trait`; no runtime detection is needed or possible.

## Contract: Middleware.detect_async

### Inputs
- `handler` (callable/Function/fn, required) — the middleware function to inspect

### Errors
- None

### Returns
- On success: bool/boolean/bool — `true` if the handler is asynchronous, `false` otherwise

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true

---

## Pipeline Step Middleware (Issue #33)

Module-level middleware (`before` / `after` / `on_error`) wraps the entire 11-step pipeline. **Pipeline step middleware** is a finer-grained extension point: it wraps the execution of a single named pipeline step (e.g., `input_validation`, `acl_check`, `execute`). This is the lifecycle-shaped surface for the step-level middleware concept introduced in [`core-executor.md` §1.3](./core-executor.md#13-step-level-middleware) — that section states the ordering guarantee relative to global middleware, while this section is the normative contract for the callbacks themselves. There is no `next`-callback wrapper form: the hooks observe the step, they do not wrap the call.

### Lifecycle

A registered `StepMiddleware` participates in three callbacks per step invocation:

```
before_step(step_name, state)
  --> step body executes
       |
       +-- on success: after_step(step_name, state, result)
       +-- on failure: on_step_error(step_name, state, error)
```

`state` is the `PipelineState` view — the step name, the outputs produced so far,
and the pipeline context. It is a single growable parameter rather than a
`ctx` / `inputs` pair, because **a step takes no `inputs` argument**: every step
is `execute(ctx)`, and reads whatever it needs off the context. All three SDKs
converged on this shape independently.

#### What `state.outputs` contains

`state.outputs` maps step name → that step's output, and it contains **exactly the
steps that completed before the current one**. The current step is **never** present,
in any of the three hooks:

| Hook | Why the current step is absent |
|---|---|
| `before_step` | It has not run. |
| `on_step_error` | It ran and failed; there is no output. |
| `after_step` | It succeeded, and its output is the `result` parameter. |

This is one rule with one meaning, deliberately, rather than the more obvious
"outputs of completed steps" — which would make `after_step` the single hook whose
`outputs` differs in shape, so a middleware reading `state.outputs` would first have
to know which hook it was in. The current step's output is not omitted from
`after_step` for lack of it; it is passed as `result`, and carrying the same value
down two paths is how the two drift apart.

Implementations **MUST NOT** insert the current step's output into `state.outputs`
before invoking `after_step`. Ordering the snapshot after the hook is what enforces
this, and it is what apcore-typescript and apcore-rust already do; apcore-python
snapshotted first and is corrected.

!!! warning "`run_until` is the deliberate exception — do not make it uniform"
    `PipelineState` has a second consumer: the `run_until` predicate, which is
    evaluated **after** a step completes and **does** see that step's output. That is
    the point of the predicate — it decides whether to stop *because of* what the step
    just produced, so hiding the value would make it useless.

    So the snapshot belongs **between** the `after_step` hook and the `run_until`
    predicate, not after both. The two consumers see deliberately different maps, and an
    implementation that "cleans this up" into one ordering breaks whichever it moves.
    apcore-python's `test_run_until_state_has_correct_outputs` pins the predicate's
    view; the fixture cases here pin the hooks'.

    A note on shape: `state.outputs` is a **live reference** to the engine's map in at
    least apcore-python and apcore-typescript, not a copy. Anything that stores the
    reference and reads it later sees the final map, not the map as it stood at the
    hook. Read it inside the hook.

### Normative Rules

- Implementations MUST provide a `StepMiddleware` extension point with three callbacks: `before_step`, `after_step`, and `on_step_error`. Each callback MAY be omitted (default no-op).
- `before_step(step_name, state)` MUST be invoked before the step body executes. It is an **observation** hook: there is no step-input parameter to replace, so its return value carries no meaning. Input rewriting is the module-level `Middleware.before` contract, which runs once per call rather than once per step.
- `after_step(step_name, state, result)` MUST be invoked after the step body completes successfully, with `result` a snapshot of the output the step produced.
- `on_step_error(step_name, state, error)` MUST be invoked when the step body raises. **Returning a non-null/non-None/`Some(...)` value MUST be treated as recovery output.** The error MUST NOT propagate further, the recovery value MUST become the step's output, and the pipeline MUST continue with the next step.
- Returning null/None/`None` from `on_step_error` MUST cause the original error to continue propagating (subject to the step's `ignore_errors` setting per [§1.1 Fail-Fast Error Handling](./core-executor.md#11-fail-fast-error-handling)).
- `after_step` MUST be invoked after a **recovered** step body as well as after a naturally successful one. A recovered step produced an output and the pipeline continued, so the onion MUST close: a middleware that acquired something in `before_step` MUST get its `after_step`, or the recovery path leaks.
- When multiple `StepMiddleware` instances are registered for the same step, `before_step` callbacks MUST run in registration order, and `after_step` callbacks MUST run in **reverse** registration order (onion model, identical to module-level middleware).
- `on_step_error` callbacks MUST run in reverse registration order over only the middlewares whose `before_step` had executed before the failure. The first non-null recovery value short-circuits remaining handlers (first-recovery-wins).
- Async `StepMiddleware` callbacks MUST be supported in all SDKs. Detection is an SDK-local concern and implementations MUST NOT be judged on the mechanism: apcore-python gates on `inspect.isawaitable()` applied to the **returned value** rather than `inspect.iscoroutinefunction()` on the callable, because the latter misses `functools.partial` and decorator wrappers (Issue #42); TypeScript awaits the return unconditionally; Rust uses `async_trait`. See [§1.5 Async Handler Detection](#15-async-handler-detection).

#### A `before_step` failure terminates the step — it is not recoverable

`before_step` raising is categorically different from the step body raising, and the two MUST NOT share a recovery path.

- An error raised by `before_step` MUST be wrapped in `MiddlewareChainError`, identical to the module-level contract.
- The step body MUST NOT execute.
- `on_step_error` MUST still be invoked, in reverse registration order, on the middlewares whose `before_step` had already been entered — **for observation and cleanup only**. Any value it returns **MUST be discarded**: it MUST NOT become the step's output and it MUST NOT allow the pipeline to continue.
- **First-recovery-wins MUST NOT apply to this pass.** Short-circuiting exists to stop shopping for a recovery once one is found; here no recovery is being sought, so **every** already-entered middleware MUST be notified. Stopping at the first middleware that happens to return a value would strand the cleanup of every middleware registered behind it — the opposite of what this pass is for.
- `after_step` MUST NOT be invoked for that step. No step body ran, so there is nothing to close over.
- The step's `ignore_errors` setting **MUST NOT apply**. `ignore_errors` declares that *this step's* failure is tolerable; a broken middleware chain is not a step failure. `MiddlewareChainError` MUST propagate regardless.

**Why recovery is forbidden here.** Honouring a recovery value would let the pipeline advance past a step whose body never ran. The built-in strategy places `acl_check` and `approval_gate` in that sequence, so a middleware that can make its own `before_step` raise and then return a value from `on_step_error` would **skip the ACL check or the approval gate outright** — a silent authorization bypass reachable from an extension point that is not supposed to carry authority. The onion is also torn at that moment: some middlewares have entered `before_step` and others have not, so no choice of `after_step` set preserves the enter/exit pairing.

!!! note "This is not the module-level contract, despite the shared vocabulary"
    Module-level `execute_on_error` **does** honour a recovery value raised during the before phase. That is consistent, because a module-level recovery value **terminates the call** — it *is* the return value and nothing further executes. A step-level recovery value **resumes a pipeline**. The two operations share a name and are not the same thing, so the module-level precedent MUST NOT be transferred.

### Configuration safety

Pipeline configuration MUST fail fast on structural errors so that misconfiguration is caught at startup rather than at first request.

- Implementations MUST raise `ConfigurationError` at YAML/config parse time when a key of the `pipeline.configure` map references a step that does not exist in the active strategy. Implementations MUST NOT silently ignore the directive or log a warning and continue.
- Implementations MUST raise `PipelineDependencyError` at strategy construction time (before any `call()` runs) when a step's declared `requires:` is not satisfied by an upstream step's `provides:`. The error message MUST include the unsatisfied capability name and the dependent step name.
- Implementations MUST NOT defer dependency validation until first invocation. Strategy construction MUST be all-or-nothing: a strategy either validates cleanly and is callable, or construction fails with a typed error.

```yaml
# apcore.yaml — pipeline step dependency contract
pipeline:
  configure:
    # A map keyed by step name, matching schemas/apcore-config.schema.json.
    input_validation:
      requires: ["context"]              # provided by context_creation
      provides: ["validated_inputs"]     # consumed by execute
```

There is no `pipeline.step_middleware:` configuration section. An earlier
revision of this document showed one; `schemas/apcore-config.schema.json`
declares `pipeline` as `additionalProperties: false` with only `remove`,
`configure` and `steps`, no SDK ever parsed it, and no rule above required it.
Register step middleware programmatically on the `ExecutionStrategy` instead —
see the examples below.

### Cross-language usage

A tracing-style `StepMiddleware` that logs the wall-clock duration of each step:

=== "Python"
    ```python
    import time
    from apcore import APCore, PipelineState, StepMiddleware, StepResult

    class TimingStepMiddleware(StepMiddleware):
        # All three hooks take the PipelineState view. There is no `inputs`
        # parameter — a Step is `execute(ctx)` — so per-step state is kept on
        # the middleware instance rather than threaded through the signature.
        def __init__(self) -> None:
            self._started: dict[str, float] = {}

        async def before_step(self, step_name: str, state: PipelineState) -> None:
            # Observation only: the engine discards whatever this returns.
            self._started[step_name] = time.perf_counter()

        async def after_step(self, step_name: str, state: PipelineState, result: StepResult) -> None:
            start = self._started.pop(step_name, None)
            if start is not None:
                elapsed_ms = (time.perf_counter() - start) * 1000
                print(f"step={step_name} elapsed_ms={elapsed_ms:.2f} action={result.action}")

        async def on_step_error(self, step_name: str, state: PipelineState, error: Exception) -> None:
            start = self._started.pop(step_name, None)
            elapsed_ms = (time.perf_counter() - start) * 1000 if start is not None else 0.0
            print(f"step={step_name} elapsed_ms={elapsed_ms:.2f} error={type(error).__name__}")
            return None  # do not recover; let the error propagate

    client = APCore()
    # Step middleware is registered on the execution strategy and runs for EVERY
    # step; filter by `step_name` inside the hook rather than binding to one step.
    client.executor.current_strategy.add_step_middleware(TimingStepMiddleware())

    @client.module(id="demo.greet", description="Greet the user")
    def greet(name: str) -> dict:
        return {"message": f"Hello, {name}!"}

    result = client.call("demo.greet", {"name": "World"})
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";
    import type { PipelineState, StepMiddleware } from "apcore-js";

    // StepMiddleware is an interface — implement it, do not extend it.
    // Every hook takes (stepName, state, …). There is no `inputs` parameter —
    // a Step is `execute(ctx)` — so per-step state lives on the instance.
    class TimingStepMiddleware implements StepMiddleware {
        private readonly started = new Map<string, number>();

        async beforeStep(stepName: string, state: PipelineState): Promise<void> {
            // Observation only: the engine ignores whatever this returns.
            this.started.set(stepName, performance.now());
        }

        async afterStep(stepName: string, state: PipelineState, result: unknown): Promise<void> {
            const start = this.started.get(stepName);
            this.started.delete(stepName);
            if (start !== undefined) {
                const elapsedMs = performance.now() - start;
                console.log(`step=${stepName} elapsed_ms=${elapsedMs.toFixed(2)}`);
            }
        }

        async onStepError(
            stepName: string,
            state: PipelineState,
            error: Error,
        ): Promise<unknown | null> {
            const start = this.started.get(stepName);
            this.started.delete(stepName);
            const elapsedMs = start !== undefined ? performance.now() - start : 0;
            console.log(`step=${stepName} elapsed_ms=${elapsedMs.toFixed(2)} error=${error.constructor.name}`);
            return null; // do not recover; let the error propagate
        }
    }

    const client = new APCore();

    // NOTE: apcore-typescript registers step middleware on `PipelineEngine`,
    // which the Executor holds privately — there is currently no public path to
    // it from the client. Track this gap before relying on step middleware in
    // TypeScript; Python and Rust expose it on the ExecutionStrategy.

    client.module({
        id: "demo.greet",
        description: "Greet the user",
        inputSchema: { type: "object", properties: { name: { type: "string" } } },
        outputSchema: { type: "object", properties: { message: { type: "string" } } },
        execute: ({ name }: { name: string }) => ({ message: `Hello, ${name}!` }),
    });

    const result = await client.call("demo.greet", { name: "World" });
    ```
=== "Rust"
    ```rust
    use apcore::{ModuleError, PipelineState, StepMiddleware};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Instant;

    // Every hook takes `&PipelineState<'_>`. There is no `inputs` parameter —
    // a Step is `execute(ctx)` — so per-step state lives on the middleware,
    // behind a Mutex because the trait takes `&self` and requires Send + Sync.
    #[derive(Default)]
    struct TimingStepMiddleware {
        started: Mutex<HashMap<String, Instant>>,
    }

    #[async_trait]
    impl StepMiddleware for TimingStepMiddleware {
        async fn before_step(
            &self,
            step_name: &str,
            _state: &PipelineState<'_>,
        ) -> Result<(), ModuleError> {
            // Observation only: `before_step` returns no value the engine reads.
            self.started
                .lock()
                .unwrap()
                .insert(step_name.to_string(), Instant::now());
            Ok(())
        }

        async fn after_step(
            &self,
            step_name: &str,
            _state: &PipelineState<'_>,
            _result: &Value,
        ) -> Result<(), ModuleError> {
            if let Some(start) = self.started.lock().unwrap().remove(step_name) {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                println!("step={step_name} elapsed_ms={elapsed_ms:.2}");
            }
            Ok(())
        }

        async fn on_step_error(
            &self,
            step_name: &str,
            _state: &PipelineState<'_>,
            error: &ModuleError,
        ) -> Result<Option<Value>, ModuleError> {
            self.started.lock().unwrap().remove(step_name);
            println!("step={} error={}", step_name, error.code);
            Ok(None) // do not recover; let the error propagate
        }
    }

    use apcore::{build_standard_strategy, Config, Executor, Registry};
    use std::sync::Arc;

    // `add_step_middleware` takes `&mut ExecutionStrategy`, and `Executor::strategy()`
    // hands out only `&` — so build and populate the strategy BEFORE constructing
    // the executor. Step middleware runs for every step; filter on `step_name`.
    let mut strategy = build_standard_strategy();
    strategy.add_step_middleware(Arc::new(TimingStepMiddleware::default()));

    let executor = Executor::with_strategy(Registry::new(), Config::from_defaults(), strategy);
    ```

### Contract: StepMiddleware.before_step

#### Inputs
- `step_name` (str/string/&str, required) — pipeline step name (e.g., `input_validation`)
- `state` (PipelineState, required) — the step name, the outputs produced so far, and the pipeline context

There is **no** `inputs` parameter. A Step is `execute(ctx)`; it reads what it needs off the context, so there is nothing for a middleware to be handed or to replace.

#### Errors
- Any error raised aborts the step body and triggers `on_step_error` callbacks of already-executed step middlewares (mirrors `MiddlewareChainError` for the module-level chain)

#### Returns
- None/void. `before_step` is an **observation** hook: any value returned is discarded and MUST NOT change what the step body sees. Input rewriting is the module-level `Middleware.before` contract.

#### Properties
- async: language-dependent (Python sync or async; TypeScript and Rust MUST be async)
- thread_safe: true
- pure: false (may mutate the context reachable through `state`)

### Contract: StepMiddleware.after_step

#### Inputs
- `step_name`, `state` (same as `before_step`)
- `result` (StepResult/unknown/&Value, required) — snapshot of the output the step produced

#### Errors
- Behavior is SDK-defined (see Middleware.after for parity rule)

#### Returns
- None/void. `after_step` observes; it does not replace the step output.

#### Properties
- async: language-dependent
- thread_safe: true

### Contract: StepMiddleware.on_step_error

#### Inputs
- `step_name`, `state` (same as `before_step`)
- `error` (ModuleError, required) — the error raised by the step body

#### Errors
- on_step_error MUST NOT raise; exceptions inside the handler MUST be logged and iteration continues with the next handler

#### Returns
- On success with recovery: dict/object/Value — replacement output, short-circuits remaining handlers
- On pass-through: null/None/None — error continues propagating

#### Properties
- async: language-dependent
- thread_safe: true

---

## Async middleware correctness

The `iscoroutinefunction`-style detection in §1.5 is **necessary but not sufficient**. Higher-order wrappers like Python's `functools.partial`, JavaScript closures returned from factories, or class methods rebound onto instances are not literally `async def` / `async function`, yet they MAY return a coroutine / Promise when invoked. Function-shape inspection misses these cases and silently drops the awaited result.

To preserve correctness across all wrapper styles, middleware managers MUST detect awaitability on the **return value** of each `before` / `after` / `on_error` invocation, not on the function shape alone.

### Normative rules

- Implementations MUST inspect each handler's RETURN value: if the returned object is awaitable (Python: `inspect.isawaitable(value)`) or a thenable (TypeScript: object with a callable `.then` method) or a future (Rust: a `Future` resolved by the runtime), the middleware manager MUST `await` it before proceeding to the next phase.
- Implementations MUST NOT rely solely on `iscoroutinefunction` / `handler.constructor.name === 'AsyncFunction'` as the gating check. These checks SHOULD remain as a fast-path optimization but MUST be supplemented by return-value detection.
- The function-shape check from §1.5 is RETAINED as guidance for the warning case (e.g., `async def` declared but never awaited at the call site), but the **authoritative** decision MUST use the actual return value.
- Synchronous middleware (return value is a plain `dict` / `null` / `()`) MUST NOT be awaited; the manager MUST forward the value as-is.

### Rationale

```python
import functools

async def _audit(module_id, inputs, ctx, *, source):
    print(f"[{source}] {module_id}")
    return None

# This is NOT a coroutine function — iscoroutinefunction returns False.
# But calling it returns a coroutine. Old detection silently swallows it.
audit = functools.partial(_audit, source="api")

client.use_before(audit)
```

The same pattern occurs in TypeScript when a factory returns an arrow function whose body is `async` (the wrapper is sync, the body is async), or when a class method is wrapped in a decorator that re-binds `this`.

=== "Python"
    ```python
    import asyncio
    import functools
    import inspect
    from typing import Any, Callable

    async def invoke_handler(handler: Callable[..., Any], *args: Any) -> Any:
        # Fast path: known async function. Optimization, not the gate.
        result = handler(*args)
        # Authoritative check: did we get an awaitable back?
        if inspect.isawaitable(result):
            return await result
        return result

    # Works for: async def, functools.partial(async_fn, ...),
    # decorated methods, lambda returning a coroutine, etc.
    async def main():
        async def audit(module_id, inputs, ctx, *, source):
            return None

        wrapped = functools.partial(audit, source="api")
        await invoke_handler(wrapped, "math.add", {"a": 1}, None)
    ```
=== "TypeScript"
    ```typescript
    type Handler = (...args: unknown[]) => unknown;

    function isThenable(v: unknown): v is PromiseLike<unknown> {
        return (
            typeof v === "object" &&
            v !== null &&
            typeof (v as { then?: unknown }).then === "function"
        );
    }

    async function invokeHandler(handler: Handler, ...args: unknown[]): Promise<unknown> {
        // Authoritative check: inspect the RETURN value, not handler.constructor.name.
        const result = handler(...args);
        if (isThenable(result)) {
            return await result;
        }
        return result;
    }

    // Works for: async function, () => somePromise, partial(asyncFn, ...),
    // decorated class methods rebound via .bind(this), etc.
    const wrapped = (...args: unknown[]) => audit("api", ...args);
    await invokeHandler(wrapped, "math.add", { a: 1 }, null);
    ```
=== "Rust"
    ```rust
    // Rust handles this statically: every Middleware trait method returns
    // `impl Future` via #[async_trait]. The compiler ENFORCES that callers
    // .await the result; there is no runtime "shape" detection and no way
    // to silently drop a Future. The async-correctness bug class is impossible.
    //
    // Higher-order wrappers (closures, partials, decorators) preserve the
    // Future signature through the trait bound, so they remain awaitable
    // by construction.

    use async_trait::async_trait;
    use apcore::middleware::Middleware;
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use serde_json::Value;

    struct Audit;

    #[async_trait]
    impl Middleware for Audit {
        async fn before(
            &self,
            module_id: &str,
            _inputs: &Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            println!("[audit] {module_id}");
            Ok(None)
        }
    }
    ```

### Migration note

Implementations that previously gated awaiting on `iscoroutinefunction` / `handler.constructor.name === 'AsyncFunction'` MUST switch to return-value inspection. The fix is backward-compatible: synchronous handlers that return non-awaitable values are unaffected; awaitable returns from non-async-declared functions are now correctly awaited instead of leaked.

## Duplicate Middleware Detection (Issue #64)

The middleware registration API accepts duplicate registrations silently. Two instances of the same middleware class — for example, two `RetryMiddleware` instances registered by different components — both fire in sequence, producing surprising compound behavior. A user expecting "retry up to 3 times" can end up with "retry up to 9 times" because two layers each multiply the attempts.

The most common collision pattern:

- An application uses a framework integration (e.g., `django-apcore`) that auto-registers resilience middleware at startup.
- The application also explicitly registers its own resilience middleware.
- Neither side knows about the other; both fire on every call.

Diagnosing this today requires runtime tracing. There is no startup-time signal that something is doubled up.

### Normative Rules

**Detection (SHOULD).** When a middleware is registered, SDKs SHOULD detect prior registrations that share the same identity and emit a warning. Identity is computed as follows:

| Language | Default identity |
|----------|------------------|
| Python | `f"{type(mw).__module__}.{type(mw).__qualname__}"` |
| TypeScript | `` `${moduleSpecifier}:${ClassName}` `` when the module specifier is statically recoverable, falling back to the constructor name only |
| Rust | `std::any::type_name::<T>()` |

A middleware MAY override the default identity by providing an explicit `identity_key` (string) at registration time. This is useful when the same class is intentionally used with different configurations (e.g., one `RetryMiddleware` for HTTP modules and another for database modules — both legitimate, but they share a class).

**Warning content (MUST).** When a duplicate is detected and no opt-out is set, the SDK MUST emit a `WARNING`-level log entry that includes:

- The identity string of the duplicate.
- The registration site of the first instance (caller frame, file:line, or stack info if available at acceptable cost).
- The registration site of the duplicate.

The warning MUST be non-blocking: the registration itself MUST succeed. Detection is observability, not enforcement.

**Order preservation (MUST).** Registration order MUST be preserved. SDKs MUST NOT dedup, reorder, or otherwise mutate the middleware chain in response to a duplicate detection. The actual chain runs both instances in registration order.

**Opt-out (SHOULD).** SDKs SHOULD provide a per-registration flag to suppress the warning when stacking is intentional:

| Language | Flag |
|----------|------|
| Python | `register_middleware(mw, allow_duplicate=True)` (or equivalent on `client.use`) |
| TypeScript | `client.use(mw, { allowDuplicate: true })` |
| Rust | builder: `.allow_duplicate(true)` |

**Identity-key namespace (SHOULD).** When using `identity_key`, third parties SHOULD prefix with their vendor namespace (`myapp.retry.http`, `my-vendor.observability.tracing`, etc.). Keys starting with `apcore.` are reserved for framework-provided middleware.

### Examples

=== "Python"

    ```python
    from apcore import APCore
    from apcore.middleware import RetryMiddleware

    client = APCore()

    # First registration — no warning.
    client.use(RetryMiddleware(max_attempts=3))

    # Second registration — emits WARNING naming both call sites.
    client.use(RetryMiddleware(max_attempts=2))

    # Intentional stacking — pass allow_duplicate=True to silence.
    client.use(RetryMiddleware(max_attempts=5), allow_duplicate=True)

    # Two instances of the same class with distinct identity_key — NOT a duplicate.
    client.use(RetryMiddleware(max_attempts=3), identity_key="myapp.retry.http")
    client.use(RetryMiddleware(max_attempts=2), identity_key="myapp.retry.db")
    ```

=== "TypeScript"

    ```typescript
    import { APCore, RetryMiddleware } from "apcore-js";

    const client = new APCore();

    // First registration — no warning.
    client.use(new RetryMiddleware({ maxAttempts: 3 }));

    // Second registration — emits WARNING naming both call sites.
    client.use(new RetryMiddleware({ maxAttempts: 2 }));

    // Intentional stacking — pass allowDuplicate.
    client.use(new RetryMiddleware({ maxAttempts: 5 }), { allowDuplicate: true });

    // Distinct identity_key — NOT a duplicate.
    client.use(new RetryMiddleware({ maxAttempts: 3 }), { identityKey: "myapp.retry.http" });
    client.use(new RetryMiddleware({ maxAttempts: 2 }), { identityKey: "myapp.retry.db" });
    ```

=== "Rust"

    ```rust
    use apcore::middleware::{RetryMiddleware, MiddlewareRegistration};
    use apcore::APCore;

    let client = APCore::new();

    // First registration — no warning.
    client.use_middleware(RetryMiddleware::new(3));

    // Second registration — emits WARNING naming both call sites.
    client.use_middleware(RetryMiddleware::new(2));

    // Intentional stacking.
    client.use_middleware(
        MiddlewareRegistration::new(RetryMiddleware::new(5))
            .allow_duplicate(true),
    );

    // Distinct identity_key — NOT a duplicate.
    client.use_middleware(
        MiddlewareRegistration::new(RetryMiddleware::new(3))
            .identity_key("myapp.retry.http"),
    );
    client.use_middleware(
        MiddlewareRegistration::new(RetryMiddleware::new(2))
            .identity_key("myapp.retry.db"),
    );
    ```

### Out of Scope

!!! info "What this rule does NOT catch"
    Detection of *behaviorally* overlapping middleware that do not share class identity (e.g., two independently-implemented retry libraries with different class names but equivalent intent) is not a problem the framework can solve generically. Reviewers and integration tests remain the safety net for that class of collision.

A stricter "reject duplicates outright" mode is not part of this requirement. SDKs MAY add an optional mode in a future release; this section reserves the design space without mandating it.
