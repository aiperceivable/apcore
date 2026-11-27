"""Exercise registry and host lifecycle through the public SDK."""

from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest
from jsonschema import Draft202012Validator
from pydantic import BaseModel

from apcore import (
    APCore,
    ApprovalRequest,
    ApprovalResult,
    AutoApproveHandler,
    CallbackApprovalHandler,
    CancelToken,
    Config,
    Context,
    ModuleAnnotations,
    Registry,
    ShutdownOptions,
    UnregisterOptions,
)
from apcore.errors import ModuleError, ModuleLoadError, ModuleNotFoundError
from apcore.events.emitter import ApCoreEvent

from .canonical_fixtures import load_fixture, schemas_dir

CASES = load_fixture("module_lifecycle.json")["test_cases"]


class _Input(BaseModel):
    hold: bool = False


class _Output(BaseModel):
    count: int


class Counter:
    description = "Record module lifecycle and transfer a counter"
    input_schema = _Input
    output_schema = _Output
    event_log: Path

    def __init__(self, label: str = "") -> None:
        self.label = label
        self.count = 0
        self.entered = asyncio.Event()
        self.release = asyncio.Event()
        self.closed = False

    def record(self, event: str) -> None:
        with self.event_log.open("a", encoding="utf-8") as log:
            if self.label:
                event = f"{event}:{self.label}"
            log.write(f"{event}\n")

    def on_load(self) -> None:
        if self.label:
            self.record("load")

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, int]:
        if inputs.get("hold"):
            self.entered.set()
            await self.release.wait()
            assert not self.closed, "module resources closed during its call"
            self.count += 1
            self.record("finish")
        return {"count": self.count}

    def on_suspend(self) -> dict[str, int]:
        self.record("suspend")
        return {"count": self.count}

    def on_unload(self) -> None:
        self.closed = True
        self.record("unload")

    def on_resume(self, state: dict[str, int]) -> None:
        self.count = state["count"]
        self.record("resume")


async def _await_admission_closed(client: APCore, module_id: str) -> str:
    async with asyncio.timeout(2):
        while True:
            try:
                await client.call_async(module_id, {})
            except ModuleNotFoundError as error:
                return error.code
            await asyncio.sleep(0.001)


async def _batch(client: APCore, module_id: str, other_id: str, log: Path) -> dict[str, Any]:
    modules = [Counter() for _ in range(3)]
    for module in modules:
        module.event_log = log
    with pytest.raises(ModuleError) as failure:
        client.register_many(zip((module_id, module_id, other_id), modules, strict=True))
    return {
        "batch_error": failure.value.code,
        "registered": client.list_modules(),
        "removed_items": await client.unregister_many([module_id, other_id, module_id]),
    }


async def _batch_drain(client: APCore, module_id: str, other_id: str, log: Path) -> dict[str, Any]:
    first = Counter("first")
    second = Counter("second")
    first.event_log = second.event_log = log
    client.register_many([(module_id, first), (other_id, second)])
    active = asyncio.create_task(client.call_async(module_id, {"hold": True}))
    removal: asyncio.Task[list[bool]] | None = None
    try:
        await asyncio.wait_for(first.entered.wait(), timeout=2)
        removal = asyncio.create_task(client.unregister_many([module_id, other_id]))
        new_call_error = await _await_admission_closed(client, module_id)
        other_output = await client.call_async(other_id, {})
        assert not removal.done()
        active_output = await _finish(active, first)
        return {
            "new_call_error": new_call_error,
            "other_output_during_drain": other_output,
            "active_output": active_output,
            "removed_items": await asyncio.wait_for(removal, timeout=2),
        }
    finally:
        first.release.set()
        await asyncio.gather(*(task for task in (active, removal) if task is not None), return_exceptions=True)


