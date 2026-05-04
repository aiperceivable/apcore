# Cookbook — Approval-Gated Modules

> **Type:** User cookbook. **Normative spec:** [PROTOCOL_SPEC §7](../../PROTOCOL_SPEC.md#7-approval-system). Feature reference: [features/approval-system.md](../features/approval-system.md).

End-to-end example: a module that requires sign-off before executing, served by a custom `ApprovalHandler`. Covers Phase A (sync block-until-decided) and Phase B (async `pending` → resume via `_approval_token`).

## When to use this pattern

- The module performs an action that needs human or policy review (refunds, deletions, broadcast emails, infra changes).
- You want enforcement at the **framework boundary** so it can't be bypassed by a misconfigured caller.
- You need an audit trail of who approved what.

## When NOT to use this pattern

- For pre-call validation that doesn't need a human: use `module.preflight()` (advisory) or input schema constraints.
- For rate limiting: write a middleware, not an approval handler.
- For "are you allowed to call this at all": that's ACL ([features/acl-system.md](../features/acl-system.md)) — Approval is finer-grained, runs *after* ACL.

---

## 1. Marking the module

The annotation is what triggers the gate. Without `requires_approval=true` the executor skips Step 5 entirely.

=== "Python"
    ```python
    from apcore import APCore
    from apcore.context import Context

    client = APCore()

    @client.module(
        id="finance.refund",
        description="Issue a refund — requires approval",
        annotations={"requires_approval": True},
        input_schema={
            "type": "object",
            "properties": {
                "order_id": {"type": "string"},
                "amount_cents": {"type": "integer", "minimum": 1},
                "reason": {"type": "string"},
            },
            "required": ["order_id", "amount_cents", "reason"],
        },
        output_schema={"type": "object", "properties": {"refund_id": {"type": "string"}}},
    )
    def refund(order_id: str, amount_cents: int, reason: str, context: Context) -> dict:
        # By the time we reach here, an approver has signed off.
        return {"refund_id": gateway.refund(order_id, amount_cents, reason)}
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore, Context } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'finance.refund',
      description: 'Issue a refund — requires approval',
      annotations: { requires_approval: true },
      inputSchema: Type.Object({
        order_id: Type.String(),
        amount_cents: Type.Integer({ minimum: 1 }),
        reason: Type.String(),
      }),
      outputSchema: Type.Object({ refund_id: Type.String() }),
      execute: async (inputs, context: Context) => {
        const refundId = await gateway.refund(
          inputs.order_id as string, inputs.amount_cents as number, inputs.reason as string,
        );
        return { refund_id: refundId };
      },
    });
    ```

=== "Rust"
    ```rust
    use apcore::{APCore, Context, ModuleAnnotations};
    use serde_json::json;

    let mut client = APCore::new();
    client.module()
        .id("finance.refund")
        .description("Issue a refund — requires approval")
        .annotations(ModuleAnnotations::default().with_requires_approval(true))
        .input_schema(json!({
            "type":"object",
            "properties":{"order_id":{"type":"string"},"amount_cents":{"type":"integer","minimum":1},"reason":{"type":"string"}},
            "required":["order_id","amount_cents","reason"]
        }))
        .output_schema(json!({"type":"object","properties":{"refund_id":{"type":"string"}}}))
        .execute(|inputs, _ctx: Context| async move {
            let refund_id = gateway::refund(
                inputs["order_id"].as_str().unwrap(),
                inputs["amount_cents"].as_i64().unwrap(),
                inputs["reason"].as_str().unwrap(),
            ).await?;
            Ok(json!({"refund_id": refund_id}))
        })
        .register();
    ```

## 2. Phase A — sync handler (block until decided)

The simplest case: the handler blocks until a human/policy returns a decision. Use this when sign-off latency is bounded.

=== "Python"
    ```python
    from apcore.approval import ApprovalRequest, ApprovalResult, CallbackApprovalHandler

    def policy_check(req: ApprovalRequest) -> ApprovalResult:
        # Inputs you can branch on:  req.module_id, req.inputs, req.context.identity
        if req.inputs.get("amount_cents", 0) > 100_00:
            # Block on Slack — pseudocode
            verdict = slack.ask_approval(req)
            return ApprovalResult(
                status="approved" if verdict.ok else "rejected",
                approved_by=verdict.user_email,
                reason=verdict.reason,
            )
        return ApprovalResult(status="approved", approved_by="auto:policy", reason="under threshold")

    client.executor.approval_handler = CallbackApprovalHandler(policy_check)

    # Caller side — blocks for as long as the handler takes
    result = client.call("finance.refund", {"order_id": "o-1", "amount_cents": 25000, "reason": "duplicate"})
    ```

=== "TypeScript"
    ```typescript
    import { ApprovalRequest, ApprovalResult, CallbackApprovalHandler } from 'apcore-js';

    const policyCheck = async (req: ApprovalRequest): Promise<ApprovalResult> => {
      if ((req.inputs.amount_cents as number) > 100_00) {
        const verdict = await slack.askApproval(req);
        return {
          status: verdict.ok ? 'approved' : 'rejected',
          approved_by: verdict.userEmail,
          reason: verdict.reason,
        };
      }
      return { status: 'approved', approved_by: 'auto:policy', reason: 'under threshold' };
    };

    client.executor.approvalHandler = new CallbackApprovalHandler(policyCheck);

    const result = await client.call(
      'finance.refund',
      { order_id: 'o-1', amount_cents: 25_000, reason: 'duplicate' },
    );
    ```

=== "Rust"
    ```rust
    use apcore::{ApprovalRequest, ApprovalResult, CallbackApprovalHandler};

    let handler = CallbackApprovalHandler::new(|req: ApprovalRequest| async move {
        let amount = req.inputs["amount_cents"].as_i64().unwrap_or(0);
        if amount > 100_00 {
            let verdict = slack::ask_approval(&req).await?;
            Ok(ApprovalResult {
                status: if verdict.ok { "approved" } else { "rejected" }.into(),
                approved_by: Some(verdict.user_email),
                reason: Some(verdict.reason),
                ..Default::default()
            })
        } else {
            Ok(ApprovalResult { status: "approved".into(), approved_by: Some("auto:policy".into()), ..Default::default() })
        }
    });
    client.executor_mut().set_approval_handler(handler);
    ```

## 3. Phase B — async resume via `_approval_token`

When approval may take minutes/hours (a human must wake up, an external workflow tool needs to fire), Phase B lets the handler return `pending` immediately. The caller catches `APPROVAL_PENDING`, persists `approval_id`, and retries later with `_approval_token` in `arguments`.

=== "Python"
    ```python
    from apcore.errors import ApprovalPendingError

    try:
        result = client.call("finance.refund", {"order_id": "o-1", "amount_cents": 25000, "reason": "duplicate"})
    except ApprovalPendingError as e:
        # Persist e.approval_id so a different process / cron can retry
        save_pending(approval_id=e.approval_id, original_call={"order_id": "o-1", ...})
        return {"status": "queued", "approval_id": e.approval_id}

    # Later — when the workflow tool decides:
    pending = load_pending(approval_id)
    result = client.call(
        "finance.refund",
        {**pending["original_call"], "_approval_token": pending["approval_id"]},
    )
    ```

=== "TypeScript"
    ```typescript
    import { ApprovalPendingError } from 'apcore-js';

    try {
      const result = await client.call('finance.refund', { order_id: 'o-1', amount_cents: 25_000, reason: 'duplicate' });
    } catch (e) {
      if (e instanceof ApprovalPendingError) {
        await savePending({ approvalId: e.approvalId, originalCall: { order_id: 'o-1', /* ... */ } });
        return { status: 'queued', approval_id: e.approvalId };
      }
      throw e;
    }

    // Later
    const pending = await loadPending(approvalId);
    const result = await client.call('finance.refund', {
      ...pending.originalCall,
      _approval_token: pending.approvalId,
    });
    ```

=== "Rust"
    ```rust
    use apcore::ApprovalPendingError;

    match client.call("finance.refund", json!({"order_id":"o-1","amount_cents":25000,"reason":"duplicate"}), None).await {
        Ok(r) => r,
        Err(e) if e.is::<ApprovalPendingError>() => {
            let pe = e.downcast_ref::<ApprovalPendingError>().unwrap();
            save_pending(&pe.approval_id, /* ... */).await?;
            return Ok(json!({"status":"queued","approval_id": pe.approval_id}));
        }
        Err(e) => return Err(e),
    }

    // Later — retry with token
    let mut args = original_call.clone();
    args["_approval_token"] = json!(approval_id);
    client.call("finance.refund", args, None).await?;
    ```

## 4. Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| Forgetting `requires_approval=true` | Module runs without sign-off | Annotation drives the gate; verify with `client.describe(id).annotations` |
| Handler returns `approved` for every request | No actual gating | Audit your handler logic; consider `AlwaysDenyHandler` as default in tests |
| Token is not single-use | Replay attack — same token approves multiple invocations | Bind tokens to `(caller_id, target_id, input_hash)` and consume on first success — see [security-considerations.md §2.3](../spec/security-considerations.md#23-approval-gate-replay-t2) |
| Pre-approval middleware has side effects on resume | Logging/metrics double-emitted on Phase B retry | Inspect `_approval_token` in the middleware and short-circuit; pipeline re-enters from Step 1 — see PROTOCOL_SPEC §7 |
| `APPROVAL_TIMEOUT` retried automatically | Infinite retry loop | Approval timeouts are retryable=Yes by default but a retry will hit the same handler; back off and surface to a human |
| Handler raises an exception instead of returning `rejected` | Caller sees `MODULE_EXECUTE_ERROR`, not `APPROVAL_DENIED` | Always return an `ApprovalResult` — wrap exceptions with `try/except` inside the handler |

## 5. Built-in handlers (when to use which)

| Handler | When | Notes |
|---------|------|-------|
| `AlwaysDenyHandler` | Default for tests; never approves | The framework default — do not rely on this in production |
| `AutoApproveHandler` | Local dev / unit tests where the gate is in the way | Never ship to production |
| `CallbackApprovalHandler(fn)` | Custom policy or human-in-the-loop bridge | What you'll use 90% of the time |
| Bridge handlers (e.g. Slack, PagerDuty) | When the company already has a sign-off tool | Build on top of `CallbackApprovalHandler` |

## 6. Wiring it in `apcore.yaml`

```yaml
# apcore.yaml — extract
approval:
  handler: my_app.approval.PolicyCheckHandler  # dotted path to your handler class
  timeout_ms: 60000                            # cap blocking handlers
```

The handler class must satisfy the `ApprovalHandler` protocol (see [features/approval-system.md](../features/approval-system.md)).

---

## See also

- [features/approval-system.md](../features/approval-system.md) — full state machine and protocol
- [spec/security-considerations.md §2.3](../spec/security-considerations.md#23-approval-gate-replay-t2) — token security
- [conformance fixture `approval_gate`](https://github.com/aiperceivable/apcore/blob/main/conformance/fixtures/approval_gate.json) — 5 behavioural cases
- [PROTOCOL_SPEC §7](../../PROTOCOL_SPEC.md#7-approval-system) — normative spec
