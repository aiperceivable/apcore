//! Tests for the events subsystem — EventEmitter, subscribers, and factory.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;

use apcore::errors::{ErrorCode, ModuleError};
use apcore::events::emitter::{ApCoreEvent, EventEmitter};
use apcore::events::subscribers::{
    create_subscriber, register_subscriber_type, reset_subscriber_registry,
    unregister_subscriber_type, A2ASubscriber, EventSubscriber, WebhookSubscriber,
};

// ---------------------------------------------------------------------------
// Test subscriber — records received events for assertions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RecordingSubscriber {
    id: String,
    pattern: String,
    received: Arc<Mutex<Vec<String>>>,
}

impl RecordingSubscriber {
    fn new(id: &str, pattern: &str) -> Self {
        Self {
            id: id.to_string(),
            pattern: pattern.to_string(),
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[allow(dead_code)] // useful for debugging test failures
    fn received_events(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventSubscriber for RecordingSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }

    fn event_pattern(&self) -> &str {
        &self.pattern
    }

    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.received.lock().unwrap().push(event.event_type.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Failing subscriber — always returns an error
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct FailingSubscriber {
    id: String,
    pattern: String,
}

#[async_trait]
impl EventSubscriber for FailingSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }

    fn event_pattern(&self) -> &str {
        &self.pattern
    }

    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        Err(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "deliberate failure",
        ))
    }
}

// ---------------------------------------------------------------------------
// ApCoreEvent construction tests
// ---------------------------------------------------------------------------

#[test]
fn test_event_new_sets_defaults() {
    let event = ApCoreEvent::new("module.loaded", json!({"key": "value"}));
    assert_eq!(event.event_type, "module.loaded");
    assert_eq!(event.severity, "info");
    assert!(event.module_id.is_none());
    assert!(!event.timestamp.is_empty());
}

#[test]
fn test_event_with_module() {
    let event = ApCoreEvent::with_module(
        "module.error",
        json!({"reason": "timeout"}),
        "executor.email.send",
        "error",
    );
    assert_eq!(event.event_type, "module.error");
    assert_eq!(event.module_id.as_deref(), Some("executor.email.send"));
    assert_eq!(event.severity, "error");
}

#[test]
fn test_event_serialization_roundtrip() {
    let event = ApCoreEvent::with_module("test.event", json!(42), "mod_a", "warning");
    let json_str = serde_json::to_string(&event).expect("serialize");
    let restored: ApCoreEvent = serde_json::from_str(&json_str).expect("deserialize");
    assert_eq!(restored.event_type, "test.event");
    assert_eq!(restored.severity, "warning");
    assert_eq!(restored.module_id.as_deref(), Some("mod_a"));
}

#[test]
fn test_event_module_id_omitted_when_none() {
    let event = ApCoreEvent::new("x", json!(null));
    let v = serde_json::to_value(&event).expect("serialize");
    assert!(v.get("module_id").is_none());
}

// ---------------------------------------------------------------------------
// EventEmitter — subscribe / emit / unsubscribe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_emitter_subscribe_and_emit() {
    let sub = RecordingSubscriber::new("sub-1", "*");
    let received = sub.received.clone();

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    let event = ApCoreEvent::new("test.ping", json!({}));
    emitter.emit(&event).await;
    emitter.flush(5000).await.unwrap();

    let events = received.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], "test.ping");
}

#[tokio::test]
async fn test_emitter_pattern_filtering() {
    let sub_all = RecordingSubscriber::new("all", "*");
    let sub_mod = RecordingSubscriber::new("mod", "module.*");
    let sub_exact = RecordingSubscriber::new("exact", "module.loaded");

    let recv_all = sub_all.received.clone();
    let recv_mod = sub_mod.received.clone();
    let recv_exact = sub_exact.received.clone();

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub_all));
    emitter.subscribe(Box::new(sub_mod));
    emitter.subscribe(Box::new(sub_exact));

    emitter
        .emit(&ApCoreEvent::new("module.loaded", json!({})))
        .await;
    emitter
        .emit(&ApCoreEvent::new("module.error", json!({})))
        .await;
    emitter
        .emit(&ApCoreEvent::new("registry.changed", json!({})))
        .await;
    emitter.flush(5000).await.unwrap();

    assert_eq!(recv_all.lock().unwrap().len(), 3);
    assert_eq!(recv_mod.lock().unwrap().len(), 2);
    assert_eq!(recv_exact.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_emitter_unsubscribe_by_id() {
    let sub = RecordingSubscriber::new("remove-me", "*");
    let received = sub.received.clone();

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub.clone()));

    assert!(emitter.unsubscribe_by_id("remove-me"));
    // Second call should return false (already removed).
    assert!(!emitter.unsubscribe_by_id("remove-me"));

    emitter
        .emit(&ApCoreEvent::new("post.remove", json!({})))
        .await;

    assert!(received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_emitter_unsubscribe_via_trait() {
    let sub = RecordingSubscriber::new("trait-unsub", "*");
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub.clone()));

    assert!(emitter.unsubscribe(&sub));
    assert!(!emitter.unsubscribe(&sub));
}

