"""Exercise registration and teardown through the Python SDK's public APIs."""

from __future__ import annotations

import asyncio
import subprocess
import sys
import threading
from collections.abc import Iterator
from dataclasses import FrozenInstanceError
from pathlib import Path
from typing import Any

import pytest

import apcore
from apcore import (
    APCore,
    AutoApproveHandler,
    Config,
    DrainAbortedContext,
    DrainContext,
    InvalidInputError,
    LifecycleOperation,
    ModuleNotFoundError,
    Registry,
    ReloadOptions,
    ShutdownOptions,
    UnregisterOptions,
)
from apcore.context import Context
from apcore.errors import ReloadFailedError, SchemaValidationError
from apcore.events.emitter import ApCoreEvent
from apcore.module import ModuleAnnotations


class RecordingModule:
    description = "Record module lifecycle"
    input_schema = {"type": "object"}
    output_schema = {"type": "object"}

    def __init__(self, *, hidden: bool = False) -> None:
        self.annotations = ModuleAnnotations(discoverable=not hidden)
        self.started = asyncio.Event()
        self.finish = asyncio.Event()
        self.loaded = 0
        self.unloaded = 0

    def on_load(self) -> None:
        self.loaded += 1

    def on_unload(self) -> None:
        self.unloaded += 1

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        self.started.set()
        if inputs.get("hold"):
            await self.finish.wait()
        return {"unloaded": self.unloaded}


async def test_shutdown_preserves_registry_failure_and_retries_cleanup(monkeypatch: pytest.MonkeyPatch) -> None:
    client = APCore()
    module = RecordingModule()
    client.register("fixture.active", module)
    failure = RuntimeError("watcher could not stop")

    def fail_unwatch() -> None:
        raise failure

    def fail_close() -> None:
        raise RuntimeError("executor cleanup failed")

    with monkeypatch.context() as patches:
        patches.setattr(client.registry, "unwatch", fail_unwatch)
        patches.setattr(client.executor, "close", fail_close)
        with pytest.raises(RuntimeError) as error:
            await client.shutdown()
        assert error.value is failure

    await client.shutdown()
    assert module.unloaded == 1
    assert not client.registry.has("fixture.active")


def test_owned_shutdown_survives_cancelling_waiter_and_closing_its_loop() -> None:
    client = APCore()
    module = RecordingModule()
    client.register("fixture.active", module)
    with pytest.raises(InvalidInputError, match="running asyncio loop"):
        client.start_shutdown()

    async def start() -> LifecycleOperation:
        operation = client.start_shutdown(options=ShutdownOptions(drain_timeout_ms=0))
        waiter = asyncio.create_task(operation.wait())
        await asyncio.sleep(0)
        waiter.cancel()
        with pytest.raises(asyncio.CancelledError):
            await waiter
        assert (await operation.wait(wait_timeout_ms=1)).status == "pending"
        return operation

    with client.registry.acquire("fixture.active"):
        operation = asyncio.run(start())
        snapshot = operation.snapshot()
        assert snapshot.status == "pending"
        assert module.unloaded == 0

    async def rejoin() -> None:
        assert client.start_shutdown() is operation
        final = await asyncio.wait_for(operation.wait(), 2)
        assert final.status == "completed"
        assert final.operation_id == snapshot.operation_id
        assert [row.outcome for row in final.modules] == ["removed"]
        assert snapshot.status == "pending"
        with pytest.raises(FrozenInstanceError):
            setattr(final, "status", "failed")

    asyncio.run(rejoin())
    assert module.unloaded == 1


async def test_shutdown_unload_hook_and_unregister_callback_can_snapshot_but_cannot_wait() -> None:
    client = APCore()
    observed: list[tuple[str, str]] = []

    def inspect_operation(label: str) -> None:
        async def attempt() -> None:
            operation = client.start_shutdown()
            snapshot = await operation.wait(wait_timeout_ms=0)
            assert snapshot.status == "pending"
            with pytest.raises(InvalidInputError, match="A lifecycle callback cannot wait for its own operation"):
                await operation.wait()
            observed.append((label, snapshot.modules[0].active_hook or "none"))

        asyncio.run(attempt())

    class InspectsShutdown(RecordingModule):
        def on_unload(self) -> None:
            inspect_operation("hook")
            super().on_unload()

    module = InspectsShutdown()
    client.register("fixture.inspect", module)
    client.registry.on("unregister", lambda module_id, module: inspect_operation("callback"))

    operation = client.start_shutdown()
    final = await asyncio.wait_for(operation.wait(), 2)
    assert final.status == "completed"
    assert module.unloaded == 1
    assert observed == [("hook", "on_unload"), ("callback", "none")]


