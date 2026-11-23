"""Drive `pipeline_step_middleware.json` — StepMiddleware lifecycle (Issue #33 §2.2).

The fixture pins nine behaviours from middleware-system.md § Pipeline Step
Middleware:

* ``before_step(step_name, state)`` runs in registration order and is an
  **observation** hook — a Step is ``execute(ctx)``, so there is no ``inputs``
  parameter for a return value to replace;
* ``after_step`` and ``on_step_error`` run in **reverse** registration order
  (onion model);
* the first ``on_step_error`` returning a non-``None`` value supplies the
  recovery result and short-circuits the remaining handlers;
* ``after_step`` fires after a **recovered** step body as well as a naturally
  successful one, so the onion closes and the recovery path does not leak;
* an exception raised inside ``before_step`` is wrapped in
  ``MiddlewareChainError``, stops the chain before the step body runs, and
  triggers ``on_step_error`` on only the already-executed middlewares;
* that ``before_step`` failure is **terminal** — the ``on_step_error`` return
  value is discarded, ``after_step`` does not fire, and the step's
  ``ignore_errors`` does not apply;
* ``state.outputs`` holds exactly the steps that completed BEFORE the current
  one — the current step is never present, in any of the three hooks. Asserted
  at BOTH snapshot sites: the natural success path
  (``state_outputs_excludes_the_current_step_in_every_hook``) and the recovery
  path (``after_step_fires_after_a_recovered_step``), which the first case
  cannot reach because it recovers from nothing.

All nine are implemented by ``apcore.pipeline.PipelineEngine``; none of the
cases is an xfail.
"""

from __future__ import annotations

import warnings
from typing import Any

import pytest

