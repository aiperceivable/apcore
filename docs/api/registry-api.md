# Registry API

> **Canonical Definition** - This document is the authoritative definition of the Registry interface

> Registry is responsible for module registration, discovery, and management.

## 1. Interface Overview

```python
from typing import Type, Iterator
from apcore import Module


class Registry:
    """Module registry"""

    def __init__(
        self,
        config: "Config | None" = None,
        extensions_dir: str | None = "./extensions",
        extensions_dirs: list[str | dict] | None = None,
        id_map_path: str | None = None
    ) -> None:
        """
        Initialize Registry

        Args:
            config: Framework configuration (optional). When provided,
                registry settings are read from config.registry.
            extensions_dir: Single root directory path (backward compatible). Set to None for no directory binding (manual registration only).
            extensions_dirs: Multiple root directory list (mutually exclusive with extensions_dir).
                Each element can be a path string (namespace auto-derived from directory name) or
                dict (e.g., {"root": "./extensions", "namespace": "core"}).
            id_map_path: ID Map configuration file path (optional)
        """
        ...

    # ============ Discovery and Registration ============

    def discover(self) -> int:
        """Auto-discover and register all modules under extensions_dir"""
        ...

    def register(self, module_id: str, module: Module) -> None:
        """Manually register a single module"""
        ...

    def unregister(self, module_id: str) -> bool:
        """Unregister a module"""
        ...

    def register_internal(self, module_id: str, module: Module) -> None:
        """
        Register a module bypassing reserved word checks

        Used for system module registration (system.* namespace).
        Normal user code should use register() instead.
        """
        ...

    # ============ Query ============

    def get(self, module_id: str) -> Module | None:
        """Get module instance"""
        ...

    def get_definition(self, module_id: str) -> "ModuleDescriptor | None":
        """Get module definition descriptor (cross-language compatible, replaces get_class)"""
        ...

    def has(self, module_id: str) -> bool:
        """Check if module exists"""
        ...

    def list(
        self,
        tags: list[str] | None = None,
        prefix: str | None = None
    ) -> list[str]:
        """List module IDs"""
        ...

    def iter(self) -> Iterator[tuple[str, Module]]:
        """Iterate over all modules"""
        ...

    # ============ Schema Query and Export (OPTIONAL) ============
    #
    # These are convenience facades. Implementations may delegate to
    # SchemaLoader/SchemaExporter instead of providing these directly
    # on the Registry. They are marked OPTIONAL — implementations may
    # provide them as independent helper functions, e.g.:
    #   get_schema(registry, module_id)
    #   export_schema(registry, module_id, format=...)

    def get_schema(self, module_id: str) -> dict | None:
        """OPTIONAL. Get module Schema (structured dict, for in-program processing)"""
        ...

    def get_all_schemas(self) -> dict[str, dict]:
        """OPTIONAL. Get all module Schemas (structured dict)"""
        ...

    def export_schema(
        self,
        module_id: str,
        format: str = "json",
        strict: bool = False,
        compact: bool = False,
        profile: str | None = None
    ) -> str:
        """OPTIONAL. Export module Schema (serialized string, for transmission/storage)"""
        ...

    def export_all_schemas(
        self,
        format: str = "json",
        strict: bool = False,
        compact: bool = False,
        profile: str | None = None
    ) -> str:
        """OPTIONAL. Export all module Schemas (serialized string)"""
        ...

    # ============ Module Control ============

    def disable(self, module_id: str) -> None:
        """
        Disable a module without unloading it

        Disabled modules remain registered but calls raise ModuleDisabledError.
        Thread-safe via ToggleState.

        Optional. Implementations may handle module toggling at the APCore
        client level instead. See client-api.md disable()/enable().

        Low-level API that bypasses approval and events. For production use
        with full audit trail, use APCore.disable() which routes through
        system.control.toggle_feature.
        """
        ...

    def enable(self, module_id: str) -> None:
        """
        Re-enable a previously disabled module

        Optional. Implementations may handle module toggling at the APCore
        client level instead. See client-api.md disable()/enable().
        """
        ...

    # ============ Safe Hot-Reload ============

    def safe_unregister(self, module_id: str, timeout_ms: int = 5000) -> bool:
        """
        Safely unregister with cooperative drain (Algorithm A21)

        Waits for in-flight executions to complete (up to timeout_ms)
        before removing the module. Returns True if successful.
        """
        ...

    def acquire(self, module_id: str) -> "ContextManager[Module]":
        """
        Context manager tracking in-flight executions

        Used by Executor to ensure safe_unregister() can wait for
        completion before removing a module.
        """
        ...

    def is_draining(self, module_id: str) -> bool:
        """Check if module is marked for unload"""
        ...

    # ============ Introspection ============

    def describe(self, module_id: str) -> str:
        """
        Return markdown-formatted human-readable module description

        Useful for AI/LLM tool discovery. Includes description,
        documentation, input/output schemas, and annotations.
        """
        ...

    def negotiate_version(
        self,
        module_id: str,
        version_hint: str,
    ) -> "Module":
        """
        Version negotiation for SDK/module compatibility (Algorithm A14)

        Resolves the best matching module version given a constraint string.
        Raises VersionIncompatibleError if no compatible version found.

        Optional. Implementations may provide this as a standalone utility
        function instead.
        """
        ...

    # ============ Event Callbacks ============

    def on(self, event: str, callback: "Callable") -> None:
        """
        Register event callback

        Args:
            event: Event name ("register" | "unregister")
            callback: Callback function, signature (module_id: str, module: Module) -> None
        """
        ...

    # ============ Properties ============

    @property
    def count(self) -> int:
        """Number of registered modules"""
        ...

    @property
    def module_ids(self) -> list[str]:
        """List of all module IDs"""
        ...
```