async def test_shutdown_reports_unload_failure_and_attempts_other_registrations() -> None:
    client = APCore()

    class RaisesOnUnload(RecordingModule):
        def on_unload(self) -> None:
            super().on_unload()
            raise RuntimeError("sensitive exception contents")

    healthy = RecordingModule()
    failing = RaisesOnUnload()
    client.register_many([("fixture.healthy", healthy), ("fixture.failing", failing)])

    final = await asyncio.wait_for(client.start_shutdown().wait(), 2)
    assert final.status == "failed"
    assert [row.outcome for row in final.modules] == ["failed", "removed"]
    assert final.error == final.modules[0].error
    assert final.error is not None and final.error.phase == "unloading"
    assert "sensitive exception contents" not in repr(final)
    await client.shutdown()
    assert (healthy.unloaded, failing.unloaded) == (1, 1)


async def test_each_client_reports_its_own_flush_and_subscriber_self_wait_is_rejected() -> None:
    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    peer = APCore(registry=client.registry)
    started = threading.Event()
    release = threading.Event()
    wait_rejected = threading.Event()

    async def subscriber(event: Any) -> None:
        operation = client.start_shutdown(options=ShutdownOptions(event_flush_timeout_ms=0))
        with pytest.raises(InvalidInputError, match="A lifecycle callback cannot wait for its own operation"):
            await operation.wait()
        wait_rejected.set()
        started.set()
        assert release.wait(timeout=2)

    client.on("apcore.registry.module_registered", subscriber)
    client.register("fixture.active", RecordingModule())
    try:
        assert await asyncio.to_thread(started.wait, 2)
        assert wait_rejected.is_set()
        operation = client.start_shutdown()
        joined = peer.start_shutdown()
        with pytest.raises(InvalidInputError, match="event_flush_timeout_ms"):
            client.start_shutdown(options=ShutdownOptions(event_flush_timeout_ms=1))
        first, second = await asyncio.wait_for(asyncio.gather(operation.wait(), joined.wait()), 2)
        assert first.operation_id == second.operation_id
        assert first.modules == second.modules
        assert first.event_flush == "timeout"
        assert second.event_flush == "not_applicable"
        assert first.status == second.status == "completed"
    finally:
        release.set()


async def test_shutdown_keeps_existing_removal_observable_and_takes_remaining_batch_entries() -> None:
    client = APCore()
    first = RecordingModule()
    second = RecordingModule()
    client.register_many([("fixture.first", first), ("fixture.second", second)])
    with client.registry.acquire("fixture.first"), client.registry.acquire("fixture.second"):
        batch = client.registry.start_unregister_many(
            ["fixture.first", "fixture.second"], options=UnregisterOptions(drain_timeout_ms=0)
        )
        await wait_for_drain(client.registry, "fixture.first")
        shutdown = client.start_shutdown()
        with pytest.raises(InvalidInputError, match="shut down"):
            client.start_unregister_many(["fixture.second"])
        rows = {row.module_id: row for row in shutdown.snapshot().modules}
        assert rows["fixture.first"].phase == "draining"
        assert rows["fixture.second"].phase == "pending"

    batch_report, shutdown_report = await asyncio.wait_for(asyncio.gather(batch.wait(), shutdown.wait()), 2)
    assert batch_report.modules[0].outcome == "removed"
    assert batch_report.modules[1].outcome in {"already_removing", "absent"}
    assert [row.outcome for row in shutdown_report.modules] == ["removed", "removed"]
    assert (first.unloaded, second.unloaded) == (1, 1)


async def test_owned_shutdown_joins_detached_registration_until_its_lease_settles() -> None:
    client = APCore()
    module = RecordingModule()
    client.register("fixture.detached", module)
    with client.registry.acquire("fixture.detached"):
        assert client.registry.unregister("fixture.detached")
        shutdown = client.start_shutdown()
        pending = await shutdown.wait(wait_timeout_ms=1)
        assert pending.status == "pending"
        assert [row.phase for row in pending.modules] == ["draining"]
        assert module.unloaded == 0

    final = await asyncio.wait_for(shutdown.wait(), 2)
    assert [row.outcome for row in final.modules] == ["removed"]
    assert module.unloaded == 1


