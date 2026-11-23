"""Drive `async_task_evolution.json` — pluggable TaskStore, retry, reaper (Issue #34).

apcore-typescript and apcore-rust drive this fixture; apcore-python had only
hand-transcribed coverage across `tests/test_async_task_*.py`, which cannot
notice a new case. This reads the canonical file.

Fixture arithmetic note (independently confirmed, NOT fixed here)
-----------------------------------------------------------------
`reaper_deletes_expired_tasks` sets `now_timestamp: 1700003000` with
`ttl_seconds: 3600`, giving a cutoff of 1699999400 — but "fresh-task-001"
completed at 1699999002, i.e. 3998 s before `now`. Both tasks are past the TTL,
so the declared `remaining_task_ids: ["fresh-task-001"]` is unreachable for any
correct implementation. apcore-typescript's driver reaches the same conclusion
and silently substitutes `stableNow = 1700000000`
(`tests/conformance.test.ts:2455-2460`).

This driver does both: it asserts the TTL-partition behaviour at a `now` that
actually separates the two tasks, **and** it drives the fixture's literal
`now_timestamp` under a strict xfail so the inconsistency stays visible instead
of living only in a comment. The fixture is canonical and is not edited here.
"""

from __future__ import annotations

import asyncio
import time
from typing import Any
from unittest.mock import patch


from apcore.async_task import (
    AsyncTaskManager,
    InMemoryTaskStore,
    RetryConfig,
    TaskInfo,
    TaskStatus,
)

from .canonical_fixtures import load_fixture

FIXTURE = load_fixture("async_task_evolution.json")
CASES: dict[str, dict[str, Any]] = {tc["id"]: tc for tc in FIXTURE["test_cases"]}

# `now` used for the TTL-partition assertion; see the module docstring for why
# the fixture's own now_timestamp cannot be used for it.
_SEPARATING_NOW = 1_700_000_000.0


class _StubExecutor:
    """Executor double: succeeds, or fails a bounded/unbounded number of times."""

    def __init__(self, *, failures: int = 0, result: Any = None) -> None:
        self.remaining_failures = failures
        self.calls = 0
        self._result = result if result is not None else {"ok": True}

    async def call_async(self, module_id: str, inputs: dict[str, Any], context: Any = None) -> Any:
        self.calls += 1
        if self.remaining_failures != 0:
            if self.remaining_failures > 0:
                self.remaining_failures -= 1
            raise RuntimeError("simulated failure")
        return self._result


def _task_info(raw: dict[str, Any]) -> TaskInfo:
    return TaskInfo(
        task_id=raw["task_id"],
        module_id=raw["module_id"],
        status=TaskStatus(raw["status"]),
        submitted_at=raw["submitted_at"],
        started_at=raw["started_at"],
        completed_at=raw["completed_at"],
        result=raw["result"],
        error=raw["error"],
        retry_count=raw["retry_count"],
        max_retries=raw["max_retries"],
    )


async def _seeded_store(raw_tasks: list[dict[str, Any]]) -> InMemoryTaskStore:
    store = InMemoryTaskStore()
    for raw in raw_tasks:
        await store.save(_task_info(raw))
    return store


def _retry_config(case: dict[str, Any]) -> RetryConfig:
    spec = case["retry_config"]
    return RetryConfig(
        max_retries=spec.get("max_retries", 0),
        retry_delay_ms=spec["retry_delay_ms"],
        backoff_multiplier=spec["backoff_multiplier"],
        max_retry_delay_ms=spec["max_retry_delay_ms"],
    )


# ---------------------------------------------------------------------------
# Store cases
# ---------------------------------------------------------------------------


async def test_in_memory_store_default() -> None:
    case = CASES["in_memory_store_default"]
    manager = AsyncTaskManager(executor=_StubExecutor())
    assert type(manager._store).__name__ == case["expected"]["store_type"], (
        f"[{case['id']}] with no store injected the manager must default to "
        f"{case['expected']['store_type']}, got {type(manager._store).__name__}"
    )


async def test_custom_store_injected() -> None:
    """A user TaskStore implementation is used verbatim, not wrapped or replaced."""
    case = CASES["custom_store_injected"]
    store_type = case["config"]["store"]["type"]

    # The fixture names a Redis-backed store; apcore ships no network backends
    # (they are explicitly out of tree), so the injected store is a stand-in
    # class carrying that name and satisfying the TaskStore protocol.
    custom_store_cls = type(store_type, (InMemoryTaskStore,), {})
    store = custom_store_cls()
    manager = AsyncTaskManager(executor=_StubExecutor(), store=store)

    assert manager._store is store, f"[{case['id']}] the injected store instance must be used as-is"
    assert (
        type(manager._store).__name__ == case["expected"]["store_type"]
    ), f"[{case['id']}] store_type mismatch: got {type(manager._store).__name__}"


