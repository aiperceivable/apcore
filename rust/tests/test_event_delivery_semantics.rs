// Issue #61 — Event delivery semantics: retry, DLQ, on_failure
// Tests emit_delivery_semantics() with retry configurations.

use apcore::errors::ModuleError;
use apcore::events::emitter::{ApCoreEvent, EventEmitter};
use apcore::events::retry::EventRetryConfig;
use apcore::events::subscribers::EventSubscriber;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Helper: subscriber that fails N times then succeeds
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CountingSubscriber {
    id: String,
    pattern: String,
    fail_count: u32,
    attempt_count: Arc<AtomicU32>,
    received: Arc<Mutex<Vec<String>>>,
    retry_config: EventRetryConfig,
}

impl CountingSubscriber {
    fn new(
        id: &str,
        fail_count: u32,
        retry_config: EventRetryConfig,
    ) -> (Self, Arc<AtomicU32>, Arc<Mutex<Vec<String>>>) {
        let attempt_count = Arc::new(AtomicU32::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));
        let sub = Self {
            id: id.to_string(),
            pattern: "*".to_string(),
            fail_count,
            attempt_count: Arc::clone(&attempt_count),
            received: Arc::clone(&received),
            retry_config,
        };
        (sub, attempt_count, received)
    }
}

#[async_trait]
impl EventSubscriber for CountingSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    fn event_pattern(&self) -> &str {
        &self.pattern
    }
    fn retry(&self) -> EventRetryConfig {
        self.retry_config
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        let attempt = self.attempt_count.fetch_add(1, Ordering::SeqCst);
        if attempt < self.fail_count {
            Err(ModuleError::new(
                apcore::errors::ErrorCode::GeneralInternalError,
                format!("intentional failure on attempt {attempt}"),
            ))
        } else {
            self.received.lock().push(event.event_type.clone());
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: subscriber that always fails (for DLQ tests)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct AlwaysFailSubscriber {
    id: String,
    attempt_count: Arc<AtomicU32>,
    failure_recorded: Arc<Mutex<Option<u32>>>,
    retry_config: EventRetryConfig,
}

impl AlwaysFailSubscriber {
    fn new(
        id: &str,
        retry_config: EventRetryConfig,
    ) -> (Self, Arc<AtomicU32>, Arc<Mutex<Option<u32>>>) {
        let attempt_count = Arc::new(AtomicU32::new(0));
        let failure_recorded = Arc::new(Mutex::new(None));
        let sub = Self {
            id: id.to_string(),
            attempt_count: Arc::clone(&attempt_count),
            failure_recorded: Arc::clone(&failure_recorded),
            retry_config,
        };
        (sub, attempt_count, failure_recorded)
    }
}

#[async_trait]
impl EventSubscriber for AlwaysFailSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    // Use a specific pattern to avoid receiving the DLQ event itself.
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        "test.*"
    }
    fn retry(&self) -> EventRetryConfig {
        self.retry_config
    }
    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.attempt_count.fetch_add(1, Ordering::SeqCst);
        Err(ModuleError::new(
            apcore::errors::ErrorCode::GeneralInternalError,
            "always fails",
        ))
    }
    async fn on_failure(&self, _event: &ApCoreEvent, _error: &ModuleError, attempt_count: u32) {
        *self.failure_recorded.lock() = Some(attempt_count);
    }
}

// ---------------------------------------------------------------------------
// Helper: DLQ subscriber that captures events
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct DlqSubscriber {
    id: String,
    received: Arc<Mutex<Vec<ApCoreEvent>>>,
}

impl DlqSubscriber {
    fn new(id: &str) -> (Self, Arc<Mutex<Vec<ApCoreEvent>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sub = Self {
            id: id.to_string(),
            received: Arc::clone(&received),
        };
        (sub, received)
    }
}

