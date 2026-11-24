"""Canonical issue #123 drivers: D-133/134/136/139/140/141/146/148/149.

Every operation uses the fixture's public-path driver contract. Expected keys
are dispatched explicitly so a new assertion can never become a silent skip.
"""

from __future__ import annotations

import asyncio
import copy
import json
import os
import time
from dataclasses import asdict
from datetime import datetime
from enum import Enum
from pathlib import Path
from typing import Any
from uuid import UUID

import pytest
import yaml
from pydantic import BaseModel

import apcore.config as config_module
from apcore import APCore, Registry
from apcore.acl import ACL, ACLRule
from apcore.bindings import BindingLoader
from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.config import Config
from apcore.context import Context, Identity
from apcore.errors import ModuleError
from apcore.executor import Executor
from apcore.module import Change, ModuleAnnotations, PreviewResult
from apcore.schema.exporter import SchemaExporter
from apcore.schema.loader import SchemaLoader
from apcore.schema.types import ExportProfile, SchemaDefinition

from .canonical_fixtures import load_fixture

BINDING = load_fixture("binding_file_validation.json")
ENV = load_fixture("env_prefix_dispatch.json")
EPHEMERAL = load_fixture("ephemeral_modules.json")
ERROR = load_fixture("error_details_shape.json")
EXPORT = load_fixture("export_profiles.json")
NATIVE = load_fixture("json_input_native_types.json")
PREFLIGHT = load_fixture("preflight_check_reporting.json")
TIMEOUT = load_fixture("timeout_cancellation.json")


def _expect_keys(case: dict[str, Any], allowed: set[str]) -> dict[str, Any]:
    expected = case["expected"]
    assert not set(expected) - allowed, f"Unhandled expectations in {case['id']}: {set(expected) - allowed}"
    return expected


def _module(contract: dict[str, Any]) -> Any:
    loader = SchemaLoader(Config({}))

    class Probe:
        input_schema = loader.generate_model(contract["input_schema"], "Issue123Input")
        output_schema = loader.generate_model(contract["output_schema"], "Issue123Output")
        description = contract.get("description", "Issue 123 conformance probe")

        def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            return {"ok": True}

    return Probe()


def typed_greet(name: str) -> dict[str, str]:
    return {"greeting": f"Hello {name}"}


def untyped_greet(name):  # type: ignore[no-untyped-def]
    return {"greeting": f"Hello {name}"}


@pytest.mark.parametrize("case", BINDING["test_cases"], ids=lambda case: case["id"])
def test_binding_file_validation(case: dict[str, Any], tmp_path: Path) -> None:
    """D-139, protocol §5.12: load real YAML, resolve targets, infer and register."""
    expected = _expect_keys(case, {"error_code", "module_ids"})
    document = copy.deepcopy(case["input"]["file"])
    for entry in document["bindings"]:
        entry["target"] = entry["target"].replace("fixture_targets:", f"{__name__}:")
    path = tmp_path / "bindings.yaml"
    path.write_text(yaml.safe_dump(document), encoding="utf-8")
    registry = Registry()
    if "error_code" in expected:
        with pytest.raises(ModuleError) as caught:
            BindingLoader().load_bindings(str(path), registry)
        assert caught.value.code == expected["error_code"]
    else:
        BindingLoader().load_bindings(str(path), registry)
        assert sorted(registry.module_ids) == expected["module_ids"]


