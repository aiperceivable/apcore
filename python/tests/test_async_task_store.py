"""Regression tests for async-task sync-audit findings against v0.22.0.

Each test pins a normative decision recorded in the apcore spec:

- A-D-AT-03 / D-11 — ``ReaperHandle.stop`` is async and drains the task.
- A-D-AT-04 / D-17 — ``TaskStore`` Protocol methods are all async.
- A-D-AT-06 — ``AsyncTaskManager.get_status`` / ``list_tasks`` return copies.
- A-D-AT-09 / D-14 — Legacy ``RetryPolicy`` defaults to ``max_retries=0``
  and emits a ``DeprecationWarning`` on instantiation.
"""

from __future__ import annotations

import asyncio
import inspect
import warnings
from typing import Any

import pytest

from apcore.async_task import (
    AsyncTaskManager,
    InMemoryTaskStore,
    ReaperHandle,
    RetryPolicy,
    TaskInfo,
    TaskStatus,
    TaskStore,
)


class _StubExecutor:
    """Minimal executor implementing the protocol surface for the manager."""

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Any = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        await asyncio.sleep(0)  # yield once so cancellation paths still race
        return {"ok": True}


# ---------------------------------------------------------------------------
# A-D-AT-04 / D-17 — TaskStore Protocol methods are async
# ---------------------------------------------------------------------------


class TestTaskStoreProtocolIsAsync:
    def test_in_memory_store_methods_are_coroutines(self) -> None:
        store = InMemoryTaskStore()
        for name in ("save", "get", "delete", "list", "list_expired"):
            assert inspect.iscoroutinefunction(getattr(store, name)), f"InMemoryTaskStore.{name} must be async (D-17)"

    def test_protocol_methods_declared_async(self) -> None:
        # Walk the Protocol annotations: every declared method must be an
        # async function so custom stores share the contract.
        for name in ("save", "get", "delete", "list", "list_expired"):
            method = getattr(TaskStore, name)
            assert inspect.iscoroutinefunction(method), f"TaskStore Protocol method {name!r} must be async (D-17)"


# ---------------------------------------------------------------------------
# A-D-AT-03 / D-11 — ReaperHandle.stop is async and drains the task
# ---------------------------------------------------------------------------


class TestReaperHandleStopIsAsync:
    def test_reaper_handle_stop_is_coroutine_function(self) -> None:
        assert inspect.iscoroutinefunction(ReaperHandle.stop)

    @pytest.mark.asyncio
    async def test_reaper_handle_stop_awaits_underlying_task(self) -> None:
        mgr = AsyncTaskManager(executor=_StubExecutor())
        handle = mgr.start_reaper(ttl_seconds=1.0, sweep_interval_ms=10)
        assert handle.is_running()
        await handle.stop()
        # After stop returns, the underlying task is guaranteed to be done —
        # no manual ``await asyncio.sleep(0)`` should be necessary.
        assert mgr._reaper_task is None
        assert not handle.is_running()

    @pytest.mark.asyncio
    async def test_stop_reaper_idempotent_when_already_stopped(self) -> None:
        mgr = AsyncTaskManager(executor=_StubExecutor())
        mgr.start_reaper(ttl_seconds=1.0, sweep_interval_ms=10)
        await mgr.stop_reaper()
        # Second call must be a clean no-op.
        await mgr.stop_reaper()


# ---------------------------------------------------------------------------
# A-D-AT-06 — get_status / list_tasks return mutation-safe copies
# ---------------------------------------------------------------------------


class TestGetStatusReturnsSnapshot:
    @pytest.mark.asyncio
    async def test_get_status_returns_copy(self) -> None:
        mgr = AsyncTaskManager(executor=_StubExecutor())
        task_id = await mgr.submit("test.x", {})
        await asyncio.sleep(0.05)
        snapshot = mgr.get_status(task_id)
        assert snapshot is not None
        # Mutate the snapshot — the store's record must not be affected.
        snapshot.status = TaskStatus.FAILED
        snapshot.error = "tampered"
        fresh = mgr.get_status(task_id)
        assert fresh is not None
        assert fresh.status != TaskStatus.FAILED or fresh.error != "tampered"
        # And the two returned objects must not be the *same* instance —
        # otherwise callers can still share state by aliasing.
        again = mgr.get_status(task_id)
        assert again is not None
        assert again is not snapshot

    @pytest.mark.asyncio
    async def test_list_tasks_returns_copies(self) -> None:
        mgr = AsyncTaskManager(executor=_StubExecutor())
        await mgr.submit("test.x", {})
        await asyncio.sleep(0.05)
        first = mgr.list_tasks()
        second = mgr.list_tasks()
        assert first and second
        # Distinct list elements per call (defensive copies).
        assert first[0] is not second[0]
        # Mutating the returned snapshot must not leak into the store.
        first[0].error = "tampered"
        again = mgr.list_tasks()
        assert again[0].error != "tampered"


