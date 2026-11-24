"""Unit tests for approval data types, error classes, and built-in handlers."""

from __future__ import annotations

import pytest

from apcore.approval import (
    AlwaysDenyHandler,
    ApprovalHandler,
    ApprovalRequest,
    ApprovalResult,
    AutoApproveHandler,
    CallbackApprovalHandler,
)
from apcore.context import Context
from apcore.errors import (
    ApprovalDeniedError,
    ApprovalError,
    ApprovalPendingError,
    ApprovalTimeoutError,
    ErrorCodes,
)
from apcore.module import ModuleAnnotations


# ---------------------------------------------------------------------------
# ApprovalRequest
# ---------------------------------------------------------------------------


class TestApprovalRequest:
    def test_fields(self) -> None:
        ctx = Context.create()
        ann = ModuleAnnotations(requires_approval=True)
        req = ApprovalRequest(
            module_id="test.module",
            arguments={"key": "value"},
            context=ctx,
            annotations=ann,
            description="Test module",
            tags=["admin"],
            caller_id="agent.orchestrator",
            action="test.module",
        )
        assert req.module_id == "test.module"
        assert req.arguments == {"key": "value"}
        assert req.context is ctx
        assert req.annotations is ann
        assert req.description == "Test module"
        assert req.tags == ["admin"]
        assert req.caller_id == "agent.orchestrator"
        assert req.action == "test.module"

    def test_defaults(self) -> None:
        ctx = Context.create()
        ann = ModuleAnnotations(requires_approval=True)
        req = ApprovalRequest(
            module_id="m",
            arguments={},
            context=ctx,
            annotations=ann,
        )
        assert req.description is None
        assert req.tags == []
        assert req.caller_id is None
        assert req.action == ""

    def test_frozen(self) -> None:
        ctx = Context.create()
        ann = ModuleAnnotations(requires_approval=True)
        req = ApprovalRequest(module_id="m", arguments={}, context=ctx, annotations=ann)
        with pytest.raises(AttributeError):
            req.module_id = "other"  # type: ignore[misc]


# ---------------------------------------------------------------------------
# ApprovalResult
# ---------------------------------------------------------------------------


class TestApprovalResult:
    def test_approved(self) -> None:
        result = ApprovalResult(status="approved", approved_by="admin")
        assert result.status == "approved"
        assert result.approved_by == "admin"
        assert result.reason is None
        assert result.approval_id is None
        assert result.metadata is None

    def test_rejected_with_reason(self) -> None:
        result = ApprovalResult(status="rejected", reason="Not authorized")
        assert result.status == "rejected"
        assert result.reason == "Not authorized"

    def test_pending_with_id(self) -> None:
        result = ApprovalResult(status="pending", approval_id="tok-123")
        assert result.status == "pending"
        assert result.approval_id == "tok-123"

    def test_frozen(self) -> None:
        result = ApprovalResult(status="approved")
        with pytest.raises(AttributeError):
            result.status = "rejected"  # type: ignore[misc]


# ---------------------------------------------------------------------------
# Error classes
# ---------------------------------------------------------------------------


class TestApprovalErrors:
    def test_approval_denied_error(self) -> None:
        result = ApprovalResult(status="rejected", reason="Policy violation")
        err = ApprovalDeniedError(result=result, module_id="test.mod")
        assert err.code == "APPROVAL_DENIED"
        assert "test.mod" in err.message
        assert "Policy violation" in err.message
        assert err.result is result
        assert err.module_id == "test.mod"
        assert isinstance(err, ApprovalError)

    def test_approval_timeout_error(self) -> None:
        result = ApprovalResult(status="timeout")
        err = ApprovalTimeoutError(result=result, module_id="test.mod")
        assert err.code == "APPROVAL_TIMEOUT"
        assert "test.mod" in err.message
        assert err.result is result

    def test_approval_pending_error(self) -> None:
        result = ApprovalResult(status="pending", approval_id="abc-123")
        err = ApprovalPendingError(result=result, module_id="test.mod")
        assert err.code == "APPROVAL_PENDING"
        assert err.approval_id == "abc-123"
        assert err.result is result

    def test_error_codes_exist(self) -> None:
        assert ErrorCodes.APPROVAL_DENIED == "APPROVAL_DENIED"
        assert ErrorCodes.APPROVAL_TIMEOUT == "APPROVAL_TIMEOUT"
        assert ErrorCodes.APPROVAL_PENDING == "APPROVAL_PENDING"

    def test_approval_error_inherits_module_error(self) -> None:
        result = ApprovalResult(status="rejected")
        err = ApprovalDeniedError(result=result)
        assert isinstance(err, ApprovalError)
        assert hasattr(err, "timestamp")
        assert hasattr(err, "code")

    def test_reason_property_returns_result_reason(self) -> None:
        """e.reason shortcut matches e.result.reason (per executor-api.md pattern)."""
        result = ApprovalResult(status="rejected", reason="Policy violation")
        err = ApprovalDeniedError(result=result, module_id="test.mod")
        assert err.reason == "Policy violation"

    def test_reason_property_none_when_no_reason(self) -> None:
        result = ApprovalResult(status="timeout")
        err = ApprovalTimeoutError(result=result, module_id="test.mod")
        assert err.reason is None

    def test_reason_property_on_pending_error(self) -> None:
        result = ApprovalResult(status="pending", reason="Awaiting manager", approval_id="tok-1")
        err = ApprovalPendingError(result=result, module_id="test.mod")
        assert err.reason == "Awaiting manager"


# ---------------------------------------------------------------------------
# Built-in handlers
# ---------------------------------------------------------------------------


