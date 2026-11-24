"""PROTOCOL_SPEC §5.16 requirements 6 and 7 — a configured `pipeline:` section.

The section was accepted, validated and then ignored, in all three SDKs
(apcore#118, decision D-72). `build_strategy_from_config` existed everywhere and
its first parameter was a dict the CALLER supplied; nothing extracted that dict
from a loaded `Config`, so `pipeline: remove: [acl_check]` left all eleven steps
in place and a declared custom step silently never ran.

The asymmetry is why these cases exist. Failing to *remove* a step is fail-safe;
failing to *insert* one is not — a declared audit, rate-limit or authorization
step that never runs is invisible from inside the running system, and the
pipeline an operator reads in configuration is not the pipeline that executes.
"""

from __future__ import annotations

import logging
from typing import Any

import pytest

from apcore import APCore
from apcore.config import Config
from apcore.policy import ExecutionPolicy


def _steps(section: dict[str, Any] | None) -> list[str]:
    doc: dict[str, Any] = {"version": "1.0", "project": {"name": "pipeline-probe"}}
    if section is not None:
        doc["pipeline"] = section
    client = APCore(config=Config(doc))
    strategy = getattr(client.executor, "_strategy", None)
    return [step.name for step in getattr(strategy, "steps", [])]


DEFAULT = [
    "context_creation",
    "call_chain_guard",
    "module_lookup",
    "acl_check",
    "approval_gate",
    "middleware_before",
    "input_validation",
    "execute",
    "output_validation",
    "middleware_after",
    "return_result",
]


def test_no_pipeline_section_leaves_the_default_pipeline() -> None:
    """The half that keeps this additive for everyone who configures nothing."""
    assert _steps(None) == DEFAULT
    assert _steps({}) == DEFAULT


def test_remove_takes_the_step_out() -> None:
    assert "output_validation" not in _steps({"remove": ["output_validation"]})


def test_configure_reaches_the_named_step() -> None:
    """`configure:` replaces a field of an existing step, keeping its position."""
    client = APCore(
        config=Config(
            {
                "version": "1.0",
                "project": {"name": "pipeline-probe"},
                "pipeline": {"configure": {"input_validation": {"ignore_errors": True}}},
            }
        )
    )
    strategy = client.executor._strategy
    step = next(s for s in strategy.steps if s.name == "input_validation")
    assert step.ignore_errors is True
    assert [s.name for s in strategy.steps] == DEFAULT, "configure must not reorder"


def test_a_declared_step_is_inserted() -> None:
    """The direction that is NOT fail-safe: a declared step must actually run.

    An operator who adds an audit or rate-limit step and gets a client that
    silently omits it has no way to notice from inside the running system.
    """
    from apcore.pipeline_config import register_step_type, unregister_step_type
    from apcore.builtin_steps import BaseStep
    from apcore.pipeline import StepResult

    class _Probe(BaseStep):
        def __init__(self) -> None:
            super().__init__(name="probe_gate", description="probe", removable=True)

        def execute(self, ctx: Any) -> Any:
            return StepResult(action="continue")

    # The registry hands the factory the step's `config` block positionally.
    register_step_type("probe_gate", lambda _config=None: _Probe())
    try:
        names = _steps({"steps": [{"name": "probe_gate", "type": "probe_gate", "after": "acl_check"}]})
    finally:
        unregister_step_type("probe_gate")
    assert "probe_gate" in names, "a declared step was accepted and never inserted"
    assert names.index("probe_gate") == names.index("acl_check") + 1


@pytest.mark.parametrize("step", ["acl_check", "approval_gate"])
def test_removing_a_security_step_warns(step: str, caplog: pytest.LogCaptureFixture) -> None:
    """§5.16 requirement 7 — the transition, not the steady state.

    Requirement 6 makes a previously ignored section take effect, so a config
    that has been carrying `remove: [acl_check]` while ACL was enforced anyway
    starts having ACL genuinely removed.
    """
    with caplog.at_level(logging.WARNING, logger="apcore.executor"):
        names = _steps({"remove": [step]})
    assert step not in names
    assert "pipeline.remove" in caplog.text and step in caplog.text


def test_removing_an_ordinary_step_is_silent(caplog: pytest.LogCaptureFixture) -> None:
    """The other half: the notice is about protections, not about every removal."""
    with caplog.at_level(logging.WARNING, logger="apcore.executor"):
        _steps({"remove": ["output_validation"]})
    assert "pipeline.remove" not in caplog.text


def test_an_explicit_strategy_still_wins_over_the_configured_section() -> None:
    """D-73's precedence: an API argument beats `Config`.

    Without this the wiring would take a caller's hand-built strategy away from
    them whenever a `pipeline:` section happened to be present.
    """
    from apcore.builtin_steps import build_standard_strategy
    from apcore.executor import Executor
    from apcore.registry import Registry

    registry = Registry()
    explicit = build_standard_strategy(registry=registry)
    executor = Executor(
        registry,
        config=Config(
            {
                "version": "1.0",
                "project": {"name": "pipeline-probe"},
                "pipeline": {"remove": ["acl_check"]},
            }
        ),
        strategy=explicit,
    )
    assert "acl_check" in [s.name for s in executor._strategy.steps]


def test_the_configured_strategy_keeps_the_governance_wiring() -> None:
    """A configured section must not cost the caller their ExecutionPolicy.

    `build_strategy_from_config` accepted a narrower kwarg set than
    `build_standard_strategy`, so routing through it would have silently dropped
    `policy`, `event_emitter` and `toggle_state` — a worse defect than the
    ignored section this wiring fixes.
    """
    from apcore.executor import Executor
    from apcore.registry import Registry

    policy = ExecutionPolicy()
    executor = Executor(
        Registry(),
        config=Config(
            {
                "version": "1.0",
                "project": {"name": "pipeline-probe"},
                "pipeline": {"remove": ["output_validation"]},
            }
        ),
        policy=policy,
    )
    gate = next(s for s in executor._strategy.steps if s.name == "approval_gate")
    assert getattr(gate, "_policy", None) is policy
