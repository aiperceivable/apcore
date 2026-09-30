---
description: "Write apcore middleware: the before/after/on_error hooks, registration, per-call state in context.data, rewriting inputs and outputs, error recovery, and when to use a pipeline step."
---

# Middleware Guide

Middleware wraps every module call the Executor runs. Use it for cross-cutting behaviour — auditing, timing, input defaults, output shaping, fallbacks — without touching module code.

This guide shows how to write and register your own middleware. The normative contract (hook semantics, context namespacing, duplicate detection, step middleware) and the reference for the built-in middleware live in [Middleware System](../features/middleware-system.md).

## 1. Where middleware runs

The standard pipeline has eleven steps. Middleware occupies two of them:

```text
context_creation → call_chain_guard → module_lookup → acl_check → approval_gate
  → middleware_before → input_validation → execute → output_validation
  → middleware_after → return_result
```

What that ordering means for your middleware:

- A call rejected by the ACL or the approval gate never reaches `before()`.
- The inputs `before()` returns are the inputs that input validation checks.
- The output `after()` returns is not validated again.
- `validate()` (preflight) runs no middleware.

Within the chain, middleware nests like an onion:

```text
call ──► A.before ──► B.before ──► C.before ──► validate → execute → validate
                                                              │
result ◄── A.after ◄── B.after ◄── C.after ◄──────────────────┘
```

