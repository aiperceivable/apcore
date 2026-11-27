---
description: "Approval gate at Executor Step 5: fires on the governance union, pluggable ApprovalHandler, sync and pending/resume-token flows, built-in handlers, ExecutionPolicy overrides and governance events."
---

# Approval System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §7 Approval System (§7.9 Execution Policy).


## Overview

The Approval System decides whether a particular invocation needs sign-off before it runs, and asks an `ApprovalHandler` for that sign-off at **Step 5** of the execution pipeline — after ACL enforcement (Step 4) and before the Middleware Before Chain (Step 6).

The gate considers the module's own flags, the registry's declared annotations, the platform's execution policy, and the [ACL decision](./acl-system.md) for the invocation. [Execution Policy](#execution-policy) defines how these sources combine.

The Approval System is architecturally separate from the ACL System. ACL answers "who is allowed to call this module?" while Approval answers "does this particular invocation need sign-off before proceeding?"

## Requirements

- Provide a pluggable `ApprovalHandler` protocol that SDK implementations can satisfy with custom logic.
- Enforce the approval gate at Executor Step 5, after ACL (Step 4) and before Middleware Before Chain (Step 6).
- Fire the gate when approval is required under the execution policy and ACL rules below.
- When approval is needed but no `ApprovalHandler` is configured, skip the gate with a warning (once per module), or fail closed with `ApprovalDeniedError` when `ExecutionPolicy(strict=true)`.
- Support synchronous approval flows (Phase A) where `request_approval()` resolves to a decision.
- Optionally support asynchronous approval flows (Phase B) where a `pending` status is returned with an `approval_id`, and execution resumes when the client retries with an `_approval_token`.
- Translate the handler's status into structured errors (`APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `APPROVAL_PENDING`).
- Ship built-in handlers for common cases: `AlwaysDenyHandler` (safe default), `AutoApproveHandler` (testing), and `CallbackApprovalHandler` (custom function).

## Technical Design

### Approval Gate (Executor Step 5)

