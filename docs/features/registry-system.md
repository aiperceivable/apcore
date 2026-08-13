---
description: "Module Registry with an 8-step discovery pipeline: directory scan, entry-point resolution, YAML metadata merge, dependency topo-sort, validation, schema load, ID-map override, register."
---

# Module Registry and Discovery System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §12.2 (Registry component).


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

### Reserved Namespaces

The following module ID prefixes are reserved for framework use and specific runtime behaviors. Standard discovery paths and manual `register()` calls MUST respect these reservations.

| Namespace | Purpose | Registration Rule |
|-----------|---------|-------------------|
| `system.*` | Built-in introspection & control | Only via `register_internal()` |
| `internal.*` | Framework-private modules | Only via `register_internal()` |
| `core.*` | Core protocol primitives | Only via `register_internal()` |
| `apcore.*` | Protocol-defined extensions | Only via `register_internal()` |
| `ephemeral.*` | Runtime-synthesized modules | Only via `register()`; MUST NOT be discovered from disk |

Modules in the `ephemeral.*` namespace are permitted to bypass filesystem validation and are intended for agent-synthesized tools or on-the-fly composition. See [RFC: Ephemeral Modules](../spec/rfc-ephemeral-modules.md).

### Event System

The registry supports registering callback functions for two event types:

- **register** -- Fired after a module is successfully added to the registry. Callbacks receive the module's ID and the module instance (`(module_id, module)`).
- **unregister** -- Fired after a module is successfully removed from the registry. Callbacks receive the module's ID and the module instance (`(module_id, module)`).

Callbacks are invoked synchronously within the registry lock, ensuring consistent state visibility.

### Query Capabilities

The registry provides several query methods:

- `get(module_id)` -- Direct lookup by ID.
- `list(tags=None, prefix=None, visibility=None)` -- Returns all registered modules, optionally filtered by tags, ID prefix, and/or visibility. When `tags` is provided, only modules whose metadata includes the specified tag(s) are returned. When `prefix` is provided, only modules whose IDs start with the given prefix are returned. When `visibility` is provided (a subset of `["public", "hidden"]`, per D-24), only modules with a matching discoverable status are returned. All filters can be combined. (Python/TypeScript also accept a deprecated `include_hidden`/`includeHidden` boolean superseded by `visibility`.)
- `get_definition(module_id)` -- Returns a `ModuleDescriptor` for the specified module, including exported schemas.

## Contract: Registry.register

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `module_id`: string, required. Must pass module-ID validation (see Schema System). Empty, malformed, or reserved IDs MUST be rejected before any mutation of the registry.
- `module`: Module instance, required. Must implement the module protocol (`description`, `input_schema`, `output_schema`, `execute`).
- `metadata`: mapping, optional. When an implementation accepts it, a `dependencies` entry — a list of `{module_id, version?, optional?}` objects — reaches the registered module's descriptor, so `get_definition(module_id).dependencies` returns what the caller declared. All three SDKs behave this way.

