"""Async task manager for background module execution."""

from __future__ import annotations

import asyncio
import dataclasses
import inspect
import logging
import time
import uuid
import warnings
from dataclasses import dataclass
from enum import Enum
from typing import Any, Protocol, runtime_checkable

from apcore.context import Context
from apcore.errors import ErrorCodes, ModuleError, TaskLimitExceededError

__all__ = [
    "TaskStatus",
    "TaskInfo",
    "TaskStore",
    "InMemoryTaskStore",
    "RetryConfig",
    "RetryPolicy",
    "BackoffStrategy",
    "AsyncTaskManager",
    "ExecutorProtocol",
    "ReaperHandle",
]


_UNSET = object()


class ReaperHandle:
    """Handle returned by :meth:`AsyncTaskManager.start_reaper`.

    Mirrors the TS / Rust ``ReaperHandle`` surface (Issue #34 D-11).  Owners
    call :meth:`stop` to cancel the periodic cleanup loop; :meth:`is_running`
    reports whether the underlying ``asyncio.Task`` is still active.
    Idempotent: ``stop()`` after the loop has already finished is a no-op.

    ``stop()`` is **async** and awaits the reaper task to fully drain
    (D-11 / A-D-AT-03), matching the TypeScript and Rust SDKs. Callers
    that still invoke ``handle.stop()`` from synchronous code (the
    pre-D-11 surface) get a ``DeprecationWarning`` and the coroutine is
    driven to completion via a private adapter — see :meth:`_run_sync_stop`.
    """

    __slots__ = ("_manager",)

    def __init__(self, manager: "AsyncTaskManager") -> None:
        self._manager = manager

    async def stop(self) -> None:
        """Cancel the periodic reap loop and await its termination (idempotent)."""
        await self._manager.stop_reaper()

    def is_running(self) -> bool:
        """Return True if the reaper task exists and has not finished."""
        task = self._manager._reaper_task
        return task is not None and not task.done()


@runtime_checkable
class ExecutorProtocol(Protocol):
    """Minimal async-call surface required by :class:`AsyncTaskManager`.

    Provided as a ``Protocol`` so tests can inject a lightweight fake
    without constructing a full :class:`apcore.executor.Executor`. The
    concrete ``Executor`` satisfies this protocol via ``call_async``.
    """

    async def call_async(
        self,
        module_id: str,
        inputs: dict[str, Any] | None = None,
        context: Context | None = None,
        version_hint: str | None = None,
    ) -> dict[str, Any]: ...


_logger = logging.getLogger(__name__)


def _emit_retrying_deprecation() -> None:
    warnings.warn(
        "TaskStatus.RETRYING is deprecated and removed in cross-language alignment "
        "(D-12); during retry backoff the status is now TaskStatus.PENDING. "
        "Use TaskStatus.PENDING in new code.",
        DeprecationWarning,
        stacklevel=3,
    )


class _TaskStatusMeta(type(Enum)):  # type: ignore[misc]
    """Metaclass that intercepts ``TaskStatus.RETRYING`` access for one-version
    deprecation: returns ``TaskStatus.PENDING`` with a ``DeprecationWarning``.
    """

    def __getattr__(cls, name: str) -> Any:
        if name == "RETRYING":
            _emit_retrying_deprecation()
            return cls.PENDING  # type: ignore[attr-defined]
        raise AttributeError(name)


class TaskStatus(str, Enum, metaclass=_TaskStatusMeta):
    """Status of an async task.

    .. note::
        ``RETRYING`` was removed in the cross-language alignment (D-12).
        Tasks waiting between retry attempts are now reported as
        :attr:`PENDING` to match the TypeScript and Rust SDKs.  The legacy
        ``TaskStatus.RETRYING`` attribute remains accessible for one minor
        release as a deprecated alias for ``PENDING``.
    """

    PENDING = "pending"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


