# Architecture Design

> apcore internal architecture and component interactions.

## 1. Overall Architecture

apcore's architecture comprises two orthogonal dimensions: **framework technical layers** (vertical) and **suggested business layers** (horizontal).

### 1.1 Framework Technical Layers (Vertical)

The framework's own technical layering, defining the complete flow from module registration to execution:

```
┌─────────────────────────────────────────────────────────────────┐
│                    Application Layer                              │
│   ┌─────────────────┐  ┌─────────────────┐  ┌────────────────┐  │
│   │   HTTP API      │  │   CLI           │  │   MCP Server   │  │
│   └────────┬────────┘  └────────┬────────┘  └───────┬────────┘  │
│            │                    │                    │           │
│            └────────────────────┼────────────────────┘           │
│                                 ▼                                │
├─────────────────────────────────────────────────────────────────┤
│                    Execution Layer                                │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │                       Executor                           │   │
│   │  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌─────────┐│   │
│   │  │  ACL   │ │Approval│ │Middlew.│ │Validate│ │ Execute ││   │
│   │  │  Check │→│  Gate  │→│ Before │→│  Input │→│ Module  ││   │
│   │  └────────┘ └────────┘ └────────┘ └────────┘ └─────────┘│   │
│   └─────────────────────────────────────────────────────────┘   │
│                                 ▲                                │
│                                 │ Look up module                 │
├─────────────────────────────────┼───────────────────────────────┤
│                    Registry Layer                                │
│   ┌─────────────────────────────┴─────────────────────────────┐ │
│   │                       Registry                             │ │
│   │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  │ │
│   │  │ Discover │  │ ID Map   │  │ Validate │  │ Modules  │  │ │
│   │  │          │→ │          │→ │          │→ │ Store    │  │ │
│   │  └──────────┘  └──────────┘  └──────────┘  └──────────┘  │ │
│   └───────────────────────────────────────────────────────────┘ │
│                                 ▲                                │
│                                 │ Read                           │
├─────────────────────────────────┼───────────────────────────────┤
│                    Module Layer                                  │
│   ┌─────────────────────────────┴─────────────────────────────┐ │
│   │         User-written business modules (Module interface)   │ │
│   │                      (extensions/)                         │ │
│   └───────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

### 1.2 Suggested Business Layers (Horizontal)

In the `extensions/` directory, it is recommended to divide modules by responsibility (enforced by ACL):

```
extensions/
│
├── api/                    # API Layer: handles external requests
│   ├── handler/
│   └── ACL: can only call orchestrator.*
│
├── orchestrator/           # Orchestration Layer: composes business workflows
│   ├── workflow/
│   └── ACL: can only call executor.* and common.*
│
├── executor/               # Execution Layer: concrete business operations
│   ├── email/
│   ├── sms/
│   ├── database/
│   └── ACL: can call common.*, can connect to external systems
│
└── common/                 # Common Layer: utility and helper functions
    ├── util/
    └── ACL: read-only operations, called by all layers
```

**Key Points:**
- **Framework technical layers** (Application → Execution → Registry → Module) are apcore's **implementation mechanism**
- **Business layers** (api → orchestrator → executor → common) are **best practice recommendations**, enforced by ACL configuration
- Both are orthogonal: modules in any business layer go through the same framework layers

---

## 2. Core Components

### 2.1 Module

Modules are the smallest execution unit in apcore. Modules can be defined via the `@module` decorator (primary approach) or by providing a class with an `execute()` method and required schema attributes. No abstract base class inheritance is required.

**Decorator approach (recommended):**

```python
from apcore import module

@module(id="executor.greet", tags=["greeting"])
def greet(name: str) -> dict:
    """Generate greeting message"""
    return {"message": f"Hello, {name}!"}
```

**Class-based approach (no ABC required):**

```python
class SendEmailModule:
    """Send email module"""

    # ====== Core Layer (Required) ======
    input_schema = SendEmailInput       # Type[BaseModel] or JSON Schema dict
    output_schema = SendEmailOutput     # Type[BaseModel] or JSON Schema dict
    description = "Send email to specified recipient"

    # ====== Annotation Layer (Optional) ======
    name = None                         # Human-readable name
    tags = ["email"]                    # Tag list
    version = "1.0.0"                   # Semantic version
    annotations = None                  # ModuleAnnotations instance
    examples = []                       # Usage examples

    # ====== Extension Layer (Optional) ======
    metadata = {}                       # Free metadata

    # Core methods (def or async def both supported, framework auto-detects)
    def execute(self, inputs: dict, context: Context) -> dict: ...
    def validate(self, inputs: dict) -> ValidationResult: ...

    # Lifecycle
    def on_load(self) -> None: ...
    def on_unload(self) -> None: ...
