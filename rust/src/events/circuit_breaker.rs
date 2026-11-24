// APCore Protocol — Per-subscriber circuit breaker (Issue #36)
// Spec reference: docs/features/event-system.md §Subscriber Circuit Breaker
//
// Wraps any `EventSubscriber` with an independent circuit breaker that
// tolerates a degraded downstream by tripping into OPEN after
// `open_threshold` consecutive failures and probing once via HALF_OPEN after
// `recovery_window_ms`. Mirrors the Python `CircuitBreakerWrapper`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde_json::json;

use super::emitter::ApCoreEvent;
use super::retry::EventRetryConfig;
use super::subscribers::EventSubscriber;
use crate::errors::{ErrorCode, ModuleError};

/// Circuit-breaker lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitState {
    /// Spec-canonical uppercase representation (`"CLOSED" | "OPEN" | "HALF_OPEN"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "CLOSED",
            Self::Open => "OPEN",
            Self::HalfOpen => "HALF_OPEN",
        }
    }
}

/// Sink for the circuit-breaker's own lifecycle events
/// (`apcore.subscriber.circuit_opened` / `circuit_closed`).
///
/// In production this is typically a wrapper around the surrounding
/// `EventEmitter`; in tests it is a recording sink that captures the events
/// for assertion. Decoupling the breaker from a concrete `EventEmitter`
/// avoids an Arc cycle (the emitter holds the wrapper, the wrapper would
/// otherwise need to hold the emitter).
pub trait CircuitEventSink: Send + Sync + std::fmt::Debug {
    /// Receive an `apcore.subscriber.*` lifecycle event.
    fn emit_circuit_event(&self, event: ApCoreEvent);
}

/// Default tunables, matching the spec.
pub const DEFAULT_TIMEOUT_MS: u64 = 5000;
pub const DEFAULT_OPEN_THRESHOLD: u32 = 5;
pub const DEFAULT_RECOVERY_WINDOW_MS: u64 = 60_000;

/// Circuit-breaker wrapper that enforces timeouts, counts failures, and
/// transitions through `CLOSED → OPEN → HALF_OPEN → CLOSED`.
pub struct CircuitBreakerWrapper {
    subscriber: Box<dyn EventSubscriber>,
    sink: Arc<dyn CircuitEventSink>,
    timeout_ms: u64,
    open_threshold: u32,
    recovery_window_ms: u64,
    state: Mutex<DeliveryCircuit>,
    /// Pluggable clock for tests; defaults to wall clock.
    clock: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    subscriber_type_name: String,
}

#[derive(Debug)]
struct DeliveryCircuit {
    phase: CircuitState,
    consecutive_failures: u32,
    last_failure_at: Option<DateTime<Utc>>,
    generation: u64,
    probe_in_flight: bool,
}

impl DeliveryCircuit {
    fn transition(&mut self, phase: CircuitState) {
        self.phase = phase;
        self.generation += 1;
        self.probe_in_flight = false;
    }
}

/// Records which circuit generation admitted a delivery. A cancelled probe
/// releases the half-open slot and an old completion cannot change a new cycle.
struct DeliveryAdmission<'a> {
    circuit: &'a Mutex<DeliveryCircuit>,
    generation: u64,
    probe: bool,
}

impl Drop for DeliveryAdmission<'_> {
    fn drop(&mut self) {
        if self.probe {
            let mut circuit = self.circuit.lock();
            if circuit.generation == self.generation {
                circuit.probe_in_flight = false;
            }
        }
    }
}

impl std::fmt::Debug for CircuitBreakerWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreakerWrapper")
            .field("circuit", &*self.state.lock())
            .field("timeout_ms", &self.timeout_ms)
            .field("open_threshold", &self.open_threshold)
            .field("recovery_window_ms", &self.recovery_window_ms)
            .field("subscriber_type_name", &self.subscriber_type_name)
            .finish_non_exhaustive()
    }
}

impl CircuitBreakerWrapper {
    /// Wrap `subscriber` with circuit-breaker behaviour.
    ///
    /// `sink` receives `apcore.subscriber.circuit_opened` and
    /// `apcore.subscriber.circuit_closed` events on every transition. The sink
    /// is mandatory: the spec requires these lifecycle events to ALWAYS be
    /// emitted (cross-language parity with Python/TS, which take a mandatory
    /// emitter — sync finding A-D-07). In production this is a thin wrapper
    /// around the surrounding `EventEmitter`.
    #[must_use]
    pub fn new(subscriber: Box<dyn EventSubscriber>, sink: Arc<dyn CircuitEventSink>) -> Self {
        let subscriber_type_name = declared_subscriber_type(subscriber.as_ref());
        Self {
            subscriber,
            sink,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            open_threshold: DEFAULT_OPEN_THRESHOLD,
            recovery_window_ms: DEFAULT_RECOVERY_WINDOW_MS,
            state: Mutex::new(DeliveryCircuit {
                phase: CircuitState::Closed,
                consecutive_failures: 0,
                last_failure_at: None,
                generation: 0,
                probe_in_flight: false,
            }),
            clock: Box::new(Utc::now),
            subscriber_type_name,
        }
    }

