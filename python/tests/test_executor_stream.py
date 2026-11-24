"""Tests for Executor.stream() async generator."""

from __future__ import annotations

import asyncio
import time
from typing import Any, AsyncIterator

import pytest
from pydantic import BaseModel

from apcore.client import APCore
from apcore.cancel import CancelToken
from apcore.config import Config
from apcore.context import Context
from apcore.errors import InvalidInputError, ModuleNotFoundError, ModuleTimeoutError
from apcore.events.emitter import ApCoreEvent
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.module import ModuleAnnotations
from apcore.registry import Registry


class SimpleInput(BaseModel):
    name: str


class SimpleOutput(BaseModel):
    greeting: str


class CountInput(BaseModel):
    count: int


class CountOutput(BaseModel):
    value: int = 0


class MockModule:
    def __init__(self) -> None:
        self.input_schema = SimpleInput
        self.output_schema = SimpleOutput

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"greeting": f"Hello, {inputs['name']}!"}


class StreamingModule:
    def __init__(self) -> None:
        self.input_schema = CountInput
        self.output_schema = CountOutput

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"value": inputs["count"]}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        for i in range(1, inputs["count"] + 1):
            yield {"value": i}


class DisjointKeyModule:
    """Module that yields chunks with different keys."""

    def __init__(self) -> None:
        self.input_schema = None
        self.output_schema = None

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"a": "val_a", "b": "val_b"}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        yield {"a": "val_a"}
        yield {"b": "val_b"}


def _make_executor(
    module: Any = None,
    module_id: str = "test.module",
    middlewares: list[Middleware] | None = None,
) -> Executor:
    reg = Registry()
    if module is not None:
        reg.register(module_id, module)
    return Executor(registry=reg, middlewares=middlewares)


