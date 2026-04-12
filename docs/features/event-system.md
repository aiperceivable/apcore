# Event System

## Overview

The event system provides a global event bus for framework-level lifecycle events. It enables real-time monitoring, alerting, and integration with external platforms through a subscriber-based architecture. Events are dispatched asynchronously via a thread pool, ensuring that event handling never blocks module execution.

## Requirements

### Core Event Bus
- Provide an `EventEmitter` class with thread-safe subscriber management and non-blocking event dispatch.
- Define an `ApCoreEvent` frozen dataclass as the standard event envelope.
- Define an `EventSubscriber` runtime-checkable protocol with a single `async on_event()` method.
- Errors in one subscriber **MUST NOT** propagate to other subscribers or block the emitter.

### Built-in Subscribers
- `WebhookSubscriber` — HTTP POST delivery with configurable retry (5xx and connection errors only).
- `A2ASubscriber` — Agent-to-Agent protocol bridge with bearer/dict auth support.
- Both require an HTTP client dependency provided by the SDK (e.g., `aiohttp` in the Python SDK, available via the `events` optional install group).

### Extensibility
- Subscriber type registry with factory pattern for config-driven instantiation.
- Custom subscribers can be created by implementing the `EventSubscriber` protocol.

## Technical Design

### ApCoreEvent

```python
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class ApCoreEvent:
    event_type: str               # Event identifier
    module_id: str | None         # Associated module (None for global events)
    timestamp: str                # ISO 8601 UTC timestamp
    severity: str                 # "info" | "warn" | "error" | "fatal"
    data: dict[str, Any]          # Event-specific payload
```

Immutable by design (`frozen=True`) — prevents accidental mutation after emission.

### EventSubscriber Protocol

```python
from typing import Protocol, runtime_checkable


@runtime_checkable
class EventSubscriber(Protocol):
    async def on_event(self, event: ApCoreEvent) -> None: ...
```

### EventEmitter

```python
class EventEmitter:
    def __init__(self, max_workers: int = 4) -> None: ...
    def subscribe(self, subscriber: EventSubscriber) -> None: ...
    def unsubscribe(self, subscriber: EventSubscriber) -> None: ...
    def emit(self, event: ApCoreEvent) -> None: ...
    def flush(self, timeout: float = 5.0) -> None: ...
```

**Dispatch model:**
- `emit()` returns immediately. Delivery is handled asynchronously by a bounded background worker pool (e.g., a thread pool in Python with a persistent async event loop).
- Each `emit()` takes a snapshot of current subscribers, so subscribe/unsubscribe during delivery is safe.
- Failed deliveries are logged but never re-raised.
- `flush()` blocks until all pending deliveries complete (useful in tests and graceful shutdown).

### WebhookSubscriber

```python
class WebhookSubscriber:
    def __init__(
        self,
        url: str,
        headers: dict[str, str] | None = None,
        retry_count: int = 3,
        timeout_ms: int = 5000,
    ) -> None: ...
```

**Delivery:**
- Sends `POST` with `Content-Type: application/json` body containing the serialized event as a JSON object.
- Custom headers are merged with the content-type header.

**Retry strategy:**

| Response | Action |
|----------|--------|
| 2xx | Success, stop |
| 4xx | No retry (client error) |
| 5xx | Retry up to `retry_count` times |
| Connection error / timeout | Retry like 5xx |

### A2ASubscriber

```python
class A2ASubscriber:
    def __init__(
        self,
        platform_url: str,
        auth: str | dict[str, str] | None = None,
        timeout_ms: int = 5000,
    ) -> None: ...
```

**Authentication:**

| `auth` value | Behavior |
|-------------|----------|
| `str` | Added as `Authorization: Bearer {auth}` |
| `dict` | Keys merged into request headers |
| `None` | No auth header |

**Payload format:**
```json
{
  "skillId": "apevo.event_receiver",
  "event": {
    "event_type": "...",
    "module_id": "...",
    "timestamp": "...",
    "severity": "...",
    "data": { }
  }
}
```

Single attempt (no retries). Errors logged but not raised.

### Subscriber Type Registry

Extensible factory system for config-driven subscriber instantiation:

```python
from apcore.sys_modules.registration import register_subscriber_type

# Register a custom subscriber type
def my_factory(config: dict) -> EventSubscriber:
    return MyCustomSubscriber(**config)

register_subscriber_type("my_type", my_factory)
```

**Built-in types:**

| Type | Factory Config |
|------|---------------|
| `webhook` | `url`, `headers`, `retry_count`, `timeout_ms` |
| `a2a` | `platform_url`, `auth`, `timeout_ms` |

**API:**
- `register_subscriber_type(type_name, factory)` — Add a custom type.
- `unregister_subscriber_type(type_name)` — Remove a type.
- `reset_subscriber_registry()` — Reset to built-in types only.

## Event Types

Events emitted by the framework:

