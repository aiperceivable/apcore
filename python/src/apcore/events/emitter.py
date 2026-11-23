"""Global event bus with fan-out delivery, per-subscriber retry, and DLQ emission."""

from __future__ import annotations

import asyncio
import inspect
import logging
import threading
from concurrent.futures import Future, ThreadPoolExecutor
from collections.abc import Mapping
from dataclasses import dataclass, fields
from datetime import datetime, timezone
from typing import Any, Protocol, runtime_checkable

from apcore.events.retry import EventRetryConfig
from apcore.utils.pattern import match_glob

logger = logging.getLogger(__name__)

_DLQ_EVENT_TYPE = "apcore.event.delivery_failed"


def _iso_now() -> str:
    return datetime.now(timezone.utc).isoformat()


@dataclass(frozen=True)
class ApCoreEvent:
    """Immutable event emitted by the apcore event bus."""

    event_type: str
    module_id: str | None
    timestamp: str
    severity: str
    data: dict[str, Any]


@runtime_checkable
class EventSubscriber(Protocol):
    """Protocol for objects that can receive apcore events."""

    async def on_event(self, event: ApCoreEvent) -> None: ...


# ---------------------------------------------------------------------------
# Per-type subscriber-ID counters (monotonic, process-scoped)
# ---------------------------------------------------------------------------

_id_counter_lock = threading.Lock()
_id_counters: dict[str, int] = {}


def _next_subscriber_id(type_name: str) -> str:
    with _id_counter_lock:
        count = _id_counters.get(type_name, 0)
        _id_counters[type_name] = count + 1
    return f"{type_name}-{count}"


def _ensure_subscriber_id(subscriber: Any) -> str:
    """Give *subscriber* a generated ``subscriber_id`` if it declares none.

    EVT-004. ``event-system.md`` requires the emitter to "MUST generate a
    stable identifier" for a subscriber that supplies no id. It did not: the
    fallback was ``repr(subscriber)``, which for an ordinary object embeds a
    heap address — so the DLQ payload, every retry log line and every
    circuit-breaker event named a value that is neither stable across runs nor
    matchable to anything an operator can see. ``_next_subscriber_id`` already
    existed for exactly this and was called only by the built-in subscriber
    classes.

    Assigned once, at ``subscribe()`` time, and never overwritten: a declared
    id is the operator's and a generated one must not change between two reads
    of the same subscriber. A subscriber whose attribute cannot be set (e.g. a
    ``__slots__`` class) keeps working — the generated id is simply not cached,
    which is the pre-existing behaviour and not worse than it.
    """
    sid = getattr(subscriber, "subscriber_id", None)
    if isinstance(sid, str) and sid:
        return sid
    generated = _next_subscriber_id(_get_subscriber_type(subscriber) or "subscriber")
    try:
        subscriber.subscriber_id = generated
    except Exception:  # pragma: no cover - exotic subscriber objects
        logger.debug("Could not cache generated subscriber_id on %s", type(subscriber).__name__)
    return generated


def _get_subscriber_id(subscriber: Any) -> str:
    sid = getattr(subscriber, "subscriber_id", None)
    if isinstance(sid, str) and sid:
        return sid
    # A subscriber that never went through `subscribe()` (or could not carry
    # the attribute) still gets a stable, address-free identifier.
    return _ensure_subscriber_id(subscriber)


def _get_subscriber_type(subscriber: Any) -> str:
    stype = getattr(subscriber, "subscriber_type", None)
    if isinstance(stype, str):
        return stype
    return type(subscriber).__name__.lower().replace("subscriber", "").lstrip("_")


def _get_event_pattern(subscriber: Any) -> str:
    pattern = getattr(subscriber, "event_pattern", None)
    return pattern if isinstance(pattern, str) else "*"