    #[must_use]
    pub fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    #[must_use]
    pub fn with_open_threshold(mut self, n: u32) -> Self {
        self.open_threshold = n;
        self
    }

    #[must_use]
    pub fn with_recovery_window_ms(mut self, ms: u64) -> Self {
        self.recovery_window_ms = ms;
        self
    }

    /// Override the clock used to compute recovery-window elapse.
    /// Intended for tests that exercise time-based transitions.
    #[must_use]
    pub fn with_clock<F>(mut self, clock: F) -> Self
    where
        F: Fn() -> DateTime<Utc> + Send + Sync + 'static,
    {
        self.clock = Box::new(clock);
        self
    }

    /// Override the human-readable subscriber type name embedded in
    /// `apcore.subscriber.*` event payloads.
    #[must_use]
    pub fn with_subscriber_type_name(mut self, name: impl Into<String>) -> Self {
        self.subscriber_type_name = name.into();
        self
    }

    /// Snapshot the current circuit state.
    #[must_use]
    pub fn state(&self) -> CircuitState {
        self.state.lock().phase
    }

    /// Snapshot the current consecutive-failure count.
    #[must_use]
    pub fn consecutive_failures(&self) -> u32 {
        self.state.lock().consecutive_failures
    }

    /// Last recorded failure timestamp, or `None` if there has been no failure
    /// yet (or the breaker has fully recovered to CLOSED).
    #[must_use]
    pub fn last_failure_at(&self) -> Option<DateTime<Utc>> {
        self.state.lock().last_failure_at
    }

    /// Force the circuit state. Public for tests + callers that need to
    /// hydrate state from external storage; no public guarantees about safety
    /// during concurrent delivery.
    pub fn force_state(&self, state: CircuitState) {
        self.state.lock().transition(state);
    }

    /// Force the recorded `last_failure_at`. Public for tests.
    pub fn force_last_failure_at(&self, t: Option<DateTime<Utc>>) {
        self.state.lock().last_failure_at = t;
    }

    /// Force the consecutive-failure counter. Public for tests.
    pub fn force_consecutive_failures(&self, n: u32) {
        self.state.lock().consecutive_failures = n;
    }

    /// Transition `OPEN → HALF_OPEN` if `recovery_window_ms` has elapsed since
    /// `last_failure_at`. Always called at the start of `on_event`; exposed
    /// publicly so tests can probe the transition without invoking delivery.
    pub fn check_recovery(&self) {
        let now = (self.clock)();
        self.update_recovery(&mut self.state.lock(), now);
    }

    fn update_recovery(&self, circuit: &mut DeliveryCircuit, now: DateTime<Utc>) {
        if circuit.phase != CircuitState::Open {
            return;
        }
        if let Some(last) = circuit.last_failure_at {
            let elapsed = now.signed_duration_since(last).num_milliseconds();
            if u64::try_from(elapsed).is_ok_and(|elapsed| elapsed >= self.recovery_window_ms) {
                circuit.transition(CircuitState::HalfOpen);
            }
        }
    }

