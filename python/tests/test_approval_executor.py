"""Unit tests for the approval gate in the Executor (Step 4.5)."""

from __future__ import annotations

import logging
from typing import Any

import pytest
from pydantic import BaseModel, ConfigDict

from apcore.approval import (
    AlwaysDenyHandler,
    ApprovalRequest,
    ApprovalResult,
    AutoApproveHandler,
    CallbackApprovalHandler,
)
from apcore.context import Context
from apcore.errors import ApprovalDeniedError, ApprovalPendingError, ApprovalTimeoutError
from apcore.executor import Executor
from apcore.module import ModuleAnnotations
from apcore.registry import Registry


# ---------------------------------------------------------------------------
# Test module implementations
# ---------------------------------------------------------------------------


class PermissiveInput(BaseModel):
    model_config = ConfigDict(extra="allow")


class PermissiveOutput(BaseModel):
    model_config = ConfigDict(extra="allow")


class ApprovalRequiredModule:
    """Module with requires_approval=True via ModuleAnnotations dataclass."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = ModuleAnnotations(requires_approval=True, destructive=True)
    description = "Destructive operation"
    tags = ["admin"]

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "executed"}


class ApprovalRequiredDictModule:
    """Module with requires_approval=True via dict annotations."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = {"requires_approval": True, "destructive": True}
    description = "Dict-annotated module"
    tags = ["admin"]

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "executed"}


class NoApprovalModule:
    """Module without requires_approval."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = ModuleAnnotations(requires_approval=False)

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "executed"}


class NoAnnotationsModule:
    """Module with no annotations attribute."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput

    def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "executed"}


class AsyncApprovalRequiredModule:
    """Async module with requires_approval=True."""

    input_schema = PermissiveInput
    output_schema = PermissiveOutput
    annotations = ModuleAnnotations(requires_approval=True)

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"status": "async_executed"}


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def registry() -> Registry:
    reg = Registry()
    reg.register("test.approval_required", ApprovalRequiredModule())
    reg.register("test.approval_dict", ApprovalRequiredDictModule())
    reg.register("test.no_approval", NoApprovalModule())
    reg.register("test.no_annotations", NoAnnotationsModule())
    reg.register("test.async_approval", AsyncApprovalRequiredModule())
    return reg


# ---------------------------------------------------------------------------
# Sync call() tests
# ---------------------------------------------------------------------------


