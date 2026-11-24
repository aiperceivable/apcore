"""Full lifecycle integration tests for the execution pipeline.

Tests verify the COMPLETE pipeline with ACL + Approval + Middleware + Schema
validation all enabled simultaneously. These are NOT isolated unit tests --
they exercise register -> validate -> execute -> result flows.
"""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import BaseModel

from apcore.acl import ACL, ACLRule
from apcore.approval import (
    ApprovalRequest,
    ApprovalResult,
    CallbackApprovalHandler,
)
from apcore.context import Context, Identity
from apcore.errors import (
    ACLDeniedError,
    ApprovalDeniedError,
    ModuleError,
    SchemaValidationError,
)
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.module import ModuleAnnotations
from apcore.registry import Registry


# ---------------------------------------------------------------------------
# Schemas
# ---------------------------------------------------------------------------


class GreetInput(BaseModel):
    """Input schema for the greet module."""

    name: str


class GreetOutput(BaseModel):
    """Output schema for the greet module."""

    message: str


class StrictInput(BaseModel):
    """Input schema that requires both fields."""

    name: str
    age: int


class StrictOutput(BaseModel):
    """Output schema for strict module."""

    result: str


# ---------------------------------------------------------------------------
# Tracking middleware
# ---------------------------------------------------------------------------


class TrackingMiddleware(Middleware):
    """Middleware that records before/after/on_error calls for assertions."""

    def __init__(self) -> None:
        self.before_calls: list[str] = []
        self.after_calls: list[str] = []
        self.error_calls: list[tuple[str, Exception]] = []

    def before(
        self,
        module_id: str,
        inputs: dict[str, Any],
        context: Any,
    ) -> dict[str, Any] | None:
        del inputs, context  # unused
        self.before_calls.append(module_id)
        return None

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Any,
    ) -> dict[str, Any] | None:
        del inputs, output, context  # unused
        self.after_calls.append(module_id)
        return None

    def on_error(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Any,
    ) -> dict[str, Any] | None:
        del inputs, context  # unused
        self.error_calls.append((module_id, error))
        return None


# ---------------------------------------------------------------------------
# Module implementations
# ---------------------------------------------------------------------------


class ApprovalGreetModule:
    """Module that requires approval and produces a greeting."""

    input_schema = GreetInput
    output_schema = GreetOutput
    annotations = ModuleAnnotations(requires_approval=True)
    description = "Greeting module that requires approval"

    def execute(self, inputs: dict[str, Any], _context: Context) -> dict[str, Any]:
        return {"message": f"Hello, {inputs['name']}!"}


class SimpleGreetModule:
    """Module without approval requirement."""

    input_schema = GreetInput
    output_schema = GreetOutput
    annotations = ModuleAnnotations(requires_approval=False)
    description = "Simple greeting module"

    def execute(self, inputs: dict[str, Any], _context: Context) -> dict[str, Any]:
        return {"message": f"Hi, {inputs['name']}!"}


class CallerModule:
    """Module A that calls module B via context.executor."""

    input_schema = GreetInput
    output_schema = GreetOutput
    annotations = ModuleAnnotations()
    description = "Caller module"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        result = context.executor.call("mod.b", inputs, context)
        return result


class DataSharingCallerModule:
    """Module A that sets context.data and calls module B."""

    input_schema = GreetInput
    output_schema = GreetOutput
    annotations = ModuleAnnotations()
    description = "Data sharing caller module"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        context.data["shared_key"] = "shared_value"
        result = context.executor.call("mod.b.reader", inputs, context)
        return result


class DataReaderModule:
    """Module B that reads context.data set by module A."""

    input_schema = GreetInput
    output_schema = GreetOutput
    annotations = ModuleAnnotations()
    description = "Data reader module"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        value = context.data.get("shared_key", "NOT_FOUND")
        return {"message": f"Read: {value}"}


