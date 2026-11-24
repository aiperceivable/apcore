"""Human-in-the-loop Approval gate — end-to-end demo.

apcore is a standalone product: this example runs real tool calls through the
execution pipeline, with NO framework integration required. It is the natural
companion to `examples/acl_agent_governance.py` — ACL governs *who* may call a
tool; Approval is the *human-in-the-loop gate* for the sensitive ones. The gate
fires at Executor Step 5, after ACL passes and before middleware runs. See the
spec repo's `docs/features/approval-system.md`.

It

  1. registers a sensitive tool that declares `requires_approval=true`
     (`executor.crm.delete`) and a normal tool that does not (`executor.crm.read`),
  2. wires a `CallbackApprovalHandler` whose decision depends on the request —
     it approves only when the caller passes `confirmed=true`, and rejects
     otherwise (the kind of policy a real reviewer UI would enforce), and
  3. runs scenarios that actually CALL the tools.

A normal tool runs without ever reaching the gate. A sensitive tool with the
handler APPROVING returns its real result; with the handler REJECTING it is
blocked and `client.call(...)` raises `ApprovalDeniedError`.

Each scenario carries its expected outcome, so this script doubles as a smoke
test: it exits non-zero if any decision drifts from the cross-language contract.

Run (from the apcore-python repo root):
    python examples/approval.py
"""

from __future__ import annotations

import sys

from apcore import APCore, Context, Identity
from apcore.approval import ApprovalRequest, ApprovalResult, CallbackApprovalHandler
from apcore.errors import ApprovalDeniedError

# ---------------------------------------------------------------------------
# 1. Register tools. The delete tool is sensitive: it declares
#    requires_approval=true, so the gate intercepts it. The read tool is normal.
# ---------------------------------------------------------------------------
client = APCore()


@client.module(id="executor.crm.read", description="Read a single CRM record")
def crm_read(record_id: str) -> dict:
    return {"record_id": record_id, "name": "Acme Corp", "tier": "gold"}


@client.module(
    id="executor.crm.delete",
    description="Delete a CRM record (sensitive — requires human approval)",
    annotations={"requires_approval": True},
)
def crm_delete(record_id: str, confirmed: bool = False) -> dict:
    # `confirmed` is part of the declared input the handler reviews; the tool
    # itself only runs once the approval gate has passed.
    return {"record_id": record_id, "deleted": True}


# ---------------------------------------------------------------------------
# 2. Wire a request-dependent ApprovalHandler. A real handler would block on a
#    reviewer UI / Slack / e-mail; here the decision is driven by the request so
#    the demo is deterministic: approve only when the caller passed confirmed=true.
# ---------------------------------------------------------------------------
async def review(request: ApprovalRequest) -> ApprovalResult:
    approver = request.context.identity.id if request.context.identity else "anonymous"
    if request.arguments.get("confirmed") is True:
        return ApprovalResult(status="approved", approved_by=approver)
    return ApprovalResult(
        status="rejected",
        approved_by=approver,
        reason="caller did not confirm the destructive operation",
    )


client.executor.set_approval_handler(CallbackApprovalHandler(review))


# ---------------------------------------------------------------------------
# 3. Drive real tool calls.
# ---------------------------------------------------------------------------
def attempt(caller_id: str, target: str, inputs: dict) -> tuple[bool, str]:
    """Call `target` as `caller_id`, returning (executed, detail) for display."""
    ctx: Context = Context.create(identity=Identity(id=caller_id, type="user", roles=("operator",)))
    try:
        return True, repr(client.call(target, inputs, context=ctx))
    except ApprovalDeniedError as exc:
        return False, f"ApprovalDeniedError: {exc.reason}"


# (label, caller_id, target, inputs, expected_executed)
SCENARIOS: list[tuple[str, str, str, dict, bool]] = [
    ("Normal tool — no gate",          "alice", "executor.crm.read",   {"record_id": "C-7"},                       True),
    ("Sensitive — confirmed (approved)", "alice", "executor.crm.delete", {"record_id": "C-7", "confirmed": True},  True),
    ("Sensitive — unconfirmed (rejected)", "alice", "executor.crm.delete", {"record_id": "C-7"},                   False),
    ("Sensitive — confirmed=false (rejected)", "bob", "executor.crm.delete", {"record_id": "C-9", "confirmed": False}, False),
]


def main() -> int:
    print("Human-in-the-loop Approval gate — end-to-end demo")
    print("Real tool calls through APCore; gate fires at Step 5 for requires_approval modules")
    print("=" * 100)
    print(f"  {'RESULT':<8}{'CALLER':<10}{'TARGET':<22}{'OUTCOME':<10}DETAIL")
    print("-" * 100)

    failures = 0
    for label, caller_id, target, inputs, expected in SCENARIOS:
        executed, detail = attempt(caller_id, target, inputs)
        ok = executed == expected
        failures += not ok

        outcome = "EXECUTED" if executed else "BLOCKED"
        mark = "PASS" if ok else "FAIL"
        print(f"  {mark:<8}{caller_id:<10}{target:<22}{outcome:<10}{detail}")
        if not ok:
            print(f"          ^ expected {'EXECUTED' if expected else 'BLOCKED'} — {label}")

    print("=" * 100)
    total = len(SCENARIOS)
    print(f"{total - failures}/{total} decisions match the cross-language contract.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
