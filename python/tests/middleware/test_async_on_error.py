"""Issue #42 — async on_error correctness for partial-wrapped/decorated handlers.

The async middleware execution path used to gate awaiting on
``inspect.iscoroutinefunction(mw.on_error)``.  That check returns False for
``functools.partial``-wrapped functions and any callable that is not literally
``async def`` even when invoking it returns a coroutine, which silently dropped
the recovery value.  The fix is to inspect the **return value** with
``inspect.isawaitable`` instead.
"""

from __future__ import annotations

import asyncio
import functools
from typing import Any

import pytest

from apcore.middleware.base import Middleware
from apcore.middleware.manager import MiddlewareManager


def _sync_wrap_returning_coro(async_fn):
    """Decorator that wraps an async function in a sync callable.

    ``inspect.iscoroutinefunction`` returns False for the resulting wrapper
    (the wrapper itself is sync) even though calling it produces a coroutine.
    This is the canonical shape that broke the old ``iscoroutinefunction``
    gate.
    """

    @functools.wraps(async_fn)
    def wrapper(*args: Any, **kwargs: Any) -> Any:
        return async_fn(*args, **kwargs)

    # functools.wraps copies __wrapped__ so iscoroutinefunction will detect.
    # Strip __wrapped__ to simulate hand-written decorators that don't.
    try:
        del wrapper.__wrapped__
    except AttributeError:
        pass
    return wrapper


class PartialAsyncOnErrorMiddleware(Middleware):
    """on_error is a sync wrapper that returns a coroutine.

    ``inspect.iscoroutinefunction`` returns False, so the old
    ``iscoroutinefunction`` gate would forward to ``asyncio.to_thread`` which
    silently produced an un-awaited coroutine as the recovery value.
    """

    def __init__(self) -> None:
        super().__init__()

        async def _handler(
            module_id: str,
            inputs: dict[str, Any],
            error: Exception,
            context: Any,
        ) -> dict[str, Any]:
            await asyncio.sleep(0)
            return {"recovered": True, "module_id": module_id}

        self.on_error = _sync_wrap_returning_coro(_handler)  # type: ignore[assignment]


class PartialAsyncBeforeMiddleware(Middleware):
    """before is a sync wrapper that returns a coroutine; should still be awaited."""

    def __init__(self) -> None:
        super().__init__()

        async def _handler(
            module_id: str,
            inputs: dict[str, Any],
            context: Any,
        ) -> dict[str, Any]:
            await asyncio.sleep(0)
            return {**inputs, "stamped_by": "before-tag"}

        self.before = _sync_wrap_returning_coro(_handler)  # type: ignore[assignment]


class PartialAsyncAfterMiddleware(Middleware):
    """after is a sync wrapper that returns a coroutine; should still be awaited."""

    def __init__(self) -> None:
        super().__init__()

        async def _handler(
            module_id: str,
            inputs: dict[str, Any],
            output: dict[str, Any],
            context: Any,
        ) -> dict[str, Any]:
            await asyncio.sleep(0)
            return {**output, "stamped_by": "after-tag"}

        self.after = _sync_wrap_returning_coro(_handler)  # type: ignore[assignment]


def _make_context() -> Any:
    class _Ctx:
        caller_id = "test"
        call_chain: list[str] = []
        data: dict[str, Any] = {}

    return _Ctx()


@pytest.mark.asyncio
async def test_partial_async_on_error_recovery_is_awaited() -> None:
    """A ``functools.partial`` wrapping an async on_error must be awaited."""
    manager = MiddlewareManager()
    mw = PartialAsyncOnErrorMiddleware()
    manager.add(mw)

    error = RuntimeError("boom")
    recovery = await manager.execute_on_error_async("test_module", {"x": 1}, error, _make_context(), [mw])

    assert recovery == {"recovered": True, "module_id": "test_module"}


@pytest.mark.asyncio
async def test_partial_async_before_is_awaited() -> None:
    """A ``functools.partial`` wrapping an async before must be awaited."""
    manager = MiddlewareManager()
    mw = PartialAsyncBeforeMiddleware()
    manager.add(mw)

    new_inputs, executed = await manager.execute_before_async("test_module", {"x": 1}, _make_context())
    assert mw in executed
    assert new_inputs == {"x": 1, "stamped_by": "before-tag"}


@pytest.mark.asyncio
async def test_partial_async_after_is_awaited() -> None:
    """A ``functools.partial`` wrapping an async after must be awaited."""
    manager = MiddlewareManager()
    mw = PartialAsyncAfterMiddleware()
    manager.add(mw)

    out = await manager.execute_after_async("test_module", {"x": 1}, {"out": "v"}, _make_context())
    assert out == {"out": "v", "stamped_by": "after-tag"}


@pytest.mark.asyncio
async def test_sync_on_error_still_works() -> None:
    """Plain sync on_error returning a dict still becomes recovery output."""

    class SyncRecovery(Middleware):
        def on_error(
            self,
            module_id: str,
            inputs: dict[str, Any],
            error: Exception,
            context: Any,
        ) -> dict[str, Any]:
            return {"recovered": "sync"}

    manager = MiddlewareManager()
    mw = SyncRecovery()
    manager.add(mw)

    recovery = await manager.execute_on_error_async(
        "test_module", {"x": 1}, RuntimeError("boom"), _make_context(), [mw]
    )
    assert recovery == {"recovered": "sync"}