#[tokio::test]
async fn test_emitter_unsubscribe_nonexistent_returns_false() {
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("ghost", "*");
    // `unsubscribe` is `&self` (interior-mutable subscribers, D1-011).
    assert!(!emitter.unsubscribe(&sub));
}

#[tokio::test]
async fn test_emitter_error_isolation() {
    // A failing subscriber must not prevent other subscribers from receiving events.
    let good = RecordingSubscriber::new("good", "*");
    let received = good.received.clone();

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(FailingSubscriber {
        id: "bad".into(),
        pattern: "*".into(),
    }));
    emitter.subscribe(Box::new(good));

    // emit_sequential() is the deterministic single-attempt path; error
    // isolation is the focus of this test. The retry+DLQ behaviour of the
    // canonical `emit()` path is covered by
    // tests/test_v022_event_delivery_semantics.rs.
    emitter
        .emit_sequential(&ApCoreEvent::new("err.test", json!({})))
        .await;
    assert_eq!(received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_emitter_no_subscribers_is_ok() {
    let emitter = EventEmitter::new();
    emitter
        .emit(&ApCoreEvent::new("lonely.event", json!({})))
        .await;
    // No subscribers means a silent no-op — emit returns unit (D10-008).
}

// ---------------------------------------------------------------------------
// D10-008: EventEmitter::emit returns unit (no Result wrapper)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_emit_returns_unit_no_result_wrapper() {
    // Spec event-system.md:448 declares "No errors raised" — the body is
    // infallible (subscriber errors are caught and logged internally).
    // Pinning the return type via `let _: () = ...` proves the wrapper
    // was dropped (D10-008).
    let emitter = EventEmitter::new();
    let event = ApCoreEvent::new("anything", serde_json::json!({}));
    let _: () = emitter.emit(&event).await;
}