```

**Responsibilities:**
- Define input/output Schema
- Implement concrete business logic
- Manage its own resources (connection pools, etc.)

---

### 2.2 Registry

Registry is responsible for module discovery, registration, and management.

```
┌─────────────────────────────────────────────────────────────┐
│                         Registry                             │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌──────────────┐     ┌──────────────┐     ┌─────────────┐ │
│  │  Discoverer  │────▶│   ID Map     │────▶│  Validator  │ │
│  └──────────────┘     └──────────────┘     └─────────────┘ │
│         │                                         │         │
│         ▼                                         ▼         │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                   Module Store                        │  │
│  │                                                       │  │
│  │   module_id        │  module_class  │  instance       │  │
│  │   ─────────────────┼────────────────┼──────────────   │  │
│  │   executor.email   │  SendEmail...  │  <instance>     │  │
│  │   executor.sms     │  SendSMS...    │  <instance>     │  │
│  │   ...              │  ...           │  ...            │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

**Component Description:**

| Component | Responsibility |
|------|------|
| Discoverer | Scan extensions directory, find classes conforming to the Module interface |
| ID Map | Handle cross-language ID conversion |
| Validator | Verify modules correctly implement interface |
| Module Store | Store module classes and instances |

---

### 2.3 Executor

Executor is responsible for module invocation and execution.

```
┌─────────────────────────────────────────────────────────────┐
│                         Executor                             │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  call(module_id, inputs, context)                           │
│                    │                                         │
│                    ▼                                         │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  1. Context Creation                                   │  │
│  │     - Create/validate Context                          │  │
│  │     - Update caller_id, call_chain                     │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  2. Safety Checks                                     │  │
│  │     - Call depth, circular call, frequency limits     │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  3. Module Lookup                                     │  │
│  │     - registry.get(module_id)                         │  │
│  │     - Throw ModuleNotFoundError if not found          │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  4. ACL Enforcement                                   │  │
│  │     - Check caller → target permission                 │  │
│  │     - Throw ACLDeniedError if rejected                 │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  5. Approval Gate                                     │  │
│  │     - Check requires_approval annotation              │  │
│  │     - Request approval if needed                       │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  6. Input Validation                                  │  │
│  │     - Validate against input_schema                   │  │
│  │     - Throw SchemaValidationError on failure           │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  7. Middleware Before                                 │  │
│  │     - Execute middleware.before() in order            │  │
│  │     - Can modify inputs                               │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  8. Module Execution                                  │  │
│  │     - module.execute(inputs, context)                 │  │
│  │     - Dual timeout (per-module + global)              │  │
│  │     - Call middleware.on_error() on exception         │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  9. Output Validation                                 │  │
│  │     - Validate against output_schema                  │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 10. Middleware After                                  │  │
│  │     - Execute middleware.after() in reverse order     │  │
│  │     - Can modify output                               │  │
│  └───────────────────────┬──────────────────────────────┘  │
│                          ▼                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 11. Result Return                                     │  │
│  │     - Return final output                              │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

---

### 2.4 Context

Context flows through the entire call chain, carrying tracing and user information.

```
┌─────────────────────────────────────────────────────────────┐
│                         Context                              │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  trace_id: "abc-123"          # Unique call chain ID         │
│  caller_id: "orchestrator.x"  # Calling module ID            │
│  call_chain: ["a", "b", "c"]  # Complete call path           │
│  executor: <Executor>         # Executor reference           │
│  identity: Identity(...)      # Caller identity (ACL uses)   │
│  data: {...}                  # Shared pipeline state        │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

**Context Propagation:**

```
Top-level call (trace_id: "abc", caller_id: None, call_chain: [])
    │
    ▼
Module A (trace_id: "abc", caller_id: None, call_chain: ["A"])
    │
    ├── context.executor.call("B", inputs, context)
    │         │
    │         ▼
    │   Module B (trace_id: "abc", caller_id: "A", call_chain: ["A", "B"])
    │         │
    │         ├── context.executor.call("C", inputs, context)
    │         │         │
    │         │         ▼
    │         │   Module C (trace_id: "abc", caller_id: "B", call_chain: ["A", "B", "C"])
    │         │
    │         └── Return
    │
    └── Return
```