@dataclass
class TaskInfo:
    """Metadata and result tracking for a submitted async task.

    .. note::
        The ``attempt_number`` field was renamed to ``retry_count`` for
        cross-language alignment (D-13).  ``attempt_number`` is kept as a
        deprecated property accessor for one minor release.
    """

    task_id: str
    module_id: str
    status: TaskStatus
    submitted_at: float
    started_at: float | None = None
    completed_at: float | None = None
    result: Any = None
    error: str | None = None
    retry_count: int = 0
    max_retries: int = 0

    # ----- Deprecated aliases (D-13) -----

    @property
    def attempt_number(self) -> int:
        """Deprecated alias for :attr:`retry_count`.

        .. deprecated:: 0.21.0
            Use :attr:`retry_count` instead.  Will be removed in a future
            release.
        """
        warnings.warn(
            "TaskInfo.attempt_number is deprecated; use retry_count instead.",
            DeprecationWarning,
            stacklevel=2,
        )
        return self.retry_count

    @attempt_number.setter
    def attempt_number(self, value: int) -> None:
        warnings.warn(
            "TaskInfo.attempt_number is deprecated; use retry_count instead.",
            DeprecationWarning,
            stacklevel=2,
        )
        self.retry_count = value


@runtime_checkable
class TaskStore(Protocol):
    """Async storage backend for :class:`TaskInfo` records (D-17).

    The default implementation is :class:`InMemoryTaskStore`. Users may
    supply a custom store (e.g. Redis, SQL) via ``AsyncTaskManager(store=...)``.
    All methods are asynchronous in every SDK so that I/O-backed stores can
    plug in without blocking the runtime's event loop; ``InMemoryTaskStore``
    keeps the uniform async shape even though its operations are CPU-only.

    .. note::
        The canonical write method is :meth:`save`.  ``put`` is retained as
        a deprecated shim for one minor release (D-10).
    """

    async def get(self, task_id: str) -> TaskInfo | None: ...
    async def save(self, info: TaskInfo) -> None: ...
    async def delete(self, task_id: str) -> None: ...
    async def list(self, status: TaskStatus | None = None) -> list[TaskInfo]: ...
    async def list_expired(self, before_timestamp: float) -> list[TaskInfo]: ...


class InMemoryTaskStore:
    """Default in-memory :class:`TaskStore` backed by a plain dict.

    All methods are ``async`` to match the :class:`TaskStore` Protocol
    (D-17). Operations remain pure CPU work — no I/O — so the coroutine
    bodies return immediately without awaiting.
    """

    def __init__(self) -> None:
        self._data: dict[str, TaskInfo] = {}

    async def get(self, task_id: str) -> TaskInfo | None:
        return self._data.get(task_id)

    async def save(self, info: TaskInfo) -> None:
        self._data[info.task_id] = info

    async def put(self, info: TaskInfo) -> None:
        """Deprecated alias for :meth:`save` (D-10).

        .. deprecated:: 0.21.0
            Use :meth:`save`.  Retained for one minor release for
            backward compatibility.
        """
        warnings.warn(
            "TaskStore.put() is deprecated; use save() instead.",
            DeprecationWarning,
            stacklevel=2,
        )
        await self.save(info)

    async def delete(self, task_id: str) -> None:
        self._data.pop(task_id, None)

    async def list(self, status: TaskStatus | None = None) -> list[TaskInfo]:
        if status is None:
            return list(self._data.values())
        return [t for t in self._data.values() if t.status == status]

    async def list_expired(self, before_timestamp: float) -> list[TaskInfo]:
        """Return terminal-state tasks whose ``completed_at`` precedes
        ``before_timestamp`` (D-10).

        Non-terminal tasks (PENDING, RUNNING) are never returned —
        ``list_expired`` is intended to drive TTL-based cleanup of
        finished work, not to interrupt running tasks.
        """
        # Terminal set is duplicated locally to avoid a forward reference to
        # the module-level ``_TERMINAL_STATUSES`` constant defined below.
        terminal = {TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED}
        result: list[TaskInfo] = []
        for info in self._data.values():
            if info.status not in terminal:
                continue
            # A task without ``completed_at`` MUST NOT be returned — there is no
            # ``submitted_at`` fallback here (that fallback belongs to cleanup).
            # Matches the TaskStore.list_expired contract and the TS/Rust SDKs.
            if info.completed_at is not None and info.completed_at < before_timestamp:
                result.append(info)
        return result


class BackoffStrategy(str, Enum):
    """Backoff formula applied between retry attempts (legacy ``RetryPolicy``)."""

    FIXED = "fixed"
    LINEAR = "linear"
    EXPONENTIAL = "exponential"


