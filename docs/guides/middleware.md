---
description: "How to write apcore middleware with before/after hooks for logging, metrics, error handling, data transformation, caching, and rate limiting around module execution."
---

# Middleware Guide

> Use middleware to extend logic before and after module execution.

## 1. Overview

Middleware are hooks that run before and after module execution, used for:

- **Logging**: Record invocation logs
- **Performance Monitoring**: Collect execution time and metrics
- **Error Handling**: Unified exception handling
- **Data Transformation**: Modify inputs/outputs
- **Caching**: Cache execution results
- **Rate Limiting**: Control invocation frequency

---

## 2. Middleware Interface

=== "Python"

    ```python
    from typing import Any
    from apcore import Context, Middleware


    class MyMiddleware(Middleware):
        """Middleware base class"""

        def before(
            self,
            module_id: str,
            inputs: dict[str, Any],
            context: Context,
        ) -> dict[str, Any] | None:
            """
            Called before module execution.

            Args:
                module_id: Module ID
                inputs: Input parameters
                context: Invocation context

            Returns:
                Modified inputs, or None to keep unchanged.
            """
            return None

        def after(
            self,
            module_id: str,
            inputs: dict[str, Any],
            output: dict[str, Any],
            context: Context,
        ) -> dict[str, Any] | None:
            """
            Called after module execution.

            Returns:
                Modified output, or None to keep unchanged.
            """
            return None

        def on_error(
            self,
            module_id: str,
            inputs: dict[str, Any],
            error: Exception,
            context: Context,
        ) -> dict[str, Any] | None:
            """
            Called when module execution fails.

            Returns:
                Alternative output (for error recovery), or None to continue
                raising the exception.
            """
            return None
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class MyMiddleware extends Middleware {
      constructor() {
        // Priority 0-1000; higher runs first. Default is 100.
        super(100);
      }

      // Called before module execution. Return modified inputs, or null to
      // keep them unchanged. May also return a Promise of either.
      override before(
        moduleId: string,
        inputs: Record<string, unknown>,
        context: Context,
      ): Record<string, unknown> | null {
        return null;
      }

      // Called after successful module execution. Return modified output,
      // or null to keep it unchanged.
      override after(
        moduleId: string,
        inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ): Record<string, unknown> | null {
        return null;
      }

      // Called when module execution fails. Return a recovery value to
      // "swallow" the error, or null to let it propagate.
      override onError(
        moduleId: string,
        inputs: Record<string, unknown>,
        error: Error,
        context: Context,
      ): Record<string, unknown> | null {
        return null;
      }
    }
    ```

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

        // Priority 0-1000; higher runs first. Default is 100.
        fn priority(&self) -> u16 {
            100
        }

        /// Called before module execution. Return `Some(inputs)` to modify,
        /// or `None` to leave unchanged.
        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        /// Called after successful module execution.
        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        /// Called when module execution fails. Return `Some(value)` to
        /// recover, or `None` to let the error propagate.
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

---

## 3. Quick Start

### 3.1 Create Simple Middleware

=== "Python"

    ```python
    from apcore import Middleware, Context


    class SimpleLoggingMiddleware(Middleware):
        """Logging middleware"""

        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            print(f"[{context.trace_id}] Calling {module_id}")
            # Use context.redacted_inputs to avoid leaking sensitive data
            print(f"  Inputs: {context.redacted_inputs}")

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            print(f"[{context.trace_id}] {module_id} completed")
            print(f"  Output: {output}")

        def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
            print(f"[{context.trace_id}] {module_id} failed: {error}")
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class SimpleLoggingMiddleware extends Middleware {
      override before(
        moduleId: string,
        inputs: Record<string, unknown>,
        context: Context,
      ): null {
        console.log(`[${context.traceId}] Calling ${moduleId}`);
        // Use context.redactedInputs to avoid leaking sensitive data
        console.log(`  Inputs:`, context.redactedInputs);
        return null;
      }

      override after(
        moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ): null {
        console.log(`[${context.traceId}] ${moduleId} completed`);
        console.log(`  Output:`, output);
        return null;
      }

      override onError(
        moduleId: string,
        _inputs: Record<string, unknown>,
        error: Error,
        context: Context,
      ): null {
        console.error(`[${context.traceId}] ${moduleId} failed: ${error}`);
        return null;
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::Value;

    #[derive(Debug)]
    struct SimpleLoggingMiddleware;

    #[async_trait]
    impl Middleware for SimpleLoggingMiddleware {
        fn name(&self) -> &str {
            "simple_logging"
        }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            println!("[{}] Calling {}", ctx.trace_id(), module_id);
            // Use ctx.redacted_inputs() to avoid leaking sensitive data
            println!("  Inputs: {:?}", ctx.redacted_inputs());
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: Value,
            output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            println!("[{}] {} completed", ctx.trace_id(), module_id);
            println!("  Output: {}", output);
            Ok(None)
        }

        async fn on_error(
            &self,
            module_id: &str,
            _inputs: Value,
            error: &ModuleError,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            eprintln!("[{}] {} failed: {}", ctx.trace_id(), module_id, error);
            Ok(None)
        }
    }
    ```

### 3.2 Using Middleware

=== "Python"

    ```python
    from apcore import Registry, Executor

    registry = Registry(extensions_dir="./extensions")
    registry.discover()

    executor = Executor(
        registry=registry,
        middlewares=[SimpleLoggingMiddleware()],
    )

    # Middleware automatically executes when calling modules
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"},
    )
    ```

=== "TypeScript"

    ```typescript
    import { Registry, Executor } from 'apcore-js';

    const registry = new Registry({ extensionsDir: './extensions' });
    await registry.discover();

    const executor = new Executor({
      registry,
      middlewares: [new SimpleLoggingMiddleware()],
    });

    // Middleware automatically executes when calling modules
    const result = await executor.call(
      'executor.email.send_email',
      { to: 'user@example.com', subject: 'Hi', body: 'Hello' },
    );
    ```

=== "Rust"

    ```rust
    use apcore::executor::Executor;
    use apcore::registry::Registry;
    use apcore::config::Config;
    use serde_json::json;
    use std::sync::Arc;

    let registry = Arc::new(Registry::from_extensions_dir("./extensions")?);
    let executor = Executor::new(registry, Config::default());

    // Middleware automatically executes when calling modules
    executor.use_middleware(Box::new(SimpleLoggingMiddleware))?;

    let result = executor
        .call(
            "executor.email.send_email",
            json!({ "to": "user@example.com", "subject": "Hi", "body": "Hello" }),
            None,
        )
        .await?;
    ```

Output:

```
[abc-123] Calling executor.email.send_email
  Inputs: {'to': 'user@example.com', 'subject': 'Hi', 'body': 'Hello'}
[abc-123] executor.email.send_email completed
  Output: {'success': True, 'message_id': 'msg_456'}
```

---

## 4. Execution Model

### 4.1 Onion Model

Middleware executes in an onion model:

```
Request → MW1.before → MW2.before → MW3.before → Module execution
                                                    ↓
Response ← MW1.after  ← MW2.after  ← MW3.after  ←  Output result
```

### 4.2 Execution Order

=== "Python"

    ```python
    from apcore import Executor

    executor = Executor(
        registry=registry,
        middlewares=[
            MiddlewareA(),  # First
            MiddlewareB(),  # Second
            MiddlewareC(),  # Third
        ],
    )
    ```

=== "TypeScript"

    ```typescript
    import { Executor } from 'apcore-js';

    const executor = new Executor({
      registry,
      middlewares: [
        new MiddlewareA(), // First
        new MiddlewareB(), // Second
        new MiddlewareC(), // Third
      ],
    });
    ```

=== "Rust"

    ```rust
    use apcore::executor::Executor;
    use apcore::config::Config;

    let executor = Executor::new(registry, Config::default());

    // Add in registration order; equal priority preserves insertion order.
    executor.use_middleware(Box::new(MiddlewareA))?; // First
    executor.use_middleware(Box::new(MiddlewareB))?; // Second
    executor.use_middleware(Box::new(MiddlewareC))?; // Third
    ```

Execution order:

```
1. MiddlewareA.before()
2. MiddlewareB.before()
3. MiddlewareC.before()
4. module.execute()
5. MiddlewareC.after()
6. MiddlewareB.after()
7. MiddlewareA.after()
```

### 4.3 Error Handling

```
Request → MW1.before → MW2.before → Module execution (Exception!)
                                      ↓
      MW1.on_error ← MW2.on_error ← Exception propagation
```

If any `on_error` returns non-None, the exception is "swallowed" and that value is returned as the result.

### 4.4 Middleware Execution State Machine

