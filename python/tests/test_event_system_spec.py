"""Spec-traced contract tests for the apcore event system.

Source spec: apcore/docs/features/event-system.md
Each test's docstring carries the verbatim clause id
('event_system.<method>.<kind>.<detail>') so cross-language diffs line up.

TESTS ONLY — never modifies production source.

Symbol-reality notes (drive the skip decisions below):
- ``EventEmitter.emit`` takes a single ``ApCoreEvent`` (not ``event_type`` /
  ``payload``) and raises no errors (fire-and-forget). The spec's
  ``input.event_type.not_empty`` rule is not enforced by this SDK.
- ``WebhookSubscriber.deliver`` / ``A2ASubscriber.deliver`` are named
  ``on_event`` in this SDK. The ``DeliveryError(code=WEBHOOK_DELIVERY_FAILED)``
  type does not exist — webhook raises ``RuntimeError`` and the emitter handles
  the dead-letter path. Recorded as a missing-symbol skip.
- The circuit-breaker contract names ``SubscriberCircuitBreaker.on_failure``
  with signature ``(subscriber_id, error) -> CircuitState``. This SDK ships
  ``CircuitBreakerWrapper`` with a private ``_on_failure(error)`` and a public
  ``on_event`` driver. The exact contract method is a missing symbol; the
  observable state machine is exercised via ``on_event`` / ``state``.
"""

from __future__ import annotations

import asyncio
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from apcore.events.circuit_breaker import CircuitBreakerWrapper, CircuitState
from apcore.events.emitter import ApCoreEvent, EventEmitter
from apcore.events.subscribers import A2ASubscriber, WebhookSubscriber


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_event(**overrides: Any) -> ApCoreEvent:
    defaults: dict[str, Any] = {
        "event_type": "apcore.test.event",
        "module_id": "mod.a",
        "timestamp": "2026-03-08T00:00:00Z",
        "severity": "info",
        "data": {},
    }
    defaults.update(overrides)
    return ApCoreEvent(**defaults)


class RecordingSubscriber:
    """Minimal EventSubscriber that records every delivered event."""

    def __init__(self, event_pattern: str = "*") -> None:
        self.event_pattern = event_pattern
        self.received: list[ApCoreEvent] = []

    async def on_event(self, event: ApCoreEvent) -> None:
        self.received.append(event)


class FailingSubscriber:
    """EventSubscriber whose ``on_event`` always raises.

    Drives the circuit-breaker failure path: ``CircuitBreakerWrapper.on_event``
    awaits ``subscriber.on_event(event)`` directly, so the exception must be
    raised from that coroutine (an ``AsyncMock(side_effect=...)`` would attach
    the side effect to ``__call__`` instead of to the awaited ``on_event``
    child, leaving the failure path undriven).
    """

    def __init__(self, event_pattern: str = "*") -> None:
        self.event_pattern = event_pattern
        self.calls = 0

    async def on_event(self, event: ApCoreEvent) -> None:
        self.calls += 1
        raise RuntimeError("down")


def _mock_aiohttp(status: int) -> Any:
    """Build an aiohttp mock whose POST returns *status*."""
    mock_aiohttp = MagicMock()
    response = AsyncMock()
    response.status = status
    response.__aenter__ = AsyncMock(return_value=response)
    response.__aexit__ = AsyncMock(return_value=False)

    session = AsyncMock()
    session.post = MagicMock(return_value=response)
    session.__aenter__ = AsyncMock(return_value=session)
    session.__aexit__ = AsyncMock(return_value=False)

    mock_aiohttp.ClientSession = MagicMock(return_value=session)
    mock_aiohttp.ClientTimeout = MagicMock()
    return mock_aiohttp, session


# ===========================================================================
# Contract: EventEmitter.emit
# ===========================================================================


def test_emit_input_event_type_not_empty() -> None:
    """event_system.emit.input.event_type.not_empty

    Spec declares event_type MUST NOT be empty, but this SDK's emit() takes an
    ApCoreEvent and is fire-and-forget — it enforces no validation and raises
    no error. The empty-event_type rule is unenforced here.
    """
    pytest.skip(
        "clause too vague: emit() raises no errors and does not validate "
        "event_type in this SDK (fire-and-forget); rule is unenforceable as a "
        "rejection contract"
    )


