# Context Object

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../../PROTOCOL_SPEC.md) §5.7 Context Object.

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

See [PROTOCOL_SPEC §12.7.2 Context.data Sharing Semantics](../../PROTOCOL_SPEC.md#1272-contextdata-sharing-semantics) for the full concurrency model.

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
    impl Module for DeleteUserModule {
        fn execute(&self, input: DeleteUserInput, ctx: &Context) -> ModuleResult<DeleteUserOutput> {
            let identity = ctx.identity().ok_or(ModuleError::Unauthenticated)?;
            if !identity.roles().iter().any(|r| r == "admin") {
                return Err(ModuleError::Forbidden);
            }
            self.delete_user(&input.user_id)?;
            Ok(DeleteUserOutput { success: true, operated_by: Some(identity.id().into()) })
        }
    }
    ```

### Calling another module

```python
class UserRegisterModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        user_id = self._create_user(inputs)

        # context is propagated automatically; framework updates caller_id and call_chain
        result = context.executor.call(
            module_id="executor.email.send_email",
            inputs={"to": inputs["email"], "subject": "Welcome", "body": "..."},
            context=context,
        )
        return {"user_id": user_id, "email_sent": result["success"]}
```

### AI orchestration via `data`

```python
context = Context.create(executor=executor, identity=user_identity)
context.data["task_info"] = {"type": "report", "date": "2024-01"}

executor.call("module_fetch",   inputs={...}, context=context)
# module_fetch writes context.data["raw_records"] = [...]

executor.call("module_analyze", inputs={...}, context=context)
# module_analyze reads context.data["raw_records"] and writes context.data["analysis"]

executor.call("module_report",  inputs={...}, context=context)
# module_report reads both prior outputs from context.data
```

### Logging via `context.logger`

```python
class SendEmailModule(Module):
    def execute(self, inputs: dict, context: Context) -> dict:
        context.logger.info(f"Sending email to {inputs['to']}")
        # Output: [abc-123] [executor.email.send_email] Sending email to user@example.com
        ...
```

### Middleware redaction

```python
class LoggingMiddleware:
    def before(self, module_id: str, inputs: dict, context: Context) -> dict:
        # ✅ Safe: use redacted data
        log.info(f"Calling {module_id}", extra={"inputs": context.redacted_inputs})
        return inputs
```

### Tracing middleware writing to `data`

```python
class TracingMiddleware:
    def before(self, module_id: str, inputs: dict, context: Context) -> None:
        context.data["_apcore.mw.tracing.span_id"] = str(uuid.uuid4())[:16]
        context.data["_apcore.mw.tracing.span_start"] = time.time()

    def after(self, module_id: str, inputs: dict, output: dict, context: Context) -> None:
        start = context.data.get("_apcore.mw.tracing.span_start", 0)
        context.data["_apcore.mw.tracing.span_duration_ms"] = round((time.time() - start) * 1000)
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
