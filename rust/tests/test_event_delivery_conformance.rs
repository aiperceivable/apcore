// Conformance tests for Event Delivery Semantics (Issue #61).
// Fixture: apcore/conformance/fixtures/event_delivery_semantics.json
#![allow(clippy::pedantic)] // fixture-driven test file: casts and struct layouts follow fixture schema

use apcore::errors::{ErrorCode, ModuleError};
use apcore::events::emitter::{ApCoreEvent, EventEmitter};
use apcore::events::retry::EventRetryConfig;
use apcore::events::subscribers::EventSubscriber;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture() -> Value {
    let path = find_fixtures_root().join("event_delivery_semantics.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON: {e}"))
}

fn fixture_case<'a>(fixture: &'a Value, id: &str) -> &'a Value {
    fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array")
        .iter()
        .find(|c| c["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("test case '{id}' not found in fixture"))
}

// ---------------------------------------------------------------------------
// Test subscriber helpers
// ---------------------------------------------------------------------------

/// Fails the first `fail_count` attempts, then succeeds.
#[derive(Debug)]
struct FailNSubscriber {
    id: String,
    fail_count: u32,
    attempt_count: Arc<AtomicU32>,
    received: Arc<Mutex<Vec<String>>>,
    retry_config: EventRetryConfig,
    /// Wall-clock instant of each delivery attempt, so the gaps between them
    /// can be measured against the fixture's declared backoff schedule.
    attempt_times: Arc<Mutex<Vec<std::time::Instant>>>,
}

#[async_trait]
impl EventSubscriber for FailNSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    fn event_pattern(&self) -> &str {
        "*"
    }
    fn retry(&self) -> EventRetryConfig {
        self.retry_config
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.attempt_times.lock().push(std::time::Instant::now());
        let attempt = self.attempt_count.fetch_add(1, Ordering::SeqCst);
        if attempt < self.fail_count {
            Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "transient failure",
            ))
        } else {
            self.received.lock().push(event.event_type.clone());
            Ok(())
        }
    }
}

/// Always fails.
#[derive(Debug)]
struct AlwaysFailSubscriber {
    id: String,
    pattern: String,
    attempt_count: Arc<AtomicU32>,
    on_failure_count: Arc<AtomicU32>,
    retry_config: EventRetryConfig,
}

#[async_trait]
impl EventSubscriber for AlwaysFailSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    fn event_pattern(&self) -> &str {
        &self.pattern
    }
    fn retry(&self) -> EventRetryConfig {
        self.retry_config
    }
    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.attempt_count.fetch_add(1, Ordering::SeqCst);
        Err(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "permanent failure",
        ))
    }
    async fn on_failure(&self, _event: &ApCoreEvent, _err: &ModuleError, _count: u32) {
        self.on_failure_count.fetch_add(1, Ordering::SeqCst);
    }
}

/// Records received events.
#[derive(Debug)]
struct RecordingSubscriber {
    id: String,
    pattern: String,
    received: Arc<Mutex<Vec<ApCoreEvent>>>,
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
        self.received.lock().push(event.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Case: retry_succeeds_before_exhaustion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_retry_succeeds_before_exhaustion() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "retry_succeeds_before_exhaustion");

    // Setup from fixture
    let retry_cfg = &case["setup"]["subscriber"]["retry"];
    let max_attempts = retry_cfg["max_attempts"].as_u64().unwrap() as u32;
    let initial_backoff_ms = retry_cfg["initial_backoff_ms"].as_u64().unwrap();
    let backoff_multiplier = retry_cfg["backoff_multiplier"].as_f64().unwrap();
    let fail_attempts_count = case["setup"]["subscriber"]["fail_attempts"]
        .as_array()
        .map(|a| a.len() as u32)
        .unwrap_or(2);

