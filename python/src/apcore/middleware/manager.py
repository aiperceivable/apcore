"""MiddlewareManager -- onion model execution engine for the middleware pipeline."""

from __future__ import annotations

import asyncio
import inspect
import logging
import threading
from typing import Any

from apcore.errors import ModuleError
from apcore.middleware.base import Context, Middleware, RetrySignal

__all__ = ["MiddlewareManager", "MiddlewareChainError", "RetrySignal"]


_logger = logging.getLogger(__name__)


def _make_identity(mw: Any, identity_key: str | None) -> str:
    if identity_key is not None:
        return identity_key
    cls = type(mw)
    return f"{cls.__module__}.{cls.__qualname__}"


def _caller_site() -> str:
    """Return a 'file:lineno' string for the frame above use()."""
    frame = inspect.stack()[2]  # [0]=_caller_site, [1]=use, [2]=actual caller
    return f"{frame.filename}:{frame.lineno}"


class MiddlewareChainError(ModuleError):
    """Raised when a middleware's before() fails. Carries context for error recovery."""

    _default_retryable = False

    def __init__(self, original: Exception, executed_middlewares: list[Middleware]) -> None:
        super().__init__(
            code="MIDDLEWARE_CHAIN_ERROR",
            message=str(original),
            cause=original,
        )
        self.original = original
        self.executed_middlewares = executed_middlewares


