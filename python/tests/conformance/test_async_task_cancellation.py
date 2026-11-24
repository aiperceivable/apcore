"""Conformance tests for AsyncTaskManager capacity/cancellation (A-D-003/004) fixture.

Locks two cross-language invariants:
- A-D-003: submitting beyond ``max_tasks`` MUST raise the typed
  ``TASK_LIMIT_EXCEEDED`` error (catchable by type, not a bare RuntimeError).
- A-D-004: cancelling a task while it is in retry backoff MUST stop further
  retry attempts and end in CANCELLED — expressed as a deterministic invariant
  on attempt count + final status, NOT a latency/timing assertion.
"""

from __future__ import annotations

import asyncio
from typing import Any

import pytest

from apcore.async_task import AsyncTaskManager, RetryConfig, TaskStatus
from apcore.errors import TaskLimitExceededError
from conformance.canonical_fixtures import (
    dispatch_or_fail,
    load_fixture,
    reject_unknown_expectations,
)


FIXTURE_NAME = "async_task_cancellation.json"


def _load_fixture() -> dict:
    """Load the canonical fixture from the apcore spec repo."""
    return load_fixture(FIXTURE_NAME)


_FIXTURE = _load_fixture()


def _case(case_id: str) -> dict:
    return next(c for c in _FIXTURE["test_cases"] if c["id"] == case_id)


class _BlockingExecutor:
    """Executor stub whose call_async blocks until released — occupies the slot."""

    def __init__(self) -> None:
        self._release = asyncio.Event()

    def release(self) -> None:
        self._release.set()

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Any | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        await self._release.wait()
        return {"ok": True}


class _AlwaysFailingExecutor:
    """Executor stub that always raises and records every invocation."""

    def __init__(self) -> None:
        self.attempts = 0

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Any | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]:
        self.attempts += 1
        raise RuntimeError(f"always fails (attempt {self.attempts})")


@pytest.mark.asyncio
async def test_submit_over_capacity_raises_task_limit_exceeded() -> None:
    case = _case("submit_over_capacity_raises_task_limit_exceeded")
    reject_unknown_expectations(FIXTURE_NAME, case, {"expected_error"})

    executor = _BlockingExecutor()
    manager = AsyncTaskManager(
        executor,
        max_concurrent=case["max_concurrent"],
        max_tasks=case["max_tasks"],
    )

    try:
        # First task occupies the only slot and stays active (blocked).
        await manager.submit("test.blocking", {})
        # Let the runner start so the task is RUNNING (active) before 2nd submit.
        await asyncio.sleep(0)

        with pytest.raises(TaskLimitExceededError) as exc_info:
            await manager.submit("test.blocking", {})

        assert exc_info.value.code == case["expected_error"]
    finally:
        executor.release()
        await manager.shutdown()


#: fixture ``expected_final_status`` (a wire-level status STRING) -> the SDK
#: enum member carrying it. A dispatch with no ``else`` would let an
#: unrecognised status skip the assertion, so ``dispatch_or_fail`` hard-fails
#: instead (apcore#93).
_TASK_STATUS_BY_WIRE_NAME: dict[str, TaskStatus] = {s.value: s for s in TaskStatus}


@pytest.mark.asyncio
async def test_cancel_during_backoff_stops_further_retries() -> None:
    case = _case("cancel_during_backoff_stops_further_retries")
    # apcore#93: this driver used to hardcode ``TaskStatus.CANCELLED`` and
    # "the counter did not move", reading neither ``expected_final_status``
    # nor ``expected_no_attempt_after_cancel``. Both declared values now reach
    # an assertion, so mutating either in the fixture turns this test red.
    reject_unknown_expectations(FIXTURE_NAME, case, {"expected_final_status", "expected_no_attempt_after_cancel"})
    expected_status = dispatch_or_fail(
        FIXTURE_NAME,
        case["id"],
        case["expected_final_status"],
        _TASK_STATUS_BY_WIRE_NAME,
        "final task status",
    )

    executor = _AlwaysFailingExecutor()
    manager = AsyncTaskManager(executor, max_concurrent=1, max_tasks=10)

    retry = RetryConfig(
        max_retries=case["max_retries"],
        retry_delay_ms=case["retry_delay_ms"],
        backoff_multiplier=case["backoff_multiplier"],
    )

    try:
        task_id = await manager.submit("test.failing", {}, retry_policy=retry)

        # Wait for the first failure to occur and the task to enter backoff
        # (status flips to PENDING during the backoff sleep). The first
        # retry_delay is retry_delay_ms (>= 1000ms) so there is a wide window.
        for _ in range(200):
            await asyncio.sleep(0.005)
            info = manager.get_status(task_id)
            if executor.attempts >= 1 and info is not None and info.status == TaskStatus.PENDING:
                break

        assert executor.attempts == 1, "expected exactly one attempt before cancel landed in backoff"
        attempts_at_cancel = executor.attempts

        # Cancel while the task is sleeping out its backoff window.
        cancelled = await manager.cancel(task_id)
        assert cancelled is True

        final = manager.get_status(task_id)
        assert final is not None
        assert final.status == expected_status, (
            f"[{FIXTURE_NAME} :: {case['id']}] fixture declares final status "
            f"{case['expected_final_status']!r}, got {final.status.value!r}"
        )

        # No further retry attempt may have been started after the cancel.
        # ``expected_no_attempt_after_cancel`` is a boolean the fixture states
        # explicitly, so it drives the comparison rather than decorating it: a
        # fixture flipping it to false would demand the opposite invariant.
        no_attempt_after_cancel = executor.attempts == attempts_at_cancel
        assert no_attempt_after_cancel is case["expected_no_attempt_after_cancel"], (
            f"[{FIXTURE_NAME} :: {case['id']}] fixture declares "
            f"expected_no_attempt_after_cancel="
            f"{case['expected_no_attempt_after_cancel']}, but the attempt counter went "
            f"{attempts_at_cancel} -> {executor.attempts}"
        )
    finally:
        await manager.shutdown()