#[tokio::test]
async fn test_emitter_emit_filtered() {
    let sub = RecordingSubscriber::new("s1", "module.*");
    let received = sub.received.clone();

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    // Caller filter "module.loaded" AND subscriber pattern "module.*" both match.
    emitter
        .emit_filtered(&ApCoreEvent::new("module.loaded", json!({})), "module.*")
        .await
        .unwrap();

    // Caller filter "registry.*" does NOT match "module.loaded", so subscriber skipped.
    emitter
        .emit_filtered(&ApCoreEvent::new("module.loaded", json!({})), "registry.*")
        .await
        .unwrap();

    assert_eq!(received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_emitter_flush_empty_succeeds() {
    let emitter = EventEmitter::new();
    let result = emitter.flush(1000).await;
    assert!(result.is_ok());
}

#[test]
fn test_emitter_default_max_workers() {
    let emitter = EventEmitter::new();
    assert_eq!(emitter.max_workers, 4);
}

#[test]
fn test_emitter_default_trait() {
    let emitter = EventEmitter::default();
    assert_eq!(emitter.max_workers, 4);
}

// ---------------------------------------------------------------------------
// Pattern matching edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_pattern_wildcard_matches_everything() {
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.received.clone();
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    emitter
        .emit(&ApCoreEvent::new("anything.at.all", json!({})))
        .await;
    emitter.flush(5000).await.unwrap();
    assert_eq!(received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_pattern_exact_match() {
    let sub = RecordingSubscriber::new("s", "exact.match");
    let received = sub.received.clone();
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    emitter
        .emit(&ApCoreEvent::new("exact.match", json!({})))
        .await;
    emitter
        .emit(&ApCoreEvent::new("exact.match.extra", json!({})))
        .await;
    emitter.flush(5000).await.unwrap();

    let events = received.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0], "exact.match");
}

#[tokio::test]
async fn test_pattern_prefix_wildcard() {
    let sub = RecordingSubscriber::new("s", "foo.*");
    let received = sub.received.clone();
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    emitter.emit(&ApCoreEvent::new("foo.bar", json!({}))).await;
    emitter
        .emit(&ApCoreEvent::new("foo.baz.qux", json!({})))
        .await;
    emitter.emit(&ApCoreEvent::new("bar.foo", json!({}))).await;
    emitter.flush(5000).await.unwrap();

    let events = received.lock().unwrap();
    assert_eq!(events.len(), 2);
}

#[tokio::test]
async fn test_pattern_no_match() {
    let sub = RecordingSubscriber::new("s", "alpha.beta");
    let received = sub.received.clone();
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    emitter
        .emit(&ApCoreEvent::new("gamma.delta", json!({})))
        .await;

    assert!(received.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// WebhookSubscriber and A2ASubscriber unit tests (construction only)
// ---------------------------------------------------------------------------

#[test]
fn test_webhook_subscriber_defaults() {
    let ws = WebhookSubscriber::new("wh-1", "https://example.com/hook", "module.*");
    assert_eq!(ws.subscriber_id(), "wh-1");
    assert_eq!(ws.event_pattern(), "module.*");
    assert_eq!(ws.retry_count, 3);
    assert_eq!(ws.timeout_ms, 5000);
    assert!(ws.headers.is_empty());
}

#[test]
fn test_a2a_subscriber_defaults() {
    let a2a = A2ASubscriber::new("a2a-1", "https://platform.example.com", "*");
    assert_eq!(a2a.subscriber_id(), "a2a-1");
    assert_eq!(a2a.event_pattern(), "*");
    assert_eq!(a2a.timeout_ms, 5000);
    assert!(a2a.auth.is_none());
}

// ---------------------------------------------------------------------------
// Subscriber factory tests — single sequential test to avoid global state races
// ---------------------------------------------------------------------------

// ModuleError is a protocol-level domain type whose rich field set is spec-required;
// boxing individual fields would break ergonomics across the entire codebase.
#[allow(clippy::result_large_err)]
#[test]
fn test_subscriber_factory_operations() {
    reset_subscriber_registry();

    // Create webhook subscriber
    let config = json!({ "type": "webhook", "url": "https://example.com/hook" });
    let sub = create_subscriber(&config).expect("should create webhook subscriber");
    assert!(sub.subscriber_id().starts_with("webhook-"));
    assert_eq!(sub.event_pattern(), "*");

    // Create a2a subscriber
    let config = json!({ "type": "a2a", "platform_url": "https://platform.example.com" });
    let sub = create_subscriber(&config).expect("should create a2a subscriber");
    assert!(sub.subscriber_id().starts_with("a2a-"));

    // Missing type field
    let config = json!({ "url": "https://example.com" });
    let err = create_subscriber(&config).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid);

    // Unknown type
    let config = json!({ "type": "unknown" });
    let err = create_subscriber(&config).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid);

    // Webhook missing url
    let config = json!({ "type": "webhook" });
    let err = create_subscriber(&config).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid);

    // A2A missing platform_url
    let config = json!({ "type": "a2a" });
    let err = create_subscriber(&config).unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigInvalid);

    // Register custom type
    register_subscriber_type(
        "custom",
        Box::new(|config| {
            let id = config
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("custom-default")
                .to_string();
            Ok(Box::new(RecordingSubscriber::new(&id, "*")) as Box<dyn EventSubscriber>)
        }),
    );
    let config = json!({ "type": "custom", "id": "my-custom" });
    let sub = create_subscriber(&config).expect("should create custom subscriber");
    assert_eq!(sub.subscriber_id(), "my-custom");

    // Unregister webhook
    assert!(unregister_subscriber_type("webhook").is_ok());
    let config = json!({ "type": "webhook", "url": "https://example.com" });
    assert!(create_subscriber(&config).is_err());

    // Unregister nonexistent type
    let err = unregister_subscriber_type("nonexistent").unwrap_err();
    assert_eq!(err.code, ErrorCode::GeneralInternalError);

    // Reset restores built-ins
    reset_subscriber_registry();
    let config = json!({ "type": "webhook", "url": "https://restored.com" });
    assert!(create_subscriber(&config).is_ok());
}

// ---------------------------------------------------------------------------
// A-D-501: emit_spawn — fire-and-forget event dispatch
// ---------------------------------------------------------------------------

