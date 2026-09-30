---
description: "Approval gate at Executor Step 5: fires on the governance union, pluggable ApprovalHandler, sync and pending/resume-token flows, built-in handlers, ExecutionPolicy overrides and governance events."
---

# Approval System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §7 Approval System (§7.9 Execution Policy).


## Overview

The Approval System decides whether a particular invocation needs sign-off before it runs, and asks an `ApprovalHandler` for that sign-off at **Step 5** of the execution pipeline — after ACL enforcement (Step 4) and before the Middleware Before Chain (Step 6).

An invocation needs approval when **any** of these governance sources says so (the governance union):

1. the module's own annotation `requires_approval: true`;
2. the registry's declared annotations for the module — a `*.binding.yaml`, `*_meta.yaml` or `metadata=` declaration, or a Rust `ModuleDescriptor` (D-96, D-125);
3. the matching ACL rule carries `approval: required` ([ACL System](./acl-system.md), §6.1.6);
4. an [`ExecutionPolicy`](#execution-policy) makes it required — a rule override of `requires_approval`, or `gate_destructive` with an effective `destructive: true` (§7.9.2).

No source can cancel another: a `false` in one place never removes a `true` from another.

The Approval System is architecturally separate from the ACL System. ACL answers "who is allowed to call this module?" while Approval answers "does this particular invocation need sign-off before proceeding?"

## Requirements

- Provide a pluggable `ApprovalHandler` protocol that SDK implementations can satisfy with custom logic.
- Enforce the approval gate at Executor Step 5, after ACL (Step 4) and before Middleware Before Chain (Step 6).
- Fire the gate on the governance union above.
- When approval is needed but no `ApprovalHandler` is configured, skip the gate with a warning (once per module), or fail closed with `ApprovalDeniedError` when `ExecutionPolicy(strict=true)`.
- Support synchronous approval flows (Phase A) where `request_approval()` resolves to a decision.
- Optionally support asynchronous approval flows (Phase B) where a `pending` status is returned with an `approval_id`, and execution resumes when the client retries with an `_approval_token`.
- Translate the handler's status into structured errors (`APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `APPROVAL_PENDING`).
- Ship built-in handlers for common cases: `AlwaysDenyHandler` (safe default), `AutoApproveHandler` (testing), and `CallbackApprovalHandler` (custom function).

## Technical Design

### Approval Gate (Executor Step 5)

0. The gate runs only if the running `ExecutionStrategy` **contains** the `approval_gate` step. The `internal`, `testing` and `minimal` presets remove it, so on those the gate never runs no matter what is configured ([PROTOCOL_SPEC §6.6.3.2](../spec/protocol-spec.md#6632-a-configured-layer-is-not-necessarily-an-enforced-one)).
1. Remove `_approval_token` from the inputs, on every path, so it never reaches input validation or the module. A non-string token raises `InvalidInputError` (`GENERAL_INVALID_INPUT`).
2. Compute whether approval is needed from the governance union. With an `ExecutionPolicy`, the policy resolves the effective `requires_approval` / `destructive` first and emits `apcore.policy.override` if it changed either.
3. If approval is not needed, continue to Step 6. (A module whose effective `destructive` is true but that nothing gates is logged as a warning, once per module.)
4. If no `ApprovalHandler` is configured: with `ExecutionPolicy(strict=true)` raise `ApprovalDeniedError` (and emit `apcore.approval.decision` with status `rejected`); otherwise log a warning once per module and continue to Step 6.
5. If the inputs carried an `_approval_token`, call `approval_handler.check_approval(token)` (Phase B resume). Otherwise build an `ApprovalRequest` and call `approval_handler.request_approval(request)`.
6. Emit `apcore.approval.decision`, then act on `ApprovalResult.status`:
    - `approved` → continue to Step 6;
    - `rejected` → raise `ApprovalDeniedError`;
    - `timeout` → raise `ApprovalTimeoutError`;
    - `pending` → raise `ApprovalPendingError` carrying `approval_id`;
    - any other value → raise `ApprovalDeniedError`.

An exception raised by the handler itself propagates unchanged; it is not converted into a denial.

To observe which parts of the gate are live on an executor, read [`governance_state()`](./core-executor.md#governance-state-api) — `builtin_approval_gate_wired`, `approval_handler_configured` and `policy_strict` are reported separately ([PROTOCOL_SPEC §6.6.5](../spec/protocol-spec.md#665-governance-state-query)). `Executor.validate()` reports the governance-effective `requires_approval` for a call, the same verdict the gate will enforce (§7.9.5).

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
                          │                         │   check_approval()  │
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
            """Check status of a previously pending approval (Phase B)."""
            ...
    ```
=== "TypeScript"
    ```typescript
    import type { ApprovalRequest, ApprovalResult } from "apcore-js";

    interface ApprovalHandler {
        requestApproval(request: ApprovalRequest): Promise<ApprovalResult>;
        checkApproval(approvalId: string): Promise<ApprovalResult>;
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
    }
    ```

A handler receives an `ApprovalRequest` and **returns** an `ApprovalResult` whose `status` is the decision. It does not raise `ApprovalDeniedError` / `ApprovalTimeoutError` / `ApprovalPendingError` itself — the gate raises those from the returned status. The handler may wait (for human input via UI, Slack, etc.) or return immediately (auto-approve for testing). Both methods are asynchronous in every SDK. A handler that does not implement Phase B returns `rejected` from `check_approval`.

### Data Types

**ApprovalRequest** carries the invocation context to the handler:

| Field | Type | Description |
|-------|------|-------------|
| `module_id` | `str` | Canonical module ID |
| `caller_id` | `str \| None` | `Context.caller_id` of the call, unmodified — `None` for a top-level call (not the `@external` sentinel) |
| `action` | `str` | The module ID being invoked (a flat duplicate of `module_id`) |
| `arguments` | `dict` | Input arguments for the call, with `_approval_token` removed and **not yet schema-validated** (validation is Step 7) |
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
| `metadata` | `dict \| None` | No | Additional metadata from the approval process |

TypeScript builds these with `createApprovalRequest()` / `createApprovalResult()` (frozen objects, camelCase fields). Rust has `ApprovalResult::approved(by)` and `ApprovalResult::rejected(reason)` constructors; both Rust structs are `#[non_exhaustive]`.

### Error Types

| Error | Code | When |
|-------|------|------|
| `ApprovalDeniedError` | `APPROVAL_DENIED` | Handler returned `rejected` (or an unknown status), or `strict` policy with no handler |
| `ApprovalTimeoutError` | `APPROVAL_TIMEOUT` | Handler returned `timeout` |
| `ApprovalPendingError` | `APPROVAL_PENDING` | Handler returned `pending` (Phase B); `details.approval_id` carries the resume token |

In Python and TypeScript all three extend `ApprovalError` (a `ModuleError`) and carry the full `ApprovalResult` as `result`. In Rust they are `ModuleError`s with the corresponding `ErrorCode`.

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
- **`gate_destructive`** — when true, a module whose effective `destructive` is true needs approval even if `requires_approval` is false.
- **`strict`** — when true, a call that needs approval but has no `ApprovalHandler` fails closed with `ApprovalDeniedError` instead of being skipped with a warning.

A policy can only **add** an approval requirement relative to the ACL: the gate ORs the policy verdict with the ACL rule's `approval: required`. Rules match on the module ID only; the call's arguments reach policy resolution (for the audit trail) but no built-in rule consults them, and `_approval_token` is stripped before resolution (§7.9.6).

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
| `apcore.approval.decision` | The gate adjudicated a call (a handler decision, or a `strict` fail-closed rejection); never when the gate is skipped | `info` for `approved` / `pending`, `warn` for `rejected` / `timeout` | `module_id`, `status`, `approved_by`, `reason`, `approval_id`, `trace_id` |
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
- **Skip tests** confirm the gate is skipped (with a warning) when no handler is set, and fails closed when `strict` is set.
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

## Contract: ApprovalHandler.check_approval

### Inputs
- `approval_id` (str/string/&str, required) — the token returned in a prior `ApprovalResult.approval_id` for a `pending` decision, taken from the caller's `_approval_token`

### Errors
- Any error the handler raises propagates to the caller unchanged.

### Returns
- `ApprovalResult` with the current decision: `approved`, `rejected`, `timeout`, or still `pending` (the caller retries later with the same `_approval_token`)
- A handler that does not implement Phase B returns `status="rejected"` rather than raising

### Properties
- async: true (Phase B resume may itself require a round-trip to an external approval store)
- thread_safe: true
- pure: false — a Phase B handler typically reads (and, on a terminal decision, clears) persisted approval state keyed by `approval_id`
- idempotent: false
