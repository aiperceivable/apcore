# Extension System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §11 Extension Mechanism.


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
- SDKs **MAY** use runtime type checking mechanisms appropriate to the language (e.g., `isinstance` in Python, duck-type guards in TypeScript, trait bounds in Rust).

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
    use apcore::extensions::ExtensionManager;

    let mut manager = ExtensionManager::new();

    // Register extensions
    manager.register("discoverer", Box::new(my_discoverer))?;
    manager.register("module_validator", Box::new(my_validator))?;
    manager.register("middleware", Box::new(logging_middleware))?;
    manager.register("middleware", Box::new(metrics_middleware))?;
    manager.register("span_exporter", Box::new(stdout_exporter))?;
    manager.register("acl", Box::new(my_acl))?;
    manager.register("approval_handler", Box::new(my_approval_handler))?;

    // Wire everything into registry and executor
    manager.apply(&mut registry, &mut executor)?;
    ```

### Wiring Behavior (`apply`)

When `apply(registry, executor)` is called, the manager performs the following in order:

1. **Discoverer** → `registry.set_discoverer(ext)` — replaces the default discovery strategy.
2. **Module Validator** → `registry.set_validator(ext)` — replaces the default module validator.
3. **ACL** → `executor.set_acl(ext)` — replaces the executor's ACL provider.
4. **Approval Handler** → `executor.set_approval_handler(ext)` — replaces the executor's approval handler.
5. **Middleware** → `executor.use(mw)` for each registered middleware — appends to the middleware chain.
6. **Span Exporters** → Locates the `TracingMiddleware` in the executor's middleware chain and sets the exporter:
   - If a single exporter is registered, it is set directly.
   - If multiple exporters are registered, they are wrapped in a `CompositeExporter` that delegates to all underlying exporters. Failures in one exporter do not affect others.

### CompositeExporter (Internal)

When multiple span exporters are registered, they are composed into a single `CompositeExporter`:

```python
class _CompositeExporter:
    def __init__(self, exporters: list[SpanExporter]) -> None: ...

    def export(self, span: Span) -> None:
        """Export span to all underlying exporters. Failures are logged but not raised."""
        for exporter in self._exporters:
            try:
                exporter.export(span)
            except Exception:
                pass  # Log and continue
