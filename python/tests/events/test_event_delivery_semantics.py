"""Tests for unified event delivery semantics: retry, DLQ, on_failure (apcore #61)."""

from __future__ import annotations

import re

import pytest

from apcore.events.emitter import ApCoreEvent, EventEmitter, _DLQ_EVENT_TYPE
from apcore.events.retry import EventRetryConfig
from apcore.events.subscribers import A2ASubscriber, FileSubscriber, StdoutSubscriber


def _make_event(name: str = "apcore.test.event", payload: dict | None = None) -> ApCoreEvent:
    return ApCoreEvent(
        event_type=name,
        module_id=None,
        timestamp="2026-01-01T00:00:00Z",
        severity="info",
        data=payload or {},
    )


class _CountingSubscriber:
    """Records delivery attempts and optionally fails on specified attempts."""

    def __init__(
        self,
        fail_count: int = 0,
        *,
        subscriber_id: str = "test-subscriber",
        retry: EventRetryConfig | None = None,
        event_pattern: str = "*",
    ) -> None:
        self.subscriber_id = subscriber_id
        self.retry = retry or EventRetryConfig()
        self.event_pattern = event_pattern
        self._fail_count = fail_count
        self._attempt = 0
        self.attempts: list[ApCoreEvent] = []
        self.failures: list[tuple[ApCoreEvent, Exception, int]] = []

    async def on_event(self, event: ApCoreEvent) -> None:
        self._attempt += 1
        self.attempts.append(event)
        if self._attempt <= self._fail_count:
            raise RuntimeError(f"Simulated failure on attempt {self._attempt}")

    async def on_failure(self, event: ApCoreEvent, error: Exception, attempt_count: int) -> None:
        self.failures.append((event, error, attempt_count))


class _DLQRecorder:
    """Subscribes only to DLQ events and records them."""

    def __init__(self, subscriber_id: str = "dlq-recorder") -> None:
        self.subscriber_id = subscriber_id
        self.event_pattern = _DLQ_EVENT_TYPE
        self.retry = EventRetryConfig()
        self.received: list[ApCoreEvent] = []

    async def on_event(self, event: ApCoreEvent) -> None:
        self.received.append(event)


# ---------------------------------------------------------------------------
# EventRetryConfig unit tests
# ---------------------------------------------------------------------------


def test_retry_config_defaults() -> None:
    cfg = EventRetryConfig()
    assert cfg.max_attempts == 3
    assert cfg.initial_backoff_ms == 100
    assert cfg.max_backoff_ms == 30_000
    assert cfg.backoff_multiplier == 2.0


def test_retry_config_compute_delay_ms() -> None:
    cfg = EventRetryConfig(initial_backoff_ms=10, backoff_multiplier=2.0, max_backoff_ms=100)
    assert cfg.compute_delay_ms(0) == 10
    assert cfg.compute_delay_ms(1) == 20
    assert cfg.compute_delay_ms(2) == 40
    assert cfg.compute_delay_ms(10) == 100  # capped


# ---------------------------------------------------------------------------
# Delivery tests
# ---------------------------------------------------------------------------


def test_successful_delivery_on_first_attempt() -> None:
    emitter = EventEmitter()
    sub = _CountingSubscriber(fail_count=0, retry=EventRetryConfig(max_attempts=3, initial_backoff_ms=10))
    emitter.subscribe(sub)
    emitter.emit(_make_event())
    emitter.flush()
    emitter.shutdown()

    assert len(sub.attempts) == 1
    assert sub.failures == []


def test_retry_succeeds_before_exhaustion() -> None:
    """Subscriber fails twice then succeeds — 3 total attempts, no DLQ."""
    emitter = EventEmitter()
    dlq_recorder = _DLQRecorder()
    emitter.subscribe(dlq_recorder)

    sub = _CountingSubscriber(
        fail_count=2,
        subscriber_id="transient-subscriber-1",
        retry=EventRetryConfig(max_attempts=3, initial_backoff_ms=1, max_backoff_ms=5, backoff_multiplier=2.0),
    )
    emitter.subscribe(sub)

    emitter.emit(_make_event("apcore.test.transient"))
    emitter.flush(timeout=5.0)
    emitter.shutdown()

    assert len(sub.attempts) == 3
    assert len(dlq_recorder.received) == 0