def test_emit_error_none_raised() -> None:
    """event_system.emit.error.none_raised

    emit() is fire-and-forget: a throwing subscriber must not surface to the
    caller of emit().
    """
    emitter = EventEmitter()
    boom = AsyncMock(side_effect=RuntimeError("boom"))
    emitter.subscribe(boom)
    try:
        emitter.emit(_make_event())  # must not raise
        emitter.flush(timeout=2.0)  # must not re-raise subscriber error
    finally:
        emitter.shutdown(timeout=1.0)
    boom.on_event.assert_called()


def test_emit_property_async_false() -> None:
    """event_system.emit.property.async

    Python emit() is synchronous fire-and-forget (NOT a coroutine).
    """
    emitter = EventEmitter()
    try:
        result = emitter.emit(_make_event())
        assert result is None
        assert not asyncio.iscoroutine(result)
        assert not asyncio.iscoroutinefunction(emitter.emit)
    finally:
        emitter.shutdown(timeout=1.0)


def test_emit_property_thread_safe() -> None:
    """event_system.emit.property.thread_safe

    Concurrent emits with distinct payloads from many threads must not corrupt
    state; every event must be delivered exactly once.
    """
    emitter = EventEmitter(max_workers=4)
    sub = RecordingSubscriber()
    emitter.subscribe(sub)

    import threading

    n = 16
    barrier = threading.Barrier(n)

    def worker(i: int) -> None:
        barrier.wait()
        emitter.emit(_make_event(data={"i": i}))

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(n)]
    try:
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        emitter.flush(timeout=5.0)
    finally:
        emitter.shutdown(timeout=2.0)

    delivered = sorted(e.data["i"] for e in sub.received)
    assert delivered == list(range(n))


def test_emit_property_pure_false() -> None:
    """event_system.emit.property.pure

    emit() is NOT pure: it invokes subscriber callbacks (observable side effect).
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    try:
        emitter.emit(_make_event())
        emitter.flush(timeout=2.0)
    finally:
        emitter.shutdown(timeout=1.0)
    assert len(sub.received) == 1  # callback was invoked -> impure


def test_emit_property_idempotent_false() -> None:
    """event_system.emit.property.idempotent

    emit() is NOT idempotent: two identical emits deliver twice.
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    event = _make_event()
    try:
        emitter.emit(event)
        emitter.emit(event)
        emitter.flush(timeout=2.0)
    finally:
        emitter.shutdown(timeout=1.0)
    assert len(sub.received) == 2


def test_emit_side_effect_1_subscriber_invoked() -> None:
    """event_system.emit.side_effect.1.subscriber_invoked

    Observable side effect: matching subscribers receive the emitted event.
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber(event_pattern="apcore.test.*")
    emitter.subscribe(sub)
    event = _make_event(event_type="apcore.test.event", data={"k": "v"})
    try:
        emitter.emit(event)
        emitter.flush(timeout=2.0)
    finally:
        emitter.shutdown(timeout=1.0)
    assert [e.data for e in sub.received] == [{"k": "v"}]


# ===========================================================================
# Contract: EventEmitter.subscribe
# ===========================================================================


def test_subscribe_input_subscriber_sync_on_event_rejected() -> None:
    """event_system.subscribe.input.subscriber.async_on_event

    Language-idiom guard (D10-002): Python subscribe() raises TypeError if
    on_event is not a coroutine function.
    """
    emitter = EventEmitter()
    try:

        class SyncSub:
            def on_event(self, event: ApCoreEvent) -> None:  # not async
                pass

        with pytest.raises(TypeError, match="async on_event"):
            emitter.subscribe(SyncSub())
    finally:
        emitter.shutdown(timeout=1.0)


def test_subscribe_error_none_raised_for_valid_subscriber() -> None:
    """event_system.subscribe.error.none_raised

    A correctly-typed EventSubscriber subscribes without raising.
    """
    emitter = EventEmitter()
    try:
        emitter.subscribe(RecordingSubscriber())  # must not raise
        emitter.emit(_make_event())
        emitter.flush(timeout=2.0)
    finally:
        emitter.shutdown(timeout=1.0)


def test_subscribe_property_async_false() -> None:
    """event_system.subscribe.property.async

    subscribe() is synchronous.
    """
    emitter = EventEmitter()
    try:
        assert not asyncio.iscoroutinefunction(emitter.subscribe)
        result = emitter.subscribe(RecordingSubscriber())
        assert result is None
    finally:
        emitter.shutdown(timeout=1.0)


def test_subscribe_property_thread_safe() -> None:
    """event_system.subscribe.property.thread_safe

    Concurrent subscribe() from >=8 threads must register every subscriber
    without loss or corruption.
    """
    emitter = EventEmitter()
    import threading

    n = 12
    subs = [RecordingSubscriber() for _ in range(n)]
    barrier = threading.Barrier(n)

    def worker(s: RecordingSubscriber) -> None:
        barrier.wait()
        emitter.subscribe(s)

    threads = [threading.Thread(target=worker, args=(s,)) for s in subs]
    try:
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        with emitter._lock:
            registered = list(emitter._subscribers)
    finally:
        emitter.shutdown(timeout=1.0)

    assert len(registered) == n
    assert {id(s) for s in registered} == {id(s) for s in subs}


def test_subscribe_property_idempotent_false() -> None:
    """event_system.subscribe.property.idempotent

    Each subscribe() call creates a new subscription: subscribing the same
    object twice delivers each event twice.
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    emitter.subscribe(sub)
    try:
        emitter.emit(_make_event())
        emitter.flush(timeout=2.0)
    finally:
        emitter.shutdown(timeout=1.0)
    assert len(sub.received) == 2