async def test_shutdown_during_managed_recovery_never_reopens_generation() -> None:
    config = Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}})
    client = APCore(config=config)
    peer = APCore(registry=client.registry)
    client.executor.set_approval_handler(AutoApproveHandler())
    recovering, ready = asyncio.Event(), asyncio.Event()

    class StatefulModule(RecordingModule):
        async def on_drain(self, context: DrainContext) -> None:
            assert context.reason_kind in {"reload", "shutdown"}

        async def on_drain_aborted(self, context: DrainAbortedContext) -> None:
            assert context.abort_reason == "drain_timeout"
            recovering.set()
            await ready.wait()

    module = StatefulModule()
    await client.register_async("fixture.stateful", module)
    active = asyncio.create_task(client.call_async("fixture.stateful", {"hold": True}))
    reload_task = None
    try:
        await asyncio.wait_for(module.started.wait(), 2)
        reload_task = asyncio.create_task(
            client.call_async(
                "system.control.reload_module",
                {
                    "module_id": "fixture.stateful",
                    "reason": "test recovery",
                    "lifecycle": {"drain_timeout_ms": 0},
                },
            )
        )
        await asyncio.wait_for(recovering.wait(), 2)
        shutdown = peer.start_shutdown()
        assert shutdown.snapshot().status == "pending"
        ready.set()
        result = await asyncio.wait_for(reload_task, 2)
        assert result["success"] is False
        assert result["lifecycle_report"]["modules"][0]["outcome"] == "retained_unavailable"
        with pytest.raises(ModuleNotFoundError):
            await client.call_async("fixture.stateful")
        assert module.unloaded == 0
        module.finish.set()
        await asyncio.wait_for(active, 2)
        final = await asyncio.wait_for(shutdown.wait(), 2)
        assert final.status == "completed"
        assert module.unloaded == 1
    finally:
        ready.set()
        module.finish.set()
        await asyncio.gather(*(task for task in (active, reload_task) if task is not None), return_exceptions=True)
        await client.shutdown()


async def test_managed_bulk_reload_attempts_every_validated_generation() -> None:
    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    client.executor.set_approval_handler(AutoApproveHandler())

    class RefusesDrain(RecordingModule):
        def on_drain(self, context: DrainContext) -> None:
            raise RuntimeError("drain refused")

        def on_drain_aborted(self, context: DrainAbortedContext) -> None:
            pass

    first, second, replacement = RefusesDrain(), RecordingModule(), RecordingModule()

    class Discoverer:
        def discover(self, roots: list[str]) -> list[dict[str, Any]]:
            return [{"module_id": "fixture.second", "module": replacement}]

    client.registry.set_discoverer(Discoverer())
    client.register_many([("fixture.first", first), ("fixture.second", second)])
    try:
        result = await client.call_async(
            "system.control.reload_module",
            {"path_filter": "fixture.*", "reason": "test partial reload", "lifecycle": {}},
        )
        assert result["success"] is False
        assert result["reloaded_modules"] == ["fixture.second"]
        assert [row["outcome"] for row in result["lifecycle_report"]["modules"]] == ["retained_ready", "reloaded"]
        assert client.registry.get("fixture.first") is first
        assert client.registry.get("fixture.second") is replacement
        assert (first.unloaded, second.unloaded) == (0, 1)
    finally:
        await client.shutdown()


async def test_async_initialization_after_shutdown_is_cleaned_without_publication() -> None:
    client = APCore()
    entered, release = asyncio.Event(), asyncio.Event()
    registrations = []
    client.registry.on("register", lambda module_id, module: registrations.append(module_id))

    class AsyncLoad(RecordingModule):
        async def on_load(self) -> None:
            entered.set()
            await release.wait()
            super().on_load()

        async def on_unload(self) -> None:
            super().on_unload()

    module = AsyncLoad()
    registering = asyncio.create_task(client.register_async("fixture.async_load", module))
    try:
        await asyncio.wait_for(entered.wait(), 2)
        assert client.registry.get("fixture.async_load") is None
        shutdown = client.start_shutdown()
        assert shutdown.snapshot().status == "pending"
        release.set()
        with pytest.raises(InvalidInputError, match="shut down"):
            await asyncio.wait_for(registering, 2)
        assert (await asyncio.wait_for(shutdown.wait(), 2)).status == "completed"
        assert registrations == []
        assert (module.loaded, module.unloaded) == (1, 1)
    finally:
        release.set()
        await asyncio.gather(registering, return_exceptions=True)


