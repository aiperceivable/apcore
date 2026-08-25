---
description: "Core Executor running the 11-step call pipeline: context creation, call-chain guard, registry lookup, ACL, approval, before/after middleware, schema validation+redaction, timeout."
---

# Core Execution Engine

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §12 SDK Implementation Guide.


## Overview

The Core Execution Engine is the central orchestration component of apcore. It processes module calls through a secured execution lifecycle, handling everything from context creation and safety checks to module execution with timeout enforcement and result validation. The engine supports both synchronous and asynchronous execution paths, bridging between the two via threading and an async event loop bridge.

## Requirements

- Orchestrate module calls through a well-defined, sequential pipeline with clear separation of concerns at each step.
- Enforce safety constraints including maximum call depth limits, circular call detection, and frequency throttling to prevent runaway or abusive execution.
- Look up modules from the Registry and enforce access control lists (ACL) before execution.
- Validate inputs and outputs using runtime model classes, with automatic redaction of fields marked as `x-sensitive`.
- Support middleware chains that execute before and after the core module invocation, enabling cross-cutting concerns such as logging, metrics, and transformation.
- Execute modules with configurable timeout enforcement, using daemon threads for synchronous modules and an async bridge for asynchronous modules.
- Return structured results that include execution metadata and any errors encountered during the pipeline.

## Technical Design

### Execution Pipeline

The executor processes every module call through the following pipeline:

1. **Context Creation** -- A `Context` object is constructed carrying the caller identity, call metadata, and any propagated state from parent calls. This context flows through every subsequent step.

2. **Call Chain Guard** -- Three safety mechanisms are evaluated before proceeding:
   - *Call depth check*: Rejects calls that exceed the configured maximum nesting depth, preventing unbounded recursion.
   - *Circular call detection*: Inspects the call chain recorded in the context to detect and reject circular module invocations.
   - *Frequency throttling*: Tracks call frequency per module and rejects calls that exceed the configured rate, protecting against tight-loop abuse.

3. **Module Lookup from Registry** -- The target module is resolved by name from the Registry. If the module is not found or not loaded, the pipeline terminates with a descriptive error.

4. **ACL Enforcement** -- The caller's `Identity` (extracted from the context) is checked against the module's access control list. Unauthorized calls are rejected before any execution occurs.

5. **Approval Gate** -- If an `ApprovalHandler` is configured and the module declares `requires_approval=true`, the handler is invoked to obtain approval before proceeding. The handler may block for human input or return immediately. Rejected, timed-out, or still-pending approvals raise `ApprovalDeniedError`, `ApprovalTimeoutError`, or `ApprovalPendingError` respectively. Skipped entirely when no handler is configured or the module does not require approval. See [Approval System](./approval-system.md).

6. **Middleware Before Chain** -- All registered "before" middleware functions are executed in order. Each middleware receives the context and input, and may modify or enrich them before validation runs.

7. **Input Validation + Sensitive Field Redaction** -- The call's input payload (including any modifications from middleware) is validated against the module's input schema (a dynamically generated runtime model). Fields annotated with `x-sensitive` are redacted from logs and error messages using the `redact_sensitive` utility.

8. **Module Execution with Timeout (Dual-Timeout Model)** -- The module's handler is invoked with dual-timeout enforcement: both a per-module timeout (`resources.timeout`, default 30s) and a global deadline (`executor.global_timeout`, default 60s). The shorter of the two is applied, preventing nested call chains from exceeding the global budget. The global deadline is set on the root call and propagated to child contexts via `Context._global_deadline`.

   **Cooperative cancellation:** On timeout, the executor sends `CancelToken.cancel()` and waits a 5-second grace period before raising `ModuleTimeoutError`. Modules that check `cancel_token` in their execution loop can clean up gracefully.

9. **Output Validation** -- The module's return value is validated against its output schema. Invalid output triggers an error rather than allowing malformed data to propagate.

10. **Middleware After Chain** -- All registered "after" middleware functions are executed in order with access to the context, input, and output. These may perform logging, transformation, or cleanup.

11. **Result Return** -- The final validated output (or error) is packaged into a structured result and returned to the caller.

!!! tip "Core vs Optional Steps"
    Of the 11 steps, only **4 are mandatory** (non-removable):

    - **Steps 1, 3, 8, 11** (`context_creation` → `module_lookup` → `execute` → `return_result`)

    The remaining 7 steps (guard, ACL, approval, middleware, validation) are **optional** and can be removed via strategy presets or custom strategies. The `minimal` strategy retains only the 4 core steps for maximum throughput on pre-validated internal hot paths. See [Execution Pipeline Design](../spec/design-execution-pipeline.md) for the full strategy reference.

!!! info "Step Metadata"
    Each pipeline step declares four metadata fields:

    | Field | Type | Default | Purpose |
    |-------|------|---------|---------|
    | `match_modules` | glob patterns or null | `null` (all) | Only run this step for matching module IDs |
    | `ignore_errors` | bool | `false` | If true, step failure logs warning and continues |
    | `pure` | bool | `false` | If true, safe to run during `validate()` dry-run mode |
    | `timeout_ms` | int | `0` | Per-step timeout in milliseconds (0 = no limit) |

    These fields enable targeted step application, fault-tolerant pipelines, and dry-run validation without code changes.

### Key Classes

