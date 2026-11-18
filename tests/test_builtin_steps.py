"""Tests for built-in pipeline steps."""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any
from unittest.mock import AsyncMock, MagicMock

import pytest

from apcore.builtin_steps import (
    BuiltinACLCheck,
    BuiltinApprovalGate,
    BuiltinContextCreation,
    BuiltinExecute,
    BuiltinInputValidation,
    BuiltinMiddlewareAfter,
    BuiltinMiddlewareBefore,
    BuiltinModuleLookup,
    BuiltinOutputValidation,
    BuiltinReturnResult,
    BuiltinCallChainGuard,
    build_standard_strategy,
)
from apcore.errors import InvalidInputError, SchemaValidationError
from apcore.pipeline import (
    BaseStep,
    ExecutionStrategy,
    PipelineContext,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_ctx(
    module_id: str = "test.module",
    inputs: dict[str, Any] | None = None,
    context: Any = None,
    module: Any = None,
) -> PipelineContext:
    """Create a minimal PipelineContext for testing."""
    return PipelineContext(
        module_id=module_id,
        inputs=inputs or {},
        context=context,
        module=module,
    )


class FakeRegistry:
    """Minimal registry that returns a module by ID."""

    def __init__(self, modules: dict[str, Any] | None = None) -> None:
        self._modules = modules or {}

    def get(self, module_id: str, **kwargs: Any) -> Any:
        return self._modules.get(module_id)


class FakeModule:
    """Minimal module for testing."""

    def __init__(
        self,
        *,
        input_schema: Any = None,
        output_schema: Any = None,
        annotations: Any = None,
    ) -> None:
        self.input_schema = input_schema
        self.output_schema = output_schema
        self.annotations = annotations

    def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
        return {"result": "ok"}


class FakeContext:
    """Minimal context for testing."""

    def __init__(self, caller_id: str = "user-1") -> None:
        self.caller_id = caller_id
        self.call_chain: list[str] = []
        self.global_deadline: float | None = None

    def child(self, module_id: str) -> FakeContext:
        c = FakeContext(caller_id=self.caller_id)
        c.call_chain = [*self.call_chain, module_id]
        return c


# ---------------------------------------------------------------------------
# Instantiation tests
# ---------------------------------------------------------------------------


class TestStepInstantiation:
    """Verify each step can be instantiated and is a BaseStep."""

    def test_context_creation(self) -> None:
        step = BuiltinContextCreation()
        assert isinstance(step, BaseStep)
        assert step.name == "context_creation"

    def test_safety_check(self) -> None:
        step = BuiltinCallChainGuard()
        assert isinstance(step, BaseStep)
        assert step.name == "call_chain_guard"

    def test_module_lookup(self) -> None:
        step = BuiltinModuleLookup(registry=FakeRegistry())
        assert isinstance(step, BaseStep)
        assert step.name == "module_lookup"

    def test_acl_check(self) -> None:
        step = BuiltinACLCheck()
        assert isinstance(step, BaseStep)
        assert step.name == "acl_check"

    def test_approval_gate(self) -> None:
        step = BuiltinApprovalGate()
        assert isinstance(step, BaseStep)
        assert step.name == "approval_gate"

    def test_input_validation(self) -> None:
        step = BuiltinInputValidation()
        assert isinstance(step, BaseStep)
        assert step.name == "input_validation"

    def test_middleware_before(self) -> None:
        step = BuiltinMiddlewareBefore()
        assert isinstance(step, BaseStep)
        assert step.name == "middleware_before"

    def test_execute(self) -> None:
        step = BuiltinExecute()
        assert isinstance(step, BaseStep)
        assert step.name == "execute"

    def test_output_validation(self) -> None:
        step = BuiltinOutputValidation()
        assert isinstance(step, BaseStep)
        assert step.name == "output_validation"

    def test_middleware_after(self) -> None:
        step = BuiltinMiddlewareAfter()
        assert isinstance(step, BaseStep)
        assert step.name == "middleware_after"

    def test_return_result(self) -> None:
        step = BuiltinReturnResult()
        assert isinstance(step, BaseStep)
        assert step.name == "return_result"


# ---------------------------------------------------------------------------
# Removable / replaceable flags
# ---------------------------------------------------------------------------


class TestStepFlags:
    """Verify removable and replaceable flags match the spec."""

    @pytest.mark.parametrize(
        "step_factory,expected_removable,expected_replaceable",
        [
            (lambda: BuiltinContextCreation(), False, False),
            (lambda: BuiltinCallChainGuard(), True, True),
            (lambda: BuiltinModuleLookup(registry=FakeRegistry()), False, False),
            (lambda: BuiltinACLCheck(), True, True),
            (lambda: BuiltinApprovalGate(), True, True),
            (lambda: BuiltinInputValidation(), True, True),
            (lambda: BuiltinMiddlewareBefore(), True, False),
            (lambda: BuiltinExecute(), False, True),
            (lambda: BuiltinOutputValidation(), True, True),
            (lambda: BuiltinMiddlewareAfter(), True, False),
            (lambda: BuiltinReturnResult(), False, False),
        ],
        ids=[
            "context_creation",
            "call_chain_guard",
            "module_lookup",
            "acl_check",
            "approval_gate",
            "input_validation",
            "middleware_before",
            "execute",
            "output_validation",
            "middleware_after",
            "return_result",
        ],
    )
    def test_flags(
        self,
        step_factory: Any,
        expected_removable: bool,
        expected_replaceable: bool,
    ) -> None:
        step = step_factory()
        assert step.removable is expected_removable
        assert step.replaceable is expected_replaceable


# ---------------------------------------------------------------------------
# build_standard_strategy
# ---------------------------------------------------------------------------


class TestBuildStandardStrategy:
    """Verify the factory creates the correct strategy."""

    def test_creates_11_steps(self) -> None:
        strategy = build_standard_strategy(registry=FakeRegistry())
        assert isinstance(strategy, ExecutionStrategy)
        assert len(strategy.steps) == 11

    def test_step_names_ordered(self) -> None:
        strategy = build_standard_strategy(registry=FakeRegistry())
        expected = [
            "context_creation",
            "call_chain_guard",
            "module_lookup",
            "acl_check",
            "approval_gate",
            "middleware_before",
            "input_validation",
            "execute",
            "output_validation",
            "middleware_after",
            "return_result",
        ]
        assert strategy.step_names() == expected

    def test_strategy_name(self) -> None:
        strategy = build_standard_strategy(registry=FakeRegistry())
        assert strategy.name == "standard"


# ---------------------------------------------------------------------------
# Step execution: happy path (async tests)
# ---------------------------------------------------------------------------


class TestContextCreationStep:
    """Test BuiltinContextCreation execute."""

    async def test_creates_context_when_none(self) -> None:
        ctx = _make_ctx(context=None)
        step = BuiltinContextCreation()
        result = await step.execute(ctx)
        assert result.action == "continue"
        assert ctx.context is not None

    async def test_preserves_existing_context(self) -> None:
        fake_ctx = FakeContext()
        ctx = _make_ctx(context=fake_ctx)
        step = BuiltinContextCreation()
        result = await step.execute(ctx)
        assert result.action == "continue"


class TestSafetyCheckStep:
    """Test BuiltinCallChainGuard execute."""

    async def test_passes_normal(self) -> None:
        fake_ctx = FakeContext()
        ctx = _make_ctx(context=fake_ctx)
        step = BuiltinCallChainGuard()
        result = await step.execute(ctx)
        assert result.action == "continue"


class TestModuleLookupStep:
    """Test BuiltinModuleLookup execute."""

    async def test_sets_module_on_found(self) -> None:
        module = FakeModule()
        registry = FakeRegistry({"test.module": module})
        ctx = _make_ctx(module_id="test.module")
        step = BuiltinModuleLookup(registry=registry)
        result = await step.execute(ctx)
        assert result.action == "continue"
        assert ctx.module is module

    async def test_raises_on_not_found(self) -> None:
        from apcore.errors import ModuleNotFoundError

        registry = FakeRegistry({})
        ctx = _make_ctx(module_id="missing.module")
        step = BuiltinModuleLookup(registry=registry)
        with pytest.raises(ModuleNotFoundError):
            await step.execute(ctx)


class TestACLCheckStep:
    """Test BuiltinACLCheck execute."""

    async def test_continues_when_no_acl(self) -> None:
        ctx = _make_ctx(context=FakeContext())
        step = BuiltinACLCheck(acl=None)
        result = await step.execute(ctx)
        assert result.action == "continue"

    async def test_continues_when_allowed(self) -> None:
        acl = MagicMock(spec=["check"])
        acl.check.return_value = True
        ctx = _make_ctx(context=FakeContext())
        step = BuiltinACLCheck(acl=acl)
        result = await step.execute(ctx)
        assert result.action == "continue"

    async def test_raises_when_denied(self) -> None:
        from apcore.errors import ACLDeniedError

        acl = MagicMock(spec=["check"])
        acl.check.return_value = False
        ctx = _make_ctx(context=FakeContext())
        step = BuiltinACLCheck(acl=acl)
        with pytest.raises(ACLDeniedError):
            await step.execute(ctx)


class TestApprovalGateStep:
    """Test BuiltinApprovalGate execute."""

    async def test_continues_when_no_handler(self) -> None:
        ctx = _make_ctx(context=FakeContext(), module=FakeModule())
        step = BuiltinApprovalGate(handler=None)
        result = await step.execute(ctx)
        assert result.action == "continue"

    async def test_continues_when_no_approval_needed(self) -> None:
        handler = AsyncMock()
        ctx = _make_ctx(context=FakeContext(), module=FakeModule())
        step = BuiltinApprovalGate(handler=handler)
        result = await step.execute(ctx)
        assert result.action == "continue"

    async def test_rejects_non_string_approval_token(self) -> None:
        # A non-string _approval_token must be rejected with
        # GENERAL_INVALID_INPUT before reaching the handler (security gate;
        # mirrors Rust).
        handler = AsyncMock()
        module = FakeModule(annotations={"requires_approval": True})
        ctx = _make_ctx(
            inputs={"_approval_token": 12345},
            context=FakeContext(),
            module=module,
        )
        step = BuiltinApprovalGate(handler=handler)
        with pytest.raises(InvalidInputError) as exc_info:
            await step.execute(ctx)
        assert exc_info.value.code == "GENERAL_INVALID_INPUT"
        handler.check_approval.assert_not_awaited()


class TestInputValidationStep:
    """Test BuiltinInputValidation execute."""

    async def test_sets_validated_inputs_no_schema(self) -> None:
        module = FakeModule(input_schema=None)
        ctx = _make_ctx(inputs={"a": 1}, module=module)
        step = BuiltinInputValidation()
        result = await step.execute(ctx)
        assert result.action == "continue"
        assert ctx.validated_inputs == {"a": 1}

    async def test_aborts_when_no_module(self) -> None:
        ctx = _make_ctx(module=None)
        step = BuiltinInputValidation()
        result = await step.execute(ctx)
        assert result.action == "abort"


class TestModuleBoundaryDoesNotCoerceTypes:
    """The module-invocation boundary performs NO type coercion (TYPE_MAPPING §17.3).

    A contract that declares ``integer`` receives an integer. ``"42"`` is a type
    error, not an integer spelled differently — and no host configuration may
    relax that, or the same contract would mean different things in different
    deployments. Parity with apcore-typescript (``new SchemaValidator(false)``)
    and apcore-rust (``validate_against_schema``), both of which already rejected
    this input while apcore-python accepted it.
    """

    @staticmethod
    def _model(name: str) -> Any:
        from apcore.config import Config
        from apcore.schema.loader import SchemaLoader

        return SchemaLoader(Config({})).generate_model(
            {
                "type": "object",
                "properties": {"a": {"type": "integer"}},
                "required": ["a"],
            },
            name,
        )

    async def test_input_numeric_string_is_rejected(self) -> None:
        module = FakeModule(input_schema=self._model("BoundaryIn"))
        ctx = _make_ctx(inputs={"a": "42"}, module=module)
        with pytest.raises(SchemaValidationError) as exc_info:
            await BuiltinInputValidation().execute(ctx)
        assert "Input validation failed" in exc_info.value.message

    async def test_output_numeric_string_is_rejected(self) -> None:
        module = FakeModule(output_schema=self._model("BoundaryOut"))
        ctx = _make_ctx(module=module)
        ctx.output = {"a": "42"}
        with pytest.raises(SchemaValidationError):
            await BuiltinOutputValidation().execute(ctx)

    async def test_input_integer_is_accepted(self) -> None:
        module = FakeModule(input_schema=self._model("BoundaryInOk"))
        ctx = _make_ctx(inputs={"a": 42}, module=module)
        result = await BuiltinInputValidation().execute(ctx)
        assert result.action == "continue"

    @staticmethod
    def _bool_model(name: str) -> Any:
        from apcore.config import Config
        from apcore.schema.loader import SchemaLoader

        return SchemaLoader(Config({})).generate_model(
            {
                "type": "object",
                "properties": {"flag": {"type": "boolean"}},
                "required": ["flag"],
            },
            name,
        )

    @pytest.mark.parametrize("spelling", ["true", "false", "yes", "0", "True"])
    async def test_input_boolean_spelled_as_a_string_is_rejected(self, spelling: str) -> None:
        """R5: the boundary MUST reject a string for a declared `boolean`.

        `"true"` and `"false"` are the two spellings the library-level knob gained
        in apcore#95, so they are the ones that could have leaked here — the knob
        and the boundary answering differently for one schema and input is the
        failure TYPE_MAPPING §11 exists to prevent. The knob lives on
        `SchemaValidator`, which this path does not construct at all.
        """
        module = FakeModule(input_schema=self._bool_model(f"BoundaryBool_{spelling}"))
        ctx = _make_ctx(inputs={"flag": spelling}, module=module)
        with pytest.raises(SchemaValidationError):
            await BuiltinInputValidation().execute(ctx)

    @pytest.mark.parametrize("number", [1, 0])
    async def test_input_number_for_boolean_is_rejected(self, number: int) -> None:
        """R5 again, on the numeric side: `1` and `0` are not booleans."""
        module = FakeModule(input_schema=self._bool_model(f"BoundaryBoolNum_{number}"))
        ctx = _make_ctx(inputs={"flag": number}, module=module)
        with pytest.raises(SchemaValidationError):
            await BuiltinInputValidation().execute(ctx)

    async def test_input_zero_fraction_number_is_accepted(self) -> None:
        """§6.1.1: `integer` matches any number with a zero fractional part.

        "No coercion" is about instance *types*, not renderings — `4.0` is an
        integer instance, and the `jsonschema` reference implementation,
        apcore-typescript and apcore-rust all accept it.
        """
        module = FakeModule(input_schema=self._model("BoundaryInFloat"))
        ctx = _make_ctx(inputs={"a": 4.0}, module=module)
        result = await BuiltinInputValidation().execute(ctx)
        assert result.action == "continue"

    async def test_input_fractional_number_is_rejected(self) -> None:
        module = FakeModule(input_schema=self._model("BoundaryInFrac"))
        ctx = _make_ctx(inputs={"a": 4.5}, module=module)
        with pytest.raises(SchemaValidationError):
            await BuiltinInputValidation().execute(ctx)

    async def test_raw_dict_schema_module_accepts_the_strict_keyword(self) -> None:
        """A module that declared its schema as a raw dict must not break.

        ``_DictSchemaAdapter.model_validate`` has to accept the ``strict``
        keyword the boundary passes. Valid input, so this asserts the signature
        and nothing else.
        """
        from apcore.registry.registry import _DictSchemaAdapter

        module = FakeModule(
            input_schema=_DictSchemaAdapter({"type": "object", "properties": {"a": {"type": "integer"}}})
        )
        ctx = _make_ctx(inputs={"a": 42}, module=module)
        result = await BuiltinInputValidation().execute(ctx)
        assert result.action == "continue"

    async def test_raw_dict_schema_is_enforced_not_passed_through(self) -> None:
        """The same parity this class exists for, for dict-declared schemas.

        ``_DictSchemaAdapter`` used to return the data unchanged, so every
        constraint a dict schema declared was inert -- while apcore-typescript
        and apcore-rust enforced them.
        """
        from apcore.registry.registry import _DictSchemaAdapter

        module = FakeModule(
            input_schema=_DictSchemaAdapter(
                {
                    "type": "object",
                    "properties": {"a": {"type": "integer"}},
                    "required": ["a"],
                }
            )
        )
        ctx = _make_ctx(inputs={"a": "42"}, module=module)
        with pytest.raises(SchemaValidationError):
            await BuiltinInputValidation().execute(ctx)


class TestMiddlewareBeforeStep:
    """Test BuiltinMiddlewareBefore execute."""

    async def test_continues_empty_middlewares(self) -> None:
        ctx = _make_ctx(context=FakeContext())
        step = BuiltinMiddlewareBefore(middlewares=[])
        result = await step.execute(ctx)
        assert result.action == "continue"


class TestExecuteStep:
    """Test BuiltinExecute execute."""

    async def test_sets_output(self) -> None:
        module = FakeModule()
        ctx = _make_ctx(inputs={"x": 1}, context=FakeContext(), module=module)
        ctx.validated_inputs = {"x": 1}
        step = BuiltinExecute()
        result = await step.execute(ctx)
        assert result.action == "continue"
        assert ctx.output == {"result": "ok"}

    async def test_aborts_when_no_module(self) -> None:
        ctx = _make_ctx(module=None)
        step = BuiltinExecute()
        result = await step.execute(ctx)
        assert result.action == "abort"


class TestOutputValidationStep:
    """Test BuiltinOutputValidation execute."""

    async def test_sets_validated_output_no_schema(self) -> None:
        module = FakeModule(output_schema=None)
        ctx = _make_ctx(module=module)
        ctx.output = {"val": 42}
        step = BuiltinOutputValidation()
        result = await step.execute(ctx)
        assert result.action == "continue"
        assert ctx.validated_output == {"val": 42}

    async def test_aborts_when_no_module(self) -> None:
        ctx = _make_ctx(module=None)
        step = BuiltinOutputValidation()
        result = await step.execute(ctx)
        assert result.action == "abort"


class TestMiddlewareAfterStep:
    """Test BuiltinMiddlewareAfter execute."""

    async def test_continues_empty_middlewares(self) -> None:
        ctx = _make_ctx(context=FakeContext())
        step = BuiltinMiddlewareAfter(middlewares=[])
        result = await step.execute(ctx)
        assert result.action == "continue"


class TestReturnResultStep:
    """Test BuiltinReturnResult execute."""

    async def test_continues(self) -> None:
        ctx = _make_ctx()
        step = BuiltinReturnResult()
        result = await step.execute(ctx)
        assert result.action == "continue"


class TestFormatWarningAtTheValidationBoundary:
    """The SHOULD-level `format` warning must fire where module invocation runs.

    Validation at the boundary goes through Pydantic, which has no
    format-annotation concept, so without an explicit call the warning only ever
    fired on the `validate_schema_dict` path the executor never reaches.
    """

    def _model(self, tmp_path: Path, name: str) -> Any:
        from apcore.config import Config
        from apcore.schema.loader import SchemaLoader

        loader = SchemaLoader(Config({}), schemas_dir=tmp_path)
        return loader.generate_model(
            {
                "type": "object",
                "properties": {"contact": {"type": "string", "format": "email"}},
                "required": ["contact"],
            },
            name,
        )

    async def test_input_validation_warns_without_failing(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        module = FakeModule(input_schema=self._model(tmp_path, "StepIn"))
        ctx = _make_ctx(inputs={"contact": "not-an-email"}, module=module)
        step = BuiltinInputValidation()
        with caplog.at_level(logging.WARNING, logger="apcore.schema.hardening"):
            result = await step.execute(ctx)
        assert result.action == "continue"
        assert ctx.validated_inputs == {"contact": "not-an-email"}
        records = [r for r in caplog.records if r.name == "apcore.schema.hardening"]
        assert len(records) == 1
        assert "email" in records[0].getMessage()

    async def test_input_validation_silent_for_a_conformant_value(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        module = FakeModule(input_schema=self._model(tmp_path, "StepInOk"))
        ctx = _make_ctx(inputs={"contact": "user@example.com"}, module=module)
        step = BuiltinInputValidation()
        with caplog.at_level(logging.WARNING, logger="apcore.schema.hardening"):
            await step.execute(ctx)
        assert [r for r in caplog.records if r.name == "apcore.schema.hardening"] == []

    async def test_output_validation_warns_without_failing(
        self, tmp_path: Path, caplog: pytest.LogCaptureFixture
    ) -> None:
        module = FakeModule(output_schema=self._model(tmp_path, "StepOut"))
        ctx = _make_ctx(module=module)
        ctx.output = {"contact": "not-an-email"}
        step = BuiltinOutputValidation()
        with caplog.at_level(logging.WARNING, logger="apcore.schema.hardening"):
            result = await step.execute(ctx)
        assert result.action == "continue"
        assert len([r for r in caplog.records if r.name == "apcore.schema.hardening"]) == 1


class TestPerModuleResourceTimeout:
    """`resources.timeout` is the per-module timeout (spec §5.2, core-executor §Timeout).

    ``BuiltinExecute`` used to read ``module.timeout_ms``, an attribute no
    apcore-python module ever defines, so the whole per-module half of the
    dual-timeout model was dead code: every call fell back to
    ``executor.default_timeout`` and the "negative timeout →
    GENERAL_INVALID_INPUT" MUST was unreachable. apcore-typescript reads
    ``mod.resources.timeout`` / ``mod.annotations.resources.timeout``;
    apcore-rust reads ``descriptor.annotations.extra["resources"]["timeout"]``.
    """

    class SlowModule:
        """Module that sleeps long enough to trip any short timeout."""

        def __init__(self, **attrs: Any) -> None:
            for key, value in attrs.items():
                setattr(self, key, value)

        async def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
            import asyncio

            await asyncio.sleep(5)
            return {"result": "too-late"}

    @staticmethod
    def _annotations(resources: dict[str, Any]) -> Any:
        from apcore.module import ModuleAnnotations

        return ModuleAnnotations(extra={"resources": resources})

    async def _run(self, module: Any) -> Any:
        ctx = _make_ctx(inputs={}, context=FakeContext(), module=module)
        ctx.validated_inputs = {}
        return await BuiltinExecute().execute(ctx)

    async def test_direct_resources_attribute_is_honoured(self) -> None:
        from apcore.errors import ModuleTimeoutError

        module = self.SlowModule(resources={"timeout": 10})
        with pytest.raises(ModuleTimeoutError) as exc_info:
            await self._run(module)
        assert exc_info.value.timeout_ms == 10

    async def test_timeout_is_not_retryable_for_a_non_idempotent_module(self) -> None:
        from apcore.errors import ModuleTimeoutError

        module = self.SlowModule(resources={"timeout": 10})
        with pytest.raises(ModuleTimeoutError) as exc_info:
            await self._run(module)
        assert exc_info.value.retryable is False

    async def test_timeout_is_retryable_when_module_declares_idempotent(self) -> None:
        from apcore.errors import ModuleTimeoutError

        module = self.SlowModule(
            resources={"timeout": 10},
            annotations={"idempotent": True},
        )
        with pytest.raises(ModuleTimeoutError) as exc_info:
            await self._run(module)
        assert exc_info.value.retryable is True

    async def test_annotations_extra_resources_is_honoured(self) -> None:
        from apcore.errors import ModuleTimeoutError

        module = self.SlowModule(annotations=self._annotations({"timeout": 10}))
        with pytest.raises(ModuleTimeoutError) as exc_info:
            await self._run(module)
        assert exc_info.value.timeout_ms == 10

    async def test_dict_annotations_resources_is_honoured(self) -> None:
        from apcore.errors import ModuleTimeoutError

        module = self.SlowModule(annotations={"resources": {"timeout": 10}})
        with pytest.raises(ModuleTimeoutError):
            await self._run(module)

    async def test_direct_resources_wins_over_annotations(self) -> None:
        module = self.SlowModule(
            resources={"timeout": 0},
            annotations=self._annotations({"timeout": 10}),
        )
        # `0` means "no per-module limit" and must beat the annotation's 10 ms,
        # so the module is allowed to run past 10 ms. Cancel rather than wait 5 s.
        import asyncio

        task = asyncio.ensure_future(self._run(module))
        await asyncio.sleep(0.05)
        assert not task.done()
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task

    @pytest.mark.parametrize(
        ("case", "module_kwargs"),
        [
            ("direct", {"resources": {"timeout": -1}}),
            ("annotations", {"annotations": None}),
        ],
    )
    async def test_negative_timeout_is_invalid_input(self, case: str, module_kwargs: dict[str, Any]) -> None:
        if case == "annotations":
            module_kwargs = {"annotations": self._annotations({"timeout": -1})}
        module = self.SlowModule(**module_kwargs)
        with pytest.raises(InvalidInputError) as exc_info:
            await self._run(module)
        assert exc_info.value.code == "GENERAL_INVALID_INPUT"

    async def test_non_numeric_timeout_falls_back_to_default(self) -> None:
        # A non-numeric value is "not declared" (parity with apcore-typescript
        # `_readModuleTimeoutMs`), not an error and not a zero timeout.
        module = self.SlowModule(resources={"timeout": "soon"})
        import asyncio

        task = asyncio.ensure_future(self._run(module))
        await asyncio.sleep(0.05)
        assert not task.done()
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task

    async def test_bool_timeout_is_not_read_as_an_integer(self) -> None:
        # `bool` subclasses `int` in Python; `True` must not become a 1 ms timeout.
        module = self.SlowModule(resources={"timeout": True})
        import asyncio

        task = asyncio.ensure_future(self._run(module))
        await asyncio.sleep(0.05)
        assert not task.done()
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task


class TestApprovalTokenStripping:
    """PROTOCOL_SPEC §7.4: "The ``_approval_token`` key **MUST** be removed from
    arguments before passing to subsequent steps." — unconditionally.

    Python stripped it only inside the gated-and-handler-configured branch, so
    on every early return (module not gated, no handler configured, policy skip)
    the protocol key travelled on into input validation — where a contract with
    ``additionalProperties: false`` rejects it as an undeclared key — and into
    the module's own ``execute()``. And the strip was ``ctx.inputs.pop()``,
    which mutates the *caller's* dict: ``PipelineContext`` holds the very object
    the caller passed to ``call()``. apcore-typescript extracts the token as the
    first statement of ``execute()``, before any early return, and rebuilds the
    inputs rather than mutating them.
    """

    @staticmethod
    def _ctx(inputs: dict[str, Any], annotations: Any = None) -> PipelineContext:
        return _make_ctx(
            inputs=inputs,
            context=FakeContext(),
            module=FakeModule(annotations=annotations),
        )

    async def test_stripped_when_the_module_is_not_gated(self) -> None:
        ctx = self._ctx({"x": 1, "_approval_token": "tok"})
        result = await BuiltinApprovalGate().execute(ctx)
        assert result.action == "continue"
        assert "_approval_token" not in ctx.inputs
        assert ctx.inputs == {"x": 1}

    async def test_stripped_when_gated_but_no_handler_is_configured(self) -> None:
        from apcore.module import ModuleAnnotations

        ctx = self._ctx(
            {"x": 1, "_approval_token": "tok"},
            annotations=ModuleAnnotations(requires_approval=True),
        )
        result = await BuiltinApprovalGate(handler=None).execute(ctx)
        assert result.action == "continue"
        assert "_approval_token" not in ctx.inputs

    async def test_the_callers_own_dict_is_never_mutated(self) -> None:
        caller_inputs = {"x": 1, "_approval_token": "tok"}
        ctx = self._ctx(caller_inputs)
        await BuiltinApprovalGate().execute(ctx)
        assert caller_inputs == {"x": 1, "_approval_token": "tok"}
        assert ctx.inputs is not caller_inputs

    async def test_absent_token_leaves_the_inputs_object_alone(self) -> None:
        caller_inputs = {"x": 1}
        ctx = self._ctx(caller_inputs)
        await BuiltinApprovalGate().execute(ctx)
        assert ctx.inputs is caller_inputs

    async def test_token_still_reaches_check_approval_on_the_gated_path(self) -> None:
        from apcore.approval import ApprovalResult
        from apcore.module import ModuleAnnotations

        from apcore.context import Context

        handler = MagicMock()
        handler.check_approval = AsyncMock(return_value=ApprovalResult(status="approved"))
        # A real Context: the gated path emits an approval audit, which reads
        # `context.data`.
        ctx = _make_ctx(
            inputs={"x": 1, "_approval_token": "tok"},
            context=Context.create(),
            module=FakeModule(annotations=ModuleAnnotations(requires_approval=True)),
        )
        result = await BuiltinApprovalGate(handler=handler).execute(ctx)
        assert result.action == "continue"
        handler.check_approval.assert_awaited_once_with("tok")
        assert "_approval_token" not in ctx.inputs

    async def test_non_string_token_is_rejected_before_reaching_the_handler(self) -> None:
        handler = MagicMock()
        handler.check_approval = AsyncMock()
        ctx = self._ctx({"_approval_token": 123})
        with pytest.raises(InvalidInputError) as exc_info:
            await BuiltinApprovalGate(handler=handler).execute(ctx)
        assert exc_info.value.code == "GENERAL_INVALID_INPUT"
        handler.check_approval.assert_not_awaited()
