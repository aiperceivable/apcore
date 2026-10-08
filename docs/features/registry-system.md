---
description: "Module Registry: filesystem discovery (scan, ID map, metadata, entry-point class, validation, dependency order, register), manual registration, queries, events, hot reload, and describe/get_definition contracts."
---

# Module Registry and Discovery System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §12.2 (Registry component).


## Overview

The Registry is where modules live. It discovers modules from extension directories, accepts modules registered in code, and answers lookups and queries for the executor and for discovery surfaces (manifests, tool exports). It runs lifecycle hooks, emits registration events, and in development can watch the extension directories and reload changed modules.

## Requirements

- Discover modules from the configured extension directories (`extensions.root` / `extensions.roots`).
- Accept modules registered in code (`register()`).
- Merge companion metadata files (`<module>_meta.yaml`) with code-defined metadata.
- Order registration by declared dependencies, rejecting cycles.
- Validate modules before registering them.
- Apply ID-map overrides (`id_map.overrides`) that remap canonical IDs.
- Run lifecycle hooks (`on_load`, `on_unload`) and keep a module invisible until its `on_load` has succeeded.
- Emit `register` / `unregister` events.
- Filter queries by tag, prefix and visibility; produce `ModuleDescriptor`s for external consumers.
- Be safe to read concurrently; single-threaded runtimes (JavaScript) need no lock.

## Discovery pipeline

`discover()` turns files under the extension roots into registered modules. The steps, in the order the SDKs run them:

