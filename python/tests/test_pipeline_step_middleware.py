"""Tests for Issue #33 §2.2 — Pipeline StepMiddleware mechanism."""

from __future__ import annotations

from typing import Any

import pytest

from apcore.context import Context
from apcore.pipeline import (
    BaseStep,
    ExecutionStrategy,
    PipelineContext,
    PipelineEngine,
    PipelineStepError,
    StepMiddleware,
    StepResult,
)


class _OkStep(BaseStep):
    def __init__(self, name: str, *, output_value: Any = None) -> None:
        super().__init__(name=name)
        self._output_value = output_value

    async def execute(self, ctx: PipelineContext) -> StepResult:
        if self._output_value is not None:
            ctx.output = {"value": self._output_value}
        return StepResult(action="continue")


class _FailStep(BaseStep):
    def __init__(self, name: str, exc: Exception) -> None:
        super().__init__(name=name)
        self._exc = exc

    async def execute(self, ctx: PipelineContext) -> StepResult:
        raise self._exc


def _make_ctx() -> PipelineContext:
    context = Context.create()
    return PipelineContext(module_id="m.x", inputs={}, context=context)


# ---------------------------------------------------------------------------
# StepMiddleware protocol shape
# ---------------------------------------------------------------------------


class TestStepMiddlewareProtocol:
    def test_protocol_importable(self) -> None:
        # Check the symbol exists and is a Protocol.
        assert StepMiddleware is not None

    def test_strategy_accepts_step_middleware_list(self) -> None:
        s = ExecutionStrategy("s", [_OkStep("a")])
        assert hasattr(s, "step_middlewares")
        assert s.step_middlewares == []

    def test_add_step_middleware(self) -> None:
        s = ExecutionStrategy("s", [_OkStep("a")])

        class _Mw:
            pass

        mw = _Mw()
        s.add_step_middleware(mw)
        assert mw in s.step_middlewares


# ---------------------------------------------------------------------------
# Invocation order
# ---------------------------------------------------------------------------


class TestStepMiddlewareInvocation:
    @pytest.mark.asyncio
    async def test_before_after_called_in_order(self) -> None:
        events: list[str] = []

        class TraceMw:
            def before_step(self, step_name, state):
                events.append(f"before:{step_name}")

            def after_step(self, step_name, state, result):
                events.append(f"after:{step_name}")

        strategy = ExecutionStrategy("s", [_OkStep("a"), _OkStep("b")])
        strategy.add_step_middleware(TraceMw())

        await PipelineEngine().run(strategy, _make_ctx())
        assert events == ["before:a", "after:a", "before:b", "after:b"]

    @pytest.mark.asyncio
    async def test_multiple_middlewares_invoked_in_registration_order_for_before(self) -> None:
        events: list[str] = []

        class Mw:
            def __init__(self, label: str) -> None:
                self.label = label

            def before_step(self, step_name, state):
                events.append(f"{self.label}:before:{step_name}")

            def after_step(self, step_name, state, result):
                events.append(f"{self.label}:after:{step_name}")

        strategy = ExecutionStrategy("s", [_OkStep("a")])
        strategy.add_step_middleware(Mw("m1"))
        strategy.add_step_middleware(Mw("m2"))

        await PipelineEngine().run(strategy, _make_ctx())

        # before runs in registration order; after runs in reverse (onion)
        assert events == [
            "m1:before:a",
            "m2:before:a",
            "m2:after:a",
            "m1:after:a",
        ]

    @pytest.mark.asyncio
    async def test_async_middleware_methods_awaited(self) -> None:
        events: list[str] = []

        class AsyncMw:
            async def before_step(self, step_name, state):
                events.append(f"before:{step_name}")

            async def after_step(self, step_name, state, result):
                events.append(f"after:{step_name}")

        strategy = ExecutionStrategy("s", [_OkStep("a")])
        strategy.add_step_middleware(AsyncMw())

        await PipelineEngine().run(strategy, _make_ctx())
        assert events == ["before:a", "after:a"]

    @pytest.mark.asyncio
    async def test_optional_methods_are_optional(self) -> None:
        # A middleware exposing only before_step should still work.
        events: list[str] = []

        class HalfMw:
            def before_step(self, step_name, state):
                events.append(step_name)

        strategy = ExecutionStrategy("s", [_OkStep("a")])
        strategy.add_step_middleware(HalfMw())

        await PipelineEngine().run(strategy, _make_ctx())
        assert events == ["a"]


# ---------------------------------------------------------------------------
# on_step_error recovery
# ---------------------------------------------------------------------------


class TestStepMiddlewareOnError:
    @pytest.mark.asyncio
    async def test_on_step_error_recovery_sync(self) -> None:
        class RecoveryMw:
            def on_step_error(self, step_name, state, error):
                # Returning non-None means recovered; treat as the step's result
                return StepResult(action="continue", explanation="recovered")

        bad = _FailStep("bad", RuntimeError("boom"))
        strategy = ExecutionStrategy("s", [bad, _OkStep("ok")])
        strategy.add_step_middleware(RecoveryMw())

        # Should not raise — middleware suppresses and recovers
        result, trace = await PipelineEngine().run(strategy, _make_ctx())
        # Both steps should appear in the trace
        names = [st.name for st in trace.steps]
        assert "bad" in names
        assert "ok" in names

    @pytest.mark.asyncio
    async def test_on_step_error_returning_none_re_raises(self) -> None:
        class NoOpMw:
            def on_step_error(self, step_name, state, error):
                return None

        strategy = ExecutionStrategy("s", [_FailStep("bad", RuntimeError("boom"))])
        strategy.add_step_middleware(NoOpMw())

        with pytest.raises(PipelineStepError):
            await PipelineEngine().run(strategy, _make_ctx())

    @pytest.mark.asyncio
    async def test_on_step_error_async_recovery(self) -> None:
        class AsyncRecoveryMw:
            async def on_step_error(self, step_name, state, error):
                return StepResult(action="continue")

        strategy = ExecutionStrategy(
            "s",
            [_FailStep("bad", RuntimeError("boom")), _OkStep("ok")],
        )
        strategy.add_step_middleware(AsyncRecoveryMw())

        result, trace = await PipelineEngine().run(strategy, _make_ctx())
        names = [st.name for st in trace.steps]
        assert "ok" in names

    @pytest.mark.asyncio
    async def test_after_step_not_called_when_step_fails(self) -> None:
        events: list[str] = []

        class TraceMw:
            def before_step(self, step_name, state):
                events.append(f"before:{step_name}")

            def after_step(self, step_name, state, result):
                events.append(f"after:{step_name}")

            def on_step_error(self, step_name, state, error):
                events.append(f"on_error:{step_name}")
                return None  # don't recover

        strategy = ExecutionStrategy("s", [_FailStep("bad", RuntimeError("x"))])
        strategy.add_step_middleware(TraceMw())

        with pytest.raises(PipelineStepError):
            await PipelineEngine().run(strategy, _make_ctx())

        assert events == ["before:bad", "on_error:bad"]