#[async_trait]
impl EventSubscriber for DlqSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        "apcore.event.delivery_failed"
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.received.lock().push(event.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retry_before_exhaustion_succeeds_on_third_attempt() {
    // fails twice, succeeds on 3rd — no DLQ emitted
    let retry_cfg = EventRetryConfig {
        max_attempts: 3,
        initial_backoff_ms: 1, // 1ms for fast tests
        max_backoff_ms: 10,
        backoff_multiplier: 2.0,
    };
    let (sub, attempt_count, received) = CountingSubscriber::new("sub-1", 2, retry_cfg);

    let (dlq_sub, dlq_received) = DlqSubscriber::new("dlq-1");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));
    emitter.subscribe(Box::new(dlq_sub));

    let event = ApCoreEvent::new("test.retry", json!({"v": 1}));
    emitter.emit_delivery_semantics(event);

    // Wait for async tasks to complete.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Exactly 3 attempts (2 failures + 1 success).
    assert_eq!(attempt_count.load(Ordering::SeqCst), 3);
    // Event delivered successfully.
    assert_eq!(received.lock().as_slice(), ["test.retry"]);
    // No DLQ emitted (delivery succeeded before exhaustion).
    assert!(dlq_received.lock().is_empty(), "no DLQ on success");
}

#[tokio::test]
async fn permanent_failure_emits_dlq_event() {
    // Always fails → DLQ emitted after exhausting max_attempts.
    let retry_cfg = EventRetryConfig {
        max_attempts: 3,
        initial_backoff_ms: 1,
        max_backoff_ms: 10,
        backoff_multiplier: 2.0,
    };
    let (sub, attempt_count, failure_recorded) = AlwaysFailSubscriber::new("sub-fail", retry_cfg);
    let (dlq_sub, dlq_received) = DlqSubscriber::new("dlq-2");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));
    emitter.subscribe(Box::new(dlq_sub));

    let event = ApCoreEvent::new("test.permanent_fail", json!({}));
    emitter.emit_delivery_semantics(event);

    tokio::time::sleep(Duration::from_millis(150)).await;

    // All 3 attempts made.
    assert_eq!(attempt_count.load(Ordering::SeqCst), 3);
    // DLQ received exactly 1 event.
    let dlq_events = dlq_received.lock();
    assert_eq!(dlq_events.len(), 1, "exactly one DLQ event");
    // DLQ payload has expected keys.
    let dlq_data = &dlq_events[0].data;
    assert!(
        dlq_data.get("subscriber_id").is_some(),
        "dlq has subscriber_id"
    );
    assert!(
        dlq_data.get("original_event").is_some(),
        "dlq has original_event"
    );
    assert!(dlq_data.get("error").is_some(), "dlq has error");
    assert!(
        dlq_data.get("attempt_count").is_some(),
        "dlq has attempt_count"
    );
    assert_eq!(
        dlq_data["attempt_count"].as_u64(),
        Some(3),
        "attempt_count matches max_attempts"
    );
    // on_failure was called with correct attempt count.
    assert_eq!(*failure_recorded.lock(), Some(3));
}

#[tokio::test]
async fn single_attempt_failure_emits_dlq() {
    // A-D-025: a single-attempt (no_retry) failure MUST also emit a DLQ event
    // on exhaustion — matching apcore-python / apcore-typescript, which emit
    // the DLQ regardless of max_attempts. (Previously Rust emitted the DLQ only
    // when max_attempts > 1.)
    let (sub, attempt_count, failure_recorded) =
        AlwaysFailSubscriber::new("sub-single", EventRetryConfig::no_retry());
    let (dlq_sub, dlq_received) = DlqSubscriber::new("dlq-single");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));
    emitter.subscribe(Box::new(dlq_sub));

    let event = ApCoreEvent::new("test.single", json!({}));
    emitter.emit_delivery_semantics(event);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Exactly 1 attempt (single-attempt mode).
    assert_eq!(attempt_count.load(Ordering::SeqCst), 1);
    // DLQ emitted even for single-attempt mode.
    let dlq_events = dlq_received.lock();
    assert_eq!(
        dlq_events.len(),
        1,
        "DLQ must be emitted on single-attempt exhaustion"
    );
    assert_eq!(dlq_events[0].data["attempt_count"].as_u64(), Some(1));
    drop(dlq_events);
    // on_failure was still called.
    assert_eq!(*failure_recorded.lock(), Some(1));
}

