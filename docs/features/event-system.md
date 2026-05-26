# Event System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §10 Observability Specification.


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

=== "Python"

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

=== "TypeScript"

    ```typescript
    // From apcore-js/events
    export interface ApCoreEvent {
        readonly eventType: string;            // Event identifier
        readonly moduleId: string | null;      // Associated module (null for global events)
        readonly timestamp: string;            // ISO 8601 UTC timestamp
        readonly severity: string;             // "info" | "warn" | "error" | "fatal"
        readonly data: Record<string, unknown>; // Event-specific payload
    }
    ```

=== "Rust"

    ```rust
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ApCoreEvent {
        pub event_type: String,
        pub timestamp: String,                  // ISO 8601 UTC timestamp
        pub data: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub module_id: Option<String>,
        pub severity: String,                   // "info" | "warn" | "error" | "fatal"
    }
    ```

Immutable by design (Python uses `frozen=True`; TypeScript uses `readonly`; Rust passes events as `&ApCoreEvent` references) — prevents accidental mutation after emission.

### EventSubscriber Protocol

=== "Python"

    ```python
    from typing import Protocol, runtime_checkable

    from apcore.events import ApCoreEvent


    @runtime_checkable
    class EventSubscriber(Protocol):
        async def on_event(self, event: ApCoreEvent) -> None: ...
    ```

=== "TypeScript"

    ```typescript
    import type { ApCoreEvent } from "apcore-js";

    export interface EventSubscriber {
        onEvent(event: ApCoreEvent): void | Promise<void>;
    }
    ```

=== "Rust"

    ```rust
    use async_trait::async_trait;
    use apcore::errors::ModuleError;
    use apcore::events::ApCoreEvent;

    #[async_trait]
    pub trait EventSubscriber: Send + Sync + std::fmt::Debug {
        fn subscriber_id(&self) -> &str { "default" }
        fn event_pattern(&self) -> &str { "*" }
        async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError>;
    }
    ```

### EventEmitter

=== "Python"

    ```python
    from apcore.events import ApCoreEvent, EventSubscriber


    class EventEmitter:
        def __init__(self, max_workers: int = 4) -> None: ...
        def subscribe(self, subscriber: EventSubscriber) -> None: ...
        def unsubscribe(self, subscriber: EventSubscriber) -> None: ...
        def emit(self, event: ApCoreEvent) -> None: ...
        def flush(self, timeout: float = 5.0) -> None: ...
    ```

=== "TypeScript"

    ```typescript
    import type { ApCoreEvent, EventSubscriber } from "apcore-js";

    export class EventEmitter {
        constructor(maxPending?: number);
        subscribe(subscriber: EventSubscriber): void;
        unsubscribe(subscriber: EventSubscriber): void;
        emit(event: ApCoreEvent): void;
        // timeoutMs in ms (default 5000); pass 0 to wait indefinitely.
        flush(timeoutMs?: number): Promise<void>;
    }
    ```

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::events::{ApCoreEvent, EventSubscriber};

    pub struct EventEmitter { /* ... */ }

    impl EventEmitter {
        pub fn new() -> Self;
        pub fn subscribe(&mut self, subscriber: Box<dyn EventSubscriber>);
        pub fn unsubscribe(&mut self, subscriber: &dyn EventSubscriber) -> bool;
        pub fn unsubscribe_by_id(&mut self, subscriber_id: &str) -> bool;
        pub async fn emit(&self, event: &ApCoreEvent);
        pub fn emit_spawn(&self, event: ApCoreEvent);
        pub fn flush(&self, timeout_ms: u64) -> Result<(), ModuleError>;
        pub async fn shutdown(&mut self, timeout_ms: u64) -> Result<(), ModuleError>;
    }
    ```

**Dispatch model:**
- `emit()` returns immediately. Delivery is handled asynchronously by a bounded background worker pool (e.g., a thread pool in Python with a persistent async event loop).
- Each `emit()` takes a snapshot of current subscribers, so subscribe/unsubscribe during delivery is safe.
- Failed deliveries are logged but never re-raised.
- `flush()` blocks until all pending deliveries complete (useful in tests and graceful shutdown).

### WebhookSubscriber

=== "Python"

    ```python
    from apcore.events import WebhookSubscriber


    class WebhookSubscriber:
        def __init__(
            self,
            url: str,
            headers: dict[str, str] | None = None,
            retry_count: int = 3,
            timeout_ms: int = 5000,
        ) -> None: ...
    ```

=== "TypeScript"

    ```typescript
    import { WebhookSubscriber } from "apcore-js";

    // Constructor signature
    new WebhookSubscriber(
        url: string,
        headers?: Record<string, string>,
        retryCount?: number,   // default 3
        timeoutMs?: number,    // default 5000
    );
    ```

=== "Rust"

    ```rust
    use apcore::events::WebhookSubscriber;
    use std::collections::HashMap;

    pub struct WebhookSubscriber {
        pub id: String,
        pub url: String,
        pub event_pattern: String,
        pub headers: HashMap<String, String>,
        pub retry_count: u32,   // default 3
        pub timeout_ms: u64,    // default 5000
    }

    impl WebhookSubscriber {
        pub fn new(
            id: impl Into<String>,
            url: impl Into<String>,
            event_pattern: impl Into<String>,
        ) -> Self;
    }
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

