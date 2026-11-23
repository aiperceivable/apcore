<div align="center">
  <img src="https://raw.githubusercontent.com/aiperceivable/apcore/main/apcore-logo.svg" alt="apcore logo" width="200"/>
</div>

# apcore

![Python](https://img.shields.io/badge/python-3.11+-blue.svg)
![License](https://img.shields.io/badge/license-Apache%202.0-green.svg)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/12294/badge)](https://www.bestpractices.dev/projects/12294)

> **Build once, invoke by Code or AI.**
> Every call validated, authorized, and evidenced.

A governed runtime for agent-callable capabilities — schema, ACL, approval, and audit enforced at every call.

**apcore** is an AI-Perceivable module standard that makes every interface naturally perceivable and understandable by AI through enforced Schema definitions and behavioral annotations. It provides strict type safety, access control, middleware pipelines, and built-in observability — enabling you to define modules with structured input/output schemas that are easily consumed by both code and AI.

## Features

- **Schema-driven modules** -- Define input/output contracts using Pydantic models (with automatic validation) or plain JSON Schema dicts
- **Execution Pipeline** -- Context creation, call chain guard, ACL enforcement, approval gate, middleware before, validation, execution, output validation, middleware after, and return -- with step metadata (`match_modules`, `ignore_errors`, `pure`, `timeout_ms`) and YAML pipeline configuration
- **`@module` decorator** -- Turn plain functions into fully schema-aware modules with zero boilerplate
- **YAML bindings** -- Register modules declaratively without modifying source code
- **Access control (ACL)** -- Pattern-based, first-match-wins rules with wildcard support
- **Middleware system** -- Composable before/after hooks with error recovery
- **Observability** -- Tracing (spans), metrics collection, and structured context logging
- **Async support** -- Seamless sync and async module execution
- **Safety guards** -- Call depth limits, circular call detection, frequency throttling
- **Approval system** -- Pluggable approval gate (Step 5) with sync/async handlers, Phase B resume, and audit events; execution-time `ExecutionPolicy` overrides (#76) can force/exempt approval on already-registered modules, gate `destructive` ops, and fail closed under `strict`
- **Extension points** -- Unified extension management for discoverers, middleware, ACL, approval handlers, span exporters, and module validators
- **Async task management** -- Background module execution with status tracking, cancellation, and concurrency limiting
- **Behavioral annotations** -- Declare module traits (readonly, destructive, idempotent, cacheable, paginated, streaming) for AI-aware orchestration
- **W3C Trace Context** -- traceparent header injection/extraction for distributed tracing interop
- **Circuit breakers** -- `CircuitBreakerMiddleware` for per-`(module_id, caller_id)` OPEN/CLOSED/HALF_OPEN protection; `CircuitBreakerWrapper` (`from apcore.events.circuit_breaker import CircuitBreakerWrapper`) for per-subscriber event delivery resilience
- **Multi-class module discovery** -- Opt-in `@multi_class` decorator (`from apcore.registry.multi_class import multi_class`) for multiple Module classes per file with snake_case ID derivation (PROTOCOL_SPEC §2.1.1)
- **Pluggable stores** -- `TaskStore` for async task persistence; `ObservabilityStore` for error/metric backends; `AuditStore` for control-module audit trails
- **Prometheus / K8s integration** -- `PrometheusExporter` (`from apcore.observability import PrometheusExporter`) with `/metrics`, `/healthz`, `/readyz` endpoints; `UsageCollector` and `MetricsCollector` emit standard Prometheus gauge/counter metrics

## API Overview

**Core**

| Class | Description |
|-------|-------------|
| `APCore` | High-level client -- register modules, call, stream, validate |
| `Registry` | Module storage -- discover, register, get, list |
| `Executor` | Execution engine -- call with middleware pipeline, ACL, approval |
| `Context` | Request context -- trace ID, identity, call chain, cancel token |
| `Config` | Configuration -- load from YAML, get/set values, namespace-partitioned Config Bus |
| `Identity` | Caller identity -- id, type, roles, attributes |
| `FunctionModule` | Wrapped function module created by `@module` decorator |

**Access Control & Approval**

| Class | Description |
|-------|-------------|
| `ACL` | Access control -- rule-based caller/target authorization |
| `ApprovalHandler` | Pluggable approval gate protocol |
| `AlwaysDenyHandler` / `AutoApproveHandler` / `CallbackApprovalHandler` | Built-in approval handlers |
| `ExecutionPolicy` / `PolicyRule` | Execution-time governance overrides (#76) -- force/exempt approval, `gate_destructive`, `strict` fail-closed |

**Middleware**

| Class | Description |
|-------|-------------|
| `Middleware` | Pipeline hooks -- before/after/on_error interception |
| `BeforeMiddleware` / `AfterMiddleware` | Single-phase middleware adapters |
| `LoggingMiddleware` | Structured logging middleware |
| `RetryMiddleware` | Automatic retry with backoff |
| `ErrorHistoryMiddleware` | Records errors into ErrorHistory |
| `PlatformNotifyMiddleware` | Emits events on error rate/latency spikes |
| `ObsLoggingMiddleware` | Observability-aware structured logging middleware |
| `CircuitBreakerMiddleware` | Circuit breaker keyed on `(module_id, caller_id)` (OPEN/CLOSED/HALF_OPEN) |

**Schema**

| Class | Description |
|-------|-------------|
| `SchemaLoader` | Load schemas from YAML or native types |
| `SchemaValidator` | Validate data against schemas |
| `SchemaExporter` | Export schemas for MCP, OpenAI, Anthropic, generic |
| `RefResolver` | Resolve `$ref` references in JSON Schema |

**Observability**

| Class | Description |
|-------|-------------|
| `TracingMiddleware` | Distributed tracing with span export |
| `BatchSpanProcessor` | Non-blocking OTEL span export with configurable queue (`from apcore.observability import BatchSpanProcessor`) |
| `MetricsMiddleware` / `MetricsCollector` | Call count, latency, error rate metrics |
| `ContextLogger` | Context-aware structured logging with `RedactionConfig` |
| `ErrorHistory` | Min-heap error ring buffer with SHA-256 fingerprint deduplication |
| `UsageCollector` | Per-module usage statistics, trends, and Prometheus export |
| `UsageMiddleware` | Per-call usage tracking middleware |
| `TraceContext` | W3C Trace Context propagation (traceparent/tracestate) |
| `InMemoryExporter` | Span exporter that stores spans in memory |
| `StdoutExporter` | Span exporter that writes spans to stdout |
| `OTLPExporter` | Span exporter using OpenTelemetry Protocol |
| `PrometheusExporter` | HTTP server for `/metrics`, `/healthz`, `/readyz` endpoints (`from apcore.observability import PrometheusExporter`) |

**Events & Extensions**

| Class | Description |
|-------|-------------|
| `EventEmitter` | Event system -- subscribe, emit, flush |
| `WebhookSubscriber` / `A2ASubscriber` | Built-in event subscribers |
| `ExtensionManager` | Unified extension point management |
| `AsyncTaskManager` | Background module execution with pluggable `TaskStore`, retry, and reaper |
| `TaskStore` / `InMemoryTaskStore` | Pluggable async task persistence backend |
| `RetryPolicy` / `BackoffStrategy` | **Deprecated (0.21.0)** — per-task retry configuration. Use `RetryConfig` for cross-language field-name parity. |
| `CancelToken` | Cooperative cancellation token |
| `BindingLoader` | Load modules from YAML binding files |
| `ErrorCodeRegistry` | Central registry for structured error codes |
| `ErrorFormatterRegistry` | Surface-specific error formatter registry (MCP, A2A, CLI adapters) |

**Name canonicalization**

`canonicalize_name()` converts a bare ASCII name into one canonical segment,
retaining the original for display and audit. It rejects Unicode rather than
dropping or transliterating characters and returns a diagnostic instead of
throwing for invalid strings. `normalize_to_canonical_id()` remains the
non-repairing A02 module-ID converter. The name `system` is a valid segment;
reserved namespace enforcement and name collisions belong to registration
and scanning, not this utility.

```python
from apcore import canonicalize_name

source_name = " HTTP / UserProfile "
result = canonicalize_name(source_name)
if result.error is None:
    print({"original_name": result.original_name, "canonical_name": result.canonical_name})
else:
    print({"original_name": result.original_name, "error": result.error.value})
```

## Cross-Language Parity Notes

The Python, TypeScript, and Rust SDKs share one verified contract. As of **v0.22.0**,
the hardening features that originally landed Python-first are at parity across all
three SDKs — including `AsyncTaskManager`, `ExtensionManager`, `CircuitBreakerMiddleware`,
`TaskStore`/`InMemoryTaskStore` (pluggable interface + in-memory backend; Redis/SQL
backends are shipped by none, by design), and `PrometheusExporter`. `version_hint`
negotiation is honored by all three executors. As of **v0.23.0**, AI error-recovery
metadata (per-code `user_fixable` defaults plus filled `ai_guidance`) is also at parity
across all three SDKs, verified by the shared `error_recovery_metadata.json` conformance fixture.

As of **v0.27.0**, `validate()` withholds module-level introspection from a caller the
ACL denied (apcore#96, spec v1.13.0 §12.8.5.1). It previously ran `preflight()` and
`preview()` on the strength of a module lookup alone, so a denied caller still made
module-authored code run and still received what it returned — for a command-wrapping
module, the resolved binary and its argv. All three SDKs did it, and all three now
suppress it; the failed `acl` check is still reported, so a denied caller still learns
*why*, and a failed `schema` check does **not** suppress introspection. Pinned by
`preflight_disclosure.json`, whose control case exists so an implementation that never
introspects at all cannot pass the denial cases for the wrong reason.

Intentional cross-language differences (e.g. Python's synchronous `call()` ergonomics,
the legacy/deprecated `RetryPolicy`/`BackoffStrategy` aliases that exist only in Python,
and the module-level convenience functions) are documented inline in the relevant feature
specs as "Cross-language note" admonitions in the canonical protocol repo
([apcore/docs/features](https://github.com/aiperceivable/apcore/tree/main/docs/features)).
Anything that differs across SDKs without such a note is drift, not a feature.

## Configuration

### Config Bus and Namespace Registration

`Config` doubles as an ecosystem-level Config Bus. Any package can register a named namespace with optional JSON Schema validation, env prefix, and default values:

```python
from apcore import Config

# Register a namespace (class-level, shared across all Config instances)
Config.register_namespace(
    "my_plugin",
    schema={"type": "object", "properties": {"timeout_ms": {"type": "integer"}}},
    env_prefix="MY_PLUGIN__",
    defaults={"timeout_ms": 5000},
)

# Load config as usual
config = Config.load("project.yaml")

# Namespace-aware access
timeout = config.get("my_plugin.timeout_ms")   # dot-path with namespace resolution
subtree = config.namespace("my_plugin")          # full subtree as dict

# Typed access
config.get_typed("my_plugin.timeout_ms", int)

# Mount an external source (no unified YAML required)
config.mount("my_plugin", from_dict={"timeout_ms": 3000})

# Introspect registered namespaces
names = Config.registered_namespaces()
```

### Built-in Namespaces

apcore pre-registers three namespaces that promote its existing flat config keys:

| Namespace | Env prefix | Keys |
|-----------|-----------|------|
| `observability` | `APCORE_OBSERVABILITY` | tracing, metrics, logging, error_history, platform_notify |
| `obs` | `APCORE_OBS` | redaction.regex_patterns, redaction.sensitive_keys, redaction.replacement |
| `sys_modules` | `APCORE_SYS` | events.thresholds.error_rate, events.thresholds.latency_p99_ms |

### Environment Variable Conventions

| Pattern | When to use | Example |
|---------|------------|---------|
| `APCORE_KEY_NAME` | Override a flat top-level apcore key (existing convention) | `APCORE_EXECUTOR_DEFAULT__TIMEOUT=5000` |
| `APCORE_NAMESPACE` prefix | Override keys inside a registered namespace | `APCORE_OBSERVABILITY_TRACING_ENABLED=true` |
| Custom prefix declared in `register_namespace` | Third-party packages with their own prefix | `MY_PLUGIN__TIMEOUT_MS=3000` |

The longest-prefix-match dispatch algorithm ensures that `APCORE_OBSERVABILITY_TRACING_ENABLED` routes to the `observability` namespace (not to a core flat key). Within each namespace, a single `_` maps to `.` and `__` maps to a literal `_`.

### New Error Codes (0.15.0)

| Code | Meaning |
|------|---------|
| `CONFIG_NAMESPACE_DUPLICATE` | A namespace with this name is already registered |
| `CONFIG_NAMESPACE_RESERVED` | The namespace name is reserved (`_config`, `apcore`) |
| `CONFIG_ENV_PREFIX_CONFLICT` | Two namespaces share the same env prefix |
| `CONFIG_MOUNT_ERROR` | Failed to load or parse a mounted config source |
| `CONFIG_BIND_ERROR` | Failed to deserialize a namespace subtree into the requested type |
| `ERROR_FORMATTER_DUPLICATE` | A formatter for this surface is already registered |

### Event Type Names

Canonical event type names use dot-namespaced identifiers. `apcore.*` is reserved for core framework events; adapter packages use their own prefix (e.g., `apcore-mcp.*`).

| Canonical name | Emitted by |
|---------------|-----------|
| `apcore.module.toggled` | `system.control.toggle_feature` |
| `apcore.health.recovered` | `PlatformNotifyMiddleware` (error rate recovery) |
| `apcore.config.updated` | `system.control.update_config` |
| `apcore.module.reloaded` | `system.control.reload_module` |

> **v0.18.0 — legacy aliases removed.** Listeners that previously subscribed to
> `module_health_changed` or `config_changed` will no longer receive events.
> Migrate subscriptions to the canonical names above.

## Documentation

For full documentation, including Quick Start guides for both Python and TypeScript, visit:
**[https://aiperceivable.github.io/apcore/getting-started/](https://aiperceivable.github.io/apcore/getting-started/)**

## Requirements

- Python >= 3.11

## Installation

```bash
pip install apcore
```

### Development

```bash
pip install -e ".[dev]"
```

## Quick Start

### Simple usage (Global Client)

For simple scripts or prototypes, you can use the global `apcore` functions:

```python
import apcore

@apcore.module(id="math.add", description="Add two integers")
def add(a: int, b: int) -> int:
    return a + b

# Directly call it
result = apcore.call("math.add", {"a": 10, "b": 5})
print(result)  # {'result': 15}
```

### Simplified Client (Recommended)

The `APCore` client provides a unified entry point that manages everything for you:

```python
from apcore import APCore

client = APCore()

@client.module(id="math.add", description="Add two integers")
def add(a: int, b: int) -> int:
    return a + b

# Call the module
result = client.call("math.add", {"a": 10, "b": 5})
print(result)  # {'result': 15}
```

### Advanced: Define a module with a class

```python
from pydantic import BaseModel
from apcore import Context, APCore

client = APCore()

class GreetInput(BaseModel):
    name: str

class GreetOutput(BaseModel):
    message: str

class GreetModule:
    input_schema = GreetInput
    output_schema = GreetOutput
    description = "Greet a user"

    def execute(self, inputs: dict, context: Context) -> dict:
        return {"message": f"Hello, {inputs['name']}!"}

client.register("greet", GreetModule())
result = client.call("greet", {"name": "Alice"})
# {"message": "Hello, Alice!"}
```

### Alternative: Define schemas with plain dicts

If you prefer not to use Pydantic, pass raw JSON Schema dicts directly:

```python
from apcore import APCore

client = APCore()

class WeatherModule:
    input_schema = {"type": "object", "properties": {"city": {"type": "string"}}}
    output_schema = {"type": "object", "properties": {"temp": {"type": "number"}}}
    description = "Get current temperature"

    def execute(self, inputs: dict, context=None) -> dict:
        return {"temp": 22.5}

client.register("weather", WeatherModule())
result = client.call("weather", {"city": "Tokyo"})
# {"temp": 22.5}
```

> **Note:** Dict schemas skip Pydantic input validation. Use Pydantic models when you need automatic type coercion and validation, or validate inside `execute()`.

### Add middleware

```python
from apcore import LoggingMiddleware, TracingMiddleware

client.use(LoggingMiddleware())
client.use(TracingMiddleware())
```


### Access control

```python
from apcore import ACL, ACLRule, Executor, Registry

registry = Registry()
acl = ACL(rules=[
    ACLRule(callers=["admin.*"], targets=["*"], effect="allow", description="Admins can call anything"),
    ACLRule(callers=["*"], targets=["admin.*"], effect="deny", description="Others cannot call admin modules"),
])
executor = Executor(registry=registry, acl=acl)
```

## Examples

The `examples/` directory contains runnable demos:

---

### `simple_client` — APCore client with decorator-based modules

Initializes an `APCore` client, registers modules with `@client.module()`, and calls them directly.

```python
from apcore import APCore

client = APCore()

@client.module(id="math.add", description="Add two integers")
def add(a: int, b: int) -> int:
    return a + b

result = client.call("math.add", {"a": 10, "b": 5})
print(result)  # {'result': 15}

@client.module(id="greet")
def greet(name: str, greeting: str = "Hello") -> dict:
    return {"message": f"{greeting}, {name}!"}

result = client.call("greet", {"name": "Alice"})
print(result)  # {'message': 'Hello, Alice!'}
```

---

### `global_client` — Minimal global client usage

No explicit initialization needed — use the default global client directly.

```python
import apcore

@apcore.module(id="math.add")
def add(a: int, b: int) -> int:
    return a + b

result = apcore.call("math.add", {"a": 10, "b": 5})
print(result)  # {'result': 15}
```

---

### `greet` — Duck-typed module with Pydantic schemas

Demonstrates the class-based module interface with Pydantic `BaseModel` for input/output schemas.

```python
from pydantic import BaseModel

class GreetInput(BaseModel):
    name: str

class GreetOutput(BaseModel):
    message: str

class GreetModule:
    input_schema = GreetInput
    output_schema = GreetOutput
    description = "Greet a user by name"

    def execute(self, inputs: dict, context) -> dict:
        name = inputs["name"]
        return {"message": f"Hello, {name}!"}
```

---

### `get_user` — Readonly module with `ModuleAnnotations`

Demonstrates behavioral annotations (`readonly`, `idempotent`) and simulated database lookup.

```python
from pydantic import BaseModel
from apcore.module import ModuleAnnotations

class GetUserInput(BaseModel):
    user_id: str

class GetUserOutput(BaseModel):
    id: str
    name: str
    email: str

class GetUserModule:
    input_schema = GetUserInput
    output_schema = GetUserOutput
    description = "Get user details by ID"
    annotations = ModuleAnnotations(readonly=True, idempotent=True)

    _users = {
        "user-1": {"id": "user-1", "name": "Alice", "email": "alice@example.com"},
        "user-2": {"id": "user-2", "name": "Bob", "email": "bob@example.com"},
    }

    def execute(self, inputs: dict, context) -> dict:
        user_id = inputs["user_id"]
        user = self._users.get(user_id)
        if user is None:
            return {"id": user_id, "name": "Unknown", "email": "unknown@example.com"}
        return dict(user)
```

---

### `send_email` — Destructive module with sensitive fields and ContextLogger

Shows `x-sensitive` on schema fields (for log redaction), `ModuleAnnotations` with metadata, `ModuleExample` for AI-perceivable documentation, and `ContextLogger` usage. Schema redaction follows local JSON pointers, anchors, and self references through object, array, and composition schemas, including positional prefixes, tuple items, and their tail schemas; recursive references are evaluated alongside the data without schema expansion or external retrieval.

```python
from pydantic import BaseModel, Field
from apcore.module import ModuleAnnotations, ModuleExample
from apcore.observability import ContextLogger

class SendEmailInput(BaseModel):
    to: str
    subject: str
    body: str
    api_key: str = Field(..., json_schema_extra={"x-sensitive": True})

class SendEmailOutput(BaseModel):
    status: str
    message_id: str

class SendEmailModule:
    input_schema = SendEmailInput
    output_schema = SendEmailOutput
    description = "Send an email message"
    tags = ["email", "communication", "external"]
    version = "1.2.0"
    metadata = {"provider": "example-smtp", "max_retries": 3}
    annotations = ModuleAnnotations(destructive=True, idempotent=False, open_world=True)
    examples = [
        ModuleExample(
            title="Send a welcome email",
            inputs={"to": "user@example.com", "subject": "Welcome!", "body": "...", "api_key": "sk-xxx"},
            output={"status": "sent", "message_id": "msg-12345"},
            description="Sends a welcome email to a new user.",
        ),
    ]

    def execute(self, inputs: dict, context) -> dict:
        logger = ContextLogger.from_context(context, name="send_email")
        logger.info("Sending email", extra={"to": inputs["to"], "subject": inputs["subject"]})
        message_id = f"msg-{hash(inputs['to']) % 100000:05d}"
        logger.info("Email sent successfully", extra={"message_id": message_id})
        return {"status": "sent", "message_id": message_id}
```

---

### `decorated_add` — `@module` decorator for simple functions

```python
from apcore.decorator import module

@module(description="Add two integers", tags=["math", "utility"])
def add(a: int, b: int) -> int:
    return a + b
```

## Development

### Run tests

```bash
pytest
```

### Run tests with coverage

```bash
pytest --cov=src/apcore --cov-report=html
```

### Lint and format

```bash
ruff check --fix src/ tests/
ruff format src/ tests/
```

### Type check

```bash
mypy src/ tests/
```


## 📄 License

Apache-2.0

## 🔗 Links

- **Documentation**: [docs/apcore](https://github.com/aiperceivable/apcore) - Complete documentation
- **Website**: [aiperceivable.com](https://aiperceivable.com)
- **GitHub**: [aiperceivable/apcore-python](https://github.com/aiperceivable/apcore-python)
- **PyPI**: [apcore](https://pypi.org/project/apcore/)
- **Issues**: [GitHub Issues](https://github.com/aiperceivable/apcore-python/issues)
- **Discussions**: [GitHub Discussions](https://github.com/aiperceivable/apcore-python/discussions)