@dataclass
class RetryConfig:
    """Retry configuration for a submitted task (canonical, sync A-002).

    Field names align with TypeScript / Rust / protocol spec:
    ``max_retries``, ``retry_delay_ms``, ``backoff_multiplier``,
    ``max_retry_delay_ms``. Delay is computed as
    ``min(retry_delay_ms * (backoff_multiplier ** attempt), max_retry_delay_ms)``
    where ``attempt`` is 0-indexed (the first retry uses attempt=0).

    The legacy :class:`RetryPolicy` (using ``backoff`` /
    ``base_delay_seconds``) remains supported as a deprecated alternative.
    """

    max_retries: int = 0
    retry_delay_ms: int = 1000
    backoff_multiplier: float = 2.0
    max_retry_delay_ms: int = 60000

    def compute_delay_ms(self, attempt: int) -> float:
        """Return the delay in milliseconds before retry ``attempt`` (0-indexed).

        Capped at :attr:`max_retry_delay_ms`.
        """
        return min(
            self.retry_delay_ms * (self.backoff_multiplier**attempt),
            self.max_retry_delay_ms,
        )

    def delay_for(self, attempt: int) -> float:
        """Return delay in **seconds** before retry ``attempt`` (1-indexed).

        Adapter that bridges the canonical (0-indexed, ms) API to the legacy
        :meth:`RetryPolicy.delay_for` (1-indexed, seconds) signature so
        ``AsyncTaskManager`` can treat both classes uniformly.
        """
        # Translate legacy 1-indexed attempt into canonical 0-indexed.
        zero_indexed = max(attempt - 1, 0)
        return self.compute_delay_ms(zero_indexed) / 1000.0


@dataclass
class RetryPolicy:
    """Legacy retry configuration for a submitted task.

    .. deprecated:: 0.21.0
        Use :class:`RetryConfig` for cross-language alignment with
        TypeScript / Rust / protocol spec field names. ``RetryPolicy`` is
        retained for backwards compatibility and will be removed in a
        future major release.

    Attributes:
        max_retries: Maximum number of retry attempts (default ``0`` —
            retries are strictly opt-in across all SDKs, D-14 / A-D-AT-09).
            Earlier versions defaulted to ``3``, which silently enabled
            retries when callers used ``RetryPolicy()`` without arguments.
        backoff: Delay growth strategy between attempts.
        base_delay_seconds: Base wait time fed into the backoff formula.
    """

    max_retries: int = 0
    backoff: BackoffStrategy = BackoffStrategy.EXPONENTIAL
    base_delay_seconds: float = 1.0

    def __post_init__(self) -> None:
        # D-14 / A-D-AT-09: surface a one-shot deprecation when the legacy
        # class is instantiated so callers migrate to RetryConfig.
        warnings.warn(
            "RetryPolicy is deprecated; use RetryConfig for cross-language "
            "alignment with the TypeScript and Rust SDKs. RetryPolicy will "
            "be removed in a future major release.",
            DeprecationWarning,
            stacklevel=3,
        )

    def delay_for(self, attempt: int) -> float:
        """Return wait seconds before retry *attempt* (1-indexed)."""
        if self.backoff == BackoffStrategy.FIXED:
            return self.base_delay_seconds
        if self.backoff == BackoffStrategy.LINEAR:
            return self.base_delay_seconds * attempt
        # EXPONENTIAL
        return self.base_delay_seconds * (2 ** (attempt - 1))


_ACTIVE_STATUSES = frozenset({TaskStatus.PENDING, TaskStatus.RUNNING})
_TERMINAL_STATUSES = frozenset({TaskStatus.COMPLETED, TaskStatus.FAILED, TaskStatus.CANCELLED})


def _snapshot(info: TaskInfo | None) -> TaskInfo | None:
    """Return a shallow copy of ``info`` so callers cannot mutate live state.

    Per D-clarification on ``AsyncTaskManager.get_status`` / ``list_tasks``
    (A-D-AT-06): the manager MUST hand out copies, matching the TypeScript
    SDK's ``{ ...info }`` and the Rust SDK's ``info.clone()``. Without this
    shim, Python callers could mutate the live dataclass and silently
    corrupt the store's view of task state.
    """
    if info is None:
        return None
    return dataclasses.replace(info)


