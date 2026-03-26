# Approval System

## Overview

The Approval System provides runtime enforcement of the `requires_approval` annotation. When a module declares `requires_approval=true` and an `ApprovalHandler` is configured on the Executor, the handler is invoked at **Step 5** of the execution pipeline — after ACL checks pass and before input validation begins. This allows human or automated review of sensitive operations before they execute.

The Approval System is architecturally separate from the ACL System. ACL answers "who is allowed to call this module?" while Approval answers "does this particular invocation need sign-off before proceeding?"

See [PROTOCOL_SPEC §7](../../PROTOCOL_SPEC.md#7-approval-system-approval-system) for the full specification.

## Requirements

- Provide a pluggable `ApprovalHandler` protocol that SDK implementations can satisfy with custom logic.
- Enforce the approval gate at Executor Step 5, after ACL (Step 4) and before Input Validation (Step 6).
- Skip the approval gate entirely when no `ApprovalHandler` is configured, or when the module does not declare `requires_approval=true`.
- Support synchronous approval flows (Phase A) where `request_approval()` blocks until a decision is returned.
- Optionally support asynchronous approval flows (Phase B) where a `pending` status is returned with an `approval_id`, and execution resumes when the client retries with an `_approval_token`.
- Raise structured errors (`APPROVAL_DENIED`, `APPROVAL_TIMEOUT`, `APPROVAL_PENDING`) that map cleanly to protocol error codes.
- Ship built-in handlers for common cases: `AlwaysDenyHandler` (safe default), `AutoApproveHandler` (testing), and `CallbackApprovalHandler` (custom function).

## Technical Design

### Approval Gate (Executor Step 5)

The approval gate is inserted between ACL Enforcement (Step 4) and Input Validation (Step 6) in the Executor's pipeline. The algorithm:

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

### ApprovalHandler Protocol

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

## Key Files

| File | Purpose |
|------|---------|
| `approval.py` | `ApprovalHandler` protocol, `ApprovalRequest`, `ApprovalResult`, built-in handlers, approval error classes |
| `executor.py` | Step 5 integration in `call()`, `call_async()`, `stream()` |

## Dependencies

### Internal
- **Executor** — The approval gate is embedded in the Executor pipeline at Step 5.
- **Module Annotations** — The `requires_approval` field on `ModuleAnnotations` (or dict equivalent) triggers the gate.
- **Context** — The full execution context (including identity, trace_id, call_chain) is passed to the handler via `ApprovalRequest`.

## Testing Strategy

- **Unit tests** verify that the approval gate fires only when both an `ApprovalHandler` is configured and the module declares `requires_approval=true`.
- **Dict annotation tests** verify that modules using `@module(annotations={"requires_approval": True})` are correctly gated alongside `ModuleAnnotations` dataclass-based modules.
- **Handler tests** confirm each built-in handler returns the expected `ApprovalResult` status.
- **Error mapping tests** verify that `rejected` → `ApprovalDeniedError`, `timeout` → `ApprovalTimeoutError`, `pending` → `ApprovalPendingError`.
- **Skip tests** confirm the gate is skipped when no handler is set, or when the module does not require approval.
- **Integration tests** run a full pipeline execution with approval handlers to verify end-to-end behavior including error propagation to callers.
- Test naming follows the `test_<unit>_<behavior>` convention.