    let attempt_count = Arc::new(AtomicU32::new(0));
    let received = Arc::new(Mutex::new(Vec::new()));
    let attempt_times = Arc::new(Mutex::new(Vec::new()));
    let retry_config = EventRetryConfig {
        max_attempts,
        initial_backoff_ms,
        max_backoff_ms: retry_cfg["max_backoff_ms"].as_u64().unwrap_or(100),
        backoff_multiplier,
    };
    let sub = FailNSubscriber {
        id: case["setup"]["subscriber"]["id"]
            .as_str()
            .unwrap()
            .to_string(),
        fail_count: fail_attempts_count,
        attempt_count: Arc::clone(&attempt_count),
        received: Arc::clone(&received),
        retry_config,
        attempt_times: Arc::clone(&attempt_times),
    };

    // DLQ recording subscriber
    let dlq_received = Arc::new(Mutex::new(Vec::new()));
    let dlq_sub = RecordingSubscriber {
        id: "dlq-recorder".to_string(),
        pattern: "apcore.event.delivery_failed".to_string(),
        received: Arc::clone(&dlq_received),
    };

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));
    emitter.subscribe(Box::new(dlq_sub));

    let event_name = case["trigger"]["event"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let event = ApCoreEvent::new(event_name, json!({"value": 42}));
    emitter.emit_delivery_semantics(event);

    // Allow tasks to settle
    tokio::time::sleep(Duration::from_millis(200)).await;

    let expected_attempts = case["expected"]["attempt_count"].as_u64().unwrap() as u32;
    let dlq_expected = case["expected"]["dlq_event_emitted"].as_bool().unwrap();

    // Verify attempt count
    assert_eq!(
        attempt_count.load(Ordering::SeqCst),
        expected_attempts,
        "attempt_count mismatch"
    );

    // Verify DLQ not emitted (succeeded before exhaustion)
    assert_eq!(
        !dlq_received.lock().is_empty(),
        dlq_expected,
        "dlq_event_emitted mismatch: expected={dlq_expected}"
    );

    // Verify the event was ultimately received by the subscriber
    assert!(
        !received.lock().is_empty(),
        "event must be received on success"
    );

    // `backoff_delays_ms` — the delay before retry N, checked two ways.
    //
    // (a) The schedule the SDK computes. `compute_delay_ms` is the function the
    //     delivery loop actually sleeps on (emitter.rs `deliver_with_dlq`), so
    //     this pins the exact sequence, not an approximation of it.
    // (b) The delays that were really taken, measured between consecutive
    //     delivery attempts. `tokio::time::sleep` never wakes early, so a lower
    //     bound here is exact rather than flaky; it catches a schedule that is
    //     computed correctly and then not applied.
    let want_delays: Vec<u64> = case["expected"]["backoff_delays_ms"]
        .as_array()
        .expect("backoff_delays_ms is an array")
        .iter()
        .map(|v| v.as_u64().expect("delay is an integer"))
        .collect();

    let computed: Vec<u64> = (0..want_delays.len() as u32)
        .map(|attempt| retry_config.compute_delay_ms(attempt))
        .collect();
    assert_eq!(
        computed, want_delays,
        "backoff_delays_ms: EventRetryConfig::compute_delay_ms produced {computed:?}"
    );

    let times = attempt_times.lock();
    assert_eq!(
        times.len() as u32,
        expected_attempts,
        "one timestamp per delivery attempt"
    );
    for (i, want_ms) in want_delays.iter().enumerate() {
        let observed = times[i + 1].duration_since(times[i]).as_millis();
        assert!(
            observed >= u128::from(*want_ms),
            "backoff_delays_ms[{i}]: retry {} started {observed}ms after the previous \
             attempt, less than the required {want_ms}ms backoff",
            i + 1
        );
    }
}