---

## 2. Initialization

### 2.1 Basic Initialization

```python
from apcore import Registry

# Use default configuration (single root directory ./extensions, backward compatible)
registry = Registry()

# Specify module directory
registry = Registry(extensions_dir="./src/extensions")

# No directory binding (manual registration only)
registry = Registry(extensions_dir=None)

# Multiple root directory mode (namespace auto-derived from directory name)
registry = Registry(extensions_dirs=["./extensions", "./plugins"])
# → extensions.executor.email.send_email, plugins.my_tool

# Multiple root directories + explicit namespace override
registry = Registry(extensions_dirs=[
    {"root": "./extensions", "namespace": "core"},  # core.executor.email.send_email
    "./plugins"                                      # plugins.my_tool
])

# Specify ID Map
registry = Registry(
    extensions_dir="./extensions",
    id_map_path="./config/id_map.yaml"
)
```

### 2.2 Configuration File

```yaml
# apcore.yaml
registry:
  extensions_dir: ./extensions
  id_map_path: ./config/id_map.yaml
  auto_discover: true
  watch: false  # Whether to monitor file changes
```

```python
from apcore import Registry, Config

config = Config.load("apcore.yaml")
registry = Registry(**config.registry)
```

---

## 3. Module Discovery

### 3.1 Auto Discovery

```python
registry = Registry(extensions_dir="./extensions")

# Scan extensions directory, auto-register all modules
count = registry.discover()
print(f"Discovered {count} modules")
```

**Discovery Rules:**

| Rule | Description |
|------|------|
| Directory structure | Recursively scan all subdirectories |
| File types | `.py` files (Python), `.ts`/`.js` files (TypeScript/JavaScript) |
| Module identification | Find classes conforming to the `Module` interface |
| ID generation | File path converted to module ID |
| Ignore rules | Skip `__pycache__`, `node_modules`, files starting with `_` |

**Path to ID Conversion:**

```
extensions/executor/email/send_email.py
  ↓
executor.email.send_email

extensions/api/handler/user_api.py
  ↓
api.handler.user_api
```

### 3.2 ID Map Override

When ID Map exists, use configured IDs:

```yaml
# config/id_map.yaml
mappings:
  - file: extensions/executor/email/send_email.py
    id: email.send
    class: SendEmailModule

  - file: extensions/legacy/old_module.py
    id: legacy.old
    class: OldModule
```

