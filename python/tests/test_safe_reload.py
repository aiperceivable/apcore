"""Tests for safe hot-reload (F09 / Algorithm A21) in the Registry class."""

from __future__ import annotations

import asyncio
import threading
from typing import Any

import pytest

from apcore.errors import ModuleNotFoundError, ModuleTimeoutError
from apcore.executor import Executor
from apcore.registry.registry import Registry


# ---------------------------------------------------------------------------
# Helper module class
# ---------------------------------------------------------------------------


class _StubModule:
    """Minimal module for hot-reload tests."""

    input_schema = {"type": "object"}
    output_schema = {"type": "object"}

    description = "stub"

    def execute(self, inputs: dict[str, Any], context: Any = None) -> dict[str, Any]:
        return {"ok": True}


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture()
def registry(tmp_path: pytest.TempPathFactory) -> Registry:
    """Create a fresh Registry with a temporary extensions directory."""
    ext = tmp_path / "extensions"  # type: ignore[union-attr]
    ext.mkdir()
    return Registry(extensions_dir=str(ext))


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


class TestSafeUnregisterIdle:
    """safe_unregister() on an idle module (no in-flight executions)."""

    def test_returns_true_immediately(self, registry: Registry) -> None:
        registry.register("idle.mod", _StubModule())
        result = registry.safe_unregister("idle.mod")

        assert result is True
        assert not registry.has("idle.mod")


class TestSafeUnregisterWaitsForInflight:
    """safe_unregister() waits for in-flight executions then returns True."""

    @pytest.mark.asyncio
    async def test_waits_for_an_executor_call_to_finish(self, registry: Registry) -> None:
        entered = asyncio.Event()
        finish = asyncio.Event()
        unloaded = threading.Event()

        class BusyModule(_StubModule):
            async def execute(self, inputs, context):
                entered.set()
                await finish.wait()
                return {"unloaded": unloaded.is_set()}

            def on_unload(self):
                unloaded.set()

        registry.register("busy.mod", BusyModule(), version="2.0.0")
        call = asyncio.create_task(Executor(registry).call_async("busy.mod", version_hint="2.0.0"))
        await asyncio.wait_for(entered.wait(), timeout=1)
        unregister = asyncio.create_task(asyncio.to_thread(registry.safe_unregister, "busy.mod", timeout_ms=1000))
        try:
            for _ in range(100):
                if registry.is_draining("busy.mod") or unregister.done():
                    break
                await asyncio.sleep(0.001)
            assert registry.is_draining("busy.mod")
            assert not unloaded.is_set()
        finally:
            finish.set()
            result = await call
            drained = await unregister

        assert result == {"unloaded": False}
        assert drained and unloaded.is_set()

    @pytest.mark.asyncio
    async def test_timeout_keeps_the_module_leased_until_execution_finishes(self, registry: Registry) -> None:
        finish = asyncio.Event()
        unloaded = threading.Event()

        class BusyModule(_StubModule):
            resources = {"timeout": 10}

            async def execute(self, inputs, context):
                await finish.wait()
                return {"ok": True}

            def on_unload(self):
                unloaded.set()

        registry.register("busy.mod", BusyModule())
        with pytest.raises(ModuleTimeoutError):
            await Executor(registry).call_async("busy.mod")

        unregister = asyncio.create_task(asyncio.to_thread(registry.safe_unregister, "busy.mod", timeout_ms=1000))
        try:
            for _ in range(100):
                if registry.is_draining("busy.mod") or unregister.done():
                    break
                await asyncio.sleep(0.001)
            assert registry.is_draining("busy.mod")
            assert not unloaded.is_set()
        finally:
            finish.set()
            drained = await unregister

        assert drained and unloaded.is_set()

    def test_waits_and_returns_true(self, registry: Registry) -> None:
        registry.register("busy.mod", _StubModule())
        unregister_result: list[bool] = []
        acquired = threading.Event()
        release = threading.Event()

        def hold_execution() -> None:
            with registry.acquire("busy.mod"):
                acquired.set()
                release.wait(timeout=5)

        def do_unregister() -> None:
            result = registry.safe_unregister("busy.mod", timeout_ms=3000)
            unregister_result.append(result)

        exec_thread = threading.Thread(target=hold_execution)
        exec_thread.start()
        acquired.wait(timeout=5)

        unreg_thread = threading.Thread(target=do_unregister)
        unreg_thread.start()

        # Give unregister a moment to start draining, then release
        threading.Event().wait(timeout=0.1)
        release.set()

        exec_thread.join(timeout=5)
        unreg_thread.join(timeout=5)

        assert len(unregister_result) == 1
        assert unregister_result[0] is True
        assert not registry.has("busy.mod")


class TestSafeUnregisterForceUnload:
    """safe_unregister() force-unloads after timeout and returns False."""

    def test_force_unloads_on_timeout(self, registry: Registry) -> None:
        registry.register("stuck.mod", _StubModule())
        unregister_result: list[bool] = []
        acquired = threading.Event()
        release = threading.Event()

        def hold_execution() -> None:
            with registry.acquire("stuck.mod"):
                acquired.set()
                release.wait(timeout=5)

        def do_unregister() -> None:
            result = registry.safe_unregister("stuck.mod", timeout_ms=100)
            unregister_result.append(result)

        exec_thread = threading.Thread(target=hold_execution)
        exec_thread.start()
        acquired.wait(timeout=5)

        unreg_thread = threading.Thread(target=do_unregister)
        unreg_thread.start()
        unreg_thread.join(timeout=5)

        assert len(unregister_result) == 1
        assert unregister_result[0] is False
        assert not registry.has("stuck.mod")

        # Release the stuck execution so the thread can exit cleanly
        release.set()
        exec_thread.join(timeout=5)


