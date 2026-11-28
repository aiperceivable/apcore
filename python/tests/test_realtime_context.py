"""Opt-in realtime interaction through regular executor contexts."""

import asyncio
import time
from typing import Any

import pytest

from apcore import APCore, CancelToken, Context, ElicitationResult
from apcore.cancel import ExecutionCancelledError
from apcore.errors import ModuleTimeoutError


async def test_callbacks_are_inherited_and_excluded_from_wire_context() -> None:
    client = APCore()
    base: Context[Any] = Context.create()
    observed = []
    schema = {"type": "object"}

    async def progress(current: float, total: float | None, message: str | None) -> None:
        observed.append((current, total, message))

    async def elicit(message: str, request: dict[str, Any] | None) -> ElicitationResult:
        assert (message, request) == ("Confirm", schema)
        return ElicitationResult("accept", {"confirmed": True})

    @client.module("realtime.confirm")
    async def confirm(ctx: Context[Any]) -> bool:
        await ctx.report_progress(1, 2, "Preparing")
        response = await ctx.elicit("Confirm", schema)
        return bool(response and response.content and response.content["confirmed"])

    context = base.with_progress_reporter(progress).with_elicitation_handler(elicit)
    assert context is not base
    assert context.serialize() == base.serialize()
    assert await base.elicit("Optional") is None
    assert await Context.deserialize(context.serialize()).elicit("Optional") is None
    assert await client.call_async("realtime.confirm", {}, context) == {"result": True}
    assert observed == [(1, 2, "Preparing")]
    assert await context.with_elicitation_handler(None).elicit("Optional") is None
    await context.with_progress_reporter(None).report_progress(9)
    assert observed == [(1, 2, "Preparing")]


async def test_realtime_wait_stops_on_parent_cancellation_and_expired_deadline() -> None:
    token = CancelToken()
    started = asyncio.Event()

    async def progress(current: float, total: float | None, message: str | None) -> None:
        started.set()
        await asyncio.Future()

    ctx: Context[Any] = Context.create(cancel_token=token.child())
    pending = asyncio.create_task(ctx.with_progress_reporter(progress).report_progress(1))
    await started.wait()
    token.cancel()
    with pytest.raises(ExecutionCancelledError):
        await pending

    started.clear()
    expired: Context[Any] = Context.create(global_deadline=time.time() - 1)
    expired = expired.with_progress_reporter(progress).child("realtime.expired")
    with pytest.raises(ModuleTimeoutError) as error:
        await expired.report_progress(1)
    assert error.value.module_id == "realtime.expired"
    assert not started.is_set()