class TestApprovalGateSync:
    def test_gate_skipped_no_handler(self, registry: Registry) -> None:
        """Gate is skipped when no approval_handler is configured."""
        executor = Executor(registry=registry)
        result = executor.call("test.approval_required")
        assert result["status"] == "executed"

    def test_gate_skipped_no_requires_approval(self, registry: Registry) -> None:
        """Gate is skipped when module does not require approval."""
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        result = executor.call("test.no_approval")
        assert result["status"] == "executed"

    def test_gate_skipped_no_annotations(self, registry: Registry) -> None:
        """Gate is skipped when module has no annotations."""
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        result = executor.call("test.no_annotations")
        assert result["status"] == "executed"

    def test_approved_proceeds(self, registry: Registry) -> None:
        """Approved result allows execution to proceed."""
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        result = executor.call("test.approval_required")
        assert result["status"] == "executed"

    def test_denied_raises_error(self, registry: Registry) -> None:
        """Rejected result raises ApprovalDeniedError."""
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        with pytest.raises(ApprovalDeniedError) as exc_info:
            executor.call("test.approval_required")
        assert exc_info.value.code == "APPROVAL_DENIED"
        assert exc_info.value.result.status == "rejected"

    def test_timeout_raises_error(self, registry: Registry) -> None:
        """Timeout result raises ApprovalTimeoutError."""

        async def timeout_handler(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status="timeout")

        handler = CallbackApprovalHandler(timeout_handler)
        executor = Executor(registry=registry, approval_handler=handler)
        with pytest.raises(ApprovalTimeoutError) as exc_info:
            executor.call("test.approval_required")
        assert exc_info.value.code == "APPROVAL_TIMEOUT"

    def test_pending_raises_error(self, registry: Registry) -> None:
        """Pending result raises ApprovalPendingError with approval_id."""

        async def pending_handler(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status="pending", approval_id="tok-abc")

        handler = CallbackApprovalHandler(pending_handler)
        executor = Executor(registry=registry, approval_handler=handler)
        with pytest.raises(ApprovalPendingError) as exc_info:
            executor.call("test.approval_required")
        assert exc_info.value.approval_id == "tok-abc"

    def test_dict_annotations_trigger_gate(self, registry: Registry) -> None:
        """Dict-form annotations trigger the approval gate."""
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        with pytest.raises(ApprovalDeniedError):
            executor.call("test.approval_dict")

    def test_dict_annotations_approved(self, registry: Registry) -> None:
        """Dict-form annotations with auto-approve allow execution."""
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        result = executor.call("test.approval_dict")
        assert result["status"] == "executed"

    def test_set_approval_handler(self, registry: Registry) -> None:
        """set_approval_handler() updates the handler."""
        executor = Executor(registry=registry)
        # No handler → executes
        result = executor.call("test.approval_required")
        assert result["status"] == "executed"

        # Set deny handler → raises
        executor.set_approval_handler(AlwaysDenyHandler())
        with pytest.raises(ApprovalDeniedError):
            executor.call("test.approval_required")

    def test_approval_request_carries_context(self, registry: Registry) -> None:
        """ApprovalRequest carries correct module metadata."""
        captured_requests: list[ApprovalRequest] = []

        async def capture_handler(request: ApprovalRequest) -> ApprovalResult:
            captured_requests.append(request)
            return ApprovalResult(status="approved", approved_by="test")

        handler = CallbackApprovalHandler(capture_handler)
        executor = Executor(registry=registry, approval_handler=handler)
        executor.call("test.approval_required", {"key": "val"})

        assert len(captured_requests) == 1
        req = captured_requests[0]
        assert req.module_id == "test.approval_required"
        assert req.arguments == {"key": "val"}
        assert req.annotations.requires_approval is True
        assert req.annotations.destructive is True
        assert req.description == "Destructive operation"
        assert req.tags == ["admin"]
        assert req.context.trace_id is not None
        # Spec decision D-03 (2026-05-decision-log.md, PROTOCOL_SPEC §7.3.1):
        # a top-level call (no incoming Context) has no caller, and `action`
        # always names the module actually being invoked.
        assert req.caller_id is None
        assert req.action == "test.approval_required"

    def test_approval_request_caller_id_is_the_invoking_module_on_a_nested_call(self, registry: Registry) -> None:
        """The other half of D-03: `caller_id` names the calling module, not `None`, on a nested call.

        `Context.child()` sets `caller_id` from the parent's `call_chain`, and
        `BuiltinApprovalGate` reads it straight off `Context.caller_id` with no
        substitution — never the ``"@external"`` sentinel ACL evaluation uses
        internally for a context-less caller.
        """
        captured_requests: list[ApprovalRequest] = []

        async def capture_handler(request: ApprovalRequest) -> ApprovalResult:
            captured_requests.append(request)
            return ApprovalResult(status="approved", approved_by="test")

        class CallerModule:
            input_schema = PermissiveInput
            output_schema = PermissiveOutput

            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                return context.executor.call("test.approval_required", inputs, context)

        registry.register("test.caller", CallerModule())

        handler = CallbackApprovalHandler(capture_handler)
        executor = Executor(registry=registry, approval_handler=handler)
        executor.call("test.caller", {"key": "val"})

        assert len(captured_requests) == 1
        req = captured_requests[0]
        assert req.caller_id == "test.caller"
        assert req.action == "test.approval_required"
        assert req.action == req.module_id

    def test_approval_token_is_consumed_without_mutating_the_callers_dict(self, registry: Registry) -> None:
        """_approval_token drives check_approval and never reaches the module.

        PROTOCOL_SPEC §7.4 requires the key to be gone "before passing to
        subsequent steps" — that is about what the pipeline sees downstream, not
        a licence to edit the caller's object. ``PipelineContext`` holds the very
        dict the caller passed to ``call()``, so the old ``ctx.inputs.pop()``
        meant a module invocation silently mutated its caller's data.
        apcore-typescript rebuilds the inputs instead
        (``const { _approval_token: _, ...rest } = ctx.inputs``); this asserts
        the same contract.
        """
        check_called_with: list[str] = []
        seen_by_module: list[dict[str, Any]] = []

        class TokenHandler:
            async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
                return ApprovalResult(status="approved")

            async def check_approval(self, approval_id: str) -> ApprovalResult:
                check_called_with.append(approval_id)
                return ApprovalResult(status="approved", approved_by="token-check")

        class RecordingModule(ApprovalRequiredModule):
            def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
                seen_by_module.append(dict(inputs))
                return {"status": "executed"}

        registry.unregister("test.approval_required")
        registry.register("test.approval_required", RecordingModule())

        executor = Executor(registry=registry, approval_handler=TokenHandler())
        inputs: dict[str, Any] = {"_approval_token": "my-token", "data": "value"}
        result = executor.call("test.approval_required", inputs)
        assert result["status"] == "executed"
        assert check_called_with == ["my-token"]
        # The module never sees the protocol key…
        assert seen_by_module == [{"data": "value"}]
        # …and the caller's own dict is left exactly as it was passed in.
        assert inputs == {"_approval_token": "my-token", "data": "value"}

    def test_from_registry_with_handler(self, registry: Registry) -> None:
        """from_registry() passes approval_handler through."""
        executor = Executor.from_registry(
            registry=registry,
            approval_handler=AlwaysDenyHandler(),
        )
        with pytest.raises(ApprovalDeniedError):
            executor.call("test.approval_required")

    def test_unknown_status_treated_as_denied(self, registry: Registry) -> None:
        """Unknown status value is treated as denial with a warning."""

        async def unknown_handler(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status="unknown_value")

        handler = CallbackApprovalHandler(unknown_handler)
        executor = Executor(registry=registry, approval_handler=handler)
        with pytest.raises(ApprovalDeniedError):
            executor.call("test.approval_required")

    def test_handler_exception_propagates(self, registry: Registry) -> None:
        """If the handler itself raises, the exception propagates (wrapped by A11)."""
        from apcore.errors import ModuleError

        async def broken_handler(request: ApprovalRequest) -> ApprovalResult:
            raise RuntimeError("handler crashed")

        handler = CallbackApprovalHandler(broken_handler)
        executor = Executor(registry=registry, approval_handler=handler)
        with pytest.raises(ModuleError, match="handler crashed"):
            executor.call("test.approval_required")


