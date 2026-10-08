---
description: "APCore facade over Registry, Executor, Config: zero- or full-config init, module registration/discovery, sync/async/streaming calls, middleware, events, runtime module toggling."
---

# APCore Unified Client

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §12 SDK Implementation Guide.


## Overview

`APCore` is the recommended entry point for application code. It owns a Registry and an Executor, optionally a Config, and — when configuration asks for them — the system modules, the event bus and config-driven tracing, so callers do not wire these components together by hand. It supports zero-config construction for prototypes and tests, and config-driven construction for production.

## Requirements

### Unified Facade
- Provide a single `APCore` class that wraps a Registry and an Executor, and optionally a Config and a MetricsCollector.
- Accept configuration as a `Config` object (`Config.load(path)` loads one from a file). Rust additionally offers `APCore::from_path(path)`.
- If no Registry or Executor is supplied, the client **MUST** create them. When an Executor is supplied, its Registry is used and a separately supplied `registry` is ignored.
- If a Config with `sys_modules.enabled: true` is supplied, the client **MUST** register the system modules and their middleware (see [Construction with a Config](#construction-with-a-config)).

### Module Lifecycle
- Support function registration (`client.module()`), direct registration (`client.register()`), and discovery (`client.discover()`).
- Support module listing with optional tag and prefix filtering.
- Support module description generation (`client.describe()`) for AI/LLM tool discovery.
- There is no `start()` / `stop()` lifecycle: the client needs no phase beyond construction (D-09). Python additionally provides `close()`, which releases the Executor's cached synchronous event loop; it is idempotent and the client stays usable afterwards.

### Execution
- Provide synchronous (`call()`, Python only), asynchronous (`call_async()` / `call()`), and streaming (`stream()`) execution that delegates to the Executor.
- Provide a non-destructive preflight, `validate()`, that runs pipeline Steps 1–5 and 7 — no middleware, no module execution — plus the module's optional `preflight()` hook ([PROTOCOL_SPEC §12.8](../spec/protocol-spec.md#128-executorvalidate-cross-language-implementation-guide)).

### Middleware
- Support middleware registration: `use()` (Rust: `use_middleware()`), `use_before()`, `use_after()`.
- Support removal of exactly one registration (by identity in Python/TypeScript, by handle in Rust).

### Event System
- When system-module events are enabled, expose `on()` / `off()` for subscribing to framework events.
- The `events` property **MUST** expose the underlying `EventEmitter`, or `None`/`null` when events are not configured.

### Module Control
- When system modules are enabled, expose `disable()` / `enable()` for runtime module toggling.
- These methods **MUST** delegate to the `system.control.toggle_feature` module, so they run through the full pipeline (ACL, approval, middleware, events).

### Global Singleton
- Python provides module-level functions (`apcore.call()`, `apcore.module()`, …) backed by a default client. TypeScript and Rust have no global singleton — explicit instances only.

## Technical Design

### Initialization Modes

| Mode | How | System modules |
|------|-----|----------------|
| Zero-config | `APCore()` / `new APCore()` / `APCore::new()` | No |
| With a Config | `APCore(config=Config.load(...))` / `new APCore({ config })` / `APCore::with_config(config)` or `APCore::from_path(path)` | If `sys_modules.enabled` |
| Pre-built components | Pass `registry` and/or `executor` (Rust: `with_components`, `with_options`) | If the Config enables them |

### Construction with a Config

When a Config is supplied and the client built the Executor itself:

1. **ACL discovery** — the ACL file at `acl.root` is loaded and attached. A missing file attaches **no** ACL; it does not synthesize a default-deny one (D-64).
2. **Tracing** — when `observability.tracing.enabled: true`, a `TracingMiddleware` built from `observability.tracing.*` is installed ([§10.1.1](../spec/protocol-spec.md#1011-tracing-from-configuration-observabilitytracing)).

Both steps are skipped for a caller-supplied Executor, whose wiring is respected as-is. Python and TypeScript also accept a `policy` (`ExecutionPolicy`) that is applied only to an Executor the client builds.

Then, when `sys_modules.enabled: true`:

1. The read-only system modules are registered: `system.health.*`, `system.manifest.*`, `system.usage.*`.
2. `ErrorHistoryMiddleware` and `UsageMiddleware` are added. The supplied `metrics_collector` is handed to the system modules (Python creates one when none is supplied).
3. When `sys_modules.events.enabled: true`:
    - an `EventEmitter` is created and exposed as `client.events`;
    - `PlatformNotifyMiddleware` is added (threshold events from `sys_modules.events.thresholds.*`);
    - subscribers are instantiated from `sys_modules.events.subscribers` (see [Event System](./event-system.md));
    - the `system.control.*` modules are registered unless `sys_modules.control.enabled: false`. `disable()` / `enable()` depend on them.

System-module registration failures are logged and do not fail construction.

### Method Summary

| Category | Method | Returns | Notes |
|----------|--------|---------|-------|
| **Registration** | `module(...)` | decorator (Python) / `FunctionModule` (TS) / `&mut Self` (Rust) | Register a function as a module |
| | `register(module_id, module)` | None | Direct registration |
| | `discover()` | int | Discover and register modules from `extensions.*` roots |
| **Execution** | `call(module_id, inputs?, context?, version_hint?)` | output | Python: synchronous. TypeScript/Rust: async |
| | `call_async(...)` | output | Python coroutine; TypeScript `callAsync` is an alias of `call`; Rust has none (`call` is async) |
| | `stream(module_id, inputs?, context?, version_hint?)` | async iterator / `Stream` | Chunked output |
| | `validate(module_id, inputs?, context?)` | `PreflightResult` | Non-destructive preflight |
| **Inspection** | `list_modules(tags?, prefix?)` | sorted list of IDs | |
| | `describe(module_id)` | Markdown string | For AI/LLM tool discovery |
| **Middleware** | `use(middleware)` | self | Rust: `use_middleware()` (`use` is a keyword) |
| | `use_before(callback)` | self | See [Contract: APCore.use_before](#contract-apcoreuse_before) |
| | `use_after(callback)` | self | See [Contract: APCore.use_after](#contract-apcoreuse_after) |
| | `remove(middleware)` | bool | Rust: `remove_handle(handle)` |
| **Events** | `on(event_type, handler)` | subscriber handle | Exact event-type match |
| | `off(subscriber)` | None | |
| **Control** | `disable(module_id, reason?)` / `enable(module_id, reason?)` | dict | Via `system.control.toggle_feature` |
| **Properties** | `registry`, `executor`, `events` | | Rust: accessor methods |

!!! note "Sync/async"
    Python `call()`, `validate()`, `disable()` and `enable()` are synchronous; use `call_async()` / `stream()` inside `async def` code. In TypeScript and Rust these methods return a `Promise` / `Future` and **MUST** be awaited.

### Callback Subscribers

`on()` wraps the handler in a lightweight subscriber that delivers an event only when its `event_type` **equals** the requested type — there is no glob matching. To receive several types, call `on()` once per type; to match a pattern, subscribe an `EventSubscriber` with an `event_pattern` directly on `client.events` (see [Event System](./event-system.md#subscribing)). Python and TypeScript accept sync or async handlers; Rust takes a `Fn(&ApCoreEvent)` closure.

### Error Behavior

| Condition | Error |
|-----------|-------|
| Config file missing (`Config.load`) | `ConfigNotFoundError` (`CONFIG_NOT_FOUND`); Rust: `ModuleError` with `ErrorCode::ConfigNotFound` |
| Config file invalid (`Config.load`) | `ConfigError` (`CONFIG_INVALID`); Rust: `ErrorCode::ConfigInvalid` |
| `on()` / `off()` without events enabled | `SysModulesDisabledError` (`SYS_MODULES_DISABLED`); Rust: `ErrorCode::SysModulesDisabled` |
| `disable()` / `enable()` without system modules | `SysModulesDisabledError` (`SYS_MODULES_DISABLED`); Rust: `ErrorCode::SysModulesDisabled` |

### Language-Specific Adaptations

**TypeScript:**

| Spec method | TypeScript | Notes |
|-------------|------------|-------|
| `call_async()` | `callAsync()` | Alias of `call()` |
| `use_before()` / `use_after()` | `useBefore()` / `useAfter()` | camelCase |
| `list_modules(tags, prefix)` | `listModules({ tags, prefix })` | Options object |
| Constructor | `new APCore({ registry, executor, config, metricsCollector, policy, toggleState })` | Options object |

**Rust:**

| Spec method | Rust | Notes |
|-------------|------|-------|
| `use()` | `use_middleware(Box<dyn Middleware>)` | Returns `Result<&Self, ModuleError>`; `use_middleware_handle()` returns a `MiddlewareHandle` instead |
| `use_before()` | `use_before(Box<dyn BeforeMiddleware>)` | Takes a `BeforeMiddleware` trait object, not a closure; returns `Result<&Self, ModuleError>` |
| `use_after()` | `use_after(Box<dyn AfterMiddleware>)` | Same shape as `use_before` |
| `remove()` | `remove_handle(MiddlewareHandle) -> bool` | See [Contract: APCore.remove](#contract-apcoreremove) |
| `on()` | `on(&mut self, event_type, impl Fn(&ApCoreEvent))` | Returns `Result<String, ModuleError>` — a subscriber ID. `on_subscriber()` takes a `Box<dyn EventSubscriber>` |
| `off()` | `off(&mut self, &str) -> Result<bool, ModuleError>` | `bool` reports whether the ID was found; `off_by_type()` removes every handler for a type |
| `stream()` | `stream()` | Returns `Pin<Box<dyn Stream<Item = Result<Value, ModuleError>>>>` — poll it with `StreamExt::next` |
| `validate()` | `validate()` | Returns `Result<PreflightResult, ModuleError>` |
| `disable()` / `enable()` | `async fn disable(&self, &str, Option<&str>)` | Returns `Result<Value, ModuleError>` |
| `module()` | `module(module_id, description, input_schema, output_schema, documentation, tags, version, metadata, examples, display, handler)` | Builds a `FunctionModule` from explicit metadata and a handler closure; `impl Module` + `register()` covers anything more |
| `policy` constructor input | not accepted | Attach the policy to an `Executor` and pass it via `APCore::with_options(None, Some(executor), …)` |
| `events` / `registry` / `executor` | `events()` / `registry()` / `executor()` | Accessor methods |
| Constructors | `new()`, `with_config(config)`, `from_path(path)?`, `with_components(registry, config)`, `with_options(registry, executor, config, metrics_collector)` | `from_path` is the only fallible one |

**Rust-only methods:**

| Method | Purpose |
|--------|---------|
| `with_components(registry, config)` | Build a client around a pre-configured Registry |
| `with_options(registry, executor, config, metrics_collector)` | Full constructor with every optional component |
| `reload()` | Re-read the `Config` from its source file. Does **not** re-discover modules — call `discover()` for that |

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
    import { Type } from "@sinclair/typebox";
    import { APCore } from "apcore-js";

    const client = new APCore();

    client.module({
        id: "math.add",
        description: "Add two numbers",
        inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
        outputSchema: Type.Object({ sum: Type.Number() }),
        execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
    });

    const result = await client.call("math.add", { a: 10, b: 5 });
    console.log(result); // { sum: 15 }
    ```
=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use apcore::APCore;
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

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        client.register("math.add", Box::new(AddModule))?;
        let result = client.call("math.add", json!({"a": 10, "b": 5}), None, None).await?;
        println!("{result}"); // {"sum":15}
        Ok(())
    }
    ```

### Production Setup

Assumes an `apcore.yaml` with `sys_modules.enabled: true` and `sys_modules.events.enabled: true`.

=== "Python"
    ```python
    from apcore import APCore
    from apcore.config import Config

    client = APCore(config=Config.load("apcore.yaml"))

    # System modules, the event bus and configured tracing are wired at construction.
    sub = client.on(
        "apcore.health.error_threshold_exceeded",
        lambda event: print(f"alert: {event.module_id} {event.data}"),
    )

    # Runtime control (synchronous in Python)
    client.disable("risky.module", reason="Investigating issue")
    client.off(sub)
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config } from "apcore-js";

    const client = new APCore({ config: Config.load("apcore.yaml") });

    // System modules, the event bus and configured tracing are wired at construction.
    const sub = client.on("apcore.health.error_threshold_exceeded", (event) => {
        console.log(`alert: ${event.moduleId}`, event.data);
    });

    // Runtime control
    await client.disable("risky.module", "Investigating issue");
    client.off(sub);
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::APCore;

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let mut client = APCore::from_path("apcore.yaml")?;

        // System modules, the event bus and configured tracing are wired at construction.
        let sub = client.on("apcore.health.error_threshold_exceeded", |event| {
            println!("alert: {:?} {}", event.module_id, event.data);
        })?;

        // Runtime control
        client.disable("risky.module", Some("Investigating issue")).await?;
        client.off(&sub)?;
        Ok(())
    }
    ```

## Dependencies

- **Registry** — Module registration, discovery, and lookup.
- **Executor** — Module execution, middleware, ACL, and approval.
- **Config Bus** — Configuration loading and system module setup.
- **Event System** — Event emission and subscription (optional).
- **System Modules** — Health, manifest, usage, and control modules (optional).
- **Observability** — [Tracing from configuration](./observability.md#tracing-from-configuration), [metrics and usage](./metrics-and-usage.md) (optional).

??? info "Python SDK reference"
    Not a protocol requirement — the Python SDK's source layout for users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `src/apcore/client.py` | `APCore`, `_CallbackSubscriber` |
    | `src/apcore/__init__.py` | Module-level singleton functions |

## Testing Strategy

- **Zero-config tests** verify that `APCore()` creates a working Registry and Executor.
- **Config tests** verify that a `Config` applies ACL discovery, configured tracing, and — with `sys_modules.enabled` — registers the system modules.
- **Registration tests** verify that `client.module()` / `register()` make modules callable, and that a supplied Executor's Registry is the one the client writes to.
- **Execution tests** verify that `call()`, `call_async()`, `stream()` and `validate()` delegate to the Executor.
- **Middleware tests** verify `use()`, `use_before()`, `use_after()` and removal.
- **Event tests** verify `on()` / `off()` and the `SYS_MODULES_DISABLED` error when events are off.
- **Control tests** verify that `disable()` / `enable()` route through `system.control.toggle_feature` and raise `SYS_MODULES_DISABLED` without system modules.

## Contract: APCore.call

### Inputs
- `module_id` (str/string/&str, required) — target module ID; validated against `MODULE_ID_PATTERN` and the length limit before the pipeline starts
- `inputs` (dict/object/Value, optional) — validated against the module's `input_schema`; absent is treated as `{}`
- `context` (Context, optional) — execution context; created fresh when absent
- `version_hint` (str/string/&str, optional) — preferred version constraint. Python resolves it in module lookup; TypeScript and Rust accept it and resolve the latest registered version

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — `module_id` is empty, malformed or over-length
- `ModuleNotFoundError(code=MODULE_NOT_FOUND)` — no module registered under `module_id`
- `SchemaValidationError(code=SCHEMA_VALIDATION_ERROR)` — `inputs` fails the module's `input_schema`
- Pipeline errors (`ACL_DENIED`, `APPROVAL_*`, `CALL_DEPTH_EXCEEDED`, `MODULE_TIMEOUT`, …) and errors raised by the module propagate as their typed error — see [Contract: Executor.call](./core-executor.md#contract-executorcall)

### Returns
- On success: `dict`/`Record<string, unknown>`/`serde_json::Value` — the module's validated output (after-middleware applied)

### Properties
- async: synchronous in Python (`call_async` is the coroutine form); async in TypeScript and Rust
- thread_safe: true
- pure: false (spans, metrics, middleware hooks, events)
- idempotent: false (module `execute` is not guaranteed idempotent)

## Contract: APCore.call_async

### Inputs
- Identical to `APCore.call`.

### Errors
- Identical to `APCore.call`; this surface adds none of its own.

### Returns
- On success: the module's validated output, identical to `APCore.call`.

### Properties
- async: true
- thread_safe: true
- pure: false
- idempotent: false

Python `call_async()` is a coroutine distinct from the blocking `call()`. TypeScript `callAsync()` is an alias of `call()`. Rust has no `call_async` because `APCore::call` is already `async`. Implementations **MUST NOT** give `call_async` behaviour that differs from `call` beyond the calling convention.

## Contract: APCore.on

### Inputs
- `event_type` (str/string/&str, required) — canonical event type (e.g. `"apcore.registry.module_registered"`). Matched by **exact equality**; a glob such as `"apcore.registry.*"` is compared literally and matches no framework event.
- `handler` (callable/function/closure, required) — receives each matching `ApCoreEvent`. Python and TypeScript accept sync or async handlers; Rust takes `impl Fn(&ApCoreEvent) + Send + Sync + 'static`.

### Errors
- `SysModulesDisabledError(code=SYS_MODULES_DISABLED)` — events are not enabled (`events` is `None`/`null`). Rust: `ModuleError` with `ErrorCode::SysModulesDisabled`.

### Returns
- Python/TypeScript: the created `EventSubscriber` — pass it to `off()`.
- Rust: `Result<String, ModuleError>` — the subscriber ID.

### Properties
- async: false
- thread_safe: true
- pure: false (adds a subscriber to the emitter)
- idempotent: false (registering the same handler twice creates two subscriptions)

## Contract: APCore.off

### Inputs
- `subscriber` — the value `on()` returned (Rust: the subscriber ID `&str`)

### Errors
- `SysModulesDisabledError(code=SYS_MODULES_DISABLED)` — events are not enabled (same guard as `on()`).

### Returns
- Python/TypeScript: None/void. Rust: `Result<bool, ModuleError>` — `true` when a subscriber with that ID was removed.

### Properties
- async: false
- thread_safe: true
- pure: false (removes a subscriber from the emitter)
- idempotent: true (removing an absent subscriber is a no-op)

## Contract: APCore.stream

### Inputs
- `module_id` (str/string/&str, required) — validated as for `call()`
- `inputs` (dict/object/Value, optional) — `None`/`null` is treated as `{}`
- `context` (Context, optional) — created when absent
- `version_hint` (str/string/&str, optional)

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)`, `ModuleNotFoundError`, `SchemaValidationError` — as for `call()`, raised before the first chunk
- `ExecutionCancelledError` — propagated if the context's cancel token fires mid-stream
- Errors raised by the module's `stream()` run the `on_error` chain; a recovery value is yielded as the last chunk (a `RetrySignal` is not honoured mid-stream)
- Output validation of the accumulated result runs after the last chunk. Chunks already delivered cannot be recalled, so a failure there is logged and published as `apcore.stream.post_validation_failed` instead of being raised (cancellation is still raised). See [Streaming](./streaming.md).

### Returns
- On success: an async iterator (Python async generator, TypeScript `AsyncGenerator`, Rust `Stream`) yielding output chunks. A module without `stream()` yields its `execute()` output as a single chunk.

### Properties
- async: true
- thread_safe: true
- pure: false
- idempotent: false

## Contract: APCore.validate

### Inputs
- `module_id` (str/string/&str, required)
- `inputs` (dict/object/Value, optional) — `None`/`null` is treated as `{}`
- `context` (Context, optional) — used for call-chain and ACL checks; created when absent

### Errors
- None. Every failure — an empty or malformed `module_id` included — is reported in the returned `PreflightResult`. (`call()` raises `INVALID_MODULE_ID` for the same input; `validate()` reports it.) Rust returns `Result<PreflightResult, ModuleError>`, whose `Err` arm is not used for check failures.

### Returns
- `PreflightResult` with:
  - `valid: bool` — `true` only when every check passed
  - `checks: list[PreflightCheckResult]` — one entry per check that ran. `module_id` comes first; a failure there, or an `executor_binding` conflict, ends the preflight. The pipeline runs Steps 1–5 and 7 in dry-run mode and each step that ran contributes a check (`context`, `call_chain`, `module_lookup`, `acl`, `schema`, …). `module_preflight` and `module_preview` are appended when the module provides those hooks and the `acl` check did not fail ([§12.8.5.1](../spec/protocol-spec.md#12851-module-level-preflight-check-7)).
  - `requires_approval: bool` — the governance-effective approval requirement the Step 5 gate would enforce; reported, never enforced (no `ApprovalHandler` is invoked)
  - `errors` — the failed checks' `error` values

### Properties
- async: synchronous in Python; async in TypeScript and Rust
- thread_safe: true
- pure: false (runs module-authored `preflight()` / `preview()` hooks)
- idempotent: true (no state mutation)

## Contract: APCore.disable

### Inputs
- `module_id` (str/string/&str, required) — module to disable; passed to `system.control.toggle_feature`
- `reason` (str/string, optional) — audit reason; defaults to `"Disabled via APCore client"` (Rust: `Option<&str>`)

### Errors
- `SysModulesDisabledError(code=SYS_MODULES_DISABLED)` — system modules are not enabled. Rust: `ErrorCode::SysModulesDisabled`.
- Errors from `system.control.toggle_feature` propagate unchanged — for example `MODULE_NOT_FOUND` for an unregistered `module_id`, or for `toggle_feature` itself when events or `sys_modules.control.enabled` are off.

### Returns
- The `toggle_feature` result: at least `success`, `module_id`, `enabled` (`false`).

### Properties
- async: synchronous in Python; async in TypeScript and Rust
- thread_safe: true
- pure: false (mutates this client's toggle state; emits `apcore.module.toggled`)
- idempotent: true

## Contract: APCore.enable

### Inputs
- `module_id` (str/string/&str, required)
- `reason` (str/string, optional) — defaults to `"Enabled via APCore client"`

### Errors
- As for `APCore.disable`.

### Returns
- The `toggle_feature` result: at least `success`, `module_id`, `enabled` (`true`).

### Properties
- async: synchronous in Python; async in TypeScript and Rust
- thread_safe: true
- pure: false
- idempotent: true

## Contract: APCore.__init__

### Inputs
- `registry` (Registry, optional) — ignored when `executor` is supplied; a Registry built from `config` otherwise
- `executor` (Executor, optional) — used as-is; config-driven ACL discovery and tracing are skipped for it
- `config` (Config, optional) — when absent the client runs zero-config, with no system modules
- `metrics_collector` (MetricsCollector, optional) — handed to the system modules
- `policy` (ExecutionPolicy, optional; Python and TypeScript) — applied to an Executor the client builds

### Errors
- None from construction itself. ACL discovery and system-module registration failures are logged and the client continues without that component.

### Returns
- A fully initialized `APCore` instance.

### Properties
- async: false
- thread_safe: false (do not share a partially constructed instance)
- pure: false (builds components, registers system modules, installs middleware)
- idempotent: false

## Contract: APCore.module

### Inputs
- `id` (str/string, optional in Python) — module ID; derived from the function when absent
- `description`, `documentation`, `annotations`, `tags`, `version` (default `"1.0.0"`), `metadata`, `display`, `examples` — descriptor fields
- TypeScript additionally requires `inputSchema`, `outputSchema` (TypeBox) and `execute`; Rust takes explicit schemas and a handler closure

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — `id` is malformed, over-length or uses a reserved first segment
- `DUPLICATE_MODULE_ID` — `id` is already registered (`InvalidInputError` in Python, `DuplicateModuleIdError` in TypeScript, `ErrorCode::DuplicateModuleId` in Rust)

### Returns
- Python: the decorated function, unchanged. TypeScript: the registered `FunctionModule`. Rust: `Result<&mut Self, ModuleError>`.

### Properties
- async: false
- thread_safe: true
- pure: false (registers a module)
- idempotent: false (a second registration under the same ID fails)

## Contract: APCore.register

### Inputs
- `module_id` (str/string/&str, required) — must match `MODULE_ID_PATTERN`, be at most 192 characters, and not use a reserved first segment
- `module` (Module instance, required)

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — malformed, over-length or reserved `module_id`
- `DUPLICATE_MODULE_ID` — `module_id` is already registered (error class as for `APCore.module`)
- An exception raised by the module's `on_load()` propagates after the partial registration is rolled back

### Returns
- Python: `None`. TypeScript: `Promise<void>` that settles after an async `onLoad` has run (synchronous validation errors are still thrown synchronously). Rust: `Result<(), ModuleError>`.

### Properties
- async: false in Python and Rust; TypeScript returns a promise (see Returns)
- thread_safe: true
- pure: false (mutates the registry, fires `register` callbacks)
- idempotent: false

## Contract: APCore.discover

### Inputs
- None — roots come from the client's Config (`extensions.root` / `extensions.roots`, `extensions.max_depth`, `id_map.overrides`), or the default root when no Config was given.

### Errors
- `CircularDependencyError` — circular inter-module dependencies in the discovered set
- `ConfigNotFoundError(code=CONFIG_NOT_FOUND)` — a configured extension root does not exist
- Per-file failures (import, validation, `on_load()`) are logged and skipped

### Returns
- The number of modules registered by this pass.

### Properties
- async: synchronous in Python; async in TypeScript (ESM entry points are loaded with dynamic `import()`) and Rust (`Result<usize, ModuleError>`; `Ok(0)` when no discoverer is configured). The outcome — count, errors, skipped files — is the same in all three.
- thread_safe: true
- pure: false (loads code, mutates the registry)
- idempotent: false (re-discovering already-registered modules reports duplicates)

## Contract: APCore.list_modules

### Inputs
- `tags` (list of strings, optional) — only modules carrying **all** listed tags
- `prefix` (string, optional) — only IDs starting with this prefix

### Errors
- None

### Returns
- Alphabetically sorted list of matching module IDs.

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true

## Contract: APCore.describe

### Inputs
- `module_id` (str/string/&str, required)

### Errors
- `ModuleNotFoundError(code=MODULE_NOT_FOUND)` — no module registered under `module_id`

### Returns
- A Markdown description: the module's own `describe()` output when it defines one, otherwise generated from its descriptor.

### Properties
- async: false
- thread_safe: true
- pure: true
- idempotent: true

## Contract: APCore.use / APCore.use_middleware

### Inputs
- `middleware` (Middleware instance, required) — its `priority` (0–1000, default 100) orders the chain; higher runs first, equal priorities keep registration order. See [Middleware System](./middleware-system.md).

### Errors
- Priority above 1000: `ValueError` (Python), `RangeError` (TypeScript), `ModuleError` with `GENERAL_INVALID_INPUT` (Rust)

### Returns
- `self` for chaining (Rust: `Result<&Self, ModuleError>`).

### Properties
- async: false
- thread_safe: true
- pure: false (mutates the middleware chain)
- idempotent: false (registering the same instance twice runs it twice; duplicate registration warns but succeeds)

## Contract: APCore.use_before

### Inputs
- Python/TypeScript: `callback(module_id, inputs, context)` — returns replacement inputs or `None`/`null`. It is wrapped in a `BeforeMiddleware` with priority 100.
- Rust: `Box<dyn BeforeMiddleware>` — a trait object implementing `name()` and `async fn before(&self, module_id, inputs: Value, ctx) -> Result<Option<Value>, ModuleError>`.

### Errors
- As for `APCore.use` — registration goes through the same manager.
- Errors raised by the callback at execution time abort the call and run the `on_error` chain.

### Returns
- `self` for chaining (Rust: `Result<&Self, ModuleError>`).

### Properties
- async: false (the callback itself may be async in Python)
- thread_safe: true
- pure: false
- idempotent: false

## Contract: APCore.use_after

### Inputs
- Python/TypeScript: `callback(module_id, inputs, output, context)` — returns replacement output or `None`/`null`. Wrapped in an `AfterMiddleware` with priority 100.
- Rust: `Box<dyn AfterMiddleware>` — `name()` and `async fn after(&self, module_id, inputs: Value, output: Value, ctx) -> Result<Option<Value>, ModuleError>`.

### Errors
- As for `APCore.use_before`.

### Returns
- `self` for chaining (Rust: `Result<&Self, ModuleError>`).

### Properties
- async: false
- thread_safe: true
- pure: false
- idempotent: false

## Contract: APCore.remove

### Inputs
- Python/TypeScript: `middleware` — the object passed to `use()`; compared by identity (`is` / `===`).
- Rust: a `MiddlewareHandle` obtained from `use_middleware_handle()`, passed to `remove_handle()`. `use_middleware` consumes the `Box`, so the handle is the identity the caller keeps.

An SDK that cannot take the middleware object back **MUST** issue a token at registration that removes exactly one registration, and **MUST NOT** present name-based removal as satisfying this contract. Rust's `remove(name)` / `remove_middleware(&dyn Middleware)` match by `name()` and drop the first match in pipeline order.

### Errors
- None

### Returns
- `true` when the registration was found and removed; `false` otherwise.

### Properties
- async: false
- thread_safe: true
- pure: false
- idempotent: true

## Contract: APCore.with_components

**SDK scope:** Rust only.

### Inputs
- `registry` (`Registry`, required)
- `config` (`Config`, required)

### Errors
- None — it delegates to `with_options(Some(registry), None, Some(config), None)`.

### Returns
- A client built around `registry`, with a new Executor over it.

### Properties
- async: false
- thread_safe: false
- pure: false (as `with_options`)
- idempotent: false

## Contract: APCore.with_options

**SDK scope:** Rust only. Python and TypeScript accept the same options on the constructor.

### Inputs
- `registry` (`Option<Registry>`) — ignored when `executor` is given
- `executor` (`Option<Executor>`) — used as-is; config-driven ACL discovery, tracing and event-emitter wiring are skipped for it
- `config` (`Option<Config>`) — `Config::default()` when absent
- `metrics_collector` (`Option<MetricsCollector>`) — used when system modules are enabled

### Errors
- None — ACL-discovery, tracing and system-module failures are logged and the component is left out.

### Returns
- A fully initialized `APCore`.

### Properties
- async: false
- thread_safe: false
- pure: false
- idempotent: false

## Contract: APCore.reload

**SDK scope:** Rust only — Python and TypeScript reload through `Config` itself.

### Inputs
- None

### Errors
- `ModuleError(code=RELOAD_FAILED)` — the Config was not loaded from a file
- `ModuleError(code=MODULE_RELOAD_CONFLICT)` — the Config changed concurrently during the reload
- Any error `Config::load` raises for the file

### Returns
- `Result<(), ModuleError>`

### Properties
- async: false
- thread_safe: false (takes `&mut self`)
- pure: false (replaces the in-memory Config; mounted namespaces are replayed)
- idempotent: true while the file is unchanged

`reload()` does not re-discover modules; call `discover()` afterwards for that.
