"""Tests for the Executor class."""

from __future__ import annotations

import threading
import time
from typing import Any

import pytest
from pydantic import BaseModel, Field

from apcore.acl import ACL, ACLRule
from apcore.config import Config
from apcore.context import Context
from apcore.errors import (
    ACLDeniedError,
    CallDepthExceededError,
    CallFrequencyExceededError,
    CircularCallError,
    InvalidInputError,
    ModuleExecuteError,
    ModuleNotFoundError,
    ModuleTimeoutError,
    SchemaValidationError,
)
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.registry import Registry


# === Test Schemas ===


class SimpleInput(BaseModel):
    name: str
    age: int = 0


class SimpleOutput(BaseModel):
    greeting: str


class SensitiveInput(BaseModel):
    username: str
    password: str = Field(json_schema_extra={"x-sensitive": True})


# === Mock Module ===


class MockModule:
    """Simple mock module with Pydantic schemas."""

    def __init__(
        self,
        output: dict[str, Any] | None = None,
        input_schema: type[BaseModel] | None = SimpleInput,
        output_schema: type[BaseModel] | None = SimpleOutput,
        side_effect: Exception | None = None,
    ) -> None:
        self.input_schema = input_schema
        self.output_schema = output_schema
        self._output = output or {"greeting": "hello"}
        self._side_effect = side_effect
        self.execute_calls: list[tuple[dict[str, Any], Any]] = []

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        self.execute_calls.append((inputs, context))
        if self._side_effect:
            raise self._side_effect
        return self._output


def _make_executor(
    module: Any = None,
    module_id: str = "test.module",
    acl: ACL | None = None,
    config: Config | None = None,
    middlewares: list[Middleware] | None = None,
) -> Executor:
    """Helper to create an Executor with a registered module."""
    reg = Registry()
    if module is not None:
        reg.register(module_id, module)
    return Executor(registry=reg, acl=acl, config=config, middlewares=middlewares)


# === Constructor Tests ===


class TestExecutorInit:
    """Tests for Executor construction."""

    def test_init_registry_only(self) -> None:
        """Executor(registry) initializes with defaults."""
        reg = Registry()
        ex = Executor(registry=reg)
        assert ex.registry is reg
        assert ex._default_timeout == 30000
        assert ex._global_timeout == 60000
        assert ex._max_call_depth == 32
        assert ex._max_module_repeat == 3
        assert ex.middlewares == []

    def test_init_with_middlewares(self) -> None:
        """Executor with middlewares passes them to MiddlewareManager."""
        mw1 = Middleware()
        mw2 = Middleware()
        reg = Registry()
        ex = Executor(registry=reg, middlewares=[mw1, mw2])
        assert len(ex.middlewares) == 2

    def test_init_with_acl(self) -> None:
        """Executor with ACL stores it for use in call()."""
        reg = Registry()
        acl = ACL(rules=[], default_effect="allow")
        ex = Executor(registry=reg, acl=acl)
        assert ex._acl is acl

    def test_init_with_config(self) -> None:
        """Executor reads settings from config."""
        config = Config(data={"executor": {"default_timeout": 5000, "max_call_depth": 10}})
        reg = Registry()
        ex = Executor(registry=reg, config=config)
        assert ex._default_timeout == 5000
        assert ex._max_call_depth == 10
        assert ex._global_timeout == 60000  # not in config, uses default

    def test_config_overrides_defaults(self) -> None:
        """Config values override hardcoded defaults."""
        config = Config(
            data={
                "executor": {
                    "default_timeout": 5000,
                    "global_timeout": 10000,
                    "max_call_depth": 16,
                    "max_module_repeat": 5,
                }
            }
        )
        reg = Registry()
        ex = Executor(registry=reg, config=config)
        assert ex._default_timeout == 5000
        assert ex._global_timeout == 10000
        assert ex._max_call_depth == 16
        assert ex._max_module_repeat == 5


# === call() -- 10-Step Flow Tests ===


