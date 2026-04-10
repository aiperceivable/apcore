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

```python
from typing import Any
from apcore import Context


class Middleware:
    """Middleware base class"""

    def before(
        self,
        module_id: str,
        inputs: dict[str, Any],
        context: Context
    ) -> dict[str, Any] | None:
        """
        Called before module execution

        Args:
            module_id: Module ID
            inputs: Input parameters
            context: Invocation context

        Returns:
            Modified inputs, or None to keep unchanged
        """
        return None

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context
    ) -> dict[str, Any] | None:
        """
        Called after module execution

        Args:
            module_id: Module ID
            inputs: Input parameters
            output: Output result
            context: Invocation context

        Returns:
            Modified output, or None to keep unchanged
        """
        return None

    def on_error(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Context
    ) -> dict[str, Any] | None:
        """
        Called when module execution fails

        Args:
            module_id: Module ID
            inputs: Input parameters
            error: Exception object
            context: Invocation context

        Returns:
            Alternative output result (for error recovery), or None to continue raising exception
        """
        return None
```

---

## 3. Quick Start

### 3.1 Create Simple Middleware

```python
from apcore import Middleware, Context


class LoggingMiddleware(Middleware):
    """Logging middleware"""

    def before(self, module_id: str, inputs: dict, context: Context) -> None:
        print(f"[{context.trace_id}] Calling {module_id}")
        # ⚠️ Use context.redacted_inputs to avoid leaking sensitive data
        print(f"  Inputs: {context.redacted_inputs}")

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        print(f"[{context.trace_id}] {module_id} completed")
        print(f"  Output: {output}")

    def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
        print(f"[{context.trace_id}] {module_id} failed: {error}")
```

### 3.2 Using Middleware

```python
from apcore import Registry, Executor

registry = Registry(extensions_dir="./extensions")
registry.discover()

executor = Executor(
    registry=registry,
    middlewares=[LoggingMiddleware()]
)

# Middleware automatically executes when calling modules
result = executor.call(
    module_id="executor.email.send_email",
    inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"}
)
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

```python
executor = Executor(
    registry=registry,
    middlewares=[
        MiddlewareA(),  # First
        MiddlewareB(),  # Second
        MiddlewareC()   # Third
    ]
)
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

```python
import logging
from datetime import datetime
from apcore import Middleware, Context


class LoggingMiddleware(Middleware):
    """Detailed logging middleware"""

    def __init__(self, logger: logging.Logger | None = None):
        self.logger = logger or logging.getLogger("apcore")
        self._start_times: dict[str, datetime] = {}

    def before(self, module_id: str, inputs: dict, context: Context) -> None:
        self._start_times[context.trace_id] = datetime.now()
        # ⚠️ Security: Use context.redacted_inputs instead of raw inputs to avoid leaking sensitive data
        self.logger.info(
            f"[{context.trace_id}] START {module_id}",
            extra={
                "trace_id": context.trace_id,
                "module_id": module_id,
                "caller_id": context.caller_id,
                "inputs": context.redacted_inputs
            }
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
                "output": output
            }
        )

    def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
        # ⚠️ Security: Use context.redacted_inputs instead of raw inputs
        self.logger.error(
            f"[{context.trace_id}] ERROR {module_id}: {error}",
            extra={
                "trace_id": context.trace_id,
                "module_id": module_id,
                "error": str(error),
                "inputs": context.redacted_inputs
            },
            exc_info=True
        )
```

### 5.2 Performance Monitoring Middleware

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

        # Increment call count
        self.call_counts[module_id] = self.call_counts.get(module_id, 0) + 1

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        start = self._start_times.pop(context.trace_id, time.perf_counter())
        duration = time.perf_counter() - start

        # Record execution time
        if module_id not in self.durations:
            self.durations[module_id] = []
        self.durations[module_id].append(duration)

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

### 5.3 Cache Middleware

```python
import hashlib
import json
from apcore import Middleware, Context


class CacheMiddleware(Middleware):
    """Simple cache middleware"""

    def __init__(self, ttl_seconds: int = 300):
        self.cache: dict[str, tuple[dict, float]] = {}
        self.ttl = ttl_seconds

    def _cache_key(self, module_id: str, inputs: dict) -> str:
        """Generate cache key"""
        content = json.dumps({"module_id": module_id, "inputs": inputs}, sort_keys=True)
        return hashlib.md5(content.encode()).hexdigest()

    def before(self, module_id: str, inputs: dict, context: Context) -> dict | None:
        key = self._cache_key(module_id, inputs)

        if key in self.cache:
            cached_output, cached_time = self.cache[key]
            import time
            if time.time() - cached_time < self.ttl:
                # Cache hit, skip execution
                context.data["cache_hit"] = True
                return cached_output

        return None

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        if not context.data.get("cache_hit"):
            # Cache miss, store result
            key = self._cache_key(module_id, inputs)
            import time
            self.cache[key] = (output, time.time())
```

