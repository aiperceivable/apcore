"""Tests for Registry on_load ordering invariants — deferred-publish (apcore #65)."""

from __future__ import annotations

import threading
import time
from concurrent.futures import ThreadPoolExecutor
from typing import Any

import pytest

from apcore.errors import InvalidInputError
from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.registry import Registry
from apcore.registry.registry import ErrorCodes


class _WarmupModule:
    """Module with a 50ms on_load that sets a flag."""

    data: dict[str, Any] = {}

    def on_load(self) -> None:
        time.sleep(0.05)
        _WarmupModule.data["_test.warmed"] = True


class _FailingModule:
    """Module whose on_load raises ConnectionError."""

    def on_load(self) -> None:
        raise ConnectionError("Failed to reach upstream during initialization")


class _SlowModule:
    """Module with configurable on_load delay."""

    def __init__(self, delay_s: float = 0.05) -> None:
        self._delay = delay_s

    def on_load(self) -> None:
        time.sleep(self._delay)


def test_visibility_after_successful_on_load() -> None:
    """Module MUST be visible after register() returns; NOT visible during on_load."""
    _WarmupModule.data.clear()
    reg = Registry()
    module = _WarmupModule()

    # Track list/get results captured mid on_load
    mid_visible: list[bool] = []
    mid_get: list[Any] = []

    def patched_on_load() -> None:
        time.sleep(0.025)  # First half: check visibility
        mid_visible.append("executor.test.warmup" in reg.list())
        mid_get.append(reg.get("executor.test.warmup"))
        time.sleep(0.025)  # Second half: finish

    module.on_load = patched_on_load  # type: ignore[method-assign]

    reg.register("executor.test.warmup", module)

    # After register() returns, module MUST be visible
    assert "executor.test.warmup" in reg.list()
    assert reg.get("executor.test.warmup") is module

    # During on_load, module was NOT visible
    assert mid_visible == [False]
    assert mid_get == [None]


def test_callback_failure_blocks_visibility() -> None:
    """If on_load raises, module MUST NOT be visible and register() re-raises."""
    reg = Registry()
    module = _FailingModule()

    with pytest.raises(ConnectionError, match="Failed to reach upstream"):
        reg.register("executor.test.failing", module)

    # Module must not be in registry
    assert "executor.test.failing" not in reg.list()
    assert reg.get("executor.test.failing") is None


def test_callback_failure_emits_module_load_failed_event() -> None:
    """on_load failure must emit apcore.registry.module_load_failed via EventEmitter."""
    emitter = EventEmitter()
    reg = Registry()
    reg.set_event_emitter(emitter)

    received: list[ApCoreEvent] = []

    class _Recorder:
        async def on_event(self, event: ApCoreEvent) -> None:
            if event.event_type == "apcore.registry.module_load_failed":
                received.append(event)

    emitter.subscribe(_Recorder())

    with pytest.raises(ConnectionError):
        reg.register("executor.test.failing2", _FailingModule())

    emitter.flush(timeout=5.0)
    emitter.shutdown()

    assert len(received) == 1
    evt = received[0]
    assert evt.data["module_id"] == "executor.test.failing2"
    assert evt.data["error_type"] == "ConnectionError"
    assert "Failed to reach upstream" in evt.data["error_message"]
    assert evt.data["callback_name"] == "on_load"
    for key in ("module_id", "callback_name", "error_type", "error_message", "timestamp"):
        assert key in evt.data


def test_concurrent_same_id_rejects_duplicate() -> None:
    """Two concurrent register() calls for the same ID → one succeeds, one DUPLICATE_MODULE_ID."""
    reg = Registry()

    results: dict[str, Any] = {"success": 0, "errors": []}
    lock = threading.Lock()

    def do_register() -> None:
        try:
            reg.register("executor.test.concurrent", _SlowModule(delay_s=0.05))
            with lock:
                results["success"] += 1
        except InvalidInputError as e:
            with lock:
                results["errors"].append(e)

    t1 = threading.Thread(target=do_register)
    t2 = threading.Thread(target=do_register)
    t1.start()
    t2.start()
    t1.join(timeout=5.0)
    t2.join(timeout=5.0)

    assert results["success"] == 1
    assert len(results["errors"]) == 1
    assert results["errors"][0].code == ErrorCodes.DUPLICATE_MODULE_ID
    assert reg.get("executor.test.concurrent") is not None


def test_concurrent_distinct_ids_run_in_parallel() -> None:
    """Distinct-ID callbacks enter together before either may finish."""
    reg = Registry()
    gate = threading.Barrier(2, timeout=5.0)
    callbacks_overlap = threading.Event()

    class _OverlapModule(_SlowModule):
        def on_load(self) -> None:
            gate.wait()
            callbacks_overlap.set()
            super().on_load()

    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = [
            pool.submit(reg.register, module_id, _OverlapModule(delay_s=0.05))
            for module_id in ("executor.test.parallel_x", "executor.test.parallel_y")
        ]
        for future in futures:
            future.result(timeout=10.0)

    assert callbacks_overlap.is_set()
    assert reg.get("executor.test.parallel_x") is not None
    assert reg.get("executor.test.parallel_y") is not None


def test_no_version_module_not_visible_until_on_load_done() -> None:
    """Module must not appear in get() until on_load completes."""
    reg = Registry()
    mid_result: list[Any] = []

    class _SpyModule:
        def on_load(self) -> None:
            time.sleep(0.01)
            mid_result.append(reg.get("executor.test.spy"))

    reg.register("executor.test.spy", _SpyModule())
    # get() inside on_load should return None (not yet visible)
    assert mid_result == [None]
    # After register() returns, module IS visible
    assert reg.get("executor.test.spy") is not None