// ---------------------------------------------------------------------------
// Case: permanent_failure_emits_dlq_event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_permanent_failure_emits_dlq_event() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "permanent_failure_emits_dlq_event");

    let retry_cfg = &case["setup"]["subscriber"]["retry"];
    let max_attempts = retry_cfg["max_attempts"].as_u64().unwrap() as u32;

    let attempt_count = Arc::new(AtomicU32::new(0));
    let on_failure_count = Arc::new(AtomicU32::new(0));
    let event_name_for_pattern = case["trigger"]["event"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let sub = AlwaysFailSubscriber {
        id: case["setup"]["subscriber"]["id"]
            .as_str()
            .unwrap()
            .to_string(),
        // Use an exact pattern so this subscriber does NOT receive its own DLQ event.
        pattern: event_name_for_pattern.clone(),
        attempt_count: Arc::clone(&attempt_count),
        on_failure_count: Arc::clone(&on_failure_count),
        retry_config: EventRetryConfig {
            max_attempts,
            initial_backoff_ms: retry_cfg["initial_backoff_ms"].as_u64().unwrap_or(10),
            max_backoff_ms: 200,
            backoff_multiplier: 2.0,
        },
    };

    let dlq_received: Arc<Mutex<Vec<ApCoreEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let dlq_sub = RecordingSubscriber {
        id: "dlq-recorder".to_string(),
        pattern: "apcore.event.delivery_failed".to_string(),
        received: Arc::clone(&dlq_received),
    };

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(sub));
    emitter.subscribe(Box::new(dlq_sub));

    let event_name = case["trigger"]["event"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let event = ApCoreEvent::new(event_name.clone(), json!({"service": "billing"}));
    emitter.emit_delivery_semantics(event);

    // Allow retry + DLQ to settle (3 attempts * 10ms each + overhead)
    tokio::time::sleep(Duration::from_millis(300)).await;

    let expected_attempts = case["expected"]["attempt_count"].as_u64().unwrap() as u32;
    assert_eq!(
        attempt_count.load(Ordering::SeqCst),
        expected_attempts,
        "attempt_count mismatch"
    );

    // DLQ emitted
    assert!(
        case["expected"]["dlq_event_emitted"].as_bool().unwrap(),
        "fixture must expect DLQ"
    );
    let dlq_events = dlq_received.lock();
    assert!(
        !dlq_events.is_empty(),
        "DLQ event must be emitted after exhaustion"
    );

    let dlq = &dlq_events[0];
    assert_eq!(dlq.event_type, "apcore.event.delivery_failed");

    // Verify required keys per fixture
    let required_keys: Vec<&str> = case["expected"]["dlq_event"]["data_required_keys"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    for key in &required_keys {
        assert!(
            dlq.data.get(key).is_some(),
            "DLQ event data missing required key: {key}"
        );
    }

    // Verify specific data values
    let data_contains = &case["expected"]["dlq_event"]["data_contains"];
    assert_eq!(
        dlq.data["subscriber_id"].as_str(),
        data_contains["subscriber_id"].as_str(),
        "subscriber_id mismatch"
    );
    assert_eq!(
        dlq.data["attempt_count"].as_u64(),
        data_contains["attempt_count"].as_u64(),
        "attempt_count in DLQ payload mismatch"
    );
    // A-D-009: original_event uses the spec wire key `name` (not event_type).
    assert_eq!(
        dlq.data["original_event"]["name"].as_str(),
        data_contains["original_event"]["name"].as_str(),
        "original_event.name mismatch"
    );

    // on_failure was called
    assert_eq!(
        on_failure_count.load(Ordering::SeqCst),
        1,
        "on_failure must be called once"
    );
}

// ---------------------------------------------------------------------------
// Case: dlq_event_subscriber_failure_is_not_retried
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_dlq_event_subscriber_failure_is_not_retried() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "dlq_event_subscriber_failure_is_not_retried");

    let primary_cfg = &case["setup"]["primary_subscriber"]["retry"];
    let primary_max = primary_cfg["max_attempts"].as_u64().unwrap() as u32;
    let primary_attempts = Arc::new(AtomicU32::new(0));
    let primary_sub = AlwaysFailSubscriber {
        id: case["setup"]["primary_subscriber"]["id"]
            .as_str()
            .unwrap()
            .to_string(),
        // Explicit pattern so primary subscriber does NOT receive the DLQ event it triggers.
        pattern: "apcore.test.broken".to_string(),
        attempt_count: Arc::clone(&primary_attempts),
        on_failure_count: Arc::new(AtomicU32::new(0)),
        retry_config: EventRetryConfig {
            max_attempts: primary_max,
            initial_backoff_ms: primary_cfg["initial_backoff_ms"].as_u64().unwrap_or(10),
            max_backoff_ms: 200,
            backoff_multiplier: 2.0,
        },
    };

    // DLQ subscriber that always fails but has high retry count
    let dlq_cfg = &case["setup"]["dlq_subscriber"]["retry"];
    let dlq_attempts = Arc::new(AtomicU32::new(0));
    let dlq_sub = AlwaysFailSubscriber {
        id: case["setup"]["dlq_subscriber"]["id"]
            .as_str()
            .unwrap()
            .to_string(),
        pattern: "apcore.event.delivery_failed".to_string(),
        attempt_count: Arc::clone(&dlq_attempts),
        on_failure_count: Arc::new(AtomicU32::new(0)),
        retry_config: EventRetryConfig {
            max_attempts: dlq_cfg["max_attempts"].as_u64().unwrap() as u32,
            initial_backoff_ms: 5,
            max_backoff_ms: 50,
            backoff_multiplier: 1.0,
        },
    };

    // A passive recorder on the same DLQ pattern. It counts every DLQ event
    // that is dispatched, which is what makes `second_order_dlq_event_emitted`
    // observable: a DLQ raised for the FAILING DLQ subscriber would land here
    // as a second event.
    let dlq_seen: Arc<Mutex<Vec<ApCoreEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let dlq_recorder = RecordingSubscriber {
        id: "dlq-recorder".to_string(),
        pattern: "apcore.event.delivery_failed".to_string(),
        received: Arc::clone(&dlq_seen),
    };

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(primary_sub));
    emitter.subscribe(Box::new(dlq_sub));
    emitter.subscribe(Box::new(dlq_recorder));

    let event = ApCoreEvent::new("apcore.test.broken", json!({}));
    emitter.emit_delivery_semantics(event);

    // Allow primary retries + DLQ delivery attempt to settle
    tokio::time::sleep(Duration::from_millis(400)).await;

    let expected_primary = case["expected"]["primary_attempt_count"].as_u64().unwrap() as u32;
    assert_eq!(
        primary_attempts.load(Ordering::SeqCst),
        expected_primary,
        "primary subscriber attempt_count mismatch"
    );

    // DLQ subscriber is called EXACTLY once (no retry on DLQ delivery)
    let expected_dlq_attempts = case["expected"]["dlq_subscriber_attempt_count"]
        .as_u64()
        .unwrap() as u32;
    assert_eq!(
        dlq_attempts.load(Ordering::SeqCst),
        expected_dlq_attempts,
        "DLQ subscriber must be called exactly {expected_dlq_attempts} time(s) — DLQ delivery is never retried"
    );

    // `dlq_event_emitted` — exactly one, for the primary subscriber.
    let dlq_events = dlq_seen.lock();
    assert_eq!(
        !dlq_events.is_empty(),
        case["expected"]["dlq_event_emitted"].as_bool().unwrap(),
        "dlq_event_emitted mismatch"
    );

    // `second_order_dlq_event_emitted` — the DLQ subscriber above fails on
    // every attempt. If the SDK raised a DLQ for THAT failure the recorder
    // would hold a second event, which is the infinite-loop this rule forbids.
    let second_order = dlq_events.len() > 1;
    assert_eq!(
        second_order,
        case["expected"]["second_order_dlq_event_emitted"]
            .as_bool()
            .unwrap(),
        "second_order_dlq_event_emitted mismatch — the recorder saw {} DLQ event(s); \
         a failing DLQ subscriber must be logged and discarded, not re-queued",
        dlq_events.len()
    );
}

