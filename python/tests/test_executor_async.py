"""Tests for Executor async support (call_async, sync-async bridging)."""

from __future__ import annotations

import asyncio
from typing import Any

import pytest

from apcore.config import Config
from apcore.context import Context
from apcore.errors import ModuleTimeoutError
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
        """call() with async module uses asyncio.run when no loop running."""
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