=== "Python"

    ```python
    from apcore.events import A2ASubscriber


    class A2ASubscriber:
        def __init__(
            self,
            platform_url: str,
            auth: str | dict[str, str] | None = None,
            skill_id: str = "apevo.event_receiver",
            timeout_ms: int = 5000,
        ) -> None: ...
    ```

=== "TypeScript"

    ```typescript
    import { A2ASubscriber } from "apcore-js";

    // Constructor signature — `auth` may be a Bearer token string or a
    // record of headers to merge into the request.
    new A2ASubscriber(
        platformUrl: string,
        auth?: string | Record<string, string>,
        skillId?: string,      // default "apevo.event_receiver"
        timeoutMs?: number,    // default 5000
    );
    ```

=== "Rust"

    ```rust
    use apcore::events::{A2AAuth, A2ASubscriber};
    use std::collections::HashMap;

    pub enum A2AAuth {
        Bearer(String),                   // → Authorization: Bearer <token>
        Headers(HashMap<String, String>), // → merged into request headers
    }

    pub struct A2ASubscriber {
        pub id: String,
        pub platform_url: String,
        pub auth: Option<A2AAuth>,
        pub skill_id: String,
        pub event_pattern: String,
        pub timeout_ms: u64,              // default 5000
    }

    impl A2ASubscriber {
        pub fn new(
            id: impl Into<String>,
            platform_url: impl Into<String>,
            event_pattern: impl Into<String>,
        ) -> Self;
    }
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

**Delivery:** Retries on 5xx, connection errors, and timeouts according to the unified `retry` policy. Errors are logged and a dead-letter event is emitted on exhaustion. (See [§Event Delivery Semantics](#event-delivery-semantics-issue-61)).

### Subscriber Type Registry

Extensible factory system for config-driven subscriber instantiation:

=== "Python"

    ```python
    from apcore.events import EventSubscriber
    from apcore.sys_modules.registration import register_subscriber_type


    # Register a custom subscriber type
    def my_factory(config: dict) -> EventSubscriber:
        return MyCustomSubscriber(**config)

    register_subscriber_type("my_type", my_factory)
    ```

=== "TypeScript"

    ```typescript
    import { registerSubscriberType } from "apcore-js/events";
    import type { EventSubscriber } from "apcore-js";

    // Register a custom subscriber type
    registerSubscriberType("my_type", (config): EventSubscriber => {
        return new MyCustomSubscriber(config);
    });
    ```

=== "Rust"

    ```rust
    use apcore::events::{register_subscriber_type, EventSubscriber};

    // Factory closure receives a config Value, returns a boxed EventSubscriber
    register_subscriber_type(
        "my_type",
        |config| Box::new(MyCustomSubscriber::from_config(config)),
    );
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

## Event Naming Convention

Framework-emitted events **MUST** use the form `apcore.<subsystem>.<event>` where:

- `<subsystem>` is one of: `registry`, `health`, `config`, `module`, `modules`, `task`, `acl`, `approval`, `subscriber` (extensible — additional subsystem names may be reserved by future spec revisions).
- `<event>` is a `snake_case` past-tense verb describing the state transition (e.g., `module_registered`, `error_threshold_exceeded`, `recovered`, `updated`).

**Examples:**

- `apcore.registry.module_registered`
- `apcore.registry.module_unregistered`
- `apcore.health.error_threshold_exceeded`
- `apcore.health.latency_threshold_exceeded`
- `apcore.health.recovered`
- `apcore.config.updated`

Subscribers **MAY** filter by glob patterns: `apcore.registry.*`, `apcore.health.*`, `apcore.*`.

### Glob subscription example

=== "Python"
    ```python
    from apcore import APCore
    from apcore.config import Config

    config = Config.load("apcore.yaml")
    client = APCore(config=config)

    # Subscribe to every registry event (module_registered + module_unregistered)
    client.on("apcore.registry.*", lambda e: print(f"registry: {e.event_type} {e.data}"))

    # Subscribe to every health event
    client.on("apcore.health.*", lambda e: print(f"health: {e.event_type}"))
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config } from "apcore-js";

    const config = Config.load("apcore.yaml");
    const client = new APCore({ config });

    // Subscribe to every registry event
    client.on("apcore.registry.*", (event) =>
        console.log(`registry: ${event.eventType}`, event.data),
    );

    // Subscribe to every health event
    client.on("apcore.health.*", (event) =>
        console.log(`health: ${event.eventType}`),
    );
    ```
=== "Rust"
    ```rust
    use apcore::APCore;

    let client = APCore::from_path("apcore.yaml")?;

    // Subscribe to every registry event
    client.on("apcore.registry.*", Box::new(RegistryLogger));

    // Subscribe to every health event
    client.on("apcore.health.*", Box::new(HealthLogger));
    ```

### Deprecation: legacy event names

Some early SDK builds emitted unprefixed names (`module_registered`, `module_unregistered`) and `apcore.error.threshold_exceeded` / `apcore.latency.threshold_exceeded` (with `error` / `latency` as the subsystem segment). These names do not conform to the `apcore.<subsystem>.<event>` convention above.