class MiddlewareManager:
    """Orchestrates the middleware pipeline using onion model execution.

    Manages an ordered list of Middleware instances and provides execution
    methods for before, after, and error handling phases.
    """

    def __init__(self) -> None:
        """Initialize an empty middleware manager."""
        self._middlewares: list[Middleware] = []
        self._lock = threading.Lock()
        # identity → (first_call_site, allow_duplicate)
        self._identity_registry: dict[str, str] = {}
        #: id(middleware) -> the identity string `use()` computed for it, so
        #: `remove()` can clear the right entry (D-114). The identity depends on
        #: an optional `identity_key` override that is a parameter of `use` and
        #: is not otherwise recoverable from the instance.
        self._identity_of: dict[int, str] = {}

    def use(
        self,
        middleware: Middleware,
        *,
        allow_duplicate: bool = False,
        identity_key: str | None = None,
    ) -> None:
        """Register a middleware with optional duplicate detection.

        Computes an identity string for the middleware (type-based by default,
        or the provided ``identity_key``). If the same identity has been
        registered before and ``allow_duplicate`` is False, emits a WARNING
        naming both call sites. Registration always proceeds.

        Args:
            middleware: The middleware instance to add.
            allow_duplicate: When True, suppress the duplicate warning.
            identity_key: Override the auto-computed identity. Keys starting
                with ``apcore.`` are reserved for framework middleware.
        """
        site = _caller_site()
        identity = _make_identity(middleware, identity_key)

        with self._lock:
            first_site = self._identity_registry.get(identity)
            if first_site is not None and not allow_duplicate:
                _logger.warning(
                    "Duplicate middleware registration detected for %r: "
                    "first registered at %s, now again at %s. "
                    "Use allow_duplicate=True to suppress this warning.",
                    identity,
                    first_site,
                    site,
                )
            if first_site is None:
                self._identity_registry[identity] = site
            self._identity_of[id(middleware)] = identity

        self.add(middleware)

    def add(self, middleware: Middleware) -> None:
        """Insert a middleware sorted by priority (higher first).

        Middlewares with equal priority preserve registration order
        (stable insertion). Priority range is 0-1000 per the protocol spec.
        """
        with self._lock:
            priority = getattr(middleware, "priority", 0)
            if priority > 1000:
                raise ValueError(
                    f"Middleware '{getattr(middleware, 'name', type(middleware).__name__)}' has priority {priority} "
                    f"which exceeds the maximum allowed value of 1000"
                )
            # Find the first position where the existing middleware has a
            # lower priority. This keeps higher-priority items first and
            # preserves registration order among equal priorities.
            insert_idx = len(self._middlewares)
            for i, existing in enumerate(self._middlewares):
                if existing.priority < middleware.priority:
                    insert_idx = i
                    break
            self._middlewares.insert(insert_idx, middleware)

    def remove(self, middleware: Middleware) -> bool:
        """Remove a middleware by identity (is). Returns True if found and removed.

        Clears the duplicate-identity entry too (D-114). The registry records the
        FIRST registration so a later duplicate can be traced back to it; leaving
        the entry behind corrupts that record in both directions. It names a
        registration that no longer exists, and ``use`` / ``remove`` / ``use`` —
        a legitimate swap — warns about a duplicate that is not one, which is how
        an operator learns to ignore the warning that will next fire for a real
        one.

        Only cleared when no OTHER registration still holds the same identity:
        duplicate registration warns but succeeds, so two instances sharing an
        identity is a reachable state, and removing one of them must not make the
        survivor invisible to duplicate detection.
        """
        with self._lock:
            for i, entry in enumerate(self._middlewares):
                if entry is middleware:
                    self._middlewares.pop(i)
                    identity = self._identity_of.pop(id(entry), None)
                    if identity is not None and not any(
                        self._identity_of.get(id(other)) == identity for other in self._middlewares
                    ):
                        self._identity_registry.pop(identity, None)
                    return True
            return False

    def snapshot(self) -> list[Middleware]:
        """Return a snapshot (copy) of the current middleware list.

        Thread-safe: acquires lock, copies list, releases lock.
        Callers can iterate the returned list without holding the lock.
        """
        with self._lock:
            return list(self._middlewares)

    def execute_before(
        self,
        module_id: str,
        inputs: dict[str, Any],
        context: Context,
    ) -> tuple[dict[str, Any], list[Middleware]]:
        """Execute before() on all middlewares in registration order.

        Returns a tuple of (final_inputs, executed_middlewares).
        Raises MiddlewareChainError if any middleware's before() raises.
        """
        current_inputs = inputs
        executed_middlewares: list[Middleware] = []
        middlewares = self.snapshot()

        for mw in middlewares:
            executed_middlewares.append(mw)
            try:
                result = mw.before(module_id, current_inputs, context)
            except Exception as e:
                raise MiddlewareChainError(original=e, executed_middlewares=executed_middlewares) from e
            if result is not None:
                current_inputs = result

        return current_inputs, executed_middlewares

    def execute_after(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> dict[str, Any]:
        """Execute after() on all middlewares in REVERSE registration order.

        Returns the final output dict. Raises if any middleware's after() raises.
        """
        current_output = output
        middlewares = self.snapshot()

        for mw in reversed(middlewares):
            result = mw.after(module_id, inputs, current_output, context)
            if result is not None:
                current_output = result

        return current_output

    def execute_on_error(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Context,
        executed_middlewares: list[Middleware],
    ) -> dict[str, Any] | RetrySignal | None:
        """Execute on_error() on executed middlewares in reverse order.

        Returns the first non-None handler result: either a recovery ``dict``
        (becomes the call's output) or a :class:`RetrySignal` (caller should
        re-run the module with the signal's inputs). ``None`` means no
        handler chose to act.
        """
        for mw in reversed(executed_middlewares):
            try:
                result = mw.on_error(module_id, inputs, error, context)
            except Exception:
                _logger.error("Exception in on_error handler %r", mw, exc_info=True)
                continue
            if isinstance(result, RetrySignal):
                return result
            if isinstance(result, dict):
                return result

        return None

    async def execute_before_async(
        self,
        module_id: str,
        inputs: dict[str, Any],
        context: Context,
    ) -> tuple[dict[str, Any], list[Middleware]]:
        """Async-aware execute_before.

        Issue #42: gate on the **return value** (``inspect.isawaitable``)
        instead of ``inspect.iscoroutinefunction(mw.before)``. The latter
        misses ``functools.partial`` wrappers and any decorator that hides
        the underlying ``async def`` (no ``__wrapped__``), silently dropping
        the awaited result.
        """
        current_inputs = inputs
        executed_middlewares: list[Middleware] = []
        middlewares = self.snapshot()

        for mw in middlewares:
            executed_middlewares.append(mw)
            try:
                ret = mw.before(module_id, current_inputs, context)
                result = await ret if inspect.isawaitable(ret) else ret
            except Exception as e:
                raise MiddlewareChainError(original=e, executed_middlewares=executed_middlewares) from e
            if result is not None:
                current_inputs = result

        return current_inputs, executed_middlewares

    async def execute_after_async(
        self,
        module_id: str,
        inputs: dict[str, Any],
        output: dict[str, Any],
        context: Context,
    ) -> dict[str, Any]:
        """Async-aware execute_after.

        Issue #42: gate on ``inspect.isawaitable(return_value)`` rather than
        ``iscoroutinefunction`` to handle decorated/partial async handlers.
        """
        current_output = output
        middlewares = self.snapshot()

        for mw in reversed(middlewares):
            ret = mw.after(module_id, inputs, current_output, context)
            result = await ret if inspect.isawaitable(ret) else ret
            if result is not None:
                current_output = result

        return current_output

    async def execute_on_error_async(
        self,
        module_id: str,
        inputs: dict[str, Any],
        error: Exception,
        context: Context,
        executed_middlewares: list[Middleware],
    ) -> dict[str, Any] | RetrySignal | None:
        """Async-aware on_error chain.

        Issue #42: gate on the return value with ``inspect.isawaitable``.
        ``inspect.iscoroutinefunction(mw.on_error)`` returns False for
        ``functools.partial`` wrappers and decorated callables that hide
        ``__wrapped__``, which previously caused the recovery coroutine to
        be silently dropped (the ``isinstance(recovery, dict)`` test then
        evaluated against an un-awaited coroutine).

        Truly synchronous ``on_error`` handlers still run via
        ``asyncio.to_thread`` so blocking operations (e.g. ``time.sleep`` in
        :class:`RetryMiddleware`) do not stall the event loop.

        Matches the sync contract: returns a recovery ``dict``, a
        :class:`RetrySignal`, or ``None``.
        """
        for mw in reversed(executed_middlewares):
            try:
                if inspect.iscoroutinefunction(mw.on_error):
                    recovery = await mw.on_error(module_id, inputs, error, context)
                else:
                    # Run in thread first; if the sync callable actually
                    # returned a coroutine/awaitable (partial / decorator
                    # over ``async def``) await it on this loop.
                    ret = await asyncio.to_thread(mw.on_error, module_id, inputs, error, context)
                    recovery = await ret if inspect.isawaitable(ret) else ret
                if isinstance(recovery, RetrySignal):
                    return recovery
                if isinstance(recovery, dict):
                    return recovery
            except Exception:
                _logger.exception("on_error handler failed in %s", type(mw).__name__)
                continue
        return None