class TestCallFlow:
    """Tests for the 10-step call pipeline."""

    def test_creates_context_when_none(self) -> None:
        """Step 1: Auto-creates context when None."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        ex.call("test.module", {"name": "Alice"})
        assert len(mod.execute_calls) == 1
        _, ctx = mod.execute_calls[0]
        assert ctx.executor is ex
        assert "test.module" in ctx.call_chain

    def test_derives_child_context(self) -> None:
        """Step 1: Derives child context when context provided."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        parent_ctx = Context.create()
        ex.call("test.module", {"name": "Alice"}, context=parent_ctx)
        _, ctx = mod.execute_calls[0]
        assert "test.module" in ctx.call_chain
        assert ctx.trace_id == parent_ctx.trace_id

    def test_depth_exceeded(self) -> None:
        """Step 2: Raises CallDepthExceededError when depth exceeds max."""
        mod = MockModule()
        config = Config(data={"executor": {"max_call_depth": 2}})
        ex = _make_executor(module=mod, config=config)
        # Create a context with a deep call chain
        ctx = Context.create()
        ctx.call_chain = ["a", "b", "c"]
        with pytest.raises(CallDepthExceededError):
            ex.call("test.module", {"name": "Alice"}, context=ctx)

    def test_circular_detection(self) -> None:
        """Step 2: Raises CircularCallError for a->b->a."""
        mod = MockModule()
        ex = _make_executor(module=mod, module_id="a")
        ctx = Context.create()
        ctx.call_chain = ["a", "b"]
        with pytest.raises(CircularCallError):
            ex.call("a", {"name": "Alice"}, context=ctx)

    def test_circular_detection_longer_chain(self) -> None:
        """Step 2: Raises CircularCallError for a->b->c->a."""
        mod = MockModule()
        ex = _make_executor(module=mod, module_id="a")
        ctx = Context.create()
        ctx.call_chain = ["a", "b", "c"]
        with pytest.raises(CircularCallError):
            ex.call("a", {"name": "Alice"}, context=ctx)

    def test_self_call_not_circular(self) -> None:
        """Step 2: Self-call a->a is NOT circular, governed by frequency."""
        mod = MockModule()
        ex = _make_executor(module=mod, module_id="a")
        ctx = Context.create()
        ctx.call_chain = ["a"]
        # Self-call should not raise CircularCallError; frequency limit governs it
        ex.call("a", {"name": "Alice"}, context=ctx)
        assert len(mod.execute_calls) == 1

    def test_frequency_exceeded(self) -> None:
        """Step 2: Raises CallFrequencyExceededError when count > max_repeat."""
        mod = MockModule()
        config = Config(data={"executor": {"max_module_repeat": 2}})
        ex = _make_executor(module=mod, module_id="a", config=config)
        ctx = Context.create()
        ctx.call_chain = ["a", "a", "a"]
        with pytest.raises(CallFrequencyExceededError):
            ex.call("a", {"name": "Alice"}, context=ctx)

    def test_module_not_found(self) -> None:
        """Step 3: Raises ModuleNotFoundError for unknown module."""
        ex = _make_executor()
        with pytest.raises(ModuleNotFoundError):
            ex.call("unknown.module", {"name": "Alice"})

    def test_acl_denied(self) -> None:
        """Step 4: ACL deny raises ACLDeniedError."""
        mod = MockModule()
        acl = ACL(rules=[], default_effect="deny")
        ex = _make_executor(module=mod, acl=acl)
        with pytest.raises(ACLDeniedError):
            ex.call("test.module", {"name": "Alice"})

    def test_acl_allowed(self) -> None:
        """Step 4: ACL allow proceeds."""
        mod = MockModule()
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")])
        ex = _make_executor(module=mod, acl=acl)
        result = ex.call("test.module", {"name": "Alice"})
        assert result == {"greeting": "hello"}

    def test_no_acl_skips_check(self) -> None:
        """Step 4: No ACL configured skips check."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        result = ex.call("test.module", {"name": "Alice"})
        assert result == {"greeting": "hello"}

    def test_validates_inputs(self) -> None:
        """Step 5: Valid inputs proceed."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        result = ex.call("test.module", {"name": "Alice", "age": 30})
        assert result == {"greeting": "hello"}

    def test_invalid_inputs(self) -> None:
        """Step 5: Invalid inputs raise SchemaValidationError."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        with pytest.raises(SchemaValidationError):
            ex.call("test.module", {"age": "not_a_number"})

    def test_sets_redacted_inputs(self) -> None:
        """Step 5: Sets context.redacted_inputs after validation."""
        mod = MockModule(input_schema=SensitiveInput)
        ex = _make_executor(module=mod)
        ex.call("test.module", {"username": "alice", "password": "secret123"})
        _, ctx = mod.execute_calls[0]
        assert ctx.redacted_inputs is not None
        assert ctx.redacted_inputs["username"] == "alice"
        assert ctx.redacted_inputs["password"] == "***REDACTED***"

    def test_middleware_before_called(self) -> None:
        """Step 6: Middleware before chain is called."""
        mod = MockModule()
        before_calls: list[str] = []

        class TrackBefore(Middleware):
            def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                before_calls.append(module_id)
                return None

        ex = _make_executor(module=mod, middlewares=[TrackBefore()])
        ex.call("test.module", {"name": "Alice"})
        assert before_calls == ["test.module"]

    def test_middleware_before_modifies_inputs(self) -> None:
        """Step 6: Middleware before can modify inputs."""
        mod = MockModule()

        class ModifyBefore(Middleware):
            def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                return {**inputs, "name": "Modified"}

        ex = _make_executor(module=mod, middlewares=[ModifyBefore()])
        ex.call("test.module", {"name": "Alice"})
        assert mod.execute_calls[0][0]["name"] == "Modified"

    def test_executes_module(self) -> None:
        """Step 7: Module.execute() is called."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        result = ex.call("test.module", {"name": "Alice"})
        assert len(mod.execute_calls) == 1
        assert result == {"greeting": "hello"}

    def test_validates_output(self) -> None:
        """Step 8: Invalid output raises SchemaValidationError."""
        mod = MockModule(output={"invalid_key": "value"})
        ex = _make_executor(module=mod)
        with pytest.raises(SchemaValidationError):
            ex.call("test.module", {"name": "Alice"})

    def test_middleware_after_called(self) -> None:
        """Step 9: Middleware after chain is called."""
        mod = MockModule()
        after_calls: list[str] = []

        class TrackAfter(Middleware):
            def after(
                self,
                module_id: str,
                inputs: dict[str, Any],
                output: dict[str, Any],
                context: Context,
            ) -> dict[str, Any] | None:
                after_calls.append(module_id)
                return None

        ex = _make_executor(module=mod, middlewares=[TrackAfter()])
        ex.call("test.module", {"name": "Alice"})
        assert after_calls == ["test.module"]

    def test_returns_output(self) -> None:
        """Step 10: Returns final output dict."""
        mod = MockModule(output={"greeting": "world"})
        ex = _make_executor(module=mod)
        result = ex.call("test.module", {"name": "Alice"})
        assert result == {"greeting": "world"}


