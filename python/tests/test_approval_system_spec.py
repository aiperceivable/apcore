"""Spec-traced contract tests for the Approval System.

Source spec: apcore/docs/features/approval-system.md
Contract: ApprovalHandler.request_approval

Each test carries a verbatim clause id of the form
`approval_system.request_approval.<kind>.<detail>` so cross-language diffs line
up row by row. Tests only — production source is never modified here.
"""

from __future__ import annotations

import asyncio
import inspect

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
    ApprovalPendingError,
    ApprovalTimeoutError,
)
from apcore.module import ModuleAnnotations


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_request(module_id: str = "test.mod", **arguments: object) -> ApprovalRequest:
    """Build a valid ApprovalRequest for the handler under test."""
    return ApprovalRequest(
        module_id=module_id,
        arguments=dict(arguments),
        context=Context.create(),
        annotations=ModuleAnnotations(requires_approval=True),
    )


def _status_handler(status: str) -> CallbackApprovalHandler:
    """A handler whose request_approval resolves to the given status."""

    async def _cb(request: ApprovalRequest) -> ApprovalResult:
        return ApprovalResult(status=status, approval_id="tok-1" if status == "pending" else None)

    return CallbackApprovalHandler(_cb)


# ---------------------------------------------------------------------------
# input.<param>.<condition>
# ---------------------------------------------------------------------------


class TestInputs:
    def test_request_module_id_required(self) -> None:
        """approval_system.request_approval.input.request.module_id_required

        The contract requires `request` to carry `module_id`. Omitting it must
        fail construction of the ApprovalRequest (the required positional field).
        """
        with pytest.raises(TypeError):
            ApprovalRequest(  # type: ignore[call-arg]
                arguments={},
                context=Context.create(),
                annotations=ModuleAnnotations(requires_approval=True),
            )

    def test_request_caller_id_and_action_required(self) -> None:
        """approval_system.request_approval.input.request.caller_id_action_required

        Spec decision D-03 (2026-05-decision-log.md, PROTOCOL_SPEC §7.3.1):
        `ApprovalRequest` carries `caller_id` and `action` directly, so a
        handler can read them without traversing `request.context`. `caller_id`
        is `None` for a top-level call (there is no caller to name) and
        `action` is always populated — it is not conditioned on `caller_id`.
        """
        request = ApprovalRequest(
            module_id="test.mod",
            arguments={},
            context=Context.create(),
            annotations=ModuleAnnotations(requires_approval=True),
            caller_id=None,
            action="test.mod",
        )
        assert request.caller_id is None
        assert request.action == "test.mod"

        nested = ApprovalRequest(
            module_id="test.mod",
            arguments={},
            context=Context(trace_id="t-1", caller_id="test.caller"),
            annotations=ModuleAnnotations(requires_approval=True),
            caller_id="test.caller",
            action="test.mod",
        )
        assert nested.caller_id == "test.caller"
        assert nested.action == "test.mod"


# ---------------------------------------------------------------------------
# error.<CODE>
# ---------------------------------------------------------------------------


class TestErrors:
    async def test_error_approval_denied(self) -> None:
        """approval_system.request_approval.error.APPROVAL_DENIED

        A handler that rejects yields a result that maps to ApprovalDeniedError
        with code APPROVAL_DENIED.
        """
        handler = AlwaysDenyHandler()
        result = await handler.request_approval(_make_request())
        assert result.status == "rejected"

        err = ApprovalDeniedError(result=result, module_id="test.mod")
        assert isinstance(err, ApprovalDeniedError)
        assert err.code == "APPROVAL_DENIED"

    async def test_error_approval_timeout(self) -> None:
        """approval_system.request_approval.error.APPROVAL_TIMEOUT

        A handler that times out yields a result that maps to
        ApprovalTimeoutError with code APPROVAL_TIMEOUT.
        """
        handler = _status_handler("timeout")
        result = await handler.request_approval(_make_request())
        assert result.status == "timeout"

        err = ApprovalTimeoutError(result=result, module_id="test.mod")
        assert isinstance(err, ApprovalTimeoutError)
        assert err.code == "APPROVAL_TIMEOUT"

    async def test_error_approval_pending(self) -> None:
        """approval_system.request_approval.error.APPROVAL_PENDING

        A handler that defers (Phase B) yields a result that maps to
        ApprovalPendingError with code APPROVAL_PENDING.
        """
        handler = _status_handler("pending")
        result = await handler.request_approval(_make_request())
        assert result.status == "pending"

        err = ApprovalPendingError(result=result, module_id="test.mod")
        assert isinstance(err, ApprovalPendingError)
        assert err.code == "APPROVAL_PENDING"
        assert err.approval_id == "tok-1"


