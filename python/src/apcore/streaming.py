"""StreamingModule Protocol — modules that produce output incrementally.

@runtime_checkable only verifies method presence at isinstance() time.
Actual signature compliance is enforced at registration time by
Registry.register when annotations.streaming = True.
"""

from __future__ import annotations

import asyncio
import logging
import time
from collections.abc import AsyncGenerator, AsyncIterator, Callable
from contextlib import AsyncExitStack, aclosing, suppress
from copy import deepcopy
from dataclasses import dataclass
from typing import Any, Literal, Protocol, runtime_checkable

from apcore.cancel import CancelToken, ExecutionCancelledError
from apcore.context import Context
from apcore.errors import ErrorCodes, ModuleError, ModuleTimeoutError
from apcore.middleware.manager import MiddlewareChainError
from apcore.pipeline import PipelineContext, PipelineStepError

_logger = logging.getLogger(__name__)

StreamCompletionReason = Literal["completed", "failed", "cancelled", "timed_out", "consumer_closed"]


@dataclass(frozen=True)
class StreamStepOutcome:
    """Outcome of a configured pipeline step in a streaming execution."""

    status: Literal["passed", "failed", "skipped"]
    disposition: Literal["unhandled", "ignored", "recovered"] | None = None
    reason: Literal["not_configured", "not_reached", "no_match", "strategy_skipped"] | None = None


@dataclass(frozen=True)
class StreamCompletion:
    """Terminal execution state and output detached at the validation boundary."""

    reason: StreamCompletionReason
    output: dict[str, Any] | None
    output_validation: StreamStepOutcome
    middleware_after: StreamStepOutcome
    error_code: str | None


def _stream_error_code(error: BaseException) -> str:
    if isinstance(error, PipelineStepError) and isinstance(error.cause, BaseException):
        error = error.cause
    if isinstance(error, MiddlewareChainError):
        error = error.original
    if isinstance(error, asyncio.CancelledError):
        return ErrorCodes.EXECUTION_CANCELLED
    return error.code if isinstance(error, ModuleError) else ErrorCodes.MODULE_EXECUTE_ERROR


class _StreamObservation:
    """Track the existing pipeline without exposing its mutable state."""

    def __init__(
        self,
        steps: list[str],
        monitor: Callable[[Context], None],
        interrupt: Callable[[ModuleError], None],
        captures_completion: bool,
    ) -> None:
        self.steps = steps
        self.monitor = monitor
        self._interrupt = interrupt
        self.captures_completion = captures_completion
        self.context: PipelineContext | None = None
        self.cancel_token: CancelToken | None = None
        self.output: dict[str, Any] | None = None
        self.execution_error_code: str | None = None
        self.timeout_error: ModuleTimeoutError | None = None
        self.post_started = False
        self._captured = False

    def start_post_processing(self) -> None:
        self.post_started = True

    def report_timeout(self, error: ModuleTimeoutError) -> None:
        self.timeout_error = error
        # Queue the typed terminal error before the same timeout cancels the
        # token and triggers its generic cancellation listener.
        asyncio.get_running_loop().call_soon(self._interrupt, error)

    def defer_step(self, name: str, ctx: PipelineContext) -> bool:
        return (
            name == "return_result"
            and self.captures_completion
            and ctx.output_stream is not None
            and not self.post_started
        )

    def capture_fallback_output(self, output: dict[str, Any] | None) -> None:
        if self.captures_completion and not self._captured and self.execution_error_code is None:
            self.output = deepcopy(output) if output is not None else {}

    def before_step(self, name: str, ctx: PipelineContext) -> None:
        if ctx.context is not None:
            self.monitor(ctx.context)
        if not self.captures_completion or self.execution_error_code is not None:
            return
        if ctx.output_stream is not None and not self.post_started:
            return
        if name == "output_validation" or (name in ("middleware_after", "return_result") and not self._captured):
            self.output = deepcopy(ctx.output) if ctx.output is not None else {}
            self._captured = True

    def cancel(self) -> None:
        if self.cancel_token is not None:
            self.cancel_token.cancel()

    def record_execution_error(self, error: Exception) -> None:
        self.execution_error_code = _stream_error_code(error)

    def record_post_error(self, error: Exception) -> None:
        trace = self.context.trace if self.context is not None else None
        last = trace.steps[-1] if trace is not None and trace.steps else None
        if last is None or last.name not in ("output_validation", "middleware_after") or last.result.action != "abort":
            self.record_execution_error(error)

    def outcome(self, name: str) -> StreamStepOutcome:
        if name not in self.steps:
            return StreamStepOutcome("skipped", reason="not_configured")
        ctx = self.context
        if ctx is None or ctx.trace is None or (ctx.output_stream is not None and not self.post_started):
            return StreamStepOutcome("skipped", reason="not_reached")
        for trace in ctx.trace.steps:
            if trace.name != name:
                continue
            if trace.skip_reason == "error_ignored":
                return StreamStepOutcome("failed", disposition="ignored")
            if trace.skip_reason == "error_recovered":
                return StreamStepOutcome("failed", disposition="recovered")
            if trace.skipped:
                reason = "no_match" if trace.skip_reason == "no_match" else "strategy_skipped"
                return StreamStepOutcome("skipped", reason=reason)
            if trace.result.action == "abort":
                return StreamStepOutcome("failed", disposition="unhandled")
            return StreamStepOutcome("passed")
        return StreamStepOutcome("skipped", reason="not_reached")