---

### 2.5 ACL (Access Control)

ACL controls call permissions between modules.

```
┌─────────────────────────────────────────────────────────────┐
│                           ACL                                │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  Rules (match in order):                                     │
│  ┌────────────────────────────────────────────────────────┐ │
│  │  1. callers: ["admin.*"]  targets: ["*"]          effect: allow│ │
│  │  2. callers: ["api.*"]   targets: ["executor.*"] effect: deny │ │
│  │  3. callers: ["orch.*"]  targets: ["executor.*"] effect: allow│ │
│  │  4. callers: ["*"]       targets: ["common.*"]   effect: allow│ │
│  │  5. callers: ["*"]       targets: ["*"]          effect: deny │ │
│  └────────────────────────────────────────────────────────┘ │
│                                                              │
│  check(caller_id, target_id) -> bool                        │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

**Matching Process:**

```
check("api.handler", "executor.email")

Rule 1: "admin.*" vs "api.handler" → No match
Rule 2: "api.*" vs "api.handler" → Match!
         "executor.*" vs "executor.email" → Match!
         effect: deny → Return False
```

---

### 2.6 Middleware

Middleware executes in an onion model.

```
┌─────────────────────────────────────────────────────────────┐
│                       Middleware Chain                       │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  Request ──┐                                                 │
│            ▼                                                 │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ MW1.before ──────────────────────────────────┐          ││
│  │  ┌─────────────────────────────────────────┐ │          ││
│  │  │ MW2.before ───────────────────┐         │ │          ││
│  │  │  ┌──────────────────────────┐ │         │ │          ││
│  │  │  │ MW3.before ────┐         │ │         │ │          ││
│  │  │  │  ┌────────────┐│         │ │         │ │          ││
│  │  │  │  │  Module    ││         │ │         │ │          ││
│  │  │  │  │  execute() ││         │ │         │ │          ││
│  │  │  │  └────────────┘│         │ │         │ │          ││
│  │  │  │ MW3.after ─────┘         │ │         │ │          ││
│  │  │  └──────────────────────────┘ │         │ │          ││
│  │  │ MW2.after ────────────────────┘         │ │          ││
│  │  └─────────────────────────────────────────┘ │          ││
│  │ MW1.after ───────────────────────────────────┘          ││
│  └─────────────────────────────────────────────────────────┘│
│            │                                                 │
│            ▼                                                 │
│  Response ──                                                 │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. Data Flow

### 3.1 Module Discovery Flow

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│ extensions/ │────▶│  Discoverer │────▶│  ID Map     │
│  structure  │     │  scan files │     │  convert ID │
└─────────────┘     └─────────────┘     └─────────────┘
                                               │
                                               ▼
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  Module     │◀────│  on_load()  │◀────│  Validator  │
│  Store      │     │  initialize │     │  verify     │
└─────────────┘     └─────────────┘     └─────────────┘
```

### 3.2 Module Call Flow

```
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌──────────┐
│ Client  │────▶│Executor │────▶│  ACL    │────▶│ Validate │
│         │     │ .call() │     │ .check()│     │  inputs  │
└─────────┘     └─────────┘     └─────────┘     └──────────┘
                                                      │
                                                      ▼
┌─────────┐     ┌──────────┐     ┌─────────┐     ┌──────────┐
│ Return  │◀────│Middleware │◀────│Validate │◀────│  Module  │
│ result  │     │  .after  │     │ output  │     │ .execute │
└─────────┘     └──────────┘     └─────────┘     └──────────┘
                                                      ▲
                                                      │
                                                 ┌──────────┐
                                                 │Middleware │
                                                 │  .before │
                                                 └──────────┘
```

### 3.3 Error Handling Flow

```
Module.execute() throws exception
        │
        ▼
MW3.on_error() ────── Has return? ─── Yes ──▶ Use as result
        │
        │ No
        ▼
MW2.on_error() ────── Has return? ─── Yes ──▶ Use as result
        │
        │ No
        ▼
MW1.on_error() ────── Has return? ─── Yes ──▶ Use as result
        │
        │ No
        ▼