def test_permanent_failure_emits_dlq_event() -> None:
    """Subscriber fails on every attempt — DLQ event emitted with normative payload."""
    emitter = EventEmitter()
    dlq_recorder = _DLQRecorder(subscriber_id="dlq-recorder-1")
    emitter.subscribe(dlq_recorder)

    sub = _CountingSubscriber(
        fail_count=999,
        subscriber_id="always-failing",
        retry=EventRetryConfig(max_attempts=3, initial_backoff_ms=1, max_backoff_ms=5),
    )
    emitter.subscribe(sub)

    emitter.emit(_make_event("apcore.test.permanent"))
    emitter.flush(timeout=5.0)
    emitter.shutdown()

    assert len(sub.attempts) == 3
    assert len(dlq_recorder.received) == 1
    dlq = dlq_recorder.received[0]
    assert dlq.event_type == _DLQ_EVENT_TYPE
    assert dlq.data["subscriber_id"] == "always-failing"
    assert dlq.data["attempt_count"] == 3
    assert dlq.data["original_event"]["name"] == "apcore.test.permanent"
    for key in ("subscriber_type", "subscriber_id", "original_event", "error", "attempt_count", "timestamp"):
        assert key in dlq.data


def test_dlq_original_event_carries_envelope_metadata() -> None:
    """DLQ original_event has shape {name, payload, metadata} with module_id+timestamp nested."""
    emitter = EventEmitter()
    dlq_recorder = _DLQRecorder(subscriber_id="dlq-recorder-meta")
    emitter.subscribe(dlq_recorder)

    sub = _CountingSubscriber(
        fail_count=999,
        subscriber_id="always-failing-meta",
        retry=EventRetryConfig(max_attempts=1, initial_backoff_ms=1, max_backoff_ms=5),
    )
    emitter.subscribe(sub)

    original = ApCoreEvent(
        event_type="apcore.test.meta",
        module_id="module-xyz",
        timestamp="2026-06-05T12:00:00Z",
        severity="info",
        data={"k": "v"},
    )
    emitter.emit(original)
    emitter.flush(timeout=5.0)
    emitter.shutdown()

    assert len(dlq_recorder.received) == 1
    original_event = dlq_recorder.received[0].data["original_event"]
    assert set(original_event.keys()) == {"name", "payload", "metadata"}
    assert original_event["name"] == "apcore.test.meta"
    assert original_event["payload"] == {"k": "v"}
    assert original_event["metadata"] == {
        "module_id": "module-xyz",
        "timestamp": "2026-06-05T12:00:00Z",
    }


def test_on_failure_called_after_exhaustion() -> None:
    """on_failure callback is invoked after all retry attempts are exhausted."""
    emitter = EventEmitter()
    sub = _CountingSubscriber(
        fail_count=999,
        retry=EventRetryConfig(max_attempts=2, initial_backoff_ms=1, max_backoff_ms=5),
    )
    emitter.subscribe(sub)

    emitter.emit(_make_event())
    emitter.flush(timeout=5.0)
    emitter.shutdown()

    assert len(sub.failures) == 1
    _, error, attempt_count = sub.failures[0]
    assert attempt_count == 2


