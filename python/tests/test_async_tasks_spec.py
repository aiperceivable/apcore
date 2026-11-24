"""Spec-traced contract tests for the async-tasks feature.

Source spec: apcore/docs/features/async-tasks.md (13 '## Contract:' blocks).

Every test carries its verbatim clause id in the form
``async_tasks.<method>.<kind>.<detail>`` inside the docstring so cross-language
diffs line up row-for-row with the TypeScript and Rust suites.

These tests exercise OBSERVABLE behaviour only — they never reach into private
attributes to assert state. Where the implementation surfaces a contract error
asynchronously (``submit`` spawns the executor call into a background task), the
test drains the task to its terminal state and asserts the observable outcome.

Framework: pytest + pytest-asyncio (asyncio_mode=auto) — plain ``async def``.
"""

from __future__ import annotations

import asyncio
import time
from typing import Any

import pytest

from apcore.async_task import (
    AsyncTaskManager,
    InMemoryTaskStore,
    TaskInfo,
    TaskStatus,
)
from apcore.context import Context
from apcore.errors import ErrorCodes, InvalidInputError, ModuleError, TaskLimitExceededError
from apcore.executor import Executor
from apcore.registry import Registry


# === Helper modules ===


class EchoModule:
    """Module that echoes a value from inputs — completes immediately."""

    input_schema = None
    output_schema = None

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"value": inputs.get("x", 0)}


class SlowModule:
    """Async module that sleeps for a configurable duration then returns."""

    input_schema = None
    output_schema = None

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        await asyncio.sleep(inputs.get("delay", 0.5))
        return {"done": True}


class FailingModule:
    """Module that always raises a deterministic error."""

    input_schema = None
    output_schema = None

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        raise RuntimeError("intentional failure")


# === Fixtures ===


@pytest.fixture
def registry() -> Registry:
    reg = Registry()
    reg.register("test.echo", EchoModule())
    reg.register("test.slow", SlowModule())
    reg.register("test.failing", FailingModule())
    return reg


@pytest.fixture
def executor(registry: Registry) -> Executor:
    return Executor(registry=registry)


@pytest.fixture
def manager(executor: Executor) -> AsyncTaskManager:
    return AsyncTaskManager(executor, max_concurrent=10, max_tasks=1000)


# === Helpers ===


async def _drain(manager: AsyncTaskManager, task_id: str, timeout: float = 2.0) -> TaskInfo:
    """Await until ``task_id`` reaches a terminal state and return its snapshot."""
    deadline = time.monotonic() + timeout
    terminal = {TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED}
    while time.monotonic() < deadline:
        info = manager.get_status(task_id)
        if info is not None and info.status in terminal:
            return info
        await asyncio.sleep(0.01)
    raise AssertionError(f"task {task_id} did not reach a terminal state within {timeout}s")


# ---------------------------------------------------------------------------
# Contract: AsyncTaskManager.submit
# ---------------------------------------------------------------------------


async def test_submit_input_module_id_malformed(manager: AsyncTaskManager) -> None:
    """async_tasks.submit.input.module_id.malformed

    A malformed module_id must surface InvalidInputError with code
    INVALID_MODULE_ID. The implementation validates module_id inside the
    spawned executor call, so the failure is observable as the task's
    terminal FAILED state carrying the error message.
    """
    task_id = await manager.submit("Bad-ID!", {"x": 1})
    info = await _drain(manager, task_id)
    assert info.status is TaskStatus.FAILED
    assert info.error is not None
    # The error originates from Executor._validate_module_id -> InvalidInputError.
    assert "Invalid module ID" in info.error
    # Direct validation path confirms the declared TYPE + CODE pairing.
    with pytest.raises(InvalidInputError) as exc:
        Executor._validate_module_id("Bad-ID!")
    assert exc.value.code == ErrorCodes.INVALID_MODULE_ID


