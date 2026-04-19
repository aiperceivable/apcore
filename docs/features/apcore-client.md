# APCore Unified Client

## Overview

The APCore class is the recommended high-level entry point for the apcore framework. It provides a single unified interface that manages the Registry, Executor, Config, and optional subsystems (system modules, events, metrics) so that users do not need to wire these components together manually. The client supports zero-config initialization for quick prototyping and full-config initialization for production deployments with system modules, observability, and event handling.

## Requirements

### Unified Facade
- Provide a single `APCore` class that wraps Registry, Executor, and optionally Config and MetricsCollector.
- Accept configuration via a `Config` object (`config`). Use `Config.load()` to load from a file path. In Rust, `APCore::from_path()` is kept as a convenience shortcut.
- If no Registry or Executor is provided, the client **MUST** create them automatically with sensible defaults.
- If a Config is provided (via either parameter) with `sys_modules.enabled: true`, the client **MUST** auto-register system modules and configure associated middleware (metrics, observability).

### Module Lifecycle
- Support decorator-based module registration (`client.module()`), direct registration (`client.register()`), and auto-discovery (`client.discover()`).
- Support module listing with optional tag and prefix filtering.
- Support module description generation (`client.describe()`) for AI/LLM tool discovery.

### Execution
- Provide synchronous (`call()`), asynchronous (`call_async()`), and streaming (`stream()`) execution methods that delegate to the underlying Executor.
- Provide a non-destructive preflight validation method (`validate()`) that runs pipeline steps 1–6 plus optional module-level preflight (7 checks total) without executing the module.

### Middleware
- Support chainable middleware registration: `use()`, `use_before()`, `use_after()`.
- Support middleware removal by identity.

### Event System
- When system modules with events are enabled, expose `on()` / `off()` methods for subscribing to framework events.
- The `events` property **MUST** expose the underlying `EventEmitter` (or return `None`/`null` if events are not configured).

### Module Control
- When system modules are enabled, expose `disable()` / `enable()` methods for runtime module toggling.
- These methods **MUST** delegate to `system.control.toggle_feature` internally.

### Global Singleton
- Python and TypeScript SDKs **SHOULD** provide module-level functions (e.g., `apcore.call()`, `apcore.module()`) backed by a default singleton client.
- Rust does **not** provide a global singleton — explicit instances only.

## Technical Design

### Initialization Modes

| Mode | Config Required | System Modules | Use Case |
|------|----------------|---------------|----------|
| Zero-config | No | No | Quick prototyping, tests |
| With config object | Yes (`config=`) | If `sys_modules.enabled` | Production |
| With defaults | Yes (`from_defaults()`) | If configured | No YAML file needed |
| Pre-built components | No | Via provided executor | Advanced / custom setups |

### Auto-Registration Behavior

When a Config with `sys_modules.enabled: true` is provided:

1. **System modules** are registered: `system.health.*`, `system.manifest.*`, `system.usage.*`, `system.control.*`.
2. **MetricsCollector** is created (if not provided) and `MetricsMiddleware` is added to the executor.
3. **Event handling** is configured if `sys_modules.events.enabled: true`:
   - `EventEmitter` is created.
   - `PlatformNotifyMiddleware` is added for health monitoring.
   - Subscribers are instantiated from config (webhook, a2a, custom types).
4. Internal `_sys_modules_context` tracks references needed by system module implementations.

### Method Summary

