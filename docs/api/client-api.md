# APCore Client API

> **Canonical Definition** - This document is the authoritative definition of the APCore unified client interface

> APCore is the recommended high-level entry point that manages Registry and Executor for you.

## 1. Interface Overview

```python
from typing import Any, AsyncIterator, Callable
from apcore import (
    Registry, Executor, Config, Context, Middleware,
    PreflightResult, EventSubscriber
)


class APCore:
    """Unified client for the apcore framework"""

    def __init__(
        self,
        registry: Registry | None = None,
        executor: Executor | None = None,
        config: Config | None = None,
        metrics_collector: "MetricsCollector | None" = None,
    ) -> None:
        """
        Initialize APCore client

        Args:
            registry: Module registry (auto-created if None)
            executor: Module executor (auto-created if None)
            config: Framework configuration (enables sys_modules if provided)
            metrics_collector: Metrics collector (auto-created if sys_modules enabled)
        """
        ...

    # ============ Module Registration ============

    def module(
        self,
        id: str,
        description: str = "",
        documentation: str = "",
        annotations: dict | None = None,
        tags: list[str] | None = None,
        version: str | None = None,
        metadata: dict | None = None,
        examples: list[Any] | None = None,
    ) -> Callable:
        """Decorator to register a function as a module"""
        ...

    def register(self, module_id: str, module: Any) -> None:
        """Directly register a module instance"""
        ...

    # ============ Module Execution ============

    def call(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        """Synchronously call a module"""
        ...

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        """Asynchronously call a module"""
        ...

    async def stream(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> AsyncIterator[dict[str, Any]]:
        """Stream module output chunk by chunk"""
        ...

    def validate(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
    ) -> PreflightResult:
        """
        Non-destructive preflight check (Steps 1-6 only, no execution)

        Returns:
            PreflightResult with per-check results and .valid / .errors properties
        """
        ...

    # ============ Module Discovery & Inspection ============

    def discover(self) -> int:
        """Discover and register modules from configured extension directories"""
        ...

    def list_modules(
        self,
        tags: list[str] | None = None,
        prefix: str | None = None,
    ) -> list[str]:
        """Return sorted list of registered module IDs with optional filtering"""
        ...

    def describe(self, module_id: str) -> str:
        """
        Get markdown-formatted human-readable module description

        Useful for AI/LLM tool discovery.
        """
        ...

    # ============ Middleware Management ============

    def use(self, middleware: Middleware) -> "APCore":
        """Add class-based middleware (returns self for chaining)"""
        ...

    def use_before(self, callback: Callable) -> "APCore":
        """Add before-middleware callback (wraps in BeforeMiddleware adapter)"""
        ...

    def use_after(self, callback: Callable) -> "APCore":
        """Add after-middleware callback (wraps in AfterMiddleware adapter)"""
        ...

    def remove(self, middleware: Middleware) -> bool:
        """Remove middleware by identity"""
        ...

    # ============ Event System ============

    def on(self, event_type: str, handler: Callable) -> EventSubscriber:
        """
        Subscribe to framework events

        Args:
            event_type: Event type to listen for
            handler: Sync or async callback receiving ApCoreEvent

        Returns:
            EventSubscriber instance (pass to off() to unsubscribe)

        Raises:
            RuntimeError: If sys_modules.events is not enabled
        """
        ...

    def off(self, subscriber: EventSubscriber) -> None:
        """
        Unsubscribe from events

        Args:
            subscriber: The subscriber returned by on()
        """
        ...

    # ============ Module Control ============

    def disable(
        self,
        module_id: str,
        reason: str = "Disabled via APCore client",
    ) -> dict[str, Any]:
        """
        Disable a module without unloading it

        Disabled modules remain registered but calls raise ModuleDisabledError.
        Internally calls system.control.toggle_feature.

        Raises:
            RuntimeError: If sys_modules is not enabled
        """
        ...

    def enable(
        self,
        module_id: str,
        reason: str = "Enabled via APCore client",
    ) -> dict[str, Any]:
        """
        Re-enable a previously disabled module

        Raises:
            RuntimeError: If sys_modules is not enabled
        """
        ...

    # ============ Properties ============

    @property
    def events(self) -> "EventEmitter | None":
        """EventEmitter instance (None if sys_modules.events not enabled)"""
        ...

    @property
    def registry(self) -> Registry:
        """Underlying Registry instance"""
        ...

    @property
    def executor(self) -> Executor:
        """Underlying Executor instance"""
        ...
```