// ---------------------------------------------------------------------------
// Case: dlq_event_subscriber_failure_is_not_retried — `error_log_count`
// ---------------------------------------------------------------------------

/// In-memory `tracing` writer so the test can count emitted log records.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<std::sync::Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
    type Writer = CapturedLogs;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// `error_log_count`: "the SDK MUST log at ERROR and discard". Discarding is
/// covered above by `dlq_subscriber_attempt_count` / `second_order_...`; this
/// covers the other half, that the discard is not silent.
///
/// Delivery is driven through `emit_filtered` rather than
/// `emit_delivery_semantics` because the latter spawns a `tokio::task` per
/// subscriber, and a task does not inherit a thread-scoped `tracing`
/// subscriber. `emit_filtered` runs the same `deliver_with_dlq` inline on this
/// thread, so the records are captured. Both entry points share that function,
/// so the logging behaviour under test is the same one.
#[tokio::test]
async fn conformance_dlq_subscriber_failure_logs_at_error() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "dlq_event_subscriber_failure_is_not_retried");

    let primary_cfg = &case["setup"]["primary_subscriber"]["retry"];
    let primary_sub = AlwaysFailSubscriber {
        id: case["setup"]["primary_subscriber"]["id"]
            .as_str()
            .unwrap()
            .to_string(),
        pattern: "apcore.test.broken".to_string(),
        attempt_count: Arc::new(AtomicU32::new(0)),
        on_failure_count: Arc::new(AtomicU32::new(0)),
        retry_config: EventRetryConfig {
            max_attempts: primary_cfg["max_attempts"].as_u64().unwrap() as u32,
            initial_backoff_ms: 1,
            max_backoff_ms: 10,
            backoff_multiplier: 1.0,
        },
    };
    let broken_dlq = AlwaysFailSubscriber {
        id: case["setup"]["dlq_subscriber"]["id"]
            .as_str()
            .unwrap()
            .to_string(),
        pattern: "apcore.event.delivery_failed".to_string(),
        attempt_count: Arc::new(AtomicU32::new(0)),
        on_failure_count: Arc::new(AtomicU32::new(0)),
        retry_config: EventRetryConfig {
            max_attempts: case["setup"]["dlq_subscriber"]["retry"]["max_attempts"]
                .as_u64()
                .unwrap() as u32,
            initial_backoff_ms: 1,
            max_backoff_ms: 10,
            backoff_multiplier: 1.0,
        },
    };

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(primary_sub));
    emitter.subscribe(Box::new(broken_dlq));

    let captured = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::ERROR)
        .with_ansi(false)
        .with_target(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let event = ApCoreEvent::new("apcore.test.broken", json!({}));
    emitter.emit_filtered(&event, "*").await.unwrap();
    drop(guard);

    let logs = captured.text();
    let error_lines = logs.lines().filter(|l| l.contains("ERROR")).count();
    assert_eq!(
        error_lines as u64,
        case["expected"]["error_log_count"].as_u64().unwrap(),
        "error_log_count mismatch — captured:\n{logs}"
    );
    assert!(
        logs.contains(case["setup"]["dlq_subscriber"]["id"].as_str().unwrap()),
        "the ERROR record must name the failing DLQ subscriber — captured:\n{logs}"
    );
}