| Category | Method | Returns | Description |
|----------|--------|---------|-------------|
| **Registration** | `module(id, ...)` | Decorator / FunctionModule | Register function as module |
| | `register(module_id, module)` | None | Direct module registration |
| | `discover()` | int | Auto-discover and register modules |
| **Execution** | `call(module_id, inputs?, context?)` | dict | Synchronous call |
| | `call_async(module_id, inputs?, context?)` | dict | Asynchronous call |
| | `stream(module_id, inputs?, context?)` | AsyncIterator | Streaming output |
| | `validate(module_id, inputs?, context?)` | PreflightResult | Non-destructive preflight |
| **Inspection** | `list_modules(tags?, prefix?)` | list[str] | List module IDs (sorted) |
| | `describe(module_id)` | str | Markdown description for AI |
| **Middleware** | `use(middleware)` | self | Add class-based middleware |
| | `use_before(callback)` | self | Add before-middleware |
| | `use_after(callback)` | self | Add after-middleware |
| | `remove(middleware)` | bool | Remove by identity |
| **Events** | `on(event_type, handler)` | EventSubscriber | Subscribe to events |
| | `off(subscriber)` | None | Unsubscribe |
| **Control** | `disable(module_id, reason?)` | dict | Disable module at runtime |
| | `enable(module_id, reason?)` | dict | Re-enable module |
| **Properties** | `registry` | Registry | Underlying registry |
| | `executor` | Executor | Underlying executor |
| | `events` | EventEmitter \| None | Event emitter (if configured) |

!!! note "Sync/async divergence"
    Python `call()` is synchronous and blocks until the module returns. TypeScript and Rust `call()` return a `Promise`/`Future` and **MUST** be awaited. Python provides a separate `call_async()` for async contexts (e.g., inside `async def` functions or running under an event loop).

### Internal Callback Subscriber

The `on()` method creates a lightweight internal subscriber that filters events by type and delegates to the user's handler:

```python
class _CallbackSubscriber:
    def __init__(self, event_type: str, handler: Callable) -> None:
        self._event_type = event_type
        self._handler = handler
        self._is_async = asyncio.iscoroutinefunction(handler)

    async def on_event(self, event: ApCoreEvent) -> None:
        if event.event_type != self._event_type:
            return
        if self._is_async:
            await self._handler(event)
        else:
            self._handler(event)
```

This allows users to subscribe with both sync and async callbacks without implementing the full `EventSubscriber` protocol.

### Error Behavior

| Condition | Error |
|-----------|-------|
| Config file not found or invalid (via `Config.load()`) | `ValueError` (Python), `ConfigNotFoundError` (TypeScript), `Err(ModuleError)` with `ConfigNotFound` or `ConfigInvalid` (Rust) |
| `on()`, `off()` without events enabled | `RuntimeError` |
| `disable()`, `enable()` without sys_modules | `RuntimeError` |

## Usage

For complete usage examples with all three languages, see the [APCore Client API Reference](../api/client-api.md).

### Quick Start

=== "Python"
    ```python
    from apcore import APCore

    client = APCore()

    @client.module(id="math.add", description="Add two numbers")
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}

    result = client.call("math.add", {"a": 10, "b": 5})
    print(result)  # {"sum": 15}
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";

    const client = new APCore();

    client.module({
        id: "math.add",
        description: "Add two numbers",
        inputSchema: { type: "object", properties: { a: { type: "number" }, b: { type: "number" } } },
        outputSchema: { type: "object", properties: { sum: { type: "number" } } },
        execute: ({ a, b }: { a: number; b: number }) => ({ sum: a + b }),
    });

    const result = await client.call("math.add", { a: 10, b: 5 });
    console.log(result); // { sum: 15 }
    ```
=== "Rust"
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
        fn description(&self) -> &str { "Add two numbers" }
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"sum": {"type": "integer"}}})
        }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let a = inputs["a"].as_i64().unwrap_or(0);
            let b = inputs["b"].as_i64().unwrap_or(0);
            Ok(json!({"sum": a + b}))
        }
    }

    let mut client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();
    let result = client.call("math.add", json!({"a": 10, "b": 5}), None, None).await?;
    ```

### Production Setup

=== "Python"
    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    client = APCore(config=config)

    # System modules, metrics, and events are auto-configured
    sub = client.on("apcore.error.threshold_exceeded", lambda e: alert(e.data))

    # Runtime control
    client.disable("risky.module", reason="Investigating issue")
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config } from "apcore-js";

    const config = Config.load('apcore.yaml');
    const client = new APCore({ config });

    // System modules, metrics, and events are auto-configured
    const sub = client.on("apcore.error.threshold_exceeded", (e) => alert(e.data));

    // Runtime control
    await client.disable("risky.module", "Investigating issue");
    ```
