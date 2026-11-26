"""Shared contract and lifecycle tests for the opted-in streaming API."""

from __future__ import annotations

import asyncio
import time
from collections.abc import AsyncIterator
from dataclasses import asdict
from typing import Any

import pytest
from pydantic import BaseModel

from apcore import (
    APCore,
    CancelToken,
    Config,
    Context,
    ExecutionCancelledError,
    Executor,
    ModuleError,
    ModuleExecuteError,
    ModuleTimeoutError,
    Registry,
)
from apcore.builtin_steps import BuiltinOutputValidation, build_standard_strategy
from apcore.middleware import Middleware
from apcore.pipeline import ExecutionStrategy, PipelineState, StepMiddleware
from conformance.canonical_fixtures import load_fixture


class _ValueOutput(BaseModel):
    value: int


class _FallbackModule:
    input_schema = None
    description = "Return fixture data"

    def __init__(self, case: dict[str, Any]) -> None:
        self.case = case
        self.execution_count = 0
        self.output_schema = (
            _ValueOutput if case["validation"] in {"invalid", "ignored", "recovered", "no_match"} else None
        )

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        self.execution_count += 1
        return dict(self.case["chunks"][0])


class _NativeModule(_FallbackModule):
    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        self.execution_count += 1
        for item in self.case["chunks"]:
            yield dict(item)
        action = self.case["action"]
        if action in {"fail_after_first", "recover_after_first"}:
            raise ModuleExecuteError("fixture.stream")
        if action == "timeout_after_first":
            await asyncio.Event().wait()


class _FixtureMiddleware(Middleware):
    def __init__(self, case: dict[str, Any], after_gate: tuple[asyncio.Event, asyncio.Event] | None = None) -> None:
        self.case = case
        self.after_gate = after_gate

    async def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
        after = self.case["after"]
        if after == "block":
            assert self.after_gate is not None
            entered, release = self.after_gate
            entered.set()
            await release.wait()
        if after == "mutate":
            output["value"] = 2
        if after == "fail":
            raise ModuleExecuteError(module_id)
        if after == "cancel":
            raise ExecutionCancelledError()

    def on_error(
        self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
    ) -> dict[str, Any] | None:
        return {"recovered": True} if self.case["action"] == "recover_after_first" else None


class _RecoverValidation(StepMiddleware):
    def on_step_error(self, step_name: str, state: PipelineState, error: Exception) -> dict[str, Any] | None:
        return {"value": 1} if step_name == "output_validation" else None


def _fixture_client(
    case: dict[str, Any], after_gate: tuple[asyncio.Event, asyncio.Event] | None = None
) -> tuple[APCore, _FallbackModule]:
    config = Config({"stream": {"max_merge_depth": case["max_merge_depth"]}}) if "max_merge_depth" in case else None
    client = APCore(config=config)
    module = _NativeModule(case) if case["mode"] == "native" else _FallbackModule(case)
    client.register("fixture.stream", module)
    client.use(_FixtureMiddleware(case, after_gate))
    strategy = client.executor.current_strategy
    match case["validation"]:
        case "removed":
            strategy.remove("output_validation")
        case "ignored":
            step = BuiltinOutputValidation()
            step.ignore_errors = True
            strategy.configure_step("output_validation", step)
        case "recovered":
            strategy.add_step_middleware(_RecoverValidation())
        case "no_match":
            step = BuiltinOutputValidation()
            step.match_modules = ["other.*"]
            strategy.configure_step("output_validation", step)
    return client, module