**Normative rule:** Implementations **MUST** emit the canonical form. Through the v0.21.x cycle, implementations also dual-emitted the legacy name with `data.deprecated: true` so existing subscribers kept receiving events. **As of v0.22.0 dual-emission has ended and the legacy names are removed — implementations MUST emit only the canonical names** (see [protocol-spec.md §9.16](../spec/protocol-spec.md)).

| Legacy name | Canonical name | Removed in |
|-------------|----------------|----------------|
| `module_registered` | `apcore.registry.module_registered` | v0.22.0 |
| `module_unregistered` | `apcore.registry.module_unregistered` | v0.22.0 |
| `apcore.error.threshold_exceeded` | `apcore.health.error_threshold_exceeded` | v0.22.0 |
| `apcore.latency.threshold_exceeded` | `apcore.health.latency_threshold_exceeded` | v0.22.0 |

> **Historical note (§9.16):** v0.18.0 removed an earlier set of legacy aliases. The dual-emit cycle described above is a separate canonicalization pass tracked under issue #36 / D-34.

## Event Types

Events emitted by the framework (canonical names):

| Event Type | Severity | Source | Payload (`data`) |
|------------|----------|--------|-------------------|
| `apcore.registry.module_registered` | info | Registry bridge | `module_id` |
| `apcore.registry.module_unregistered` | info | Registry bridge | `module_id` |
| `apcore.config.updated` | info | `system.control.update_config` | `key`, `old_value`, `new_value` |
| `apcore.module.reloaded` | info | `system.control.reload_module` | `module_id`, `previous_version`, `new_version` |
| `apcore.module.toggled` | info | `system.control.toggle_feature` | `module_id`, `enabled` |
| `apcore.health.recovered` | info | `PlatformNotifyMiddleware` | `module_id`, recovery details |
| `apcore.health.error_threshold_exceeded` | error | `PlatformNotifyMiddleware` | `module_id`, `error_rate`, `threshold` |
| `apcore.health.latency_threshold_exceeded` | warn | `PlatformNotifyMiddleware` | `module_id`, `p99_latency_ms`, `threshold` |

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
    sub = client.on("apcore.health.error_threshold_exceeded", lambda e: print(f"Alert: {e.data}"))

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
    const sub = client.on("apcore.health.error_threshold_exceeded", (event) => console.log(event.data));

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
    let sub = client.on("apcore.health.error_threshold_exceeded", Box::new(AlertSubscriber));

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

=== "Python"
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
=== "TypeScript"
    ```typescript
    import { EventEmitter, ApCoreEvent, WebhookSubscriber } from "apcore-js";

    const emitter = new EventEmitter({ maxWorkers: 4 });
    emitter.subscribe(new WebhookSubscriber({ url: "https://example.com/hook" }));

    emitter.emit(new ApCoreEvent({
        eventType: "custom.event",
        moduleId: "my.module",
        timestamp: "2026-03-08T12:00:00Z",
        severity: "info",
        data: { key: "value" },
    }));

    // Wait for delivery in tests
    await emitter.flush(5.0);
    ```
