"""Cooperative cancellation support for apcore module execution."""

from __future__ import annotations

import asyncio
import threading
from collections.abc import Awaitable, Callable
from contextlib import suppress
from typing import TypeVar

from apcore.errors import ModuleError

R = TypeVar("R")


class ExecutionCancelledError(ModuleError):
    """Raised when a module execution is cancelled via CancelToken."""

    def __init__(self, message: str = "Execution was cancelled") -> None:
        super().__init__(code="EXECUTION_CANCELLED", message=message)


class CancelToken:
    """Cooperative cancellation token for module execution.

    Pass to Context and check periodically during long-running operations.
    A linked parent's signal is visible here; cancelling this token never
    cancels its parent. Cancellation cannot reverse already consumed resources.
    """

    def __init__(self, parent: CancelToken | None = None) -> None:
        self._cancelled: bool = False
        self._parent = parent
        self._lock = threading.Lock()
        self._listeners: set[Callable[[], None]] = set()

    @property
    def is_cancelled(self) -> bool:
        """Whether cancellation has been requested."""
        return self._cancelled or (self._parent is not None and self._parent.is_cancelled)

    def child(self) -> CancelToken:
        """Create a linked token; child cancellation never cancels its parent."""
        return CancelToken(parent=self)

    def cancel(self) -> None:
        """Request cancellation."""
        with self._lock:
            if self._cancelled:
                return
            self._cancelled = True
            listeners = tuple(self._listeners)
        for listener in listeners:
            listener()

    def _listen(self, listener: Callable[[], None]) -> Callable[[], None]:
        """Watch this token and its parents until the returned unsubscribe is called."""
        with self._lock:
            self._listeners.add(listener)
            cancelled = self._cancelled
        unsubscribe_parent = self._parent._listen(listener) if self._parent is not None else None
        if cancelled:
            listener()

        def unsubscribe() -> None:
            with self._lock:
                self._listeners.discard(listener)
            if unsubscribe_parent is not None:
                unsubscribe_parent()

        return unsubscribe

    def check(self) -> None:
        """Raise ExecutionCancelledError if cancelled.

        Call this periodically in long-running operations.
        """
        if self.is_cancelled:
            raise ExecutionCancelledError()

    def raise_if_cancelled(self) -> None:
        """Raise ExecutionCancelledError if cancelled.

        Canonical spec method name (`docs/features/cancellation.md`, "Contract:
        CancelToken.raise_if_cancelled"). Identical behavior to :meth:`check`,
        which is used throughout this SDK and elsewhere by external callers and
        is NOT deprecated by this method's addition — both names are supported.
        """
        self.check()

    def reset(self) -> None:
        """Reset the token for reuse."""
        with self._lock:
            self._cancelled = False


class _AwaitDeadlineExpired(TimeoutError):
    """The framework's deadline elapsed while an asynchronous callback ran."""


async def _await_with_cancellation(
    operation: Callable[[], Awaitable[R]], token: CancelToken | None, timeout: float | None
) -> R:
    """Bound an async operation independently of its cancellation cooperation."""
    if token is not None:
        token.check()
    if timeout is not None and timeout <= 0:
        raise _AwaitDeadlineExpired()
    loop = asyncio.get_running_loop()
    expires_at = loop.time() + timeout if timeout is not None else None
    cancelled = loop.create_future()

    def report_cancellation() -> None:
        if not cancelled.done():
            cancelled.set_result(None)

    def signal_cancellation() -> None:
        with suppress(RuntimeError):
            loop.call_soon_threadsafe(report_cancellation)

    async def run() -> R:
        if token is not None:
            token.check()
        if expires_at is not None and loop.time() >= expires_at:
            raise _AwaitDeadlineExpired()
        return await operation()

    unsubscribe = token._listen(signal_cancellation) if token is not None else None
    task = asyncio.create_task(run())
    try:
        await asyncio.wait((task, cancelled), timeout=timeout, return_when=asyncio.FIRST_COMPLETED)
        if token is not None:
            token.check()
        if cancelled.done():
            raise ExecutionCancelledError()
        if expires_at is not None and (not task.done() or loop.time() >= expires_at):
            raise _AwaitDeadlineExpired()
        return task.result()
    finally:
        if unsubscribe is not None:
            unsubscribe()
        cancelled.cancel()
        if not task.done():
            task.cancel()
            task.add_done_callback(_consume_task_result)
        else:
            _consume_task_result(task)


def _consume_task_result(task: asyncio.Future[R]) -> None:
    if not task.cancelled():
        task.exception()