// ---------------------------------------------------------------------------
// Case: subscriber_id_sdk_generated_when_omitted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_subscriber_id_sdk_generated_when_omitted() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "subscriber_id_sdk_generated_when_omitted");

    // The fixture's two subscribers are `stdout` with no `id` and
    // `fail_attempts: "all"`. `StdoutSubscriber::new()` is what generates the
    // id, so each subscriber under test takes its id FROM a real
    // StdoutSubscriber and layers the fixture's injected failure on top — the
    // identifier is the SDK's, only the delivery outcome is the test's.
    use apcore::events::subscribers::StdoutSubscriber;

    #[derive(Debug)]
    struct FailingStdout {
        inner: StdoutSubscriber,
        retry_config: EventRetryConfig,
    }

    #[async_trait]
    impl EventSubscriber for FailingStdout {
        fn subscriber_id(&self) -> &str {
            self.inner.subscriber_id()
        }
        fn subscriber_type(&self) -> &str {
            self.inner.subscriber_type()
        }
        fn event_pattern(&self) -> &str {
            "apcore.test.dlq_count"
        }
        fn retry(&self) -> EventRetryConfig {
            self.retry_config
        }
        async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
            Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "fail_attempts: all",
            ))
        }
    }

    let subscribers = case["setup"]["subscribers"]
        .as_array()
        .expect("setup.subscribers is an array");
    let mut generated_ids = Vec::new();
    let emitter = EventEmitter::new();
    for sub_cfg in subscribers {
        assert!(
            sub_cfg.get("id").is_none(),
            "this case is about subscribers that OMIT the id field"
        );
        let inner = StdoutSubscriber::new();
        generated_ids.push(inner.subscriber_id().to_string());
        emitter.subscribe(Box::new(FailingStdout {
            inner,
            retry_config: EventRetryConfig {
                max_attempts: sub_cfg["retry"]["max_attempts"].as_u64().unwrap() as u32,
                initial_backoff_ms: 1,
                max_backoff_ms: 10,
                backoff_multiplier: 1.0,
            },
        }));
    }

    let dlq_seen: Arc<Mutex<Vec<ApCoreEvent>>> = Arc::new(Mutex::new(Vec::new()));
    emitter.subscribe(Box::new(RecordingSubscriber {
        id: "dlq-recorder".to_string(),
        pattern: "apcore.event.delivery_failed".to_string(),
        received: Arc::clone(&dlq_seen),
    }));

    emitter.emit_delivery_semantics(ApCoreEvent::new("apcore.test.dlq_count", json!({})));
    emitter.flush(5_000).await.unwrap();

    // `dlq_events_emitted` — one per exhausted subscriber.
    let dlq_events = dlq_seen.lock();
    assert_eq!(
        dlq_events.len() as u64,
        case["expected"]["dlq_events_emitted"].as_u64().unwrap(),
        "dlq_events_emitted mismatch"
    );

    // `subscriber_ids_distinct` / `subscriber_ids_pattern` — asserted against
    // the ids that actually travelled in the DLQ payloads, which is what
    // "used consistently across all DLQ events" means.
    let payload_ids: Vec<String> = dlq_events
        .iter()
        .map(|e| e.data["subscriber_id"].as_str().unwrap().to_string())
        .collect();
    let distinct: std::collections::HashSet<&String> = payload_ids.iter().collect();
    assert_eq!(
        distinct.len() == payload_ids.len(),
        case["expected"]["subscriber_ids_distinct"]
            .as_bool()
            .unwrap(),
        "subscriber_ids_distinct mismatch: {payload_ids:?}"
    );
    let pattern = regex::Regex::new(case["expected"]["subscriber_ids_pattern"].as_str().unwrap())
        .expect("subscriber_ids_pattern is a valid regex");
    for id in &payload_ids {
        assert!(
            pattern.is_match(id),
            "DLQ payload subscriber_id {id:?} does not match {pattern}"
        );
    }
    // The id in the DLQ payload must be the one the SDK generated, not a
    // re-derivation: the two sets must agree.
    let generated: std::collections::HashSet<&String> = generated_ids.iter().collect();
    assert_eq!(
        distinct, generated,
        "DLQ events must carry the SDK-generated subscriber ids"
    );
}