class TestAcquireRaisesWhenDraining:
    """acquire() raises ModuleNotFoundError when module is draining."""

    def test_raises_during_drain(self, registry: Registry) -> None:
        registry.register("draining.mod", _StubModule())
        acquired = threading.Event()
        release = threading.Event()
        drain_started = threading.Event()
        error_raised: list[bool] = []

        def hold_execution() -> None:
            with registry.acquire("draining.mod"):
                acquired.set()
                release.wait(timeout=5)

        def do_unregister() -> None:
            # This blocks until ref count drops; marks as draining immediately
            drain_started.set()
            registry.safe_unregister(module_id="draining.mod", timeout_ms=3000)

        def try_acquire_during_drain() -> None:
            drain_started.wait(timeout=5)
            # Give unregister a moment to mark as draining
            threading.Event().wait(timeout=0.1)
            try:
                with registry.acquire("draining.mod"):
                    pass
            except ModuleNotFoundError:
                error_raised.append(True)

        exec_thread = threading.Thread(target=hold_execution)
        exec_thread.start()
        acquired.wait(timeout=5)

        unreg_thread = threading.Thread(target=do_unregister)
        unreg_thread.start()

        acquire_thread = threading.Thread(target=try_acquire_during_drain)
        acquire_thread.start()
        acquire_thread.join(timeout=5)

        assert len(error_raised) == 1
        assert error_raised[0] is True

        # Clean up
        release.set()
        exec_thread.join(timeout=5)
        unreg_thread.join(timeout=5)


class TestIsDraining:
    """is_draining() returns correct state."""

    def test_not_draining_by_default(self, registry: Registry) -> None:
        registry.register("normal.mod", _StubModule())
        assert registry.is_draining("normal.mod") is False

    def test_not_draining_for_unknown_module(self, registry: Registry) -> None:
        assert registry.is_draining("nonexistent") is False

    def test_draining_during_safe_unregister(self, registry: Registry) -> None:
        registry.register("drain.check", _StubModule())
        draining_observed: list[bool] = []
        acquired = threading.Event()
        release = threading.Event()
        drain_started = threading.Event()

        def hold_execution() -> None:
            with registry.acquire("drain.check"):
                acquired.set()
                release.wait(timeout=5)

        def do_unregister() -> None:
            drain_started.set()
            registry.safe_unregister(module_id="drain.check", timeout_ms=3000)

        def check_draining() -> None:
            drain_started.wait(timeout=5)
            # Give unregister a moment to mark as draining
            threading.Event().wait(timeout=0.1)
            draining_observed.append(registry.is_draining("drain.check"))

        exec_thread = threading.Thread(target=hold_execution)
        exec_thread.start()
        acquired.wait(timeout=5)

        unreg_thread = threading.Thread(target=do_unregister)
        unreg_thread.start()

        check_thread = threading.Thread(target=check_draining)
        check_thread.start()
        check_thread.join(timeout=5)

        assert len(draining_observed) == 1
        assert draining_observed[0] is True

        release.set()
        exec_thread.join(timeout=5)
        unreg_thread.join(timeout=5)


class TestAcquireNormal:
    """acquire() works normally for active modules."""

    def test_acquire_yields_module(self, registry: Registry) -> None:
        mod = _StubModule()
        registry.register("active.mod", mod)

        with registry.acquire("active.mod") as acquired:
            assert acquired is mod

    def test_acquire_increments_and_decrements_refcount(self, registry: Registry) -> None:
        registry.register("refcount.mod", _StubModule())

        with registry.acquire("refcount.mod"):
            # During execution, ref count should be 1
            assert registry._ref_counts.get("refcount.mod", 0) == 1

        # After exit, ref count should be cleaned up
        assert registry._ref_counts.get("refcount.mod", 0) == 0

    def test_multiple_concurrent_acquires(self, registry: Registry) -> None:
        registry.register("multi.mod", _StubModule())
        barrier = threading.Barrier(3, timeout=5)
        ref_counts: list[int] = []
        lock = threading.Lock()

        def acquire_and_record() -> None:
            with registry.acquire("multi.mod"):
                barrier.wait()
                # Read ref count under the registry lock for consistency
                with registry._lock:
                    count = registry._ref_counts.get("multi.mod", 0)
                with lock:
                    ref_counts.append(count)
                # Hold acquire open until all threads have recorded
                barrier.wait()

        threads = [threading.Thread(target=acquire_and_record) for _ in range(3)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=5)

        # All three threads should have seen ref_count == 3
        assert len(ref_counts) == 3
        assert all(c == 3 for c in ref_counts)


class TestSafeUnregisterNonExistent:
    """safe_unregister() on a non-existent module returns False."""

    def test_returns_false_for_missing_module(self, registry: Registry) -> None:
        result = registry.safe_unregister("does.not.exist")
        assert result is False
