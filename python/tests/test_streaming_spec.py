"""Spec-traced contract tests for the apcore streaming feature.

Source spec: apcore/docs/features/streaming.md — "## Contract: Module.stream".

This Python suite is the CANONICAL clause source. TS and Rust suites mirror
these clause-ids verbatim, so the ids must stay clean and stable.

Clause-id format: ``streaming.<method>.<kind>.<detail>`` where
``kind in {input, error, property, side_effect, return}``.

The streaming contract is defined on the module's ``stream()`` method but is
*enforced* by the executor's three-phase pipeline (``Executor.stream()``),
which drives chunk emission, deep-merge accumulation, and chunk-shape
validation. These tests therefore exercise the contract through
``Executor.stream()`` — the real surface that implements the normative rules.

Conventions copied from tests/test_executor_stream.py.
"""

from __future__ import annotations

import asyncio
import inspect
from typing import Any, AsyncIterator

import pytest
from pydantic import BaseModel

from apcore.context import Context
from apcore.errors import (
    InvalidInputError,
    ModuleError,
    SchemaValidationError,
)
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.registry import Registry

# --------------------------------------------------------------------------- #
# Fixtures / helpers (mirrors tests/test_executor_stream.py)
# --------------------------------------------------------------------------- #


class CountInput(BaseModel):
    count: int


class CountOutput(BaseModel):
    value: int = 0


class StreamingCounter:
    """Minimal streaming module: yields {"value": i} for i in 1..count."""

    def __init__(self) -> None:
        self.input_schema = CountInput
        self.output_schema = CountOutput

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"value": inputs["count"]}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        for i in range(1, inputs["count"] + 1):
            yield {"value": i}


class PlainModule:
    """Non-streaming module — only execute()."""

    def __init__(self) -> None:
        self.input_schema = None
        self.output_schema = None

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"result": "done"}


class NestedMergeModule:
    """Yields overlapping nested dicts to exercise the deep-merge accumulator."""

    def __init__(self) -> None:
        self.input_schema = None
        self.output_schema = None

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        yield {"content": "Hello", "metadata": {"tokens": 1}}
        yield {"content": " world", "metadata": {"tokens": 1, "model": "gpt-4"}}


class BadChunkModule:
    """Yields one valid object chunk then a single non-object chunk."""

    def __init__(self, bad_chunk: Any) -> None:
        self.input_schema = None
        self.output_schema = None
        self._bad_chunk = bad_chunk

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        yield {"a": 1}
        yield self._bad_chunk  # type: ignore[misc]  # deliberately invalid


def _make_executor(
    module: Any = None,
    module_id: str = "test.module",
    middlewares: list[Middleware] | None = None,
) -> Executor:
    reg = Registry()
    if module is not None:
        reg.register(module_id, module)
    return Executor(registry=reg, middlewares=middlewares)


async def _collect(stream: AsyncIterator[dict[str, Any]]) -> list[dict[str, Any]]:
    return [chunk async for chunk in stream]


# --------------------------------------------------------------------------- #
# INPUT
# --------------------------------------------------------------------------- #


class TestStreamInputs:
    async def test_stream_input_inputs_validated_against_schema(self) -> None:
        """streaming.stream.input.inputs_validated_against_schema

        Contract.Inputs: `inputs` (dict, required) — validated against the
        module's input_schema. A schema-conformant input streams normally.
        """
        ex = _make_executor(module=StreamingCounter(), module_id="counter")
        chunks = await _collect(ex.stream("counter", {"count": 2}))
        assert chunks == [{"value": 1}, {"value": 2}]

    async def test_stream_input_context_accepted(self) -> None:
        """streaming.stream.input.context_accepted

        Contract.Inputs: `context` (Context, required) — execution context.
        An explicitly-supplied Context is accepted and the stream runs under it.
        """
        ex = _make_executor(module=StreamingCounter(), module_id="counter")
        ctx = Context.create()
        chunks = await _collect(ex.stream("counter", {"count": 1}, context=ctx))
        assert chunks == [{"value": 1}]


# --------------------------------------------------------------------------- #
# ERROR
# --------------------------------------------------------------------------- #


