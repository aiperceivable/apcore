"""Event subscribers for webhook, A2A, file, stdout, and filter delivery (PRD F9, Issue #36)."""

from __future__ import annotations

import json
import logging
import os
import sys
from dataclasses import asdict

try:
    import aiohttp  # type: ignore[import-not-found]
except ImportError:
    aiohttp = None  # type: ignore[assignment]

from apcore.events.emitter import ApCoreEvent, EventSubscriber
from apcore.events.retry import EventRetryConfig
from apcore.utils.pattern import match_glob

__all__ = [
    "WebhookSubscriber",
    "A2ASubscriber",
    "FileSubscriber",
    "StdoutSubscriber",
    "FilterSubscriber",
]

_SEVERITY_ORDER: dict[str, int] = {"info": 0, "warn": 1, "error": 2, "fatal": 3}

logger = logging.getLogger(__name__)


class WebhookSubscriber:
    """Delivers events via HTTP POST to a webhook URL.

    Retry policy is controlled entirely by ``retry: EventRetryConfig``; the
    emitter handles backoff and DLQ emission on exhaustion. Raises on every
    non-2xx/4xx response and on network errors so the emitter retry loop can
    observe failures.
    """

    subscriber_type = "webhook"

    def __init__(
        self,
        url: str,
        headers: dict[str, str] | None = None,
        timeout_ms: int = 5000,
        *,
        id: str | None = None,  # noqa: A002
        retry: EventRetryConfig | None = None,
        event_pattern: str = "*",
    ) -> None:
        from apcore.events.emitter import _next_subscriber_id

        self._url = url
        self._headers = headers or {}
        self._timeout_ms = timeout_ms
        self.subscriber_id: str = id if id is not None else _next_subscriber_id("webhook")
        self.retry: EventRetryConfig = retry if retry is not None else EventRetryConfig()
        self.event_pattern: str = event_pattern

    async def on_event(self, event: ApCoreEvent) -> None:
        """Send the event as a JSON POST request to the configured URL.

        Raises on 5xx responses and network errors so the emitter's retry loop
        can act. 4xx responses are logged as warnings and treated as permanent
        failures (no retry).
        """
        if aiohttp is None:
            raise ImportError("aiohttp is required for WebhookSubscriber. Install with: pip install apcore[events]")

        payload = asdict(event)
        timeout = aiohttp.ClientTimeout(total=self._timeout_ms / 1000.0)
        merged_headers = {"Content-Type": "application/json", **self._headers}

        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.post(self._url, json=payload, headers=merged_headers) as response:
                if response.status >= 500:
                    raise RuntimeError(f"Webhook {self._url} returned {response.status} for event {event.event_type}")
                if response.status >= 400:
                    logger.warning(
                        "Webhook %s returned %d for event %s (permanent failure — no retry)",
                        self._url,
                        response.status,
                        event.event_type,
                    )


class FileSubscriber:
    """Writes events to a local file (built-in type: 'file')."""

    subscriber_type = "file"

    def __init__(
        self,
        path: str,
        append: bool = True,
        output_format: str = "json",
        rotate_bytes: int | None = None,
        *,
        id: str | None = None,  # noqa: A002
        retry: EventRetryConfig | None = None,
        event_pattern: str = "*",
    ) -> None:
        from apcore.events.emitter import _next_subscriber_id

        self._path = path
        self._append = append
        self._format = output_format
        self._rotate_bytes = rotate_bytes
        self.subscriber_id: str = id if id is not None else _next_subscriber_id("file")
        self.retry: EventRetryConfig = retry if retry is not None else EventRetryConfig()
        self.event_pattern: str = event_pattern

    async def on_event(self, event: ApCoreEvent) -> None:
        try:
            if (
                self._rotate_bytes is not None
                and os.path.exists(self._path)
                and os.path.getsize(self._path) >= self._rotate_bytes
            ):
                os.rename(self._path, f"{self._path}.1")

            mode = "a" if self._append else "w"
            with open(self._path, mode, encoding="utf-8") as fh:
                if self._format == "json":
                    fh.write(json.dumps(asdict(event)) + "\n")
                else:
                    fh.write(
                        f"[{event.timestamp}] [{event.severity.upper()}] "
                        f"{event.event_type} module={event.module_id} data={event.data}\n"
                    )
        except Exception:
            logger.exception(
                "FileSubscriber failed to write event %s to %s",
                event.event_type,
                self._path,
            )
            raise


