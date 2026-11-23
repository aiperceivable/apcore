"""Async-task behaviour that only a shared / persistent ``TaskStore`` exposes.

Three contracts, all of which the in-process-only happy path hides:

1. ``cancel()`` — the spec's cancel Contract defines exactly two ``False``
   cases: the task did not exist, or it had already reached a terminal state.
   A missing local ``asyncio.Task`` handle is not a third one; a record left
   PENDING/RUNNING by a previous process must still be cancellable, or it
   consumes the ``max_tasks`` active budget forever.
2. The runner's terminal writes — a status written by another process (or by a
   concurrent ``cancel``) must not be clobbered by the runner's stale
   in-memory snapshot. protocol-spec.md §5.8 forbids transitions out of a
   terminal state, and the retry path must not resurrect one as PENDING.
3. The reaper — it sweeps through ``TaskStore.list_expired`` so a store whose
   ``list_expired`` is the only efficient expiry query is actually used, and
   so the expiry predicate matches the TypeScript and Rust SDKs.
"""

from __future__ import annotations

import asyncio
import time
from typing import Any

import pytest

from apcore.async_task import (
    AsyncTaskManager,
    InMemoryTaskStore,
    RetryConfig,
    TaskInfo,
    TaskStatus,
)


class _StubExecutor:
    """Executor that returns immediately."""

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Any = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        return {"ok": True}


class _CancellingExecutor:
    """Executor that flips the stored record to CANCELLED, then raises.

    Stands in for "another process cancelled task T while this process was
    running attempt N" without needing a second process.
    """

    def __init__(self, store: Any, task_id: str) -> None:
        self._store = store
        self._task_id = task_id
        self.calls = 0

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Any = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        self.calls += 1
        info = await self._store.get(self._task_id)
        assert info is not None
        info.status = TaskStatus.CANCELLED
        info.completed_at = time.time()
        await self._store.save(info)
        raise RuntimeError("boom")


class _RecordingStore(InMemoryTaskStore):
    """In-memory store that records which query the reaper actually used."""

    def __init__(self) -> None:
        super().__init__()
        self.list_expired_calls: list[float] = []
        self.list_calls = 0

    async def list_expired(self, before_timestamp: float) -> list[TaskInfo]:
        self.list_expired_calls.append(before_timestamp)
        return await super().list_expired(before_timestamp)

    async def list(self, status: TaskStatus | None = None) -> list[TaskInfo]:
        self.list_calls += 1
        return await super().list(status)


def _record(task_id: str, status: TaskStatus, **kwargs: Any) -> TaskInfo:
    return TaskInfo(
        task_id=task_id,
        module_id="mod.probe",
        status=status,
        submitted_at=kwargs.pop("submitted_at", time.time()),
        **kwargs,
    )


# ---------------------------------------------------------------------------
# 1. cancel() without a local handle
# ---------------------------------------------------------------------------


class TestCancelWithoutLocalHandle:
    @pytest.mark.asyncio
    @pytest.mark.parametrize("status", [TaskStatus.PENDING, TaskStatus.RUNNING])
    async def test_cancel_returns_true_and_writes_cancelled(self, status: TaskStatus) -> None:
        """An active store record with no in-process handle is still cancellable."""
        store = InMemoryTaskStore()
        await store.save(_record("orphan", status))
        mgr = AsyncTaskManager(_StubExecutor(), store=store)

        assert await mgr.cancel("orphan") is True

        info = await store.get("orphan")
        assert info is not None
        assert info.status is TaskStatus.CANCELLED
        assert info.completed_at is not None

    @pytest.mark.asyncio
    async def test_cancel_frees_the_max_tasks_budget(self) -> None:
        """The uncancellable record used to pin the active-task budget forever."""
        store = InMemoryTaskStore()
        await store.save(_record("orphan", TaskStatus.RUNNING))
        mgr = AsyncTaskManager(_StubExecutor(), store=store, max_tasks=1)

        from apcore.errors import TaskLimitExceededError

        with pytest.raises(TaskLimitExceededError):
            await mgr.submit("mod.probe", {})

        assert await mgr.cancel("orphan") is True
        task_id = await mgr.submit("mod.probe", {})
        assert task_id
        await mgr.shutdown()

    @pytest.mark.asyncio
    async def test_cancel_still_returns_false_for_the_two_spec_cases(self) -> None:
        """False keeps meaning "no such task" or "already terminal" — nothing else."""
        store = InMemoryTaskStore()
        await store.save(_record("done", TaskStatus.COMPLETED, completed_at=time.time()))
        mgr = AsyncTaskManager(_StubExecutor(), store=store)

        assert await mgr.cancel("no-such-task") is False
        assert await mgr.cancel("done") is False

    @pytest.mark.asyncio
    async def test_shutdown_cancels_store_only_active_records(self) -> None:
        store = InMemoryTaskStore()
        await store.save(_record("orphan-a", TaskStatus.RUNNING))
        await store.save(_record("orphan-b", TaskStatus.PENDING))
        mgr = AsyncTaskManager(_StubExecutor(), store=store)

        await mgr.shutdown()

        for task_id in ("orphan-a", "orphan-b"):
            info = await store.get(task_id)
            assert info is not None
            assert info.status is TaskStatus.CANCELLED