0. The gate runs only if the running `ExecutionStrategy` **contains** the `approval_gate` step. The `internal`, `testing` and `minimal` presets remove it, so on those the gate never runs no matter what is configured ([PROTOCOL_SPEC §6.6.3.2](../spec/protocol-spec.md#6632-a-configured-layer-is-not-necessarily-an-enforced-one)).
1. Remove `_approval_token` from the inputs, on every path, so it never reaches input validation or the module. A non-string token raises `InvalidInputError` (`GENERAL_INVALID_INPUT`).
2. Compute whether approval is needed from the module's declarations, the execution policy, and the ACL decision for the call. An `ExecutionPolicy` emits `apcore.policy.override` if it changed the effective `requires_approval` or `destructive` annotation. See [Execution Policy](#execution-policy) for the order of evaluation.
3. If approval is not needed, continue to Step 6. (A module whose effective `destructive` is true but that nothing gates is logged as a warning, once per module.)
4. If no `ApprovalHandler` is configured: with `ExecutionPolicy(strict=true)` raise `ApprovalDeniedError` (and emit `apcore.approval.decision` with status `rejected`); otherwise log a warning once per module and continue to Step 6.
5. Build an `ApprovalRequest` from the current call. For a fresh call, use `request_approval(request)`. For Phase B, use `check_approval_for_call(token, request)` or its TypeScript equivalent, `checkApprovalForCall`. The gate rejects a resume when this method is unavailable.
6. Emit `apcore.approval.decision`, then act on `ApprovalResult.status`:
    - `approved` → continue to Step 6;
    - `rejected` → raise `ApprovalDeniedError`;
    - `timeout` → raise `ApprovalTimeoutError`;
    - `pending` → raise `ApprovalPendingError` carrying `approval_id`;
    - any other value → raise `ApprovalDeniedError`.

An exception raised by the handler itself propagates unchanged; it is not converted into a denial.

To observe which parts of the gate are live on an executor, read [`governance_state()`](./core-executor.md#governance-state-api) — `builtin_approval_gate_wired`, `approval_handler_configured` and `policy_strict` are reported separately ([PROTOCOL_SPEC §6.6.5](../spec/protocol-spec.md#665-governance-state-query)). `Executor.validate()` reports the governance-effective `requires_approval` for a call, the same verdict the gate will enforce (§7.9.5).

### Startup Approval Report

After registering modules and configuring the executor, call its startup report to list module IDs that declare `requires_approval` without an effective built-in approval gate.

| SDK | Method |
|---|---|
| Python | `executor.ungated_approval_modules()` |
| TypeScript | `executor.ungatedApprovalModules()` |
| Rust | `executor.ungated_approval_modules()` |

The report includes hidden modules and returns IDs in sorted order. It lists a module if the default strategy omits the gate, the running gate has neither a handler nor strict policy, or the policy waives the module's approval requirement. A wired gate with strict policy protects a call that requires approval by denying it when the handler is absent.

The report reads current configuration without executing modules or calling an approval handler. It describes module-level declarations. Caller-specific ACL rules, call arguments, and per-call strategies are evaluated during invocation. Run the report again if registrations or the executor's governance configuration change.

### Approval Lifecycle State Machine

```text
                         ┌──────────────────────────────────────────────────┐
                         │         caller invokes target module             │
                         │       whose governance union needs approval      │
                         └──────────────────┬───────────────────────────────┘
                                            ▼
                                  ┌─────────────────────┐
               no _approval_token │  request_approval() │ _approval_token in args
                          ┌───────│   (initial entry)   │────────┐
                          │       └─────────────────────┘        │
                          │                                      ▼
                          │                         ┌─────────────────────┐
                          │                         │ scoped resume check │
                          │                         │   (Phase B resume)  │
                          │                         └──────────┬──────────┘
                          ▼                                    ▼
                   ┌────────────────────────────────────────────────────┐
                   │                ApprovalResult.status               │
                   └────┬──────────┬──────────┬──────────┬──────────────┘
                        │          │          │          │
                  approved      rejected    timeout    pending (Phase B only)
                        │          │          │          │
                        ▼          ▼          ▼          ▼
                 ┌──────────┐ ┌─────────┐┌─────────┐┌──────────────────┐
                 │ proceed  │ │  raise  ││  raise  ││  raise           │
                 │ to Step 6│ │ Approval││ Approval││  ApprovalPending │
                 │          │ │ Denied  ││ Timeout ││  Error           │
                 │          │ │ Error   ││ Error   ││  (approval_id)   │
                 └──────────┘ └─────────┘└─────────┘└────────┬─────────┘
                                                              │
                                                              │ caller retries with
                                                              │ _approval_token in
                                                              │ the arguments
                                                              ▼
                                                     (pipeline re-enters from
                                                      Step 1; middleware side
                                                      effects before Step 5
                                                      re-execute on resume)
```

**Status legend:**

| Status     | Result                                          | Phase     | Retryable                        |
|------------|-------------------------------------------------|-----------|----------------------------------|
| `approved` | Pipeline continues to Step 6                    | A and B   | n/a                              |
| `rejected` | `ApprovalDeniedError` (`APPROVAL_DENIED`)       | A and B   | No                               |
| `timeout`  | `ApprovalTimeoutError` (`APPROVAL_TIMEOUT`)     | A and B   | Yes                              |
| `pending`  | `ApprovalPendingError` (`APPROVAL_PENDING`)     | B only    | No — resume with `_approval_token` |

### ApprovalHandler Protocol

=== "Python"
    ```python
    from typing import Protocol

    from apcore import ApprovalRequest, ApprovalResult


    class ApprovalHandler(Protocol):
        async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
            """Request approval for a module invocation. Returns the decision."""
            ...

        async def check_approval(self, approval_id: str) -> ApprovalResult:
            """Query the status of a pending approval."""
            ...


    class ScopedApprovalHandler(ApprovalHandler, Protocol):
        async def check_approval_for_call(self, approval_id: str, request: ApprovalRequest) -> ApprovalResult:
            ...
    ```
=== "TypeScript"
    ```typescript
    import type { ApprovalRequest, ApprovalResult } from "apcore-js";

    interface ApprovalHandler {
        requestApproval(request: ApprovalRequest): Promise<ApprovalResult>;
        checkApproval(approvalId: string): Promise<ApprovalResult>;
        checkApprovalForCall?(approvalId: string, request: ApprovalRequest): Promise<ApprovalResult>;
    }
    ```
=== "Rust"
    ```rust
    use apcore::{ApprovalRequest, ApprovalResult, ModuleError};
    use async_trait::async_trait;

    #[async_trait]
    pub trait ApprovalHandler: Send + Sync + std::fmt::Debug {
        async fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalResult, ModuleError>;
        async fn check_approval(&self, approval_id: &str) -> Result<ApprovalResult, ModuleError>;
        async fn check_approval_for_call(
            &self, _approval_id: &str, _request: &ApprovalRequest,
        ) -> Result<ApprovalResult, ModuleError> {
            Ok(ApprovalResult::rejected("The approval handler does not support scoped resumption"))
        }
    }
    ```

A handler receives an `ApprovalRequest` and returns an `ApprovalResult` whose `status` is the decision. The gate maps statuses to execution outcomes and errors. The handler may wait for a decision or return immediately. All methods are asynchronous. The ID-only method remains available for status queries.

### Data Types

**ApprovalRequest** carries the invocation context to the handler:

| Field | Type | Description |
|-------|------|-------------|
| `module_id` | `str` | Canonical module ID |
| `caller_id` | `str \| None` | `Context.caller_id` of the call, unmodified — `None` for a top-level call (not the `@external` sentinel) |
| `action` | `str` | The module ID being invoked (a flat duplicate of `module_id`) |
| `arguments` | `dict` | Detached business arguments at the gate, before the standard before-middleware and input-validation steps |
| `context` | `Context` | Execution context (trace_id, identity, call_chain); `Option<Context>` in Rust |
| `annotations` | `ModuleAnnotations` | The **effective** annotations: `requires_approval` is always true here and `destructive` is the policy-effective value (§7.9.3) |
| `description` | `str \| None` | Module's human-readable description |
| `tags` | `list[str]` | Module's tags |

**ApprovalResult** carries the handler's decision:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `status` | `str` | Yes | One of: `approved`, `rejected`, `timeout`, `pending` |
| `approved_by` | `str \| None` | No | Identifier of the approver (human, agent, policy) |
| `reason` | `str \| None` | No | Human-readable explanation |
| `approval_id` | `str \| None` | No | Phase B: token for async resume |
| `audit_id` | `str \| None` | No | Non-secret identifier to correlate approval telemetry |
| `metadata` | `dict \| None` | No | Additional metadata from the approval process |

TypeScript builds these with `createApprovalRequest()` / `createApprovalResult()` (frozen objects, camelCase fields). Rust has `ApprovalResult::approved(by)` and `ApprovalResult::rejected(reason)` constructors; both Rust structs are `#[non_exhaustive]`.

The initial request and scoped resume use the same gate-time argument snapshot. Keep `_approval_token` in the invocation's reserved credential field. The executor separates it from business arguments even when the selected strategy omits the gate. Preflight checks business arguments and can report `requires_approval` without consulting an approval handler.

The standard pipeline allows before-middleware to transform inputs after the gate. If the transformation changes the resource or action a caller is being approved to use, normalize before the gate or authorize the transformed values separately.

### Error Types

| Error | Code | When |
|-------|------|------|
| `ApprovalDeniedError` | `APPROVAL_DENIED` | Handler returned `rejected` (or an unknown status), or `strict` policy with no handler |
| `ApprovalTimeoutError` | `APPROVAL_TIMEOUT` | Handler returned `timeout` |
| `ApprovalPendingError` | `APPROVAL_PENDING` | Handler returned `pending` (Phase B); `details.approval_id` carries the resume token |

In Python and TypeScript all three extend `ApprovalError` (a `ModuleError`) and carry the full `ApprovalResult` as `result`. Rust returns a `ModuleError` with the corresponding `ErrorCode` and the approval result in `details.result`. Treat caller-facing approval results as confidential when they carry an `approval_id`.

The status value `"rejected"` maps to error code `APPROVAL_DENIED` — the handler **rejects** a request, the framework reports approval was **denied**.

### Built-in Handlers

| Handler | Behavior | Use Case |
|---------|----------|----------|
| `AlwaysDenyHandler` | Always returns `rejected` | Safe default when approval enforcement is desired without a specific handler |
| `AutoApproveHandler` | Always returns `approved` | Testing and development |
| `CallbackApprovalHandler` | Delegates `request_approval` to a user-provided callback; `check_approval` returns `rejected` | Custom approval logic |

The callback is asynchronous and may fail in every SDK (§7.6.1):

| SDK | Callback |
|---|---|
| apcore-python | `Callable[[ApprovalRequest], Coroutine[Any, Any, ApprovalResult]]` — awaited, may raise |
| apcore-typescript | `(request: ApprovalRequest) => Promise<ApprovalResult>` — may reject |
| apcore-rust | `CallbackApprovalHandler::new(Fn(ApprovalRequest) -> impl Future<Output = Result<ApprovalResult, ModuleError>>)`; `new_sync` takes a synchronous `Fn(&ApprovalRequest) -> Result<ApprovalResult, ModuleError>` for in-process decisions |

A callback error propagates out of the gate as-is; it is not treated as a rejection.

### Protocol Bridge Handlers

Protocol bridges provide their own `ApprovalHandler` implementations that use protocol-native mechanisms:

- **`ElicitationApprovalHandler`** (apcore-mcp) — Uses the MCP elicitation protocol to present an approval prompt to the AI client, which relays it to the human user.
- **`CliApprovalHandler`** (apcore-cli) — Uses an interactive terminal prompt to request confirmation from the user.

These handlers are provided by the respective bridge packages, not by apcore core.

### Phased Implementation

| Phase | Scope | Requirement |
|-------|-------|-------------|
| **Phase A** | Synchronous approval: the handler resolves to a decision | **MUST** implement for conformance |
| **Phase B** | Asynchronous approval: `pending` + `approval_id` + retry with `_approval_token` | **MAY** implement |

### Execution Policy

An `ExecutionPolicy` is an execution-time governance layer attached to the Executor (§7.9). It lets an operator change the governance of already-registered modules without editing them:

- **`rules`** — `PolicyRule`s, each with a module-ID `pattern` (ACL wildcard semantics) and optional overrides for `requires_approval` and `destructive` (unset = keep the module's value), plus a `reason` for the audit trail. The most specific matching rule wins (ACL specificity scoring); on a tie the more restrictive rule wins. A matched override beats the module's own declaration.
- **`gate_destructive`** defaults to true and requires approval for a module whose effective `destructive` is true.
- **`strict`** defaults to true and denies calls requiring approval when no `ApprovalHandler` is configured.

The gate applies these sources in order.

1. Combine the live module's `requires_approval` and `destructive` flags with the corresponding declared metadata flags. A true value from either source counts. The registry can receive those declarations from `*.binding.yaml`, `*_meta.yaml`, `metadata=`, or a Rust `ModuleDescriptor` (D-96, D-125).
2. Resolve the module-scoped requirement using the winning policy rule, if any, then add the requirement from `gate_destructive` when it applies. A policy rule can override a module annotation.
3. Combine that result with the ACL's `approval: required` decision for this invocation. If either requires approval, the gate requires approval. A policy override cannot clear the ACL's requirement.
4. Ask the `ApprovalHandler` when approval is required. The default strict gate denies the call if the handler is absent. An explicit `strict: false` policy warns and continues in that case.

Built-in policy rules match on the module ID. The invocation arguments reach policy resolution for the audit trail after `_approval_token` is stripped (§7.9.6).

A policy is passed programmatically — there are no `policy.*` config keys. To load one from a YAML/JSON document, use `ExecutionPolicy.from_dict()` (Python), `ExecutionPolicy.fromObject()` (TypeScript) or `ExecutionPolicy::from_value()` (Rust); unknown keys and a missing `pattern` are rejected. The document shape is:

```yaml
gate_destructive: true
strict: true
rules:
  - pattern: "orders.delete_*"
    requires_approval: true
    reason: "destructive order operations need human sign-off"
```

=== "Python"
    ```python
    from apcore import APCore, AutoApproveHandler, ExecutionPolicy, PolicyRule

    policy = ExecutionPolicy(
        [PolicyRule("orders.delete_*", requires_approval=True, reason="human sign-off")],
        gate_destructive=True,
        strict=True,
    )

    # APCore applies the policy to the Executor it creates.
    client = APCore(policy=policy)
    client.executor.set_approval_handler(AutoApproveHandler())

    # On an existing Executor: client.executor.set_policy(policy)
    ```
=== "TypeScript"
    ```typescript
    import { APCore, AutoApproveHandler, ExecutionPolicy, PolicyRule } from "apcore-js";

    const policy = new ExecutionPolicy(
        [new PolicyRule("orders.delete_*", { requiresApproval: true, reason: "human sign-off" })],
        { gateDestructive: true, strict: true },
    );

    // APCore applies the policy to the Executor it creates.
    const client = new APCore({ policy });
    client.executor.setApprovalHandler(new AutoApproveHandler());

    // On an existing Executor: client.executor.setPolicy(policy)
    ```
=== "Rust"
    ```rust
    use apcore::{
        APCore, AutoApproveHandler, Config, ExecutionPolicy, Executor, ModuleError, PolicyRule,
        Registry,
    };

    fn main() -> Result<(), ModuleError> {
        let policy = ExecutionPolicy::new(vec![PolicyRule::new("orders.delete_*")?
            .with_requires_approval(true)
            .with_reason("human sign-off")])
        .with_gate_destructive(true)
        .with_strict(true);

        let mut executor = Executor::new(Registry::default(), Config::default());
        executor.set_policy(Some(policy));
        executor.set_approval_handler(Box::new(AutoApproveHandler));

        let _client = APCore::with_options(None, Some(executor), None, None);
        Ok(())
    }
    ```

### Governance Events

When the Executor has an event emitter, the approval gate publishes two events (§9.16.2). Both are best-effort side channels — the call's outcome never depends on their delivery — and neither is emitted during a `validate()` preflight.

| Event | Emitted when | Severity | Payload |
|---|---|---|---|
| `apcore.approval.decision` | The gate adjudicated a call through a handler decision or a fail-closed rejection | `info` for `approved` / `pending`, `warn` for `rejected` / `timeout` | `module_id`, `status`, `approved_by`, `reason`, `approval_id` (non-secret `audit_id` or null), `trace_id` |
| `apcore.policy.override` | A policy rule changed the module's effective `requires_approval` or `destructive` | `info` | `module_id`, `pattern`, `requires_approval`, `destructive`, `needs_approval`, `reason`, `trace_id` |

The emitter is the one given to the Executor: `Executor(registry, event_emitter=emitter)` in Python, `new Executor({ registry, eventEmitter })` in TypeScript, `executor.set_event_emitter(Some(emitter))` in Rust. Subscribe with the emitter's exact event type (see [Event System](./event-system.md)).

## Usage

=== "Python"
    ```python
    from apcore import APCore, ApprovalRequest, ApprovalResult, CallbackApprovalHandler


    async def ask_slack(module_id: str, arguments: dict) -> bool:
        # Replace with a real Slack round-trip.
        return True


    async def my_approver(request: ApprovalRequest) -> ApprovalResult:
        approved = await ask_slack(request.module_id, request.arguments)
        return ApprovalResult(
            status="approved" if approved else "rejected",
            approved_by="slack_user@example.com",
        )


    client = APCore()
    # set_approval_handler() wires the handler into the approval_gate step;
    # assigning an attribute does not.
    client.executor.set_approval_handler(CallbackApprovalHandler(my_approver))


    # A module that requires approval
    @client.module(
        id="data.export",
        description="Export sensitive data",
        annotations={"requires_approval": True},
    )
    def export_data(query: str) -> dict:
        return {"rows": []}


    try:
        result = client.call("data.export", {"query": "SELECT *"})
    except Exception as e:  # ApprovalDeniedError / ApprovalTimeoutError / ApprovalPendingError
        print(f"Not approved: {e}")
    ```
=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import {
        APCore,
        CallbackApprovalHandler,
        createAnnotations,
        createApprovalResult,
    } from "apcore-js";
    import type { ApprovalRequest, ApprovalResult } from "apcore-js";

    async function askSlack(moduleId: string, args: Record<string, unknown>): Promise<boolean> {
        // Replace with a real Slack round-trip.
        return true;
    }

    const client = new APCore();
    // setApprovalHandler() wires the handler into the approval_gate step;
    // assigning a field does not.
    client.executor.setApprovalHandler(new CallbackApprovalHandler(
        async (request: ApprovalRequest): Promise<ApprovalResult> => {
            const approved = await askSlack(request.moduleId, request.arguments);
            return createApprovalResult({
                status: approved ? "approved" : "rejected",
                approvedBy: "slack_user",
            });
        },
    ));

    // A module that requires approval
    client.module({
        id: "data.export",
        description: "Export sensitive data",
        annotations: createAnnotations({ requiresApproval: true }),
        inputSchema: Type.Object({ query: Type.String() }),
        outputSchema: Type.Object({ rows: Type.Array(Type.Unknown()) }),
        execute: () => ({ rows: [] }),
    });

    try {
        const result = await client.call("data.export", { query: "SELECT *" });
    } catch (e) {
        // ApprovalDeniedError / ApprovalTimeoutError / ApprovalPendingError
        console.error("Not approved:", e);
    }
    ```
=== "Rust"
    ```rust
    use apcore::{
        APCore, ApprovalHandler, ApprovalRequest, ApprovalResult, Config, Executor, ModuleError,
        Registry,
    };
    use async_trait::async_trait;

    // The trait requires Debug (`ApprovalHandler: Send + Sync + Debug`), so derive it.
    #[derive(Debug)]
    struct SlackApprovalHandler;

    async fn ask_slack(_module_id: &str, _arguments: &serde_json::Value) -> bool {
        // Replace with a real Slack round-trip.
        true
    }

    #[async_trait]
    impl ApprovalHandler for SlackApprovalHandler {
        async fn request_approval(
            &self,
            request: &ApprovalRequest,
        ) -> Result<ApprovalResult, ModuleError> {
            if ask_slack(&request.module_id, &request.arguments).await {
                Ok(ApprovalResult::approved("slack_user"))
            } else {
                Ok(ApprovalResult::rejected("declined in Slack"))
            }
        }

        async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
            Ok(ApprovalResult::rejected("Phase B not supported by this handler"))
        }
    }

    fn main() {
        // `APCore` exposes only `executor() -> &Executor`, and `set_approval_handler`
        // needs `&mut` — configure the Executor first, then hand it to APCore.
        let mut executor = Executor::new(Registry::default(), Config::default());
        executor.set_approval_handler(Box::new(SlackApprovalHandler));
        let _client = APCore::with_options(None, Some(executor), None, None);
    }
    ```

## Dependencies

- **Executor** — The approval gate is the `approval_gate` step (Step 5) of the pipeline.
- **Module Annotations / Registry** — `requires_approval` and `destructive` from the module and the registry's declared annotations feed the governance union.
- **ACL System** — Step 4 reports whether the matching rule carries `approval: required`.
- **Context** — The execution context (identity, trace_id, call_chain) is passed to the handler via `ApprovalRequest`.
- **Event System** — Receives `apcore.approval.decision` and `apcore.policy.override`.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `approval.py` | `ApprovalHandler` protocol, `ApprovalRequest`, `ApprovalResult`, built-in handlers |
    | `policy.py` | `ExecutionPolicy`, `PolicyRule`, `PolicyDecision` |
    | `builtin_steps.py` | `BuiltinApprovalGate` (Step 5), governance events |
    | `errors.py` | `ApprovalError` and its three subclasses |

## Testing Strategy

- **Union tests** verify that the gate fires when any single governance source requires approval (annotation, declared metadata, ACL `approval: required`, policy rule, `gate_destructive`) and that a `false` in one source never cancels a `true` in another.
- **Default and opt-out tests** cover zero executions for a denied call and the warning from an explicit non-strict policy.
- **Handler tests** confirm each built-in handler returns the expected `ApprovalResult` status.
- **Error mapping tests** verify that `rejected` → `ApprovalDeniedError`, `timeout` → `ApprovalTimeoutError`, `pending` → `ApprovalPendingError`, and that a handler exception propagates unchanged.
- **Token tests** verify that `_approval_token` is removed from the inputs on every path and routed to `check_approval`.
- **Event tests** verify `apcore.approval.decision` / `apcore.policy.override` payloads, and that neither is emitted for a skipped gate or a `validate()` preflight.
- Cross-language cases live in `conformance/fixtures/approval_gate.json`, `approval_request_fields.json` and `acl_argument_scoped_approval.json`.

## Contract: ApprovalHandler.request_approval

### Inputs
- `request` (ApprovalRequest, required) — describes the invocation needing approval; always carries `module_id`, `action` (= `module_id`), `caller_id` (may be null), `arguments`, `context` and effective `annotations`

### Errors
- Any error the handler raises propagates to the caller unchanged. The handler does **not** raise the approval errors; the gate derives them from the returned status.

### Returns
- `ApprovalResult` with `status` one of `approved`, `rejected`, `timeout`, `pending` (Phase B only; set `approval_id`)

### Properties
- async: true (approval may require human interaction or external service call)
- thread_safe: true

## Contract: ApprovalHandler.check_approval_for_call

### Inputs
- `approval_id` (str/string/&str, required) — the token returned in a prior `ApprovalResult.approval_id` for a `pending` decision, taken from the caller's `_approval_token`
- `request` (ApprovalRequest, required) — the current invocation's identity, `caller_id`, target, and arguments with the credential removed

### Errors
- Any error the handler raises propagates to the caller unchanged.

### Returns
- `ApprovalResult` with the current decision: `approved`, `rejected`, `timeout`, or still `pending` (the caller retries later with the same `_approval_token`)
- The gate returns `APPROVAL_DENIED` when the handler does not support scoped resumption.
- A handler that approves a resume verifies the scope and expiry and atomically consumes the credential.

### Properties
- async: true (Phase B resume may itself require a round-trip to an external approval store)
- thread_safe: true
- pure: false — a Phase B handler typically reads (and, on a terminal decision, clears) persisted approval state keyed by `approval_id`
- idempotent: false
