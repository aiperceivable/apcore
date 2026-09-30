---
description: "Core Executor running the 11-step call pipeline: context creation, call-chain guard, registry lookup, ACL, approval, before/after middleware, schema validation+redaction, timeout."
---

# Core Execution Engine

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §12 SDK Implementation Guide.


## Overview

The Executor is the central orchestration component of apcore. Every module call runs through an ordered pipeline of steps — context creation, safety checks, lookup, access control, approval, middleware, validation, execution with timeouts, and result return. The pipeline is an `ExecutionStrategy` run by a `PipelineEngine`; this page describes what the Executor does with it. Strategies, the Step protocol, presets and the engine itself are described in [Execution Pipeline](./execution-pipeline.md).

## Requirements

- Orchestrate module calls through a well-defined, sequential pipeline with clear separation of concerns at each step.
- Enforce call-chain safety — depth limit, circular-call detection and self-recursion bound — before any module code runs.
- Look up modules from the Registry and enforce the ACL and the approval gate before execution.
- Validate inputs and outputs against the module schemas, and expose redacted copies (fields marked `x-sensitive`) for logging.
- Run module-level middleware before and after the module (onion model), with error recovery through `on_error`.
- Enforce a per-module timeout and a global deadline for the whole call tree.
- Surface every failure to the caller as its original typed error.

## Technical Design

### Execution Pipeline

The standard strategy runs eleven steps, in this order in all three SDKs:

1. **`context_creation`** — Derives the Context for this call: a child of the caller's Context (the target appended to `call_chain`, `caller_id` set to the previous module) or a fresh top-level one. When the Context carries no `global_deadline`, it is set from `executor.global_timeout`.

2. **`call_chain_guard`** — Rejects the call when the chain is too deep (`executor.max_call_depth`, default 32 → `CALL_DEPTH_EXCEEDED`), circular (A→B→A → `CIRCULAR_CALL`), or a module recurses into itself more than `executor.max_module_repeat` times (default 3 → `CALL_FREQUENCY_EXCEEDED`). A cancelled `cancel_token` is also detected here. The exact rules are Algorithm A20 — see [Call Chain Guard](./call-chain-guard.md).

3. **`module_lookup`** — Resolves the module from the Registry (`MODULE_NOT_FOUND` when it is not registered, `MODULE_DISABLED` when it has been disabled through `system.control.toggle_feature`). It also resolves the registry's declared annotations, computes the governance projection of the arguments for the ACL `arguments` condition, and sets `context.redacted_inputs` from the incoming inputs so that before-middleware can log safely.

4. **`acl_check`** — Evaluates the attached ACL for (`caller_id`, or `@external` when it is null; target module ID; Context). A denial raises `ACL_DENIED`. With no ACL attached the step allows the call. A rule carrying `approval: required` is recorded for Step 5.

5. **`approval_gate`** — Engages when **any** of these requires approval: the module's `requires_approval` annotation, the registry descriptor/metadata (D-96, D-125), an ACL rule with `approval: required` ([§6.9](../spec/protocol-spec.md)), or an `ExecutionPolicy` with `gate_destructive` ([§7.9.2](../spec/protocol-spec.md)). The configured `ApprovalHandler` decides; rejected, timed-out and pending decisions raise `APPROVAL_DENIED`, `APPROVAL_TIMEOUT` and `APPROVAL_PENDING`. With no handler the gate is skipped with a warning, unless `ExecutionPolicy(strict=true)` makes it fail closed. See [Approval System](./approval-system.md).

6. **`middleware_before`** — Runs every middleware's `before()` in priority order; each may replace the inputs. A failure here runs the `on_error` chain. See [Middleware System](./middleware-system.md).

7. **`input_validation`** — Validates the (possibly middleware-modified) inputs against the module's `input_schema` (`SCHEMA_VALIDATION_ERROR`) and refreshes `context.redacted_inputs` from them.