async def test_submit_error_module_not_found(manager: AsyncTaskManager) -> None:
    """async_tasks.submit.error.MODULE_NOT_FOUND

    Submitting a well-formed but unregistered module_id must drive the task
    to FAILED, with the underlying executor error code MODULE_NOT_FOUND.
    """
    task_id = await manager.submit("no.such.module", {"x": 1})
    info = await _drain(manager, task_id)
    assert info.status is TaskStatus.FAILED
    assert info.error is not None
    assert "no.such.module" in info.error


async def test_submit_error_task_limit_exceeded(executor: Executor) -> None:
    """async_tasks.submit.error.TASK_LIMIT_EXCEEDED

    With max_tasks=1, a second concurrent active submission must raise the
    typed TaskLimitExceededError whose code is TASK_LIMIT_EXCEEDED.
    """
    limited = AsyncTaskManager(executor, max_concurrent=10, max_tasks=1)
    await limited.submit("test.slow", {"delay": 0.5})
    with pytest.raises(TaskLimitExceededError) as exc:
        await limited.submit("test.slow", {"delay": 0.5})
    assert exc.value.code == ErrorCodes.TASK_LIMIT_EXCEEDED
    await limited.shutdown()


async def test_submit_property_async(manager: AsyncTaskManager) -> None:
    """async_tasks.submit.property.async

    submit is awaitable and resolves to a string task_id.
    """
    coro = manager.submit("test.echo", {"x": 7})
    assert asyncio.iscoroutine(coro)
    task_id = await coro
    assert isinstance(task_id, str) and len(task_id) > 0
    info = await _drain(manager, task_id)
    assert info.status is TaskStatus.COMPLETED


async def test_submit_property_thread_safe(manager: AsyncTaskManager) -> None:
    """async_tasks.submit.property.thread_safe

    N>=8 concurrent submissions with distinct inputs complete without error
    and produce N distinct task_ids with consistent final state.
    """
    n = 12
    task_ids = await asyncio.gather(*(manager.submit("test.echo", {"x": i}) for i in range(n)))
    assert len(set(task_ids)) == n
    infos = [await _drain(manager, tid) for tid in task_ids]
    assert all(info.status is TaskStatus.COMPLETED for info in infos)
    assert {info.result["value"] for info in infos} == set(range(n))


async def test_submit_property_not_idempotent(manager: AsyncTaskManager) -> None:
    """async_tasks.submit.property.idempotent_false

    submit is declared NOT idempotent: two identical calls create two
    distinct tasks (distinct task_ids, both tracked).
    """
    first = await manager.submit("test.echo", {"x": 1})
    second = await manager.submit("test.echo", {"x": 1})
    assert first != second
    assert {t.task_id for t in manager.list_tasks()} >= {first, second}


# ---------------------------------------------------------------------------
# Contract: AsyncTaskManager.cancel
# ---------------------------------------------------------------------------


async def test_cancel_property_async(manager: AsyncTaskManager) -> None:
    """async_tasks.cancel.property.async

    cancel is awaitable and resolves to a bool.
    """
    task_id = await manager.submit("test.slow", {"delay": 1.0})
    coro = manager.cancel(task_id)
    assert asyncio.iscoroutine(coro)
    result = await coro
    assert result is True
    info = manager.get_status(task_id)
    assert info is not None and info.status is TaskStatus.CANCELLED


async def test_cancel_returns_false_for_unknown(manager: AsyncTaskManager) -> None:
    """async_tasks.cancel.return.unknown_task_false

    Cancelling a non-existent task_id returns False (no error raised).
    """
    result = await manager.cancel("does-not-exist")
    assert result is False


