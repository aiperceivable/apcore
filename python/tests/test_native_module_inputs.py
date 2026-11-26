"""Public invocation coverage for native Pydantic module contracts."""

from __future__ import annotations

from collections.abc import AsyncIterator
from datetime import datetime, timezone
from typing import Annotated, Any

import pytest
from pydantic import BaseModel, ConfigDict, Field

from apcore import APCore, Context, ModuleAnnotations, SchemaValidationError


class Node(BaseModel):
    count: Annotated[int, Field(ge=0)]
    children: list[Node] = Field(default_factory=list)


class ProbeInput(BaseModel):
    model_config = ConfigDict(extra="allow")

    count: int
    tree: Node
    buckets: dict[str, list[int]]
    when: datetime = Field(alias="event-time")
    reading: float
    untyped: Any
    optional: int = 7


class ProbeOutput(BaseModel):
    count: int


class ProbeModule:
    input_schema = ProbeInput
    output_schema = ProbeOutput
    description = "Records values received from the native model pipeline"
    annotations = ModuleAnnotations(streaming=True)

    def __init__(self) -> None:
        self.received: dict[str, Any] = {}

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        self.received = inputs
        return {"count": 4.0}

    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[dict[str, Any]]:
        yield self.execute(inputs, context)


class NativeProbeModule(ProbeModule):
    native_inputs = True


def _probe_inputs(count: float | int = 4.0) -> dict[str, Any]:
    return {
        "count": count,
        "tree": {"count": 4.0, "children": [{"count": 5.0}]},
        "buckets": {"first": [4.0, 7]},
        "event-time": "2024-01-01T00:00:00Z",
        "reading": 4.0,
        "untyped": 4.0,
        "extra": "keep",
    }


def test_class_module_preserves_wire_values_after_integral_float_validation() -> None:
    client = APCore()
    module = ProbeModule()
    client.register("probe.wire_values", module)
    inputs = _probe_inputs()

    assert client.validate("probe.wire_values", inputs).valid
    result = client.call("probe.wire_values", inputs)

    assert module.received == inputs
    assert type(module.received["count"]) is float
    assert type(module.received["tree"]["children"][0]["count"]) is float
    assert type(result["count"]) is float


@pytest.mark.asyncio
async def test_class_module_can_opt_into_native_values_for_calls_and_streams() -> None:
    client = APCore()
    module = NativeProbeModule()
    client.register("probe.native_values", module)
    inputs = _probe_inputs()

    await client.call_async("probe.native_values", inputs)
    calls = module.received
    assert [chunk async for chunk in client.stream("probe.native_values", inputs)] == [{"count": 4.0}]
    assert module.received == calls
    assert calls["when"] == datetime(2024, 1, 1, tzinfo=timezone.utc)
    assert type(calls["count"]) is int
    assert isinstance(calls["tree"], Node)
    assert type(calls["tree"].children[0].count) is int
    assert type(calls["buckets"]["first"][0]) is int
    assert type(calls["reading"]) is float
    assert type(calls["untyped"]) is float
    assert calls["extra"] == "keep"
    assert "event-time" not in calls
    assert "optional" not in calls
    assert type(inputs["count"]) is float


@pytest.mark.parametrize("count", ["4", True, 4.5])
def test_native_integer_rejects_other_json_types(count: Any) -> None:
    client = APCore()
    module = ProbeModule()
    client.register("probe.strict_integer", module)
    inputs = _probe_inputs(count)

    with pytest.raises(SchemaValidationError) as exc:
        client.call("probe.strict_integer", inputs)

    assert any(error["path"] == "/count" and error["keyword"] == "type" for error in exc.value.details["errors"])
    assert module.received == {}


def test_native_integer_applies_bounds_after_normalization() -> None:
    client = APCore()
    client.register("probe.integer_bounds", ProbeModule())
    inputs = _probe_inputs()
    inputs["tree"]["count"] = -4.0

    with pytest.raises(SchemaValidationError) as exc:
        client.call("probe.integer_bounds", inputs)

    assert exc.value.details["errors"] == [
        {"path": "/tree/count", "keyword": "minimum", "message": "Value does not satisfy the minimum schema constraint"}
    ]


def test_function_module_keeps_existing_smart_union_numeric_choice() -> None:
    client = APCore()

    @client.module(id="probe.numeric_union")
    def report_type(count: int | float) -> str:
        """Report the native numeric type."""
        return type(count).__name__

    assert client.call("probe.numeric_union", {"count": 4.0}) == {"result": "float"}


def test_class_module_honors_an_overridden_validation_entry_point() -> None:
    class CustomInput(BaseModel):
        count: int

        @classmethod
        def model_validate_json(cls, json_data: str, **kwargs: Any) -> CustomInput:
            return super().model_validate_json('{"count": 7}', **kwargs)

    class CustomInputModule(NativeProbeModule):
        input_schema = CustomInput

    client = APCore()
    module = CustomInputModule()
    client.register("probe.custom_input", module)

    client.call("probe.custom_input", {"count": 4})

    assert module.received == {"count": 7}
