# Context Object

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.7 Context Object.

## Overview

`Context` is the per-invocation state object carried through every `execute()` call. It exposes the framework's trace identifier, call chain, caller identity, an executor reference for inter-module calls, a context-aware logger with automatic redaction, and a free-form `data` map for pipeline state. The design follows industry precedent — Go's `context.Context` (independent fields + value bag), OpenTelemetry Context (KV bag), AutoGen `context_variables` (shared dict) — but elevates a small set of fields the apcore engine itself depends on, leaving everything else in `data`.

For caller-identity semantics, type values, and ACL integration see [Identity System](./identity-system.md). For Context creation and the `Context.create` contract see [Core Executor](./core-executor.md#contract-contextcreate).

## Requirements

- Context MUST carry a `trace_id` that uniquely identifies the call chain and is preserved across all child invocations.
- Context MUST carry the `caller_id` of the module that initiated the current call, or `None` for top-level calls.
- Context MUST carry the `call_chain` (ordered list of module IDs from root to current invocation), maintained automatically by the Executor.
- Context MUST carry an `executor` reference so modules can dispatch inter-module calls.
- Context SHOULD carry an `identity` describing the caller (used by ACL).
- Context SHOULD expose a `logger` that auto-injects `trace_id`, `module_id`, and `caller_id`.
- Context SHOULD expose `redacted_inputs` — the input dict with `x-sensitive` fields replaced by `"***REDACTED***"` — so middleware can log safely.
- Context MAY expose a cooperative `cancel_token` and a dependency-injection `services` container.
- Context MUST own a `data` dict shared by reference along the call chain for pipeline state flow.
- The framework MUST enforce call-chain safety bounds: depth limit, strict-cycle detection, and frequency limit.
- Context MUST support a serialization round-trip for cross-process transfer, excluding runtime-only fields.

## Technical Design

### Field Layout

| Category | Fields | Rationale |
|----------|--------|-----------|
| Framework engine dependency | `trace_id`, `caller_id`, `call_chain`, `executor` | Removing any one breaks the framework |
| Almost always needed | `identity` | ACL is first-class; needs a standardized "who" |
| SHOULD-level conveniences | `logger`, `redacted_inputs` | Logging safety hooks |
| Optional extensions | `cancel_token`, `services` | Cooperative cancellation, DI |
| Generic bag | `data` | Pipeline state, span IDs, locale, feature flags |

### Field Constraints

| Field | Type | Level | Limit | Thread Safety | Serializable |
|-------|------|-------|-------|---------------|--------------|
| `trace_id` | string (32-char hex) | MUST | 32 chars | Read-only, safe | MUST |
| `caller_id` | string \| null | MUST | 128 chars | Read-only, safe | MUST |
| `call_chain` | list[string] | MUST | Max depth 32 | Read-only, safe | MUST |
| `executor` | Executor | MUST | — | Thread-safe | MUST NOT |
| `identity` | Identity \| null | SHOULD | — | Read-only, safe | MUST |
| `logger` | ContextLogger | SHOULD | — | Thread-safe | MUST NOT |
| `redacted_inputs` | dict \| null | SHOULD | — | Read-only, safe | MAY |
| `cancel_token` | CancelToken \| null | MAY | — | Thread-safe | MUST NOT |
| `services` | T \| null | MAY | — | Read-only, safe | MUST NOT |
| `data` | dict[str, Any] | MUST | — | Not thread-safe | SHOULD |

### Call-Chain Safety

The Executor enforces three protection checks before dispatching any module call. Modules typically do not need to re-implement them.

```text
Executor.call(module_id, inputs, context)
    ├─ Depth check:     len(call_chain) >= max_call_depth         → CALL_DEPTH_EXCEEDED
    ├─ Cycle detection: module_id in call_chain                   → CIRCULAR_CALL
    └─ Frequency check: call_chain.count(module_id) >= max_repeat → CALL_FREQUENCY_EXCEEDED
```

- `max_call_depth` defaults to 32. Reaching it MUST raise `CALL_DEPTH_EXCEEDED`.
- Strict cycles (target ID already in `call_chain`) MUST raise `CIRCULAR_CALL`.
- Frequency limit (`executor.max_module_repeat`, default 3) catches repeated patterns like `A→B→C→B→C…` before the depth bound triggers and MUST raise `CALL_FREQUENCY_EXCEEDED`.

### Concurrency Semantics

- `trace_id`, `caller_id`, `identity` MUST NOT be mutated after Context creation.
- `call_chain` is managed exclusively by the Executor; module code MUST NOT modify it.
- `data` is shared by reference. In concurrent scenarios callers SHOULD synchronize externally.
- `executor` references MUST be thread-safe.

See [PROTOCOL_SPEC §12.7.2 Context.data Sharing Semantics](../spec/protocol-spec.md#1272-contextdata-sharing-semantics) for the full concurrency model.

### `data` Key Convention

`context.data` is a shared mutable dictionary. To prevent collisions between framework internals and module code, a single reserved prefix applies:

| Prefix | Reserved For | Example |
|--------|--------------|---------|
| `_apcore.` | Framework internals | See hierarchy below |
| (no prefix) | Application / module use | `user_session_id`, `pipeline_result` |

Hierarchy under `_apcore.`:

| Key pattern | Subsystem | Example |
|-------------|-----------|---------|
| `_apcore.mw.{middleware}.{field}` | Middleware private state | `_apcore.mw.logging.start_time` |
| `_apcore.mw.{middleware}.{field}.{id}` | Per-module middleware state | `_apcore.mw.retry.count.mod.a` |
| `_apcore.executor.{field}` | Executor internals | `_apcore.executor.global_deadline` |

- Framework components (middleware, executor, registry) MUST use `_apcore.` prefixed keys.
- Module developers MUST NOT use `_apcore.` prefixed keys.

### Logger and Redaction

`context.logger` automatically attaches `trace_id`, `module_id`, and `caller_id` to every log entry, so modules and middleware do not need to manually pass them.

`context.redacted_inputs` is the input dict with all fields marked `x-sensitive: true` replaced by `"***REDACTED***"`. Middleware that emits structured logs MUST prefer `redacted_inputs` over the raw `inputs` argument.

For sensitive payloads stored in `data`, use the `_secret_` prefix. Logging systems and serializers SHOULD strip keys whose names start with `_secret_` before emitting log records.

### Field Independence — `inputs` vs. `data`

|                    | `inputs` | `data` |
|--------------------|----------|--------|
| Semantics          | Explicit input for this call | Shared pipeline state |
| Schema             | Validated by `input_schema` | No schema, free read/write |
| Source             | Explicitly passed by caller | Accumulated along call chain |
| Lifecycle          | Per-call | Shared across the whole call chain |
| Passing            | By value | By reference |

### Auto-Propagation Across Calls

When the Executor dispatches a module call with an existing Context, it produces a child Context that:

1. Keeps `trace_id` unchanged.
2. Updates `caller_id` to the previous module ID (or `None` if top-level).
3. Appends the target module ID to `call_chain`.
4. Keeps `identity` unchanged.
5. Shares `data` by reference (same dict instance).

```text
Top-level call:
  trace_id   = "abc-123"
  caller_id  = None
  call_chain = []
  data       = {"locale": "zh-CN"}            ← same dict

  ↓ Calls orchestrator.user_register

orchestrator.user_register:
  trace_id   = "abc-123"
  caller_id  = None
  call_chain = ["orchestrator.user_register"]
  data       = {"locale": "zh-CN"}            ← same dict (reference shared)

  ↓ Calls executor.email.send_email

executor.email.send_email:
  trace_id   = "abc-123"
  caller_id  = "orchestrator.user_register"
  call_chain = ["orchestrator.user_register", "executor.email.send_email"]
  data       = {"locale": "zh-CN"}            ← same dict (reference shared)
```

### Serialization

For cross-process transfer (distributed execution, task queues) Context supports `serialize()` / `deserialize()`.

- Serialized fields: `trace_id`, `caller_id`, `call_chain`, `identity`, `data`, `redacted_inputs`.
- Skipped (runtime-only) fields: `executor`, `cancel_token`, `services`.
- A `_context_version: 1` field is included for forward compatibility.

After deserialization, the `executor` reference MUST be re-injected before the Context can be used for inter-module calls.

## Edge Cases

| Scenario | Behavior | Level |
|----------|----------|-------|
| `context.data` exceeds memory limit | Behavior depends on language runtime (OOM or exception); SHOULD log WARN | SHOULD |
| Non-serializable value stored in `context.data` | Allowed (in-memory passing); fails when crossing processes | MUST |
| `call_chain` reaches `max_call_depth` | Raise `CALL_DEPTH_EXCEEDED` | MUST |
| `trace_id` not valid 32-hex format | Log WARN and regenerate a 32-char hex trace_id | SHOULD |
| `caller_id` exceeds 128 chars | Log WARN, allow execution | SHOULD |
| `data` key conflict (parent/child same key) | Last write wins (dict semantics) | MUST |
| Concurrent modification of `data` from multiple threads | Race condition; callers SHOULD use locks | SHOULD |

**Best practices:**

- Avoid storing large objects (>1 MB) in `context.data`; use an external cache.
- Namespace keys to avoid collisions (e.g. `my_module:result`).

## Usage

### Read-only access in modules

=== "Python"

    ```python
    class DeleteUserModule(Module):
        def execute(self, inputs: dict, context: Context) -> dict:
            if not context.identity:
                return {"success": False, "error": "Authentication required"}
            if "admin" not in context.identity.roles:
                return {"success": False, "error": "Admin permission required"}

            self._delete_user(inputs["user_id"])
            return {"success": True, "operated_by": context.identity.id}
    ```

=== "TypeScript"

    ```typescript
    export class DeleteUserModule implements Module {
      async execute(inputs: { userId: string }, ctx: Context) {
        if (!ctx.identity) return { success: false, error: "Authentication required" };
        if (!ctx.identity.roles.includes("admin"))
          return { success: false, error: "Admin permission required" };

        await this.deleteUser(inputs.userId);
        return { success: true, operatedBy: ctx.identity.id };
      }
    }
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module};
    use apcore::errors::{ErrorCode, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    pub struct DeleteUserModule;

    #[async_trait]
    impl Module for DeleteUserModule {
        fn description(&self) -> &str { "Delete a user (admin only)" }
        fn input_schema(&self) -> Value { json!({ "type": "object" }) }
        fn output_schema(&self) -> Value { json!({ "type": "object" }) }

        async fn execute(
            &self,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let identity = ctx.identity.as_ref().ok_or_else(|| {
                ModuleError::new(ErrorCode::ACLDenied, "Authentication required")
            })?;
            if !identity.roles().iter().any(|r| r == "admin") {
                return Err(ModuleError::new(ErrorCode::ACLDenied, "Admin permission required"));
            }
            let user_id = inputs.get("user_id").and_then(|v| v.as_str()).unwrap_or_default();
            // self.delete_user(user_id).await?;
            let _ = user_id;
            Ok(json!({ "success": true, "operated_by": identity.id() }))
        }
    }
    ```

### Calling another module

=== "Python"

    ```python
    from apcore import Context, Module

    class UserRegisterModule(Module):
        def execute(self, inputs: dict, context: Context) -> dict:
            user_id = self._create_user(inputs)

            # context is propagated automatically;
            # framework updates caller_id and call_chain
            result = context.executor.call(
                module_id="executor.email.send_email",
                inputs={"to": inputs["email"], "subject": "Welcome", "body": "..."},
                context=context,
            )
            return {"user_id": user_id, "email_sent": result["success"]}
    ```

=== "TypeScript"

    ```typescript
    import type { Context, Executor } from 'apcore-js';

    interface RegisterInput { email: string; name: string }
    interface RegisterOutput { userId: string; emailSent: boolean }

    export class UserRegisterModule {
        async execute(inputs: RegisterInput, context: Context): Promise<RegisterOutput> {
            const userId = await this.createUser(inputs);

            // context is propagated automatically;
            // framework updates callerId and callChain
            const executor = context.executor as Executor;
            const result = await executor.call(
                'executor.email.send_email',
                { to: inputs.email, subject: 'Welcome', body: '...' },
                context,
            );
            return { userId, emailSent: Boolean(result.success) };
        }

        private async createUser(_inputs: RegisterInput): Promise<string> {
            return 'u-123';
        }
    }
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Executor, Module, ModuleError};
    use serde_json::{json, Value};
    use std::sync::Arc;

    #[derive(Debug)]
    pub struct UserRegisterModule {
        executor: Arc<Executor>,
    }

    #[async_trait::async_trait]
    impl Module for UserRegisterModule {
        fn id(&self) -> &str { "orchestrator.user_register" }
        fn description(&self) -> &str { "Register a new user and send welcome email" }

        async fn execute(
            &self,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let email = inputs["email"].as_str().unwrap_or_default();
            let user_id = self.create_user(&inputs).await?;

            // context is propagated automatically;
            // framework updates caller_id and call_chain
            let result = self.executor.call(
                "executor.email.send_email",
                json!({ "to": email, "subject": "Welcome", "body": "..." }),
                Some(ctx),
                None,
            ).await?;

            Ok(json!({
                "user_id": user_id,
                "email_sent": result["success"].as_bool().unwrap_or(false),
            }))
        }
    }

    impl UserRegisterModule {
        async fn create_user(&self, _inputs: &Value) -> Result<String, ModuleError> {
            Ok("u-123".to_string())
        }
    }
    ```

### AI orchestration via `data`

=== "Python"

    ```python
    from apcore import APCore, Context, Identity

    client = APCore()
    user_identity = Identity(id="user-42", type="user", roles=["analyst"])

    context = Context.create(executor=client.executor, identity=user_identity)
    context.data["task_info"] = {"type": "report", "date": "2024-01"}

    client.executor.call("module_fetch",   inputs={}, context=context)
    # module_fetch writes context.data["raw_records"] = [...]

    client.executor.call("module_analyze", inputs={}, context=context)
    # module_analyze reads context.data["raw_records"]
    # and writes context.data["analysis"]

    client.executor.call("module_report",  inputs={}, context=context)
    # module_report reads both prior outputs from context.data
    ```

=== "TypeScript"

    ```typescript
    import { APCore, Context, createIdentity } from 'apcore-js';

    const client = new APCore();
    const userIdentity = createIdentity('user-42', 'user', ['analyst']);

    const context = Context.create(client.executor, userIdentity);
    context.data['task_info'] = { type: 'report', date: '2024-01' };

    await client.executor.call('module_fetch',   {}, context);
    // module_fetch writes context.data['raw_records'] = [...]

    await client.executor.call('module_analyze', {}, context);
    // module_analyze reads context.data['raw_records']
    // and writes context.data['analysis']

    await client.executor.call('module_report',  {}, context);
    // module_report reads both prior outputs from context.data
    ```

=== "Rust"

    ```rust
    use apcore::{APCore, Context, Identity};
    use serde_json::{json, Value};
    use std::collections::HashMap;

    # async fn run(client: APCore) -> Result<(), Box<dyn std::error::Error>> {
    let user_identity = Identity::new(
        "user-42".into(),
        "user".into(),
        vec!["analyst".into()],
        HashMap::new(),
    );

    let context: Context<Value> = Context::create(
        Some(user_identity),
        Value::Null,
        None,
        None,
    );
    context.data.write().insert(
        "task_info".into(),
        json!({ "type": "report", "date": "2024-01" }),
    );

    client.executor.call("module_fetch",   json!({}), Some(&context), None).await?;
    // module_fetch writes context.data["raw_records"] = [...]

    client.executor.call("module_analyze", json!({}), Some(&context), None).await?;
    // module_analyze reads context.data["raw_records"]
    // and writes context.data["analysis"]

    client.executor.call("module_report",  json!({}), Some(&context), None).await?;
    // module_report reads both prior outputs from context.data
    # Ok(()) }
    ```

### Logging via `context.logger`

=== "Python"

    ```python
    from apcore import Context, Module

    class SendEmailModule(Module):
        def execute(self, inputs: dict, context: Context) -> dict:
            context.logger.info(f"Sending email to {inputs['to']}")
            # Output: [abc-123] [executor.email.send_email] Sending email to user@example.com
            return {"success": True}
    ```

=== "TypeScript"

    ```typescript
    import type { Context } from 'apcore-js';

    interface SendEmailInput { to: string; subject: string; body: string }

    export class SendEmailModule {
        async execute(inputs: SendEmailInput, context: Context) {
            context.logger.info(`Sending email to ${inputs.to}`);
            // Output: [abc-123] [executor.email.send_email] Sending email to user@example.com
            return { success: true };
        }
    }
    ```

=== "Rust"

    ```rust
    use apcore::{Context, Module, ModuleError};
    use serde_json::{json, Value};

    #[derive(Debug)]
    pub struct SendEmailModule;

    #[async_trait::async_trait]
    impl Module for SendEmailModule {
        fn id(&self) -> &str { "executor.email.send_email" }
        fn description(&self) -> &str { "Send a transactional email" }

        async fn execute(
            &self,
            inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            let to = inputs["to"].as_str().unwrap_or_default();
            ctx.logger().info(&format!("Sending email to {to}"));
            // Output: [abc-123] [executor.email.send_email] Sending email to user@example.com
            Ok(json!({ "success": true }))
        }
    }
    ```

### Middleware redaction

=== "Python"

    ```python
    import logging
    from apcore import Context
    from apcore.middleware import Middleware

    log = logging.getLogger(__name__)

    class SafeLoggingMiddleware(Middleware):
        def before(self, module_id: str, inputs: dict, context: Context) -> dict | None:
            # Safe: use redacted data instead of raw inputs
            log.info("Calling %s", module_id, extra={"inputs": context.redacted_inputs})
            return None  # pass inputs through unchanged
    ```

=== "TypeScript"

    ```typescript
    import { Middleware, type Context } from 'apcore-js';

    export class SafeLoggingMiddleware extends Middleware {
        before(
            moduleId: string,
            inputs: Record<string, unknown>,
            context: Context,
        ): Record<string, unknown> | null {
            // Safe: use redacted data instead of raw inputs
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
            // Safe: use redacted data instead of raw inputs
            tracing::info!(
                module_id = module_id,
                inputs = ?ctx.redacted_inputs,
                "Calling module",
            );
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
    ```

### Tracing middleware writing to `data`

=== "Python"

    ```python
    import time
    import uuid
    from apcore import Context
    from apcore.middleware import Middleware

    class TracingMiddleware(Middleware):
        def before(self, module_id: str, inputs: dict, context: Context) -> None:
            context.data["_apcore.mw.tracing.span_id"] = str(uuid.uuid4())[:16]
            context.data["_apcore.mw.tracing.span_start"] = time.time()
            return None

        def after(
            self,
            module_id: str,
            inputs: dict,
            output: dict,
            context: Context,
        ) -> None:
            start = context.data.get("_apcore.mw.tracing.span_start", 0)
            context.data["_apcore.mw.tracing.span_duration_ms"] = round(
                (time.time() - start) * 1000
            )
            return None
    ```

=== "TypeScript"

    ```typescript
    import { Middleware, type Context } from 'apcore-js';
    import { randomUUID } from 'node:crypto';

    export class TracingMiddleware extends Middleware {
        before(
            _moduleId: string,
            _inputs: Record<string, unknown>,
            context: Context,
        ): null {
            context.data['_apcore.mw.tracing.span_id'] = randomUUID().slice(0, 16);
            context.data['_apcore.mw.tracing.span_start'] = Date.now();
            return null;
        }

        after(
            _moduleId: string,
            _inputs: Record<string, unknown>,
            _output: Record<string, unknown>,
            context: Context,
        ): null {
            const start = (context.data['_apcore.mw.tracing.span_start'] as number) ?? 0;
            context.data['_apcore.mw.tracing.span_duration_ms'] = Date.now() - start;
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

    fn now_ms() -> u128 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
    }

    #[derive(Debug)]
    pub struct TracingMiddleware;

    #[async_trait]
    impl Middleware for TracingMiddleware {
        fn name(&self) -> &str { "tracing" }

        async fn before(
            &self,
            _module_id: &str,
            _inputs: Value,
            ctx: &Context<Value>,
        ) -> Result<Option<Value>, ModuleError> {
            let span_id: String = uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .chars()
                .take(16)
                .collect();
            let mut data = ctx.data.write();
            data.insert("_apcore.mw.tracing.span_id".into(), json!(span_id));
            data.insert("_apcore.mw.tracing.span_start".into(), json!(now_ms() as u64));
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
            let start = data
                .get("_apcore.mw.tracing.span_start")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u128;
            data.insert(
                "_apcore.mw.tracing.span_duration_ms".into(),
                json!((now_ms().saturating_sub(start)) as u64),
            );
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
    ```

## Common `data` Uses

| Purpose | Example keys | Notes |
|---------|--------------|-------|
| Pipeline intermediate state | `raw_records`, `analysis` | AI orchestration multi-step calls |
| Observability | `_apcore.mw.tracing.span_id`, `_apcore.mw.tracing.parent_span_id` | Written by middleware |
| Internationalization | `locale`, `timezone` | Set at top level, read as needed |
| Feature flags | `feature_flags` | Set at top level |
| Request metadata | `source`, `client_ip`, `session_id` | Written at the entry layer |

## Dependencies

- [Identity System](./identity-system.md) — `Identity` type, ACL integration, `ContextFactory` protocol.
- [Core Executor](./core-executor.md) — `Context.create`, child-context propagation, call-chain enforcement, sensitive-field redaction.
- [Observability](./observability.md) — `ContextLogger` and tracing integration.
- [Cancellation](./cancellation.md) — `CancelToken` semantics.

## Testing Strategy

- Round-trip tests for `serialize()` / `deserialize()` covering all serialized fields and `_context_version` validation.
- Call-chain bound tests for `CALL_DEPTH_EXCEEDED`, `CIRCULAR_CALL`, `CALL_FREQUENCY_EXCEEDED`.
- Logger tests asserting auto-injection of `trace_id` / `module_id` / `caller_id`.
- Redaction tests covering both `redacted_inputs` and `_secret_` prefix stripping.
- `data` reference-sharing tests across multi-step calls.
- Concurrency tests for safe immutable fields and unsafe `data` writes.

## Next Steps

- [Module Interface](./module-interface.md) — how `execute(inputs, context)` consumes Context.
- [Core Executor](./core-executor.md) — Context creation and propagation pipeline.
- [Identity System](./identity-system.md) — caller identity and ACL.
- [Middleware System](./middleware-system.md) — accessing Context inside middleware.
- [Observability](./observability.md) — tracing, metrics, and the context logger.
