"""Conformance tests for Pipeline Hardening (Issue #33, core-executor.md §Pipeline Hardening).

Drives the canonical ``apcore/conformance/fixtures/pipeline_hardening.json``.
Each case needs its own pipeline wiring, so the assertions are hand-written
rather than generated from the fixture; ``TestFixtureCoverage`` at the bottom
holds the two in step, so a case added on the spec side fails here instead of
going unnoticed.

Exercises five fixture cases:
  fail_fast_on_step_error        — §1.1 fail-fast error wrapping
  continue_on_ignored_error      — §1.1 ignore_errors: true continues
  replace_semantic_no_duplicate  — §1.2 configure_step is idempotent
  run_until_stops_early          — §1.4 run_until predicate halts pipeline
  step_lookup_is_not_linear      — §1.5 O(1) _name_to_idx map present
"""

from __future__ import annotations

import asyncio
import sys
from typing import Any

import pytest

from apcore.pipeline import (
    BaseStep,
    ExecutionStrategy,
    PipelineContext,
    PipelineEngine,
    PipelineState,
    PipelineStepError,
    PipelineStepNotFoundError,
    StepResult,
)
from apcore.errors import ModuleTimeoutError
from conformance.canonical_fixtures import (
    case_ids,
    dispatch_or_fail,
    load_fixture,
    reject_unknown_expectations,
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

FIXTURE = "pipeline_hardening.json"

#: canonical case id -> case body, so hand-written assertions still take their
#: values from the fixture rather than from a transcription of it.
_CASES: dict[str, Any] = {case["id"]: case for case in load_fixture(FIXTURE)["test_cases"]}


def _case(case_id: str) -> dict[str, Any]:
    return _CASES[case_id]


class _ContinueStep(BaseStep):
    async def execute(self, ctx: PipelineContext) -> StepResult:
        return StepResult(action="continue")


class _RaisingStep(BaseStep):
    async def execute(self, ctx: PipelineContext) -> StepResult:
        raise ValueError(f"step '{self.name}' intentionally raised")


def _make_simple_strategy(step_names: list[str]) -> ExecutionStrategy:
    steps = [_ContinueStep(n, f"step {n}") for n in step_names]
    return ExecutionStrategy("test", steps)


class _TrackingStep(BaseStep):
    """Records its own name on the shared list handed to it."""

    def __init__(self, name: str, log: list[str], **kwargs: Any) -> None:
        super().__init__(name, **kwargs)
        self._log = log

    async def execute(self, ctx: PipelineContext) -> StepResult:
        self._log.append(self.name)
        return StepResult(action="continue")


class _TrackingRaisingStep(BaseStep):
    """Records its own name, then raises."""

    def __init__(self, name: str, log: list[str], **kwargs: Any) -> None:
        super().__init__(name, **kwargs)
        self._log = log

    async def execute(self, ctx: PipelineContext) -> StepResult:
        self._log.append(self.name)
        raise ValueError(f"step '{self.name}' intentionally raised")


class _SlowStep(BaseStep):
    """A fixture-driven step that can cross or remain within its timeout."""

    def __init__(self, name: str, sleep_ms: int, timeout_ms: int) -> None:
        super().__init__(name, timeout_ms=timeout_ms)
        self._sleep_ms = sleep_ms

    async def execute(self, ctx: PipelineContext) -> StepResult:
        await asyncio.sleep(self._sleep_ms / 1000)
        return StepResult(action="continue")


# ---------------------------------------------------------------------------
# Fixture 1: fail_fast_on_step_error
# ---------------------------------------------------------------------------


class TestFailFastOnStepError:
    """§1.1 — step raises exception → pipeline stops, PipelineStepError raised."""

    @pytest.mark.asyncio
    async def test_fail_fast_on_step_error(self) -> None:
        """apcore#93: every declared value is now read from the fixture.

        This test used to hardcode the step names (``validate_input``, which is
        not even the name the fixture uses), the wire code, and the executed
        set — so ``expected.steps_executed``, ``expected.error_code`` and
        ``expected.stopped`` reached no assertion and mutating any of them in
        the canonical JSON left this green.
        """
        case = _case("fail_fast_on_step_error")
        reject_unknown_expectations(FIXTURE, case, {"expected"})
        params, expected = case["input"], case["expected"]

        # The pipeline is the fixture's own executed set, with the declared
        # step raising, plus one sentinel that must NOT run — the observable
        # that makes "stopped" mean something.
        executed_names: list[str] = list(expected["steps_executed"])
        raising_name = params["step"]
        sentinel = "step_after_the_failure"
        assert raising_name in executed_names, (
            f"[{case['id']}] the raising step {raising_name!r} must be one of the "
            f"steps the fixture says executed: {executed_names}"
        )
        assert params["raises"] is True, f"[{case['id']}] this driver models a raising step"

        steps: list[BaseStep] = [
            _RaisingStep(name, ignore_errors=params["ignore_errors"]) if name == raising_name else _ContinueStep(name)
            for name in executed_names
        ]
        steps.append(_ContinueStep(sentinel))
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="demo.process", inputs={}, context=None)
        engine = PipelineEngine()

        with pytest.raises(PipelineStepError) as exc_info:
            await engine.run(strategy, ctx)

        err = exc_info.value
        assert err.code == expected["error_code"], (
            f"[{case['id']}] fixture declares error code {expected['error_code']!r}, " f"got {err.code!r}"
        )
        assert err.step_name == raising_name
        assert isinstance(err.cause, ValueError)
        assert err.pipeline_trace is not None

        executed = [s.name for s in err.pipeline_trace.steps if not s.skipped]
        assert executed == executed_names, (
            f"[{case['id']}] fixture declares steps_executed={executed_names}, " f"pipeline ran {executed}"
        )
        stopped = sentinel not in executed
        assert stopped is expected["stopped"], (
            f"[{case['id']}] fixture declares stopped={expected['stopped']}; the step "
            f"after the failure {'did not run' if stopped else 'ran'}"
        )

    @pytest.mark.asyncio
    async def test_fail_fast_stops_subsequent_steps(self) -> None:
        steps_run: list[str] = []

        class _TrackingStep(BaseStep):
            async def execute(self, ctx: PipelineContext) -> StepResult:
                steps_run.append(self.name)
                return StepResult(action="continue")

        class _TrackingRaisingStep(BaseStep):
            async def execute(self, ctx: PipelineContext) -> StepResult:
                steps_run.append(self.name)
                raise RuntimeError("boom")

        steps = [
            _TrackingStep("step_a"),
            _TrackingRaisingStep("step_b"),
            _TrackingStep("step_c"),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()

        with pytest.raises(PipelineStepError):
            await engine.run(strategy, ctx)

        assert "step_a" in steps_run
        assert "step_b" in steps_run
        assert "step_c" not in steps_run


# ---------------------------------------------------------------------------
# Fixture 2: continue_on_ignored_error
# ---------------------------------------------------------------------------


class TestContinueOnIgnoredError:
    """§1.1 — ignore_errors: true → step failure logs warning, pipeline continues."""

    @pytest.mark.asyncio
    async def test_continue_on_ignored_error(self) -> None:
        """apcore#93: ``expected.stopped`` / ``expected.continued`` now decide.

        The old body hardcoded the step name and asserted the two outcomes
        positionally, so neither declared boolean reached a comparison: a
        fixture flipped to ``continued: false`` still passed.
        """
        case = _case("continue_on_ignored_error")
        reject_unknown_expectations(FIXTURE, case, {"expected"})
        params, expected = case["input"], case["expected"]
        assert params["raises"] is True, f"[{case['id']}] this driver models a raising step"

        steps_run: list[str] = []
        ignored_name = params["step"]
        after_name = "step_after_the_ignored_failure"

        steps: list[BaseStep] = [
            _TrackingStep("step_before", steps_run),
            _TrackingRaisingStep(ignored_name, steps_run, ignore_errors=params["ignore_errors"]),
            _TrackingStep(after_name, steps_run),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="demo.process", inputs={}, context=None, output={"result": 42})
        engine = PipelineEngine()

        result, trace = await engine.run(strategy, ctx)

        assert result == {"result": 42}
        assert ignored_name in steps_run, "the ignore_errors step must have been entered"

        # The two declared booleans are the contract: did the pipeline halt at
        # the failing step, and did it carry on past it?
        stopped = after_name not in steps_run
        continued = after_name in steps_run and trace.success is True
        assert stopped is expected["stopped"], (
            f"[{case['id']}] fixture declares stopped={expected['stopped']}; steps run " f"were {steps_run}"
        )
        assert continued is expected["continued"], (
            f"[{case['id']}] fixture declares continued={expected['continued']}; steps run "
            f"were {steps_run}, trace.success={trace.success}"
        )

        ignored_step = next(s for s in trace.steps if s.name == ignored_name)
        assert ignored_step.skip_reason == "error_ignored"

    @pytest.mark.asyncio
    async def test_ignored_step_output_is_absent(self) -> None:
        """Step with ignore_errors does not set ctx.output; downstream sees prior value."""

        class _SetOutputStep(BaseStep):
            async def execute(self, ctx: PipelineContext) -> StepResult:
                ctx.output = {"prior": True}
                return StepResult(action="continue")

        class _IgnoredRaisingStep(BaseStep):
            async def execute(self, ctx: PipelineContext) -> StepResult:
                raise RuntimeError("ignored")

        steps = [
            _SetOutputStep("set_output"),
            _IgnoredRaisingStep("bad_step", ignore_errors=True),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()

        result, trace = await engine.run(strategy, ctx)
        assert result == {"prior": True}
        assert trace.success is True


# ---------------------------------------------------------------------------
# Fixture 3: replace_semantic_no_duplicate
# ---------------------------------------------------------------------------


class TestReplaceSemanticNoDuplicate:
    """§1.2 — configure_step is idempotent: calling twice yields exactly one step."""

    def test_configure_step_no_duplicate(self) -> None:
        """apcore#93: driven by ``input.configure_step`` / ``input.times`` and
        asserted against ``expected.step_count_for_name``.

        The literal ``1`` used to be the driver's own, so a fixture declaring
        any other count still passed.
        """
        case = _case("replace_semantic_no_duplicate")
        reject_unknown_expectations(FIXTURE, case, {"expected"})
        params, expected = case["input"], case["expected"]
        name = params["configure_step"]

        strategy = _make_simple_strategy(["a", name, "b"])
        replacements = [_ContinueStep(name, f"replacement {i}") for i in range(params["times"])]
        for replacement in replacements:
            strategy.configure_step(name, replacement)

        count = sum(1 for s in strategy.steps if s.name == name)
        assert count == expected["step_count_for_name"], (
            f"[{case['id']}] configuring {name!r} {params['times']}x left {count} step(s); "
            f"the fixture declares {expected['step_count_for_name']}"
        )
        # The surviving step must be the LAST configured one — "replace", not
        # "keep the original and drop the update".
        assert strategy.steps[strategy._name_to_idx[name]] is replacements[-1]

    def test_configure_step_twice_still_one(self) -> None:
        strategy = _make_simple_strategy(["a", "validate_input", "b"])

        first = _ContinueStep("validate_input", "first replacement")
        second = _ContinueStep("validate_input", "second replacement")
        strategy.configure_step("validate_input", first)
        strategy.configure_step("validate_input", second)

        count = sum(1 for s in strategy.steps if s.name == "validate_input")
        assert count == 1
        assert strategy.steps[strategy._name_to_idx["validate_input"]] is second

    def test_configure_step_preserves_position(self) -> None:
        strategy = _make_simple_strategy(["a", "validate_input", "b"])
        original_idx = strategy._name_to_idx["validate_input"]

        new_step = _ContinueStep("validate_input", "replacement")
        strategy.configure_step("validate_input", new_step)

        assert strategy._name_to_idx["validate_input"] == original_idx

    def test_configure_step_not_found_raises(self) -> None:
        strategy = _make_simple_strategy(["a", "b"])
        with pytest.raises(PipelineStepNotFoundError) as exc_info:
            strategy.configure_step("nonexistent", _ContinueStep("nonexistent"))
        assert exc_info.value.code == "PIPELINE_STEP_NOT_FOUND"


# ---------------------------------------------------------------------------
# Fixture 4: run_until_stops_early
# ---------------------------------------------------------------------------


class TestRunUntilStopsEarly:
    """§1.4 — run_until predicate halts pipeline after matching step."""

    @pytest.mark.asyncio
    async def test_run_until_stops_after_module_lookup(self) -> None:
        case = _case("run_until_stops_early")
        expected = case["expected"]
        stop_after = case["input"]["run_until_after"]

        steps_run: list[str] = []

        class _TrackingStep(BaseStep):
            async def execute(self, ctx: PipelineContext) -> StepResult:
                steps_run.append(self.name)
                return StepResult(action="continue")

        step_names = ["context_creation", "module_lookup", "execute", "return_result"]
        steps = [_TrackingStep(name) for name in step_names]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()

        def stop_after_module_lookup(state: PipelineState) -> bool:
            return state.step_name == stop_after

        _, trace = await engine.run(strategy, ctx, run_until=stop_after_module_lookup)

        # `steps_after_skipped`: every step positioned after the halting one
        # must not have run. Derived from the strategy's own step list, so
        # extending the pipeline cannot quietly narrow what this checks.
        after_halt = step_names[step_names.index(expected["last_step_executed"]) + 1 :]
        steps_after_skipped = not any(name in steps_run for name in after_halt)
        assert steps_after_skipped is expected["steps_after_skipped"], f"ran {steps_run!r}"
        assert steps_run[-1] == expected["last_step_executed"]
        assert trace.success is True

    @pytest.mark.asyncio
    async def test_run_until_state_has_correct_outputs(self) -> None:
        """A ``run_until`` predicate DOES see the step it is judging in ``state.outputs``.

        This is the deliberate exception to the rule that ``state.outputs``
        holds exactly the steps completed *before* ``state.step_name``
        (middleware-system.md § "What ``state.outputs`` contains"). That rule
        governs the three ``StepMiddleware`` hooks, which observe a step from
        inside its own execution; ``run_until`` is evaluated *after* the step
        completes and exists precisely to decide "have we produced what we
        came for yet?" — a question it cannot answer without the step that
        just ran. The engine therefore snapshots ``step_outputs`` between the
        ``after_step`` hook and this predicate; do not "fix" the assertions
        below to match the middleware rule.
        """
        observed_states: list[PipelineState] = []

        class _OutputStep(BaseStep):
            def __init__(self, name: str, value: Any) -> None:
                super().__init__(name)
                self._value = value

            async def execute(self, ctx: PipelineContext) -> StepResult:
                ctx.output = self._value
                return StepResult(action="continue")

        steps = [
            _OutputStep("step_a", {"a": 1}),
            _OutputStep("step_b", {"b": 2}),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()

        def capture_and_continue(state: PipelineState) -> bool:
            observed_states.append(
                PipelineState(
                    step_name=state.step_name,
                    outputs=dict(state.outputs),
                    context=state.context,
                )
            )
            return False

        await engine.run(strategy, ctx, run_until=capture_and_continue)

        assert len(observed_states) == 2
        assert observed_states[0].step_name == "step_a"
        assert observed_states[0].outputs["step_a"] == {"a": 1}
        assert observed_states[1].step_name == "step_b"
        assert observed_states[1].outputs["step_b"] == {"b": 2}

    @pytest.mark.asyncio
    async def test_run_until_never_true_runs_full_pipeline(self) -> None:
        steps = [_ContinueStep(f"step_{i}") for i in range(5)]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()

        _, trace = await engine.run(strategy, ctx, run_until=lambda _: False)

        assert trace.success is True
        assert len([s for s in trace.steps if not s.skipped]) == 5

    @pytest.mark.asyncio
    async def test_run_until_via_pipeline_context(self) -> None:
        """run_until can be passed through PipelineContext.run_until."""
        steps_run: list[str] = []

        class _TrackStep(BaseStep):
            async def execute(self, ctx: PipelineContext) -> StepResult:
                steps_run.append(self.name)
                return StepResult(action="continue")

        steps = [_TrackStep("a"), _TrackStep("b"), _TrackStep("c")]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(
            module_id="m",
            inputs={},
            context=None,
            run_until=lambda state: state.step_name == "b",
        )
        engine = PipelineEngine()

        await engine.run(strategy, ctx)

        assert steps_run == ["a", "b"]


# ---------------------------------------------------------------------------
# Fixture 5: step_lookup_is_not_linear (O(1) compliance)
# ---------------------------------------------------------------------------


class _CountingIndex(dict):  # type: ignore[type-arg]
    """A ``_name_to_idx`` map that records every lookup performed against it.

    Lets the driver state the O(1) claim as something observable: resolving a
    ``skip_to`` target must cost exactly ONE map probe, whatever the distance
    skipped. A linear scan would perform zero (it has no map to probe) and a
    per-candidate probe would perform N.
    """

    def __init__(self, base: dict[str, int]) -> None:
        super().__init__(base)
        self.lookups: list[str] = []

    def get(self, key: Any, default: Any = None) -> Any:
        self.lookups.append(key)
        return super().get(key, default)

    def __getitem__(self, key: Any) -> Any:
        self.lookups.append(key)
        return super().__getitem__(key)


async def _probe_skip_lookup_cost(step_count: int) -> list[str]:
    """Run a ``step_count``-step pipeline that skips from the first step to the
    last, and return every name the engine looked up in the index."""

    class _SkipStep(BaseStep):
        def __init__(self, name: str, target: str) -> None:
            super().__init__(name)
            self._target = target

        async def execute(self, ctx: PipelineContext) -> StepResult:
            return StepResult(action="skip_to", skip_to=self._target)

    names = [f"step_{i}" for i in range(step_count)]
    steps: list[BaseStep] = [_ContinueStep(n) for n in names]
    steps[0] = _SkipStep(names[0], names[-1])
    strategy = ExecutionStrategy("test", steps)
    index = _CountingIndex(strategy._name_to_idx)
    strategy._name_to_idx = index
    await PipelineEngine().run(strategy, PipelineContext(module_id="m", inputs={}, context=None))
    return index.lookups


async def _assert_o1_step_lookup(case_id: str, step_count: int) -> None:
    """The observable form of ``lookup_complexity: "O(1)"``.

    "verified by implementation" (the fixture's own words) used to mean the
    driver asserted that a ``dict`` attribute exists — true of an
    implementation that keeps the map and still scans the list. The cost of
    resolution is measured here instead, at the fixture's ``step_count`` and at
    a far larger one: constant means the two agree.
    """
    strategy = _make_simple_strategy([f"step_{i}" for i in range(step_count)])
    assert set(strategy._name_to_idx) == {
        f"step_{i}" for i in range(step_count)
    }, f"[{case_id}] the index must cover every one of the {step_count} steps"

    at_declared = await _probe_skip_lookup_cost(step_count)
    at_scale = await _probe_skip_lookup_cost(step_count * 20)

    assert at_declared == [f"step_{step_count - 1}"], (
        f"[{case_id}] resolving a skip_to target must cost exactly one index probe; " f"the engine probed {at_declared}"
    )
    assert len(at_scale) == len(at_declared), (
        f"[{case_id}] lookup cost grew with the pipeline: {len(at_declared)} probe(s) at "
        f"{step_count} steps, {len(at_scale)} at {step_count * 20} — that is not O(1)"
    )


#: fixture ``expected.lookup_complexity`` -> the check that makes it observable.
#: A dispatch with no ``else`` would let an unrecognised complexity skip the
#: assertion entirely, so ``dispatch_or_fail`` hard-fails instead (apcore#93).
_LOOKUP_COMPLEXITY_CHECKS: dict[str, Any] = {"O(1)": _assert_o1_step_lookup}


class TestStepLookupIsNotLinear:
    """§1.5 — ExecutionStrategy maintains a _name_to_idx hash map."""

    @pytest.mark.asyncio
    async def test_declared_lookup_complexity_holds(self) -> None:
        """apcore#93: ``expected.lookup_complexity`` reaches an assertion.

        Every assertion in this class used to be about the driver's own
        hand-built strategies; the fixture's declared complexity and its
        ``step_count`` were read by nothing.
        """
        case = _case("step_lookup_is_not_linear")
        reject_unknown_expectations(FIXTURE, case, {"expected"})
        check = dispatch_or_fail(
            FIXTURE,
            case["id"],
            case["expected"]["lookup_complexity"],
            _LOOKUP_COMPLEXITY_CHECKS,
            "lookup complexity",
        )
        await check(case["id"], case["input"]["step_count"])

    def test_name_to_idx_exists_and_is_dict(self) -> None:
        strategy = _make_simple_strategy(["a", "b", "c"])
        assert isinstance(strategy._name_to_idx, dict)

    def test_name_to_idx_covers_all_steps(self) -> None:
        names = ["context_creation", "module_lookup", "execute", "return_result"]
        strategy = _make_simple_strategy(names)
        assert set(strategy._name_to_idx.keys()) == set(names)

    def test_name_to_idx_has_correct_indices(self) -> None:
        names = ["a", "b", "c", "d"]
        strategy = _make_simple_strategy(names)
        for expected_idx, name in enumerate(names):
            assert strategy._name_to_idx[name] == expected_idx

    def test_name_to_idx_updated_after_insert(self) -> None:
        strategy = _make_simple_strategy(["a", "c"])
        strategy.insert_after("a", _ContinueStep("b"))
        assert strategy._name_to_idx["a"] == 0
        assert strategy._name_to_idx["b"] == 1
        assert strategy._name_to_idx["c"] == 2

    def test_name_to_idx_updated_after_remove(self) -> None:
        strategy = _make_simple_strategy(["a", "b", "c"])
        strategy.remove("b")
        assert "b" not in strategy._name_to_idx
        assert strategy._name_to_idx["a"] == 0
        assert strategy._name_to_idx["c"] == 1

    def test_name_to_idx_updated_after_replace(self) -> None:
        strategy = _make_simple_strategy(["a", "b", "c"])
        strategy.replace("b", _ContinueStep("b", "replaced"))
        assert strategy._name_to_idx["b"] == 1

    @pytest.mark.asyncio
    async def test_skip_to_uses_o1_lookup(self) -> None:
        """skip_to with 9 steps uses O(1) lookup (verified by behavior)."""

        class _SkipStep(BaseStep):
            def __init__(self, name: str, target: str) -> None:
                super().__init__(name)
                self._target = target

            async def execute(self, ctx: PipelineContext) -> StepResult:
                return StepResult(action="skip_to", skip_to=self._target)

        base_steps: list[BaseStep] = [_ContinueStep(f"step_{i}") for i in range(9)]
        base_steps[0] = _SkipStep("step_0", "step_8")
        strategy = ExecutionStrategy("test", base_steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()

        _, trace = await engine.run(strategy, ctx)

        executed_non_skipped = [s.name for s in trace.steps if not s.skipped]
        assert "step_0" in executed_non_skipped
        assert "step_8" in executed_non_skipped
        skipped = [s.name for s in trace.steps if s.skipped]
        for i in range(1, 8):
            assert f"step_{i}" in skipped

    @pytest.mark.asyncio
    async def test_skip_to_backward_target_raises(self) -> None:
        """skip_to a target at or before the current step raises StepNotFoundError.

        Before the O(1) refactor, the linear scan implicitly rejected backward
        targets because it only searched forward from i+1. The O(1) lookup must
        enforce the same guard explicitly to prevent infinite loops.
        """

        class _BackwardSkipStep(BaseStep):
            async def execute(self, ctx: PipelineContext) -> StepResult:
                return StepResult(action="skip_to", skip_to="step_a")

        steps: list[BaseStep] = [
            _ContinueStep("step_a"),
            _ContinueStep("step_b"),
            _BackwardSkipStep("step_c"),
        ]
        strategy = ExecutionStrategy("test", steps)
        ctx = PipelineContext(module_id="m", inputs={}, context=None)
        engine = PipelineEngine()

        from apcore.pipeline import StepNotFoundError

        with pytest.raises(StepNotFoundError):
            await engine.run(strategy, ctx)


class TestStepTimeout:
    """D-142: a configured step timeout reaches callers as MODULE_TIMEOUT."""

    @pytest.mark.asyncio
    async def test_step_timeout_raises_module_timeout(self) -> None:
        case = _case("step_timeout_raises_module_timeout")
        spec = case["input"]
        custom = spec["custom_step"]
        step = _SlowStep(custom["name"], custom["sleep_ms"], spec["timeout_ms"])
        strategy = ExecutionStrategy("test", [step])
        ctx = PipelineContext(module_id="fixture.module", inputs={}, context=None)

        with pytest.raises(PipelineStepError) as exc_info:
            await PipelineEngine().run(strategy, ctx)

        cause = exc_info.value.cause
        assert isinstance(cause, ModuleTimeoutError)
        assert cause.code == case["expected"]["call_error_code"]

    @pytest.mark.asyncio
    async def test_step_within_its_timeout_runs(self) -> None:
        case = _case("step_within_its_timeout_runs")
        spec = case["input"]
        custom = spec["custom_step"]
        step = _SlowStep(custom["name"], custom["sleep_ms"], spec["timeout_ms"])
        strategy = ExecutionStrategy("test", [step])
        ctx = PipelineContext(module_id="fixture.module", inputs={}, context=None)

        _, trace = await PipelineEngine().run(strategy, ctx)

        assert trace.steps[-1].name == custom["name"]
        assert trace.steps[-1].result.action == "continue"
        assert trace.steps[-1].skipped is False
        assert case["expected"]["call_succeeds"] is True


# ---------------------------------------------------------------------------
# Fixture coverage guard
# ---------------------------------------------------------------------------


class TestFixtureCoverage:
    """Every case in the canonical fixture has a driver class in this file.

    The assertions above are hand-written rather than generated from the fixture
    (each case needs its own pipeline wiring, which does not reduce to a
    schema-in / verdict-out loop). That is fine, but it used to mean the fixture
    was named only in a module docstring while a vendored copy sat unread in
    ``tests/conformance/fixtures/`` — so a case added on the spec side left no
    trace here at all. This guard closes that gap: a new canonical case fails
    until someone writes the class for it.
    """

    FIXTURE = "pipeline_hardening.json"

    #: canonical case id → the class in this module that asserts it.
    COVERED: dict[str, str] = {
        "fail_fast_on_step_error": "TestFailFastOnStepError",
        "continue_on_ignored_error": "TestContinueOnIgnoredError",
        "replace_semantic_no_duplicate": "TestReplaceSemanticNoDuplicate",
        "run_until_stops_early": "TestRunUntilStopsEarly",
        "step_lookup_is_not_linear": "TestStepLookupIsNotLinear",
        "step_timeout_raises_module_timeout": "TestStepTimeout",
        "step_within_its_timeout_runs": "TestStepTimeout",
    }

    def test_every_canonical_case_is_claimed(self) -> None:
        canonical = set(case_ids(self.FIXTURE))
        claimed = set(self.COVERED)
        assert canonical - claimed == set(), f"canonical fixture {self.FIXTURE} gained case(s) with no driver here"
        assert claimed - canonical == set(), f"this file claims case(s) {self.FIXTURE} no longer defines"

    def test_every_claimed_class_exists(self) -> None:
        module = sys.modules[__name__]
        missing = [cls for cls in self.COVERED.values() if not hasattr(module, cls)]
        assert missing == [], f"claimed driver class(es) not defined: {missing}"
