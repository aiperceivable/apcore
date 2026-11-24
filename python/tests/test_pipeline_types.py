"""Tests for execution pipeline core types."""

from __future__ import annotations

import pytest

from apcore.errors import ModuleError
from apcore.pipeline import (
    BaseStep,
    ExecutionStrategy,
    PipelineAbortError,
    PipelineContext,
    PipelineEngine,
    PipelineTrace,
    Step,
    StepNameDuplicateError,
    StepNotFoundError,
    StepNotRemovableError,
    StepNotReplaceableError,
    StepResult,
    StepTrace,
    StrategyInfo,
    StrategyNotFoundError,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


class DummyStep(BaseStep):
    """Minimal BaseStep subclass for testing."""

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


class ProtocolStep:
    """Plain class that satisfies the Step protocol via structural typing."""

    def __init__(self, name: str, description: str = "", removable: bool = True, replaceable: bool = True) -> None:
        self._name = name
        self._description = description
        self._removable = removable
        self._replaceable = replaceable

    @property
    def name(self) -> str:
        return self._name

    @property
    def description(self) -> str:
        return self._description

    @property
    def removable(self) -> bool:
        return self._removable

    @property
    def replaceable(self) -> bool:
        return self._replaceable

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


def _make_steps(*names: str, removable: bool = True, replaceable: bool = True) -> list[DummyStep]:
    return [DummyStep(n, f"Step {n}", removable=removable, replaceable=replaceable) for n in names]


# ---------------------------------------------------------------------------
# StepResult
# ---------------------------------------------------------------------------


class TestStepResult:
    def test_step_result_continue(self) -> None:
        r = StepResult(action="continue")
        assert r.action == "continue"
        assert r.skip_to is None
        assert r.explanation is None

    def test_step_result_skip_to(self) -> None:
        r = StepResult(action="skip_to", skip_to="validate_output")
        assert r.action == "skip_to"
        assert r.skip_to == "validate_output"

    def test_step_result_abort(self) -> None:
        r = StepResult(action="abort", explanation="bad input", alternatives=["retry", "fallback"])
        assert r.action == "abort"
        assert r.explanation == "bad input"
        assert r.alternatives == ["retry", "fallback"]

    def test_step_result_confidence(self) -> None:
        r = StepResult(action="continue", confidence=0.95)
        assert r.confidence == 0.95


# ---------------------------------------------------------------------------
# PipelineContext
# ---------------------------------------------------------------------------


class TestPipelineContext:
    def test_pipeline_context_creation(self) -> None:
        ctx = PipelineContext(module_id="test.module", inputs={"key": "value"}, context=None)
        assert ctx.module_id == "test.module"
        assert ctx.inputs == {"key": "value"}
        assert ctx.context is None

    def test_pipeline_context_resolved_fields_initially_none(self) -> None:
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        assert ctx.module is None
        assert ctx.validated_inputs is None
        assert ctx.output is None
        assert ctx.validated_output is None
        assert ctx.strategy is None
        assert ctx.trace is None


# ---------------------------------------------------------------------------
# StepTrace / PipelineTrace
# ---------------------------------------------------------------------------


class TestTraceTypes:
    def test_step_trace_creation(self) -> None:
        r = StepResult(action="continue")
        st = StepTrace(name="resolve", duration_ms=12.5, result=r, skipped=False, decision_point=True)
        assert st.name == "resolve"
        assert st.duration_ms == 12.5
        assert st.result is r
        assert st.skipped is False
        assert st.decision_point is True

    def test_pipeline_trace_creation(self) -> None:
        pt = PipelineTrace(module_id="m", strategy_name="default")
        assert pt.module_id == "m"
        assert pt.strategy_name == "default"
        assert pt.steps == []
        assert pt.total_duration_ms == 0.0
        assert pt.success is False


# ---------------------------------------------------------------------------
# StrategyInfo
# ---------------------------------------------------------------------------


class TestStrategyInfo:
    def test_strategy_info_creation(self) -> None:
        si = StrategyInfo(name="default", step_count=3, step_names=["a", "b", "c"], description="a -> b -> c")
        assert si.name == "default"
        assert si.step_count == 3
        assert si.step_names == ["a", "b", "c"]
        assert si.description == "a -> b -> c"


# ---------------------------------------------------------------------------
# ExecutionStrategy
# ---------------------------------------------------------------------------


class TestExecutionStrategy:
    def test_creation_with_steps(self) -> None:
        steps = _make_steps("resolve", "validate", "execute")
        strategy = ExecutionStrategy("default", steps)
        assert strategy.name == "default"
        assert len(strategy.steps) == 3

    def test_step_names(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a", "b", "c"))
        assert strategy.step_names() == ["a", "b", "c"]

    def test_info(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a", "b"))
        info = strategy.info()
        assert info.name == "s"
        assert info.step_count == 2
        assert info.step_names == ["a", "b"]
        assert "\u2192" in info.description

    def test_insert_after(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a", "c"))
        strategy.insert_after("a", DummyStep("b", "inserted"))
        assert strategy.step_names() == ["a", "b", "c"]

    def test_insert_before(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a", "c"))
        strategy.insert_before("c", DummyStep("b", "inserted"))
        assert strategy.step_names() == ["a", "b", "c"]

    def test_remove(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a", "b", "c"))
        strategy.remove("b")
        assert strategy.step_names() == ["a", "c"]

    def test_remove_non_removable_raises(self) -> None:
        steps = _make_steps("a", "b", removable=False)
        strategy = ExecutionStrategy("s", steps)
        with pytest.raises(StepNotRemovableError):
            strategy.remove("a")

    def test_replace(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a", "b"))
        new_step = DummyStep("b", "replaced")
        strategy.replace("b", new_step)
        assert strategy.steps[1] is new_step

    def test_replace_non_replaceable_raises(self) -> None:
        steps = _make_steps("a", "b", replaceable=False)
        strategy = ExecutionStrategy("s", steps)
        with pytest.raises(StepNotReplaceableError):
            strategy.replace("a", DummyStep("a", "new"))

    def test_insert_duplicate_raises(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a", "b"))
        with pytest.raises(StepNameDuplicateError):
            strategy.insert_after("a", DummyStep("b", "dup"))

    def test_insert_before_duplicate_raises(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a", "b"))
        with pytest.raises(StepNameDuplicateError):
            strategy.insert_before("b", DummyStep("a", "dup"))

    def test_constructor_duplicate_raises(self) -> None:
        with pytest.raises(StepNameDuplicateError):
            ExecutionStrategy("s", _make_steps("a", "a"))

    def test_remove_not_found_raises(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a"))
        with pytest.raises(StepNotFoundError):
            strategy.remove("missing")

    def test_insert_after_not_found_raises(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a"))
        with pytest.raises(StepNotFoundError):
            strategy.insert_after("missing", DummyStep("b", "new"))

    def test_replace_not_found_raises(self) -> None:
        strategy = ExecutionStrategy("s", _make_steps("a"))
        with pytest.raises(StepNotFoundError):
            strategy.replace("missing", DummyStep("x", "new"))


# ---------------------------------------------------------------------------
# BaseStep
# ---------------------------------------------------------------------------


class TestBaseStep:
    def test_base_step_subclass(self) -> None:
        step = DummyStep("resolve", "Resolve the module", removable=False, replaceable=True)
        assert step.name == "resolve"
        assert step.description == "Resolve the module"
        assert step.removable is False
        assert step.replaceable is True


# ---------------------------------------------------------------------------
# Step Protocol
# ---------------------------------------------------------------------------


class TestStepProtocol:
    def test_step_protocol_structural_typing(self) -> None:
        ps = ProtocolStep("test")
        assert isinstance(ps, Step)

    def test_base_step_satisfies_protocol(self) -> None:
        ds = DummyStep("test", "desc")
        assert isinstance(ds, Step)


# ---------------------------------------------------------------------------
# Error types
# ---------------------------------------------------------------------------


class TestPipelineErrors:
    def test_error_types_extend_module_error(self) -> None:
        error_classes = [
            PipelineAbortError,
            StepNotFoundError,
            StepNotRemovableError,
            StepNotReplaceableError,
            StepNameDuplicateError,
            StrategyNotFoundError,
        ]
        for cls in error_classes:
            assert issubclass(cls, ModuleError), f"{cls.__name__} does not extend ModuleError"

    def test_pipeline_abort_error_carries_trace(self) -> None:
        trace = PipelineTrace(module_id="m", strategy_name="default")
        err = PipelineAbortError(
            step="validate",
            explanation="invalid input",
            alternatives=["retry"],
            trace=trace,
        )
        assert err.step == "validate"
        assert err.explanation == "invalid input"
        assert err.alternatives == ["retry"]
        assert err.pipeline_trace is trace
        assert err.code == "PIPELINE_ABORT"
        assert "validate" in str(err)

    def test_step_not_found_error_code(self) -> None:
        err = StepNotFoundError("step missing")
        assert err.code == "STEP_NOT_FOUND"

    def test_strategy_not_found_error_code(self) -> None:
        err = StrategyNotFoundError("no such strategy")
        assert err.code == "STRATEGY_NOT_FOUND"


# ---------------------------------------------------------------------------
# PipelineEngine
# ---------------------------------------------------------------------------


class _ContinueStep(BaseStep):
    """Step that always continues."""

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


class _AbortStep(BaseStep):
    """Step that always aborts."""

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="abort", explanation="forced abort", alternatives=["retry"])


class _SkipToStep(BaseStep):
    """Step that skips to a named target."""

    def __init__(self, name: str, target: str) -> None:
        super().__init__(name, f"skip to {target}")
        self._target = target

    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="skip_to", skip_to=self._target)


class TestPipelineEngine:
    @pytest.mark.asyncio
    async def test_run_all_continue_success(self) -> None:
        steps = [
            _ContinueStep("step_a", "A"),
            _ContinueStep("step_b", "B"),
            _ContinueStep("step_c", "C"),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None, output={"ok": True})
        engine = PipelineEngine()
        result, trace = await engine.run(strategy, ctx)
        assert result == {"ok": True}
        assert trace.success is True
        assert len(trace.steps) == 3
        assert [s.name for s in trace.steps] == ["step_a", "step_b", "step_c"]

    @pytest.mark.asyncio
    async def test_run_abort_raises(self) -> None:
        steps = [
            _ContinueStep("step_a", "A"),
            _AbortStep("step_b", "B"),
            _ContinueStep("step_c", "C"),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()
        with pytest.raises(PipelineAbortError) as exc_info:
            await engine.run(strategy, ctx)
        assert exc_info.value.step == "step_b"
        assert exc_info.value.explanation == "forced abort"
        assert exc_info.value.alternatives == ["retry"]

    @pytest.mark.asyncio
    async def test_run_skip_to_records_skipped_steps(self) -> None:
        steps = [
            _SkipToStep("step_a", target="step_d"),
            _ContinueStep("step_b", "B"),
            _ContinueStep("step_c", "C"),
            _ContinueStep("step_d", "D"),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None, output={"v": 1})
        engine = PipelineEngine()
        result, trace = await engine.run(strategy, ctx)
        assert trace.success is True
        # step_a (skip_to) + step_b (skipped) + step_c (skipped) + step_d (continue)
        assert len(trace.steps) == 4
        assert trace.steps[0].name == "step_a"
        assert trace.steps[0].skipped is False
        assert trace.steps[1].name == "step_b"
        assert trace.steps[1].skipped is True
        assert trace.steps[2].name == "step_c"
        assert trace.steps[2].skipped is True
        assert trace.steps[3].name == "step_d"
        assert trace.steps[3].skipped is False

    @pytest.mark.asyncio
    async def test_trace_step_count_and_duration(self) -> None:
        steps = [
            _ContinueStep("step_a", "A"),
            _ContinueStep("step_b", "B"),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()
        _, trace = await engine.run(strategy, ctx)
        assert len(trace.steps) == 2
        assert trace.total_duration_ms > 0
        for st in trace.steps:
            assert st.duration_ms >= 0

    @pytest.mark.asyncio
    async def test_empty_strategy_returns_none_success(self) -> None:
        strategy = ExecutionStrategy("empty", [])
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()
        result, trace = await engine.run(strategy, ctx)
        assert result is None
        assert trace.success is True
        assert len(trace.steps) == 0