---

## 2. Initialization

### 2.1 Basic (Zero-Config)

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let client = APCore::new();
    ```

### 2.2 With Config (Enables System Modules)

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    client = APCore(config=config)

    # System modules are auto-registered when config has sys_modules.enabled=true
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const config = Config.load("apcore.yaml");
    const client = new APCore({ config });

    // System modules are auto-registered when config has sys_modules.enabled=true
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, Config};

    let config = Config::load("apcore.yaml")?;
    let client = APCore::with_config(config);

    // System modules are auto-registered when config has sys_modules.enabled=true
    ```

### 2.3 With Defaults (No YAML File)

```python
from apcore.config import Config

config = Config.from_defaults()
client = APCore(config=config)
```

### 2.4 With Existing Registry/Executor

=== "Python"

    ```python
    from apcore import APCore, Registry, Executor

    registry = Registry(extensions_dir="./extensions")
    executor = Executor(registry=registry)
    client = APCore(registry=registry, executor=executor)
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Registry, Executor } from 'apcore-js';

    const registry = new Registry({ extensionsDir: "./extensions" });
    const executor = new Executor({ registry });
    const client = new APCore({ registry, executor });
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, Registry, Executor};

    let registry = Registry::new("./extensions");
    let executor = Executor::new(registry.clone());
    let client = APCore::with_components(registry, executor);
    ```

---

## 3. Module Registration

### 3.1 Decorator-Based

```python
@client.module(
    id="math.add",
    description="Add two numbers",
    tags=["math", "utility"],
)
def add(a: int, b: int) -> dict:
    return {"sum": a + b}
```

### 3.2 Direct Registration

```python
client.register("math.add", add_module)
```

### 3.3 Auto-Discovery

```python
count = client.discover()
print(f"Discovered {count} modules")
```

---

## 4. Module Execution

### 4.1 Synchronous Call

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    result = client.call("math.add", {"a": 10, "b": 5})
    # {'sum': 15}
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    const result = await client.call("math.add", { a: 10, b: 5 });
    // { sum: 15 }
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let client = APCore::new();
    let result = client.call("math.add", serde_json::json!({"a": 10, "b": 5})).await?;
    // {"sum": 15}
    ```

### 4.2 Async Call

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    result = await client.call_async("math.add", {"a": 10, "b": 5})
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    const result = await client.callAsync("math.add", { a: 10, b: 5 });
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let client = APCore::new();
    let result = client.call_async("math.add", serde_json::json!({"a": 10, "b": 5})).await?;
    ```

### 4.3 Streaming

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    async for chunk in client.stream("my.streaming_module", {"query": "hello"}):
        print(chunk)
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    for await (const chunk of client.stream("my.streaming_module", { query: "hello" })) {
        console.log(chunk);
    }
    ```

=== "Rust"

    ```rust
    use apcore::APCore;
    use futures::StreamExt;

    let client = APCore::new();
    let mut stream = client.stream("my.streaming_module", serde_json::json!({"query": "hello"})).await?;
    while let Some(chunk) = stream.next().await {
        println!("{:?}", chunk?);
    }
    ```

### 4.4 Preflight Validation

```python
preflight = client.validate("math.add", {"a": 10, "b": 5})

if preflight.valid:
    result = client.call("math.add", {"a": 10, "b": 5})
else:
    for check in preflight.checks:
        if not check.passed:
            print(f"Failed: {check.check} — {check.error}")
```

### 4.5 Version Hint

```python
result = client.call("math.add", {"a": 1, "b": 2}, version_hint=">=1.0.0")
```

---

## 5. Module Discovery & Inspection

### 5.1 List Modules

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()

    # All modules
    all_ids = client.list_modules()

    # Filter by tags
    math_ids = client.list_modules(tags=["math"])

    # Filter by prefix
    system_ids = client.list_modules(prefix="system.")
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();

    // All modules
    const allIds = client.listModules();

    // Filter by tags
    const mathIds = client.listModules({ tags: ["math"] });

    // Filter by prefix
    const systemIds = client.listModules({ prefix: "system." });
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let client = APCore::new();

    // All modules
    let all_ids = client.list_modules(None, None);

    // Filter by tags
    let math_ids = client.list_modules(Some(&["math"]), None);

    // Filter by prefix
    let system_ids = client.list_modules(None, Some("system."));
    ```

