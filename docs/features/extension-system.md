---
description: "ExtensionManager wiring named extension points (discoverer, middleware, acl, span_exporter, module_validator, approval_handler) into Registry and Executor, with cardinality/type checks."
---

# Extension System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §11 Extension Mechanism.


## Overview

The Extension System provides a pluggable architecture for customizing and extending the apcore framework without modifying core source code. It defines a set of named extension points — each accepting one or multiple implementations — and an `ExtensionManager` that wires registered extensions into the Registry and Executor at startup. This enables third-party libraries and application code to inject custom discoverers, middleware, ACL providers, span exporters, module validators, and approval handlers.

## Requirements

### Extension Points
- Define a fixed set of built-in extension points, each with a name, expected type, description, and cardinality (`multiple: true/false`).
- For single-cardinality points, only one extension can be registered at a time; re-registration replaces the previous one.
- For multi-cardinality points, multiple extensions can coexist and are all applied.

### ExtensionManager
- Provide an `ExtensionManager` class that manages extension registration, retrieval, and wiring.
- Registration **MUST** perform type checking against the extension point's expected type.
- The `apply()` method **MUST** wire all registered extensions into the provided Registry and Executor instances.
- Span exporters registered as extensions **MUST** be composed into a single composite exporter when multiple are present.

### Type Safety
- Each extension point declares an expected type (protocol/interface). Registration of a value that does not satisfy the expected type **MUST** raise an error.
- SDKs use the language's own mechanism: `isinstance` against a runtime-checkable Protocol in Python, duck-type guards in TypeScript, and the `ExtensionKind` enum variant in Rust (the variant must match the point name).

## Technical Design

### Built-in Extension Points

| Name | Multiple | Expected Type | Description |
|------|----------|--------------|-------------|
| `discoverer` | No | Discoverer protocol | Custom module discovery strategy |
| `middleware` | Yes | Middleware protocol | Execution middleware |
| `acl` | No | ACL protocol | Access control provider |
| `span_exporter` | Yes | SpanExporter protocol | Tracing span exporter |
| `module_validator` | No | ModuleValidator protocol | Custom module validation |
| `approval_handler` | No | ApprovalHandler protocol | Approval gate handler |

### ExtensionPoint

=== "Python"
    ```python
    from dataclasses import dataclass

    @dataclass
    class ExtensionPoint:
        name: str            # Slot name (e.g., "discoverer")
        extension_type: type # Required type/protocol
        description: str     # Human-readable purpose
        multiple: bool       # Whether multiple can be registered
    ```
=== "TypeScript"
    ```typescript
    interface ExtensionPoint {
        readonly name: string;
        readonly description: string;
        readonly multiple: boolean;
    }
    ```
=== "Rust"
    ```rust
    pub struct ExtensionPoint {
        pub name: String,
        pub description: String,
        pub multiple: bool,
    }
    ```

### ExtensionManager

=== "Python"
    ```python
    from apcore.extensions import ExtensionManager

    manager = ExtensionManager()

    # Register extensions
    manager.register("discoverer", my_discoverer)
    manager.register("middleware", logging_middleware)
    manager.register("middleware", metrics_middleware)
    manager.register("span_exporter", stdout_exporter)
    manager.register("span_exporter", otlp_exporter)
    manager.register("acl", my_acl)
    manager.register("approval_handler", my_approval_handler)
    manager.register("module_validator", my_validator)

    # Retrieve extensions
    discoverer = manager.get("discoverer")         # Single or None
    all_mw = manager.get_all("middleware")          # List of all registered

    # Unregister
    manager.unregister("middleware", logging_middleware)

    # List all extension points
    points = manager.list_points()  # List[ExtensionPoint]

    # Wire everything into registry and executor
    manager.apply(registry, executor)
    ```
