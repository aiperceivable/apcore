"""Tests for EventEmitter, ApCoreEvent, and EventSubscriber."""

from __future__ import annotations

import asyncio
import dataclasses
import time
import threading
from typing import Any
from unittest.mock import AsyncMock

import pytest

from apcore.events.emitter import ApCoreEvent, EventEmitter, EventSubscriber


# ---------------------------------------------------------------------------
# ApCoreEvent tests
# ---------------------------------------------------------------------------


class TestApCoreEvent:
    def test_apcore_event_is_frozen(self) -> None:
        event = ApCoreEvent(
            event_type="test",
            module_id="mod.a",
            timestamp="2026-01-01T00:00:00Z",
            severity="info",
            data={},
        )
        with pytest.raises(dataclasses.FrozenInstanceError):
            event.event_type = "other"  # type: ignore[misc]

    def test_apcore_event_fields(self) -> None:
        event = ApCoreEvent(
            event_type="error",
            module_id=None,
            timestamp="2026-03-08T12:00:00Z",
            severity="critical",
            data={"key": "value"},
        )
        assert event.event_type == "error"
        assert event.module_id is None
        assert event.timestamp == "2026-03-08T12:00:00Z"
        assert event.severity == "critical"
        assert event.data == {"key": "value"}


# ---------------------------------------------------------------------------
# EventSubscriber protocol tests
# ---------------------------------------------------------------------------


class TestEventSubscriberProtocol:
    def test_event_subscriber_protocol(self) -> None:
        """EventSubscriber is a runtime-checkable Protocol with async on_event."""

        class MySubscriber:
            async def on_event(self, event: ApCoreEvent) -> None:
                pass

        assert isinstance(MySubscriber(), EventSubscriber)

    def test_non_subscriber_not_instance(self) -> None:
        class NotASubscriber:
            pass

        assert not isinstance(NotASubscriber(), EventSubscriber)


# ---------------------------------------------------------------------------
# EventEmitter tests
# ---------------------------------------------------------------------------


class TestEventEmitter:
    def _make_event(self, **overrides: Any) -> ApCoreEvent:
        defaults: dict[str, Any] = {
            "event_type": "test.event",
            "module_id": "mod.a",
            "timestamp": "2026-03-08T00:00:00Z",
            "severity": "info",
            "data": {},
        }
        defaults.update(overrides)
        return ApCoreEvent(**defaults)

    def test_subscribe_and_emit(self) -> None:
        emitter = EventEmitter()
        subscriber = AsyncMock()
        emitter.subscribe(subscriber)

        event = self._make_event()
        emitter.emit(event)

        # Wait for async delivery
        emitter.flush()
        subscriber.on_event.assert_called_once_with(event)

    def test_emit_fans_out_to_all_subscribers(self) -> None:
        emitter = EventEmitter()
        subs = [AsyncMock() for _ in range(3)]
        for s in subs:
            emitter.subscribe(s)

        event = self._make_event()
        emitter.emit(event)

        emitter.flush()
        for s in subs:
            s.on_event.assert_called_once_with(event)

    def test_unsubscribe_stops_delivery(self) -> None:
        emitter = EventEmitter()
        subscriber = AsyncMock()
        emitter.subscribe(subscriber)
        emitter.unsubscribe(subscriber)

        emitter.emit(self._make_event())

        emitter.flush()
        subscriber.on_event.assert_not_called()

    def test_subscriber_error_does_not_block_others(self) -> None:
        emitter = EventEmitter()
        sub1 = AsyncMock()
        sub2 = AsyncMock(side_effect=RuntimeError("boom"))
        sub3 = AsyncMock()
        emitter.subscribe(sub1)
        emitter.subscribe(sub2)
        emitter.subscribe(sub3)

        event = self._make_event()
        emitter.emit(event)

        emitter.flush()
        sub1.on_event.assert_called_once_with(event)
        sub3.on_event.assert_called_once_with(event)

    def test_emit_is_non_blocking(self) -> None:
        emitter = EventEmitter()

        slow_called = threading.Event()

        class SlowSubscriber:
            async def on_event(self, event: ApCoreEvent) -> None:
                await asyncio.sleep(1.0)
                slow_called.set()

        emitter.subscribe(SlowSubscriber())

        start = time.monotonic()
        emitter.emit(self._make_event())
        elapsed = time.monotonic() - start

        assert elapsed < 0.1, f"emit() took {elapsed:.3f}s, expected < 0.1s"
        # Wait for background to finish so no dangling threads
        slow_called.wait(timeout=3.0)

    def test_emit_with_no_subscribers(self) -> None:
        emitter = EventEmitter()
        # Should not raise
        emitter.emit(self._make_event())

    def test_emit_delivers_correct_event_data(self) -> None:
        emitter = EventEmitter()
        subscriber = AsyncMock()
        emitter.subscribe(subscriber)

        event = self._make_event(
            event_type="health.degraded",
            module_id="mod.x",
            data={"error_rate": 0.15, "threshold": 0.1},
        )
        emitter.emit(event)

        emitter.flush()
        delivered = subscriber.on_event.call_args[0][0]
        assert delivered.event_type == "health.degraded"
        assert delivered.module_id == "mod.x"
        assert delivered.data == {"error_rate": 0.15, "threshold": 0.1}

    def test_subscribe_rejects_sync_on_event(self) -> None:
        """subscribe() raises TypeError if on_event is not a coroutine function."""
        emitter = EventEmitter()

        class SyncSubscriber:
            def on_event(self, event: ApCoreEvent) -> None:
                pass

        with pytest.raises(TypeError, match="async on_event"):
            emitter.subscribe(SyncSubscriber())

    def test_subscribe_rejects_object_without_on_event(self) -> None:
        emitter = EventEmitter()
        with pytest.raises(TypeError, match="async on_event"):
            emitter.subscribe(object())  # type: ignore[arg-type]

    def test_shutdown_releases_executor_and_loop(self) -> None:
        """shutdown() closes the thread pool and asyncio loop."""
        emitter = EventEmitter()
        subscriber = AsyncMock()
        emitter.subscribe(subscriber)
        emitter.emit(self._make_event())

        emitter.shutdown(timeout=1.0)

        # Loop closed.
        assert emitter._loop.is_closed()
        # Executor no longer accepts new work.
        with pytest.raises(RuntimeError):
            emitter._executor.submit(lambda: None)

    def test_shutdown_is_idempotent(self) -> None:
        emitter = EventEmitter()
        emitter.shutdown(timeout=0.5)
        emitter.shutdown(timeout=0.5)  # must not raise

    def test_emit_after_shutdown_is_noop(self) -> None:
        emitter = EventEmitter()
        subscriber = AsyncMock()
        emitter.subscribe(subscriber)
        emitter.shutdown(timeout=0.5)

        # Does not raise, does not deliver.
        emitter.emit(self._make_event())
        subscriber.on_event.assert_not_called()