### 5.2 Describe a Module

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    description = client.describe("math.add")
    print(description)
    # Returns markdown-formatted description suitable for AI/LLM tool discovery
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    const description = client.describe("math.add");
    console.log(description);
    // Returns markdown-formatted description suitable for AI/LLM tool discovery
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let client = APCore::new();
    let description = client.describe("math.add");
    println!("{}", description);
    // Returns markdown-formatted description suitable for AI/LLM tool discovery
    ```

---

## 6. Middleware

```python
from apcore import LoggingMiddleware, TracingMiddleware

# Class-based
client.use(LoggingMiddleware()).use(TracingMiddleware())

# Function-first
client.use_before(lambda module_id, inputs, ctx: print(f"→ {module_id}"))
client.use_after(lambda module_id, inputs, output, ctx: print(f"← {module_id}"))

# Remove
mw = LoggingMiddleware()
client.use(mw)
client.remove(mw)
```

---

## 7. Event System

Requires `sys_modules.events.enabled: true` in config.

### 7.1 Subscribe to Events

```python
# Simple callback
sub = client.on("module_health_changed", lambda event: print(event.data))

# Async callback
async def on_error(event):
    await notify_admin(event.data)

sub = client.on("error_threshold_exceeded", on_error)
```

### 7.2 Unsubscribe

```python
client.off(sub)
```

### 7.3 Available Event Types

| Event Type | Emitted When |
|------------|-------------|
| `module_registered` | Module added to registry |
| `module_unregistered` | Module removed from registry |
| `config_changed` | Runtime config updated or module reloaded |
| `module_health_changed` | Module disabled/enabled or health recovery |
| `error_threshold_exceeded` | Module error rate crosses threshold |
| `latency_threshold_exceeded` | Module p99 latency exceeds threshold |

### 7.4 Direct EventEmitter Access

```python
emitter = client.events  # EventEmitter | None
if emitter:
    emitter.subscribe(my_custom_subscriber)
```

---

## 8. Module Control

Requires `sys_modules.enabled: true` in config.

### 8.1 Disable/Enable Modules

```python
# Disable — calls to this module will raise ModuleDisabledError
client.disable("risky.module", reason="Investigating issue")

# Re-enable
client.enable("risky.module", reason="Issue resolved")
```

### 8.2 System Module Queries

When system modules are enabled, you can query health, usage, and manifests directly:

```python
# Health overview
health = client.call("system.health.summary", {})

# Single module health
detail = client.call("system.health.module", {"module_id": "math.add"})

# Usage statistics
usage = client.call("system.usage.summary", {"period": "24h"})

# Full module manifest
manifest = client.call("system.manifest.full", {"prefix": "math."})
```

---

## 9. Global Entry Points

All `APCore` methods are also available as module-level functions via a default client:

```python
import apcore

# Registration
@apcore.module(id="math.add", description="Add two numbers")
def add(a: int, b: int) -> dict:
    return {"sum": a + b}

# Execution
result = apcore.call("math.add", {"a": 1, "b": 2})
result = await apcore.call_async("math.add", {"a": 1, "b": 2})
async for chunk in apcore.stream("my.module", {}):
    print(chunk)

# Validation
preflight = apcore.validate("math.add", {"a": 1})

# Discovery
apcore.register("math.add", my_module)
apcore.discover()
modules = apcore.list_modules(prefix="math.")
desc = apcore.describe("math.add")

# Middleware
apcore.use(LoggingMiddleware())
apcore.use_before(my_before_hook)
apcore.use_after(my_after_hook)
apcore.remove(mw)

# Events & Control (requires config with sys_modules enabled)
sub = apcore.on("module_health_changed", handler)
apcore.off(sub)
apcore.disable("some.module", reason="maintenance")
apcore.enable("some.module", reason="done")
```

---

## Next Steps

- [Executor API](./executor-api.md) — Lower-level execution API
- [Registry API](./registry-api.md) — Module registration and discovery
- [Event System](../features/event-system.md) — Full event system specification
- [System Modules](../features/system-modules.md) — Built-in system modules