=== "TypeScript"
    ```typescript
    import { ExtensionManager } from "apcore-js";

    const manager = new ExtensionManager();

    // Register extensions
    manager.register("discoverer", myDiscoverer);
    manager.register("middleware", loggingMiddleware);
    manager.register("middleware", metricsMiddleware);
    manager.register("span_exporter", stdoutExporter);
    manager.register("span_exporter", otlpExporter);
    manager.register("acl", myAcl);
    manager.register("approval_handler", myApprovalHandler);
    manager.register("module_validator", myValidator);

    // Retrieve extensions
    const discoverer = manager.get("discoverer");       // Single or null
    const allMw = manager.getAll("middleware");          // Array of all registered

    // Unregister
    manager.unregister("middleware", loggingMiddleware);

    // List all extension points
    const points = manager.listPoints(); // ExtensionPoint[]

    // Wire everything into registry and executor
    manager.apply(registry, executor);
    ```
=== "Rust"
    ```rust
    use std::sync::Arc;

    use apcore::{ExtensionKind, ExtensionManager};

    let mut manager = ExtensionManager::new();

    // Each extension is wrapped in the ExtensionKind variant for its point;
    // register() returns a handle usable with unregister_handle().
    manager.register("discoverer", ExtensionKind::Discoverer(Arc::new(my_discoverer)))?;
    manager.register("module_validator", ExtensionKind::ModuleValidator(Arc::new(my_validator)))?;
    let logging = manager.register("middleware", ExtensionKind::Middleware(Arc::new(logging_middleware)))?;
    manager.register("middleware", ExtensionKind::Middleware(Arc::new(metrics_middleware)))?;
    manager.register("span_exporter", ExtensionKind::SpanExporter(Arc::new(stdout_exporter)))?;
    manager.register("acl", ExtensionKind::Acl(Arc::new(my_acl)))?;
    manager.register("approval_handler", ExtensionKind::ApprovalHandler(Arc::new(my_approval_handler)))?;

    // Retrieve extensions
    let discoverer = manager.get("discoverer")?;        // Option<&ExtensionKind>
    let all_mw = manager.get_all("middleware")?;        // &[ExtensionKind]

    // Unregister by handle
    manager.unregister_handle(logging);

    // Wire everything into registry and executor
    manager.apply(&registry, &mut executor)?;
    ```

    The placeholders (`my_discoverer`, `logging_middleware`, …) stand for values implementing the corresponding trait (`Discoverer`, `Middleware`, `SpanExporter`, `ModuleValidator`, `ApprovalHandler`) or an `ACL`.

### Wiring Behavior (`apply`)

When `apply(registry, executor)` is called, the manager performs the following in order:

1. **Discoverer** → `registry.set_discoverer(ext)` — replaces the default discovery strategy.
2. **Module Validator** → `registry.set_validator(ext)` — replaces the default module validator.
3. **ACL** → `executor.set_acl(ext)` — replaces the executor's ACL provider.
4. **Approval Handler** → `executor.set_approval_handler(ext)` — replaces the executor's approval handler.
5. **Middleware** → `executor.use(mw)` for each registered middleware — appends to the middleware chain.
6. **Span Exporters** → Locates the `TracingMiddleware` already in the executor's middleware chain and sets its exporter:
   - If a single exporter is registered, it is set directly.
   - If multiple exporters are registered, they are wrapped in a composite exporter that delegates to all of them. A failure in one exporter is logged and does not affect the others.
   - If the chain has no `TracingMiddleware`, a warning is logged and the exporters are not applied — `apply` does not add one.

## Contract: ExtensionManager.register

### Inputs
- `point_name` (str/string/&str, required) — name of an existing extension point (e.g., `"discoverer"`, `"middleware"`); unknown names raise an error
- `extension` (Any/unknown/Box<dyn Trait>, required) — must satisfy the extension point's declared type; type checking is performed at registration time

### Errors
- `InvalidInputError` (`code=GENERAL_INVALID_INPUT`; Rust `Err(ModuleError)` with `ErrorCode::GeneralInvalidInput`) — `point_name` is not a registered extension point (D-108)
- `TypeError` (Python, TypeScript) / `Err(ModuleError)` with `ErrorCode::GeneralInvalidInput` (Rust: the `ExtensionKind` variant does not match the point) — `extension` does not satisfy the point's expected type

### Returns
- On success: void/None (Python, TypeScript); `ExtensionHandle` (Rust), accepted by `unregister_handle`. For single-cardinality points the new extension replaces any prior registration.

