"""Integration tests for the approval system through the full executor pipeline."""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import BaseModel, ConfigDict

from apcore.acl import ACL, ACLRule
from apcore.approval import (
    AlwaysDenyHandler,
    ApprovalRequest,
    ApprovalResult,
    AutoApproveHandler,
    CallbackApprovalHandler,
)
from apcore.context import Context, Identity
from apcore.errors import ACLDeniedError, ApprovalDeniedError, ApprovalPendingError
from apcore.executor import Executor
from apcore.extensions import ExtensionManager
from apcore.middleware import Middleware
from apcore.module import ModuleAnnotations
from apcore.registry import Registry


# ---------------------------------------------------------------------------
# Module implementations
# ---------------------------------------------------------------------------


class PermissiveInput(BaseModel):
    model_config = ConfigDict(extra="allow")


class PermissiveOutput(BaseModel):
    model_config = ConfigDict(extra="allow")


class DestructiveModule:
    """A destructive module requiring approval."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = ModuleAnnotations(requires_approval=True, destructive=True)
    description = "Delete user data"
    tags = ["admin", "destructive"]

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"deleted": True, "user_id": inputs.get("user_id")}


class SafeModule:
    """A safe module not requiring approval."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = ModuleAnnotations(readonly=True)

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"data": "safe"}


class RecordingMiddleware(Middleware):
    """Records middleware calls for verification."""

    def __init__(self) -> None:
        self.before_calls: list[str] = []
        self.after_calls: list[str] = []
        self.error_calls: list[str] = []

    def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
        self.before_calls.append(module_id)
        return None

    def after(self, module_id: str, inputs: dict[str, Any], output: dict[str, Any], context: Context) -> None:
        self.after_calls.append(module_id)
        return None

    def on_error(self, module_id: str, inputs: dict[str, Any], error: Exception, context: Context) -> None:
        self.error_calls.append(module_id)
        return None


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def registry() -> Registry:
    reg = Registry()
    reg.register("admin.delete_user", DestructiveModule())
    reg.register("data.read", SafeModule())
    return reg


# ---------------------------------------------------------------------------
# Integration tests
# ---------------------------------------------------------------------------


class TestApprovalWithACL:
    def test_acl_deny_fires_before_approval(self, registry: Registry) -> None:
        """ACL deny (Step 4) prevents reaching approval gate (Step 4.5)."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["admin.*"], effect="deny")])

        approval_called = False

        async def track_handler(request: ApprovalRequest) -> ApprovalResult:
            nonlocal approval_called
            approval_called = True
            return ApprovalResult(status="approved")

        handler = CallbackApprovalHandler(track_handler)
        executor = Executor(registry=registry, acl=acl, approval_handler=handler)

        with pytest.raises(ACLDeniedError):
            executor.call("admin.delete_user", {"user_id": "123"})

        assert not approval_called, "Approval handler should not be called when ACL denies"

    def test_acl_allow_then_approval_deny(self, registry: Registry) -> None:
        """ACL allows but approval denies."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["admin.*"], effect="allow")])

        executor = Executor(registry=registry, acl=acl, approval_handler=AlwaysDenyHandler())
        with pytest.raises(ApprovalDeniedError):
            executor.call("admin.delete_user", {"user_id": "123"})

    def test_acl_allow_then_approval_approve(self, registry: Registry) -> None:
        """ACL allows and approval approves."""
        acl = ACL(rules=[ACLRule(callers=["*"], targets=["*"], effect="allow")])

        executor = Executor(registry=registry, acl=acl, approval_handler=AutoApproveHandler())
        result = executor.call("admin.delete_user", {"user_id": "123"})
        assert result["deleted"] is True


class TestApprovalWithMiddleware:
    def test_middleware_runs_after_approval(self, registry: Registry) -> None:
        """Middleware before/after still executes when approval passes."""
        mw = RecordingMiddleware()
        executor = Executor(
            registry=registry,
            middlewares=[mw],
            approval_handler=AutoApproveHandler(),
        )
        result = executor.call("admin.delete_user", {"user_id": "123"})
        assert result["deleted"] is True
        assert "admin.delete_user" in mw.before_calls
        assert "admin.delete_user" in mw.after_calls

    def test_middleware_not_reached_on_approval_deny(self, registry: Registry) -> None:
        """Middleware is NOT invoked when approval is denied (Step 4.5 < Step 6)."""
        mw = RecordingMiddleware()
        executor = Executor(
            registry=registry,
            middlewares=[mw],
            approval_handler=AlwaysDenyHandler(),
        )
        with pytest.raises(ApprovalDeniedError):
            executor.call("admin.delete_user", {"user_id": "123"})

        assert len(mw.before_calls) == 0, "Middleware should not run when approval denied"
        assert len(mw.after_calls) == 0

    def test_safe_module_with_middleware_and_handler(self, registry: Registry) -> None:
        """Safe module (no approval) still goes through middleware normally."""
        mw = RecordingMiddleware()
        executor = Executor(
            registry=registry,
            middlewares=[mw],
            approval_handler=AlwaysDenyHandler(),
        )
        result = executor.call("data.read")
        assert result["data"] == "safe"
        assert "data.read" in mw.before_calls
        assert "data.read" in mw.after_calls


