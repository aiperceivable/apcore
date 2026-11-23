"""Drive `pipeline_section_wiring.json` — §5.16 requirements 6 and 7 (#118 D-72).

Every case goes through ``APCore(config=Config(document))``. That is the whole
point of the fixture, and it is why the three fixtures that already cover the
pipeline builder could not have caught this: they hand the section straight to
``build_strategy_from_config(section, …)``, whose first parameter is a dict the
CALLER supplies. Nothing extracted that dict from a ``Config``, so
``pipeline: remove: [acl_check]`` left all eleven steps in place while every
builder fixture stayed green.

A driver here that called the builder directly would reproduce the defect and
pass.
"""

from __future__ import annotations

import logging
from typing import Any

import pytest

from apcore import APCore
from apcore.config import Config
from apcore.pipeline import BaseStep, PipelineContext, StepResult
from apcore.pipeline_config import register_step_type, unregister_step_type

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("pipeline_section_wiring.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}


class _ProbeStep(BaseStep):
    """No-op step registered under the fixture's `register_step_type` name."""

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


def _build(case: dict[str, Any], caplog: pytest.LogCaptureFixture) -> Any:
    type_name = case["input"].get("register_step_type")
    if type_name:
        # `(config_dict) -> BaseStep`, per `register_step_type`'s contract; the
        # builder overwrites `.name` from the YAML entry straight after.
        register_step_type(type_name, lambda _config: _ProbeStep(type_name))
    try:
        with caplog.at_level(logging.WARNING, logger="apcore"):
            client = APCore(config=Config(dict(case["input"]["config"])))
    finally:
        if type_name:
            unregister_step_type(type_name)
    return client.executor._strategy


@pytest.mark.parametrize("case_id", list(CASES))
def test_pipeline_section_wiring(case_id: str, caplog: pytest.LogCaptureFixture) -> None:
    case = CASES[case_id]
    expected = case["expected"]
    caplog.clear()
    strategy = _build(case, caplog)

    assert [step.name for step in strategy.steps] == expected["steps"]

    configured = expected.get("configured_step")
    if configured is not None:
        step = next(s for s in strategy.steps if s.name == configured["name"])
        assert getattr(step, configured["field"]) == configured["value"]

    warned = [r for r in caplog.records if "pipeline.remove" in r.getMessage()]
    if expected.get("security_step_warning"):
        # Once per configuration load, per §9.2.2's cadence — not once per step.
        assert len(warned) == 1, [r.getMessage() for r in warned]
        assert expected["warning_names_step"] in warned[0].getMessage()
    else:
        assert warned == []


def test_every_fixture_case_is_driven() -> None:
    """A case added to the canonical fixture must fail here, not go unnoticed."""
    from .canonical_fixtures import case_ids

    assert set(case_ids("pipeline_section_wiring.json")) == set(CASES)