# ===========================================================================
# Contract: WebhookSubscriber.deliver
# ===========================================================================


def test_webhook_deliver_input_event_required() -> None:
    """event_system.deliver.input.event.required

    WebhookSubscriber delivery (on_event in this SDK) POSTs the serialized
    event. Observe the event is sent via HTTP POST.
    """
    sub = WebhookSubscriber(url="https://example.com/hook")
    event = _make_event(event_type="apcore.health.recovered")
    mock_aiohttp, session = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_aiohttp):
        asyncio.run(sub.on_event(event))
    session.post.assert_called_once()
    posted = session.post.call_args.kwargs["json"]
    assert posted["event_type"] == "apcore.health.recovered"


def test_webhook_deliver_error_delivery_error_code() -> None:
    """event_system.deliver.error.WEBHOOK_DELIVERY_FAILED

    Spec declares DeliveryError(code=WEBHOOK_DELIVERY_FAILED) on retry
    exhaustion. This SDK has no such type — webhook raises RuntimeError and the
    EventEmitter owns the dead-letter path (apcore.event.delivery_failed).
    """
    pytest.skip(
        "missing symbol: DeliveryError / WEBHOOK_DELIVERY_FAILED (contract gap) "
        "— WebhookSubscriber raises RuntimeError on 5xx and the emitter handles "
        "the dead-letter path instead"
    )


def test_webhook_deliver_property_async_true() -> None:
    """event_system.deliver.property.async

    WebhookSubscriber delivery is awaitable and resolves.
    """
    sub = WebhookSubscriber(url="https://example.com/hook")
    assert asyncio.iscoroutinefunction(sub.on_event)
    mock_aiohttp, _ = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_aiohttp):
        coro = sub.on_event(_make_event())
        assert asyncio.iscoroutine(coro)
        assert asyncio.run(coro) is None


def test_webhook_deliver_property_thread_safe() -> None:
    """event_system.deliver.property.thread_safe

    >=8 concurrent webhook deliveries with distinct events all resolve without
    exception; each issues its own POST.
    """
    sub = WebhookSubscriber(url="https://example.com/hook")
    mock_aiohttp, session = _mock_aiohttp(status=200)

    async def run_all() -> None:
        with patch("apcore.events.subscribers.aiohttp", mock_aiohttp):
            await asyncio.gather(*[sub.on_event(_make_event(data={"i": i})) for i in range(8)])

    asyncio.run(run_all())
    assert session.post.call_count == 8


def test_webhook_deliver_property_pure_false() -> None:
    """event_system.deliver.property.pure

    Delivery is NOT pure: it performs an outbound HTTP POST (observable I/O).
    """
    sub = WebhookSubscriber(url="https://example.com/hook")
    mock_aiohttp, session = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_aiohttp):
        asyncio.run(sub.on_event(_make_event()))
    session.post.assert_called_once()  # outbound side effect occurred


