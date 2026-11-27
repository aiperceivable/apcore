"""Own the event loops and workers used by the Executor's synchronous entry points."""

from __future__ import annotations

import asyncio
import logging
import threading
import time
import weakref
from collections.abc import Coroutine
from concurrent.futures import Future, ThreadPoolExecutor
from typing import Any

from apcore.errors import ErrorCodes, ModuleError, ModuleTimeoutError

_NESTED_WORKERS = 4
_worker_thread = threading.local()
_logger = logging.getLogger("apcore.executor")


class _ThreadLoop:
    def __init__(self) -> None:
        self.loop = asyncio.new_event_loop()
        self.active = False
        self.retired = False
        self.close = weakref.finalize(self, self.loop.close)


class _SyncBridge:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._local = threading.local()
        self._loops: weakref.WeakSet[_ThreadLoop] = weakref.WeakSet()
        self._pool: ThreadPoolExecutor | None = None
        self._slots = threading.BoundedSemaphore(_NESTED_WORKERS)

    def close(self) -> None:
        with self._lock:
            loops = list(self._loops)
            self._loops.clear()
            for owner in loops:
                owner.retired = True
                if not owner.active:
                    owner.close()
            pool, self._pool = self._pool, None
        if pool is not None:
            pool.shutdown(wait=False)

    def run(self, coro: Coroutine[Any, Any, Any], module_id: str, wait: float | None) -> Any:
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            pass
        else:
            return self._run_on_worker(coro, module_id, wait)

        try:
            with self._lock:
                owner: _ThreadLoop | None = getattr(self._local, "owner", None)
                if owner is None or owner.retired:
                    owner = _ThreadLoop()
                    self._local.owner = owner
                    self._loops.add(owner)
                owner.active = True
        except BaseException:
            coro.close()
            raise

        try:
            return owner.loop.run_until_complete(coro)
        finally:
            with self._lock:
                owner.active = False
                if owner.retired:
                    owner.close()

    def _run_on_worker(self, coro: Coroutine[Any, Any, Any], module_id: str, wait: float | None) -> Any:
        deadline = None if wait is None else time.monotonic() + wait
        reentrant = getattr(_worker_thread, "active", False)
        # A worker waiting for another slot could deadlock the pool. It can
        # make a nested sync call while there is spare capacity.
        admitted = self._slots.acquire(blocking=False) if reentrant else self._slots.acquire(timeout=wait)
        if not admitted:
            coro.close()
            if reentrant:
                raise ModuleError(
                    code=ErrorCodes.MODULE_EXECUTE_ERROR,
                    message="Sync bridge is busy. Await the asynchronous Executor method from a bridge worker.",
                    details={"module_id": module_id},
                )
            assert wait is not None
            raise _bridge_timeout(module_id, wait)

        result: Future[Any] = Future()
        expired = threading.Event()
        worker: tuple[asyncio.AbstractEventLoop, asyncio.Task[Any]] | None = None

        def run() -> None:
            nonlocal worker
            _worker_thread.active = True
            try:
                if expired.is_set() or (deadline is not None and time.monotonic() >= deadline):
                    coro.close()
                    return
                loop = asyncio.new_event_loop()
                try:
                    task = loop.create_task(coro)
                    worker = loop, task
                    if expired.is_set():
                        task.cancel()
                    result.set_result(loop.run_until_complete(task))
                except BaseException as exc:
                    result.set_exception(exc)
                finally:
                    # Publish before draining so D-133 timeout/cancellation
                    # never waits for cooperative cleanup on the caller.
                    pending = asyncio.all_tasks(loop)
                    try:
                        if pending:
                            loop.run_until_complete(asyncio.gather(*pending, return_exceptions=True))
                    finally:
                        loop.close()
            except BaseException as exc:
                if not result.done():
                    coro.close()
                    result.set_exception(exc)
                else:
                    _logger.warning("Sync bridge cleanup failed", exc_info=True)
            finally:
                _worker_thread.active = False
                self._slots.release()

        try:
            with self._lock:
                if self._pool is None:
                    self._pool = ThreadPoolExecutor(max_workers=_NESTED_WORKERS, thread_name_prefix="apcore-sync")
                self._pool.submit(run)
        except BaseException:
            self._slots.release()
            coro.close()
            raise

        remaining = None if deadline is None else max(0.0, deadline - time.monotonic())
        try:
            return result.result(remaining)
        except TimeoutError:
            if result.done() or wait is None:
                return result.result()

        expired.set()
        if worker is not None:
            loop, task = worker
            try:
                loop.call_soon_threadsafe(task.cancel)
            except RuntimeError:
                # The worker finished between the wait and the cancellation.
                pass
        raise _bridge_timeout(module_id, wait)


def _bridge_timeout(module_id: str, wait: float) -> ModuleTimeoutError:
    _logger.warning("Sync bridge wait expired for module %s", module_id)
    return ModuleTimeoutError(module_id=module_id, timeout_ms=round(wait * 1000), retryable=False)