class TestStreamErrors:
    async def test_stream_error_schema_validation_on_bad_inputs(self) -> None:
        """streaming.stream.error.schema_validation_on_bad_inputs

        Contract.Errors: SchemaValidationError(code=SCHEMA_VALIDATION_ERROR)
        when `inputs` fail validation against input_schema.
        """
        ex = _make_executor(module=StreamingCounter(), module_id="counter")
        with pytest.raises(SchemaValidationError) as exc_info:
            # count must be int; a string fails CountInput validation.
            await _collect(ex.stream("counter", {"count": "not-an-int"}))
        assert exc_info.value.code == "SCHEMA_VALIDATION_ERROR"

    async def test_stream_error_mid_stream_error_surfaced_as_final_item(self) -> None:
        """streaming.stream.error.mid_stream_error_surfaced

        Contract.Errors: an error raised mid-stream is surfaced (the iterator
        terminates after the error) — chunks emitted before the error are
        delivered, and the error propagates to the consumer.
        """

        class MidFailModule:
            def __init__(self) -> None:
                self.input_schema = None
                self.output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {}

            async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
                yield {"ok": 1}
                raise RuntimeError("boom mid-stream")

        ex = _make_executor(module=MidFailModule(), module_id="midfail")
        delivered: list[dict[str, Any]] = []
        # The executor wraps the module's mid-stream exception into a
        # ModuleError before surfacing it to the consumer; either way it
        # propagates and the iterator terminates.
        with pytest.raises(ModuleError):
            async for chunk in ex.stream("midfail", {}):
                delivered.append(chunk)
        # Chunk emitted before the error was delivered; iterator then terminated.
        assert delivered == [{"ok": 1}]


# --------------------------------------------------------------------------- #
# RETURN  (D-58 chunk-shape rule + lazy async iterator)
# --------------------------------------------------------------------------- #


class TestStreamReturn:
    async def test_stream_return_async_iterator_of_objects(self) -> None:
        """streaming.stream.return.async_iterator_of_objects

        Contract.Returns: on success, an AsyncIterator[dict] — a lazy sequence
        of partial output objects. Every yielded chunk is a dict.
        """
        ex = _make_executor(module=StreamingCounter(), module_id="counter")
        stream = ex.stream("counter", {"count": 3})
        assert inspect.isasyncgen(stream) or hasattr(stream, "__anext__")
        chunks = await _collect(stream)
        assert chunks == [{"value": 1}, {"value": 2}, {"value": 3}]
        assert all(isinstance(c, dict) for c in chunks)

    async def test_stream_return_d58_rejects_string_chunk_before_delivery(self) -> None:
        """streaming.stream.return.d58_reject_non_object_string

        Contract.Returns D-58: every chunk MUST be an object. A non-object
        chunk MUST be rejected with InvalidInputError(GENERAL_INVALID_INPUT,
        details.code=STREAM_CHUNK_NOT_OBJECT) and MUST NOT be yielded.
        details.chunk_index and details.actual_type ('string') are set.
        """
        ex = _make_executor(module=BadChunkModule("nope"), module_id="bad")
        delivered: list[Any] = []
        with pytest.raises(InvalidInputError) as exc_info:
            async for chunk in ex.stream("bad", {}):
                delivered.append(chunk)
        assert delivered == [{"a": 1}]  # invalid chunk never delivered
        err = exc_info.value
        assert err.code == "GENERAL_INVALID_INPUT"
        assert err.details["code"] == "STREAM_CHUNK_NOT_OBJECT"
        assert err.details["actual_type"] == "string"
        assert err.details["chunk_index"] == 1

    async def test_stream_return_d58_rejects_array_chunk_before_delivery(self) -> None:
        """streaming.stream.return.d58_reject_non_object_array

        Contract.Returns D-58: an array chunk is non-object and MUST be
        rejected (actual_type='array') without being delivered.
        """
        ex = _make_executor(module=BadChunkModule([1, 2]), module_id="bad")
        delivered: list[Any] = []
        with pytest.raises(InvalidInputError) as exc_info:
            async for chunk in ex.stream("bad", {}):
                delivered.append(chunk)
        assert delivered == [{"a": 1}]
        assert exc_info.value.details["actual_type"] == "array"

    @pytest.mark.parametrize(
        "bad_chunk,expected_type",
        [(3, "number"), (True, "bool"), (None, "null")],
    )
    async def test_stream_return_d58_rejects_scalar_chunks(self, bad_chunk: Any, expected_type: str) -> None:
        """streaming.stream.return.d58_reject_non_object_scalars

        Contract.Returns D-58: number, bool, and null chunks are all
        non-object and MUST be rejected with the correct JSON type name in
        details.actual_type, without being delivered.
        """
        ex = _make_executor(module=BadChunkModule(bad_chunk), module_id="bad")
        delivered: list[Any] = []
        with pytest.raises(InvalidInputError) as exc_info:
            async for chunk in ex.stream("bad", {}):
                delivered.append(chunk)
        assert delivered == [{"a": 1}]
        assert exc_info.value.details["actual_type"] == expected_type
        assert exc_info.value.details["chunk_index"] == 1