async def _exercise(client: APCore, counter: Counter, module_id: str, operation: str) -> dict[str, Any]:
    active = asyncio.create_task(client.call_async(module_id, {"hold": True}))
    lifecycle: asyncio.Task[Any] | None = None
    try:
        await asyncio.wait_for(counter.entered.wait(), timeout=2)
        if operation == "unregister":
            removed = client.registry.unregister(module_id)
            return {
                "removed": removed,
                "new_call_error": await _await_admission_closed(client, module_id),
                "active_output": await _finish(active, counter),
            }
        if operation == "shutdown":
            lifecycle = asyncio.create_task(client.shutdown())
            error = await _await_admission_closed(client, module_id)
            assert not lifecycle.done()
            active_output = await _finish(active, counter)
            await asyncio.wait_for(lifecycle, timeout=2)
            await client.shutdown()
            return {"active_output": active_output, "new_call_error": error}
        assert operation in {"reload", "reload_bulk"}
        selector = {"module_id": module_id} if operation == "reload" else {"path_filter": module_id}
        lifecycle = asyncio.create_task(
            client.call_async("system.control.reload_module", {**selector, "reason": "conformance"})
        )
        await _await_admission_closed(client, module_id)
        active_output = await _finish(active, counter)
        result = await asyncio.wait_for(lifecycle, timeout=2)
        return {
            "active_output": active_output,
            "reload_succeeded": result["success"],
            "next_output": await client.call_async(module_id, {}),
        }
    finally:
        counter.release.set()
        pending = [task for task in (active, lifecycle) if task is not None]
        await asyncio.gather(*pending, return_exceptions=True)


async def _finish(active: asyncio.Task[dict[str, Any]], counter: Counter) -> dict[str, Any]:
    counter.release.set()
    return await asyncio.wait_for(active, timeout=2)


async def _exercise_control_guard(case: dict[str, Any], log: Path) -> dict[str, Any]:
    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    client.executor.set_approval_handler(AutoApproveHandler())
    probe = Counter()
    probe.event_log = log
    client.register("aaa.counter", probe)
    client.registry.on("unregister", lambda module_id, module: probe.record(f"unregister:{module_id}"))
    registered = client.list_modules()
    module_id = case["module_id"]
    if case["operation"] == "toggle_self":
        inputs = {"module_id": module_id, "enabled": False}
        followup = {**inputs, "enabled": True}
    else:
        inputs = case["selector"]
        followup = {"path_filter": "missing.module"}
    try:
        with pytest.raises(ModuleError) as failure:
            await client.call_async(module_id, {**inputs, "reason": "conformance"})
        preserved = client.list_modules() == registered
        result = await client.call_async(module_id, {**followup, "reason": "conformance"})
        return {
            "control_error": failure.value.code,
            "registrations_preserved": preserved,
            "control_usable": result["success"],
            "events": log.read_text(encoding="utf-8").splitlines() if log.exists() else [],
        }
    finally:
        await client.shutdown()


async def _managed_shutdown(case: dict[str, Any], log: Path) -> dict[str, Any]:
    from apcore import ShutdownOptions

    client = APCore()
    counter = Counter()
    counter.event_log = log
    module_id = case["module_id"]
    client.register(module_id, counter)
    options = ShutdownOptions(drain_timeout_ms=case["drain_timeout_ms"])
    if case["operation"] == "managed_shutdown_shared":
        peer = APCore(registry=client.registry)
        operation = client.start_shutdown(options=options)
        joined = peer.start_shutdown()
        with pytest.raises(ModuleError) as conflict:
            peer.start_shutdown(options=ShutdownOptions(drain_timeout_ms=100))
        final = await asyncio.wait_for(joined.wait(), 2)
        assert await operation.wait() == final
        return {
            "same_operation": operation.snapshot().operation_id == final.operation_id,
            "conflict_error": conflict.value.code,
            "final_status": final.status,
            "outcomes": [row.outcome for row in final.modules],
        }

    active = asyncio.create_task(client.call_async(module_id, {"hold": True}))
    try:
        await asyncio.wait_for(counter.entered.wait(), timeout=2)
        operation = client.start_shutdown(options=options)
        pending = await operation.wait(wait_timeout_ms=case["wait_timeout_ms"])
        async with asyncio.timeout(2):
            while not any(row.drain_timed_out for row in operation.snapshot().modules):
                await asyncio.sleep(0.001)
        error = await _await_admission_closed(client, module_id)
        active_output = await _finish(active, counter)
        final = await asyncio.wait_for(client.start_shutdown().wait(), 2)
        return {
            "wait_status": pending.status,
            "drain_timed_out": final.modules[0].drain_timed_out,
            "new_call_error": error,
            "active_output": active_output,
            "same_operation": pending.operation_id == final.operation_id,
            "final_status": final.status,
            "outcomes": [row.outcome for row in final.modules],
        }
    finally:
        counter.release.set()
        await asyncio.gather(active, return_exceptions=True)


