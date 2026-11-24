"""Tests for StreamingModule Protocol and registration-time signature validation (apcore #62)."""

from __future__ import annotations

from collections.abc import AsyncIterator
from typing import Any

import pytest

from apcore import StreamingModule, StreamingInterfaceError
from apcore.context import Context
from apcore.module import ModuleAnnotations
from apcore.registry import Registry


class GoodStreamingModule:
    """Satisfies the StreamingModule Protocol."""

    annotations = ModuleAnnotations(streaming=True)

    async def stream(self, inputs: dict, context: Context) -> AsyncIterator[dict]:
        yield {"chunk": 1}
        yield {"chunk": 2}

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}


class NoStreamMethod:
    """Has streaming=True annotation but no stream() method."""

    annotations = ModuleAnnotations(streaming=True)

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}


class SyncStreamMethod:
    """Has streaming=True annotation but stream() is not async."""

    annotations = ModuleAnnotations(streaming=True)

    def stream(self, inputs: dict, context: Context):  # type: ignore[return]
        return iter([{"chunk": 1}])

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}


class WrongArityStream:
    """Has streaming=True annotation but stream() has wrong parameters."""

    annotations = ModuleAnnotations(streaming=True)

    async def stream(self) -> AsyncIterator[dict]:  # type: ignore[override]
        yield {}

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {}


class NonStreamingModule:
    """Regular module without streaming annotation — no stream() method."""

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"result": "ok"}


def test_streaming_module_isinstance_passes() -> None:
    """A module with stream() must satisfy isinstance(module, StreamingModule)."""
    module = GoodStreamingModule()
    assert isinstance(module, StreamingModule)


def test_non_streaming_module_fails_isinstance() -> None:
    """A module without stream() must NOT satisfy isinstance(module, StreamingModule)."""
    module = NonStreamingModule()
    assert not isinstance(module, StreamingModule)


def test_good_streaming_module_registers_cleanly() -> None:
    """A module with valid stream() and streaming=True must register without error."""
    reg = Registry()
    reg.register("ext.test.good_stream", GoodStreamingModule())


def test_missing_stream_method_raises_at_registration() -> None:
    """streaming=True with no stream() must raise StreamingInterfaceError."""
    reg = Registry()
    with pytest.raises(StreamingInterfaceError) as exc_info:
        reg.register("ext.test.no_stream", NoStreamMethod())
    assert exc_info.value.mismatch_reason == "missing_marker"
    assert exc_info.value.module_id == "ext.test.no_stream"


def test_sync_stream_raises_at_registration() -> None:
    """streaming=True with a synchronous stream() must raise StreamingInterfaceError."""
    reg = Registry()
    with pytest.raises(StreamingInterfaceError) as exc_info:
        reg.register("ext.test.sync_stream", SyncStreamMethod())
    assert exc_info.value.mismatch_reason == "not_async"


def test_wrong_arity_raises_at_registration() -> None:
    """streaming=True with wrong stream() params must raise StreamingInterfaceError."""
    reg = Registry()
    with pytest.raises(StreamingInterfaceError) as exc_info:
        reg.register("ext.test.wrong_arity", WrongArityStream())
    assert exc_info.value.mismatch_reason == "wrong_arity"


def test_non_streaming_module_registers_without_validation() -> None:
    """A module without streaming annotation registers regardless of stream() presence."""
    reg = Registry()
    # Should not raise — no streaming annotation, so no validation
    reg.register("ext.test.non_streaming", NonStreamingModule())


def test_streaming_interface_error_fields() -> None:
    """StreamingInterfaceError must carry all expected fields."""
    err = StreamingInterfaceError(
        module_id="ext.test.foo",
        expected_signature="(self, inputs: dict, context: Context) -> AsyncIterator[dict]",
        actual_signature="(self) -> AsyncIterator[dict]",
        mismatch_reason="wrong_arity",
    )
    assert err.module_id == "ext.test.foo"
    assert err.mismatch_reason == "wrong_arity"
    assert "STREAMING_INTERFACE_MISMATCH" in str(err)