!!! info "Normative as of spec v1.10.0"

    [PROTOCOL_SPEC](../spec/protocol-spec.md#122-core-component-interface-contracts) §12.2 `Interface: Registry` states `register(module_id, module, version?, metadata?)` and requires that a `dependencies` entry in `metadata` reach the module descriptor. Until v1.10.0 §12.2 declared only `discover`, `get`, `list` and `describe` — `register` was outside the normative interface entirely, so nothing above the SDKs required this.

    That gap is why each of the three lost it at least once and each was fixed independently (apcore-python `ad2998d`, apcore-typescript#35, apcore-rust#35). The loss is quiet by construction: discovery-time dependency sorting reads its own parse and keeps working, so `resolve_dependencies` looks healthy, while the post-registration accessor returns nothing and a dependency-ordered reload degrades to the sort's seed order — usually alphabetical, therefore plausible, therefore not reported.

    The ordered side effects, the in-flight reservation and the visibility rule stay on this page; §12.2 carries the signature and the data-survival requirement. Governance: [apcore#90](https://github.com/aiperceivable/apcore/issues/90).

!!! info "Multi-version registration (optional, Phase B)"
    SDKs MAY accept additional `version` and `metadata` parameters to support [§5.4 Multi-version Coexistence](../spec/protocol-spec.md#54-multi-version-coexistence). When supported, the same `module_id` MAY be registered with multiple distinct versions, and `Registry.get(module_id, version_hint=...)` resolves via semantic-version range matching.

    Accepting the parameters and resolving by version are separate things, and only apcore-python does both.

    **SDK status (Phase B)**:

    - **apcore-python** — accepts and resolves. `register(module_id, module, version=None, metadata=None)`, backed by an internal `VersionedStore`; `get(module_id, version_hint=...)` performs semantic-version range matching.
    - **apcore-typescript** — accepts, does not resolve. `register(moduleId, module, version?, metadata?, options?)` takes the full signature; `version` and `metadata` are merged into the module's metadata and readable back through `getDefinition()` and `list({tags})`. `get(moduleId, versionHint?)` and `getDefinition(moduleId, versionHint?)` accept the hint for signature parity and ignore it. A duplicate `module_id` is rejected by A03 conflict detection with `DUPLICATE_MODULE_ID`; it does **not** replace the prior registration.
    - **apcore-rust** — accepts, does not resolve. `register_versioned(name, module, version: Option<&str>, metadata: Option<HashMap<..>>)` is the spec-shaped four-argument form and honours `metadata["dependencies"]`. Two other forms exist: `register(name, module, descriptor: Option<ModuleDescriptor>)` takes a code-side `ModuleDescriptor` directly, and `register_module(name, module)` is the two-argument convenience form. `get` / `get_definition` take no version hint.

    Implementations that omit multi-version support MUST behave as single-version registries. Cross-language portable code SHOULD NOT rely on `version` resolution until all SDKs implement it; `metadata.dependencies` is safe today and normative per the `metadata` input above.

### Preconditions

- The registry's internal lock MUST be acquired before the duplicate-ID check.
- `module.on_load()` MUST NOT be invoked until the registry has confirmed the module is uniquely registered.
- The module MUST NOT become visible to discovery APIs (`get`, `list`, `get_definition`) until `module.on_load()` has completed successfully. See [Registration Ordering Invariants](#registration-ordering-invariants-issue-65).

### Side Effects (ordered)

1. Acquire registry lock.
2. Validate `module_id` and module structure.
3. Check for duplicate `module_id` against both the visible store **and** the in-flight loading set; reject with `InvalidInputError(code=DUPLICATE_MODULE_ID)` if already registered (unless overwrite semantics are explicitly opted in).
4. Reserve `module_id` in an in-flight loading set so concurrent registrations for the same ID are rejected with `DUPLICATE_MODULE_ID`.
5. Release the registry lock.
6. Invoke `module.on_load()` if defined, **outside** the registry lock but **before** the module becomes visible. If it raises:
   - Remove `module_id` from the in-flight loading set.
   - Emit `apcore.registry.module_load_failed` carrying `{module_id, callback_name, error_type, error_message}`.
   - Re-raise the original exception.
7. Atomically publish the module into the visible discovery store (briefly re-acquiring the registry lock) and remove `module_id` from the in-flight loading set. After this step the module is observable via `get`, `list`, and `get_definition`.
8. Emit a `register` event to subscribers.

### Errors

- `InvalidInputError(code=INVALID_MODULE_ID)` -- `module_id` fails validation.
- `InvalidInputError(code=DUPLICATE_MODULE_ID)` -- `module_id` is already registered, or a concurrent registration is currently loading the same ID.

### Returns

- On success: `None` (Python), `void` (TypeScript), `Ok(())` (Rust).
- On failure: raises (Python/TypeScript) / returns `Err` (Rust).

### Properties

- `async`: `false`.
- `thread_safe`: `true` -- reentrant lock held during mutation. Single-threaded language runtimes (e.g., JavaScript) MAY treat the lock as a no-op.
- `pure`: `false` -- mutates the internal store; may trigger external `on_load` side effects.
- `idempotent`: `false` -- duplicate registration is an error, not a no-op.

## Contract: Scanner.scan_extensions

!!! info "Internal component"
    `Scanner` is an internal pipeline component — no SDK exposes it as a public class. This contract documents the behavior that `Registry.discover()` delegates to step 1 of the discovery pipeline; it is normative for SDK implementors, not for module authors.

Normative contract for the filesystem scanner used by step 1 of the discovery pipeline.

### Inputs

- `root`: string path, required. Directory to scan for module candidates.
- `max_depth`: integer, optional (default `8`). Maximum directory depth.
- `follow_symlinks`: boolean, optional (default `false`).
- `extensions`: list of strings, optional (default language-specific — `[".py"]` for Python, `[".ts", ".js"]` for TypeScript, `[".rs"]` for Rust). File extensions accepted as candidate modules.

> **Cross-language signature note (D10-014).** The Rust SDK exposes
> `extensions` as an explicit `Option<&[&str]>` parameter
> (`apcore-rust/src/registry/scanner.rs:26`), while Python and TypeScript
> infer it from the language convention. The fourth parameter is therefore
> language-idiomatic — Rust callers may override the default extension
> set; Python/TypeScript callers do not.

### Errors

- `ConfigNotFoundError(code=CONFIG_NOT_FOUND)` -- `root` does not exist or is not readable.

### Returns

- On success: ordered sequence of file-path records, in stable filesystem-traversal order. Python and TypeScript return `DiscoveredModule` (path + derived module ID); Rust returns `DiscoveredFile` (path only — the module-ID derivation step is performed downstream by `derive_module_ids`). The structural payload is equivalent — only the type name differs (D10-014).
- On failure: raises / returns `Err`.

### Properties

- `async`: `false`.
- `thread_safe`: `true` -- no shared mutable state.
- `pure`: `false` -- reads the filesystem.

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

    # Auto-discover modules from extension directories
    # (extension dirs are configured via Registry constructor or Config)
    discovered_count = registry.discover()

    # Subscribe to register/unregister events
    def on_register(module_id, metadata):
        print(f"Registered: {module_id}")

    registry.on("register", on_register)

    # Wire into executor
    executor = Executor(registry=registry)
    ```
=== "TypeScript"
    ```typescript
    import { Registry } from "apcore-js";
    import { Executor } from "apcore-js";

    // Create registry and register a module manually
    const registry = new Registry();

    await registry.register("math.add", {
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

    // Auto-discover modules from extension directories
    // (extension dirs are configured via Registry constructor options)
    const discoveredCount = await registry.discover();

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
    // 2-arg form for parity with Python/TS; equivalent to `register("math.add", Box::new(AddModule), None)`
    registry.register_module("math.add", Box::new(AddModule))?;

    // Query the registry
    let module_ids = registry.module_ids();           // vec!["math.add"]
    let has_mod = registry.has("math.add");           // true
    let count = registry.count();                     // 1

    // Auto-discover modules from extension directories
    // (configure extension roots via `registry.set_extension_roots(...)` or `Registry::with_options`)
    let discoverer = apcore::registry::DefaultDiscoverer::new();
    let discovered_count = registry.discover(&discoverer).await?;

    // Wire into executor
    let executor = Executor::from_registry(registry);
    ```

### Thread Safety Specifications

| Operation | Thread Safe | Notes |
|-----------|-------------|-------|
| `get()` | MUST be safe | Read-only query |
| `has()` | MUST be safe | Read-only query |
| `list()` | MUST be safe | Read-only query |
| `iter()` | SHOULD be safe | Snapshot iteration |
| `discover()` | MUST NOT be concurrent | Called once at startup |
| `register()` | SHOULD be safe | Synchronized write |
| `unregister()` | SHOULD be safe | Synchronized write |

### Error Condition Table

| Condition | Error Code | Description |
|-----------|------------|-------------|
| `extensions_dir` does not exist | `CONFIG_NOT_FOUND` | Extensions directory MUST exist |
| Module file syntax error | `MODULE_LOAD_ERROR` | Log warning and skip |
| Module interface incomplete | `MODULE_LOAD_ERROR` | Missing required attributes |
| Module ID conflict | `MODULE_LOAD_ERROR` | Same ID registered twice |
| ID Map file format error | `CONFIG_INVALID` | ID Map YAML parsing failed |
| Circular dependency | `CIRCULAR_DEPENDENCY` | Cycle detected between modules |

### Hot Reload (Development Mode)

Hot reload watches the extension directory for file changes and re-runs discovery. It is intended for development; production registries SHOULD NOT enable file watching.

```python
from apcore import Registry

registry = Registry(extensions_dir="./extensions")
registry.discover()

registry.on("change", lambda module_id: print(f"Module changed: {module_id}"))
registry.on("add",    lambda module_id: print(f"Module added: {module_id}"))
registry.on("remove", lambda module_id: print(f"Module removed: {module_id}"))
registry.watch()

# Stop watching
registry.unwatch()
```

> Available in apcore-python v0.5.1+ (requires the optional `watchdog` dependency) and apcore-typescript v0.3.0+.

!!! warning "Hot-reload behaviour is language-defined (D11-005)"
    The post-state of a `watch()`-triggered file change is **not normatively specified** across SDKs:

    - **apcore-python** re-imports the changed module file, calls `on_suspend()` on the old instance, calls `on_unload()`, then calls `on_resume(suspended_state)` on the new instance.
    - **apcore-typescript** unregisters the module and emits the `'file_changed'` event; consumers must trigger re-discovery themselves. (ES module specifiers are immutable in Node, so programmatic re-import is not portable.)
    - **apcore-rust** triggers `discover_internal()` to re-run the configured `Discoverer`; `on_suspend` / `on_resume` are **not** invoked.

    Code that relies on `on_suspend` / `on_resume` lifecycle hooks firing on file change is **portable only on Python**. Cross-language integration tests SHOULD subscribe to the `'file_changed'` / `'change'` event and orchestrate state migration explicitly. The hooks themselves remain a `MAY`-level optional Module API for explicit caller-driven suspend/resume flows (see [Module Interface §Lifecycle Hooks](./module-interface.md#lifecycle-hooks)).

## Registration Ordering Invariants (Issue #65)

Earlier versions of the apcore-python implementation intentionally ran `on_load` callbacks **outside** the registry lock and **after** the module had been inserted into the visible discovery store. The rationale recorded in the source was "running callbacks under the RLock would make lock scenarios complex." The side effect is a small but reliably-reproducible window in which a module is observable via `get` / `list` but its `on_load`-installed state (warmed pools, primed caches, wired dependencies) is incomplete.

!!! warning "Discovered during apcore-a2a upgrade"
    Adapter code publishes synthetic modules into the registry during application start. Under parallel registration, downstream components doing `registry.get(\"executor.email.send_email\")` were occasionally receiving a `Module` reference whose `on_load` callback was still mid-flight. The defensive fix in adapter code was to busy-wait — fragile, and not portable to TS/Rust. The invariant below closes this window at the protocol level.

This section defines the canonical visibility/initialization ordering. All SDKs MUST conform.

!!! warning "Applies to every registration path"
    The invariants below apply uniformly to **every** path that registers a module — the public `register()` API, internal helpers (e.g., `register_internal` used by sys-modules), and discovery-driven paths (`discover()`, `register_discovered`, hot-reload, etc.). SDKs MUST NOT create per-path exceptions; if a discover-time `on_load` callback needs to enumerate sibling modules, the callback MUST be re-shaped as a post-discover hook rather than as grounds for an early-visibility carveout.

### Strong-Guarantee Visibility (Normative)

- **MUST** — A module MUST NOT appear in `registry.list()`, `registry.get()`, `registry.get_definition()`, or any other discovery API until **all** `on_load` callbacks registered for that module have completed successfully.
- **MUST** — If any `on_load` callback raises, the module MUST NOT become visible. The registration call MUST surface the original exception unchanged (no wrapping).
- **MUST** — On callback failure the registry MUST emit `apcore.registry.module_load_failed` carrying:

  | Field | Type | Meaning |
  |-------|------|---------|
  | `module_id` | string | The module ID under which registration was attempted. |
  | `callback_name` | string | Identifier of the failing callback (e.g., `module.on_load`, or a third-party hook name). |
  | `error_type` | string | The exception class name. |
  | `error_message` | string | The exception message. |
  | `timestamp` | string (ISO 8601 UTC) | Time of failure. |

- The registry MUST NOT roll back side effects performed inside `on_load` (network connections opened, files written, etc.). Cleanup of partial state is the callback's responsibility. The DLQ-style event above gives subscribers a hook to react.

### Per-Module Init Locks (Informative)

SDKs SHOULD implement the strong-guarantee invariant via a **deferred-publish** pattern that avoids the lock-ordering problem the original apcore-python implementation cited:

1. Acquire the registry lock briefly to reserve `module_id` in an in-flight loading set (rejecting concurrent registrations of the same ID with `DUPLICATE_MODULE_ID`).
2. Release the registry lock.
3. Run `on_load` callbacks while holding a **per-module** initialization lock (not the global registry lock). This avoids serializing all module registrations through one mutex.
4. On success, briefly re-acquire the registry lock and atomically publish into the visible discovery map.
5. On failure, re-acquire the registry lock long enough to remove `module_id` from the in-flight set, then emit `apcore.registry.module_load_failed` and re-raise.

This is consistent with the updated [Side Effects ordering](#side-effects-ordered) under [Contract: Registry.register](#contract-registryregister).

### Concurrency Across Distinct Modules (Permissive)

- **MAY** — Callbacks for **different** modules MAY run concurrently. The strong-guarantee invariant is **per-module**, not global. SDKs are free to register modules in parallel and run their `on_load` hooks concurrently; visibility serialization happens at publish time.
- **SHOULD** — When `on_load` performs expensive work (network connection warmup, JIT compilation, large memory allocations), SDKs SHOULD document the per-module locking behavior so operators know that long-running callbacks block only callers waiting on **that specific** module via `get`/`list` (they do not block registration of unrelated modules).

### Out of Scope

- `on_unload` ordering during deregistration is **not** covered here; it follows the existing reverse-order semantics applied during module unregistration (see [Module Interface §Lifecycle Hooks](./module-interface.md#lifecycle-hooks)) and the language-specific hot-reload behavior described in [Hot Reload (Development Mode)](#hot-reload-development-mode).
- Re-registration of an already-loaded module ID (hot-swap) follows the existing language-defined semantics; the strong-guarantee invariant applies to the **new** instance's `on_load` independently.

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

## Contract: Registry.get

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `module_id` (str/string/&str, required) — the canonical module ID to look up.
- `version_hint` (str/string/&str, optional, default `None`/`null`) — if provided, the registry MUST resolve to the best matching version using semantic-version range matching. If omitted, the latest registered version MUST be returned.

### Preconditions

- `module_id` MUST NOT be an empty string. An empty `module_id` MUST be rejected before any lock is acquired.

### Errors

- `ModuleNotFoundError(module_id="")` — raised/thrown if `module_id` is an empty string. Implementations MUST NOT accept empty IDs silently.
- No error is raised for a well-formed `module_id` that is simply not registered — `get` returns `None`/`null` in that case.

### Returns

- On success (found): the registered module instance (type varies by SDK).
- On success (not found): `None` (Python), `null` (TypeScript), `None` (Rust `Option`).

!!! note "Version resolution"
    In Python, `version_hint` routes through the `VersionedStore`; if no versioned entry exists, it falls back to the primary `_modules` map. TypeScript accepts but does NOT route `versionHint` on `get()` — the parameter `get(moduleId, _versionHint?)` exists in the signature for source-compat with Python's `version_hint`, but the TS runtime always returns the single registered instance regardless of the hint. Rust mirrors the TS shape via `Option<&str>`. Cross-language portable code SHOULD register a single version per ID until [Phase B multi-version](#contract-registryregister) is implemented in all SDKs.

### Properties

- async: false
- thread_safe: true (Python acquires the reentrant lock; TypeScript is single-threaded)
- pure: false (acquires lock; reads shared state)
- idempotent: true (read-only; repeated calls with the same args return the same result if the registry has not changed)

## Contract: Registry.list

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `tags` (list[str]/string[]/Vec<String>, optional) — if provided, only modules whose tag set is a superset of all supplied tags are included. Tag matching MUST be all-or-nothing (every supplied tag must be present). An empty `tags` list MUST be treated the same as `None`/`null` (no tag filter).
- `prefix` (str/string/&str, optional) — if provided, only modules whose `module_id` starts with this prefix are included. Prefix matching is exact string prefix (`startsWith`), not a glob or regex.

### Errors

- None. Invalid or unknown tags and prefixes that match nothing return an empty list without error.

### Returns

- On success: lexicographically sorted list of unique module ID strings. The sort order MUST be consistent across calls when the registry has not changed.

!!! note "Tag source"
    Tags are sourced from both the module object's own `tags` attribute and from merged YAML metadata. Both sources are unioned before the filter is applied.

### Properties

- async: false
- thread_safe: true (Python takes a snapshot under the lock before filtering; TypeScript iterates `_modules` without a lock due to single-threaded runtime)
- pure: false (reads shared mutable state)
- idempotent: true

## Contract: Registry.get_definition

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `module_id` (str/string/&str, required) — the canonical module ID to describe.
- `version_hint` (str/string/&str, optional, default `None`/`null`) — passed through to `get()` for version-aware lookup (Python only; TypeScript omits this parameter).

### Errors

- Any error that `get(module_id)` raises is propagated (e.g., `ModuleNotFoundError` on an empty string).
- No error is raised for a well-formed `module_id` that is simply not registered — returns `None`/`null`.

### Returns

- On success (found): a `ModuleDescriptor` record with the following fields:

| Field | Type | Notes |
|-------|------|-------|
| `module_id` | string | Canonical module ID |
| `name` | string \| null | Human-readable name, or null if not set |
| `description` | string | Plain text, ≤ 200 chars; empty string if absent |
| `documentation` | string \| null | Markdown, ≤ 5000 chars; null if absent |
| `input_schema` | object | JSON Schema object; `{}` if absent |
| `output_schema` | object | JSON Schema object; `{}` if absent |
| `version` | string | Semantic version; defaults to `"1.0.0"` if not set |
| `tags` | string[] | List of tag strings; empty list if absent |
| `annotations` | object \| null | `ModuleAnnotations` or null |
| `examples` | object[] | `ModuleExample[]`; empty list if absent |
| `metadata` | object | Arbitrary extension metadata; `{}` if absent |
| `sunset_date` | string \| null | ISO 8601 date or null |

- On success (not found): `None` (Python), `null` (TypeScript).

!!! note "Schema coercion"
    Python calls `model_rebuild()` on Pydantic schema classes before exporting to ensure forward-references are resolved. The TypeScript implementation reads `inputSchema`/`outputSchema` directly from the module object without additional coercion.

!!! note "Versioned metadata merge"
    In Python, if versioned metadata exists for the resolved version, it is merged on top of the base metadata before constructing the `ModuleDescriptor`. TypeScript reads `_moduleMeta` as-is without version-layer merging.

### Properties

- async: false
- thread_safe: true
- pure: false (may invoke Pydantic `model_rebuild()` as a side effect in Python)
- idempotent: true (for the same registry state, returns the same descriptor)