class TestAlwaysDenyHandler:
    @pytest.mark.asyncio
    async def test_request_approval_rejects(self) -> None:
        handler = AlwaysDenyHandler()
        ctx = Context.create()
        request = ApprovalRequest(
            module_id="test.mod",
            arguments={},
            context=ctx,
            annotations=ModuleAnnotations(requires_approval=True),
        )
        result = await handler.request_approval(request)
        assert result.status == "rejected"
        assert result.reason == "Always denied"

    @pytest.mark.asyncio
    async def test_check_approval_rejects(self) -> None:
        handler = AlwaysDenyHandler()
        result = await handler.check_approval("some-id")
        assert result.status == "rejected"

    def test_satisfies_protocol(self) -> None:
        handler = AlwaysDenyHandler()
        assert isinstance(handler, ApprovalHandler)


class TestAutoApproveHandler:
    @pytest.mark.asyncio
    async def test_request_approval_approves(self) -> None:
        handler = AutoApproveHandler()
        ctx = Context.create()
        request = ApprovalRequest(
            module_id="test.mod",
            arguments={},
            context=ctx,
            annotations=ModuleAnnotations(requires_approval=True),
        )
        result = await handler.request_approval(request)
        assert result.status == "approved"
        assert result.approved_by == "auto"

    @pytest.mark.asyncio
    async def test_check_approval_approves(self) -> None:
        handler = AutoApproveHandler()
        result = await handler.check_approval("some-id")
        assert result.status == "approved"

    def test_satisfies_protocol(self) -> None:
        handler = AutoApproveHandler()
        assert isinstance(handler, ApprovalHandler)


class TestCallbackApprovalHandler:
    @pytest.mark.asyncio
    async def test_delegates_to_callback(self) -> None:
        async def my_callback(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(
                status="approved",
                approved_by="callback",
                metadata={"module": request.module_id},
            )

        handler = CallbackApprovalHandler(my_callback)
        ctx = Context.create()
        request = ApprovalRequest(
            module_id="test.mod",
            arguments={"x": 1},
            context=ctx,
            annotations=ModuleAnnotations(requires_approval=True),
        )
        result = await handler.request_approval(request)
        assert result.status == "approved"
        assert result.approved_by == "callback"
        assert result.metadata == {"module": "test.mod"}

    @pytest.mark.asyncio
    async def test_check_approval_rejects(self) -> None:
        async def noop(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status="approved")

        handler = CallbackApprovalHandler(noop)
        result = await handler.check_approval("some-id")
        assert result.status == "rejected"

    def test_satisfies_protocol(self) -> None:
        async def noop(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status="approved")

        handler = CallbackApprovalHandler(noop)
        assert isinstance(handler, ApprovalHandler)


class TestCallbackApprovalHandlerRejectsSyncCallbacks:
    """apcore#104: a synchronous callback must fail with an actionable message.

    ``CallbackApprovalHandler`` declares an async callback, but nothing checked.
    A plain function reached ``await self._callback(request)`` and died with
    ``TypeError: object ApprovalResult can't be used in 'await' expression`` —
    which names neither the callback nor the fix, and is emitted at the approval
    gate rather than where the handler was constructed.
    """

    @pytest.mark.asyncio
    async def test_sync_callback_raises_a_named_error(self) -> None:
        def sync_callback(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status="approved")

        handler = CallbackApprovalHandler(sync_callback)  # type: ignore[arg-type]
        request = ApprovalRequest(
            module_id="test.mod",
            arguments={},
            context=Context.create(),
            annotations=ModuleAnnotations(requires_approval=True),
        )
        with pytest.raises(TypeError) as excinfo:
            await handler.request_approval(request)

        message = str(excinfo.value)
        assert "CallbackApprovalHandler" in message
        assert "sync_callback" in message, "the message must name the offending callback"
        assert "ApprovalResult" in message, "the message must name what was returned"
        assert "async def" in message, "the message must name the fix"
        assert "can't be used in 'await' expression" not in message

    @pytest.mark.asyncio
    async def test_a_callback_returning_a_non_result_is_also_named(self) -> None:
        handler = CallbackApprovalHandler(lambda request: None)  # type: ignore[arg-type,return-value]
        request = ApprovalRequest(
            module_id="test.mod",
            arguments={},
            context=Context.create(),
            annotations=ModuleAnnotations(requires_approval=True),
        )
        with pytest.raises(TypeError, match="requires an async callback"):
            await handler.request_approval(request)

    @pytest.mark.asyncio
    async def test_an_async_callback_is_unaffected(self) -> None:
        async def async_callback(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status="approved", approved_by="cb")

        handler = CallbackApprovalHandler(async_callback)
        request = ApprovalRequest(
            module_id="test.mod",
            arguments={},
            context=Context.create(),
            annotations=ModuleAnnotations(requires_approval=True),
        )
        result = await handler.request_approval(request)
        assert result.approved_by == "cb"

    @pytest.mark.asyncio
    async def test_a_callable_object_with_async_call_is_accepted(self) -> None:
        """Detection is on the returned value, not on ``iscoroutinefunction``."""

        class AsyncCallable:
            async def __call__(self, request: ApprovalRequest) -> ApprovalResult:
                return ApprovalResult(status="rejected", reason="nope")

        handler = CallbackApprovalHandler(AsyncCallable())
        request = ApprovalRequest(
            module_id="test.mod",
            arguments={},
            context=Context.create(),
            annotations=ModuleAnnotations(requires_approval=True),
        )
        result = await handler.request_approval(request)
        assert result.status == "rejected"