async def test_cancel_property_thread_safe(executor: Executor) -> None:
    """async_tasks.cancel.property.thread_safe

    Concurrently cancelling N>=8 distinct running tasks raises no exception
    and leaves every task in CANCELLED state.
    """
    mgr = AsyncTaskManager(executor, max_concurrent=20, max_tasks=1000)
    n = 8
    task_ids = [await mgr.submit("test.slow", {"delay": 1.0}) for _ in range(n)]
    await asyncio.sleep(0.05)  # let them reach RUNNING
    results = await asyncio.gather(*(mgr.cancel(tid) for tid in task_ids))
    assert all(r is True for r in results)
    for tid in task_ids:
        info = mgr.get_status(tid)
        assert info is not None and info.status is TaskStatus.CANCELLED
    await mgr.shutdown()


async def test_cancel_property_idempotent(manager: AsyncTaskManager) -> None:
    """async_tasks.cancel.property.idempotent

    Calling cancel twice: first applies cancellation (True), the second is a
    no-op (False). Observable state stays CANCELLED across both calls.
    """
    task_id = await manager.submit("test.slow", {"delay": 1.0})
    await asyncio.sleep(0.02)
    first = await manager.cancel(task_id)
    second = await manager.cancel(task_id)
    assert first is True
    assert second is False
    info = manager.get_status(task_id)
    assert info is not None and info.status is TaskStatus.CANCELLED


# ---------------------------------------------------------------------------
# Contract: AsyncTaskManager.get_status
# ---------------------------------------------------------------------------


async def test_get_status_property_not_async(manager: AsyncTaskManager) -> None:
    """async_tasks.get_status.property.async_false

    In Python get_status is synchronous (async:false): it returns a TaskInfo,
    not a coroutine.
    """
    task_id = await manager.submit("test.echo", {"x": 1})
    info = manager.get_status(task_id)
    assert not asyncio.iscoroutine(info)
    assert isinstance(info, TaskInfo)
    assert info.task_id == task_id


async def test_get_status_returns_shallow_copy(manager: AsyncTaskManager) -> None:
    """async_tasks.get_status.return.shallow_copy

    The returned object is a shallow copy (D-23): mutating it must not
    propagate back to the store.
    """
    task_id = await manager.submit("test.echo", {"x": 1})
    info = manager.get_status(task_id)
    assert info is not None
    info.module_id = "tampered"
    again = manager.get_status(task_id)
    assert again is not None and again.module_id == "test.echo"


async def test_get_status_property_idempotent(manager: AsyncTaskManager) -> None:
    """async_tasks.get_status.property.idempotent

    Repeated reads of the same terminal task return equal snapshots without
    altering observable state.
    """
    task_id = await manager.submit("test.echo", {"x": 9})
    await _drain(manager, task_id)
    a = manager.get_status(task_id)
    b = manager.get_status(task_id)
    assert a is not None and b is not None
    assert (a.task_id, a.status, a.result) == (b.task_id, b.status, b.result)


async def test_get_status_unknown_returns_none(manager: AsyncTaskManager) -> None:
    """async_tasks.get_status.return.unknown_none

    An unknown task_id returns None rather than raising.
    """
    assert manager.get_status("nope") is None


# ---------------------------------------------------------------------------
# Contract: AsyncTaskManager.get_result
# ---------------------------------------------------------------------------


async def test_get_result_error_not_found(manager: AsyncTaskManager) -> None:
    """async_tasks.get_result.error.task_not_found

    get_result raises KeyError when no task with the id exists.
    """
    with pytest.raises(KeyError):
        manager.get_result("missing")


async def test_get_result_error_not_completed(manager: AsyncTaskManager) -> None:
    """async_tasks.get_result.error.not_completed

    get_result raises RuntimeError when the task exists but is not COMPLETED
    (here: RUNNING).
    """
    task_id = await manager.submit("test.slow", {"delay": 1.0})
    await asyncio.sleep(0.02)
    with pytest.raises(RuntimeError) as exc:
        manager.get_result(task_id)
    assert "not completed" in str(exc.value)
    await manager.cancel(task_id)


