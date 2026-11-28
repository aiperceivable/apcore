"""Keep policy evaluation and approval bound to the dispatch snapshot."""

from __future__ import annotations

import asyncio
import hashlib
import json
import math
import re
import struct
import time
from contextlib import suppress
from copy import copy, deepcopy
from typing import TYPE_CHECKING, Any

from apcore.errors import PolicyEvaluationError
from apcore.context import _APPROVAL_TOKEN_KEY
from apcore.policy import PolicyEvaluationRequest, PolicyEvaluator, PolicyVerdict

if TYPE_CHECKING:
    from apcore.cancel import CancelToken
    from apcore.context import Context
    from apcore.pipeline import PipelineContext

_SAFE_INT = (1 << 53) - 1
_RULE_ID = re.compile(r"[A-Za-z0-9_.-]{1,64}\Z")
_EVALUATOR_TIMEOUT_SECONDS = 1.0


def _scope_text(value: Any) -> str:
    """Encode supported native values without conflating distinct numbers."""
    if value is None:
        return "null"
    if type(value) is bool:
        return "true" if value else "false"
    if type(value) is int and abs(value) <= _SAFE_INT:
        return f"i{value};"
    if type(value) is float and math.isfinite(value) and (not value.is_integer() or abs(value) <= _SAFE_INT):
        return f"f{struct.pack('>d', value).hex()};"
    if type(value) is str:
        return json.dumps(value, ensure_ascii=False)
    if type(value) is list:
        return "[" + ",".join(_scope_text(item) for item in value) + "]"
    if type(value) is dict and all(type(key) is str for key in value):
        keys = sorted(value, key=lambda key: key.encode("utf-16-be"))
        return "{" + ",".join(f"{_scope_text(key)}:{_scope_text(value[key])}" for key in keys) + "}"
    raise ValueError("Unsupported value in policy scope")


def _argument_digest(arguments: dict[str, Any]) -> str:
    try:
        encoded = _scope_text(arguments).encode("utf-8")
    except (ValueError, RecursionError, KeyError, RuntimeError):
        raise PolicyEvaluationError() from None
    return hashlib.sha256(encoded).hexdigest()


class _PolicyInvocation:
    """Record the selected invocation before middleware or provider callbacks."""

    def __init__(self, ctx: PipelineContext, version: str | None) -> None:
        self.module_id = ctx.module_id
        self.module = ctx.module
        self.version = version
        self.caller_id = ctx.context.caller_id
        try:
            identity = ctx.context.identity
            self.authority_key = identity.authority_key() if identity is not None else None
            self.identity = deepcopy(identity)
        except Exception:
            raise PolicyEvaluationError() from None
        self.arguments: dict[str, Any] | None = None
        self.digest: str | None = None
        self.authorized = False

    def capture(self, ctx: PipelineContext) -> None:
        arguments = ctx.validated_inputs
        if arguments is None or _APPROVAL_TOKEN_KEY in arguments:
            raise PolicyEvaluationError()
        self.digest = _argument_digest(arguments)
        self.arguments = deepcopy(arguments)

    def _check_binding(self, ctx: PipelineContext) -> None:
        identity = ctx.context.identity
        if (
            ctx.module_id != self.module_id
            or ctx.module is not self.module
            or ctx.context.caller_id != self.caller_id
            or identity != self.identity
            or (self.authority_key is not None and (identity is None or identity.authority_key() != self.authority_key))
        ):
            raise PolicyEvaluationError()

    def checked_arguments(self, ctx: PipelineContext) -> dict[str, Any]:
        self._check_binding(ctx)
        capture = deepcopy(self.arguments)
        if (
            ctx.validated_inputs is None
            or capture is None
            or self.digest is None
            or _argument_digest(ctx.validated_inputs) != self.digest
            or _argument_digest(capture) != self.digest
        ):
            raise PolicyEvaluationError()
        return capture

    def preflight_snapshot(self, ctx: PipelineContext) -> tuple[dict[str, Any], Context[Any]]:
        self._check_binding(ctx)
        if self.arguments is None or self.digest is None or _argument_digest(self.arguments) != self.digest:
            raise PolicyEvaluationError()
        context = copy(ctx.context)
        context.identity = deepcopy(self.identity)
        context.call_chain = list(ctx.context.call_chain)
        return self.arguments, context


async def _evaluate_policy(
    evaluator: PolicyEvaluator,
    request: PolicyEvaluationRequest,
    token: CancelToken | None,
    deadline: float | None,
) -> PolicyVerdict:
    """Bound a trusted provider independently of its cancellation cooperation."""
    if token is not None:
        token.check()
    remaining = _EVALUATOR_TIMEOUT_SECONDS
    if deadline is not None:
        remaining = min(remaining, deadline - time.time())
    if remaining <= 0:
        raise PolicyEvaluationError()

    loop = asyncio.get_running_loop()
    expires_at = loop.time() + remaining
    cancelled = loop.create_future()

    def report_cancellation() -> None:
        if not cancelled.done():
            cancelled.set_result(None)

    def signal_cancellation() -> None:
        with suppress(RuntimeError):
            loop.call_soon_threadsafe(report_cancellation)

    unsubscribe = token._listen(signal_cancellation) if token is not None else None
    task = asyncio.create_task(_read_verdict(evaluator, request))
    try:
        await asyncio.wait((task, cancelled), timeout=remaining, return_when=asyncio.FIRST_COMPLETED)
        if token is not None:
            token.check()
        # A non-yielding evaluator can finish before the loop services its timer.
        if not task.done() or loop.time() >= expires_at:
            raise PolicyEvaluationError()
        return task.result()
    finally:
        if unsubscribe is not None:
            unsubscribe()
        cancelled.cancel()
        if not task.done():
            task.cancel()
            task.add_done_callback(_consume_result)
        elif not task.cancelled():
            task.exception()


async def _read_verdict(evaluator: PolicyEvaluator, request: PolicyEvaluationRequest) -> PolicyVerdict:
    try:
        verdict = await evaluator.evaluate(request)
        if isinstance(verdict, PolicyVerdict) and verdict.decision in ("allow", "deny", "require_approval"):
            return verdict
    except (Exception, asyncio.CancelledError):
        pass
    raise PolicyEvaluationError()


def _consume_result(task: asyncio.Future[PolicyVerdict]) -> None:
    if not task.cancelled():
        task.exception()


def _safe_rule_id(rule_id: str | None) -> str | None:
    return rule_id if isinstance(rule_id, str) and _RULE_ID.fullmatch(rule_id) else None
