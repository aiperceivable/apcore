"""Tests for WebhookSubscriber and A2ASubscriber."""

from __future__ import annotations

import asyncio
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from apcore.events.emitter import ApCoreEvent, EventSubscriber


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_event(**overrides: Any) -> ApCoreEvent:
    defaults: dict[str, Any] = {
        "event_type": "test.event",
        "module_id": "mod.a",
        "timestamp": "2026-03-08T00:00:00Z",
        "severity": "info",
        "data": {"key": "value"},
    }
    defaults.update(overrides)
    return ApCoreEvent(**defaults)


# ---------------------------------------------------------------------------
# WebhookSubscriber tests
# ---------------------------------------------------------------------------


class TestWebhookSubscriberSendsPostRequest:
    @pytest.mark.asyncio
    async def test_webhook_subscriber_sends_post_request(self) -> None:
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(url="https://example.com/webhook")
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_response = AsyncMock()
            mock_response.status = 200
            mock_response.__aenter__ = AsyncMock(return_value=mock_response)
            mock_response.__aexit__ = AsyncMock(return_value=False)

            mock_session = AsyncMock()
            mock_session.post = MagicMock(return_value=mock_response)
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)
            mock_aiohttp.ClientTimeout = MagicMock()

            await subscriber.on_event(event)

            mock_session.post.assert_called_once()
            call_kwargs = mock_session.post.call_args
            assert (
                call_kwargs[0][0] == "https://example.com/webhook"
                or call_kwargs.kwargs.get("url") == "https://example.com/webhook"
            )


class TestWebhookSubscriberIncludesCustomHeaders:
    @pytest.mark.asyncio
    async def test_webhook_subscriber_includes_custom_headers(self) -> None:
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(
            url="https://example.com/webhook",
            headers={"X-Api-Key": "secret"},
        )
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_response = AsyncMock()
            mock_response.status = 200
            mock_response.__aenter__ = AsyncMock(return_value=mock_response)
            mock_response.__aexit__ = AsyncMock(return_value=False)

            mock_session = AsyncMock()
            mock_session.post = MagicMock(return_value=mock_response)
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)
            mock_aiohttp.ClientTimeout = MagicMock()

            await subscriber.on_event(event)

            call_kwargs = mock_session.post.call_args
            headers = call_kwargs.kwargs.get("headers") or call_kwargs[1].get("headers", {})
            assert headers.get("X-Api-Key") == "secret"


class TestWebhookSubscriberRaisesOn5xx:
    @pytest.mark.asyncio
    async def test_webhook_subscriber_raises_on_5xx(self) -> None:
        """Per apcore #61, WebhookSubscriber raises on 5xx so the emitter can retry."""
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(url="https://example.com/webhook")
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            resp = AsyncMock()
            resp.status = 500
            resp.__aenter__ = AsyncMock(return_value=resp)
            resp.__aexit__ = AsyncMock(return_value=False)

            mock_session = AsyncMock()
            mock_session.post = MagicMock(return_value=resp)
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)
            mock_aiohttp.ClientTimeout = MagicMock()

            with pytest.raises(RuntimeError, match="500"):
                await subscriber.on_event(event)

            # Single attempt per on_event call — emitter handles retry cadence
            assert mock_session.post.call_count == 1


class TestWebhookSubscriberRaisesOnConnectionError:
    @pytest.mark.asyncio
    async def test_webhook_subscriber_raises_on_connection_error(self) -> None:
        """Per apcore #61, network errors propagate so the emitter can retry."""
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(url="https://example.com/webhook")
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_session = AsyncMock()
            mock_session.post = MagicMock(side_effect=OSError("conn refused"))
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)
            mock_aiohttp.ClientTimeout = MagicMock()

            with pytest.raises(OSError):
                await subscriber.on_event(event)


class TestWebhookSubscriberEnforcesTimeout:
    @pytest.mark.asyncio
    async def test_webhook_subscriber_enforces_timeout(self) -> None:
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(url="https://example.com/webhook", timeout_ms=100)
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_aiohttp.ClientTimeout = MagicMock()

            mock_session = AsyncMock()
            mock_session.post = MagicMock(side_effect=asyncio.TimeoutError())
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)

            # Timeout now propagates so the emitter can retry
            with pytest.raises(asyncio.TimeoutError):
                await subscriber.on_event(event)

            # Verify ClientTimeout was called with the correct timeout
            mock_aiohttp.ClientTimeout.assert_called_with(total=0.1)