@pytest.mark.parametrize("case", load_fixture("stream_completion.json")["test_cases"], ids=lambda case: case["id"])
async def test_stream_completion_shared_contract(case: dict[str, Any]) -> None:
    inputs = case["input"]
    expected = case["expected"]
    action = inputs["action"]
    blocked_after = action in {"timeout_during_after", "cancel_during_after"}
    after_gate = (asyncio.Event(), asyncio.Event()) if blocked_after else None
    client, module = _fixture_client(inputs, after_gate)
    token = CancelToken()
    deadline = time.time() + 0.2 if action in {"timeout_after_first", "timeout_during_after"} else None
    context = Context.create(cancel_token=token, global_deadline=deadline)
    execution = client.stream_with_completion("fixture.stream", {}, context)
    chunks = []
    async with asyncio.timeout(2):
        try:
            if action == "close_before_first":
                await execution.aclose()
            elif after_gate is not None:
                entered, release = after_gate
                if inputs["mode"] == "native":
                    chunks.append(await anext(execution.chunks))
                pull = asyncio.create_task(anext(execution.chunks))
                try:
                    await entered.wait()
                    if action == "cancel_during_after":
                        token.cancel()
                    await pull
                    pytest.fail("The final pull must report cancellation or timeout")
                finally:
                    release.set()
            else:
                async for chunk in execution.chunks:
                    chunks.append(chunk)
                    if len(chunks) == 1 and action == "close_after_first":
                        await execution.aclose()
                    if len(chunks) == 1 and action == "cancel_after_first":
                        token.cancel()
        except ModuleError:
            pass
        completion = await execution.completion()
        assert await execution.completion() is completion
    assert completion.reason == expected["reason"]
    assert completion.output == expected["output"]
    assert {key: value for key, value in asdict(completion.output_validation).items() if value is not None} == expected[
        "output_validation"
    ]
    assert {key: value for key, value in asdict(completion.middleware_after).items() if value is not None} == expected[
        "middleware_after"
    ]
    assert completion.error_code == expected["error_code"]
    assert module.execution_count == expected["execution_count"]
    assert chunks == expected["chunks"]
    if action not in {"cancel_after_first", "cancel_during_after"}:
        assert not token.is_cancelled


async def test_close_settles_while_a_producer_is_still_cleaning_up() -> None:
    entered, cleanup_started, release_cleanup, cleanup_finished = [asyncio.Event() for _ in range(4)]
    token = CancelToken()

    class SlowCleanup(_NativeModule):
        child_token: CancelToken | None = None

        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
            self.child_token = context.cancel_token
            try:
                yield {"value": 1}
                entered.set()
                await asyncio.Event().wait()
                yield {"value": 2}
            finally:
                cleanup_started.set()
                await release_cleanup.wait()
                cleanup_finished.set()

    module = SlowCleanup({"validation": "pass"})
    client = APCore()
    client.register("fixture.close", module)
    execution = client.executor.stream_with_completion("fixture.close", context=Context.create(cancel_token=token))
    async with asyncio.timeout(2):
        assert await anext(execution.chunks) == {"value": 1}
        pull = asyncio.create_task(anext(execution.chunks))
        await entered.wait()
        try:
            await execution.aclose()
            assert (await execution.completion()).reason == "consumer_closed"
            with pytest.raises(StopAsyncIteration):
                await pull
            await cleanup_started.wait()
            assert not cleanup_finished.is_set()
            assert module.child_token is not None and module.child_token.is_cancelled
            assert not token.is_cancelled
        finally:
            release_cleanup.set()
        await cleanup_finished.wait()


async def test_pre_execution_recovery_keeps_the_failure_without_a_snapshot() -> None:
    class RecoverBefore(Middleware):
        def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
            raise ModuleExecuteError(module_id, "before failed")

        def on_error(
            self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context
        ) -> dict[str, bool]:
            return {"recovered": True}

    client, module = _fixture_client({"mode": "native", "validation": "pass", "after": "pass"})
    client.use(RecoverBefore())
    execution = client.stream_with_completion("fixture.stream")
    assert [chunk async for chunk in execution.chunks] == [{"recovered": True}]
    assert module.execution_count == 0
    completion = await execution.completion()
    assert completion.reason == "failed"
    assert completion.error_code == "MODULE_EXECUTE_ERROR"
    assert completion.output is None
    assert completion.output_validation.reason == "not_reached"


async def test_fallback_captures_output_without_any_post_processing_steps() -> None:
    client, module = _fixture_client(
        {"mode": "fallback", "validation": "removed", "after": "pass", "action": "drain", "chunks": [{"value": 1}]}
    )
    steps = client.executor.current_strategy.steps
    strategy = ExecutionStrategy(
        "without_post_processing",
        [step for step in steps if step.name not in ("middleware_after", "return_result")],
    )
    executor = Executor(client.registry, strategy=strategy)
    execution = executor.stream_with_completion("fixture.stream")

    chunk = await anext(execution.chunks)
    assert chunk == {"value": 1}
    chunk["value"] = 2
    assert [other async for other in execution.chunks] == []

    completion = await execution.completion()
    assert completion.reason == "completed"
    assert completion.output == {"value": 1}
    assert completion.output_validation.reason == "not_configured"
    assert completion.middleware_after.reason == "not_configured"
    assert module.execution_count == 1