class StreamExecution:
    """One lazy, pull-driven execution with a separately awaitable terminal state.

    Drain ``chunks`` or explicitly close the execution before awaiting
    ``completion()``. Closing publishes the outcome before producer cleanup.
    """

    def __init__(
        self,
        source: Callable[[_StreamObservation, AsyncExitStack], AsyncGenerator[dict[str, Any], None]],
        steps: list[str],
        *,
        timeout_error: Callable[[PipelineContext | None], ModuleError],
        captures_completion: bool = True,
    ) -> None:
        self._source = source
        self._observation = _StreamObservation(steps, self._monitor_context, self._interrupt, captures_completion)
        self._timeout_error = timeout_error
        self._ready = asyncio.Event()
        self._result: StreamCompletion | None = None
        self._requests: asyncio.Queue[asyncio.Future[dict[str, Any]]] = asyncio.Queue(maxsize=1)
        self._pending: asyncio.Future[dict[str, Any]] | None = None
        self._producer: asyncio.Task[None] | None = None
        self._terminal_error: Exception | None = None
        self._close_error: Exception | None = None
        self._deadline: float | None = None
        self._timer: asyncio.TimerHandle | None = None
        self._watched_token: CancelToken | None = None
        self._unsubscribe: Callable[[], None] | None = None

    @property
    def chunks(self) -> AsyncIterator[dict[str, Any]]:
        """The execution's single async chunk iterator."""
        return self

    def __aiter__(self) -> StreamExecution:
        return self

    async def __anext__(self) -> dict[str, Any]:
        if self._result is not None:
            if self._terminal_error is not None:
                error, self._terminal_error = self._terminal_error, None
                raise error
            raise StopAsyncIteration
        if self._pending is not None:
            raise RuntimeError("Cannot advance a stream execution concurrently")
        response = asyncio.get_running_loop().create_future()
        self._pending = response
        self._requests.put_nowait(response)
        if self._producer is None:
            self._producer = asyncio.create_task(self._drive())
        try:
            return await asyncio.shield(response)
        except asyncio.CancelledError as error:
            if self._finish(error=error) and self._producer is not None:
                self._producer.cancel()
            raise
        finally:
            self._pending = None

    async def completion(self) -> StreamCompletion:
        """Wait for the terminal outcome as a value, including on failure."""
        await self._ready.wait()
        assert self._result is not None
        return self._result

    async def aclose(self) -> None:
        """Close even before the first pull, without waiting for cleanup."""
        if self._finish(closed=True) and self._producer is not None:
            self._producer.cancel()

    async def _wait_for_cleanup(self) -> None:
        if self._producer is not None:
            await asyncio.gather(self._producer, return_exceptions=True)
        if self._close_error is not None:
            raise self._close_error

    def _monitor_context(self, context: Context) -> None:
        if self._result is not None:
            return
        loop = asyncio.get_running_loop()
        deadline = context.global_deadline
        if deadline is not None and (self._deadline is None or deadline < self._deadline):
            if self._timer is not None:
                self._timer.cancel()
            self._deadline = deadline
            self._timer = loop.call_later(max(0, deadline - time.time()), self._expire)
        token = context.cancel_token
        if token is self._watched_token:
            return
        if self._unsubscribe is not None:
            self._unsubscribe()
        self._watched_token = token

        def signal() -> None:
            with suppress(RuntimeError):
                loop.call_soon_threadsafe(self._interrupt, ExecutionCancelledError())

        self._unsubscribe = token._listen(signal) if token is not None else None

    def _expire(self) -> None:
        if self._result is None:
            self._interrupt(self._timeout_error(self._observation.context))

    def _interrupt(self, error: ModuleError) -> None:
        if self._finish(error=error) and self._producer is not None:
            self._producer.cancel()

    async def _drive(self) -> None:
        # One task keeps invocation and cleanup in the same async context.
        try:
            async with AsyncExitStack() as scope, aclosing(self._source(self._observation, scope)) as source:
                while self._result is None:
                    response = await self._requests.get()
                    try:
                        chunk = await anext(source)
                    except StopAsyncIteration:
                        if self._observation.captures_completion:
                            self._finish()
                        break
                    except (Exception, asyncio.CancelledError) as error:
                        self._finish(error=error)
                    else:
                        if not response.done():
                            response.set_result(chunk)
            self._finish()
        except (Exception, asyncio.CancelledError) as error:
            if not self._finish(error=error) and not isinstance(error, asyncio.CancelledError):
                if (
                    not self._observation.captures_completion
                    and self._result is not None
                    and (self._result.reason == "consumer_closed")
                ):
                    self._close_error = error
                else:
                    _logger.warning("Stream producer cleanup failed (%s)", type(error).__name__)

    def _finish(self, *, error: BaseException | None = None, closed: bool = False) -> bool:
        if self._result is not None:
            return False
        code = self._observation.execution_error_code
        if closed:
            code = None
        elif error is not None:
            code = _stream_error_code(error)
        reason: StreamCompletionReason
        if closed:
            reason = "consumer_closed"
        elif code == ErrorCodes.EXECUTION_CANCELLED:
            reason = "cancelled"
        elif code == ErrorCodes.MODULE_TIMEOUT:
            reason = "timed_out"
        else:
            reason = "failed" if code is not None else "completed"
        self._result = StreamCompletion(
            reason,
            self._observation.output,
            self._observation.outcome("output_validation"),
            self._observation.outcome("middleware_after"),
            code,
        )
        if self._timer is not None:
            self._timer.cancel()
            self._timer = None
        if self._unsubscribe is not None:
            self._unsubscribe()
            self._unsubscribe = None
        self._watched_token = None
        if reason in ("consumer_closed", "cancelled", "timed_out"):
            self._observation.cancel()
        self._observation.context = None
        self._ready.set()
        response = self._pending
        if response is not None and not response.done():
            if isinstance(error, asyncio.CancelledError):
                response.cancel()
            else:
                response.set_exception(error if error is not None else StopAsyncIteration())
        elif isinstance(error, Exception):
            self._terminal_error = error
        return True


@runtime_checkable
class StreamingModule(Protocol):
    """Modules that produce output incrementally MUST satisfy this Protocol.

    The ``stream`` method is an async generator returning dict chunks.
    Signature validation (arity, async, return type) is checked at
    registration time when ``ModuleAnnotations.streaming = True``.
    """

    async def stream(
        self,
        inputs: dict,
        context: Context,
    ) -> AsyncIterator[dict]: ...