class TestExecutorStream:
    async def test_deadline_interrupts_an_unfinished_chunk_and_cancels_only_the_call(self) -> None:
        class WaitingModule(StreamingModule):
            token: CancelToken | None = None

            async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
                self.token = context.cancel_token
                await asyncio.Event().wait()
                yield {"value": 1}

        module = WaitingModule()
        executor = _make_executor(module)
        parent = CancelToken()
        context = Context.create(cancel_token=parent, global_deadline=time.time() + 0.03)
        async with asyncio.timeout(1):
            with pytest.raises(ModuleTimeoutError):
                await anext(executor.stream("test.module", {"count": 1}, context=context))
        assert module.token is not None and module.token.is_cancelled
        assert not parent.is_cancelled

    async def test_after_uses_the_middleware_that_started_the_stream(self) -> None:
        recorded: list[str] = []

        class RecordingMiddleware(Middleware):
            def before(self, module_id, inputs, context):
                recorded.append("before")

            def after(self, module_id, inputs, output, context):
                recorded.append("after")

        middleware = RecordingMiddleware()
        executor = _make_executor(StreamingModule(), middlewares=[middleware])
        async for _ in executor.stream("test.module", {"count": 1}):
            assert executor.remove(middleware)
        assert recorded == ["before", "after"]

    async def test_closing_client_stream_closes_the_module_source(self) -> None:
        closed = asyncio.Event()

        class CleanupModule(StreamingModule):
            async def stream(self, inputs, context):
                try:
                    yield {"value": 1}
                    yield {"value": 2}
                finally:
                    closed.set()

        client = APCore()
        client.register("test.cleanup", CleanupModule())
        output = client.stream("test.cleanup", {"count": 2})
        assert await anext(output) == {"value": 1}
        await output.aclose()
        assert closed.is_set()

    async def test_fallback_single_chunk_when_stream_is_not_callable(self) -> None:
        module = MockModule()
        module.stream = 1
        executor = _make_executor(module)

        assert [chunk async for chunk in executor.stream("test.module", {"name": "Ada"})] == [
            {"greeting": "Hello, Ada!"}
        ]

    async def test_nested_merge_preserves_previously_yielded_chunks(self) -> None:
        class NestedModule(DisjointKeyModule):
            async def stream(self, inputs, context):
                yield {"nested": {"first": 1}}
                yield {"nested": {"second": 2}}

        executor = _make_executor(NestedModule())
        chunks = [chunk async for chunk in executor.stream("test.module")]

        assert chunks == [{"nested": {"first": 1}}, {"nested": {"second": 2}}]

    async def test_fallback_single_chunk_when_no_stream(self) -> None:
        mod = MockModule()
        ex = _make_executor(module=mod)

        chunks: list[dict[str, Any]] = []
        async for chunk in ex.stream("test.module", {"name": "Alice"}):
            chunks.append(chunk)

        assert len(chunks) == 1
        assert chunks[0] == {"greeting": "Hello, Alice!"}

    async def test_yields_multiple_chunks(self) -> None:
        mod = StreamingModule()
        ex = _make_executor(module=mod, module_id="counter")

        chunks: list[dict[str, Any]] = []
        async for chunk in ex.stream("counter", {"count": 3}):
            chunks.append(chunk)

        assert len(chunks) == 3
        assert chunks[0] == {"value": 1}
        assert chunks[1] == {"value": 2}
        assert chunks[2] == {"value": 3}

    async def test_module_not_found(self) -> None:
        ex = _make_executor()

        with pytest.raises(ModuleNotFoundError):
            async for _ in ex.stream("nonexistent"):
                pass

    async def test_before_and_after_middleware(self) -> None:
        mod = StreamingModule()
        log: list[str] = []

        class TrackingMiddleware(Middleware):
            def before(
                self,
                module_id: str,
                inputs: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                log.append("before")
                return None

            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                log.append("after")
                return None

        ex = _make_executor(module=mod, module_id="counter", middlewares=[TrackingMiddleware()])

        chunks: list[dict[str, Any]] = []
        async for chunk in ex.stream("counter", {"count": 2}):
            chunks.append(chunk)

        assert len(chunks) == 2
        assert log == ["before", "after"]

    async def test_accumulates_disjoint_keys(self) -> None:
        mod = DisjointKeyModule()

        after_output: dict[str, Any] = {}

        class CaptureAfter(Middleware):
            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                nonlocal after_output
                after_output = dict(output)
                return None

        ex = _make_executor(module=mod, module_id="disjoint", middlewares=[CaptureAfter()])

        chunks: list[dict[str, Any]] = []
        async for chunk in ex.stream("disjoint", {}):
            chunks.append(chunk)

        assert len(chunks) == 2
        assert chunks[0] == {"a": "val_a"}
        assert chunks[1] == {"b": "val_b"}
        # After-middleware receives the merged result
        assert after_output == {"a": "val_a", "b": "val_b"}

    async def test_deep_merge_preserves_nested_keys(self) -> None:
        """Chunks with overlapping nested dicts should merge recursively."""

        class NestedStreamModule:
            def __init__(self) -> None:
                self.input_schema = None
                self.output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {"user": {"name": "Alice", "age": 30}}

            async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
                yield {"user": {"name": "Alice", "email": "alice@example.com"}}
                yield {"user": {"age": 30}}

        after_output: dict[str, Any] = {}

        class CaptureAfter(Middleware):
            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                nonlocal after_output
                after_output = output
                return None

        ex = _make_executor(module=NestedStreamModule(), module_id="nested", middlewares=[CaptureAfter()])

        chunks: list[dict[str, Any]] = []
        async for chunk in ex.stream("nested", {}):
            chunks.append(chunk)

        assert len(chunks) == 2
        # Deep merge: name + email from chunk 1, age from chunk 2 — all preserved
        assert after_output == {"user": {"name": "Alice", "email": "alice@example.com", "age": 30}}


class SchemaFailStreamingModule:
    """Module whose accumulated stream output fails output_schema validation."""

    def __init__(self) -> None:
        self.input_schema = None

        class _OutSchema(BaseModel):
            value: int  # stream yields str, forcing validation failure

        self.output_schema = _OutSchema

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        yield {"value": "not-an-int-a"}
        yield {"value": "not-an-int-b"}


class TestStreamPostValidationFailure:
    """Post-stream validation failures surface through the emitter + log."""

    @pytest.mark.asyncio
    async def test_post_stream_failure_emits_event(self) -> None:
        config = Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}})
        client = APCore(config=config)
        client.register("failstream", SchemaFailStreamingModule())
        received: list[ApCoreEvent] = []
        client.on("apcore.stream.post_validation_failed", received.append)

        chunks: list[dict[str, Any]] = []
        async for chunk in client.stream("failstream", {}):
            chunks.append(chunk)

        assert len(chunks) == 2  # All chunks still yielded

        assert client.events is not None
        client.events.flush()
        assert len(received) == 1
        event = received[0]
        assert event.event_type == "apcore.stream.post_validation_failed"
        assert event.module_id == "failstream"
        assert event.severity == "error"
        assert "error_type" in event.data