def test_async_hook_does_not_enable_async_lifecycle_in_nested_sync_registration() -> None:
    probe = """
import asyncio
from apcore import APCore
from apcore.errors import ModuleError

client = APCore()

class AsyncChild:
    description = "Async nested registration"
    input_schema = {"type": "object"}
    output_schema = {"type": "object"}

    def execute(self, inputs, context):
        return {}

    async def on_load(self):
        raise AssertionError("synchronous register must reject this hook before running it")

class Parent(AsyncChild):
    async def on_load(self):
        try:
            client.register("fixture.child", AsyncChild())
        except ModuleError as error:
            assert error.code == "MODULE_LOAD_ERROR"
        else:
            raise AssertionError("expected nested synchronous registration to fail")

async def main():
    await client.register_async("fixture.parent", Parent())
    assert client.registry.has("fixture.parent")
    assert not client.registry.has("fixture.child")
    await client.shutdown()

asyncio.run(main())
"""
    result = subprocess.run([sys.executable, "-c", probe], capture_output=True, text=True, timeout=5)
    assert result.returncode == 0, result.stderr


async def test_declined_managed_reload_cleanup_rejects_a_shutdown_self_wait() -> None:
    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    client.executor.set_approval_handler(AutoApproveHandler())
    entered, release = asyncio.Event(), asyncio.Event()
    cleanup_results = []

    class DeclinedModule(RecordingModule):
        async def on_load(self) -> None:
            entered.set()
            await release.wait()

        async def on_unload(self) -> None:
            try:
                await asyncio.wait_for(client.shutdown(), timeout=0.1)
            except (InvalidInputError, TimeoutError) as error:
                cleanup_results.append(error)
            super().on_unload()

    replacement = DeclinedModule()

    class Discoverer:
        def discover(self, roots: list[str]) -> list[dict[str, Any]]:
            return [{"module_id": "fixture.declined", "module": replacement}]

    client.registry.set_discoverer(Discoverer())
    await client.register_async("fixture.declined", RecordingModule())
    reloading = asyncio.create_task(
        client.call_async(
            "system.control.reload_module",
            {"module_id": "fixture.declined", "reason": "test rejected publication", "lifecycle": {}},
        )
    )
    try:
        await asyncio.wait_for(entered.wait(), timeout=2)
        shutdown = client.start_shutdown()
        release.set()
        assert (await asyncio.wait_for(reloading, timeout=2))["success"] is False
        assert (await asyncio.wait_for(shutdown.wait(), timeout=2)).status == "completed"
        assert replacement.unloaded == 1
        assert len(cleanup_results) == 1 and isinstance(cleanup_results[0], InvalidInputError)
    finally:
        release.set()
        await asyncio.gather(reloading, return_exceptions=True)
        await client.shutdown()


async def test_each_shutdown_client_emits_once_with_the_shared_registry_operation_id() -> None:
    config = Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}})
    first = APCore(config=config)
    second = APCore(registry=first.registry, config=config)
    first_events: list[ApCoreEvent] = []
    second_events: list[ApCoreEvent] = []
    event = "apcore.client.shutdown_resources_completed"
    first.on(event, first_events.append)
    second.on(event, second_events.append)
    module = RecordingModule()
    first.register("fixture.active", module)
    a, b = await asyncio.wait_for(asyncio.gather(first.start_shutdown().wait(), second.start_shutdown().wait()), 2)
    assert (await first.start_shutdown().wait()) == a
    assert (await second.start_shutdown().wait()) == b
    assert a.event_flush == b.event_flush == "completed"
    assert a.operation_id == b.operation_id
    assert [item.data["operation_id"] for item in first_events] == [a.operation_id]
    assert [item.data["operation_id"] for item in second_events] == [a.operation_id]
    assert module.unloaded == 1


async def test_forced_removal_keeps_late_cleanup_visible_to_shutdown_owner() -> None:
    client = APCore()
    old = RecordingModule()
    client.register("fixture.held", old)
    active = asyncio.create_task(client.call_async("fixture.held", {"hold": True}))
    try:
        await asyncio.wait_for(old.started.wait(), 2)
        removing = client.start_unregister_many(
            ["fixture.held"],
            options=UnregisterOptions(drain_timeout_ms=0, on_drain_timeout="cancel_then_force", cancel_grace_ms=0),
        )
        async with asyncio.timeout(2):
            while not removing.snapshot().modules[0].cleanup_pending:
                await asyncio.sleep(0.001)
        assert old.unloaded == 1
        shutdown = client.start_shutdown()
        snapshot = shutdown.snapshot()
        assert snapshot.status == "pending"
        row = snapshot.modules[0]
        assert row.outcome == "forced_removed" and row.cleanup_pending
        assert row.active_at_unload == 1
        old.finish.set()
        await asyncio.gather(active, return_exceptions=True)
        results = await asyncio.wait_for(asyncio.gather(removing.wait(), shutdown.wait()), 2)
        assert all(report.status == "completed" and not report.modules[0].cleanup_pending for report in results)
        assert old.unloaded == 1
    finally:
        old.finish.set()
        await asyncio.gather(active, return_exceptions=True)