class TestApprovalCallback:
    def test_callback_receives_identity(self, registry: Registry) -> None:
        """CallbackApprovalHandler receives identity from context."""
        captured: list[ApprovalRequest] = []

        async def capture(request: ApprovalRequest) -> ApprovalResult:
            captured.append(request)
            return ApprovalResult(status="approved", approved_by="callback")

        handler = CallbackApprovalHandler(capture)
        executor = Executor(registry=registry, approval_handler=handler)
        identity = Identity(id="user-42", type="user", roles=("admin",))
        ctx = Context.create(identity=identity)

        executor.call("admin.delete_user", {"user_id": "123"}, context=ctx)

        assert len(captured) == 1
        assert captured[0].context.identity is not None
        assert captured[0].context.identity.id == "user-42"
        assert "admin" in captured[0].context.identity.roles

    def test_callback_conditional_approval(self, registry: Registry) -> None:
        """CallbackApprovalHandler can make conditional decisions."""

        async def conditional(request: ApprovalRequest) -> ApprovalResult:
            if request.context.identity and "admin" in request.context.identity.roles:
                return ApprovalResult(status="approved", approved_by="policy")
            return ApprovalResult(status="rejected", reason="Admin role required")

        handler = CallbackApprovalHandler(conditional)
        executor = Executor(registry=registry, approval_handler=handler)

        # Admin user → approved
        admin_ctx = Context.create(
            identity=Identity(id="admin-1", roles=("admin",)),
        )
        result = executor.call("admin.delete_user", {"user_id": "123"}, context=admin_ctx)
        assert result["deleted"] is True

        # Regular user → denied
        user_ctx = Context.create(
            identity=Identity(id="user-1", roles=("viewer",)),
        )
        with pytest.raises(ApprovalDeniedError) as exc_info:
            executor.call("admin.delete_user", {"user_id": "123"}, context=user_ctx)
        assert "Admin role required" in exc_info.value.message


class TestApprovalPhaseB:
    def test_pending_then_resume_with_token(self, registry: Registry) -> None:
        """Phase B: first call returns pending, second call resumes with token."""
        call_count = 0

        class PhaseBHandler:
            async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
                nonlocal call_count
                call_count += 1
                return ApprovalResult(status="pending", approval_id="pending-tok-1")

            async def check_approval(self, approval_id: str) -> ApprovalResult:
                if approval_id == "pending-tok-1":
                    return ApprovalResult(status="approved", approved_by="reviewer")
                return ApprovalResult(status="rejected")

        executor = Executor(registry=registry, approval_handler=PhaseBHandler())

        # First call → pending
        with pytest.raises(ApprovalPendingError) as exc_info:
            executor.call("admin.delete_user", {"user_id": "123"})
        assert exc_info.value.approval_id == "pending-tok-1"
        assert call_count == 1

        # Resume with token → approved
        result = executor.call(
            "admin.delete_user",
            {"user_id": "123", "_approval_token": "pending-tok-1"},
        )
        assert result["deleted"] is True


class TestApprovalExtensionManager:
    def test_extension_manager_wires_handler(self, registry: Registry) -> None:
        """ExtensionManager can register and wire an approval handler."""
        em = ExtensionManager()
        em.register("approval_handler", AutoApproveHandler())

        executor = Executor(registry=registry)
        em.apply(registry, executor)

        result = executor.call("admin.delete_user", {"user_id": "123"})
        assert result["deleted"] is True

    def test_extension_manager_deny_handler(self, registry: Registry) -> None:
        """ExtensionManager with AlwaysDenyHandler blocks execution."""
        em = ExtensionManager()
        em.register("approval_handler", AlwaysDenyHandler())

        executor = Executor(registry=registry)
        em.apply(registry, executor)

        with pytest.raises(ApprovalDeniedError):
            executor.call("admin.delete_user", {"user_id": "123"})


class TestApprovalImports:
    def test_all_types_importable_from_apcore(self) -> None:
        """All approval types are importable from the apcore package."""
        from apcore import (
            AlwaysDenyHandler,
            ApprovalDeniedError,
            ApprovalError,
            ApprovalHandler,
            ApprovalPendingError,
            ApprovalRequest,
            ApprovalResult,
            ApprovalTimeoutError,
            AutoApproveHandler,
            CallbackApprovalHandler,
        )

        # Verify they are the correct types
        assert AlwaysDenyHandler is not None
        assert ApprovalDeniedError is not None
        assert ApprovalError is not None
        assert ApprovalHandler is not None
        assert ApprovalPendingError is not None
        assert ApprovalRequest is not None
        assert ApprovalResult is not None
        assert ApprovalTimeoutError is not None
        assert AutoApproveHandler is not None
        assert CallbackApprovalHandler is not None