# ---------------------------------------------------------------------------
# Async call_async() tests
# ---------------------------------------------------------------------------


class TestApprovalGateAsync:
    @pytest.mark.asyncio
    async def test_gate_skipped_no_handler(self, registry: Registry) -> None:
        executor = Executor(registry=registry)
        result = await executor.call_async("test.approval_required")
        assert result["status"] == "executed"

    @pytest.mark.asyncio
    async def test_approved_proceeds(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        result = await executor.call_async("test.approval_required")
        assert result["status"] == "executed"

    @pytest.mark.asyncio
    async def test_denied_raises_error(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        with pytest.raises(ApprovalDeniedError):
            await executor.call_async("test.approval_required")

    @pytest.mark.asyncio
    async def test_async_module_approved(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        result = await executor.call_async("test.async_approval")
        assert result["status"] == "async_executed"

    @pytest.mark.asyncio
    async def test_async_module_denied(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        with pytest.raises(ApprovalDeniedError):
            await executor.call_async("test.async_approval")

    @pytest.mark.asyncio
    async def test_approval_token_async(self, registry: Registry) -> None:
        """_approval_token works in async path."""
        check_called_with: list[str] = []

        class TokenHandler:
            async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
                return ApprovalResult(status="approved")

            async def check_approval(self, approval_id: str) -> ApprovalResult:
                check_called_with.append(approval_id)
                return ApprovalResult(status="approved")

        executor = Executor(registry=registry, approval_handler=TokenHandler())
        inputs: dict[str, Any] = {"_approval_token": "async-tok"}
        result = await executor.call_async("test.approval_required", inputs)
        assert result["status"] == "executed"
        assert check_called_with == ["async-tok"]

    @pytest.mark.asyncio
    async def test_dict_annotations_async(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        with pytest.raises(ApprovalDeniedError):
            await executor.call_async("test.approval_dict")


# ---------------------------------------------------------------------------
# Stream tests
# ---------------------------------------------------------------------------


class TestApprovalGateStream:
    @pytest.mark.asyncio
    async def test_stream_approved(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        chunks = []
        async for chunk in executor.stream("test.approval_required"):
            chunks.append(chunk)
        assert len(chunks) == 1
        assert chunks[0]["status"] == "executed"

    @pytest.mark.asyncio
    async def test_stream_denied(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        with pytest.raises(ApprovalDeniedError):
            async for _ in executor.stream("test.approval_required"):
                pass

    @pytest.mark.asyncio
    async def test_stream_skipped_no_approval(self, registry: Registry) -> None:
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        chunks = []
        async for chunk in executor.stream("test.no_approval"):
            chunks.append(chunk)
        assert len(chunks) == 1
        assert chunks[0]["status"] == "executed"


# ---------------------------------------------------------------------------
# Audit event tests
# ---------------------------------------------------------------------------


async def _make_coro(result: ApprovalResult) -> ApprovalResult:
    return result


class TestApprovalAuditEvents:
    """Tests for approval audit event emission (Level 3 conformance)."""

    def test_audit_log_emitted_on_approved(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        """logging.info is emitted with approval decision details when approved."""
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        with caplog.at_level(logging.INFO, logger="apcore.builtin_steps"):
            executor.call("test.approval_required")

        approval_logs = [r for r in caplog.records if "Approval decision" in r.message]
        assert len(approval_logs) == 1
        assert "status=approved" in approval_logs[0].message
        assert "approved_by=auto" in approval_logs[0].message

    def test_audit_log_emitted_on_denied(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        """logging.info is emitted before ApprovalDeniedError is raised."""
        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        with caplog.at_level(logging.INFO, logger="apcore.builtin_steps"):
            with pytest.raises(ApprovalDeniedError):
                executor.call("test.approval_required")

        approval_logs = [r for r in caplog.records if "Approval decision" in r.message]
        assert len(approval_logs) == 1
        assert "status=rejected" in approval_logs[0].message

    def test_audit_log_emitted_on_pending(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        """logging.info is emitted before ApprovalPendingError is raised."""

        async def pending_cb(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status="pending", approval_id="tok-123")

        executor = Executor(registry=registry, approval_handler=CallbackApprovalHandler(pending_cb))
        with caplog.at_level(logging.INFO, logger="apcore.builtin_steps"):
            with pytest.raises(ApprovalPendingError):
                executor.call("test.approval_required")

        approval_logs = [r for r in caplog.records if "Approval decision" in r.message]
        assert len(approval_logs) == 1
        assert "status=pending" in approval_logs[0].message

    @pytest.mark.asyncio
    async def test_audit_log_emitted_async(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        """Audit log is emitted in async call_async() path."""
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        with caplog.at_level(logging.INFO, logger="apcore.builtin_steps"):
            await executor.call_async("test.approval_required")

        approval_logs = [r for r in caplog.records if "Approval decision" in r.message]
        assert len(approval_logs) == 1
        assert "status=approved" in approval_logs[0].message

    def test_no_audit_log_when_gate_skipped(self, registry: Registry, caplog: pytest.LogCaptureFixture) -> None:
        """No audit log when gate is skipped (no handler or no requires_approval)."""
        executor = Executor(registry=registry)
        with caplog.at_level(logging.INFO, logger="apcore.builtin_steps"):
            executor.call("test.approval_required")

        approval_logs = [r for r in caplog.records if "Approval decision" in r.message]
        assert len(approval_logs) == 0

    def test_span_event_emitted_when_tracing_active(self, registry: Registry) -> None:
        """Span event is appended to tracing span when tracing is active.

        In real usage, span events appear during nested calls where the parent's
        TracingMiddleware has already pushed a span (context.data is shared).
        We simulate this by passing a pre-populated context.
        """
        mock_span_events: list[dict[str, Any]] = []

        class MockSpan:
            events = mock_span_events

        handler = CallbackApprovalHandler(
            lambda req: _make_coro(ApprovalResult(status="approved", approved_by="test-user", reason="looks good"))
        )
        executor = Executor(registry=registry, approval_handler=handler)

        # Simulate a parent call that already has tracing spans (nested call scenario)
        ctx = Context.create()
        ctx.data["_apcore.mw.tracing.spans"] = [MockSpan()]
        executor.call("test.approval_required", {}, context=ctx)

        assert len(mock_span_events) == 1
        event = mock_span_events[0]
        assert event["name"] == "approval_decision"
        assert event["module_id"] == "test.approval_required"
        assert event["status"] == "approved"
        assert event["approved_by"] == "test-user"
        assert event["reason"] == "looks good"

    def test_span_event_on_denied_decision(self, registry: Registry) -> None:
        """Span event is emitted even when approval is denied."""
        mock_span_events: list[dict[str, Any]] = []

        class MockSpan:
            events = mock_span_events

        executor = Executor(registry=registry, approval_handler=AlwaysDenyHandler())
        ctx = Context.create()
        ctx.data["_apcore.mw.tracing.spans"] = [MockSpan()]

        with pytest.raises(ApprovalDeniedError):
            executor.call("test.approval_required", {}, context=ctx)

        assert len(mock_span_events) == 1
        assert mock_span_events[0]["status"] == "rejected"

    def test_no_span_event_when_no_tracing(self, registry: Registry) -> None:
        """No span event when no tracing spans in context."""
        executor = Executor(registry=registry, approval_handler=AutoApproveHandler())
        # Should not raise even without tracing spans
        result = executor.call("test.approval_required")
        assert result["status"] == "executed"