def _completed_result(task_id: str, info: TaskInfo | None) -> Any:
    """Return ``info.result``, or raise as :meth:`AsyncTaskManager.get_result` documents."""
    if info is None:
        raise KeyError(f"Task not found: {task_id}")
    if info.status != TaskStatus.COMPLETED:
        raise RuntimeError(f"Task {task_id} is not completed (status={info.status.value})")
    return info.result


class AsyncTaskManager:
    """Manages background execution of modules via asyncio tasks.

    Limits concurrency with a semaphore and tracks task lifecycle.
    Accepts a pluggable :class:`TaskStore` for custom persistence.
    """

    def __init__(
        self,
        executor: ExecutorProtocol,
        max_concurrent: int = 10,
        max_tasks: int = 1000,
        store: TaskStore | None = None,
    ) -> None:
        self._executor = executor
        self._max_tasks = max_tasks
        self._store: TaskStore = store if store is not None else InMemoryTaskStore()
        self._semaphore = asyncio.Semaphore(max_concurrent)
        # Admission guard (mirrors Rust's admission_lock): serializes the
        # max_tasks capacity check together with the PENDING save so that two
        # concurrent submits cannot both pass the check at capacity-1 and then
        # both persist, overshooting the cap (submit TOCTOU).
        self._admission_lock = asyncio.Lock()
        self._async_tasks: dict[str, asyncio.Task[Any]] = {}
        self._reaper_task: asyncio.Task[Any] | None = None
        self._reaper_interval: float = 3600.0
        self._reaper_max_age: float = 3600.0

    async def _save(self, info: TaskInfo) -> None:
        """Persist ``info`` via the configured store.

        Prefers the canonical async :meth:`TaskStore.save` method (D-17).
        Falls back to the deprecated :meth:`put` for legacy custom stores
        that have not yet been updated to the post-D-10 API. Awaits both
        coroutine and sync return values so pre-D-17 sync stores remain
        usable for one deprecation window.
        """
        save = getattr(self._store, "save", None)
        if callable(save):
            result = save(info)
        else:  # pragma: no cover - legacy custom stores
            result = self._store.put(info)  # type: ignore[attr-defined]
        if inspect.isawaitable(result):
            await result

    async def _store_get(self, task_id: str) -> TaskInfo | None:
        """Await-or-return adapter for ``self._store.get`` (D-17 migration shim)."""
        result = self._store.get(task_id)
        if inspect.isawaitable(result):
            return await result
        return result

    async def _store_list(self, status: TaskStatus | None = None) -> list[TaskInfo]:
        """Await-or-return adapter for ``self._store.list`` (D-17 migration shim)."""
        result = self._store.list(status)
        if inspect.isawaitable(result):
            return await result
        return result

    async def _store_delete(self, task_id: str) -> None:
        """Await-or-return adapter for ``self._store.delete`` (D-17 migration shim)."""
        result = self._store.delete(task_id)
        if inspect.isawaitable(result):
            await result

    async def _store_list_expired(self, before_timestamp: float) -> list[TaskInfo]:
        """Await-or-return adapter for ``self._store.list_expired`` (D-17 migration shim)."""
        result = self._store.list_expired(before_timestamp)
        if inspect.isawaitable(result):
            return await result
        return result

    async def submit(
        self,
        module_id: str,
        inputs: dict[str, Any],
        context: Context | None = None,
        retry_policy: "RetryPolicy | RetryConfig | None" = None,
    ) -> str:
        """Submit a module for background execution.

        Creates a TaskInfo in PENDING state, spawns an asyncio.Task that
        acquires the concurrency semaphore before calling executor.call_async().

        Args:
            module_id: The module to execute.
            inputs: Input data for the module.
            context: Optional execution context.
            retry_policy: Optional retry configuration — either the canonical
                :class:`RetryConfig` or the legacy :class:`RetryPolicy`.
                None means no retries.

        Returns:
            The generated task_id (UUID4 string).
        """
        task_id = str(uuid.uuid4())
        # Atomically check capacity and persist the PENDING record. Without
        # the lock, the await inside _store_list / _save lets a second submit
        # interleave between the check and the save, so both can pass at
        # capacity-1 and exceed max_tasks (mirrors Rust's admission_lock).
        async with self._admission_lock:
            all_tasks = await self._store_list()
            active = sum(1 for info in all_tasks if info.status in _ACTIVE_STATUSES)
            if active >= self._max_tasks:
                raise TaskLimitExceededError(max_tasks=self._max_tasks)

            info = TaskInfo(
                task_id=task_id,
                module_id=module_id,
                status=TaskStatus.PENDING,
                submitted_at=time.time(),
                max_retries=retry_policy.max_retries if retry_policy is not None else 0,
            )
            await self._save(info)

        async_task = asyncio.create_task(self._run(task_id, module_id, inputs, context, retry_policy))
        async_task.add_done_callback(lambda _: self._async_tasks.pop(task_id, None))
        self._async_tasks[task_id] = async_task

        return task_id

    def get_status(self, task_id: str) -> TaskInfo | None:
        """Return a snapshot of the TaskInfo for a task, or None if not found.

        The returned object is a shallow copy (A-D-AT-06): callers MUST
        treat it as a read-only snapshot. Mutating the copy never affects
        the live store. The store's ``get`` is read synchronously for the
        in-memory default; async-backed stores SHOULD await
        :meth:`get_status_async` instead.
        """
        # For the default in-memory store the coroutine completes
        # synchronously on the first send; we drive it directly to keep
        # the legacy sync surface working. Custom async-only stores
        # should use ``get_status_async``.
        get = self._store.get(task_id)
        if inspect.isawaitable(get):
            try:
                # Pump the coroutine without an event loop. This works for
                # the bundled InMemoryTaskStore (whose body is pure CPU)
                # and for any custom store whose ``get`` returns without
                # actually awaiting external I/O. Stores that perform
                # real awaits MUST call ``get_status_async`` instead.
                try:
                    get.send(None)  # type: ignore[union-attr]
                except StopIteration as stop:  # noqa: PERF203 — coroutine return value
                    return _snapshot(stop.value)
                raise RuntimeError("TaskStore.get() suspended; call get_status_async() for I/O-backed stores")
            finally:
                get.close()  # type: ignore[union-attr]
        return _snapshot(get)

    async def get_status_async(self, task_id: str) -> TaskInfo | None:
        """Async variant of :meth:`get_status` for I/O-backed stores (D-17)."""
        return _snapshot(await self._store_get(task_id))

    def get_result(self, task_id: str) -> Any:
        """Return the result of a completed task.

        Raises:
            KeyError: If the task_id is not found.
            RuntimeError: If the task is not in COMPLETED status.
        """
        return _completed_result(task_id, self.get_status(task_id))

    async def get_result_async(self, task_id: str) -> Any:
        """Async variant of :meth:`get_result` for I/O-backed stores (D-17).

        Raises:
            KeyError: If the task_id is not found.
            RuntimeError: If the task is not in COMPLETED status.
        """
        return _completed_result(task_id, await self.get_status_async(task_id))

    async def cancel(self, task_id: str) -> bool:
        """Cancel a running, pending, or retrying task.

        ``False`` means exactly what the spec's cancel contract says it means:
        the task did not exist, or it had already reached a terminal state. A
        missing local ``asyncio.Task`` handle is **not** a third false case —
        with a persistent/shared store an active record left behind by another
        process has no in-process coroutine to interrupt, yet it must still be
        cancellable (otherwise it consumes the ``max_tasks`` active budget
        forever). In that case the interrupt step is skipped and the
        active -> CANCELLED store write still happens. Mirrors the Rust SDK,
        where a missing ``JoinHandle`` is not an early return.

        Returns:
            True if the task was successfully cancelled, False otherwise.
        """
        info = await self._store_get(task_id)
        if info is None:
            return False
        if info.status not in _ACTIVE_STATUSES:
            return False

        async_task = self._async_tasks.get(task_id)
        if async_task is not None:
            async_task.cancel()
            try:
                await async_task
            except asyncio.CancelledError:
                pass
            except Exception as exc:
                _logger.warning(
                    "Task %s raised while being cancelled: %s",
                    task_id,
                    exc,
                    exc_info=True,
                )

        # Re-read after the task settles in case the runner already
        # transitioned to a terminal state inside its cancel-handler.
        info = await self._store_get(task_id)
        if info is not None and info.status in _ACTIVE_STATUSES:
            info.status = TaskStatus.CANCELLED
            info.completed_at = time.time()
            await self._save(info)

        return True

    async def shutdown(self) -> None:
        """Cancel all active tasks and stop the reaper.

        Iterates the **store** (not the local live-handle dict) so that a
        store-resident active task with no local asyncio handle is still
        cancelled — required by the spec shutdown postcondition (no active
        task survives) and matching the TypeScript/Rust SDKs. Every active
        task goes through :meth:`cancel`, which itself handles the
        no-local-handle case by skipping the interrupt and still performing
        the active -> CANCELLED store write.

        Every active task is attempted even after one cancellation fails, and
        the first failure is re-raised once they all have been (D-122). The two
        failure modes are not symmetric: if the store is unreachable every
        cancellation fails and stopping early reaches the same outcome sooner,
        but if ONE task fails, stopping leaves every remaining task uncancelled
        when they could have been cancelled. An uncancelled task in a shared
        store is a lasting cost — it holds a ``max_tasks`` slot for every
        manager sharing that store, and outlives the process that could have
        cancelled it — while a slower shutdown is transient. This method is
        already an unbounded wait by contract and takes no timeout, so a caller
        needing a bound already imposes one.

        Raises:
            The first store failure encountered, re-raised after every active
            task has been attempted.
        """
        await self.stop_reaper()
        first_error: BaseException | None = None
        for info in await self._store_list():
            if info.status not in _ACTIVE_STATUSES:
                continue
            try:
                await self.cancel(info.task_id)
            except Exception as exc:  # noqa: BLE001 — re-raised after the loop
                if first_error is None:
                    first_error = exc
        if first_error is not None:
            raise first_error

    def list_tasks(self, status: TaskStatus | None = None) -> list[TaskInfo]:
        """Return snapshots of all tasks, optionally filtered by status.

        Each entry is a shallow copy of the live :class:`TaskInfo` — see
        :meth:`get_status` for the mutation-safety contract (A-D-AT-06).
        I/O-backed stores SHOULD use :meth:`list_tasks_async` instead.
        """
        result = self._store.list(status)
        if inspect.isawaitable(result):
            try:
                try:
                    result.send(None)  # type: ignore[union-attr]
                except StopIteration as stop:  # noqa: PERF203
                    items: list[TaskInfo] = list(stop.value or [])
                    return [info for info in (_snapshot(it) for it in items) if info is not None]
                raise RuntimeError("TaskStore.list() suspended; call list_tasks_async() for I/O-backed stores")
            finally:
                result.close()  # type: ignore[union-attr]
        return [info for info in (_snapshot(it) for it in result) if info is not None]

    async def list_tasks_async(self, status: TaskStatus | None = None) -> list[TaskInfo]:
        """Async variant of :meth:`list_tasks` for I/O-backed stores (D-17)."""
        items = await self._store_list(status)
        return [info for info in (_snapshot(it) for it in items) if info is not None]

    async def cleanup(self, max_age_seconds: float = 3600.0) -> int:
        """Remove terminal-state tasks older than max_age_seconds.

        Terminal states: COMPLETED, FAILED, CANCELLED.

        Returns:
            The number of tasks removed.
        """
        now = time.time()
        to_remove: list[str] = []

        for info in await self._store_list():
            if info.status not in _TERMINAL_STATUSES:
                continue
            ref_time = info.completed_at if info.completed_at is not None else info.submitted_at
            if now - ref_time >= max_age_seconds:
                to_remove.append(info.task_id)

        for task_id in to_remove:
            await self._store_delete(task_id)
            self._async_tasks.pop(task_id, None)

        return len(to_remove)

    def start_reaper(
        self,
        *,
        ttl_seconds: float | int = 3600.0,
        sweep_interval_ms: float | int = 300_000,
        interval_seconds: Any = _UNSET,
        max_age_seconds: Any = _UNSET,
    ) -> ReaperHandle:
        """Start a background asyncio task that periodically calls cleanup().

        Cross-language signature alignment (Issue #34 D-11): TS and Rust
        SDKs expose ``ttl_seconds`` (seconds) and ``sweep_interval_ms``
        (milliseconds) and return a :class:`ReaperHandle`. The default
        sweep cadence is **300_000 ms (5 minutes)**, matching TS and Rust.

        Args:
            ttl_seconds: Terminal tasks older than this are removed
                (seconds). Default 3600.0 (1 hour).
            sweep_interval_ms: How often to run cleanup (milliseconds).
                Default 300_000 (5 minutes).
            interval_seconds: **Deprecated** alias for ``sweep_interval_ms``
                expressed in seconds; emits ``DeprecationWarning``.
            max_age_seconds: **Deprecated** alias for ``ttl_seconds``; emits
                ``DeprecationWarning``.

        Returns:
            A :class:`ReaperHandle` whose :meth:`~ReaperHandle.stop` cancels
            the loop.

        Raises:
            ModuleError: With code ``REAPER_ALREADY_RUNNING`` if the reaper
                is already running.
            TypeError: If both legacy and new kwargs are supplied for the
                same value.
        """
        # Resolve legacy aliases with explicit conflict checks.
        if interval_seconds is not _UNSET:
            if sweep_interval_ms != 300_000:
                raise TypeError(
                    "start_reaper() received both 'sweep_interval_ms' and deprecated 'interval_seconds'; pass only one."
                )
            warnings.warn(
                "AsyncTaskManager.start_reaper(interval_seconds=...) is deprecated; "
                "use sweep_interval_ms (milliseconds) for cross-language alignment.",
                DeprecationWarning,
                stacklevel=2,
            )
            sweep_interval_ms = float(interval_seconds) * 1000.0

        if max_age_seconds is not _UNSET:
            if ttl_seconds != 3600.0:
                raise TypeError(
                    "start_reaper() received both 'ttl_seconds' and deprecated 'max_age_seconds'; pass only one."
                )
            warnings.warn(
                "AsyncTaskManager.start_reaper(max_age_seconds=...) is deprecated; "
                "use ttl_seconds for cross-language alignment.",
                DeprecationWarning,
                stacklevel=2,
            )
            ttl_seconds = float(max_age_seconds)

        if self._reaper_task is not None and not self._reaper_task.done():
            raise ModuleError(
                ErrorCodes.REAPER_ALREADY_RUNNING,
                "Reaper is already running; call stop_reaper() first",
            )

        self._reaper_interval = float(sweep_interval_ms) / 1000.0
        self._reaper_max_age = float(ttl_seconds)
        self._reaper_task = asyncio.create_task(self._reap_loop())
        return ReaperHandle(self)

    async def stop_reaper(self) -> None:
        """Stop the background reaper task and await its termination.

        Per D-11 / A-D-AT-03 the stop surface is async and MUST drain the
        underlying coroutine, mirroring the TypeScript and Rust SDKs. The
        method is idempotent and is a no-op when no reaper is active.
        """
        task = self._reaper_task
        self._reaper_task = None
        if task is None or task.done():
            return
        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            # Expected — the loop awaits asyncio.sleep which raises here.
            pass
        except Exception as exc:  # pragma: no cover - defensive
            _logger.warning("Reaper task raised during shutdown: %s", exc, exc_info=True)

    async def _reap_loop(self) -> None:
        """Periodic expiry sweep executed by the reaper task.

        Drives the sweep through :meth:`TaskStore.list_expired` rather than
        :meth:`cleanup`, matching the TypeScript and Rust SDKs and
        async-tasks.md § "Reaper (TTL-Based Cleanup)". Two reasons this matters: a custom store whose
        ``list_expired`` is the only efficient expiry query (Redis
        ``ZRANGEBYSCORE``, SQL ``WHERE``) is otherwise bypassed and the entire
        task set is pulled into the process on every sweep; and the two
        predicates differ — ``cleanup()`` falls back to ``submitted_at`` when
        ``completed_at`` is None, which ``list_expired`` deliberately does not.

        A failing store must not kill the loop: the sweep is best-effort and
        the next tick retries.
        """
        try:
            while True:
                await asyncio.sleep(self._reaper_interval)
                try:
                    if callable(getattr(self._store, "list_expired", None)):
                        expired = await self._store_list_expired(time.time() - self._reaper_max_age)
                        for info in expired:
                            await self._store_delete(info.task_id)
                            self._async_tasks.pop(info.task_id, None)
                    else:  # pragma: no cover - legacy custom stores
                        # Custom store predating TaskStore.list_expired (D-17):
                        # fall back to the full-scan cleanup predicate.
                        await self.cleanup(self._reaper_max_age)
                except asyncio.CancelledError:
                    raise
                except Exception as exc:
                    _logger.warning("Reaper sweep failed: %s", exc, exc_info=True)
        except asyncio.CancelledError:
            pass

    async def _store_reached_terminal(self, task_id: str) -> bool:
        """Report whether the *stored* record for ``task_id`` is already terminal.

        Ported from the Rust SDK's ``save_terminal_if_not_cancelled``: the
        runner consults this immediately before every status write. With a
        shared or persistent :class:`TaskStore` another process (or a
        concurrent :meth:`cancel`) may have finished the task while this
        attempt was in flight, and protocol-spec.md §5.8 forbids transitions
        out of a terminal state. Writing the runner's stale snapshot back
        would resurrect a CANCELLED record as FAILED — or, on the retry path,
        as PENDING.

        The check is deliberately performed *before* the in-memory mutation,
        not just before ``save``: :class:`InMemoryTaskStore` hands out the
        live :class:`TaskInfo`, so mutating first would corrupt the stored
        record even when the write is abandoned.

        The check + mutate + ``save`` sequence is not atomic at the store
        level, so a perfectly-timed concurrent write can still slip into the
        gap; closing it entirely would require compare-and-swap support in the
        :class:`TaskStore` Protocol. The remaining window is small and
        non-corrupting (one extra terminal write that the next cancel no-ops
        against).
        """
        current = await self._store_get(task_id)
        return current is not None and current.status in _TERMINAL_STATUSES

    async def _run(
        self,
        task_id: str,
        module_id: str,
        inputs: dict[str, Any],
        context: Context | None,
        retry_policy: "RetryPolicy | RetryConfig | None",
    ) -> None:
        """Internal coroutine: execute a module with optional retry/backoff."""
        max_retries = retry_policy.max_retries if retry_policy is not None else 0

        while True:
            # Re-fetch at the top of every attempt rather than reusing the
            # snapshot captured before the loop: with a shared store the
            # record may have been cancelled or deleted during the backoff
            # sleep, and a stale object would write that change away.
            info = await self._store_get(task_id)
            if info is None:
                return
            if info.status in _TERMINAL_STATUSES:
                return

            try:
                async with self._semaphore:
                    if await self._store_reached_terminal(task_id):
                        return
                    info.status = TaskStatus.RUNNING
                    if info.started_at is None:
                        info.started_at = time.time()
                    await self._save(info)

                    result = await self._executor.call_async(module_id, inputs, context)

                    if await self._store_reached_terminal(task_id):
                        return
                    info.status = TaskStatus.COMPLETED
                    info.completed_at = time.time()
                    info.result = result
                    await self._save(info)
                    return

            except asyncio.CancelledError:
                if not await self._store_reached_terminal(task_id):
                    info.status = TaskStatus.CANCELLED
                    info.completed_at = time.time()
                    await self._save(info)
                _logger.info("Task %s cancelled", task_id)
                return

            except Exception as exc:
                if retry_policy is not None and info.retry_count < max_retries:
                    if await self._store_reached_terminal(task_id):
                        return
                    info.retry_count += 1
                    delay = retry_policy.delay_for(info.retry_count)
                    # D-12: backoff state is PENDING (was RETRYING) — matches
                    # TypeScript and Rust SDKs.
                    info.status = TaskStatus.PENDING
                    await self._save(info)
                    _logger.info(
                        "Task %s failed (attempt %d/%d), retrying in %.3fs",
                        task_id,
                        info.retry_count,
                        max_retries,
                        delay,
                    )
                    try:
                        await asyncio.sleep(delay)
                    except asyncio.CancelledError:
                        if not await self._store_reached_terminal(task_id):
                            info.status = TaskStatus.CANCELLED
                            info.completed_at = time.time()
                            await self._save(info)
                        _logger.info("Task %s cancelled during backoff", task_id)
                        return
                else:
                    if await self._store_reached_terminal(task_id):
                        return
                    info.status = TaskStatus.FAILED
                    info.completed_at = time.time()
                    info.error = str(exc)
                    await self._save(info)
                    _logger.error("Task %s failed: %s", task_id, exc, exc_info=True)
                    return
