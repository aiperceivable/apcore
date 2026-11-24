"""Stub tests to confirm apcore.pipeline_config is importable."""

import pytest

from apcore.pipeline import BaseStep, ConfigurationError, PipelineContext, StepResult
from apcore.pipeline_config import build_strategy_from_config, register_step_type, unregister_step_type
from apcore.registry import Registry


def test_pipeline_config_importable():
    import apcore.pipeline_config as pipeline_config

    assert pipeline_config is not None


@pytest.mark.parametrize("form", ["configure", "steps"])
def test_pipeline_config_rejects_string_match_modules(form: str) -> None:
    class FilterStep(BaseStep):
        async def execute(self, ctx: PipelineContext) -> StepResult:
            return StepResult(action="continue")

    register_step_type("filter_test", lambda _config: FilterStep("filter_test"))
    if form == "steps":
        pipeline = {
            "steps": [{"name": "filter_test", "type": "filter_test", "after": "execute", "match_modules": "orders.*"}]
        }
    else:
        pipeline = {"configure": {"input_validation": {"match_modules": "orders.*"}}}
    try:
        with pytest.raises(ConfigurationError, match="match_modules"):
            build_strategy_from_config(pipeline, registry=Registry())
    finally:
        unregister_step_type("filter_test")
