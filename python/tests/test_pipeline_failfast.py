"""Tests for Issue #33 §1.2 + §2.1 — pipeline configuration and dependency
validation now fail-fast (raise) instead of warning."""

from __future__ import annotations

import pytest

from apcore.errors import ModuleError
from apcore.pipeline import (
    BaseStep,
    ConfigurationError,
    ExecutionStrategy,
    PipelineContext,
    PipelineDependencyError,
    StepResult,
)


class _Step(BaseStep):
    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# §2.1 hardened dependency validation
# ---------------------------------------------------------------------------


class TestDependencyValidationRaises:
    def test_unmet_requires_raises_pipeline_dependency_error(self) -> None:
        a = _Step("a")  # provides nothing
        b = _Step("b", requires=("missing_key",))
        with pytest.raises(PipelineDependencyError) as excinfo:
            ExecutionStrategy("s", [a, b])
        assert "missing_key" in str(excinfo.value)
        assert excinfo.value.step_name == "b"

    def test_satisfied_requires_no_error(self) -> None:
        a = _Step("a", provides=("ctx",))
        b = _Step("b", requires=("ctx",))
        ExecutionStrategy("s", [a, b])  # should not raise

    def test_dependency_error_subclasses_module_error(self) -> None:
        assert issubclass(PipelineDependencyError, ModuleError)


# ---------------------------------------------------------------------------
# §1.2 configuration fail-fast
# ---------------------------------------------------------------------------


class TestPipelineConfigFailFast:
    def _stub_registry(self) -> object:
        # build_standard_strategy needs a registry-like object — we just need it to
        # produce a strategy. We can pass None to many of its kwargs; but it will
        # always require a registry for the lookup step. Build a minimal stand-in.
        class _Reg:
            def get(self, *a, **k):
                return None

            def discover(self, *a, **k):
                return []

        # Force use of build_standard_strategy via build_strategy_from_config.
        # We simply assert that errors raise; we don't need the strategy to run.
        return _Reg()

    def test_remove_missing_step_raises_configuration_error(self) -> None:
        from apcore.pipeline_config import build_strategy_from_config

        cfg = {"remove": ["nonexistent_step_xyz"]}
        with pytest.raises(ConfigurationError) as excinfo:
            build_strategy_from_config(cfg, registry=self._stub_registry())
        assert "nonexistent_step_xyz" in str(excinfo.value)

    def test_configure_missing_step_raises_configuration_error(self) -> None:
        from apcore.pipeline_config import build_strategy_from_config

        cfg = {"configure": {"nonexistent_step_xyz": {"timeout_ms": 100}}}
        with pytest.raises(ConfigurationError) as excinfo:
            build_strategy_from_config(cfg, registry=self._stub_registry())
        assert "nonexistent_step_xyz" in str(excinfo.value)

    def test_configure_unknown_field_raises_configuration_error(self) -> None:
        from apcore.builtin_steps import build_standard_strategy
        from apcore.pipeline_config import build_strategy_from_config

        # Use a real built-in step name; trip the unknown-field path
        strategy = build_standard_strategy(registry=self._stub_registry())
        any_existing = strategy.steps[0].name

        cfg = {"configure": {any_existing: {"completely_unknown_field": 42}}}
        with pytest.raises(ConfigurationError) as excinfo:
            build_strategy_from_config(cfg, registry=self._stub_registry())
        assert "completely_unknown_field" in str(excinfo.value)

    @pytest.mark.parametrize("field, value", [("description", "rewritten"), ("removable", False)])
    def test_configure_rejects_step_attributes_outside_the_declared_four(self, field: str, value: object) -> None:
        """A field can be a real attribute of the step and still not be configurable.

        This case did not exist while the gate was ``hasattr(step, key)``, and
        the case above could not have caught the gap: ``completely_unknown_field``
        is an attribute of no step, so it was rejected by accident of the
        attribute lookup rather than by a closed field set. ``description`` and
        ``removable`` ARE attributes of every ``BaseStep``, so the old gate
        accepted and applied them.

        ``pipeline.configure`` accepts exactly the four behavioural modifiers of
        protocol-spec §5.16.1 — ``match_modules``, ``ignore_errors``,
        ``pure``, ``timeout_ms``. Anything else an operator writes here is
        either structural (belongs on the ``pipeline.steps`` entry) or a
        property of the step implementation, and accepting it made a working
        apcore-python config non-portable: apcore-typescript raises on these
        keys and apcore-rust drops them (aiperceivable/apcore#89).
        """
        from apcore.builtin_steps import build_standard_strategy
        from apcore.pipeline_config import build_strategy_from_config

        strategy = build_standard_strategy(registry=self._stub_registry())
        target = strategy.steps[0]
        assert hasattr(target, field), (
            f"this test is only meaningful while '{field}' is a real step attribute — "
            f"it is what separates a closed field set from hasattr()"
        )

        cfg = {"configure": {target.name: {field: value}}}
        with pytest.raises(ConfigurationError) as excinfo:
            build_strategy_from_config(cfg, registry=self._stub_registry())
        message = str(excinfo.value)
        assert field in message, f"the message must name the rejected key; got: {message}"
        for valid in ("match_modules", "ignore_errors", "pure", "timeout_ms"):
            assert valid in message, (
                f"the message must list the four valid fields so an operator can correct the "
                f"config without opening the spec; '{valid}' missing from: {message}"
            )

    def test_step_without_anchor_raises_configuration_error(self) -> None:
        from apcore.pipeline_config import build_strategy_from_config

        cfg = {"steps": [{"name": "free_floating", "handler": "tests.binding_helpers:DummyHandler"}]}
        # Even before reaching the handler import, lacking after/before is a
        # configuration error.
        with pytest.raises(ConfigurationError):
            build_strategy_from_config(cfg, registry=self._stub_registry())

    def test_configuration_error_subclasses_module_error(self) -> None:
        assert issubclass(ConfigurationError, ModuleError)