#[tokio::test]
async fn shutdown_drops_event_as_noop() {
    let retry_cfg = EventRetryConfig {
        max_attempts: 3,
        initial_backoff_ms: 1,
        max_backoff_ms: 10,
        backoff_multiplier: 2.0,
    };
    let (sub, attempt_count, _received) = CountingSubscriber::new("sub-shutdown", 0, retry_cfg);

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    // Shut down before emitting.
    emitter.shutdown(100).await.unwrap();

    let event = ApCoreEvent::new("test.after_shutdown", json!({}));
    emitter.emit_delivery_semantics(event);

    tokio::time::sleep(Duration::from_millis(50)).await;

    // No attempts made after shutdown.
    assert_eq!(attempt_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sdk_generated_subscriber_ids_are_distinct() {
    // Verify that multiple instances of the same built-in subscriber type
    // get distinct IDs (SDK-generated id pattern).
    use apcore::events::subscribers::StdoutSubscriber;

    let s1 = StdoutSubscriber::new();
    let s2 = StdoutSubscriber::new();
    assert_ne!(
        s1.subscriber_id(),
        s2.subscriber_id(),
        "each StdoutSubscriber instance must have a unique ID"
    );
    assert!(
        s1.subscriber_id().starts_with("stdout-"),
        "ID format: stdout-<n>"
    );
}

/// A-D-010: auto-generated subscriber IDs follow `^{type}-[0-9]+$` (a
/// process-scoped monotonic per-type counter, not a UUID), matching the
/// event_delivery_semantics conformance fixture and Python/TS.
#[tokio::test]
async fn sdk_subscriber_ids_match_monotonic_numeric_pattern() {
    use apcore::events::subscribers::StdoutSubscriber;

    let a = StdoutSubscriber::new();
    let b = StdoutSubscriber::new();

    let parse = |id: &str| -> u64 {
        let rest = id
            .strip_prefix("stdout-")
            .unwrap_or_else(|| panic!("id must start with 'stdout-': {id}"));
        assert!(
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()),
            "id suffix must be all digits (^stdout-[0-9]+$): {id}"
        );
        rest.parse::<u64>().expect("numeric suffix")
    };

    let na = parse(a.subscriber_id());
    let nb = parse(b.subscriber_id());
    // Strictly increasing. (Tests share the process-wide counter, so other
    // concurrently-running tests may bump it between the two `new()` calls;
    // assert monotonicity rather than an exact +1 delta.)
    assert!(
        nb > na,
        "per-type counter must increment: {} then {}",
        a.subscriber_id(),
        b.subscriber_id()
    );
}

/// A-D-009: the DLQ payload's `original_event` MUST use the spec wire keys
/// `name` / `payload` / `metadata` (event-system.md, conformance fixture
/// event_delivery_semantics.json) — not the legacy `event_type` / `data` keys.
#[tokio::test]
async fn dlq_payload_original_event_uses_name_payload_metadata() {
    let retry_cfg = EventRetryConfig {
        max_attempts: 2,
        initial_backoff_ms: 1,
        max_backoff_ms: 10,
        backoff_multiplier: 2.0,
    };
    let (sub, _attempts, _failure) = AlwaysFailSubscriber::new("sub-dlq-payload", retry_cfg);
    let (dlq_sub, dlq_received) = DlqSubscriber::new("dlq-payload");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));
    emitter.subscribe(Box::new(dlq_sub));

    // The pattern for AlwaysFailSubscriber is "test.*", so use a test.* event.
    let event = ApCoreEvent::with_module(
        "test.specific.event",
        json!({"key": "value"}),
        "executor.test.mod",
        "info",
    );
    emitter.emit_delivery_semantics(event);

    tokio::time::sleep(Duration::from_millis(100)).await;

    let dlq_events = dlq_received.lock();
    assert_eq!(dlq_events.len(), 1);
    let original_event = &dlq_events[0].data["original_event"];

    // Canonical spec keys present.
    assert_eq!(
        original_event["name"].as_str(),
        Some("test.specific.event"),
        "original_event must carry `name`"
    );
    assert_eq!(
        original_event["payload"]["key"].as_str(),
        Some("value"),
        "original_event must carry `payload`"
    );
    assert!(
        original_event
            .get("metadata")
            .is_some_and(serde_json::Value::is_object),
        "original_event must carry a `metadata` object"
    );
    // module_id/timestamp preserved under metadata (no information lost).
    assert_eq!(
        original_event["metadata"]["module_id"].as_str(),
        Some("executor.test.mod")
    );
    assert!(original_event["metadata"]["timestamp"].as_str().is_some());

    // Legacy keys MUST be gone.
    assert!(
        original_event.get("event_type").is_none(),
        "legacy `event_type` key must not be present"
    );
    assert!(
        original_event.get("data").is_none(),
        "legacy `data` key must not be present"
    );
}

// ---------------------------------------------------------------------------
// A-D-024 — emit() is non-blocking (does not await the subscriber retry loop)
// ---------------------------------------------------------------------------