```

## Contract: ExtensionManager.register

### Inputs
- `point_name` (str/string/&str, required) — name of an existing extension point (e.g., `"discoverer"`, `"middleware"`); unknown names raise an error
- `extension` (Any/unknown/Box<dyn Trait>, required) — must satisfy the extension point's declared type; type checking is performed at registration time

### Errors
- `ExtensionPointNotFoundError` (or `ValueError` / `Err(ModuleError)`) — `point_name` is not a registered extension point
- `ExtensionTypeError` (or `TypeError` / `Err(ModuleError)`) — `extension` does not satisfy the point's expected type/trait

### Returns
- On success: void/None/() — extension is stored; for single-cardinality points, replaces any prior registration

### Properties
- async: false
- thread_safe: false (do not call concurrently with `apply()` or other `register()` calls)
- pure: false (mutates internal extension store)
- idempotent: false for single-cardinality (replaces); accumulating for multi-cardinality (`middleware`, `span_exporter`)

## Contract: ExtensionManager.get

### Inputs
- `point_name` (str/string/&str, required) — name of a single-cardinality extension point

### Errors
- No errors raised; returns `None`/`null`/`None` when nothing is registered

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
- No errors raised; returns empty list when nothing is registered

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
- No error if the extension is not found (silent no-op)

### Returns
- On success: void/None/()

### Properties
- async: false
- thread_safe: false
- pure: false (mutates extension store)

## Contract: ExtensionManager.apply

### Inputs
- `registry` (Registry, required) — registry to wire extensions into (discoverer, module_validator)
- `executor` (Executor, required) — executor to wire extensions into (acl, approval_handler, middleware, span_exporter)

### Errors
- `ExtensionApplyError` (or propagated from Registry/Executor) — wiring a span exporter fails because no `TracingMiddleware` is present in the executor chain; or Registry/Executor APIs raise

### Returns
- On success: void/None/() — all registered extensions wired in the documented order (discoverer → module_validator → acl → approval_handler → middleware chain → span exporters)

### Side Effects (ordered)
1. `registry.set_discoverer(ext)` if discoverer registered
2. `registry.set_validator(ext)` if module_validator registered
3. `executor.set_acl(ext)` if acl registered
4. `executor.set_approval_handler(ext)` if approval_handler registered
5. `executor.use(mw)` for each middleware in registration order
6. Locate `TracingMiddleware` in executor chain; set single exporter directly or wrap multiple in `CompositeExporter`

### Properties
- async: false
- thread_safe: false (call once during startup, before concurrent request handling)
- pure: false (mutates registry and executor)
- idempotent: false (calling apply twice stacks middleware and re-wires other extensions)

## Usage

### Custom Discoverer

A custom `Discoverer` replaces the default filesystem scan. `discover()` receives the configured extension roots and returns the discovered entries. The Registry then performs module-id validation (per [PROTOCOL_SPEC §2.7](../../PROTOCOL_SPEC.md)) → duplicate check → optional custom-validator call → registration. Malformed or rejected entries are skipped with a warning; a single bad entry MUST NOT abort the batch.

=== "Python"

    ```python
    from apcore import Registry
    from apcore.registry.registry import Discoverer

    class CustomDiscoverer(Discoverer):
        """Return a list of dicts, each with 'module_id' and 'module' keys."""

        def discover(self, roots: list[str]) -> list[dict]:
            return [
                {"module_id": "custom.hello", "module": HelloModule()},
            ]

    registry = Registry(extensions_dir="./extensions")
    registry.set_discoverer(CustomDiscoverer())
    registry.discover()  # returns the count of successfully registered modules
    ```

=== "TypeScript"

    ```typescript
    import { Registry, Discoverer } from "apcore";

    const discoverer: Discoverer = {
      async discover(roots: string[]) {
        return [{ moduleId: "custom.hello", module: new HelloModule() }];
      },
    };

    const registry = new Registry({ extensionsDir: "./extensions" });
    registry.setDiscoverer(discoverer);
    await registry.discover();
    ```

=== "Rust"

    ```rust
    use apcore::registry::registry::{DiscoveredModule, Discoverer, Registry};
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct CustomDiscoverer;

    #[async_trait]
    impl Discoverer for CustomDiscoverer {
        async fn discover(
            &self,
            _roots: &[String],
        ) -> Result<Vec<DiscoveredModule>, ModuleError> {
            Ok(vec![DiscoveredModule {
                name: "custom.hello".to_string(),
                source: "in-memory".to_string(),
                descriptor: /* build a ModuleDescriptor for this module */ todo!(),
                module: Arc::new(HelloModule) as Arc<dyn apcore::module::Module>,
            }])
        }
    }

    let registry = Registry::new();
    registry.set_discoverer(Box::new(CustomDiscoverer));
    let count = registry.discover_internal().await?;
    ```

!!! tip "Out-of-process modules"
    Discoverers for subprocess, RPC, or network-hosted modules wrap the external resource in a `Module` impl — for example, a `SubprocessModule { executable: PathBuf, descriptor }` whose `execute` spawns the binary and pipes JSON through stdin/stdout. The Registry then treats subprocess-backed and in-process modules identically: it runs the custom validator, calls `on_load`, and exposes the instance through `registry.get(name)`.

### Custom Module Validator

Override the default structural conformance check with a stricter rule set:

```python
from apcore import Registry, ModuleValidator

class StrictValidator(ModuleValidator):
    def validate(self, module_class: Type[Module]) -> list[str]:
        errors = super().validate(module_class)

        if not module_class.tags:
            errors.append("Module must have at least one tag")
        if not module_class.__doc__ or len(module_class.__doc__) < 20:
            errors.append("Module description must be at least 20 characters")

        return errors

registry = Registry(extensions_dir="./extensions")
registry.set_validator(StrictValidator())
registry.discover()
```

> Available in apcore-python v0.5.1+ and apcore-typescript v0.3.0+.

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