### 5.4 RetryMiddleware (Built-in)

apcore provides a built-in `RetryMiddleware` with configurable backoff strategies. It only retries errors marked `retryable=True`.

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

        if module_id not in self._call_times:
            self._call_times[module_id] = deque()

        calls = self._call_times[module_id]

        # Remove calls outside the window
        while calls and calls[0] < now - self.window:
            calls.popleft()

        # Check if limit is exceeded
        if len(calls) >= self.max_calls:
            raise ModuleError(
                f"Rate limit exceeded for {module_id}",
                module_id=module_id,
                trace_id=context.trace_id
            )

        # Record current call
        calls.append(now)
```

---

## 6. Modifying Inputs/Outputs

### 6.1 Modifying Inputs

```python
class InputTransformMiddleware(Middleware):
    """Input transformation middleware"""

    def before(self, module_id: str, inputs: dict, context: Context) -> dict:
        # Return modified inputs
        modified = inputs.copy()

        # Add default values
        if "timeout" not in modified:
            modified["timeout"] = 30

        # Add trace information
        modified["_trace_id"] = context.trace_id

        return modified
```

### 6.2 Modifying Outputs

```python
class OutputEnrichMiddleware(Middleware):
    """Output enrichment middleware"""

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> dict:
        # Return modified output
        enriched = output.copy()

        # Add metadata
        enriched["_metadata"] = {
            "module_id": module_id,
            "trace_id": context.trace_id,
            "timestamp": datetime.now().isoformat()
        }

        return enriched
```

### 6.3 Error Recovery

```python
class FallbackMiddleware(Middleware):
    """Fallback middleware"""

    def __init__(self, fallback_values: dict[str, dict]):
        """
        Args:
            fallback_values: {module_id: default_output}
        """
        self.fallback_values = fallback_values

    def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> dict | None:
        if module_id in self.fallback_values:
            # Return fallback value, "swallow" exception
            return self.fallback_values[module_id]

        return None  # Continue raising exception
```

---

## 7. Middleware Composition

### 7.1 Function-first Middleware (Function-first API)

In addition to class-based middleware, you can directly register functions using `use_before()` / `use_after()`:

```python
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

**Use cases:**

| Approach | Use Case |
|------|---------|
| Class-based (`Middleware`) | Need `on_error`, need shared state, complex logic |
| Function-first (`use_before`/`use_after`) | Simple hooks, rapid prototyping, single concern |

### 7.2 Chaining

```python
executor = Executor(registry=registry)

# Chain middleware
executor \
    .use(LoggingMiddleware()) \
    .use(MetricsMiddleware()) \
    .use(CacheMiddleware(ttl_seconds=60)) \
    .use(RetryMiddleware(max_retries=3))
```

### 7.3 Conditional Middleware

```python
class ConditionalMiddleware(Middleware):
    """Conditional middleware: only applies to specific modules"""

    def __init__(self, inner: Middleware, pattern: str):
        self.inner = inner
        self.pattern = pattern

    def _matches(self, module_id: str) -> bool:
        import fnmatch
        return fnmatch.fnmatch(module_id, self.pattern)

    def before(self, module_id: str, inputs: dict, context: Context):
        if self._matches(module_id):
            return self.inner.before(module_id, inputs, context)

    def after(self, module_id: str, inputs: dict, output: dict, context: Context):
        if self._matches(module_id):
            return self.inner.after(module_id, inputs, output, context)

    def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context):
        if self._matches(module_id):
            return self.inner.on_error(module_id, inputs, error, context)


# Usage: Enable cache only for executor.* modules
executor.use(
    ConditionalMiddleware(CacheMiddleware(), "executor.*")
)
```

### 7.4 Middleware Group

```python
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
        return result if result != inputs else None

    def after(self, module_id: str, inputs: dict, output: dict, context: Context):
        result = output
        for mw in reversed(self.middlewares):
            new_result = mw.after(module_id, inputs, result, context)
            if new_result is not None:
                result = new_result
        return result if result != output else None


# Usage
production_middlewares = MiddlewareGroup([
    LoggingMiddleware(),
    MetricsMiddleware(),
    RateLimitMiddleware()
])

executor.use(production_middlewares)
```

---

## 8. Async Middleware