    fn admit_delivery(&self) -> Option<DeliveryAdmission<'_>> {
        let now = (self.clock)();
        let mut circuit = self.state.lock();
        self.update_recovery(&mut circuit, now);
        if circuit.phase == CircuitState::Open || circuit.probe_in_flight {
            return None;
        }
        let probe = circuit.phase == CircuitState::HalfOpen;
        circuit.probe_in_flight = probe;
        Some(DeliveryAdmission {
            circuit: &self.state,
            generation: circuit.generation,
            probe,
        })
    }

    fn on_success(&self, admission: &DeliveryAdmission<'_>) -> Option<ApCoreEvent> {
        {
            let mut circuit = self.state.lock();
            if circuit.generation != admission.generation {
                return None;
            }
            circuit.consecutive_failures = 0;
            circuit.last_failure_at = None;
            if !admission.probe {
                return None;
            }
            circuit.transition(CircuitState::Closed);
        }
        Some(self.make_event(
            "apcore.subscriber.circuit_closed",
            "info",
            json!({
                "subscriber_id": self.subscriber.subscriber_id(),
                "subscriber_type": self.subscriber_type_name,
                "recovery_attempt": true,
            }),
        ))
    }

    fn on_failure(
        &self,
        admission: &DeliveryAdmission<'_>,
        error: &ModuleError,
    ) -> Option<ApCoreEvent> {
        let now = (self.clock)();
        let count = {
            let mut circuit = self.state.lock();
            if circuit.generation != admission.generation {
                return None;
            }
            circuit.consecutive_failures = circuit.consecutive_failures.saturating_add(1);
            circuit.last_failure_at = Some(now);
            if !admission.probe && circuit.consecutive_failures < self.open_threshold {
                return None;
            }
            circuit.transition(CircuitState::Open);
            circuit.consecutive_failures
        };
        tracing::warn!(
            subscriber_type = %self.subscriber_type_name,
            consecutive_failures = count,
            error = %error,
            "circuit opened for subscriber"
        );
        Some(self.make_event(
            "apcore.subscriber.circuit_opened",
            "warn",
            json!({
                "subscriber_id": self.subscriber.subscriber_id(),
                "subscriber_type": self.subscriber_type_name,
                "consecutive_failures": count,
            }),
        ))
    }

    fn make_event(&self, event_type: &str, severity: &str, data: serde_json::Value) -> ApCoreEvent {
        ApCoreEvent {
            event_type: event_type.to_string(),
            timestamp: (self.clock)().to_rfc3339(),
            data,
            module_id: None,
            severity: severity.to_string(),
        }
    }

    fn dispatch_circuit_event(&self, event: ApCoreEvent) {
        self.sink.emit_circuit_event(event);
    }
}

#[async_trait]
impl EventSubscriber for CircuitBreakerWrapper {
    fn subscriber_id(&self) -> &str {
        self.subscriber.subscriber_id()
    }

    fn event_pattern(&self) -> &str {
        self.subscriber.event_pattern()
    }

    fn event_type_filter(&self) -> Option<&str> {
        self.subscriber.event_type_filter()
    }

    fn subscriber_type(&self) -> &str {
        self.subscriber.subscriber_type()
    }

    fn retry(&self) -> EventRetryConfig {
        self.subscriber.retry()
    }

    async fn on_failure(&self, event: &ApCoreEvent, error: &ModuleError, attempt_count: u32) {
        self.subscriber
            .on_failure(event, error, attempt_count)
            .await;
    }

    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        let _ = self.delivery_attempt(event).await;
        Ok(())
    }

    async fn delivery_attempt(&self, event: &ApCoreEvent) -> Option<Result<(), ModuleError>> {
        let admission = self.admit_delivery()?;
        let timeout = Duration::from_millis(self.timeout_ms);
        let outcome = tokio::time::timeout(timeout, self.subscriber.on_event(event))
            .await
            .unwrap_or_else(|_| {
                Err(ModuleError::new(
                    ErrorCode::ModuleTimeout,
                    "Subscriber delivery timed out",
                ))
            });

        let circuit_event = match &outcome {
            Ok(()) => self.on_success(&admission),
            Err(error) => self.on_failure(&admission, error),
        };

        if let Some(ev) = circuit_event {
            self.dispatch_circuit_event(ev);
        }
        Some(outcome)
    }
}