/// Subscriber that blocks until released by an external signal.
#[derive(Debug)]
struct BlockingSubscriber {
    id: String,
    barrier: Arc<tokio::sync::Notify>,
    started: Arc<std::sync::atomic::AtomicBool>,
    finished: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl EventSubscriber for BlockingSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    fn event_pattern(&self) -> &'static str {
        "*"
    }
    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.started
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.barrier.notified().await;
        self.finished
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn test_emit_spawn_returns_immediately() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let barrier = Arc::new(tokio::sync::Notify::new());
    let started = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));

    let sub = BlockingSubscriber {
        id: "block-1".into(),
        barrier: Arc::clone(&barrier),
        started: Arc::clone(&started),
        finished: Arc::clone(&finished),
    };

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    let event = ApCoreEvent::new("spawn.test", json!({}));

    let t0 = std::time::Instant::now();
    emitter.emit_spawn(event);
    let elapsed = t0.elapsed();

    // emit_spawn must not block on subscriber execution. Allow 250ms slack
    // for CI scheduling jitter — the actual subscriber will block ~forever
    // without the notify, so a non-spawn implementation would never return.
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "emit_spawn returned in {elapsed:?} — should be near-instant"
    );

    // Wait for subscriber to start running, then release it.
    for _ in 0..50 {
        if started.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        started.load(Ordering::SeqCst),
        "subscriber should have started"
    );
    barrier.notify_one();

    for _ in 0..50 {
        if finished.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        finished.load(Ordering::SeqCst),
        "subscriber should have finished"
    );
}

#[tokio::test]
async fn test_emit_spawn_dispatches_subscribers_concurrently() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Subscriber that sleeps a fixed duration, then increments a counter.
    #[derive(Debug)]
    struct SlowSub {
        id: String,
        sleep_ms: u64,
        counter: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EventSubscriber for SlowSub {
        fn subscriber_id(&self) -> &str {
            &self.id
        }
        fn event_pattern(&self) -> &'static str {
            "*"
        }
        async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
            tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
            self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let emitter = EventEmitter::new();
    for i in 0..3 {
        emitter.subscribe(Box::new(SlowSub {
            id: format!("slow-{i}"),
            sleep_ms: 100,
            counter: Arc::clone(&counter),
        }));
    }

    let event = ApCoreEvent::new("conc.test", json!({}));
    let t0 = std::time::Instant::now();
    emitter.emit_spawn(event);

    // Wait until all 3 subscribers complete.
    for _ in 0..50 {
        if counter.load(Ordering::SeqCst) == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let elapsed = t0.elapsed();
    assert_eq!(counter.load(Ordering::SeqCst), 3);
    // Concurrent execution: total wall-clock should be closer to 100ms than
    // 300ms (sequential). Allow 250ms ceiling for CI variance.
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "subscribers should run concurrently, took {elapsed:?}"
    );
}

#[tokio::test]
async fn test_emit_spawn_subscriber_error_does_not_propagate() {
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(FailingSubscriber {
        id: "fail-1".into(),
        pattern: "*".into(),
    }));
    let good = RecordingSubscriber::new("good-1", "*");
    let received = good.received.clone();
    emitter.subscribe(Box::new(good));

    let event = ApCoreEvent::new("err.iso", json!({}));
    emitter.emit_spawn(event);

    // Wait for the good subscriber to receive.
    for _ in 0..50 {
        if !received.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        received.lock().unwrap().as_slice(),
        &["err.iso".to_string()]
    );
}

// ---------------------------------------------------------------------------
// A-D-502: shutdown — idempotent flush + drop post-shutdown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_shutdown_is_idempotent() {
    let emitter = EventEmitter::new();
    emitter.shutdown(100).await.expect("first shutdown");
    // Second call must succeed without error.
    emitter
        .shutdown(100)
        .await
        .expect("second shutdown idempotent");
}

#[tokio::test]
async fn test_emit_spawn_after_shutdown_is_dropped() {
    let sub = RecordingSubscriber::new("post-shutdown", "*");
    let received = sub.received.clone();

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    emitter.shutdown(100).await.expect("shutdown");

    let event = ApCoreEvent::new("dropped.event", json!({}));
    emitter.emit_spawn(event);

    // Give any spawned task a chance to run (it must not).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        received.lock().unwrap().is_empty(),
        "events emitted after shutdown must be dropped"
    );
}

#[tokio::test]
async fn test_emit_async_after_shutdown_is_dropped() {
    let sub = RecordingSubscriber::new("post-shutdown-async", "*");
    let received = sub.received.clone();

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    emitter.shutdown(100).await.expect("shutdown");

    let event = ApCoreEvent::new("dropped.async", json!({}));
    emitter.emit(&event).await;

    assert!(
        received.lock().unwrap().is_empty(),
        "async emit() after shutdown must drop the event"
    );
}