# === Error Handling Tests ===


class TestCallErrorHandling:
    """Tests for error handling in call()."""

    def test_exception_triggers_on_error(self) -> None:
        """When module.execute() raises, on_error chain is called."""
        mod = MockModule(side_effect=RuntimeError("boom"))
        on_error_calls: list[str] = []

        class ErrorHandler(Middleware):
            def on_error(
                self,
                module_id: str,
                inputs: dict[str, Any],
                error: Exception,
                context: Context,
            ) -> dict[str, Any] | None:
                on_error_calls.append(str(error))
                return None

        ex = _make_executor(module=mod, middlewares=[ErrorHandler()])
        with pytest.raises(ModuleExecuteError, match="boom"):
            ex.call("test.module", {"name": "Alice"})
        assert len(on_error_calls) == 1

    def test_on_error_recovery(self) -> None:
        """on_error recovery dict used as return value."""
        mod = MockModule(side_effect=RuntimeError("boom"))

        class RecoveryHandler(Middleware):
            def on_error(
                self,
                module_id: str,
                inputs: dict[str, Any],
                error: Exception,
                context: Context,
            ) -> dict[str, Any] | None:
                return {"greeting": "recovered"}

        ex = _make_executor(module=mod, middlewares=[RecoveryHandler()])
        result = ex.call("test.module", {"name": "Alice"})
        assert result == {"greeting": "recovered"}

    def test_on_error_no_recovery(self) -> None:
        """on_error returns None -> original exception re-raised."""
        mod = MockModule(side_effect=RuntimeError("boom"))

        class NoRecovery(Middleware):
            def on_error(
                self,
                module_id: str,
                inputs: dict[str, Any],
                error: Exception,
                context: Context,
            ) -> dict[str, Any] | None:
                return None

        ex = _make_executor(module=mod, middlewares=[NoRecovery()])
        with pytest.raises(ModuleExecuteError, match="boom"):
            ex.call("test.module", {"name": "Alice"})

    def test_middleware_chain_error(self) -> None:
        """MiddlewareChainError extracts executed_middlewares for on_error."""
        mod = MockModule()
        on_error_calls: list[str] = []

        class FailBefore(Middleware):
            def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> dict[str, Any] | None:
                raise RuntimeError("before failed")

            def on_error(
                self,
                module_id: str,
                inputs: dict[str, Any],
                error: Exception,
                context: Context,
            ) -> dict[str, Any] | None:
                on_error_calls.append("handled")
                return None

        ex = _make_executor(module=mod, middlewares=[FailBefore()])
        with pytest.raises(ModuleExecuteError, match="before failed"):
            ex.call("test.module", {"name": "Alice"})
        assert len(on_error_calls) == 1


