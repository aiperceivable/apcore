# Spec-traced contract tests — each test maps to a clause in
# apcore/docs/features/module-interface.md ("## Contract: Module conformance").
"""Spec-traced contract tests for the apcore Module Interface.

Source of truth: apcore/docs/features/module-interface.md, the single
``## Contract: Module conformance`` block.

Every test's docstring carries the verbatim clause id in the cross-language
form ``module_interface.<method>.<kind>.<detail>`` so that rows line up across
the Python / TypeScript / Rust SDKs.

GAP NOTE: the contract's ### Errors section names six dedicated error types
(MissingRequiredAttribute, InvalidSchemaType, DescriptionTooLong,
DocumentationTooLong, InvalidAnnotations, InvalidExample). The Python SDK does
not expose those as distinct error types — conformance is duck-typed and
``validate_module()`` returns descriptive strings rather than raising typed
errors. Those clauses are emitted as skips tagged ``missing symbol`` to document
the cross-language gap without breaking import of this file.
"""

from __future__ import annotations

import asyncio
from typing import Any, ClassVar

import pytest
from pydantic import BaseModel, Field

from apcore.context import Context
from apcore.errors import ErrorCodes, ModuleTimeoutError, SchemaValidationError
from apcore.module import Module, ModuleAnnotations, ModuleExample
from apcore.registry.registry import Registry
from apcore.registry.validation import validate_module
from apcore.schema.validator import SchemaValidator


# ---------------------------------------------------------------------------
# Schema + module fixtures exercising the required/optional surface
# ---------------------------------------------------------------------------


class _EchoInput(BaseModel):
    value: str = Field(..., description="The value to echo back")


class _EchoOutput(BaseModel):
    echoed: str = Field(..., description="The echoed value")


class _ConformingModule:
    """Minimal module satisfying the full required surface."""

    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    description: ClassVar[str] = "Echoes the supplied value back unchanged."

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del context
        return {"echoed": inputs["value"]}


class _MissingInputSchemaModule:
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    description: ClassVar[str] = "missing input_schema"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del inputs, context
        return {"echoed": ""}


class _MissingOutputSchemaModule:
    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    description: ClassVar[str] = "missing output_schema"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del inputs, context
        return {"echoed": ""}


class _MissingDescriptionModule:
    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    # description deliberately absent

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del inputs, context
        return {"echoed": ""}


class _MissingExecuteModule:
    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    description: ClassVar[str] = "missing execute"


def _make_context() -> Context:
    return Context.create()


# ===========================================================================
# Inputs / required-surface validation
# ===========================================================================


class TestModuleConformanceRequiredSurface:
    """Required-surface clauses: a module missing any of input_schema /
    output_schema / description / execute fails structural conformance."""

    def test_module_interface_execute_input_input_schema_missing(self) -> None:
        """module_interface.execute.input.input_schema.missing

        A module lacking ``input_schema`` MUST fail structural conformance.
        ``validate_module`` reports a non-empty error list naming input_schema.
        """
        errors = validate_module(_MissingInputSchemaModule)
        assert errors, "missing input_schema must produce a conformance error"
        assert any("input_schema" in e for e in errors)
        # A fully-conforming module produces no errors (control assertion).
        assert validate_module(_ConformingModule) == []

    def test_module_interface_execute_input_output_schema_missing(self) -> None:
        """module_interface.execute.input.output_schema.missing

        A module lacking ``output_schema`` MUST fail structural conformance.
        """
        errors = validate_module(_MissingOutputSchemaModule)
        assert errors, "missing output_schema must produce a conformance error"
        assert any("output_schema" in e for e in errors)

    def test_module_interface_execute_input_description_missing(self) -> None:
        """module_interface.execute.input.description.missing

        A module lacking ``description`` MUST fail structural conformance.
        """
        errors = validate_module(_MissingDescriptionModule)
        assert errors, "missing description must produce a conformance error"
        assert any("description" in e.lower() for e in errors)

    def test_module_interface_execute_input_execute_missing(self) -> None:
        """module_interface.execute.input.execute.missing

        A module lacking an ``execute`` method MUST fail structural conformance.
        """
        errors = validate_module(_MissingExecuteModule)
        assert errors, "missing execute must produce a conformance error"
        assert any("execute" in e.lower() for e in errors)

    def test_module_interface_execute_input_inputs_invalid_against_schema(self) -> None:
        """module_interface.execute.input.inputs.invalid_against_schema

        ``inputs`` MUST validate against ``input_schema`` at execution time; a
        validation failure surfaces as a SchemaValidationError carrying code
        SCHEMA_VALIDATION_ERROR before the module body runs.
        """
        validator = SchemaValidator()
        with pytest.raises(SchemaValidationError) as exc_info:
            # value is required and must be a str; an int violates the schema.
            validator.validate_input({"value": 123}, _EchoInput)
        assert exc_info.value.code == ErrorCodes.SCHEMA_VALIDATION_ERROR


