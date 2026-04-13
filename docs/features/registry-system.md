# Module Registry and Discovery System

## Overview

The Module Registry and Discovery System is the central hub for discovering, registering, and querying modules within apcore. It implements an 8-step discovery pipeline that automatically finds modules from extension directories, resolves entry points from installed packages, loads metadata from YAML, validates module integrity, and registers them for use by the executor. The registry provides thread-safe access to all registered modules and supports lifecycle hooks, event callbacks, and flexible querying by tags, prefixes, and IDs.

## Requirements

- Automatically discover modules from configured extension directories by scanning the filesystem for module definitions and their associated metadata.
- Support manual module registration for programmatically defined modules that do not reside on disk.
- Resolve entry points via standard package plugin discovery, enabling third-party packages to contribute modules without filesystem scanning.
- Load and merge module metadata from YAML files, combining filesystem metadata with code-defined metadata into a unified representation.
- Perform topological sorting of module dependencies with cycle detection, ensuring modules are loaded in the correct order.
- Validate discovered modules before registration, rejecting modules that do not meet structural or interface requirements.
- Support ID map overrides that allow remapping module identifiers, enabling aliasing and version-based routing.
- Provide lifecycle hooks (`on_load`, `on_unload`) for modules that need to perform setup or teardown when entering or leaving the registry.
- Emit events (`register`, `unregister`) via a callback system so that other components can react to registry changes.
- Offer flexible query capabilities: filter modules by tag, prefix, or arbitrary predicates, and generate `ModuleDescriptor` objects for external consumption.
- Guarantee thread safety on all read and write paths using reentrant locks.

## Technical Design

### 8-Step Discovery Pipeline

The registry's `discover()` method processes modules through the following pipeline:

1. **Extension Directory Scanning** -- The `Scanner` component walks configured extension root directories, identifying module candidates by locating module source files and their companion YAML metadata.

2. **Entry Point Resolution** -- The `EntryPoint` component resolves registered entry points from installed packages, enabling third-party packages to contribute modules to the registry without any filesystem scanning.

3. **Metadata Loading and Merging** -- The `Metadata` component loads YAML metadata files for each discovered module and merges them with any code-defined metadata (such as decorators or class attributes). The merge follows a "YAML overrides code" strategy for conflicting keys.

4. **Dependency Analysis** -- The `Dependencies` component builds a dependency graph from module metadata and performs a topological sort. Cycles are detected and reported as errors, preventing registration of mutually dependent modules that cannot be loaded in any valid order.

5. **Module Validation** -- The `Validation` component checks each module against structural and interface requirements: required exports, handler signatures, schema presence, and metadata completeness. Invalid modules are rejected with descriptive error messages.

6. **Schema Loading** -- For each validated module, the associated input/output schemas are loaded via the Schema System. This step ensures that schemas are parseable and that all `$ref` references resolve before the module is registered.

7. **ID Map Override Application** -- If an ID map is configured, module identifiers are remapped according to the map. This allows operators to alias modules (e.g., `summarize` -> `summarize-v2`) or redirect calls without changing calling code.

8. **Registration and Event Emission** -- The module is added to the registry's internal store, its `on_load` lifecycle hook is called (if defined), and a `register` event is emitted to all registered callbacks.

### Key Components

- **Registry** -- The central registry class. Manages the module store, coordinates the discovery pipeline, handles manual registration, and provides query methods. All public methods take a reentrant lock before reading or writing the module store.

- **SchemaExport** -- Utility component that generates `ModuleDescriptor` objects from registered modules. A `ModuleDescriptor` includes the module's metadata, input/output schemas (in multiple export formats), and capability declarations. This is used by external systems (e.g., LLM tool registries) to understand available modules.

The pipeline-specific components (`Scanner`, `Metadata`, `Dependencies`, `EntryPoint`, `Validation`) are described in the 8-Step Discovery Pipeline above.

### Thread Safety

All public methods on the Registry acquire a reentrant lock before accessing the internal module store. This ensures safe concurrent access from multiple threads, including during discovery (which may be triggered from a background thread) and query (which may be called from request-handling threads). The reentrant nature of the lock allows lifecycle hooks and event callbacks to safely call back into the registry (e.g., to query other modules during `on_load`). Single-threaded language runtimes (e.g., JavaScript) MAY treat the lock as a no-op.

### Event System

The registry supports registering callback functions for two event types:

- **register** -- Fired after a module is successfully added to the registry. Callbacks receive the module's ID and metadata.
- **unregister** -- Fired after a module is successfully removed from the registry. Callbacks receive the module's ID and metadata.

Callbacks are invoked synchronously within the registry lock, ensuring consistent state visibility.

### Query Capabilities

The registry provides several query methods:

- `get(module_id)` -- Direct lookup by ID.
- `list(tags=None, prefix=None)` -- Returns all registered modules, optionally filtered by tags and/or ID prefix. When `tags` is provided, only modules whose metadata includes the specified tag(s) are returned. When `prefix` is provided, only modules whose IDs start with the given prefix are returned. Both filters can be combined.
- `get_definition(module_id)` -- Returns a `ModuleDescriptor` for the specified module, including exported schemas.

## Usage