- **Priority** (0–1000, default 100) decides the order: higher priority runs first in `before()` and last in `after()`. Equal priorities keep registration order.
- **On failure**, `on_error()` runs in reverse order on every middleware whose `before()` was entered — including one whose `before()` raised. Failures before step 6 (unknown module, ACL denial, approval rejection) reach no `on_error()`.
- **Instances are shared** by every concurrent call. Keep per-call state in `context.data` (see [§4](#4-example-an-audit-middleware)), never on `self`.

See [Core Executor](../features/core-executor.md) for the full pipeline.

## 2. The Middleware interface

Every hook returns "nothing" (`None` / `null` / `Ok(None)`) to leave the call unchanged, or a value to replace the inputs, the output, or — in `on_error()` — the error.

=== "Python"

    ```python
    from typing import Any

    from apcore import Context, Middleware, RetrySignal


    class MyMiddleware(Middleware):
        def __init__(self) -> None:
            super().__init__(priority=100)  # 0-1000, higher runs first

        def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
            """Return new inputs, or None to keep them."""
            return None

        def after(
            self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context
        ) -> dict[str, Any] | None:
            """Return a new output, or None to keep it."""
            return None

        def on_error(
            self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
        ) -> dict[str, Any] | RetrySignal | None:
            """Return a recovery output, a RetrySignal, or None to let the error propagate."""
            return None
    ```

    The base class implements all three hooks as no-ops, so override only what you need. Hooks may also be `async def`; the executor awaits them.

=== "TypeScript"

    ```typescript
    import { Middleware, RetrySignal } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class MyMiddleware extends Middleware {
      constructor() {
        super(100); // priority 0-1000, higher runs first
      }

      // Return new inputs, or null to keep them.
      override before(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _context: Context,
      ): Record<string, unknown> | null {
        return null;
      }

      // Return a new output, or null to keep it.
      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _output: Record<string, unknown>,
        _context: Context,
      ): Record<string, unknown> | null {
        return null;
      }

      // Return a recovery output, a RetrySignal, or null to let the error propagate.
      override onError(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _error: Error,
        _context: Context,
      ): Record<string, unknown> | RetrySignal | null {
        return null;
      }
    }
    ```

    The base class implements all three hooks as no-ops, so override only what you need. Hooks may return a `Promise`; the executor awaits it.

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::Value;

    #[derive(Debug)]
    struct MyMiddleware;

    #[async_trait]
    impl Middleware for MyMiddleware {
        fn name(&self) -> &str {
            "my_middleware"
        }

        // Optional: defaults to 100. Range 0-1000, higher runs first.
        fn priority(&self) -> u16 {
            100
        }

        /// Return `Some(inputs)` to replace the inputs, `None` to keep them.
        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        /// Return `Some(output)` to replace the output, `None` to keep it.
        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        /// Return `Some(value)` to recover with that output, `None` to let the error propagate.
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
    ```

    `name()`, `before()`, `after()` and `on_error()` are required; the trait has no default hooks. For a single-hook middleware, use the closure adapters in [§3.2](#32-function-adapters) instead. To request a retry, override `on_error_outcome()` and return `OnErrorOutcome::Retry`.

## 3. Registering middleware

### 3.1 Class-based middleware

Register on the client. Registration can happen at any time; it applies to calls that start afterwards.

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    client.use(MyMiddleware())  # returns the client, so calls chain
    ```

    `Executor(registry=registry, middlewares=[...])` does the same when you build the Executor yourself.

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    client.use(new MyMiddleware()); // returns the client, so calls chain
    ```

    `new Executor({ registry, middlewares: [...] })` does the same when you build the Executor yourself.

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::APCore;

    fn register(client: &APCore) -> Result<(), ModuleError> {
        // `use` is a Rust keyword, so the method is `use_middleware`.
        client.use_middleware(Box::new(MyMiddleware))?;
        Ok(())
    }
    ```

    To keep your own handle on the instance, register an `Arc` through the executor: `client.executor().use_middleware_shared(Arc::new(MyMiddleware))?`.

Registering the same middleware type twice logs a duplicate-registration warning; the second registration still takes effect.

### 3.2 Function adapters

For a middleware that only needs `before()` or `after()`, register a function.

=== "Python"

    ```python
    from typing import Any

    from apcore import APCore, Context

    client = APCore()


    def log_call(module_id: str, inputs: dict[str, Any], context: Context) -> None:
        print(f"-> {module_id} trace={context.trace_id}")
        return None


    def log_result(module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
        print(f"<- {module_id} keys={sorted(output)}")
        return None


    client.use_before(log_call).use_after(log_result)
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();

    client
      .useBefore((moduleId, _inputs, context) => {
        console.log(`-> ${moduleId} trace=${context.traceId}`);
        return null;
      })
      .useAfter((moduleId, _inputs, output) => {
        console.log(`<- ${moduleId} keys=${Object.keys(output).sort().join(',')}`);
        return null;
      });
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::adapters::{AfterAdapter, BeforeAdapter};
    use apcore::APCore;
    use serde_json::Value;

    fn register_logging(client: &APCore) -> Result<(), ModuleError> {
        client.use_middleware(Box::new(BeforeAdapter::new(
            "log_call",
            |module_id: String, _inputs: Value, ctx: Context<Value>| async move {
                println!("-> {module_id} trace={}", ctx.trace_id);
                Ok::<Option<Value>, ModuleError>(None)
            },
        )))?;
        client.use_middleware(Box::new(AfterAdapter::new(
            "log_result",
            |module_id: String, _inputs: Value, output: Value, _ctx: Context<Value>| async move {
                let keys: Vec<String> = output
                    .as_object()
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                println!("<- {module_id} keys={keys:?}");
                Ok::<Option<Value>, ModuleError>(None)
            },
        )))?;
        Ok(())
    }
    ```

    `client.use_before()` / `client.use_after()` take a type implementing the `BeforeMiddleware` / `AfterMiddleware` trait (a `name()` plus the one hook) instead of a closure.

### 3.3 Removing middleware

In Python and TypeScript, `client.remove(mw)` removes the instance you registered (matched by identity) and returns whether it was found. In Rust, `use_middleware` consumes the box, so register with `client.use_middleware_handle(Box::new(mw))?` and later pass the returned handle to `client.remove_handle(handle)`.

## 4. Example: an audit middleware

This middleware writes one line per call with the caller, the redacted inputs, and the duration. It shows the two rules for per-call state:

- **Store it in `context.data`**, under an `ext.<vendor>.<field>` key. The `_apcore.*` prefix belongs to the framework's own middleware; user middleware must not write there.
- **Expect nesting.** A module that calls another module passes its context on, and parent and child share one `context.data`. A single `start_time` value would be overwritten by the nested call, so the example keeps a stack.

It logs `context.redacted_inputs` rather than the raw inputs: the executor fills it in before middleware runs, with fields marked `x-sensitive` in the schema — and keys matched by the `obs.redaction.*` rules — replaced by `***REDACTED***` (see [Redaction](../features/redaction.md)).

=== "Python"

    ```python
    import logging
    import time
    from typing import Any

    from apcore import APCore, Context, Middleware, ModuleError

    logging.basicConfig(level=logging.INFO)
    logger = logging.getLogger("acme.audit")

    STARTS_KEY = "ext.acme.audit.starts"


    class AuditMiddleware(Middleware):
        """One audit line per call: caller, redacted inputs, outcome, duration."""

        def __init__(self) -> None:
            super().__init__(priority=900)  # outermost, so the timing covers inner middleware

        def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
            context.data.setdefault(STARTS_KEY, []).append(time.perf_counter())
            return None

        def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
            logger.info(
                "ok module=%s trace=%s caller=%s inputs=%s duration_ms=%.1f",
                module_id,
                context.trace_id,
                context.caller_id,
                context.redacted_inputs,
                self._elapsed_ms(context),
            )
            return None

        def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context) -> None:
            code = error.code if isinstance(error, ModuleError) else type(error).__name__
            logger.warning(
                "failed module=%s trace=%s code=%s duration_ms=%.1f",
                module_id,
                context.trace_id,
                code,
                self._elapsed_ms(context),
            )
            return None  # observe only: the error keeps propagating

        @staticmethod
        def _elapsed_ms(context: Context) -> float:
            starts = context.data.get(STARTS_KEY)
            if not starts:
                return 0.0
            return (time.perf_counter() - starts.pop()) * 1000


    client = APCore()


    @client.module(id="greet.hello", description="Say hello")
    def hello(name: str) -> dict:
        return {"message": f"Hello, {name}!"}


    client.use(AuditMiddleware())
    print(client.call("greet.hello", {"name": "Ada"}))
    ```

=== "TypeScript"

    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, Middleware, ModuleError } from 'apcore-js';
    import type { Context } from 'apcore-js';

    const STARTS_KEY = 'ext.acme.audit.starts';

    /** One audit line per call: caller, redacted inputs, outcome, duration. */
    class AuditMiddleware extends Middleware {
      constructor() {
        super(900); // outermost, so the timing covers inner middleware
      }

      override before(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        context: Context,
      ): null {
        const starts = (context.data[STARTS_KEY] as number[] | undefined) ?? [];
        starts.push(performance.now());
        context.data[STARTS_KEY] = starts;
        return null;
      }

      override after(
        moduleId: string,
        _inputs: Record<string, unknown>,
        _output: Record<string, unknown>,
        context: Context,
      ): null {
        console.info(
          `ok module=${moduleId} trace=${context.traceId} caller=${context.callerId} ` +
            `inputs=${JSON.stringify(context.redactedInputs)} ` +
            `duration_ms=${this.elapsedMs(context).toFixed(1)}`,
        );
        return null;
      }

      override onError(
        moduleId: string,
        _inputs: Record<string, unknown>,
        error: Error,
        context: Context,
      ): null {
        const code = error instanceof ModuleError ? error.code : error.name;
        console.warn(
          `failed module=${moduleId} trace=${context.traceId} code=${code} ` +
            `duration_ms=${this.elapsedMs(context).toFixed(1)}`,
        );
        return null; // observe only: the error keeps propagating
      }

      private elapsedMs(context: Context): number {
        const start = (context.data[STARTS_KEY] as number[] | undefined)?.pop();
        return start === undefined ? 0 : performance.now() - start;
      }
    }

    const client = new APCore();
    client.module({
      id: 'greet.hello',
      description: 'Say hello',
      inputSchema: Type.Object({ name: Type.String() }),
      outputSchema: Type.Object({ message: Type.String() }),
      execute: (inputs) => ({ message: `Hello, ${String(inputs['name'])}!` }),
    });

    client.use(new AuditMiddleware());
    console.log(await client.call('greet.hello', { name: 'Ada' }));
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use apcore::module::Module;
    use apcore::APCore;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::time::{SystemTime, UNIX_EPOCH};

    const STARTS_KEY: &str = "ext.acme.audit.starts";

    fn now_ms() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }

    /// One audit line per call: caller, redacted inputs, outcome, duration.
    #[derive(Debug)]
    struct AuditMiddleware;

    impl AuditMiddleware {
        // `ctx.data` is an `Arc<RwLock<HashMap<String, Value>>>`. These helpers are
        // synchronous, so the lock guard is always released before any `.await`.
        fn push_start(ctx: &Context<Value>) {
            let mut data = ctx.data.write();
            let starts = data.entry(STARTS_KEY.to_string()).or_insert_with(|| json!([]));
            if let Some(stack) = starts.as_array_mut() {
                stack.push(json!(now_ms()));
            }
        }

        fn elapsed_ms(ctx: &Context<Value>) -> f64 {
            let mut data = ctx.data.write();
            data.get_mut(STARTS_KEY)
                .and_then(Value::as_array_mut)
                .and_then(Vec::pop)
                .and_then(|start| start.as_f64())
                .map_or(0.0, |start| now_ms() - start)
        }
    }

    #[async_trait]
    impl Middleware for AuditMiddleware {
        fn name(&self) -> &str {
            "acme_audit"
        }

        fn priority(&self) -> u16 {
            900 // outermost, so the timing covers inner middleware
        }

        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Self::push_start(ctx);
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: Value,
            _output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            println!(
                "ok module={module_id} trace={} caller={:?} inputs={:?} duration_ms={:.1}",
                ctx.trace_id,
                ctx.caller_id,
                ctx.redacted_inputs,
                Self::elapsed_ms(ctx),
            );
            Ok(None)
        }

        async fn on_error(
            &self,
            module_id: &str,
            _inputs: Value,
            error: &ModuleError,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            eprintln!(
                "failed module={module_id} trace={} code={} duration_ms={:.1}",
                ctx.trace_id,
                error.code.wire_str(),
                Self::elapsed_ms(ctx),
            );
            Ok(None) // observe only: the error keeps propagating
        }
    }

    struct Hello;

    #[async_trait]
    impl Module for Hello {
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "message": { "type": "string" } },
                "required": ["message"]
            })
        }

        fn description(&self) -> &str {
            "Say hello"
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let name = inputs["name"].as_str().unwrap_or("world");
            Ok(json!({ "message": format!("Hello, {name}!") }))
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        client.register("greet.hello", Box::new(Hello))?;
        client.use_middleware(Box::new(AuditMiddleware))?;

        let output = client.call("greet.hello", json!({ "name": "Ada" }), None, None).await?;
        println!("{output}");
        Ok(())
    }
    ```

`ext.*` keys travel with the context when it is serialized — for example to hand a call to another process — while `_`-prefixed keys are dropped from the wire form. Keep secrets out of `context.data`; mark sensitive input fields `x-sensitive` in the schema so they are redacted instead.

## 5. Modifying inputs and outputs

Return a new value from `before()` or `after()`. Build a copy rather than mutating the argument: middleware further out may still hold the original.

The middleware below fills in a default `locale` for `notify.*` modules on the way in and stamps the trace ID on every result on the way out. Mind the pipeline position of each hook:

- Input validation runs **after** `before()`, so an added input field must be one the module's input schema accepts.
- Output validation runs **before** `after()`, so the rewritten output is not checked again — keep it within what your callers expect.

=== "Python"

    ```python
    from typing import Any

    from apcore import Context, Middleware


    class ShapingMiddleware(Middleware):
        def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
            if not module_id.startswith("notify.") or "locale" in inputs:
                return None  # unchanged
            return {**inputs, "locale": "en-US"}

        def after(
            self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context
        ) -> dict[str, Any]:
            return {**output, "trace_id": context.trace_id}


    client.use(ShapingMiddleware())
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class ShapingMiddleware extends Middleware {
      override before(
        moduleId: string,
        inputs: Record<string, unknown>,
        _context: Context,
      ): Record<string, unknown> | null {
        if (!moduleId.startsWith('notify.') || 'locale' in inputs) {
          return null; // unchanged
        }
        return { ...inputs, locale: 'en-US' };
      }

      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ): Record<string, unknown> {
        return { ...output, trace_id: context.traceId };
      }
    }

    client.use(new ShapingMiddleware());
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    #[derive(Debug)]
    struct ShapingMiddleware;

    #[async_trait]
    impl Middleware for ShapingMiddleware {
        fn name(&self) -> &str {
            "shaping"
        }

        async fn before(
            &self,
            module_id: &str,
            mut inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if !module_id.starts_with("notify.") || inputs.get("locale").is_some() {
                return Ok(None); // unchanged
            }
            if let Some(obj) = inputs.as_object_mut() {
                obj.insert("locale".to_string(), json!("en-US"));
            }
            Ok(Some(inputs))
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            mut output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if let Some(obj) = output.as_object_mut() {
                obj.insert("trace_id".to_string(), json!(ctx.trace_id));
            }
            Ok(Some(output))
        }

        async fn on_error(&self, _: &str, _: Value, _: &ModuleError, _: &Context<Value>) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
    }
    ```

    Register it with `client.use_middleware(Box::new(ShapingMiddleware))?`. The arguments arrive by value, so rewriting them in place is safe.

## 6. Handling errors

### 6.1 Recovering in `on_error()`

Returning a value from `on_error()` turns a failed call into a successful one: the value becomes the call's result and the remaining `on_error()` handlers are skipped. The recovery value is not validated against the output schema, so return something that matches it. If `on_error()` itself raises, the error is logged and the next handler runs.

`on_error()` receives a module error: in Python and TypeScript a non-`ModuleError` exception is wrapped as `ModuleExecuteError` (code `MODULE_EXECUTE_ERROR`) first. A cancelled call skips `on_error()` entirely, so middleware cannot swallow a cancellation (D-20).

=== "Python"

    ```python
    from typing import Any

    from apcore import Context, Middleware


    class FallbackMiddleware(Middleware):
        """Serve a canned result when a listed module fails."""

        def __init__(self, fallbacks: dict[str, dict[str, Any]]) -> None:
            super().__init__()
            self._fallbacks = fallbacks

        def on_error(
            self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
        ) -> dict[str, Any] | None:
            return self._fallbacks.get(module_id)  # None lets the error propagate


    client.use(FallbackMiddleware({"quotes.latest": {"price": None, "stale": True}}))
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    /** Serve a canned result when a listed module fails. */
    class FallbackMiddleware extends Middleware {
      constructor(private readonly fallbacks: Record<string, Record<string, unknown>>) {
        super();
      }

      override onError(
        moduleId: string,
        _inputs: Record<string, unknown>,
        _error: Error,
        _context: Context,
      ): Record<string, unknown> | null {
        return this.fallbacks[moduleId] ?? null; // null lets the error propagate
      }
    }

    client.use(new FallbackMiddleware({ 'quotes.latest': { price: null, stale: true } }));
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;

    /// Serve a canned result when a listed module fails.
    #[derive(Debug)]
    struct FallbackMiddleware {
        fallbacks: HashMap<String, Value>,
    }

    #[async_trait]
    impl Middleware for FallbackMiddleware {
        fn name(&self) -> &str {
            "fallback"
        }

        async fn before(&self, _: &str, _: Value, _: &Context<Value>) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        async fn after(&self, _: &str, _: Value, _: Value, _: &Context<Value>) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        async fn on_error(
            &self,
            module_id: &str,
            _inputs: Value,
            _error: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            // None lets the error propagate.
            Ok(self.fallbacks.get(module_id).cloned())
        }
    }
    ```

    Register it with `client.use_middleware(Box::new(FallbackMiddleware { fallbacks }))?`.

### 6.2 Rejecting a call from `before()`

Raise a `ModuleError` from `before()` to stop the call. The module does not run, and the caller receives your error unchanged — the executor unwraps the internal `MiddlewareChainError` (D-22). In Python and TypeScript, raising any other exception type reaches the caller as `ModuleExecuteError`.

=== "Python"

    ```python
    import json
    from typing import Any

    from apcore import Context, InvalidInputError, Middleware

    MAX_INPUT_BYTES = 64 * 1024


    class InputSizeLimitMiddleware(Middleware):
        def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
            size = len(json.dumps(inputs, default=str).encode())
            if size > MAX_INPUT_BYTES:
                raise InvalidInputError(message=f"{module_id}: inputs are {size} bytes, limit {MAX_INPUT_BYTES}")
            return None
    ```

=== "TypeScript"

    ```typescript
    import { InvalidInputError, Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    const MAX_INPUT_BYTES = 64 * 1024;

    class InputSizeLimitMiddleware extends Middleware {
      override before(
        moduleId: string,
        inputs: Record<string, unknown>,
        _context: Context,
      ): null {
        const size = new TextEncoder().encode(JSON.stringify(inputs)).length;
        if (size > MAX_INPUT_BYTES) {
          throw new InvalidInputError(`${moduleId}: inputs are ${size} bytes, limit ${MAX_INPUT_BYTES}`);
        }
        return null;
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::adapters::BeforeAdapter;
    use apcore::APCore;
    use serde_json::Value;

    const MAX_INPUT_BYTES: usize = 64 * 1024;

    fn register_size_limit(client: &APCore) -> Result<(), ModuleError> {
        client.use_middleware(Box::new(BeforeAdapter::new(
            "input_size_limit",
            |module_id: String, inputs: Value, _ctx: Context<Value>| async move {
                let size = inputs.to_string().len();
                if size > MAX_INPUT_BYTES {
                    return Err(ModuleError::invalid_input(format!(
                        "{module_id}: inputs are {size} bytes, limit {MAX_INPUT_BYTES}"
                    )));
                }
                Ok::<Option<Value>, ModuleError>(None)
            },
        )))?;
        Ok(())
    }
    ```

### 6.3 Retrying

`on_error()` can also return a `RetrySignal` carrying the inputs for another attempt; the executor re-runs the pipeline with them, keeping the same context. Rather than writing your own, use the built-in `RetryMiddleware` ([§8](#8-built-in-middleware)), which retries errors marked `retryable` with exponential or fixed backoff.

## 7. Middleware or a pipeline step?

The executor has two extension points. Middleware sits at fixed positions (steps 6 and 10) and sees each call as a before/after pair. A custom step is inserted at a position you choose and runs once per call.

| Use middleware for | Use a custom step for |
|---|---|
| Logging, timing, tracing — anything that pairs inputs with outputs | Checks that must run at a specific position, e.g. right after `acl_check` |
| Input defaults and output shaping | Quotas, rate limits and budgets that gate execution |
| Fallbacks and retries (`on_error`) | Logic that should show up as its own entry in the pipeline trace |

A step declares whether it is **pure**. `validate()` runs only pure steps, so anything with a side effect — consuming quota, writing a record — must be `pure=False`, or a preflight check would use up the budget it is checking.

=== "Python"

    ```python
    import threading

    from apcore import APCore
    from apcore.pipeline import BaseStep, PipelineContext, StepResult


    class CallBudgetStep(BaseStep):
        """Abort calls once a module has used up its call budget."""

        def __init__(self, max_calls: int) -> None:
            super().__init__(
                name="call_budget",
                description="Per-module call budget",
                pure=False,  # consumes budget: validate() must skip it
            )
            self._max_calls = max_calls
            self._counts: dict[str, int] = {}
            self._lock = threading.Lock()

        async def execute(self, ctx: PipelineContext) -> StepResult:
            with self._lock:
                used = self._counts.get(ctx.module_id, 0)
                if used >= self._max_calls:
                    return StepResult(action="abort", explanation=f"call budget exhausted for {ctx.module_id}")
                self._counts[ctx.module_id] = used + 1
            return StepResult(action="continue")


    client = APCore()
    client.executor.current_strategy.insert_after("acl_check", CallBudgetStep(max_calls=1000))
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';
    import type { PipelineContext, Step, StepResult } from 'apcore-js';

    /** Abort calls once a module has used up its call budget. */
    class CallBudgetStep implements Step {
      readonly name = 'call_budget';
      readonly description = 'Per-module call budget';
      readonly removable = true;
      readonly replaceable = true;
      readonly pure = false; // consumes budget: validate() must skip it
      private readonly counts = new Map<string, number>();

      constructor(private readonly maxCalls: number) {}

      async execute(ctx: PipelineContext): Promise<StepResult> {
        const used = this.counts.get(ctx.moduleId) ?? 0;
        if (used >= this.maxCalls) {
          return { action: 'abort', explanation: `call budget exhausted for ${ctx.moduleId}` };
        }
        this.counts.set(ctx.moduleId, used + 1);
        return { action: 'continue' };
      }
    }

    const client = new APCore();
    client.executor.currentStrategy.insertAfter('acl_check', new CallBudgetStep(1000));
    ```

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::{build_standard_strategy, APCore, Config, Executor, PipelineContext, Registry, Step, StepResult};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Abort calls once a module has used up its call budget.
    struct CallBudgetStep {
        max_calls: u64,
        counts: Mutex<HashMap<String, u64>>,
    }

    #[async_trait]
    impl Step for CallBudgetStep {
        fn name(&self) -> &str {
            "call_budget"
        }
        fn description(&self) -> &str {
            "Per-module call budget"
        }
        fn removable(&self) -> bool {
            true
        }
        fn replaceable(&self) -> bool {
            true
        }
        // `pure()` defaults to false: the step consumes budget, so validate() skips it.

        async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            let mut counts = self.counts.lock().expect("call budget lock poisoned");
            let used = counts.entry(ctx.module_id.clone()).or_insert(0);
            if *used >= self.max_calls {
                return Ok(StepResult::abort("call budget exhausted"));
            }
            *used += 1;
            Ok(StepResult::continue_step())
        }
    }

    fn build_client() -> Result<APCore, ModuleError> {
        let mut strategy = build_standard_strategy();
        strategy.insert_after(
            "acl_check",
            Box::new(CallBudgetStep { max_calls: 1000, counts: Mutex::new(HashMap::new()) }),
        )?;
        let executor = Executor::with_strategy(Registry::new(), Config::default(), strategy);
        Ok(APCore::with_options(None, Some(executor), None, None))
    }
    ```

An aborted step reaches the caller as a `ModuleError` (code `PIPELINE_ABORT` for a custom step). Strategies, presets, step fields and `pipeline:` configuration are covered in [Execution Pipeline](../features/execution-pipeline.md).

## 8. Built-in middleware

The SDKs ship these. Their options and exact behaviour are specified in [Middleware System](../features/middleware-system.md) and [Observability](../features/observability.md).

| Middleware | What it does | How it gets installed |
|---|---|---|
| `ObsLoggingMiddleware` | Structured call logs through `ContextLogger`, using redacted inputs. (`LoggingMiddleware`, its plain-logger predecessor, is deprecated in Python and TypeScript.) | `client.use(...)` |
| `MetricsMiddleware` | Call counts, error counts and durations into a `MetricsCollector` | `client.use(MetricsMiddleware(collector))` |
| `RetryMiddleware` + `RetryConfig` | Retries errors whose `retryable` is true, with exponential or fixed backoff | `client.use(...)` |
| `CircuitBreakerMiddleware` | Opens a per-module circuit when the error rate crosses a threshold; fails fast while open | `client.use(...)` |
| `TracingMiddleware` | Spans for every call, exported to stdout, memory or OTLP | Installed by the client when `observability.tracing.enabled` is true (also reads `strategy`, `sampling_rate`, `exporter`, `otlp_endpoint`); or `client.use(...)` |
| `ErrorHistoryMiddleware`, `UsageMiddleware`, `PlatformNotifyMiddleware` | Feed the `system.*` health, usage and event modules | Installed when `sys_modules.enabled` is true (`PlatformNotifyMiddleware` also needs `sys_modules.events.enabled`) |
| `StepMiddleware` | Hooks around each individual pipeline step (`before_step` / `after_step` / `on_step_error`), not around the module call | `strategy.add_step_middleware(...)` (TypeScript: `addStepMiddleware`) |

Adding retry and a circuit breaker:

=== "Python"

    ```python
    from apcore import APCore, CircuitBreakerMiddleware, RetryConfig, RetryMiddleware

    client = APCore()
    client.use(RetryMiddleware(RetryConfig(max_retries=3, base_delay_ms=200)))
    client.use(CircuitBreakerMiddleware(open_threshold=0.5, recovery_window_ms=30_000))
    ```

=== "TypeScript"

    ```typescript
    import { APCore, CircuitBreakerMiddleware, RetryMiddleware } from 'apcore-js';

    const client = new APCore();
    client.use(new RetryMiddleware({ maxRetries: 3, baseDelayMs: 200 }));
    client.use(new CircuitBreakerMiddleware({ openThreshold: 0.5, recoveryWindowMs: 30000 }));
    ```

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::{APCore, CircuitBreakerMiddleware, RetryConfig, RetryMiddleware};

    fn add_resilience(client: &APCore) -> Result<(), ModuleError> {
        // RetryConfig is #[non_exhaustive]: start from the default and set fields.
        let mut retry = RetryConfig::default();
        retry.max_retries = 3;
        retry.base_delay_ms = 200;
        client.use_middleware(Box::new(RetryMiddleware::new(retry)))?;

        let breaker = CircuitBreakerMiddleware::builder()
            .open_threshold(0.5)
            .recovery_window_ms(30_000)
            .build();
        client.use_middleware(Box::new(breaker))?;
        Ok(())
    }
    ```

## Next steps

- [Middleware System](../features/middleware-system.md) — hook contracts, context namespacing, built-in middleware reference
- [Execution Pipeline](../features/execution-pipeline.md) — steps, strategies and step middleware
- [Testing Modules](./testing-modules.md#5-testing-middleware) — testing your middleware
- [Core Executor](../features/core-executor.md) — the full call pipeline