=== "Rust"
    ```rust
    use apcore::APCore;

    let client = APCore::from_path("apcore.yaml")?;

    // System modules, metrics, and events are auto-configured
    let sub = client.on("apcore.error.threshold_exceeded", Box::new(AlertSubscriber));

    // Runtime control
    client.disable("risky.module", Some("Investigating issue"))?;
    ```

## Dependencies

- **Registry** — Module registration, discovery, and lookup.
- **Executor** — Module execution, middleware, ACL, and approval.
- **Config Bus** — Configuration loading and system module setup.
- **Event System** — Event emission and subscription (optional).
- **System Modules** — Health, manifest, usage, and control modules (optional).
- **Observability** — MetricsCollector and MetricsMiddleware (optional).

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/client.py` | `APCore`, `_CallbackSubscriber` |
    | `src/apcore/__init__.py` | Module-level singleton functions |

## Testing Strategy

- **Zero-config tests** verify that `APCore()` creates a functional Registry and Executor without any arguments.
- **Config object tests** verify that passing a `Config` object (loaded via `Config.load()`) applies the config file correctly.
- **Config-based tests** verify that system modules are auto-registered when config provides `sys_modules.enabled: true`.
- **Decorator tests** verify that `client.module()` registers functions as modules and they are callable.
- **Execution tests** verify that `call()`, `call_async()`, `stream()`, and `validate()` delegate correctly to the underlying Executor.
- **Middleware tests** verify chainable `use()`, `use_before()`, `use_after()`, and `remove()`.
- **Event tests** verify that `on()` / `off()` work correctly and raise `RuntimeError` when events are not configured.
- **Control tests** verify that `disable()` / `enable()` delegate to system.control.toggle_feature and raise `RuntimeError` when sys_modules are not enabled.

## Contract: ApCoreClient.call

### Inputs
- `module_id` (str/string/&str, required) — target module ID; validated against `MODULE_ID_PATTERN`; reject empty or malformed with `InvalidInputError(code=INVALID_MODULE_ID)`
- `inputs` (dict/object/Value, required) — validated against the module's `input_schema`
- `context` (Context, optional) — execution context; created fresh when absent
- `version_hint` (str/string/&str, optional) — preferred version constraint; falls back to latest on TS/Rust pending implementation

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — `module_id` is empty or malformed
- `ModuleNotFoundError(code=MODULE_NOT_FOUND)` — no module registered under `module_id`
- `SchemaValidationError(code=SCHEMA_VALIDATION_FAILED)` — `inputs` fails the module's `input_schema`
- Any error raised by the module's `execute` handler propagates unchanged

### Returns
- On success: `dict`/`Record<string, unknown>`/`serde_json::Value` — the module's validated output

### Properties
- async: sync surface (`call`) + async surface (`call_async`) in Python; async-only in TypeScript and Rust
- thread_safe: true (Executor holds an internal lock on shared state)
- pure: false (side-effects: span created, metrics emitted, middleware hooks invoked)
- idempotent: false (module `execute` is not guaranteed idempotent)

## Contract: ApCoreClient.start

### Inputs
- No required inputs (uses configuration from constructor)

### Errors
- `ConfigError(code=CONFIG_INVALID)` — if configuration validation fails on startup

### Returns
- On success: void/None/()

### Properties
- async: false in Python; async in TypeScript and Rust
- thread_safe: false (call once before any concurrent usage)
- idempotent: false

## Contract: ApCoreClient.stop

### Inputs
- No required inputs

### Errors
- No errors raised under normal operation

### Returns
- On success: void/None/()

### Properties
- async: false in Python; async in TypeScript and Rust
- thread_safe: false (do not call concurrently with active requests)
- idempotent: true (multiple stops are safe)