/// Subscriber whose first delivery attempt blocks for `delay`, then fails,
/// so the retry loop (with backoff) keeps the delivery task alive a while.
#[derive(Debug)]
struct SlowFailingSubscriber {
    id: String,
    delay: Duration,
    done: Arc<Mutex<bool>>,
    retry_config: EventRetryConfig,
}

#[async_trait]
impl EventSubscriber for SlowFailingSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        "test.*"
    }
    fn retry(&self) -> EventRetryConfig {
        self.retry_config
    }
    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        tokio::time::sleep(self.delay).await;
        *self.done.lock() = true;
        Err(ModuleError::new(
            apcore::errors::ErrorCode::GeneralInternalError,
            "slow failure",
        ))
    }
}

#[tokio::test]
async fn emit_returns_before_slow_subscriber_completes() {
    // A-D-024: emit() must return immediately, before the subscriber's slow
    // retry/backoff loop finishes.
    let done = Arc::new(Mutex::new(false));
    let sub = SlowFailingSubscriber {
        id: "slow".into(),
        delay: Duration::from_millis(200),
        done: Arc::clone(&done),
        retry_config: EventRetryConfig::no_retry(),
    };
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));

    let start = std::time::Instant::now();
    emitter
        .emit(&ApCoreEvent::new("test.slow", json!({})))
        .await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(150),
        "emit() blocked for {elapsed:?}; should return before the slow subscriber finishes"
    );
    assert!(
        !*done.lock(),
        "subscriber should not yet have completed when emit() returned"
    );

    // A-D-027: flush() waits for the in-flight delivery to complete.
    emitter.flush(5000).await.unwrap();
    assert!(*done.lock(), "flush() must await the spawned delivery task");
}

// ---------------------------------------------------------------------------
// A-D-026 — wildcard '*' subscribers are excluded from DLQ delivery
// ---------------------------------------------------------------------------

/// A wildcard ('*') DLQ-capturing subscriber. Under A-D-026 it must NOT
/// receive `apcore.event.delivery_failed` events.
#[derive(Debug)]
struct WildcardDlqSubscriber {
    id: String,
    dlq_received: Arc<Mutex<u32>>,
}

#[async_trait]
impl EventSubscriber for WildcardDlqSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        "*"
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        if event.event_type == "apcore.event.delivery_failed" {
            *self.dlq_received.lock() += 1;
        }
        Ok(())
    }
}

#[tokio::test]
async fn wildcard_subscriber_excluded_from_dlq() {
    // A-D-026: a '*' subscriber must NOT receive delivery_failed DLQ events.
    let retry_cfg = EventRetryConfig::no_retry();
    let (fail_sub, _attempts, _failure) = AlwaysFailSubscriber::new("sub-fail", retry_cfg);
    let dlq_count = Arc::new(Mutex::new(0u32));
    let wildcard = WildcardDlqSubscriber {
        id: "wildcard".into(),
        dlq_received: Arc::clone(&dlq_count),
    };
    // Also add an explicit DLQ subscriber to prove the DLQ WAS emitted.
    let (dlq_sub, dlq_received) = DlqSubscriber::new("explicit-dlq");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(fail_sub));
    emitter.subscribe(Box::new(wildcard));
    emitter.subscribe(Box::new(dlq_sub));

    emitter
        .emit(&ApCoreEvent::new("test.fail", json!({})))
        .await;
    emitter.flush(5000).await.unwrap();

    assert_eq!(
        dlq_received.lock().len(),
        1,
        "explicit (non-wildcard) DLQ subscriber should receive the DLQ event"
    );
    assert_eq!(
        *dlq_count.lock(),
        0,
        "wildcard '*' subscriber must be excluded from DLQ delivery"
    );
}

// ---------------------------------------------------------------------------
// A-D-029 — DLQ payload subscriber_type uses the declared type, not the id
// ---------------------------------------------------------------------------

/// A subscriber whose id has NO dash (so the old id-parsing logic would have
/// produced the whole id), but declares a distinct subscriber_type.
#[derive(Debug)]
struct DeclaredTypeSubscriber {
    id: String,
}

#[async_trait]
impl EventSubscriber for DeclaredTypeSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    #[allow(clippy::unnecessary_literal_bound)]
    fn subscriber_type(&self) -> &str {
        "customsink"
    }
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        "test.*"
    }
    fn retry(&self) -> EventRetryConfig {
        EventRetryConfig::no_retry()
    }
    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        Err(ModuleError::new(
            apcore::errors::ErrorCode::GeneralInternalError,
            "always fails",
        ))
    }
}

