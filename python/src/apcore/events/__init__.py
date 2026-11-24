"""apcore events package.

Re-exports the global event bus and related types::

    from apcore.events import ApCoreEvent, EventSubscriber, EventEmitter
    from apcore.events import register_subscriber_factory, create_subscriber_from_config
"""

from apcore.events.circuit_breaker import CircuitBreakerWrapper, CircuitState
from apcore.events.emitter import ApCoreEvent, EventEmitter, EventSubscriber
from apcore.events.retry import EventRetryConfig
from apcore.events.subscribers import (
    A2ASubscriber,
    FileSubscriber,
    FilterSubscriber,
    StdoutSubscriber,
    WebhookSubscriber,
)


def register_subscriber_factory(type_name: str, factory):  # type: ignore[no-untyped-def]
    """Register a custom subscriber-type factory.

    Re-exported from :mod:`apcore.sys_modules.registration` for cross-language
    parity with TypeScript (``registerSubscriberType``) and Rust
    (``register_subscriber_type``). Lazy-imported to avoid a circular import
    at package init time.
    """
    from apcore.sys_modules.registration import (
        register_subscriber_factory as _impl,
    )

    return _impl(type_name, factory)


def create_subscriber_from_config(config: dict) -> EventSubscriber:  # type: ignore[type-arg]
    """Build an :class:`EventSubscriber` from a config dict.

    Re-exported from :mod:`apcore.sys_modules.registration` for cross-language
    parity with TypeScript (``createSubscriberFromConfig``) and Rust
    (``create_subscriber``).
    """
    from apcore.sys_modules.registration import (
        create_subscriber_from_config as _impl,
    )

    return _impl(config)


__all__ = [
    "ApCoreEvent",
    "EventEmitter",
    "EventSubscriber",
    "EventRetryConfig",
    "A2ASubscriber",
    "WebhookSubscriber",
    "FileSubscriber",
    "StdoutSubscriber",
    "FilterSubscriber",
    "CircuitBreakerWrapper",
    "CircuitState",
    "register_subscriber_factory",
    "create_subscriber_from_config",
]