- **Executor** -- The main engine class that implements the execution pipeline. Manages middleware registration, timeout configuration, and the execution loop.
- **Context** -- Immutable data class carrying call metadata: caller identity, call chain history, depth counter, and propagated key-value state.
- **Identity** -- Represents the caller's identity for ACL enforcement. Carries `id`, `type`, `roles`, and extensible `attrs` dict.
- **Config** -- Configuration data class holding executor-level settings such as max call depth, timeout defaults, and throttle limits.

### Sync/Async Bridge

The executor exposes both `call()` (sync) and `call_async()` (async) entry points. Internally:
- Synchronous modules called from an async context are dispatched to a worker thread to avoid blocking the event loop.
- Asynchronous modules called from a synchronous context are executed through a temporary event loop on a worker thread.
- A cache lock protects concurrent access to shared module state.

These mechanics are SDK-specific; languages without an async/await split (e.g., Rust with a single runtime) MAY implement the bridge differently.

### Sensitive Field Redaction

The `redact_sensitive` utility walks the input/output dictionaries and replaces values of fields marked `x-sensitive: true` in the schema with a placeholder string. This ensures sensitive data never appears in logs or error reports.

### Error Propagation (Algorithm A11)

All execution paths (sync, async, stream) wrap exceptions via `propagate_error()`, ensuring middleware always receives `ModuleError` instances with trace context attached. This guarantees consistent error handling regardless of the execution mode.

### Deep Merge for Streaming

Streaming chunk accumulation uses recursive deep merge (depth-capped at 32) instead of shallow merge. This correctly handles nested response structures where chunks contribute to different levels of the output tree.

### Validation (Preflight)