@pytest.mark.parametrize("case", ENV["test_cases"], ids=lambda case: case["id"])
def test_env_prefix_dispatch(case: dict[str, Any], tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """D-146, protocol §9.8.2: exact reservation and longest-prefix routing."""
    expected = _expect_keys(case, {"register_error_code", "values", "absent"})
    monkeypatch.setattr(config_module, "_GLOBAL_NS_REGISTRY", {})
    monkeypatch.setattr(config_module, "_GLOBAL_ENV_MAP", {})
    monkeypatch.setattr(config_module, "_GLOBAL_ENV_MAP_CLAIMED", {})
    for name in os.environ:
        if name.startswith("APCORE"):
            monkeypatch.delenv(name)
    spec = case["input"]
    for index, namespace in enumerate(spec["namespaces"]):
        if "register_error_code" in expected and index == len(spec["namespaces"]) - 1:
            with pytest.raises(ModuleError) as caught:
                Config.register_namespace(**namespace)
            assert caught.value.code == expected["register_error_code"]
            return
        Config.register_namespace(**namespace)
    for name, value in spec["env"].items():
        monkeypatch.setenv(name, value)
    path = tmp_path / "apcore.yaml"
    path.write_text(yaml.safe_dump(spec["config_file"]), encoding="utf-8")
    config = Config.load(str(path))
    for name, value in expected.get("values", {}).items():
        assert config.get(name) == value
    for name in expected.get("absent", []):
        assert config.get(name) is None


class Recorder:
    def __init__(self) -> None:
        self.events: list[Any] = []

    async def on_event(self, event: Any) -> None:
        self.events.append(event)


_DISCOVERY_SOURCE = """from pydantic import BaseModel
class Input(BaseModel):
    pass
class Output(BaseModel):
    ok: bool
class Probe:
    input_schema = Input
    output_schema = Output
    description = "Ephemeral discovery probe"
    def execute(self, inputs, context):
        return {"ok": True}
"""


@pytest.mark.parametrize("case", EPHEMERAL["test_cases"], ids=lambda case: case["id"])
def test_ephemeral_modules(case: dict[str, Any], tmp_path: Path) -> None:
    """D-148, protocol §2.5.1: standard bootstrap, real registry, audit payload."""
    expected = _expect_keys(case, {"error_code", "events", "secret_absent"})
    client = APCore(
        config=Config(
            {"sys_modules": {"enabled": True, "events": {"enabled": True}}, "extensions": {"root": str(tmp_path)}}
        )
    )
    recorder = Recorder()
    assert client.events is not None
    client.events.subscribe(recorder)
    error: ModuleError | None = None
    try:
        for operation in case["input"]["operations"]:
            context = None
            if "context" in operation:
                context = Context.create(identity=Identity(**operation["context"]["identity"]))
                context.caller_id = operation["context"]["caller_id"]
            try:
                if operation["op"] == "register":
                    client.registry.register(operation["module_id"], _module(ERROR["module_contract"]), context=context)
                elif operation["op"] == "unregister":
                    client.registry.unregister(operation["module_id"], context=context)
                elif operation["op"] == "register_internal":
                    client.registry.register_internal(operation["module_id"], _module(ERROR["module_contract"]))
                elif operation["op"] == "discover":
                    path = tmp_path / (operation["file"] + ".py")
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(_DISCOVERY_SOURCE, encoding="utf-8")
                    client.registry.discover()
                else:
                    pytest.fail(f"Unhandled operation: {operation['op']}")
            except ModuleError as caught:
                error = caught
                break
        if "error_code" in expected:
            assert error is not None and error.code == expected["error_code"]
        else:
            assert error is None
        client.events.flush()
        events = [
            event
            for event in recorder.events
            if event.event_type in {"apcore.registry.module_registered", "apcore.registry.module_unregistered"}
            and not (event.module_id or "").startswith("system.")
        ]
        assert len(events) == len(expected["events"])
        for actual, want in zip(events, expected["events"], strict=True):
            assert actual.event_type == want["event_type"]
            assert actual.module_id == want["module_id"]
            for name, value in want.get("payload", {}).items():
                assert name in actual.data and actual.data[name] == value
            for name in want.get("payload_absent_keys", []):
                assert name not in actual.data
            if "identity_id" in want:
                assert actual.data["identity"]["id"] == want["identity_id"]
        if "secret_absent" in expected:
            assert expected["secret_absent"] not in json.dumps([event.data for event in events])
    finally:
        client.close()


@pytest.mark.parametrize("case", ERROR["test_cases"], ids=lambda case: case["id"])
def test_error_details_shape(case: dict[str, Any]) -> None:
    """D-149, protocol §4.14: canonical serialization from the public call path."""
    expected = _expect_keys(case, {"error_code", "errors", "error_count", "detail_keys_present", "detail_keys_absent"})
    contract = ERROR["module_contract"]
    registry = Registry()
    registry.register(contract["module_id"], _module(contract))
    executor = Executor(registry)
    with pytest.raises(ModuleError) as caught:
        executor.call(case["input"].get("call_module_id", contract["module_id"]), case["input"]["inputs"])
    serialized = caught.value.to_dict()
    assert serialized["code"] == expected["error_code"]
    details = serialized["details"]
    if "errors" in expected:
        errors = details["errors"]
        for item in errors:
            assert set(item) == {"path", "keyword", "message"}
            assert isinstance(item["message"], str) and item["message"]
        for want in expected["errors"]:
            assert any({"path": item["path"], "keyword": item["keyword"]} == want for item in errors)
    if "error_count" in expected:
        assert len(details["errors"]) == expected["error_count"]
    for name in expected.get("detail_keys_present", []):
        assert name in details
    for name in expected.get("detail_keys_absent", []):
        assert name not in details


def _pointer(document: Any, pointer: str) -> Any:
    value = document
    for segment in pointer.split("/")[1:]:
        key = segment.replace("~1", "/").replace("~0", "~")
        value = value[int(key)] if isinstance(value, list) else value[key]
    return value


def _assert_no_x_keywords(value: Any, property_names: bool = False) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            assert property_names or not key.startswith("x-"), f"Extension keyword survived: {key}"
            _assert_no_x_keywords(child, key == "properties" and not property_names)
    elif isinstance(value, list):
        for child in value:
            _assert_no_x_keywords(child)


@pytest.mark.parametrize("case", EXPORT["test_cases"], ids=lambda case: case["id"])
def test_export_profiles(case: dict[str, Any]) -> None:
    """D-140, protocol §4.17: real profile exporter and keyword/property distinction."""
    expected = _expect_keys(case, {"paths", "absent_paths", "no_x_keywords_under"})
    contract = EXPORT["module_contract"]
    registry = Registry()
    module = _module(contract)
    module.annotations = ModuleAnnotations(**case["input"]["annotations"])
    if module.annotations.streaming:

        async def stream(inputs: dict[str, Any], context: Context) -> Any:
            yield {"sent": True}

        module.stream = stream
    registry.register(contract["module_id"], module)
    definition = SchemaDefinition(**contract)
    result = SchemaExporter().export(definition, ExportProfile(case["input"]["profile"]), module.annotations)
    for pointer, value in expected["paths"].items():
        assert _pointer(result, pointer) == value
    for pointer in expected.get("absent_paths", []):
        with pytest.raises((KeyError, IndexError)):
            _pointer(result, pointer)
    if "no_x_keywords_under" in expected:
        _assert_no_x_keywords(_pointer(result, expected["no_x_keywords_under"]))


class Level(str, Enum):
    LOW = "low"
    HIGH = "high"


class NativeInput(BaseModel):
    when: datetime
    request_id: UUID
    level: Level


class NativeOutput(BaseModel):
    accepted: bool


class NativeModule:
    input_schema = NativeInput
    output_schema = NativeOutput
    description = "Native JSON input conformance probe"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, bool]:
        return dict(NATIVE["module_contract"]["returns"])


