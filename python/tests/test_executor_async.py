"""Tests for Executor async support (call_async, sync-async bridging)."""

from __future__ import annotations

import asyncio
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from threading import Barrier, Event, current_thread
from typing import Any

import pytest

from apcore.builtin_steps import build_standard_strategy
from apcore.config import Config
from apcore.context import Context
from apcore.errors import ModuleError, ModuleTimeoutError
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.registry import Registry


# === Test Helpers ===


class SyncModule:
    """A sync module for testing."""

    input_schema = None
    output_schema = None

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"greeting": f"Hello, {inputs.get('name', 'World')}"}


class AsyncModule:
    """An async module for testing."""

    input_schema = None
    output_schema = None

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"greeting": f"Hello, {inputs.get('name', 'World')}"}


class SlowAsyncModule:
    """An async module that takes too long."""

    input_schema = None
    output_schema = None

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        await asyncio.sleep(2)
        return {"result": "slow"}


def _make_executor(
    module: Any = None,
    module_id: str = "test.module",
    config: Config | None = None,
) -> Executor:
    """Create an Executor with a single registered module."""
    reg = Registry()
    if module is not None:
        reg.register(module_id, module)
    else:
        reg.register(module_id, SyncModule())
    return Executor(registry=reg, config=config)


# === call_async() with async module ===


class TestCallAsyncWithAsyncModule:
    """Tests for call_async() with async modules."""

    @pytest.mark.asyncio
    async def test_directly_awaits_async_module(self) -> None:
        """call_async() directly awaits an async module's execute()."""
        ex = _make_executor(module=AsyncModule())
        result = await ex.call_async("test.module", {"name": "Alice"})
        assert result == {"greeting": "Hello, Alice"}

    @pytest.mark.asyncio
    async def test_timeout_raises_module_timeout_error(self) -> None:
        """call_async() raises ModuleTimeoutError on async timeout."""
        config = Config(data={"executor": {"default_timeout": 100}})
        ex = _make_executor(module=SlowAsyncModule(), config=config)
        with pytest.raises(ModuleTimeoutError):
            await ex.call_async("test.module", {})

    @pytest.mark.asyncio
    async def test_timeout_zero_disables(self) -> None:
        """call_async() with timeout=0 disables timeout enforcement."""
        config = Config(data={"executor": {"default_timeout": 0}})

        class QuickAsync:
            input_schema = None
            output_schema = None

            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {"ok": True}

        ex = _make_executor(module=QuickAsync(), config=config)
        result = await ex.call_async("test.module", {})
        assert result == {"ok": True}


# === call_async() with sync module ===


class TestCallAsyncWithSyncModule:
    """Tests for call_async() offloading sync modules to threads."""

    @pytest.mark.asyncio
    async def test_offloads_sync_module(self) -> None:
        """call_async() uses asyncio.to_thread for sync modules."""
        ex = _make_executor(module=SyncModule())
        result = await ex.call_async("test.module", {"name": "Bob"})
        assert result == {"greeting": "Hello, Bob"}

    @pytest.mark.asyncio
    async def test_result_matches_sync_call(self) -> None:
        """call_async() with sync module returns same result as call()."""
        mod = SyncModule()
        ex = _make_executor(module=mod)
        sync_result = ex.call("test.module", {"name": "Charlie"})
        async_result = await ex.call_async("test.module", {"name": "Charlie"})
        assert sync_result == async_result


# === Sync call() with async module ===


