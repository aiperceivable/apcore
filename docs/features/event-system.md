---
description: "EventEmitter bus for framework lifecycle events: ApCoreEvent envelope, EventSubscriber protocol, exact and pattern subscriptions, built-in subscribers, per-subscriber retry, dead-letter events."
---

# Event System

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC §9.16](../spec/protocol-spec.md#916-event-type-naming-convention-and-canonical-definitions) Event Type Naming Convention and Canonical Definitions.


## Overview

The event system is a bus for framework lifecycle events — module registration, health thresholds, runtime control, governance decisions, delivery failures. Subscribers receive events asynchronously: `emit()` never blocks module execution and never raises into the emitting code. Each subscriber gets its own retry policy, and an event that cannot be delivered produces a dead-letter event instead of disappearing.

With `sys_modules.enabled` and `sys_modules.events.enabled`, `APCore` creates the emitter, wires the framework's emitters to it, and exposes it as `client.events` ([APCore Unified Client](./apcore-client.md#construction-with-a-config)).

## Requirements

- Provide an `EventEmitter` with thread-safe subscriber management and non-blocking dispatch.
- Define `ApCoreEvent` as the immutable event envelope.
- Define the `EventSubscriber` protocol: an async `on_event()` plus optional `event_pattern`, `subscriber_id`, `subscriber_type`, `retry` and `on_failure` members.
- Errors in one subscriber **MUST NOT** propagate to other subscribers or to the emitter's caller.
- Retry failed deliveries per subscriber and emit `apcore.event.delivery_failed` when retries are exhausted.
- Provide built-in `webhook`, `a2a`, `file`, `stdout` and `filter` subscribers, and a registry of subscriber factories for config-driven instantiation.

## Technical Design

### ApCoreEvent

=== "Python"

    ```python
    from dataclasses import dataclass
    from typing import Any


    @dataclass(frozen=True)
    class ApCoreEvent:
        event_type: str               # e.g. "apcore.module.toggled"
        module_id: str | None         # Associated module (None for global events)
        timestamp: str                # ISO 8601 UTC
        severity: str                 # "info" | "warn" | "error" | "fatal"
        data: dict[str, Any]          # Event-specific payload
    ```

=== "TypeScript"

    ```typescript
    export interface ApCoreEvent {
        readonly eventType: string;
        readonly moduleId: string | null;       // null for global events
        readonly timestamp: string;             // ISO 8601 UTC
        readonly severity: string;              // "info" | "warn" | "error" | "fatal"
        readonly data: Record<string, unknown>;
    }

    // Build one with createEvent(eventType, moduleId, severity, data) — it stamps the timestamp.
    ```

=== "Rust"

    ```rust
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ApCoreEvent {
        pub event_type: String,
        pub timestamp: String,                  // ISO 8601 UTC
        pub data: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub module_id: Option<String>,
        pub severity: String,                   // "info" | "warn" | "error" | "fatal"
    }

    // Construct with ApCoreEvent::new(event_type, data) (severity "info", no module)
    // or ApCoreEvent::with_module(event_type, data, module_id, severity).
    ```

Events are immutable once emitted (Python `frozen=True`, TypeScript `readonly`, Rust subscribers receive `&ApCoreEvent`). On the wire (webhook, A2A, file) an event is a JSON object with the snake_case keys `event_type`, `module_id`, `timestamp`, `severity`, `data`.

### EventSubscriber

=== "Python"

    ```python
    from typing import Protocol, runtime_checkable

    from apcore.events import ApCoreEvent


    @runtime_checkable
    class EventSubscriber(Protocol):
        async def on_event(self, event: ApCoreEvent) -> None: ...

    # Optional attributes read by the emitter:
    #   event_pattern: str              (default "*")
    #   subscriber_id: str              (generated as "<type>-<n>" when absent)
    #   subscriber_type: str            (default derived from the class name)
    #   retry: EventRetryConfig | dict  (default policy below)
    #   async on_failure(event, error, attempt_count)
    ```

=== "TypeScript"

    ```typescript
    import type { ApCoreEvent, EventRetryConfig } from "apcore-js";

    export interface EventSubscriber {
        onEvent(event: ApCoreEvent): void | Promise<void>;
        readonly subscriberId?: string;
        readonly subscriberType?: string;
        readonly eventPattern?: string;          // default "*"
        readonly retry?: EventRetryConfig;
        onFailure?(event: ApCoreEvent, error: Error, attemptCount: number): void | Promise<void>;
    }
    ```

=== "Rust"

    ```rust
    use apcore::errors::ModuleError;
    use apcore::events::{ApCoreEvent, EventRetryConfig};
    use async_trait::async_trait;

    #[async_trait]
    pub trait EventSubscriber: Send + Sync + std::fmt::Debug {
        fn subscriber_id(&self) -> &str { "default" }
        fn event_pattern(&self) -> &str { "*" }
        fn subscriber_type(&self) -> &str { "subscriber" }
        fn retry(&self) -> EventRetryConfig { EventRetryConfig::default() }
        async fn on_failure(&self, _event: &ApCoreEvent, _error: &ModuleError, _attempt_count: u32) {}
        async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError>;
    }
    ```

Python's `EventEmitter.subscribe()` raises `TypeError` when `on_event` is not a coroutine function. In Rust, give every subscriber a distinct `subscriber_id()`: the default `"default"` makes subscribers indistinguishable to `unsubscribe_by_id`.

### EventEmitter

=== "Python"

    ```python
    from apcore.events import ApCoreEvent, EventSubscriber


    class EventEmitter:
        def __init__(self, max_workers: int = 4) -> None: ...
        def subscribe(self, subscriber: EventSubscriber) -> None: ...
        def unsubscribe(self, subscriber: EventSubscriber) -> None: ...
        def emit(self, event: ApCoreEvent) -> None: ...
        def flush(self, timeout: float = 5.0) -> None: ...     # seconds
        def shutdown(self, timeout: float = 5.0) -> None: ...  # seconds
    ```

=== "TypeScript"

    ```typescript
    import type { ApCoreEvent, EventSubscriber } from "apcore-js";

    export declare class EventEmitter {
        constructor(maxPending?: number);                // default 1000
        subscribe(subscriber: EventSubscriber): void;
        unsubscribe(subscriber: EventSubscriber): void;
        emit(event: ApCoreEvent): void;
        flush(timeoutMs?: number): Promise<void>;        // default 5000; 0 waits indefinitely
        shutdown(timeoutMs?: number): Promise<void>;
    }
    ```

=== "Rust"

    | Method | Signature |
    |--------|-----------|
    | `new` | `EventEmitter::new() -> EventEmitter` |
    | `subscribe` | `fn subscribe(&self, Box<dyn EventSubscriber>) -> SubscriberHandle` |
    | `unsubscribe_handle` | `fn unsubscribe_handle(&self, SubscriberHandle) -> bool` |
    | `unsubscribe` / `unsubscribe_by_id` | `fn unsubscribe(&self, &dyn EventSubscriber) -> bool` / `fn unsubscribe_by_id(&self, &str) -> bool` — match by `subscriber_id()` |
    | `emit` | `async fn emit(&self, &ApCoreEvent)` — spawns the deliveries and returns |
    | `emit_spawn` | `fn emit_spawn(&self, ApCoreEvent)` — the same, from synchronous code |
    | `flush` | `async fn flush(&self, timeout_ms: u64) -> Result<(), ModuleError>` (`0` waits indefinitely); `flush_default()` uses 5000 ms |
    | `shutdown` | `async fn shutdown(&self, timeout_ms: u64) -> Result<(), ModuleError>` |

**Dispatch model:**

- `emit()` snapshots the subscribers and returns; delivery runs in the background (a thread pool driving a private event loop in Python, pending promises in TypeScript, spawned tasks in Rust). Subscribing or unsubscribing during delivery is safe.
- Each matching subscriber is delivered to independently, with its own retry loop, so a slow or failing subscriber does not delay the others.
- `flush()` waits for pending deliveries up to its timeout and returns without error when the timeout elapses. The unit is per-SDK and named by the parameter: Python takes seconds, TypeScript and Rust milliseconds. Pass a positive value — `0` returns immediately in Python and waits indefinitely in TypeScript and Rust.
- After `shutdown()`, `emit()` drops events silently and `flush()` returns immediately.

### Subscribing

`client.on(event_type, handler)` delivers an event only when its type **equals** `event_type`. There is no glob matching: `client.on("apcore.registry.*", …)` subscribes to the literal type `apcore.registry.*` and receives nothing. To receive several types, subscribe once per type:

=== "Python"
    ```python
    from apcore import APCore
    from apcore.config import Config

    client = APCore(config=Config.load("apcore.yaml"))  # sys_modules.events.enabled: true

    def on_registry_event(event) -> None:
        print(f"registry: {event.event_type} {event.module_id}")

    subs = [
        client.on(event_type, on_registry_event)
        for event_type in ("apcore.registry.module_registered", "apcore.registry.module_unregistered")
    ]
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config, type ApCoreEvent } from "apcore-js";

    const client = new APCore({ config: Config.load("apcore.yaml") }); // sys_modules.events.enabled: true

    const onRegistryEvent = (event: ApCoreEvent): void => {
        console.log(`registry: ${event.eventType} ${event.moduleId}`);
    };

    const subs = ["apcore.registry.module_registered", "apcore.registry.module_unregistered"].map(
        (eventType) => client.on(eventType, onRegistryEvent),
    );
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::APCore;

    fn main() -> Result<(), ModuleError> {
        let mut client = APCore::from_path("apcore.yaml")?; // sys_modules.events.enabled: true

        let mut ids = Vec::new();
        for event_type in ["apcore.registry.module_registered", "apcore.registry.module_unregistered"] {
            ids.push(client.on(event_type, |event| {
                println!("registry: {} {:?}", event.event_type, event.module_id);
            })?);
        }
        Ok(())
    }
    ```

**Pattern subscriptions** use a subscriber's `event_pattern`, subscribed directly on the emitter. Patterns are matched with algorithm **A25** ([§9.16.3](../spec/protocol-spec.md#9163-event-pattern-matching)) against the event type, case-sensitively: `*` matches any run of characters, `?` exactly one, and every other character is literal. A pattern is never rejected — one that matches nothing delivers nothing.

=== "Python"
    ```python
    from apcore import APCore
    from apcore.config import Config
    from apcore.events import ApCoreEvent

    client = APCore(config=Config.load("apcore.yaml"))

    class RegistryWatcher:
        event_pattern = "apcore.registry.*"
        subscriber_id = "registry-watcher"

        async def on_event(self, event: ApCoreEvent) -> None:
            print(f"registry: {event.event_type} {event.module_id}")

    if client.events is not None:
        client.events.subscribe(RegistryWatcher())
    ```
=== "TypeScript"
    ```typescript
    import { APCore, Config, type EventSubscriber } from "apcore-js";

    const client = new APCore({ config: Config.load("apcore.yaml") });

    const registryWatcher: EventSubscriber = {
        subscriberId: "registry-watcher",
        eventPattern: "apcore.registry.*",
        onEvent: (event) => {
            console.log(`registry: ${event.eventType} ${event.moduleId}`);
        },
    };

    client.events?.subscribe(registryWatcher);
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::events::{ApCoreEvent, EventSubscriber};
    use apcore::APCore;
    use async_trait::async_trait;

    #[derive(Debug)]
    struct RegistryWatcher;

    #[async_trait]
    impl EventSubscriber for RegistryWatcher {
        fn subscriber_id(&self) -> &str { "registry-watcher" }
        fn event_pattern(&self) -> &str { "apcore.registry.*" }

        async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
            println!("registry: {} {:?}", event.event_type, event.module_id);
            Ok(())
        }
    }

    fn main() -> Result<(), ModuleError> {
        let client = APCore::from_path("apcore.yaml")?;
        if let Some(events) = client.events() {
            let _handle = events.subscribe(Box::new(RegistryWatcher));
        }
        Ok(())
    }
    ```

A subscriber whose pattern is `"*"` receives every event **except** `apcore.event.delivery_failed`, which is delivered only to subscribers with a non-wildcard pattern that matches it — so a failing catch-all subscriber cannot recurse on reports about itself.

### Built-in Subscribers

| Type | Delivers to | Config fields (besides `id`, `retry`) |
|------|-------------|---------------------------------------|
| `webhook` | HTTP POST of the event JSON | `url`, `headers`, `timeout_ms` (default 5000); `retry_count` (retries after the first attempt, i.e. `max_attempts = retry_count + 1`; ignored when `retry` is present) |
| `a2a` | HTTP POST of `{"skillId": …, "event": {…}}` | `platform_url`, `auth` (string → `Authorization: Bearer <auth>`, map → merged into headers), `skill_id` (default `"apevo.event_receiver"`), `timeout_ms` |
| `file` | Appends to a local file | `path`, `append` (default `true`), `format` (`json` default / `text`), `rotate_bytes` |
| `stdout` | Standard output | `format` (`text` default / `json`), `level_filter` |
| `filter` | Another subscriber, for matching events only | `delegate_type`, `delegate_config`, `include_events`, `exclude_events` |

- **HTTP delivery (`webhook`, `a2a`).** Sends `Content-Type: application/json`. 2xx is success. 5xx, connection errors and timeouts raise inside the subscriber, so the emitter's retry policy applies. 4xx is logged as a permanent failure and is not retried. Python needs the `events` extra (`pip install apcore[events]`, which provides `aiohttp`) — without it every delivery fails with `ImportError` and takes the retry/dead-letter path; Rust needs the `events` cargo feature.
- **`filter`.** When `include_events` is present it is decisive: an event is forwarded if it matches any entry and discarded otherwise, and `exclude_events` is not consulted. Otherwise an event matching any `exclude_events` entry is discarded. Both lists use A25. Because a pattern that fails to match means the event is *delivered*, `exclude_events` fails open — keep its patterns simple and test them.
- **`id`.** Every subscriber has a stable `subscriber_id`; when the config omits `id`, the SDK generates `"<type>-<n>"`. It appears in dead-letter and circuit events.

Constructors of the two HTTP subscribers:

=== "Python"

    ```python
    from apcore.events import A2ASubscriber, EventRetryConfig, WebhookSubscriber

    webhook = WebhookSubscriber(
        "https://example.com/hook",
        headers={"X-Source": "apcore"},
        timeout_ms=5000,
        id="audit-hook",
        retry=EventRetryConfig(max_attempts=5),
        event_pattern="apcore.*",
    )

    a2a = A2ASubscriber(
        "https://agent.example.com",
        auth="bearer-token-123",
        timeout_ms=5000,
        skill_id="myapp.event_receiver",
        id="agent-bridge",
    )
    ```

=== "TypeScript"

    ```typescript
    import { A2ASubscriber, WebhookSubscriber } from "apcore-js";

    const webhook = new WebhookSubscriber(
        "https://example.com/hook",
        { "X-Source": "apcore" },
        { timeoutMs: 5000, id: "audit-hook", retry: { maxAttempts: 5 } },
    );

    const a2a = new A2ASubscriber(
        "https://agent.example.com",
        "bearer-token-123",
        { timeoutMs: 5000, id: "agent-bridge", skillId: "myapp.event_receiver" },
    );
    ```

=== "Rust"

    ```rust
    use apcore::events::subscribers::{A2AAuth, A2ASubscriber};
    use apcore::events::{EventRetryConfig, WebhookSubscriber};

    fn main() {
        let mut webhook = WebhookSubscriber::new("audit-hook", "https://example.com/hook", "apcore.*")
            .with_retry(EventRetryConfig { max_attempts: 5, ..Default::default() });
        webhook.headers.insert("X-Source".to_string(), "apcore".to_string());

        let mut a2a = A2ASubscriber::new("agent-bridge", "https://agent.example.com", "*")
            .with_skill_id("myapp.event_receiver");
        a2a.auth = Some(A2AAuth::Bearer("bearer-token-123".to_string()));
    }
    ```

### Subscriber Factories

Config-driven subscribers are built by factories keyed by `type`. The five built-in types are registered automatically; add your own with `register_subscriber_type` — the factory is called once per configured entry and receives that entry's config object:

=== "Python"
    ```python
    from apcore.events import ApCoreEvent
    from apcore.sys_modules.registration import register_subscriber_type


    class SlackSubscriber:
        subscriber_type = "slack"

        def __init__(self, webhook_url: str, channel: str = "#general") -> None:
            self.webhook_url = webhook_url
            self.channel = channel

        async def on_event(self, event: ApCoreEvent) -> None:
            print(f"post {event.event_type} to {self.channel}")


    register_subscriber_type(
        "slack",
        lambda config: SlackSubscriber(config["webhook_url"], config.get("channel", "#general")),
    )
    ```
=== "TypeScript"
    ```typescript
    import { registerSubscriberType, type ApCoreEvent, type EventSubscriber } from "apcore-js";

    class SlackSubscriber implements EventSubscriber {
        readonly subscriberType = "slack";

        constructor(
            private readonly webhookUrl: string,
            private readonly channel: string,
        ) {}

        async onEvent(event: ApCoreEvent): Promise<void> {
            console.log(`post ${event.eventType} to ${this.channel} via ${this.webhookUrl}`);
        }
    }

    registerSubscriberType("slack", (config) =>
        new SlackSubscriber(String(config.webhook_url), String(config.channel ?? "#general")),
    );
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::events::{register_subscriber_type, ApCoreEvent, EventSubscriber};
    use async_trait::async_trait;
    use serde_json::Value;

    #[derive(Debug)]
    struct SlackSubscriber {
        webhook_url: String,
        channel: String,
    }

    #[async_trait]
    impl EventSubscriber for SlackSubscriber {
        fn subscriber_type(&self) -> &str { "slack" }

        async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
            println!("post {} to {} via {}", event.event_type, self.channel, self.webhook_url);
            Ok(())
        }
    }

    fn main() {
        register_subscriber_type(
            "slack",
            Box::new(|config: &Value| {
                let subscriber = SlackSubscriber {
                    webhook_url: config["webhook_url"].as_str().unwrap_or_default().to_string(),
                    channel: config["channel"].as_str().unwrap_or("#general").to_string(),
                };
                Ok(Box::new(subscriber) as Box<dyn EventSubscriber>)
            }),
        );
    }
    ```

The registry also offers `unregister_subscriber_type(type_name)`, `reset_subscriber_registry()` (back to the built-ins) and a function that builds one subscriber from a config object (`create_subscriber_from_config` / `createSubscriberFromConfig` / `create_subscriber`). Python additionally re-exports the registration function as `apcore.events.register_subscriber_factory`.

### Configuration

```yaml
sys_modules:
  enabled: true
  events:
    enabled: true
    thresholds:
      error_rate: 0.1              # apcore.health.error_threshold_exceeded at >= 10%
      latency_p99_ms: 5000.0       # apcore.health.latency_threshold_exceeded at p99 >= 5 s
    subscribers:
      - type: "webhook"
        id: "platform-hook"
        url: "https://platform.example.com/events"
        headers:
          Authorization: "Bearer ${PLATFORM_TOKEN}"
        timeout_ms: 5000
        retry:
          max_attempts: 5
          initial_backoff_ms: 250

      - type: "file"
        path: "/var/log/apcore/events.jsonl"
        format: "json"
        rotate_bytes: 10485760     # 10 MB

      - type: "filter"
        id: "health-pager"
        delegate_type: "a2a"
        delegate_config:
          platform_url: "https://agent.example.com"
          auth: "bearer-token-123"
          skill_id: "myapp.event_receiver"
        include_events:
          - "apcore.health.*"

      - type: "slack"              # a type added with register_subscriber_type
        webhook_url: "https://hooks.slack.com/services/..."
        channel: "#alerts"
```

## Event Naming Convention

Event types use dot-namespaced names; the prefix identifies the owner ([§9.16.1](../spec/protocol-spec.md#9161-naming-convention)). The `apcore.*` prefix is reserved for the core framework — ecosystem packages use their own (`apcore-mcp.*`, `apcore-a2a.*`, …) and applications any other prefix (`billing.invoice_generated`).

Framework events have the form `apcore.<subsystem>.<event>`:

- `<subsystem>` is the emitting subsystem: `registry`, `module`, `config`, `health`, `approval`, `policy`, `acl`, `stream`, `circuit`, `subscriber`, `event`.
- `<event>` is a `snake_case` description of the state transition (`module_registered`, `error_threshold_exceeded`, `updated`).

### Deprecation: legacy event names

Implementations emit only the canonical names. These earlier names are not emitted; subscribe to the canonical name instead:

| Legacy name | Canonical name |
|-------------|----------------|
| `module_registered` | `apcore.registry.module_registered` |
| `module_unregistered` | `apcore.registry.module_unregistered` |
| `apcore.error.threshold_exceeded` | `apcore.health.error_threshold_exceeded` |
| `apcore.latency.threshold_exceeded` | `apcore.health.latency_threshold_exceeded` |
| `module_health_changed` | `apcore.module.toggled` / `apcore.health.recovered` |
| `config_changed` | `apcore.config.updated` / `apcore.module.reloaded` |

## Event Types

Events the framework emits. The envelope's `module_id` carries the associated module; the table lists the keys of `data`.

| Event Type | Severity | Emitted by | `data` keys |
|------------|----------|------------|-------------|
| `apcore.registry.module_registered` | info | Registry bridge (event-enabled system modules) | — (module on the envelope) |
| `apcore.registry.module_unregistered` | info | Registry bridge | — (module on the envelope) |
| `apcore.registry.module_load_failed` | error | Registry | `module_id`, `callback_name`, `error_type`, `error_message` |
| `apcore.module.toggled` | info | `system.control.toggle_feature` | `module_id`, `enabled`, `caller_id`, `identity` (when present) |
| `apcore.module.reloaded` | info | `system.control.reload_module` | `module_id`, `previous_version`, `new_version`, `caller_id`, `identity` (when present) |
| `apcore.config.updated` | info | `system.control.update_config` | `key`, `old_value`, `new_value` (sensitive keys redacted), `caller_id`, `identity` (when present) |
| `apcore.health.error_threshold_exceeded` | error | `PlatformNotifyMiddleware` | `error_rate`, `threshold` |
| `apcore.health.latency_threshold_exceeded` | warn | `PlatformNotifyMiddleware` | `p99_latency_ms`, `threshold` |
| `apcore.health.recovered` | info | `PlatformNotifyMiddleware` (error rate back below half the threshold) | `status`, `error_rate` |
| `apcore.approval.decision` | info (approved/pending) / warn (rejected/timeout) | Approval gate (§7) | `module_id`, `status`, `approved_by`, `reason`, `approval_id`, `trace_id` |
| `apcore.policy.override` | info | Approval gate, when an `ExecutionPolicy` rule applies (§7.9) | `module_id`, `pattern`, `requires_approval`, `destructive`, `needs_approval`, `reason`, `trace_id` |
| `apcore.acl.denied` | warn | ACL check (§6) | `module_id`, `caller_id`, `reason`, `trace_id` |
| `apcore.acl.audit` | the ACL file's `audit.log_level` (default info) | ACL check — the default audit sink (§6.3.2) | the `AuditEntry` fields (§6.3.1) |
| `apcore.stream.post_validation_failed` | error | Executor (after a stream's last chunk) | `error_type`, `message`, `trace_id` |
| `apcore.circuit.opened` | warn | `CircuitBreakerMiddleware` | `module_id`, `caller_id`, `error_rate` |
| `apcore.circuit.closed` | info | `CircuitBreakerMiddleware` | `module_id`, `caller_id`, `error_rate` |
| `apcore.subscriber.circuit_opened` | warn | `CircuitBreakerWrapper` | `subscriber_type`, `consecutive_failures` |
| `apcore.subscriber.circuit_closed` | info | `CircuitBreakerWrapper` | `subscriber_type`, `recovery_attempt` |
| `apcore.event.delivery_failed` | error | Event bus (dead-letter path) | see [Dead-Letter Event](#dead-letter-event-apcoreeventdelivery_failed) |

- **Governance events** (`apcore.approval.decision`, `apcore.policy.override`, `apcore.acl.denied`) make the ACL → policy → approval chain observable. They are emitted only when an emitter is attached to the Executor, never influence the call's outcome, are not emitted for a gate that did not engage, and `apcore.acl.denied` is not emitted during a `validate()` preflight. See [§7.9](../spec/protocol-spec.md#79-execution-policy-v190-76).
- The `apcore.subscriber.*` payloads above are what all three SDKs emit; §9.16.2 also lists `subscriber_id`, which no SDK currently includes.
- apcore-rust additionally emits `apcore.config.reloaded` when `system.control.reload_module` re-reads the configuration from disk.

## Delivery Semantics

### Per-Subscriber Retry Policy

Every subscriber — built-in or custom — has a `retry` policy; the field names, defaults and backoff formula are the same in all SDKs:

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `max_attempts` | int ≥ 1 | `3` | Total attempts including the first. `1` disables retry. |
| `initial_backoff_ms` | int ≥ 0 | `100` | Delay before the first retry. |
| `max_backoff_ms` | int ≥ `initial_backoff_ms` | `30000` | Upper bound on a single delay. |
| `backoff_multiplier` | float ≥ 1.0 | `2.0` | Growth factor per retry. |

```text
delay_ms(attempt) = min(max_backoff_ms, initial_backoff_ms * backoff_multiplier ** attempt)
    attempt is zero-based: attempt = 0 is the first retry after the initial try

Defaults:  try → (100 ms) → retry 1 → (200 ms) → retry 2 → dead-letter event
```

Set it in configuration with a `retry:` block on the subscriber entry, or in code on the subscriber (`retry = EventRetryConfig(...)` or a plain mapping in Python, `retry: { maxAttempts, … }` in TypeScript, `fn retry(&self) -> EventRetryConfig` in Rust). The webhook's `retry_count` shorthand is described under [Built-in Subscribers](#built-in-subscribers).

### Dead-Letter Event (`apcore.event.delivery_failed`)

When a subscriber's attempts are exhausted, the emitter emits `apcore.event.delivery_failed` (severity `error`, no `module_id`) with this `data`:

```json
{
  "subscriber_type": "a2a",
  "subscriber_id": "agent-bridge",
  "original_event": {
    "name": "apcore.health.error_threshold_exceeded",
    "payload": { "error_rate": 0.42, "threshold": 0.1 },
    "metadata": {
      "module_id": "billing.charge",
      "timestamp": "2026-05-19T10:14:22.301Z"
    }
  },
  "error": {
    "type": "RuntimeError",
    "message": "A2A delivery to https://agent.example.com failed with status 503"
  },
  "attempt_count": 3,
  "timestamp": "2026-05-19T10:14:28.812Z"
}
```

- `subscriber_type` is the subscriber's declared type (`webhook`, `a2a`, …); the circuit events report the same value (D-116).
- The dead-letter event is delivered once, without retry, and only to subscribers whose non-wildcard `event_pattern` matches it. A dead-letter subscriber that fails is logged and discarded — there is no second-order dead letter.
- It is emitted through the same emitter as the original event, so any subscriber can opt in by name — for example a `filter` around a `file` subscriber with `include_events: ["apcore.event.delivery_failed"]`.
- TypeScript's emitter bounds its pending deliveries (`maxPending`, default 1000); a delivery that overflows the buffer is failed through this path with an extra `"reason": "pending_overflow"` field rather than dropped.

### `on_failure` Callback

A subscriber may also define `on_failure(event, error, attempt_count)`. It is invoked once per exhausted delivery, after the dead-letter event, with the same information. An exception it raises is logged. It is independent of any dead-letter subscriber — both fire.

=== "Python"
    ```python
    from apcore.events import ApCoreEvent, EventEmitter


    class HealthAlertSubscriber:
        event_pattern = "apcore.health.*"
        retry = {"max_attempts": 5, "initial_backoff_ms": 250}

        async def on_event(self, event: ApCoreEvent) -> None:
            print(f"forwarding {event.event_type}")

        async def on_failure(self, event: ApCoreEvent, error: Exception, attempt_count: int) -> None:
            print(f"gave up on {event.event_type} after {attempt_count} attempts: {error}")


    emitter = EventEmitter()
    emitter.subscribe(HealthAlertSubscriber())
    ```
=== "TypeScript"
    ```typescript
    import { EventEmitter, type ApCoreEvent, type EventSubscriber } from "apcore-js";

    class HealthAlertSubscriber implements EventSubscriber {
        readonly eventPattern = "apcore.health.*";
        readonly retry = { maxAttempts: 5, initialBackoffMs: 250 };

        async onEvent(event: ApCoreEvent): Promise<void> {
            console.log(`forwarding ${event.eventType}`);
        }

        async onFailure(event: ApCoreEvent, error: Error, attemptCount: number): Promise<void> {
            console.log(`gave up on ${event.eventType} after ${attemptCount} attempts: ${error.message}`);
        }
    }

    const emitter = new EventEmitter();
    emitter.subscribe(new HealthAlertSubscriber());
    ```
=== "Rust"
    ```rust
    use apcore::errors::ModuleError;
    use apcore::events::{ApCoreEvent, EventEmitter, EventRetryConfig, EventSubscriber};
    use async_trait::async_trait;

    #[derive(Debug)]
    struct HealthAlertSubscriber;

    #[async_trait]
    impl EventSubscriber for HealthAlertSubscriber {
        fn subscriber_id(&self) -> &str { "health-alert" }
        fn event_pattern(&self) -> &str { "apcore.health.*" }

        fn retry(&self) -> EventRetryConfig {
            EventRetryConfig { max_attempts: 5, initial_backoff_ms: 250, ..Default::default() }
        }

        async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
            println!("forwarding {}", event.event_type);
            Ok(())
        }

        async fn on_failure(&self, event: &ApCoreEvent, error: &ModuleError, attempt_count: u32) {
            println!("gave up on {} after {attempt_count} attempts: {error}", event.event_type);
        }
    }

    fn main() {
        let emitter = EventEmitter::new();
        let _handle = emitter.subscribe(Box::new(HealthAlertSubscriber));
    }
    ```

### Subscriber Circuit Breaker

`CircuitBreakerWrapper` wraps a subscriber so that a degraded destination cannot keep consuming delivery work:

```text
CLOSED    → (consecutive_failures >= open_threshold) → OPEN
OPEN      → (recovery_window_ms elapsed since the last failure) → HALF_OPEN
HALF_OPEN → (delivery succeeds) → CLOSED
HALF_OPEN → (delivery fails)    → OPEN
```

- Defaults: `timeout_ms` 5000 (a delivery running longer counts as a failure), `open_threshold` 5, `recovery_window_ms` 60 000.
- While `OPEN`, the wrapped subscriber is not called and its events are discarded.
- Transitions emit `apcore.subscriber.circuit_opened` (with a WARN log) and `apcore.subscriber.circuit_closed`.

Python wraps every subscriber created from `sys_modules.events.subscribers` in a `CircuitBreakerWrapper`, tuned by an optional per-entry `circuit_breaker:` block (`timeout_ms`, `open_threshold`, `recovery_window_ms`). TypeScript and Rust export `CircuitBreakerWrapper` for explicit use and do not wrap configured subscribers automatically.

## Dependencies

- `Middleware` — base of `PlatformNotifyMiddleware`, which emits the `apcore.health.*` events.
- `MetricsCollector` — read by `PlatformNotifyMiddleware` for its threshold checks.

??? info "Python SDK reference"
    Not a protocol requirement — the Python SDK's source layout for users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `src/apcore/events/emitter.py` | `EventEmitter`, `ApCoreEvent`, `EventSubscriber`, retry and dead-letter delivery |
    | `src/apcore/events/subscribers.py` | `WebhookSubscriber`, `A2ASubscriber`, `FileSubscriber`, `StdoutSubscriber`, `FilterSubscriber` |
    | `src/apcore/events/circuit_breaker.py` | `CircuitBreakerWrapper` |
    | `src/apcore/events/retry.py` | `EventRetryConfig` |
    | `src/apcore/sys_modules/registration.py` | Subscriber factory registry, config-driven subscribers |
    | `src/apcore/middleware/platform_notify.py` | `PlatformNotifyMiddleware` |
    | `src/apcore/client.py` | `APCore.on()`, `APCore.off()` |

    `aiohttp` (optional, `pip install apcore[events]`) is required for the HTTP subscribers.

## Testing Strategy

- **EventEmitter**: subscribe/emit/unsubscribe, snapshot isolation, subscriber error isolation, `flush` and `shutdown`.
- **Matching**: exact matching for `client.on()`, A25 patterns for `event_pattern`, dead-letter delivery only to non-wildcard subscribers.
- **Retry and dead letter**: attempt counts and delays, dead-letter payload shape, `on_failure` invocation.
- **HTTP subscribers**: 2xx/4xx/5xx handling, timeouts, header merging, A2A auth modes and payload.
- **Factories**: custom type registration, config instantiation, reset.
- **PlatformNotifyMiddleware**: threshold crossing, recovery at half the threshold.

## Contract: EventEmitter.emit

### Inputs
- `event` (`ApCoreEvent`, required) — the event to deliver; Rust takes `&ApCoreEvent` (`emit_spawn` takes it by value). The caller SHOULD supply a non-empty `event_type`; `emit()` does not validate it.

### Returns
- Nothing.

### Overflow
- An event accepted by `emit()` **MUST** eventually be delivered to each matching subscriber or routed through the dead-letter path; it **MUST NOT** be silently discarded.
- An implementation **MAY** bound its pending-delivery buffer. On overflow it **MUST** either apply backpressure or fail the delivery through the dead-letter path with `reason: "pending_overflow"` (TypeScript does the latter).
- Events for a subscriber whose circuit is `OPEN` are discarded by design; that is not overflow.

### Errors
- None raised to the caller. Subscriber errors are retried, then reported through the dead-letter path and `on_failure`.

### Properties
- async: synchronous in Python and TypeScript; `async fn` in Rust, whose body spawns the deliveries and does not wait on subscribers. In all three, `emit()` returns without waiting for subscribers and never raises.
- thread_safe: true
- pure: false
- idempotent: false

## Contract: EventEmitter.subscribe

### Inputs
- `subscriber` (`EventSubscriber`, required) — an object carrying its own `on_event` and optional members (`event_pattern`, `subscriber_id`, `retry`, …). There is no `subscribe(event_type, handler)` form on the emitter; `APCore.on()` provides the callback convenience (D10-016).

### Errors
- None in the cross-language contract. Python raises `TypeError` when `on_event` is not a coroutine function — the runtime equivalent of the static check TypeScript and Rust perform (D10-002).

### Returns
- Python/TypeScript: nothing — remove the subscription by passing the same object to `unsubscribe`. Rust: a `SubscriberHandle` for `unsubscribe_handle`.

### Properties
- async: false
- thread_safe: true
- idempotent: false (each call adds a subscription)

## Contract: EventEmitter.unsubscribe

### Inputs
- `subscriber` — the object passed to `subscribe` (Python and TypeScript compare by identity). Rust's `subscribe` consumes the `Box`, so Rust removes by `SubscriberHandle` (`unsubscribe_handle`) or by `subscriber_id` (`unsubscribe_by_id`; `unsubscribe(&dyn EventSubscriber)` also matches by ID and warns when several subscribers share it).

### Errors
- None. Removing an absent subscriber is a no-op.

### Returns
- Python/TypeScript: nothing. Rust: `bool` — whether a subscription was removed.

### Properties
- async: false
- thread_safe: true
- pure: false
- idempotent: true

## Contract: EventEmitter.flush

### Inputs
- `timeout` — maximum wait for in-flight deliveries: Python `timeout` in **seconds** (default 5.0), TypeScript `timeoutMs` and Rust `timeout_ms` in **milliseconds** (default 5000; Rust `flush_default()`). Pass a positive value; `0` is not portable.

### Errors
- None raised to the caller. If the timeout elapses, `flush` returns and the remaining deliveries continue in the background. Python applies the timeout per pending batch, so the worst-case wait is longer than `timeout`.

### Returns
- Nothing (Rust `Result<(), ModuleError>`, always `Ok`).

### Properties
- async: blocking in Python; `async` in TypeScript and Rust
- thread_safe: true
- pure: false
- idempotent: true

## Contract: WebhookSubscriber.on_event

### Inputs
- `event` (`ApCoreEvent`, required) — POSTed as JSON to the configured URL with the configured headers.

### Errors
- Raises (Rust: returns `Err`) on 5xx responses, connection errors and timeouts, so the emitter retries per the subscriber's `retry` policy and emits `apcore.event.delivery_failed` when attempts are exhausted. Nothing reaches the code that called `emit()`.
- A 4xx response is logged as a permanent failure and not retried.

### Returns
- Nothing on 2xx.

### Properties
- async: true
- thread_safe: true
- pure: false (outbound HTTP)

## Contract: A2ASubscriber.on_event

### Inputs
- `event` (`ApCoreEvent`, required) — sent as `{"skillId": <skill_id>, "event": {<event fields>}}` to `platform_url`. A string `auth` becomes `Authorization: Bearer <auth>`; a map is merged into the headers.

### Errors
- As for `WebhookSubscriber.on_event`: 5xx, connection errors and timeouts go through the retry policy and the dead-letter path; 4xx is logged and not retried.

### Returns
- Nothing on 2xx.

### Properties
- async: true
- thread_safe: true
- pure: false (outbound HTTP)

## Contract: CircuitBreakerWrapper._on_failure

Internal in every SDK (Python `_on_failure`, TypeScript `_onFailure`, Rust private `on_failure`); specified because all three must behave the same.

### Inputs
- `error` (Exception / Error / `&str`, required) — the delivery error that just occurred. The wrapper is per-subscriber, so no subscriber ID is passed.

### Errors
- None — it records state and never raises.

### Returns
- The lifecycle event to emit when the call changed the circuit's state (`apcore.subscriber.circuit_opened`), or the language's empty value when it did not: `ApCoreEvent | None` (Python), `ApCoreEvent | null` (TypeScript), `Option<ApCoreEvent>` (Rust). The current state is read through the wrapper's own accessor.

### Properties
- async: false
- thread_safe: true (state is lock-protected)
- pure: false (mutates circuit state)
- idempotent: false
