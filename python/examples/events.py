"""Lifecycle event subscription (event bus) — end-to-end demo.

apcore ships a global event bus for framework lifecycle events: module
registration, feature toggles, config updates, and health thresholds. This
example wires the bus into an `APCore` client and watches real lifecycle
events fire as the client is driven — see the feature spec at
`apcore/docs/features/event-system.md` ("Event Naming Convention" and the
"Event Types" table for the canonical `apcore.<subsystem>.<event>` names).

It

  1. builds an `APCore` with `sys_modules` + `events` ENABLED in `Config`
     (the bus is off by default; `on()` raises `SysModulesDisabledError`
     otherwise),
  2. subscribes via `client.on(event_type, callback)` to two canonical
     lifecycle events — `apcore.registry.module_registered` and
     `apcore.module.toggled` — collecting each into a list,
  3. performs the actions that fire them: registers a real tool (fires
     `module_registered`) and disables + re-enables it (fires
     `module.toggled` twice), then
  4. prints the collected events and unsubscribes via `client.off()`.

Each expected event carries its count, so this script doubles as a smoke
test: it exits non-zero if the observed lifecycle stream drifts.

Run (from the apcore-python repo root):
    python examples/events.py
"""

from __future__ import annotations

import sys

from apcore import APCore
from apcore.config import Config
from apcore.events.emitter import ApCoreEvent

# ---------------------------------------------------------------------------
# 1. Build an APCore with the event bus enabled. The bus is off by default;
#    sys_modules.enabled + sys_modules.events.enabled turn it on so that
#    client.events / client.on / client.off are live.
# ---------------------------------------------------------------------------
config = Config(
    {
        "sys_modules": {
            "enabled": True,
            "events": {"enabled": True},
        }
    }
)
client = APCore(config=config)


# ---------------------------------------------------------------------------
# 2. Subscribe to canonical lifecycle events; every delivery is collected.
# ---------------------------------------------------------------------------
collected: list[ApCoreEvent] = []

sub_registered = client.on("apcore.registry.module_registered", collected.append)
sub_toggled = client.on("apcore.module.toggled", collected.append)


# ---------------------------------------------------------------------------
# 3. Drive the actions that fire those events.
# ---------------------------------------------------------------------------
def run_lifecycle() -> None:
    """Register a tool, then disable and re-enable it."""
    # Registration fires apcore.registry.module_registered.
    @client.module(id="executor.crm.read", description="Read a single CRM record")
    def crm_read(record_id: str) -> dict:
        return {"record_id": record_id, "name": "Acme Corp", "tier": "gold"}

    # A real call through the pipeline — the result is unaffected by the bus.
    client.call("executor.crm.read", {"record_id": "C-7"})

    # Each toggle fires apcore.module.toggled (enabled=false, then enabled=true).
    client.disable("executor.crm.read", reason="maintenance window")
    client.enable("executor.crm.read", reason="maintenance complete")

    # Delivery is async (thread-pool dispatch); block until the bus drains.
    assert client.events is not None
    client.events.flush()


# (event_type, expected count)
EXPECTED: list[tuple[str, int]] = [
    ("apcore.registry.module_registered", 1),
    ("apcore.module.toggled", 2),
]


def main() -> int:
    print("Lifecycle event subscription (event bus) — end-to-end demo")
    print("APCore with sys_modules.events enabled; canonical apcore.<subsystem>.<event> names")
    print("=" * 92)

    run_lifecycle()

    print(f"Collected {len(collected)} lifecycle events (in delivery order):")
    print("-" * 92)
    for e in collected:
        if e.event_type == "apcore.module.toggled":
            detail = f"module_id={e.data.get('module_id')}, enabled={e.data.get('enabled')}"
        else:
            detail = f"module_id={e.module_id}"
        print(f"  {e.severity:<6}{e.event_type:<40}{detail}")

    # Unsubscribe; subsequent events would no longer be collected.
    client.off(sub_registered)
    client.off(sub_toggled)

    print("=" * 92)
    failures = 0
    for event_type, want in EXPECTED:
        got = sum(1 for e in collected if e.event_type == event_type)
        ok = got == want
        failures += not ok
        mark = "PASS" if ok else "FAIL"
        print(f"  {mark}  {event_type:<40} received {got}, expected {want}")

    print("=" * 92)
    total = len(EXPECTED)
    print(f"{total - failures}/{total} lifecycle event expectations met.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
