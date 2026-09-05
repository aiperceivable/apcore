---
description: "APCore facade over Registry, Executor, Config: zero- or full-config init, module registration/discovery, sync/async/streaming calls, middleware, events, runtime module toggling."
---

# APCore Unified Client

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §12 SDK Implementation Guide.


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
- There is no cross-language `start()` / `stop()` lifecycle contract (decision **D-09**, 2026-05-02 alignment review): the client needs no startup or shutdown phase beyond construction. Python **MAY** additionally provide `close()` as a Python-only convenience that releases the cached synchronous event loop inside the Executor; it is idempotent, raises no error under normal operation, and the same `APCore` instance remains usable afterward — a synchronous `call()` lazily allocates a fresh loop on next use. TypeScript and Rust have no equivalent because neither caches a loop of that kind.

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
- The Python SDK **SHOULD** provide module-level functions (e.g., `apcore.call()`, `apcore.module()`) backed by a default singleton client. This is a Python-only convenience layer.
- TypeScript and Rust do **not** provide a global singleton — explicit instances only.

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
| **Middleware** | `use(middleware)` | self | Add class-based middleware (Rust: `use_middleware()` — `use` is a reserved keyword) |
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

!!! note "Rust keyword conflict: `use` → `use_middleware`"
    In Rust, `use` is a reserved keyword. The method that Python and TypeScript expose as `.use(middleware)` is named **`.use_middleware(middleware)`** in the Rust SDK. All other method names are identical across languages. This is the only renamed method in the APCore client API.

    ```rust
    // Rust — use use_middleware() where Python/TS use .use()
    client.use_middleware(Box::new(logging_middleware));
    client.use_before(|ctx| { ... });
    client.use_after(|ctx| { ... });
    ```

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

### Language-Specific Adaptations

The APCore interface follows each language's idioms while maintaining functional equivalence.

**TypeScript:**

| Spec method | TypeScript name | Notes |
|-------------|-----------------|-------|
| `call_async()` | `callAsync()` | camelCase |
| `use_before()` | `useBefore()` | camelCase |
| `use_after()` | `useAfter()` | camelCase |
| `list_modules()` | `listModules()` | camelCase |
| Constructor | `new APCore({ config })` | Options object pattern |

**Rust:**

| Spec method | Rust name | Notes |
|-------------|-----------|-------|
| `use()` | `use_middleware()` | `use` is a reserved keyword |
| `use_before()` | `use_before()` | Accepts `Box<dyn BeforeMiddleware>`, returns `Result<&mut Self, ModuleError>` |
| `use_after()` | `use_after()` | Accepts `Box<dyn AfterMiddleware>`, returns `Result<&mut Self, ModuleError>` |
| `on()` | `on()` | Returns `String` (subscriber ID) instead of an `EventSubscriber` object |
| `off()` | `off()` | Accepts `&str` (subscriber ID) instead of an `EventSubscriber` object |
| `stream()` | `stream()` | Returns `Stream<Item = Result<Value, ModuleError>>` (true incremental streaming) |
| `disable()` | `disable()` | Returns `Result<Value, ModuleError>`; `reason` is `Option<&str>` |
| `enable()` | `enable()` | Returns `Result<Value, ModuleError>`; `reason` is `Option<&str>` |
| Constructor | `APCore::new()`, `APCore::with_config(config)`, `APCore::from_path(path)` | Three construction forms |
| `module()` | N/A | Rust has no decorators; use `impl Module` + `register()` |
| `events` property | `events()` method | Accessor methods instead of properties |
| `registry` property | `registry()` method | Accessor methods instead of properties |
| `executor` property | `executor()` method | Accessor methods instead of properties |

**Rust-only methods** (not in the cross-language spec):

| Method | Purpose |
|--------|---------|
| `with_components(registry, config)` | Build client from a pre-configured Registry |
| `with_options(registry, executor, config, metrics_collector)` | Full constructor with all optional parameters |
| `reload()` | Reload config and re-discover modules |