/// Best-effort subscriber-type label for `apcore.subscriber.*` payloads.
///
/// We don't have access to the concrete generic type name at runtime, but the
/// `subscriber_id` carries a deterministic prefix for built-in types
/// (`webhook-…`, `a2a-…`, `file-…`, `stdout-…`, `filter-…`). Use that prefix
/// when present; otherwise fall back to the literal id.
/// The DECLARED subscriber kind, the same value the DLQ path reports (D-116).
///
/// This used to split `subscriber_id` on the first hyphen, so `health-alert`
/// became the type `health` and an id with no hyphen became the whole id —
/// neither the declared kind nor what this same SDK puts in its own
/// `apcore.event.delivery_failed` payload, which reads
/// `EventSubscriber::subscriber_type`. A consumer routing on `subscriber_type`
/// therefore got one answer for a delivery failure and a different one for the
/// circuit opening on the SAME subscriber.
///
/// An undeclared subscriber takes the trait's existing default; no second
/// default is invented for this surface, which the decision forbids and which
/// is how the three SDKs came to disagree three ways.
fn declared_subscriber_type(subscriber: &dyn EventSubscriber) -> String {
    subscriber.subscriber_type().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Default)]
    struct RecordingSink {
        events: StdMutex<Vec<ApCoreEvent>>,
    }

    impl RecordingSink {
        fn captured(&self) -> Vec<ApCoreEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl CircuitEventSink for RecordingSink {
        fn emit_circuit_event(&self, event: ApCoreEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[derive(Debug)]
    struct AlwaysFail {
        id: String,
    }

    #[async_trait]
    impl EventSubscriber for AlwaysFail {
        fn subscriber_id(&self) -> &str {
            &self.id
        }
        #[allow(clippy::unnecessary_literal_bound)]
        fn event_pattern(&self) -> &str {
            "*"
        }
        async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
            Err(ModuleError::new(ErrorCode::GeneralInternalError, "boom"))
        }
    }

    #[derive(Debug)]
    struct AlwaysOk {
        id: String,
    }

    #[async_trait]
    impl EventSubscriber for AlwaysOk {
        fn subscriber_id(&self) -> &str {
            &self.id
        }
        #[allow(clippy::unnecessary_literal_bound)]
        fn event_pattern(&self) -> &str {
            "*"
        }
        async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
            Ok(())
        }
    }

    fn make_event() -> ApCoreEvent {
        ApCoreEvent::new("test.event", json!({}))
    }

    #[tokio::test]
    async fn opens_after_threshold() {
        let sink = Arc::new(RecordingSink::default());
        let wrapper = CircuitBreakerWrapper::new(
            Box::new(AlwaysFail {
                id: "webhook-x".into(),
            }),
            sink.clone(),
        )
        .with_open_threshold(3)
        .with_subscriber_type_name("webhook");

        let event = make_event();
        for _ in 0..3 {
            wrapper.on_event(&event).await.unwrap();
        }

        assert_eq!(wrapper.state(), CircuitState::Open);
        assert_eq!(wrapper.consecutive_failures(), 3);
        let captured = sink.captured();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].event_type, "apcore.subscriber.circuit_opened");
    }

    #[tokio::test]
    async fn emitter_uses_wrapped_retry_and_failure_callback() {
        use crate::events::emitter::EventEmitter;
        use crate::events::retry::EventRetryConfig;

        #[derive(Debug, Default)]
        struct RetryCounts {
            delivered: AtomicU32,
            exhausted: AtomicU32,
        }

        #[derive(Debug)]
        struct RecordingFailure(Arc<RetryCounts>);

        #[async_trait]
        impl EventSubscriber for RecordingFailure {
            fn retry(&self) -> EventRetryConfig {
                EventRetryConfig {
                    max_attempts: 2,
                    ..EventRetryConfig::no_retry()
                }
            }

            async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
                self.0.delivered.fetch_add(1, Ordering::SeqCst);
                Err(ModuleError::new(ErrorCode::GeneralInternalError, "offline"))
            }

            async fn on_failure(&self, _event: &ApCoreEvent, error: &ModuleError, attempts: u32) {
                assert_eq!(error.message, "offline");
                self.0.exhausted.store(attempts, Ordering::SeqCst);
            }
        }

        let counts = Arc::new(RetryCounts::default());
        let emitter = EventEmitter::new();
        emitter.subscribe(Box::new(CircuitBreakerWrapper::new(
            Box::new(RecordingFailure(counts.clone())),
            Arc::new(RecordingSink::default()),
        )));
        emitter.emit(&make_event()).await;
        emitter.flush(0).await.unwrap();
        assert_eq!(counts.delivered.load(Ordering::SeqCst), 2);
        assert_eq!(counts.exhausted.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn open_state_discards_without_delivery() {
        let sink = Arc::new(RecordingSink::default());
        let wrapper = CircuitBreakerWrapper::new(
            Box::new(AlwaysFail {
                id: "webhook-x".into(),
            }),
            sink.clone(),
        );
        wrapper.force_state(CircuitState::Open);
        wrapper.force_last_failure_at(Some(Utc::now()));

        let event = make_event();
        wrapper.on_event(&event).await.unwrap();
        assert_eq!(wrapper.state(), CircuitState::Open);
        // No delivery attempted ⇒ no failure recorded.
        assert_eq!(wrapper.consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn half_open_on_success_closes_and_emits() {
        let sink = Arc::new(RecordingSink::default());
        let wrapper = CircuitBreakerWrapper::new(
            Box::new(AlwaysOk {
                id: "webhook-x".into(),
            }),
            sink.clone(),
        );
        wrapper.force_state(CircuitState::HalfOpen);

        wrapper.on_event(&make_event()).await.unwrap();
        assert_eq!(wrapper.state(), CircuitState::Closed);
        assert_eq!(wrapper.consecutive_failures(), 0);
        let captured = sink.captured();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].event_type, "apcore.subscriber.circuit_closed");
    }

    #[tokio::test]
    async fn half_open_admits_one_subscriber_delivery() {
        #[derive(Debug, Default)]
        struct BlockingDelivery {
            started: tokio::sync::Notify,
            release: tokio::sync::Notify,
        }
        #[async_trait]
        impl EventSubscriber for Arc<BlockingDelivery> {
            async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
                self.started.notify_one();
                self.release.notified().await;
                Ok(())
            }
        }
        let subscriber = Arc::new(BlockingDelivery::default());
        let wrapper = Arc::new(CircuitBreakerWrapper::new(
            Box::new(subscriber.clone()),
            Arc::new(RecordingSink::default()),
        ));
        wrapper.force_state(CircuitState::HalfOpen);
        let probe = wrapper.clone();
        let running = tokio::spawn(async move { probe.on_event(&make_event()).await });
        subscriber.started.notified().await;
        tokio::time::timeout(Duration::from_millis(20), wrapper.on_event(&make_event()))
            .await
            .expect("half-open circuit discards additional deliveries")
            .unwrap();
        subscriber.release.notify_one();
        running.await.unwrap().unwrap();
        assert_eq!(wrapper.state(), CircuitState::Closed);
    }

    #[tokio::test]
    async fn check_recovery_transitions_open_to_half_open() {
        let wrapper = CircuitBreakerWrapper::new(
            Box::new(AlwaysOk {
                id: "webhook-x".into(),
            }),
            Arc::new(RecordingSink::default()),
        )
        .with_recovery_window_ms(30_000);
        wrapper.force_state(CircuitState::Open);
        let last = Utc::now() - chrono::Duration::seconds(31);
        wrapper.force_last_failure_at(Some(last));
        wrapper.check_recovery();
        assert_eq!(wrapper.state(), CircuitState::HalfOpen);
    }

    /// Subscriber that fails while the shared `fail` flag is set, then
    /// succeeds once it is cleared. Sharing the flag via `Arc` lets the test
    /// drive a full OPEN → HALF_OPEN → CLOSED cycle through one wrapper.
    #[derive(Debug)]
    struct Toggling {
        id: String,
        fail: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl EventSubscriber for Toggling {
        fn subscriber_id(&self) -> &str {
            &self.id
        }
        #[allow(clippy::unnecessary_literal_bound)]
        fn event_pattern(&self) -> &str {
            "*"
        }
        async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
            if self.fail.load(Ordering::SeqCst) {
                Err(ModuleError::new(ErrorCode::GeneralInternalError, "boom"))
            } else {
                Ok(())
            }
        }
    }

    // A-D-07: tripping and then recovering the circuit MUST emit BOTH the
    // circuit_opened and circuit_closed lifecycle events. Previously the sink
    // was optional; a None sink transitioned state silently, violating the
    // spec MUST.
    #[tokio::test]
    async fn tripping_then_recovering_emits_both_events() {
        let sink = Arc::new(RecordingSink::default());
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let subscriber = Box::new(Toggling {
            id: "webhook-x".into(),
            fail: fail.clone(),
        });
        let wrapper = CircuitBreakerWrapper::new(subscriber, sink.clone())
            .with_open_threshold(2)
            .with_recovery_window_ms(30_000);

        let event = make_event();
        // Trip the circuit OPEN.
        wrapper.on_event(&event).await.unwrap();
        wrapper.on_event(&event).await.unwrap();
        assert_eq!(wrapper.state(), CircuitState::Open);

        // Advance past the recovery window so the next delivery probes HALF_OPEN,
        // and stop failing so the probe succeeds and closes the circuit.
        wrapper.force_last_failure_at(Some(Utc::now() - chrono::Duration::seconds(31)));
        fail.store(false, Ordering::SeqCst);
        wrapper.on_event(&event).await.unwrap();
        assert_eq!(wrapper.state(), CircuitState::Closed);

        let captured = sink.captured();
        assert!(
            captured
                .iter()
                .any(|e| e.event_type == "apcore.subscriber.circuit_opened"),
            "expected circuit_opened, got {captured:?}"
        );
        assert!(
            captured
                .iter()
                .any(|e| e.event_type == "apcore.subscriber.circuit_closed"),
            "expected circuit_closed, got {captured:?}"
        );
    }
}