=== "Rust"
    ```rust
    use apcore::events::{EventEmitter, ApCoreEvent, WebhookSubscriber};

    let emitter = EventEmitter::new(4);
    emitter.subscribe(Box::new(WebhookSubscriber::new("https://example.com/hook")));

    emitter.emit(ApCoreEvent {
        event_type: "custom.event".to_string(),
        module_id: "my.module".to_string(),
        timestamp: "2026-03-08T12:00:00Z".to_string(),
        severity: "info".to_string(),
        data: serde_json::json!({"key": "value"}),
    });

    // Wait for delivery in tests
    emitter.flush(std::time::Duration::from_secs(5));
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

## Contract: EventEmitter.emit

### Inputs
- `event_type` (str/string/&str, required) — event type identifier; MUST NOT be empty
- `payload` (dict/object/Value, optional) — event payload; passed as-is to subscribers

### Errors
- No errors raised (emit is fire-and-forget; subscriber errors are caught and logged internally)

### Returns
- On success: void/None/()

### Properties
- async: false in Python and TypeScript (synchronous fire-and-forget dispatch — TypeScript pushes async subscriber promises into an internal pending list and returns synchronously); async in Rust (the Rust async runtime model requires `pub async fn emit`, but observable semantics still match Python/TS fire-and-forget — subscriber errors are caught and logged internally, never propagated to the caller). All three SDKs deliver the same observable contract: emit returns immediately, never raises, and never blocks the caller on subscriber execution.
- thread_safe: true
- pure: false (invokes subscriber callbacks)
- idempotent: false

## Contract: EventEmitter.subscribe

> **Spec amendment (D10-016).** Earlier drafts described a handler-pattern
> shape `subscribe(event_type, handler)` returning a subscription handle.
> No SDK implements that. All three implementations adopt the
> subscriber-pattern: callers construct a typed `EventSubscriber`
> (carrying its own `event_pattern`, `subscriber_id`, and `on_event`
> callback) and pass that single object to `subscribe`. The SDKs are
> mutually consistent; the spec text was the outlier.

### Inputs
- `subscriber` (`EventSubscriber` instance, required) — subscriber object owning its own `event_pattern`, `subscriber_id`, and `on_event` callback. Construct via the SDK's typed subscriber classes (e.g. `RecordingSubscriber`, `WebhookSubscriber`, `A2ASubscriber`) or implement the `EventSubscriber` protocol/trait directly.

### Errors
- No errors raised by the cross-language contract. **Language-idiom exception (D10-002):** the Python SDK MAY raise `TypeError` at subscribe time if the subscriber's `on_event` is not a coroutine function. This is a dynamic-language precondition guard — the runtime equivalent of the compile-time type constraint that TypeScript (the `EventSubscriber` interface) and Rust (`Box<dyn EventSubscriber>`) enforce statically. A correctly-typed `EventSubscriber` never triggers it, so the guard is not an observable cross-SDK divergence.

### Returns
- On success: void/None/() — the SDKs do not return a separate handle. Removal uses `unsubscribe` / `unsubscribe_by_id` keyed on the subscriber object's identity (`apcore-python/src/apcore/events/emitter.py:55`, `apcore-typescript/src/events/emitter.ts:75`, `apcore-rust/src/events/emitter.rs:79`).

### Properties
- async: false
- thread_safe: true
- idempotent: false (each call creates a new subscription)

## Contract: WebhookSubscriber.deliver

### Inputs
- `event` (ApCoreEvent, required) — event to deliver via HTTP POST

### Errors
- `DeliveryError(code=WEBHOOK_DELIVERY_FAILED)` — HTTP delivery failed after retry exhaustion

### Returns
- On success: void/None/()

### Properties
- async: true
- thread_safe: true
- pure: false (outbound HTTP)

---

## Event Management Hardening (Issue #36)

### Cross-Language SubscriberFactory Parity

The `register_subscriber_type` API MUST be available in all three SDKs. Implementations MUST provide `register_subscriber_type(type_name, factory)` as a public function. The factory MUST be called once per subscriber entry in the configuration and MUST receive the subscriber's config sub-object.

=== "Python"
    ```python
    from apcore.sys_modules.registration import register_subscriber_type

    # Factory receives a config dict, returns an EventSubscriber instance
    def slack_factory(config: dict) -> EventSubscriber:
        return SlackSubscriber(
            webhook_url=config["webhook_url"],
            channel=config.get("channel", "#general"),
        )

    register_subscriber_type("slack", slack_factory)
    ```
=== "TypeScript"
    ```typescript
    import { registerSubscriberType } from "apcore-js/events";

    // Factory function receives a config object, returns an EventSubscriber
    registerSubscriberType("slack", (config) => new SlackSubscriber(config));
    ```
=== "Rust"
    ```rust
    use apcore::events::register_subscriber_type;

    // Rust uses a closure or a struct that implements the SubscriberFactory trait
    register_subscriber_type("slack", |config| Box::new(SlackSubscriber::from_config(config)));
    ```

Once registered, the `"slack"` type can be referenced by name in configuration:

```yaml
subscribers:
  - type: "slack"
    webhook_url: "https://hooks.slack.com/services/..."
    channel: "#alerts"
```

### Built-in Subscriber Types

Implementations MUST provide `file`, `stdout`, and `filter` as built-in subscriber types in addition to the existing `webhook` and `a2a` types. No registration call is required for built-in types.

**Updated built-in type table:**

| Type | Factory Config | Description |
|------|---------------|-------------|
| `webhook` | `id` (optional), `url`, `headers`, `retry_count`, `timeout_ms`, `retry` | HTTP POST delivery with configurable retry. |
| `a2a` | `id` (optional), `platform_url`, `skill_id` (default `"apevo.event_receiver"`), `auth`, `timeout_ms`, `retry` | Agent-to-Agent protocol bridge with bearer/dict auth. |
| `file` | `id` (optional), `path`, `append` (bool, default `true`), `format` (`json`/`text`), `rotate_bytes`, `retry` | Writes events to a local file. |
| `stdout` | `id` (optional), `format` (`json`/`text`, default `text`), `level_filter`, `retry` | Writes events to stdout/stderr. |
| `filter` | `id` (optional), `delegate_type`, `delegate_config`, `include_events` (list), `exclude_events` (list), `retry` | Wraps another subscriber with event-name filtering. |

The `id` field (string, optional; SDK-generates a stable identifier when omitted) and the `retry` block (see [§Event Delivery Semantics](#event-delivery-semantics-issue-61)) apply uniformly to every subscriber type. The `skill_id` field on `a2a` was promoted from a hardcoded constant to a normative config field by issue #61.

**Normative rules for `filter`:** A `filter` subscriber MUST forward matching events to its delegate and MUST silently discard non-matching events. Matching is evaluated against `include_events` first (if present); if the event name matches any pattern in `include_events`, it is forwarded. Events matching any pattern in `exclude_events` are discarded even when `include_events` is absent.

**YAML configuration examples:**

```yaml
subscribers:
  - type: "stdout"
    format: "json"
    level_filter: "error"   # only emit error-level events

  - type: "file"
    path: "/var/log/apcore/events.jsonl"
    format: "json"
    rotate_bytes: 10485760  # 10 MB

  - type: "filter"
    delegate_type: "webhook"
    delegate_config:
      url: "https://pagerduty.example.com/hook"
    include_events:
      - "apcore.health.*"
      - "apcore.health.error_threshold_exceeded"