async def test_task_store_save_and_get() -> None:
    case = CASES["task_store_save_and_get"]
    expected = case["expected"]
    store = await _seeded_store([case["task_info"]])

    found = await store.get(case["lookup_id"])
    assert (found is not None) is expected["found"], f"[{case['id']}] found mismatch"
    assert found is not None
    assert found.task_id == expected["task_id"], f"[{case['id']}] task_id mismatch"
    assert found.status.value == expected["status"], f"[{case['id']}] status mismatch"
    assert found.result == expected["result"], f"[{case['id']}] result mismatch"


async def test_task_store_list_by_status() -> None:
    case = CASES["task_store_list_by_status"]
    expected = case["expected"]
    store = await _seeded_store(case["stored_tasks"])

    listed = await store.list(TaskStatus(case["status_filter"]))
    ids = [info.task_id for info in listed]
    assert len(listed) == expected["count"], (
        f"[{case['id']}] list(status={case['status_filter']!r}) returned {len(listed)} tasks "
        f"({ids}), expected {expected['count']}"
    )
    # D-82: insertion order is normative, so compare the lists AS ORDERED.
    # This used to be `sorted(ids) == sorted(expected["task_ids"])`, which
    # asserts the order away by construction — the case could not have caught a
    # reordering no matter how many tasks it seeded, and apcore-rust was the only
    # driver of the three actually enforcing the decision.
    assert ids == expected["task_ids"], (
        f"[{case['id']}] task_ids mismatch: got {ids}, expected {expected['task_ids']} "
        f"(D-82 — insertion order, not lexicographic and not submitted_at)"
    )


# ---------------------------------------------------------------------------
# Retry cases
# ---------------------------------------------------------------------------


async def test_retry_scheduled_on_failure() -> None:
    case = CASES["retry_scheduled_on_failure"]
    expected = case["expected"]
    config = _retry_config(case)

    # One failure, then the task would succeed — but the assertion happens while
    # the manager is sleeping out the backoff, i.e. between failure and retry.
    executor = _StubExecutor(failures=1)
    manager = AsyncTaskManager(executor=executor)
    task_id = await manager.submit(case["task_info"]["module_id"], {}, retry_policy=config)

    deadline = time.monotonic() + 5.0
    info = manager.get_status(task_id)
    while (info is None or info.retry_count == 0) and time.monotonic() < deadline:
        await asyncio.sleep(0.005)
        info = manager.get_status(task_id)

    assert info is not None, f"[{case['id']}] the task record disappeared"
    assert info.retry_count == expected["retry_count_after_first_failure"], (
        f"[{case['id']}] retry_count after the first failure: got {info.retry_count}, "
        f"expected {expected['retry_count_after_first_failure']}"
    )
    assert info.status.value == expected["status_after_first_failure"], (
        f"[{case['id']}] a task awaiting retry must be {expected['status_after_first_failure']!r}, "
        f"not FAILED; got {info.status.value!r}"
    )
    assert (
        config.compute_delay_ms(0) == expected["next_retry_delay_ms"]
    ), f"[{case['id']}] the first retry delay must be {expected['next_retry_delay_ms']}ms"

    await manager.shutdown()


def test_backoff_multiplier_applied() -> None:
    case = CASES["backoff_multiplier_applied"]
    config = _retry_config(case)

    for key, expected_ms in case["expected"].items():
        attempt = int(key.split("_")[1])
        got = config.compute_delay_ms(attempt)
        assert got == expected_ms, (
            f"[{case['id']}] attempt {attempt}: got {got}ms, expected {expected_ms}ms — "
            f"delay is min(retry_delay_ms * multiplier**attempt, max_retry_delay_ms)"
        )


async def test_max_retries_exhausted_becomes_failed() -> None:
    case = CASES["max_retries_exhausted_becomes_failed"]
    expected = case["expected"]
    config = _retry_config(case)

    executor = _StubExecutor(failures=-1)  # always fails
    manager = AsyncTaskManager(executor=executor)
    task_id = await manager.submit(case["task_info"]["module_id"], {}, retry_policy=config)

    deadline = time.monotonic() + 5.0
    info = manager.get_status(task_id)
    while (info is None or info.status is not TaskStatus.FAILED) and time.monotonic() < deadline:
        await asyncio.sleep(0.005)
        info = manager.get_status(task_id)

    assert info is not None
    assert info.status.value == expected["final_status"], (
        f"[{case['id']}] after exhausting retries the status must be {expected['final_status']!r}, "
        f"got {info.status.value!r}"
    )
    assert (
        info.retry_count == expected["retry_count"]
    ), f"[{case['id']}] retry_count: got {info.retry_count}, expected {expected['retry_count']}"
    assert (info.error is not None and info.error != "") is expected[
        "error_populated"
    ], f"[{case['id']}] the error field must be populated on the terminal FAILED record"
    assert executor.calls == expected["retry_count"] + 1, (
        f"[{case['id']}] expected {expected['retry_count'] + 1} attempts "
        f"(initial + {expected['retry_count']} retries), got {executor.calls}"
    )

    await manager.shutdown()