# === Edge Cases Tests ===


class TestCallEdgeCases:
    """Tests for call() edge cases."""

    def test_empty_module_id(self) -> None:
        """module_id='' raises InvalidInputError."""
        ex = _make_executor()
        with pytest.raises(InvalidInputError):
            ex.call("", {"name": "Alice"})

    def test_none_inputs_treated_as_empty(self) -> None:
        """inputs=None is treated as {}."""
        mod = MockModule(input_schema=None, output_schema=None, output={"result": "ok"})
        ex = _make_executor(module=mod)
        result = ex.call("test.module", None)
        assert result == {"result": "ok"}
        assert mod.execute_calls[0][0] == {}

    def test_dict_schema_call(self) -> None:
        """call() works when module schemas are plain dicts (not Pydantic models)."""

        class DictSchemaModule:
            description = "Dict schema module"
            input_schema = {"type": "object", "properties": {"x": {"type": "integer"}}}
            output_schema = {"type": "object", "properties": {"y": {"type": "integer"}}}
            version = "1.0.0"
            tags: list[str] = []

            def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
                return {"y": inputs.get("x", 0) + 1}

        ex = _make_executor(module=DictSchemaModule())
        result = ex.call("test.module", {"x": 5})
        assert result == {"y": 6}

    @pytest.mark.asyncio
    async def test_dict_schema_call_async(self) -> None:
        """call_async() works when module schemas are plain dicts."""

        class DictSchemaModule:
            description = "Dict schema module"
            input_schema = {"type": "object", "properties": {"x": {"type": "integer"}}}
            output_schema = {"type": "object", "properties": {"y": {"type": "integer"}}}
            version = "1.0.0"
            tags: list[str] = []

            async def execute(self, inputs: dict[str, Any], context: Any) -> dict[str, Any]:
                return {"y": inputs.get("x", 0) + 1}

        ex = _make_executor(module=DictSchemaModule())
        result = await ex.call_async("test.module", {"x": 5})
        assert result == {"y": 6}


# === validate() Tests ===


