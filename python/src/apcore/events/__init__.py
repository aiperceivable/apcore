"""apcore events package.

Provides the event bus, subscriber factories, and delivery configuration::

    from apcore.events import ApCoreEvent, EventSubscriber, EventEmitter
    from apcore.events import SubscriberRegistry
"""

from apcore.events.circuit_breaker import CircuitBreakerWrapper, CircuitState
from apcore.events.emitter import ApCoreEvent, EventEmitter, EventSubscriber
from apcore.events.factory import SubscriberRegistry, create_subscriber_from_config
from apcore.events.retry import EventRetryConfig
from apcore.events.subscribers import (
    FileSubscriber,
    FilterSubscriber,
    StdoutSubscriber,
    WebhookSubscriber,
)


__all__ = [
    "ApCoreEvent",
    "EventEmitter",
    "EventSubscriber",
    "SubscriberRegistry",
    "EventRetryConfig",
    "WebhookSubscriber",
    "FileSubscriber",
    "StdoutSubscriber",
    "FilterSubscriber",
    "CircuitBreakerWrapper",
    "CircuitState",
    "create_subscriber_from_config",
]