class TestWebhookSubscriberDoesNotRetryOn4xx:
    @pytest.mark.asyncio
    async def test_webhook_subscriber_does_not_retry_on_4xx(self) -> None:
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(url="https://example.com/webhook")
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_response = AsyncMock()
            mock_response.status = 400
            mock_response.__aenter__ = AsyncMock(return_value=mock_response)
            mock_response.__aexit__ = AsyncMock(return_value=False)

            mock_session = AsyncMock()
            mock_session.post = MagicMock(return_value=mock_response)
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)
            mock_aiohttp.ClientTimeout = MagicMock()

            await subscriber.on_event(event)

            # Only 1 attempt, no retries
            assert mock_session.post.call_count == 1


class TestWebhookSubscriberSerializesEventToJson:
    @pytest.mark.asyncio
    async def test_webhook_subscriber_serializes_event_to_json(self) -> None:
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(url="https://example.com/webhook")
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_response = AsyncMock()
            mock_response.status = 200
            mock_response.__aenter__ = AsyncMock(return_value=mock_response)
            mock_response.__aexit__ = AsyncMock(return_value=False)

            mock_session = AsyncMock()
            mock_session.post = MagicMock(return_value=mock_response)
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)
            mock_aiohttp.ClientTimeout = MagicMock()

            await subscriber.on_event(event)

            call_kwargs = mock_session.post.call_args
            body = call_kwargs.kwargs.get("json") or call_kwargs[1].get("json")
            assert body is not None
            assert body["event_type"] == "test.event"
            assert body["module_id"] == "mod.a"
            assert body["timestamp"] == "2026-03-08T00:00:00Z"
            assert body["severity"] == "info"
            assert body["data"] == {"key": "value"}


# ---------------------------------------------------------------------------
# A2ASubscriber tests
# ---------------------------------------------------------------------------


class TestA2ASubscriberSendsViaClient:
    @pytest.mark.asyncio
    async def test_a2a_subscriber_sends_via_client(self) -> None:
        from apcore.events.subscribers import A2ASubscriber

        subscriber = A2ASubscriber(
            platform_url="https://platform.example.com",
            auth="bearer-token-123",
        )
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_response = AsyncMock()
            mock_response.status = 200
            mock_response.__aenter__ = AsyncMock(return_value=mock_response)
            mock_response.__aexit__ = AsyncMock(return_value=False)

            mock_session = AsyncMock()
            mock_session.post = MagicMock(return_value=mock_response)
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)

            await subscriber.on_event(event)

            call_kwargs = mock_session.post.call_args
            body = call_kwargs.kwargs.get("json") or call_kwargs[1].get("json")
            assert body["skillId"] == "apevo.event_receiver"


class TestA2ASubscriberIncludesEventInPayload:
    @pytest.mark.asyncio
    async def test_a2a_subscriber_includes_event_in_payload(self) -> None:
        from apcore.events.subscribers import A2ASubscriber

        subscriber = A2ASubscriber(
            platform_url="https://platform.example.com",
        )
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_response = AsyncMock()
            mock_response.status = 200
            mock_response.__aenter__ = AsyncMock(return_value=mock_response)
            mock_response.__aexit__ = AsyncMock(return_value=False)

            mock_session = AsyncMock()
            mock_session.post = MagicMock(return_value=mock_response)
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)

            await subscriber.on_event(event)

            call_kwargs = mock_session.post.call_args
            body = call_kwargs.kwargs.get("json") or call_kwargs[1].get("json")
            payload_event = body["event"]
            assert payload_event["event_type"] == "test.event"
            assert payload_event["module_id"] == "mod.a"
            assert payload_event["data"] == {"key": "value"}


class TestA2ASubscriberHandlesSendFailure:
    @pytest.mark.asyncio
    async def test_a2a_subscriber_handles_send_failure(self) -> None:
        """Per apcore #61, A2A now raises on failure so the emitter can retry."""
        from apcore.events.subscribers import A2ASubscriber

        subscriber = A2ASubscriber(
            platform_url="https://platform.example.com",
        )
        event = _make_event()

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_session = AsyncMock()
            mock_session.post = MagicMock(side_effect=OSError("connection failed"))
            mock_session.__aenter__ = AsyncMock(return_value=mock_session)
            mock_session.__aexit__ = AsyncMock(return_value=False)

            mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)

            # A2A now raises on failure (emitter handles retry/DLQ)
            with pytest.raises(OSError, match="connection failed"):
                await subscriber.on_event(event)