```python
registry = Registry(
    extensions_dir="./extensions",
    id_map_path="./config/id_map.yaml"
)
registry.discover()

# Use configured ID
module = registry.get("email.send")
```

---

## 4. Manual Registration

### 4.1 Register Module Class

```python
from apcore import Registry, Module
from pydantic import BaseModel, Field


class MyInput(BaseModel):
    name: str = Field(..., description="Name")


class MyOutput(BaseModel):
    greeting: str = Field(..., description="Greeting")


class GreetingModule(Module):
    """Greeting module"""
    input_schema = MyInput
    output_schema = MyOutput

    def execute(self, inputs: dict, context) -> dict:
        return {"greeting": f"Hello, {inputs['name']}!"}


# Manual registration
registry = Registry()
registry.register("custom.greeting", GreetingModule())
```

### 4.2 Unregister Module

```python
# Unregister module
success = registry.unregister("custom.greeting")
if success:
    print("Module unregistered")
else:
    print("Module not found")
```

---

## 5. Query Modules

### 5.1 Get Module Instance

```python
# Get module instance
module = registry.get("executor.email.send_email")
if module:
    # Direct execution (not recommended, should use Executor)
    result = module.execute(inputs, context)
```

### 5.2 Get Module Definition Descriptor

```python
# Get module definition descriptor (cross-language compatible)
# Returns ModuleDescriptor, contains module type information, applicable to all languages
# (Class in Python, struct in Rust, struct in Go, function pointer in C)
definition = registry.get_definition("executor.email.send_email")
if definition:
    print(f"Module: {definition.name}")
    print(f"Input Schema: {definition.input_schema}")
    print(f"Output Schema: {definition.output_schema}")
    print(f"Description: {definition.description}")
    print(f"Annotations: {definition.annotations}")
```

### 5.3 Check Module Existence

```python
if registry.has("executor.email.send_email"):
    print("Module exists")
else:
    print("Module not found")
```

### 5.4 List Modules

```python
# List all modules
all_modules = registry.list()
print(all_modules)
# ["executor.email.send_email", "executor.sms.send_sms", ...]

# Filter by prefix
email_modules = registry.list(prefix="executor.email")
print(email_modules)
# ["executor.email.send_email", "executor.email.send_template"]

# Filter by tags
notification_modules = registry.list(tags=["notification"])
print(notification_modules)
# ["executor.email.send_email", "executor.sms.send_sms"]

# Combined filtering
executor_notifications = registry.list(
    prefix="executor",
    tags=["notification"]
)
```

### 5.5 Iterate Modules

```python
# Iterate over all modules
for module_id, module in registry.iter():
    print(f"{module_id}: {module.description}")
```

---

## 6. Schema Query and Export

> **`get_schema()` vs `export_schema()`**: `get_schema()` returns structured `dict` for in-program processing (e.g., passing to LLM, validators); `export_schema()` returns serialized string (JSON/YAML) for transmission, storage, or file export.

### 6.1 Get Structured Schema

```python
# Get single module's structured Schema (dict)
schema = registry.get_schema("executor.email.send_email")
if schema:
    print(schema["input_schema"]["properties"])
    # Can pass directly to LLM as tool definition

# Get all module Schemas (structured)
all_schemas = registry.get_all_schemas()
# {"executor.email.send_email": {...}, "executor.sms.send_sms": {...}}
```

### 6.2 Export Serialized Schema

```python
# JSON format (string)
json_schema = registry.export_schema(
    module_id="executor.email.send_email",
    format="json"
)
print(json_schema)
```

```json
{
  "module_id": "executor.email.send_email",
  "name": "Send Email",
  "description": "Send email module",
  "version": "1.0.0",
  "tags": ["email", "notification"],
  "input_schema": {
    "type": "object",
    "properties": {
      "to": {
        "type": "string",
        "description": "Recipient email address"
      },
      "subject": {
        "type": "string",
        "description": "Email subject"
      },
      "body": {
        "type": "string",
        "description": "Email body"
      }
    },
    "required": ["to", "subject", "body"]
  },
  "output_schema": {
    "type": "object",
    "properties": {
      "success": {
        "type": "boolean",
        "description": "Whether sending was successful"
      },
      "message_id": {
        "type": "string",
        "description": "Message ID"
      }
    },
    "required": ["success"]
  }
}
```