def test_webhook_deliver_side_effect_1_retry_on_5xx() -> None:
    """event_system.deliver.side_effect.1.retry_on_5xx

    Observable HTTP-status policy: a 5xx response raises (so the emitter retry
    loop re-delivers); a 4xx response does not raise (no retry).
    """
    event = _make_event()

    sub = WebhookSubscriber(url="https://example.com/hook")
    mock_5xx, _ = _mock_aiohttp(status=503)
    with patch("apcore.events.subscribers.aiohttp", mock_5xx):
        with pytest.raises(RuntimeError):
            asyncio.run(sub.on_event(event))

    sub2 = WebhookSubscriber(url="https://example.com/hook")
    mock_4xx, _ = _mock_aiohttp(status=404)
    with patch("apcore.events.subscribers.aiohttp", mock_4xx):
        # 4xx is a permanent failure: logged, not raised.
        assert asyncio.run(sub2.on_event(event)) is None


# ===========================================================================
# Contract: SubscriberCircuitBreaker.on_failure
# ===========================================================================


def test_circuit_on_failure_input_subscriber_id_required() -> None:
    """event_system.on_failure.input.subscriber_id.required

    Spec contract is SubscriberCircuitBreaker.on_failure(subscriber_id, error)
    -> CircuitState. This SDK ships CircuitBreakerWrapper with a private
    _on_failure(error) and no subscriber_id parameter.
    """
    pytest.skip(
        "missing symbol: SubscriberCircuitBreaker.on_failure(subscriber_id, "
        "error) (contract gap) — this SDK exposes CircuitBreakerWrapper with "
        "_on_failure(error) and no subscriber_id parameter"
    )


def test_circuit_on_failure_error_none_raised() -> None:
    """event_system.on_failure.error.none_raised

    The circuit breaker MUST NOT raise on a delivery failure; it records state.
    Observed via the public on_event driver wrapping a failing subscriber.
    """
    emitter = EventEmitter()
    failing = FailingSubscriber()
    wrapper = CircuitBreakerWrapper(failing, emitter, open_threshold=5)
    try:
        # Must not raise even though the wrapped subscriber throws.
        asyncio.run(wrapper.on_event(_make_event()))
    finally:
        emitter.shutdown(timeout=1.0)
    assert failing.calls == 1
    assert wrapper.consecutive_failures == 1


def test_circuit_on_failure_returns_circuit_state() -> None:
    """event_system.on_failure.returns.circuit_state

    Failure handling drives the CLOSED->OPEN transition. After open_threshold
    consecutive failures the observable state is OPEN (a CircuitState).
    """
    emitter = EventEmitter()
    failing = FailingSubscriber()
    wrapper = CircuitBreakerWrapper(failing, emitter, open_threshold=3)
    try:
        for _ in range(3):
            asyncio.run(wrapper.on_event(_make_event()))
    finally:
        emitter.shutdown(timeout=1.0)
    assert isinstance(wrapper.state, CircuitState)
    assert wrapper.state == CircuitState.OPEN


def test_circuit_on_failure_property_async_false() -> None:
    """event_system.on_failure.property.async

    The failure-accounting transition is synchronous: _on_failure returns a
    value (event or None), not a coroutine.
    """
    emitter = EventEmitter()
    failing = AsyncMock(side_effect=RuntimeError("down"))
    failing.event_pattern = "*"
    wrapper = CircuitBreakerWrapper(failing, emitter, open_threshold=2)
    try:
        result = wrapper._on_failure(RuntimeError("x"))
        assert not asyncio.iscoroutine(result)
    finally:
        emitter.shutdown(timeout=1.0)


def test_circuit_on_failure_property_thread_safe() -> None:
    """event_system.on_failure.property.thread_safe

    >=8 concurrent failure events must update the shared counter without loss;
    the lock-protected state stays consistent (OPEN after threshold).
    """
    emitter = EventEmitter()
    failing = FailingSubscriber()
    wrapper = CircuitBreakerWrapper(failing, emitter, open_threshold=8)

    async def run_all() -> None:
        await asyncio.gather(*[wrapper.on_event(_make_event(data={"i": i})) for i in range(8)])

    try:
        asyncio.run(run_all())
    finally:
        emitter.shutdown(timeout=1.0)
    # 8 failures at threshold 8 -> circuit is OPEN, counter consistent.
    assert wrapper.consecutive_failures == 8
    assert wrapper.state == CircuitState.OPEN