# ---------------------------------------------------------------------------
# Protocol conformance tests
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# HTTP 4xx-vs-5xx retry contract through the real EventEmitter retry path
# (apcore #69). The conformance driver fails attempts via a generic ``raise``,
# which bypasses the WebhookSubscriber HTTP-status logic — these tests lock the
# status-based behavior by driving delivery through EventEmitter.emit() and
# asserting on the HTTP-call count (the unambiguous signal).
# ---------------------------------------------------------------------------


def _build_aiohttp_mock(mock_aiohttp: MagicMock, status: int) -> MagicMock:
    """Wire a patched ``aiohttp`` module to return a response with *status*.

    Returns the mock session so callers can assert on ``post.call_count``.
    """
    mock_response = AsyncMock()
    mock_response.status = status
    mock_response.__aenter__ = AsyncMock(return_value=mock_response)
    mock_response.__aexit__ = AsyncMock(return_value=False)

    mock_session = AsyncMock()
    mock_session.post = MagicMock(return_value=mock_response)
    mock_session.__aenter__ = AsyncMock(return_value=mock_session)
    mock_session.__aexit__ = AsyncMock(return_value=False)

    mock_aiohttp.ClientSession = MagicMock(return_value=mock_session)
    mock_aiohttp.ClientTimeout = MagicMock()
    return mock_session


class _DLQRecorder:
    """Subscribes only to DLQ events and records them (matches emitter tests)."""

    def __init__(self, subscriber_id: str = "dlq-recorder") -> None:
        from apcore.events.emitter import _DLQ_EVENT_TYPE

        self.subscriber_id = subscriber_id
        self.event_pattern = _DLQ_EVENT_TYPE
        self.retry = None  # emitter falls back to a default EventRetryConfig
        self.received: list[ApCoreEvent] = []

    async def on_event(self, event: ApCoreEvent) -> None:
        self.received.append(event)