// ---------------------------------------------------------------------------
// Subscriber retry configuration (apcore#85)
// ---------------------------------------------------------------------------

/// Assert the policy the DELIVERY PATH resolves, per the fixture's
/// `read_the_resolved_policy_not_the_config` contract. `retry()` is the trait
/// method the emitter consults; this SDK previously stored `retry_count` on a
/// field `retry()` never read, so an assertion against the config the factory
/// was handed was green while delivery used the default.
fn assert_resolved_policy(type_name: &str, sub: &dyn EventSubscriber, want: &Value) {
    let got = sub.retry();
    let num = |k: &str| want[k].as_u64().expect("policy field is numeric");
    assert_eq!(
        u64::from(got.max_attempts),
        num("max_attempts"),
        "[{type_name}] max_attempts: the declared policy did not reach retry()"
    );
    assert_eq!(
        got.initial_backoff_ms,
        num("initial_backoff_ms"),
        "[{type_name}] initial_backoff_ms"
    );
    assert_eq!(
        got.max_backoff_ms,
        num("max_backoff_ms"),
        "[{type_name}] max_backoff_ms"
    );
    let want_mult = want["backoff_multiplier"].as_f64().expect("numeric");
    assert!(
        (got.backoff_multiplier - want_mult).abs() < 1e-9,
        "[{type_name}] backoff_multiplier: got {}, want {want_mult}",
        got.backoff_multiplier
    );
}

