<div align="center">
  <img src="https://raw.githubusercontent.com/aiperceivable/apcore/main/apcore-logo.svg" alt="apcore logo" width="200"/>
</div>

# apcore

![Rust](https://img.shields.io/badge/rust-1.86+-orange.svg)
![License](https://img.shields.io/badge/license-Apache%202.0-green.svg)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/12294/badge)](https://www.bestpractices.dev/projects/12294)

> **Build once, invoke by Code or AI.**
> Every call validated, authorized, and evidenced.

A governed runtime for agent-callable capabilities — schema, ACL, approval, and audit enforced at every call.

**apcore** is an AI-Perceivable module standard that makes every interface naturally perceivable and understandable by AI through enforced Schema definitions and behavioral annotations. It provides strict type safety, access control, middleware pipelines, and built-in observability — enabling you to define modules with structured input/output schemas that are easily consumed by both code and AI.

## Features

- **Schema-driven modules** — Define input/output contracts using `schemars`-derived types with automatic validation
- **Execution Pipeline** — Context creation, call chain guard, ACL enforcement, approval gate, middleware before, validation, execution, output validation, middleware after, and return — with step metadata (`match_modules`, `ignore_errors`, `pure`, `timeout_ms`) and YAML pipeline configuration
- **`Module` trait** — Implement the `Module` trait to create fully schema-aware modules
- **YAML bindings** — Register modules declaratively without modifying source code
- **Access control (ACL)** — Pattern-based, first-match-wins rules with wildcard support
- **Middleware system** — Composable before/after hooks with error recovery
- **Observability** — Tracing (spans), metrics collection, and structured context logging
- **Async support** — Built on `tokio` for seamless async module execution
- **Safety guards** — Call depth limits, circular call detection, frequency throttling
- **Approval system** — Pluggable approval gate with async handlers, Phase B resume, and audit events
- **AI error-recovery metadata** — Framework errors carry structured `retryable` / `user_fixable` / `ai_guidance` recovery hints resolved per error code, so self-healing agents get a consistent recovery contract on every surface
- **Behavioral annotations** — Declare module traits (readonly, destructive, idempotent, cacheable, paginated, streaming) for AI-aware orchestration
- **W3C Trace Context** — `traceparent` header injection/extraction for distributed tracing interop

## Cross-Language Feature Parity

The Rust SDK tracks the apcore protocol spec and ships full feature parity
with the Python and TypeScript SDKs. The table below covers the v0.23
hardening items (#60–#65), the v0.24–v0.26 governance surfaces, the v0.27
authorization and descriptor fixes, and the long-standing background-task and
extension surfaces.

| Feature | Python | TypeScript | Rust |
|---------|:------:|:----------:|:----:|
| Reserved namespace query API (#60) | Yes | Yes | Yes |
| Event delivery semantics — retry + DLQ (#61) | Yes | Yes | Yes |
| Streaming module interface (#62) | Yes | Yes | Yes |
| `ContextKey` typed context accessors (#63) | Yes | Yes | Yes |
| Middleware duplicate-name detection (#64) | Yes | Yes | Yes |
| Registry load-ordering guarantees (#65) | Yes | Yes | Yes |
| `AsyncTaskManager` (background task execution) | Yes | Yes | Yes |
| `ExtensionManager` / `ExtensionPoint` (plugin registry) | Yes | Yes | Yes |
| `validate()` withholds introspection from a denied caller (apcore#96) | Yes | Yes | Yes |
| `get_definition().dependencies` is a parsed field (apcore#90) | Yes | Yes | Yes |

**v0.27.0, BREAKING (security).** `validate()` used to run `preflight()` and
`preview()` on the strength of a module lookup alone, so a caller the ACL had
just denied still made module-authored code run and still received what it
returned — for a command-wrapping module, the resolved binary and its argv. All
three SDKs did it; all three now suppress it when the `acl` check fails, while
still reporting that failed check so a denied caller learns *why*. A failed
`schema` check does **not** suppress introspection: a permitted caller is
entitled to the module's account of what would happen even when its inputs are
malformed. See spec v1.13.0 §12.8.5.1 and `preflight_disclosure.json`.

See the [`AsyncTaskManager`](./src/async_task.rs) and
[`ExtensionManager`](./src/extensions.rs) source, plus the corresponding
tests at `tests/test_async_task.rs` and `tests/test_extensions.rs`.

## API Overview

**Core**

| Type | Description |
|------|-------------|
| `APCore` | High-level client — register modules, call, stream, validate |
| `Registry` | Module storage — discover, register, get, list, plus `watch()` for filesystem hot-reload (parity with apcore-python `Registry.watch`) and `reload()` for explicit re-discovery |
| `Executor` | Execution engine — call with middleware pipeline, ACL, approval |
| `Context` | Request context — trace ID, identity, call chain, cancel token |
| `Config` | Configuration — from_defaults with env overrides, load YAML/JSON, get/set dot-path, validate, reload |
| `Identity` | Caller identity — id, type, roles, attributes |
| `Module` | Core trait for implementing schema-aware modules |

**Access Control & Approval**

| Type | Description |
|------|-------------|
| `ACL` | Access control — rule-based caller/target authorization |
| `ApprovalHandler` | Pluggable approval gate trait |
| `AlwaysDenyHandler` / `AutoApproveHandler` | Built-in approval handlers |

**Middleware**

| Type | Description |
|------|-------------|
| `Middleware` | Pipeline hooks — before/after/on_error interception |
| `BeforeMiddleware` / `AfterMiddleware` | Single-phase middleware adapters |
| `ObsLoggingMiddleware` | Structured logging middleware |
| `RetryMiddleware` | Automatic retry with backoff |
| `ErrorHistoryMiddleware` | Records errors into `ErrorHistory` |
| `PlatformNotifyMiddleware` | Emits events on error rate/latency spikes |

**Schema**

| Type | Description |
|------|-------------|
| `SchemaLoader` | Load schemas from YAML or native types |
| `SchemaValidator` | Validate data against schemas |
| `SchemaExporter` | Export schemas for MCP, OpenAI, Anthropic, generic |
| `RefResolver` | Resolve `$ref` references in JSON Schema |

**Observability**

| Type | Description |
|------|-------------|
| `TracingMiddleware` | Distributed tracing with span export |
| `MetricsMiddleware` / `MetricsCollector` | Call count, latency, error rate metrics |
| `ContextLogger` | Context-aware structured logging |
| `ErrorHistory` | Ring buffer of recent errors with deduplication |
| `UsageCollector` | Per-module usage statistics and trends |

**Events & Utilities**

| Type | Description |
|------|-------------|
| `EventEmitter` | Event system — subscribe, unsubscribe, emit, emit_filtered, flush |
| `WebhookSubscriber` | Built-in event subscriber |
| `CancelToken` | Cooperative cancellation token |
| `BindingLoader` | Load modules from YAML binding files |

> See [Cross-Language Feature Parity](#cross-language-feature-parity) for the
> full parity matrix across Python, TypeScript, and Rust.

## Documentation

For full documentation, including Quick Start guides for Python and Rust, visit:
**[https://aiperceivable.github.io/apcore/getting-started/](https://aiperceivable.github.io/apcore/getting-started/)**

## Requirements

- Rust >= 1.86 (enforced by `rust-version` in `Cargo.toml`)
- Tokio async runtime

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
apcore = "0.32"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
async-trait = "0.1"
```

## Bare-name canonicalization

Use `canonicalize_name` for one external name, such as a CLI command or
operation name. It returns a canonical segment or a structured diagnostic,
always retaining the exact original input:

```rust
use apcore::{canonicalize_name, CanonicalNameError};

fn main() {
    let command = canonicalize_name("cat-file");
    assert_eq!(command.original_name, "cat-file");
    assert_eq!(command.canonical_name.as_deref(), Some("cat_file"));
    assert_eq!(command.error, None);

    let invalid = canonicalize_name("7z");
    assert_eq!(invalid.original_name, "7z");
    assert_eq!(invalid.canonical_name, None);
    assert_eq!(invalid.error, Some(CanonicalNameError::InvalidStart));
}
```

The function rejects non-ASCII input before trimming ASCII edge punctuation
and whitespace, applies ASCII case boundaries, and replaces each internal
separator run with one underscore. Existing underscores are preserved.
Diagnostics are `non_ascii`, `empty_name`, `invalid_start`, and `name_too_long`,
checked in that order. The candidate must start with an ASCII lowercase letter
and must not exceed 192 characters; names are never prefixed or truncated.
`CanonicalNameResult` and `CanonicalNameError` support serde and JSON Schema.

This API does not process a module ID hierarchy: `api.GetUser` becomes the
single segment `api_get_user`. Existing `normalize_to_canonical_id` remains
the strict, language-aware module ID API and still rejects hyphens.
`system` is a valid bare segment; reserved namespaces and collisions must be
checked when registering or scanning modules.

## Quick Start

### Simple client

```rust
use apcore::APCore;
use apcore::module::Module;
use apcore::context::Context;
use serde_json::{json, Value};

struct AddModule;

#[async_trait::async_trait]
impl Module for AddModule {
    fn description(&self) -> &str { "Add two integers" }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}, "required": ["a", "b"]})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object", "properties": {"result": {"type": "integer"}}})
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &Context<Value>,
    ) -> Result<Value, apcore::errors::ModuleError> {
        let a = input["a"].as_i64().unwrap_or(0);
        let b = input["b"].as_i64().unwrap_or(0);
        Ok(json!({ "result": a + b }))
    }
}

#[tokio::main]
async fn main() {
    let mut client = APCore::new();
    client.register("math.add", Box::new(AddModule)).unwrap();

    let result = client
        .call("math.add", json!({"a": 10, "b": 5}), None, None)
        .await
        .unwrap();
    println!("{}", result); // {"result": 15}
}
```

### With configuration

```rust
use apcore::APCore;

#[tokio::main]
async fn main() {
    // Load directly from file path
    let client = APCore::from_path("apcore.yaml").unwrap();

    // Or load and modify config before constructing
    // let config = Config::from_yaml_file(Path::new("apcore.yaml")).unwrap();
    // let client = APCore::with_config(config);
}
```

### Module with typed schemas

```rust
use apcore::module::{Module, ModuleAnnotations};
use apcore::context::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize)]
struct GetUserInput {
    user_id: String,
}

#[derive(Serialize, Deserialize)]
struct GetUserOutput {
    id: String,
    name: String,
    email: String,
}

struct GetUserModule;

#[async_trait::async_trait]
impl Module for GetUserModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "user_id": { "type": "string" }
            },
            "required": ["user_id"]
        })
    }

    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "name": { "type": "string" },
                "email": { "type": "string" }
            }
        })
    }

    fn description(&self) -> &str { "Get user details by ID" }

    // Annotations (readonly, idempotent, etc.) are set on
    // ModuleDescriptor when registering the module with the registry.

    async fn execute(
        &self,
        input: Value,
        _ctx: &Context<Value>,
    ) -> Result<Value, apcore::errors::ModuleError> {
        let req: GetUserInput = serde_json::from_value(input)?;
        let user = match req.user_id.as_str() {
            "user-1" => GetUserOutput { id: "user-1".into(), name: "Alice".into(), email: "alice@example.com".into() },
            "user-2" => GetUserOutput { id: "user-2".into(), name: "Bob".into(),   email: "bob@example.com".into() },
            id       => GetUserOutput { id: id.into(),       name: "Unknown".into(), email: "unknown@example.com".into() },
        };
        Ok(serde_json::to_value(user)?)
    }
}
```

### Add middleware

> Rust reserves `use` as a keyword, so the middleware-attachment method is
> named `use_middleware()` rather than `use()` as in apcore-python and
> apcore-typescript. Functionally equivalent.

```rust
use apcore::observability::{ContextLogger, ObsLoggingMiddleware};