async def wait_for_drain(registry: Registry, module_id: str) -> None:
    async with asyncio.timeout(2):
        while not registry.is_draining(module_id):
            await asyncio.sleep(0)


async def test_unregister_drains_the_removed_registration_without_touching_its_replacement() -> None:
    client = APCore()
    old = RecordingModule()
    replacement = RecordingModule()
    client.register("fixture.busy", old)
    call = asyncio.create_task(client.call_async("fixture.busy", {"hold": True}))
    await asyncio.wait_for(old.started.wait(), 2)
    try:
        assert client.registry.unregister("fixture.busy")
        assert client.registry.get("fixture.busy") is None
        assert not client.registry.unregister("fixture.busy")
        with pytest.raises(ModuleNotFoundError):
            await client.call_async("fixture.busy")
        with pytest.raises(InvalidInputError) as reused:
            client.register("fixture.busy", old)
        assert reused.value.code == "DUPLICATE_MODULE_ID"
        client.register("fixture.busy", replacement)
        assert await client.call_async("fixture.busy") == {"unloaded": 0}
    finally:
        old.finish.set()
        result = await asyncio.wait_for(call, 2)
    assert result == {"unloaded": 0}
    assert old.unloaded == 1
    assert replacement.unloaded == 0


async def test_safe_unregister_requires_the_async_waiter_for_active_calls_on_a_loop() -> None:
    client = APCore()
    module = RecordingModule()
    client.register("fixture.busy", module)
    call = asyncio.create_task(client.call_async("fixture.busy", {"hold": True}))
    await asyncio.wait_for(module.started.wait(), 2)
    unregister = None
    try:
        with pytest.raises(InvalidInputError, match="safe_unregister_async"):
            client.registry.safe_unregister("fixture.busy")
        assert client.registry.get("fixture.busy") is module
        unregister = asyncio.create_task(client.registry.safe_unregister_async("fixture.busy"))
        await wait_for_drain(client.registry, "fixture.busy")
    finally:
        module.finish.set()
        assert await asyncio.wait_for(call, 2) == {"unloaded": 0}
        if unregister is not None:
            assert await asyncio.wait_for(unregister, 2)
    assert module.unloaded == 1


@pytest.mark.parametrize("wait", [False, True])
def test_unregister_unloads_each_registered_version(wait: bool) -> None:
    registry = Registry()
    first = RecordingModule()
    second = RecordingModule()
    registry.register("fixture.versions", first, version="1.0.0")
    registry.register("fixture.versions", second, version="2.0.0")
    unregister = registry.safe_unregister if wait else registry.unregister

    assert unregister("fixture.versions")
    assert not unregister("fixture.versions")
    assert (first.unloaded, second.unloaded) == (1, 1)


def test_duplicate_version_rejected_before_load() -> None:
    registry = Registry()
    original = RecordingModule()
    duplicate = RecordingModule()
    registry.register("fixture.versions", original, version="1.0.0")
    with pytest.raises(InvalidInputError) as error:
        registry.register("fixture.versions", duplicate, version="1.0.0")

    assert error.value.code == "DUPLICATE_MODULE_ID"
    assert duplicate.loaded == 0
    assert registry.get("fixture.versions") is original


COUNTER_SOURCE = """
import asyncio
from pydantic import BaseModel

class Input(BaseModel):
    hold: bool = False

class Output(BaseModel):
    count: int

class Counter:
    description = "Count successful calls across a reload"
    input_schema = Input
    output_schema = Output

    def __init__(self):
        self.count = 0
        self.started = asyncio.Event()
        self.finish = asyncio.Event()

    async def execute(self, inputs, context):
        if inputs.get("hold"):
            self.started.set()
            await self.finish.wait()
            self.count += 1
        return {"count": self.count}

    def on_suspend(self):
        return {"count": self.count}

    def on_resume(self, state):
        self.count = state["count"]
"""