class TestValidate:
    """Tests for the validate() method."""

    def test_valid_inputs(self) -> None:
        """validate() returns PreflightResult(valid=True) for valid inputs."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice", "age": 30})
        assert result.valid is True
        assert result.errors == []

    def test_invalid_inputs(self) -> None:
        """validate() returns PreflightResult(valid=False) for invalid inputs."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"age": "not_a_number"})
        assert result.valid is False
        assert len(result.errors) > 0

    def test_module_not_found(self) -> None:
        """validate() returns PreflightResult with module_lookup failure for unknown module."""
        ex = _make_executor()
        result = ex.validate("unknown.module", {})
        assert result.valid is False
        assert any(e.get("code") == "MODULE_NOT_FOUND" for e in result.errors)

    def test_validate_with_foreign_bound_context_does_not_raise(self) -> None:
        """A-D-008: validate() is non-throwing — a Context already bound to a
        DIFFERENT executor returns PreflightResult(valid=False) with a failed
        ``executor_binding`` check instead of raising ContextBindingError."""
        mod = MockModule()
        ex_a = _make_executor(module=mod)
        ex_b = _make_executor(module=MockModule())

        ctx = Context.create()
        ctx._bind_executor(ex_b)  # bind to a different executor

        result = ex_a.validate("test.module", {"name": "Alice", "age": 30}, context=ctx)
        assert result.valid is False
        assert any(c.check == "executor_binding" and c.passed is False for c in result.checks)

    def test_acl_check(self) -> None:
        """validate() checks ACL and reports denial without executing."""
        mod = MockModule()
        acl = ACL(rules=[], default_effect="deny")
        ex = _make_executor(module=mod, acl=acl)
        result = ex.validate("test.module", {"name": "Alice"})
        assert result.valid is False
        assert any(e.get("code") == "ACL_DENIED" for e in result.errors)
        assert len(mod.execute_calls) == 0

    def test_skips_execution(self) -> None:
        """validate() does NOT run middleware or execution."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice", "age": 30})
        assert result.valid is True
        assert len(mod.execute_calls) == 0

    def test_preflight_with_warnings(self) -> None:
        """validate() includes module_preflight check with warnings when preflight() returns warnings."""

        class PreflightModule(MockModule):
            def preflight(self, inputs: dict[str, Any], context: Any) -> list[str]:
                return ["disk space low", "rate limit approaching"]

        mod = PreflightModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice"})
        assert result.valid is True
        preflight_checks = [c for c in result.checks if c.check == "module_preflight"]
        assert len(preflight_checks) == 1
        assert preflight_checks[0].passed is True
        assert preflight_checks[0].warnings == ["disk space low", "rate limit approaching"]

    def test_no_preflight_method(self) -> None:
        """validate() produces no module_preflight check when module lacks preflight()."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice"})
        assert result.valid is True
        preflight_checks = [c for c in result.checks if c.check == "module_preflight"]
        assert len(preflight_checks) == 0

    def test_preflight_exception_produces_warning(self) -> None:
        """validate() gracefully handles preflight() raising an exception."""

        class FailingPreflightModule(MockModule):
            def preflight(self, inputs: dict[str, Any], context: Any) -> list[str]:
                raise RuntimeError("connection refused")

        mod = FailingPreflightModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice"})
        assert result.valid is True
        preflight_checks = [c for c in result.checks if c.check == "module_preflight"]
        assert len(preflight_checks) == 1
        assert preflight_checks[0].passed is True
        assert len(preflight_checks[0].warnings) == 1
        assert "RuntimeError" in preflight_checks[0].warnings[0]
        assert "connection refused" in preflight_checks[0].warnings[0]

    # === Module.preview() integration (PROTOCOL_SPEC §5.6 / §12.8) ===

    def test_no_preview_method_yields_empty_predicted_changes(self) -> None:
        """validate() leaves predicted_changes empty when module lacks preview()."""
        from apcore.module import PreflightResult

        mod = MockModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice"})
        assert isinstance(result, PreflightResult)
        assert result.valid is True
        assert result.predicted_changes == []
        assert all(c.check != "module_preview" for c in result.checks)

    def test_preview_returns_none_yields_empty_predicted_changes(self) -> None:
        """D-141: null preview records no check and leaves predicted_changes empty."""
        from apcore.module import PreviewResult

        class NonePreviewModule(MockModule):
            def preview(self, inputs: dict[str, Any], context: Any) -> PreviewResult | None:
                return None

        mod = NonePreviewModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice"})
        assert result.valid is True
        assert result.predicted_changes == []
        preview_checks = [c for c in result.checks if c.check == "module_preview"]
        assert preview_checks == []

    def test_preview_returns_result_populates_predicted_changes(self) -> None:
        """validate() folds PreviewResult.changes into PreflightResult.predicted_changes."""
        from apcore.module import Change, PreviewResult

        class PreviewModule(MockModule):
            def preview(self, inputs: dict[str, Any], context: Any) -> PreviewResult:
                return PreviewResult(
                    changes=[
                        Change(
                            action="delete",
                            target=f"users.{inputs['name']}",
                            summary=f"Permanently delete user {inputs['name']}",
                            before={"name": inputs["name"]},
                        ),
                    ]
                )

        mod = PreviewModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice"})
        assert result.valid is True
        assert len(result.predicted_changes) == 1
        change = result.predicted_changes[0]
        assert change.action == "delete"
        assert change.target == "users.Alice"
        assert change.summary == "Permanently delete user Alice"
        assert change.before == {"name": "Alice"}
        preview_checks = [c for c in result.checks if c.check == "module_preview"]
        assert len(preview_checks) == 1
        assert preview_checks[0].passed is True

    def test_preview_async_is_awaited(self) -> None:
        """validate() awaits an async preview() implementation."""
        from apcore.module import Change, PreviewResult

        class AsyncPreviewModule(MockModule):
            async def preview(self, inputs: dict[str, Any], context: Any) -> PreviewResult:
                return PreviewResult(changes=[Change(action="send", target="smtp", summary="send email")])

        mod = AsyncPreviewModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice"})
        assert result.valid is True
        assert len(result.predicted_changes) == 1
        assert result.predicted_changes[0].action == "send"

    def test_preview_exception_produces_warning(self) -> None:
        """validate() gracefully handles preview() raising; validation still succeeds."""

        class FailingPreviewModule(MockModule):
            def preview(self, inputs: dict[str, Any], context: Any) -> Any:
                raise RuntimeError("preview backend unreachable")

        mod = FailingPreviewModule()
        ex = _make_executor(module=mod)
        result = ex.validate("test.module", {"name": "Alice"})
        assert result.valid is True
        assert result.predicted_changes == []
        preview_checks = [c for c in result.checks if c.check == "module_preview"]
        assert len(preview_checks) == 1
        assert preview_checks[0].passed is True
        assert len(preview_checks[0].warnings) == 1
        assert "RuntimeError" in preview_checks[0].warnings[0]
        assert "preview backend unreachable" in preview_checks[0].warnings[0]