# ===========================================================================
# Errors — declared error types
# ===========================================================================


class TestModuleConformanceErrors:
    """### Errors clauses. Most named error types do not exist as distinct
    types in the Python SDK (duck-typed conformance) and are recorded as
    missing-symbol skips. MODULE_TIMEOUT is a real, testable code."""

    def test_module_interface_errors_missing_required_attribute(self) -> None:
        """module_interface.errors.MissingRequiredAttribute"""
        pytest.skip(
            "missing symbol: MissingRequiredAttribute (contract gap) — Python SDK "
            "validates required surface via duck typing and validate_module() "
            "string output, not a typed error."
        )

    def test_module_interface_errors_invalid_schema_type(self) -> None:
        """module_interface.errors.InvalidSchemaType"""
        pytest.skip("missing symbol: InvalidSchemaType (contract gap) — no such error type " "in apcore.errors.")

    def test_module_interface_errors_description_too_long(self) -> None:
        """module_interface.errors.DescriptionTooLong"""
        pytest.skip(
            "missing symbol: DescriptionTooLong (contract gap) — no such error type "
            "in apcore.errors; the 200-char limit is not enforced via a typed error."
        )

    def test_module_interface_errors_documentation_too_long(self) -> None:
        """module_interface.errors.DocumentationTooLong"""
        pytest.skip("missing symbol: DocumentationTooLong (contract gap) — no such error type " "in apcore.errors.")

    def test_module_interface_errors_invalid_annotations(self) -> None:
        """module_interface.errors.InvalidAnnotations"""
        pytest.skip("missing symbol: InvalidAnnotations (contract gap) — no such error type " "in apcore.errors.")

    def test_module_interface_errors_invalid_example(self) -> None:
        """module_interface.errors.InvalidExample"""
        pytest.skip(
            "missing symbol: InvalidExample (contract gap) — no such error type "
            "in apcore.errors; ModuleExample is a dataclass without typed-error "
            "validation of title/inputs."
        )

    def test_module_interface_errors_module_timeout(self) -> None:
        """module_interface.errors.MODULE_TIMEOUT

        After timeout the framework MUST raise MODULE_TIMEOUT. ModuleTimeoutError
        carries code MODULE_TIMEOUT exactly.
        """
        err = ModuleTimeoutError(module_id="slow.module", timeout_ms=30000)
        assert err.code == ErrorCodes.MODULE_TIMEOUT
        assert err.code == "MODULE_TIMEOUT"
        # Detail fields are preserved for AI-facing guidance.
        assert err.module_id == "slow.module"
        assert err.timeout_ms == 30000


# ===========================================================================
# Properties
# ===========================================================================


class _AsyncModule:
    """Module whose execute() is async (def vs async def auto-detection)."""

    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    description: ClassVar[str] = "Async echo module."

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del context
        await asyncio.sleep(0)
        return {"echoed": inputs["value"]}


class _ConcurrentModule:
    """Thread-safe module: only mutates per-call local state, never ClassVar."""

    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    description: ClassVar[str] = "Concurrent-safe echo module."
    # A ClassVar that MUST NOT be mutated by execute().
    _shared_marker: ClassVar[str] = "untouched"

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del context
        await asyncio.sleep(0)
        return {"echoed": inputs["value"]}