async def test_watcher_uses_managed_drain_policy_and_registration_hook_loop(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    observers = pytest.importorskip("watchdog.observers")

    class IdleObserver:
        def schedule(self, handler: Any, path: str, recursive: bool) -> None:
            pass

        def start(self) -> None:
            pass

        def stop(self) -> None:
            pass

        def join(self) -> None:
            pass

    monkeypatch.setattr(observers, "Observer", IdleObserver)
    source = tmp_path / "counter.py"
    source.write_text(
        COUNTER_SOURCE
        + """
    async def on_drain(self, context):
        self.finish.set()

    async def on_drain_aborted(self, context):
        pass
""",
        encoding="utf-8",
    )
    client = APCore(
        registry=Registry(extensions_dir=tmp_path),
        config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}),
    )
    drain_events: list[ApCoreEvent] = []
    client.on("apcore.registry.module_drain_started", drain_events.append)
    assert await client.discover_async() == 1
    old = client.registry.get("counter")
    assert old is not None
    client.registry.watch(lifecycle=ReloadOptions(drain_timeout_ms=2000))
    active = asyncio.create_task(client.call_async("counter", {"hold": True}))
    try:
        await asyncio.wait_for(old.started.wait(), 2)
        source.write_text(
            COUNTER_SOURCE.replace('return {"count": self.count}', 'return {"count": self.count + 10}')
            + """
    async def on_load(self):
        await asyncio.sleep(0)
""",
            encoding="utf-8",
        )
        await asyncio.to_thread(client.registry._handle_file_change, str(source))
        assert await asyncio.wait_for(active, 2) == {"count": 1}
        async with asyncio.timeout(2):
            while client.registry.get("counter") in (None, old):
                await asyncio.sleep(0.001)
        assert await client.call_async("counter") == {"count": 11}
        assert client.events is not None
        await asyncio.to_thread(client.events.flush)
        assert [entry.data["drain_timeout_ms"] for entry in drain_events] == [2000]
        assert [entry.data["reason_kind"] for entry in drain_events] == ["reload"]
    finally:
        old.finish.set()
        await asyncio.gather(active, return_exceptions=True)
        await client.shutdown()


@pytest.mark.parametrize("final_change", ["modify", "delete"])
async def test_managed_watch_does_not_publish_a_stale_file_update(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, final_change: str
) -> None:
    observers = pytest.importorskip("watchdog.observers")

    class IdleObserver:
        def schedule(self, handler: Any, path: str, recursive: bool) -> None:
            pass

        def start(self) -> None:
            pass

        def stop(self) -> None:
            pass

        def join(self) -> None:
            pass

    monkeypatch.setattr(observers, "Observer", IdleObserver)
    source = tmp_path / "counter.py"
    source.write_text(
        COUNTER_SOURCE
        + """
    async def on_drain(self, context):
        self.started.set()
        await self.finish.wait()

    async def on_drain_aborted(self, context):
        pass
""",
        encoding="utf-8",
    )
    registry = Registry(extensions_dir=tmp_path)
    client = APCore(registry=registry)
    assert await client.discover_async() == 1
    old = registry.get("counter")
    operations = []
    published = []
    start = registry._start_managed_reload

    def capture_started_reload(*args: Any, **kwargs: Any) -> LifecycleOperation:
        operation = start(*args, **kwargs)
        operations.append(operation)
        return operation

    monkeypatch.setattr(registry, "_start_managed_reload", capture_started_reload)
    registry.on("register", lambda _, module: published.append(getattr(module, "version", None)))
    registry.watch(lifecycle=ReloadOptions(drain_timeout_ms=2000))
    try:
        source.write_text(COUNTER_SOURCE + '\n    version = "2.0.0"\n', encoding="utf-8")
        await asyncio.to_thread(registry._handle_file_change, str(source))
        await asyncio.wait_for(old.started.wait(), timeout=2)
        if final_change == "delete":
            source.unlink()
            await asyncio.to_thread(registry._handle_file_deletion, str(source))
        else:
            source.write_text(COUNTER_SOURCE + '\n    version = "3.0.0"\n\n', encoding="utf-8")
            await asyncio.to_thread(registry._handle_file_change, str(source))
        old.finish.set()
        await asyncio.wait_for(operations[0].wait(), timeout=2)
        if final_change == "delete":
            assert not registry.has("counter")
        else:
            async with asyncio.timeout(2):
                while getattr(registry.get("counter"), "version", None) != "3.0.0":
                    await asyncio.sleep(0.001)
        assert "2.0.0" not in published
    finally:
        old.finish.set()
        await client.shutdown()


@pytest.mark.parametrize("watch", [False, True])
@pytest.mark.parametrize("declare_in", ["code", "metadata"])
def test_filesystem_registration_rejects_invalid_streaming_module(tmp_path: Path, watch: bool, declare_in: str) -> None:
    source = tmp_path / "counter.py"
    source.write_text(COUNTER_SOURCE, encoding="utf-8")
    client = APCore(registry=Registry(extensions_dir=str(tmp_path)))
    old = None
    if watch:
        assert client.discover() == 1
        old = client.registry.get("counter")

    if declare_in == "metadata":
        (tmp_path / "counter_meta.yaml").write_text("annotations:\n  streaming: true\n", encoding="utf-8")
    else:
        source.write_text(
            COUNTER_SOURCE.replace(
                "    input_schema = Input", '    annotations = {"streaming": True}\n    input_schema = Input'
            ),
            encoding="utf-8",
        )

    if watch:
        client.registry._handle_file_change(str(source))
    else:
        assert client.discover() == 0
    assert client.registry.get("counter") is old


