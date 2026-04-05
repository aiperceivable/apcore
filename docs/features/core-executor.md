# Core Execution Engine

## Overview

The Core Execution Engine is the central orchestration component of apcore. It processes module calls through a secured execution lifecycle, handling everything from context creation and safety checks to module execution with timeout enforcement and result validation. The engine supports both synchronous and asynchronous execution paths, bridging between the two via threading and an async event loop bridge.

## Requirements

- Orchestrate module calls through a well-defined, sequential pipeline with clear separation of concerns at each step.
- Enforce safety constraints including maximum call depth limits, circular call detection, and frequency throttling to prevent runaway or abusive execution.
- Look up modules from the Registry and enforce access control lists (ACL) before execution.
- Validate inputs and outputs using Pydantic models, with automatic redaction of fields marked as `x-sensitive`.
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

7. **Input Validation with Pydantic + Sensitive Field Redaction** -- The call's input payload (including any modifications from middleware) is validated against the module's input schema (a dynamically generated Pydantic model). Fields annotated with `x-sensitive` are redacted from logs and error messages using the `redact_sensitive` utility.

8. **Module Execution with Timeout (Dual-Timeout Model)** -- The module's handler is invoked with dual-timeout enforcement: both a per-module timeout (`resources.timeout`, default 30s) and a global deadline (`executor.global_timeout`, default 60s). The shorter of the two is applied, preventing nested call chains from exceeding the global budget. The global deadline is set on the root call and propagated to child contexts via `Context._global_deadline`.

   **Cooperative cancellation:** On timeout, the executor sends `CancelToken.cancel()` and waits a 5-second grace period before raising `ModuleTimeoutError`. Modules that check `cancel_token` in their execution loop can clean up gracefully.

9. **Output Validation** -- The module's return value is validated against its output schema. Invalid output triggers an error rather than allowing malformed data to propagate.

10. **Middleware After Chain** -- All registered "after" middleware functions are executed in order with access to the context, input, and output. These may perform logging, transformation, or cleanup.

11. **Result Return** -- The final validated output (or error) is packaged into a structured result and returned to the caller.

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
- Synchronous modules called from an async context are dispatched to a daemon thread via `asyncio.to_thread`.
- Asynchronous modules called from a synchronous context are executed through a temporary event loop on a daemon thread.
- An async module cache lock protects concurrent access to shared module state.

### Sensitive Field Redaction

The `redact_sensitive` utility walks the input/output dictionaries and replaces values of fields marked `x-sensitive: true` in the schema with a placeholder string. This ensures sensitive data never appears in logs or error reports.

### Error Propagation (Algorithm A11)

All execution paths (sync, async, stream) wrap exceptions via `propagate_error()`, ensuring middleware always receives `ModuleError` instances with trace context attached. This guarantees consistent error handling regardless of the execution mode.

### Deep Merge for Streaming

Streaming chunk accumulation uses recursive deep merge (depth-capped at 32) instead of shallow merge. This correctly handles nested response structures where chunks contribute to different levels of the output tree.

### Validation (Preflight)

The `validate()` method provides a non-destructive preflight check that runs Steps 1–6 of the pipeline (module ID format, module lookup, call chain safety, ACL, approval detection, schema validation) without executing module code or middleware. It returns a `PreflightResult` with per-check results and a `requires_approval` flag. The result is duck-type compatible with the legacy `ValidationResult` — `.valid` and `.errors` properties work identically.

## Key Files

| File | Lines | Purpose |
|------|-------|---------|
| `executor.py` | 634 | Core execution engine implementing the execution pipeline |
| `context.py` | 66 | Context and Identity data classes |
| `config.py` | 29 | Executor configuration data class |
| `errors.py` | 395 | Structured error types for every failure mode in the pipeline |

## Dependencies

### External
- `pydantic>=2.0` -- Used for input/output schema validation, dynamic model generation, and field metadata.

### Internal
- **Registry** -- Module lookup (step 3) depends on the Registry system to resolve module names to loaded module instances.
- **Schema System** -- Input and output validation (steps 6 and 9) depend on the Schema System for Pydantic model generation from YAML schemas.

## Testing Strategy

- **Unit tests** cover each pipeline step in isolation, verifying that context creation, safety checks, ACL enforcement, validation, middleware chains, and result packaging all behave correctly for both success and failure cases.
- **Timeout tests** verify that both synchronous and asynchronous modules are correctly cancelled when exceeding configured timeouts, and that daemon threads do not leak.
- **Safety check tests** exercise call depth limits, circular detection with various call chain topologies, and frequency throttle edge cases.
- **Redaction tests** confirm that `x-sensitive` fields are properly masked in logs and error messages while remaining intact in the actual data passed to the module.
- **Integration tests** run full pipeline executions through the executor with real Registry and Schema instances to verify end-to-end behavior.
- Test naming follows the `test_<unit>_<behavior>` convention.