async def test_get_result_returns_result_when_completed(manager: AsyncTaskManager) -> None:
    """async_tasks.get_result.return.completed_result

    get_result returns the module output once the task is COMPLETED.
    """
    task_id = await manager.submit("test.echo", {"x": 42})
    await _drain(manager, task_id)
    assert manager.get_result(task_id) == {"value": 42}


async def test_get_result_property_idempotent(manager: AsyncTaskManager) -> None:
    """async_tasks.get_result.property.idempotent

    Two get_result calls on the same completed task return identical output.
    """
    task_id = await manager.submit("test.echo", {"x": 5})
    await _drain(manager, task_id)
    assert manager.get_result(task_id) == manager.get_result(task_id) == {"value": 5}


# ---------------------------------------------------------------------------
# Contract: AsyncTaskManager.list_tasks
# ---------------------------------------------------------------------------


async def test_list_tasks_filter_by_status(manager: AsyncTaskManager) -> None:
    """async_tasks.list_tasks.input.status.filter

    When a status is supplied, only tasks with that exact status are returned.
    """
    completed_id = await manager.submit("test.echo", {"x": 1})
    await _drain(manager, completed_id)
    running_id = await manager.submit("test.slow", {"delay": 1.0})
    await asyncio.sleep(0.02)
    completed = manager.list_tasks(status=TaskStatus.COMPLETED)
    assert [t.task_id for t in completed] == [completed_id]
    running = manager.list_tasks(status=TaskStatus.RUNNING)
    assert running_id in {t.task_id for t in running}
    await manager.cancel(running_id)


async def test_list_tasks_returns_shallow_copies(manager: AsyncTaskManager) -> None:
    """async_tasks.list_tasks.return.shallow_copy

    Each entry is a shallow copy (D-23): mutating a listed entry must not
    affect the stored task.
    """
    task_id = await manager.submit("test.echo", {"x": 1})
    await _drain(manager, task_id)
    listed = manager.list_tasks()
    assert listed
    listed[0].module_id = "tampered"
    again = manager.get_status(task_id)
    assert again is not None and again.module_id == "test.echo"


async def test_list_tasks_property_idempotent(manager: AsyncTaskManager) -> None:
    """async_tasks.list_tasks.property.idempotent

    Repeated list_tasks calls over an unchanged store return the same set of
    task_ids without mutating state.
    """
    ids = [await manager.submit("test.echo", {"x": i}) for i in range(3)]
    for tid in ids:
        await _drain(manager, tid)
    first = {t.task_id for t in manager.list_tasks()}
    second = {t.task_id for t in manager.list_tasks()}
    assert first == second == set(ids)


# ---------------------------------------------------------------------------
# Contract: AsyncTaskManager.cleanup
# ---------------------------------------------------------------------------


async def test_cleanup_removes_only_terminal(manager: AsyncTaskManager) -> None:
    """async_tasks.cleanup.eligible.terminal_only

    cleanup removes terminal-state tasks past the age threshold and never
    removes PENDING/RUNNING tasks.
    """
    done_id = await manager.submit("test.echo", {"x": 1})
    await _drain(manager, done_id)
    running_id = await manager.submit("test.slow", {"delay": 1.0})
    await asyncio.sleep(0.02)
    # max_age_seconds=0 makes every terminal task eligible immediately.
    removed = await manager.cleanup(max_age_seconds=0.0)
    assert removed == 1
    assert manager.get_status(done_id) is None
    assert manager.get_status(running_id) is not None
    await manager.cancel(running_id)


async def test_cleanup_property_not_idempotent(manager: AsyncTaskManager) -> None:
    """async_tasks.cleanup.property.idempotent_false

    cleanup is declared non-idempotent: the first call removes the eligible
    task (count 1); a second identical call removes nothing (count 0) because
    the state already changed.
    """
    done_id = await manager.submit("test.echo", {"x": 1})
    await _drain(manager, done_id)
    first = await manager.cleanup(max_age_seconds=0.0)
    second = await manager.cleanup(max_age_seconds=0.0)
    assert first == 1
    assert second == 0