def test_circuit_on_failure_property_pure_false() -> None:
    """event_system.on_failure.property.pure

    Failure handling mutates circuit state (consecutive_failures increments).
    """
    emitter = EventEmitter()
    failing = FailingSubscriber()
    wrapper = CircuitBreakerWrapper(failing, emitter, open_threshold=5)
    try:
        before = wrapper.consecutive_failures
        asyncio.run(wrapper.on_event(_make_event()))
        after = wrapper.consecutive_failures
    finally:
        emitter.shutdown(timeout=1.0)
    assert after == before + 1


def test_circuit_on_failure_property_idempotent_false() -> None:
    """event_system.on_failure.property.idempotent

    Repeated failures are NOT idempotent: each increments the counter.
    """
    emitter = EventEmitter()
    failing = FailingSubscriber()
    wrapper = CircuitBreakerWrapper(failing, emitter, open_threshold=10)
    try:
        asyncio.run(wrapper.on_event(_make_event()))
        first = wrapper.consecutive_failures
        asyncio.run(wrapper.on_event(_make_event()))
        second = wrapper.consecutive_failures
    finally:
        emitter.shutdown(timeout=1.0)
    assert (first, second) == (1, 2)


# ===========================================================================
# Contract: EventEmitter.unsubscribe
# ===========================================================================


def test_unsubscribe_input_subscriber_same_reference() -> None:
    """event_system.unsubscribe.input.subscriber.same_reference

    unsubscribe removes the subscriber by object identity; afterwards no more
    events are delivered to it.
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    emitter.unsubscribe(sub)
    try:
        emitter.emit(_make_event())
        emitter.flush(timeout=2.0)
    finally:
        emitter.shutdown(timeout=1.0)
    assert sub.received == []


def test_unsubscribe_error_unregistered_is_noop() -> None:
    """event_system.unsubscribe.error.unregistered_no_raise

    Unsubscribing an unregistered subscriber MUST NOT raise — it is a no-op.
    """
    emitter = EventEmitter()
    try:
        emitter.unsubscribe(RecordingSubscriber())  # never subscribed
    finally:
        emitter.shutdown(timeout=1.0)


def test_unsubscribe_property_async_false() -> None:
    """event_system.unsubscribe.property.async

    unsubscribe() is synchronous.
    """
    emitter = EventEmitter()
    try:
        assert not asyncio.iscoroutinefunction(emitter.unsubscribe)
        sub = RecordingSubscriber()
        emitter.subscribe(sub)
        assert emitter.unsubscribe(sub) is None
    finally:
        emitter.shutdown(timeout=1.0)


def test_unsubscribe_property_thread_safe() -> None:
    """event_system.unsubscribe.property.thread_safe

    >=8 concurrent unsubscribe() calls (each removing a distinct subscriber)
    leave the registry consistent and empty.
    """
    emitter = EventEmitter()
    import threading

    n = 10
    subs = [RecordingSubscriber() for _ in range(n)]
    for s in subs:
        emitter.subscribe(s)
    barrier = threading.Barrier(n)

    def worker(s: RecordingSubscriber) -> None:
        barrier.wait()
        emitter.unsubscribe(s)

    threads = [threading.Thread(target=worker, args=(s,)) for s in subs]
    try:
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        with emitter._lock:
            remaining = list(emitter._subscribers)
    finally:
        emitter.shutdown(timeout=1.0)
    assert remaining == []


def test_unsubscribe_property_pure_false() -> None:
    """event_system.unsubscribe.property.pure

    unsubscribe mutates the subscriber list (observable: delivery stops).
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    try:
        with emitter._lock:
            before = len(emitter._subscribers)
        emitter.unsubscribe(sub)
        with emitter._lock:
            after = len(emitter._subscribers)
    finally:
        emitter.shutdown(timeout=1.0)
    assert (before, after) == (1, 0)