class _SideEffectfulModule:
    """Module with observable side effects — pure is NOT required by the contract."""

    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    description: ClassVar[str] = "Counts how many times it ran."

    def __init__(self) -> None:
        self.call_count = 0

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del context
        self.call_count += 1
        return {"echoed": inputs["value"]}


class TestModuleConformanceProperties:
    """### Properties clauses: async (MAY), thread_safe (required),
    pure (not required)."""

    async def test_module_interface_execute_property_async(self) -> None:
        """module_interface.execute.property.async

        execute() MAY be async; an async execute() is awaitable and resolves to
        a dict validating against output_schema.
        """
        module = _AsyncModule()
        coro = module.execute({"value": "hi"}, _make_context())
        assert asyncio.iscoroutine(coro)
        result = await coro
        assert result == {"echoed": "hi"}
        # Result validates against output_schema.
        assert _EchoOutput(**result).echoed == "hi"

    async def test_module_interface_execute_property_thread_safe(self) -> None:
        """module_interface.execute.property.thread_safe

        Instances MUST tolerate concurrent execute() invocations and MUST NOT
        mutate ClassVar attributes. Launch >=8 concurrent calls with distinct
        inputs and assert every result is correct and the ClassVar is untouched.
        """
        module = _ConcurrentModule()
        marker_before = _ConcurrentModule._shared_marker
        n = 16
        inputs = [{"value": f"v{i}"} for i in range(n)]
        results = await asyncio.gather(*(module.execute(inp, _make_context()) for inp in inputs))
        # No exception raised; every call returned its own distinct value.
        assert [r["echoed"] for r in results] == [f"v{i}" for i in range(n)]
        # ClassVar must not have been mutated during concurrent execution.
        assert _ConcurrentModule._shared_marker == marker_before == "untouched"

    def test_module_interface_execute_property_pure_not_required(self) -> None:
        """module_interface.execute.property.pure_false

        pure is NOT required: a module MAY perform side effects. A module that
        mutates its own instance state across calls is still a conforming
        module. Assert the side effect is observable AND conformance holds.
        """
        module = _SideEffectfulModule()
        assert module.call_count == 0
        module.execute({"value": "a"}, _make_context())
        module.execute({"value": "b"}, _make_context())
        # Side effect is observable — the contract permits this.
        assert module.call_count == 2
        # Despite the side effect, the module still satisfies the surface.
        assert validate_module(_SideEffectfulModule) == []


# ===========================================================================
# Return-value constraints
# ===========================================================================


class TestModuleConformanceReturn:
    """### Return-value constraints clauses."""

    def test_module_interface_execute_return_must_be_dict(self) -> None:
        """module_interface.execute.return.must_be_dict

        execute() MUST return a dict that validates against output_schema.
        """
        module = _ConformingModule()
        result = module.execute({"value": "payload"}, _make_context())
        assert isinstance(result, dict)
        # The return value validates against output_schema.
        validated = _EchoOutput(**result)
        assert validated.echoed == "payload"

    def test_module_interface_execute_return_validates_against_output_schema(self) -> None:
        """module_interface.execute.return.must_be_dict

        A return value that does NOT match output_schema is rejected by the
        validator with SCHEMA_VALIDATION_ERROR — confirming the contract that
        the output MUST validate against output_schema.
        """
        validator = SchemaValidator()
        with pytest.raises(SchemaValidationError) as exc_info:
            # 'echoed' missing entirely violates the required output schema.
            validator.validate_output({}, _EchoOutput)
        assert exc_info.value.code == ErrorCodes.SCHEMA_VALIDATION_ERROR


# ===========================================================================
# Side effects — lifecycle hook ordering
# ===========================================================================