```

### Configuration-driven subscribers

The five built-in factories (`webhook`, `a2a`, `file`, `stdout`, `filter`) plus any types added through `register_subscriber_type` can be instantiated declaratively from `apcore.yaml`. Implementations **MUST** invoke each registered factory exactly once per matching subscriber entry, passing the entry's config sub-object (with `type` removed).

**Factory registration API (cross-language):**

=== "Python"
    ```python
    from apcore.sys_modules.registration import register_subscriber_type

    def slack_factory(config: dict) -> "EventSubscriber":
        return SlackSubscriber(webhook_url=config["webhook_url"])

    register_subscriber_type("slack", slack_factory)
    ```
=== "TypeScript"
    ```typescript
    import { registerSubscriberType } from "apcore-js/events";

    registerSubscriberType("slack", (config) => new SlackSubscriber(config));
    ```
=== "Rust"
    ```rust
    use apcore::events::register_subscriber_type;

    register_subscriber_type(
        "slack",
        |config| Box::new(SlackSubscriber::from_config(config)),
    );
    ```

**YAML example loading three subscribers:**

```yaml
sys_modules:
  enabled: true
  events:
    enabled: true
    subscribers:
      - type: "stdout"            # Built-in: pretty-print to stderr/stdout
        format: "json"
        level_filter: "info"

      - type: "file"              # Built-in: append to a local JSONL log
        path: "/var/log/apcore/events.jsonl"
        format: "json"
        rotate_bytes: 10485760

      - type: "filter"            # Built-in: wrap a webhook with event filter
        delegate_type: "webhook"
        delegate_config:
          url: "https://platform.example.com/events"
          headers:
            Authorization: "Bearer ${PLATFORM_TOKEN}"
        include_events:
          - "apcore.health.*"
          - "apcore.registry.*"
```

### Circuit-Breaker Resilience

Each subscriber instance is independently wrapped by a circuit-breaker that prevents a degraded downstream system from blocking or starving the event bus.

**State machine:**

```
CLOSED → (consecutive_failures >= open_threshold) → OPEN
OPEN → (recovery_window_ms elapsed since last_failure_at) → HALF_OPEN
HALF_OPEN → (delivery success) → CLOSED
HALF_OPEN → (delivery failure) → OPEN
```

**Normative rules:**

- Implementations MUST wrap each subscriber's `deliver()` call in a timeout enforcer. If delivery exceeds `timeout_ms` (default: `5000`), the call MUST be abandoned and counted as a failure.
- Implementations MUST track per-subscriber health: a `consecutive_failures` counter and a `last_failure_at` timestamp.
- When `consecutive_failures` reaches `open_threshold` (default: `5`), the circuit MUST transition to `OPEN` state. In `OPEN` state, `deliver()` MUST NOT be called — events are silently discarded.
- After `recovery_window_ms` (default: `60000`) elapses since `last_failure_at`, the circuit MUST transition to `HALF_OPEN` state and attempt one delivery. If the delivery succeeds, the circuit transitions to `CLOSED`. If it fails, the circuit returns to `OPEN`.
- Implementations MUST emit an `apcore.subscriber.circuit_opened` event when transitioning to `OPEN`, and `apcore.subscriber.circuit_closed` when transitioning back to `CLOSED`.
- Implementations SHOULD log at `WARN` level when a subscriber's circuit opens.

**YAML configuration:**

```yaml
subscribers:
  - type: "webhook"
    url: "https://example.com/hook"
    circuit_breaker:
      timeout_ms: 3000
      open_threshold: 3
      recovery_window_ms: 30000