```
  ┌──────┐    ┌────────────────┐    ┌──────────┐    ┌───────────────┐    ┌──────┐
  │ init │───▶│ before (order) │───▶│ execute  │───▶│ after (reverse)│───▶│ done │
  └──────┘    └───────┬────────┘    └────┬─────┘    └───────┬───────┘    └──────┘
                      │                  │                  │
                      │ exception        │ exception        │ exception
                      ▼                  ▼                  ▼
                  ┌──────────┐      ┌──────────┐       ┌──────────┐
                  │ skip rest│      │ on_error │       │ on_error │
                  │ go error │      │ (reverse)│       │ (reverse)│
                 └──────────┘      └──────────┘       └──────────┘
```

**Precise exception handling semantics:**

| Stage | Exception Behavior | Description |
|------|---------|------|
| `before()` stage | Skip remaining before and execute, enter on_error chain | e.g., rate limit rejection |
| `execute()` stage | Enter on_error chain (reverse order) | Module execution failure |
| `after()` stage | Enter on_error chain (reverse order); remaining after hooks are skipped | After-stage errors should be handled uniformly |
| `on_error()` stage | Log error but **should** continue executing remaining on_error | Error handling should not fail again |

**Thread safety requirements:**

- Middleware instances **can** be shared by multiple Executors
- Middleware's `before()`/`after()`/`on_error()` **must** be thread-safe
- When using instance variables to store state, **must** use thread-safe data structures (each SDK provides its own primitives — e.g., `threading.Lock` in Python, `Mutex` in Rust)
- Using `context.data` to pass middleware state is the **recommended** thread-safe approach

---

## 5. Common Middleware

### 5.1 Logging Middleware

=== "Python"

    ```python
    import logging
    from datetime import datetime
    from apcore import Middleware, Context


    class DetailedLoggingMiddleware(Middleware):
        """Detailed logging middleware"""

        def __init__(self, logger: logging.Logger | None = None):
            self.logger = logger or logging.getLogger("apcore")
            self._start_times: dict[str, datetime] = {}

        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            self._start_times[context.trace_id] = datetime.now()
            # Security: use context.redacted_inputs to avoid leaking sensitive data
            self.logger.info(
                f"[{context.trace_id}] START {module_id}",
                extra={
                    "trace_id": context.trace_id,
                    "module_id": module_id,
                    "caller_id": context.caller_id,
                    "inputs": context.redacted_inputs,
                },
            )

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            start = self._start_times.pop(context.trace_id, datetime.now())
            duration = (datetime.now() - start).total_seconds() * 1000

            self.logger.info(
                f"[{context.trace_id}] END {module_id} ({duration:.2f}ms)",
                extra={
                    "trace_id": context.trace_id,
                    "module_id": module_id,
                    "duration_ms": duration,
                    "output": output,
                },
            )

        def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
            # Security: use context.redacted_inputs instead of raw inputs
            self.logger.error(
                f"[{context.trace_id}] ERROR {module_id}: {error}",
                extra={
                    "trace_id": context.trace_id,
                    "module_id": module_id,
                    "error": str(error),
                    "inputs": context.redacted_inputs,
                },
                exc_info=True,
            )
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    interface Logger {
      info(message: string, extra?: Record<string, unknown>): void;
      error(message: string, extra?: Record<string, unknown>): void;
    }

    const defaultLogger: Logger = {
      info: (msg, extra) => console.info(msg, extra ?? ''),
      error: (msg, extra) => console.error(msg, extra ?? ''),
    };

    class DetailedLoggingMiddleware extends Middleware {
      private readonly logger: Logger;

      constructor(logger: Logger = defaultLogger) {
        super();
        this.logger = logger;
      }

      override before(
        moduleId: string,
        _inputs: Record<string, unknown>,
        context: Context,
      ): null {
        context.data['_apcore.mw.logging.start_time'] = performance.now();
        // Security: use context.redactedInputs to avoid leaking sensitive data
        this.logger.info(`[${context.traceId}] START ${moduleId}`, {
          traceId: context.traceId,
          moduleId,
          callerId: context.callerId,
          inputs: context.redactedInputs,
        });
        return null;
      }

      override after(
        moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ): null {
        const start = (context.data['_apcore.mw.logging.start_time'] as number) ?? performance.now();
        const durationMs = performance.now() - start;
        this.logger.info(
          `[${context.traceId}] END ${moduleId} (${durationMs.toFixed(2)}ms)`,
          {
            traceId: context.traceId,
            moduleId,
            durationMs,
            output,
          },
        );
        return null;
      }

      override onError(
        moduleId: string,
        _inputs: Record<string, unknown>,
        error: Error,
        context: Context,
      ): null {
        // Security: use context.redactedInputs instead of raw inputs
        this.logger.error(`[${context.traceId}] ERROR ${moduleId}: ${error}`, {
          traceId: context.traceId,
          moduleId,
          error: String(error),
          inputs: context.redactedInputs,
        });
        return null;
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::time::Instant;

    #[derive(Debug, Default)]
    struct DetailedLoggingMiddleware {
        start_times: Mutex<HashMap<String, Instant>>,
    }

    #[async_trait]
    impl Middleware for DetailedLoggingMiddleware {
        fn name(&self) -> &str {
            "detailed_logging"
        }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            self.start_times
                .lock()
                .insert(ctx.trace_id().to_string(), Instant::now());
            // Security: use ctx.redacted_inputs() to avoid leaking sensitive data
            tracing::info!(
                trace_id = %ctx.trace_id(),
                module_id = %module_id,
                caller_id = ?ctx.caller_id(),
                inputs = ?ctx.redacted_inputs(),
                "[{}] START {}",
                ctx.trace_id(),
                module_id,
            );
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: Value,
            output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let start = self
                .start_times
                .lock()
                .remove(ctx.trace_id())
                .unwrap_or_else(Instant::now);
            let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
            tracing::info!(
                trace_id = %ctx.trace_id(),
                module_id = %module_id,
                duration_ms = duration_ms,
                output = ?output,
                "[{}] END {} ({:.2}ms)",
                ctx.trace_id(),
                module_id,
                duration_ms,
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
            // Security: use ctx.redacted_inputs() instead of raw inputs
            tracing::error!(
                trace_id = %ctx.trace_id(),
                module_id = %module_id,
                error = %error,
                inputs = ?ctx.redacted_inputs(),
                "[{}] ERROR {}: {}",
                ctx.trace_id(),
                module_id,
                error,
            );
            Ok(None)
        }
    }
    ```

### 5.2 Performance Monitoring Middleware