@pytest.mark.parametrize("watch", [False, True])
async def test_filesystem_registration_adapts_instance_dict_schemas(tmp_path: Path, watch: bool) -> None:
    source = tmp_path / "counter.py"
    source.write_text(COUNTER_SOURCE, encoding="utf-8")
    client = APCore(registry=Registry(extensions_dir=str(tmp_path)))
    if watch:
        assert client.discover() == 1
    source.write_text(
        COUNTER_SOURCE.replace(
            "        self.count = 0",
            '        self.input_schema = {"type": "object", "properties": {"hold": {"type": "boolean"}}}\n'
            '        self.output_schema = {"type": "object", "required": ["count"]}\n'
            "        self.count = 0",
        ),
        encoding="utf-8",
    )

    if watch:
        client.registry._handle_file_change(str(source))
    else:
        assert client.discover() == 1

    with pytest.raises(SchemaValidationError):
        await client.call_async("counter", {"hold": 0})
    assert await client.call_async("counter") == {"count": 0}


@pytest.mark.parametrize("selector", ["module_id", "path_filter"])
@pytest.mark.parametrize("fails", [False, True])
async def test_legacy_control_reload_preserves_the_registration_hook_adapter(selector: str, fails: bool) -> None:
    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    client.executor.set_approval_handler(AutoApproveHandler())
    registration_loop = asyncio.get_running_loop()

    class AsyncLifecycle(RecordingModule):
        async def on_load(self) -> None:
            assert asyncio.get_running_loop() is registration_loop
            super().on_load()

        async def on_unload(self) -> None:
            assert asyncio.get_running_loop() is registration_loop
            super().on_unload()

    class Replacement(AsyncLifecycle):
        async def on_load(self) -> None:
            await super().on_load()
            if fails:
                raise RuntimeError("replacement load rejected")

    old, replacement = AsyncLifecycle(), Replacement()

    class Discoverer:
        def discover(self, roots: list[str]) -> list[dict[str, Any]]:
            return [{"module_id": "fixture.legacy", "module": replacement}]

    client.registry.set_discoverer(Discoverer())
    await client.register_async("fixture.legacy", old)
    inputs = {selector: "fixture.legacy", "reason": "test async hooks through the legacy reload contract"}
    try:
        if fails:
            with pytest.raises(ReloadFailedError):
                await client.call_async("system.control.reload_module", inputs)
            assert client.registry.get("fixture.legacy") is old
            assert old.loaded == 2
            assert replacement.loaded == 1
        else:
            result = await client.call_async("system.control.reload_module", inputs)
            assert result["success"] is True
            assert "lifecycle_report" not in result
            assert client.registry.get("fixture.legacy") is replacement
            assert old.unloaded == 1 and replacement.loaded == 1
    finally:
        await client.shutdown()


@pytest.mark.parametrize("selector", ["module_id", "path_filter"])
async def test_control_reload_snapshots_after_the_last_call(tmp_path: Path, selector: str) -> None:
    (tmp_path / "counter.py").write_text(COUNTER_SOURCE, encoding="utf-8")
    client = APCore(
        registry=Registry(extensions_dir=str(tmp_path)),
        config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}),
    )
    client.executor.set_approval_handler(AutoApproveHandler())
    assert client.discover() == 1
    old = client.registry.get("counter")
    call = asyncio.create_task(client.call_async("counter", {"hold": True}))
    await asyncio.wait_for(old.started.wait(), 2)
    reload = asyncio.create_task(
        client.call_async(
            "system.control.reload_module",
            {
                selector: "counter",
                "reason": "preserve the final call's state",
            },
        )
    )
    try:
        await wait_for_drain(client.registry, "counter")
    finally:
        old.finish.set()
        assert await asyncio.wait_for(call, 2) == {"count": 1}
        assert (await asyncio.wait_for(reload, 2))["success"]
        client.close()
    assert client.registry.get("counter") is not old
    assert await client.call_async("counter") == {"count": 1}