The standard `Middleware` base class supports async `before()`, `after()`, and `on_error()` methods. There is no separate `AsyncMiddleware` class — simply define async methods on a regular `Middleware` subclass:

```python
from apcore.middleware import Middleware
from apcore.context import Context


class AsyncLoggingMiddleware(Middleware):
    """Middleware with async hooks"""

    async def before(self, module_id: str, inputs: dict, context: Context) -> None:
        # Async log writing
        await self._async_log(f"Calling {module_id}")

    async def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        await self._async_log(f"Completed {module_id}")

    async def _async_log(self, message: str) -> None:
        # Async write to logging service
        import aiohttp
        async with aiohttp.ClientSession() as session:
            await session.post("https://logs.example.com/api/log", json={"message": message})
```

---

## 9. Middleware Security Specifications

### 9.1 Log Redaction

When middleware records logs, it is **forbidden** to directly log raw `inputs` parameters. **Must** use `context.redacted_inputs`, which is automatically generated by Executor based on `x-sensitive: true` markers in the Schema.

```python
# ❌ Dangerous: May leak passwords, API keys, and other sensitive data
self.logger.info(f"inputs: {inputs}")

# ✅ Safe: Sensitive fields are replaced with ***REDACTED***
self.logger.info(f"inputs: {context.redacted_inputs}")
```

### 9.2 `_secret_` Prefix Convention

When passing middleware state via `context.data`, if it contains sensitive information, use the `_secret_` prefix:

```python
# Set sensitive data
context.data["_secret_auth_token"] = "Bearer sk-..."

# Framework guarantee: keys starting with _secret_ are automatically redacted in log serialization
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

```python
# Recommended order
executor = Executor(
    registry=registry,
    middlewares=[
        # 1. Rate limiting (check first)
        RateLimitMiddleware(),

        # 2. Logging (record all requests)
        LoggingMiddleware(),

        # 3. Performance monitoring
        MetricsMiddleware(),

        # 4. Cache (avoid duplicate execution)
        CacheMiddleware(),

        # 5. Retry (retry on execution failure)
        RetryMiddleware(),

        # 6. Fallback (last resort)
        FallbackMiddleware(fallbacks={...})
    ]
)
```

### 10.2 Performance Considerations

```python
class EfficientMiddleware(Middleware):
    """Efficient middleware"""

    def before(self, module_id: str, inputs: dict, context: Context) -> None:
        # ✓ Quick check
        if not self._should_process(module_id):
            return

        # ✗ Avoid time-consuming operations in before
        # self._slow_operation()

        # ✓ Use context.data to pass data to after
        context.data["start_time"] = time.perf_counter()

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        start = context.data.get("start_time")
        if start:
            duration = time.perf_counter() - start
            # Process...
```

### 10.3 Error Handling

```python
class SafeMiddleware(Middleware):
    """Safe middleware: own errors don't affect main flow"""

    def before(self, module_id: str, inputs: dict, context: Context) -> None:
        try:
            # Middleware logic
            self._do_something()
        except Exception as e:
            # Log error but don't raise
            logging.warning(f"Middleware error: {e}")

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        try:
            self._do_something_else()
        except Exception as e:
            logging.warning(f"Middleware error: {e}")
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

```python
from apcore import Registry, Executor, Middleware, Context
import logging
import time


# 1. Custom logging middleware
class AppLoggingMiddleware(Middleware):
    def __init__(self):
        self.logger = logging.getLogger("app")
        self._times: dict[str, float] = {}

    def before(self, module_id: str, inputs: dict, context: Context) -> None:
        self._times[context.trace_id] = time.perf_counter()
        self.logger.info(f"[{context.trace_id}] → {module_id}")

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        duration = (time.perf_counter() - self._times.pop(context.trace_id, 0)) * 1000
        success = output.get("success", True)
        status = "✓" if success else "✗"
        self.logger.info(f"[{context.trace_id}] {status} {module_id} ({duration:.1f}ms)")

    def on_error(self, module_id: str, inputs: dict, error: Exception, context: Context) -> None:
        self.logger.error(f"[{context.trace_id}] ✗ {module_id}: {error}")


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
    middlewares=[
        AppLoggingMiddleware(),
        metrics
    ]
)

# 4. Usage
result = executor.call(
    module_id="executor.email.send_email",
    inputs={"to": "user@example.com", "subject": "Hi", "body": "Hello"}
)

print(f"\nTotal calls: {metrics.calls}")
print(f"Total errors: {metrics.errors}")
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

- [Executor API](../api/executor-api.md) - Detailed executor API
- [ACL Configuration Guide](./acl-configuration.md) - Access control configuration
- [Architecture Design](../architecture.md) - Overall system architecture
