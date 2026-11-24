"""Verify the JSON output boundary for native and wrapped modules."""

from collections.abc import AsyncIterator
from datetime import datetime
from typing import Any

import pytest
from pydantic import AliasChoices, BaseModel, Field, field_serializer

from apcore import Context, Executor, Registry
from apcore.decorator import FunctionModule
from apcore.errors import ModuleExecuteError
from apcore.middleware import Middleware
from apcore.utils.module_output import normalize_module_output
from conformance.canonical_fixtures import load_fixture


class _Item(BaseModel):
    item_id: str = Field(alias="id")


class _Output(BaseModel):
    item: _Item
    updated_at: datetime


class _Input(BaseModel):
    pass


class _NativeModule:
    input_schema = _Input
    output_schema = _Output
    description = "Return a typed object"

    def __init__(self, output: _Output) -> None:
        self._output = output

    def execute(self, inputs: dict[str, Any], context: Context) -> _Output | dict[str, Any]:
        return self._output


class _NativeStreamingModule(_NativeModule):
    async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[_Output]:
        yield self._output


class _NativeMappingModule(_NativeModule):
    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"item": self._output.item, "updated_at": self._output.updated_at}


class _Recorder(Middleware):
    def __init__(self) -> None:
        super().__init__()
        self.outputs: list[dict[str, Any]] = []

    def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
        self.outputs.append(output)


def assert_json_output(output: dict[str, Any], expected: dict[str, Any]) -> None:
    assert type(output) is dict
    assert output.keys() == expected.keys()
    assert output["item"] == expected["item"]
    # RFC 3339 permits different fractional-second precision for the same time.
    assert datetime.fromisoformat(output["updated_at"]) == datetime.fromisoformat(expected["updated_at"])


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "case", load_fixture("module_output_normalization.json")["test_cases"], ids=lambda case: case["id"]
)
@pytest.mark.parametrize("kind", ("class", "wrapped", "native-stream", "native-mapping"))
async def test_typed_module_outputs(case: dict[str, Any], kind: str) -> None:
    output = _Output.model_validate(case["input"])
    module_id = "test.typed_output"

    def wrapped() -> _Output:
        """Return a typed object."""
        return output

    registry = Registry()
    variant = {
        "class": _NativeModule(output),
        "wrapped": FunctionModule(wrapped, module_id),
        "native-stream": _NativeStreamingModule(output),
        "native-mapping": _NativeMappingModule(output),
    }
    registry.register(module_id, variant[kind])
    recorder = _Recorder()
    executor = Executor(registry=registry, middlewares=[recorder])

    result = await executor.call_async(module_id, {})
    assert_json_output(result, case["expected"]["output"])

    chunks = [chunk async for chunk in executor.stream(module_id, {})]
    for chunk, expected in zip(chunks, case["expected"]["chunks"], strict=True):
        assert_json_output(chunk, expected)
    assert len(recorder.outputs) == 2
    for output_seen_by_middleware in recorder.outputs:
        assert_json_output(output_seen_by_middleware, case["expected"]["output"])


def test_projection_preserves_unsupported_nested_values() -> None:
    opaque = object()
    output = {
        "items": [_Item(id="alpha"), opaque],
        "date": datetime.fromisoformat("2026-11-24T12:00:00Z"),
    }

    projected = normalize_module_output(output)

    assert projected["items"] == [{"id": "alpha"}, opaque]
    assert projected["items"][1] is opaque
    assert isinstance(output["items"][0], _Item)
    assert datetime.fromisoformat(projected["date"]) == output["date"]


@pytest.mark.asyncio
async def test_typed_outputs_use_the_published_names_with_distinct_serialization_aliases() -> None:
    class AliasedItem(BaseModel):
        item_id: str = Field(
            validation_alias=AliasChoices("id", "identifier"),
            serialization_alias="itemId",
        )
        name: str = Field(alias="displayName")

        @field_serializer("item_id")
        def serialize_item_id(self, item_id: str) -> str:
            return item_id.upper()

    class AliasedOutput(BaseModel):
        items: list[AliasedItem] = Field(serialization_alias="entries")

    result = AliasedOutput(items=[AliasedItem(id="alpha", displayName="First")])

    class AliasedModule:
        input_schema = _Input
        output_schema = AliasedOutput
        description = "Return values under the published field names"

        def execute(self, inputs: dict[str, Any], context: Context) -> AliasedOutput:
            return result

        async def stream(self, inputs: dict[str, Any], context: Context) -> AsyncIterator[AliasedOutput]:
            yield result

    def wrapped() -> AliasedOutput:
        """Return values under the published field names."""
        return result

    registry = Registry()
    registry.register("test.typed_aliases", AliasedModule())
    registry.register("test.wrapped_aliases", FunctionModule(wrapped, "test.wrapped_aliases"))
    executor = Executor(registry=registry)
    for module_id in ("test.typed_aliases", "test.wrapped_aliases"):
        definition = registry.get_definition(module_id)
        assert definition is not None
        assert set(definition.output_schema["properties"]) == {"items"}
        assert set(definition.output_schema["$defs"]["AliasedItem"]["properties"]) == {"id", "displayName"}
        expected = {"items": [{"id": "ALPHA", "displayName": "First"}]}
        assert await executor.call_async(module_id, {}) == expected
        assert [chunk async for chunk in executor.stream(module_id, {})] == [expected]


@pytest.mark.asyncio
async def test_execute_rejects_non_map_results_even_without_output_validation() -> None:
    class ScalarModule(_NativeModule):
        def execute(self, inputs: dict[str, Any], context: Context) -> Any:
            return ["not an object"]

    module_id = "test.invalid_output"
    registry = Registry()
    output = _Output(item=_Item(id="alpha"), updated_at=datetime.fromisoformat("2026-11-24T12:00:00Z"))
    registry.register(module_id, ScalarModule(output))
    executor = Executor(registry=registry, strategy="minimal")

    with pytest.raises(ModuleExecuteError, match="Return value must be Map"):
        await executor.call_async(module_id, {})
    with pytest.raises(ModuleExecuteError, match="Return value must be Map"):
        await anext(executor.stream(module_id, {}))
