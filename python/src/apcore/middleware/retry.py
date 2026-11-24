"""RetryMiddleware for automatic retry of retryable module errors."""

from __future__ import annotations

import logging
import random
import time
from dataclasses import dataclass
from typing import Any, Literal

from apcore.context_keys import RETRY_COUNT_BASE
from apcore.middleware.base import Context, Middleware, RetrySignal

__all__ = ["RetryConfig", "RetryMiddleware"]

_logger = logging.getLogger(__name__)


@dataclass
class RetryConfig:
    """Configuration for retry behavior."""

    max_retries: int = 3
    strategy: Literal["exponential", "fixed"] = "exponential"
    base_delay_ms: int = 100
    max_delay_ms: int = 5000
    jitter: bool = True  # Add random jitter to delays


class RetryMiddleware(Middleware):
    """Middleware that retries failed module executions based on error retryability.

    When ``on_error`` is called with a retryable error (``error.retryable is True``),
    this middleware sleeps for a calculated delay and returns a
    :class:`~apcore.middleware.manager.RetrySignal` carrying the original
    inputs. The executor recognises the signal and re-runs the module;
    remaining middlewares' ``on_error`` handlers are not invoked for this
    attempt. After *max_retries* attempts or for non-retryable errors, it
    returns ``None`` so the error propagates.

    Retry state is tracked per-module in ``context.data`` using the key
    ``_apcore.mw.retry.count.{module_id}`` to remain thread-safe across concurrent calls.

    .. note::

       ``on_error`` uses ``time.sleep()`` for backoff delays, which blocks
       the calling thread.  In async pipelines (``call_async``/``stream``),
       the middleware manager runs ``on_error`` via ``asyncio.to_thread``,
       so the event loop is not blocked.
    """

    def __init__(self, config: RetryConfig | None = None, *, priority: int = 100) -> None:
        super().__init__(priority=priority)
        self._config = config or RetryConfig()

    def on_error(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Context,
    ) -> RetrySignal | None:
        """Retry retryable errors up to max_retries with configurable backoff."""
        retryable = getattr(error, "retryable", None)
        if retryable is not True:
            return None

        retry_key = RETRY_COUNT_BASE.scoped(module_id)
        retry_count: int = retry_key.get(context, default=0)

        if retry_count >= self._config.max_retries:
            _logger.warning(
                "Max retries (%d) exceeded for module '%s'",
                self._config.max_retries,
                module_id,
            )
            return None

        delay_ms = self._calculate_delay(retry_count)
        retry_key.set(context, retry_count + 1)

        _logger.info(
            "Retrying module '%s' (attempt %d/%d) after %dms",
            module_id,
            retry_count + 1,
            self._config.max_retries,
            delay_ms,
        )

        time.sleep(delay_ms / 1000.0)
        return RetrySignal(inputs=dict(inputs))

    def after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> dict[str, Any] | None:
        """Clear the per-module retry counter on successful completion.

        Without this, context.data accumulates
        ``_apcore.mw.retry.count.{module_id}`` keys that are never reclaimed,
        causing unbounded growth across long call chains that happened to
        recover before the limit was hit.
        """
        RETRY_COUNT_BASE.scoped(module_id).delete(context)
        return None

    def _calculate_delay(self, attempt: int) -> float:
        """Calculate delay in milliseconds for the given attempt number."""
        if self._config.strategy == "fixed":
            delay = float(self._config.base_delay_ms)
        else:
            # Exponential: base_delay_ms * 2^attempt, capped at max_delay_ms
            delay = min(
                self._config.base_delay_ms * (2**attempt),
                self._config.max_delay_ms,
            )

        if self._config.jitter:
            delay *= random.uniform(0.5, 1.5)  # noqa: S311

        return delay