#[tokio::test]
async fn dlq_payload_uses_declared_subscriber_type() {
    // A-D-029: subscriber_type comes from the declared trait method, even when
    // the subscriber_id has no dash (the old split('-') heuristic would have
    // returned "noDashId" instead of the declared "customsink").
    let sub = DeclaredTypeSubscriber {
        id: "noDashId".into(),
    };
    let (dlq_sub, dlq_received) = DlqSubscriber::new("dlq-type");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));
    emitter.subscribe(Box::new(dlq_sub));

    emitter
        .emit(&ApCoreEvent::new("test.typed", json!({})))
        .await;
    emitter.flush(5000).await.unwrap();

    let dlq_events = dlq_received.lock();
    assert_eq!(dlq_events.len(), 1);
    assert_eq!(
        dlq_events[0].data["subscriber_type"].as_str(),
        Some("customsink"),
        "DLQ subscriber_type must use the declared type"
    );
    assert_eq!(
        dlq_events[0].data["subscriber_id"].as_str(),
        Some("noDashId")
    );
}

// ---------------------------------------------------------------------------
// A-D-013: the Registry MUST dispatch its events via the DLQ-bearing path so a
// failing subscriber on a registry event produces an
// `apcore.event.delivery_failed` event rather than a silent drop. Previously
// the registry used `emit_spawn` (single attempt, no retry, no DLQ).
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct RegistryEventFailSubscriber {
    id: String,
    attempts: Arc<AtomicU32>,
}

#[async_trait]
impl EventSubscriber for RegistryEventFailSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        // Match only the registry event, NOT the DLQ event it generates.
        "apcore.registry.module_load_failed"
    }
    fn retry(&self) -> EventRetryConfig {
        EventRetryConfig::no_retry()
    }
    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(ModuleError::new(
            apcore::errors::ErrorCode::GeneralInternalError,
            "registry subscriber always fails",
        ))
    }
}

#[tokio::test]
async fn registry_event_failure_is_dead_lettered_not_dropped() {
    use apcore::context::Context;
    use apcore::module::Module;
    use apcore::registry::Registry;
    use serde_json::Value;

    // A module whose on_load() fails — this drives the registry to emit
    // `apcore.registry.module_load_failed`.
    #[derive(Debug)]
    struct OnLoadFailModule;
    #[async_trait]
    impl Module for OnLoadFailModule {
        fn description(&self) -> &'static str {
            "on_load fails"
        }
        fn input_schema(&self) -> Value {
            json!({ "type": "object" })
        }
        fn output_schema(&self) -> Value {
            json!({ "type": "object" })
        }
        fn on_load(&self) -> Result<(), ModuleError> {
            Err(ModuleError::new(
                apcore::errors::ErrorCode::GeneralInternalError,
                "boom",
            ))
        }
        async fn execute(&self, _i: Value, _c: &Context<Value>) -> Result<Value, ModuleError> {
            Ok(json!({}))
        }
    }

    let attempts = Arc::new(AtomicU32::new(0));
    let fail_sub = RegistryEventFailSubscriber {
        id: "registry-fail-sub".into(),
        attempts: Arc::clone(&attempts),
    };
    let (dlq_sub, dlq_received) = DlqSubscriber::new("registry-dlq");

    let emitter = Arc::new(EventEmitter::new());
    emitter.subscribe(Box::new(fail_sub));
    emitter.subscribe(Box::new(dlq_sub));

    let registry = Registry::new();
    registry.set_event_emitter(Arc::clone(&emitter));

    // Registering a module whose on_load fails emits module_load_failed.
    let _ = registry.register_module("executor.fail.mod", Box::new(OnLoadFailModule));

    // Let the spawned delivery + DLQ tasks run.
    emitter.flush(5000).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        attempts.load(Ordering::SeqCst) >= 1,
        "the failing registry-event subscriber must have been invoked"
    );
    let dlq_events = dlq_received.lock();
    assert_eq!(
        dlq_events.len(),
        1,
        "a failing registry-event subscriber must produce exactly one DLQ event (no silent drop)"
    );
    assert_eq!(
        dlq_events[0].event_type, "apcore.event.delivery_failed",
        "the dead-letter event type must be apcore.event.delivery_failed"
    );
}