class ErrorModule:
    """Module that always raises a RuntimeError."""

    input_schema = GreetInput
    output_schema = GreetOutput
    annotations = ModuleAnnotations()
    description = "Error module"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        raise RuntimeError("intentional failure")


class ErrorCallerModule:
    """Module A that calls the error module B."""

    input_schema = GreetInput
    output_schema = GreetOutput
    annotations = ModuleAnnotations()
    description = "Error caller module"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return context.executor.call("mod.b.error", inputs, context)


class StrictModule:
    """Module with strict input schema for validation tests."""

    input_schema = StrictInput
    output_schema = StrictOutput
    annotations = ModuleAnnotations()
    description = "Strict schema module"

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"result": f"{inputs['name']} is {inputs['age']}"}


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_acl_allow(target: str) -> ACL:
    """Create an ACL that allows all callers to reach target pattern."""
    return ACL(
        rules=[ACLRule(callers=["*"], targets=[target], effect="allow")],
        default_effect="deny",
    )


def _make_acl_deny(target: str) -> ACL:
    """Create an ACL that denies all callers for target pattern."""
    return ACL(
        rules=[ACLRule(callers=["*"], targets=[target], effect="deny")],
        default_effect="deny",
    )


def _make_approving_handler() -> CallbackApprovalHandler:
    """Create a CallbackApprovalHandler that always approves."""

    async def _approve(request: ApprovalRequest) -> ApprovalResult:
        return ApprovalResult(status="approved", approved_by="test")

    return CallbackApprovalHandler(_approve)


def _make_denying_handler() -> CallbackApprovalHandler:
    """Create a CallbackApprovalHandler that always denies."""

    async def _deny(request: ApprovalRequest) -> ApprovalResult:
        return ApprovalResult(status="rejected", reason="denied by test")

    return CallbackApprovalHandler(_deny)


# ---------------------------------------------------------------------------
# Test class
# ---------------------------------------------------------------------------