### 6.3 Export YAML Format

```python
yaml_schema = registry.export_schema(
    module_id="executor.email.send_email",
    format="yaml"
)
print(yaml_schema)
```

```yaml
module_id: executor.email.send_email
name: Send Email
description: Send email module
version: "1.0.0"
tags:
  - email
  - notification
input_schema:
  type: object
  properties:
    to:
      type: string
      description: Recipient email address
    subject:
      type: string
      description: Email subject
    body:
      type: string
      description: Email body
  required:
    - to
    - subject
    - body
output_schema:
  type: object
  properties:
    success:
      type: boolean
      description: Whether sending was successful
    message_id:
      type: string
      description: Message ID
  required:
    - success
```

### 6.4 Export All Schemas

```python
# Export all module Schemas (serialized string, for AI/LLM)
all_schemas_json = registry.export_all_schemas(format="json")

# Or get structured data to pass directly to LLM
all_schemas = registry.get_all_schemas()
```

### 6.5 Strict Mode Export

OpenAI and Anthropic's `strict: true` mode requires Schemas to meet additional constraints (all nested objects set `additionalProperties: false`, all fields must be in `required`). Use the `strict=True` parameter to automatically apply transformations.

```python
# Strict Mode export (for OpenAI / Anthropic strict: true)
strict_schema = registry.export_schema(
    module_id="executor.email.send_email",
    strict=True
)
# Optional fields auto-converted to required + nullable, x-* fields auto-stripped
```

