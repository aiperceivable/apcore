"""PROTOCOL_SPEC §5.16.1 (D-130) — a governance gate cannot be weakened by configure.

The canonical YAML cases are in ``conformance/fixtures/gate_step_configure.json``
(driven by ``tests/conformance/test_gate_step_configure.py``). These cover the
programmatic door the fixture cannot reach — assigning the field on the step
object — and the Executor's own ``pipeline:`` section path.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore import Config, Executor, Registry
from apcore.builtin_steps import BuiltinACLCheck, BuiltinApprovalGate, build_standard_strategy
from apcore.pipeline import ConfigurationError
from apcore.pipeline_config import build_strategy_from_config


def _gate(kind: type) -> Any:
    strategy = build_standard_strategy(registry=Registry())
    return next(step for step in strategy.steps if isinstance(step, kind))


@pytest.mark.parametrize("kind", [BuiltinACLCheck, BuiltinApprovalGate])
@pytest.mark.parametrize(
    ("key", "value"),
    [("ignore_errors", True), ("match_modules", ("public.*",)), ("match_modules", ())],
)
def test_programmatic_assignment_cannot_weaken_a_gate(kind: type, key: str, value: Any) -> None:
    gate = _gate(kind)
    before = getattr(gate, key)
    with pytest.raises(ConfigurationError) as excinfo:
        setattr(gate, key, value)
    assert excinfo.value.code == "PIPELINE_CONFIGURATION_ERROR"
    assert gate.name in str(excinfo.value)
    assert key in str(excinfo.value)
    assert getattr(gate, key) == before, "a rejected value must not have been applied"


def test_programmatic_pure_true_on_approval_gate_rejected() -> None:
    gate = _gate(BuiltinApprovalGate)
    with pytest.raises(ConfigurationError) as excinfo:
        gate.pure = True
    assert excinfo.value.code == "PIPELINE_CONFIGURATION_ERROR"
    assert "approval_gate" in str(excinfo.value) and "pure" in str(excinfo.value)
    assert gate.pure is False


@pytest.mark.parametrize("kind", [BuiltinACLCheck, BuiltinApprovalGate])
def test_default_values_and_timeout_stay_configurable(kind: type) -> None:
    gate = _gate(kind)
    gate.ignore_errors = False
    gate.match_modules = None
    gate.pure = False
    gate.timeout_ms = 500
    assert (gate.ignore_errors, gate.match_modules, gate.pure, gate.timeout_ms) == (False, None, False, 500)


def test_acl_check_default_pure_true_is_accepted() -> None:
    """``acl_check`` is pure by default (validate() runs it); writing the default is accepted."""
    strategy = build_strategy_from_config({"configure": {"acl_check": {"pure": True}}}, registry=Registry())
    gate = next(step for step in strategy.steps if isinstance(step, BuiltinACLCheck))
    assert gate.pure is True


def test_config_error_names_every_offending_key() -> None:
    with pytest.raises(ConfigurationError) as excinfo:
        build_strategy_from_config(
            {"configure": {"approval_gate": {"ignore_errors": True, "match_modules": ["x.*"], "pure": True}}},
            registry=Registry(),
        )
    message = str(excinfo.value)
    for needle in ("approval_gate", "ignore_errors", "match_modules", "pure"):
        assert needle in message


def test_executor_pipeline_section_rejects_a_weakened_gate() -> None:
    """The apcore.yaml path: the Executor builds its strategy from ``pipeline:`` and fails the load."""
    config = Config({"pipeline": {"configure": {"acl_check": {"ignore_errors": True}}}})
    with pytest.raises(ConfigurationError) as excinfo:
        Executor(Registry(), config=config)
    assert excinfo.value.code == "PIPELINE_CONFIGURATION_ERROR"
    assert "acl_check" in str(excinfo.value) and "ignore_errors" in str(excinfo.value)


def test_non_gate_step_programmatic_assignment_unaffected() -> None:
    strategy = build_standard_strategy(registry=Registry())
    step = next(s for s in strategy.steps if s.name == "input_validation")
    step.ignore_errors = True
    step.match_modules = ("a.*",)
    step.pure = True
    assert (step.ignore_errors, step.match_modules, step.pure) == (True, ("a.*",), True)