#[tokio::test]
async fn conformance_declared_retry_policy_is_read_for_every_subscriber_type() {
    let fixture = load_fixture();
    let case = fixture_case(
        &fixture,
        "declared_retry_policy_is_read_for_every_subscriber_type",
    );

    // The fixture's `every_type_or_the_case_is_half_done` contract: the defect
    // was NOT uniform — `webhook` parsed a different (flat) spelling while
    // `file` / `stdout` / `filter` had no `retry` field at all here, so a
    // webhook-only assertion was green while four types ignored their config.
    let declared = case["subscribers"]
        .as_array()
        .expect("subscribers is an array");
    let expected = &case["expected"]["resolved_policy_per_type"];
    assert!(
        case["expected"]["every_field_differs_from_the_default"]
            .as_bool()
            .unwrap_or(false),
        "the fixture must declare a policy differing from the shipped default on every field, \
         or this case passes whether or not the config was read"
    );

    let default = EventRetryConfig::default();
    let mut checked = Vec::new();
    for entry in declared {
        let type_name = entry["type"].as_str().expect("type");
        // The fixture carries every field a type requires to construct, so the
        // entry goes to the factory verbatim — see its `a_case_must_carry_its_
        // own_inputs` contract.
        let sub = apcore::events::subscribers::create_subscriber(entry)
            .unwrap_or_else(|e| panic!("[{type_name}] factory failed: {}", e.message));
        let want = &expected[type_name];
        assert!(
            want.is_object(),
            "[{type_name}] fixture declares no expected policy for this type"
        );
        assert_ne!(
            want["max_attempts"].as_u64().unwrap(),
            u64::from(default.max_attempts),
            "[{type_name}] the expected policy must differ from the default"
        );
        assert_resolved_policy(type_name, sub.as_ref(), want);
        checked.push(type_name.to_string());
    }

    let mut want_types: Vec<String> = expected
        .as_object()
        .expect("resolved_policy_per_type is an object")
        .keys()
        .cloned()
        .collect();
    want_types.sort();
    checked.sort();
    assert_eq!(
        checked, want_types,
        "every built-in subscriber type the fixture names must be exercised"
    );
}

#[tokio::test]
async fn conformance_nested_retry_block_wins_over_legacy_retry_count() {
    let fixture = load_fixture();
    let case = fixture_case(&fixture, "nested_retry_block_wins_over_legacy_retry_count");
    let subs = case["subscribers"].as_array().expect("subscribers");

    // [0] carries BOTH spellings — the nested block must win.
    let both = apcore::events::subscribers::create_subscriber(&subs[0])
        .expect("factory accepts both spellings");
    assert_eq!(
        u64::from(both.retry().max_attempts),
        case["expected"]["nested_wins"]["max_attempts"]
            .as_u64()
            .expect("numeric"),
        "the nested retry block must win over the legacy flat retry_count"
    );

    // [1] carries only the legacy spelling — it must still translate.
    let legacy = apcore::events::subscribers::create_subscriber(&subs[1])
        .expect("factory accepts the legacy spelling");
    assert_eq!(
        u64::from(legacy.retry().max_attempts),
        case["expected"]["legacy_alone_still_translates"]["max_attempts"]
            .as_u64()
            .expect("numeric"),
        "retry_count alone must still translate as max_attempts = retry_count + 1"
    );
}