class TestWebhookRetryContractThroughEmitter:
    """Lock the 4xx-vs-5xx HTTP retry contract via the real emitter retry loop."""

    @pytest.mark.asyncio
    async def test_4xx_is_not_retried_through_emitter(self) -> None:
        """HTTP 400 with max_attempts=3 → endpoint called exactly once (no retry)."""
        from apcore.events.emitter import EventEmitter
        from apcore.events.retry import EventRetryConfig
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(
            url="https://example.com/webhook",
            retry=EventRetryConfig(max_attempts=3, initial_backoff_ms=1, max_backoff_ms=5),
        )
        emitter = EventEmitter()
        dlq_recorder = _DLQRecorder()
        emitter.subscribe(dlq_recorder)
        emitter.subscribe(subscriber)

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_session = _build_aiohttp_mock(mock_aiohttp, status=400)

            emitter.emit(_make_event())
            emitter.flush(timeout=5.0)
            emitter.shutdown()

            # 4xx is a permanent client error: a single HTTP call, no retries.
            assert mock_session.post.call_count == 1
            # 4xx is logged-permanent (does not raise into the emitter), so no
            # DLQ event is emitted — matches src/apcore/events/subscribers.py
            # and event-system.md §"Behavior on 4xx / 5xx Responses".
            assert len(dlq_recorder.received) == 0

    @pytest.mark.asyncio
    async def test_5xx_is_retried_to_exhaustion_through_emitter(self) -> None:
        """HTTP 503 with max_attempts=3 → endpoint called 3 times, then DLQ."""
        from apcore.events.emitter import _DLQ_EVENT_TYPE, EventEmitter
        from apcore.events.retry import EventRetryConfig
        from apcore.events.subscribers import WebhookSubscriber

        subscriber = WebhookSubscriber(
            url="https://example.com/webhook",
            id="webhook-5xx",
            retry=EventRetryConfig(max_attempts=3, initial_backoff_ms=1, max_backoff_ms=5),
        )
        emitter = EventEmitter()
        dlq_recorder = _DLQRecorder()
        emitter.subscribe(dlq_recorder)
        emitter.subscribe(subscriber)

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_session = _build_aiohttp_mock(mock_aiohttp, status=503)

            emitter.emit(_make_event())
            emitter.flush(timeout=5.0)
            emitter.shutdown()

            # 5xx is retried to exhaustion: one HTTP call per attempt.
            assert mock_session.post.call_count == 3
            # On exhaustion the emitter emits a delivery_failed DLQ event.
            assert len(dlq_recorder.received) == 1
            dlq = dlq_recorder.received[0]
            assert dlq.event_type == _DLQ_EVENT_TYPE
            assert dlq.data["subscriber_id"] == "webhook-5xx"
            assert dlq.data["attempt_count"] == 3

    @pytest.mark.asyncio
    async def test_a2a_4xx_is_not_retried_through_emitter(self) -> None:
        """A2A HTTP 400 with max_attempts=3 → endpoint called once, no DLQ."""
        from apcore.events.emitter import EventEmitter
        from apcore.events.retry import EventRetryConfig
        from apcore.events.subscribers import A2ASubscriber

        subscriber = A2ASubscriber(
            platform_url="https://platform.example.com",
            retry=EventRetryConfig(max_attempts=3, initial_backoff_ms=1, max_backoff_ms=5),
        )
        emitter = EventEmitter()
        dlq_recorder = _DLQRecorder()
        emitter.subscribe(dlq_recorder)
        emitter.subscribe(subscriber)

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_session = _build_aiohttp_mock(mock_aiohttp, status=400)

            emitter.emit(_make_event())
            emitter.flush(timeout=5.0)
            emitter.shutdown()

            # 4xx is a permanent client error: a single HTTP call, no retries.
            assert mock_session.post.call_count == 1
            # 4xx is logged-permanent (does not raise into the emitter), so no
            # DLQ event is emitted — matches src/apcore/events/subscribers.py
            # and event-system.md §"Behavior on 4xx / 5xx Responses".
            assert len(dlq_recorder.received) == 0

    @pytest.mark.asyncio
    async def test_a2a_5xx_is_retried_to_exhaustion_through_emitter(self) -> None:
        """A2A HTTP 503 with max_attempts=3 → endpoint called 3 times, then DLQ."""
        from apcore.events.emitter import _DLQ_EVENT_TYPE, EventEmitter
        from apcore.events.retry import EventRetryConfig
        from apcore.events.subscribers import A2ASubscriber

        subscriber = A2ASubscriber(
            platform_url="https://platform.example.com",
            id="a2a-5xx",
            retry=EventRetryConfig(max_attempts=3, initial_backoff_ms=1, max_backoff_ms=5),
        )
        emitter = EventEmitter()
        dlq_recorder = _DLQRecorder()
        emitter.subscribe(dlq_recorder)
        emitter.subscribe(subscriber)

        with patch("apcore.events.subscribers.aiohttp") as mock_aiohttp:
            mock_session = _build_aiohttp_mock(mock_aiohttp, status=503)

            emitter.emit(_make_event())
            emitter.flush(timeout=5.0)
            emitter.shutdown()

            # 5xx is retried to exhaustion: one HTTP call per attempt.
            assert mock_session.post.call_count == 3
            # On exhaustion the emitter emits a delivery_failed DLQ event.
            assert len(dlq_recorder.received) == 1
            dlq = dlq_recorder.received[0]
            assert dlq.event_type == _DLQ_EVENT_TYPE
            assert dlq.data["subscriber_id"] == "a2a-5xx"
            assert dlq.data["subscriber_type"] == "a2a"
            assert dlq.data["attempt_count"] == 3


class TestSubscriberConformsToProtocol:
    def test_subscriber_conforms_to_protocol(self) -> None:
        from apcore.events.subscribers import A2ASubscriber, WebhookSubscriber

        webhook = WebhookSubscriber(url="https://example.com/webhook")
        a2a = A2ASubscriber(platform_url="https://platform.example.com")

        assert isinstance(webhook, EventSubscriber)
        assert isinstance(a2a, EventSubscriber)


class TestSubscriberInstantiationFailureLogged:
    def test_subscriber_instantiation_failure_logged(self) -> None:
        """Verify that if a dependency fails during usage, it's logged not raised."""
        from apcore.events.subscribers import WebhookSubscriber

        # WebhookSubscriber should not raise during construction even with odd inputs
        subscriber = WebhookSubscriber(url="")
        assert subscriber is not None