async def _managed_batch(case: dict[str, Any], log: Path) -> dict[str, Any]:
    from apcore import UnregisterOptions

    client = APCore()
    first, second = Counter("first"), Counter("second")
    first.event_log = second.event_log = log
    module_id, other_id = case["module_id"], case["other_module_id"]
    client.register_many([(module_id, first), (other_id, second)])
    active = asyncio.create_task(client.call_async(module_id, {"hold": True}))
    try:
        await asyncio.wait_for(first.entered.wait(), 2)
        operation = client.start_unregister_many(
            [module_id, other_id], options=UnregisterOptions(operation_timeout_ms=case.get("operation_timeout_ms"))
        )
        await _await_admission_closed(client, module_id)
        if case["operation"] == "managed_batch_budget":
            pending = await operation.wait(wait_timeout_ms=1)
            other_output = await client.call_async(other_id, {})
            async with asyncio.timeout(2):
                while not operation.snapshot().modules[0].drain_timed_out:
                    await asyncio.sleep(0.001)
            active_output = await _finish(active, first)
            final = await asyncio.wait_for(operation.wait(), 2)
            return {
                "wait_status": pending.status,
                "drain_timed_out": final.modules[0].drain_timed_out,
                "other_output_during_drain": other_output,
                "active_output": active_output,
                "final_status": final.status,
                "stop_reason": final.stop_reason,
                "outcomes": [row.outcome for row in final.modules],
                "other_output_after": await client.call_async(other_id, {}),
            }

        assert client.registry.unregister(other_id)
        replacement = Counter("replacement")
        replacement.event_log = log
        client.register(other_id, replacement)
        await _finish(active, first)
        final = await asyncio.wait_for(operation.wait(), 2)
        return {
            "final_status": final.status,
            "outcomes": [row.outcome for row in final.modules],
            "replacement_output": await client.call_async(other_id, {}),
        }
    finally:
        first.release.set()
        await asyncio.gather(active, return_exceptions=True)


async def _phase_b_reload(case: dict[str, Any], log: Path) -> dict[str, Any]:
    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    client.executor.set_approval_handler(AutoApproveHandler())
    target = case["module_id"]
    mode = case["operation"]
    recover_entered, recover_release = asyncio.Event(), asyncio.Event()
    load_entered, load_release = asyncio.Event(), asyncio.Event()

    class OldModule(Counter):
        async def on_drain(self, context: Any) -> None:
            self.record("drain")
            if mode in {"managed_reload_drain", "managed_reload_stream"}:
                self.release.set()
            elif mode == "managed_reload_hook_failure":
                raise RuntimeError("drain failed")

        async def on_drain_aborted(self, context: Any) -> None:
            self.record("drain_aborted")
            recover_entered.set()
            if mode == "managed_reload_abort":
                await recover_release.wait()

    class Replacement(Counter):
        async def on_load(self) -> None:
            self.record("load")
            if mode == "managed_reload_replacement_failure":
                load_entered.set()
                await load_release.wait()
                raise ModuleLoadError(target, "replacement refused initialization")

    class StreamingModule(OldModule):
        annotations = ModuleAnnotations(streaming=True)

        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, int]]:
            self.entered.set()
            await self.release.wait()
            assert not self.closed, "the stream lost its resources before finishing"
            self.count += 1
            yield {"count": self.count}
            self.record("stream_finished")

    old = StreamingModule() if mode == "managed_reload_stream" else OldModule()
    replacement = Replacement()
    old.event_log = replacement.event_log = log

    class Discover:
        def discover(self, roots: list[str]) -> list[dict[str, Any]]:
            return [{"module_id": target, "module": replacement}]

    client.registry.set_discoverer(Discover())
    active = None
    consuming = None
    reloading = None

    async def consume() -> list[dict[str, Any]]:
        return [chunk async for chunk in client.stream(target, {})]

    try:
        if mode != "managed_reload_no_match":
            await client.register_async(target, old)
        if mode in {"managed_reload_drain", "managed_reload_abort"}:
            active = asyncio.create_task(client.call_async(target, {"hold": True}))
            await asyncio.wait_for(old.entered.wait(), 2)
        elif mode == "managed_reload_stream":
            consuming = asyncio.create_task(consume())
            await asyncio.wait_for(old.entered.wait(), 2)
        selection = {"path_filter": case["path_filter"]} if "path_filter" in case else {"module_id": target}
        lifecycle = {"drain_timeout_ms": case["drain_timeout_ms"]} if "drain_timeout_ms" in case else {}
        reloading = asyncio.create_task(
            client.call_async(
                "system.control.reload_module", {**selection, "reason": "conformance", "lifecycle": lifecycle}
            )
        )
        actual: dict[str, Any] = {}
        if mode in {"managed_reload_abort", "managed_reload_replacement_failure"}:
            entered = recover_entered if mode == "managed_reload_abort" else load_entered
            await asyncio.wait_for(entered.wait(), 2)
            actual["new_call_error"] = await _await_admission_closed(client, target)
            recover_release.set()
            load_release.set()
        result = await asyncio.wait_for(reloading, 2)
        schema = json.loads((schemas_dir() / "sys-control-reload-module.schema.json").read_text(encoding="utf-8"))
        Draft202012Validator(schema).validate(result)
        report = result["lifecycle_report"]
        actual.update(
            reload_succeeded=result["success"],
            report_status=report["status"],
            outcomes=[row["outcome"] for row in report["modules"]],
        )
        if mode == "managed_reload_no_match":
            return {
                **actual,
                "report_kind": report["kind"],
                "event_flush": report["event_flush"],
                "operation_id_present": bool(report["operation_id"]),
                "reloaded_modules": result["reloaded_modules"],
            }
        if active is not None:
            actual["active_output"] = await _finish(active, old)
        if consuming is not None:
            actual["chunks"] = await asyncio.wait_for(consuming, 2)
        actual["next_output"] = await client.call_async(target, {})
        row = report["modules"][0]
        if mode in {"managed_reload_abort", "managed_reload_hook_failure"}:
            actual["abort_reason"] = row["abort_reason"]
        if mode == "managed_reload_replacement_failure":
            actual["compensation"] = row["compensation"]
            actual["primary_error"] = row["error"]["code"]
        if mode != "managed_reload_replacement_failure":
            actual["events"] = log.read_text(encoding="utf-8").splitlines()
        return actual
    finally:
        recover_release.set()
        load_release.set()
        old.release.set()
        await asyncio.gather(
            *(task for task in (active, consuming, reloading) if task is not None), return_exceptions=True
        )
        await client.shutdown()


