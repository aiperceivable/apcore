"""AI Agent Tool-Governance ACL — end-to-end demo (issue #72).

apcore is a standalone product: this example governs real tool calls through the
execution pipeline, with NO framework integration required. It

  1. registers real `executor.*` / `data.*` tools,
  2. wires the canonical default-deny agent-governance ACL into an `APCore`
     instance (the same policy as the spec repo's
     `examples/acl/agent-tool-governance.yaml`, locked cross-language by
     `conformance/fixtures/acl_agent_scoping.json`),
  3. has agents of different roles actually CALL the tools, and
  4. prints the resulting audit trail.

Allowed calls return real results; denied calls raise `ACLDeniedError`. The two
conditions that make ACL valuable for per-agent tool governance:
  - `roles`          — set-intersection against the caller's Identity.
  - `max_call_depth` — fuses runaway tool chains (the pipeline measures the
                       caller's live call-chain length, so a deeper chain trips
                       the fuse even for the same role).

Each scenario carries its expected outcome, so this script doubles as a smoke
test: it exits non-zero if any decision drifts from the cross-language contract.

Run (from the apcore-python repo root):
    python examples/acl_agent_governance.py
"""

from __future__ import annotations

import sys

from apcore import ACL, APCore, ACLDeniedError, ACLRule, Context, Identity
from apcore.acl import AuditEntry

# ---------------------------------------------------------------------------
# 1. Register real tools (executor / data modules) on an APCore instance.
# ---------------------------------------------------------------------------
client = APCore()


@client.module(id="executor.crm.read", description="Read a single CRM record")
def crm_read(record_id: str) -> dict:
    return {"record_id": record_id, "name": "Acme Corp", "tier": "gold"}


@client.module(id="executor.crm.query", description="Query CRM records by filter")
def crm_query(filter: str) -> dict:
    return {"filter": filter, "matched": 3}


@client.module(id="executor.crm.delete", description="Delete a CRM record")
def crm_delete(record_id: str) -> dict:
    return {"record_id": record_id, "deleted": True}


@client.module(id="data.export", description="Export a dataset to cold storage")
def data_export(dataset: str) -> dict:
    return {"dataset": dataset, "rows": 1280}


# ---------------------------------------------------------------------------
# 2. The canonical agent-tool-governance policy (default-deny), with an audit
#    logger that captures every decision, then wire it into the executor.
# ---------------------------------------------------------------------------
RULES = [
    # External / unauthenticated callers (no caller_id) — read-only.
    ACLRule(callers=["@external"], targets=["executor.*.read"], effect="allow"),
    # Reader-role agents — read + query, depth-capped to fuse runaway chains.
    ACLRule(
        callers=["agent.*"],
        targets=["executor.*.read", "executor.*.query"],
        effect="allow",
        conditions={"roles": ["reader"], "max_call_depth": 3},
    ),
    # Data-admin agents — exports and sensitive deletes (no depth cap).
    ACLRule(
        callers=["agent.*"],
        targets=["data.export", "executor.*.delete"],
        effect="allow",
        conditions={"roles": ["data_admin"]},
    ),
]

audit_trail: list[AuditEntry] = []
acl = ACL(rules=RULES, default_effect="deny", audit_logger=audit_trail.append)
client.executor.set_acl(acl)


# ---------------------------------------------------------------------------
# 3. Drive real tool calls as different agents.
# ---------------------------------------------------------------------------
def attempt(
    caller_id: str | None, roles: list[str] | None, upstream_hops: int, target: str, inputs: dict
) -> tuple[bool, object]:
    """Call `target` as `caller_id`, returning (allowed, result_or_error).

    The pipeline derives the ACL caller from the context's call chain
    (`caller_id == call_chain[-1]`) and propagates the agent Identity to the
    child context. `upstream_hops` synthetic prior entries lengthen the chain so
    the `max_call_depth` fuse can be exercised — the deeper the chain, the closer
    a reader agent gets to its cap.
    """
    identity = (
        Identity(id=caller_id or "unknown", type="ai", roles=tuple(roles)) if roles else None
    )
    ctx: Context = Context.create(identity=identity) if identity else Context.create()
    prior = [f"upstream.hop{i}" for i in range(upstream_hops)]
    ctx.call_chain = [*prior, caller_id] if caller_id else list(prior)
    try:
        return True, client.call(target, inputs, context=ctx)
    except ACLDeniedError as exc:
        return False, exc