Throw ModuleError
```

> **Note:** `on_error()` only iterates middlewares whose `before()` was successfully called before the failure occurred (the "executed middlewares" list). If a middleware's `before()` throws, only middlewares executed *before* it participate in `on_error()` unwinding. The diagram above shows the case where all three middlewares' `before()` completed and the error originated from `Module.execute()`.

---

## 4. Directory Structure

### 4.1 Framework Source Structure

```
apcore/
├── __init__.py           # Public API
├── module.py             # Module base class
├── decorator.py          # Module decorator and FunctionModule
├── bindings.py           # YAML binding loader for zero-code integration
├── executor.py           # Executor implementation
├── context.py            # Context definition
├── acl.py                # ACL implementation
├── errors.py             # Exception definitions
├── config.py             # Config loading
│
├── registry/             # Module discovery & registration
│   ├── __init__.py
│   ├── registry.py       # Central Registry class
│   ├── types.py          # ModuleDescriptor, DiscoveredModule, DependencyInfo
│   ├── scanner.py        # Directory/file scanning
│   ├── validation.py     # Module interface validation
│   ├── metadata.py       # Module metadata extraction
│   ├── dependencies.py   # Dependency resolution & load order
│   ├── entry_point.py    # Entry point resolution
│   └── schema_export.py  # Schema export helpers
│
├── schema/               # Schema processing
│   ├── __init__.py
│   ├── types.py          # SchemaStrategy, ExportProfile, type definitions
│   ├── loader.py         # Schema loading
│   ├── validator.py      # Schema validation
│   ├── exporter.py       # Schema export (JSON/YAML)
│   ├── ref_resolver.py   # $ref resolution
│   ├── strict.py         # Strict mode transformation
│   └── annotations.py    # x-* annotation handling
│
├── middleware/           # Middleware
│   ├── __init__.py       # Public API
│   ├── base.py           # Middleware base class
│   ├── adapters.py       # BeforeMiddleware, AfterMiddleware adapters
│   ├── manager.py        # MiddlewareManager pipeline engine
│   └── logging.py        # Logging middleware
│
├── observability/        # Observability
│   ├── __init__.py
│   ├── tracing.py        # Distributed tracing
│   ├── metrics.py        # Metrics collection
│   └── context_logger.py # Context-aware logging
│
└── utils/                # Utilities
    ├── __init__.py
    └── pattern.py        # Wildcard matching
```

### 4.2 Application Project Structure

```
my-project/
├── apcore.yaml           # Framework config
├── extensions/           # Extensions directory
│   ├── api/              # API entry layer
│   │   └── handler/
│   ├── orchestrator/     # Orchestration layer
│   │   └── workflow/
│   ├── executor/         # Execution layer
│   │   ├── email/
│   │   ├── sms/
│   │   └── database/
│   └── common/           # Common components
│       └── util/
├── acl/                  # Permission config
│   └── global_acl.yaml   # ACL config
├── schemas/              # External schemas (optional)
│   └── email.yaml
└── config/               # Other config (optional)
    └── id_map.yaml       # ID Map (optional)
```

---

## 5. Extension Points

apcore provides a formal `ExtensionManager` API for managing pluggable extension points. The five built-in extension points are: `discoverer`, `middleware`, `acl`, `span_exporter`, and `module_validator`.

### 5.1 ExtensionManager API

The `ExtensionManager` provides a unified interface for registering, retrieving, and applying extensions:

| Method | Signature | Description |
|--------|-----------|-------------|
| `register` | `register(point_name, extension)` | Register an extension for a named extension point |
| `get` | `get(point_name)` | Retrieve the single registered extension, or `None` |
| `get_all` | `get_all(point_name)` | Retrieve all registered extensions for a point as a list |
| `unregister` | `unregister(point_name, extension)` | Remove a registered extension; returns `bool` |
| `apply` | `apply(registry, executor)` | Apply all registered extensions to the given registry and executor |
| `list_points` | `list_points()` | List all registered extension points as a list of `ExtensionPoint` |

```python
from apcore import ExtensionManager

manager = ExtensionManager()

# Register a custom discoverer
manager.register("discoverer", remote_discoverer)

# Register a custom module validator
manager.register("module_validator", strict_validator)

# Retrieve all discoverers
discoverers = manager.get_all("discoverer")

# Retrieve single extension (or None)
acl_ext = manager.get("acl")

# Remove an extension
removed = manager.unregister("discoverer", remote_discoverer)  # True

# Apply all extensions to registry and executor
manager.apply(registry, executor)