| Event Type | Legacy Alias | Severity | Source | Payload (`data`) |
|------------|-------------|----------|--------|-------------------|
| `module_registered` | — | info | Registry bridge | `module_id` |
| `module_unregistered` | — | info | Registry bridge | `module_id` |
| `apcore.config.updated` | `config_changed` | info | `system.control.update_config` | `key`, `old_value`, `new_value` |
| `apcore.module.reloaded` | `config_changed` | info | `system.control.reload_module` | `module_id`, `previous_version`, `new_version` |
| `apcore.module.toggled` | `module_health_changed` | info | `system.control.toggle_feature` | `module_id`, `enabled` |
| `apcore.health.recovered` | `module_health_changed` | info | `PlatformNotifyMiddleware` | `module_id`, recovery details |
| `apcore.error.threshold_exceeded` | `error_threshold_exceeded` | error | `PlatformNotifyMiddleware` | `module_id`, `error_rate`, `threshold` |
| `apcore.latency.threshold_exceeded` | `latency_threshold_exceeded` | warn | `PlatformNotifyMiddleware` | `module_id`, `p99_latency_ms`, `threshold` |

> **Note (§9.16):** Canonical `apcore.*` names are the only supported event types as of v0.18.0. Legacy aliases (`config_changed`, `module_health_changed`) were removed in v0.18.0 — subscribe to canonical names only.

## Configuration

```yaml
sys_modules:
  enabled: true
  events:
    enabled: true
    thresholds:
      error_rate: 0.1              # 10% error rate triggers alert
      latency_p99_ms: 5000.0       # 5s p99 triggers alert
    subscribers:
      - type: "webhook"
        url: "https://platform.example.com/events"
        headers:
          Authorization: "Bearer token"
          X-Custom: "value"
        retry_count: 3
        timeout_ms: 5000
      - type: "a2a"
        platform_url: "https://agent.example.com"
        auth: "bearer-token-123"
        timeout_ms: 5000
```

## Integration

### Via APCore Client (Recommended)

=== "Python"
    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    client = APCore(config=config)

    # Subscribe with simple callback
    sub = client.on("error_threshold_exceeded", lambda e: print(f"Alert: {e.data}"))

    # Async handler
    async def notify_admin(event):
        await send_notification(event.data)

    sub2 = client.on("apcore.module.toggled", notify_admin)

    # Unsubscribe
    client.off(sub)

    # Direct emitter access
    if client.events:
        client.events.subscribe(my_custom_subscriber)
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config } from "apcore-js";

    const config = Config.load('apcore.yaml');
    const client = new APCore({ config });

    // Subscribe with simple callback
    const sub = client.on("error_threshold_exceeded", (event) => console.log(event.data));

    // Another subscription
    const sub2 = client.on("apcore.module.toggled", (event) => console.log(event.data));

    // Unsubscribe
    client.off(sub);

    // Direct emitter access
    if (client.events) {
        client.events.subscribe(myCustomSubscriber);
    }
    ```
=== "Rust"
    ```rust
    use apcore::APCore;

    let client = APCore::from_path("apcore.yaml")?;

    // Subscribe with simple callback
    let sub = client.on("error_threshold_exceeded", Box::new(AlertSubscriber));

    // Another subscription
    let sub2 = client.on("apcore.module.toggled", Box::new(MySubscriber));

    // Unsubscribe
    client.off(&sub);

    // Direct emitter access
    if let Some(events) = client.events() {
        events.subscribe(my_custom_subscriber);
    }
    ```

### Via Direct EventEmitter

```python
from apcore.events import EventEmitter, ApCoreEvent, WebhookSubscriber

emitter = EventEmitter(max_workers=4)
emitter.subscribe(WebhookSubscriber(url="https://example.com/hook"))

emitter.emit(ApCoreEvent(
    event_type="custom.event",
    module_id="my.module",
    timestamp="2026-03-08T12:00:00Z",
    severity="info",
    data={"key": "value"},
))

# Wait for delivery in tests
emitter.flush(timeout=5.0)
```

## Dependencies

- `apcore.middleware.Middleware` — Base class for `PlatformNotifyMiddleware`.
- `apcore.observability.metrics.MetricsCollector` — Used by `PlatformNotifyMiddleware` for threshold checks.

??? info "Python SDK reference"
    The following tables are **not protocol requirements** — they document the Python SDK's source layout and runtime dependencies for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/events/emitter.py` | `EventEmitter`, `ApCoreEvent`, `EventSubscriber` |
    | `src/apcore/events/subscribers.py` | `WebhookSubscriber`, `A2ASubscriber` |
    | `src/apcore/sys_modules/registration.py` | Subscriber factory registry, `register_sys_modules()` integration |
    | `src/apcore/middleware/platform_notify.py` | `PlatformNotifyMiddleware` (threshold-based event emission) |
    | `src/apcore/client.py` | `APCore.on()`, `APCore.off()`, `_CallbackSubscriber` |

    **External dependencies:**

    - `aiohttp` (optional) — Required for `WebhookSubscriber` and `A2ASubscriber`. Install with `pip install apcore[events]`.
    - `threading` (stdlib) — Lock for subscriber list and pending futures.
    - `concurrent.futures` (stdlib) — `ThreadPoolExecutor` for async dispatch.

## Testing Strategy

- **EventEmitter**: Subscribe/emit/unsubscribe lifecycle, concurrent emit safety, subscriber error isolation, flush blocking behavior.
- **WebhookSubscriber**: 2xx/4xx/5xx response handling, retry count enforcement, timeout behavior, header merging.
- **A2ASubscriber**: Auth modes (string/dict/None), payload format, error logging.
- **Subscriber registry**: Custom type registration, factory invocation from config, reset to defaults.
- **PlatformNotifyMiddleware**: Threshold crossing detection, hysteresis (recovery at 50% of threshold), event emission verification.