The `validate()` method provides a non-destructive preflight check: **6 pipeline checks plus an optional module-level preflight** (no execution, no middleware). It runs Steps 1–5 and Step 7 of the canonical 11-step pipeline (module ID format, module lookup, call chain safety, ACL, approval detection, input schema validation), explicitly skipping Step 6 Middleware Before Chain, and then optionally invokes `module.preflight()` for advisory warnings. It returns a `PreflightResult` with per-check results and a `requires_approval` flag. The result is duck-type compatible with the legacy `ValidationResult` — `.valid` and `.errors` properties work identically. See [PROTOCOL_SPEC §12.8](../spec/protocol-spec.md#128-executorvalidate-cross-language-implementation-guide) for the cross-language implementation guide.

### Execution State Machine

The Executor processes each `call()` through a fixed state machine. Failures at any stage transition into either an immediate error or the `on_error` middleware chain.

```text
  ┌─────────┐
  │  idle   │
  └────┬────┘
       │ call()
       ▼
  ┌──────────┐  depth/cycle/freq  ┌──────────────────────────┐
  │call_chain│───────────────────▶│ error: DEPTH_EXCEEDED    │
  │  guard   │                    │      / CIRCULAR_CALL     │
  └────┬─────┘                    │      / FREQUENCY_EXCEEDED│
       │ check passed             └──────────────────────────┘
       ▼
  ┌─────────┐    module not exist ┌──────────────────┐
  │ resolve │────────────────────▶│ error: NOT_FOUND │
  └────┬────┘                     └──────────────────┘
       │ module found
       ▼
  ┌─────────┐    permission denied ┌──────────────────┐
  │  acl    │─────────────────────▶│ error: ACL_DENIED│
  └────┬────┘                      └──────────────────┘
       │ permission passed
       ▼
  ┌──────────┐  rejected/timeout ┌──────────────────────────┐
  │ approval │──────────────────▶│ error: APPROVAL_DENIED   │
  │   gate   │                   │      / APPROVAL_TIMEOUT  │
  └────┬─────┘                   │      / APPROVAL_PENDING  │
       │                         └──────────────────────────┘
       │ approved (or skipped)
       ▼
  ┌──────────┐
  │ before   │──── middleware error ──▶ on_error chain
  │middleware│
  └────┬─────┘
       │ transforms applied
       ▼
  ┌──────────┐   validation failed ┌──────────────────────┐
  │ validate │────────────────────▶│ error: VALIDATION    │
  │  input   │                     └──────────────────────┘
  └────┬─────┘
       │ validation passed
       ▼
  ┌──────────┐   execution error  ┌──────────────────────┐
  │ execute  │───────────────────▶│ on_error middleware  │
  │  module  │                    └──────────────────────┘
  └────┬─────┘
       │ success
       ▼
  ┌──────────┐   validation failed ┌──────────────────────┐
  │ validate │────────────────────▶│ error: VALIDATION    │
  │  output  │                     └──────────────────────┘
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │  after   │
  │middleware│
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │ return   │
  │ result   │
  └──────────┘
```

### Timeout Specification (Dual-Timeout Model)

| Setting | Default | Description |
|---------|---------|-------------|
| Module execution timeout | 30 000 ms | Override via `resources.timeout` |
| Global timeout | 60 000 ms | Total budget including middleware and validation |
| ACL check timeout | 1 000 ms | Maximum time for ACL rule evaluation |

The Executor enforces a **dual-timeout model**: both the per-module timeout and a global deadline are tracked, and the shorter of the two applies. The global deadline is set on the root call and propagated to child contexts via `Context._global_deadline`, preventing nested call chains from exceeding the global budget.

- After timeout the Executor MUST raise `MODULE_TIMEOUT`.
- Timeout counting MUST start from the first `before()` middleware.
- Middleware execution time SHOULD count toward the total.

**Cooperative cancellation.** On timeout the Executor invokes `CancelToken.cancel()` and waits a 5-second grace window before raising `ModuleTimeoutError`. Modules that poll `cancel_token` can clean up gracefully:

```python
@client.module(id="long.task", description="Long-running task")
async def long_task(inputs: dict, context: Context) -> dict:
    for item in items:
        if context.cancel_token.is_cancelled:
            return {"partial": True, "processed": count}
        await process(item)
    return {"partial": False, "processed": len(items)}
```

### Concurrent Execution Semantics

- A single Executor instance MUST tolerate concurrent calls from multiple threads/coroutines.
- Each `call()` MUST receive its own Context (independent `call_chain` and `caller_id`).
- `context.data` is shared by reference; concurrent calls SHOULD use distinct Context instances when isolation matters.
- Batch `call_async()` MAY execute concurrently; ordering is not guaranteed.

See [PROTOCOL_SPEC §12.7 Concurrency Model Specification](../spec/protocol-spec.md#127-concurrency-model-specification).

### Edge Cases

| Scenario | Behavior | Level |
|----------|----------|-------|
| `timeout = 0` | Disable timeout, log WARN | MUST |
| `timeout` is negative | Raise `GENERAL_INVALID_INPUT` | MUST |
| `module_id` is empty string `""` | Raise `MODULE_NOT_FOUND` | MUST |
| `inputs = null` | Treat as empty dict `{}`, continue validation | MUST |
| `context = null` | Create a new Context (empty `call_chain`) | MUST |
| Concurrent calls sharing one Context instance | Race condition; SHOULD log WARN | SHOULD |
| `call()` during module `unregister()` | If execution started, continue; otherwise raise `MODULE_NOT_FOUND` | MUST |
| `call_chain` length reaches `max_call_depth` | Raise `CALL_DEPTH_EXCEEDED` | MUST |

### Pipeline Strategy API

The execution pipeline is driven by an `ExecutionStrategy` — a named, ordered sequence of steps. Strategies can be swapped at construction time or registered globally for selection by name.

| Surface | Type | Description |
|---------|------|-------------|
| `Executor.register_strategy(name, strategy)` | class method | Register a named strategy resolvable at construction time |
| `executor.list_strategies() -> list[StrategyInfo]` | instance method | Returns `StrategyInfo` for the current strategy and all registered strategies |

```python
Executor.register_strategy("audit", AuditStrategy())

executor = Executor(registry, strategy="audit")
for info in executor.list_strategies():
    print(info.name, info.step_count)
```

Built-in strategies and authoring custom ones are described in [Pipeline Hardening](#pipeline-hardening-issue-33) below.

## Contract: Executor.call

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `module_id`: string, required. Validated at method entry via `validate_module_id(allow_reserved=true)`. `system` is a reserved first segment ([PROTOCOL_SPEC §2.5](../spec/protocol-spec.md#25-reserved-words)), so the bypass is what makes `system.*` invocation legal; every other validation (empty, pattern, length) still applies. Empty / over-length / malformed IDs MUST be rejected before the pipeline context is constructed.
- `inputs`: object, required. Payload conforming to the module's input schema.
- `options`: object, optional. Call-site overrides (identity, trace_parent, per-call timeout).

### Preconditions

- Entry-guard: `module_id` MUST be validated before constructing a pipeline context. Implementations MUST NOT defer this check to downstream pipeline steps.

### Side Effects (ordered)

1. Validate `module_id` at method entry; reject fast with `InvalidInputError(code=INVALID_MODULE_ID)`.
2. Construct `PipelineContext` and run the 11-step execution pipeline (see Execution Pipeline above).
3. Emit observability spans and metrics per observability configuration.

### Errors

- `InvalidInputError(code=INVALID_MODULE_ID)` -- `module_id` fails entry-guard validation.
- `ModuleNotFoundError(code=MODULE_NOT_FOUND)` -- `module_id` not present in the registry.
- `ACLDeniedError`, `ApprovalDeniedError`, `ApprovalTimeoutError`, `ApprovalPendingError`, `ModuleTimeoutError`, `ExecutionCancelledError`, `ModuleError` -- propagated from pipeline stages.

### Returns

- On success: validated output object conforming to the module's output schema.
- On failure: raises (Python/TypeScript) / returns `Err` (Rust).

### Properties

- `async`: SDK-specific. `call()` is synchronous in Python (wraps `call_async`); asynchronous in TypeScript and Rust. Both surfaces MUST be provided where the host language supports both.
- `thread_safe`: `true`.
- `pure`: `false` -- pipeline stages may emit events, mutate observability state, and transitively invoke other modules.

### Trace Variants (`call_with_trace` / `callWithTrace`)

SDKs MAY expose a trace-returning variant — `call_with_trace` in Python and Rust, `callWithTrace` in TypeScript — that returns the call result paired with a `PipelineTrace` value describing per-step timings and middleware events. This variant is OPTIONAL; SDKs MAY omit it.

When implemented, the trace variant MUST share **identical error-recovery semantics** with the underlying `call()`:

- **MUST** run the same 11-step pipeline, including the `on_error` middleware chain. A middleware that recovers from a `PipelineStepError` in `call()` MUST also recover in the trace variant; conversely, an error that propagates in `call()` MUST also propagate in the trace variant.
- **MUST** apply the `ExecutionCancelledError` short-circuit (cancellation bypasses `on_error` in both surfaces — see [Cancellation Short-Circuit](#cancellation-short-circuit) below).
- **MUST** apply the `MiddlewareChainError` unwrap rule (the original typed cause MUST be surfaced; the wrapper MUST NOT replace the cause with a generic `ModuleExecuteError` — see [Error Unwrap Rule](#error-unwrap-rule) below).
- **MUST** populate the returned `PipelineTrace` with every middleware event observed during execution, including any `on_error` recovery — the trace is the observable record of what happened, not a sanitized projection.

The trace variant differs from `call()` only in its return shape: a tuple/object pairing the result (or error) with the trace. All side-effect semantics — events emitted, metrics recorded, ACL audits written — MUST be identical. (Decision **D-19**.)

### Cancellation Short-Circuit

When an execution is cancelled (via `CancelToken` triggering `ExecutionCancelledError`), the executor MUST short-circuit before invoking the `on_error` middleware chain. Rationale: cancellation is a caller-driven request to stop, not a recoverable failure; allowing `on_error` middleware to observe cancellation as an error opens the door to logging middleware swallowing it or to retry middleware reissuing a `RetrySignal` that restarts the loop. SDKs MUST detect `ExecutionCancelledError` after pipeline-error unwrap and propagate it directly, bypassing `on_error`. (Decision **D-20**.)

### Cancel Token Mid-Pipeline Check

The pipeline MUST observe `cancel_token` cancellation at **two** points, in addition to honoring it inside `module.execute()` itself:

1. **Step 2 (Call-Chain Guard)** — before any expensive validation or middleware work. If the token is already cancelled here, the pipeline short-circuits with `ExecutionCancelledError`.
2. **Step 8 (Execute)** — immediately before invoking the module. Acts as a defensive backstop for tokens that became cancelled while earlier steps were running.

SDKs MUST implement both check points. Single-check implementations leak compute (the pipeline runs through ACL/middleware/validation even though the caller has already cancelled) and are non-conforming. (Decision **D-21**.)

### Error Unwrap Rule

When a middleware (`before` / `after` / `on_error`) raises a domain-typed error such as `ApprovalDeniedError`, the underlying chain machinery may wrap it in a `MiddlewareChainError` for diagnostic purposes. The executor MUST unwrap this wrapper before propagating to the caller, surfacing the **original typed cause** unchanged. SDKs MUST NOT replace the cause with a generic `ModuleExecuteError`; doing so collapses callers' ability to dispatch on the typed error (e.g., MCP/A2A bridges keying off `APPROVAL_DENIED` vs `MODULE_EXECUTE_ERROR`). (Decision **D-22**.)

## Contract: Context.create

Normative behavioral contract for the canonical factory entry point used by callers producing a new top-level call context. See [Issue #66](https://github.com/aiperceivable/apcore/issues/66) for the unified-signature decision rationale.

### Inputs

The canonical input list — across all SDKs, the public factory MUST accept **exactly** these six caller-supplied fields, named as listed (snake_case in Python/Rust, camelCase in TypeScript). Order is significant for positional languages and MUST be followed.

| # | Name | Type | Default | Notes |
|---|------|------|---------|-------|
| 1 | `identity` | Identity \| null | null | When null, the constructor synthesizes an `@external` identity. |
| 2 | `trace_parent` | TraceParent \| null | null | W3C Trace Context entry. The TraceParent type itself carries `tracestate` (vendor state) — SDKs MUST embed `tracestate` in the `TraceParent` type, not expose it as a separate factory parameter. Invalid values (non-32-hex, all-zero, all-f) MUST log WARN and be replaced with a fresh `trace_id`. |
| 3 | `cancel_token` | CancelToken \| null | null | External cooperative-cancellation source. When omitted, the Executor synthesizes a fresh token at pipeline entry. Adopting this parameter eliminates the post-hoc `ctx.cancel_token = token` anti-pattern that proliferated across the ecosystem. |
| 4 | `data` | Mapping<string, Any> \| null | empty | User-propagated state carried through the call chain by reference. |
| 5 | `services` | T \| null | null | Caller-supplied DI container. MUST NOT be used to smuggle framework-owned fields (e.g., a `cancel_token` sub-key); the `cancel_token` parameter above is now first-class. |
| 6 | `global_deadline` | absolute timestamp \| null | null | Bounds total execution time for the call tree rooted at this context. Local-only; see §Contract: `global_deadline` distributed semantics. |

The following Context fields are **NOT** caller inputs to `Context.create()`:

- `trace_id` — generated internally (derived from `trace_parent` when present and valid; otherwise a fresh 32-char lowercase hex value).
- `caller_id` — top-level Contexts always have `caller_id = null`. Managed exclusively by `Context.child()`. Reserved name; future revisions MAY surface it if a use case emerges. Current SDKs MUST NOT accept it as a `Context.create()` parameter.
- `call_chain` — empty `[]` at top-level; managed exclusively by the Executor.
- `executor` — bound by the Executor at pipeline entry. See §Contract: Executor binding to Context.
- `logger` — derived property; computed from `trace_id` and `caller_id`.
- `redacted_inputs`, `redacted_output` — set by Executor pipeline steps 5 and 9 respectively. Never a caller input.

### Preconditions

- `trace_parent` (if present) is validated; invalid values trigger regeneration with a WARN log, not rejection.

### Errors

None under normal operation. Invalid `trace_parent` values log a WARN and a fresh `trace_id` is generated instead of raising.

### Returns

A fresh `Context` instance with:

- A 32-character lowercase hex `trace_id` (derived from a valid `trace_parent`, or newly generated otherwise).
- `executor`, `call_chain`, `caller_id` all unset (`null` / `None` / empty list).
- All caller-supplied fields populated as provided.
- `redacted_inputs` / `redacted_output` unset (populated later by the Executor pipeline).

### Properties

- `async`: `false`.
- `thread_safe`: `true` — constructor only; no shared state is mutated.
- `pure`: `false` — a new `trace_id` is generated for each call.
- `idempotent`: `false` — each call yields a new Context with a unique `trace_id`.

## Contract: Executor binding to Context

A Context whose `executor` field is null MAY originate from three distinct sources:

1. **Local construction** via `Context.create()` (executor is intentionally not an input).
2. **Cross-process deserialization** via `Context.deserialize()` (the `executor` field MUST NOT serialize, per PROTOCOL_SPEC §5.7).
3. **Hot-reload survivor**, restored from persistence (async task store, task queue, etc.) after a process restart.

The Executor MUST treat all three sources identically. The following normative rules apply at the Executor level (not at Context.create itself):

1. **Bind** — When the Executor receives a Context whose `executor` field is null, it MUST bind itself to `context.executor` **before** pipeline step 1.
2. **Stability** — Once bound, `context.executor` MUST NOT change for the remainder of the call chain.
3. **Same-executor idempotency** — If `context.executor` is non-null and refers to **the same** Executor instance (identity comparison), the rebind is a noop. The Executor MUST NOT raise. This case covers the common pattern of reusing a single Context across multiple top-level `Executor.call()` invocations.
4. **Cross-executor conflict** — If `context.executor` is non-null and refers to a **different** Executor instance, the Executor **MUST** raise `CONTEXT_BINDING_ERROR` ([PROTOCOL_SPEC](../spec/protocol-spec.md#122-core-component-interface-contracts) §12.2, normative as of v1.11.0). Through v1.10.0 this was a SHOULD with a documented-deviation escape hatch; no SDK took it, and the alternative made the behaviour unassertable — a conformance case cannot state two legal outcomes without each driver deciding which applies to it, so all three hardcoded the raise branch and the fixture went unread (apcore#92).
5. **Propagation** — `Context.child()` MUST propagate the bound `executor` reference to the child Context unchanged.

This section unifies the previously separate "re-inject after deserialize" requirement — see [Context Object §Serialization](./context-object.md#serialization) for the cross-reference — with the construction-time binding model. Implementation mechanism is language-idiomatic: mutable field assignment for Python/Rust dataclasses; copy-on-write returning a new instance for TypeScript's `readonly` fields.

The binding method is a **cross-boundary contract member** — the Executor calls it, and a bridge's duck-typed `Context` MUST implement it — so it MUST be public-named in every SDK (no leading-underscore / `private` / non-`pub`). See [API Surface & Naming Conventions](../spec/api-surface-conventions.md) for the visibility-vs-discoverability rules and the worked example.

## Contract: Distributed cancellation

`cancel_token` is runtime-only and MUST NOT serialize (per PROTOCOL_SPEC §5.7). On the receiving node of a deserialized Context:

- The Executor MUST synthesize a fresh local `CancelToken` at pipeline entry. The remote node never observes the originating node's `CancelToken` object.
- Distributed cancellation MUST go through **out-of-band channels** (e.g., `AsyncTaskStore` task_id lookup, a `RemoteCancelSignal` subscription, or a control plane RPC). It MUST NOT attempt to ride the in-context `cancel_token` field across process boundaries.

The `cancel_token` parameter on `Context.create()` exists solely for **in-process cooperation** — a request handler binding the HTTP/RPC request's abort signal to the call tree it spawns locally.

## Contract: `global_deadline` distributed semantics

`global_deadline` is runtime-only and MUST NOT serialize. When a deserialized Context arrives at a remote node:

- The receiving Executor MUST recompute its own `global_deadline` from local `executor.global_timeout` config (per the dual-timeout model documented earlier in this spec).
- The originating node's deadline intent is intentionally not propagated through the `global_deadline` field.

Callers that need a wall-clock deadline to traverse process boundaries SHOULD store the absolute timestamp in `context.data` (a serializable field) under an extension key — e.g., `context.data["x-deadline"]`. The receiving SDK or middleware can read that key and translate it back into a local deadline if desired. `global_deadline` itself remains local-only by design, mirroring the same separation as `cancel_token` and `services`.

## Usage

=== "Python"
    ```python
    import apcore
    from apcore import APCore, Config, Identity

    # Build a client with default config
    client = APCore(Config())

    # Register a module
    @client.module(
        id="math.add",
        description="Add two numbers",
    )
    def add(inputs, ctx):
        return {"sum": inputs["a"] + inputs["b"]}

    # Synchronous call
    result = client.call("math.add", {"a": 1, "b": 2})
    print(result)  # {"sum": 3}

    # Async call
    import asyncio

    async def main():
        result = await client.call_async("math.add", {"a": 10, "b": 20})
        print(result)  # {"sum": 30}

    asyncio.run(main())
    ```

=== "TypeScript"
    ```typescript
    import { APCore } from 'apcore-js';

    const client = new APCore();

    // Register a module
    client.module({
        id: 'math.add',
        description: 'Add two numbers',
        inputSchema: { type: 'object', properties: { a: { type: 'number' }, b: { type: 'number' } }, required: ['a', 'b'] },
        outputSchema: { type: 'object', properties: { sum: { type: 'number' } } },
        execute: ({ a, b }: { a: number; b: number }) => ({ sum: a + b }),
    });

    // Call the module
    const result = await client.call('math.add', { a: 1, b: 2 });
    console.log(result); // { sum: 3 }
    ```

=== "Rust"
    ```rust
    use apcore::APCore;
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct AddModule;

    #[async_trait]
    impl Module for AddModule {
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "a": { "type": "number" }, "b": { "type": "number" } }, "required": ["a", "b"] })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object", "properties": { "sum": { "type": "number" } } })
        }
        fn description(&self) -> &'static str { "Add two numbers" }
        async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let a = input["a"].as_f64().unwrap_or(0.0);
            let b = input["b"].as_f64().unwrap_or(0.0);
            Ok(json!({ "sum": a + b }))
        }
    }

    #[tokio::main]
    async fn main() {
        let client = APCore::default();
        client.register("math.add", Box::new(AddModule));

        let result = client.call("math.add", json!({"a": 1.0, "b": 2.0})).await.unwrap();
        println!("{result}"); // {"sum":3.0}
    }
    ```

## Dependencies

- **Registry** -- Module lookup (step 3) depends on the Registry system to resolve module names to loaded module instances.
- **Schema System** -- Input and output validation (steps 7 and 9) depend on the Schema System for runtime model generation from YAML schemas.

??? info "Python SDK reference"
    The following tables are **not protocol requirements** — they document the Python SDK's source layout and runtime dependencies for implementers/users of `apcore-python`.

    **Source files:**

    | File | Lines | Purpose |
    |------|-------|---------|
    | `executor.py` | 634 | Core execution engine implementing the execution pipeline |
    | `context.py` | 66 | Context and Identity data classes |
    | `config.py` | 29 | Executor configuration data class |
    | `errors.py` | 395 | Structured error types for every failure mode in the pipeline |

    **Runtime dependencies:**

    - `pydantic>=2.0` -- Used for input/output schema validation, dynamic model generation, and field metadata.

## Testing Strategy

- **Unit tests** cover each pipeline step in isolation, verifying that context creation, safety checks, ACL enforcement, validation, middleware chains, and result packaging all behave correctly for both success and failure cases.
- **Timeout tests** verify that both synchronous and asynchronous modules are correctly cancelled when exceeding configured timeouts, and that daemon threads do not leak.
- **Safety check tests** exercise call depth limits, circular detection with various call chain topologies, and frequency throttle edge cases.
- **Redaction tests** confirm that `x-sensitive` fields are properly masked in logs and error messages while remaining intact in the actual data passed to the module.
- **Integration tests** run full pipeline executions through the executor with real Registry and Schema instances to verify end-to-end behavior.
- Test naming follows the `test_<unit>_<behavior>` convention.

---

## Pipeline Hardening (Issue #33)

This section documents normative hardening requirements added on top of the base 11-step pipeline. These rules apply to all SDK implementations.

### 1.1 Fail-Fast Error Handling

When a pipeline step produces an error, implementations MUST stop pipeline execution and propagate the error **unless** the step is configured with `ignore_errors: true`. Implementations MUST NOT silently swallow errors and continue to the next step. The error MUST be wrapped in a `PipelineStepError` that includes the failing step name and the original error.

When `ignore_errors: true` is set on a step, a failure logs a WARN and execution continues to the next step. The step's output is treated as absent (null/None/nil) for downstream steps.

=== "Python"
    ```python
    # apcore.yaml — step with ignore_errors: true
    # pipeline:
    #   configure:
    #     input_validation:            # map keyed by step name
    #       ignore_errors: true

    import apcore
    from apcore import APCore, Config

    client = APCore(Config.load("apcore.yaml"))

    @client.module(id="demo.process", description="Process with lenient validation")
    def process(inputs, ctx):
        return {"result": inputs.get("value", "default")}

    # Even if input_validation raises, the pipeline continues to execute.
    result = client.call("demo.process", {"value": 42})
    print(result)  # {"result": 42}

    # Step WITHOUT ignore_errors — fail fast
    # apcore.yaml:
    # pipeline:
    #   configure:
    #     input_validation:            # map keyed by step name
    #       ignore_errors: false       # default

    # A validation failure here raises PipelineStepError immediately;
    # no subsequent steps run.
    try:
        client.call("demo.process", {"unexpected_key": True})
    except apcore.PipelineStepError as e:
        print(e.step_name)   # "input_validation"
        print(e.cause)       # original SchemaValidationError
    ```

=== "TypeScript"
    ```typescript
    // apcore.yaml — step with ignore_errors: true
    // pipeline:
    //   configure:
    //     input_validation:           // map keyed by step name
    //       ignore_errors: true

    import { APCore } from 'apcore-js';

    const client = new APCore({ configPath: 'apcore.yaml' });

    client.module({
        id: 'demo.process',
        description: 'Process with lenient validation',
        execute: ({ value }: { value?: number }) => ({ result: value ?? 'default' }),
    });

    // ignore_errors: true — pipeline continues even if input_validation fails.
    const result = await client.call('demo.process', { value: 42 });
    console.log(result); // { result: 42 }

    // Step WITHOUT ignore_errors — fail fast
    try {
        await client.call('demo.process', { unexpected_key: true });
    } catch (e) {
        if (e instanceof PipelineStepError) {
            console.log(e.stepName);  // "input_validation"
            console.log(e.cause);     // original SchemaValidationError
        }
    }
    ```

=== "Rust"
    ```rust
    // apcore.yaml — step with ignore_errors: true
    // pipeline:
    //   configure:
    //     input_validation:           // map keyed by step name
    //       ignore_errors: true

    use apcore::{APCore, Config};
    use apcore::errors::PipelineStepError;
    use serde_json::json;

    #[tokio::main]
    async fn main() {
        let client = APCore::with_config(Config::load("apcore.yaml").unwrap());

        // ignore_errors: true — pipeline continues even if input_validation fails.
        let result = client.call("demo.process", json!({"value": 42})).await.unwrap();
        println!("{result}"); // {"result":42}

        // Step WITHOUT ignore_errors — fail fast
        match client.call("demo.process", json!({"unexpected_key": true})).await {
            Err(e) if e.is::<PipelineStepError>() => {
                let pse = e.downcast_ref::<PipelineStepError>().unwrap();
                println!("{}", pse.step_name);  // "input_validation"
                println!("{:?}", pse.cause);    // original SchemaValidationError
            }
            _ => {}
        }
    }
    ```

### 1.2 Replace Semantic for Pipeline Configuration

When configuring a pipeline step that already exists (same step name), implementations MUST replace the existing step definition entirely. Implementations MUST NOT create a duplicate step or append a second step with the same name. The replacement MUST preserve the step's position in the execution order.

This applies to both built-in steps and custom steps. Calling `configure_step` (or the equivalent YAML `configure:` directive) twice with the same step name is idempotent with respect to count — there is always exactly one step with that name.

```yaml
# apcore.yaml — replace the built-in input_validation step with a custom handler
pipeline:
  configure:
    # `configure` is an object MAP keyed by step name, per
    # schemas/apcore-config.schema.json $defs/PipelineConfig — not an array.
    input_validation:
      handler: "myapp.pipeline.custom_validator:validate"
      ignore_errors: false
      timeout_ms: 500
```

After this configuration, the pipeline has exactly one `input_validation` step (the custom one). The built-in handler is fully replaced. The step remains at position 7 in the execution order (between the Middleware Before Chain and Module Execution steps).

### 1.3 Step-Level Middleware

Implementations SHOULD support step-level middleware — middleware that applies only to specific pipeline steps rather than the entire call. Step-level middleware MUST execute in the same before/after pattern as global middleware but scoped to the target step only. Global middleware MUST execute before step-level middleware in the before-phase, and after step-level middleware in the after-phase.

The execution order for a step with both global and step-level middleware is:

1. Global middleware — before phase (all registered global before-hooks)
2. Step-level middleware — before phase (scoped to this step)
3. Step handler executes
4. Step-level middleware — after phase (scoped to this step, reverse order)
5. Global middleware — after phase (all registered global after-hooks, reverse order)

=== "Python"
    ```python
    import time
    from apcore import APCore, Config, PipelineState, StepMiddleware, StepResult


    class TimingStepMiddleware(StepMiddleware):
        """Runs for EVERY step — filter on `step_name` to scope it.

        The hooks are `(step_name, state)` / `(step_name, state, result)`:
        there is no `inputs` parameter, because a Step is `execute(ctx)`.
        """

        def __init__(self) -> None:
            self._started: dict[str, float] = {}

        async def before_step(self, step_name: str, state: PipelineState) -> None:
            if step_name == "input_validation":
                self._started[step_name] = time.perf_counter()

        async def after_step(self, step_name: str, state: PipelineState, result: StepResult) -> None:
            start = self._started.pop(step_name, None)
            if start is not None:
                print(f"step={step_name} elapsed_ms={(time.perf_counter() - start) * 1000:.2f}")


    client = APCore(Config())
    client.executor.current_strategy.add_step_middleware(TimingStepMiddleware())

    @client.module(id="demo.greet", description="Greet the user")
    def greet(name: str) -> dict:
        return {"message": f"Hello, {name}!"}

    print(client.call("demo.greet", {"name": "World"}))
    ```

=== "TypeScript"
    ```typescript
    // apcore-typescript registers step middleware on `PipelineEngine`, which the
    // Executor holds privately — there is currently no public path to it from the
    // client. See features/middleware-system.md; Python and Rust expose it on the
    // ExecutionStrategy.
    ```

=== "Rust"
    ```rust
    use apcore::{build_standard_strategy, Config, Executor, Registry, StepMiddleware};
    use std::sync::Arc;

    // `add_step_middleware` needs `&mut ExecutionStrategy` and `Executor::strategy()`
    // yields only `&`, so populate the strategy before constructing the executor.
    let mut strategy = build_standard_strategy();
    strategy.add_step_middleware(Arc::new(TimingStepMiddleware::default()));

    let executor = Executor::with_strategy(Registry::new(), Config::from_defaults(), strategy);
    ```

Step middleware is **not** scoped to a single step: every registered `StepMiddleware`
receives a `before_step` / `after_step` / `on_step_error` callback for every step in the
pipeline and filters on `step_name` itself. There is no `next`-style continuation, and no
step-inputs parameter — a Step is `execute(ctx)`. `before_step` and `after_step` observe
only; their return values are discarded. `on_step_error` is the one hook whose return
value is read: a non-null value is a recovery result that short-circuits the remaining
handlers. See [middleware-system.md](./middleware-system.md) for the full contract.


### 1.4 Unified run_until Pattern

Implementations MUST support a `run_until` termination condition that halts pipeline execution when a predicate returns true. The predicate receives the current `PipelineState` (step name, outputs so far, context) and MUST return a boolean. When `run_until` returns true after step N, steps N+1 onward MUST NOT execute and the pipeline MUST return the accumulated result from steps 1 through N.

`run_until` is evaluated **after** each step completes (not before). If the predicate never returns true, the full pipeline runs to completion normally.

=== "Python"
    ```python
    import apcore
    from apcore import APCore, Config

    client = APCore(Config())

    @client.module(id="cache.fetch", description="Fetch from cache or compute")
    def cache_fetch(inputs, ctx):
        # Simulate a cache hit for known keys
        cache = {"key_abc": {"value": 99}}
        return {"hit": inputs["key"] in cache, "result": cache.get(inputs["key"])}

    # run_until: stop as soon as we get a cache hit after module_lookup
    def stop_on_cache_hit(state):
        # state.step_name is the step that just completed
        # state.outputs is a dict of step_name -> output so far
        if state.step_name == "module_lookup":
            # We haven't executed yet; continue
            return False
        # After execute step, check if we got a cache hit
        execute_output = state.outputs.get("execute")
        return bool(execute_output and execute_output.get("hit"))

    result = client.call(
        "cache.fetch",
        {"key": "key_abc"},
        options={"run_until": stop_on_cache_hit},
    )
    print(result)  # {"hit": True, "result": {"value": 99}}
    # Steps after execute (output_validation, middleware_after, return_result) did NOT run.
    ```

=== "TypeScript"
    ```typescript
    import { APCore, PipelineState } from 'apcore-js';

    const client = new APCore();

    client.module({
        id: 'cache.fetch',
        description: 'Fetch from cache or compute',
        execute: ({ key }: { key: string }) => {
            const cache: Record<string, unknown> = { key_abc: { value: 99 } };
            return { hit: key in cache, result: cache[key] ?? null };
        },
    });

    // run_until: stop as soon as we get a cache hit
    const stopOnCacheHit = (state: PipelineState): boolean => {
        if (state.stepName !== 'execute') return false;
        const output = state.outputs['execute'] as { hit?: boolean } | undefined;
        return output?.hit === true;
    };

    const result = await client.call(
        'cache.fetch',
        { key: 'key_abc' },
        { runUntil: stopOnCacheHit },
    );
    console.log(result); // { hit: true, result: { value: 99 } }
    // Steps after execute did NOT run.
    ```

=== "Rust"
    ```rust
    use apcore::{APCore, PipelineState};
    use serde_json::json;

    #[tokio::main]
    async fn main() {
        let client = APCore::default();

        // run_until: stop as soon as we get a cache hit
        let stop_on_cache_hit = |state: &PipelineState| -> bool {
            if state.step_name != "execute" {
                return false;
            }
            state
                .outputs
                .get("execute")
                .and_then(|o| o.get("hit"))
                .and_then(|h| h.as_bool())
                .unwrap_or(false)
        };

        let result = client
            .call_with_options(
                "cache.fetch",
                json!({"key": "key_abc"}),
                |opts| opts.run_until(stop_on_cache_hit),
            )
            .await
            .unwrap();
        println!("{result}"); // {"hit":true,"result":{"value":99}}
        // Steps after execute did NOT run.
    }
    ```

### 1.5 O(1) Control Flow Lookups

Implementations MUST use O(1) lookup structures (hash maps, dictionaries) for step name resolution within the pipeline. Implementations MUST NOT use linear scans (list iteration) to find a step by name during execution. This is a **performance requirement**; violation does not cause incorrect behavior but MUST be flagged during code review.

The step registry MUST be a hash map keyed by step name, built once when the pipeline is configured. Any operation that modifies the pipeline (adding, replacing, or removing a step) MUST update both the ordered list and the hash map atomically so they remain in sync.

!!! warning "Code Review Requirement"
    During code review, reviewers MUST verify that step name resolution inside the execution loop uses a hash map lookup (e.g., `steps_by_name[step_name]`) and never iterates over a list to find a step by name (e.g., `next(s for s in steps if s.name == step_name)`). Flag any violation even if tests pass.

---

## Contract: Pipeline.configure_step

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `step_name` (str/string/String, required) — target step to configure or replace
- `new_step` (PipelineStep instance, required) — replacement step object that owns its own handler / options

> **Spec amendment (D10-013).** Earlier spec drafts described separate
> `handler` and `options` arguments; no SDK implements that shape.
> Python (`pipeline.py:300`), TypeScript (`pipeline.ts:343`), and Rust
> (`pipeline.rs:619`) all accept a single `(step_name, new_step)` pair
> where the step instance carries its own handler and per-step options.

### Errors

- `PipelineStepNotFoundError(code=PIPELINE_STEP_NOT_FOUND)` — `step_name` does not exist in the current strategy
- `StepNotReplaceableError` — `step_name` resolves to a step marked non-replaceable by the strategy (raised by all three SDKs — `pipeline.py:310`, `pipeline.ts:349`, `pipeline.rs:635`). Cross-language note: Python and TypeScript carry the dedicated `STEP_NOT_REPLACEABLE` code; Rust reports this through `ErrorCode::GeneralInvalidInput` (`GENERAL_INVALID_INPUT`).
- `StepNameDuplicateError` — `new_step` declares a step name that conflicts with an existing step (raised by Python+TypeScript — `pipeline.ts:357`)

### Returns

- On success: void/None/()

### Properties

- `async`: false
- `thread_safe`: false — pipeline configuration MUST be completed before the first `call()` invocation
- `pure`: false — mutates pipeline state
- `idempotent`: true — replacing the same step twice with the same handler produces the same result