class TestChangeExtensionKeys:
    """protocol-spec §12.8.5.1: Change.x-* extension fields cross-SDK encoding."""

    def test_change_accepts_x_prefixed_extras(self) -> None:
        from apcore.module import Change

        ch = Change(
            action="write",
            target="db.row.42",
            summary="Set value",
            **{"x-correlation-id": "abc-123", "x-priority": 5},
        )
        dumped = ch.model_dump()
        assert dumped["x-correlation-id"] == "abc-123"
        assert dumped["x-priority"] == 5

    def test_change_rejects_unknown_non_x_keys(self) -> None:
        from apcore.module import Change
        from pydantic import ValidationError

        with pytest.raises(ValidationError) as exc_info:
            Change(
                action="write",
                target="db.row.42",
                summary="Set value",
                random_key="boom",
            )
        # The model-validator wraps the message — assert the key shows up.
        assert "random_key" in str(exc_info.value)


# === Middleware Management Tests ===


class TestMiddlewareManagement:
    """Tests for use(), use_before(), use_after(), remove()."""

    def test_use_returns_self(self) -> None:
        """use() returns self for chaining."""
        ex = _make_executor()
        mw = Middleware()
        result = ex.use(mw)
        assert result is ex
        assert mw in ex.middlewares

    def test_use_before_wraps_callback(self) -> None:
        """use_before() creates BeforeMiddleware and adds it, returns self."""
        ex = _make_executor()
        result = ex.use_before(lambda mid, inp, ctx: None)
        assert result is ex
        assert len(ex.middlewares) == 1

    def test_use_after_wraps_callback(self) -> None:
        """use_after() creates AfterMiddleware and adds it, returns self."""
        ex = _make_executor()
        result = ex.use_after(lambda mid, inp, out, ctx: None)
        assert result is ex
        assert len(ex.middlewares) == 1

    def test_remove_delegates(self) -> None:
        """remove() delegates to middleware manager."""
        ex = _make_executor()
        mw = Middleware()
        ex.use(mw)
        assert ex.remove(mw) is True
        assert len(ex.middlewares) == 0