@pytest.mark.parametrize("case", NATIVE["test_cases"], ids=lambda case: case["id"])
def test_json_input_native_types(case: dict[str, Any]) -> None:
    """D-136, protocol §4.2 R5: native Pydantic datetime, UUID and Enum fields."""
    expected = _expect_keys(case, {"output", "error_code"})
    registry = Registry()
    module_id = NATIVE["module_contract"]["module_id"]
    registry.register(module_id, NativeModule())
    executor = Executor(registry)
    inputs = json.loads(json.dumps(case["input"]["inputs"]))
    if "error_code" in expected:
        with pytest.raises(ModuleError) as caught:
            executor.call(module_id, inputs)
        assert caught.value.code == expected["error_code"]
    else:
        assert executor.call(module_id, inputs) == expected["output"]


@pytest.mark.parametrize("case", PREFLIGHT["test_cases"], ids=lambda case: case["id"])
def test_preflight_check_reporting(case: dict[str, Any]) -> None:
    """D-134/141, protocol §12.8.5.1: real validate, ACL, hooks and serialization."""
    expected = _expect_keys(
        case,
        {
            "valid",
            "failed_checks",
            "passed_checks",
            "checks_present",
            "checks_absent",
            "optional_passed_checks",
            "predicted_changes_count",
            "predicted_changes_present",
        },
    )
    spec = case["input"]
    registry = Registry()
    module = _module(PREFLIGHT["module_contract"])
    invoked: list[str] = []
    if spec.get("implements_preflight"):

        def preflight(inputs: dict[str, Any], context: Context) -> list[str]:
            invoked.append("module_preflight")
            return list(PREFLIGHT["module_contract"]["preflight_returns"])

        module.preflight = preflight
    if spec.get("implements_preview"):

        def preview(inputs: dict[str, Any], context: Context) -> PreviewResult | None:
            invoked.append("module_preview")
            return (
                None
                if spec.get("preview_returns_null")
                else PreviewResult(changes=[Change(**PREFLIGHT["module_contract"]["preview_change"])])
            )

        module.preview = preview
    if spec.get("register", True):
        registry.register(spec["module_id"], module)
    acl = ACL(rules=[ACLRule(**rule) for rule in spec["acl_rules"]], default_effect=spec["default_effect"])
    executor = Executor(registry, acl=acl)
    context = Context.create()
    context.caller_id = spec["caller_id"]
    result = executor.validate(spec["module_id"], spec["inputs"], context)
    checks = {check.check: check for check in result.checks}
    assert result.valid == expected["valid"]
    assert sorted(check.check for check in result.checks if not check.passed) == sorted(expected["failed_checks"])
    for name in expected.get("passed_checks", []):
        assert name in checks and checks[name].passed
    for name in expected.get("checks_present", []):
        assert name in checks
    for name in expected.get("checks_absent", []):
        assert name not in checks
    for name in expected.get("optional_passed_checks", []):
        assert name not in checks or (checks[name].passed and not checks[name].warnings)
    serialized = asdict(result)
    if "predicted_changes_present" in expected:
        assert ("predicted_changes" in serialized and isinstance(serialized["predicted_changes"], list)) == expected[
            "predicted_changes_present"
        ]
    if "predicted_changes_count" in expected:
        assert len(result.predicted_changes) == expected["predicted_changes_count"]
    if "acl" in expected["failed_checks"]:
        assert invoked == [], "ACL-denied validate must not invoke module hooks"