# ---------------------------------------------------------------------------
# Contract: AsyncTaskManager.shutdown
# ---------------------------------------------------------------------------


async def test_shutdown_property_async(executor: Executor) -> None:
    """async_tasks.shutdown.property.async

    shutdown is awaitable and resolves to None.
    """
    mgr = AsyncTaskManager(executor, max_concurrent=10, max_tasks=1000)
    coro = mgr.shutdown()
    assert asyncio.iscoroutine(coro)
    assert await coro is None


async def test_shutdown_side_effect_1_all_active_cancelled(executor: Executor) -> None:
    """async_tasks.shutdown.side_effect.1.cancel_active

    After shutdown, every task that was PENDING/RUNNING is CANCELLED — observed
    via post-state queries on each task.
    """
    mgr = AsyncTaskManager(executor, max_concurrent=20, max_tasks=1000)
    ids = [await mgr.submit("test.slow", {"delay": 1.0}) for _ in range(8)]
    await asyncio.sleep(0.05)
    await mgr.shutdown()
    for tid in ids:
        info = mgr.get_status(tid)
        assert info is not None and info.status is TaskStatus.CANCELLED


async def test_shutdown_property_idempotent(executor: Executor) -> None:
    """async_tasks.shutdown.property.idempotent

    Calling shutdown twice (the second on an already-quiesced manager) is a
    no-op and does not raise; task state is unchanged across the second call.
    """
    mgr = AsyncTaskManager(executor, max_concurrent=10, max_tasks=1000)
    task_id = await mgr.submit("test.slow", {"delay": 1.0})
    await asyncio.sleep(0.05)
    await mgr.shutdown()
    before = mgr.get_status(task_id)
    await mgr.shutdown()
    after = mgr.get_status(task_id)
    assert before is not None and after is not None
    assert before.status is TaskStatus.CANCELLED
    assert after.status is TaskStatus.CANCELLED


# ---------------------------------------------------------------------------
# Contract: AsyncTaskManager.start_reaper
# ---------------------------------------------------------------------------


async def test_start_reaper_property_async(manager: AsyncTaskManager) -> None:
    """async_tasks.start_reaper.property.async

    start_reaper spawns a background sweep loop and returns a ReaperHandle
    whose stop() is awaitable. (Python start_reaper itself returns the handle
    synchronously; the spawned loop is the async/background effect.)
    """
    handle = manager.start_reaper(ttl_seconds=3600.0, sweep_interval_ms=300_000)
    assert handle.is_running() is True
    stop_coro = handle.stop()
    assert asyncio.iscoroutine(stop_coro)
    await stop_coro
    assert handle.is_running() is False


async def test_start_reaper_property_not_idempotent(manager: AsyncTaskManager) -> None:
    """async_tasks.start_reaper.property.idempotent_false

    start_reaper is declared NOT idempotent and SHOULD guard against a second
    concurrent start: starting again while a reaper runs raises
    ModuleError(REAPER_ALREADY_RUNNING).
    """
    handle = manager.start_reaper(ttl_seconds=3600.0, sweep_interval_ms=300_000)
    with pytest.raises(ModuleError) as exc_info:
        manager.start_reaper(ttl_seconds=3600.0, sweep_interval_ms=300_000)
    assert exc_info.value.code == ErrorCodes.REAPER_ALREADY_RUNNING
    await handle.stop()


# ---------------------------------------------------------------------------
# Contract: TaskStore.save
# ---------------------------------------------------------------------------


async def test_taskstore_save_property_async() -> None:
    """async_tasks.save.property.async

    TaskStore.save is async in every SDK (D-17): InMemoryTaskStore.save is a
    coroutine that resolves to None.
    """
    store = InMemoryTaskStore()
    info = TaskInfo("t1", "test.echo", TaskStatus.PENDING, submitted_at=time.time())
    coro = store.save(info)
    assert asyncio.iscoroutine(coro)
    assert await coro is None
    assert await store.get("t1") is not None