from apcore.context import Context
from apcore.errors import ModuleError
from apcore.pipeline import (
    BaseStep,
    ExecutionStrategy,
    PipelineContext,
    PipelineEngine,
    StepResult,
)

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("pipeline_step_middleware.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}


class _OkStep(BaseStep):
    """Step that records the inputs it observed and succeeds."""

    def __init__(self, name: str, seen: list[dict[str, Any]]) -> None:
        super().__init__(name=name)
        self._seen = seen

    async def execute(self, ctx: PipelineContext) -> StepResult:
        self._seen.append(dict(ctx.inputs or {}))
        return StepResult(action="continue")


class _FailStep(BaseStep):
    def __init__(self, name: str, exc: Exception) -> None:
        super().__init__(name=name)
        self._exc = exc

    async def execute(self, ctx: PipelineContext) -> StepResult:
        raise self._exc


class _RecordingStep(BaseStep):
    """Step that appends its own name to a shared log when its body runs.

    The log is how ``before_step_failure_recovery_is_discarded`` observes that
    the FOLLOWING step never executed — the fixture's ``driver_contract``
    requires that, not merely that an error was raised.
    """

    def __init__(self, name: str, executed: list[str], *, ignore_errors: bool = False) -> None:
        super().__init__(name=name, ignore_errors=ignore_errors)
        self._executed = executed

    async def execute(self, ctx: PipelineContext) -> StepResult:
        self._executed.append(self.name)
        return StepResult(action="continue")


class _ValueStep(BaseStep):
    """Step that publishes a fixed output, or raises before publishing one.

    The raising variant models the fixture's ``third_step_raises``: the step
    ran and failed, so it never produced an output — which is precisely why
    its name must not appear in ``state.outputs`` when ``on_step_error`` fires.
    """

    def __init__(self, name: str, value: Any, *, raises: Exception | None = None) -> None:
        super().__init__(name=name)
        self._value = value
        self._raises = raises

    async def execute(self, ctx: PipelineContext) -> StepResult:
        if self._raises is not None:
            raise self._raises
        ctx.output = self._value
        return StepResult(action="continue")


def _pipeline_context(inputs: dict[str, Any] | None = None) -> PipelineContext:
    return PipelineContext(
        module_id="executor.conformance.step_mw", inputs=dict(inputs or {}), context=Context.create()
    )


def _fixture_error(case: dict[str, Any]) -> ModuleError:
    """Build the exception the fixture's `step_raises` block describes."""
    spec = case["input"]["step_raises"]
    return ModuleError(code=spec["code"], message=spec["message"])


async def _run(strategy: ExecutionStrategy, ctx: PipelineContext) -> Any:
    return await PipelineEngine().run(strategy, ctx)


class TestInvocationOrder:
    async def test_before_after_invocation_order(self) -> None:
        case = CASES["before_after_invocation_order"]
        step_name = case["input"]["step"]
        before: list[str] = []
        after: list[str] = []

        class _Recorder:
            def __init__(self, label: str) -> None:
                self._label = label

            def before_step(self, step_name: str, state: Any) -> None:
                before.append(self._label)

            def after_step(self, step_name: str, state: Any, result: StepResult) -> None:
                after.append(self._label)

        strategy = ExecutionStrategy("conformance", [_OkStep(step_name, [])])
        for label in case["input"]["register_order"]:
            strategy.add_step_middleware(_Recorder(label))

        await _run(strategy, _pipeline_context())

        assert before == case["expected"]["before_step_order"], (
            f"[{case['id']}] before_step must run in registration order: "
            f"got {before}, expected {case['expected']['before_step_order']}"
        )
        assert after == case["expected"]["after_step_order"], (
            f"[{case['id']}] after_step must run in REVERSE registration order (onion model): "
            f"got {after}, expected {case['expected']['after_step_order']}"
        )


class TestErrorHandling:
    async def test_on_step_error_recovery_short_circuits(self) -> None:
        case = CASES["on_step_error_recovery_short_circuits"]
        invoked: list[str] = []
        returns: dict[str, Any] = case["input"]["on_step_error_returns"]

        class _Handler:
            def __init__(self, label: str) -> None:
                self._label = label

            def on_step_error(self, step_name: str, state: Any, error: Exception) -> Any:
                invoked.append(self._label)
                return returns[self._label]

        strategy = ExecutionStrategy("conformance", [_FailStep(case["input"]["step"], _fixture_error(case))])
        for label in case["input"]["register_order"]:
            strategy.add_step_middleware(_Handler(label))

        ctx = _pipeline_context()
        propagated = False
        pipeline_output: Any = None
        try:
            pipeline_output, _trace = await _run(strategy, ctx)
        except Exception:  # noqa: BLE001 - the fixture says the error must not escape
            propagated = True

        assert (
            propagated is case["expected"]["error_propagated"]
        ), f"[{case['id']}] error_propagated mismatch: recovery must swallow the step error"
        assert invoked == case["expected"]["on_step_error_invoked"], (
            f"[{case['id']}] on_step_error invocation: got {invoked}, "
            f"expected {case['expected']['on_step_error_invoked']} (reverse order, first-recovery-wins)"
        )
        # `step_output` is a MUST (middleware-system.md Normative Rules): the
        # recovery value BECOMES the step's output. Assert it on the value the
        # engine actually returns — ctx.output alone would pass against an
        # implementation that wrote the context but dropped the value on the way
        # out.
        assert pipeline_output == case["expected"]["step_output"], (
            f"[{case['id']}] the recovery value must become the pipeline output: "
            f"got {pipeline_output!r}, expected {case['expected']['step_output']!r}"
        )
        assert ctx.output == case["expected"]["step_output"], (
            f"[{case['id']}] the recovery value must become the step output on the context: "
            f"got {ctx.output!r}, expected {case['expected']['step_output']!r}"
        )

    async def test_on_step_error_null_propagates_error(self) -> None:
        case = CASES["on_step_error_null_propagates_error"]
        invoked: list[str] = []

        class _Handler:
            def __init__(self, label: str) -> None:
                self._label = label

            def on_step_error(self, step_name: str, state: Any, error: Exception) -> Any:
                invoked.append(self._label)
                return None

        strategy = ExecutionStrategy("conformance", [_FailStep(case["input"]["step"], _fixture_error(case))])
        for label in case["input"]["register_order"]:
            strategy.add_step_middleware(_Handler(label))

        # Catch by ModuleError, the carrier of the wire `code`, NOT by
        # PipelineStepError: the fixture pins a CODE, and pinning the class
        # would re-import the very anti-pattern issue #81 exists to retire
        # (apcore-rust has no such type, only ErrorCode variants).
        with pytest.raises(ModuleError) as excinfo:
            await _run(strategy, _pipeline_context())

        assert excinfo.value.code == case["expected"]["wrapper_error_code"], (
            f"[{case['id']}] the propagating error must carry wrapper code "
            f"{case['expected']['wrapper_error_code']!r}, got {excinfo.value.code!r}"
        )
        assert getattr(excinfo.value.cause, "code", None) == case["expected"]["original_error_code"], (
            f"[{case['id']}] the original error code must survive wrapping: expected "
            f"{case['expected']['original_error_code']!r}, got "
            f"{getattr(excinfo.value.cause, 'code', None)!r}"
        )
        assert invoked == case["expected"]["on_step_error_invoked"], (
            f"[{case['id']}] on_step_error invocation: got {invoked}, "
            f"expected {case['expected']['on_step_error_invoked']} (reverse registration order)"
        )

    async def test_on_step_error_only_executed_middlewares(self) -> None:
        case = CASES["on_step_error_only_executed_middlewares"]
        before: list[str] = []
        errored: list[str] = []
        step_bodies: list[dict[str, Any]] = []
        raiser = case["input"]["before_step_raises_in"]

        class _Handler:
            def __init__(self, label: str) -> None:
                self._label = label

            def before_step(self, step_name: str, state: Any) -> None:
                before.append(self._label)
                if self._label == raiser:
                    raise RuntimeError(f"before_step exploded in {self._label}")

            def on_step_error(self, step_name: str, state: Any, error: Exception) -> Any:
                errored.append(self._label)
                return None

        strategy = ExecutionStrategy("conformance", [_OkStep(case["input"]["step"], step_bodies)])
        for label in case["input"]["register_order"]:
            strategy.add_step_middleware(_Handler(label))

        # Caught by ModuleError so the assertion is on the WIRE CODE, not the
        # class. `wrapper_is_load_bearing` in the fixture's driver_contract:
        # deleting the wrapping was verified to leave every SDK driver green
        # before this was pinned, so this assertion is the whole point of the
        # before_step fix.
        with pytest.raises(ModuleError) as excinfo:
            await _run(strategy, _pipeline_context())

        assert excinfo.value.code == case["expected"]["wrapper_error_code"], (
            f"[{case['id']}] a before_step failure must surface as "
            f"{case['expected']['wrapper_error_code']!r}, got {excinfo.value.code!r}"
        )
        assert isinstance(excinfo.value.cause, RuntimeError), (
            f"[{case['id']}] the before_step exception must be carried as the wrapper's cause, "
            f"got {excinfo.value.cause!r}"
        )
        assert isinstance(
            getattr(excinfo.value, "original", None), RuntimeError
        ), f"[{case['id']}] apcore-python additionally exposes it as MiddlewareChainError.original"
        assert before == case["expected"]["before_step_invoked"], (
            f"[{case['id']}] before_step must stop at the raising middleware: got {before}, "
            f"expected {case['expected']['before_step_invoked']}"
        )
        assert errored == case["expected"]["on_step_error_invoked"], (
            f"[{case['id']}] on_step_error must run only on already-executed middlewares, in reverse: "
            f"got {errored}, expected {case['expected']['on_step_error_invoked']}"
        )
        assert step_bodies == [], f"[{case['id']}] the step body must not execute after a before_step failure"

    async def test_before_step_failure_recovery_is_discarded(self) -> None:
        """A before_step failure is terminal: the recovery value MUST be discarded.

        The whole point of the case is that a `before_step` failure and a
        step-body failure MUST NOT share a recovery path. Honouring the
        recovery advances the pipeline past a step whose body never ran, and
        `acl_check` / `approval_gate` sit in the built-in strategy — hence the
        fixture's choice of `acl_check` as the step name.
        """
        case = CASES["before_step_failure_recovery_is_discarded"]
        expected = case["expected"]
        before: list[str] = []
        errored: list[str] = []
        after: list[str] = []
        executed: list[str] = []
        raiser: str = case["input"]["before_step_raises_in"]
        gated_step: str = case["input"]["step"]
        returns: dict[str, Any] = case["input"]["on_step_error_returns"]

        class _Mw:
            def __init__(self, label: str) -> None:
                self._label = label

            def before_step(self, step_name: str, state: Any) -> None:
                before.append(self._label)
                # Scoped to the gated step ONLY. A middleware that raises on
                # every step would make `following_step_executed` vacuous: the
                # following step would be blocked by its own chain failure, so
                # an implementation that honoured the recovery would still look
                # like it had stopped.
                if self._label == raiser and step_name == gated_step:
                    raise RuntimeError(f"before_step exploded in {self._label}")

            def after_step(self, step_name: str, state: Any, result: StepResult) -> None:
                after.append(self._label)

            def on_step_error(self, step_name: str, state: Any, error: Exception) -> Any:
                errored.append(self._label)
                return returns[self._label]

        # The gated step carries `ignore_errors: true` from the fixture — the
        # setting that MUST NOT apply to a MiddlewareChainError — and is
        # followed by a second step whose non-execution is the load-bearing
        # proof that the recovery was discarded.
        gated = _RecordingStep(case["input"]["step"], executed, ignore_errors=case["input"]["ignore_errors"])
        following = _RecordingStep(case["input"]["following_step"], executed)
        strategy = ExecutionStrategy("conformance", [gated, following])
        for label in case["input"]["register_order"]:
            strategy.add_step_middleware(_Mw(label))

        ctx = _pipeline_context()
        # Deliberately NOT `pytest.raises`: the raise is asserted below, after
        # the bypass check, so that an implementation which honours the
        # recovery is caught by `following_step_executed` rather than by
        # "DID NOT RAISE". The fixture's driver_contract requires the bypass
        # itself to be the observation.
        raised: Exception | None = None
        try:
            await _run(strategy, ctx)
        except Exception as exc:  # noqa: BLE001 - the code is asserted below
            raised = exc

        # `following_step_executed: false` is the assertion the fixture's
        # driver_contract singles out: an implementation that HONOURS the
        # recovery and then happens to fail later would satisfy a raise-only
        # assertion while the authorization bypass is live.
        assert case["input"]["following_step"] not in executed, (
            f"[{case['id']}] recovery from a before_step failure MUST NOT let the pipeline continue: "
            f"step {case['input']['following_step']!r} executed (steps run: {executed})"
        )
        assert (case["input"]["following_step"] in executed) is expected["following_step_executed"]
        assert executed == [], (
            f"[{case['id']}] no step body may run: the gated step's own body must not execute "
            f"either (steps run: {executed})"
        )
        # `recovery_honored` bound to the OBSERVATION: the recovery is honoured
        # iff a value returned by on_step_error became the output or the
        # pipeline advanced past the gated step. The old
        # `assert expected["recovery_honored"] is False` restated the fixture
        # and could not fail on SDK behaviour (apcore-python#32 /
        # aiperceivable/apcore#81).
        recovery_values = [value for value in returns.values() if value is not None]
        recovery_honored = ctx.output in recovery_values or case["input"]["following_step"] in executed
        assert recovery_honored is expected["recovery_honored"], (
            f"[{case['id']}] the value returned by on_step_error MUST be discarded: "
            f"output={ctx.output!r}, steps run={executed}"
        )
        assert ctx.output == expected["step_output"], (
            f"[{case['id']}] the discarded recovery value MUST NOT become the step's output: "
            f"got {ctx.output!r}, expected {expected['step_output']!r}"
        )
        # `ignore_errors_applies: false` — the step declares ignore_errors, and
        # a MiddlewareChainError propagates through it regardless, because a
        # broken middleware chain is not a step failure.
        assert gated.ignore_errors is True, f"[{case['id']}] the fixture requires ignore_errors on the step"
        assert (raised is not None) is expected[
            "error_propagated"
        ], f"[{case['id']}] ignore_errors MUST NOT swallow the chain failure; nothing was raised"
        # Asserted on the WIRE CODE via ModuleError, never on a class name.
        assert isinstance(raised, ModuleError) and raised.code == expected["wrapper_error_code"], (
            f"[{case['id']}] the chain failure must surface as "
            f"{expected['wrapper_error_code']!r}, got {getattr(raised, 'code', raised)!r}"
        )
        # `ignore_errors_applies` bound to the OBSERVATION: the step declares
        # ignore_errors, so the setting "applied" iff the chain failure was
        # swallowed. Previously asserted against a literal.
        ignore_errors_applies = gated.ignore_errors and raised is None
        assert ignore_errors_applies is expected["ignore_errors_applies"], (
            f"[{case['id']}] ignore_errors MUST NOT apply to a MiddlewareChainError: "
            f"ignore_errors={gated.ignore_errors}, raised={raised!r}"
        )
        assert (
            before == expected["before_step_invoked"]
        ), f"[{case['id']}] before_step invocation: got {before}, expected {expected['before_step_invoked']}"
        # Every already-entered middleware gets its cleanup call: there is no
        # recovery on this path, so there is nothing to short-circuit on, and
        # short-circuiting would skip mw_a's cleanup for a discarded value.
        assert errored == expected["on_step_error_invoked"], (
            f"[{case['id']}] on_step_error must run on every already-entered middleware, in reverse: "
            f"got {errored}, expected {expected['on_step_error_invoked']}"
        )
        assert after == expected["after_step_invoked"], (
            f"[{case['id']}] after_step MUST NOT fire for a step whose body never ran: "
            f"got {after}, expected {expected['after_step_invoked']}"
        )

    async def test_after_step_fires_after_a_recovered_step(self) -> None:
        """The STEP-BODY failure path — deliberately the opposite of the case above.

        A recovered step produced an output and the pipeline continued, so the
        onion MUST close or whatever ``before_step`` acquired leaks.

        This case also carries the SECOND `state.outputs` snapshot site
        (fixture ``driver_contract.both_snapshot_sites``). A recovered step is
        still the current step, so ``after_step`` MUST NOT see it — but
        ``state_outputs_excludes_the_current_step_in_every_hook`` recovers from
        nothing and therefore never reaches the recovery-path snapshot. In
        apcore-typescript and apcore-rust the two sites are separate lines, and
        swapping the recovery one left every conformance case green until this
        expectation existed. apcore-python has a SINGLE site — the recovery
        branch assigns ``result = recovery`` and falls through to the shared
        success tail — so the same line serves both, but the assertion is made
        here regardless: the driver must not depend on that being true.
        """
        case = CASES["after_step_fires_after_a_recovered_step"]
        expected = case["expected"]
        errored: list[str] = []
        after: list[str] = []
        returns: dict[str, Any] = case["input"]["on_step_error_returns"]
        recovered_step: str = case["input"]["step"]
        preceding_step: str = case["input"]["preceding_step"]
        # Snapshotted INSIDE the hook, per `driver_contract.outputs_is_a_live_reference`:
        # `state.outputs` aliases the engine's live map in apcore-python exactly
        # as it does in apcore-typescript (`PipelineState(outputs=step_outputs)`
        # passes the dict, not a copy). A driver that stashed the state object
        # and read it after `run()` returned would see the FINAL map — with the
        # recovered step present — on every entry, and could never fail.
        after_outputs: dict[str, list[str]] = {}

        class _Mw:
            def __init__(self, label: str) -> None:
                self._label = label

            def before_step(self, step_name: str, state: Any) -> None:
                pass

            def after_step(self, step_name: str, state: Any, result: StepResult) -> None:
                after_outputs[step_name] = list(state.outputs.keys())
                # Scoped to the recovered step: the preceding step gets its own
                # after_step pass, which would otherwise double the order list.
                if step_name == recovered_step:
                    after.append(self._label)

            def on_step_error(self, step_name: str, state: Any, error: Exception) -> Any:
                errored.append(self._label)
                return returns[self._label]

        # The preceding step exists to put ONE completed step in `state.outputs`,
        # so the expectation below is an exact key set rather than "empty" — an
        # engine that never populated the map at all would satisfy "empty".
        strategy = ExecutionStrategy(
            "conformance",
            [
                _ValueStep(preceding_step, {"v": 1}),
                _FailStep(recovered_step, _fixture_error(case)),
            ],
        )
        for label in case["input"]["register_order"]:
            strategy.add_step_middleware(_Mw(label))

        ctx = _pipeline_context()
        propagated = False
        pipeline_output: Any = None
        try:
            pipeline_output, _trace = await _run(strategy, ctx)
        except Exception:  # noqa: BLE001 - the fixture says the error must not escape
            propagated = True

        assert (
            propagated is expected["error_propagated"]
        ), f"[{case['id']}] a recovered step body must not propagate its error"
        assert errored == expected["on_step_error_invoked"], (
            f"[{case['id']}] first-recovery-wins short-circuits the remaining handlers: "
            f"got {errored}, expected {expected['on_step_error_invoked']}"
        )
        assert pipeline_output == expected["step_output"], (
            f"[{case['id']}] the recovery value must become the step output: "
            f"got {pipeline_output!r}, expected {expected['step_output']!r}"
        )
        assert after == expected["after_step_invoked"], (
            f"[{case['id']}] after_step MUST fire after a RECOVERED step body, in reverse "
            f"registration order: got {after}, expected {expected['after_step_invoked']}"
        )
        assert after_outputs.get(recovered_step) == expected["outputs_keys_in_after_step"], (
            f"[{case['id']}] after_step on the RECOVERED step {recovered_step!r} must see exactly "
            f"{expected['outputs_keys_in_after_step']} — a recovered step is still the current step, "
            f"and its output is the `result` argument; got {after_outputs.get(recovered_step)}"
        )


class TestObservationSemantics:
    async def test_async_middleware_awaited(self) -> None:
        """Async callbacks must be resolved before the pipeline advances."""
        case = CASES["async_middleware_awaited"]
        recorded: list[str] = []
        before_token: str = case["input"]["async_before_step_records"]
        after_token: str = case["input"]["async_after_step_records"]

        class _AsyncMw:
            async def before_step(self, step_name: str, state: Any) -> None:
                recorded.append(before_token)

            async def after_step(self, step_name: str, state: Any, result: StepResult) -> None:
                recorded.append(after_token)

        strategy = ExecutionStrategy("conformance", [_OkStep(case["input"]["step"], [])])
        strategy.add_step_middleware(_AsyncMw())

        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            await _run(strategy, _pipeline_context())

        assert recorded == case["expected"]["recorded_in_order"], (
            f"[{case['id']}] the awaited side effects must land in order: "
            f"got {recorded}, expected {case['expected']['recorded_in_order']}"
        )
        # Both flags bound to OBSERVATIONS. `callback_was_awaited is True` and
        # the `and ... is True` tail of the next assertion restated the fixture
        # (apcore-python#32 / aiperceivable/apcore#81); an unawaited coroutine
        # records nothing, so the side effects landing IS the proof.
        unawaited = [w for w in caught if "never awaited" in str(w.message)]
        callback_was_awaited = recorded == case["expected"]["recorded_in_order"] and not unawaited
        assert callback_was_awaited is case["expected"]["callback_was_awaited"], (
            f"[{case['id']}] async step middleware callbacks must be awaited: "
            f"recorded={recorded}, unawaited warnings={unawaited}"
        )
        assert (not unawaited) is case["expected"][
            "no_unawaited_warning"
        ], f"[{case['id']}] coroutines must be awaited, not created and dropped: {unawaited}"

    async def test_before_step_return_value_is_ignored(self) -> None:
        """A ``before_step`` return value MUST NOT change what the step body sees."""
        case = CASES["before_step_return_value_is_ignored"]
        seen: list[dict[str, Any]] = []
        invoked: list[str] = []
        returns: dict[str, Any] = case["input"]["before_step_returns"]

        class _Mw:
            def __init__(self, label: str) -> None:
                self._label = label

            def before_step(self, step_name: str, state: Any) -> Any:
                invoked.append(self._label)
                return returns[self._label]

        strategy = ExecutionStrategy("conformance", [_OkStep(case["input"]["step"], seen)])
        for label in case["input"]["register_order"]:
            strategy.add_step_middleware(_Mw(label))

        raised: Exception | None = None
        try:
            await _run(strategy, _pipeline_context(case["input"]["original_call_inputs"]))
        except Exception as exc:  # noqa: BLE001 - the fixture asserts nothing is raised
            raised = exc

        assert (raised is not None) is case["expected"][
            "error_raised"
        ], f"[{case['id']}] returning a dict from before_step must not raise; got {raised!r}"
        assert (
            invoked == case["expected"]["before_step_invoked"]
        ), f"[{case['id']}] before_step invocation: got {invoked}, expected {case['expected']['before_step_invoked']}"
        assert seen and seen[0] == case["expected"]["module_received_inputs"], (
            f"[{case['id']}] before_step observes but does not rewrite: the step body must still see "
            f"{case['expected']['module_received_inputs']!r}, got {seen[0] if seen else None!r}"
        )


class TestStateOutputs:
    async def test_state_outputs_excludes_the_current_step_in_every_hook(self) -> None:
        """`state.outputs` holds exactly the steps that completed BEFORE the current one.

        The current step is never present, in any of the three hooks:
        ``before_step`` has not run it, ``on_step_error`` has no output to
        record, and in ``after_step`` its output is the ``result`` argument.
        One rule with one meaning — the alternative reading, "outputs of
        completed steps", would make ``after_step`` the single hook whose
        ``outputs`` differs in shape, so a middleware would have to know which
        hook it was in before reading the map.

        Every assertion below is on the EXACT key set, per the fixture's
        ``assert_the_exact_key_set`` contract: ``"second" not in outputs`` also
        passes against an implementation that lost ``first``, and against one
        that never populated the map at all. The ``on_step_error`` expectation
        is observed on the THIRD step for that reason — it proves the map DOES
        keep earlier steps while excluding the failing one, which an all-empty
        map would not satisfy.
        """
        case = CASES["state_outputs_excludes_the_current_step_in_every_hook"]
        expected = case["expected"]
        step_names: list[str] = case["input"]["steps"]
        values: dict[str, Any] = case["input"]["step_outputs"]
        observed_step: str = case["input"]["observe_hooks_on"]
        # `third_step_raises` — the last of the three. on_step_error is
        # observed here, one step LATER than before_step/after_step, so the
        # expected key sets differ from each other.
        error_step: str = step_names[-1]
        raises = case["input"]["third_step_raises"]

        # (hook, step_name) -> the key list `state.outputs` carried at that call.
        seen_keys: dict[tuple[str, str], list[str]] = {}

        class _Observer:
            def before_step(self, step_name: str, state: Any) -> None:
                seen_keys[("before_step", step_name)] = list(state.outputs.keys())

            def after_step(self, step_name: str, state: Any, result: StepResult) -> None:
                seen_keys[("after_step", step_name)] = list(state.outputs.keys())

            def on_step_error(self, step_name: str, state: Any, error: Exception) -> Any:
                seen_keys[("on_step_error", step_name)] = list(state.outputs.keys())
                return None  # no recovery: the case is about the map, not the error

        strategy = ExecutionStrategy(
            "conformance",
            [
                _ValueStep(
                    name,
                    values[name],
                    raises=(
                        ModuleError(code=raises["code"], message=raises["message"]) if name == error_step else None
                    ),
                )
                for name in step_names
            ],
        )
        strategy.add_step_middleware(_Observer())

        raised: Exception | None = None
        try:
            await _run(strategy, _pipeline_context())
        except Exception as exc:  # noqa: BLE001 - the failing step is the point; codes are pinned elsewhere
            raised = exc

        # The failing step must actually have failed, or the on_step_error
        # observation below would be vacuously absent rather than asserted.
        assert raised is not None, f"[{case['id']}] the {error_step!r} step must fail so on_step_error is observed"
        assert ("on_step_error", error_step) in seen_keys, (
            f"[{case['id']}] on_step_error was never invoked on {error_step!r}; " f"observations: {sorted(seen_keys)}"
        )

        assert seen_keys[("before_step", observed_step)] == expected["outputs_keys_in_before_step"], (
            f"[{case['id']}] before_step on {observed_step!r} must see exactly "
            f"{expected['outputs_keys_in_before_step']}, got "
            f"{seen_keys[('before_step', observed_step)]}"
        )
        assert seen_keys[("after_step", observed_step)] == expected["outputs_keys_in_after_step"], (
            f"[{case['id']}] after_step on {observed_step!r} must see exactly "
            f"{expected['outputs_keys_in_after_step']} — the current step's output is the `result` "
            f"argument, NOT an entry in state.outputs; got {seen_keys[('after_step', observed_step)]}"
        )
        assert seen_keys[("on_step_error", error_step)] == expected["outputs_keys_in_on_step_error"], (
            f"[{case['id']}] on_step_error on {error_step!r} must see exactly "
            f"{expected['outputs_keys_in_on_step_error']} — earlier steps kept, the failing step "
            f"excluded; got {seen_keys[('on_step_error', error_step)]}"
        )

        # `current_step_never_present` bound to the OBSERVATION across every
        # hook of every step, not restated from the fixture (apcore-python#32 /
        # aiperceivable/apcore#81).
        offenders = {(hook, step_name): keys for (hook, step_name), keys in seen_keys.items() if step_name in keys}
        current_step_never_present = not offenders
        assert current_step_never_present is expected["current_step_never_present"], (
            f"[{case['id']}] the current step MUST NOT appear in state.outputs in any hook; "
            f"offending observations: {offenders}"
        )


def test_every_fixture_case_has_a_driver() -> None:
    driven = {
        "before_after_invocation_order",
        "on_step_error_recovery_short_circuits",
        "on_step_error_null_propagates_error",
        "on_step_error_only_executed_middlewares",
        "async_middleware_awaited",
        "before_step_return_value_is_ignored",
        "before_step_failure_recovery_is_discarded",
        "after_step_fires_after_a_recovered_step",
        "state_outputs_excludes_the_current_step_in_every_hook",
    }
    assert set(CASES) == driven, (
        f"pipeline_step_middleware.json cases without a driver: {sorted(set(CASES) - driven)}; "
        f"drivers with no matching case: {sorted(driven - set(CASES))}"
    )