8. **`execute`** — Checks the cancel token once more, then invokes the module under the per-module timeout, clamped to the remaining global deadline (see [Timeouts](#timeouts)). An error from the module runs the `on_error` chain.

9. **`output_validation`** — Validates the output against the module's `output_schema` and sets `context.redacted_output`.

10. **`middleware_after`** — Runs every middleware's `after()` in reverse order; each may replace the output.

11. **`return_result`** — Returns the final output.

`context_creation`, `module_lookup`, `execute` and `return_result` cannot be removed from a strategy; the other seven can be removed by presets (`internal`, `testing`, `performance`, `minimal`) or custom strategies. Each step also declares `match_modules`, `ignore_errors`, `pure` and `timeout_ms`. See [Execution Pipeline](./execution-pipeline.md).

### Key Types

- **Executor** — Runs the pipeline; owns the middleware chain, the ACL, the approval handler, the optional `ExecutionPolicy` and the timeout settings.
- **Context** — Per-call state: `trace_id`, `caller_id`, `call_chain`, `identity`, the shared `data` map and the redacted copies. See [Context Object](./context-object.md).
- **Identity** — The caller identity the ACL evaluates. See [Identity System](./identity-system.md).
- **Config** — Supplies `executor.default_timeout`, `executor.global_timeout`, `executor.max_call_depth`, `executor.max_module_repeat`, `pipeline.*` and the redaction rules.

### Sync/Async Bridge

Python exposes both `call()` (blocking) and `call_async()` (coroutine). The blocking form runs the async pipeline on a cached event loop, or on a background thread when it is called from inside a running loop. Synchronous module functions called from the async pipeline run in a worker thread so they do not block the loop. TypeScript and Rust have a single async `call()`.

### Sensitive Field Redaction

`redacted_inputs` / `redacted_output` are copies of the payload with fields marked `x-sensitive: true` — and keys matching the configured `obs.redaction.*` rules — replaced by `***REDACTED***`. The module always receives the real values. Redaction rules are described in [Redaction](./redaction.md).

### Error Propagation (Algorithm A11)

Every execution path (sync, async, stream) wraps exceptions via `propagate_error()`, so middleware always receives a `ModuleError` carrying the trace context. Internally, a failing step is wrapped in `PipelineStepError` (see [Fail-Fast Error Handling](#fail-fast-error-handling)); the Executor unwraps it, so callers catch the original typed error.

### Deep Merge for Streaming

Streamed chunks are accumulated with a recursive deep merge, capped at `stream.max_merge_depth` (default 32), so chunks can contribute to different levels of the output tree. See [Streaming](./streaming.md).

### Validation (Preflight)

`validate()` is a non-destructive preflight: it runs Steps 1–5 and 7 of the pipeline — no middleware, no module execution — plus the module's optional `preflight()` hook, and returns a `PreflightResult` (`valid`, `checks`, `requires_approval`, `errors`). The pipeline runs in dry-run mode, which skips every step not declared `pure`; Step 5 is therefore evaluated as detection only — `requires_approval` reports the governance-effective requirement and no `ApprovalHandler` is invoked. A failed `acl` check suppresses the module's `preflight()` / `preview()` hooks. See [PROTOCOL_SPEC §12.8](../spec/protocol-spec.md#128-executorvalidate-cross-language-implementation-guide) and [Contract: APCore.validate](./apcore-client.md#contract-apcorevalidate).

### Execution State Machine

```text
  ┌─────────┐
  │  idle   │
  └────┬────┘
       │ call()
       ▼
  ┌──────────┐  depth/cycle/freq  ┌────────────────────────────────┐
  │call_chain│───────────────────▶│ CALL_DEPTH_EXCEEDED            │
  │  guard   │                    │ / CIRCULAR_CALL                │
  └────┬─────┘                    │ / CALL_FREQUENCY_EXCEEDED      │
       │ check passed             └────────────────────────────────┘
       ▼
  ┌─────────┐   not registered    ┌──────────────────┐
  │ lookup  │────────────────────▶│ MODULE_NOT_FOUND │
  └────┬────┘   / disabled        │ / MODULE_DISABLED│
       │                          └──────────────────┘
       │ module found
       ▼
  ┌─────────┐   denied            ┌──────────────────┐
  │  acl    │────────────────────▶│ ACL_DENIED       │
  └────┬────┘                     └──────────────────┘
       │ allowed
       ▼
  ┌──────────┐ rejected/timeout  ┌──────────────────────────┐
  │ approval │──────────────────▶│ APPROVAL_DENIED          │
  │   gate   │                   │ / APPROVAL_TIMEOUT       │
  └────┬─────┘                   │ / APPROVAL_PENDING       │
       │                         └──────────────────────────┘
       │ approved (or not required)
       ▼
  ┌──────────┐
  │ before   │──── middleware error ──▶ on_error chain
  │middleware│
  └────┬─────┘
       │ inputs possibly replaced
       ▼
  ┌──────────┐   invalid          ┌─────────────────────────┐
  │ validate │───────────────────▶│ SCHEMA_VALIDATION_ERROR │
  │  input   │                    └─────────────────────────┘
  └────┬─────┘
       │ valid
       ▼
  ┌──────────┐   error / timeout  ┌──────────────────────┐
  │ execute  │───────────────────▶│ on_error chain       │
  │  module  │                    └──────────────────────┘
  └────┬─────┘
       │ success
       ▼
  ┌──────────┐   invalid          ┌─────────────────────────┐
  │ validate │───────────────────▶│ SCHEMA_VALIDATION_ERROR │
  │  output  │                    └─────────────────────────┘
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
  │  return  │
  │  result  │
  └──────────┘
```

The `on_error` chain runs over the middlewares whose `before()` was entered; a recovery value becomes the call's output, and a `RetrySignal` re-runs the pipeline with new inputs. Cancellation bypasses it (see [Cancellation Short-Circuit](#cancellation-short-circuit)).

### Timeouts

| Setting | Default | Source |
|---------|---------|--------|
| Per-module timeout | 30 000 ms | The module's `resources.timeout`, else `executor.default_timeout` |
| Global deadline | 60 000 ms | `executor.global_timeout`, set on the root Context at Step 1 and inherited by child calls |

The per-module timeout applies to Step 8 and is clamped to the time left before the global deadline, so a nested call tree cannot outlive its root's budget. `0` disables the per-module limit (the global deadline still applies); a negative timeout raises `GENERAL_INVALID_INPUT`.

**On timeout** the Executor raises `MODULE_TIMEOUT` immediately. It does not signal the Context's `cancel_token`: Python cancels the module's coroutine (a synchronous module running in a worker thread keeps running to completion), TypeScript stops awaiting the module's promise, and Rust drops the module's future. The cooperative-cancel-then-grace-period sequence of [PROTOCOL_SPEC §12.7.5](../spec/protocol-spec.md#1275-timeout-enforcement) is not implemented by any SDK.

A module that wants to stop early on caller-driven cancellation checks its cancel token ([Cancellation](./cancellation.md)):

=== "Python"
    ```python
    import asyncio

    from apcore import APCore, Context

    client = APCore()

    @client.module(id="batch.process", description="Process items until cancelled")
    async def process(items: list[str], context: Context) -> dict:
        done = 0
        for _item in items:
            if context.cancel_token is not None and context.cancel_token.is_cancelled:
                return {"partial": True, "processed": done}
            await asyncio.sleep(0.01)
            done += 1
        return {"partial": False, "processed": done}

    print(client.call("batch.process", {"items": ["a", "b", "c"]}))
    ```
=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { APCore } from "apcore-js";

    const client = new APCore();

    client.module({
        id: "batch.process",
        description: "Process items until cancelled",
        inputSchema: Type.Object({ items: Type.Array(Type.String()) }),
        outputSchema: Type.Object({ partial: Type.Boolean(), processed: Type.Number() }),
        execute: async (inputs, context) => {
            let done = 0;
            for (const _item of inputs.items as string[]) {
                if (context.cancelToken?.isCancelled) {
                    return { partial: true, processed: done };
                }
                await new Promise((resolve) => setTimeout(resolve, 10));
                done += 1;
            }
            return { partial: false, processed: done };
        },
    });

    console.log(await client.call("batch.process", { items: ["a", "b", "c"] }));
    ```
=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use apcore::APCore;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct BatchProcess;

    #[async_trait]
    impl Module for BatchProcess {
        fn description(&self) -> &str { "Process items until cancelled" }
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"items": {"type": "array", "items": {"type": "string"}}}})
        }
        fn output_schema(&self) -> Value { json!({"type": "object"}) }
        async fn execute(&self, inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let items = inputs["items"].as_array().cloned().unwrap_or_default();
            let mut done = 0;
            for _item in items {
                if ctx.cancel_token.as_ref().is_some_and(|t| t.is_cancelled()) {
                    return Ok(json!({"partial": true, "processed": done}));
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                done += 1;
            }
            Ok(json!({"partial": false, "processed": done}))
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        client.register("batch.process", Box::new(BatchProcess))?;
        let out = client.call("batch.process", json!({"items": ["a", "b", "c"]}), None, None).await?;
        println!("{out}");
        Ok(())
    }
    ```

### Concurrent Execution Semantics

- A single Executor instance MUST tolerate concurrent calls from multiple threads/coroutines.
- Each call MUST receive its own derived Context (independent `call_chain` and `caller_id`).
- `context.data` is shared by reference along a call tree; concurrent calls SHOULD use distinct top-level Contexts when isolation matters.
- Concurrent `call_async()` calls MAY complete in any order.

See [PROTOCOL_SPEC §12.7 Concurrency Model Specification](../spec/protocol-spec.md#127-concurrency-model-specification).

### Edge Cases

| Scenario | Behavior |
|----------|----------|
| `module_id` is empty, malformed or over-length | `INVALID_MODULE_ID`, before the pipeline starts |
| `inputs` is null/absent | Treated as `{}` |
| `context` is null/absent | A new top-level Context is created |
| Declared timeout is `0` | No per-module limit; the global deadline still applies |
| Declared timeout is negative | `GENERAL_INVALID_INPUT` |
| `call()` while the module is being unregistered | A call that has passed lookup completes; a later call raises `MODULE_NOT_FOUND` |
| `call_chain` longer than `max_call_depth` | `CALL_DEPTH_EXCEEDED` |
| One Context shared by concurrent calls | `data` is shared; synchronize externally |

### Fail-Fast Error Handling

When a step fails, the engine stops and wraps the error in `PipelineStepError` (carrying the step name), unless the step is configured with `ignore_errors: true` — then the failure is logged and the pipeline continues with the step's output absent. `ignore_errors` is set per step through `pipeline.configure` (D-72):

```yaml
pipeline:
  configure:
    output_validation:      # map keyed by step name
      ignore_errors: true
```

`PipelineStepError` is an engine-level type. `Executor.call()` unwraps it, so callers catch the typed cause — for a failed input validation, `SchemaValidationError` (`SCHEMA_VALIDATION_ERROR`) — and middleware `on_error` hooks see the same cause. It is observable directly only when driving a `PipelineEngine` yourself. Normative text: [PROTOCOL_SPEC §5.16](../spec/protocol-spec.md#516-pipeline-control-flow-requirements).

### Strategies and Step-Level Extension

The Executor is constructed with a strategy — the standard one by default, a preset by name, a custom `ExecutionStrategy`, or the strategy built from the `pipeline:` section of the Config (§5.16 requirement 6). Introspection lives on the Executor:

| Surface | Description |
|---------|-------------|
| `Executor.register_strategy(name, strategy)` | Python / TypeScript: registers a strategy resolvable by name at construction. Rust's module-level `register_strategy(StrategyInfo)` registers introspection info only; `Executor::with_strategy_name` resolves the five built-in presets |
| `executor.list_strategies()` | `StrategyInfo` (`name`, `step_count`, `step_names`, `description`) for the current strategy followed by every registered one |
| `executor.describe_pipeline()` | `StrategyInfo` for the current strategy |
| `executor.current_strategy` | The running `ExecutionStrategy` (Python property, TypeScript getter; Rust `strategy()`) |

Replacing a step (`configure_step`), step middleware (`StepMiddleware`), and the `run_until` predicate are strategy- and engine-level features:

- **Step middleware** is registered on the strategy (Python `strategy.add_step_middleware()`, Rust `ExecutionStrategy::add_step_middleware()` before the Executor is built) or, in TypeScript, on a `PipelineEngine` instance — the Executor's own engine is not reachable from `APCore`. Its contract is in [Middleware System](./middleware-system.md#pipeline-step-middleware).
- **[`run_until`](./execution-pipeline.md#run_until)** is a predicate over `PipelineState` that halts the pipeline after the step for which it returns `true`. It is accepted by the pipeline engine — Python `PipelineEngine().run(strategy, ctx, run_until=...)` or `PipelineContext.run_until`, TypeScript `PipelineContext.runUntil`, Rust `PipelineEngine::run_until(&strategy, &mut ctx, predicate)` / `RunOptions::run_until` — and is **not** an option of `Executor.call()` or `APCore.call()` in any SDK.

See [Execution Pipeline](./execution-pipeline.md) for all three.

## Contract: Executor.governance_state

`acl != null` does not mean ACL evaluation runs. The ACL and approval gates are pipeline **steps**, and three of the built-in presets — `internal`, `testing` and `minimal` — remove `acl_check` ([§6.6.3.2](../spec/protocol-spec.md#6632-a-configured-layer-is-not-necessarily-an-enforced-one)). An executor can therefore hold an ACL that is never consulted. `governance_state()` separates the two questions; it is a **pure read** that never enforces, warns, throws or mutates. Normative specification: [PROTOCOL_SPEC §6.6.5](../spec/protocol-spec.md#665-governance-state-query).

### Governance State API

| Field | Answers |
|---|---|
| `control_modules_registered` | Is at least one `system.control.*` module in the registry? |
| `read_modules_registered` | Is at least one read-only `system.*` module registered? |
| `acl_configured` | Is an ACL object attached? |
| `builtin_acl_gate_wired` | Does the **running strategy** contain the built-in ACL gate (matched by type, never by step name)? |
| `approval_handler_configured` | Is an `ApprovalHandler` attached? |
| `builtin_approval_gate_wired` | Does the running strategy contain the built-in approval gate? |
| `policy_strict` | Is `ExecutionPolicy(strict=true)` set — the approval gate failing closed **on a call it engages on**? |
| `all_control_modules_require_approval` | Does **every** registered `system.control.*` module declare `requires_approval`? |
| `unprotected_control_surface` | Derived: control modules are registered and no recognised built-in gate is configured, wired **and actually engaged** for them. |

**The two gates are not symmetric.** `acl_check` evaluates every call, so "configured + wired" means the gate stands in front of `system.control.*`. `approval_gate` resolves per module and returns immediately when the module does not need approval — so a wired gate with a handler attached, or with `strict=true`, gates nothing for a control module that never declares `requires_approval`. That is why `all_control_modules_require_approval` is a separate observation and a required conjunct of the derived flag ([§6.6.5.1.1](../spec/protocol-spec.md#66511-why-the-two-gates-are-not-symmetric)).

=== "Python"
    ```python
    from apcore import APCore, Config

    client = APCore(config=Config())
    state = client.executor.governance_state()

    # An attached ACL that no step consults.
    if state.acl_configured and not state.builtin_acl_gate_wired:
        print("ACL attached but the running strategy has no acl_check step")

    # An approval handler the gate never reaches, because a control module
    # does not declare requires_approval.
    if state.approval_handler_configured and not state.all_control_modules_require_approval:
        print("approval handler attached but some system.control.* module is ungated")

    if state.unprotected_control_surface:
        print("system.control.* is registered with no recognised gate in front of it")
    ```

=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";

    const client = new APCore();
    const state = client.executor.governanceState();

    if (state.aclConfigured && !state.builtinAclGateWired) {
        console.warn("ACL attached but the running strategy has no acl_check step");
    }

    if (state.unprotectedControlSurface) {
        console.warn("system.control.* is registered with no recognised gate in front of it");
    }
    ```

=== "Rust"
    ```rust
    use apcore::APCore;

    fn main() {
        let client = APCore::new();
        let state = client.executor().governance_state();

        if state.acl_configured && !state.builtin_acl_gate_wired {
            eprintln!("ACL attached but the running strategy has no acl_check step");
        }

        if state.unprotected_control_surface {
            eprintln!("system.control.* is registered with no recognised gate in front of it");
        }
    }
    ```

**What `unprotected_control_surface` does not say.** It reports the *absence of a gate*, never the presence of protection: a wired ACL that permits every call still yields `false`. And `true` does not mean the call will succeed — a deployment may enforce through a custom step, custom middleware or an upstream gateway, none of which this accessor can see. Do not surface it as a security verdict, and do not add an `is_secure`-shaped inverse. Adapters that warn at startup about an unprotected control surface should call this accessor rather than re-derive it from `describe_pipeline()`, which reports step names only and cannot tell a built-in gate from a look-alike.

### Inputs
- No inputs

### Errors
- None — `governance_state()` MUST NOT enforce, warn, throw, or mutate ([§6.6.5.3](../spec/protocol-spec.md#6653-constraints), constraint 1)

### Returns
- `GovernanceState` — the eight observation booleans plus the derived `unprotected_control_surface`. MUST NOT expose the ACL object, the `ApprovalHandler`, the `ExecutionPolicy`, or any rule content ([§6.6.5.3](../spec/protocol-spec.md#6653-constraints), constraint 2).

### Properties
- async: false
- thread_safe: true
- pure: true — a live read of current executor state, never cached ([§6.6.5.3](../spec/protocol-spec.md#6653-constraints), constraint 4)
- idempotent: true

## Contract: Executor.call

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `module_id`: string, required. Validated at method entry — empty, malformed and over-length IDs are rejected before the pipeline context is constructed. Reserved first segments ([§2.5](../spec/protocol-spec.md#25-reserved-words)) are not rejected here, which is what makes `system.*` modules callable.
- `inputs`: object, optional. Payload conforming to the module's input schema; absent is `{}`.
- `context`: Context, optional. Created when absent.
- `version_hint`: string, optional. Preferred version (resolved in Python; TypeScript and Rust resolve the latest registered version).

### Preconditions

- Entry guard: `module_id` MUST be validated before constructing a pipeline context. Implementations MUST NOT defer this check to downstream steps.

### Side Effects (ordered)

1. Validate `module_id`; reject with `InvalidInputError(code=INVALID_MODULE_ID)`.
2. Bind the Executor to the Context ([Contract: Executor binding to Context](#contract-executor-binding-to-context)).
3. Run the pipeline (see [Execution Pipeline](#execution-pipeline)), including `on_error` recovery and `RetrySignal` re-runs.
4. Emit spans, metrics and events per configuration.

### Errors

- `InvalidInputError(code=INVALID_MODULE_ID)` — `module_id` fails the entry guard.
- `ModuleNotFoundError(code=MODULE_NOT_FOUND)` — `module_id` not present in the registry.
- `ModuleDisabledError(code=MODULE_DISABLED)` — the module has been disabled at runtime.
- `CallDepthExceededError`, `CircularCallError`, `CallFrequencyExceededError`, `ACLDeniedError`, `ApprovalDeniedError`, `ApprovalTimeoutError`, `ApprovalPendingError`, `SchemaValidationError`, `ModuleTimeoutError`, `ExecutionCancelledError`, `ContextBindingError`, and errors raised by the module or middleware — propagated as their original typed error ([Error Unwrap Rule](#error-unwrap-rule)).

### Returns

- On success: validated output conforming to the module's output schema (after-middleware applied), or an `on_error` recovery value.
- On failure: raises (Python/TypeScript) / returns `Err` (Rust).

### Properties

- `async`: synchronous in Python (`call_async` is the coroutine); asynchronous in TypeScript and Rust.
- `thread_safe`: `true`.
- `pure`: `false` — pipeline steps may emit events, mutate observability state, and transitively invoke other modules.

### Trace Variants (`call_with_trace` / `callWithTrace`)

SDKs MAY expose a trace-returning variant — `call_with_trace` in Python and Rust, `callWithTrace` in TypeScript — that returns the result together with a `PipelineTrace` (per-step timings and middleware events). When implemented, it MUST share **identical error-recovery semantics** with `call()` (D-19):

- **MUST** run the same pipeline, including the `on_error` chain — a middleware that recovers in `call()` recovers here, and an error that propagates in `call()` propagates here.
- **MUST** apply the [Cancellation Short-Circuit](#cancellation-short-circuit).
- **MUST** apply the [Error Unwrap Rule](#error-unwrap-rule).
- **MUST** populate the returned `PipelineTrace` with every middleware event observed, including `on_error` recovery.

It differs from `call()` only in its return shape; events, metrics and ACL audit records MUST be identical.

### Cancellation Short-Circuit

When an execution is cancelled (`ExecutionCancelledError` from the `CancelToken`), the Executor MUST propagate it directly, bypassing the `on_error` chain — cancellation is a caller-driven request to stop, not a recoverable failure, and a logging or retry middleware must not be able to swallow or restart it. SDKs detect it after unwrapping `PipelineStepError` / `MiddlewareChainError` (D-20).

### Cancel Token Mid-Pipeline Check

The pipeline MUST observe `cancel_token` cancellation at two points, in addition to the module's own checks (D-21):

1. **Step 2 (`call_chain_guard`)** — before any validation or middleware work.
2. **Step 8 (`execute`)** — immediately before invoking the module.

### Error Unwrap Rule

When a middleware (`before` / `after` / `on_error`) raises a typed error such as `ApprovalDeniedError`, the chain machinery may wrap it in `MiddlewareChainError`. The Executor MUST unwrap the wrapper and surface the **original typed cause** unchanged; SDKs MUST NOT replace it with a generic `ModuleExecuteError`, which would stop callers — MCP/A2A bridges in particular — from dispatching on the error code (D-22).

## Contract: Context.create

Normative behavioral contract for the factory that produces a new top-level call context.

### Inputs

Across all SDKs the factory accepts **exactly** these six caller-supplied fields, in this order (snake_case in Python/Rust, camelCase in TypeScript; positional in TypeScript and Rust):

| # | Name | Type | Default | Notes |
|---|------|------|---------|-------|
| 1 | `identity` | Identity \| null | null | Stays null when absent. `@external` is the caller-side ACL sentinel for a null `caller_id`; an implementation MUST NOT synthesize an `Identity` for a call that supplied none (D-103). |
| 2 | `trace_parent` | TraceParent \| null | null | W3C Trace Context entry; `tracestate` travels inside the `TraceParent`, not as a separate parameter. Invalid values (non-32-hex, all-zero, all-f) MUST log WARN and be replaced with a fresh `trace_id`. |
| 3 | `cancel_token` | CancelToken \| null | null | External cooperative-cancellation source. |
| 4 | `data` | Mapping<string, Any> \| null | empty | Initial shared state, carried through the call tree by reference. |
| 5 | `services` | T \| null | null | Caller-supplied DI container. MUST NOT carry framework-owned fields. Rust takes it as a positional `T` (e.g. `Value::Null`), not an `Option`. |
| 6 | `global_deadline` | epoch seconds \| null | null | Bounds total execution time for the call tree rooted here. Local-only (see [`global_deadline`](#global_deadline-representation-and-lifetime)). |

These Context fields are **not** caller inputs:

- `trace_id` — derived from a valid `trace_parent`, otherwise a fresh 32-char lowercase hex value.
- `caller_id` — always null at top level; managed by `Context.child()`.
- `call_chain` — empty at top level; managed by the Executor.
- `executor` — bound by the Executor at pipeline entry ([Contract: Executor binding to Context](#contract-executor-binding-to-context)).
- `redacted_inputs` — set by pipeline Step 3 (`module_lookup`, from the incoming inputs) and refreshed at Step 7 (`input_validation`); `redacted_output` — set at Step 9 (`output_validation`).

### Preconditions

- `trace_parent` (if present) is validated; invalid values trigger regeneration with a WARN log, not rejection.

### Errors

None under normal operation.

### Returns

A fresh `Context` with a 32-character lowercase hex `trace_id`; `executor`, `caller_id` unset and `call_chain` empty; the caller-supplied fields as provided; `redacted_inputs` / `redacted_output` unset.

### Properties

- `async`: `false`.
- `thread_safe`: `true`.
- `pure`: `false` — a new `trace_id` is generated for each call.
- `idempotent`: `false`.

## Contract: Executor binding to Context

A Context whose `executor` field is null can come from local construction (`Context.create()`), cross-process deserialization (`executor` never serializes — §5.7), or restoration from persistence after a restart. The Executor treats all three the same:

1. **Bind** — When the Executor receives a Context whose `executor` is null, it MUST bind itself **before** pipeline Step 1.
2. **Stability** — Once bound, `context.executor` MUST NOT change for the remainder of the call chain.
3. **Same-executor idempotency** — If `context.executor` is the **same** Executor instance (identity comparison), the rebind is a no-op; the Executor MUST NOT raise. This covers reusing one Context across several top-level calls.
4. **Cross-executor conflict** — If `context.executor` is a **different** Executor instance, the Executor MUST raise `CONTEXT_BINDING_ERROR` ([PROTOCOL_SPEC §12.2](../spec/protocol-spec.md#122-core-component-interface-contracts)).
5. **Propagation** — `Context.child()` MUST propagate the bound `executor` unchanged.

The mechanism is language-idiomatic: in-place assignment in Python and Rust, copy-on-write returning a new instance for TypeScript's `readonly` fields. See [Context Object §Serialization](./context-object.md#serialization).

The binding method is a cross-boundary contract member — the Executor calls it, and a bridge's duck-typed `Context` MUST implement it — so it MUST be public in every SDK. See [API Surface & Naming Conventions](../spec/api-surface-conventions.md).

### Inputs
- `executor` (Executor instance, required) — passed to the Context's binding method (`bind_executor` in Python/Rust, `withExecutor` in TypeScript — see [API Surface & Naming Conventions §5](../spec/api-surface-conventions.md#5-worked-example-executor-binding-to-context))

### Preconditions
- Invoked by the Executor itself before pipeline Step 1

### Errors
- `CONTEXT_BINDING_ERROR` — the Context is already bound to a different Executor instance (Rule 4). `validate()` reports this as a failed `executor_binding` check instead of raising.

### Returns
- Python / Rust (`bind_executor`): nothing — `context.executor` is set in place
- TypeScript (`withExecutor`): a new `Context` with `executor` populated; the original is unchanged

### Properties
- async: false
- thread_safe: not separately specified; once bound, `context.executor` does not change (Rule 2)
- pure: false
- idempotent: true for the same Executor instance (Rule 3); not across distinct instances (Rule 4)

## Distributed Cancellation Semantics

`cancel_token` is runtime-only and MUST NOT serialize (§5.7). On the receiving node of a deserialized Context:

- The Executor MUST synthesize a fresh local `CancelToken` at pipeline entry. The remote node never observes the originating node's token.
- Distributed cancellation MUST go through **out-of-band channels** (an `AsyncTaskManager` task-ID lookup, a remote cancel signal, a control-plane RPC). It MUST NOT ride the in-context `cancel_token` across process boundaries.

The `cancel_token` parameter of `Context.create()` exists for **in-process cooperation** — for example binding an HTTP request's abort signal to the call tree it spawns.

## `global_deadline` Representation and Lifetime

- **Epoch seconds.** `global_deadline` is an absolute deadline in epoch seconds (`float` / `number` / `f64`). An implementation MUST NOT use a monotonic clock or milliseconds, because callers write it as `time.time() + budget` (D-99).
- **A first-class field.** It is stored in `Context.global_deadline`, never under a `context.data` key (D-100).
- **Owned by the call tree.** It is computed onto the Context the pipeline derives for the call and MUST NOT be written onto a caller-supplied Context that outlives the call, so reusing a Context does not inherit a previous call's remaining budget (D-101).
- **Recomputed when absent.** A deserialized Context carries no deadline, so the receiving Executor recomputes it from its own `executor.global_timeout`, whatever the length of `call_chain`; a caller's explicit deadline is kept (D-102).

## `global_deadline` Distributed Semantics

`global_deadline` is runtime-only and MUST NOT serialize. The originating node's deadline is intentionally not propagated; the receiving Executor applies its local `executor.global_timeout`.

Callers that need a wall-clock deadline to cross process boundaries SHOULD store the absolute timestamp in `context.data` under a serializable key of their own (any key not starting with `_`, which serialization drops — e.g. `ext.myapp.deadline`). The receiving side can translate it back into a local deadline.

## Usage

=== "Python"
    ```python
    import asyncio

    from apcore import APCore, Config

    client = APCore(config=Config())

    @client.module(id="math.add", description="Add two numbers")
    def add(a: int, b: int) -> dict:
        return {"sum": a + b}

    # Synchronous call
    print(client.call("math.add", {"a": 1, "b": 2}))  # {"sum": 3}

    # Asynchronous call
    async def main() -> None:
        result = await client.call_async("math.add", {"a": 10, "b": 20})
        print(result)  # {"sum": 30}

    asyncio.run(main())
    ```

=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { APCore } from "apcore-js";

    const client = new APCore();

    client.module({
        id: "math.add",
        description: "Add two numbers",
        inputSchema: Type.Object({ a: Type.Number(), b: Type.Number() }),
        outputSchema: Type.Object({ sum: Type.Number() }),
        execute: (inputs) => ({ sum: (inputs.a as number) + (inputs.b as number) }),
    });

    const result = await client.call("math.add", { a: 1, b: 2 });
    console.log(result); // { sum: 3 }
    ```

=== "Rust"
    ```rust
    use apcore::context::Context;
    use apcore::errors::ModuleError;
    use apcore::module::Module;
    use apcore::APCore;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct AddModule;

    #[async_trait]
    impl Module for AddModule {
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]})
        }
        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"sum": {"type": "number"}}})
        }
        fn description(&self) -> &str { "Add two numbers" }
        async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            let a = input["a"].as_f64().unwrap_or(0.0);
            let b = input["b"].as_f64().unwrap_or(0.0);
            Ok(json!({"sum": a + b}))
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = APCore::new();
        client.register("math.add", Box::new(AddModule))?;

        let result = client.call("math.add", json!({"a": 1.0, "b": 2.0}), None, None).await?;
        println!("{result}"); // {"sum":3.0}
        Ok(())
    }
    ```

## Dependencies

- **Registry** — Module lookup (Step 3).
- **Schema System** — Input and output validation (Steps 7 and 9).
- **ACL System**, **Approval System** — Steps 4 and 5.
- **Middleware System** — Steps 6 and 10 and the `on_error` chain.

??? info "Python SDK reference"
    Not a protocol requirement — the Python SDK's source layout for users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `executor.py` | `Executor`: entry points, validate, error recovery |
    | `builtin_steps.py` | The eleven built-in steps and the preset strategies |
    | `pipeline.py` | `PipelineEngine`, `ExecutionStrategy`, `PipelineContext`, `StepMiddleware` |
    | `context.py` | `Context`, `Identity` |

    Runtime dependency: `pydantic>=2.0` for schema validation.

## Testing Strategy

- **Unit tests** cover each pipeline step in isolation for success and failure.
- **Timeout tests** verify per-module and global-deadline enforcement for sync and async modules.
- **Call-chain tests** exercise depth limits, circular detection and self-recursion bounds.
- **Redaction tests** confirm that `x-sensitive` fields are masked in `redacted_inputs` / `redacted_output` while the module receives the real values.
- **Error tests** confirm that callers receive the typed cause, not `PipelineStepError` or `MiddlewareChainError`, and that cancellation bypasses `on_error`.
- **Integration tests** run full pipelines through the Executor with real Registry and schema instances.