def _get_retry_config(subscriber: Any) -> EventRetryConfig:
    """Resolve a subscriber's ``retry`` attribute into an EventRetryConfig.

    A plain mapping is accepted and merged over the defaults. This used to be a
    bare ``isinstance(retry, EventRetryConfig)`` check that silently substituted
    the default policy for anything else — including the exact form
    ``docs/features/event-system.md`` shows in its own Python example::

        retry = {"max_attempts": 5, "initial_backoff_ms": 250}

    A subscriber copied from the spec therefore retried 3 times at 100 ms
    instead of 5 at 250 ms, with no error and no log line, while
    apcore-typescript's ``resolveRetry`` spread the equivalent object over its
    defaults and honoured it. The failure was invisible because
    ``event_pattern`` on the same class IS read by a plain ``getattr`` and
    works, so the subscriber looked correctly wired.
    """
    retry = getattr(subscriber, "retry", None)
    if isinstance(retry, EventRetryConfig):
        return retry
    if isinstance(retry, Mapping):
        known = {f.name for f in fields(EventRetryConfig)}
        unknown = set(retry) - known
        if unknown:
            # Warn rather than drop silently: a typo'd key is exactly how a
            # declared policy quietly becomes the default one.
            logger.warning(
                "Subscriber %r declares unknown retry key(s) %s — ignored; valid keys are %s",
                _get_subscriber_id(subscriber),
                sorted(unknown),
                sorted(known),
            )
        return EventRetryConfig(**{k: v for k, v in retry.items() if k in known})
    if retry is not None:
        logger.warning(
            "Subscriber %r declares a `retry` of type %s; expected EventRetryConfig or a "
            "mapping — falling back to the default policy",
            _get_subscriber_id(subscriber),
            type(retry).__name__,
        )
    return EventRetryConfig()


def _matches_event(subscriber: Any, event_type: str, *, is_dlq: bool) -> bool:
    """Return True when *subscriber* should receive an event of *event_type*.

    DLQ events (apcore.event.delivery_failed) are ONLY delivered to subscribers
    that have an explicit, non-wildcard event_pattern. Catch-all subscribers
    (pattern '*') never receive DLQ events — this prevents cascading failures
    where every subscriber would recursively fail on the DLQ about itself.
    """
    pattern = _get_event_pattern(subscriber)
    if is_dlq and pattern == "*":
        return False
    # PROTOCOL_SPEC 9.16.3: event patterns are matched with Algorithm A25,
    # not fnmatch — `[…]` is a literal here, and the three SDKs supported
    # three different metacharacter sets while nothing said which was right.
    return match_glob(pattern, event_type)


# ---------------------------------------------------------------------------
# EventEmitter
# ---------------------------------------------------------------------------


