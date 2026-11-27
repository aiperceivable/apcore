"""Protocol-neutral subscriber assembly."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from apcore.events.emitter import EventSubscriber
from apcore.events.retry import EventRetryConfig
from apcore.events.subscribers import (
    FileSubscriber,
    FilterSubscriber,
    StdoutSubscriber,
    WebhookSubscriber,
)

_Factory = Callable[[dict[str, Any]], EventSubscriber]


def _parse_retry_config(cfg: dict[str, Any]) -> EventRetryConfig | None:
    """Build an :class:`EventRetryConfig` from a subscriber's nested ``retry:`` block.

    Implements the per-subscriber retry policy documented in
    ``docs/features/event-system.md`` §"Per-Subscriber Retry Policy" (apcore#85).
    The block is optional and may be partial — omitted keys fall back to the
    spec defaults (``max_attempts=3``, ``initial_backoff_ms=100``,
    ``max_backoff_ms=30000``, ``backoff_multiplier=2.0``).

    Args:
        cfg: Raw subscriber config dict from ``sys_modules.events.subscribers``.

    Returns:
        The parsed policy, or ``None`` when no ``retry:`` block is present, so
        the subscriber falls back to its own default.
    """
    nested = cfg.get("retry")
    if not isinstance(nested, dict):
        return None
    default = EventRetryConfig()
    return EventRetryConfig(
        max_attempts=int(nested.get("max_attempts", default.max_attempts)),
        initial_backoff_ms=int(nested.get("initial_backoff_ms", default.initial_backoff_ms)),
        max_backoff_ms=int(nested.get("max_backoff_ms", default.max_backoff_ms)),
        backoff_multiplier=float(nested.get("backoff_multiplier", default.backoff_multiplier)),
    )


def _default_webhook_factory(cfg: dict[str, Any]) -> EventSubscriber:
    retry_cfg = _parse_retry_config(cfg)
    # Deprecated: flat retry_count is the legacy webhook-only shorthand. It maps
    # to EventRetryConfig.max_attempts + 1 (retry_count counted retries *after*
    # the first attempt; max_attempts counts total attempts). The nested retry:
    # block wins when both are present (apcore#85).
    if retry_cfg is None and "retry_count" in cfg:
        retry_cfg = EventRetryConfig(max_attempts=int(cfg["retry_count"]) + 1)
    return WebhookSubscriber(
        url=cfg["url"],
        headers=cfg.get("headers"),
        timeout_ms=cfg.get("timeout_ms", 5000),
        retry=retry_cfg,
    )


def _default_file_factory(cfg: dict[str, Any]) -> EventSubscriber:
    return FileSubscriber(
        path=cfg["path"],
        append=cfg.get("append", True),
        output_format=cfg.get("format", "json"),
        rotate_bytes=cfg.get("rotate_bytes"),
        retry=_parse_retry_config(cfg),
    )


def _default_stdout_factory(cfg: dict[str, Any]) -> EventSubscriber:
    return StdoutSubscriber(
        output_format=cfg.get("format", "text"),
        level_filter=cfg.get("level_filter"),
        retry=_parse_retry_config(cfg),
    )


class SubscriberRegistry:
    """Factories selected by one host when assembling its event subscribers."""

    def __init__(self) -> None:
        self._factories: dict[str, _Factory] = {
            "webhook": _default_webhook_factory,
            "file": _default_file_factory,
            "stdout": _default_stdout_factory,
            "filter": self._make_filter,
        }

    def register(self, type_name: str, factory: _Factory) -> None:
        """Add or replace a factory for the named configuration type."""
        self._factories[type_name] = factory

    def create(self, config: dict[str, Any]) -> EventSubscriber:
        """Create a subscriber, resolving any nested delegates in this registry."""
        type_name = config.get("type", "")
        factory = self._factories.get(type_name)
        if factory is None:
            registered = ", ".join(sorted(self._factories))
            raise ValueError(f"Unknown subscriber type: {type_name!r}. Registered types: [{registered}].")
        return factory(config)

    def _make_filter(self, config: dict[str, Any]) -> EventSubscriber:
        delegate_config = {**config.get("delegate_config", {}), "type": config["delegate_type"]}
        return FilterSubscriber(
            delegate=self.create(delegate_config),
            include_events=config.get("include_events"),
            exclude_events=config.get("exclude_events"),
            retry=_parse_retry_config(config),
        )


def create_subscriber_from_config(config: dict[str, Any]) -> EventSubscriber:
    """Build a subscriber using the protocol-neutral built-in factories."""
    return SubscriberRegistry().create(config)