client.use_middleware(Box::new(ObsLoggingMiddleware::new(ContextLogger::new("app"))));
// TracingMiddleware requires a SpanExporter — see observability docs
```

> **`TracingMiddleware`** — there is exactly one, `apcore::TracingMiddleware`
> (re-exported from `observability::tracing_middleware`). It opens the
> `apcore.module.execute` span the protocol specifies, keeps a span stack so
> nested module-to-module calls get real `parent_span_id` links, and exports
> through a pluggable `SpanExporter` with sampling controls and zero external
> dependencies. Point it at an OpenTelemetry collector with `OTLPExporter`.

### Access control

`ACLRule` is `#[non_exhaustive]`, so a rule is built through `ACLRule::new` and
its optional fields assigned afterwards — the form that compiles from outside
the crate, and the one that stays source-compatible when the spec adds a field.

```rust
use apcore::acl::{ACL, ACLRule};

let mut admins = ACLRule::new(vec!["admin.*".into()], vec!["*".into()], "allow");
admins.description = Some("Admins can call anything".into());

let mut others = ACLRule::new(vec!["*".into()], vec!["admin.*".into()], "deny");
others.description = Some("Others cannot call admin modules".into());

let acl = ACL::new(vec![admins, others], "deny", None);
```

A call that arrives with no `caller_id` is matched as `apcore::EXTERNAL_CALLER`
(`"@external"`), an exact literal that no wildcard reaches — grant it explicitly
with `callers: ["@external"]`.