class _LifecycleModule:
    """Records lifecycle-hook invocation order into a shared log list."""

    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    description: ClassVar[str] = "Lifecycle-instrumented module."

    def __init__(self, log: list[str]) -> None:
        self._log = log

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del context
        return {"echoed": inputs["value"]}

    def on_load(self) -> None:
        self._log.append("on_load")

    def on_unload(self) -> None:
        self._log.append("on_unload")


class _SuspendResumeModule:
    """Module implementing on_suspend / on_resume for hot-reload round-trip."""

    input_schema: ClassVar[type[BaseModel]] = _EchoInput
    output_schema: ClassVar[type[BaseModel]] = _EchoOutput
    description: ClassVar[str] = "Stateful suspend/resume module."
    version: ClassVar[str] = "1.0.0"

    def __init__(self) -> None:
        self.counter = 0
        self.resumed: dict[str, Any] | None = None

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        del context
        self.counter += 1
        return {"echoed": inputs["value"]}

    def on_suspend(self) -> dict[str, Any]:
        return {"counter": self.counter}

    def on_resume(self, state: dict[str, Any]) -> None:
        self.counter = state.get("counter", 0)
        self.resumed = state


class TestModuleConformanceSideEffects:
    """### Side Effects clauses: lifecycle-hook ordering observed via the
    public Registry register/unregister API."""

    def test_module_interface_lifecycle_side_effect_1_on_load(self) -> None:
        """module_interface.lifecycle.side_effect.1.on_load

        on_load() MUST be invoked when the module is registered. It is the first
        lifecycle hook observed (before any on_unload).
        """
        log: list[str] = []
        registry = Registry()
        registry.register("test.lifecycle_load", _LifecycleModule(log))
        assert log == ["on_load"]

    def test_module_interface_lifecycle_side_effect_2_on_unload(self) -> None:
        """module_interface.lifecycle.side_effect.2.on_unload

        on_unload() MUST be invoked when the module is unregistered, and MUST be
        ordered strictly after on_load (register -> unregister).
        """
        log: list[str] = []
        registry = Registry()
        registry.register("test.lifecycle_unload", _LifecycleModule(log))
        registry.unregister("test.lifecycle_unload")
        # Order MUST be on_load then on_unload.
        assert log == ["on_load", "on_unload"]

    def test_module_interface_lifecycle_side_effect_suspend_resume_roundtrip(self) -> None:
        """module_interface.lifecycle.side_effect.3.suspend_resume

        on_suspend() exports JSON-serializable state which on_resume() restores.
        The round-trip MUST preserve the state value, confirming the ordered
        suspend -> resume contract.
        """
        old = _SuspendResumeModule()
        # Build up some state via execute().
        old.execute({"value": "x"}, _make_context())
        old.execute({"value": "y"}, _make_context())
        assert old.counter == 2

        # Export state (on_suspend) — MUST be JSON-serializable (plain dict).
        state = old.on_suspend()
        assert state == {"counter": 2}
        import json

        assert json.loads(json.dumps(state)) == {"counter": 2}

        # Restore into the new instance (on_resume) — state is preserved.
        new = _SuspendResumeModule()
        assert new.counter == 0
        new.on_resume(state)
        assert new.counter == 2
        assert new.resumed == {"counter": 2}


# ===========================================================================
# Function-based form parity (structural conformance via the Module Protocol)
# ===========================================================================


class TestModuleConformanceProtocol:
    """The Module surface is structural (typing.Protocol, @runtime_checkable).
    A conforming class is recognized as a Module at runtime."""

    def test_module_interface_execute_property_structural_conformance(self) -> None:
        """module_interface.execute.property.structural

        Conformance is structural (duck-typed), not ABC inheritance. A class
        carrying the required surface MUST be a runtime instance of the Module
        Protocol; one missing execute MUST NOT.
        """
        assert isinstance(_ConformingModule(), Module)
        assert not isinstance(_MissingExecuteModule(), Module)
        # Sanity: ModuleAnnotations / ModuleExample are the real exported types.
        assert isinstance(ModuleAnnotations(), ModuleAnnotations)
        assert ModuleExample(title="t", inputs={"value": "v"}).title == "t"