def test_dlq_event_not_retried(caplog: pytest.LogCaptureFixture) -> None:
    """DLQ subscriber that fails gets 1 attempt only, and no second-order DLQ."""
    import logging

    emitter = EventEmitter()
    dlq_recorder = _DLQRecorder()
    emitter.subscribe(dlq_recorder)

    # Broken subscriber that always fails
    primary = _CountingSubscriber(
        fail_count=999,
        retry=EventRetryConfig(max_attempts=2, initial_backoff_ms=1),
    )
    emitter.subscribe(primary)

    # Broken DLQ subscriber
    broken_dlq_calls: list[int] = []

    class _BrokenDLQSub:
        subscriber_id = "broken-dlq"
        event_pattern = _DLQ_EVENT_TYPE
        retry = EventRetryConfig(max_attempts=5, initial_backoff_ms=100)

        async def on_event(self, event: ApCoreEvent) -> None:
            broken_dlq_calls.append(1)
            raise RuntimeError("DLQ subscriber also broken")

    broken_dlq = _BrokenDLQSub()
    emitter.subscribe(broken_dlq)

    with caplog.at_level(logging.ERROR):
        emitter.emit(_make_event("apcore.test.broken"))
        emitter.flush(timeout=5.0)
    emitter.shutdown()

    # Primary: 2 attempts; DLQ subscriber: 1 attempt (no retry for DLQ delivery)
    assert len(primary.attempts) == 2
    assert len(broken_dlq_calls) == 1
    # No second-order DLQ event (dlq_recorder only sees one DLQ event from primary failure)
    assert len(dlq_recorder.received) == 1
    # Error was logged for the broken DLQ subscriber
    error_msgs = [r.message for r in caplog.records if r.levelno == logging.ERROR]
    assert any("broken-dlq" in m or "DLQ" in m or "delivery_failed" in m for m in error_msgs)


def test_subscriber_id_generated_when_omitted() -> None:
    """Two stdout subscribers without explicit id must get distinct, generated IDs."""
    sub1 = StdoutSubscriber()
    sub2 = StdoutSubscriber()
    assert sub1.subscriber_id != sub2.subscriber_id
    assert re.match(r"^stdout-\d+$", sub1.subscriber_id)
    assert re.match(r"^stdout-\d+$", sub2.subscriber_id)


def test_explicit_subscriber_id_preserved() -> None:
    """A subscriber constructed with id=... must use that string."""
    sub = StdoutSubscriber(id="my-explicit-id")
    assert sub.subscriber_id == "my-explicit-id"


def test_event_pattern_wildcard_receives_normal_events() -> None:
    """A catch-all subscriber (pattern='*') receives regular events."""
    emitter = EventEmitter()
    sub = _CountingSubscriber()  # default event_pattern="*"
    emitter.subscribe(sub)
    emitter.emit(_make_event("apcore.test.normal"))
    emitter.flush()
    emitter.shutdown()
    assert len(sub.attempts) == 1


def test_event_pattern_wildcard_skips_dlq_events() -> None:
    """A catch-all subscriber (pattern='*') must NOT receive DLQ events."""
    emitter = EventEmitter()
    catch_all = _CountingSubscriber()  # pattern="*"
    emitter.subscribe(catch_all)

    failing = _CountingSubscriber(
        fail_count=999,
        retry=EventRetryConfig(max_attempts=1, initial_backoff_ms=1),
    )
    emitter.subscribe(failing)

    emitter.emit(_make_event("apcore.test.event"))
    emitter.flush(timeout=5.0)
    emitter.shutdown()

    # catch_all gets the normal event but NOT the DLQ event about 'failing'
    dlq_received = [e for e in catch_all.attempts if e.event_type == _DLQ_EVENT_TYPE]
    assert len(dlq_received) == 0


def test_a2a_subscriber_id_and_skill_id() -> None:
    """A2ASubscriber supports explicit id and skill_id constructor params."""
    sub = A2ASubscriber(
        platform_url="https://example.com",
        skill_id="myapp.event_receiver",
        id="remote-monitor-1",
    )
    assert sub.subscriber_id == "remote-monitor-1"
    assert sub._skill_id == "myapp.event_receiver"


def test_file_subscriber_has_retry_and_event_pattern() -> None:
    sub = FileSubscriber(
        path="/tmp/test.jsonl",
        retry=EventRetryConfig(max_attempts=5, initial_backoff_ms=10),
        event_pattern="apcore.health.*",
        id="health-logger",
    )
    assert sub.subscriber_id == "health-logger"
    assert sub.retry.max_attempts == 5
    assert sub.event_pattern == "apcore.health.*"