# --------------------------------------------------------------------------- #
# PROPERTY
# --------------------------------------------------------------------------- #


class TestStreamProperties:
    async def test_stream_property_async(self) -> None:
        """streaming.stream.property.async

        Contract.Properties: async = true. stream() returns an async iterator;
        the module's stream() is an async generator function.
        """
        mod = StreamingCounter()
        assert inspect.isasyncgenfunction(mod.stream)
        ex = _make_executor(module=mod, module_id="counter")
        result = ex.stream("counter", {"count": 1})
        assert hasattr(result, "__anext__")
        assert await result.__anext__() == {"value": 1}

    async def test_stream_property_thread_safe_false_concurrent_consumers(self) -> None:
        """streaming.stream.property.thread_safe_false

        Contract.Properties: thread_safe = false — a single stream instance
        MUST NOT be shared across concurrent consumers. We assert the contract
        operationally: ≥8 *independent* stream instances run concurrently via
        asyncio.gather and each produces its own correct, isolated sequence
        (no cross-talk), while sharing one instance is unsupported.
        """
        ex = _make_executor(module=StreamingCounter(), module_id="counter")

        async def run(n: int) -> list[dict[str, Any]]:
            # Each consumer gets its OWN stream instance — the supported pattern.
            return await _collect(ex.stream("counter", {"count": n}))

        results = await asyncio.gather(*(run(n) for n in range(1, 9)))
        assert len(results) == 8
        for n, res in zip(range(1, 9), results):
            assert res == [{"value": i} for i in range(1, n + 1)]

    async def test_stream_property_idempotent_false(self) -> None:
        """streaming.stream.property.idempotent_false

        Contract.Properties: idempotent = false. A stateful streaming module
        whose output depends on prior calls demonstrates non-idempotence:
        repeated invocations need not yield identical sequences.
        """

        class CounterStateModule:
            def __init__(self) -> None:
                self.input_schema = None
                self.output_schema = None
                self._calls = 0

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {}

            async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
                self._calls += 1
                yield {"call": self._calls}

        mod = CounterStateModule()
        ex = _make_executor(module=mod, module_id="stateful")
        first = await _collect(ex.stream("stateful", {}))
        second = await _collect(ex.stream("stateful", {}))
        assert first == [{"call": 1}]
        assert second == [{"call": 2}]
        assert first != second  # not idempotent

    async def test_stream_property_pure_false(self) -> None:
        """streaming.stream.property.pure_false

        Contract.Properties: pure = false (may hold open connections / side
        state). A module that mutates external state during streaming proves
        the call is not pure.
        """
        side_effects: list[int] = []

        class ImpureModule:
            def __init__(self) -> None:
                self.input_schema = None
                self.output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {}

            async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
                for i in range(3):
                    side_effects.append(i)  # external mutation
                    yield {"i": i}

        ex = _make_executor(module=ImpureModule(), module_id="impure")
        await _collect(ex.stream("impure", {}))
        assert side_effects == [0, 1, 2]  # external state was mutated


# --------------------------------------------------------------------------- #
# SIDE_EFFECT  (ordering, accumulation, fallback, post-validation phase)
# --------------------------------------------------------------------------- #