async def test_taskstore_save_property_idempotent() -> None:
    """async_tasks.save.property.idempotent

    Saving twice with the same task_id overwrites — exactly one record remains.
    """
    store = InMemoryTaskStore()
    info = TaskInfo("t1", "test.echo", TaskStatus.PENDING, submitted_at=1.0)
    await store.save(info)
    info2 = TaskInfo("t1", "test.echo", TaskStatus.COMPLETED, submitted_at=1.0)
    await store.save(info2)
    all_tasks = await store.list()
    assert len(all_tasks) == 1
    stored = await store.get("t1")
    assert stored is not None and stored.status is TaskStatus.COMPLETED


async def test_taskstore_save_error_store_unavailable() -> None:
    """async_tasks.save.error.TASK_STORE_UNAVAILABLE

    Network-backed stores raise TaskStoreError(code=TASK_STORE_UNAVAILABLE)
    when the backend is unreachable. The type is defined and exported as of
    spec v1.50.0 (D-92); the bundled InMemoryTaskStore MUST NOT raise it.
    """
    from apcore.errors import TaskStoreError

    class _UnreachableStore:
        async def save(self, info):
            raise TaskStoreError(operation="save", reason="connection refused")

    store = _UnreachableStore()
    info = TaskInfo("t1", "test.echo", TaskStatus.PENDING, submitted_at=1.0)
    with pytest.raises(TaskStoreError) as excinfo:
        await store.save(info)
    assert excinfo.value.code == "TASK_STORE_UNAVAILABLE"

    # The bundled store cannot fail, so it never raises it.
    await InMemoryTaskStore().save(info)


# ---------------------------------------------------------------------------
# Contract: TaskStore.get
# ---------------------------------------------------------------------------


async def test_taskstore_get_property_async() -> None:
    """async_tasks.get.property.async

    TaskStore.get is async (D-17) and returns the stored record or None.
    """
    store = InMemoryTaskStore()
    info = TaskInfo("g1", "test.echo", TaskStatus.PENDING, submitted_at=time.time())
    await store.save(info)
    coro = store.get("g1")
    assert asyncio.iscoroutine(coro)
    fetched = await coro
    assert fetched is not None and fetched.task_id == "g1"


async def test_taskstore_get_property_idempotent() -> None:
    """async_tasks.get.property.idempotent

    Two gets of the same id return equal records and never mutate the store.
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("g1", "test.echo", TaskStatus.PENDING, submitted_at=1.0))
    a = await store.get("g1")
    b = await store.get("g1")
    assert a is not None and b is not None and a.task_id == b.task_id
    assert len(await store.list()) == 1


async def test_taskstore_get_unknown_returns_none() -> None:
    """async_tasks.get.return.unknown_none

    An unknown task_id returns None; in-memory store never raises.
    """
    store = InMemoryTaskStore()
    assert await store.get("absent") is None


# ---------------------------------------------------------------------------
# Contract: TaskStore.list
# ---------------------------------------------------------------------------


async def test_taskstore_list_property_async() -> None:
    """async_tasks.list.property.async

    TaskStore.list is async (D-17) and returns a list of records.
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("l1", "test.echo", TaskStatus.PENDING, submitted_at=1.0))
    coro = store.list()
    assert asyncio.iscoroutine(coro)
    items = await coro
    assert [i.task_id for i in items] == ["l1"]


async def test_taskstore_list_filter_by_status() -> None:
    """async_tasks.list.input.status.filter

    When a status is supplied, only matching records are returned.
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("l1", "m", TaskStatus.PENDING, submitted_at=1.0))
    await store.save(TaskInfo("l2", "m", TaskStatus.COMPLETED, submitted_at=1.0))
    done = await store.list(TaskStatus.COMPLETED)
    assert [i.task_id for i in done] == ["l2"]


async def test_taskstore_list_property_idempotent() -> None:
    """async_tasks.list.property.idempotent

    Repeated list calls over an unchanged store return the same ids.
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("l1", "m", TaskStatus.PENDING, submitted_at=1.0))
    await store.save(TaskInfo("l2", "m", TaskStatus.PENDING, submitted_at=1.0))
    a = {i.task_id for i in await store.list()}
    b = {i.task_id for i in await store.list()}
    assert a == b == {"l1", "l2"}