### Execution policy (external governance overrides)

`ExecutionPolicy` lets a platform operator override a module's governance
annotations (`requires_approval`, `destructive`) at execution time, without
touching the module's source or its registration. It attaches to the
`Executor` and is consulted by the approval gate (pipeline step 5). Rules use
the same wildcard matching and specificity scoring as ACL rules; on a
specificity tie the more restrictive rule wins.

```rust
use std::sync::Arc;

use apcore::approval::AutoApproveHandler;
use apcore::config::Config;
use apcore::executor::Executor;
use apcore::registry::registry::Registry;
use apcore::{ExecutionPolicy, PolicyRule};

fn configure(registry: Arc<Registry>) -> Result<Executor, apcore::errors::ModuleError> {
    let mut executor = Executor::new(registry, Config::from_defaults());

    let policy = ExecutionPolicy::new(vec![
        // Force approval for every module under `executor.payments.*`,
        // whatever the module itself declares.
        PolicyRule::new("executor.payments.*")?
            .with_requires_approval(true)
            .with_reason("PCI scope — platform requires human sign-off"),
        // Exempt a known-safe read path from the broader rule above; the more
        // specific pattern wins.
        PolicyRule::new("executor.payments.read_balance")?
            .with_requires_approval(false)
            .with_reason("read-only balance lookup"),
    ])
    // Also gate anything annotated `destructive: true`, even without a rule.
    .with_gate_destructive(true)
    // Fail CLOSED when approval is required but no ApprovalHandler is
    // configured, instead of warning and proceeding.
    .with_strict(true);

    executor.set_policy(Some(policy));
    executor.set_approval_handler(Box::new(AutoApproveHandler));
    Ok(executor)
}
```

