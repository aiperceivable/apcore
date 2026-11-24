"""Execution-time governance policy — end-to-end demo (apcore#76 RFC pilot).

apcore is a standalone product: this example runs real tool calls through the
execution pipeline, with NO framework integration required. It is the natural
companion to `examples/approval.py` — that demo gates a module that *declares*
`requires_approval=true`; here a platform operator applies an **external**
`ExecutionPolicy` that overrides the governance of already-registered modules
at execution time, without touching their code or annotations.

It demonstrates the three #76 pillars:

  1. External override — `orders.delete_order` ships with NO `requires_approval`,
     yet a policy rule forces it through the approval gate.
  2. destructive -> approval — `orders.purge` is annotated `destructive=true`
     but ungated; `gate_destructive=True` makes destructive imply approval.
  3. Fail loud, not silent — with `strict=True`, a module that needs approval
     but has no handler configured fails CLOSED (ApprovalDeniedError) instead
     of silently executing.

Each scenario carries its expected outcome, so this script doubles as a smoke
test: it exits non-zero if any decision drifts from the intended semantics.

Run (from the apcore-python repo root):
    python examples/execution_policy.py
"""

from __future__ import annotations

import sys

from apcore import APCore, Context, ExecutionPolicy, Identity, PolicyRule
from apcore.approval import ApprovalRequest, ApprovalResult, CallbackApprovalHandler
from apcore.errors import ApprovalDeniedError

# ---------------------------------------------------------------------------
# 1. A declarative governance policy, as an operator would write it in YAML and
#    load via ExecutionPolicy.from_dict(). Here we build it directly.
#
#    - Force approval on every delete, regardless of the module's own annotations.
#    - gate_destructive: any module annotated destructive=true also needs approval.
#    - strict: if a gated module has no ApprovalHandler, FAIL CLOSED (deny),
#      never silently execute.
# ---------------------------------------------------------------------------
policy = ExecutionPolicy(
    [PolicyRule("orders.delete_*", requires_approval=True, reason="destructive order ops need human sign-off")],
    gate_destructive=True,
    strict=True,
)

# Equivalent declarative form (what an operator ships in a governance file):
#   policy = ExecutionPolicy.from_dict(yaml.safe_load(open("governance.yaml")))
#   # governance.yaml:
#   #   gate_destructive: true
#   #   strict: true
#   #   rules:
#   #     - pattern: "orders.delete_*"
#   #       requires_approval: true
#   #       reason: "destructive order ops need human sign-off"


# ---------------------------------------------------------------------------
# 2. Register tools that are NAIVE about governance — none declares
#    requires_approval. The policy governs them from the outside.
# ---------------------------------------------------------------------------
client = APCore(policy=policy)


@client.module(id="orders.list_orders", description="List orders (read-only)")
def list_orders() -> dict:
    return {"orders": [1, 2, 3]}


@client.module(id="orders.delete_order", description="Delete an order")
def delete_order(order_id: int, confirmed: bool = False) -> dict:
    # `confirmed` is declared input the reviewer inspects; the tool runs only
    # once the policy-driven approval gate has passed.
    return {"deleted": order_id}


@client.module(
    id="orders.purge",
    description="Purge soft-deleted orders",
    annotations={"destructive": True},  # destructive but NOT requires_approval
)
def purge(confirmed: bool = False) -> dict:
    return {"purged": 7}


# ---------------------------------------------------------------------------
# 3. A reviewer that approves only when the caller passed confirmed=true.
# ---------------------------------------------------------------------------
async def review(request: ApprovalRequest) -> ApprovalResult:
    approver = request.context.identity.id if request.context.identity else "anonymous"
    if request.arguments.get("confirmed") is True:
        return ApprovalResult(status="approved", approved_by=approver)
    return ApprovalResult(status="rejected", approved_by=approver, reason="caller did not confirm")


def attempt(target: str, inputs: dict, *, with_handler: bool) -> tuple[bool, str]:
    """Call `target`, optionally with the approval handler wired, for display."""
    # set_policy / set_approval_handler are runtime setters — here we swap the
    # handler in/out to contrast strict fail-closed (no handler) vs. review.
    if with_handler:
        client.executor.set_approval_handler(CallbackApprovalHandler(review))
    else:
        client.executor.set_approval_handler(None)  # type: ignore[arg-type]
    ctx: Context = Context.create(identity=Identity(id="alice", type="user", roles=("operator",)))
    try:
        return True, repr(client.call(target, inputs, context=ctx))
    except ApprovalDeniedError as exc:
        return False, f"ApprovalDeniedError: {exc.reason}"


# (label, target, inputs, with_handler, expected_executed)
SCENARIOS: list[tuple[str, str, dict, bool, bool]] = [
    ("Read — no gate",                         "orders.list_orders", {},                          True,  True),
    ("Policy forces approval — confirmed",     "orders.delete_order", {"order_id": 1, "confirmed": True}, True, True),
    ("Policy forces approval — unconfirmed",   "orders.delete_order", {"order_id": 1},            True,  False),
    ("gate_destructive — confirmed",           "orders.purge",       {"confirmed": True},         True,  True),
    ("strict fail-closed — no handler",        "orders.delete_order", {"order_id": 1},            False, False),
]


def main() -> int:
    print("Execution-time governance policy — end-to-end demo (apcore#76)")
    print("External ExecutionPolicy overrides governance of naive, already-registered modules")
    print("=" * 104)
    print(f"  {'RESULT':<8}{'TARGET':<22}{'OUTCOME':<10}DETAIL")
    print("-" * 104)

    failures = 0
    for label, target, inputs, with_handler, expected in SCENARIOS:
        executed, detail = attempt(target, inputs, with_handler=with_handler)
        ok = executed == expected
        failures += not ok

        outcome = "EXECUTED" if executed else "BLOCKED"
        mark = "PASS" if ok else "FAIL"
        print(f"  {mark:<8}{target:<22}{outcome:<10}{detail}")
        if not ok:
            print(f"          ^ expected {'EXECUTED' if expected else 'BLOCKED'} — {label}")

    print("=" * 104)
    total = len(SCENARIOS)
    print(f"{total - failures}/{total} decisions match the intended policy semantics.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