Transformation rules detailed in [PROTOCOL_SPEC §4.16](../../PROTOCOL_SPEC.md) and [Algorithm A23](../spec/algorithms.md#a23-to_strict_schema--strict-mode-conversion).

### 6.6 Compact Export

During module discovery phase, AI typically only needs basic information to decide whether to invoke. Use `compact=True` to export condensed Schema, reducing token consumption:

```python
# Compact export (module discovery phase)
compact_schema = registry.export_schema(
    module_id="executor.email.send_email",
    compact=True
)

# Load full Schema after selecting module
full_schema = registry.export_schema(
    module_id="executor.email.send_email"
)
```

**Compact Export (`compact=True`) Transformation Rules:**

| Transformation | Description |
|------|------|
| Strip `x-*` extension fields | Remove `x-llm-description`, `x-examples`, `x-constraints`, `x-sensitive` |
| Truncate `description` | Keep only first sentence (content before first period or newline) |
| Remove `documentation` | Detailed documentation not needed in discovery phase |
| Remove `examples` | Examples not needed in discovery phase |

This naturally aligns with apcore's progressive disclosure design (`description` → `documentation` → `examples`): use `compact` in discovery phase, full Schema in decision phase.

### 6.7 Profile Export

Use `profile` parameter to export for target AI protocol, automatically applying corresponding transformation rules:

```python
# Export by protocol profile
mcp_schema = registry.export_schema("executor.email.send_email", profile="mcp")
openai_schema = registry.export_schema("executor.email.send_email", profile="openai")
anthropic_schema = registry.export_schema("executor.email.send_email", profile="anthropic")
```

Profile definitions detailed in [PROTOCOL_SPEC §4.17](../../PROTOCOL_SPEC.md).

> **Note**: `profile` parameter is mutually exclusive with `strict`/`compact`. When `profile` is specified, transformation rules are determined by profile (e.g., `openai` profile automatically includes strict transformation).

---

## 7. Event Callbacks

Registry supports event callbacks for executing custom logic when modules are registered and unregistered:

```python
# Register event callbacks
registry.on("register", lambda module_id, module: print(f"Registered: {module_id}"))
registry.on("unregister", lambda module_id, module: print(f"Unregistered: {module_id}"))

# Subsequent register/unregister operations will trigger callbacks
registry.register("custom.module", MyModule())  # Triggers "register" callback
registry.unregister("custom.module")          # Triggers "unregister" callback
```

| Event | Trigger Time | Callback Signature |
|------|---------|---------|
| `register` | After module successfully registered | `(module_id: str, module: Module) -> None` |
| `unregister` | After module successfully unregistered | `(module_id: str, module: Module) -> None` |

> **Design Note**: Uses event callbacks (`on("register", ...)`) rather than separate Hook system, keeping API concise. This pattern is consistent with industry conventions like Node.js EventEmitter, Python signals, etc.

---

## 8. Module Lifecycle

### 8.1 Loading Flow

```
registry.discover()
    ↓
Scan extensions_dir
    ↓
Find classes conforming to the Module interface
    ↓
Generate/lookup module_id (directory as ID / ID Map)
    ↓
Validate module interface (input_schema, output_schema, description, execute)
    ↓
Instantiate module
    ↓
Call on_load()
    ↓
Register to Registry
```

### 8.2 Unloading Flow

```
registry.unregister(module_id)
    ↓
Call on_unload()
    ↓
Remove from Registry
```

### 8.3 Error Handling

```python
from apcore import Registry, ModuleLoadError

registry = Registry(extensions_dir="./extensions")

try:
    registry.discover()
except ModuleLoadError as e:
    print(f"Failed to load module: {e.module_path}")
    print(f"Error: {e.message}")
    print(f"Validation errors: {e.validation_errors}")
```

### 8.4 Discovery Algorithm

Registry **must** discover modules according to the following algorithm:

```
Algorithm: discover_modules(extensions_dir, config)

Steps:
  1. file_list ← scan_extensions(extensions_dir, config)    // See PROTOCOL_SPEC §3.6
  2. modules ← []
  3. For each (file_path, canonical_id) ∈ file_list:
     a. entry_point ← resolve_entry_point(null, file_path)  // See PROTOCOL_SPEC §5.2
     b. module_class ← dynamically load class pointed to by entry_point
     c. errors ← validate_module(module_class)               // See §7 Interface Validation
     d. If errors non-empty → log warning, skip this module
     e. Otherwise → modules.append((canonical_id, module_class))
  4. load_order ← resolve_dependencies(modules)              // See PROTOCOL_SPEC §5.3
  5. Instantiate and register modules in load_order
  6. Return number of registered modules
```

### 8.5 Thread Safety Specifications

| Operation | Thread Safe | Description |
|------|---------|------|
| `get()` | **MUST** be safe | Read-only query |
| `has()` | **MUST** be safe | Read-only query |
| `list()` | **MUST** be safe | Read-only query |
| `iter()` | **SHOULD** be safe | Snapshot iteration |
| `discover()` | **MUST NOT** be concurrent | Called once at startup |
| `register()` | **SHOULD** be safe | Write operation needs synchronization |
| `unregister()` | **SHOULD** be safe | Write operation needs synchronization |

### 8.6 Error Condition Table

| Condition | Error Code | Description |
|------|--------|------|
| extensions_dir does not exist | `CONFIG_NOT_FOUND` | Extensions directory must exist |
| Module file syntax error | `MODULE_LOAD_ERROR` | Log warning and skip |
| Module interface incomplete | `MODULE_LOAD_ERROR` | Missing required attributes |
| Module ID conflict | `MODULE_LOAD_ERROR` | Same ID registered twice |
| ID Map file format error | `CONFIG_INVALID` | ID Map YAML parsing failed |
| Circular dependency | `CIRCULAR_DEPENDENCY` | Circular dependency exists between modules |

---

## 9. Advanced Usage

### 9.1 Custom Discoverer

> Implemented in apcore-python v0.5.1+ and apcore-typescript v0.3.0+.

```python
from apcore import Registry, ModuleDiscoverer


class CustomDiscoverer(ModuleDiscoverer):
    """Custom module discoverer"""

    def discover(self, path: str) -> list[tuple[str, Type[Module]]]:
        """
        Discover modules

        Returns:
            [(module_id, module_class), ...]
        """
        # Custom discovery logic
        modules = []
        # ...
        return modules


registry = Registry(extensions_dir="./extensions")
registry.set_discoverer(CustomDiscoverer())
registry.discover()
```

### 9.2 Module Validator

> Implemented in apcore-python v0.5.1+ and apcore-typescript v0.3.0+.

```python
from apcore import Registry, ModuleValidator


class StrictValidator(ModuleValidator):
    """Strict module validator"""

    def validate(self, module_class: Type[Module]) -> list[str]:
        errors = super().validate(module_class)

        # Add custom validation rules
        if not module_class.tags:
            errors.append("Module must have at least one tag")

        if not module_class.__doc__ or len(module_class.__doc__) < 20:
            errors.append("Module description must be at least 20 characters")

        return errors


registry = Registry(extensions_dir="./extensions")
registry.set_validator(StrictValidator())
registry.discover()
```

### 9.3 Hot Reload (Development Mode)

> Implemented in apcore-python v0.5.1+ and apcore-typescript v0.3.0+. Python requires the optional `watchdog` dependency.

```python
from apcore import Registry

registry = Registry(extensions_dir="./extensions")
registry.discover()

# Enable file watching (development mode)
# Change callbacks are registered separately via registry.on(event, callback)
registry.on("change", lambda module_id: print(f"Module changed: {module_id}"))
registry.on("add", lambda module_id: print(f"Module added: {module_id}"))
registry.on("remove", lambda module_id: print(f"Module removed: {module_id}"))
registry.watch()

# Stop watching
registry.unwatch()
```

---

## 10. Complete Example

```python
from apcore import Registry, Executor

# 1. Create Registry
registry = Registry(
    extensions_dir="./extensions",
    id_map_path="./config/id_map.yaml"
)

# 2. Discover modules
count = registry.discover()
print(f"Discovered {count} modules")

# 3. View registered modules
print("\n=== Registered Modules ===")
for module_id in registry.list():
    module = registry.get(module_id)
    print(f"  {module_id}: {module.description}")

# 4. View by category
print("\n=== Email Modules ===")
for module_id in registry.list(prefix="executor.email"):
    print(f"  {module_id}")

# 5. Export Schemas (for AI/LLM use)
schemas = registry.export_all_schemas(format="json")
print(f"\nExported {registry.count} module schemas")

# 6. Create Executor and use
executor = Executor(registry)
result = executor.call(
    module_id="executor.email.send_email",
    inputs={
        "to": "user@example.com",
        "subject": "Hello",
        "body": "World"
    }
)
print(f"\nResult: {result}")
```

## 11. Edge Case Handling

Implementations **must** handle Registry edge cases according to the following table:

| Scenario | Behavior | Level |
|------|------|------|
| `register()` with existing `module_id` | Throw `GENERAL_INVALID_INPUT` ("Module already exists") | **MUST** |
| `unregister()` non-existent `module_id` | Silently succeed (idempotent) | **MUST** |
| `get()` passed empty string `""` | Throw `MODULE_NOT_FOUND` | **MUST** |
| `list()` called during `discover()` | Return modules with completed discovery (partial list) | **SHOULD** |
| `discover()` scans 0 modules | Log WARN, do not throw exception | **MUST** |
| Scan depth exceeds `max_depth` | Stop recursion, log INFO | **MUST** |
| Module directory insufficient permissions (no read access) | Log ERROR, skip directory, continue scanning | **MUST** |

**Hot Reload Considerations:**
- During `unregister()`, module may be executing, see [PROTOCOL_SPEC §12.7.4 Hot Reload Race Conditions](../../PROTOCOL_SPEC.md#1274-hot-reload-race-conditions)
- Safe unloading algorithm see [algorithms.md A21 — safe_unregister()](../spec/algorithms.md#a21-safe_unregister--hot-reload-safe-unloading)

---

## Next Steps

- [Executor API](./executor-api.md) - Module executor
- [Module Interface Definition](./module-interface.md) - How to implement modules
- [Creating Modules Guide](../guides/creating-modules.md) - Module creation tutorial
