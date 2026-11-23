"""Conformance driver for ``governance_state.json`` (PROTOCOL_SPEC 6.6.5).

Drives the real ``Executor.governance_state()`` on a real Registry and a real
strategy. The nine booleans are asserted against the fixture as returned by the
SDK — including the derived flag, which is asserted rather than recomputed: a
driver that derives it from the other eight is green against an implementation
that never computes it.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore import ACL, Config, Executor, ExecutionPolicy, ModuleAnnotations, Registry
from apcore.pipeline import ExecutionStrategy
from conformance.canonical_fixtures import case_ids, load_fixture

FIXTURE = load_fixture("governance_state.json")
CASES = FIXTURE["test_cases"]


class _NeverCalledHandler:
    """6.6.5.3 — the accessor reports that a handler is attached, never consults it."""

    def request_approval(self, request: Any) -> Any:
        raise AssertionError("governance_state() must not invoke the approval handler")

    def check_approval(self, token: str) -> Any:
        raise AssertionError("governance_state() must not invoke the approval handler")


class _LookalikeACLCheck:
    """A step whose NAME is ``acl_check`` and which is not the built-in gate."""

    name = "acl_check"
    requires: list[str] = []
    provides: list[str] = []

    def execute(self, ctx: Any) -> Any:  # pragma: no cover - never executed
        return None


def _register(registry: Registry, module_id: str, *, requires_approval: bool) -> None:
    def handler(inputs: dict[str, Any], ctx: Any) -> dict[str, Any]:  # pragma: no cover
        return {}

    handler.annotations = ModuleAnnotations(requires_approval=requires_approval)
    handler.description = "conformance control module"
    handler.input_schema = {"type": "object"}
    handler.output_schema = {"type": "object"}
    registry.register_internal(module_id, handler)


def _build(setup: dict[str, Any]) -> Executor:
    registry = Registry()
    for entry in setup["control_modules"]:
        _register(registry, entry["module_id"], requires_approval=entry["requires_approval"])
    if setup["read_modules"]:
        _register(registry, "system.health.summary", requires_approval=False)

    config = Config({})
    strategy = setup["strategy"]
    if strategy == "lookalike_acl_check":
        executor = Executor(
            registry,
            config=config,
            strategy=ExecutionStrategy("lookalike_acl_check", [_LookalikeACLCheck()], validate_dependencies=False),
        )
    elif strategy == "standard":
        executor = Executor(registry, config=config)
    else:
        executor = Executor(registry, config=config, strategy=strategy)

    if setup["acl_attached"]:
        executor.set_acl(ACL(default_effect="deny"))
    if setup["approval_handler_attached"]:
        executor.set_approval_handler(_NeverCalledHandler())
    if setup["policy_strict"]:
        executor.set_policy(ExecutionPolicy(strict=True))
    return executor


@pytest.mark.parametrize("case", CASES, ids=case_ids("governance_state.json"))
def test_governance_state(case: dict[str, Any]) -> None:
    executor = _build(case["setup"])
    state = executor.governance_state()

    for field, expected in case["expected"].items():
        actual = getattr(state, field)
        assert actual == expected, (
            f"{case['id']}: {field} is {actual}, fixture expects {expected}\n" f"  {case['note']}"
        )


def test_accessor_is_a_pure_read() -> None:
    """driver_contract.purity — two reads are equal and the handler is never called."""
    executor = _build(
        {
            "control_modules": [{"module_id": "system.control.reload_module", "requires_approval": True}],
            "read_modules": True,
            "strategy": "standard",
            "acl_attached": False,
            "approval_handler_attached": True,
            "policy_strict": False,
        }
    )
    assert executor.governance_state() == executor.governance_state()


def test_accessor_is_live_not_cached() -> None:
    """driver_contract.liveness — a cached value passes every static case."""
    executor = _build(
        {
            "control_modules": [{"module_id": "system.control.reload_module", "requires_approval": True}],
            "read_modules": True,
            "strategy": "standard",
            "acl_attached": False,
            "approval_handler_attached": False,
            "policy_strict": False,
        }
    )
    before = executor.governance_state()
    executor.set_acl(ACL(default_effect="deny"))
    after = executor.governance_state()

    assert before.acl_configured is False
    assert after.acl_configured is True
    assert before.builtin_acl_gate_wired is True