=== "Python"
    ```python
    from apcore.registry import Registry
    from apcore.executor import Executor

    # Create registry and register a module manually
    registry = Registry()

    class AddModule:
        description = "Add two integers"
        input_schema = {"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}}
        output_schema = {"type": "object", "properties": {"sum": {"type": "integer"}}}

        async def execute(self, inputs, ctx):
            return {"sum": inputs["a"] + inputs["b"]}

    registry.register("math.add", AddModule())

    # Query the registry
    module_ids = registry.list()                         # ["math.add"]
    filtered = registry.list(tags=["math"])              # filter by tag
    mod = registry.get("math.add")                       # retrieve module
    descriptor = registry.get_definition("math.add")     # ModuleDescriptor with schemas

    # Auto-discover from a directory
    registry.discover(extension_dirs=["./modules"])

    # Subscribe to register/unregister events
    def on_register(module_id, metadata):
        print(f"Registered: {module_id}")

    registry.on("register", on_register)

    # Wire into executor
    executor = Executor(registry=registry)
    ```
=== "TypeScript"
    ```typescript
    import { Registry } from "apcore-js/registry";
    import { Executor } from "apcore-js/executor";

    // Create registry and register a module manually
    const registry = new Registry();

    registry.register("math.add", {
        description: "Add two integers",
        inputSchema: { type: "object", properties: { a: { type: "number" }, b: { type: "number" } } },
        outputSchema: { type: "object", properties: { sum: { type: "number" } } },
        execute: ({ a, b }: { a: number; b: number }) => ({ sum: a + b }),
    });

    // Query the registry
    const moduleIds = registry.list();                        // ["math.add"]
    const filtered = registry.list({ tags: ["math"] });       // filter by tag
    const mod = registry.get("math.add");                     // retrieve module
    const descriptor = registry.getDefinition("math.add");    // ModuleDescriptor with schemas

    // Auto-discover from a directory
    await registry.discover({ extensionDirs: ["./modules"] });

    // Subscribe to register/unregister events
    registry.on("register", (moduleId, metadata) => {
        console.log(`Registered: ${moduleId}`);
    });

    // Wire into executor
    const executor = new Executor({ registry });
    ```
=== "Rust"
    ```rust
    use apcore::registry::Registry;
    use apcore::executor::Executor;
    use apcore::module::Module;
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct AddModule;

    #[async_trait]
    impl Module for AddModule {
        fn description(&self) -> &str { "Add two integers" }
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"sum": {"type": "integer"}}})
        }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let sum = inputs["a"].as_i64().unwrap_or(0) + inputs["b"].as_i64().unwrap_or(0);
            Ok(json!({"sum": sum}))
        }
    }

    let mut registry = Registry::new();
    registry.register("math.add", Box::new(AddModule))?;

    // Query the registry
    let module_ids = registry.module_ids();           // vec!["math.add"]
    let has_mod = registry.has("math.add");           // true
    let count = registry.count();                     // 1

    // Auto-discover from a directory
    registry.discover(&["./modules"])?;

    // Wire into executor
    let executor = Executor::from_registry(registry);
    ```

## Dependencies

- **Schema System** -- The registry uses the Schema System (step 6 of the discovery pipeline) to load and validate module schemas.
- **Executor** -- The executor depends on the registry for module lookup (step 3 of the execution pipeline).

??? info "Python SDK reference"
    The following tables are **not protocol requirements** — they document the Python SDK's source layout and runtime dependencies for implementers/users of `apcore-python`.

    **Source files:**

    | File | Lines | Purpose |
    |------|-------|---------|
    | `registry/registry.py` | 410 | Central registry with discovery pipeline and query methods |
    | `registry/scanner.py` | 156 | Multi-root extension directory scanning |
    | `registry/metadata.py` | 123 | YAML metadata loading and merging |
    | `registry/dependencies.py` | 112 | Topological sort with cycle detection |
    | `registry/entry_point.py` | 91 | Pluggy-based entry point resolution |
    | `registry/schema_export.py` | 189 | ModuleDescriptor generation and schema export |
    | `registry/validation.py` | 46 | Module structural validation |
    | `registry/types.py` | 51 | Shared type definitions (ModuleDescriptor, etc.) |

    **Runtime dependencies:**

    - `pluggy>=1.0` -- Entry point discovery and plugin resolution.
    - `pyyaml>=6.0` -- YAML metadata file parsing.

## Testing Strategy

- **Discovery pipeline tests** exercise the full 8-step pipeline with fixture extension directories containing valid modules, invalid modules, modules with dependencies, and modules with cycles. Tests verify correct ordering, rejection of invalid modules, and proper event emission.
- **Scanner tests** verify multi-root scanning, file pattern matching, exclusion rules, and graceful handling of unreadable directories or broken symlinks.
- **Metadata tests** cover YAML loading, code-defined fallback, merge conflict resolution, and validation of malformed metadata files.
- **Dependency tests** verify topological sort correctness for various DAG shapes (linear chains, diamonds, wide graphs) and confirm that cycles are detected and reported with full paths.
- **EntryPoint tests** verify entry point resolution, instantiation, and extraction of module definitions from plugin packages.
- **Thread safety tests** run concurrent registration, unregistration, and query operations to verify that the reentrant lock prevents data corruption and deadlocks.
- **Event system tests** verify that register/unregister callbacks are invoked with correct arguments and that callback exceptions do not break the registry.
- **ID map override tests** confirm that module IDs are correctly remapped and that queries use the overridden IDs.
- Test naming follows the `test_<unit>_<behavior>` convention.