# === Timeout Tests ===


class TestTimeout:
    """Tests for timeout enforcement."""

    def test_module_timeout(self) -> None:
        """Module execution that exceeds timeout raises ModuleTimeoutError."""

        class SlowModule:
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                time.sleep(2)
                return {"result": "slow"}

        config = Config(data={"executor": {"default_timeout": 100}})  # 100ms
        ex = _make_executor(module=SlowModule(), config=config)
        with pytest.raises(ModuleTimeoutError):
            ex.call("test.module", {})

    def test_timeout_zero_disables(self) -> None:
        """timeout=0 disables timeout enforcement (no thread wrapping)."""

        class QuickModule:
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {"result": "quick"}

        config = Config(data={"executor": {"default_timeout": 0}})
        ex = _make_executor(module=QuickModule(), config=config)
        assert ex._default_timeout == 0
        result = ex.call("test.module", {})
        assert result == {"result": "quick"}

    def test_timeout_negative_raises(self) -> None:
        """timeout < 0 raises InvalidInputError at construction time."""
        from apcore.errors import InvalidInputError

        class QuickModule:
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return {"result": "quick"}

        config = Config(data={"executor": {"default_timeout": -100}})
        with pytest.raises(InvalidInputError):
            _make_executor(module=QuickModule(), config=config)

    def test_timeout_cooperative_cancel(self) -> None:
        """Module that checks cancel_token should exit gracefully during grace period."""

        class CooperativeModule:
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                cancel_token = getattr(context, "cancel_token", None)
                # Simulate long work with periodic cancel check
                for _ in range(100):
                    if cancel_token and cancel_token.is_cancelled:
                        return {"result": "cancelled_gracefully"}
                    time.sleep(0.05)
                return {"result": "completed"}

        config = Config(data={"executor": {"default_timeout": 100}})  # 100ms timeout
        ex = _make_executor(module=CooperativeModule(), config=config)
        # Module should timeout but the CancelToken should be set
        # The module will check cancel_token during grace period and return
        with pytest.raises(ModuleTimeoutError):
            ex.call("test.module", {})


# === Thread Safety Tests ===


class TestPipelineThreadSafety:
    """Tests for thread-safe pipeline execution."""

    def test_concurrent_sync_calls_no_error(self) -> None:
        """Concurrent sync call() invocations should not raise."""
        mod = MockModule()
        ex = _make_executor(module=mod)
        errors: list[Exception] = []

        def caller() -> None:
            try:
                for _ in range(20):
                    ex.call("test.module", {"name": "test"})
            except Exception as e:
                errors.append(e)

        threads = [threading.Thread(target=caller) for _ in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == []

    def test_slow_module_timeout_via_pipeline(self) -> None:
        """Slow module should timeout through pipeline BuiltinExecute."""

        class SlowModule:
            input_schema = None
            output_schema = None

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                time.sleep(5)
                return {"result": "slow"}

        config = Config(data={"executor": {"default_timeout": 50}})
        ex = _make_executor(module=SlowModule(), config=config)
        with pytest.raises(ModuleTimeoutError):
            ex.call("test.module", {})