class TestCallSyncWithAsyncModule:
    """Tests for sync call() bridging to async modules."""

    def test_bridges_async_no_running_loop(self) -> None:
        """call() with async module works without a running loop."""
        ex = _make_executor(module=AsyncModule())
        result = ex.call("test.module", {"name": "Dave"})
        assert result == {"greeting": "Hello, Dave"}

    def test_bridges_async_with_running_loop(self) -> None:
        """call() with async module works even inside a running event loop."""
        ex = _make_executor(module=AsyncModule())

        # Run inside an event loop to simulate Jupyter/async framework
        async def inner():
            # call() is sync, but we're inside a running loop
            return ex.call("test.module", {"name": "Eve"})

        loop = asyncio.new_event_loop()
        try:
            result = loop.run_until_complete(inner())
            assert result == {"greeting": "Hello, Eve"}
        finally:
            loop.close()

    def test_concurrent_callers_own_and_reuse_distinct_loops(self) -> None:
        entered = Barrier(2, timeout=5)
        observations: dict[str, tuple[asyncio.AbstractEventLoop, str]] = {}

        class LoopRecorder(AsyncModule):
            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                observations[inputs["name"]] = asyncio.get_running_loop(), current_thread().name
                if inputs["name"].endswith("first"):
                    entered.wait()
                return await super().execute(inputs, context)

        def call_twice(prefix: str) -> str:
            owner = current_thread().name
            ex.call("test.module", {"name": f"{prefix}-first"})
            ex.call("test.module", {"name": f"{prefix}-again"})
            return owner

        with _make_executor(module=LoopRecorder()) as ex, ThreadPoolExecutor(2) as callers:
            calls = {prefix: callers.submit(call_twice, prefix) for prefix in ("one", "two")}
            for prefix, result in calls.items():
                owner = result.result(timeout=5)
                first, executing_thread = observations[f"{prefix}-first"]
                assert executing_thread == owner
                assert observations[f"{prefix}-again"] == (first, owner)
            assert observations["one-first"][0] is not observations["two-first"][0]

    def test_close_retires_an_active_thread_loop_and_allows_reuse(self) -> None:
        entered = Event()
        release = Event()
        loops: list[asyncio.AbstractEventLoop] = []

        class PausingModule(AsyncModule):
            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                loops.append(asyncio.get_running_loop())
                if inputs.get("name") == "pause":
                    entered.set()
                    assert release.wait(5)
                return await super().execute(inputs, context)

        with _make_executor(module=PausingModule()) as ex, ThreadPoolExecutor(1) as caller:
            first = caller.submit(ex.call, "test.module", {"name": "pause"})
            try:
                assert entered.wait(5)
                ex.close()
                assert not loops[0].is_closed()
            finally:
                release.set()
            assert first.result(timeout=5) == {"greeting": "Hello, pause"}
            assert loops[0].is_closed()
            assert caller.submit(ex.call, "test.module").result(timeout=5) == {"greeting": "Hello, World"}
            assert loops[1] is not loops[0]
            ex.close()
            assert loops[1].is_closed()

    def test_nested_loop_bridge_bounds_admission_across_close(self) -> None:
        entered = Barrier(5, timeout=5)
        release = Event()
        observed: list[str] = []

        class PausingModule(AsyncModule):
            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                name = inputs["name"]
                observed.append(name)
                if name.startswith("active"):
                    entered.wait()
                    assert release.wait(5)
                return await super().execute(inputs, context)

        def run_inside_loop(name: str, timeout_ms: int = 0) -> dict[str, Any]:
            async def invoke() -> dict[str, Any]:
                return ex.call("test.module", {"name": name}, timeout_ms=timeout_ms)

            return asyncio.run(invoke())

        with _make_executor(module=PausingModule()) as ex, ThreadPoolExecutor(4) as callers:
            calls = [callers.submit(run_inside_loop, f"active{i}") for i in range(4)]
            try:
                entered.wait()
                ex.close()
                with pytest.raises(ModuleTimeoutError) as caught:
                    run_inside_loop("expired", timeout_ms=40)
                assert caught.value.details["timeout_ms"] == 40
            finally:
                release.set()
            assert [call.result(timeout=5)["greeting"] for call in calls] == [f"Hello, active{i}" for i in range(4)]
            assert run_inside_loop("fresh") == {"greeting": "Hello, fresh"}
            assert "expired" not in observed

    def test_reentrant_bridge_does_not_wait_for_its_own_pool_when_saturated(self) -> None:
        entered = Barrier(4, timeout=5)

        class NestingModule(AsyncModule):
            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                entered.wait()
                try:
                    return ex.call("test.inner", timeout_ms=0)
                except ModuleError as exc:
                    assert "Sync bridge is busy" in str(exc)
                    return {"greeting": "Pool full"}

        def run_inside_loop() -> dict[str, Any]:
            async def invoke() -> dict[str, Any]:
                return ex.call("test.module", timeout_ms=1000)

            return asyncio.run(invoke())

        with _make_executor(module=NestingModule()) as ex, ThreadPoolExecutor(4) as callers:
            ex.registry.register("test.inner", AsyncModule())
            calls = [callers.submit(run_inside_loop) for _ in range(4)]
            assert "Pool full" in [call.result(timeout=5)["greeting"] for call in calls]

    def test_shared_executor_handles_repeated_sync_calls_from_worker_threads(self) -> None:
        with _make_executor(module=AsyncModule()) as ex, ThreadPoolExecutor(8) as workers:
            calls = [workers.submit(ex.call, "test.module", {"name": str(index)}) for index in range(64)]
            assert [call.result(timeout=5) for call in calls] == [
                {"greeting": f"Hello, {index}"} for index in range(len(calls))
            ]

    @pytest.mark.parametrize("global_timeout", [0, 10])
    async def test_bridge_does_not_truncate_the_pipeline_budget(self, global_timeout: int) -> None:
        class LongBudgetModule(AsyncModule):
            resources = {"timeout": 2000}

            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                await asyncio.sleep(1.05)
                return await super().execute(inputs, context)

        registry = Registry()
        registry.register("test.module", LongBudgetModule())
        config = Config(data={"executor": {"default_timeout": 10, "global_timeout": global_timeout}})
        with Executor(registry, config=config) as ex:
            context = Context.create(global_deadline=time.time() + 3) if global_timeout else None
            wait = ex._sync_bridge_wait(context, None)
            assert wait is None or wait * 1000 >= LongBudgetModule.resources["timeout"]
            assert ex.call("test.module", {"name": "bridge"}, context) == {"greeting": "Hello, bridge"}

    @pytest.mark.parametrize("trace", [False, True])
    async def test_bridge_uses_the_selected_strategy_budget(self, trace: bool) -> None:
        class LongBudgetModule(AsyncModule):
            resources = {"timeout": 2000}

            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                await asyncio.sleep(1.05)
                return await super().execute(inputs, context)

        registry = Registry()
        registry.register("test.module", LongBudgetModule())
        executor_config = Config(data={"executor": {"global_timeout": 10}})
        strategy_config = Config(data={"executor": {"global_timeout": 3000}})
        strategy = build_standard_strategy(registry=registry, config=strategy_config)
        with Executor(registry, config=executor_config, strategy=None if trace else strategy) as ex:
            if trace:
                result, _ = ex.call_with_trace("test.module", {}, strategy=strategy)
            else:
                result = ex.call("test.module", {})
            assert result == {"greeting": "Hello, World"}

    async def test_per_call_bridge_wait_overrides_constructor(self) -> None:
        class BrieflyWaitingModule(AsyncModule):
            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                await asyncio.sleep(0.05)
                return await super().execute(inputs, context)

        registry = Registry()
        registry.register("test.module", BrieflyWaitingModule())
        with Executor(registry, sync_bridge_timeout_ms=5) as ex:
            with pytest.raises(ModuleTimeoutError):
                ex.call("test.module", {})
            assert ex.call("test.module", {}, timeout_ms=600_000) == {"greeting": "Hello, World"}
            assert ex.call("test.module", {}, timeout_ms=0) == {"greeting": "Hello, World"}

    @pytest.mark.parametrize("source", ["yaml", "environment", "mapping"])
    async def test_bridge_wait_can_be_configured_and_overridden(
        self, source: str, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        class BrieflyWaitingModule(AsyncModule):
            async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                await asyncio.sleep(0.05)
                return await super().execute(inputs, context)

        if source == "mapping":
            config = Config(data={"apcore-python": {"executor": {"sync_bridge_timeout": 5}}})
        else:
            timeout = 1000 if source == "environment" else 5
            path = tmp_path / "apcore.yaml"
            path.write_text(
                f"apcore: {{}}\n_config: {{strict: true}}\n"
                f"apcore-python:\n  executor: {{sync_bridge_timeout: {timeout}}}\n",
                encoding="utf-8",
            )
            if source == "environment":
                monkeypatch.setenv("APCORE_PYTHON_EXECUTOR_SYNC__BRIDGE__TIMEOUT", "5")
            config = Config.load(str(path))

        registry = Registry()
        registry.register("test.module", BrieflyWaitingModule())
        with Executor(registry, config=config) as ex:
            with pytest.raises(ModuleTimeoutError) as caught:
                ex.call("test.module", {})
            assert caught.value.details["timeout_ms"] == 5
            assert ex.call("test.module", {}, timeout_ms=600_000) == {"greeting": "Hello, World"}
        with Executor(registry, config=config, sync_bridge_timeout_ms=0) as ex:
            assert ex.call("test.module", {}) == {"greeting": "Hello, World"}

    async def test_unbounded_constructor_bridge_wait_preserves_pipeline_timeout(self) -> None:
        registry = Registry()
        registry.register("test.module", SlowAsyncModule())
        config = Config(data={"executor": {"default_timeout": 10}})
        with Executor(registry, config=config, sync_bridge_timeout_ms=0) as ex:
            with pytest.raises(ModuleTimeoutError) as raised:
                ex.call("test.module", {})
            assert raised.value.details["timeout_ms"] == 10


# === Async middleware ===


class TestAsyncMiddleware:
    """Tests for async middleware detection and invocation."""

    @pytest.mark.asyncio
    async def test_awaits_async_before_middleware(self) -> None:
        """call_async() awaits async before() middleware."""
        calls: list[str] = []

        class AsyncBeforeMW(Middleware):
            async def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                calls.append("async_before")
                return None

        reg = Registry()
        reg.register("test.module", SyncModule())
        ex = Executor(registry=reg, middlewares=[AsyncBeforeMW()])
        await ex.call_async("test.module", {"name": "test"})
        assert "async_before" in calls

    @pytest.mark.asyncio
    async def test_awaits_async_after_middleware(self) -> None:
        """call_async() awaits async after() middleware."""
        calls: list[str] = []

        class AsyncAfterMW(Middleware):
            async def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                calls.append("async_after")
                return None

        reg = Registry()
        reg.register("test.module", SyncModule())
        ex = Executor(registry=reg, middlewares=[AsyncAfterMW()])
        await ex.call_async("test.module", {"name": "test"})
        assert "async_after" in calls

    @pytest.mark.asyncio
    async def test_sync_middleware_called_directly(self) -> None:
        """Sync middleware is called directly without thread offload."""
        calls: list[str] = []

        class SyncMW(Middleware):
            def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                calls.append("sync_before")
                return None

        reg = Registry()
        reg.register("test.module", SyncModule())
        ex = Executor(registry=reg, middlewares=[SyncMW()])
        await ex.call_async("test.module", {"name": "test"})
        assert "sync_before" in calls


# === Module type caching ===


class TestModuleTypeCaching:
    """Tests for _is_async_module caching."""

    @pytest.mark.asyncio
    async def test_async_module_executes_correctly(self) -> None:
        """Async module executes via pipeline without issues."""
        ex = _make_executor(module=AsyncModule())
        result = await ex.call_async("test.module", {"name": "test"})
        assert result is not None

    def test_sync_module_executes_correctly(self) -> None:
        """Sync module executes via pipeline without issues."""
        ex = _make_executor(module=SyncModule())
        result = ex.call("test.module", {"name": "test"})
        assert result is not None