A matched rule overrides the module's own annotations — external governance is
the platform's word against the module author's. Policy decisions are carried
into `ApprovalRequest` and emitted as `apcore.policy.override` /
`apcore.approval.decision` events. See `examples/execution_policy.rs` for a
runnable end-to-end version.

### Rust-specific quirks: two `RetryConfig` types

The Rust SDK ships two distinct `RetryConfig` structs that cannot share a
crate-root alias because they collide on name. Pick the import path that
matches the subsystem you are configuring:

```rust
// Middleware-level retry (used by `RetryMiddleware`). This one IS
// re-exported at the crate root for convenience.
use apcore::RetryConfig as MiddlewareRetryConfig;
// equivalent to: use apcore::middleware::RetryConfig;

// AsyncTaskManager-level retry (used by background task scheduling).
// Re-exported at the crate root under the non-colliding name
// `AsyncRetryConfig`, mirroring apcore-python and apcore-typescript which
// both rename-export the same type. The nested path also still works.
use apcore::AsyncRetryConfig as TaskRetryConfig;
// equivalent to: use apcore::async_task::RetryConfig;
```

### YAML bindings

Register modules without touching Rust source — define a `binding.yaml`:

```yaml
bindings:
  - module_id: "utils.format_date"
    target: "format_date::format_date_string"
    description: "Format a date string into a specified format"
    tags: ["utility", "date"]
    version: "1.0.0"
    input_schema:
      type: object
      properties:
        date_string:   { type: string }
        output_format: { type: string }
      required: [date_string, output_format]
    output_schema:
      type: object
      properties:
        formatted: { type: string }
      required: [formatted]
```

Load it at runtime:

```rust
use apcore::bindings::BindingLoader;

let mut loader = BindingLoader::new();
loader.load_from_yaml(std::path::Path::new("binding.yaml")).unwrap();
```

### Annotation overlay (cross-SDK difference)

The Python and TypeScript SDKs support `*_meta.yaml` sidecar files that override
code-defined module annotations at load time (PROTOCOL_SPEC.md  §4.13: field-level
merge, YAML wins over code). The Rust SDK **does not** implement this feature.

This is a deliberate design choice:

- **Spec §4.13 is conditional**: it mandates field-level merge only "*when both* YAML
  metadata file and code define Annotations". If the SDK never loads YAML metadata
  files, the rule is never triggered.
- **Rust favours explicit configuration**: annotations are declared via
  `ModuleAnnotations` in code and are type-checked at compile time. YAML-based
  override introduces implicit, late-bound behavior that conflicts with Rust's
  "explicit > implicit" philosophy.
- **No user demand**: as of v0.18.0 there are zero issues or RFCs requesting YAML
  annotation overlays for the Rust SDK.

If you need runtime-configurable annotations (e.g., ops teams toggling `readonly` or
`requires_approval` without recompiling), you can load a YAML/JSON file yourself and
construct `ModuleAnnotations` via `serde`:

The Rust `Registry` exposes two registration entry points:

- `register_module(module_id, module)` — **spec-compliant two-argument form**
  (parity with `apcore-python.Registry.register` and
  `apcore-typescript.Registry.register`). The `ModuleDescriptor` is
  auto-generated from the module's `input_schema()` / `output_schema()` /
  `description()` / annotations.
- `register(module_id, module, descriptor)` — **extended three-argument form**
  for supplying a pre-built descriptor (e.g. one with overlay annotations
  loaded from YAML).

```rust
use apcore::module::{Module, ModuleAnnotations};
use apcore::registry::{ModuleDescriptor, Registry};
use std::collections::HashMap;

// (1) Auto descriptor — parity with Python / TypeScript `register`.
let registry = Registry::new();
registry.register_module("my.module", Box::new(MyModule))?;

// (2) Explicit descriptor — Rust-only extended form. Construct the
// descriptor fully (there is no `Default` impl that matches every field);
// the literal below mirrors the one used by `APCore::register` internally
// (see src/client.rs).
let yaml: serde_json::Value = serde_yaml_ng::from_reader(file)?;
let annotations: ModuleAnnotations = serde_json::from_value(yaml)?;

let module = Box::new(MyModule);
let descriptor = ModuleDescriptor {
    module_id: "my.module".to_string(),
    name: None,
    description: module.description().to_string(),
    documentation: None,
    input_schema: module.input_schema(),
    output_schema: module.output_schema(),
    version: "1.0.0".to_string(),
    tags: vec![],
    annotations: Some(annotations), // overlay annotations from YAML
    examples: vec![],
    metadata: HashMap::new(),
    display: None,
    sunset_date: None,
    dependencies: vec![],
    enabled: true,
};
registry.register("my.module", module, descriptor)?;
```

## Examples

The `examples/` directory contains runnable demos. Run any example with:

```bash
cargo run --example simple_client
cargo run --example greet
cargo run --example get_user
cargo run --example send_email
cargo run --example cancel_token
cargo run --example execution_policy   # external governance overrides (#76)
cargo run --example format_date        # YAML binding loader
```

---

### `simple_client` — Implement `Module` and execute directly

Defines two modules (`AddModule`, `GreetModule`), builds an `Identity` + `Context`, and calls them directly without a registry.

```rust
use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::Module;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

struct AddModule;

#[async_trait]
impl Module for AddModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "a": { "type": "integer" },
                "b": { "type": "integer" }
            },
            "required": ["a", "b"]
        })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "result": { "type": "integer" } } })
    }
    fn description(&self) -> &str { "Add two integers" }

    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let a = input["a"].as_i64().unwrap_or(0);
        let b = input["b"].as_i64().unwrap_or(0);
        Ok(json!({ "result": a + b }))
    }
}

#[tokio::main]
async fn main() {
    let identity = Identity::new(
        "user-1".to_string(),
        "user".to_string(),
        vec!["user".to_string()],
        HashMap::new(),
    );
    let ctx: Context<Value> = Context::new(identity);
    let module = AddModule;

    let result = module.execute(json!({"a": 10, "b": 5}), &ctx).await.unwrap();
    println!("{result}"); // {"result":15}
}
```