async def _phase_b_reload_denied(case: dict[str, Any], log: Path) -> dict[str, Any]:
    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    target = case["module_id"]
    events: list[ApCoreEvent] = []

    async def deny(request: ApprovalRequest) -> ApprovalResult:
        return ApprovalResult(status="rejected", reason="conformance")

    class StatefulModule(Counter):
        def on_drain(self, context: Any) -> None:
            self.record("drain")

        def on_drain_aborted(self, context: Any) -> None:
            self.record("drain_aborted")

    module = StatefulModule()
    module.event_log = log
    module.count = 1
    log.write_text("", encoding="utf-8")
    client.register(target, module)
    client.executor.set_approval_handler(CallbackApprovalHandler(deny))
    for suffix in ("module_drain_started", "module_drain_timed_out", "module_drain_recovered", "module_reload_failed"):
        client.on(f"apcore.registry.{suffix}", events.append)
    try:
        with pytest.raises(ModuleError) as failure:
            await client.call_async(
                "system.control.reload_module",
                {
                    "module_id": target,
                    "reason": "conformance",
                    "lifecycle": {"drain_timeout_ms": case["drain_timeout_ms"]},
                },
            )
        next_output = await client.call_async(target, {})
        assert client.registry.get(target) is module
        assert client.events is not None
        await asyncio.to_thread(client.events.flush)
        return {
            "control_error": failure.value.code,
            "next_output": next_output,
            "lifecycle_event_count": len(events),
            "events": log.read_text(encoding="utf-8").splitlines(),
        }
    finally:
        await client.shutdown()