## Usage

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
    sub = client.on("apcore.health.error_threshold_exceeded", lambda e: alert(e.data))

    # Runtime control
    client.disable("risky.module", reason="Investigating issue")
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config } from "apcore-js";

    const config = Config.load('apcore.yaml');
    const client = new APCore({ config });

    // System modules, metrics, and events are auto-configured
    const sub = client.on("apcore.health.error_threshold_exceeded", (e) => alert(e.data));

    // Runtime control
    await client.disable("risky.module", "Investigating issue");
    ```
=== "Rust"
    ```rust
    use apcore::APCore;

    let client = APCore::from_path("apcore.yaml")?;

    // System modules, metrics, and events are auto-configured
    let sub = client.on("apcore.health.error_threshold_exceeded", Box::new(AlertSubscriber));

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

## Contract: APCore.call

### Inputs
- `module_id` (str/string/&str, required) — target module ID; validated against `MODULE_ID_PATTERN`; reject empty or malformed with `InvalidInputError(code=INVALID_MODULE_ID)`
- `inputs` (dict/object/Value, required) — validated against the module's `input_schema`
- `context` (Context, optional) — execution context; created fresh when absent
- `version_hint` (str/string/&str, optional) — preferred version constraint; falls back to latest on TS/Rust pending implementation

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — `module_id` is empty or malformed
- `ModuleNotFoundError(code=MODULE_NOT_FOUND)` — no module registered under `module_id`
- `SchemaValidationError(code=SCHEMA_VALIDATION_ERROR)` — `inputs` fails the module's `input_schema`
- Any error raised by the module's `execute` handler propagates unchanged

### Returns
- On success: `dict`/`Record<string, unknown>`/`serde_json::Value` — the module's validated output

### Properties
- async: sync surface (`call`) + async surface (`call_async`) in Python; async-only in TypeScript and Rust
- thread_safe: true (Executor holds an internal lock on shared state)
- pure: false (side-effects: span created, metrics emitted, middleware hooks invoked)
- idempotent: false (module `execute` is not guaranteed idempotent)

## Contract: APCore.on

### Inputs
- `event_type` (str/string, required) — canonical event type string (e.g. `"apcore.registry.module_registered"`); MUST be a non-empty string; filtered by exact equality match inside the subscriber
- `handler` (callable/Function, required) — sync or async callback receiving an `ApCoreEvent`; MUST NOT be null/None; Python accepts both sync and async callables (detected via `asyncio.iscoroutinefunction`); TypeScript accepts `(event: ApCoreEvent) => void | Promise<void>`

### Errors
- `RuntimeError` (Python) / `Error` (TypeScript) — raised immediately if `sys_modules.events` is not enabled (i.e., the `events` property returns `None`/`null`); message: `"Events are not enabled. Set sys_modules.enabled=true and sys_modules.events.enabled=true in config."`

### Returns
- On success: `EventSubscriber` — the created subscriber object; pass to `off()` to cancel the subscription

