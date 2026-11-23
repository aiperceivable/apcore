"""PROTOCOL_SPEC §6.6.5.5 (D-129) — providers reach the gate they configure.

The canonical cases live in ``conformance/fixtures/gate_provider_binding.json``
and are driven by ``tests/conformance/test_gate_provider_binding.py``. These
cover the Python-specific doors the fixture does not: the per-call strategy
override, a named registered strategy, the setters' type-based lookup and the
"no provider given leaves the step alone" half of the rule.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore import ACL, Executor, ExecutionPolicy, ModuleAnnotations, Registry
from apcore.acl import ACLRule
from apcore.approval import AlwaysDenyHandler
from apcore.builtin_steps import BuiltinACLCheck, BuiltinApprovalGate, build_standard_strategy
from apcore.errors import ModuleError
from apcore.pipeline import BaseStep, ExecutionStrategy, PipelineContext, StepResult


class _Target:
    description = "D-129 target"

    def __init__(self, *, requires_approval: bool = False) -> None:
        self.annotations = ModuleAnnotations(requires_approval=requires_approval)

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"ok": True}


def _registry(requires_approval: bool = False) -> Registry:
    registry = Registry()
    registry.register("demo.target", _Target(requires_approval=requires_approval))
    return registry


def _deny_all() -> ACL:
    return ACL(rules=[], default_effect="deny")


def _allow_all() -> ACL:
    return ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")], default_effect="deny")


def _gate(strategy: ExecutionStrategy, kind: type) -> Any:
    return next(step for step in strategy.steps if isinstance(step, kind))


def _code(executor: Executor, **kwargs: Any) -> str:
    try:
        executor.call("demo.target", {}, **kwargs)
    except ModuleError as exc:
        return exc.code
    return "ok"


def test_explicit_strategy_instance_receives_executor_acl() -> None:
    """The reported repro: a deny-all ACL on the executor, a bare pre-built strategy."""
    registry = _registry()
    executor = Executor(registry=registry, acl=_deny_all(), strategy=build_standard_strategy(registry=registry))

    assert _code(executor) == "ACL_DENIED"
    state = executor.governance_state()
    assert state.acl_configured is True
    assert state.builtin_acl_gate_wired is True


def test_explicit_strategy_instance_receives_approval_handler_and_policy() -> None:
    registry = _registry(requires_approval=True)
    handler = AlwaysDenyHandler()
    policy = ExecutionPolicy(strict=True)
    strategy = build_standard_strategy(registry=registry)
    executor = Executor(registry=registry, approval_handler=handler, policy=policy, strategy=strategy)

    gate = _gate(strategy, BuiltinApprovalGate)
    assert gate.handler is handler
    assert gate.policy is policy
    assert _code(executor) == "APPROVAL_DENIED"


def test_executor_without_provider_leaves_the_step_provider_alone() -> None:
    registry = _registry()
    own_acl = _deny_all()
    strategy = build_standard_strategy(registry=registry, acl=own_acl)
    executor = Executor(registry=registry, strategy=strategy)

    assert _gate(strategy, BuiltinACLCheck).acl is own_acl
    assert _code(executor) == "ACL_DENIED"
    # Reported from the running gate, although the executor was never given it.
    assert executor.governance_state().acl_configured is True


def test_executor_provider_replaces_the_step_provider() -> None:
    registry = _registry()
    executor_acl = _deny_all()
    strategy = build_standard_strategy(registry=registry, acl=_allow_all())
    executor = Executor(registry=registry, acl=executor_acl, strategy=strategy)

    assert _gate(strategy, BuiltinACLCheck).acl is executor_acl
    assert _code(executor) == "ACL_DENIED"


def test_governance_state_reports_configured_not_wired_when_no_gate_runs() -> None:
    registry = _registry()
    executor = Executor(registry=registry, acl=_deny_all(), strategy="internal")

    assert _code(executor) == "ok"
    state = executor.governance_state()
    assert state.acl_configured is True
    assert state.builtin_acl_gate_wired is False


def test_registered_named_strategy_receives_executor_acl() -> None:
    registry = _registry()
    Executor.register_strategy("d129_registered", build_standard_strategy(registry=registry))
    try:
        executor = Executor(registry=registry, acl=_deny_all(), strategy="d129_registered")
        assert _code(executor) == "ACL_DENIED"
    finally:
        Executor._registered_strategies.pop("d129_registered", None)


def test_per_call_strategy_override_receives_executor_providers() -> None:
    """``call_with_trace(strategy=...)`` is another way the executor gets a strategy."""
    registry = _registry()
    executor = Executor(registry=registry, acl=_deny_all())

    with pytest.raises(ModuleError) as excinfo:
        executor.call_with_trace("demo.target", {}, strategy=build_standard_strategy(registry=registry))
    assert excinfo.value.code == "ACL_DENIED"


def test_per_call_preset_override_receives_executor_policy() -> None:
    registry = _registry(requires_approval=True)
    executor = Executor(registry=registry, policy=ExecutionPolicy(strict=True))

    with pytest.raises(ModuleError) as excinfo:
        executor.call_with_trace("demo.target", {}, strategy="standard")
    assert excinfo.value.code == "APPROVAL_DENIED"


class _LookalikeACLCheck(BaseStep):
    """Named ``acl_check`` and exposing ``set_acl``, but not the built-in gate."""

    def __init__(self) -> None:
        super().__init__("acl_check", requires=("context", "module"))
        self.received: Any = None

    def set_acl(self, acl: Any) -> None:
        self.received = acl

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


def test_setters_locate_the_gate_by_type_not_by_name() -> None:
    registry = _registry()
    strategy = build_standard_strategy(registry=registry)
    lookalike = _LookalikeACLCheck()
    strategy.replace("acl_check", lookalike)
    executor = Executor(registry=registry, strategy=strategy)

    executor.set_acl(_deny_all())

    assert lookalike.received is None
    state = executor.governance_state()
    assert state.builtin_acl_gate_wired is False
    assert state.acl_configured is True


class TestSetAclWarnsWithoutGate:
    """#123: matches apcore-typescript ``setAcl`` — an ACL no step will consult is announced."""

    def test_warns_when_strategy_has_no_acl_step(self, caplog: pytest.LogCaptureFixture) -> None:
        registry = Registry()
        executor = Executor(registry=registry, strategy="internal")
        with caplog.at_level("WARNING", logger="apcore.executor"):
            executor.set_acl(_deny_all())
        assert any("no BuiltinACLCheck step" in r.getMessage() for r in caplog.records)

    def test_silent_when_strategy_has_acl_step(self, caplog: pytest.LogCaptureFixture) -> None:
        registry = Registry()
        executor = Executor(registry=registry, strategy=build_standard_strategy(registry=registry))
        with caplog.at_level("WARNING", logger="apcore.executor"):
            executor.set_acl(_deny_all())
        assert not any("no BuiltinACLCheck step" in r.getMessage() for r in caplog.records)
