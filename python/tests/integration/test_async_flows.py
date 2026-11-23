"""Integration tests for async module execution flows."""

from __future__ import annotations

import pytest

from apcore.context import Context


class TestAsyncFlows:
    """Async execution integration tests."""

    @pytest.mark.asyncio
    async def test_call_async_with_async_module(self, int_executor):
        result = await int_executor.call_async("async_greet", {"name": "Alice"})
        assert result == {"message": "Hello, Alice!"}

    def test_call_sync_with_async_module(self, int_executor):
        result = int_executor.call("async_greet", {"name": "Bob"})
        assert result == {"message": "Hello, Bob!"}

    @pytest.mark.asyncio
    async def test_call_async_with_sync_module(self, int_executor):
        result = await int_executor.call_async("greet", {"name": "Charlie"})
        assert result == {"message": "Hello, Charlie!"}

    @pytest.mark.asyncio
    async def test_context_propagation_in_async(self, int_executor):
        ctx = Context.create()
        result = await int_executor.call_async("async_greet", {"name": "Delta"}, context=ctx)
        assert result == {"message": "Hello, Delta!"}
        assert ctx.trace_id is not None
        assert len(ctx.trace_id) > 0