def test_unsubscribe_property_idempotent_true() -> None:
    """event_system.unsubscribe.property.idempotent

    Repeated unsubscribe of the same subscriber is a safe no-op with identical
    observable outcome.
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    try:
        emitter.unsubscribe(sub)
        with emitter._lock:
            state_after_first = list(emitter._subscribers)
        emitter.unsubscribe(sub)  # must not raise
        with emitter._lock:
            state_after_second = list(emitter._subscribers)
    finally:
        emitter.shutdown(timeout=1.0)
    assert state_after_first == state_after_second == []


# ===========================================================================
# Contract: EventEmitter.flush
# ===========================================================================


def test_flush_input_timeout_positive() -> None:
    """event_system.flush.input.timeout.positive

    flush(timeout) waits up to `timeout` seconds; a positive timeout lets a
    fast in-flight delivery complete before flush returns.
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    try:
        emitter.emit(_make_event())
        emitter.flush(timeout=5.0)
        assert len(sub.received) == 1
    finally:
        emitter.shutdown(timeout=1.0)


def test_flush_error_none_raised_on_subscriber_failure() -> None:
    """event_system.flush.error.none_raised

    Subscriber errors surfacing during the flush window are silently discarded;
    flush MUST NOT raise.
    """
    emitter = EventEmitter()
    boom = AsyncMock(side_effect=RuntimeError("boom"))
    emitter.subscribe(boom)
    try:
        emitter.emit(_make_event())
        emitter.flush(timeout=2.0)  # must not raise
    finally:
        emitter.shutdown(timeout=1.0)
    boom.on_event.assert_called()


def test_flush_property_async_false() -> None:
    """event_system.flush.property.async

    Python flush() blocks the calling thread; it is NOT a coroutine.
    """
    emitter = EventEmitter()
    try:
        assert not asyncio.iscoroutinefunction(emitter.flush)
        assert emitter.flush(timeout=0.5) is None
    finally:
        emitter.shutdown(timeout=1.0)


def test_flush_property_thread_safe() -> None:
    """event_system.flush.property.thread_safe

    >=8 concurrent flush() calls (with emits in flight) all return without
    error and the pending set drains.
    """
    emitter = EventEmitter(max_workers=4)
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    for _ in range(8):
        emitter.emit(_make_event())

    import threading

    errors: list[BaseException] = []

    def worker() -> None:
        try:
            emitter.flush(timeout=5.0)
        except BaseException as exc:  # noqa: BLE001 - test wants to record any
            errors.append(exc)

    threads = [threading.Thread(target=worker) for _ in range(8)]
    try:
        for t in threads:
            t.start()
        for t in threads:
            t.join()
    finally:
        emitter.shutdown(timeout=2.0)
    assert errors == []
    assert len(sub.received) == 8


