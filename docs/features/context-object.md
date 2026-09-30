---
description: "Per-invocation Context with trace_id, caller_id, call_chain, executor ref, identity, optional cancel_token/services, a shared data bag and typed ContextKey access; serializable cross-process."
---

# Context Object

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.7 Context Parameter Specification.

## Overview

`Context` is the per-invocation state object carried through every `execute()` call. It exposes the trace identifier, the call chain, the caller identity, an executor reference for inter-module calls, redacted copies of the payload for safe logging, and a free-form `data` map for pipeline state. The design follows Go's `context.Context` and OpenTelemetry Context: the fields the engine itself depends on are first-class, and everything else goes in `data`.

For caller-identity semantics see [Identity System](./identity-system.md). For `Context.create` and executor binding see [Core Executor](./core-executor.md#contract-contextcreate).

## Requirements

- Context MUST carry a `trace_id` that identifies the call tree and is preserved across all child invocations.
- Context MUST carry the `caller_id` of the module that initiated the current call, or null for top-level calls.
- Context MUST carry the `call_chain` (module IDs from root to the current invocation), maintained by the Executor.
- Context MUST carry an `executor` reference so modules can call other modules. It is **bound by the Executor** at pipeline entry, not by `Context.create()`; see [Executor binding to Context](./core-executor.md#contract-executor-binding-to-context).
- Context SHOULD carry an `identity` describing the caller (used by the ACL).
- Context SHOULD expose `redacted_inputs` / `redacted_output` — the payload with `x-sensitive` fields replaced by `***REDACTED***` — so middleware can log safely.
- Context MAY carry a cooperative `cancel_token` and a dependency-injection `services` container.
- Context MUST own a `data` map shared by reference along the call tree.
- Context MUST support a serialization round-trip for cross-process transfer, excluding runtime-only fields.

## Technical Design

### Field Layout

| Category | Fields | Rationale |
|----------|--------|-----------|
| Framework engine dependency | `trace_id`, `caller_id`, `call_chain`, `executor` | Removing any one breaks the framework |
| Almost always needed | `identity` | ACL is first-class; it needs a standardized "who" |
| Logging safety | `redacted_inputs`, `redacted_output` | Payload copies safe to log |
| Optional extensions | `cancel_token`, `services`, `global_deadline` | Cooperative cancellation, DI, call-tree budget |
| Generic bag | `data` | Pipeline state, span stacks, locale, feature flags |

### Field Constraints

| Field | Type | Level | Thread Safety | Serialized | Notes |
|-------|------|-------|---------------|------------|-------|
| `trace_id` | string (32-char lowercase hex) | MUST | Read-only | Yes | Generated or taken from a valid `trace_parent`. Not a `Context.create()` input. |
| `caller_id` | string \| null | MUST | Read-only | Yes | Null at top level; set by `Context.child()`. Not a `Context.create()` input. |
| `call_chain` | list[string] | MUST | Read-only | Yes | Managed by the Executor; bounded by `executor.max_call_depth`. |
| `executor` | Executor \| null | MUST (after binding) | Thread-safe | No | Bound by the Executor at pipeline entry. |
| `identity` | Identity \| null | SHOULD | Read-only | Yes | |
| `redacted_inputs` | map \| null | SHOULD | Read-only | Yes, when set | Set by pipeline Step 3 (`module_lookup`) from the incoming inputs, refreshed at Step 7 (`input_validation`) after before-middleware. |
| `redacted_output` | map \| null | SHOULD | Read-only | Yes, when set | Set by Step 9 (`output_validation`). |
| `cancel_token` | CancelToken \| null | MAY | Thread-safe | No | A `Context.create()` parameter. See [Cancellation](./cancellation.md). |
| `services` | T \| null | MAY | Read-only | No | Caller-supplied DI container only — never framework-owned fields. |
| `global_deadline` | epoch seconds \| null | MAY | Read-only | No | See [`global_deadline`](./core-executor.md#global_deadline-representation-and-lifetime). |
| `data` | map[string, Any] | MUST | Not thread-safe | Keys not starting with `_` | Shared by reference. |

`context.logger` is **deprecated** in all three SDKs and removed at v2.0 — log through the host application's logger instead ([Logging from a module](#logging-from-a-module)).

### Call-Chain Safety

The Executor checks the call chain at pipeline Step 2, before any lookup or module code runs: a depth limit (`executor.max_call_depth`, default 32 → `CALL_DEPTH_EXCEEDED`), circular-call detection (`CIRCULAR_CALL`), and a bound on direct self-recursion (`executor.max_module_repeat`, default 3 → `CALL_FREQUENCY_EXCEEDED`). Modules do not re-implement these checks. The exact rules — including why A→B→A is circular while A→A is not — are Algorithm A20, explained in [Call Chain Guard](./call-chain-guard.md).

### Concurrency Semantics

- `trace_id`, `caller_id`, `identity` MUST NOT be mutated after Context creation.
- `call_chain` is managed by the Executor; module code MUST NOT modify it.
- `data` is shared by reference. In concurrent scenarios callers SHOULD synchronize externally (Rust guards it with a `parking_lot::RwLock`; never hold the guard across an `.await`).
- `executor` references MUST be thread-safe.

See [PROTOCOL_SPEC §12.7.2 Context.data Sharing Semantics](../spec/protocol-spec.md#1272-contextdata-sharing-semantics).

### `data` Key Convention

`context.data` is one shared map. Three kinds of writer use it, and each has its own part of the key space:

| Prefix | Owner | Examples |
|--------|-------|----------|
| `_apcore.` | The framework: built-in middleware, the executor, `Context.create()` | `_apcore.mw.tracing.spans`, `_apcore.mw.logging.start_time`, `_apcore.trace.flags` |
| `ext.<vendor>.` | Reusable third-party code: middleware, adapters, integrations shipped as packages | `ext.acme.retry.count`, `ext.acme.tenant` |
| no prefix | The application's own modules and entry point | `locale`, `raw_records`, `x-correlation-id` |

- `_apcore.*` is reserved. Application and third-party code MUST NOT write it, and the framework MUST NOT write `ext.*`. The canonical rules — and the `validate_context_key(writer, key)` helper each SDK exports — are in [Middleware System](./middleware-system.md#context-namespacing).
- Unprefixed keys belong to the application that owns the call. `x-correlation-id` is the well-known key for an external correlation ID ([§5.7](../spec/protocol-spec.md#57-context-parameter-specification)).
- Keys starting with `_` are **not serialized** when a Context crosses a process boundary. Keys named `_secret_*` also match the default `obs.redaction.sensitive_keys` patterns, so their values are redacted in logs.

Framework hierarchy under `_apcore.`:

| Key pattern | Subsystem | Example |
|-------------|-----------|---------|
| `_apcore.mw.{middleware}.{field}` | Built-in middleware state | `_apcore.mw.logging.start_time` |
| `_apcore.mw.{middleware}.{field}.{module_id}` | Per-module middleware state | `_apcore.mw.retry.count.email.send` |
| `_apcore.trace.{field}` | Inbound W3C trace state from `Context.create(trace_parent=...)` | `_apcore.trace.flags`, `_apcore.trace.state` |
| `_apcore.executor.{field}` | Executor internals | `_apcore.executor.redacted_output` |

### Redacted Payloads

`context.redacted_inputs` is the input map with every field marked `x-sensitive: true` — and every key matching the `obs.redaction.*` rules — replaced by `***REDACTED***`. Middleware that emits structured logs MUST prefer it over the raw `inputs` argument. The module itself always receives the real values. Redaction rules are described in [Redaction](./redaction.md).

### Field Independence — `inputs` vs. `data`

|                    | `inputs` | `data` |
|--------------------|----------|--------|
| Semantics          | Explicit input for this call | Shared pipeline state |
| Schema             | Validated by `input_schema` | No schema, free read/write |
| Source             | Passed by the caller | Accumulated along the call tree |
| Lifecycle          | Per call | Shared across the whole call tree |
| Passing            | By value | By reference |

### Auto-Propagation Across Calls

When the Executor dispatches a module call with an existing Context, it derives a child Context that:

1. Keeps `trace_id` unchanged.
2. Sets `caller_id` to the previous module ID (null at top level).
3. Appends the target module ID to `call_chain`.
4. Keeps `identity`, `executor`, `cancel_token`, `services` and `global_deadline`.
5. Shares `data` by reference (same map instance).

```text
Top-level call:
  trace_id   = "4bf92f3577b34da6a3ce929d0e0e4736"
  caller_id  = None
  call_chain = []
  data       = {"locale": "zh-CN"}            ← same map

  ↓ Calls orchestrator.user_register

orchestrator.user_register:
  trace_id   = "4bf92f3577b34da6a3ce929d0e0e4736"
  caller_id  = None
  call_chain = ["orchestrator.user_register"]
  data       = {"locale": "zh-CN"}            ← same map (shared reference)

  ↓ Calls executor.email.send_email

executor.email.send_email:
  trace_id   = "4bf92f3577b34da6a3ce929d0e0e4736"
  caller_id  = "orchestrator.user_register"
  call_chain = ["orchestrator.user_register", "executor.email.send_email"]
  data       = {"locale": "zh-CN"}            ← same map (shared reference)
```

### Serialization

For cross-process transfer (distributed execution, task queues) a Context serializes to a snake_case JSON object:

| Language | Serialize | Deserialize |
|----------|-----------|-------------|
| Python | `context.serialize()` | `Context.deserialize(data)` |
| TypeScript | `context.toJSON()` (used by `JSON.stringify`) | `Context.fromJSON(data)` |
| Rust | `context.serialize()` | `Context::deserialize(value)` |

TypeScript's `serialize()` / `deserialize()` remain as deprecated aliases of `toJSON()` / `fromJSON()`.

- Serialized: `_context_version: 1`, `trace_id`, `caller_id`, `call_chain`, `identity`, `data` (keys not starting with `_`), and `redacted_inputs` / `redacted_output` when set.
- Not serialized (runtime-only): `executor`, `cancel_token`, `services`, `global_deadline`.
- A `_context_version` greater than 1 logs a warning and deserialization proceeds best-effort.

After deserialization `executor` is null; the receiving Executor binds itself on the first call ([Executor binding to Context](./core-executor.md#contract-executor-binding-to-context)), synthesizes a fresh local `CancelToken`, and recomputes `global_deadline` from its own configuration. `services` is re-injected by the application.

## Edge Cases

| Scenario | Behavior | Level |
|----------|----------|-------|
| Non-serializable value stored in `context.data` | Allowed in-process; fails when crossing processes | MUST |
| `call_chain` longer than `max_call_depth` | `CALL_DEPTH_EXCEEDED` | MUST |
| `trace_parent` carries an invalid trace ID | Log WARN and generate a fresh 32-char hex `trace_id` | MUST |
| `data` key written by parent and child | Last write wins (map semantics) | MUST |
| Concurrent modification of `data` from multiple threads | Race condition; callers SHOULD synchronize | SHOULD |

**Best practices:**

- Avoid storing large objects (>1 MB) in `context.data`; use an external cache.
- Use `ContextKey[T]` for state with a stable schema (below).

## Typed Access via `ContextKey[T]`

`context.data` is untyped: two pieces of code that pick the same string key overwrite each other silently, and a reader has to know the expected type from documentation.

```python
# In one middleware
context.data["retry_count"] = 3            # int

# In another middleware, imported later
context.data["retry_count"] = "three"      # str, silently overwrites
```

`ContextKey[T]` combines a namespaced string identifier (so collisions are visible in review) with a type parameter (so the type checker catches misuse), and puts the accessors on the **key** — `KEY.set(ctx, value)`, `KEY.get(ctx, default)`, `KEY.exists(ctx)`, `KEY.delete(ctx)`, `KEY.scoped(suffix)`. Keeping the methods on the key lets it carry both its name and its type parameter without changing the `Context` class. `ContextKey` is for middleware and custom-step extension state; state the pipeline itself needs (the resolved module, validated inputs and outputs) lives on the pipeline context, not in `data`.

### The `ContextKey[T]` API

=== "Python"

    ```python
    from apcore import Context, ContextKey

    # Define keys once, near where the state's schema is defined.
    RETRY_COUNT: ContextKey[int] = ContextKey("ext.myapp.retry.count")
    RETRY_DEADLINE_MS: ContextKey[int] = ContextKey("ext.myapp.retry.deadline_ms")

    def use_keys(ctx: Context) -> None:
        RETRY_COUNT.set(ctx, 3)
        RETRY_DEADLINE_MS.set(ctx, 5000)

        # Type checker knows this is int | None
        attempts: int | None = RETRY_COUNT.get(ctx)

        # With a default
        attempts = RETRY_COUNT.get(ctx, default=0)

        if RETRY_COUNT.exists(ctx):
            RETRY_COUNT.delete(ctx)

    use_keys(Context.create())
    ```

=== "TypeScript"

    ```typescript
    import { Context, ContextKey } from "apcore-js";

    export const RETRY_COUNT = new ContextKey<number>("ext.myapp.retry.count");
    export const RETRY_DEADLINE_MS = new ContextKey<number>("ext.myapp.retry.deadline_ms");

    export function useKeys(ctx: Context): void {
        RETRY_COUNT.set(ctx, 3);
        RETRY_DEADLINE_MS.set(ctx, 5000);

        const attempts: number | undefined = RETRY_COUNT.get(ctx);
        console.log(attempts);

        if (RETRY_COUNT.exists(ctx)) {
            RETRY_COUNT.delete(ctx);
        }
    }

    useKeys(Context.create());
    ```

=== "Rust"

    ```rust
    use apcore::{Context, ContextKey};
    use serde_json::Value;

    pub static RETRY_COUNT: ContextKey<u32> = ContextKey::new("ext.myapp.retry.count");
    pub static RETRY_DEADLINE_MS: ContextKey<u64> = ContextKey::new("ext.myapp.retry.deadline_ms");

    pub fn use_keys(ctx: &Context<Value>) {
        RETRY_COUNT.set(ctx, 3u32);
        RETRY_DEADLINE_MS.set(ctx, 5000u64);

        let attempts: Option<u32> = RETRY_COUNT.get(ctx);
        println!("{attempts:?}");

        if RETRY_COUNT.exists(ctx) {
            RETRY_COUNT.delete(ctx);
        }
    }

    fn main() {
        let ctx: Context<Value> = Context::create(None, None, None, None, Value::Null, None);
        use_keys(&ctx);
    }
    ```

For per-module sub-keys use `.scoped(suffix)`. By convention a key that is only ever used scoped is named with a `_BASE` suffix:

```python
from apcore import Context, ContextKey

REQUEST_COUNT_BASE: ContextKey[int] = ContextKey("ext.myapp.requests")

def count_request(ctx: Context, module_id: str) -> int:
    key = REQUEST_COUNT_BASE.scoped(module_id)   # "ext.myapp.requests.<module_id>"
    count = key.get(ctx, default=0) + 1
    key.set(ctx, count)
    return count
```

In Rust, values round-trip through `serde_json`: `get` requires `T: DeserializeOwned` and `set` requires `T: Serialize`, declared in separate `impl` blocks so a key that is only read (or only written) needs only one bound.

## Contract: ContextKey[T]

Normative behavioral contract for the typed accessor. The methods are defined on the **key** (`ContextKey`), not on `Context`. A key is immutable — a frozen dataclass in Python, a value type in Rust, a `readonly` `name` in TypeScript.

A key wraps a single `name` string. `ContextKey(name)` and `key.scoped(suffix)` are the only constructors; `scoped(suffix)` returns a new key named `{name}.{suffix}` and never mutates the receiver.

### Inputs

- `ContextKey(name)` — `name` (string, required) is the identifier into `context.data`. It shares the namespace of raw string keys (see *Namespace Convention* below).
- `set(ctx, value)` — `ctx` (context-like object exposing a `data` map, required); `value` (`T`, required).
- `get(ctx, default=None)` — `ctx` (required); `default` (`T`, optional) — returned when the key is absent.
- `exists(ctx)` — `ctx` (required).
- `delete(ctx)` — `ctx` (required).
- `scoped(suffix)` — `suffix` (string, required).

### Errors

- None. `get` reports a missing key by returning the default (or `None` / `undefined`); `delete` is a no-op on an absent key.
- Rust: `get` returns `None` when the stored value cannot be deserialized into `T`, and `set` drops a value that cannot be serialized — neither panics.

### Returns

- `set(ctx, value)` — nothing.
- `get(ctx)` — `T | None` (Python), `T | undefined` (TypeScript), `Option<T>` (Rust). `get(ctx, default)` returns `default` when the key is absent.
- `exists(ctx)` — `true` if `name` is present in `context.data`.
- `delete(ctx)` — nothing; removes `name` if present.
- `scoped(suffix)` — a new `ContextKey[T]` named `{name}.{suffix}`.

### Properties

- async: false.
- thread_safe: single map reads/writes; Rust guards `context.data` with a read/write lock. Cross-key atomicity is not provided.
- pure: `get` and `exists` are read-only; `set` and `delete` mutate `context.data`; `scoped` allocates a new key.
- idempotent: `set` (same value), `delete`, `exists` and `get`.

### Namespace Convention (Normative)

`ContextKey` identifiers and raw string keys are two views of one map, so the [`data` Key Convention](#data-key-convention) applies unchanged — `ContextKey("_apcore.foo")` and `context.data["_apcore.foo"]` are the same slot:

- **MUST** — Identifiers starting with `_apcore.` are reserved for the framework. Third-party code MUST NOT define `ContextKey`s with that prefix.
- **MUST** — `ContextKey`s shipped in reusable third-party code MUST use `ext.<vendor>.` (e.g. `ext.my_company.retry.count`).
- **SHOULD** — For data with a stable schema, code SHOULD use `ContextKey[T]` rather than raw `context.data[...]` access.

### Framework-Reserved `ContextKey` Slots (Informative)

Each SDK exports these constants (`SCREAMING_SNAKE_CASE` in Python and Rust, exported `const` in TypeScript); the identifier strings are identical across languages. Third-party code MUST NOT redefine them.

| Constant | Identifier | Type | Purpose |
|----------|------------|------|---------|
| `TRACING_SPANS` | `_apcore.mw.tracing.spans` | list | Stack of active spans for the current call tree |
| `TRACING_SAMPLED` | `_apcore.mw.tracing.sampled` | bool | Whether this trace is sampled for export |
| `METRICS_STARTS` | `_apcore.mw.metrics.starts` | list | Start markers used by `MetricsMiddleware` |
| `LOGGING_START` | `_apcore.mw.logging.start_time` | float (epoch s) | Start time recorded by `LoggingMiddleware.before()` |
| `REDACTED_OUTPUT` | `_apcore.executor.redacted_output` | map | Executor-redacted snapshot of the call output |
| `RETRY_COUNT_BASE` | `_apcore.mw.retry.count` | int | Base key for `RetryMiddleware`; scoped per target module |

Python additionally exports `USAGE_STARTS` (`_apcore.mw.usage.starts`) and `LOGGING_STARTS` (`_apcore.mw.logging.starts`). Framework middleware also writes some `_apcore.*` keys by raw string (for example `_apcore.mw.circuit.state`).

## Usage

### Read-only access in modules

=== "Python"

    ```python
    from apcore import Context

    class DeleteUserModule:
        description = "Delete a user (admin only)"
        input_schema = {"type": "object", "properties": {"user_id": {"type": "string"}}}
        output_schema = {"type": "object"}

        def execute(self, inputs: dict, context: Context) -> dict:
            if not context.identity:
                return {"success": False, "error": "Authentication required"}
            if "admin" not in context.identity.roles:
                return {"success": False, "error": "Admin permission required"}

            # delete inputs["user_id"] here
            return {"success": True, "operated_by": context.identity.id}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from "@sinclair/typebox";
    import type { Context } from "apcore-js";

    export class DeleteUserModule {
        readonly description = "Delete a user (admin only)";
        readonly inputSchema = Type.Object({ userId: Type.String() });
        readonly outputSchema = Type.Object({});

        async execute(inputs: Record<string, unknown>, ctx: Context): Promise<Record<string, unknown>> {
            if (!ctx.identity) return { success: false, error: "Authentication required" };
            if (!ctx.identity.roles.includes("admin")) {
                return { success: false, error: "Admin permission required" };
            }
            // delete inputs.userId here
            return { success: true, operatedBy: ctx.identity.id };
        }
    }
    ```

=== "Rust"

    ```rust
    use apcore::errors::{ErrorCode, ModuleError};
    use apcore::{Context, Module};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct DeleteUserModule;

    #[async_trait]
    impl Module for DeleteUserModule {
        fn description(&self) -> &str { "Delete a user (admin only)" }
        fn input_schema(&self) -> Value { json!({"type": "object"}) }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }

        async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let identity = ctx
                .identity
                .as_ref()
                .ok_or_else(|| ModuleError::new(ErrorCode::ACLDenied, "Authentication required"))?;
            if !identity.roles().iter().any(|r| r == "admin") {
                return Err(ModuleError::new(ErrorCode::ACLDenied, "Admin permission required"));
            }
            let _user_id = inputs["user_id"].as_str().unwrap_or_default();
            // delete the user here
            Ok(json!({"success": true, "operated_by": identity.id()}))
        }
    }

    fn main() {}
    ```

### Calling another module

The module passes its own Context to `context.executor.call(...)`; the Executor derives the child Context (updates `caller_id` and `call_chain`).

=== "Python"

    ```python
    from apcore import Context

    class UserRegisterModule:
        description = "Register a new user and send a welcome email"
        input_schema = {"type": "object", "properties": {"email": {"type": "string"}}}
        output_schema = {"type": "object"}

        def execute(self, inputs: dict, context: Context) -> dict:
            user_id = "u-123"  # create the user here
            result = context.executor.call(
                "executor.email.send_email",
                {"to": inputs["email"], "subject": "Welcome", "body": "..."},
                context,
            )
            return {"user_id": user_id, "email_sent": result["success"]}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from "@sinclair/typebox";
    import type { Context, Executor } from "apcore-js";

    export class UserRegisterModule {
        readonly description = "Register a new user and send a welcome email";
        readonly inputSchema = Type.Object({ email: Type.String() });
        readonly outputSchema = Type.Object({});

        async execute(inputs: Record<string, unknown>, context: Context): Promise<Record<string, unknown>> {
            const userId = "u-123"; // create the user here
            const executor = context.executor as Executor;
            const result = await executor.call(
                "executor.email.send_email",
                { to: inputs.email, subject: "Welcome", body: "..." },
                context,
            );
            return { userId, emailSent: Boolean(result.success) };
        }
    }
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Executor, Module, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::Arc;

    // Rust's Context carries the executor as an opaque handle, so a module that
    // calls other modules holds its own Executor reference.
    pub struct UserRegisterModule {
        executor: Arc<Executor>,
    }

    #[async_trait]
    impl Module for UserRegisterModule {
        fn description(&self) -> &str { "Register a new user and send a welcome email" }
        fn input_schema(&self) -> Value { json!({"type": "object"}) }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }

        async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let email = inputs["email"].as_str().unwrap_or_default();
            let user_id = "u-123"; // create the user here

            let result = self
                .executor
                .call(
                    "executor.email.send_email",
                    json!({"to": email, "subject": "Welcome", "body": "..."}),
                    Some(ctx),
                    None,
                )
                .await?;

            Ok(json!({
                "user_id": user_id,
                "email_sent": result["success"].as_bool().unwrap_or(false),
            }))
        }
    }

    fn main() {}
    ```

### AI orchestration via `data`

=== "Python"

    ```python
    from apcore import APCore, Context, Identity

    client = APCore()
    user_identity = Identity(id="user-42", type="user", roles=("analyst",))

    # The Executor binds itself on the first call.
    context = Context.create(identity=user_identity)
    context.data["task_info"] = {"type": "report", "date": "2024-01"}

    client.executor.call("report.fetch", {}, context)
    # report.fetch writes context.data["raw_records"]

    client.executor.call("report.analyze", {}, context)
    # report.analyze reads context.data["raw_records"] and writes context.data["analysis"]

    client.executor.call("report.render", {}, context)
    # report.render reads both prior results from context.data
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Context, createIdentity } from "apcore-js";

    const client = new APCore();
    const userIdentity = createIdentity("user-42", "user", ["analyst"]);

    // The Executor binds itself on the first call.
    const context = Context.create(userIdentity);
    context.data["task_info"] = { type: "report", date: "2024-01" };

    await client.executor.call("report.fetch", {}, context);
    // report.fetch writes context.data["raw_records"]

    await client.executor.call("report.analyze", {}, context);
    // report.analyze reads context.data["raw_records"] and writes context.data["analysis"]

    await client.executor.call("report.render", {}, context);
    // report.render reads both prior results from context.data
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, Context, Identity, ModuleError};
    use serde_json::{json, Value};
    use std::collections::HashMap;

    async fn run(client: &APCore) -> Result<(), ModuleError> {
        let user_identity = Identity::new(
            "user-42".into(),
            "user".into(),
            vec!["analyst".into()],
            HashMap::new(),
        );

        let context: Context<Value> = Context::create(
            Some(user_identity), // identity
            None,                // trace_parent
            None,                // cancel_token
            None,                // data
            Value::Null,         // services
            None,                // global_deadline
        );

        context
            .data
            .write()
            .insert("task_info".into(), json!({"type": "report", "date": "2024-01"}));

        client.executor().call("report.fetch", json!({}), Some(&context), None).await?;
        // report.fetch writes context.data["raw_records"]

        client.executor().call("report.analyze", json!({}), Some(&context), None).await?;
        // report.analyze reads context.data["raw_records"] and writes context.data["analysis"]

        client.executor().call("report.render", json!({}), Some(&context), None).await?;
        // report.render reads both prior results from context.data
        Ok(())
    }

    fn main() {}
    ```

### Logging from a module

`context.logger` is deprecated and removed at v2.0: its output is fixed at stderr, `info` level, JSON, outside whatever logging the host has configured, and a `Context` carries no `Config` through which it could be configured. Log through the logger your application already uses and carry the correlation fields — `trace_id` above all — explicitly. apcore does not own the host's logging policy ([§9.2.4](../spec/protocol-spec.md#924-deprecated-configuration-keys), D-67).

=== "Python"

    ```python
    import logging

    from apcore import Context

    logger = logging.getLogger(__name__)


    class SendEmailModule:
        description = "Send a transactional email"
        input_schema = {"type": "object", "properties": {"to": {"type": "string"}}}
        output_schema = {"type": "object"}

        def execute(self, inputs: dict, context: Context) -> dict:
            logger.info(
                "Sending email to %s",
                inputs["to"],
                extra={
                    "trace_id": context.trace_id,
                    "caller_id": context.caller_id,
                    "module_id": context.call_chain[-1] if context.call_chain else None,
                },
            )
            return {"success": True}
    ```

=== "TypeScript"

    ```typescript
    import { Type } from "@sinclair/typebox";
    import type { Context } from "apcore-js";

    // Whatever the application installed — pino, winston, a console wrapper.
    const logger = {
        info(fields: Record<string, unknown>, msg: string): void {
            console.log(JSON.stringify({ ...fields, msg }));
        },
    };

    export class SendEmailModule {
        readonly description = "Send a transactional email";
        readonly inputSchema = Type.Object({ to: Type.String() });
        readonly outputSchema = Type.Object({});

        async execute(inputs: Record<string, unknown>, context: Context): Promise<Record<string, unknown>> {
            logger.info(
                {
                    traceId: context.traceId,
                    callerId: context.callerId,
                    moduleId: context.callChain.at(-1) ?? null,
                },
                `Sending email to ${String(inputs.to)}`,
            );
            return { success: true };
        }
    }
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct SendEmailModule;

    #[async_trait]
    impl Module for SendEmailModule {
        fn description(&self) -> &str { "Send a transactional email" }
        fn input_schema(&self) -> Value { json!({"type": "object"}) }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }

        async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let to = inputs["to"].as_str().unwrap_or_default();
            tracing::info!(
                trace_id = %ctx.trace_id,
                caller_id = ?ctx.caller_id,
                module_id = ?ctx.call_chain.last(),
                "Sending email to {to}"
            );
            Ok(json!({"success": true}))
        }
    }

    fn main() {}
    ```

`ObsLoggingMiddleware` is a different facility: it emits one execution record per call (inputs, outputs, timing), redacted per [§10.6.1](../spec/protocol-spec.md#1061-configured-redaction-rules-obsredaction). Install it for automatic call auditing; it does not give module code a place to write an ad-hoc line.

### Middleware redaction

=== "Python"

    ```python
    import logging

    from apcore import Context
    from apcore.middleware import Middleware

    log = logging.getLogger(__name__)

    class SafeLoggingMiddleware(Middleware):
        def before(self, module_id: str, inputs: dict, context: Context) -> dict | None:
            # Safe: log the redacted copy, never the raw inputs
            log.info("Calling %s", module_id, extra={"inputs": context.redacted_inputs})
            return None  # pass inputs through unchanged
    ```

=== "TypeScript"

    ```typescript
    import { Middleware, type Context } from "apcore-js";

    export class SafeLoggingMiddleware extends Middleware {
        override before(
            moduleId: string,
            _inputs: Record<string, unknown>,
            context: Context,
        ): Record<string, unknown> | null {
            // Safe: log the redacted copy, never the raw inputs
            console.info(`Calling ${moduleId}`, { inputs: context.redactedInputs });
            return null; // pass inputs through unchanged
        }
    }
    ```

=== "Rust"

    ```rust
    use apcore::middleware::Middleware;
    use apcore::{Context, ModuleError};
    use async_trait::async_trait;
    use serde_json::Value;

    #[derive(Debug)]
    pub struct SafeLoggingMiddleware;

    #[async_trait]
    impl Middleware for SafeLoggingMiddleware {
        fn name(&self) -> &str { "safe_logging" }

        async fn before(
            &self,
            module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            // Safe: log the redacted copy, never the raw inputs
            tracing::info!(module_id = module_id, inputs = ?ctx.redacted_inputs, "Calling module");
            Ok(None) // pass inputs through unchanged
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }

        async fn on_error(
            &self,
            _module_id: &str,
            _inputs: Value,
            _error: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
    }

    fn main() {}
    ```

### Middleware state in `data`

Per-call middleware state belongs in `context.data` — never on the middleware instance, which is shared by concurrent calls ([§12.7.6](../spec/protocol-spec.md#1276-middleware-chain-atomicity)). A user middleware uses its own `ext.<vendor>.` keys:

=== "Python"

    ```python
    import time

    from apcore import Context
    from apcore.middleware import Middleware

    START = "ext.myapp.timing.start"
    DURATION = "ext.myapp.timing.duration_ms"

    class TimingMiddleware(Middleware):
        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            context.data[START] = time.time()
            return None

        def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
            started = context.data.pop(START, None)
            if started is not None:
                context.data[DURATION] = round((time.time() - started) * 1000)
            return None
    ```

=== "TypeScript"

    ```typescript
    import { Middleware, type Context } from "apcore-js";

    const START = "ext.myapp.timing.start";
    const DURATION = "ext.myapp.timing.duration_ms";

    export class TimingMiddleware extends Middleware {
        override before(_moduleId: string, _inputs: Record<string, unknown>, context: Context): null {
            context.data[START] = Date.now();
            return null;
        }

        override after(
            _moduleId: string,
            _inputs: Record<string, unknown>,
            _output: Record<string, unknown>,
            context: Context,
        ): null {
            const started = context.data[START] as number | undefined;
            delete context.data[START];
            if (started !== undefined) {
                context.data[DURATION] = Date.now() - started;
            }
            return null;
        }
    }
    ```

=== "Rust"

    ```rust
    use apcore::middleware::Middleware;
    use apcore::{Context, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::time::{SystemTime, UNIX_EPOCH};

    const START: &str = "ext.myapp.timing.start";
    const DURATION: &str = "ext.myapp.timing.duration_ms";

    fn now_ms() -> u64 {
        u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis())
            .unwrap_or(u64::MAX)
    }

    #[derive(Debug)]
    pub struct TimingMiddleware;

    #[async_trait]
    impl Middleware for TimingMiddleware {
        fn name(&self) -> &str { "myapp.timing" }

        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            ctx.data.write().insert(START.into(), json!(now_ms()));
            Ok(None)
        }

        async fn after(
            &self,
            _module_id: &str,
            _inputs: Value,
            _output: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let mut data = ctx.data.write();
            if let Some(started) = data.remove(START).and_then(|v| v.as_u64()) {
                data.insert(DURATION.into(), json!(now_ms().saturating_sub(started)));
            }
            Ok(None)
        }

        async fn on_error(
            &self,
            _module_id: &str,
            _inputs: Value,
            _error: &ModuleError,
            _ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            Ok(None)
        }
    }

    fn main() {}
    ```

## Common `data` Uses

| Purpose | Example keys | Notes |
|---------|--------------|-------|
| Pipeline intermediate state | `raw_records`, `analysis` | AI orchestration across several calls |
| Observability | `_apcore.mw.tracing.spans`, `_apcore.mw.tracing.sampled` | Written by the framework's `TracingMiddleware` |
| Internationalization | `locale`, `timezone` | Set at top level, read as needed |
| Feature flags | `feature_flags` | Set at top level |
| Request metadata | `x-correlation-id`, `client_ip`, `session_id` | Written at the entry layer |
| Third-party middleware state | `ext.<vendor>.…` | One vendor segment per package |

## Dependencies

- [Identity System](./identity-system.md) — `Identity` type, ACL integration, `ContextFactory`.
- [Core Executor](./core-executor.md) — `Context.create`, executor binding, child-context derivation, redaction.
- [Call Chain Guard](./call-chain-guard.md) — call-chain limits.
- [Cancellation](./cancellation.md) — `CancelToken` semantics.

## Testing Strategy

- Round-trip serialization tests covering all serialized fields, the `_`-prefix filter and `_context_version` handling.
- Call-chain bound tests for `CALL_DEPTH_EXCEEDED`, `CIRCULAR_CALL`, `CALL_FREQUENCY_EXCEEDED`.
- Redaction tests covering `redacted_inputs` / `redacted_output`.
- `data` reference-sharing tests across multi-step calls, and `ContextKey` accessor tests.
- Concurrency tests for read-only fields and unsynchronized `data` writes.

## Next Steps

- [Module Interface](./module-interface.md) — how `execute(inputs, context)` consumes Context.
- [Core Executor](./core-executor.md) — Context creation and propagation.
- [Identity System](./identity-system.md) — caller identity and ACL.
- [Middleware System](./middleware-system.md) — accessing Context inside middleware.
