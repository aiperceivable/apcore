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
        config_path: str | None = None,
        metrics_collector: "MetricsCollector | None" = None,
    ) -> None:
        """
        Initialize APCore client

        Args:
            registry: Module registry (auto-created if None)
            executor: Module executor (auto-created if None)
            config: Framework configuration (enables sys_modules if provided)
            config_path: Path to config file (e.g. "apcore.yaml").
                Shorthand for Config.load(path). Mutually exclusive with config.
            metrics_collector: Metrics collector (auto-created if sys_modules enabled)

        Raises:
            ValueError: If both config and config_path are provided
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
        Non-destructive preflight check (Steps 1-7, no execution)

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

Load directly from a config file path:

=== "Python"

    ```python
    from apcore import APCore

    client = APCore(config_path="apcore.yaml")

    # System modules are auto-registered when config has sys_modules.enabled=true
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore({ configPath: 'apcore.yaml' });

    // System modules are auto-registered when config has sys_modules.enabled=true
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let client = APCore::from_path("apcore.yaml")?;

    // System modules are auto-registered when config has sys_modules.enabled=true
    ```

Or pass a pre-loaded `Config` object for full control:

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    # Modify config programmatically if needed
    client = APCore(config=config)
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const config = Config.load("apcore.yaml");
    // Modify config programmatically if needed
    const client = new APCore({ config });
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, Config};

    let config = Config::load("apcore.yaml")?;
    // Modify config programmatically if needed
    let client = APCore::with_config(config);
    ```

!!! note "Mutually exclusive"
    `config` and `config_path` cannot be used together. Providing both raises a `ValueError` (Python) or throws a `TypeError` (TypeScript). In Rust, this conflict is prevented by design — `from_path()` and `with_config()` are separate constructors.

### 2.3 With Defaults (No YAML File)

=== "Python"

    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.from_defaults()
    client = APCore(config=config)
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Config } from 'apcore-js';

    const config = Config.fromDefaults();
    const client = new APCore({ config });
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, Config};

    let config = Config::default();
    let client = APCore::with_config(config);
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
    use apcore::{APCore, Config, Registry};

    let registry = Registry::new();
    let config = Config::default();
    let client = APCore::with_components(registry, config);
    ```

---

## 3. Module Registration

### 3.1 Decorator / Trait-Based

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()

    @client.module(
        id="math.add",
        description="Add two numbers",
        tags=["math", "utility"],
    )
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();

    const add = client.module({
        id: "math.add",
        description: "Add two numbers",
        tags: ["math", "utility"],
    });

    add.implement(({ a, b }: { a: number; b: number }) => ({ sum: a + b }));
    ```

=== "Rust"

    Rust does not have decorators. Implement the `Module` trait and register explicitly:

    ```rust
    use apcore::APCore;
    use apcore::module::Module;
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct AddModule;

    #[async_trait]
    impl Module for AddModule {
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"sum": {"type": "integer"}}})
        }
        fn description(&self) -> &str {
            "Add two numbers"
        }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let a = inputs["a"].as_i64().unwrap_or(0);
            let b = inputs["b"].as_i64().unwrap_or(0);
            Ok(json!({"sum": a + b}))
        }
    }

    let mut client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();
    ```

### 3.2 Direct Registration

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    client.register("math.add", add_module)
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    client.register("math.add", addModule);
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let mut client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();
    ```

### 3.3 Auto-Discovery

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    count = client.discover()
    print(f"Discovered {count} modules")
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    const count = await client.discover();
    console.log(`Discovered ${count} modules`);
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let mut client = APCore::new();
    let count = client.discover().await?;
    println!("Discovered {} modules", count);
    ```

---

## 4. Module Execution

### 4.1 Basic Call

!!! note
    Python's `call()` is synchronous; TypeScript and Rust's `call()` are async (require `await`). Use `call_async()` / `callAsync()` in Python for async contexts.

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
    let result = client.call("math.add", serde_json::json!({"a": 10, "b": 5}), None, None).await?;
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

    In Rust, `stream()` collects all chunks and returns them as a `Vec<Value>`:

    ```rust
    use apcore::APCore;

    let client = APCore::new();
    let chunks = client.stream(
        "my.streaming_module",
        serde_json::json!({"query": "hello"}),
        None,
        None,
    ).await?;
    for chunk in &chunks {
        println!("{:?}", chunk);
    }
    ```

### 4.4 Preflight Validation

=== "Python"

    ```python
    from apcore import APCore

    client = APCore()
    preflight = client.validate("math.add", {"a": 10, "b": 5})

    if preflight.valid:
        result = client.call("math.add", {"a": 10, "b": 5})
    else:
        for check in preflight.checks:
            if not check.passed:
                print(f"Failed: {check.check} — {check.error}")
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();
    const preflight = await client.validate("math.add", { a: 10, b: 5 });

    if (preflight.valid) {
        const result = await client.call("math.add", { a: 10, b: 5 });
    } else {
        for (const check of preflight.checks) {
            if (!check.passed) {
                console.log(`Failed: ${check.check} — ${check.error}`);
            }
        }
    }
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let client = APCore::new();
    let preflight = client.validate(
        "math.add",
        &serde_json::json!({"a": 10, "b": 5}),
        None,
    ).await?;

    if preflight.valid {
        let result = client.call(
            "math.add",
            serde_json::json!({"a": 10, "b": 5}),
            None,
            None,
        ).await?;
    } else {
        for check in &preflight.checks {
            if !check.passed {
                println!("Failed: {} — {:?}", check.check, check.error);
            }
        }
    }
    ```

### 4.5 Version Hint

=== "Python"

    ```python
    result = client.call("math.add", {"a": 1, "b": 2}, version_hint=">=1.0.0")
    ```

=== "TypeScript"

    ```typescript
    const result = await client.call("math.add", { a: 1, b: 2 }, undefined, ">=1.0.0");
    ```

=== "Rust"

    ```rust
    let result = client.call(
        "math.add",
        serde_json::json!({"a": 1, "b": 2}),
        None,
        Some(">=1.0.0"),
    ).await?;
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

=== "Python"

    ```python
    from apcore import APCore, LoggingMiddleware, TracingMiddleware

    client = APCore()

    # Class-based (chainable)
    client.use(LoggingMiddleware()).use(TracingMiddleware())

    # Function-first (chainable)
    client.use_before(lambda module_id, inputs, ctx: print(f"→ {module_id}"))
    client.use_after(lambda module_id, inputs, output, ctx: print(f"← {module_id}"))

    # Remove
    mw = LoggingMiddleware()
    client.use(mw)
    client.remove(mw)
    ```

=== "TypeScript"

    ```typescript
    import { APCore, LoggingMiddleware, TracingMiddleware } from 'apcore-js';

    const client = new APCore();

    // Class-based (chainable)
    client.use(new LoggingMiddleware()).use(new TracingMiddleware());

    // Function-first (chainable)
    client.useBefore((moduleId, inputs, ctx) => console.log(`→ ${moduleId}`));
    client.useAfter((moduleId, inputs, output, ctx) => console.log(`← ${moduleId}`));

    // Remove
    const mw = new LoggingMiddleware();
    client.use(mw);
    client.remove(mw);
    ```

=== "Rust"

    In Rust, `use` is a reserved keyword so the method is named `use_middleware`.
    Middleware methods return `Result<&mut Self, ModuleError>` for chaining:

    ```rust
    use apcore::APCore;

    let mut client = APCore::new();

    // Class-based (chainable via Result)
    client
        .use_middleware(Box::new(LoggingMiddleware::new()))?
        .use_middleware(Box::new(TracingMiddleware::new()))?;

    // Before/after callbacks (chainable)
    client
        .use_before(Box::new(MyBeforeMiddleware))?
        .use_after(Box::new(MyAfterMiddleware))?;

    // Remove by name
    let removed = client.remove("logging");
    ```

---

## 7. Event System

Requires `sys_modules.events.enabled: true` in config.

### 7.1 Subscribe to Events

=== "Python"

    ```python
    from apcore import APCore

    client = APCore(config_path="apcore.yaml")  # sys_modules.events.enabled: true

    # Simple callback
    sub = client.on("apcore.module.toggled", lambda event: print(event.data))

    # Async callback
    async def on_error(event):
        await notify_admin(event.data)

    sub = client.on("error_threshold_exceeded", on_error)
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore({ configPath: 'apcore.yaml' }); // sys_modules.events.enabled: true

    const sub = client.on("apcore.module.toggled", (event) => console.log(event.data));
    ```

=== "Rust"

    In Rust, `on()` returns a `String` subscriber ID (not an object).
    Implement the `EventSubscriber` trait and pass a boxed instance:

    ```rust
    use apcore::APCore;
    use apcore::events::subscribers::EventSubscriber;

    let mut client = APCore::new();

    let sub_id = client.on("apcore.module.toggled", Box::new(MySubscriber));
    ```

### 7.2 Unsubscribe

=== "Python"

    ```python
    client.off(sub)
    ```

=== "TypeScript"

    ```typescript
    client.off(sub);
    ```

=== "Rust"

    In Rust, pass the subscriber ID string returned by `on()`:

    ```rust
    client.off(&sub_id);
    ```

### 7.3 Available Event Types

| Event Type | Emitted When |
|------------|-------------|
| `module_registered` | Module added to registry |
| `module_unregistered` | Module removed from registry |
| `apcore.config.updated` | Runtime config updated via `system.control.update_config` |
| `apcore.module.reloaded` | Module hot-reloaded via `system.control.reload_module` |
| `apcore.module.toggled` | Module disabled/enabled via `system.control.toggle_feature` |
| `apcore.health.recovered` | Module error rate recovered below threshold |
| `error_threshold_exceeded` | Module error rate crosses threshold |
| `latency_threshold_exceeded` | Module p99 latency exceeds threshold |

### 7.4 Direct EventEmitter Access

=== "Python"

    ```python
    emitter = client.events  # EventEmitter | None
    if emitter:
        emitter.subscribe(my_custom_subscriber)
    ```

=== "TypeScript"

    ```typescript
    const emitter = client.events; // EventEmitter | null
    if (emitter) {
        emitter.subscribe(myCustomSubscriber);
    }
    ```

=== "Rust"

    ```rust
    if let Some(emitter) = client.events() {
        // Access the EventEmitter directly
        println!("Event emitter is configured");
    }
    ```

---

## 8. Module Control

Requires `sys_modules.enabled: true` in config.

### 8.1 Disable/Enable Modules

=== "Python"

    ```python
    from apcore import APCore

    client = APCore(config_path="apcore.yaml")  # sys_modules.enabled: true

    # Disable — calls to this module will raise ModuleDisabledError
    client.disable("risky.module", reason="Investigating issue")

    # Re-enable
    client.enable("risky.module", reason="Issue resolved")
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore({ configPath: 'apcore.yaml' }); // sys_modules.enabled: true

    // Disable — calls to this module will throw ModuleDisabledError
    await client.disable("risky.module", "Investigating issue");

    // Re-enable
    await client.enable("risky.module", "Issue resolved");
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let mut client = APCore::new();
    // Register modules first...

    // Disable — calls to this module will return ModuleDisabledError
    client.disable("risky.module", Some("Investigating issue"))?;

    // Re-enable
    client.enable("risky.module", Some("Issue resolved"))?;
    ```

### 8.2 System Module Queries

When system modules are enabled, you can query health, usage, and manifests directly:

=== "Python"

    ```python
    from apcore import APCore

    client = APCore(config_path="apcore.yaml")  # sys_modules.enabled: true

    # Health overview
    health = client.call("system.health.summary", {})

    # Single module health
    detail = client.call("system.health.module", {"module_id": "math.add"})

    # Usage statistics
    usage = client.call("system.usage.summary", {"period": "24h"})

    # Full module manifest
    manifest = client.call("system.manifest.full", {"prefix": "math."})
    ```

=== "TypeScript"

    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore({ configPath: 'apcore.yaml' }); // sys_modules.enabled: true

    // Health overview
    const health = await client.call("system.health.summary", {});

    // Single module health
    const detail = await client.call("system.health.module", { module_id: "math.add" });

    // Usage statistics
    const usage = await client.call("system.usage.summary", { period: "24h" });

    // Full module manifest
    const manifest = await client.call("system.manifest.full", { prefix: "math." });
    ```

=== "Rust"

    ```rust
    use apcore::APCore;

    let client = APCore::new();

    // Health overview
    let health = client.call("system.health.summary", serde_json::json!({}), None, None).await?;

    // Single module health
    let detail = client.call(
        "system.health.module",
        serde_json::json!({"module_id": "math.add"}),
        None,
        None,
    ).await?;

    // Usage statistics
    let usage = client.call(
        "system.usage.summary",
        serde_json::json!({"period": "24h"}),
        None,
        None,
    ).await?;

    // Full module manifest
    let manifest = client.call(
        "system.manifest.full",
        serde_json::json!({"prefix": "math."}),
        None,
        None,
    ).await?;
    ```

---

## 9. Global Entry Points

All `APCore` methods are also available as module-level functions via a default singleton client.
This pattern is supported in Python and TypeScript. Rust does not provide a global singleton —
use an explicit `APCore` instance instead.

=== "Python"

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
    sub = apcore.on("apcore.module.toggled", handler)
    apcore.off(sub)
    apcore.disable("some.module", reason="maintenance")
    apcore.enable("some.module", reason="done")
    ```

=== "TypeScript"

    ```typescript
    import apcore from 'apcore-js';

    // Registration
    const add = apcore.module({
        id: "math.add",
        description: "Add two numbers",
    });
    add.implement(({ a, b }: { a: number; b: number }) => ({ sum: a + b }));

    // Execution
    const result = await apcore.call("math.add", { a: 1, b: 2 });
    const result2 = await apcore.callAsync("math.add", { a: 1, b: 2 });
    for await (const chunk of apcore.stream("my.module", {})) {
        console.log(chunk);
    }

    // Validation
    const preflight = await apcore.validate("math.add", { a: 1 });

    // Discovery
    apcore.register("math.add", myModule);
    await apcore.discover();
    const modules = apcore.listModules({ prefix: "math." });
    const desc = apcore.describe("math.add");

    // Middleware
    apcore.use(new LoggingMiddleware());
    apcore.useBefore(myBeforeHook);
    apcore.useAfter(myAfterHook);
    apcore.remove(mw);

    // Events & Control (requires config with sys_modules enabled)
    const sub = apcore.on("apcore.module.toggled", handler);
    apcore.off(sub);
    await apcore.disable("some.module", "maintenance");
    await apcore.enable("some.module", "done");
    ```

=== "Rust"

    Rust does not provide a global singleton. Use an explicit `APCore` instance:

    ```rust
    use apcore::APCore;

    let mut client = APCore::new();
    // All methods are called on the client instance
    // See sections 2–8 for usage examples
    ```

---

## 10. Language-Specific Adaptations

The APCore interface follows each language's idioms while maintaining functional equivalence.

### TypeScript

| Spec Method | TypeScript Name | Notes |
|---|---|---|
| `call_async()` | `callAsync()` | camelCase convention |
| `use_before()` | `useBefore()` | camelCase convention |
| `use_after()` | `useAfter()` | camelCase convention |
| `list_modules()` | `listModules()` | camelCase convention |
| Constructor | `new APCore({ config })` or `new APCore({ configPath })` | Options object pattern |

### Rust

| Spec Method | Rust Name | Notes |
|---|---|---|
| `use()` | `use_middleware()` | `use` is a reserved keyword in Rust |
| `use_before()` | `use_before()` | Accepts `Box<dyn BeforeMiddleware>`, returns `Result<&mut Self, ModuleError>` |
| `use_after()` | `use_after()` | Accepts `Box<dyn AfterMiddleware>`, returns `Result<&mut Self, ModuleError>` |
| `on()` | `on()` | Returns `String` (subscriber ID) instead of `EventSubscriber` object |
| `off()` | `off()` | Accepts `&str` (subscriber ID) instead of `EventSubscriber` object |
| `stream()` | `stream()` | Returns `Vec<Value>` (batch-collected chunks) instead of async iterator |
| `disable()` | `disable()` | Returns `Result<(), ModuleError>` instead of `dict`; `reason` is `Option<&str>` |
| `enable()` | `enable()` | Returns `Result<(), ModuleError>` instead of `dict`; `reason` is `Option<&str>` |
| Constructor | `APCore::new()`, `APCore::with_config(config)`, or `APCore::from_path(path)` | Three construction methods |
| `module()` | N/A | Rust has no decorators; use `impl Module` trait + `register()` instead |
| `events` property | `events()` method | Rust uses accessor methods instead of properties |
| `registry` property | `registry()` method | Rust uses accessor methods instead of properties |
| `executor` property | `executor()` method | Rust uses accessor methods instead of properties |

**Rust-only methods** (not in the cross-language spec):

| Method | Purpose |
|---|---|
| `with_components(registry, config)` | Build client from a pre-configured Registry |
| `with_options(registry, executor, config, metrics_collector)` | Full constructor with all optional parameters |
| `reload()` | Reload config and re-discover modules |
| `shutdown()` | Release resources |

---

## Next Steps

- [Executor API](./executor-api.md) — Lower-level execution API
- [Registry API](./registry-api.md) — Module registration and discovery
- [Event System](../features/event-system.md) — Full event system specification
- [System Modules](../features/system-modules.md) — Built-in system modules