# ---------------------------------------------------------------------------
# 2. The runner must not clobber an out-of-band terminal status
# ---------------------------------------------------------------------------


class TestRunnerDoesNotClobberTerminalStatus:
    """The runner is driven directly here.

    ``submit()`` would spawn a second, concurrent runner for the same task id
    and the assertions would race it; the contract under test is the runner's
    own write discipline, so the store record is seeded by hand instead.
    """

    @pytest.mark.asyncio
    async def test_failed_write_is_abandoned_when_the_record_is_cancelled(self) -> None:
        """FAILED must not overwrite a CANCELLED written while the attempt ran."""
        store = InMemoryTaskStore()
        await store.save(_record("t1", TaskStatus.PENDING))
        executor = _CancellingExecutor(store, "t1")
        mgr = AsyncTaskManager(executor, store=store)

        await mgr._run("t1", "mod.probe", {}, None, None)

        info = await store.get("t1")
        assert info is not None
        assert info.status is TaskStatus.CANCELLED
        assert info.error is None

    @pytest.mark.asyncio
    async def test_retry_does_not_resurrect_a_cancelled_record_as_pending(self) -> None:
        """The backoff write is a transition out of a terminal state (§5.8)."""
        store = InMemoryTaskStore()
        await store.save(_record("t1", TaskStatus.PENDING))
        executor = _CancellingExecutor(store, "t1")
        mgr = AsyncTaskManager(executor, store=store)

        await mgr._run(
            "t1",
            "mod.probe",
            {},
            None,
            RetryConfig(max_retries=3, retry_delay_ms=0),
        )

        info = await store.get("t1")
        assert info is not None
        assert info.status is TaskStatus.CANCELLED
        # The attempt that observed the cancel is the last one executed —
        # the retry must not re-enter the loop against a terminal record.
        assert executor.calls == 1

    @pytest.mark.asyncio
    async def test_running_write_is_abandoned_when_the_record_is_already_terminal(self) -> None:
        """A record cancelled before the attempt starts is never re-opened."""
        store = InMemoryTaskStore()
        await store.save(_record("done", TaskStatus.CANCELLED, completed_at=time.time()))
        executor = _StubExecutor()
        mgr = AsyncTaskManager(executor, store=store)

        await mgr._run("done", "mod.probe", {}, None, None)

        info = await store.get("done")
        assert info is not None
        assert info.status is TaskStatus.CANCELLED
        assert info.started_at is None

    @pytest.mark.asyncio
    async def test_completed_write_is_abandoned_when_the_record_is_cancelled(self) -> None:
        store = InMemoryTaskStore()
        await store.save(_record("t1", TaskStatus.PENDING))

        class _CancelThenSucceed:
            async def call_async(
                self,
                module_id: str,
                inputs: dict[str, Any] | None = None,
                context: Any = None,
                version_hint: str | None = None,
            ) -> dict[str, Any]:
                info = await store.get("t1")
                assert info is not None
                info.status = TaskStatus.CANCELLED
                info.completed_at = time.time()
                await store.save(info)
                return {"ok": True}

        mgr = AsyncTaskManager(_CancelThenSucceed(), store=store)
        await mgr._run("t1", "mod.probe", {}, None, None)

        info = await store.get("t1")
        assert info is not None
        assert info.status is TaskStatus.CANCELLED
        assert info.result is None

    @pytest.mark.asyncio
    async def test_a_normal_failure_still_records_failed(self) -> None:
        """The guard must not swallow the ordinary FAILED write."""

        class _BoomExecutor:
            async def call_async(
                self,
                module_id: str,
                inputs: dict[str, Any] | None = None,
                context: Any = None,
                version_hint: str | None = None,
            ) -> dict[str, Any]:
                raise RuntimeError("boom")

        store = InMemoryTaskStore()
        await store.save(_record("t1", TaskStatus.PENDING))
        mgr = AsyncTaskManager(_BoomExecutor(), store=store)

        await mgr._run("t1", "mod.probe", {}, None, None)

        info = await store.get("t1")
        assert info is not None
        assert info.status is TaskStatus.FAILED
        assert info.error == "boom"

    @pytest.mark.asyncio
    async def test_a_normal_retry_still_reaches_completed(self) -> None:
        """Retry/backoff still works when nothing cancels the record."""

        class _FailOnceExecutor:
            def __init__(self) -> None:
                self.calls = 0

            async def call_async(
                self,
                module_id: str,
                inputs: dict[str, Any] | None = None,
                context: Any = None,
                version_hint: str | None = None,
            ) -> dict[str, Any]:
                self.calls += 1
                if self.calls == 1:
                    raise RuntimeError("transient")
                return {"ok": True}

        store = InMemoryTaskStore()
        await store.save(_record("t1", TaskStatus.PENDING))
        executor = _FailOnceExecutor()
        mgr = AsyncTaskManager(executor, store=store)

        await mgr._run("t1", "mod.probe", {}, None, RetryConfig(max_retries=2, retry_delay_ms=0))

        info = await store.get("t1")
        assert info is not None
        assert info.status is TaskStatus.COMPLETED
        assert info.result == {"ok": True}
        assert info.retry_count == 1
        assert executor.calls == 2


