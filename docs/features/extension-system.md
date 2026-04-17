# Extension System

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
