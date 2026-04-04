# Module Registry and Discovery System

## Overview

The Module Registry and Discovery System is the central hub for discovering, registering, and querying modules within apcore. It implements an 8-step discovery pipeline that automatically finds modules from extension directories, resolves entry points via pluggy, loads metadata from YAML, validates module integrity, and registers them for use by the executor. The registry provides thread-safe access to all registered modules and supports lifecycle hooks, event callbacks, and flexible querying by tags, prefixes, and IDs.

## Requirements

- Automatically discover modules from configured extension directories by scanning the filesystem for module definitions and their associated metadata.
- Support manual module registration for programmatically defined modules that do not reside on disk.
- Resolve entry points using pluggy-based plugin discovery, enabling third-party packages to contribute modules via standard Python packaging mechanisms.
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

1. **Extension Directory Scanning** -- The `Scanner` component walks configured extension root directories, identifying module candidates by locating module definition files (Python files and their companion YAML metadata).

2. **Entry Point Resolution** -- The `EntryPoint` component uses pluggy to resolve registered entry points from installed Python packages. This enables third-party packages to contribute modules to the registry without any filesystem scanning.

3. **Metadata Loading and Merging** -- The `Metadata` component loads YAML metadata files for each discovered module and merges them with any code-defined metadata (such as decorators or class attributes). The merge follows a "YAML overrides code" strategy for conflicting keys.

4. **Dependency Analysis** -- The `Dependencies` component builds a dependency graph from module metadata and performs a topological sort. Cycles are detected and reported as errors, preventing registration of mutually dependent modules that cannot be loaded in any valid order.

5. **Module Validation** -- The `Validation` component checks each module against structural and interface requirements: required exports, handler signatures, schema presence, and metadata completeness. Invalid modules are rejected with descriptive error messages.

6. **Schema Loading** -- For each validated module, the associated input/output schemas are loaded via the Schema System. This step ensures that schemas are parseable and that all `$ref` references resolve before the module is registered.

7. **ID Map Override Application** -- If an ID map is configured, module identifiers are remapped according to the map. This allows operators to alias modules (e.g., `summarize` -> `summarize-v2`) or redirect calls without changing calling code.

8. **Registration and Event Emission** -- The module is added to the registry's internal store, its `on_load` lifecycle hook is called (if defined), and a `register` event is emitted to all registered callbacks.

### Key Components

- **Registry** -- The central registry class. Manages the module store, coordinates the discovery pipeline, handles manual registration, and provides query methods. All public methods acquire an `RLock` before reading or writing the module store.

- **Scanner** -- Walks multiple extension root directories in parallel, identifying module candidates. Supports configurable file patterns and exclusion rules. Returns a list of candidate paths with preliminary classification (single-file module vs. package module).

- **Metadata** -- Loads YAML metadata files and merges them with code-defined metadata. Handles missing or partial YAML gracefully by falling back to code-defined values. Validates metadata against a known schema to catch misconfigurations early.

- **Dependencies** -- Builds a directed acyclic graph (DAG) from module dependency declarations and produces a topological ordering. Cycle detection uses Kahn's algorithm; detected cycles are reported with the full cycle path for debugging.

- **EntryPoint** -- Wraps pluggy's entry point discovery. Resolves entry points from the `apcore.modules` group, instantiates plugin classes, and extracts module definitions from them.

- **Validation** -- Validates module structure: checks for required handler functions, verifies handler signatures, confirms schema availability, and validates metadata completeness.

- **SchemaExport** -- Utility component that generates `ModuleDescriptor` objects from registered modules. A `ModuleDescriptor` includes the module's metadata, input/output schemas (in multiple export formats), and capability declarations. This is used by external systems (e.g., LLM tool registries) to understand available modules.

### Thread Safety

All public methods on the `Registry` class acquire a reentrant lock (`threading.RLock`) before accessing the internal module store. This ensures safe concurrent access from multiple threads, including during discovery (which may be triggered from a background thread) and query (which may be called from request-handling threads). The reentrant nature of the lock allows lifecycle hooks and event callbacks to safely call back into the registry (e.g., to query other modules during `on_load`).

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

## Key Files

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

## Dependencies

### External
- `pluggy>=1.0` -- Entry point discovery and plugin resolution.
- `pyyaml>=6.0` -- YAML metadata file parsing.

### Internal
- **Schema System** -- The registry uses the Schema System (step 6 of the discovery pipeline) to load and validate module schemas.
- **Executor** -- The executor depends on the registry for module lookup (step 3 of the execution pipeline).

## Testing Strategy

- **Discovery pipeline tests** exercise the full 8-step pipeline with fixture extension directories containing valid modules, invalid modules, modules with dependencies, and modules with cycles. Tests verify correct ordering, rejection of invalid modules, and proper event emission.
- **Scanner tests** verify multi-root scanning, file pattern matching, exclusion rules, and graceful handling of unreadable directories or broken symlinks.
- **Metadata tests** cover YAML loading, code-defined fallback, merge conflict resolution, and validation of malformed metadata files.
- **Dependency tests** verify topological sort correctness for various DAG shapes (linear chains, diamonds, wide graphs) and confirm that cycles are detected and reported with full paths.
- **EntryPoint tests** mock pluggy entry points to verify resolution, instantiation, and extraction of module definitions from plugin classes.
- **Thread safety tests** run concurrent registration, unregistration, and query operations to verify that the `RLock` prevents data corruption and deadlocks.
- **Event system tests** verify that register/unregister callbacks are invoked with correct arguments and that callback exceptions do not break the registry.
- **ID map override tests** confirm that module IDs are correctly remapped and that queries use the overridden IDs.
- Test naming follows the `test_<unit>_<behavior>` convention.