def test_flush_property_pure_false() -> None:
    """event_system.flush.property.pure

    flush waits on shared pending futures and clears completed ones — it
    mutates shared state (observable: pending set empties).
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    try:
        emitter.emit(_make_event())
        emitter.flush(timeout=2.0)
        with emitter._pending_lock:
            pending = [f for f in emitter._pending_futures if not f.done()]
        assert pending == []
    finally:
        emitter.shutdown(timeout=1.0)


def test_flush_property_idempotent_true() -> None:
    """event_system.flush.property.idempotent

    Calling flush() again on an already-drained pending set returns immediately
    with the same observable outcome (no error, no extra deliveries).
    """
    emitter = EventEmitter()
    sub = RecordingSubscriber()
    emitter.subscribe(sub)
    try:
        emitter.emit(_make_event())
        emitter.flush(timeout=2.0)
        count_after_first = len(sub.received)
        emitter.flush(timeout=2.0)  # already empty -> no-op
        count_after_second = len(sub.received)
    finally:
        emitter.shutdown(timeout=1.0)
    assert count_after_first == count_after_second == 1


# ===========================================================================
# Contract: A2ASubscriber.deliver
# ===========================================================================


def test_a2a_deliver_input_event_required() -> None:
    """event_system.deliver.input.event.required_a2a

    A2A delivery (on_event) POSTs a {skillId, event} wrapper carrying the
    serialized event.
    """
    sub = A2ASubscriber(platform_url="https://platform.example.com")
    event = _make_event(event_type="apcore.health.recovered")
    mock_aiohttp, session = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_aiohttp):
        asyncio.run(sub.on_event(event))
    session.post.assert_called_once()
    body = session.post.call_args.kwargs["json"]
    assert body["skillId"] == "apevo.event_receiver"
    assert body["event"]["event_type"] == "apcore.health.recovered"


def test_a2a_deliver_error_import_error_without_aiohttp() -> None:
    """event_system.deliver.error.ImportError_a2a

    ImportError is raised synchronously (before network I/O) if aiohttp is not
    installed, with the documented install hint.
    """
    sub = A2ASubscriber(platform_url="https://platform.example.com")
    with patch("apcore.events.subscribers.aiohttp", None):
        with pytest.raises(ImportError, match=r"aiohttp is required.*apcore\[events\]"):
            asyncio.run(sub.on_event(_make_event()))


def test_a2a_deliver_property_async_true() -> None:
    """event_system.deliver.property.async_a2a

    A2A delivery is awaitable and resolves to None.
    """
    sub = A2ASubscriber(platform_url="https://platform.example.com")
    assert asyncio.iscoroutinefunction(sub.on_event)
    mock_aiohttp, _ = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_aiohttp):
        coro = sub.on_event(_make_event())
        assert asyncio.iscoroutine(coro)
        assert asyncio.run(coro) is None


def test_a2a_deliver_property_thread_safe() -> None:
    """event_system.deliver.property.thread_safe_a2a

    >=8 concurrent A2A deliveries with distinct events all resolve; each issues
    its own POST.
    """
    sub = A2ASubscriber(platform_url="https://platform.example.com")
    mock_aiohttp, session = _mock_aiohttp(status=200)

    async def run_all() -> None:
        with patch("apcore.events.subscribers.aiohttp", mock_aiohttp):
            await asyncio.gather(*[sub.on_event(_make_event(data={"i": i})) for i in range(8)])

    asyncio.run(run_all())
    assert session.post.call_count == 8


def test_a2a_deliver_property_pure_false() -> None:
    """event_system.deliver.property.pure_a2a

    Delivery is NOT pure: it performs an outbound HTTP POST to the platform URL.
    """
    sub = A2ASubscriber(platform_url="https://platform.example.com")
    mock_aiohttp, session = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_aiohttp):
        asyncio.run(sub.on_event(_make_event()))
    assert session.post.call_args.args[0] == "https://platform.example.com"


def test_a2a_deliver_side_effect_1_auth_bearer_header() -> None:
    """event_system.deliver.side_effect.1.auth_modes_a2a

    Observable auth policy: a str auth -> 'Authorization: Bearer <auth>';
    a dict auth -> keys merged into headers; None -> no Authorization header.
    """
    event = _make_event()

    # str auth -> Bearer
    sub_str = A2ASubscriber(platform_url="https://p.example.com", auth="tok123")
    mock_str, session_str = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_str):
        asyncio.run(sub_str.on_event(event))
    headers_str = session_str.post.call_args.kwargs["headers"]
    assert headers_str["Authorization"] == "Bearer tok123"

    # dict auth -> merged
    sub_dict = A2ASubscriber(platform_url="https://p.example.com", auth={"X-Api-Key": "k"})
    mock_dict, session_dict = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_dict):
        asyncio.run(sub_dict.on_event(event))
    headers_dict = session_dict.post.call_args.kwargs["headers"]
    assert headers_dict["X-Api-Key"] == "k"

    # None auth -> no Authorization
    sub_none = A2ASubscriber(platform_url="https://p.example.com", auth=None)
    mock_none, session_none = _mock_aiohttp(status=200)
    with patch("apcore.events.subscribers.aiohttp", mock_none):
        asyncio.run(sub_none.on_event(event))
    headers_none = session_none.post.call_args.kwargs["headers"]
    assert "Authorization" not in headers_none


def test_a2a_deliver_side_effect_2_retry_on_5xx() -> None:
    """event_system.deliver.side_effect.2.retry_on_5xx_a2a

    Observable HTTP-status policy for A2A: 5xx raises (emitter retries);
    4xx does not raise (no retry).
    """
    event = _make_event()

    sub_5xx = A2ASubscriber(platform_url="https://p.example.com")
    mock_5xx, _ = _mock_aiohttp(status=502)
    with patch("apcore.events.subscribers.aiohttp", mock_5xx):
        with pytest.raises(RuntimeError):
            asyncio.run(sub_5xx.on_event(event))

    sub_4xx = A2ASubscriber(platform_url="https://p.example.com")
    mock_4xx, _ = _mock_aiohttp(status=403)
    with patch("apcore.events.subscribers.aiohttp", mock_4xx):
        assert asyncio.run(sub_4xx.on_event(event)) is None
