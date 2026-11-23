"""Tests for global_timeout (dual-timeout model) enforcement."""

from __future__ import annotations

import asyncio
import time
from typing import Any

import pytest

from apcore.config import Config
from apcore.context import Context
from apcore.errors import ModuleTimeoutError
from apcore.executor import Executor
from apcore.registry import Registry


class SlowModule:
    """Module that sleeps for a configurable duration."""

    description = "slow"
    input_schema = None
    output_schema = None

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        time.sleep(inputs.get("sleep_seconds", 0.3))
        return {"ok": True}


class AsyncSlowModule:
    """Async module that sleeps for a configurable duration."""

    description = "async slow"
    input_schema = None
    output_schema = None

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        await asyncio.sleep(inputs.get("sleep_seconds", 0.3))
        return {"ok": True}


class ChainingModule:
    """Module that calls another module via context.executor."""

    description = "chaining"
    input_schema = None
    output_schema = None

    def __init__(self, target_module_id: str) -> None:
        self._target_module_id = target_module_id

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        time.sleep(inputs.get("sleep_seconds", 0.15))
        return context.executor.call(self._target_module_id, inputs, context)


class FastModule:
    """Module that returns immediately."""

    description = "fast"
    input_schema = None
    output_schema = None

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"ok": True}


def _make_executor(
    modules: dict[str, Any],
    default_timeout: int = 30000,
    global_timeout: int = 60000,
) -> Executor:
    reg = Registry()
    for module_id, module in modules.items():
        reg.register(module_id, module)
    config = Config(data={"executor": {"default_timeout": default_timeout, "global_timeout": global_timeout}})
    return Executor(registry=reg, config=config)


class TestGlobalDeadlineSetOnRootCall:
    def testglobal_deadline_set_on_root_call(self) -> None:
        captured_ctx: list[Context] = []

        class CapturingModule:
            description = "captures context"
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                captured_ctx.append(context)
                return {"ok": True}

        executor = _make_executor({"capture.mod": CapturingModule()}, global_timeout=5000)
        executor.call("capture.mod", {})

        assert len(captured_ctx) == 1
        ctx = captured_ctx[0]
        assert ctx.global_deadline is not None
        # D-99: `global_deadline` is EPOCH SECONDS, so it is measured against
        # `time.time()`. This assertion read `time.monotonic()` while the
        # executor wrote a monotonic value; both sides were internally
        # consistent and the public `Context.create(global_deadline=...)`
        # parameter was unusable.
        remaining = ctx.global_deadline - time.time()
        assert 0 < remaining <= 5.0


class TestGlobalDeadlineInheritedByChild:
    def test_child_inheritsglobal_deadline(self) -> None:
        parent = Context.create()
        parent.global_deadline = 12345.678

        child = parent.child("child.module")

        assert child.global_deadline == parent.global_deadline


class TestGlobalDeadlineNotSetWhenDisabled:
    def testglobal_deadline_none_when_timeout_zero(self) -> None:
        captured_ctx: list[Context] = []

        class CapturingModule:
            description = "captures context"
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                captured_ctx.append(context)
                return {"ok": True}

        executor = _make_executor({"cap.mod": CapturingModule()}, global_timeout=0)
        executor.call("cap.mod", {})

        assert len(captured_ctx) == 1
        assert captured_ctx[0].global_deadline is None


class TestGlobalTimeoutTriggersBeforeDefault:
    def test_global_timeout_triggers(self) -> None:
        executor = _make_executor(
            {"slow.mod": SlowModule()},
            default_timeout=30000,
            global_timeout=200,
        )

        with pytest.raises(ModuleTimeoutError):
            executor.call("slow.mod", {"sleep_seconds": 0.5})


class TestDefaultTimeoutTriggersBeforeGlobal:
    def test_default_timeout_triggers(self) -> None:
        executor = _make_executor(
            {"slow.mod": SlowModule()},
            default_timeout=100,
            global_timeout=30000,
        )

        with pytest.raises(ModuleTimeoutError):
            executor.call("slow.mod", {"sleep_seconds": 0.5})


class TestGlobalTimeoutAcrossChain:
    def test_chain_global_timeout(self) -> None:
        slow = SlowModule()
        chainer = ChainingModule(target_module_id="slow.mod")

        executor = _make_executor(
            {"chain.mod": chainer, "slow.mod": slow},
            default_timeout=30000,
            global_timeout=200,
        )

        with pytest.raises(ModuleTimeoutError):
            executor.call("chain.mod", {"sleep_seconds": 0.15})


class TestContextSerializationExcludesDeadline:
    def test_serialize_excludesglobal_deadline(self) -> None:
        ctx = Context(trace_id="t1", data={"key": "val"})
        ctx.global_deadline = 99999.0

        serialized = ctx.serialize()

        assert "global_deadline" not in serialized

    def test_deserialize_does_not_restore_deadline(self) -> None:
        ctx = Context(trace_id="t1")
        ctx.global_deadline = 99999.0

        serialized = ctx.serialize()
        restored = Context.deserialize(serialized)

        assert restored.global_deadline is None


class TestGlobalTimeoutAsync:
    @pytest.mark.asyncio
    async def test_async_global_timeout_triggers(self) -> None:
        executor = _make_executor(
            {"async.slow": AsyncSlowModule()},
            default_timeout=30000,
            global_timeout=200,
        )

        with pytest.raises(ModuleTimeoutError):
            await executor.call_async("async.slow", {"sleep_seconds": 0.5})

    @pytest.mark.asyncio
    async def test_async_default_timeout_triggers_before_global(self) -> None:
        executor = _make_executor(
            {"async.slow": AsyncSlowModule()},
            default_timeout=100,
            global_timeout=30000,
        )

        with pytest.raises(ModuleTimeoutError):
            await executor.call_async("async.slow", {"sleep_seconds": 0.5})