# ---------------------------------------------------------------------------
# property.<name>
# ---------------------------------------------------------------------------


class TestProperties:
    async def test_property_async(self) -> None:
        """approval_system.request_approval.property.async

        request_approval is a coroutine function; awaiting it resolves to an
        ApprovalResult.
        """
        handler = AutoApproveHandler()
        assert inspect.iscoroutinefunction(handler.request_approval)
        coro = handler.request_approval(_make_request())
        assert inspect.iscoroutine(coro)
        result = await coro
        assert isinstance(result, ApprovalResult)
        assert result.status == "approved"

    async def test_property_thread_safe(self) -> None:
        """approval_system.request_approval.property.thread_safe

        N concurrent request_approval calls with distinct inputs complete
        without exception and each returns the result for its own input.
        """
        n = 12

        async def _cb(request: ApprovalRequest) -> ApprovalResult:
            # Yield control to interleave concurrent calls.
            await asyncio.sleep(0)
            return ApprovalResult(status="approved", metadata={"mod": request.module_id})

        handler = CallbackApprovalHandler(_cb)
        requests = [_make_request(module_id=f"mod.{i}", idx=i) for i in range(n)]
        results = await asyncio.gather(*(handler.request_approval(req) for req in requests))

        assert len(results) == n
        assert all(r.status == "approved" for r in results)
        # Each result is correlated with its own distinct input — no cross-talk.
        seen = {r.metadata["mod"] for r in results if r.metadata}
        assert seen == {f"mod.{i}" for i in range(n)}

    async def test_property_idempotent_false(self) -> None:
        """approval_system.request_approval.property.idempotent

        Contract declares idempotent: false. A handler may legitimately return
        different outcomes for identical inputs across calls; the type does not
        force same-outcome. We observe a non-idempotent handler producing
        distinct results for the same request.
        """
        states = iter(["approved", "rejected"])

        async def _cb(request: ApprovalRequest) -> ApprovalResult:
            return ApprovalResult(status=next(states))

        handler = CallbackApprovalHandler(_cb)
        request = _make_request()
        first = await handler.request_approval(request)
        second = await handler.request_approval(request)
        assert first.status == "approved"
        assert second.status == "rejected"
        assert first != second

    async def test_property_pure_false(self) -> None:
        """approval_system.request_approval.property.pure

        Contract declares pure: false — the handler may emit notifications or
        persist state. We observe a side effect (state mutation) caused by a
        single request_approval call, visible via a public query on the handler.
        """

        class RecordingHandler:
            def __init__(self) -> None:
                self.calls: list[str] = []

            async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
                self.calls.append(request.module_id)
                return ApprovalResult(status="approved")

            async def check_approval(self, approval_id: str) -> ApprovalResult:
                return ApprovalResult(status="rejected")

        handler = RecordingHandler()
        assert handler.calls == []
        await handler.request_approval(_make_request(module_id="audit.me"))
        # Observable state changed: the call was recorded (impure / side effect).
        assert handler.calls == ["audit.me"]


# ---------------------------------------------------------------------------
# Protocol conformance (request_approval is part of the ApprovalHandler protocol)
# ---------------------------------------------------------------------------


class TestProtocolConformance:
    def test_handlers_satisfy_protocol(self) -> None:
        """approval_system.request_approval.property.protocol_conformance

        All built-in handlers exposing request_approval satisfy the
        runtime-checkable ApprovalHandler protocol.
        """
        for handler in (
            AlwaysDenyHandler(),
            AutoApproveHandler(),
            CallbackApprovalHandler(lambda r: _coro_approved()),
        ):
            assert isinstance(handler, ApprovalHandler)


async def _coro_approved() -> ApprovalResult:
    return ApprovalResult(status="approved")