```

**Circuit-breaker events (additions to the Event Types table):**

| Event Type | Severity | Source | Payload (`data`) |
|------------|----------|--------|-------------------|
| `apcore.subscriber.circuit_opened` | warn | EventEmitter circuit breaker | `subscriber_type`, `consecutive_failures` |
| `apcore.subscriber.circuit_closed` | info | EventEmitter circuit breaker | `subscriber_type`, `recovery_attempt` |

## Contract: SubscriberCircuitBreaker.on_failure

### Inputs
- `subscriber_id` (str/string/&str, required) — unique identifier for the subscriber instance
- `error` (Exception/Error/Box<dyn Error>, required) — the delivery error

### Errors
- None (circuit breaker MUST NOT raise; it records state internally)

### Returns
- On success: `CircuitState` — the new state (`OPEN`, `CLOSED`, or `HALF_OPEN`)

### Properties
- async: false
- thread_safe: true (state MUST be protected by a lock/mutex/RwLock)
- pure: false (mutates circuit state)
- idempotent: false

## Contract: EventEmitter.unsubscribe

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `subscriber` (EventSubscriber/EventSubscriber/&dyn EventSubscriber, required) — the subscriber instance to remove; MUST be the same object reference that was passed to `subscribe`.

### Errors

- None. If `subscriber` is not currently registered, the call is a no-op. Implementations MUST NOT raise or panic on an unregistered subscriber.

### Returns

- On success: void/None/()

### Properties

- async: false
- thread_safe: true (registry lock MUST be held during the removal)
- pure: false (mutates the subscriber list)
- idempotent: true (repeated calls with the same subscriber are safe no-ops)

!!! note "Python SDK behavior"
    `EventEmitter.unsubscribe(subscriber)` removes the first occurrence of `subscriber` from the internal list using object identity. Subsequent identical calls succeed silently because the `ValueError` from `list.remove()` is swallowed.

## Contract: EventEmitter.flush

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `timeout` (float/number/f64, optional, default `5.0`) — maximum seconds to wait for all in-flight deliveries to complete. MUST be a positive finite number.

### Errors

- None raised to the caller. Any subscriber errors that surface during the flush window are silently discarded (they were already logged at delivery time). If the timeout elapses before all pending deliveries complete, `flush` returns without error — it does not guarantee all deliveries finished.

### Returns

- On success: void/None/()

### Properties

- async: false (blocks the calling thread until deliveries complete or timeout expires)
- thread_safe: true
- pure: false (waits on futures held in shared state)
- idempotent: true (safe to call multiple times; extra calls on an already-empty pending set return immediately)

!!! note "Python SDK behavior"
    `flush(timeout)` iterates the snapshot of pending `Future` objects, calling `future.result(timeout=timeout)` on each, catching all exceptions. Completed futures are removed from `_pending_futures` at the end of the call. The per-future timeout is the full `timeout` value, not divided across futures; in the worst case the total wall-clock wait is `len(pending_futures) × timeout`.

!!! warning "Post-shutdown behavior"
    After `EventEmitter.shutdown()` is called, `emit()` drops all events silently. `flush()` called after shutdown will return immediately with no pending work.

## Contract: A2ASubscriber.deliver

Normative behavioral contract. All SDK implementations MUST satisfy these guarantees.

### Inputs

- `event` (ApCoreEvent, required) — event to deliver to the A2A platform endpoint.

### Errors

- `ImportError` — raised synchronously before any network I/O if the `aiohttp` optional dependency is not installed. Message: `"aiohttp is required for A2ASubscriber. Install with: pip install apcore[events]"`.
- No other errors are raised to the caller. All HTTP and network failures are caught, logged at `ERROR` level, and silently swallowed. The circuit breaker (if active) records the failure.

### Returns

- On success: void/None/()

### Properties

- async: true (MUST be awaited; uses an async HTTP client internally)
- thread_safe: true (no mutable shared state between concurrent calls)
- pure: false (outbound HTTP POST to the platform URL)
- retry: yes — retries on 5xx, connection errors, and timeouts according to the unified `retry` policy. On retry exhaustion it emits `apcore.event.delivery_failed` and does not raise.

### Payload Format

The HTTP POST body is a JSON object with two keys:

```json
{
  "skillId": "apevo.event_receiver",
  "event": { "<serialized ApCoreEvent fields>" }
}
```

The `Content-Type` header is always `application/json`. If `auth` is a plain string it is sent as `Authorization: Bearer <auth>`. If `auth` is a dict its key-value pairs are merged directly into the headers.

### Behavior on 4xx / 5xx Responses

Both `WebhookSubscriber` and `A2ASubscriber` apply the [generic delivery retry policy](#per-subscriber-retry-policy-normative) defined below. The HTTP-status policy is the same for both: 4xx responses are client errors and MUST NOT be retried; 5xx responses, connection errors, and timeouts MUST be retried according to the subscriber's `retry` config. On retry exhaustion the SDK MUST emit [`apcore.event.delivery_failed`](#dead-letter-event-on-permanent-failure-normative); it MUST NOT raise into the emitter.

### Comparison with WebhookSubscriber.deliver

| Property | WebhookSubscriber | A2ASubscriber |
|----------|-------------------|---------------|
| Retry on 5xx | Yes (per [`retry` config](#per-subscriber-retry-policy-normative); default `max_attempts: 3`) | Yes (per [`retry` config](#per-subscriber-retry-policy-normative); default `max_attempts: 3`) |
| Retry on 4xx | No | No |
| On exhaustion | Emit [`apcore.event.delivery_failed`](#dead-letter-event-on-permanent-failure-normative) | Emit [`apcore.event.delivery_failed`](#dead-letter-event-on-permanent-failure-normative) |
| Auth mechanism | Custom headers only | Bearer string or header dict |
| Payload format | Serialized `ApCoreEvent` fields | `{skillId, event: <fields>}` wrapper |
| Raises on HTTP error | No | No |
| Raises on missing dep | `ImportError` | `ImportError` |

## Event Delivery Semantics (Issue #61)

Earlier sections specify how individual subscriber types (notably [`WebhookSubscriber`](#webhooksubscriber)) handle delivery failure. The rules in this section extend those guarantees to **every** subscriber type — built-in (`webhook`, `a2a`, `file`, `stdout`, `filter`) and any third-party subscriber registered via [`register_subscriber_type`](#cross-language-subscriberfactory-parity).

The motivating defect: in-process delivery to non-`webhook` subscribers has historically been *fire-and-forget*. A transient exception causes the event to be logged once and discarded. There is no retry, no dead-letter signal, and the emitter has no observability handle on the failure. In production this silently drops audit records, monitoring alerts, and cross-service notifications.

!!! warning "Discovered during apcore-a2a upgrade"
    `A2ASubscriber` previously hardcoded `skillId="apevo.event_receiver"`. When the receiving agent was restarting or unreachable, the event was lost with no fallback. The `skill_id` configuration field defined below resolves that specific case, and the generic delivery contract resolves the broader class.

### Per-Subscriber Retry Policy (Normative)

Every subscriber type — built-in **and** user-registered — **MUST** accept a `retry` config block. SDKs **MUST** honor the same field names, defaults, and backoff formula across languages.

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `max_attempts` | int ≥ 1 | `3` | Total attempts, including the first. `1` disables retry. |
| `initial_backoff_ms` | int ≥ 0 | `100` | Delay before attempt 1 (the first retry). |
| `max_backoff_ms` | int ≥ `initial_backoff_ms` | `30000` | Upper bound on per-attempt delay. |
| `backoff_multiplier` | float ≥ 1.0 | `2.0` | Multiplicative growth factor. |

**Backoff formula** (`attempt` is zero-based; `attempt = 0` is the first retry after the initial try):

```
delay_ms(attempt) = min(max_backoff_ms, initial_backoff_ms * backoff_multiplier ** attempt)
```

Worked example with defaults (`max_attempts=3`, `initial_backoff_ms=100`, `max_backoff_ms=30000`, `backoff_multiplier=2.0`):

| Step | Action | Delay before action |
|------|--------|---------------------|
| Initial try | `subscriber.on_event(event)` | 0 ms |
| Retry 1 (`attempt=0`) | re-delivery | 100 ms |
| Retry 2 (`attempt=1`) | re-delivery | 200 ms |
| (give up — `max_attempts=3` reached) | emit DLQ event | — |

**Interaction with the legacy `webhook.retry_count` field.** The pre-existing `retry_count` config on `webhook` (see [WebhookSubscriber](#webhooksubscriber)) is an alias for `retry.max_attempts`. If both are present in a single `webhook` config block, **`retry.max_attempts` wins** and the SDK **SHOULD** emit a `WARNING` log naming the subscriber and the conflicting fields. The 4xx-no-retry / 5xx-retry policy still governs *which* webhook responses trigger a retry — the `retry` block governs *how many* and *how long*.

**YAML example** showing the policy on multiple subscriber types:

```yaml
subscribers:
  - type: "a2a"
    platform_url: "https://platform.example.com"
    skill_id: "myapp.event_receiver"
    auth: "Bearer XXXX"
    retry:
      max_attempts: 5
      initial_backoff_ms: 250
      max_backoff_ms: 10000
      backoff_multiplier: 2.0

  - type: "file"
    path: "/var/log/apcore/events.jsonl"
    retry:
      max_attempts: 2      # one retry on transient I/O error
      initial_backoff_ms: 50

  - type: "stdout"
    retry:
      max_attempts: 1      # no retry; stdout is local
