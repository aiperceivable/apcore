"""Approval system: protocol, data types, built-in handlers, and errors."""

from __future__ import annotations

import inspect
from collections.abc import Callable, Coroutine
from dataclasses import dataclass, field
from typing import Any, Literal, Protocol, runtime_checkable

from apcore.context import Context
from apcore.errors import (
    ApprovalDeniedError as ApprovalDeniedError,
    ApprovalError as ApprovalError,
    ApprovalPendingError as ApprovalPendingError,
    ApprovalTimeoutError as ApprovalTimeoutError,
)
from apcore.module import ModuleAnnotations

__all__ = [
    "ApprovalRequest",
    "ApprovalResult",
    "ApprovalHandler",
    "AlwaysDenyHandler",
    "AutoApproveHandler",
    "CallbackApprovalHandler",
    "ApprovalError",
    "ApprovalDeniedError",
    "ApprovalTimeoutError",
    "ApprovalPendingError",
]


@dataclass(frozen=True)
class ApprovalRequest:
    """Carries invocation context to the approval handler.

    Attributes:
        module_id: Canonical module ID being invoked.
        arguments: Input arguments for the call.
        context: Full execution context (trace_id, identity, call_chain).
        annotations: Module's annotation set (requires_approval is guaranteed true).
        description: Module's human-readable description.
        tags: Module's tags.
        caller_id: The invoking module's ID, from ``context.caller_id`` — ``None``
            on a top-level call (spec decision D-03, PROTOCOL_SPEC §7.3.1). Lets a
            handler (Slack approver, audit log) read who is asking without
            traversing ``context``. Defaults to ``None`` only so this field can
            follow ``tags`` in argument order; ``BuiltinApprovalGate`` always
            supplies it explicitly.
        action: The module ID being invoked — equal to ``module_id``, exposed
            under its own name so a handler can read "what is being done"
            without cross-referencing ``module_id`` (spec decision D-03).
            Defaults to ``""`` for the same field-ordering reason as
            ``caller_id``; ``BuiltinApprovalGate`` always supplies it explicitly.
    """

    module_id: str
    arguments: dict[str, Any]
    context: Context
    annotations: ModuleAnnotations
    description: str | None = None
    tags: list[str] = field(default_factory=list)
    caller_id: str | None = None
    action: str = ""


@dataclass(frozen=True)
class ApprovalResult:
    """Carries the approval handler's decision.

    Attributes:
        status: One of: "approved", "rejected", "timeout", "pending".
        approved_by: Identifier of the approver (human, agent, policy).
        reason: Human-readable explanation for the decision.
        approval_id: Phase B token for async resume.
        metadata: Additional metadata from the approval process.
    """

    status: Literal["approved", "rejected", "timeout", "pending"]
    approved_by: str | None = None
    reason: str | None = None
    approval_id: str | None = None
    metadata: dict[str, Any] | None = None


@runtime_checkable
class ApprovalHandler(Protocol):
    """Protocol for pluggable approval handlers.

    Implementations receive an ApprovalRequest and return an ApprovalResult.
    Both methods are asynchronous.
    """

    async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
        """Request approval for a module invocation."""
        ...

    async def check_approval(self, approval_id: str) -> ApprovalResult:
        """Check status of a previously pending approval (Phase B).

        Default implementations SHOULD return rejected.
        """
        ...


class AlwaysDenyHandler:
    """Built-in handler that always rejects. Safe default for enforcement."""

    async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
        """Always reject the request."""
        return ApprovalResult(status="rejected", reason="Always denied")

    async def check_approval(self, approval_id: str) -> ApprovalResult:
        """Always reject Phase B checks."""
        return ApprovalResult(status="rejected", reason="Always denied")


class AutoApproveHandler:
    """Built-in handler that always approves. For testing and development."""

    async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
        """Always approve the request."""
        return ApprovalResult(status="approved", approved_by="auto")

    async def check_approval(self, approval_id: str) -> ApprovalResult:
        """Always approve Phase B checks."""
        return ApprovalResult(status="approved", approved_by="auto")


class CallbackApprovalHandler:
    """Built-in handler that delegates to a user-provided **async** callback.

    The callback must be asynchronous — the same contract apcore-typescript's
    ``CallbackApprovalHandler`` declares. An approval decision is an I/O-bound
    round-trip (a Slack message, a ticket, a human), and the
    :class:`ApprovalHandler` protocol this wraps is async in both directions;
    a synchronous convenience form would be weaker than the contract it stands
    in for (apcore#104).

    Args:
        callback: Async function that receives an ApprovalRequest and returns
            an ApprovalResult. A plain (non-async) function raises
            :class:`TypeError` from :meth:`request_approval` — see below.

    Example::

        async def ask_slack(request: ApprovalRequest) -> ApprovalResult:
            ok = await slack.ask(request.module_id, request.arguments)
            return ApprovalResult(status="approved" if ok else "rejected")

        handler = CallbackApprovalHandler(ask_slack)

    To reuse an existing synchronous decision function, wrap it::

        handler = CallbackApprovalHandler(lambda req: asyncio.to_thread(decide, req))
    """

    def __init__(
        self,
        callback: Callable[[ApprovalRequest], Coroutine[Any, Any, ApprovalResult]],
    ) -> None:
        self._callback = callback

    async def request_approval(self, request: ApprovalRequest) -> ApprovalResult:
        """Delegate to the user-provided callback.

        Raises:
            TypeError: When the callback is not asynchronous. Awaiting its
                return value would otherwise fail with the opaque
                ``object ApprovalResult can't be used in 'await' expression``
                (apcore#104), which names neither the callback nor the fix.
        """
        result = self._callback(request)
        if not inspect.isawaitable(result):
            name = getattr(self._callback, "__qualname__", None) or repr(self._callback)
            raise TypeError(
                f"CallbackApprovalHandler requires an async callback, but {name} returned "
                f"{type(result).__name__!r}, which is not awaitable. Declare the callback with "
                "'async def', or wrap a synchronous one — "
                "CallbackApprovalHandler(lambda req: asyncio.to_thread(decide, req)). "
                "For a fixed verdict use AutoApproveHandler or AlwaysDenyHandler instead."
            )
        return await result

    async def check_approval(self, approval_id: str) -> ApprovalResult:
        """Default Phase B check: return rejected."""
        return ApprovalResult(status="rejected", reason="Phase B not supported by callback handler")