# ---------------------------------------------------------------------------
# A-D-AT-09 / D-14 — RetryPolicy default + deprecation warning
# ---------------------------------------------------------------------------


class TestRetryPolicyOptInDefault:
    def test_default_max_retries_is_zero(self) -> None:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            policy = RetryPolicy()
        assert policy.max_retries == 0, "RetryPolicy() must default to max_retries=0 (D-14) so retries are opt-in"

    def test_instantiation_emits_deprecation_warning(self) -> None:
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            RetryPolicy(max_retries=1)
        deprecations = [w for w in caught if issubclass(w.category, DeprecationWarning)]
        assert deprecations, (
            "Constructing RetryPolicy must emit a DeprecationWarning steering " "callers to RetryConfig (A-D-AT-09)"
        )


# ---------------------------------------------------------------------------
# Sanity check that the manager survives an async-only custom store
# ---------------------------------------------------------------------------


class _AsyncOnlyStore:
    """Custom store whose methods perform a real await — proves the manager
    no longer relies on synchronous fall-through for store I/O."""

    def __init__(self) -> None:
        self._data: dict[str, TaskInfo] = {}

    async def get(self, task_id: str) -> TaskInfo | None:
        await asyncio.sleep(0)
        return self._data.get(task_id)

    async def save(self, info: TaskInfo) -> None:
        await asyncio.sleep(0)
        self._data[info.task_id] = info

    async def delete(self, task_id: str) -> None:
        await asyncio.sleep(0)
        self._data.pop(task_id, None)

    async def list(self, status: TaskStatus | None = None) -> list[TaskInfo]:
        await asyncio.sleep(0)
        if status is None:
            return list(self._data.values())
        return [t for t in self._data.values() if t.status == status]

    async def list_expired(self, before_timestamp: float) -> list[TaskInfo]:
        await asyncio.sleep(0)
        return [t for t in self._data.values() if t.completed_at is not None and t.completed_at < before_timestamp]


class TestManagerWithAsyncStore:
    @pytest.mark.asyncio
    async def test_submit_and_inspect_works_against_async_store(self) -> None:
        mgr = AsyncTaskManager(executor=_StubExecutor(), store=_AsyncOnlyStore())
        task_id = await mgr.submit("test.x", {})
        await asyncio.sleep(0.05)
        # ``get_status_async`` is the async-friendly surface for I/O-backed stores.
        info = await mgr.get_status_async(task_id)
        assert info is not None
        assert info.task_id == task_id
        listed = await mgr.list_tasks_async()
        assert any(t.task_id == task_id for t in listed)


class TestGetResultAsync:
    """#123: ``get_result`` pumps the store synchronously, so I/O-backed stores need an async twin."""

    @pytest.mark.asyncio
    async def test_get_result_async_reads_an_io_backed_store(self) -> None:
        mgr = AsyncTaskManager(executor=_StubExecutor(), store=_AsyncOnlyStore())
        task_id = await mgr.submit("test.x", {})
        await asyncio.sleep(0.05)

        with pytest.raises(RuntimeError, match="suspended"):
            mgr.get_result(task_id)
        assert await mgr.get_result_async(task_id) == {"ok": True}

    @pytest.mark.asyncio
    async def test_get_result_async_unknown_task_raises_key_error(self) -> None:
        mgr = AsyncTaskManager(executor=_StubExecutor(), store=_AsyncOnlyStore())
        with pytest.raises(KeyError):
            await mgr.get_result_async("missing")

    @pytest.mark.asyncio
    async def test_get_result_async_incomplete_task_raises_runtime_error(self) -> None:
        store = _AsyncOnlyStore()
        mgr = AsyncTaskManager(executor=_StubExecutor(), store=store)
        await store.save(TaskInfo(task_id="t1", module_id="test.x", status=TaskStatus.PENDING, submitted_at=0.0))
        with pytest.raises(RuntimeError, match="not completed"):
            await mgr.get_result_async("t1")