class StdoutSubscriber:
    """Writes events to stdout (built-in type: 'stdout')."""

    subscriber_type = "stdout"

    def __init__(
        self,
        output_format: str = "text",
        level_filter: str | None = None,
        *,
        id: str | None = None,  # noqa: A002
        retry: EventRetryConfig | None = None,
        event_pattern: str = "*",
    ) -> None:
        from apcore.events.emitter import _next_subscriber_id

        self._format = output_format
        self._level_filter = level_filter
        self.subscriber_id: str = id if id is not None else _next_subscriber_id("stdout")
        self.retry: EventRetryConfig = retry if retry is not None else EventRetryConfig()
        self.event_pattern: str = event_pattern

    async def on_event(self, event: ApCoreEvent) -> None:
        if self._level_filter is not None:
            min_level = _SEVERITY_ORDER.get(self._level_filter, 0)
            event_level = _SEVERITY_ORDER.get(event.severity, 0)
            if event_level < min_level:
                return

        if self._format == "json":
            line = json.dumps(asdict(event))
        else:
            line = (
                f"[{event.timestamp}] [{event.severity.upper()}] "
                f"{event.event_type} module={event.module_id} data={event.data}"
            )
        print(line, file=sys.stdout)


class FilterSubscriber:
    """Wraps a delegate subscriber with event-name filtering (built-in type: 'filter')."""

    subscriber_type = "filter"

    def __init__(
        self,
        delegate: EventSubscriber,
        include_events: list[str] | None = None,
        exclude_events: list[str] | None = None,
        *,
        id: str | None = None,  # noqa: A002
        retry: EventRetryConfig | None = None,
        event_pattern: str = "*",
    ) -> None:
        from apcore.events.emitter import _next_subscriber_id

        self._delegate = delegate
        self._include_events = include_events
        self._exclude_events = exclude_events
        self.subscriber_id: str = id if id is not None else _next_subscriber_id("filter")
        self.retry: EventRetryConfig = retry if retry is not None else EventRetryConfig()
        self.event_pattern: str = event_pattern

    async def on_event(self, event: ApCoreEvent) -> None:
        if self._matches(event.event_type):
            await self._delegate.on_event(event)

    def _matches(self, event_type: str) -> bool:
        # PROTOCOL_SPEC 9.16.3 — Algorithm A25, case-sensitive. `include_events`
        # is decisive when present; `exclude_events` applies only in its absence.
        # exclude_events FAILS OPEN: a pattern that does not match means the
        # event is delivered, so a matcher understanding fewer metacharacters
        # than the operator wrote opens the filter rather than narrowing it.
        if self._include_events is not None:
            return any(match_glob(pattern, event_type) for pattern in self._include_events)
        if self._exclude_events is not None:
            return not any(match_glob(pattern, event_type) for pattern in self._exclude_events)
        return True


class A2ASubscriber:
    """Delivers events via the A2A protocol to the platform."""

    subscriber_type = "a2a"

    def __init__(
        self,
        platform_url: str,
        auth: str | dict[str, str] | None = None,
        timeout_ms: int = 5000,
        skill_id: str = "apevo.event_receiver",
        *,
        id: str | None = None,  # noqa: A002
        retry: EventRetryConfig | None = None,
        event_pattern: str = "*",
    ) -> None:
        from apcore.events.emitter import _next_subscriber_id

        self._platform_url = platform_url
        self._auth = auth
        self._timeout_ms = timeout_ms
        self._skill_id = skill_id
        self.subscriber_id: str = id if id is not None else _next_subscriber_id("a2a")
        self.retry: EventRetryConfig = retry if retry is not None else EventRetryConfig()
        self.event_pattern: str = event_pattern

    async def on_event(self, event: ApCoreEvent) -> None:
        """Send the event to the A2A platform endpoint.

        Raises on 5xx responses and network errors so the emitter's retry loop
        can act. 4xx responses are logged as warnings and treated as permanent
        failures (no retry).
        """
        payload = {
            "skillId": self._skill_id,
            "event": asdict(event),
        }
        headers: dict[str, str] = {"Content-Type": "application/json"}
        if isinstance(self._auth, str):
            headers["Authorization"] = f"Bearer {self._auth}"
        elif isinstance(self._auth, dict):
            headers.update(self._auth)

        if aiohttp is None:
            raise ImportError("aiohttp is required for A2ASubscriber. Install with: pip install apcore[events]")

        timeout = aiohttp.ClientTimeout(total=self._timeout_ms / 1000.0)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.post(self._platform_url, json=payload, headers=headers) as response:
                if response.status >= 500:
                    raise RuntimeError(f"A2A delivery to {self._platform_url} failed with status {response.status}")
                if response.status >= 400:
                    logger.warning(
                        "A2A delivery to %s returned %d for event %s (permanent failure — no retry)",
                        self._platform_url,
                        response.status,
                        event.event_type,
                    )