class TestFullLifecycle:
    """Integration tests for the complete execution pipeline."""

    def test_full_pipeline_all_gates_enabled(self) -> None:
        """All gates pass: ACL allows, approval approves, middleware runs."""
        registry = Registry()
        registry.register("mod.greet", ApprovalGreetModule())

        middleware = TrackingMiddleware()
        acl = _make_acl_allow("mod.*")
        handler = _make_approving_handler()

        executor = Executor(
            registry=registry,
            middlewares=[middleware],
            acl=acl,
            approval_handler=handler,
        )

        result = executor.call("mod.greet", {"name": "Alice"})

        assert result == {"message": "Hello, Alice!"}
        assert "mod.greet" in middleware.before_calls
        assert "mod.greet" in middleware.after_calls
        assert len(middleware.error_calls) == 0

    def test_full_pipeline_acl_denies_before_approval(self) -> None:
        """ACL denies: approval handler and middleware must NOT be called."""
        registry = Registry()
        registry.register("mod.greet", ApprovalGreetModule())

        middleware = TrackingMiddleware()
        acl = _make_acl_deny("mod.*")
        handler = _make_approving_handler()

        executor = Executor(
            registry=registry,
            middlewares=[middleware],
            acl=acl,
            approval_handler=handler,
        )

        with pytest.raises(ACLDeniedError):
            executor.call("mod.greet", {"name": "Alice"})

        assert len(middleware.before_calls) == 0
        assert len(middleware.after_calls) == 0

    def test_full_pipeline_approval_denied_before_middleware(self) -> None:
        """Approval denies: middleware must NOT be called, module NOT executed."""
        registry = Registry()
        registry.register("mod.greet", ApprovalGreetModule())

        middleware = TrackingMiddleware()
        acl = _make_acl_allow("mod.*")
        handler = _make_denying_handler()

        executor = Executor(
            registry=registry,
            middlewares=[middleware],
            acl=acl,
            approval_handler=handler,
        )

        with pytest.raises(ApprovalDeniedError):
            executor.call("mod.greet", {"name": "Alice"})

        assert len(middleware.before_calls) == 0
        assert len(middleware.after_calls) == 0

    def test_nested_module_call_full_lifecycle(self) -> None:
        """Module A calls Module B; trace_id shared, middleware runs for both."""
        registry = Registry()
        registry.register("mod.a", CallerModule())
        registry.register("mod.b", SimpleGreetModule())

        middleware = TrackingMiddleware()
        acl = _make_acl_allow("mod.*")

        executor = Executor(
            registry=registry,
            middlewares=[middleware],
            acl=acl,
        )

        result = executor.call("mod.a", {"name": "Bob"})

        assert result == {"message": "Hi, Bob!"}
        assert "mod.a" in middleware.before_calls
        assert "mod.b" in middleware.before_calls
        assert "mod.a" in middleware.after_calls
        assert "mod.b" in middleware.after_calls

    def test_context_data_shared_between_nested_calls(self) -> None:
        """Module A sets context.data; Module B reads the same data (reference semantics)."""
        registry = Registry()
        registry.register("mod.a.writer", DataSharingCallerModule())
        registry.register("mod.b.reader", DataReaderModule())

        acl = _make_acl_allow("mod.*")
        executor = Executor(registry=registry, acl=acl)

        result = executor.call("mod.a.writer", {"name": "X"})

        assert result == {"message": "Read: shared_value"}

    def test_error_in_nested_call_propagates_correctly(self) -> None:
        """Module A calls Module B which errors; error wraps with trace_id, middleware on_error fires."""
        registry = Registry()
        registry.register("mod.a.caller", ErrorCallerModule())
        registry.register("mod.b.error", ErrorModule())

        middleware = TrackingMiddleware()
        acl = _make_acl_allow("mod.*")

        executor = Executor(
            registry=registry,
            middlewares=[middleware],
            acl=acl,
        )

        with pytest.raises(ModuleError) as exc_info:
            executor.call("mod.a.caller", {"name": "Z"})

        error = exc_info.value
        assert error.trace_id is not None
        # Middleware on_error should fire for the outer module
        assert len(middleware.error_calls) >= 1

    def test_schema_validation_in_full_pipeline(self) -> None:
        """Invalid inputs must raise SchemaValidationError.

        v0.17: middleware_before runs BEFORE input_validation (swap).
        So middleware IS called even when validation later fails.
        """
        registry = Registry()
        registry.register("mod.strict", StrictModule())

        middleware = TrackingMiddleware()
        acl = _make_acl_allow("mod.*")

        executor = Executor(
            registry=registry,
            middlewares=[middleware],
            acl=acl,
        )

        with pytest.raises(SchemaValidationError):
            executor.call("mod.strict", {"name": "Alice"})

        # v0.17: middleware_before runs before validation, so before IS called
        assert len(middleware.before_calls) == 1
        # after is NOT called because pipeline aborts at validation
        assert len(middleware.after_calls) == 0

    def test_full_pipeline_with_acl_conditions(self) -> None:
        """ACL rule with identity_types condition: allow/deny based on identity."""
        registry = Registry()
        registry.register("mod.cond", SimpleGreetModule())

        acl = ACL(
            rules=[
                ACLRule(
                    callers=["*"],
                    targets=["mod.cond"],
                    effect="allow",
                    conditions={"identity_types": ["service"]},
                ),
            ],
            default_effect="deny",
        )

        executor = Executor(registry=registry, acl=acl)

        # Call with matching identity type: should succeed
        service_ctx = Context.create(
            identity=Identity(id="svc-1", type="service"),
        )
        result = executor.call("mod.cond", {"name": "Svc"}, context=service_ctx)
        assert result == {"message": "Hi, Svc!"}

        # Call with non-matching identity type: should be denied
        user_ctx = Context.create(
            identity=Identity(id="user-1", type="user"),
        )
        with pytest.raises(ACLDeniedError):
            executor.call("mod.cond", {"name": "User"}, context=user_ctx)
