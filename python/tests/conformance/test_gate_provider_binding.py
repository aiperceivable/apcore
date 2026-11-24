"""Drive ``gate_provider_binding.json`` — providers reach the gate they configure.

PROTOCOL_SPEC §6.6.5.5 (D-129). An ACL, ``ApprovalHandler`` or ``ExecutionPolicy``
given to an ``Executor`` MUST be bound into the built-in gate step of the RUNNING
strategy however that strategy was supplied, and ``governance_state()`` MUST
report what the running gate step holds.

``driver_contract.path``: every case constructs a real Executor, registers a real
module, makes one real call as ``@external`` and only then reads
``governance_state()``. Asserting the accessor alone is not enough — the defect
this fixture pins is a call that RAN while the accessor said a gate stood in
front of it.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore import ACL, Executor, ExecutionPolicy, ModuleAnnotations, Registry
from apcore.acl import ACLRule
from apcore.approval import AlwaysDenyHandler
from apcore.builtin_steps import build_standard_strategy
from apcore.errors import ModuleError
from conformance.canonical_fixtures import case_ids, load_fixture

FIXTURE = load_fixture("gate_provider_binding.json")
CASES = FIXTURE["test_cases"]
MODULE_ID = "demo.target"


class _DemoTarget:
    """A trivial module returning ``{"ok": true}``."""

    description = "Conformance target for gate_provider_binding.json"

    def __init__(self, *, requires_approval: bool) -> None:
        self.annotations = ModuleAnnotations(requires_approval=requires_approval)

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"ok": True}


def _acl(kind: str | None) -> ACL | None:
    """``driver_contract.providers`` — build the named ACL."""
    if kind is None:
        return None
    if kind == "deny_all":
        return ACL(rules=[], default_effect="deny")
    if kind == "allow_all":
        return ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")], default_effect="deny")
    raise AssertionError(f"unknown ACL kind in fixture: {kind!r}")


def _handler(kind: str | None) -> Any:
    if kind is None:
        return None
    if kind == "always_deny":
        return AlwaysDenyHandler()
    raise AssertionError(f"unknown approval_handler kind in fixture: {kind!r}")


def _build(setup: dict[str, Any]) -> Executor:
    registry = Registry()
    registry.register(MODULE_ID, _DemoTarget(requires_approval=setup["requires_approval"]))

    executor_kwargs: dict[str, Any] = {
        "acl": _acl(setup["executor_acl"]),
        "approval_handler": _handler(setup["approval_handler"]),
        "policy": ExecutionPolicy(strict=True) if setup["policy_strict"] else None,
    }

    form = setup["strategy_form"]
    if form == "default":
        pass
    elif form == "preset:standard":
        executor_kwargs["strategy"] = "standard"
    elif form == "preset:internal":
        executor_kwargs["strategy"] = "internal"
    elif form == "instance:bare":
        # The SDK's public builder WITHOUT any provider.
        executor_kwargs["strategy"] = build_standard_strategy(registry=registry)
    elif form == "instance:own_acl":
        executor_kwargs["strategy"] = build_standard_strategy(registry=registry, acl=_acl(setup["strategy_acl"]))
    else:
        raise AssertionError(f"unknown strategy_form in fixture: {form!r}")

    if form != "instance:own_acl":
        assert setup["strategy_acl"] is None, f"strategy_acl is only meaningful for instance:own_acl, got {form!r}"

    return Executor(registry, **executor_kwargs)


def _call_outcome(executor: Executor) -> str:
    """``expected.call``: ``ok`` when the call returned ``{"ok": true}``, else the error code."""
    try:
        result = executor.call(MODULE_ID, {})
    except ModuleError as exc:
        return exc.code
    assert result == {"ok": True}, f"the call returned {result!r}, not {{'ok': True}}"
    return "ok"


@pytest.mark.parametrize("case", CASES, ids=case_ids("gate_provider_binding.json"))
def test_gate_provider_binding(case: dict[str, Any]) -> None:
    executor = _build(case["setup"])
    try:
        outcome = _call_outcome(executor)
        assert outcome == case["expected"]["call"], (
            f"[{case['id']}] call outcome is {outcome!r}, fixture expects "
            f"{case['expected']['call']!r}. {case.get('note', '')}"
        )

        state = executor.governance_state()
        for field, expected in case["expected"]["governance"].items():
            actual = getattr(state, field)
            assert actual == expected, (
                f"[{case['id']}] governance_state().{field} is {actual}, fixture expects {expected}. "
                f"{case.get('note', '')}"
            )
    finally:
        executor.close()


def test_every_fixture_case_is_driven() -> None:
    """Every case goes through the one parametrized driver above; pin the id set."""
    ids = [c["id"] for c in CASES]
    assert len(ids) == len(set(ids)), f"duplicate case ids in gate_provider_binding.json: {ids}"
    for case in CASES:
        assert set(case["setup"]) == {
            "strategy_form",
            "executor_acl",
            "strategy_acl",
            "approval_handler",
            "policy_strict",
            "requires_approval",
        }, f"[{case['id']}] setup carries keys this driver does not read: {sorted(case['setup'])}"
        assert set(case["expected"]) == {"call", "governance"}, f"[{case['id']}] unread expected keys"
