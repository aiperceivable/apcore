"""Tests for AsyncTaskManager / TaskStore cross-language alignment (D-10/12/13).

Covers:
- D-10: ``TaskStore.put`` → ``save`` rename with deprecated shim,
        new ``TaskStore.list_expired`` method.
- D-12: ``TaskStatus.RETRYING`` removal — backoff state is now ``PENDING``.
- D-13: ``TaskInfo.attempt_number`` → ``retry_count`` rename with deprecated
        ``attempt_number`` property alias.
"""

from __future__ import annotations

import asyncio
import time
import uuid
import warnings
from typing import Any

import pytest

from apcore.async_task import (
    AsyncTaskManager,
    BackoffStrategy,
    InMemoryTaskStore,
    RetryPolicy,
    TaskInfo,
    TaskStatus,
)
from apcore.context import Context
from apcore.executor import Executor
from apcore.registry import Registry


# ---------------------------------------------------------------------------
# D-10: TaskStore.save and list_expired
# ---------------------------------------------------------------------------


class TestSaveRename:
    @pytest.mark.asyncio
    async def test_save_stores_task(self) -> None:
        store = InMemoryTaskStore()
        info = TaskInfo(task_id="t1", module_id="m", status=TaskStatus.PENDING, submitted_at=time.time())
        await store.save(info)
        assert await store.get("t1") is info

    @pytest.mark.asyncio
    async def test_put_emits_deprecation_warning(self) -> None:
        store = InMemoryTaskStore()
        info = TaskInfo(task_id="t2", module_id="m", status=TaskStatus.PENDING, submitted_at=time.time())
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            await store.put(info)
        # At least one DeprecationWarning mentioning save should have fired.
        assert any(issubclass(w.category, DeprecationWarning) for w in caught)
        # Functional behaviour preserved.
        assert await store.get("t2") is info


class TestListExpired:
    @pytest.mark.asyncio
    async def test_list_expired_returns_terminal_tasks_older_than(self) -> None:
        store = InMemoryTaskStore()
        old_completed = TaskInfo(
            task_id="old",
            module_id="m",
            status=TaskStatus.COMPLETED,
            submitted_at=100.0,
            completed_at=100.0,
        )
        recent = TaskInfo(
            task_id="recent",
            module_id="m",
            status=TaskStatus.COMPLETED,
            submitted_at=time.time(),
            completed_at=time.time(),
        )
        active = TaskInfo(
            task_id="active",
            module_id="m",
            status=TaskStatus.RUNNING,
            submitted_at=time.time(),
        )
        await store.save(old_completed)
        await store.save(recent)
        await store.save(active)

        expired = await store.list_expired(before_timestamp=200.0)
        ids = {t.task_id for t in expired}
        # old_completed is older than 200 → expired.  recent is newer.
        # active is non-terminal → never expired.
        assert "old" in ids
        assert "recent" not in ids
        assert "active" not in ids

    @pytest.mark.asyncio
    async def test_list_expired_excludes_terminal_task_with_null_completed_at(self) -> None:
        """Regression (A-D-004): a terminal task whose ``completed_at`` is None
        MUST NOT be returned, even when ``submitted_at`` predates the threshold.

        The TaskStore.list_expired contract (async-tasks.md) and the TS/Rust
        SDKs require ``completed_at`` to be non-null; there is no ``submitted_at``
        fallback for this method (that fallback belongs to cleanup, not here).
        """
        store = InMemoryTaskStore()
        # Terminal status but no completed_at, with an old submitted_at.
        terminal_no_completed = TaskInfo(
            task_id="terminal_no_completed",
            module_id="m",
            status=TaskStatus.CANCELLED,
            submitted_at=100.0,
            completed_at=None,
        )
        await store.save(terminal_no_completed)

        expired = await store.list_expired(before_timestamp=200.0)
        ids = {t.task_id for t in expired}
        # submitted_at (100) < 200, but completed_at is None → MUST NOT expire.
        assert "terminal_no_completed" not in ids


# ---------------------------------------------------------------------------
# D-12: TaskStatus.RETRYING removed — backoff state is PENDING
# ---------------------------------------------------------------------------


class _FailingModule:
    input_schema = None
    output_schema = None

    def execute(self, _inputs: dict[str, Any], _context: Context) -> dict[str, Any]:
        raise RuntimeError("intentional failure")


class _SpyStore:
    def __init__(self) -> None:
        self._data: dict[str, TaskInfo] = {}
        self.statuses: list[TaskStatus] = []

    async def get(self, task_id: str) -> TaskInfo | None:
        return self._data.get(task_id)

    async def save(self, info: TaskInfo) -> None:
        self._data[info.task_id] = info
        self.statuses.append(info.status)

    async def put(self, info: TaskInfo) -> None:  # back-compat shim
        await self.save(info)

    async def delete(self, task_id: str) -> None:
        self._data.pop(task_id, None)

    async def list(self, status: TaskStatus | None = None) -> list[TaskInfo]:
        if status is None:
            return list(self._data.values())
        return [t for t in self._data.values() if t.status == status]

    async def list_expired(self, before_timestamp: float) -> list[TaskInfo]:
        return [t for t in self._data.values() if t.completed_at is not None and t.completed_at < before_timestamp]


class TestStatusDuringBackoffIsPending:
    @pytest.mark.asyncio
    async def test_backoff_status_is_pending_not_retrying(self) -> None:
        reg = Registry()
        reg.register("test.failing", _FailingModule())
        executor = Executor(registry=reg)

        spy = _SpyStore()
        policy = RetryPolicy(max_retries=1, backoff=BackoffStrategy.FIXED, base_delay_seconds=0.01)
        mgr = AsyncTaskManager(executor, store=spy)
        await mgr.submit("test.failing", {}, retry_policy=policy)
        await asyncio.sleep(0.3)

        # The new contract: during backoff, status is PENDING — never the
        # removed "retrying" string value.  Compare by raw value to avoid
        # the deprecated TaskStatus.RETRYING alias (which now resolves to
        # PENDING and would obscure the test).
        retrying_strings = [s.value for s in spy.statuses if s.value == "retrying"]
        assert retrying_strings == [], "backoff state should be PENDING, not RETRYING"

        # And we should still see PENDING multiple times (initial + retry backoffs).
        assert spy.statuses.count(TaskStatus.PENDING) >= 2


# ---------------------------------------------------------------------------
# D-13: attempt_number → retry_count rename with deprecation alias
# ---------------------------------------------------------------------------


class TestRetryCountRename:
    def test_retry_count_field_accessible(self) -> None:
        info = TaskInfo(
            task_id=str(uuid.uuid4()),
            module_id="m",
            status=TaskStatus.PENDING,
            submitted_at=time.time(),
        )
        assert info.retry_count == 0
        info.retry_count = 3
        assert info.retry_count == 3

    def test_attempt_number_property_alias_emits_deprecation(self) -> None:
        info = TaskInfo(
            task_id=str(uuid.uuid4()),
            module_id="m",
            status=TaskStatus.PENDING,
            submitted_at=time.time(),
            retry_count=2,
        )
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            value = info.attempt_number
        assert value == 2
        assert any(issubclass(w.category, DeprecationWarning) for w in caught)

    def test_attempt_number_setter_writes_through_with_deprecation(self) -> None:
        info = TaskInfo(
            task_id=str(uuid.uuid4()),
            module_id="m",
            status=TaskStatus.PENDING,
            submitted_at=time.time(),
        )
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            info.attempt_number = 5
        assert info.retry_count == 5
        assert any(issubclass(w.category, DeprecationWarning) for w in caught)