```

### Dead-Letter Event on Permanent Failure (Normative)

When the configured retries are exhausted, the SDK **MUST** emit a built-in event named `apcore.event.delivery_failed`. The payload schema **MUST** be:

```json
{
  "subscriber_type": "a2a",
  "subscriber_id": "remote-monitor-1",
  "original_event": {
    "name": "apcore.health.error_threshold_exceeded",
    "payload": { "service": "billing", "error_rate": 0.42 },
    "metadata": { "emitted_at": "2026-05-19T10:14:22.301Z" }
  },
  "error": {
    "type": "ConnectionError",
    "message": "Failed to connect to https://platform.example.com after 5 attempts"
  },
  "attempt_count": 5,
  "timestamp": "2026-05-19T10:14:28.812Z"
}
```

Normative rules:

- The DLQ event itself **MUST NOT** be retried, regardless of subscriber configuration. If a subscriber registered for `apcore.event.delivery_failed` itself raises, the SDK logs at `ERROR` level and discards. This prevents an unbounded loop when the DLQ destination is also broken.
- The DLQ event **MUST** be emitted via the same `EventEmitter` as the original event, so any subscriber (including persistent storage, on-call paging, etc.) can opt in by name.
- `subscriber_id` is taken from an optional `id` field in the subscriber's configuration (newly added — see table below). If the config omits `id`, the SDK MUST generate a stable identifier for that subscriber instance (e.g., `"{type}-{N}"` where `N` is the registration order). The same `subscriber_id` MUST be used across all DLQ events emitted by that subscriber instance.

The optional `id` field is added to every subscriber type's config schema:

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `id` | string | SDK-generated | Stable identifier surfaced in `apcore.event.delivery_failed` and other observability hooks. Recommended for production deployments so operators can correlate DLQ events with config entries. |

**Example — persistent DLQ via a dedicated subscriber:**

The example below routes only `apcore.event.delivery_failed` to a file by composing a `filter` around a delegate `file` subscriber. Note that the **delegate's config is inline** under `delegate_config`; the outer `filter` is the only top-level subscriber, so no other events leak to disk.

```yaml
subscribers:
  - type: "filter"
    id: "dlq-recorder"
    delegate_type: "file"
    delegate_config:
      path: "/var/log/apcore/dlq.jsonl"
      format: "json"
      rotate_bytes: 10485760
    include_events:
      - "apcore.event.delivery_failed"