class EventEmitter:
    """Thread-safe global event bus with per-subscriber retry and DLQ emission.

    Delivery model:
    - Each emit() fans out to all subscribers concurrently (one asyncio.gather).
    - Each subscriber gets up to retry.max_attempts delivery attempts with
      exponential backoff.
    - On exhaustion: a ``apcore.event.delivery_failed`` DLQ event is emitted
      to subscribers that explicitly watch for it (non-wildcard event_pattern).
    - DLQ delivery is single-attempt (no retry, no second-order DLQ).
    """

    def __init__(self, max_workers: int = 4) -> None:
        self._subscribers: list[Any] = []
        self._lock = threading.Lock()
        self._executor = ThreadPoolExecutor(
            max_workers=max_workers,
            thread_name_prefix="apcore-event",
        )
        self._pending_futures: list[Future[None]] = []
        self._pending_lock = threading.Lock()
        self._loop = asyncio.new_event_loop()
        self._loop_lock = threading.Lock()
        self._shutdown = False

    def subscribe(self, subscriber: Any) -> None:
        """Add a subscriber to receive future events.

        Raises:
            TypeError: If ``subscriber.on_event`` is not a coroutine function.
        """
        if not asyncio.iscoroutinefunction(getattr(subscriber, "on_event", None)):
            raise TypeError(
                f"Subscriber {subscriber!r} must define an async on_event(event) "
                "method; synchronous on_event is not supported."
            )
        _ensure_subscriber_id(subscriber)
        with self._lock:
            self._subscribers.append(subscriber)

    def unsubscribe(self, subscriber: Any) -> None:
        """Remove a subscriber. No-op if not found."""
        with self._lock:
            try:
                self._subscribers.remove(subscriber)
            except ValueError:
                pass

    def emit(self, event: ApCoreEvent) -> None:
        """Fan-out event to all subscribers via thread pool.

        Returns immediately. After :meth:`shutdown`, emits are silently dropped.
        """
        if self._shutdown:
            return
        with self._lock:
            snapshot = list(self._subscribers)

        if not snapshot:
            return

        future = self._executor.submit(self._deliver, snapshot, event)
        with self._pending_lock:
            self._pending_futures = [f for f in self._pending_futures if not f.done()]
            self._pending_futures.append(future)

    def flush(self, timeout: float = 5.0) -> None:
        """Wait for all pending event deliveries to complete."""
        with self._pending_lock:
            futures = list(self._pending_futures)
        for future in futures:
            try:
                future.result(timeout=timeout)
            except Exception:
                pass
        with self._pending_lock:
            self._pending_futures = [f for f in self._pending_futures if not f.done()]

    # ------------------------------------------------------------------
    # Internal delivery machinery
    # ------------------------------------------------------------------

    def _deliver(self, subscribers: list[Any], event: ApCoreEvent) -> None:
        """Run the async delivery coroutine inside the persistent event loop."""
        is_dlq = event.event_type == _DLQ_EVENT_TYPE
        with self._loop_lock:
            self._loop.run_until_complete(self._deliver_async(subscribers, event, is_dlq=is_dlq))

    async def _deliver_async(
        self,
        subscribers: list[Any],
        event: ApCoreEvent,
        *,
        is_dlq: bool,
    ) -> None:
        """Fan out delivery to all matching subscribers."""
        tasks = []
        for sub in subscribers:
            if not _matches_event(sub, event.event_type, is_dlq=is_dlq):
                continue
            if is_dlq:
                tasks.append(self._deliver_single_no_retry(sub, event))
            else:
                tasks.append(self._deliver_with_retry(sub, event))
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)

    async def _deliver_with_retry(self, subscriber: Any, event: ApCoreEvent) -> None:
        """Deliver event with retry per subscriber's EventRetryConfig."""
        retry = _get_retry_config(subscriber)
        last_error: Exception | None = None

        for attempt in range(retry.max_attempts):
            try:
                await subscriber.on_event(event)
                return
            except Exception as exc:
                last_error = exc
                if attempt + 1 < retry.max_attempts:
                    delay_s = retry.compute_delay_ms(attempt) / 1000.0
                    await asyncio.sleep(delay_s)

        # All attempts exhausted — emit DLQ event then call on_failure
        await self._emit_dlq(subscriber, event, last_error, retry.max_attempts)

        if hasattr(subscriber, "on_failure") and callable(subscriber.on_failure):
            try:
                result = subscriber.on_failure(event, last_error, retry.max_attempts)
                if inspect.isawaitable(result):
                    await result
            except Exception:
                logger.exception("on_failure callback raised for subscriber %r", subscriber)

    async def _deliver_single_no_retry(self, subscriber: Any, event: ApCoreEvent) -> None:
        """Single delivery attempt for DLQ events — errors are logged and discarded."""
        try:
            await subscriber.on_event(event)
        except Exception:
            logger.error(
                "Subscriber %r failed to handle DLQ event %s — discarded",
                subscriber,
                event.event_type,
            )

    async def _emit_dlq(
        self,
        subscriber: Any,
        original_event: ApCoreEvent,
        error: Exception | None,
        attempt_count: int,
    ) -> None:
        """Build and deliver apcore.event.delivery_failed to DLQ-watching subscribers."""
        sub_type = _get_subscriber_type(subscriber)
        ts = _iso_now()
        dlq_event = ApCoreEvent(
            event_type=_DLQ_EVENT_TYPE,
            module_id=None,
            timestamp=ts,
            severity="error",
            data={
                "subscriber_type": sub_type,
                "subscriber_id": _get_subscriber_id(subscriber),
                "original_event": {
                    "name": original_event.event_type,
                    "payload": original_event.data,
                    "metadata": {
                        "module_id": original_event.module_id,
                        "timestamp": original_event.timestamp,
                    },
                },
                "error": {
                    "type": type(error).__name__ if error is not None else "Unknown",
                    "message": str(error) if error is not None else "",
                },
                "attempt_count": attempt_count,
                "timestamp": ts,
            },
        )

        with self._lock:
            snapshot = list(self._subscribers)

        dlq_subs = [s for s in snapshot if _matches_event(s, _DLQ_EVENT_TYPE, is_dlq=True)]
        if dlq_subs:
            await asyncio.gather(
                *[self._deliver_single_no_retry(s, dlq_event) for s in dlq_subs],
                return_exceptions=True,
            )

    def shutdown(self, timeout: float = 5.0) -> None:
        """Release the executor thread pool and asyncio loop."""
        if self._shutdown:
            return
        self._shutdown = True
        try:
            self.flush(timeout=timeout)
        finally:
            self._executor.shutdown(wait=True, cancel_futures=True)
            with self._loop_lock:
                if not self._loop.is_closed():
                    self._loop.close()