1. **Scan** — walk each extension root (see [Scanner](#contract-scannerscan_extensions)) and derive a canonical ID from each file path (Algorithm A01). Files that would land in the `ephemeral.*` namespace are rejected.
2. **ID map** — apply `id_map.overrides` (or an ID-map file given to the registry), replacing derived IDs.
3. **Metadata** — load each module's companion `<stem>_meta.yaml` and merge it with code-defined metadata; the YAML wins on conflicting keys.
4. **Entry point** — load the file and pick the module inside it: the single class/export that implements the module interface, or the one named by the metadata's `entry_point`. A file with several candidates and no `entry_point` is not loaded (for several modules per file, see [Multi-Module Discovery](./multi-module-discovery.md)).
5. **Validate** — check each module's structure, or run the custom validator instead if one is installed. Invalid modules are skipped (Python logs a warning; TypeScript drops them silently).
6. **Dependency order** — topologically sort by declared `dependencies`; a cycle raises `CIRCULAR_DEPENDENCY`, a missing required dependency `DEPENDENCY_NOT_FOUND`.
7. **Conflict check** — check the ID grammar and conflicts with already-registered modules; a clashing module is skipped with a warning. (TypeScript runs this check before step 6.)
8. **Register** — register each module in dependency order through the same path as `register()`: `on_load`, then publish, then the `register` event.

Discovery loads no schema files and resolves no package-level plugins — modules come only from the extension roots (and from a custom discoverer, see [Extension System](./extension-system.md#custom-discoverer)).

| SDK | Entry point | Returns |
|---|---|---|
| Python | `registry.discover(path_filter=None)` (sync) | number of modules registered |
| TypeScript | `await registry.discover()` | number of modules registered |
| Rust | `registry.discover_internal().await` after `set_discoverer(...)` + `set_extension_roots(...)`; filesystem discovery uses `DefaultDiscoverer` (configured with `DefaultDiscoverer::from_config(&config)` and a `ModuleFactory` that builds each module) | `Result<usize, ModuleError>` |

Rust has no runtime reflection, so its `DefaultDiscoverer` finds the files and derives the IDs, and a `ModuleFactory` you supply turns each into a module instance.

### Key components

- **Registry** — the module store, the discovery pipeline, registration and queries.
- **Schema export** — builds `ModuleDescriptor`s and exported schemas for external consumers (LLM tool registries, manifests).

### Thread safety

Python guards the store with a re-entrant lock; Rust with interior `RwLock`s (every registry method takes `&self`). Event callbacks and `on_load` run **outside** the lock, so they may call back into the registry. JavaScript is single-threaded and needs no lock.

| Operation | Concurrency |
|-----------|-------------|
| `get()`, `has()`, `list()` | safe |
| `iter()` | snapshot iteration |
| `register()`, `unregister()` | synchronized writes |
| `discover()` | call once at startup; not concurrently with itself |

### Reserved namespaces

| Namespace | Registration rule |
|-----------|-------------------|
| `system.*` | only via `register_internal()` (built-in system modules) |
| `internal.*`, `core.*` | only via `register_internal()` |
| `apcore.*`, `plugin.*`, `schema.*`, `acl.*` | reserved, no current use |
| `ephemeral.*` | only via `register()`; never discovered from disk |

The first seven are the SDKs' `RESERVED_WORDS` (PROTOCOL_SPEC §2.5): a module ID whose first segment is one of them is rejected by `register()` and by discovery. `ephemeral.*` is enforced separately, because its rule is about which path may create it. Ephemeral modules are for agent-synthesized tools and on-the-fly composition (§2.5, §4.4).

### Queries

- `get(module_id)` — direct lookup.
- `list(tags=None, prefix=None, visibility=None)` — sorted module IDs. `tags` requires every listed tag; `prefix` is a plain string prefix; `visibility` is a subset of `["public", "hidden"]` and defaults to `["public"]`, where a module is hidden when its `discoverable` annotation is `false`. Filters combine.
- `has(module_id)`, `count`, `module_ids`, `iter()` — existence, size, and iteration.
- `get_definition(module_id)` — the `ModuleDescriptor`, including schemas.
- `describe(module_id)` — a human-readable description string.

## Contract: Registry.register

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `module_id`: string, required. Must pass module-ID validation (pattern, length, reserved words). Invalid IDs MUST be rejected before any mutation of the registry.
- `module`: Module instance, required. Must implement the module protocol (`description`, `input_schema`, `output_schema`, `execute`).
- `version`: string, optional. See [Multi-version registration](#multi-version-registration).
- `metadata`: mapping, optional. A `dependencies` entry — a list of `{module_id, version?, optional?}` — reaches the registered descriptor as a **parsed** field, so `get_definition(module_id).dependencies` returns what the caller declared (§12.2).

| SDK | Signature |
|---|---|
| Python | `register(module_id, module, version=None, metadata=None, *, context=None) -> None` |
| TypeScript | `register(moduleId, module, version?, metadata?, options?) -> Promise<void>` |
| Rust | `register(name, module, descriptor)`, `register_module(name, module)`, `register_versioned(name, module, version, metadata)` → `Result<(), ModuleError>` |

For ephemeral registration and removal, pass the invoking Context to include its caller identity in the audit event: Python accepts `context=` on `register()` and `unregister()`; TypeScript accepts `{ context }` in their options; Rust exposes `register_with_context()`, `register_module_with_context()` and `unregister_with_context()`, each taking an optional Context reference. The standard executor bootstrap connects these events to the configured event emitter (D-148).

### Preconditions

- The registry's lock MUST be held for the duplicate-ID check.
- `module.on_load()` MUST NOT run until the registry has confirmed the ID is free.
- The module MUST NOT become visible (`get`, `list`, `get_definition`) until `on_load()` has completed successfully. See [Registration ordering invariants](#registration-ordering-invariants).

### Side Effects (ordered)

1. Acquire the registry lock.
2. Validate `module_id` (pattern and length — see `Contract: Executor.call`).
3. Validate module structure, including the `streaming` annotation / `stream()` consistency check.
4. Run the custom validator, if one is installed via `set_validator`.
5. Check for a duplicate `module_id` against both the visible store **and** the in-flight loading set; reject with `DUPLICATE_MODULE_ID`.
6. Reserve `module_id` in the in-flight loading set, so concurrent registrations of the same ID are rejected with `DUPLICATE_MODULE_ID`.
7. Release the lock.
8. Invoke `module.on_load()` if defined — **outside** the lock but **before** the module becomes visible. If it raises: remove `module_id` from the in-flight set, emit `apcore.registry.module_load_failed` with `{module_id, callback_name, error_type, error_message}`, and re-raise.
9. Publish the module into the visible store (briefly re-acquiring the lock) and remove it from the in-flight set. It is now observable via `get`, `list` and `get_definition`.
10. Emit the `register` event.

Steps 2–5 are ordered intrinsic-then-extrinsic: what is wrong with the module itself is reported before what is wrong with where it is being put (D-86). A stateful custom validator is therefore invoked even for a registration that then fails the duplicate check.

### Errors

- `INVALID_MODULE_ID` (`InvalidInputError`) — `module_id` fails validation.
- `DUPLICATE_MODULE_ID` (`InvalidInputError` in Python, `DuplicateModuleIdError` in TypeScript, `ErrorCode::DuplicateModuleId` in Rust) — the ID is already registered or currently loading.
- `STREAMING_INTERFACE_MISMATCH` — the module declares `streaming` but its `stream()` does not satisfy the streaming interface.
- A custom-validator rejection — `GENERAL_INVALID_INPUT` (Python, TypeScript) / `MODULE_LOAD_ERROR` (Rust).
- Whatever `on_load()` raised, unchanged.

### Returns

- `None` (Python), `Promise<void>` (TypeScript), `Ok(())` (Rust).

`on_load` is synchronous in Python and Rust. TypeScript also accepts an async `onLoad`; its `register` returns a promise that resolves once the hook has run, and the module stays invisible until then. Everything else — ID validation, the duplicate check — throws synchronously. Python refuses an `async def on_load` with `MODULE_LOAD_ERROR` rather than publishing a module whose initialisation never ran.

### Properties

- `async`: `false` — except that TypeScript returns a promise to await an async `onLoad`.
- `thread_safe`: `true`.
- `pure`: `false` — mutates the store; runs `on_load`.
- `idempotent`: `false` — a duplicate registration is an error.

### Multi-version registration

§5.4 allows the same `module_id` to be registered with several versions and resolved with a version hint. Only Python implements it:

- **Python** — `register(module_id, module, version=...)` keeps each version; `get(module_id, version_hint=...)` resolves by semantic-version range, and a malformed hint raises `VERSION_CONSTRAINT_INVALID`.
- **TypeScript** — `version` is stored in the module's metadata; a second registration of the same ID is rejected with `DUPLICATE_MODULE_ID`. `get(moduleId, versionHint)` ignores the hint and warns once per module ID that it is deprecated (D-126).
- **Rust** — `register_versioned` stores the version, but a second version of an existing ID is rejected with `DUPLICATE_MODULE_ID`; `get` takes no hint.

Portable code should register one version per ID. `metadata.dependencies` works in all three.

## Contract: Scanner.scan_extensions

!!! info "Internal component"
    Scanning is step 1 of `discover()`. The function is public in Python (`apcore.registry.scanner.scan_extensions`) and Rust (`apcore::registry::scanner::scan_extensions`) and internal in TypeScript. The contract is normative for SDK implementers, not for module authors.

### Inputs

- `root`: path, required — the directory to scan.
- `max_depth`: integer — maximum directory depth (from `extensions.max_depth`, default `8`).
- `follow_symlinks`: boolean — from `extensions.follow_symlinks`, default `false`. A symlink whose target escapes the root is skipped with a warning.
- `ignore_patterns`: list of patterns — from `extensions.ignore_patterns`, matched with Algorithm A25 against each entry's name.
- `extensions` (Rust only): `Option<&[&str]>` — file extensions to accept (default `[".rs"]`; `DefaultDiscoverer::with_extensions` overrides it).

| SDK | Signature |
|---|---|
| Python | `scan_extensions(root, max_depth=8, follow_symlinks=False, ignore_patterns=None) -> list[DiscoveredModule]` |
| TypeScript | `scanExtensions(root, maxDepth = 8, followSymlinks = false, ignorePatterns = [])` |
| Rust | `scan_extensions(root, max_depth, follow_symlinks, extensions, ignore_patterns) -> Result<Vec<DiscoveredFile>, ModuleError>` |

### File selection

| SDK | Accepted files | Always skipped |
|---|---|---|
| Python | `.py` | entries starting with `.` or `_`; `__pycache__/`, `node_modules/`; `*.pyc` |
| TypeScript | `.ts`, `.js` | entries starting with `.` or `_`; `node_modules/`, `__pycache__/`; `*.d.ts`, `*.test.ts`/`.js`, `*.spec.ts`/`.js` |
| Rust | `.rs` (configurable) | entries starting with `.` or `_`; `__pycache__/`, `node_modules/`, `target/`, `.git/`; `*.pyc`, `*.pyo` |

Two files under one root that map to the same ID: the first one found is kept and the other is skipped with a warning; IDs that differ only in case produce a warning.

### Errors

- `CONFIG_NOT_FOUND` — `root` does not exist or is not a directory.

### Returns

- An ordered list of discovered files with their derived canonical IDs (`DiscoveredModule` in Python/TypeScript; `DiscoveredFile` in Rust).

### Properties

- `async`: `false`.
- `thread_safe`: `true`.
- `pure`: `false` — reads the filesystem.

## Usage

=== "Python"
    ```python
    from pydantic import BaseModel

    from apcore import Context, Executor, Registry


    class AddInput(BaseModel):
        a: int
        b: int


    class AddOutput(BaseModel):
        sum: int


    class AddModule:
        description = "Add two integers"
        input_schema = AddInput
        output_schema = AddOutput
        tags = ["math"]

        def execute(self, inputs: dict, context: Context) -> dict:
            return {"sum": inputs["a"] + inputs["b"]}


    registry = Registry(extensions_dir="./extensions")


    def on_register(module_id: str, module: object) -> None:
        print(f"Registered: {module_id}")


    registry.on("register", on_register)
    registry.register("math.add", AddModule())

    module_ids = registry.list()                      # ["math.add"]
    filtered = registry.list(tags=["math"])           # ["math.add"]
    mod = registry.get("math.add")                    # the AddModule instance
    descriptor = registry.get_definition("math.add")  # ModuleDescriptor

    discovered = registry.discover()                  # modules found under ./extensions

    executor = Executor(registry)
    print(executor.call("math.add", {"a": 1, "b": 2}))  # {'sum': 3}
    ```
=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { Executor, Registry } from "apcore-js";

    const registry = new Registry({ extensionsDir: "./extensions" });

    registry.on("register", (moduleId) => {
      console.log(`Registered: ${moduleId}`);
    });

    await registry.register("math.add", {
      description: "Add two integers",
      inputSchema: Type.Object({ a: Type.Integer(), b: Type.Integer() }),
      outputSchema: Type.Object({ sum: Type.Integer() }),
      tags: ["math"],
      execute: (inputs: Record<string, unknown>) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
    });

    const moduleIds = registry.list();                     // ["math.add"]
    const filtered = registry.list({ tags: ["math"] });    // ["math.add"]
    const mod = registry.get("math.add");
    const descriptor = registry.getDefinition("math.add");

    const discovered = await registry.discover();         // modules found under ./extensions

    const executor = new Executor({ registry });
    console.log(await executor.call("math.add", { a: 1, b: 2 })); // { sum: 3 }
    ```
=== "Rust"
    ```rust
    use std::sync::Arc;

    use apcore::registry::Registry;
    use apcore::{Config, Context, Executor, Module, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct AddModule;

    #[async_trait]
    impl Module for AddModule {
        fn description(&self) -> &str { "Add two integers" }
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "a": { "type": "integer" }, "b": { "type": "integer" } }, "required": ["a", "b"] })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "sum": { "type": "integer" } } })
        }
        fn tags(&self) -> Vec<String> { vec!["math".into()] }
        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let sum = inputs["a"].as_i64().unwrap_or(0) + inputs["b"].as_i64().unwrap_or(0);
            Ok(json!({ "sum": sum }))
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let registry = Registry::new();

        // on() returns a handle for off().
        let handle = registry.on(
            "register",
            Box::new(|module_id: &str, _module: &dyn Module| println!("Registered: {module_id}")),
        )?;
        registry.register_module("math.add", Box::new(AddModule))?;

        let module_ids = registry.list(None, None, None);        // ["math.add"]
        let filtered = registry.list(Some(&["math"][..]), None, None);
        let found = registry.has("math.add");                    // true
        let descriptor = registry.get_definition("math.add")?;   // Option<ModuleDescriptor>
        registry.off(handle);

        let executor = Executor::from_registry(Arc::new(registry), Config::from_defaults());
        let out = executor.call("math.add", json!({ "a": 1, "b": 2 }), None, None).await?;
        println!("{out} {module_ids:?} {filtered:?} {found} {}", descriptor.is_some());
        Ok(())
    }
    ```

### Error conditions

| Condition | Error code | Behaviour |
|-----------|------------|-----------|
| Extension root does not exist | `CONFIG_NOT_FOUND` | `discover()` fails |
| Module file fails to import / has no module class | `MODULE_LOAD_ERROR` | logged; the file is skipped |
| Module fails structural or custom validation | — | logged; the module is skipped |
| Discovered ID already registered | — | logged; the module is skipped |
| Explicit `register()` of a registered ID | `DUPLICATE_MODULE_ID` | raised |
| ID-map file invalid | `CONFIG_INVALID` | raised |
| Dependency cycle | `CIRCULAR_DEPENDENCY` | raised |
| `ephemeral.*` file on disk | `INVALID_MODULE_ID` | raised |

## Hot reload (development mode)

`watch()` watches the extension roots for file changes; `unwatch()` stops it. It is for development — production registries should not watch. What happens on a change is language-defined (D11-005):

- **Python** (`watch()` needs the `watchdog` package): re-imports the changed file, calls `on_suspend()` and `on_unload()` on the old instance, `on_load()` on the new one — restoring the old instance if that fails — publishes it, then calls `on_resume(state)`. A deleted file unregisters its module.
- **TypeScript**: unregisters the module and emits `file_changed` with `{ filePath }`; it does not re-import (ES module specifiers cannot be reloaded portably), so the application re-runs discovery itself.
- **Rust**: re-runs `discover_internal()`. New files are registered; modules that are already registered are not replaced.

Whatever the mechanism, a reloaded module MUST NOT become visible before its `on_load()` has run — the same rule as [`register`](#contract-registryregister) step 8 — and recovery from a failed load follows the reload rules of [System Modules](./system-modules.md#reload-failure-semantics) (D-123).

Code that relies on `on_suspend` / `on_resume` firing on file change is portable only on Python. Cross-language hosts should subscribe to `register` / `unregister` (and `file_changed` on TypeScript) and move state explicitly; `system.control.reload_module` calls the suspend/resume hooks in every SDK.

```python
from apcore import Registry