```

### `on_failure` Callback (SHOULD)

As an ergonomic alternative to subscribing to `apcore.event.delivery_failed`, SDKs **SHOULD** extend the [`EventSubscriber` Protocol](#eventsubscriber-protocol) with an optional `on_failure(event, error, attempt_count)` method. When present, it **MUST** be invoked exactly once per permanent delivery failure, with the same information that populates the DLQ event payload. The retry policy is carried on the subscriber instance itself (as a `retry` attribute / getter / method) so the subscriber remains the single argument to the canonical `EventEmitter.subscribe(subscriber)` call.

=== "Python"
    ```python
    from apcore.events import ApCoreEvent, EventEmitter, EventSubscriber

    class HealthAlertSubscriber:
        """Implements EventSubscriber and the optional on_failure / retry hooks."""

        event_pattern = "apcore.health.*"
        retry = {"max_attempts": 5, "initial_backoff_ms": 250}

        async def on_event(self, event: ApCoreEvent) -> None:
            await deliver_to_external_system(event)

        async def on_failure(
            self,
            event: ApCoreEvent,
            error: Exception,
            attempt_count: int,
        ) -> None:
            await pager.alert(
                f"Permanent delivery failure: {event.name} "
                f"after {attempt_count} attempts: {error}"
            )

    emitter = EventEmitter()
    emitter.subscribe(HealthAlertSubscriber())
    ```
=== "TypeScript"
    ```typescript
    import { ApCoreEvent, EventEmitter, EventSubscriber } from "apcore-js";

    class HealthAlertSubscriber implements EventSubscriber {
        readonly eventPattern = "apcore.health.*";
        readonly retry = { maxAttempts: 5, initialBackoffMs: 250 };

        async onEvent(event: ApCoreEvent): Promise<void> {
            await deliverToExternalSystem(event);
        }

        async onFailure(
            event: ApCoreEvent,
            error: Error,
            attemptCount: number,
        ): Promise<void> {
            await pager.alert(
                `Permanent delivery failure: ${event.name} ` +
                `after ${attemptCount} attempts: ${error.message}`
            );
        }
    }

    const emitter = new EventEmitter();
    emitter.subscribe(new HealthAlertSubscriber());
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::events::{ApCoreEvent, EventEmitter, EventSubscriber, RetryConfig};
    use async_trait::async_trait;

    #[derive(Debug)]
    struct HealthAlertSubscriber;

    #[async_trait]
    impl EventSubscriber for HealthAlertSubscriber {
        fn subscriber_id(&self) -> &str { "health-alert" }
        fn event_pattern(&self) -> &str { "apcore.health.*" }

        fn retry(&self) -> RetryConfig {
            RetryConfig { max_attempts: 5, initial_backoff_ms: 250, ..Default::default() }
        }

        async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
            deliver_to_external_system(event).await
        }

        async fn on_failure(
            &self,
            event: &ApCoreEvent,
            error: &ModuleError,
            attempt_count: u32,
        ) {
            pager::alert(&format!(
                "Permanent delivery failure: {} after {} attempts: {}",
                event.name, attempt_count, error,
            ));
        }
    }

    let mut emitter = EventEmitter::new();
    emitter.subscribe(Box::new(HealthAlertSubscriber));
    ```

The `retry` config and the `on_failure` method are **additive optional members** on the existing `EventSubscriber` Protocol. SDKs **MUST NOT** introduce a parallel `subscribe(pattern, callback, options)` signature — the canonical `subscribe(subscriber)` form remains the single registration entry point.

If both an `on_failure` method on the subscriber and a separate subscriber registered for `apcore.event.delivery_failed` are configured, **both MUST fire** — they are independent observability channels.

### `a2a` Subscriber: Configurable `skill_id` (Normative)

The `a2a` subscriber configuration **MUST** accept an optional `skill_id` field. The default value remains `"apevo.event_receiver"` for backward compatibility.

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `skill_id` | string | `"apevo.event_receiver"` | The receiving agent's skill ID. The outgoing payload's `skillId` field is set from this. |

When configured, the JSON payload (see [Payload Format](#payload-format)) carries the configured value:

```json
{
  "skillId": "myapp.event_receiver",
  "event": { "<serialized ApCoreEvent fields>" }
}
```

This unblocks deployments where the receiving agent registers under an application-specific skill ID rather than the framework default. The hardcoded value is no longer normative — only the default is.

### Implementation Notes (Informative)

The following notes are guidance for SDK implementers and are **not normative**:

- **Per-subscriber retry isolation.** SDKs should run each subscriber's delivery + retry loop in its own task / coroutine / future so that one slow or hanging subscriber does not delay delivery to others. A bounded worker pool with per-subscriber concurrency limits is one viable pattern.
- **Jitter.** SDKs MAY add small random jitter (`±10%`) to the computed backoff delay to avoid thundering-herd retries against a recovering downstream. If applied, the resulting delay MUST still fall within `[0, max_backoff_ms]`.
- **Cancellation.** If the host process is shutting down, in-flight retry timers SHOULD be cancelled and the corresponding events SHOULD emit DLQ events with `error.type = "ShutdownInterrupted"` so persistent DLQ subscribers can record them before exit.
- **Memory bound.** The DLQ event mechanism is in-process and best-effort. Deployments that need durable failure records should subscribe a persistent storage subscriber (file, S3, database) to `apcore.event.delivery_failed`.