async def _phase_b_force(case: dict[str, Any], log: Path) -> dict[str, Any]:
    client = APCore()
    target_id = case["module_id"]
    sibling_id = case["other_module_id"]
    parent = CancelToken()
    context = Context.create(cancel_token=parent)

    class Cancellable(Counter):
        async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, int]:
            self.entered.set()
            assert context.cancel_token is not None
            while not self.release.is_set():
                context.cancel_token.check()
                await asyncio.sleep(0.001)
            return {"count": 0}

    target, sibling = Cancellable(), Counter()
    target.event_log = sibling.event_log = log
    client.register_many([(target_id, target), (sibling_id, sibling)])
    target_call = asyncio.create_task(client.call_async(target_id, {"hold": True}, context))
    other = asyncio.create_task(client.call_async(sibling_id, {"hold": True}, context))
    try:
        await asyncio.wait_for(asyncio.gather(target.entered.wait(), sibling.entered.wait()), 2)
        operation = client.start_unregister_many(
            [target_id],
            options=UnregisterOptions(
                drain_timeout_ms=case["drain_timeout_ms"],
                on_drain_timeout="cancel_then_force",
                cancel_grace_ms=case["cancel_grace_ms"],
            ),
        )
        with pytest.raises(ModuleError) as failure:
            await asyncio.wait_for(asyncio.shield(target_call), 2)
        final = await asyncio.wait_for(operation.wait(), 2)
        return {
            "target_cancel_error": failure.value.code,
            "parent_cancelled": parent.is_cancelled,
            "other_output_after": await _finish(other, sibling),
            "final_status": final.status,
            "unload_count": log.read_text(encoding="utf-8").splitlines().count("unload"),
        }
    finally:
        target.release.set()
        sibling.release.set()
        await asyncio.gather(target_call, other, return_exceptions=True)
        await client.shutdown()


async def _phase_b_shutdown_event(case: dict[str, Any], log: Path) -> dict[str, Any]:
    client = APCore(config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}))
    counter = Counter()
    counter.event_log = log
    events: list[ApCoreEvent] = []
    client.on("apcore.client.shutdown_resources_completed", events.append)
    client.register(case["module_id"], counter)
    operation = client.start_shutdown(options=ShutdownOptions())
    final = await asyncio.wait_for(operation.wait(), 2)
    assert (await client.start_shutdown().wait()) == final
    return {
        "final_status": final.status,
        "event_flush": final.event_flush,
        "event_count": len(events),
        "event_matches_operation": bool(events) and events[0].data["operation_id"] == final.operation_id,
        "events": log.read_text(encoding="utf-8").splitlines(),
    }


@pytest.mark.parametrize("case", CASES, ids=lambda case: case["id"])
async def test_module_lifecycle(case: dict[str, Any], tmp_path: Path) -> None:
    module_id = case["module_id"]
    operation = case["operation"]
    log = tmp_path / "events.log"
    if operation == "managed_reload_denied":
        assert await _phase_b_reload_denied(case, log) == case["expected"]
        return
    if operation.startswith("managed_reload_"):
        assert await _phase_b_reload(case, log) == case["expected"]
        return
    if operation == "managed_force_scope":
        assert await _phase_b_force(case, log) == case["expected"]
        return
    if operation == "managed_shutdown_event":
        assert await _phase_b_shutdown_event(case, log) == case["expected"]
        return
    if operation in {"toggle_self", "reload_self"}:
        assert await _exercise_control_guard(case, log) == case["expected"]
        return
    if operation in {"managed_shutdown_waiter", "managed_shutdown_shared"}:
        actual = await _managed_shutdown(case, log)
    elif operation in {"managed_batch_budget", "managed_batch_stale_generation"}:
        actual = await _managed_batch(case, log)
    elif operation == "batch":
        actual = await _batch(APCore(), module_id, case["other_module_id"], log)
    elif operation == "batch_drain":
        actual = await _batch_drain(APCore(), module_id, case["other_module_id"], log)
    elif operation in {"reload", "reload_bulk"}:
        source = tmp_path.joinpath(*module_id.split(".")).with_suffix(".py")
        source.parent.mkdir(parents=True)
        source.write_text(
            f"from pathlib import Path\nfrom {__name__} import Counter as BaseCounter\n"
            f"class Counter(BaseCounter):\n    event_log = Path({str(log)!r})\n",
            encoding="utf-8",
        )
        client = APCore(
            registry=Registry(extensions_dir=tmp_path),
            config=Config({"sys_modules": {"enabled": True, "events": {"enabled": True}}}),
        )
        client.executor.set_approval_handler(AutoApproveHandler())
        try:
            assert client.discover() == 1
            actual = await _exercise(client, client.registry.get(module_id), module_id, operation)
        finally:
            client.close()
    else:
        counter = Counter()
        counter.event_log = log
        client = APCore()
        client.register(module_id, counter)
        actual = await _exercise(client, counter, module_id, operation)
    assert {**actual, "events": log.read_text(encoding="utf-8").splitlines()} == case["expected"]