registry = Registry(extensions_dir="./extensions")
registry.discover()

registry.on("register", lambda module_id, module: print(f"Module registered: {module_id}"))
registry.on("unregister", lambda module_id, module: print(f"Module removed: {module_id}"))
registry.watch()

# ... later
registry.unwatch()
```

## Registration ordering invariants

These invariants apply to **every** path that registers a module — `register()`, `register_internal()` (used by system modules), discovery, and hot reload. There are no per-path exceptions: an `on_load` that needs to enumerate sibling modules belongs in a post-discovery hook.

### Visibility

- **MUST** — A module MUST NOT appear in `list()`, `get()`, `get_definition()` or any other discovery API until all of its `on_load` callbacks have completed successfully.
- **MUST** — If an `on_load` callback raises, the module MUST NOT become visible, and the registration call MUST surface the original exception unchanged.
- **MUST** — On callback failure the registry MUST emit `apcore.registry.module_load_failed` carrying:

  | Field | Type | Meaning |
  |-------|------|---------|
  | `module_id` | string | The module ID under which registration was attempted. |
  | `callback_name` | string | Identifier of the failing callback (e.g. `on_load`). |
  | `error_type` | string | The exception class name. |
  | `error_message` | string | The exception message. |
  | `timestamp` | string (ISO 8601 UTC) | Time of failure. |

- The registry does not roll back side effects performed inside `on_load` (connections opened, files written); cleaning up partial state is the callback's job, and the event above gives subscribers a hook.

### Deferred publish (informative)

The invariant is implemented by publishing late rather than by holding the global lock through `on_load`:

1. Briefly take the registry lock to reserve `module_id` in an in-flight set (concurrent registrations of the same ID get `DUPLICATE_MODULE_ID`).
2. Release the lock.
3. Run `on_load` under a per-module lock, so unrelated registrations are not serialized behind it.
4. On success, briefly re-take the registry lock and publish.
5. On failure, re-take the lock to drop the in-flight entry, emit `apcore.registry.module_load_failed`, and re-raise.

### Concurrency across modules

- **MAY** — `on_load` callbacks for **different** modules may run concurrently; the invariant is per module.
- **SHOULD** — SDKs should document that a slow `on_load` blocks only callers waiting on that module, not registration of others.

`on_unload` ordering on unregistration is outside these invariants; see [Module Interface § Lifecycle Hooks](./module-interface.md#lifecycle-hooks).

## Dependencies

- **Executor** — looks modules up at pipeline step 3 (`module_lookup`).
- **Config** — `extensions.*`, `id_map.overrides`.

??? info "Python SDK reference"
    Not a protocol requirement — the `apcore-python` registry package.

    | File | Purpose |
    |------|---------|
    | `registry/registry.py` | `Registry`: discovery pipeline, registration, queries, events, hot reload |
    | `registry/scanner.py` | Extension-root scanning |
    | `registry/metadata.py` | `_meta.yaml` loading and merging; ID-map loading |
    | `registry/dependencies.py` | Topological sort with cycle detection |
    | `registry/entry_point.py` | Importing a discovered file and picking its module class |
    | `registry/schema_export.py` | `ModuleDescriptor` generation and schema export |
    | `registry/validation.py` | Structural module validation |
    | `registry/conflicts.py` | ID conflict detection |
    | `registry/multi_class.py` | [Multi-module discovery](./multi-module-discovery.md) |
    | `registry/types.py` | `ModuleDescriptor` and related types |

    Runtime dependencies: `pyyaml` (metadata and ID-map files); `watchdog` (optional, for `watch()`).

## Testing strategy

- **Discovery** — fixture extension roots with valid modules, invalid modules, dependencies and cycles; ordering, rejection and events.
- **Scanner** — multi-root scanning, skip rules, `ignore_patterns`, depth limit, unreadable directories, symlinks escaping the root.
- **Metadata** — `_meta.yaml` loading, merge precedence, malformed files.
- **Dependencies** — linear chains, diamonds, wide graphs, cycles reported with their path.
- **Thread safety** — concurrent register / unregister / query.
- **Events** — callbacks receive the right arguments; a failing callback does not break the registry; unknown event names are rejected.
- **ID map** — remapped IDs are used by queries.

## Contract: Registry.get

### Inputs

- `module_id` (required) — the canonical module ID.
- `version_hint` (optional, Python only) — see [Multi-version registration](#multi-version-registration).

| SDK | Signature |
|---|---|
| Python | `get(module_id, version_hint=None) -> Any \| None` |
| TypeScript | `get(moduleId, versionHint?) -> unknown \| null` (hint ignored, deprecated) |
| Rust | `get(&self, name) -> Result<Option<Arc<dyn Module>>, ModuleError>` |

### Errors

- `MODULE_NOT_FOUND` (`ModuleNotFoundError`) — `module_id` is the empty string. Empty IDs are never accepted silently.
- No error for a well-formed ID that is not registered — `get` returns `None` / `null` / `Ok(None)`.

### Returns

- The registered module instance, or `None` / `null` / `Ok(None)`. A module whose `on_load` is still running is not returned.

### Properties

- async: false
- thread_safe: true
- pure: false (reads shared state under the lock)
- idempotent: true

## Contract: Registry.list

### Inputs

- `tags` (optional) — only modules carrying **every** listed tag; an empty list means no tag filter. Tags come from both the module's own `tags` and its merged metadata.
- `prefix` (optional) — plain string prefix on `module_id` (not a glob).
- `visibility` (optional) — subset of `["public", "hidden"]`, default `["public"]`.

### Errors

- None. Filters that match nothing return an empty list.

### Returns

- A lexicographically sorted list of unique module IDs.

### Properties

- async: false
- thread_safe: true (Python snapshots under the lock)
- pure: false (reads shared state)
- idempotent: true

## Contract: Registry.get_definition

### Inputs

- `module_id` (required).
- `version_hint` (optional, Python only; TypeScript accepts and ignores it).

### Errors

- Whatever `get(module_id)` raises (e.g. `MODULE_NOT_FOUND` for an empty ID).
- No error for an unregistered ID — returns `None` / `null` / `Ok(None)`.

### Returns

A `ModuleDescriptor`:

| Field | Type | Notes |
|-------|------|-------|
| `module_id` | string | Canonical module ID |
| `name` | string \| null | Human-readable name |
| `description` | string | Plain text, ≤ 200 chars; empty string if absent |
| `documentation` | string \| null | Markdown, ≤ 5000 chars |
| `input_schema` | object | JSON Schema; `{}` if absent |
| `output_schema` | object | JSON Schema; `{}` if absent |
| `version` | string | Semantic version; default `"1.0.0"` |
| `tags` | string[] | Empty list if absent |
| `annotations` | object \| null | `ModuleAnnotations` |
| `examples` | object[] | `ModuleExample[]` |
| `metadata` | object | Free-form extension metadata |
| `sunset_date` | string \| null | ISO 8601 date, from `metadata["x-deprecation"].sunset_date` |
| `dependencies` | object[] | Parsed `{module_id, version?, optional?}` records; `[]` if none |

Rust's `ModuleDescriptor` also carries `display` and a runtime-only `enabled` flag.

### Deprecation warning cadence

When a module carries `metadata["x-deprecation"]`, `get_definition` emits a deprecation warning (D-89):

- The warning is emitted from `get_definition` (the read), never from `register` — registration often happens before the host has installed a log handler, and a warning lost there could not be re-emitted.
- It is emitted at most once per registry instance for each `(module_id, version, x-deprecation block)`, with blocks compared by value. Re-registering with the same notice does not warn again; a changed or new notice does.
- The dedupe state is **not** cleared on `unregister`, so hot reload does not re-warn.

A module that is registered but whose definition is never read is not warned about.

### Properties

- async: false
- thread_safe: true
- pure: false (Python may call Pydantic `model_rebuild()` while exporting)
- idempotent: true

## Version constraint validation

A version constraint operand **MUST** begin with a digit; `"latest"`, `"v1.0.0"` and `""` are malformed, and an implementation **MUST NOT** resolve a malformed constraint to a comparison (D-85).

Each SDK exposes a fallible form that reports it as `VERSION_CONSTRAINT_INVALID` — `VersionConstraintError` in Python and TypeScript, the `try_*` functions (`try_matches_version_hint`, `try_select_best_version`) in Rust — so a caller can tell "this constraint is nonsense" from "this version does not satisfy it". A non-fallible convenience form, where one exists, fails closed (treats the constraint as unsatisfied) and logs a warning.

## Registry events

The event set is **closed** (D-80):

| Event | Emitted when | Callback arguments |
|---|---|---|
| `register` | a module becomes visible | `(module_id, module)` |
| `unregister` | a module is removed | `(module_id, module)` |
| `file_changed` | a watched file changed and the registry does not re-register it itself (TypeScript's notify-only `watch()`) | `(moduleId, { filePath })` |

1. `on` / `off` **MUST** reject an event name outside this set with `InvalidInputError(code=GENERAL_INVALID_INPUT)`, so a typo does not become a silently dead subscription.
2. An implementation **MUST** accept in `on` every event name it can itself emit — which is why TypeScript accepts `file_changed` and Python and Rust (whose `watch()` re-registers or re-discovers) do not.
3. An implementation whose `watch()` re-registers the module emits `unregister` / `register` and **MUST NOT** also emit `file_changed` for the same change.

| SDK | Subscribe | Unsubscribe |
|---|---|---|
| Python | `on(event, callback) -> None` | `off(event, callback) -> bool` |
| TypeScript | `on(event, callback): void` | `off(event, callback): boolean` |
| Rust | `on(&self, event, Box<callback>) -> Result<u64, ModuleError>` | `off(&self, handle) -> bool` |

## Contract: Registry.describe

### Inputs
- `module_id` (required) — the canonical module ID.

### Errors
- `MODULE_NOT_FOUND` — no module registered under `module_id`.

### Returns
- A **human-readable description string** (Rust: `Result<String, ModuleError>`). The machine-readable accessor is [`get_definition`](#contract-registryget_definition); `describe` **MUST NOT** return a structured object (D-77).

### Module-supplied override

A module MAY implement its own `describe()` ([Module Interface](./module-interface.md#optional-methods)). That method's declared return is an introspection mapping, not a string, so the registry **MUST NOT** pass a structured return through as the description:

1. If the module implements `describe()` **and** it returns a string, return that string verbatim.
2. Otherwise (no `describe()`, a null return, a structured return, or one that cannot be resolved synchronously) fall back to the generated description.

An implementation **MUST NOT** stringify a structured return.

### Properties
- async: false
- thread_safe: true
- pure: true (read-only rendering)
- idempotent: true
