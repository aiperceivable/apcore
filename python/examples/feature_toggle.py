"""Runtime feature-toggle + per-instance isolation — end-to-end demo (issue #71).

apcore is a standalone product: this example governs real tool calls through the
execution pipeline, with NO framework integration required. It demonstrates the
`system.control.toggle_feature` capability via the `APCore` convenience methods
(`client.disable` / `client.enable`):

  1. registers real `executor.*` tools on an `APCore` instance,
  2. calls a tool and gets a real result,
  3. disables it — the next call raises `ModuleDisabledError` while a *different*
     tool keeps working,
  4. re-enables it — the call works again, and
  5. proves **per-instance isolation (#71)**: two independent `APCore` instances
     each register the same tool; disabling it on instance A blocks A's call
     while instance B's identical call still succeeds. Each instance owns its own
     `ToggleState` (locked cross-language by
     `conformance/fixtures/toggle_state_isolation.json`).

Toggle control requires `sys_modules` (and `sys_modules.events`) enabled in
config, so the example constructs that `Config` itself.

Each step carries its expected outcome, so this script doubles as a smoke test:
it exits non-zero if any step drifts from the cross-language contract.

Run (from the apcore-python repo root):
    python examples/feature_toggle.py
"""

from __future__ import annotations

import sys

from apcore import APCore, Context, ModuleDisabledError
from apcore.config import Config


def sys_config() -> Config:
    """Config with sys_modules + events enabled — required for toggle control."""
    return Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}})


def build_client() -> APCore:
    """A client with two real CRM tools registered."""
    client = APCore(config=sys_config())

    @client.module(id="executor.crm.read", description="Read a single CRM record")
    def crm_read(record_id: str) -> dict:
        return {"record_id": record_id, "name": "Acme Corp", "tier": "gold"}

    @client.module(id="executor.crm.query", description="Query CRM records by filter")
    def crm_query(filter: str) -> dict:  # noqa: A002 - mirror cross-language field name
        return {"filter": filter, "matched": 3}

    return client


def call_ok(client: APCore, target: str, inputs: dict) -> tuple[bool, object]:
    """Call `target`; return (allowed, result_or_error). False == ModuleDisabledError."""
    try:
        return True, client.call(target, inputs, context=Context.create())
    except ModuleDisabledError as exc:
        return False, exc


class Checker:
    """Records one PASS/FAIL line per step and tracks the failure count."""

    def __init__(self) -> None:
        self.failures = 0
        self.total = 0

    def check(self, label: str, allowed: bool, expected: bool, detail: object) -> None:
        self.total += 1
        ok = allowed == expected
        self.failures += not ok
        outcome = "ALLOW" if allowed else "DISABLED"
        shown = repr(detail) if allowed else "ModuleDisabledError"
        mark = "PASS" if ok else "FAIL"
        print(f"  {mark:<7}{label:<46}{outcome:<10}{shown}")
        if not ok:
            print(f"         ^ expected {'ALLOW' if expected else 'DISABLED'}")


def main() -> int:
    print("Runtime feature-toggle + per-instance isolation — end-to-end demo (issue #71)")
    print("Real tool calls through APCore; disable/enable via system.control.toggle_feature")
    print("=" * 92)
    print(f"  {'RESULT':<7}{'STEP':<46}{'OUTCOME':<10}DETAIL")
    print("-" * 92)

    chk = Checker()

    # --- Single-instance toggle lifecycle ---------------------------------
    client = build_client()

    allowed, payload = call_ok(client, "executor.crm.read", {"record_id": "C-7"})
    chk.check("1. read works before toggle", allowed, True, payload)

    client.disable("executor.crm.read", reason="incident-1234")
    allowed, payload = call_ok(client, "executor.crm.read", {"record_id": "C-7"})
    chk.check("2. read disabled -> blocked", allowed, False, payload)

    allowed, payload = call_ok(client, "executor.crm.query", {"filter": "tier=gold"})
    chk.check("3. different tool still works", allowed, True, payload)

    client.enable("executor.crm.read", reason="incident resolved")
    allowed, payload = call_ok(client, "executor.crm.read", {"record_id": "C-7"})
    chk.check("4. read re-enabled -> works", allowed, True, payload)

    # --- Per-instance isolation (#71) -------------------------------------
    client_a = build_client()
    client_b = build_client()
    client_a.disable("executor.crm.read", reason="A-only maintenance")

    allowed, payload = call_ok(client_a, "executor.crm.read", {"record_id": "C-7"})
    chk.check("5. instance A disabled -> blocked", allowed, False, payload)

    allowed, payload = call_ok(client_b, "executor.crm.read", {"record_id": "C-7"})
    chk.check("6. instance B unaffected -> works", allowed, True, payload)

    print("=" * 92)
    print(f"{chk.total - chk.failures}/{chk.total} steps match the cross-language contract.")
    return 1 if chk.failures else 0


if __name__ == "__main__":
    sys.exit(main())