def _timed_module(module_id: str, spec: dict[str, Any], tokens: dict[str, CancelToken | None]) -> Any:
    class EmptyInput(BaseModel):
        pass

    class TimedOutput(BaseModel):
        slept: bool | None = None
        caught: str | None = None

    class TimedModule:
        input_schema = EmptyInput
        output_schema = TimedOutput
        description = "Timeout cancellation conformance probe"
        resources = {"timeout": spec["module_timeout_ms"]}

        async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
            tokens[module_id] = context.cancel_token
            if spec["kind"] == "caller":
                try:
                    return await context.executor.call_async(spec["calls"], {}, context)
                except ModuleError as error:
                    if error.code != spec["catches"]:
                        raise
                    return {"caught": error.code}
            assert spec["kind"] == "sleeper", f"Unhandled module kind: {spec['kind']}"
            remaining = spec["sleep_ms"]
            while remaining > 0:
                if spec["checks_token"] and context.cancel_token and context.cancel_token.is_cancelled:
                    raise ExecutionCancelledError()
                interval = min(10, remaining)
                await asyncio.sleep(interval / 1000)
                remaining -= interval
            return {"slept": True}

    return TimedModule()


@pytest.mark.asyncio
@pytest.mark.parametrize("case", TIMEOUT["test_cases"], ids=lambda case: case["id"])
async def test_timeout_cancellation(case: dict[str, Any]) -> None:
    """D-133, protocol §12.7.5: per-call child cancellation and no grace period."""
    expected = _expect_keys(case, {"error_code", "output", "token_cancelled", "returns_within_ms"})
    spec = case["input"]
    config = Config(spec.get("config", {}))
    registry = Registry(config)
    tokens: dict[str, CancelToken | None] = {}
    for module_id, module_spec in spec["modules"].items():
        registry.register(module_id, _timed_module(module_id, module_spec, tokens))
    acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")], default_effect="deny")
    executor = Executor(registry, acl=acl, config=config)
    application_token = CancelToken() if spec["application_token"] else None
    context = Context.create(cancel_token=application_token)
    cancellation: asyncio.Task[None] | None = None
    if "cancel_application_token_after_ms" in spec:

        async def cancel_application() -> None:
            await asyncio.sleep(spec["cancel_application_token_after_ms"] / 1000)
            assert application_token is not None
            application_token.cancel()

        cancellation = asyncio.create_task(cancel_application())
    started = time.monotonic()
    try:
        if "error_code" in expected:
            with pytest.raises(ModuleError) as caught:
                await executor.call_async(spec["call"], {}, context)
            assert caught.value.code == expected["error_code"]
        else:
            assert await executor.call_async(spec["call"], {}, context) == expected["output"]
        assert (time.monotonic() - started) * 1000 < expected["returns_within_ms"]
        tokens["application"] = application_token
        for module_id, cancelled in expected["token_cancelled"].items():
            token = tokens[module_id]
            assert bool(token and token.is_cancelled) == cancelled, module_id
    finally:
        if cancellation is not None:
            cancellation.cancel()
            await asyncio.gather(cancellation, return_exceptions=True)