=== "Python"

    ```python
    import time
    from apcore import Middleware, Context


    class MetricsMiddleware(Middleware):
        """Performance metrics collection middleware"""

        def __init__(self):
            self.call_counts: dict[str, int] = {}
            self.durations: dict[str, list[float]] = {}
            self.error_counts: dict[str, int] = {}
            self._start_times: dict[str, float] = {}

        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            self._start_times[context.trace_id] = time.perf_counter()
            self.call_counts[module_id] = self.call_counts.get(module_id, 0) + 1

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            start = self._start_times.pop(context.trace_id, time.perf_counter())
            duration = time.perf_counter() - start
            self.durations.setdefault(module_id, []).append(duration)

        def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
            self.error_counts[module_id] = self.error_counts.get(module_id, 0) + 1

        def get_stats(self, module_id: str) -> dict:
            """Get module statistics"""
            durations = self.durations.get(module_id, [])
            return {
                "call_count": self.call_counts.get(module_id, 0),
                "error_count": self.error_counts.get(module_id, 0),
                "avg_duration": sum(durations) / len(durations) if durations else 0,
                "min_duration": min(durations) if durations else 0,
                "max_duration": max(durations) if durations else 0,
            }
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    interface Stats {
      callCount: number;
      errorCount: number;
      avgDuration: number;
      minDuration: number;
      maxDuration: number;
    }

    class MetricsMiddleware extends Middleware {
      private callCounts = new Map<string, number>();
      private durations = new Map<string, number[]>();
      private errorCounts = new Map<string, number>();

      override before(
        moduleId: string,
        _inputs: Record<string, unknown>,
        context: Context,
      ): null {
        context.data['_apcore.mw.metrics.start_time'] = performance.now();
        this.callCounts.set(moduleId, (this.callCounts.get(moduleId) ?? 0) + 1);
        return null;
      }

      override after(
        moduleId: string,
        _inputs: Record<string, unknown>,
        _output: Record<string, unknown>,
        context: Context,
      ): null {
        const start = (context.data['_apcore.mw.metrics.start_time'] as number) ?? performance.now();
        const duration = (performance.now() - start) / 1000;
        const list = this.durations.get(moduleId) ?? [];
        list.push(duration);
        this.durations.set(moduleId, list);
        return null;
      }

      override onError(
        moduleId: string,
        _inputs: Record<string, unknown>,
        _error: Error,
        _context: Context,
      ): null {
        this.errorCounts.set(moduleId, (this.errorCounts.get(moduleId) ?? 0) + 1);
        return null;
      }

      getStats(moduleId: string): Stats {
        const durations = this.durations.get(moduleId) ?? [];
        const sum = durations.reduce((a, b) => a + b, 0);
        return {
          callCount: this.callCounts.get(moduleId) ?? 0,
          errorCount: this.errorCounts.get(moduleId) ?? 0,
          avgDuration: durations.length ? sum / durations.length : 0,
          minDuration: durations.length ? Math.min(...durations) : 0,
          maxDuration: durations.length ? Math.max(...durations) : 0,
        };
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::time::Instant;

    #[derive(Debug, Default)]
    struct MetricsState {
        call_counts: HashMap<String, u64>,
        durations: HashMap<String, Vec<f64>>,
        error_counts: HashMap<String, u64>,
        start_times: HashMap<String, Instant>,
    }

    #[derive(Debug, Default)]
    struct MetricsMiddleware {
        state: Mutex<MetricsState>,
    }

    #[derive(Debug)]
    pub struct Stats {
        pub call_count: u64,
        pub error_count: u64,
        pub avg_duration: f64,
        pub min_duration: f64,
        pub max_duration: f64,
    }

    impl MetricsMiddleware {
        pub fn get_stats(&self, module_id: &str) -> Stats {
            let s = self.state.lock();
            let durations = s.durations.get(module_id).cloned().unwrap_or_default();
            let sum: f64 = durations.iter().sum();
            Stats {
                call_count: *s.call_counts.get(module_id).unwrap_or(&0),
                error_count: *s.error_counts.get(module_id).unwrap_or(&0),
                avg_duration: if durations.is_empty() {
                    0.0
                } else {
                    sum / durations.len() as f64
                },
                min_duration: if durations.is_empty() {
                    0.0
                } else {
                    durations.iter().copied().fold(f64::INFINITY, f64::min)
                },
                max_duration: if durations.is_empty() {
                    0.0
                } else {
                    durations.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                },
            }
        }
    }

    #[async_trait]
    impl Middleware for MetricsMiddleware {
        fn name(&self) -> &str {
            "metrics"
        }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let mut s = self.state.lock();
            s.start_times
                .insert(ctx.trace_id().to_string(), Instant::now());
            *s.call_counts.entry(module_id.to_string()).or_insert(0) += 1;
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: Value,
            _output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let mut s = self.state.lock();
            let start = s
                .start_times
                .remove(ctx.trace_id())
                .unwrap_or_else(Instant::now);
            let duration = start.elapsed().as_secs_f64();
            s.durations
                .entry(module_id.to_string())
                .or_default()
                .push(duration);
            Ok(None)
        }

        async fn on_error(
            &self,
            module_id: &str,
            _inputs: Value,
            _error: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let mut s = self.state.lock();
            *s.error_counts.entry(module_id.to_string()).or_insert(0) += 1;
            Ok(None)
        }
    }
    ```

### 5.3 Cache Middleware

!!! note "Returning cached output from `before()`"
    Returning a non-None value from `before()` modifies the inputs that the
    module will receive. To short-circuit execution and return a cached
    output as the final result, use a custom pipeline step or signal it via
    `context.data` and let an outer wrapper handle it. The example below
    illustrates the timing/key-derivation pattern; production caching should
    use a dedicated step or a recovery in `on_error`.

=== "Python"

    ```python
    import hashlib
    import json
    import time
    from apcore import Middleware, Context


    class CacheMiddleware(Middleware):
        """Simple cache middleware (illustrative)"""

        def __init__(self, ttl_seconds: int = 300):
            self.cache: dict[str, tuple[dict, float]] = {}
            self.ttl = ttl_seconds

        def _cache_key(self, module_id: str, inputs: dict) -> str:
            content = json.dumps({"module_id": module_id, "inputs": inputs}, sort_keys=True)
            return hashlib.md5(content.encode()).hexdigest()

        def before(self, module_id: str, inputs: dict, context: Context) -> dict | None:
            key = self._cache_key(module_id, inputs)
            if key in self.cache:
                cached_output, cached_time = self.cache[key]
                if time.time() - cached_time < self.ttl:
                    # Cache hit; record for after() and downstream consumers.
                    context.data["_apcore.mw.cache.hit"] = True
                    context.data["_apcore.mw.cache.value"] = cached_output
            context.data["_apcore.mw.cache.key"] = key
            return None

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            if context.data.get("_apcore.mw.cache.hit"):
                return
            key = context.data.get("_apcore.mw.cache.key")
            if key:
                self.cache[key] = (output, time.time())
    ```

=== "TypeScript"

    ```typescript
    import { createHash } from 'node:crypto';
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    interface CacheEntry {
      output: Record<string, unknown>;
      time: number;
    }

    class CacheMiddleware extends Middleware {
      private cache = new Map<string, CacheEntry>();

      constructor(private readonly ttlSeconds: number = 300) {
        super();
      }

      private cacheKey(moduleId: string, inputs: Record<string, unknown>): string {
        const content = JSON.stringify({ moduleId, inputs }, Object.keys({ moduleId, inputs }).sort());
        return createHash('md5').update(content).digest('hex');
      }

      override before(
        moduleId: string,
        inputs: Record<string, unknown>,
        context: Context,
      ): null {
        const key = this.cacheKey(moduleId, inputs);
        const entry = this.cache.get(key);
        if (entry && Date.now() / 1000 - entry.time < this.ttlSeconds) {
          context.data['_apcore.mw.cache.hit'] = true;
          context.data['_apcore.mw.cache.value'] = entry.output;
        }
        context.data['_apcore.mw.cache.key'] = key;
        return null;
      }

      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ): null {
        if (context.data['_apcore.mw.cache.hit']) return null;
        const key = context.data['_apcore.mw.cache.key'] as string | undefined;
        if (key) {
          this.cache.set(key, { output, time: Date.now() / 1000 });
        }
        return null;
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use md5::{Digest, Md5};
    use parking_lot::Mutex;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug)]
    struct CacheMiddleware {
        cache: Mutex<HashMap<String, (Value, f64)>>,
        ttl_seconds: f64,
    }

    impl CacheMiddleware {
        pub fn new(ttl_seconds: f64) -> Self {
            Self {
                cache: Mutex::new(HashMap::new()),
                ttl_seconds,
            }
        }

        fn cache_key(module_id: &str, inputs: &Value) -> String {
            let canonical = json!({ "module_id": module_id, "inputs": inputs }).to_string();
            let mut hasher = Md5::new();
            hasher.update(canonical.as_bytes());
            format!("{:x}", hasher.finalize())
        }

        fn now_secs() -> f64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        }
    }

    #[async_trait]
    impl Middleware for CacheMiddleware {
        fn name(&self) -> &str {
            "cache"
        }

        async fn before(
            &self,
            module_id: &str,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let key = Self::cache_key(module_id, &inputs);
            if let Some((cached_output, cached_time)) = self.cache.lock().get(&key).cloned() {
                if Self::now_secs() - cached_time < self.ttl_seconds {
                    ctx.data().insert("_apcore.mw.cache.hit", json!(true));
                    ctx.data().insert("_apcore.mw.cache.value", cached_output);
                }
            }
            ctx.data().insert("_apcore.mw.cache.key", json!(key));
            Ok(None)
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if ctx.data().get("_apcore.mw.cache.hit").is_some() {
                return Ok(None);
            }
            if let Some(Value::String(key)) = ctx.data().get("_apcore.mw.cache.key") {
                self.cache.lock().insert(key, (output, Self::now_secs()));
            }
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
    ```

### 5.4 RetryMiddleware (Built-in)

apcore provides a built-in `RetryMiddleware` with configurable backoff strategies. It only retries errors marked `retryable=True`.

=== "Python"

    ```python
    from apcore import RetryMiddleware, RetryConfig

    # Default: 3 retries, exponential backoff, jitter enabled
    executor.use(RetryMiddleware())

    # Custom configuration
    executor.use(RetryMiddleware(RetryConfig(
        max_retries=5,
        strategy="fixed",          # "exponential" (default) or "fixed"
        base_delay_ms=200,         # Base delay between retries (default: 100)
        max_delay_ms=10000,        # Cap for exponential growth (default: 5000)
        jitter=True,               # Add 0.5-1.5x random multiplier (default: True)
    )))
    ```

=== "TypeScript"

    ```typescript
    import { RetryHintMiddleware } from 'apcore-js';

    // Default: 3 retries, exponential backoff, jitter enabled.
    // Note: in apcore-js this middleware writes retry hints to context.data
    // (CTX_RETRY_COUNT_PREFIX / CTX_RETRY_DELAY_PREFIX) for an outer retry
    // loop; the error still propagates so the caller controls the retry.
    executor.use(new RetryHintMiddleware());

    // Custom configuration
    executor.use(new RetryHintMiddleware({
      maxRetries: 5,
      strategy: 'fixed',         // 'exponential' (default) or 'fixed'
      baseDelayMs: 200,          // Base delay between retries (default: 100)
      maxDelayMs: 10000,         // Cap for exponential growth (default: 5000)
      jitter: true,              // Add 0.5-1.5x random multiplier (default: true)
    }));
    ```

