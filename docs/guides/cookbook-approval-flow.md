---
description: "Cookbook: approval-gated modules with an ApprovalHandler — decide immediately (Phase A) or return pending and resume later with _approval_token (Phase B)."
---

# Cookbook — Approval-Gated Modules

> **Type:** User cookbook. **Normative spec:** [PROTOCOL_SPEC §7](../spec/protocol-spec.md#7-approval-system). Feature reference: [features/approval-system.md](../features/approval-system.md).

End-to-end recipe: a module that must be signed off before it runs, served by an `ApprovalHandler`. Phase A decides during the call; Phase B returns `pending`, and the caller retries later with `_approval_token`.

## When to use this pattern

- The module does something that needs human or policy review: refunds, deletions, broadcast emails, infrastructure changes.
- You want the check enforced by the executor, where a caller cannot skip it.
- You need an audit trail of who approved what.

## When NOT to use this pattern

- Checks that need no human: use input schema constraints or the module's `preflight()`.
- "May this caller use this module at all?" is an ACL question ([ACL Configuration Guide](./acl-configuration.md)). The ACL check runs before the approval gate.
- Rate limiting belongs in middleware.

---

## 1. Mark the module

`requires_approval` in the module's annotations makes the approval gate stop the call. The gate also fires when an ACL rule says `approval: required` or an `ExecutionPolicy` with `gate_destructive` matches a destructive module — see [features/approval-system.md](../features/approval-system.md).

=== "Python"
    ```python
    from apcore import APCore
    from apcore.context import Context

    client = APCore()


    @client.module(
        id="finance.refund",
        description="Issue a refund (requires approval)",
        annotations={"requires_approval": True},
    )
    def refund(order_id: str, amount_cents: int, reason: str, context: Context) -> dict:
        # Only reached once the approval gate has let the call through.
        return {"refund_id": f"rf_{order_id}"}
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, createAnnotations } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'finance.refund',
      description: 'Issue a refund (requires approval)',
      annotations: createAnnotations({ requiresApproval: true }),
      inputSchema: Type.Object({
        order_id: Type.String(),
        amount_cents: Type.Integer({ minimum: 1 }),
        reason: Type.String(),
      }),
      outputSchema: Type.Object({ refund_id: Type.String() }),
      // Only reached once the approval gate has let the call through.
      execute: async (inputs) => ({ refund_id: `rf_${inputs.order_id as string}` }),
    });
    ```

=== "Rust"
    ```rust
    use apcore::{Context, Module, ModuleAnnotations, ModuleError};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct RefundModule;

    #[async_trait]
    impl Module for RefundModule {
        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "order_id":     {"type": "string"},
                    "amount_cents": {"type": "integer", "minimum": 1},
                    "reason":       {"type": "string"}
                },
                "required": ["order_id", "amount_cents", "reason"]
            })
        }

        fn output_schema(&self) -> Value {
            json!({"type": "object", "properties": {"refund_id": {"type": "string"}}})
        }

        fn description(&self) -> &str {
            "Issue a refund (requires approval)"
        }

        // The approval gate reads the module's own annotations.
        fn annotations(&self) -> ModuleAnnotations {
            ModuleAnnotations {
                requires_approval: true,
                ..Default::default()
            }
        }

        async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
            // Only reached once the approval gate has let the call through.
            let order_id = inputs["order_id"].as_str().unwrap_or_default();
            Ok(json!({"refund_id": format!("rf_{order_id}")}))
        }
    }
    ```

To confirm a module is gated, run a preflight: `validate()` reports the effective requirement as `requires_approval` (`requiresApproval` in TypeScript).

## 2. Phase A — decide during the call

The handler returns `approved` or `rejected` before the call continues. `CallbackApprovalHandler` wraps an async function. This policy approves small refunds and rejects the rest; replace it with a call to your review tool when the answer comes back within the request.

=== "Python"
    ```python
    from apcore.approval import ApprovalRequest, ApprovalResult, CallbackApprovalHandler


    async def refund_policy(request: ApprovalRequest) -> ApprovalResult:
        if request.arguments.get("amount_cents", 0) <= 10_000:
            return ApprovalResult(status="approved", approved_by="policy:small-refund")
        return ApprovalResult(status="rejected", reason="refunds over 100.00 need a reviewer")


    client.executor.set_approval_handler(CallbackApprovalHandler(refund_policy))

    result = client.call("finance.refund", {"order_id": "o-1", "amount_cents": 2_500, "reason": "duplicate"})
    print(result)  # {'refund_id': 'rf_o-1'}
    ```

=== "TypeScript"
    ```typescript
    import { CallbackApprovalHandler, createApprovalResult } from 'apcore-js';
    import type { ApprovalRequest, ApprovalResult } from 'apcore-js';

    const refundPolicy = async (request: ApprovalRequest): Promise<ApprovalResult> => {
      if ((request.arguments.amount_cents as number) <= 10_000) {
        return createApprovalResult({ status: 'approved', approvedBy: 'policy:small-refund' });
      }
      return createApprovalResult({ status: 'rejected', reason: 'refunds over 100.00 need a reviewer' });
    };

    client.executor.setApprovalHandler(new CallbackApprovalHandler(refundPolicy));

    const result = await client.call('finance.refund', { order_id: 'o-1', amount_cents: 2_500, reason: 'duplicate' });
    console.log(result); // { refund_id: 'rf_o-1' }
    ```

=== "Rust"
    ```rust
    use apcore::{APCore, ApprovalResult, CallbackApprovalHandler, Config, Executor, Registry};
    use std::sync::Arc;

    // `APCore::executor()` is read-only, so build the Executor, attach the
    // handler, then hand the Executor to the client.
    fn build_client() -> Result<APCore, ModuleError> {
        let mut executor = Executor::new(Arc::new(Registry::new()), Arc::new(Config::default()));
        executor.set_approval_handler(Box::new(CallbackApprovalHandler::new(|request| async move {
            if request.arguments["amount_cents"].as_i64().unwrap_or(0) <= 10_000 {
                Ok(ApprovalResult::approved("policy:small-refund"))
            } else {
                Ok(ApprovalResult::rejected("refunds over 100.00 need a reviewer"))
            }
        })));
        let client = APCore::with_options(None, Some(executor), None, None);
        client.register("finance.refund", Box::new(RefundModule))?;
        Ok(client)
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = build_client()?;
        let args = json!({"order_id": "o-1", "amount_cents": 2_500, "reason": "duplicate"});
        let result = client.call("finance.refund", args, None, None).await?;
        println!("{result}"); // {"refund_id":"rf_o-1"}
        Ok(())
    }
    ```

A `rejected` result reaches the caller as `APPROVAL_DENIED`; `timeout` as `APPROVAL_TIMEOUT`.

## 3. Phase B — return `pending`, resume later

When sign-off takes minutes or hours, the handler returns `pending` with an `approval_id` straight away. The call fails with `APPROVAL_PENDING` carrying that id. Once a reviewer has decided, the caller repeats the call with the id as `_approval_token` in the inputs; the gate removes the credential and passes the current invocation to the handler's scoped resume method.

Phase B uses a scoped handler. This example keeps short-lived approvals in memory. For resumption across processes or restarts, store the scope, expiry, and decision together, and consume an approved credential in one transaction.

=== "Python"
    ```python
    import json
    import time
    import uuid
    from threading import Lock

    from apcore import ApprovalRequest, ApprovalResult, Context, Identity
    from apcore.errors import ApprovalPendingError


    class ReviewQueueHandler:
        """Queues each request and answers once a reviewer has decided."""

        def __init__(self) -> None:
            self._lock = Lock()
            self._reviews: dict[str, tuple[str, float, ApprovalResult]] = {}

        @staticmethod
        def _scope(request: ApprovalRequest) -> str:
            identity = request.context.identity
            principal = [identity.type, identity.id] if identity else None
            return json.dumps(
                [principal, request.caller_id, request.module_id, request.arguments],
                sort_keys=True,
                separators=(",", ":"),
                allow_nan=False,
            )

        async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
            approval_id = uuid.uuid4().hex
            pending = ApprovalResult(status="pending", approval_id=approval_id)
            with self._lock:
                self._reviews[approval_id] = (self._scope(request), time.monotonic() + 300, pending)
            return pending

        async def check_approval(self, approval_id: str) -> ApprovalResult:
            with self._lock:
                entry = self._reviews.get(approval_id)
                if entry is None:
                    return ApprovalResult(status="rejected")
                return ApprovalResult(status=entry[2].status, approved_by=entry[2].approved_by)

        async def check_approval_for_call(self, approval_id: str, request: ApprovalRequest) -> ApprovalResult:
            scope = self._scope(request)
            with self._lock:
                entry = self._reviews.get(approval_id)
                if entry is None or entry[0] != scope:
                    return ApprovalResult(status="rejected")
                if time.monotonic() >= entry[1]:
                    del self._reviews[approval_id]
                    return ApprovalResult(status="timeout")
                decision = entry[2]
                if decision.status == "approved":
                    del self._reviews[approval_id]
                return decision

        def record(self, approval_id: str, approved: bool, reviewer: str) -> None:
            with self._lock:
                scope, expiry, _ = self._reviews[approval_id]
                self._reviews[approval_id] = (
                    scope, expiry, ApprovalResult(status="approved" if approved else "rejected", approved_by=reviewer)
                )


    queue = ReviewQueueHandler()
    client.executor.set_approval_handler(queue)

    args = {"order_id": "o-2", "amount_cents": 25_000, "reason": "damaged"}
    context = Context.create(identity=Identity(id="requester"))
    try:
        client.call("finance.refund", args, context=context)
    except ApprovalPendingError as error:
        approval_id = error.approval_id
        assert approval_id is not None
    else:
        raise RuntimeError("Expected the refund to await review")

    # Later, after the reviewer has signed off:
    queue.record(approval_id, approved=True, reviewer="alice@example.com")
    result = client.call("finance.refund", {**args, "_approval_token": approval_id}, context=context)
    print(result)  # {'refund_id': 'rf_o-2'}
    ```

=== "TypeScript"
    ```typescript
    import { randomUUID } from 'node:crypto';
    import { isDeepStrictEqual } from 'node:util';
    import { ApprovalPendingError, Context, createApprovalResult, createIdentity } from 'apcore-js';
    import type { ApprovalHandler, ApprovalRequest, ApprovalResult } from 'apcore-js';

    function scopeOf(request: ApprovalRequest) {
      const identity = request.context.identity;
      return {
        principal: identity ? [identity.type, identity.id] : null,
        callerId: request.callerId,
        moduleId: request.moduleId,
        arguments: structuredClone(request.arguments),
      };
    }

    class ReviewQueueHandler implements ApprovalHandler {
      private readonly reviews = new Map<string, {
        scope: ReturnType<typeof scopeOf>;
        expiry: number;
        decision: ApprovalResult;
      }>();

      async requestApproval(request: ApprovalRequest): Promise<ApprovalResult> {
        const approvalId = randomUUID();
        const decision = createApprovalResult({ status: 'pending', approvalId });
        this.reviews.set(approvalId, { scope: scopeOf(request), expiry: Date.now() + 300_000, decision });
        return decision;
      }

      async checkApproval(approvalId: string): Promise<ApprovalResult> {
        const decision = this.reviews.get(approvalId)?.decision;
        return createApprovalResult({ status: decision?.status ?? 'rejected', approvedBy: decision?.approvedBy });
      }

      async checkApprovalForCall(approvalId: string, request: ApprovalRequest): Promise<ApprovalResult> {
        const entry = this.reviews.get(approvalId);
        if (!entry || !isDeepStrictEqual(entry.scope, scopeOf(request))) {
          return createApprovalResult({ status: 'rejected' });
        }
        if (Date.now() >= entry.expiry) {
          this.reviews.delete(approvalId);
          return createApprovalResult({ status: 'timeout' });
        }
        if (entry.decision.status === 'approved') this.reviews.delete(approvalId);
        return entry.decision;
      }

      record(approvalId: string, approved: boolean, reviewer: string): void {
        const entry = this.reviews.get(approvalId);
        if (!entry) throw new Error('No pending review');
        entry.decision = createApprovalResult({ status: approved ? 'approved' : 'rejected', approvedBy: reviewer });
      }
    }

    const queue = new ReviewQueueHandler();
    client.executor.setApprovalHandler(queue);

    const args = { order_id: 'o-2', amount_cents: 25_000, reason: 'damaged' };
    const context = Context.create(createIdentity('requester'));
    let approvalId: string;
    try {
      await client.call('finance.refund', args, context);
      throw new Error('Expected the refund to await review');
    } catch (e) {
      if (!(e instanceof ApprovalPendingError) || e.approvalId === null) throw e;
      approvalId = e.approvalId; // persist it together with args
    }

    // Later, after the reviewer has signed off:
    queue.record(approvalId, true, 'alice@example.com');
    const resumed = await client.call('finance.refund', { ...args, _approval_token: approvalId }, context);
    console.log(resumed); // { refund_id: 'rf_o-2' }
    ```

=== "Rust"
    ```rust
    use apcore::{APCore, ApprovalHandler, ApprovalRequest, ApprovalResult, Config, Context, ErrorCode, Executor, Identity, ModuleError, Registry};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    #[derive(Debug)]
    struct Review {
        scope: Value,
        expiry: Instant,
        decision: ApprovalResult,
    }

    #[derive(Debug, Default)]
    struct ReviewQueueHandler {
        reviews: Mutex<HashMap<String, Review>>,
    }

    fn scope_of(request: &ApprovalRequest) -> Value {
        let identity = request.context.as_ref().and_then(|ctx| ctx.identity.as_ref());
        let principal = identity.map(|identity| [identity.identity_type(), identity.id()]);
        json!([principal, request.caller_id, request.module_id, request.arguments])
    }

    impl ReviewQueueHandler {
        fn record(&self, approval_id: &str, approved: bool, reviewer: &str) {
            let mut reviews = self.reviews.lock().unwrap();
            let entry = reviews.get_mut(approval_id).expect("pending review");
            entry.decision = if approved {
                ApprovalResult::approved(reviewer)
            } else {
                ApprovalResult::rejected(format!("rejected by {reviewer}"))
            };
        }
    }

    #[async_trait]
    impl ApprovalHandler for ReviewQueueHandler {
        async fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalResult, ModuleError> {
            let approval_id = Uuid::new_v4().to_string();
            let mut decision = ApprovalResult::default();
            decision.status = "pending".to_owned();
            decision.approval_id = Some(approval_id.clone());
            self.reviews.lock().unwrap().insert(approval_id, Review {
                scope: scope_of(request),
                expiry: Instant::now() + Duration::from_secs(300),
                decision: decision.clone(),
            });
            Ok(decision)
        }

        async fn check_approval(&self, approval_id: &str) -> Result<ApprovalResult, ModuleError> {
            let reviews = self.reviews.lock().unwrap();
            let Some(entry) = reviews.get(approval_id) else {
                return Ok(ApprovalResult::rejected("No pending review"));
            };
            let mut result = ApprovalResult::default();
            result.status = entry.decision.status.clone();
            Ok(result)
        }

        async fn check_approval_for_call(
            &self, approval_id: &str, request: &ApprovalRequest,
        ) -> Result<ApprovalResult, ModuleError> {
            let mut reviews = self.reviews.lock().unwrap();
            let Some(entry) = reviews.get(approval_id) else {
                return Ok(ApprovalResult::rejected("No pending review"));
            };
            if entry.scope != scope_of(request) {
                return Ok(ApprovalResult::rejected("Review scope differs"));
            }
            if Instant::now() >= entry.expiry {
                reviews.remove(approval_id);
                let mut result = ApprovalResult::default();
                result.status = "timeout".to_owned();
                return Ok(result);
            }
            let result = entry.decision.clone();
            if result.status == "approved" {
                reviews.remove(approval_id);
            }
            Ok(result)
        }
    }

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let queue = Arc::new(ReviewQueueHandler::default());
        let mut executor = Executor::new(Arc::new(Registry::new()), Arc::new(Config::default()));
        executor.set_approval_handler_shared(queue.clone());
        let client = APCore::with_options(None, Some(executor), None, None);
        client.register("finance.refund", Box::new(RefundModule))?;

        let args = json!({"order_id": "o-2", "amount_cents": 25_000, "reason": "damaged"});
        let context = Context::new(Identity::new("requester".into(), "user".into(), vec![], HashMap::new()));
        let approval_id = match client.call("finance.refund", args.clone(), Some(&context), None).await {
            // The approval id travels in the error's details.
            Err(e) if e.code == ErrorCode::ApprovalPending => e.details["approval_id"]
                .as_str()
                .unwrap_or_default()
                .to_string(), // persist it together with args
            Err(e) => return Err(e),
            Ok(_) => unreachable!("the refund is gated"),
        };

        // Later, after the reviewer has signed off:
        queue.record(&approval_id, true, "alice@example.com");
        let mut resume = args.clone();
        resume["_approval_token"] = json!(approval_id);
        let result = client.call("finance.refund", resume, Some(&context), None).await?;
        println!("{result}"); // {"refund_id":"rf_o-2"}
        Ok(())
    }
    ```

The Rust tab replaces the Phase A `main` and `build_client`; the Python and TypeScript tabs continue the Phase A file.

## 4. What happens when no handler is attached

With no `ApprovalHandler`, the gate is **skipped** with a warning and the module runs unapproved. To fail closed instead, attach an `ExecutionPolicy` with `strict` enabled — gated calls are then denied with `APPROVAL_DENIED` until a handler is attached:

=== "Python"
    ```python
    from apcore import APCore, ExecutionPolicy

    client = APCore(policy=ExecutionPolicy(strict=True))
    ```

=== "TypeScript"
    ```typescript
    import { APCore, ExecutionPolicy } from 'apcore-js';

    // ExecutionPolicy(rules, options)
const client = new APCore({ policy: new ExecutionPolicy(null, { strict: true }) });
    ```

=== "Rust"
    ```rust
    use apcore::{Config, Executor, ExecutionPolicy, Registry};
    use std::sync::Arc;

    fn strict_executor() -> Executor {
        let mut executor = Executor::new(Arc::new(Registry::new()), Arc::new(Config::default()));
        executor.set_policy(Some(ExecutionPolicy::default().with_strict(true)));
        executor // hand to APCore::with_options(None, Some(executor), None, None)
    }
    ```

The approval handler is always attached in code; there is no `apcore.yaml` key for it.

### Deploying a scoped approval handler

For deployments with pending approvals, deploy a handler that supports scoped resumption before routing resume requests to the new runtime. Plan the cutover with any service that shares approval records.

1. Resume existing records only when the approval store can establish the original trusted principal when present, `caller_id`, canonical target ID, arguments and expiry. Keep credential verification and consumption in one atomic operation.
2. Expire records that lack this information and ask the requester to submit a new approval request. Preserve the audit record and avoid replaying the business action during migration.
3. In a staging deployment using the same store and adapter as production, create a pending request before the cutover. Confirm it resumes once after cutover when the original scope is available. Confirm that expired records and records missing scope require a new request. Use a module with a harmless, observable side effect for this check.

## 5. Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| No handler attached | Warning at call time; the module runs without sign-off | Attach a handler, or use `ExecutionPolicy(strict=True)` to fail closed |
| Using `CallbackApprovalHandler` for Phase B | Every resume fails with `APPROVAL_DENIED` ("Phase B not supported") | Implement `check_approval_for_call()` in your handler (section 3) |
| Handler raises instead of returning `rejected` | The caller gets the raised error (a non-apcore exception arrives as `MODULE_EXECUTE_ERROR`), not `APPROVAL_DENIED` | Return an `ApprovalResult` with `status="rejected"` for a refusal; let only genuine failures raise |
| Reusable tokens | One sign-off approves many calls | Consume the credential in `check_approval_for_call()` (section 3); see [security-considerations.md §2.3](../spec/security-considerations.md#23-approval-gate-replay-t2) |
| Resuming re-runs the early pipeline | ACL audit entries and approval events appear twice for one logical call | Expected: a resume starts again at step 1 — context creation, call-chain guard, lookup, ACL, approval. Middleware `before` hooks run once, on the call that gets through |
| Retrying `APPROVAL_TIMEOUT` in a tight loop | Repeated prompts to the same reviewer | Back off, or turn the timeout into a Phase B `pending` |

## 6. Built-in handlers

| Handler | Use |
|---------|-----|
| `CallbackApprovalHandler(fn)` | Phase A decisions from an async function |
| `AutoApproveHandler` | Tests and local development only — approves everything |
| `AlwaysDenyHandler` | Tests, or to block every gated module explicitly |

---

## See also

- [features/approval-system.md](../features/approval-system.md) — approval states, request fields and handler protocol
- [spec/security-considerations.md §2.3](../spec/security-considerations.md#23-approval-gate-replay-t2) — token replay
- [PROTOCOL_SPEC §7](../spec/protocol-spec.md#7-approval-system) — normative approval rules