@pytest.mark.asyncio
async def test_d133_timeout_does_not_cancel_or_wait_for_module_cleanup() -> None:
    """D-133: even a module suppressing task cancellation cannot delay timeout."""
    captured: dict[str, Any] = {}
    completed = asyncio.Event()
    module = _module(ERROR["module_contract"])
    module.resources = {"timeout": 20}

    async def execute(inputs: dict[str, Any], context: Context) -> dict[str, bool]:
        captured["token"] = context.cancel_token
        try:
            await asyncio.sleep(0.15)
        except asyncio.CancelledError:
            captured["forced_cancel"] = True
            await asyncio.sleep(0.15)
        completed.set()
        raise RuntimeError("Discarded late failure")

    module.execute = execute
    registry = Registry()
    registry.register("slow.cleanup", module)
    executor = Executor(registry)
    parent = CancelToken()
    started = time.monotonic()
    with pytest.raises(ModuleError, match="MODULE_TIMEOUT"):
        await executor.call_async("slow.cleanup", {"count": 1}, Context.create(cancel_token=parent))
    assert time.monotonic() - started < 0.1
    assert captured["token"].is_cancelled and not parent.is_cancelled
    assert not completed.is_set()
    await asyncio.wait_for(completed.wait(), timeout=1)
    await asyncio.sleep(0)
    assert "forced_cancel" not in captured


def test_d133_sync_timeout_returns_before_a_blocking_module_finishes() -> None:
    """D-133: the public sync entry point does not wait for worker completion."""
    import threading

    completed = threading.Event()
    captured: dict[str, Any] = {}
    module = _module(ERROR["module_contract"])
    module.resources = {"timeout": 20}

    def execute(inputs: dict[str, Any], context: Context) -> dict[str, bool]:
        captured["token"] = context.cancel_token
        time.sleep(0.15)
        completed.set()
        return {"ok": True}

    module.execute = execute
    registry = Registry()
    registry.register("slow.blocking", module)
    executor = Executor(registry)
    started = time.monotonic()
    with pytest.raises(ModuleError, match="MODULE_TIMEOUT"):
        executor.call("slow.blocking", {"count": 1})
    assert time.monotonic() - started < 0.1
    assert captured["token"].is_cancelled
    assert not completed.is_set()
    assert completed.wait(timeout=1)
    # Let the cached loop consume the discarded worker future before closing.
    assert executor._sync_loop is not None
    executor._sync_loop.run_until_complete(asyncio.sleep(0))
    executor.close()


def test_d133_parent_cancellation_and_child_isolation() -> None:
    """D-133: token links are directional and check() uses inherited state."""
    parent = CancelToken()
    child = parent.child()
    grandchild = child.child()
    sibling = parent.child()
    child.cancel()
    assert child.is_cancelled and grandchild.is_cancelled
    assert not parent.is_cancelled and not sibling.is_cancelled
    parent.cancel()
    assert sibling.is_cancelled
    with pytest.raises(ExecutionCancelledError):
        sibling.raise_if_cancelled()