# List registered points
points = manager.list_points()  # [ExtensionPoint(...), ...]
```

### 5.2 Custom Discoverer

```python
from apcore.registry.scanner import scan_extensions


class RemoteDiscoverer:
    """Discover modules from remote service"""

    def discover(self, config: dict) -> list[tuple[str, Type[Module]]]:
        # Get module definitions from remote service
        response = requests.get(config["remote_url"])
        modules = []
        for item in response.json():
            module_class = self._build_module(item)
            modules.append((item["id"], module_class))
        return modules
```

### 5.3 Custom Validator

```python
from apcore.registry.validation import validate_module


class StrictValidator:
    """Strict module validator"""

    def validate(self, module_class: Type[Module]) -> list[str]:
        errors = validate_module(module_class)

        # Custom rules
        if len(module_class.tags) == 0:
            errors.append("Module must have at least one tag")

        return errors
```

### 5.4 Custom ACL

```python
from apcore import ACL


class RBACAuthorizer(ACL):
    """Role-based access control"""

    def check(self, caller_id: str, target_id: str, context: Context) -> bool:
        if not context.identity:
            return False

        required_roles = self._get_required_roles(target_id)

        return bool(set(context.identity.roles) & set(required_roles))
```

> **Backward compatibility note:** The informal patterns shown above (subclassing, function-based APIs) continue to work. The `ExtensionManager` provides a formal unified API on top of these patterns for implementations that need programmatic extension point management.

---

## 6. Design Principles

### 6.1 Schema-Driven

All modules must define explicit schemas, ensuring:
- AI/LLM can understand module functionality
- Automatic validation of inputs/outputs
- Automatic documentation and SDK generation

### 6.2 Zero-Configuration Priority

- Directory as ID: file paths automatically become module IDs
- Auto-discovery: scan directories to auto-register modules
- Convention over configuration: reasonable defaults

### 6.3 Pluggable

- Middleware system: flexibly extend execution flow
- Custom discoverers: support multiple module sources
- Custom ACL: support complex permission models

### 6.4 Observable

- trace_id flows through call chain
- Built-in metrics collection
- Structured logging support

---

## 7. Concurrency Model

### 7.1 Thread Safety Guarantees

> Method names below (`get()`, `register()`, `add_rule()`, etc.) follow the Python SDK's API surface; equivalent methods exist in other SDKs with the same thread-safety guarantees.

| Component | Thread Safety Level | Description |
|------|-------------|------|
| Registry (read) | Fully safe | `get()`, `has()`, `list()` can be called concurrently |
| Registry (write) | Needs sync | `register()`, `unregister()` need locks |
| Executor | Fully safe | `call()` and `call_async()` can be called concurrently |
| Context | Partially safe | Immutable fields safe, `data` needs caller sync |
| ACL | Fully safe | Read-only checks are thread-safe. Runtime mutation (`add_rule`, `remove_rule`, `reload`) is protected by an internal lock |
| Middleware | Must ensure | Instance methods must be thread-safe |

### 7.2 Concurrent Execution Model

```
Single call: serial execution
  Executor.call() → ACL → Validate → Before MW → Execute → After MW → Return

Concurrent calls: independent contexts per call
  Thread 1: Executor.call(A) → [independent ACL/Validate/MW chain]
  Thread 2: Executor.call(B) → [independent ACL/Validate/MW chain]
  Thread 3: Executor.call(C) → [independent ACL/Validate/MW chain]

Shared resources:
  - Registry (read-only, safe)
  - ACL rules (read-only, safe)
  - Middleware instances (must be thread-safe)
  - Module instances (execute() must be thread-safe)
```

**Module Instance Lifecycle:**

- **Singleton model**: Each `module_id` corresponds to unique instance, `on_load()` called only once
- **Reentrancy**: `execute()` must support multi-threaded concurrent calls
- Internal module state should use locks or atomic variables for protection

**Hot Reload Safety:**

- During `unregister()`, module may be executing in other threads
- Implementation must maintain reference count, wait for execution completion before unload
- After timeout (default 5 seconds), force unload and log error
- See [PROTOCOL_SPEC §12.7.4 Hot Reload Race Conditions](../PROTOCOL_SPEC.md#1274-hot-reload-race-conditions)

**Middleware Chain Atomicity:**

- Each `call()`'s middleware chain (before → execute → after) doesn't interleave with other calls
- Middleware instances shared at application level, must be thread-safe
- Call-level state should be stored in `context.data`, not instance variables

### 7.3 Sync/Async Mixing

apcore supports mixed sync/async module calls, with automatic bridging:

| Caller | Called Module | Bridging Strategy |
|--------|-----------|---------|
| Sync | Sync | Direct call |
| Sync | Async | Block wait (await) |
| Async | Sync | Thread pool offload |
| Async | Async | Direct await |

**Python Example:**

```python
# Async calling sync module (thread pool offload)
async def async_caller():
    executor = Executor()
    result = await executor.call_async("sync_module", {})  # Auto offload to thread pool