# (label, caller_id, roles, upstream_hops, target, inputs, expected_allow)
SCENARIOS: list[tuple[str, str | None, list[str] | None, int, str, dict, bool]] = [
    ("External — read",                 None,             None,           0, "executor.crm.read",   {"record_id": "C-7"},      True),
    ("External — query (blocked)",      None,             None,           0, "executor.crm.query",  {"filter": "tier=gold"},   False),
    ("External — delete (blocked)",     None,             None,           0, "executor.crm.delete", {"record_id": "C-7"},      False),
    ("Reader — read",                   "agent.research", ["reader"],     0, "executor.crm.read",   {"record_id": "C-7"},      True),
    ("Reader — query",                  "agent.research", ["reader"],     0, "executor.crm.query",  {"filter": "tier=gold"},   True),
    ("Reader — query at depth cap",     "agent.research", ["reader"],     1, "executor.crm.query",  {"filter": "tier=gold"},   True),
    ("Reader — query over depth cap",   "agent.research", ["reader"],     2, "executor.crm.query",  {"filter": "tier=gold"},   False),
    ("Reader — delete (blocked)",       "agent.research", ["reader"],     0, "executor.crm.delete", {"record_id": "C-7"},      False),
    ("Reader — export (blocked)",       "agent.research", ["reader"],     0, "data.export",         {"dataset": "crm"},        False),
    ("Data-admin — export",             "agent.etl",      ["data_admin"], 0, "data.export",         {"dataset": "crm"},        True),
    ("Data-admin — delete",             "agent.etl",      ["data_admin"], 0, "executor.crm.delete", {"record_id": "C-7"},      True),
    ("Data-admin — delete deep chain",  "agent.etl",      ["data_admin"], 3, "executor.crm.delete", {"record_id": "C-7"},      True),
    ("Data-admin — query (blocked)",    "agent.etl",      ["data_admin"], 0, "executor.crm.query",  {"filter": "tier=gold"},   False),
    ("Unknown-role agent (blocked)",    "agent.guest",    ["guest"],      0, "executor.crm.read",   {"record_id": "C-7"},      False),
]


def main() -> int:
    print("AI Agent Tool-Governance ACL — end-to-end demo (issue #72)")
    print("Real tool calls through APCore, default_effect: deny, gradient @external < reader < data_admin")
    print("=" * 100)
    print(f"  {'RESULT':<8}{'CALLER':<17}{'ROLES':<13}{'TARGET':<22}{'OUTCOME':<8}DETAIL")
    print("-" * 100)

    failures = 0
    for label, caller_id, roles, hops, target, inputs, expected in SCENARIOS:
        allowed, payload = attempt(caller_id, roles, hops, target, inputs)
        ok = allowed == expected
        failures += not ok

        caller_disp = caller_id or "@external"
        roles_disp = ",".join(roles) if roles else "-"
        outcome = "ALLOW" if allowed else "DENY"
        detail = repr(payload) if allowed else "ACLDeniedError"
        mark = "PASS" if ok else "FAIL"
        print(f"  {mark:<8}{caller_disp:<17}{roles_disp:<13}{target:<22}{outcome:<8}{detail}")
        if not ok:
            print(f"          ^ expected {'ALLOW' if expected else 'DENY'} — {label}")

    print("=" * 100)
    print(f"Audit trail ({len(audit_trail)} decisions recorded by the ACL):")
    for e in audit_trail:
        roles = ",".join(e.roles) if e.roles else "-"
        print(
            f"  {e.caller_id:<17} -> {e.target_id:<22} {e.decision:<5}"
            f" (reason={e.reason}, roles={roles}, depth={e.call_depth})"
        )

    print("=" * 100)
    total = len(SCENARIOS)
    print(f"{total - failures}/{total} decisions match the cross-language contract.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
