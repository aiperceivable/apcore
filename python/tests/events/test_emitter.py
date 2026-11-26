"""Tests for EventEmitter, ApCoreEvent, and EventSubscriber."""

from __future__ import annotations

import asyncio
import dataclasses
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
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

    def test_unsubscribe_removes_only_the_exact_subscriber(self) -> None:
        @dataclasses.dataclass
        class EquivalentSubscriber:
            received: list[ApCoreEvent] = dataclasses.field(default_factory=list, compare=False)

            async def on_event(self, event: ApCoreEvent) -> None:
                self.received.append(event)

        first, second, absent = EquivalentSubscriber(), EquivalentSubscriber(), EquivalentSubscriber()
        assert first == second == absent
        emitter = EventEmitter()
        emitter.subscribe(first)
        emitter.subscribe(second)
        event = self._make_event()
        try:
            emitter.unsubscribe(second)
            emitter.unsubscribe(absent)
            emitter.emit(event)
            emitter.flush()
            assert first.received == [event]
            assert second.received == []
        finally:
            emitter.shutdown()

    def test_flush_uses_one_budget_for_queued_deliveries(self) -> None:
        emitter = EventEmitter(max_workers=1)
        entered, release, flushed = threading.Event(), threading.Event(), threading.Event()

        class BlockingSubscriber:
            async def on_event(self, event: ApCoreEvent) -> None:
                entered.set()
                release.wait(3)

        def flush() -> None:
            emitter.flush(timeout=0.08)
            flushed.set()

        emitter.subscribe(BlockingSubscriber())
        for _ in range(8):
            emitter.emit(self._make_event())
        thread = threading.Thread(target=flush, daemon=True)
        try:
            assert entered.wait(1)
            thread.start()
            assert flushed.wait(0.4)
        finally:
            release.set()
            thread.join(timeout=1)
            emitter.shutdown()

    @pytest.mark.parametrize("method", ["close", "shutdown"])
    def test_release_returns_at_deadline_and_discards_queued_work(self, method: str) -> None:
        emitter = EventEmitter()
        started, release, returned = threading.Event(), threading.Event(), threading.Event()
        received: list[str] = []

        class Subscriber:
            async def on_event(self, event: ApCoreEvent) -> None:
                received.append(event.event_type)
                if event.event_type == "test.running":
                    started.set()
                    release.wait(3)

        def close() -> None:
            getattr(emitter, method)(timeout=0.08)
            returned.set()

        emitter.subscribe(Subscriber())
        emitter.emit(self._make_event(event_type="test.running"))
        thread = threading.Thread(target=close, daemon=True)
        try:
            assert started.wait(1)
            emitter.emit(self._make_event(event_type="test.queued"))
            thread.start()
            assert returned.wait(0.4)
            assert received == ["test.running"]
            assert not emitter._worker.loop.is_closed()
        finally:
            release.set()
            thread.join(timeout=1)
            emitter.close()
        assert received == ["test.running"]
        assert emitter._worker.loop.is_closed()
        emitter.emit(self._make_event(event_type="test.next"))
        emitter.flush()
        assert received == (["test.running", "test.next"] if method == "close" else ["test.running"])
        emitter.shutdown()

    def test_stuck_delivery_does_not_keep_process_alive_after_shutdown(self) -> None:
        code = """
import threading
from apcore.events import ApCoreEvent, EventEmitter

entered = threading.Event()

class Subscriber:
    async def on_event(self, event):
        entered.set()
        threading.Event().wait()

emitter = EventEmitter()
emitter.subscribe(Subscriber())
emitter.emit(ApCoreEvent('test.running', None, 'now', 'info', {}))
assert entered.wait(1)
emitter.shutdown(timeout=0.02)
"""
        completed = subprocess.run([sys.executable, "-c", code], check=False, capture_output=True, timeout=3)
        assert completed.returncode == 0, completed.stderr.decode()

    @pytest.mark.parametrize("method", ["close", "shutdown"])
    def test_expired_queue_releases_payloads_without_holding_lifecycle_locks(self, method: str) -> None:
        code = """
import sys
import threading
from apcore.events import ApCoreEvent, EventEmitter

entered = threading.Event()
release_delivery = threading.Event()
disposing = threading.Event()
release_disposal = threading.Event()
emitter = EventEmitter()
received = []

def event(kind, data=None):
    return ApCoreEvent(kind, None, 'now', 'info', data or {})

class Subscriber:
    async def on_event(self, event):
        received.append(event.event_type)
        entered.set()
        release_delivery.wait()

class Payload:
    def __del__(self):
        emitter.emit(event('test.cleanup'))
        disposing.set()
        release_disposal.wait()

subscriber = Subscriber()
emitter.subscribe(subscriber)
emitter.emit(event('test.running'))
assert entered.wait(1)
emitter.emit(event('test.discarded', {'payload': Payload()}))
getattr(emitter, sys.argv[1])(timeout=0)

release_delivery.set()
assert disposing.wait(1)
emitter.unsubscribe(subscriber)
emitter.close(timeout=0)
release_disposal.set()
emitter.shutdown(timeout=1)
assert received == ['test.running']
"""
        completed = subprocess.run([sys.executable, "-c", code, method], check=False, capture_output=True, timeout=3)
        assert completed.returncode == 0, completed.stderr.decode()

    @pytest.mark.parametrize("method", ["close", "shutdown"])
    def test_subscriber_can_release_its_own_emitter(self, method: str) -> None:
        emitter = EventEmitter()
        entered, release, returned = threading.Event(), threading.Event(), threading.Event()
        received: list[str] = []

        class Subscriber:
            async def on_event(self, event: ApCoreEvent) -> None:
                received.append(event.event_type)
                if event.event_type == "test.close":
                    entered.set()
                    release.wait(3)
                    emitter.flush()
                    getattr(emitter, method)(timeout=0)
                    returned.set()

        emitter.subscribe(Subscriber())
        emitter.emit(self._make_event(event_type="test.close"))
        try:
            assert entered.wait(1)
            emitter.emit(self._make_event(event_type="test.queued"))
            release.set()
            assert returned.wait(1)
        finally:
            release.set()
            emitter.close(timeout=1)

        assert received == ["test.close"]
        assert emitter._worker.loop.is_closed()
        emitter.emit(self._make_event(event_type="test.next"))
        emitter.flush()
        assert received == (["test.close", "test.next"] if method == "close" else ["test.close"])
        emitter.shutdown()

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

    def test_shutdown_releases_worker_and_loop(self) -> None:
        """shutdown() stops its delivery worker and closes its event loop."""
        emitter = EventEmitter()
        subscriber = AsyncMock()
        emitter.subscribe(subscriber)
        emitter.emit(self._make_event())

        emitter.shutdown(timeout=1.0)

        assert emitter._worker.loop.is_closed()
        thread = emitter._worker._thread
        assert thread is not None and not thread.is_alive()

    def test_shutdown_is_idempotent(self) -> None:
        emitter = EventEmitter()
        emitter.shutdown(timeout=0.5)
        emitter.shutdown(timeout=0.5)  # must not raise

    def test_shutdown_waits_for_admitted_delivery_to_be_enqueued(self, monkeypatch: pytest.MonkeyPatch) -> None:
        emitter = EventEmitter()
        subscriber = AsyncMock()
        emitter.subscribe(subscriber)
        submit_entered, release_submit = threading.Event(), threading.Event()
        close_called, worker_stopped = threading.Event(), threading.Event()
        submit = emitter._worker.submit
        stop_worker = emitter._worker.stop

        def delayed_submit(*args: Any, **kwargs: Any) -> Any:
            submit_entered.set()
            assert release_submit.wait(1)
            return submit(*args, **kwargs)

        def observe_worker_stop(*args: Any, **kwargs: Any) -> None:
            worker_stopped.set()
            stop_worker(*args, **kwargs)

        def close() -> None:
            close_called.set()
            emitter.shutdown()

        monkeypatch.setattr(emitter._worker, "submit", delayed_submit)
        monkeypatch.setattr(emitter._worker, "stop", observe_worker_stop)
        event = self._make_event()
        with ThreadPoolExecutor(max_workers=2) as racers:
            pending_emit = racers.submit(emitter.emit, event)
            try:
                assert submit_entered.wait(1)
                pending_close = racers.submit(close)
                assert close_called.wait(1)
                assert not worker_stopped.wait(0.03)
            finally:
                release_submit.set()
            pending_emit.result(timeout=1)
            pending_close.result(timeout=1)
        subscriber.on_event.assert_awaited_once_with(event)

    def test_concurrent_shutdown_is_terminal_during_close(self, monkeypatch: pytest.MonkeyPatch) -> None:
        emitter = EventEmitter()
        entered, release, close_started = threading.Event(), threading.Event(), threading.Event()

        class BlockingSubscriber:
            async def on_event(self, event: ApCoreEvent) -> None:
                entered.set()
                release.wait(3)

        original_stop = emitter._worker.stop

        def stop(deadline: float) -> None:
            original_stop(deadline)
            close_started.set()

        monkeypatch.setattr(emitter._worker, "stop", stop)
        emitter.subscribe(BlockingSubscriber())
        emitter.emit(self._make_event())
        with ThreadPoolExecutor(max_workers=1) as racers:
            try:
                assert entered.wait(1)
                closing = racers.submit(emitter.close)
                assert close_started.wait(1)
                emitter.shutdown(timeout=0)
            finally:
                release.set()
            closing.result(timeout=1)
        closed_worker = emitter._worker
        emitter.emit(self._make_event())
        emitter.flush()
        assert emitter._worker is closed_worker
        assert closed_worker.loop.is_closed()

    def test_emit_after_shutdown_is_noop(self) -> None:
        emitter = EventEmitter()
        subscriber = AsyncMock()
        emitter.subscribe(subscriber)
        emitter.shutdown(timeout=0.5)

        # Does not raise, does not deliver.
        emitter.emit(self._make_event())
        subscriber.on_event.assert_not_called()