# ---------------------------------------------------------------------------
# 3. The reaper sweeps through TaskStore.list_expired
# ---------------------------------------------------------------------------


class TestReaperUsesListExpired:
    @pytest.mark.asyncio
    async def test_reaper_invokes_the_store_list_expired_query(self) -> None:
        """A custom store's ``list_expired`` is the query the reaper runs."""
        store = _RecordingStore()
        await store.save(_record("old", TaskStatus.COMPLETED, completed_at=time.time() - 500.0))
        mgr = AsyncTaskManager(_StubExecutor(), store=store)

        handle = mgr.start_reaper(ttl_seconds=1.0, sweep_interval_ms=10)
        try:
            for _ in range(200):
                if store.list_expired_calls:
                    break
                await asyncio.sleep(0.01)
        finally:
            await handle.stop()

        assert store.list_expired_calls, "reaper never called TaskStore.list_expired"
        # The threshold is now - ttl_seconds, as in the TS and Rust SDKs.
        assert store.list_expired_calls[0] == pytest.approx(time.time() - 1.0, abs=5.0)
        assert store.list_calls == 0
        assert await store.get("old") is None

    @pytest.mark.asyncio
    async def test_reaper_leaves_a_terminal_record_without_completed_at(self) -> None:
        """``list_expired`` deliberately has no ``submitted_at`` fallback.

        ``cleanup()`` keeps that fallback as its own documented behaviour, so
        the two predicates differ and the reaper must use the store's.
        """
        store = _RecordingStore()
        await store.save(
            _record(
                "no-completed-at",
                TaskStatus.FAILED,
                submitted_at=time.time() - 5000.0,
                completed_at=None,
            )
        )
        mgr = AsyncTaskManager(_StubExecutor(), store=store)

        handle = mgr.start_reaper(ttl_seconds=1.0, sweep_interval_ms=10)
        try:
            for _ in range(200):
                if store.list_expired_calls:
                    break
                await asyncio.sleep(0.01)
        finally:
            await handle.stop()

        assert await store.get("no-completed-at") is not None
        # ...while the public cleanup() surface still reaps it.
        assert await mgr.cleanup(1.0) == 1
        assert await store.get("no-completed-at") is None

    @pytest.mark.asyncio
    async def test_reaper_survives_a_failing_store(self) -> None:
        """A raising ``list_expired`` warns and retries; it must not kill the loop."""

        class _BrokenStore(InMemoryTaskStore):
            def __init__(self) -> None:
                super().__init__()
                self.attempts = 0

            async def list_expired(self, before_timestamp: float) -> list[TaskInfo]:
                self.attempts += 1
                raise RuntimeError("backend unavailable")

        store = _BrokenStore()
        mgr = AsyncTaskManager(_StubExecutor(), store=store)

        handle = mgr.start_reaper(ttl_seconds=1.0, sweep_interval_ms=10)
        try:
            for _ in range(200):
                if store.attempts >= 2:
                    break
                await asyncio.sleep(0.01)
        finally:
            await handle.stop()

        assert store.attempts >= 2, "reaper loop stopped after the first failure"