async def test_failure_outside_the_reported_post_steps_has_a_terminal_error() -> None:
    class FailReturn(StepMiddleware):
        def before_step(self, step_name: str, state: PipelineState) -> None:
            if step_name == "return_result" and state.context.output is not None:
                raise ModuleExecuteError("fixture.stream", "return failed")

    client, _ = _fixture_client(
        {"mode": "native", "validation": "pass", "after": "pass", "action": "drain", "chunks": [{"value": 1}]}
    )
    client.executor.current_strategy.add_step_middleware(FailReturn())
    execution = client.stream_with_completion("fixture.stream")
    assert [chunk async for chunk in execution.chunks] == [{"value": 1}]
    completion = await execution.completion()
    assert completion.reason == "failed"
    assert completion.error_code == "MODULE_EXECUTE_ERROR"
    assert completion.output == {"value": 1}
    assert completion.output_validation.status == "passed"
    assert completion.middleware_after.status == "passed"


async def test_stream_factory_cannot_hold_first_pull_past_global_deadline() -> None:
    entered, release = asyncio.Event(), asyncio.Event()

    class SlowFactory(_NativeModule):
        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
            entered.set()
            try:
                await release.wait()
            except asyncio.CancelledError:
                await release.wait()

            async def chunks() -> AsyncIterator[dict[str, Any]]:
                yield {"value": 1}

            return chunks()

    client = APCore()
    client.register("fixture.factory", SlowFactory({"validation": "pass"}))
    global_deadline = time.time() + 0.05
    execution = client.stream_with_completion(
        "fixture.factory", context=Context.create(global_deadline=global_deadline)
    )
    pull = asyncio.create_task(anext(execution.chunks))
    try:
        await asyncio.wait_for(entered.wait(), 1)
        done, _ = await asyncio.wait({pull}, timeout=0.5)
        assert pull in done, "factory cleanup cannot keep the consumer blocked"
        with pytest.raises(ModuleError) as error:
            await pull
        assert error.value.code == "MODULE_TIMEOUT"
        assert (await execution.completion()).reason == "timed_out"
    finally:
        release.set()
        await execution.aclose()
        await asyncio.gather(pull, return_exceptions=True)


async def test_cancel_while_waiting_for_final_eof() -> None:
    entered, release = asyncio.Event(), asyncio.Event()

    class SlowEnd(_NativeModule):
        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
            yield {"value": 1}
            entered.set()
            await release.wait()

    client = APCore()
    client.register("fixture.end", SlowEnd({"validation": "pass"}))
    parent = CancelToken()
    execution = client.stream_with_completion("fixture.end", context=Context.create(cancel_token=parent))
    assert await anext(execution.chunks) == {"value": 1}
    pull = asyncio.create_task(anext(execution.chunks))
    try:
        await asyncio.wait_for(entered.wait(), 1)
        parent.cancel()
        release.set()
        with pytest.raises(ModuleError) as error:
            await asyncio.wait_for(pull, 0.5)
        assert error.value.code == "EXECUTION_CANCELLED"
        assert (await execution.completion()).reason == "cancelled"
    finally:
        release.set()
        await execution.aclose()
        await asyncio.gather(pull, return_exceptions=True)


async def test_unary_none_reports_a_module_execution_error() -> None:
    class Empty(_FallbackModule):
        async def execute(self, inputs: dict[str, Any], context: Context) -> Any:
            return None

    client = APCore()
    client.register("fixture.empty", Empty({"validation": "pass"}))
    execution = client.stream_with_completion("fixture.empty")
    with pytest.raises(ModuleExecuteError):
        await anext(execution.chunks)
    completion = await execution.completion()
    assert completion.output is None
    assert completion.error_code == "MODULE_EXECUTE_ERROR"
    assert completion.output_validation.status == "skipped"