# ---------------------------------------------------------------------------
# Reaper cases
# ---------------------------------------------------------------------------


async def test_reaper_disabled_by_default() -> None:
    case = CASES["reaper_disabled_by_default"]
    expected = case["expected"]
    store = await _seeded_store(case["stored_expired_tasks"])
    manager = AsyncTaskManager(executor=_StubExecutor(), store=store)

    running = manager._reaper_task is not None and not manager._reaper_task.done()
    assert running is expected["reaper_running"], f"[{case['id']}] no reaper may run until start_reaper() is called"

    await asyncio.sleep(0.02)
    task_id = case["stored_expired_tasks"][0]["task_id"]
    still_present = await store.get(task_id) is not None
    assert (
        still_present is expected["expired_task_still_present"]
    ), f"[{case['id']}] with no reaper running, the expired task must remain in the store"


async def _sweep(case: dict[str, Any], now: float) -> InMemoryTaskStore:
    """Run one reaper sweep against the case's stored tasks at wall-clock *now*."""
    store = await _seeded_store(case["stored_tasks"])
    manager = AsyncTaskManager(executor=_StubExecutor(), store=store)
    ttl_seconds = case["config"]["reaper"]["ttl_seconds"]
    with patch("apcore.async_task.time.time", return_value=now):
        await manager.cleanup(max_age_seconds=ttl_seconds)
    return store


async def _assert_sweep(case: dict[str, Any], store: InMemoryTaskStore) -> None:
    expected = case["expected"]
    for task_id in expected["deleted_task_ids"]:
        assert await store.get(task_id) is None, f"[{case['id']}] {task_id!r} is past the TTL and must be swept"
    for task_id in expected["remaining_task_ids"]:
        assert await store.get(task_id) is not None, f"[{case['id']}] {task_id!r} must survive the sweep"


async def test_reaper_deletes_expired_tasks() -> None:
    """TTL partitioning, asserted at a `now` that actually separates the tasks."""
    case = CASES["reaper_deletes_expired_tasks"]
    await _assert_sweep(case, await _sweep(case, _SEPARATING_NOW))


async def test_reaper_deletes_expired_tasks_at_fixture_timestamp() -> None:
    """The fixture's own `now_timestamp` must separate the tasks.

    This was a strict xfail while the fixture declared now_timestamp=1700003000,
    which put the cutoff at 1699999400 — 398s AFTER fresh-task-001 completed, so
    both tasks were expired and the declared remaining_task_ids was unreachable.
    The spec repo corrected it to 1700002000; the strict xfail turned into an
    XPASS the moment it did, which is how the marker announced its own removal.
    """
    case = CASES["reaper_deletes_expired_tasks"]
    await _assert_sweep(case, await _sweep(case, case["now_timestamp"]))


async def test_reaper_skips_running_tasks() -> None:
    case = CASES["reaper_skips_running_tasks"]
    # PENDING / RUNNING are non-terminal, so the fixture's own now_timestamp is
    # usable here: no amount of elapsed time may make them eligible.
    await _assert_sweep(case, await _sweep(case, case["now_timestamp"]))


async def test_reaper_wiring_uses_fixture_config() -> None:
    """start_reaper must adopt the fixture's ttl_seconds / sweep_interval_ms."""
    case = CASES["reaper_deletes_expired_tasks"]
    reaper = case["config"]["reaper"]
    manager = AsyncTaskManager(executor=_StubExecutor())

    handle = manager.start_reaper(
        ttl_seconds=reaper["ttl_seconds"],
        sweep_interval_ms=reaper["sweep_interval_ms"],
    )
    try:
        assert handle.is_running() is reaper["enabled"]
        assert manager._reaper_max_age == reaper["ttl_seconds"]
        assert manager._reaper_interval == reaper["sweep_interval_ms"] / 1000.0
    finally:
        await handle.stop()
    assert handle.is_running() is False, "stop() must drain the reaper loop"


def test_every_fixture_case_has_a_driver() -> None:
    driven = {
        "in_memory_store_default",
        "custom_store_injected",
        "task_store_save_and_get",
        "task_store_list_by_status",
        "retry_scheduled_on_failure",
        "backoff_multiplier_applied",
        "max_retries_exhausted_becomes_failed",
        "reaper_disabled_by_default",
        "reaper_deletes_expired_tasks",
        "reaper_skips_running_tasks",
    }
    assert set(CASES) == driven, (
        f"async_task_evolution.json cases without a driver: {sorted(set(CASES) - driven)}; "
        f"drivers with no matching case: {sorted(driven - set(CASES))}"
    )