---

### `greet` — Typed input/output with `serde` and default field values

Uses `#[serde(default)]` for optional fields and shows schema introspection and validation error handling.

```rust
use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::Module;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
struct GreetInput {
    name: String,
    #[serde(default = "default_greeting")]
    greeting: String,
}
fn default_greeting() -> String { "Hello".to_string() }

#[derive(Debug, Serialize, Deserialize)]
struct GreetOutput { message: String }

struct GreetModule;

#[async_trait]
impl Module for GreetModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name":     { "type": "string", "description": "Name of the person to greet" },
                "greeting": { "type": "string", "description": "Custom greeting prefix", "default": "Hello" }
            },
            "required": ["name"]
        })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "message": { "type": "string" } }, "required": ["message"] })
    }
    fn description(&self) -> &str { "Greet a user by name" }

    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let req: GreetInput = serde_json::from_value(input)
            .map_err(|e| ModuleError::new(apcore::errors::ErrorCode::GeneralInvalidInput, e.to_string()))?;
        Ok(serde_json::to_value(GreetOutput { message: format!("{}, {}!", req.greeting, req.name) }).unwrap())
    }
}

#[tokio::main]
async fn main() {
    let identity = Identity::new("agent-1".to_string(), "agent".to_string(), vec![], HashMap::new());
    let ctx: Context<Value> = Context::new(identity);
    let module = GreetModule;

    let out = module.execute(json!({"name": "Alice", "greeting": "Good morning"}), &ctx).await.unwrap();
    println!("{out}"); // {"message":"Good morning, Alice!"}

    let out = module.execute(json!({"name": "Bob"}), &ctx).await.unwrap();
    println!("{out}"); // {"message":"Hello, Bob!"}  ← default greeting applied

    // Schema introspection
    println!("{}", serde_json::to_string_pretty(&module.input_schema()).unwrap());

    // Missing required field → validation error
    let err = module.execute(json!({"greeting": "Hi"}), &ctx).await.unwrap_err();
    println!("Error: {err}");
}
```

---

### `get_user` — Readonly module with `ModuleAnnotations`

Demonstrates behavioral annotations (`readonly`, `idempotent`, `cacheable`), typed input/output schemas, and looking up records by ID.

```rust
use apcore::module::{Module, ModuleAnnotations};
// ...

fn get_user_annotations() -> ModuleAnnotations {
    ModuleAnnotations {
        readonly: true,
        idempotent: true,
        cacheable: true,
        cache_ttl: 60,
        ..Default::default()
    }
}
```

```
user-1: {"email":"alice@example.com","id":"user-1","name":"Alice"}
user-2: {"email":"bob@example.com","id":"user-2","name":"Bob"}
user-999: {"email":"unknown@example.com","id":"user-999","name":"Unknown"}
```

---

### `send_email` — Destructive module with sensitive fields

Schema-driven redaction follows local `$ref` definitions, escaped JSON Pointers,
anchors and references within scoped embedded `$id` resources, including recursive object and
array fields and composition branches. It uses a deep copy, retains configured
redaction and `_secret_` protection, and never retrieves external HTTP/file schemas.
Object roots retain their object shape; whole-value sensitivity applies to nested
fields. Schema indexing and positional array rules follow the declared dialect
(default 2020-12), so unknown keywords cannot shadow local sensitivity anchors.

Shows `x-sensitive: true` on schema fields (for log redaction), `ModuleAnnotations` with metadata, and behavioral annotation for destructive operations.

```rust
fn input_schema(&self) -> Value {
    json!({
        "type": "object",
        "properties": {
            "to":      { "type": "string" },
            "subject": { "type": "string" },
            "body":    { "type": "string" },
            "api_key": { "type": "string", "x-sensitive": true }  // redacted in logs
        },
        "required": ["to", "subject", "body", "api_key"]
    })
}
```