class TestStreamSideEffects:
    async def test_stream_side_effect_chunk_ordering_preserved(self) -> None:
        """streaming.stream.side_effect.chunk_ordering_preserved

        Phase 2: the executor yields each chunk to the caller immediately and
        in source order — chunk ordering MUST be preserved.
        """
        ex = _make_executor(module=StreamingCounter(), module_id="counter")
        chunks = await _collect(ex.stream("counter", {"count": 5}))
        assert chunks == [{"value": i} for i in range(1, 6)]

    async def test_stream_side_effect_deep_merge_accumulation(self) -> None:
        """streaming.stream.side_effect.deep_merge_accumulation

        Requirement: chunks MUST be accumulated via deep merge to produce the
        final combined output for validation. Nested dicts merge recursively;
        right value wins for scalars (per the spec's worked example).
        """
        captured: dict[str, Any] = {}

        class CaptureAfter(Middleware):
            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                nonlocal captured
                captured = dict(output)
                return None

        ex = _make_executor(
            module=NestedMergeModule(),
            module_id="merge",
            middlewares=[CaptureAfter()],
        )
        await _collect(ex.stream("merge", {}))
        # Spec example: content right-wins; metadata deep-merges.
        assert captured == {
            "content": " world",
            "metadata": {"tokens": 1, "model": "gpt-4"},
        }

    async def test_stream_side_effect_fallback_single_chunk_for_non_streaming(
        self,
    ) -> None:
        """streaming.stream.side_effect.fallback_single_chunk

        Requirement: if a module does not implement stream(), the executor's
        stream() MUST fall back to execute() and yield the complete result as
        a single chunk.
        """
        ex = _make_executor(module=PlainModule(), module_id="plain")
        chunks = await _collect(ex.stream("plain", {}))
        assert chunks == [{"result": "done"}]

    async def test_stream_side_effect_after_middleware_runs_post_accumulation(
        self,
    ) -> None:
        """streaming.stream.side_effect.after_middleware_post_accumulation

        Phase 3: output validation + after-middleware MUST run on the
        accumulated output AFTER all chunks are emitted — before runs first,
        after runs last, and after receives the merged output.
        """
        order: list[str] = []
        captured: dict[str, Any] = {}

        class OrderMiddleware(Middleware):
            def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                order.append("before")
                return None

            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                order.append("after")
                nonlocal captured
                captured = dict(output)
                return None

        ex = _make_executor(
            module=NestedMergeModule(),
            module_id="merge",
            middlewares=[OrderMiddleware()],
        )
        chunks = await _collect(ex.stream("merge", {}))
        assert len(chunks) == 2
        assert order == ["before", "after"]
        # After-middleware saw the fully accumulated output.
        assert captured["metadata"] == {"tokens": 1, "model": "gpt-4"}

    async def test_stream_side_effect_deep_merge_depth_capped(self) -> None:
        """streaming.stream.side_effect.deep_merge_depth_capped

        Requirement: deep merge MUST be depth-capped (default 32). Per
        streaming.md ("Depth cap"): "If nesting exceeds this limit, the right
        value replaces the left at that level without further recursion."
        We build two chunks nested past 32 levels sharing a divergent leaf;
        the merge must terminate (no stack overflow) AND, at the cap, the
        right value must win.

        NOTE: currently FAILING — a genuine spec violation. The impl
        (src/apcore/executor.py:179-180) does `if _depth >= _MAX_MERGE_DEPTH:
        return`, dropping the override entirely instead of replacing the left
        with the right value as the spec mandates. The surviving leaf is
        therefore the LEFT chunk's "a" rather than the required right "b".
        """

        def nested(depth: int, leaf: Any) -> dict[str, Any]:
            node: dict[str, Any] = {"leaf": leaf}
            for _ in range(depth):
                node = {"n": node}
            return node

        class DeepModule:
            def __init__(self) -> None:
                self.input_schema = None
                self.output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {}

            async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
                yield nested(40, "a")
                yield nested(40, "b")

        captured: dict[str, Any] = {}

        class CaptureAfter(Middleware):
            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                nonlocal captured
                captured = output
                return None

        ex = _make_executor(module=DeepModule(), module_id="deep", middlewares=[CaptureAfter()])
        # Must complete without RecursionError despite 40 > 32 levels.
        chunks = await _collect(ex.stream("deep", {}))
        assert len(chunks) == 2
        # Past the cap the override subtree replaced the base wholesale, so the
        # surviving leaf comes from the second (right) chunk.
        node = captured
        for _ in range(40):
            node = node["n"]
        # Spec mandates right-value-wins at the depth cap.
        assert node["leaf"] == "b"