# ---------------------------------------------------------------------------
# Contract: TaskStore.delete
# ---------------------------------------------------------------------------


async def test_taskstore_delete_property_async() -> None:
    """async_tasks.delete.property.async

    TaskStore.delete is async (D-17) and resolves to None, removing the record.
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("d1", "m", TaskStatus.COMPLETED, submitted_at=1.0))
    coro = store.delete("d1")
    assert asyncio.iscoroutine(coro)
    assert await coro is None
    assert await store.get("d1") is None


async def test_taskstore_delete_property_idempotent() -> None:
    """async_tasks.delete.property.idempotent

    Deleting an already-absent task_id succeeds silently (no error, no change).
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("d1", "m", TaskStatus.COMPLETED, submitted_at=1.0))
    await store.delete("d1")
    await store.delete("d1")  # second delete is a silent no-op
    assert await store.get("d1") is None
    assert await store.list() == []


# ---------------------------------------------------------------------------
# Contract: TaskStore.list_expired
# ---------------------------------------------------------------------------


async def test_taskstore_list_expired_property_async() -> None:
    """async_tasks.list_expired.property.async

    TaskStore.list_expired is async (D-17) and returns a list.
    """
    store = InMemoryTaskStore()
    info = TaskInfo("e1", "m", TaskStatus.COMPLETED, submitted_at=1.0, completed_at=10.0)
    await store.save(info)
    coro = store.list_expired(before_timestamp=100.0)
    assert asyncio.iscoroutine(coro)
    expired = await coro
    assert [i.task_id for i in expired] == ["e1"]


async def test_taskstore_list_expired_eligible_terminal_only() -> None:
    """async_tasks.list_expired.eligible.terminal_only

    Only terminal tasks with completed_at < before_timestamp are returned;
    PENDING/RUNNING tasks (no completed_at) are never returned.
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("done", "m", TaskStatus.COMPLETED, submitted_at=1.0, completed_at=10.0))
    await store.save(TaskInfo("pending", "m", TaskStatus.PENDING, submitted_at=1.0))
    await store.save(TaskInfo("running", "m", TaskStatus.RUNNING, submitted_at=1.0, started_at=2.0))
    expired = await store.list_expired(before_timestamp=100.0)
    assert [i.task_id for i in expired] == ["done"]


async def test_taskstore_list_expired_boundary_strictly_before() -> None:
    """async_tasks.list_expired.input.before_timestamp.strict

    Expiry is strict (completed_at < before_timestamp): a task whose
    completed_at equals before_timestamp is NOT expired.
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("eq", "m", TaskStatus.COMPLETED, submitted_at=1.0, completed_at=50.0))
    assert await store.list_expired(before_timestamp=50.0) == []
    expired = await store.list_expired(before_timestamp=50.0001)
    assert [i.task_id for i in expired] == ["eq"]


async def test_taskstore_list_expired_property_idempotent() -> None:
    """async_tasks.list_expired.property.idempotent

    Repeated list_expired calls over an unchanged store return the same ids
    and never mutate state.
    """
    store = InMemoryTaskStore()
    await store.save(TaskInfo("e1", "m", TaskStatus.COMPLETED, submitted_at=1.0, completed_at=10.0))
    a = {i.task_id for i in await store.list_expired(before_timestamp=100.0)}
    b = {i.task_id for i in await store.list_expired(before_timestamp=100.0)}
    assert a == b == {"e1"}
    assert len(await store.list()) == 1