### Properties
- async: false
- thread_safe: false (do not call concurrently with `apply()` or other `register()` calls)
- pure: false (mutates internal extension store)
- idempotent: false for single-cardinality (replaces); accumulating for multi-cardinality (`middleware`, `span_exporter`)

## Contract: ExtensionManager.get

### Inputs
- `point_name` (str/string/&str, required) — name of a single-cardinality extension point

### Errors
- `InvalidInputError` (`code=GENERAL_INVALID_INPUT`) — `point_name` is not a registered extension point
- No error for a registered point that currently holds nothing; returns `None`/`null`/`None`. See [An unknown extension point is an error; an empty one is not](#an-unknown-extension-point-is-an-error-an-empty-one-is-not).

### Returns
- On success: the registered extension object, or `None`/`null`/`None`

### Properties
- async: false
- thread_safe: true
- pure: true

## Contract: ExtensionManager.get_all

### Inputs
- `point_name` (str/string/&str, required) — name of a multi-cardinality extension point

### Errors
- `InvalidInputError` (`code=GENERAL_INVALID_INPUT`) — `point_name` is not a registered extension point
- No error for a registered point that currently holds nothing; returns an empty collection. See ["An unknown extension point is an error; an empty one is not"](#an-unknown-extension-point-is-an-error-an-empty-one-is-not).

### Returns
- On success: `list` / `Array` / `Vec` of all registered extensions in registration order

### Properties
- async: false
- thread_safe: true
- pure: true

## Contract: ExtensionManager.unregister

### Inputs
- `point_name` (str/string/&str, required) — name of the extension point
- `extension` (Any/unknown/ref, required) — the exact extension object to remove (identity comparison)

### Errors
- `InvalidInputError` (`code=GENERAL_INVALID_INPUT`) — `point_name` is not a registered extension point
- No error if the point is registered and does not hold the given extension — that is a silent `false`. See [An unknown extension point is an error; an empty one is not](#an-unknown-extension-point-is-an-error-an-empty-one-is-not).

### Returns
- On success: `True`/`true`/`Ok(true)` when the extension was removed, `False`/`false`/`Ok(false)` when the point does not hold it

### Properties
- async: false
- thread_safe: false
- pure: false (mutates extension store)

## An unknown extension point is an error; an empty one is not

`get`, `get_all` and `unregister` **MUST** reject an extension point name that is
not registered, with `InvalidInputError(code=GENERAL_INVALID_INPUT)`, so a
misspelled point name fails where it is written rather than surfacing later as
a wiring bug at `apply()`. They **MUST NOT** raise for a point that exists but
currently holds nothing: `get` returns null, `get_all` returns an empty
collection, `unregister` returns false (D-108).

## Removal must be expressible

`unregister(point_name, extension)` identifies its target by **identity** —
Python `is`, TypeScript `===`, pointer address in Rust — never by value
equality: two registrations that compare equal are two registrations (D-128).

Every implementation **MUST** offer a removal path a host can actually call. In
Rust, where the manager owns its extensions, that path is
`unregister_handle(handle)` with the `ExtensionHandle` returned by `register`
(D-91).

## Contract: ExtensionManager.apply

### Inputs
- `registry` (Registry, required) — registry to wire extensions into (discoverer, module_validator)
- `executor` (Executor, required) — executor to wire extensions into (acl, approval_handler, middleware, span_exporter)

### Errors
- Errors raised by the Registry / Executor setters propagate (Rust: `Err(ModuleError)`, e.g. a rejected middleware). A registered span exporter with no `TracingMiddleware` in the chain is **not** an error: it is logged as a warning and skipped.

### Returns
- On success: void/None/() — all registered extensions wired in the documented order (discoverer → module_validator → acl → approval_handler → middleware chain → span exporters)

### Side Effects (ordered)
1. `registry.set_discoverer(ext)` if discoverer registered
2. `registry.set_validator(ext)` if module_validator registered
3. `executor.set_acl(ext)` if acl registered
4. `executor.set_approval_handler(ext)` if approval_handler registered
5. `executor.use(mw)` for each middleware in registration order
6. Locate `TracingMiddleware` in the executor chain; set a single exporter directly or wrap several in a composite exporter; warn and skip if there is none

### Postconditions

`apply` **MUST NOT** consume the extension store (D-78). After it returns, the
manager still holds every registration: `get` and `get_all` report the same
extensions, and applying the same manager to a second registry / executor pair
wires the same set again.

### Properties
- async: false
- thread_safe: false (call once during startup, before concurrent request handling)
- pure: false (mutates registry and executor; does NOT mutate the extension store — see Postconditions)
- idempotent: false (calling apply twice stacks middleware and re-wires other extensions)

## Usage

### Custom Discoverer

A custom `Discoverer` replaces the default filesystem scan. `discover()` receives the configured extension roots and returns the discovered entries. The Registry then performs module-id validation (per [PROTOCOL_SPEC §2.7](../spec/protocol-spec.md)) → duplicate check → optional custom-validator call → registration. Malformed or rejected entries are skipped with a warning; a single bad entry MUST NOT abort the batch.

=== "Python"

    ```python
    from pydantic import BaseModel

    from apcore import Context, Discoverer, Registry


    class HelloInput(BaseModel):
        name: str


    class HelloOutput(BaseModel):
        message: str


    class HelloModule:
        input_schema = HelloInput
        output_schema = HelloOutput
        description = "Say hello"

        def execute(self, inputs: dict, context: Context) -> dict:
            return {"message": f"Hello, {inputs['name']}!"}


    class CustomDiscoverer(Discoverer):
        """Return a list of dicts, each with 'module_id' and 'module' keys."""

        def discover(self, roots: list[str]) -> list[dict]:
            return [{"module_id": "custom.hello", "module": HelloModule()}]


    registry = Registry()
    registry.set_discoverer(CustomDiscoverer())
    count = registry.discover()  # number of modules registered
    ```

=== "TypeScript"

    ```typescript
    import { Type } from "@sinclair/typebox";
    import { Registry, type Context, type Discoverer, type Module } from "apcore-js";

    const hello: Module = {
      inputSchema: Type.Object({ name: Type.String() }),
      outputSchema: Type.Object({ message: Type.String() }),
      description: "Say hello",
      execute: (inputs: Record<string, unknown>, _context: Context) => ({ message: `Hello, ${String(inputs.name)}!` }),
    };

    const discoverer: Discoverer = {
      async discover(_roots: string[]) {
        return [{ moduleId: "custom.hello", module: hello }];
      },
    };

    const registry = new Registry();
    registry.setDiscoverer(discoverer);
    const count = await registry.discover(); // number of modules registered
    ```

=== "Rust"

    ```rust
    use std::sync::Arc;

    use apcore::registry::{DiscoveredModule, Discoverer, ModuleDescriptor, Registry};
    use apcore::{Context, Module, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct HelloModule;

    #[async_trait]
    impl Module for HelloModule {
        fn description(&self) -> &str { "Say hello" }
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "message": { "type": "string" } } })
        }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            Ok(json!({ "message": format!("Hello, {}!", inputs["name"].as_str().unwrap_or("")) }))
        }
    }

    struct CustomDiscoverer;

    #[async_trait]
    impl Discoverer for CustomDiscoverer {
        async fn discover(&self, _roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError> {
            let module = HelloModule;
            // ModuleDescriptor has no Default; unspecified fields take their serde defaults.
            let descriptor: ModuleDescriptor = serde_json::from_value(json!({
                "module_id": "custom.hello",
                "description": module.description(),
                "input_schema": module.input_schema(),
                "output_schema": module.output_schema(),
            }))
            .map_err(|e| ModuleError::new(apcore::ErrorCode::GeneralInvalidInput, e.to_string()))?;
            Ok(vec![DiscoveredModule {
                name: "custom.hello".to_string(),
                source: "in-memory".to_string(),
                descriptor,
                module: Arc::new(module),
            }])
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let registry = Registry::new();
        registry.set_discoverer(Box::new(CustomDiscoverer));
        let count = registry.discover_internal().await?; // runs the configured discoverer
        println!("registered {count} module(s)");
        Ok(())
    }
    ```

!!! tip "Out-of-process modules"
    Discoverers for subprocess, RPC, or network-hosted modules wrap the external resource in a `Module` impl — for example, a `SubprocessModule { executable: PathBuf, descriptor }` whose `execute` spawns the binary and pipes JSON through stdin/stdout. The Registry then treats subprocess-backed and in-process modules identically: it runs the custom validator, calls `on_load`, and exposes the instance through `registry.get(name)`.

### Custom Module Validator

A custom module validator replaces the registry's default module validation with rules of your own (keep the structural checks in it if you still want them). It receives the module — in Python, the class during discovery and the instance at `register()` — and returns its errors (a list of strings in Python and TypeScript, a `ValidationResult` in Rust); any error rejects the module — at `register()` (`GENERAL_INVALID_INPUT` in Python and TypeScript, `MODULE_LOAD_ERROR` in Rust) and during discovery (the module is skipped with a warning).

=== "Python"

    ```python
    from typing import Any

    from apcore import ModuleValidator, Registry


    class StrictValidator(ModuleValidator):
        def validate(self, module: Any) -> list[str]:
            errors: list[str] = []
            if not getattr(module, "tags", None):
                errors.append("Module must have at least one tag")
            if len(getattr(module, "description", "") or "") < 20:
                errors.append("Module description must be at least 20 characters")
            return errors


    registry = Registry(extensions_dir="./extensions")
    registry.set_validator(StrictValidator())
    registry.discover()
    ```

=== "TypeScript"

    ```typescript
    import { Registry, type ModuleValidator } from "apcore-js";

    const strict: ModuleValidator = {
      validate(module: unknown): string[] {
        const m = module as { tags?: string[]; description?: string };
        const errors: string[] = [];
        if (!m.tags || m.tags.length === 0) errors.push("Module must have at least one tag");
        if ((m.description ?? "").length < 20) errors.push("Module description must be at least 20 characters");
        return errors;
      },
    };

    const registry = new Registry({ extensionsDir: "./extensions" });
    registry.setValidator(strict);
    await registry.discover();
    ```

=== "Rust"

    ```rust
    use apcore::registry::{ModuleDescriptor, ModuleValidator, Registry};
    use apcore::{Module, ValidationErrorDetail, ValidationResult};

    struct StrictValidator;

    impl ModuleValidator for StrictValidator {
        fn validate(&self, module: &dyn Module, _descriptor: Option<&ModuleDescriptor>) -> ValidationResult {
            let mut errors = Vec::new();
            if module.tags().is_empty() {
                errors.push(ValidationErrorDetail::message_only("Module must have at least one tag"));
            }
            if module.description().len() < 20 {
                errors.push(ValidationErrorDetail::message_only("Module description must be at least 20 characters"));
            }
            // ValidationResult is #[non_exhaustive]: start from Default and set fields.
            let mut result = ValidationResult::default();
            result.valid = errors.is_empty();
            result.errors = errors;
            result
        }
    }

    fn main() {
        let registry = Registry::new();
        registry.set_validator(Box::new(StrictValidator));
    }
    ```

## Dependencies

- **Registry** — Extension points `discoverer` and `module_validator` are wired into the Registry.
- **Executor** — Extension points `acl`, `approval_handler`, and `middleware` are wired into the Executor.
- **Observability** — Extension point `span_exporter` integrates with `TracingMiddleware`.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/extensions.py` | `ExtensionManager`, `ExtensionPoint`, `_CompositeExporter`, built-in points |

## Testing Strategy

- **Registration tests** verify that extensions of correct type are accepted and incorrect types are rejected.
- **Cardinality tests** confirm that single-cardinality points replace on re-registration and multi-cardinality points accumulate.
- **Wiring tests** confirm that `apply()` correctly sets the discoverer, validator, ACL, approval handler, middleware, and span exporters on the Registry and Executor.
- **CompositeExporter tests** verify fan-out delivery and error isolation between exporters.
- **Unregistration tests** verify that unregistered extensions are no longer returned by `get()` / `get_all()` and are not applied on subsequent `apply()` calls.