@pytest.mark.parametrize("with_completion", [False, True])
async def test_unary_module_timeout_survives_suspending_on_error(with_completion: bool) -> None:
    release = asyncio.Event()
    finished = asyncio.Event()
    parent = CancelToken()

    class SlowFallback(_FallbackModule):
        resources = {"timeout": 15}

        async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            try:
                await release.wait()
                assert context.cancel_token is not None and context.cancel_token.is_cancelled
                return {"value": 1}
            finally:
                finished.set()

    class ObserveError(Middleware):
        async def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context) -> None:
            await asyncio.sleep(0)

    client = APCore()
    client.register("fixture.slow", SlowFallback({"validation": "pass"}))
    client.use(ObserveError())
    context = Context.create(cancel_token=parent)
    execution = client.stream_with_completion("fixture.slow", context=context) if with_completion else None
    chunks = execution.chunks if execution is not None else client.stream("fixture.slow", context=context)
    try:
        with pytest.raises(ModuleTimeoutError):
            await asyncio.wait_for(anext(chunks), 1)
        if execution is not None:
            completion = await execution.completion()
            assert completion.reason == "timed_out"
            assert completion.error_code == "MODULE_TIMEOUT"
        assert not parent.is_cancelled
    finally:
        release.set()
        await asyncio.wait_for(finished.wait(), 1)


async def test_return_result_middleware_runs_once_after_native_chunks() -> None:
    observed = []

    class ObserveReturn(StepMiddleware):
        def before_step(self, step_name: str, state: PipelineState) -> None:
            if step_name == "return_result":
                observed.append(state.context.output)

    client, _ = _fixture_client(
        {"mode": "native", "validation": "pass", "after": "pass", "action": "drain", "chunks": [{"value": 1}]}
    )
    client.executor.current_strategy.add_step_middleware(ObserveReturn())
    execution = client.stream_with_completion("fixture.stream")
    assert await anext(execution.chunks) == {"value": 1}
    assert observed == []
    assert [chunk async for chunk in execution.chunks] == []
    assert (await execution.completion()).reason == "completed"
    assert observed == [{"value": 1}]


async def test_before_validation_hook_failure_does_not_capture_an_early_snapshot() -> None:
    class FailBeforeValidation(StepMiddleware):
        def before_step(self, step_name: str, state: PipelineState) -> None:
            if step_name == "output_validation":
                raise ModuleExecuteError("fixture.stream", "output setup failed")

    client, _ = _fixture_client(
        {"mode": "native", "validation": "pass", "after": "pass", "action": "drain", "chunks": [{"value": 1}]}
    )
    client.executor.current_strategy.add_step_middleware(FailBeforeValidation())
    execution = client.stream_with_completion("fixture.stream")
    assert [chunk async for chunk in execution.chunks] == [{"value": 1}]
    completion = await execution.completion()
    assert completion.output is None
    assert completion.output_validation.status != "passed"


async def test_native_output_is_detached_before_yield_and_exact_at_validation() -> None:
    class ChangeAtBoundary(StepMiddleware):
        def before_step(self, step_name: str, state: PipelineState) -> None:
            if step_name == "output_validation" and state.context.output is not None:
                state.context.output["boundary"] = True

    client, _ = _fixture_client(
        {
            "mode": "native",
            "validation": "pass",
            "after": "pass",
            "action": "drain",
            "chunks": [{"nested": {"value": 1}}],
        }
    )
    client.executor.current_strategy.add_step_middleware(ChangeAtBoundary())
    execution = client.stream_with_completion("fixture.stream")
    chunk = await anext(execution.chunks)
    chunk["nested"]["value"] = 2
    assert [next_chunk async for next_chunk in execution.chunks] == []
    assert (await execution.completion()).output == {"nested": {"value": 1}, "boundary": True}


async def test_close_owns_cancellation_even_if_custom_strategy_keeps_the_bound_context() -> None:
    closed = asyncio.Event()

    class ProbeToken(_NativeModule):
        seen: CancelToken | None = None

        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
            self.seen = context.cancel_token
            try:
                yield {"value": 1}
            finally:
                closed.set()

    module = ProbeToken({"validation": "pass"})
    registry = Registry()
    registry.register("fixture.close", module)
    standard = build_standard_strategy(registry=registry)
    custom = ExecutionStrategy(
        "host_context",
        [step for step in standard.steps if step.name != "context_creation"],
        validate_dependencies=False,
    )
    executor = Executor(registry, strategy=custom)
    parent = CancelToken()
    execution = executor.stream_with_completion("fixture.close", context=Context.create(cancel_token=parent))
    async with asyncio.timeout(2):
        assert await anext(execution.chunks) == {"value": 1}
        await execution.aclose()
        assert (await execution.completion()).reason == "consumer_closed"
        await closed.wait()
    assert module.seen is not None and module.seen is not parent and module.seen.is_cancelled
    assert not parent.is_cancelled
