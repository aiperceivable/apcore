"""Regression tests for executor sync-audit findings against v0.22.0.

- A-D-EXEC-002 / D-21 — Cancel-token check at the CallChainGuard step
  (Step 2) — fires before any expensive ACL / middleware / validation
  work runs.
- A-D-EXEC-005 / D-22 — Error Unwrap Rule. MiddlewareChainError MUST
  surface its ``.original`` typed cause unchanged; the executor MUST NOT
  replace it with a generic ``ModuleExecuteError``.
"""

from __future__ import annotations

from typing import Any

import pytest

from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.context import Context
from apcore.errors import ApprovalDeniedError, ModuleExecuteError
from apcore.executor import Executor
from apcore.middleware import Middleware
from apcore.registry import Registry


class _NoopModule:
    """Module whose execute() should never run when the call is cancelled
    at Step 2."""

    input_schema = None
    output_schema = None

    def __init__(self) -> None:
        self.executed = False

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        self.executed = True
        return {"ok": True}


class _ApprovalResult:
    """Minimal stand-in for an ApprovalResult — carries the ``reason`` attr
    that :class:`ApprovalDeniedError` reads when formatting its message."""

    reason = "testing unwrap"


class _RaisingApprovalMiddleware(Middleware):
    """Middleware whose before() always raises ApprovalDeniedError.

    The middleware chain machinery will wrap this in MiddlewareChainError;
    per D-22 the executor MUST unwrap and surface the original typed
    cause to the caller.
    """

    async def before(self, module_id: str, inputs: dict[str, Any], context: Context) -> None:
        raise ApprovalDeniedError(result=_ApprovalResult(), module_id=module_id)


# ---------------------------------------------------------------------------
# A-D-EXEC-002 / D-21 — Cancel token observed at CallChainGuard (Step 2)
# ---------------------------------------------------------------------------


class TestCancelTokenAtCallChainGuard:
    @pytest.mark.asyncio
    async def test_cancelled_token_short_circuits_before_module_runs(self) -> None:
        module = _NoopModule()
        reg = Registry()
        reg.register("test.noop", module)
        executor = Executor(registry=reg)

        token = CancelToken()
        token.cancel()
        # cancel_token is a first-class Context.create() parameter (v0.22.0 +
        # Issue #66 unified signature). The Executor auto-binds itself at
        # entry, so executor= is no longer accepted as a public input.
        ctx = Context.create(cancel_token=token)

        with pytest.raises(ExecutionCancelledError):
            await executor.call_async("test.noop", {}, context=ctx)

        # Step 2 short-circuit means the module body never ran — no ACL,
        # no input validation, no middleware-before work happened first.
        assert module.executed is False


# ---------------------------------------------------------------------------
# A-D-EXEC-005 / D-22 — MiddlewareChainError unwrap rule
# ---------------------------------------------------------------------------


class _PlainModule:
    input_schema = None
    output_schema = None

    async def execute(self, inputs: dict[str, Any], context: Context) -> dict[str, Any]:
        return {"ok": True}


class TestMiddlewareChainErrorUnwrap:
    @pytest.mark.asyncio
    async def test_typed_cause_propagates_unwrapped(self) -> None:
        reg = Registry()
        reg.register("test.plain", _PlainModule())
        executor = Executor(registry=reg)
        executor.use(_RaisingApprovalMiddleware())

        # The caller MUST receive ApprovalDeniedError unchanged, NOT a
        # generic ModuleExecuteError that swallows the typed cause.
        with pytest.raises(ApprovalDeniedError):
            await executor.call_async("test.plain", {})

    @pytest.mark.asyncio
    async def test_unwrap_does_not_collapse_to_module_execute_error(self) -> None:
        reg = Registry()
        reg.register("test.plain", _PlainModule())
        executor = Executor(registry=reg)
        executor.use(_RaisingApprovalMiddleware())

        try:
            await executor.call_async("test.plain", {})
        except ModuleExecuteError as exc:
            pytest.fail(f"Caller received generic ModuleExecuteError instead of typed cause: {exc!r}")
        except ApprovalDeniedError:
            pass  # expected