@pytest.mark.asyncio
async def test_d133_sync_call_inside_async_loop_returns_before_cooperative_drain() -> None:
    """D-133: the sync bridge drains abandoned work without blocking its caller."""
    import threading

    completed = threading.Event()
    captured: dict[str, Any] = {}
    module = _module(ERROR["module_contract"])
    module.resources = {"timeout": 20}

    async def execute(inputs: dict[str, Any], context: Context) -> dict[str, bool]:
        captured["token"] = context.cancel_token
        await asyncio.sleep(0.15)
        completed.set()
        return {"ok": True}

    module.execute = execute
    registry = Registry()
    registry.register("slow.bridge", module)
    executor = Executor(registry)
    started = time.monotonic()
    with pytest.raises(ModuleError, match="MODULE_TIMEOUT"):
        executor.call("slow.bridge", {"count": 1})
    assert time.monotonic() - started < 0.1
    assert captured["token"].is_cancelled
    assert not completed.is_set()
    assert await asyncio.to_thread(completed.wait, 1)


def test_d136_native_python_inputs_remain_supported() -> None:
    """D-136 preserves already typed Python inputs while rejecting coercion."""
    registry = Registry()
    registry.register("demo.native", NativeModule())
    executor = Executor(registry)
    assert executor.call(
        "demo.native",
        {
            "when": datetime.fromisoformat("2026-01-01T09:30:00+00:00"),
            "request_id": UUID("123e4567-e89b-12d3-a456-426614174000"),
            "level": Level.HIGH,
        },
    ) == {"accepted": True}


def test_d149_json_pointer_escapes_property_names() -> None:
    """D-149: slashes and tildes are escaped as RFC 6901 segments."""
    contract = copy.deepcopy(ERROR["module_contract"])
    contract["input_schema"] = {"type": "object", "properties": {"a~/b": {"type": "integer"}}}
    registry = Registry()
    registry.register("demo.pointer", _module(contract))
    with pytest.raises(ModuleError) as caught:
        Executor(registry).call("demo.pointer", {"a~/b": "invalid"})
    assert caught.value.to_dict()["details"]["errors"] == [
        {"path": "/a~0~1b", "keyword": "type", "message": "Input should be a valid integer"}
    ]


@pytest.mark.parametrize("case", ERROR["test_cases"], ids=lambda case: case["id"])
def test_d149_raw_json_schema_errors_share_the_native_wire_shape(case: dict[str, Any]) -> None:
    """D-149 covers raw JSON Schema declarations as well as native models."""
    contract = ERROR["module_contract"]
    module = _module(contract)
    module.input_schema = copy.deepcopy(contract["input_schema"])
    registry = Registry()
    registry.register(contract["module_id"], module)
    with pytest.raises(ModuleError) as caught:
        Executor(registry).call(case["input"].get("call_module_id", contract["module_id"]), case["input"]["inputs"])
    serialized = caught.value.to_dict()
    assert serialized["code"] == case["expected"]["error_code"]
    json.dumps(serialized)
    if "errors" in case["expected"]:
        errors = serialized["details"]["errors"]
        assert len(errors) == case["expected"]["error_count"]
        for item in errors:
            assert set(item) == {"path", "keyword", "message"}
            assert item["message"]
        assert sorted((item["path"], item["keyword"]) for item in errors) == sorted(
            (item["path"], item["keyword"]) for item in case["expected"]["errors"]
        )