=== "Rust"

    ```rust
    use apcore::middleware::{RetryConfig, RetryMiddleware};

    // Default: 3 retries, exponential backoff, jitter enabled
    executor.use_middleware(Box::new(RetryMiddleware::new(RetryConfig::default())))?;

    // Custom configuration
    executor.use_middleware(Box::new(RetryMiddleware::new(RetryConfig {
        max_retries: 5,
        strategy: "fixed".to_string(), // "exponential" (default) or "fixed"
        base_delay_ms: 200,            // Base delay between retries (default: 100)
        max_delay_ms: 10_000,          // Cap for exponential growth (default: 5000)
        jitter: true,                  // Add 0.5-1.5x random multiplier (default: true)
        ..RetryConfig::default()
    })))?;
    ```

**`RetryConfig` fields:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_retries` | int | 3 | Maximum retry attempts |
| `strategy` | str | `"exponential"` | `"exponential"` (base × 2^attempt) or `"fixed"` (constant delay) |
| `base_delay_ms` | int | 100 | Base delay in milliseconds |
| `max_delay_ms` | int | 5000 | Maximum delay cap (exponential only) |
| `jitter` | bool | True | Add random 0.5–1.5x multiplier to prevent thundering herd |

**Retry logic:**
- Only retries errors where `error.retryable is True` (each error code has a default, see PROTOCOL_SPEC §8.6).
- Tracks retry count per module in `context.data` using key `_retry_count_{module_id}`.
- In async pipelines, `on_error` is dispatched to a worker thread so the event loop is not blocked.

### 5.5 ErrorHistoryMiddleware (Built-in)

Records `ModuleError` instances into `ErrorHistory` for health monitoring. Automatically registered by `register_sys_modules()`. See [Observability](../features/observability.md#error-history).

### 5.6 UsageMiddleware (Built-in)

Records call counts and latency into `UsageCollector` for usage analytics. Automatically registered by `register_sys_modules()`. See [Observability](../features/observability.md#usage-collector).

### 5.7 PlatformNotifyMiddleware (Built-in)

Emits events when module error rates or p99 latency exceed configured thresholds. Automatically registered when `sys_modules.events.enabled: true`. See [Event System](../features/event-system.md).

### 5.8 Rate Limit Middleware

=== "Python"

    ```python
    import time
    from collections import deque
    from apcore import Middleware, Context, ModuleError


    class RateLimitMiddleware(Middleware):
        """Rate limiting middleware"""

        def __init__(self, max_calls: int = 100, window_seconds: int = 60):
            self.max_calls = max_calls
            self.window = window_seconds
            self._call_times: dict[str, deque] = {}

        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            now = time.time()
            calls = self._call_times.setdefault(module_id, deque())

            # Remove calls outside the window
            while calls and calls[0] < now - self.window:
                calls.popleft()

            # Check if limit is exceeded
            if len(calls) >= self.max_calls:
                raise ModuleError(
                    f"Rate limit exceeded for {module_id}",
                    module_id=module_id,
                    trace_id=context.trace_id,
                )

            # Record current call
            calls.append(now)
    ```

=== "TypeScript"

    ```typescript
    import { Middleware, ModuleError } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class RateLimitMiddleware extends Middleware {
      private readonly callTimes = new Map<string, number[]>();

      constructor(
        private readonly maxCalls: number = 100,
        private readonly windowSeconds: number = 60,
      ) {
        super();
      }

      override before(
        moduleId: string,
        _inputs: Record<string, unknown>,
        context: Context,
      ): null {
        const now = Date.now() / 1000;
        let calls = this.callTimes.get(moduleId);
        if (!calls) {
          calls = [];
          this.callTimes.set(moduleId, calls);
        }

        // Remove calls outside the window
        while (calls.length > 0 && calls[0]! < now - this.windowSeconds) {
          calls.shift();
        }

        // Check if limit is exceeded
        if (calls.length >= this.maxCalls) {
          throw new ModuleError(`Rate limit exceeded for ${moduleId}`, {
            moduleId,
            traceId: context.traceId,
          });
        }

        calls.push(now);
        return null;
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use serde_json::Value;
    use std::collections::{HashMap, VecDeque};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug)]
    struct RateLimitMiddleware {
        max_calls: usize,
        window_seconds: f64,
        call_times: Mutex<HashMap<String, VecDeque<f64>>>,
    }

    impl RateLimitMiddleware {
        pub fn new(max_calls: usize, window_seconds: f64) -> Self {
            Self {
                max_calls,
                window_seconds,
                call_times: Mutex::new(HashMap::new()),
            }
        }

        fn now_secs() -> f64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        }
    }

    #[async_trait]
    impl Middleware for RateLimitMiddleware {
        fn name(&self) -> &str {
            "rate_limit"
        }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let now = Self::now_secs();
            let mut map = self.call_times.lock();
            let calls = map.entry(module_id.to_string()).or_default();

            // Remove calls outside the window
            while let Some(&front) = calls.front() {
                if front < now - self.window_seconds {
                    calls.pop_front();
                } else {
                    break;
                }
            }

            if calls.len() >= self.max_calls {
                return Err(ModuleError::new(
                    ErrorCode::ModuleExecuteError,
                    format!("Rate limit exceeded for {}", module_id),
                ));
            }

            calls.push_back(now);
            Ok(None)
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
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
    ```

---

## 6. Modifying Inputs/Outputs

### 6.1 Modifying Inputs

=== "Python"

    ```python
    from apcore import Middleware, Context


    class InputTransformMiddleware(Middleware):
        """Input transformation middleware"""

        def before(self, module_id: str, inputs: dict, context: Context) -> dict:
            modified = inputs.copy()

            # Add default values
            if "timeout" not in modified:
                modified["timeout"] = 30

            # Add trace information
            modified["_trace_id"] = context.trace_id

            return modified
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class InputTransformMiddleware extends Middleware {
      override before(
        _moduleId: string,
        inputs: Record<string, unknown>,
        context: Context,
      ): Record<string, unknown> {
        const modified = { ...inputs };

        // Add default values
        if (!('timeout' in modified)) {
          modified['timeout'] = 30;
        }

        // Add trace information
        modified['_trace_id'] = context.traceId;

        return modified;
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    #[derive(Debug)]
    struct InputTransformMiddleware;

    #[async_trait]
    impl Middleware for InputTransformMiddleware {
        fn name(&self) -> &str {
            "input_transform"
        }

        async fn before(
            &self,
            _module_id: &str,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let mut modified = inputs.clone();
            if let Some(obj) = modified.as_object_mut() {
                obj.entry("timeout".to_string()).or_insert(json!(30));
                obj.insert("_trace_id".to_string(), json!(ctx.trace_id()));
            }
            Ok(Some(modified))
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
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
    ```

### 6.2 Modifying Outputs

=== "Python"

    ```python
    from datetime import datetime
    from apcore import Middleware, Context


    class OutputEnrichMiddleware(Middleware):
        """Output enrichment middleware"""

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> dict:
            enriched = output.copy()
            enriched["_metadata"] = {
                "module_id": module_id,
                "trace_id": context.trace_id,
                "timestamp": datetime.now().isoformat(),
            }
            return enriched
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class OutputEnrichMiddleware extends Middleware {
      override after(
        moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ): Record<string, unknown> {
        return {
          ...output,
          _metadata: {
            moduleId,
            traceId: context.traceId,
            timestamp: new Date().toISOString(),
          },
        };
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::{json, Value};

    #[derive(Debug)]
    struct OutputEnrichMiddleware;

    #[async_trait]
    impl Middleware for OutputEnrichMiddleware {
        fn name(&self) -> &str {
            "output_enrich"
        }

        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: Value,
            output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let mut enriched = output.clone();
            if let Some(obj) = enriched.as_object_mut() {
                obj.insert(
                    "_metadata".to_string(),
                    json!({
                        "module_id": module_id,
                        "trace_id": ctx.trace_id(),
                        "timestamp": Utc::now().to_rfc3339(),
                    }),
                );
            }
            Ok(Some(enriched))
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
    ```

### 6.3 Error Recovery

=== "Python"

    ```python
    from apcore import Middleware, Context


    class FallbackMiddleware(Middleware):
        """Fallback middleware"""

        def __init__(self, fallback_values: dict[str, dict]):
            """
            Args:
                fallback_values: {module_id: default_output}
            """
            self.fallback_values = fallback_values

        def on_error(
            self,
            module_id: str,
            inputs: dict,
            error: Exception,
            context: Context,
        ) -> dict | None:
            if module_id in self.fallback_values:
                # Return fallback value, "swallow" exception
                return self.fallback_values[module_id]
            return None  # Continue raising exception
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class FallbackMiddleware extends Middleware {
      constructor(
        private readonly fallbackValues: Record<string, Record<string, unknown>>,
      ) {
        super();
      }

      override onError(
        moduleId: string,
        _inputs: Record<string, unknown>,
        _error: Error,
        _context: Context,
      ): Record<string, unknown> | null {
        const fallback = this.fallbackValues[moduleId];
        if (fallback !== undefined) {
          // Return fallback value, "swallow" the error
          return fallback;
        }
        return null; // Let the error propagate
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::HashMap;

    #[derive(Debug)]
    struct FallbackMiddleware {
        fallback_values: HashMap<String, Value>,
    }

    impl FallbackMiddleware {
        pub fn new(fallback_values: HashMap<String, Value>) -> Self {
            Self { fallback_values }
        }
    }

    #[async_trait]
    impl Middleware for FallbackMiddleware {
        fn name(&self) -> &str {
            "fallback"
        }

        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        async fn on_error(
            &self,
            module_id: &str,
            _inputs: Value,
            _error: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            // Some(value) returns fallback and swallows the error;
            // None lets the error propagate.
            Ok(self.fallback_values.get(module_id).cloned())
        }
    }
    ```

---

## 7. Middleware Composition

### 7.1 Function-first Middleware (Function-first API)

In addition to class-based middleware, you can directly register functions using `use_before()` / `use_after()` (Python/Rust) or `useBefore()` / `useAfter()` (TypeScript):

=== "Python"

    ```python
    from apcore import Executor, Context

    executor = Executor(registry=registry)

    # Register before hook
    executor.use_before(
        lambda module_id, inputs, ctx: print(f"Calling {module_id}")
    )

    # Register after hook
    executor.use_after(
        lambda module_id, inputs, output, ctx: print(f"Done {module_id}")
    )

    # Can also use regular functions
    def log_before(module_id: str, inputs: dict, context: Context) -> None:
        context.logger.info(f"START {module_id}")

    def log_after(module_id: str, inputs: dict, output: dict, context: Context) -> None:
        context.logger.info(f"END {module_id}")

    executor.use_before(log_before)
    executor.use_after(log_after)
    ```

=== "TypeScript"

    ```typescript
    import { Executor } from 'apcore-js';
    import type { Context } from 'apcore-js';

    const executor = new Executor({ registry });

    // Register before hook
    executor.useBefore((moduleId, _inputs, _ctx) => {
      console.log(`Calling ${moduleId}`);
      return null;
    });

    // Register after hook
    executor.useAfter((moduleId, _inputs, _output, _ctx) => {
      console.log(`Done ${moduleId}`);
      return null;
    });

    // Can also use regular functions
    function logBefore(
      moduleId: string,
      _inputs: Record<string, unknown>,
      _context: Context,
    ) {
      console.info(`START ${moduleId}`);
      return null;
    }

    function logAfter(
      moduleId: string,
      _inputs: Record<string, unknown>,
      _output: Record<string, unknown>,
      _context: Context,
    ) {
      console.info(`END ${moduleId}`);
      return null;
    }

    executor.useBefore(logBefore);
    executor.useAfter(logAfter);
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::adapters::{BeforeAdapter, AfterAdapter};
    use serde_json::Value;

    // Register a before hook with an async closure adapter
    executor.use_middleware(Box::new(BeforeAdapter::new(
        "log_before",
        |module_id: String, _inputs: Value, _ctx: Context<Value>| async move {
            println!("Calling {}", module_id);
            Ok::<Option<Value>, ModuleError>(None)
        },
    )))?;

    // Register an after hook with an async closure adapter
    executor.use_middleware(Box::new(AfterAdapter::new(
        "log_after",
        |module_id: String, _inputs: Value, _output: Value, _ctx: Context<Value>| async move {
            println!("Done {}", module_id);
            Ok::<Option<Value>, ModuleError>(None)
        },
    )))?;
    ```

**Use cases:**

| Approach | Use Case |
|------|---------|
| Class-based (`Middleware`) | Need `on_error`, need shared state, complex logic |
| Function-first (`use_before`/`use_after`) | Simple hooks, rapid prototyping, single concern |

### 7.2 Chaining

=== "Python"

    ```python
    from apcore import Executor, RetryMiddleware, RetryConfig

    executor = Executor(registry=registry)

    # Chain middleware
    executor \
        .use(LoggingMiddleware()) \
        .use(MetricsMiddleware()) \
        .use(CacheMiddleware(ttl_seconds=60)) \
        .use(RetryMiddleware(RetryConfig(max_retries=3)))
    ```

=== "TypeScript"

    ```typescript
    import { Executor, RetryHintMiddleware } from 'apcore-js';

    const executor = new Executor({ registry });

    // Chain middleware
    executor
      .use(new LoggingMiddleware())
      .use(new MetricsMiddleware())
      .use(new CacheMiddleware(60))
      .use(new RetryHintMiddleware({ maxRetries: 3 }));
    ```

=== "Rust"

    ```rust
    use apcore::executor::Executor;
    use apcore::middleware::{RetryConfig, RetryMiddleware};

    // Chain middleware (Rust uses sequential calls — `use_middleware`
    // returns `Result<(), ModuleError>`, not `&Self`).
    executor.use_middleware(Box::new(LoggingMiddleware::default()))?;
    executor.use_middleware(Box::new(MetricsMiddleware::default()))?;
    executor.use_middleware(Box::new(CacheMiddleware::new(60.0)))?;
    executor.use_middleware(Box::new(RetryMiddleware::new(RetryConfig {
        max_retries: 3,
        ..RetryConfig::default()
    })))?;
    ```

### 7.3 Conditional Middleware

=== "Python"

    ```python
    import fnmatch
    from apcore import Middleware, Context


    class ConditionalMiddleware(Middleware):
        """Conditional middleware: only applies to specific modules"""

        def __init__(self, inner: Middleware, pattern: str):
            self.inner = inner
            self.pattern = pattern

        def _matches(self, module_id: str) -> bool:
            return fnmatch.fnmatch(module_id, self.pattern)

        def before(self, module_id: str, inputs: dict, context: Context):
            if self._matches(module_id):
                return self.inner.before(module_id, inputs, context)
            return None

        def after(self, module_id: str, inputs: dict, output: dict, context: Context):
            if self._matches(module_id):
                return self.inner.after(module_id, inputs, output, context)
            return None

        def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context):
            if self._matches(module_id):
                return self.inner.on_error(module_id, inputs, error, context)
            return None


    # Usage: enable cache only for executor.* modules
    executor.use(
        ConditionalMiddleware(CacheMiddleware(), "executor.*")
    )
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    function fnmatch(name: string, pattern: string): boolean {
      const escaped = pattern
        .replace(/[.+^${}()|[\]\\]/g, '\\$&')
        .replace(/\*/g, '.*')
        .replace(/\?/g, '.');
      return new RegExp(`^${escaped}$`).test(name);
    }

    class ConditionalMiddleware extends Middleware {
      constructor(
        private readonly inner: Middleware,
        private readonly pattern: string,
      ) {
        super();
      }

      private matches(moduleId: string): boolean {
        return fnmatch(moduleId, this.pattern);
      }

      override before(
        moduleId: string,
        inputs: Record<string, unknown>,
        context: Context,
      ) {
        if (this.matches(moduleId)) {
          return this.inner.before(moduleId, inputs, context);
        }
        return null;
      }

      override after(
        moduleId: string,
        inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ) {
        if (this.matches(moduleId)) {
          return this.inner.after(moduleId, inputs, output, context);
        }
        return null;
      }

      override onError(
        moduleId: string,
        inputs: Record<string, unknown>,
        error: Error,
        context: Context,
      ) {
        if (this.matches(moduleId)) {
          return this.inner.onError(moduleId, inputs, error, context);
        }
        return null;
      }
    }

    // Usage: enable cache only for executor.* modules
    executor.use(new ConditionalMiddleware(new CacheMiddleware(60), 'executor.*'));
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::Value;

    /// Glob match supporting `*` and `?` (sufficient for module-id patterns).
    fn fnmatch(name: &str, pattern: &str) -> bool {
        let mut re = String::with_capacity(pattern.len() + 4);
        re.push('^');
        for c in pattern.chars() {
            match c {
                '*' => re.push_str(".*"),
                '?' => re.push('.'),
                '.' | '+' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '\\' | '^' | '$' => {
                    re.push('\\');
                    re.push(c);
                }
                _ => re.push(c),
            }
        }
        re.push('$');
        regex::Regex::new(&re).map(|r| r.is_match(name)).unwrap_or(false)
    }

    #[derive(Debug)]
    struct ConditionalMiddleware {
        inner: Box<dyn Middleware>,
        pattern: String,
    }

    impl ConditionalMiddleware {
        pub fn new(inner: Box<dyn Middleware>, pattern: impl Into<String>) -> Self {
            Self {
                inner,
                pattern: pattern.into(),
            }
        }

        fn matches(&self, module_id: &str) -> bool {
            fnmatch(module_id, &self.pattern)
        }
    }

    #[async_trait]
    impl Middleware for ConditionalMiddleware {
        fn name(&self) -> &str {
            "conditional"
        }

        async fn before(
            &self,
            module_id: &str,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if self.matches(module_id) {
                self.inner.before(module_id, inputs, ctx).await
            } else {
                Ok(None)
            }
        }

        async fn after(
            &self,
            module_id: &str,
            inputs: Value,
            output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if self.matches(module_id) {
                self.inner.after(module_id, inputs, output, ctx).await
            } else {
                Ok(None)
            }
        }

        async fn on_error(
            &self,
            module_id: &str,
            inputs: Value,
            error: &ModuleError,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if self.matches(module_id) {
                self.inner.on_error(module_id, inputs, error, ctx).await
            } else {
                Ok(None)
            }
        }
    }
    ```

### 7.4 Middleware Group

=== "Python"

    ```python
    from apcore import Middleware, Context


    class MiddlewareGroup(Middleware):
        """Middleware group: use multiple middleware as one"""

        def __init__(self, middlewares: list[Middleware]):
            self.middlewares = middlewares

        def before(self, module_id: str, inputs: dict, context: Context):
            result = inputs
            for mw in self.middlewares:
                new_result = mw.before(module_id, result, context)
                if new_result is not None:
                    result = new_result
            return result if result is not inputs else None

        def after(self, module_id: str, inputs: dict, output: dict, context: Context):
            result = output
            for mw in reversed(self.middlewares):
                new_result = mw.after(module_id, inputs, result, context)
                if new_result is not None:
                    result = new_result
            return result if result is not output else None


    # Usage
    production_middlewares = MiddlewareGroup([
        LoggingMiddleware(),
        MetricsMiddleware(),
        RateLimitMiddleware(),
    ])

    executor.use(production_middlewares)
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class MiddlewareGroup extends Middleware {
      constructor(private readonly middlewares: Middleware[]) {
        super();
      }

      override async before(
        moduleId: string,
        inputs: Record<string, unknown>,
        context: Context,
      ): Promise<Record<string, unknown> | null> {
        let result: Record<string, unknown> = inputs;
        let modified = false;
        for (const mw of this.middlewares) {
          const next = await mw.before(moduleId, result, context);
          if (next != null) {
            result = next;
            modified = true;
          }
        }
        return modified ? result : null;
      }

      override async after(
        moduleId: string,
        inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ): Promise<Record<string, unknown> | null> {
        let result: Record<string, unknown> = output;
        let modified = false;
        for (const mw of [...this.middlewares].reverse()) {
          const next = await mw.after(moduleId, inputs, result, context);
          if (next != null) {
            result = next;
            modified = true;
          }
        }
        return modified ? result : null;
      }
    }

    // Usage
    const productionMiddlewares = new MiddlewareGroup([
      new LoggingMiddleware(),
      new MetricsMiddleware(),
      new RateLimitMiddleware(),
    ]);

    executor.use(productionMiddlewares);
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::Value;

    #[derive(Debug)]
    struct MiddlewareGroup {
        middlewares: Vec<Box<dyn Middleware>>,
    }

    impl MiddlewareGroup {
        pub fn new(middlewares: Vec<Box<dyn Middleware>>) -> Self {
            Self { middlewares }
        }
    }

    #[async_trait]
    impl Middleware for MiddlewareGroup {
        fn name(&self) -> &str {
            "middleware_group"
        }

        async fn before(
            &self,
            module_id: &str,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let mut result = inputs;
            let mut modified = false;
            for mw in &self.middlewares {
                if let Some(next) = mw.before(module_id, result.clone(), ctx).await? {
                    result = next;
                    modified = true;
                }
            }
            Ok(if modified { Some(result) } else { None })
        }

        async fn after(
            &self,
            module_id: &str,
            inputs: Value,
            output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let mut result = output;
            let mut modified = false;
            for mw in self.middlewares.iter().rev() {
                if let Some(next) = mw
                    .after(module_id, inputs.clone(), result.clone(), ctx)
                    .await?
                {
                    result = next;
                    modified = true;
                }
            }
            Ok(if modified { Some(result) } else { None })
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

    // Usage
    let production_middlewares = MiddlewareGroup::new(vec![
        Box::new(LoggingMiddleware::default()),
        Box::new(MetricsMiddleware::default()),
        Box::new(RateLimitMiddleware::new(100, 60.0)),
    ]);

    executor.use_middleware(Box::new(production_middlewares))?;
    ```

---

## 8. Async Middleware

The standard `Middleware` base class supports async `before()`, `after()`, and `on_error()` methods. There is no separate `AsyncMiddleware` class — simply define async methods on a regular `Middleware` subclass. In Rust, all hooks are already async by trait definition.

=== "Python"

    ```python
    from apcore.middleware import Middleware
    from apcore.context import Context


    class AsyncLoggingMiddleware(Middleware):
        """Middleware with async hooks"""

        async def before(self, module_id: str, inputs: dict, context: Context) -> None:
            await self._async_log(f"Calling {module_id}")

        async def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            await self._async_log(f"Completed {module_id}")

        async def _async_log(self, message: str) -> None:
            import aiohttp
            async with aiohttp.ClientSession() as session:
                await session.post(
                    "https://logs.example.com/api/log",
                    json={"message": message},
                )
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class AsyncLoggingMiddleware extends Middleware {
      override async before(
        moduleId: string,
        _inputs: Record<string, unknown>,
        _context: Context,
      ): Promise<null> {
        await this.asyncLog(`Calling ${moduleId}`);
        return null;
      }

      override async after(
        moduleId: string,
        _inputs: Record<string, unknown>,
        _output: Record<string, unknown>,
        _context: Context,
      ): Promise<null> {
        await this.asyncLog(`Completed ${moduleId}`);
        return null;
      }

      private async asyncLog(message: string): Promise<void> {
        await fetch('https://logs.example.com/api/log', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ message }),
        });
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    #[derive(Debug)]
    struct AsyncLoggingMiddleware {
        client: reqwest::Client,
    }

    impl AsyncLoggingMiddleware {
        pub fn new() -> Self {
            Self {
                client: reqwest::Client::new(),
            }
        }

        async fn async_log(&self, message: &str) -> Result<(), reqwest::Error> {
            self.client
                .post("https://logs.example.com/api/log")
                .json(&json!({ "message": message }))
                .send()
                .await?;
            Ok(())
        }
    }

    #[async_trait]
    impl Middleware for AsyncLoggingMiddleware {
        fn name(&self) -> &str {
            "async_logging"
        }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let _ = self.async_log(&format!("Calling {}", module_id)).await;
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let _ = self.async_log(&format!("Completed {}", module_id)).await;
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
    ```

---

## 9. Middleware Security Specifications

### 9.1 Log Redaction

When middleware records logs, it is **forbidden** to directly log raw `inputs` parameters. **Must** use `context.redacted_inputs` (Python/Rust) or `context.redactedInputs` (TypeScript), which is automatically generated by Executor based on `x-sensitive: true` markers in the Schema.

=== "Python"

    ```python
    # Dangerous: may leak passwords, API keys, and other sensitive data
    self.logger.info(f"inputs: {inputs}")

    # Safe: sensitive fields are replaced with ***REDACTED***
    self.logger.info(f"inputs: {context.redacted_inputs}")
    ```

=== "TypeScript"

    ```typescript
    // Dangerous: may leak passwords, API keys, and other sensitive data
    this.logger.info('inputs:', inputs);

    // Safe: sensitive fields are replaced with ***REDACTED***
    this.logger.info('inputs:', context.redactedInputs);
    ```

=== "Rust"

    ```rust
    // Dangerous: may leak passwords, API keys, and other sensitive data
    tracing::info!(inputs = ?inputs, "inputs logged");

    // Safe: sensitive fields are replaced with ***REDACTED***
    tracing::info!(inputs = ?ctx.redacted_inputs(), "inputs logged");
    ```

### 9.2 `_secret_` Prefix Convention

When passing middleware state via `context.data`, if it contains sensitive information, use the `_secret_` prefix:

=== "Python"

    ```python
    # Set sensitive data
    context.data["_secret_auth_token"] = "Bearer sk-..."

    # Framework guarantee: keys starting with _secret_ are automatically
    # redacted in log serialization.
    ```

=== "TypeScript"

    ```typescript
    // Set sensitive data
    context.data['_secret_auth_token'] = 'Bearer sk-...';

    // Framework guarantee: keys starting with _secret_ are automatically
    // redacted in log serialization.
    ```

=== "Rust"

    ```rust
    use serde_json::json;

    // Set sensitive data
    ctx.data().insert("_secret_auth_token", json!("Bearer sk-..."));

    // Framework guarantee: keys starting with _secret_ are automatically
    // redacted in log serialization.
    ```

### 9.3 Five-Layer Log Security Model

| Layer | Mechanism | Description |
|------|------|------|
| L1 | Schema `x-sensitive: true` | Mark sensitive fields in Schema |
| L2 | Executor auto-generates `redacted_inputs` | Based on A13 `redact_sensitive()` algorithm |
| L3 | `_secret_` prefix convention | Sensitive keys in `context.data` |
| L4 | Error message sanitization | `propagate_error()` does not include raw inputs in error messages |
| L5 | `context.logger` | Auto-inject trace_id/module_id, unified log format |

---

## 10. Best Practices

### 10.1 Middleware Order

=== "Python"

    ```python
    from apcore import Executor, RetryMiddleware, RetryConfig

    # Recommended order
    executor = Executor(
        registry=registry,
        middlewares=[
            RateLimitMiddleware(),                       # 1. Rate limiting (check first)
            LoggingMiddleware(),                         # 2. Logging (record all requests)
            MetricsMiddleware(),                         # 3. Performance monitoring
            CacheMiddleware(),                           # 4. Cache (avoid duplicate execution)
            RetryMiddleware(RetryConfig()),              # 5. Retry (retry on failure)
            FallbackMiddleware(fallback_values={}),      # 6. Fallback (last resort)
        ],
    )
    ```

=== "TypeScript"

    ```typescript
    import { Executor, RetryHintMiddleware } from 'apcore-js';

    // Recommended order
    const executor = new Executor({
      registry,
      middlewares: [
        new RateLimitMiddleware(),       // 1. Rate limiting (check first)
        new LoggingMiddleware(),         // 2. Logging (record all requests)
        new MetricsMiddleware(),         // 3. Performance monitoring
        new CacheMiddleware(60),         // 4. Cache (avoid duplicate execution)
        new RetryHintMiddleware(),       // 5. Retry hints (retry on failure)
        new FallbackMiddleware({}),      // 6. Fallback (last resort)
      ],
    });
    ```

=== "Rust"

    ```rust
    use apcore::executor::Executor;
    use apcore::config::Config;
    use apcore::middleware::{RetryConfig, RetryMiddleware};
    use std::collections::HashMap;

    // Recommended order
    let executor = Executor::new(registry, Config::default());
    executor.use_middleware(Box::new(RateLimitMiddleware::new(100, 60.0)))?;     // 1. Rate limiting
    executor.use_middleware(Box::new(LoggingMiddleware::default()))?;            // 2. Logging
    executor.use_middleware(Box::new(MetricsMiddleware::default()))?;            // 3. Metrics
    executor.use_middleware(Box::new(CacheMiddleware::new(60.0)))?;              // 4. Cache
    executor.use_middleware(Box::new(RetryMiddleware::new(RetryConfig::default())))?; // 5. Retry
    executor.use_middleware(Box::new(FallbackMiddleware::new(HashMap::new())))?; // 6. Fallback
    ```

### 10.2 Performance Considerations

=== "Python"

    ```python
    import time
    from apcore import Middleware, Context


    class EfficientMiddleware(Middleware):
        """Efficient middleware"""

        def _should_process(self, module_id: str) -> bool:
            return module_id.startswith("executor.")

        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            # Quick check
            if not self._should_process(module_id):
                return

            # Avoid time-consuming operations in before; use context.data
            # to pass timing state to after().
            context.data["_apcore.mw.start_time"] = time.perf_counter()

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            start = context.data.get("_apcore.mw.start_time")
            if start is not None:
                duration = time.perf_counter() - start
                # Process duration...
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class EfficientMiddleware extends Middleware {
      private shouldProcess(moduleId: string): boolean {
        return moduleId.startsWith('executor.');
      }

      override before(
        moduleId: string,
        _inputs: Record<string, unknown>,
        context: Context,
      ): null {
        // Quick check
        if (!this.shouldProcess(moduleId)) return null;

        // Avoid time-consuming operations in before; use context.data
        // to pass timing state to after().
        context.data['_apcore.mw.start_time'] = performance.now();
        return null;
      }

      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _output: Record<string, unknown>,
        context: Context,
      ): null {
        const start = context.data['_apcore.mw.start_time'] as number | undefined;
        if (start !== undefined) {
          const duration = performance.now() - start;
          // Process duration...
          void duration;
        }
        return null;
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::time::Instant;

    #[derive(Debug)]
    struct EfficientMiddleware;

    impl EfficientMiddleware {
        fn should_process(&self, module_id: &str) -> bool {
            module_id.starts_with("executor.")
        }
    }

    #[async_trait]
    impl Middleware for EfficientMiddleware {
        fn name(&self) -> &str {
            "efficient"
        }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if !self.should_process(module_id) {
                return Ok(None);
            }
            // Store an opaque token in context.data so after() can read it.
            // (Using elapsed-millis-from-now as a portable u128 stored as string.)
            ctx.data().insert(
                "_apcore.mw.start_time_ns",
                json!(Instant::now().elapsed().as_nanos().to_string()),
            );
            Ok(None)
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if let Some(Value::String(_token)) = ctx.data().get("_apcore.mw.start_time_ns") {
                // Process duration here using your timing source of choice.
            }
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
    ```

### 10.3 Error Handling

=== "Python"

    ```python
    import logging
    from apcore import Middleware, Context


    class SafeMiddleware(Middleware):
        """Safe middleware: own errors don't affect main flow"""

        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            try:
                self._do_something()
            except Exception as e:
                # Log error but don't raise
                logging.warning(f"Middleware error: {e}")

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            try:
                self._do_something_else()
            except Exception as e:
                logging.warning(f"Middleware error: {e}")

        def _do_something(self) -> None:
            ...

        def _do_something_else(self) -> None:
            ...
    ```

=== "TypeScript"

    ```typescript
    import { Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    class SafeMiddleware extends Middleware {
      override before(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _context: Context,
      ): null {
        try {
          this.doSomething();
        } catch (e) {
          console.warn(`Middleware error: ${e}`);
        }
        return null;
      }

      override after(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _output: Record<string, unknown>,
        _context: Context,
      ): null {
        try {
          this.doSomethingElse();
        } catch (e) {
          console.warn(`Middleware error: ${e}`);
        }
        return null;
      }

      private doSomething(): void {}
      private doSomethingElse(): void {}
    }
    ```

=== "Rust"

    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::middleware::Middleware;
    use async_trait::async_trait;
    use serde_json::Value;

    #[derive(Debug)]
    struct SafeMiddleware;

    impl SafeMiddleware {
        fn do_something(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
        fn do_something_else(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    #[async_trait]
    impl Middleware for SafeMiddleware {
        fn name(&self) -> &str {
            "safe"
        }

        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if let Err(e) = self.do_something() {
                tracing::warn!("Middleware error: {}", e);
            }
            Ok(None)
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            if let Err(e) = self.do_something_else() {
                tracing::warn!("Middleware error: {}", e);
            }
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
    ```

---

## 11. Middleware vs Custom Pipeline Steps

The executor has two extension mechanisms. Choosing the wrong one leads to awkward workarounds.

### When to Use Middleware

- **Logging / metrics / tracing** — You need to see both inputs and outputs as a pair
- **Retry logic** — `on_error()` lets you return a recovery value
- **Input enrichment** — Add headers, inject defaults, normalize formats
- **Output transformation** — Redact fields, add metadata, reshape responses

Middleware runs at fixed positions (before validation, after execution) and participates in the onion model automatically.

### When to Use a Custom Step

- **Rate limiting** — Needs to run early (e.g., right after ACL), not at the fixed middleware position
- **Cost budgeting** — Must gate execution before it happens, at a specific point
- **Custom validation** — Schema validation isn't enough; you need semantic checks
- **Feature flags** — Conditionally skip execution based on external config

Custom steps can be inserted at any position and appear individually in `PipelineTrace`.

### Quick Decision

```
Does your logic need to wrap execution (see inputs AND outputs)?
  → Yes: Use Middleware (before/after pair)
  → No: Does it need to run at a specific pipeline position?
    → Yes: Use Custom Step (insert_before/insert_after)
    → No: Use Middleware (simpler registration)
```

### Example: Rate Limiter as a Custom Step

=== "Python"
    ```python
    from apcore.pipeline import BaseStep, StepResult, PipelineContext

    class RateLimiterStep(BaseStep):
        def __init__(self, max_rps: int = 100):
            super().__init__(
                name="rate_limiter",
                description="Per-module rate limiting",
                removable=True,
                replaceable=True,
                pure=True,
            )
            self.max_rps = max_rps

        async def execute(self, ctx: PipelineContext) -> StepResult:
            if self._over_limit(ctx.module_id):
                return StepResult(action="abort", explanation="Rate limit exceeded")
            return StepResult(action="continue")

    # Insert after ACL check — before middleware and validation
    strategy.insert_after("acl_check", RateLimiterStep(max_rps=50))
    ```

=== "TypeScript"
    ```typescript
    import { BaseStep, StepResult, PipelineContext } from 'apcore-js';

    class RateLimiterStep extends BaseStep {
      constructor(private maxRps: number = 100) {
        super({
          name: 'rate_limiter',
          description: 'Per-module rate limiting',
          removable: true,
          replaceable: true,
          pure: true,
        });
      }

      async execute(ctx: PipelineContext): Promise<StepResult> {
        if (this.overLimit(ctx.moduleId)) {
          return { action: 'abort', explanation: 'Rate limit exceeded' };
        }
        return { action: 'continue' };
      }
    }

    strategy.insertAfter('acl_check', new RateLimiterStep(50));
    ```

=== "Rust"
    ```rust
    use apcore::pipeline::{BaseStep, StepResult, PipelineContext};

    struct RateLimiterStep { max_rps: u32 }

    #[async_trait]
    impl Step for RateLimiterStep {
        fn name(&self) -> &str { "rate_limiter" }
        fn description(&self) -> &str { "Per-module rate limiting" }
        fn removable(&self) -> bool { true }
        fn replaceable(&self) -> bool { true }
        fn pure(&self) -> bool { true }

        async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            if self.over_limit(&ctx.module_id) {
                return Ok(StepResult::abort("Rate limit exceeded"));
            }
            Ok(StepResult::continue_step())
        }
    }

    strategy.insert_after("acl_check", Box::new(RateLimiterStep { max_rps: 50 }))?;
    ```

---

## 12. Complete Example

=== "Python"

    ```python
    import logging
    import time
    from apcore import Registry, Executor, Middleware, Context


    # 1. Custom logging middleware
    class AppLoggingMiddleware(Middleware):
        def __init__(self):
            self.logger = logging.getLogger("app")
            self._times: dict[str, float] = {}

        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            self._times[context.trace_id] = time.perf_counter()
            self.logger.info(f"[{context.trace_id}] -> {module_id}")

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            duration = (time.perf_counter() - self._times.pop(context.trace_id, 0)) * 1000
            success = output.get("success", True)
            status = "OK" if success else "FAIL"
            self.logger.info(f"[{context.trace_id}] {status} {module_id} ({duration:.1f}ms)")

        def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
            self.logger.error(f"[{context.trace_id}] FAIL {module_id}: {error}")


    # 2. Simple metrics middleware
    class SimpleMetricsMiddleware(Middleware):
        def __init__(self):
            self.calls = 0
            self.errors = 0

        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            self.calls += 1

        def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
            self.errors += 1


    # 3. Setup
    logging.basicConfig(level=logging.INFO, format="%(message)s")

    registry = Registry(extensions_dir="./extensions")
    registry.discover()

    metrics = SimpleMetricsMiddleware()
    executor = Executor(
        registry=registry,
        middlewares=[AppLoggingMiddleware(), metrics],
    )

    # 4. Usage
    result = executor.call(
        module_id="executor.email.send_email",
        inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"},
    )

    print(f"\nTotal calls: {metrics.calls}")
    print(f"Total errors: {metrics.errors}")
    ```

=== "TypeScript"

    ```typescript
    import { Registry, Executor, Middleware } from 'apcore-js';
    import type { Context } from 'apcore-js';

    // 1. Custom logging middleware
    class AppLoggingMiddleware extends Middleware {
      private readonly times = new Map<string, number>();

      override before(
        moduleId: string,
        _inputs: Record<string, unknown>,
        context: Context,
      ): null {
        this.times.set(context.traceId, performance.now());
        console.info(`[${context.traceId}] -> ${moduleId}`);
        return null;
      }

      override after(
        moduleId: string,
        _inputs: Record<string, unknown>,
        output: Record<string, unknown>,
        context: Context,
      ): null {
        const start = this.times.get(context.traceId) ?? performance.now();
        this.times.delete(context.traceId);
        const duration = performance.now() - start;
        const success = (output['success'] as boolean | undefined) ?? true;
        const status = success ? 'OK' : 'FAIL';
        console.info(`[${context.traceId}] ${status} ${moduleId} (${duration.toFixed(1)}ms)`);
        return null;
      }

      override onError(
        moduleId: string,
        _inputs: Record<string, unknown>,
        error: Error,
        context: Context,
      ): null {
        console.error(`[${context.traceId}] FAIL ${moduleId}: ${error}`);
        return null;
      }
    }

    // 2. Simple metrics middleware
    class SimpleMetricsMiddleware extends Middleware {
      calls = 0;
      errors = 0;

      override before(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _context: Context,
      ): null {
        this.calls += 1;
        return null;
      }

      override onError(
        _moduleId: string,
        _inputs: Record<string, unknown>,
        _error: Error,
        _context: Context,
      ): null {
        this.errors += 1;
        return null;
      }
    }

    // 3. Setup
    const registry = new Registry({ extensionsDir: './extensions' });
    await registry.discover();

    const metrics = new SimpleMetricsMiddleware();
    const executor = new Executor({
      registry,
      middlewares: [new AppLoggingMiddleware(), metrics],
    });

    // 4. Usage
    const result = await executor.call(
      'executor.email.send_email',
      { to: 'user@example.com', subject: 'Hi', body: 'Hello' },
    );

    console.log(`\nTotal calls: ${metrics.calls}`);
    console.log(`Total errors: ${metrics.errors}`);
    ```

=== "Rust"

    ```rust
    use apcore::config::Config;
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::executor::Executor;
    use apcore::middleware::Middleware;
    use apcore::registry::Registry;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Instant;

    // 1. Custom logging middleware
    #[derive(Debug, Default)]
    struct AppLoggingMiddleware {
        times: Mutex<HashMap<String, Instant>>,
    }

    #[async_trait]
    impl Middleware for AppLoggingMiddleware {
        fn name(&self) -> &str {
            "app_logging"
        }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            self.times
                .lock()
                .insert(ctx.trace_id().to_string(), Instant::now());
            tracing::info!("[{}] -> {}", ctx.trace_id(), module_id);
            Ok(None)
        }

        async fn after(
            &self,
            module_id: &str,
            _inputs: Value,
            output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let start = self
                .times
                .lock()
                .remove(ctx.trace_id())
                .unwrap_or_else(Instant::now);
            let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
            let success = output
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let status = if success { "OK" } else { "FAIL" };
            tracing::info!(
                "[{}] {} {} ({:.1}ms)",
                ctx.trace_id(),
                status,
                module_id,
                duration_ms,
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
            tracing::error!("[{}] FAIL {}: {}", ctx.trace_id(), module_id, error);
            Ok(None)
        }
    }

    // 2. Simple metrics middleware
    #[derive(Debug, Default)]
    struct SimpleMetricsMiddleware {
        calls: Mutex<u64>,
        errors: Mutex<u64>,
    }

    #[async_trait]
    impl Middleware for SimpleMetricsMiddleware {
        fn name(&self) -> &str {
            "simple_metrics"
        }

        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            *self.calls.lock() += 1;
            Ok(None)
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        async fn on_error(
            &self,
            _module_id: &str,
            _inputs: Value,
            _error: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            *self.errors.lock() += 1;
            Ok(None)
        }
    }

    // 3. Setup
    let registry = Arc::new(Registry::from_extensions_dir("./extensions")?);
    let executor = Executor::new(registry, Config::default());
    let metrics = Arc::new(SimpleMetricsMiddleware::default());

    executor.use_middleware(Box::new(AppLoggingMiddleware::default()))?;
    executor.use_middleware(Box::new(Arc::clone(&metrics)) as Box<dyn Middleware>)?;

    // 4. Usage
    let _result = executor
        .call(
            "executor.email.send_email",
            json!({ "to": "user@example.com", "subject": "Hi", "body": "Hello" }),
            None,
        )
        .await?;

    println!("\nTotal calls: {}", *metrics.calls.lock());
    println!("Total errors: {}", *metrics.errors.lock());
    ```

Output:

```
[abc-123] → executor.email.send_email
[abc-123] ✓ executor.email.send_email (45.2ms)

Total calls: 1
Total errors: 0
```

---

## Next Steps

- [Core Executor](../features/core-executor.md) - Executor feature spec
- [ACL Configuration Guide](./acl-configuration.md) - Access control configuration
- [Architecture Design](../architecture.md) - Overall system architecture