# Sync calling async module (blocking wait)
def sync_caller():
    executor = Executor()
    result = executor.call("async_module", {})  # Auto await
```

**Note**: Sync→async bridging blocks caller thread, should avoid frequent use in async contexts.

### 7.4 Timeout and Cancellation

**Timeout Levels:**

- **Global timeout**: includes before + execute + after (default 60 seconds)
- **ACL check timeout**: independent timing (default 1 second)
- Timing starts from first `before()` middleware

**Cancellation Strategy:** Cooperative cancellation is implemented in both SDKs via `CancelToken`. Forced termination fallback is not yet implemented.

1. **Cooperative cancellation** (implemented):
   - Module checks `context.cancel_token.is_cancelled()` and actively exits
   - Suitable for long-running tasks (loops, I/O, etc.)

2. **Forced termination** (fallback, not yet implemented):
   - After cooperative cancellation fails, wait grace period (default 5 seconds)
   - If still not exited, force terminate thread/coroutine (may cause resource leaks)

See [PROTOCOL_SPEC §12.7.5 Timeout Enforcement](../PROTOCOL_SPEC.md#1275-timeout-enforcement)

## 8. Memory Model

### 8.1 Object Lifecycle

| Object | Creation Time | Destruction Time | Lifecycle |
|------|---------|---------|---------|
| Registry | App startup | App shutdown | App-level |
| Executor | App startup | App shutdown | App-level |
| Module instance | discover() or first call (lazy load) | unregister() or app shutdown | App-level |
| Context | Each call() | After call() returns | Request-level |
| Middleware | App startup | App shutdown | App-level |

### 8.2 Context.data Sharing Semantics

**Reference Sharing Rules:**

- `context.data` across entire call chain is **the same** dict object (reference sharing)
- Parent module modifications to `context.data` visible to child modules, and vice versa
- When `child()` creates new Context, `data` field copies reference (not deep copy)

**Isolation:**

- Different top-level `call()` invocations use independent `context.data` instances
- Concurrently executing call chains must not share `context.data` (avoid race conditions)

**Example:**

```python
# Top-level call 1
context1 = Context(data={})
executor.call("module_a", {}, context1)
# module_a internally:
context.data["key"] = "value_a"
sub_context = context.child("module_b")
executor.call("module_b", {}, sub_context)
# module_b reads "value_a" (reference sharing)

# Top-level call 2 (concurrent)
context2 = Context(data={})
executor.call("module_c", {}, context2)
# module_c's context.data is independent, doesn't contain "key"
```

**Concurrency Safety:**

- If `context.data` may be accessed by multiple threads, use thread-safe Map implementation
- Python's `dict` is partially thread-safe in CPython (GIL), but shouldn't rely on it

See [PROTOCOL_SPEC §12.7.2 Context.data Sharing Semantics](../PROTOCOL_SPEC.md#1272-contextdata-sharing-semantics)

### 8.3 Memory Considerations

- `context.data` accumulates along call chain, reclaimed by GC after call completes
- Avoid storing large objects (> 1MB) in `context.data`, use external cache
- Module instances are singleton (one instance per ID), resident in memory
- Schema objects cached after loading, not reparsed

## 9. Performance Characteristics

| Operation | Expected Latency | Description |
|------|---------|------|
| Registry.get() | < 1μs | Hash table lookup |
| ACL.check() | < 100μs | Rule linear scan (< 50 rules) |
| Schema validation | < 1ms | Depends on schema complexity |
| Middleware chain | < 1ms | Depends on middleware count and complexity |
| Module execution | Depends on business | Framework overhead < 5ms |

---

## Next Steps

- [Module Interface Definition](./api/module-interface.md) - Module API
- [Registry API](./api/registry-api.md) - Registry center API
- [Executor API](./api/executor-api.md) - Executor API