def test_subscriber_circuit_breaker_configuration_reaches_standard_bootstrap() -> None:
    """Issue #123 and protocol event system: honor every circuit_breaker setting."""
    from apcore.events.emitter import ApCoreEvent
    from apcore.sys_modules.registration import register_subscriber_type, unregister_subscriber_type

    class SlowSubscriber:
        subscriber_id = "issue123-configured"
        event_pattern = "probe.run"

        def __init__(self) -> None:
            self.calls = 0
            self.slow = True

        async def on_event(self, event: ApCoreEvent) -> None:
            self.calls += 1
            if self.slow:
                await asyncio.sleep(0.3)

    subscriber = SlowSubscriber()
    register_subscriber_type("issue123-probe", lambda config: subscriber)
    client = APCore(
        config=Config(
            {
                "sys_modules": {
                    "enabled": True,
                    "events": {
                        "enabled": True,
                        "subscribers": [
                            {
                                "type": "issue123-probe",
                                "circuit_breaker": {
                                    "timeout_ms": 10,
                                    "open_threshold": 1,
                                    "recovery_window_ms": 30,
                                },
                            }
                        ],
                    },
                }
            }
        )
    )
    try:
        assert client.events is not None
        recorder = Recorder()
        client.events.subscribe(recorder)
        event = ApCoreEvent("probe.run", "demo.probe", "2026-01-01T00:00:00Z", "info", {})
        started = time.monotonic()
        client.events.emit(event)
        client.events.flush()
        assert time.monotonic() - started < 0.2
        client.events.flush()
        opened = [item for item in recorder.events if item.event_type == "apcore.subscriber.circuit_opened"]
        assert len(opened) == 1
        assert opened[0].data["subscriber_id"] == subscriber.subscriber_id
        assert opened[0].data["consecutive_failures"] == 1
        client.events.emit(event)
        client.events.flush()
        assert subscriber.calls == 1
        subscriber.slow = False
        time.sleep(0.05)
        client.events.emit(event)
        client.events.flush()
        client.events.flush()
        assert subscriber.calls == 2
        assert len([item for item in recorder.events if item.event_type == "apcore.subscriber.circuit_closed"]) == 1
    finally:
        client.close()
        unregister_subscriber_type("issue123-probe")


def test_d151_declaration_matches_verified_fixture_scope_and_package() -> None:
    """D-151, conformance §6.2: declarations cannot inflate fixture/test counts."""
    import tomllib

    root = Path(__file__).resolve().parents[2]
    declaration = yaml.safe_load((root / "apcore-conformance.yaml").read_text(encoding="utf-8"))
    project = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))
    implementation = declaration["implementation"]
    assert implementation["name"] == "apcore-python"
    assert implementation["language"] == "python"
    assert implementation["version"] == project["project"]["version"]
    assert implementation["spec_version"] == "1.65.0"
    conformance = declaration["conformance"]
    assert conformance["level"] == 0
    results = conformance["fixture_results"]
    assert results["fixtures"] == len(results["fixture_names"]) == len(set(results["fixture_names"])) == 9
    cases = sum(len(load_fixture(name + ".json")["test_cases"]) for name in results["fixture_names"])
    assert results["cases"] == results["passed"] == cases == 79
    assert results["failed"] == results["skipped"] == 0
    assert (root / results["report"]).is_file()
    assert len(results["reports"]) == 2
    assert all((root / report).is_file() for report in results["reports"])
    assert any(item["feature"] == "full-fixture-verification" for item in conformance["known_deviations"])


@pytest.mark.parametrize("producer_fails", [False, True])
def test_d134_custom_pure_steps_use_logical_capability_dependencies(producer_fails: bool) -> None:
    """D-134: logical provides/requires names need not be Context attributes."""
    from apcore.pipeline import BaseStep, ExecutionStrategy, PipelineContext, StepResult

    observed: list[str] = []

    class Producer(BaseStep):
        def __init__(self) -> None:
            super().__init__(name="producer", pure=True, provides=("logical_ready",))

        async def execute(self, context: PipelineContext) -> StepResult:
            observed.append("producer")
            if producer_fails:
                raise RuntimeError("Producer unavailable")
            return StepResult(action="continue")

    class Consumer(BaseStep):
        def __init__(self) -> None:
            super().__init__(name="consumer", pure=True, requires=("logical_ready",))

        async def execute(self, context: PipelineContext) -> StepResult:
            observed.append("consumer")
            return StepResult(action="continue")

    class Independent(BaseStep):
        def __init__(self) -> None:
            super().__init__(name="independent", pure=True)

        async def execute(self, context: PipelineContext) -> StepResult:
            observed.append("independent")
            return StepResult(action="continue")

    strategy = ExecutionStrategy("logical-capabilities", [Producer(), Consumer(), Independent()])
    result = Executor(Registry(), strategy=strategy).validate("demo.logical", {})
    assert observed == (["producer", "independent"] if producer_fails else ["producer", "consumer", "independent"])
    assert result.valid is not producer_fails
    checks = {check.check: check.passed for check in result.checks}
    assert checks["independent"]
    assert ("consumer" in checks) is not producer_fails
