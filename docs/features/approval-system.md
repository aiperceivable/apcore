---
description: "Approval gate at Executor Step 5 for requires_approval modules: pluggable ApprovalHandler, sync and async pending/resume-token flows, built-in deny/auto/callback handlers."
---

# Approval System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §7 Approval System.


## Overview

The Approval System provides runtime enforcement of the `requires_approval` annotation. When a module declares `requires_approval=true` and an `ApprovalHandler` is configured on the Executor, the handler is invoked at **Step 5** of the execution pipeline — after ACL checks pass and before the Middleware Before Chain begins. This allows human or automated review of sensitive operations before they execute.

The Approval System is architecturally separate from the ACL System. ACL answers "who is allowed to call this module?" while Approval answers "does this particular invocation need sign-off before proceeding?"

See [PROTOCOL_SPEC §7](../spec/protocol-spec.md#7-approval-system) for the full specification.

## Requirements

- Provide a pluggable `ApprovalHandler` protocol that SDK implementations can satisfy with custom logic.
- Enforce the approval gate at Executor Step 5, after ACL (Step 4) and before Middleware Before Chain (Step 6).
- Skip the approval gate entirely when no `ApprovalHandler` is configured, or when the module does not declare `requires_approval=true`.
- Support synchronous approval flows (Phase A) where `request_approval()` blocks until a decision is returned.
- Optionally support asynchronous approval flows (Phase B) where a `pending` status is returned with an `approval_id`, and execution resumes when the client retries with an `_approval_token`.
- Raise structured errors (`APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `APPROVAL_PENDING`) that map cleanly to protocol error codes.
- Ship built-in handlers for common cases: `AlwaysDenyHandler` (safe default), `AutoApproveHandler` (testing), and `CallbackApprovalHandler` (custom function).

## Technical Design

### Approval Gate (Executor Step 5)

The approval gate is inserted between ACL Enforcement (Step 4) and Middleware Before Chain (Step 6) in the Executor's pipeline. The algorithm:

1. Check if `approval_handler` is configured on the Executor.
2. If not configured, skip to Step 6.
3. Check if the target module declares `requires_approval=true` in its annotations.
4. If not, skip to Step 6.
5. If arguments contain `_approval_token`, pop the token and call `approval_handler.check_approval(token)` (Phase B resume). Otherwise build an `ApprovalRequest` and call `approval_handler.request_approval(request)`.
6. If `approved` → proceed to Step 6.
7. If `rejected` → raise `ApprovalDeniedError`.
8. If `timeout` → raise `ApprovalTimeoutError`.
9. If `pending` (Phase B only) → raise `ApprovalPendingError` with `approval_id`.

**Annotation access note:** Modules created via `@module(annotations={"requires_approval": True})` store annotations as a `dict`, not a `ModuleAnnotations` dataclass. Implementations **must** handle both forms when checking `requires_approval`.

### Approval Lifecycle State Machine

```
                         ┌──────────────────────────────────────────────────┐
                         │         caller_id invokes target module          │
                         │            with requires_approval=true            │
                         └──────────────────┬───────────────────────────────┘
                                            ▼
                                  ┌─────────────────────┐
                  no _approval_token │   request_approval()│ _approval_token in args
                          ┌────────│   (initial entry)   │────────┐
                          │         └─────────────────────┘         │
                          │                                          ▼
                          │                             ┌─────────────────────┐
                          │                             │   check_approval()  │
                          │                             │   (Phase B resume)  │
                          │                             └──────────┬──────────┘
                          │                                        │
                          ▼                                        ▼
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
                 │ (Middle  │ │ Denied  ││ Timeout ││  Error           │
                 │ ware     │ │ Error   ││ Error   ││  (caller_id retries
                 │ Before   │ │ APPROVAL││ APPROVAL││   later with     │
                 │ Chain)   │ │ _DENIED ││ _TIMEOUT││   _approval_token)
                 └──────────┘ └─────────┘└─────────┘└────────┬─────────┘
                                                              │
                                                              │ caller_id retries
                                                              │ with _approval_token
                                                              │ in arguments
                                                              ▼
                                                     (loop back to top —
                                                      pipeline re-enters
                                                      from Step 1; pre-approval
                                                      middleware side effects
                                                      re-execute on resume,
                                                      see PROTOCOL_SPEC §7)
```

**Status legend:**

| Status     | Result                                          | Phase     | Retryable                        |
|------------|-------------------------------------------------|-----------|----------------------------------|
| `approved` | Pipeline continues to Step 6                    | A and B   | n/a                              |
| `rejected` | `ApprovalDeniedError` (`APPROVAL_DENIED`)       | A and B   | No                               |
| `timeout`  | `ApprovalTimeoutError` (`APPROVAL_TIMEOUT`)     | A and B   | Yes (handler-defined)            |
| `pending`  | `ApprovalPendingError` (`APPROVAL_PENDING`)     | B only    | Yes via `_approval_token` retry  |

### ApprovalHandler Protocol

=== "Python"
    ```python
    class ApprovalHandler(Protocol):
        async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
            """Request approval for a module invocation. Returns the decision."""
            ...

        async def check_approval(self, approval_id: str) -> ApprovalResult:
            """Check status of a previously pending approval (Phase B).
            Default implementation SHOULD return rejected."""
            ...
    ```
=== "TypeScript"
    ```typescript
    interface ApprovalHandler {
        requestApproval(request: ApprovalRequest): Promise<ApprovalResult>;
        checkApproval(approvalId: string): Promise<ApprovalResult>;
    }
    ```
=== "Rust"
    ```rust
    #[async_trait]
    pub trait ApprovalHandler: Send + Sync {
        async fn request_approval(&self, request: ApprovalRequest) -> Result<ApprovalResult, ModuleError>;
        async fn check_approval(&self, approval_id: &str) -> Result<ApprovalResult, ModuleError>;
    }
    ```

Implementations receive an `ApprovalRequest` and return an `ApprovalResult`. The handler may block (waiting for human input via UI, Slack, etc.) or return immediately (auto-approve for testing).

Both methods are asynchronous. In synchronous `call()` paths, the Executor bridges to async using its existing `_run_async_in_sync()` pattern (thread-based event loop bridge).

### Data Types

**ApprovalRequest** carries the invocation context to the handler:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `module_id` | `str` | Yes | Canonical module ID |
| `arguments` | `dict` | Yes | Input arguments for the call |
| `context` | `Context` | Yes | Execution context (trace_id, identity, call_chain) |
| `annotations` | `ModuleAnnotations` | Yes | Full annotation set of the module (`requires_approval` is guaranteed true) |
| `description` | `str \| None` | No | Module's human-readable description |
| `tags` | `list[str]` | No | Module's tags |

**ApprovalResult** carries the handler's decision:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `status` | `str` | Yes | One of: `approved`, `rejected`, `timeout`, `pending` |
| `approved_by` | `str \| None` | No | Identifier of the approver (human, agent, policy) |
| `reason` | `str \| None` | No | Human-readable explanation |
| `approval_id` | `str \| None` | No | Phase B: token for async resume |
| `metadata` | `dict \| None` | No | Additional metadata from the approval process |

### Error Types

| Error | Code | HTTP | When |
|-------|------|------|------|
| `ApprovalDeniedError` | `APPROVAL_DENIED` | 403 | Handler returns `status: "rejected"` |
| `ApprovalTimeoutError` | `APPROVAL_TIMEOUT` | 408 | Handler returns `status: "timeout"` |
| `ApprovalPendingError` | `APPROVAL_PENDING` | 202 | Handler returns `status: "pending"` (Phase B) |

All three extend the base `ApprovalError`, which extends `ModuleError`. Each error carries the full `ApprovalResult` as `self.result`.

Note: The status value `"rejected"` maps to error code `APPROVAL_DENIED` — the handler **rejects** a request, the framework reports approval was **denied**.

### Built-in Handlers

| Handler | Behavior | Use Case |
|---------|----------|----------|
| `AlwaysDenyHandler` | Always returns `rejected` | Safe default when approval enforcement is desired without a specific handler |
| `AutoApproveHandler` | Always returns `approved` | Testing and development |
| `CallbackApprovalHandler` | Delegates to a user-provided async callback | Custom approval logic |

### Protocol Bridge Handlers

Protocol bridges (apcore-mcp, apcore-a2a, apcore-cli) provide their own `ApprovalHandler` implementations that use protocol-native mechanisms:

- **`ElicitationApprovalHandler`** (apcore-mcp) — Uses the MCP elicitation protocol to present an approval prompt to the AI client, which relays it to the human user.
- **`A2AApprovalHandler`** (apcore-a2a, future) — Uses the A2A protocol interaction to request confirmation from the calling agent.
- **`CliApprovalHandler`** (apcore-cli) — Uses an interactive terminal prompt to request confirmation from the user.

These handlers are provided by the respective bridge packages, not by apcore core.

### Phased Implementation

| Phase | Scope | Requirement |
|-------|-------|-------------|
| **Phase A** | Synchronous approval: handler blocks until decision | **MUST** implement for conformance |
| **Phase B** | Asynchronous approval: `pending` + `approval_id` + retry with `_approval_token` | **MAY** implement |

## Usage

=== "Python"
    ```python
    from apcore import APCore
    from apcore.approval import (
        ApprovalHandler,
        ApprovalRequest,
        ApprovalResult,
        AutoApproveHandler,
        CallbackApprovalHandler,
    )

    # Use the built-in auto-approve handler (for testing).
    # Use set_approval_handler(): it propagates the handler to the pipeline's
    # approval_gate step. Plain attribute assignment does NOT wire the gate.
    client = APCore()
    client.executor.set_approval_handler(AutoApproveHandler())

    # Register a module that requires approval
    @client.module(
        id="data.export",
        description="Export sensitive data",
        annotations={"requires_approval": True},
    )
    def export_data(query: str) -> dict:
        return {"rows": []}

    # Custom approval handler via callback
    async def my_approver(request: ApprovalRequest) -> ApprovalResult:
        # Check with human via Slack, email, etc.
        approved = await ask_slack(request.module_id, request.arguments)
        return ApprovalResult(
            status="approved" if approved else "rejected",
            approved_by="slack_user@example.com",
        )

    client.executor.set_approval_handler(CallbackApprovalHandler(my_approver))

    # Calling a requires_approval module
    try:
        result = client.call("data.export", {"query": "SELECT *"})
    except Exception as e:
        print(f"Approval denied: {e}")
    ```
=== "TypeScript"
    ```typescript
    import { APCore } from "apcore-js";
    import {
        AutoApproveHandler,
        CallbackApprovalHandler,
        ApprovalRequest,
        ApprovalResult,
    } from "apcore-js/approval";

    // Use the built-in auto-approve handler (for testing).
    // Use setApprovalHandler(): it propagates the handler to the pipeline's
    // approval_gate step. Plain field assignment does NOT wire the gate.
    const client = new APCore();
    client.executor.setApprovalHandler(new AutoApproveHandler());

    // Register a module that requires approval
    client.module({
        id: "data.export",
        description: "Export sensitive data",
        annotations: { requiresApproval: true },
        inputSchema: { type: "object", properties: { query: { type: "string" } } },
        outputSchema: { type: "object", properties: { rows: { type: "array" } } },
        execute: ({ query }: { query: string }) => ({ rows: [] }),
    });

    // Custom approval handler via callback
    client.executor.setApprovalHandler(new CallbackApprovalHandler(
        async (request: ApprovalRequest): Promise<ApprovalResult> => {
            const approved = await askSlack(request.moduleId, request.arguments);
            return { status: approved ? "approved" : "rejected", approvedBy: "slack_user" };
        }
    ));

    // Calling a requires_approval module
    try {
        const result = await client.call("data.export", { query: "SELECT *" });
    } catch (e) {
        console.error("Approval denied:", e);
    }
    ```
=== "Rust"
    ```rust
    use apcore::APCore;
    use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult, AutoApproveHandler};
    use apcore::errors::ModuleError;
    use async_trait::async_trait;

    // Use the built-in auto-approve handler (for testing)
    let mut client = APCore::new();
    client.executor_mut().set_approval_handler(Box::new(AutoApproveHandler));

    // Custom approval handler
    struct SlackApprovalHandler;

    #[async_trait]
    impl ApprovalHandler for SlackApprovalHandler {
        async fn request_approval(
            &self,
            request: ApprovalRequest,
        ) -> Result<ApprovalResult, ModuleError> {
            // Ask human via Slack
            let approved = ask_slack(&request.module_id, &request.arguments).await;
            Ok(ApprovalResult {
                status: if approved { "approved" } else { "rejected" }.to_string(),
                approved_by: Some("slack_user".to_string()),
                reason: None,
                approval_id: None,
                metadata: None,
            })
        }

        async fn check_approval(
            &self,
            _approval_id: &str,
        ) -> Result<ApprovalResult, ModuleError> {
            Ok(ApprovalResult { status: "rejected".to_string(), ..Default::default() })
        }
    }

    client.executor_mut().set_approval_handler(Box::new(SlackApprovalHandler));
    ```

## Dependencies

- **Executor** — The approval gate is embedded in the Executor pipeline at Step 5.
- **Module Annotations** — The `requires_approval` field on `ModuleAnnotations` (or dict equivalent) triggers the gate.
- **Context** — The full execution context (including identity, trace_id, call_chain) is passed to the handler via `ApprovalRequest`.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `approval.py` | `ApprovalHandler` protocol, `ApprovalRequest`, `ApprovalResult`, built-in handlers, approval error classes |
    | `executor.py` | Step 5 integration in `call()`, `call_async()`, `stream()` |

## Testing Strategy

- **Unit tests** verify that the approval gate fires only when both an `ApprovalHandler` is configured and the module declares `requires_approval=true`.
- **Dict annotation tests** verify that modules using `@module(annotations={"requires_approval": True})` are correctly gated alongside `ModuleAnnotations` dataclass-based modules.
- **Handler tests** confirm each built-in handler returns the expected `ApprovalResult` status.
- **Error mapping tests** verify that `rejected` → `ApprovalDeniedError`, `timeout` → `ApprovalTimeoutError`, `pending` → `ApprovalPendingError`.
- **Skip tests** confirm the gate is skipped when no handler is set, or when the module does not require approval.
- **Integration tests** run a full pipeline execution with approval handlers to verify end-to-end behavior including error propagation to callers.
- Test naming follows the `test_<unit>_<behavior>` convention.

## Contract: ApprovalHandler.request_approval

### Inputs
- `request` (ApprovalRequest, required) — describes the action requiring approval; MUST contain `module_id`, `caller_id`, and `action`

### Errors
- `ApprovalDeniedError(code=APPROVAL_DENIED)` — approval was explicitly denied by the handler
- `ApprovalTimeoutError(code=APPROVAL_TIMEOUT)` — approval was not received within the deadline
- `ApprovalPendingError(code=APPROVAL_PENDING)` — approval is deferred to an async out-of-band process

### Returns
- On success (approved): `ApprovalResult` with `approved=true`

### Properties
- async: true (approval may require human interaction or external service call)
- thread_safe: true
- pure: false (may emit notifications or persist state)
- idempotent: false