@pytest.mark.parametrize("restore_previous", [False, True])
async def test_reload_does_not_publish_a_module_while_it_is_resuming(restore_previous: bool) -> None:
    started = threading.Event()
    finish = threading.Event()

    class ResumingModule(RecordingModule):
        def on_suspend(self) -> dict[str, bool]:
            return {"ready": True}

        def on_resume(self, state: dict[str, bool]) -> None:
            started.set()
            assert finish.wait(timeout=2)

    old = ResumingModule()
    replacement = ResumingModule()

    class TestDiscoverer:
        def discover(self, roots: list[str]) -> list[dict[str, Any]]:
            if restore_previous:
                raise RuntimeError("replacement could not be discovered")
            return [{"module_id": "fixture.resume", "module": replacement}]

    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    client.executor.set_approval_handler(AutoApproveHandler())
    client.registry.set_discoverer(TestDiscoverer())
    client.register("fixture.resume", old)
    reload = asyncio.create_task(
        client.call_async("system.control.reload_module", {"module_id": "fixture.resume", "reason": "test resume"})
    )
    try:
        assert await asyncio.to_thread(started.wait, 2)
        assert client.registry.get("fixture.resume") is None
        with pytest.raises(ModuleNotFoundError):
            await client.call_async("fixture.resume")
    finally:
        finish.set()
        if restore_previous:
            with pytest.raises(ReloadFailedError):
                await asyncio.wait_for(reload, 2)
        else:
            assert (await asyncio.wait_for(reload, 2))["success"]
        await client.shutdown()


async def test_batch_registration_and_removal_preserve_input_order() -> None:
    client = APCore()
    first = RecordingModule()
    duplicate = RecordingModule()
    skipped = RecordingModule()
    with pytest.raises(InvalidInputError):
        client.register_many([("fixture.first", first), ("fixture.first", duplicate), ("fixture.skip", skipped)])
    assert client.registry.get("fixture.first") is first
    assert (first.loaded, duplicate.loaded, skipped.loaded) == (1, 0, 0)
    assert await client.unregister_many(["fixture.missing", "fixture.first", "fixture.first"]) == [False, True, False]
    assert first.unloaded == 1


async def test_batch_unregistration_consumes_ids_between_unload_hooks() -> None:
    client = APCore()
    first = RecordingModule()
    second = RecordingModule()
    client.register_many([("fixture.first", first), ("fixture.second", second)])
    caller_loop = asyncio.get_running_loop()

    def module_ids() -> Iterator[str]:
        yield "fixture.first"
        assert asyncio.get_running_loop() is caller_loop
        assert first.unloaded == 1
        assert second.unloaded == 0
        yield "fixture.second"

    assert await client.unregister_many(module_ids()) == [True, True]
    assert second.unloaded == 1


async def test_host_shutdown_waits_for_inflight_calls_and_unloads_hidden_modules() -> None:
    client = APCore()
    active = RecordingModule()
    hidden = RecordingModule(hidden=True)
    client.register("fixture.active", active)
    client.register("fixture.hidden", hidden)
    assert "fixture.hidden" not in client.list_modules()
    call = asyncio.create_task(client.call_async("fixture.active", {"hold": True}))
    await asyncio.wait_for(active.started.wait(), 2)
    shutdown = None
    try:
        shutdown = asyncio.create_task(client.shutdown())
        await wait_for_drain(client.registry, "fixture.active")
        with pytest.raises(ModuleNotFoundError):
            await client.call_async("fixture.active")
        with pytest.raises(InvalidInputError, match="shut down"):
            client.register("fixture.extra", RecordingModule())
    finally:
        active.finish.set()
        assert await asyncio.wait_for(call, 2) == {"unloaded": 0}
        if shutdown is not None:
            await asyncio.wait_for(shutdown, 2)
    await client.shutdown()
    assert client.registry.list(visibility=["public", "hidden"]) == []
    assert (active.unloaded, hidden.unloaded) == (1, 1)


async def test_host_shutdown_joins_a_detached_registration() -> None:
    client = APCore()
    module = RecordingModule()
    client.register("fixture.detached", module)
    call = asyncio.create_task(client.call_async("fixture.detached", {"hold": True}))
    await asyncio.wait_for(module.started.wait(), 2)
    shutdown = None
    try:
        assert client.registry.unregister("fixture.detached")
        shutdown = asyncio.create_task(client.shutdown())
        completed, _ = await asyncio.wait({shutdown}, timeout=0.05)
        assert not completed
        assert module.unloaded == 0
    finally:
        module.finish.set()
        assert await asyncio.wait_for(call, 2) == {"unloaded": 0}
        if shutdown is not None:
            await asyncio.wait_for(shutdown, 2)
    assert module.unloaded == 1


def test_closing_the_default_client_unloads_idle_modules() -> None:
    apcore.close()
    module = RecordingModule()
    apcore.register("fixture.default", module)
    apcore.close()
    apcore.close()
    assert module.unloaded == 1
    assert "fixture.default" not in apcore.list_modules()