class BadChunkStreamingModule:
    """Yields a valid object chunk followed by a single non-object chunk.

    The non-object chunk is supplied at construction time so a single class can
    exercise strings, arrays, numbers, etc. Per the cross-SDK contract (Rust
    ``deep_merge_chunks_checked``), the executor MUST reject the first
    non-object chunk BEFORE yielding it.
    """

    def __init__(self, bad_chunk: Any) -> None:
        self.input_schema = None
        self.output_schema = None
        self._bad_chunk = bad_chunk

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        yield {"a": 1}
        yield self._bad_chunk  # type: ignore[misc]  # deliberately invalid


class TestStreamChunkShapeValidation:
    """Stream chunks MUST be JSON objects; the first non-object chunk is
    rejected with InvalidInputError(STREAM_CHUNK_NOT_OBJECT) before delivery.

    Mirrors apcore-rust ``deep_merge_chunks_checked`` +
    ``tests/test_chunk_shape.rs`` for cross-SDK parity (audit D10-001).
    """

    @pytest.mark.asyncio
    async def test_rejects_string_chunk_before_delivery(self) -> None:
        ex = _make_executor(module=BadChunkStreamingModule("not an object"), module_id="badstream")

        delivered: list[Any] = []
        with pytest.raises(InvalidInputError) as exc_info:
            async for chunk in ex.stream("badstream", {}):
                delivered.append(chunk)

        # Hard requirement: the invalid chunk is never delivered to the consumer.
        assert "not an object" not in delivered
        assert delivered == [{"a": 1}]

        err = exc_info.value
        assert err.code == "GENERAL_INVALID_INPUT"
        assert err.details["code"] == "STREAM_CHUNK_NOT_OBJECT"
        assert err.details["actual_type"] == "string"
        assert err.details["chunk_index"] == 1

    @pytest.mark.asyncio
    async def test_rejects_array_chunk_before_delivery(self) -> None:
        ex = _make_executor(module=BadChunkStreamingModule([1, 2, 3]), module_id="badstream")

        delivered: list[Any] = []
        with pytest.raises(InvalidInputError) as exc_info:
            async for chunk in ex.stream("badstream", {}):
                delivered.append(chunk)

        assert [1, 2, 3] not in delivered
        assert delivered == [{"a": 1}]

        err = exc_info.value
        assert err.code == "GENERAL_INVALID_INPUT"
        assert err.details["code"] == "STREAM_CHUNK_NOT_OBJECT"
        assert err.details["actual_type"] == "array"
        assert err.details["chunk_index"] == 1

    @pytest.mark.asyncio
    async def test_accepts_all_object_stream(self) -> None:
        mod = DisjointKeyModule()

        after_output: dict[str, Any] = {}

        class CaptureAfter(Middleware):
            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                nonlocal after_output
                after_output = dict(output)
                return None

        ex = _make_executor(module=mod, module_id="disjoint", middlewares=[CaptureAfter()])

        delivered: list[dict[str, Any]] = []
        async for chunk in ex.stream("disjoint", {}):
            delivered.append(chunk)

        assert delivered == [{"a": "val_a"}, {"b": "val_b"}]
        # Accumulated result is the deep merge of both object chunks.
        assert after_output == {"a": "val_a", "b": "val_b"}


class SlowStreamingModule:
    """Emits chunks with a sleep between each, enough to cross a short deadline."""

    def __init__(self, idempotent: bool) -> None:
        self.input_schema = None
        self.output_schema = None
        self.annotations = ModuleAnnotations(idempotent=idempotent)

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        for i in range(10):
            yield {"chunk": i}
            await asyncio.sleep(0.05)


class TestStreamGlobalDeadline:
    """Executor.stream() must raise ModuleTimeoutError when global_deadline elapses mid-iteration."""

    @pytest.mark.asyncio
    @pytest.mark.parametrize("idempotent", [False, True])
    async def test_stream_raises_on_deadline_elapsed(self, idempotent: bool) -> None:
        ex = _make_executor(module=SlowStreamingModule(idempotent), module_id="slow")
        ctx = Context.create()
        ctx.global_deadline = time.time() + 0.1  # 100ms budget (D-99: epoch seconds)

        collected: list[dict[str, Any]] = []
        with pytest.raises(ModuleTimeoutError) as exc:
            async for chunk in ex.stream("slow", {}, context=ctx):
                collected.append(chunk)
        # At least one chunk produced before deadline hit.
        assert len(collected) >= 1
        assert len(collected) < 10  # Did not run to completion.
        assert exc.value.retryable is idempotent
