---
description: "Onion-model middleware: before/after/on_error, priority ordering, input/output replacement, recovery and retry, function adapters, context namespacing, built-ins, and pipeline step middleware."
---

# Middleware System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §11.1 Middleware/Interceptors. This page is the canonical reference for middleware contracts; the [Writing Middleware](../guides/middleware.md) guide builds on it.


## Overview

Middleware wraps every module call in an onion: `before()` hooks run on the way in, `after()` hooks on the way out, and `on_error()` hooks when the call fails. A middleware can inspect or replace the inputs, replace the output, recover from an error, or ask for the call to be retried. Module-level middleware runs as two pipeline steps — `middleware_before` (Step 6) and `middleware_after` (Step 10) of the [execution pipeline](./core-executor.md#execution-pipeline). A finer-grained hook, [pipeline step middleware](#pipeline-step-middleware), observes each individual pipeline step.

## Requirements

- Provide a `Middleware` base (Python/TypeScript class with no-op defaults, Rust trait) with `before`, `after` and `on_error`.
- Order middleware by `priority` (0–1000, default 100): higher runs first in the before phase; equal priorities keep registration order. A priority above 1000 is rejected at registration.
- Run `before()` in that order, `after()` in reverse order, and `on_error()` in reverse order over every middleware whose `before()` was entered.
- Let `before()` replace the inputs and `after()` replace the output (return a new map, or `None`/`null` to pass through).
- Let `on_error()` recover (return an output) or request a retry (`RetrySignal`); the first handler that does either wins.
- Provide function adapters (`BeforeMiddleware` / `AfterMiddleware`) for single-phase hooks.
- Wrap before-phase failures in `MiddlewareChainError` carrying the original error and the executed middlewares.
- Keep registration and removal thread-safe without blocking in-flight calls.

## Technical Design

### Onion Execution

The `MiddlewareManager` holds the ordered chain and runs three phases:

1. **`execute_before`** — Calls each middleware's `before(module_id, inputs, context)` in chain order. A non-null return replaces the inputs for the rest of the chain. The manager records each middleware as *executed* **before** calling it, so if `before()` raises, the list includes the middleware that raised. The failure is wrapped in `MiddlewareChainError(original, executed_middlewares)`.
2. **`execute_after`** — Calls `after(module_id, inputs, output, context)` in reverse chain order. A non-null return replaces the output. The first error stops the phase (fail-fast): the remaining `after()` hooks are skipped and the error goes through the call's `on_error` chain.
3. **`execute_on_error`** — Calls `on_error(module_id, inputs, error, context)` in reverse order over the executed list. The first handler that returns a recovery value or a `RetrySignal` ends the phase. A handler that itself raises is logged and skipped.

```text
Inputs --> [MW1.before] --> [MW2.before] --> [MW3.before] --> Module.execute()
                                                                  |
Output <-- [MW1.after]  <-- [MW2.after]  <-- [MW3.after]  <------+

If MW3.before raises:
    MW3.on_error --> MW2.on_error --> MW1.on_error
    (every middleware whose before() was entered, the failing one first;
     the first recovery value or RetrySignal stops the walk)

If the module raises:
    MW3.on_error --> MW2.on_error --> MW1.on_error
```

This unwinding rule is identical in all three SDKs. It is what lets a middleware that rejects a call in its own `before()` — `CircuitBreakerMiddleware` raising `CIRCUIT_BREAKER_OPEN`, for example — observe that rejection in its `on_error()`.

**Recovery and retry.** A recovery value becomes the call's output; the remaining `on_error` hooks and the rest of the pipeline do not run. A `RetrySignal(inputs)` (Rust: `OnErrorOutcome::Retry` from `on_error_outcome`) makes the Executor re-run the pipeline with those inputs; it is ignored mid-stream, where retrying is not meaningful. `ExecutionCancelledError` never reaches `on_error` ([Cancellation Short-Circuit](./core-executor.md#cancellation-short-circuit)), and the Executor surfaces the typed cause of a `MiddlewareChainError` to the caller ([Error Unwrap Rule](./core-executor.md#error-unwrap-rule)).

### Snapshot Pattern

Each phase takes a snapshot of the chain under a lock and iterates the copy without holding it, so a concurrent `use()` / `remove()` never disturbs a call in flight and never waits for one. Middleware instances are shared by concurrent calls: per-call state MUST live in `context.data`, not on the instance ([§12.7.6](../spec/protocol-spec.md#1276-middleware-chain-atomicity)).

### Sync and Async Hooks

- **Python** hooks may be plain functions or coroutines. The manager decides by the **return value** (`inspect.isawaitable`), not by the function's shape, so `functools.partial` objects and decorated coroutines are awaited correctly. On the async path, a synchronous `on_error()` runs in a worker thread so a blocking retry delay does not stall the event loop.
- **TypeScript** awaits every hook's return value, so sync, `async` and promise-returning hooks all work.
- **Rust** hooks are `async fn` (`#[async_trait]`); the compiler enforces the await.

### Components

| Component | Purpose |
|-----------|---------|
| `Middleware` | Base with `before` / `after` / `on_error`. Python and TypeScript: a class with no-op defaults and a `priority` constructor argument. Rust: a trait (`Send + Sync + Debug`) whose `name`, `before`, `after` and `on_error` are all required; `priority()` defaults to 100 and `on_error_outcome()` wraps `on_error`. |
| `MiddlewareManager` | Holds the ordered chain; runs the three phases; detects duplicate registrations. |
| `BeforeMiddleware` / `AfterMiddleware` | Python/TypeScript classes wrapping a single callback (`use_before` / `use_after` build them, priority 100). In Rust these are single-method **traits** accepted by `use_before` / `use_after`; `BeforeAdapter` / `AfterAdapter` wrap an async closure as a full `Middleware`. |
| `RetrySignal` | Returned from `on_error` to request a retry with new inputs. |
| `MiddlewareChainError` | `MIDDLEWARE_CHAIN_ERROR`: carries `original` and `executed_middlewares`. |

### Registration and Removal

| Operation | Python | TypeScript | Rust |
|-----------|--------|------------|------|
| Add | `client.use(mw)` | `client.use(mw)` | `client.use_middleware(Box::new(mw))?` |
| Add a callback | `client.use_before(fn)` / `use_after(fn)` | `client.useBefore(fn)` / `useAfter(fn)` | `client.use_before(Box::new(impl BeforeMiddleware))?` |
| Remove exactly one | `client.remove(mw)` (identity) | `client.remove(mw)` (identity) | `let h = client.use_middleware_handle(Box::new(mw))?; client.remove_handle(h)` |

Python/TypeScript callbacks take `(module_id, inputs, context)` for before and `(module_id, inputs, output, context)` for after, and return a replacement map or `None`/`null`. Removal semantics are specified in [Contract: APCore.remove](./apcore-client.md#contract-apcoreremove).

### Duplicate Registration

Two instances of the same middleware class — say one `RetryMiddleware` installed by a framework integration and another by the application — both run, compounding their effect. Registration therefore detects duplicates by **identity** and warns:

| Language | Default identity |
|----------|------------------|
| Python | `f"{type(mw).__module__}.{type(mw).__qualname__}"` |
| TypeScript | the constructor name |
| Rust | `std::any::type_name::<T>()` |

Rules:

- A duplicate MUST produce a `WARNING` naming the identity and both registration sites, and the registration MUST still succeed. The chain is never deduplicated or reordered.
- A per-registration opt-out suppresses the warning, and an explicit identity key distinguishes two intended instances of one class. Keys starting with `apcore.` are reserved for framework middleware; third parties SHOULD prefix theirs with a vendor namespace (`myapp.retry.http`).
- Removing a middleware MUST clear its duplicate-detection entry unless another registration still holds the same identity, so a `use` / `remove` / `use` swap does not warn (D-114).

`APCore.use()` takes a single argument. The opt-out and the identity key are options of the manager's registration call:

| Language | Call |
|----------|------|
| Python | `MiddlewareManager.use(mw, allow_duplicate=True, identity_key="myapp.retry.http")` — `APCore.use()` / `Executor.use()` detect duplicates with the default identity |
| TypeScript | `MiddlewareManager.add(mw, { allowDuplicate: true, identityKey: "myapp.retry.http" })` — `APCore.use()` / `Executor.use()` detect duplicates with the default identity |
| Rust | `MiddlewareManager::add_with_opts(MiddlewareRegistration::new(mw).allow_duplicate(true).identity_key("myapp.retry.http"))` — `APCore::use_middleware()` registers without duplicate detection |

### Context Namespacing

Middleware keeps per-call state in `context.data`, which the framework, third-party packages and the application share. The key space is partitioned:

- Framework-owned keys use the `_apcore.` prefix (`_apcore.mw.logging.start_time`, `_apcore.mw.tracing.spans`, `_apcore.mw.circuit.state`).
- Third-party middleware — anything shipped for reuse — uses `ext.<vendor>.` (`ext.acme.request_id`).
- Unprefixed keys belong to the application's own code (`locale`, `x-correlation-id`).
- The framework MUST NOT write `ext.*` keys; user and third-party code MUST NOT write `_apcore.*` keys.

Each SDK exports the check: `validate_context_key(writer, key)` returns `{valid, warning}` for a `"framework"` or `"user"` writer (Python and Rust also `enforce_context_key`, which logs the violation; TypeScript `validateContextKey`). Keys starting with `_` are not serialized across processes. For typed access use `ContextKey[T]` — see [Context Object](./context-object.md#data-key-convention).

Framework-owned middleware keys:

| Key | Set by | Value |
|-----|--------|-------|
| `_apcore.mw.logging.start_time` | `LoggingMiddleware.before()` | Wall-clock start time (epoch seconds) |
| `_apcore.mw.tracing.spans` | `TracingMiddleware.before()` | Stack of active spans; each entry links `parent_span_id` to the one below, so nested calls do not overwrite each other |
| `_apcore.mw.tracing.sampled` | `TracingMiddleware.before()` | The sampling decision for this trace |
| `_apcore.mw.circuit.state` | `CircuitBreakerMiddleware.before()` | `CLOSED`, `OPEN` or `HALF_OPEN` |
| `_apcore.mw.retry.count.{module_id}` | `RetryMiddleware.on_error()` | Retries attempted for that module in this call tree |

## Built-in Middleware

| Middleware | What it does | Installed |
|------------|--------------|-----------|
| `TracingMiddleware` | One span per call, W3C trace context, sampling strategies | Automatically at client construction when `observability.tracing.enabled: true`, or with `use()` |
| `ObsLoggingMiddleware` | One structured execution record per call (inputs, outputs, timing), redacted per `obs.redaction.*` | With `use()` |
| `LoggingMiddleware` | Start/end/error log lines using `context.redacted_inputs` (priority 700). Deprecated in Python and TypeScript in favour of `ObsLoggingMiddleware` | With `use()` |
| `MetricsMiddleware` | Call counts and latency into a `MetricsCollector` | With `use()` |
| `UsageMiddleware`, `ErrorHistoryMiddleware` | Usage statistics and recent-error history for the system modules | By the system modules (`sys_modules.enabled: true`) |
| `PlatformNotifyMiddleware` | Emits `apcore.health.*` threshold events | By the system modules when `sys_modules.events.enabled: true` |
| `RetryMiddleware` | Retries retryable errors with backoff | With `use()` |
| `CircuitBreakerMiddleware` | Short-circuits calls to an unhealthy module | With `use()` |

Tracing and logging are described in [Observability](./observability.md), metrics and usage in [Metrics and Usage](./metrics-and-usage.md), error history in [Error History](./error-history.md), and the health events in [Event System](./event-system.md).

### TracingMiddleware from Configuration

When the loaded Config sets `observability.tracing.enabled: true`, the client installs one `TracingMiddleware` ([Observability § Tracing from configuration](./observability.md#tracing-from-configuration)) built from `observability.tracing.strategy`, `sampling_rate`, `exporter` (`stdout` default, `otlp`) and `otlp_endpoint` ([§10.1.1](../spec/protocol-spec.md#1011-tracing-from-configuration-observabilitytracing)). Nothing is installed for a caller-supplied Executor, and configuration never adds a second tracing middleware.

```yaml
observability:
  tracing:
    enabled: true
    strategy: proportional
    sampling_rate: 0.1
    exporter: otlp
    otlp_endpoint: "http://collector:4318/v1/traces"
```

### RetryMiddleware

`RetryMiddleware` acts in `on_error`: when the error is retryable (`error.retryable` is true), it waits for the backoff delay and returns a `RetrySignal` with the original inputs, so the Executor re-runs the pipeline. After `max_retries` retries, or for a non-retryable error, it returns nothing and the error propagates. The retry count is kept in `context.data["_apcore.mw.retry.count.{module_id}"]` and cleared on success.

| `RetryConfig` field | Default | Meaning |
|---------------------|---------|---------|
| `max_retries` | 3 | Retries after the first attempt |
| `strategy` | `"exponential"` | `"exponential"` (`base_delay_ms × 2^attempt`, capped) or `"fixed"` |
| `base_delay_ms` | 100 | Base delay |
| `max_delay_ms` | 5000 | Cap for exponential backoff |
| `jitter` | `true` | Randomize each delay (×0.5–1.5) |

=== "Python"
    ```python
    from apcore import APCore
    from apcore.middleware import RetryConfig, RetryMiddleware

    client = APCore()
    client.use(RetryMiddleware(RetryConfig(max_retries=5, strategy="fixed", base_delay_ms=200)))
    ```
=== "TypeScript"
    ```typescript
    import { APCore, RetryMiddleware } from "apcore-js";

    const client = new APCore();
    client.use(new RetryMiddleware({ maxRetries: 5, strategy: "fixed", baseDelayMs: 200 }));
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::middleware::{RetryConfig, RetryMiddleware};
    use apcore::APCore;

    fn main() -> Result<(), ModuleError> {
        let mut config = RetryConfig::default(); // #[non_exhaustive]: start from Default
        config.max_retries = 5;
        config.strategy = "fixed".to_string();
        config.base_delay_ms = 200;

        let client = APCore::new();
        client.use_middleware(Box::new(RetryMiddleware::new(config)))?;
        Ok(())
    }
    ```

### CircuitBreakerMiddleware

`CircuitBreakerMiddleware` tracks outcomes per (`module_id`, `caller_id`) pair in a rolling window:

- The circuit opens when the window holds at least `min_samples` outcomes (default 5) and the error rate reaches `open_threshold` (default 0.5); the window keeps the last `window_size` outcomes (default 20).
- While `OPEN`, `before()` raises `CircuitBreakerOpenError` (`CIRCUIT_BREAKER_OPEN`, retryable) without calling the module.
- After `recovery_window_ms` (default 30 000) the circuit is `HALF_OPEN` and admits exactly one probe; success closes it, failure reopens it.
- The state is written to `context.data["_apcore.mw.circuit.state"]` on every call, and `apcore.circuit.opened` / `apcore.circuit.closed` are emitted when an event emitter is supplied.

=== "Python"
    ```python
    from apcore import APCore
    from apcore.middleware import CircuitBreakerMiddleware

    client = APCore()
    client.use(CircuitBreakerMiddleware(
        open_threshold=0.3,        # open at a 30% error rate
        recovery_window_ms=60000,  # probe after 60 seconds
        window_size=20,            # rolling window of 20 outcomes
    ))
    ```
=== "TypeScript"
    ```typescript
    import { APCore, CircuitBreakerMiddleware } from "apcore-js";

    const client = new APCore();
    client.use(new CircuitBreakerMiddleware({
        openThreshold: 0.3,        // open at a 30% error rate
        recoveryWindowMs: 60000,   // probe after 60 seconds
        windowSize: 20,            // rolling window of 20 outcomes
    }));
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::middleware::CircuitBreakerMiddleware;
    use apcore::APCore;

    fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        client.use_middleware(Box::new(
            CircuitBreakerMiddleware::builder()
                .open_threshold(0.3)         // open at a 30% error rate
                .recovery_window_ms(60_000)  // probe after 60 seconds
                .window_size(20)             // rolling window of 20 outcomes
                .build(),
        ))?;
        Ok(())
    }
    ```

Pass the client's event emitter (`emitter=` / `emitter:` / `.emitter(...)`) to publish the circuit events.

## Usage

=== "Python"
    ```python
    from apcore import APCore, Context
    from apcore.middleware import Middleware

    client = APCore()

    class AuditMiddleware(Middleware):
        def before(self, module_id: str, inputs: dict, context: Context) -> dict | None:
            print(f"[AUDIT] calling {module_id}")
            return None

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> dict | None:
            print(f"[AUDIT] {module_id} returned {output}")
            return None

    client.use(AuditMiddleware())

    # Single-phase callbacks
    client.use_before(lambda module_id, inputs, context: print(f"before: {module_id}"))
    client.use_after(lambda module_id, inputs, output, context: print(f"after: {module_id}"))

    @client.module(id="demo.greet", description="Say hello")
    def greet(name: str) -> dict:
        return {"message": f"Hello, {name}!"}

    print(client.call("demo.greet", {"name": "World"}))
    ```
=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { APCore, Middleware, type Context } from "apcore-js";

    const client = new APCore();

    class AuditMiddleware extends Middleware {
        override before(moduleId: string, _inputs: Record<string, unknown>, _context: Context): null {
            console.log(`[AUDIT] calling ${moduleId}`);
            return null;
        }

        override after(
            moduleId: string,
            _inputs: Record<string, unknown>,
            output: Record<string, unknown>,
            _context: Context,
        ): null {
            console.log(`[AUDIT] ${moduleId} returned`, output);
            return null;
        }
    }

    client.use(new AuditMiddleware());

    // Single-phase callbacks
    client.useBefore((moduleId) => { console.log(`before: ${moduleId}`); return null; });
    client.useAfter((moduleId) => { console.log(`after: ${moduleId}`); return null; });

    client.module({
        id: "demo.greet",
        description: "Say hello",
        inputSchema: Type.Object({ name: Type.String() }),
        outputSchema: Type.Object({ message: Type.String() }),
        execute: (inputs) => ({ message: `Hello, ${String(inputs.name)}!` }),
    });

    console.log(await client.call("demo.greet", { name: "World" }));
    ```
=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use apcore::APCore;
    use async_trait::async_trait;
    use serde_json::Value;

    #[derive(Debug)]
    struct AuditMiddleware;

    #[async_trait]
    impl Middleware for AuditMiddleware {
        fn name(&self) -> &str { "myapp.audit" }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            println!("[AUDIT] calling {module_id}");
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: Value,
            output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            println!("[AUDIT] {module_id} returned {output}");
            Ok(None)
        }

        async fn on_error(
            &self,
            _module_id: &str,
            _inputs: Value,
            _error: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
    }

    fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        client.use_middleware(Box::new(AuditMiddleware))?;
        Ok(())
    }
    ```

## Dependencies

- [Context Object](./context-object.md) — passed to every hook; provides `trace_id`, `caller_id`, `redacted_inputs` and the `data` map for per-call state.
- [Core Executor](./core-executor.md) — runs the chain at Steps 6 and 10 and drives `on_error` recovery and retries.

??? info "Python SDK reference"
    Not a protocol requirement — the Python SDK's source layout for users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `src/apcore/middleware/base.py` | `Middleware`, `RetrySignal` |
    | `src/apcore/middleware/manager.py` | `MiddlewareManager`, `MiddlewareChainError` |
    | `src/apcore/middleware/adapters.py` | `BeforeMiddleware`, `AfterMiddleware` |
    | `src/apcore/middleware/retry.py` | `RetryMiddleware`, `RetryConfig` |
    | `src/apcore/middleware/circuit_breaker.py` | `CircuitBreakerMiddleware` |
    | `src/apcore/middleware/context_namespace.py` | `validate_context_key`, `enforce_context_key` |
    | `src/apcore/middleware/logging.py` | `LoggingMiddleware` (deprecated) |

## Testing Strategy

- **Ordering**: priority order, stable registration order for equal priorities, reverse order for `after`, rejection of priority > 1000.
- **Replacement**: input replacement in `before`, output replacement in `after`, `None` pass-through.
- **Unwinding**: `on_error` runs over every entered middleware including the one whose `before()` raised; first recovery wins; a raising `on_error` is logged and skipped; `RetrySignal` re-runs the pipeline; cancellation bypasses `on_error`.
- **After-chain failure**: the first `after()` error stops the phase.
- **Duplicates**: warning content, registration still succeeding, opt-out, removal clearing the entry.
- **Thread safety**: concurrent registration and snapshots.
- **Namespacing**: `validate_context_key` for every writer/prefix combination.

## Contract: Middleware.before

### Inputs
- `module_id` (str/string/&str, required) — ID of the module about to execute
- `inputs` (dict/object/`Value`, required) — current inputs (Rust receives an owned `Value`)
- `context` (Context, required)

### Errors
- Any error aborts the before phase: later `before()` hooks are skipped, the error is wrapped in `MiddlewareChainError`, and `on_error` runs over every middleware whose `before()` was entered — this one included.

### Returns
- A replacement input map, or `None`/`null`/`Ok(None)` to pass the inputs through unchanged.

### Properties
- async: Python sync or async; TypeScript sync or promise-returning (always awaited); Rust `async fn`
- thread_safe: true — shared by concurrent calls; keep per-call state in `context.data`
- pure: false (may write `context.data`)

## Contract: Middleware.after

### Inputs
- `module_id` (str/string/&str, required)
- `inputs` (dict/object/`Value`, required) — the inputs after the before phase
- `output` (dict/object/`Value`, required) — the current output
- `context` (Context, required)

### Errors
- Fail-fast in all three SDKs: the first error stops the after phase and propagates; the remaining `after()` hooks do not run, and the error goes through the call's `on_error` chain.

### Returns
- A replacement output map, or `None`/`null`/`Ok(None)` to pass the output through unchanged.

### Properties
- async: as `before`
- thread_safe: true

## Contract: Middleware.on_error

### Inputs
- `module_id` (str/string/&str, required)
- `inputs` (dict/object/`Value`, required)
- `error` (Exception/Error/`&ModuleError`, required) — the error that terminated execution; non-apcore exceptions arrive wrapped as `ModuleError` (Algorithm A11)
- `context` (Context, required)

### Errors
- `on_error` SHOULD NOT raise. An error it raises is logged and the next handler is tried.

### Returns
- A recovery output map — becomes the call's output; no further handler runs.
- A `RetrySignal(inputs)` (Rust: `OnErrorOutcome::Retry` from `on_error_outcome`) — the Executor re-runs the pipeline with those inputs; no further handler runs.
- `None`/`null`/`Ok(None)` — no recovery; the next handler is tried, and the error propagates when none recovers.

First recovery wins in all three SDKs.

### Properties
- async: as `before` (a synchronous Python `on_error` runs in a worker thread on the async path)
- thread_safe: true

---

## Pipeline Step Middleware

Module-level middleware wraps the whole call. **Step middleware** (`StepMiddleware`) is finer-grained: it is notified around **every** pipeline step — `acl_check`, `input_validation`, `execute`, … — and filters on the step name itself. There is no `next`-style continuation and no step-inputs parameter: a Step is `execute(ctx)`, and step middleware observes it.

Registration:

| Language | Where |
|----------|-------|
| Python | `client.executor.current_strategy.add_step_middleware(mw)` |
| Rust | `ExecutionStrategy::add_step_middleware(Arc::new(mw))` on the strategy **before** building the `Executor` (`Executor::strategy()` hands out `&` only) |
| TypeScript | `PipelineEngine.addStepMiddleware(mw)` on an engine you drive yourself; the Executor's engine is private, so there is no path from `APCore` |

Strategies and the engine are described in [Execution Pipeline](./execution-pipeline.md); the ordering requirement relative to module-level middleware is [PROTOCOL_SPEC §5.16](../spec/protocol-spec.md#516-pipeline-control-flow-requirements) item 5.

### Lifecycle

```text
before_step(step_name, state)
  --> step body executes
       |
       +-- on success: after_step(step_name, state, result)
       +-- on failure: on_step_error(step_name, state, error)
```

`state` is the `PipelineState` view — the step name, the outputs produced so far, and the pipeline context.

#### What `state.outputs` contains

`state.outputs` maps step name → that step's output and contains **exactly the steps that completed before the current one**. The current step is never present in any hook: `before_step` runs before it, `on_step_error` has no output to record, and in `after_step` the output is the `result` argument. Implementations **MUST NOT** insert the current step's output into `state.outputs` before invoking `after_step`.

!!! warning "`run_until` sees the current step — deliberately"
    The `run_until` predicate is evaluated **after** a step completes and decides whether to stop *because of* what the step produced, so its `state.outputs` does include that step. The snapshot is taken between the `after_step` hook and the predicate; the two consumers see deliberately different maps.

    `state.outputs` is a live reference to the engine's map in Python and TypeScript. Read it (or copy it) inside the hook — a stored reference later shows the final map.

### Normative Rules

- Implementations MUST provide `StepMiddleware` with three callbacks — `before_step`, `after_step`, `on_step_error` — each optional (no-op default).
- `before_step(step_name, state)` MUST be invoked before the step body. It is an **observation** hook; its return value carries no meaning. Input rewriting is the module-level `Middleware.before` contract.
- `after_step(step_name, state, result)` MUST be invoked after the step body completes successfully, with `result` a snapshot of the step's output.
- `on_step_error(step_name, state, error)` MUST be invoked when the step body raises. A non-null return MUST be treated as recovery output: the error does not propagate, the value becomes the step's output, and the pipeline continues with the next step.
- A null return from every `on_step_error` MUST let the original error propagate (subject to the step's `ignore_errors` — [Fail-Fast Error Handling](./core-executor.md#fail-fast-error-handling)).
- `after_step` MUST also be invoked after a **recovered** step body, so a middleware that acquired something in `before_step` always gets its `after_step`.
- `before_step` callbacks MUST run in registration order; `after_step` and `on_step_error` in **reverse** registration order (onion model).
- `on_step_error` callbacks run over the middlewares whose `before_step` had executed; the first non-null recovery value short-circuits the rest.
- Sync and async callbacks MUST both be supported; detection is an SDK-local concern (Python inspects the returned value, TypeScript awaits unconditionally, Rust uses `async_trait`).

#### A `before_step` failure terminates the step — it is not recoverable

- An error raised by `before_step` MUST be wrapped in `MiddlewareChainError`.
- The step body MUST NOT execute.
- `on_step_error` MUST still be invoked, in reverse registration order, on the middlewares whose `before_step` had been entered — **for observation and cleanup only**. Its return value MUST be discarded: it MUST NOT become the step's output or let the pipeline continue.
- First-recovery-wins MUST NOT apply to this pass: **every** entered middleware MUST be notified, or the cleanup of the ones behind the first would be stranded.
- `after_step` MUST NOT be invoked for that step.
- The step's `ignore_errors` MUST NOT apply; `MiddlewareChainError` propagates regardless.

**Why recovery is forbidden here.** Honouring a recovery value would advance the pipeline past a step whose body never ran. The standard strategy places `acl_check` and `approval_gate` in that sequence, so a middleware that can make its own `before_step` raise and then return a value from `on_step_error` would skip the ACL check or the approval gate outright — an authorization bypass from an extension point that carries no authority.

!!! note "Not the module-level rule, despite the shared vocabulary"
    Module-level `on_error` **does** honour a recovery value raised during the before phase. That is consistent: a module-level recovery value **ends the call** — it is the return value and nothing further executes. A step-level recovery value **resumes a pipeline**.

Pipeline configuration errors — an unknown step under `pipeline.configure`, or a step whose `requires` is not satisfied — are raised when the strategy is built; see [Execution Pipeline](./execution-pipeline.md).

### Cross-language usage

A timing `StepMiddleware`:

=== "Python"
    ```python
    import time

    from apcore import APCore, PipelineState, StepMiddleware, StepResult

    class TimingStepMiddleware(StepMiddleware):
        # Per-step state lives on the instance: the hooks get no `inputs`.
        def __init__(self) -> None:
            self._started: dict[str, float] = {}

        async def before_step(self, step_name: str, state: PipelineState) -> None:
            self._started[step_name] = time.perf_counter()

        async def after_step(self, step_name: str, state: PipelineState, result: StepResult) -> None:
            start = self._started.pop(step_name, None)
            if start is not None:
                print(f"step={step_name} elapsed_ms={(time.perf_counter() - start) * 1000:.2f}")

        async def on_step_error(self, step_name: str, state: PipelineState, error: Exception) -> None:
            self._started.pop(step_name, None)
            print(f"step={step_name} error={type(error).__name__}")
            return None  # do not recover

    client = APCore()
    client.executor.current_strategy.add_step_middleware(TimingStepMiddleware())

    @client.module(id="demo.greet", description="Greet the user")
    def greet(name: str) -> dict:
        return {"message": f"Hello, {name}!"}

    print(client.call("demo.greet", {"name": "World"}))
    ```
=== "TypeScript"
    ```typescript
    import { PipelineEngine, type PipelineState, type StepMiddleware } from "apcore-js";

    // StepMiddleware is an interface — implement it.
    class TimingStepMiddleware implements StepMiddleware {
        private readonly started = new Map<string, number>();

        async beforeStep(stepName: string, _state: PipelineState): Promise<void> {
            this.started.set(stepName, performance.now());
        }

        async afterStep(stepName: string, _state: PipelineState, _result: unknown): Promise<void> {
            const start = this.started.get(stepName);
            this.started.delete(stepName);
            if (start !== undefined) {
                console.log(`step=${stepName} elapsed_ms=${(performance.now() - start).toFixed(2)}`);
            }
        }

        async onStepError(stepName: string, _state: PipelineState, error: Error): Promise<unknown> {
            this.started.delete(stepName);
            console.log(`step=${stepName} error=${error.constructor.name}`);
            return null; // do not recover
        }
    }

    // Registered on a PipelineEngine that you run yourself — see Execution Pipeline.
    const engine = new PipelineEngine();
    engine.addStepMiddleware(new TimingStepMiddleware());
    ```
=== "Rust"
    ```rust
    use apcore::{
        build_standard_strategy, Config, Executor, ModuleError, PipelineState, Registry,
        StepMiddleware,
    };
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    // The trait takes `&self` and is Send + Sync, so per-step state sits behind a Mutex.
    #[derive(Default)]
    struct TimingStepMiddleware {
        started: Mutex<HashMap<String, Instant>>,
    }

    #[async_trait]
    impl StepMiddleware for TimingStepMiddleware {
        async fn before_step(&self, step_name: &str, _state: &PipelineState<'_>) -> Result<(), ModuleError> {
            self.started.lock().unwrap().insert(step_name.to_string(), Instant::now());
            Ok(())
        }

        async fn after_step(
            &self,
            step_name: &str,
            _state: &PipelineState<'_>,
            _result: &Value,
        ) -> Result<(), ModuleError> {
            if let Some(start) = self.started.lock().unwrap().remove(step_name) {
                println!("step={step_name} elapsed_ms={:.2}", start.elapsed().as_secs_f64() * 1000.0);
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
            println!("step={step_name} error={:?}", error.code);
            Ok(None) // do not recover
        }
    }

    fn main() {
        // Populate the strategy BEFORE constructing the executor.
        let mut strategy = build_standard_strategy();
        strategy.add_step_middleware(Arc::new(TimingStepMiddleware::default()));
        let _executor = Executor::with_strategy(Registry::new(), Config::from_defaults(), strategy);
    }
    ```

## Contract: StepMiddleware.before_step

### Inputs
- `step_name` (str/string/&str, required) — pipeline step name (e.g. `input_validation`)
- `state` (PipelineState, required) — the step name, the outputs produced so far, and the pipeline context

### Errors
- Any error terminates the step: it is wrapped in `MiddlewareChainError`, the step body does not run, and `on_step_error` is invoked on the already-entered step middlewares for observation only (see above).

### Returns
- Nothing. The return value is discarded.

### Properties
- async: Python sync or async; TypeScript and Rust async
- thread_safe: true
- pure: false (may mutate the context reachable through `state`)

## Contract: StepMiddleware.after_step

### Inputs
- `step_name`, `state` (as `before_step`)
- `result` (StepResult / unknown / `&Value`, required) — snapshot of the step's output

### Errors
- An error propagates as a step failure.

### Returns
- Nothing. `after_step` observes; it does not replace the step output.

### Properties
- async: as `before_step`
- thread_safe: true

## Contract: StepMiddleware.on_step_error

### Inputs
- `step_name`, `state` (as `before_step`)
- `error` (Exception / Error / `&ModuleError`, required) — the error raised by the step body

### Errors
- SHOULD NOT raise; an error inside the handler is logged and the next handler is tried.

### Returns
- A recovery value — becomes the step's output; remaining handlers are skipped and the pipeline continues.
- `None`/`null`/`Ok(None)` — the error continues propagating.

### Properties
- async: as `before_step`
- thread_safe: true