### Properties
- async: false (synchronous in both Python and TypeScript)
- thread_safe: true (Python `EventEmitter.subscribe` holds an internal lock before appending)
- pure: false (registers handler into the emitter's internal subscriber list)
- idempotent: false (registering the same handler twice creates two independent subscriptions that each fire)

## Contract: APCore.off

### Inputs
- `subscriber` (EventSubscriber, required) — the handle returned by a prior call to `on()`; MUST NOT be null/None

### Errors
- `RuntimeError` (Python) / `Error` (TypeScript) — raised immediately if `sys_modules.events` is not enabled (same guard as `on()`); message identical to `on()` error

### Returns
- On success: void/None — no return value

### Properties
- async: false (synchronous in both Python and TypeScript)
- thread_safe: true (Python `EventEmitter.unsubscribe` holds an internal lock before removing)
- pure: false (mutates the emitter's internal subscriber list)
- idempotent: true (unsubscribing a subscriber that is not present is a no-op; Python implementation uses `list.remove` under a guard that tolerates absence)

## Contract: APCore.stream

### Inputs
- `module_id` (str/string, required) — target module ID; validated via `_validate_module_id`; MUST be a non-empty string matching the module ID pattern
- `inputs` (dict/object, optional) — input arguments for the module; `None`/`null` is treated as `{}`
- `context` (Context, optional) — execution context; auto-created when absent
- `version_hint` (str/string, optional) — preferred version constraint; falls back to latest

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — `module_id` is empty or malformed (raised before pipeline starts)
- `ModuleNotFoundError(code=MODULE_NOT_FOUND)` — no module registered under `module_id`
- `SchemaValidationError(code=SCHEMA_VALIDATION_ERROR)` — `inputs` fails the module's `input_schema`
- `ExecutionCancelledError` — propagated unchanged if the execution context is cancelled mid-stream
- Any error raised by the module's `execute`/`stream` handler: in Python, a recovery dict chunk is yielded and the generator returns cleanly; retry signals during streaming are ignored and the original error is re-raised

### Returns
- On success: async generator / `AsyncGenerator` / `AsyncIterator` that yields `dict`/`Record<string, unknown>` chunks
- If the module does not implement a `stream()` method, the pipeline falls back to a single `execute()` call and yields its output as one chunk

### Properties
- async: true (Python: `async def stream()` coroutine / async generator; TypeScript: `async *stream()` async generator)
- thread_safe: true (delegates to Executor which holds internal locks)
- pure: false (side-effects: span created, metrics emitted, middleware hooks invoked)
- idempotent: false (module execution is not guaranteed idempotent)

## Contract: APCore.validate

### Inputs
- `module_id` (str/string, required) — target module ID; MUST be a non-empty string matching the module ID pattern
- `inputs` (dict/object, optional) — input data to validate against the module's `input_schema`; `None`/`null` is treated as `{}`
- `context` (Context, optional) — execution context used for ACL and call-chain checks; auto-created when absent

### Errors
- No errors are raised for validation failures — failures are captured in the returned `PreflightResult`
- `InvalidInputError(code=INVALID_MODULE_ID)` — raised if `module_id` is empty or malformed (before pipeline begins)

### Returns
- On success: `PreflightResult` — an object with:
  - `valid: bool` — `True` only if all checks passed
  - `checks: list[PreflightCheckResult]` — per-step results covering: `module_id`, `executor_binding`, `module_lookup`, `call_chain`, `acl`, `approval`, `schema`, `module_preflight` (8 checks), plus an optional `module_preview` check appended only when the target module implements a preview hook (all three SDKs emit this set)
  - `requires_approval: bool` — `True` if the module carries a `requires_approval` annotation (informational only; not enforced by validate)
  - `errors: list[dict]` — convenience property; aggregates `error` fields from failed checks

### Properties
- async: sync in Python (delegates to async impl via sync-in-thread model); async in TypeScript (`Promise<PreflightResult>`)
- thread_safe: true
- pure: false (pipeline dry-run creates a span and invokes middleware up to the execute step)
- idempotent: true (no state mutation; repeated calls with the same arguments return equivalent results)

## Contract: APCore.disable

### Inputs
- `module_id` (str/string, required) — ID of the module to disable; passed directly to `system.control.toggle_feature`
- `reason` (str/string, optional) — audit reason string; defaults to `"Disabled via APCore client"`

### Errors
- `RuntimeError` (Python) / `Error` (TypeScript) / `Err(ModuleError(code=SYS_MODULES_DISABLED))` (Rust) — raised immediately if `sys_modules` are not enabled. Python message: `"disable() requires sys_modules to be enabled. Pass a Config with sys_modules.enabled=true to APCore()."` TypeScript message: `"Cannot call disable(): sys_modules must be enabled in config."` Rust message: `"disable() requires sys_modules to be enabled. Pass a Config with sys_modules.enabled=true to APCore::new()."`
- Any error raised by `system.control.toggle_feature` (e.g., `ModuleNotFoundError` if `module_id` is not registered) propagates unchanged

### Returns
- On success: `dict`/`Record<string, unknown>` — result from `system.control.toggle_feature`, containing at minimum: `success` (bool), `module_id` (str), `enabled` (bool, `false` on success)

### Properties
- async: sync in Python (delegates via sync-in-thread model); async in TypeScript (`Promise<Record<string, unknown>>`)
- thread_safe: true (delegates to Executor)
- pure: false (mutates the runtime disabled-modules registry)
- idempotent: true (disabling an already-disabled module SHOULD succeed without error)

## Contract: APCore.enable

### Inputs
- `module_id` (str/string, required) — ID of the module to re-enable; passed directly to `system.control.toggle_feature`
- `reason` (str/string, optional) — audit reason string; defaults to `"Enabled via APCore client"`

### Errors
- `RuntimeError` (Python) / `Error` (TypeScript) / `Err(ModuleError(code=SYS_MODULES_DISABLED))` (Rust) — raised immediately if `sys_modules` are not enabled. Python message: `"enable() requires sys_modules to be enabled. Pass a Config with sys_modules.enabled=true to APCore()."` TypeScript message: `"Cannot call enable(): sys_modules must be enabled in config."` Rust message: `"enable() requires sys_modules to be enabled. Pass a Config with sys_modules.enabled=true to APCore::new()."`
- Any error raised by `system.control.toggle_feature` propagates unchanged

### Returns
- On success: `dict`/`Record<string, unknown>` — result from `system.control.toggle_feature`, containing at minimum: `success` (bool), `module_id` (str), `enabled` (bool, `true` on success)

### Properties
- async: sync in Python (delegates via sync-in-thread model); async in TypeScript (`Promise<Record<string, unknown>>`)
- thread_safe: true (delegates to Executor)
- pure: false (mutates the runtime disabled-modules registry)
- idempotent: true (enabling an already-enabled module SHOULD succeed without error)

## Contract: APCore.__init__

### Inputs
- `registry` (Registry, optional) — pre-built Registry instance; a new `Registry()` is created when absent
- `executor` (Executor, optional) — pre-built Executor instance; a new `Executor(registry=..., config=...)` is created when absent
- `config` (Config, optional) — framework configuration object; use `Config.load(path)` to load from a YAML file; when absent the client runs in zero-config mode with no system modules
- `metrics_collector` (MetricsCollector, optional) — observability collector; auto-created when `config.sys_modules.enabled=true` and none is provided; ignored in zero-config mode

### Errors
- No errors are raised by `__init__` itself; if `sys_modules` registration fails, the error is caught, logged at WARNING level, and the client continues with an empty `_sys_modules_context` (system-module methods will raise `RuntimeError` on use)

### Returns
- On success: a fully initialized `APCore` instance

### Properties
- async: false (synchronous in all languages)
- thread_safe: false (do not share a partially-constructed instance across threads; all concurrent usage must start after construction is complete)
- pure: false (creates Registry/Executor, optionally registers system modules, adds middleware)
- idempotent: false

## Contract: APCore.module

### Inputs
- `id` (str/string, optional) — module ID to register the function under; MUST match `MODULE_ID_PATTERN` when provided; when absent the registry derives an ID from the decorated function's name
- `description` (str/string, optional) — short human-readable description of the module
- `documentation` (str/string, optional) — extended Markdown documentation
- `annotations` (dict/object, optional) — key-value annotations for routing or platform metadata (e.g. `requires_approval`)
- `tags` (list[str]/string[], optional) — tag strings for filtering via `list_modules(tags=...)`
- `version` (str/string, optional) — semver version string; defaults to `"1.0.0"`
- `metadata` (dict/object, optional) — additional metadata stored alongside the module descriptor
- `display` (dict/object, optional) — display hints (name, icon, color) for UIs
- `examples` (list/array, optional) — input/output examples for documentation and AI tool discovery

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — `id` is provided but empty, malformed, exceeds `MAX_MODULE_ID_LENGTH`, or contains a reserved first-segment word
- `InvalidInputError(code=DUPLICATE_MODULE_ID)` — `id` is already registered (duplicate registration)

### Returns
- On success: the decorated function, unchanged (the function is registered as a `FunctionModule` as a side effect; the original callable is returned so it remains directly callable in Python)

### Properties
- async: false (synchronous decorator in Python and TypeScript)
- thread_safe: true (delegates to `Registry.register` which holds an internal RLock)
- pure: false (registers the function into the client's Registry as a side effect)
- idempotent: false (applying the decorator twice registers two entries and raises `InvalidInputError` on the second)

## Contract: APCore.register

### Inputs
- `module_id` (str/string/&str, required) — unique ID to register the module under; MUST be a non-empty string matching `MODULE_ID_PATTERN`, ≤192 characters, with no reserved first-segment word
- `module_obj` (any Module instance, required) — the module object to register; MUST NOT be null/None; raw-dict `input_schema`/`output_schema` are wrapped in a `_DictSchemaAdapter` automatically

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — `module_id` is empty, malformed, exceeds the length limit, contains a reserved word, or is already registered under that ID
- `RuntimeError` — if the module's `on_load()` hook raises (the partial registration is rolled back before propagating)

### Returns
- On success: void/None/() — no return value

### Properties
- async: false (synchronous in all languages)
- thread_safe: true (Registry holds an internal RLock around the write)
- pure: false (mutates the registry's module map, triggers `register` event callbacks)
- idempotent: false (registering the same `module_id` twice raises `InvalidInputError` on the second call)

## Contract: APCore.discover

### Inputs
- No parameters — discovery roots come from `extensions.root` in the config provided at construction time (defaults to the framework default root when no config is given)

### Errors
- `CircularDependencyError` — if circular inter-module dependencies are detected in the discovered set
- `ConfigNotFoundError` — if a configured extension root directory does not exist on disk
- File-level errors (import failures, validation failures, `on_load()` failures) are logged at WARNING/ERROR level and silently skipped; they do NOT propagate to the caller

### Returns
- On success: `int` — count of modules successfully registered in this discovery pass (0 if no modules are found or all fail validation)

### Properties
- async: **false in Python** (synchronous file-system scan and import, on the calling thread); **async in TypeScript and Rust**. Neither is a gap: TypeScript's discovery resolves each module's entry point via ESM dynamic `import()`, which has no synchronous form in Node — a discovery root containing an ESM module file structurally cannot be scanned without an `await` somewhere. Rust's `Registry::discover` takes a `discoverer: &dyn Discoverer` and awaits `discoverer.discover(...)` — the trait itself is `async`, not merely reserved for a hypothetical future implementation, mirroring TypeScript's `CustomDiscoverer` interface (which likewise may return a `Promise`) and Python's own `ApprovalHandler` pattern of an async, pluggable extension point. The cross-language contract is the **outcome**, not the calling convention: all three return the same `int`/`number`/`usize` count of newly-registered modules, raise the same categories of error (`CircularDependencyError`, `ConfigNotFoundError`), and silently skip-and-log the same per-file failures — a caller adapting to each language's native async idiom sees identical registry state afterward.
- thread_safe: true (each `Registry.register` call inside discovery holds the registry's RLock)
- pure: false (imports Python files, instantiates module classes, and mutates the registry)
- idempotent: false (calling `discover()` twice on a directory that has not changed will attempt to re-register already-registered modules, raising `InvalidInputError` for duplicates; callers should guard with `list_modules()` or unregister first)

## Contract: APCore.list_modules

### Inputs
- `tags` (list[str]/string[], optional) — when provided, only modules possessing ALL listed tags (via module attribute or merged metadata) are included; `None`/`null` means no tag filtering
- `prefix` (str/string, optional) — when provided, only modules whose ID starts with this string are included; `None`/`null` means no prefix filtering

### Errors
- No errors raised under normal operation

### Returns
- On success: `list[str]`/`string[]` — alphabetically sorted list of matching module IDs; empty list when no modules match

### Properties
- async: false (synchronous in all languages)
- thread_safe: true (Registry takes a snapshot under its RLock before filtering)
- pure: true (read-only; no state mutation)
- idempotent: true

## Contract: APCore.describe

### Inputs
- `module_id` (str/string/&str, required) — ID of the module to describe; MUST be a non-empty string

### Errors
- `ModuleNotFoundError` — raised if no module is registered under `module_id`

### Returns
- On success: `str`/`string` — Markdown-formatted description string; if the module defines a `describe()` method, its return value is used verbatim; otherwise the registry auto-generates a description from the module's `ModuleDescriptor` (title, description, tags, parameter list, documentation)

### Properties
- async: false (synchronous in all languages)
- thread_safe: true (reads from the Registry under its RLock)
- pure: true (read-only; no state mutation)
- idempotent: true

## Contract: APCore.use / APCore.use_middleware

### Inputs
- `middleware` (Middleware instance, required) — a class-based middleware object implementing the `Middleware` protocol; MUST NOT be null/None; `priority` attribute (int, 0–1000) controls insertion order — higher priority runs first; equal priorities preserve registration order

### Errors
- `ValueError` (Python) / `RangeError` (TypeScript) / `Err(ModuleError)` with `GENERAL_INVALID_INPUT` (Rust) — if `middleware.priority` exceeds 1000. Each SDK uses its idiomatic invalid-argument signal for this caller-side misconfiguration (A-D-017); the rejection itself is enforced in all three.

### Returns
- On success: `self`/`APCore` — returns the client instance for method chaining (e.g. `client.use(a).use(b).use(c)`)

### Properties
- async: false (synchronous in all languages)
- thread_safe: true (MiddlewareManager holds an internal lock during insertion)
- pure: false (mutates the executor's middleware chain)
- idempotent: false (adding the same middleware instance twice inserts it twice, running it twice per execution)

!!! note "Rust keyword conflict"
    In Rust, this method is named `use_middleware()` because `use` is a reserved keyword. Python and TypeScript expose it as `.use()`.

## Contract: APCore.use_before

### Inputs
- `callback` (callable/Function, required) — a sync or async function invoked before module execution; signature: `(context: Context) -> None` (Python) / `(ctx: Context) => void | Promise<void>` (TypeScript); MUST NOT be null/None; the callback is wrapped in a `BeforeMiddleware` adapter with default priority 0

### Errors
- No errors raised during registration; errors raised inside `callback` at execution time propagate through the middleware chain

### Returns
- On success: `self`/`APCore` — returns the client instance for method chaining

### Properties
- async: false (the registration call is synchronous; the callback itself may be sync or async)
- thread_safe: true (delegates to MiddlewareManager which holds an internal lock)
- pure: false (mutates the executor's middleware chain by inserting a wrapped `BeforeMiddleware`)
- idempotent: false (registering the same callback twice inserts two independent `BeforeMiddleware` wrappers)

## Contract: APCore.use_after

### Inputs
- `callback` (callable/Function, required) — a sync or async function invoked after module execution; signature: `(context: Context) -> None` (Python) / `(ctx: Context) => void | Promise<void>` (TypeScript); MUST NOT be null/None; the callback is wrapped in an `AfterMiddleware` adapter with default priority 0

### Errors
- No errors raised during registration; errors raised inside `callback` at execution time propagate through the middleware chain

### Returns
- On success: `self`/`APCore` — returns the client instance for method chaining

### Properties
- async: false (the registration call is synchronous; the callback itself may be sync or async)
- thread_safe: true (delegates to MiddlewareManager which holds an internal lock)
- pure: false (mutates the executor's middleware chain by inserting a wrapped `AfterMiddleware`)
- idempotent: false (registering the same callback twice inserts two independent `AfterMiddleware` wrappers)

## Contract: APCore.remove

### Inputs
- `middleware` (Middleware instance, required) — the exact middleware object to remove; identity comparison (`is`) is used, not equality (`==`); pass the same object reference returned or stored when originally calling `use()`, `use_before()`, or `use_after()`

!!! info "Rust removes by handle, for a reason that is not about trait objects (spec v1.21.0)"

    Identity removal needs the caller to still hold the thing it registered.
    apcore-python and apcore-typescript do: `use()` borrows the object and the
    caller keeps its reference. Rust's `use_middleware` **consumes** the `Box`,
    so by the time a caller wants to remove one it has no pointer left to
    compare against — the obstacle is ownership, not comparison. (`Arc::ptr_eq`
    has ignored vtable metadata since Rust 1.76, below the crate's MSRV, so a
    doc comment claiming trait objects cannot be compared by identity was
    wrong, and wrong in the direction that made the gap look unfixable.)

    So `use_middleware` returns a `MiddlewareHandle`, and `remove_handle(handle)`
    removes exactly that registration. That is the identity this contract
    requires, in the form the language allows, and it mirrors
    `EventEmitter::subscribe` → `unsubscribe_handle`, which exists for the same
    reason on the event bus.

    `APCore::remove(name)` and `remove_middleware(&dyn Middleware)` remain, and
    both resolve by `name()` — they remove the FIRST match in pipeline order.
    That is not equivalent to identity removal: duplicate registration only
    warns and always succeeds, so two instances answering one name is a
    reachable state, and the name-based forms then drop whichever comes first
    rather than the one the caller meant.

    An SDK that cannot take the middleware object back **MUST** provide a token
    issued at registration that removes exactly one registration, and **MUST
    NOT** present a name-based removal as satisfying this contract.

### Errors
- No errors raised under normal operation

### Returns
- On success: `bool` — `True` if the middleware was found by identity and removed; `False` if no matching instance was present in the chain

### Properties
- async: false (synchronous in all languages)
- thread_safe: true (MiddlewareManager holds an internal lock during removal)
- pure: false (mutates the executor's middleware chain)
- idempotent: true (calling `remove()` on a middleware not in the chain returns `False` without error; calling it again after a successful removal also returns `False` safely)