```rust
fn send_email_annotations() -> ModuleAnnotations {
    ModuleAnnotations {
        destructive: true,
        requires_approval: true,
        ..Default::default()
    }
}

fn send_email_examples() -> Vec<ModuleExample> {
    vec![ModuleExample {
        title: "Send a welcome email".to_string(),
        inputs: json!({ "to": "user@example.com", "subject": "Welcome!", "body": "...", "api_key": "sk-xxx" }),
        output: json!({ "status": "sent", "message_id": "msg-12345" }),
        description: None,
    }]
}
```

---

### `cancel_token` — Cooperative cancellation during long-running execution

`CancelToken::drain().await` waits for actual execution in that token's scope
and its descendants, including cleanup continuing after a timeout response.
It does not request cancellation or force termination. Call `cancel()` to
signal cooperative cancellation separately. `has_active_executions()` reports
whether the scope still owns execution; completed scopes retain no invocation
handles. A deadline continues to return `MODULE_TIMEOUT` immediately, while
successful active-task cancellation joins its owned background work, and
`AsyncTaskManager::shutdown` drains remaining local work. Cancelling an already
terminal task still returns `false` without interrupting its late cleanup.
These execution-draining APIs are unreleased additions
after version 0.32.2.

`CancelToken` is a cloneable, shared cancellation signal. Modules poll `token.is_cancelled()` between steps to stop early.

```rust
use apcore::cancel::CancelToken;

// Attach a token to the context
let mut ctx: Context<Value> = Context::new(identity);
let token = CancelToken::new();
ctx.cancel_token = Some(token.clone());

// Cancel from another task after 80 ms
tokio::spawn(async move {
    tokio::time::sleep(Duration::from_millis(80)).await;
    token.cancel();
});

// Module checks the token between steps
async fn execute(&self, input: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
    for i in 0..steps {
        if let Some(t) = &ctx.cancel_token {
            if t.is_cancelled() {
                return Err(ModuleError::new(ErrorCode::ExecutionCancelled, format!("cancelled at step {i}")));
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(json!({ "completed_steps": steps }))
}
```

```
=== Run 1: normal execution ===
  [SlowModule] Executing step 0...
  [SlowModule] Executing step 1...
  [SlowModule] Executing step 2...
Result: {"completed_steps":3}

=== Run 2: cancelled mid-flight ===
  [SlowModule] Executing step 0...
  [SlowModule] Executing step 1...
  [main] Sending cancel signal…
  [SlowModule] Cancelled at step 2
Error (expected): Execution cancelled after 2 steps
```

## Tests

Run all tests:

```bash
cargo test
```

> Most integration tests compile into one binary (`tests/it.rs`, `autotests = false`)
> for fast builds; the few files that touch process-global `Config`/env state stay
> as their own binaries (see the `[[test]]` entries in `Cargo.toml`). To add a new
> test file, add a `mod` line to `tests/it.rs` — or, if it registers Config
> namespaces / mutates env, a `[[test]]` entry instead.

Run a specific consolidated test file (now a module of the `it` binary):

```bash
cargo test --test it test_cancel
cargo test --test it test_errors
```

Run a specific test by name:

```bash
cargo test test_cancel_token
```

Run with output visible:

```bash
cargo test -- --nocapture
```

## Development

### Prerequisites

Install Rust via [rustup](https://rustup.rs):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Clone and build

```bash
git clone https://github.com/aiperceivable/apcore-rust.git
cd apcore-rust
cargo build
```

### Run tests

```bash
cargo test
```

### Run tests with output

```bash
cargo test -- --nocapture
```

### Run a specific test

```bash
cargo test test_cancel_token
```

### Lint and format

```bash
cargo fmt           # auto-format code
cargo clippy        # lint
```

### Build documentation

```bash
cargo doc --open
```

### Check without building

```bash
cargo check
```

## License

Apache-2.0

## Links

- **Documentation**: [https://aiperceivable.github.io/apcore/](https://aiperceivable.github.io/apcore/)
- **Website**: [aiperceivable.com](https://aiperceivable.com)
- **GitHub**: [aiperceivable/apcore-rust](https://github.com/aiperceivable/apcore-rust)
- **crates.io**: [apcore](https://crates.io/crates/apcore)
- **Issues**: [GitHub Issues](https://github.com/aiperceivable/apcore-rust/issues)
- **Discussions**: [GitHub Discussions](https://github.com/aiperceivable/apcore-rust/discussions)
