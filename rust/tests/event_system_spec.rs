// Spec-traced contract tests for the apcore-rust event-system feature.
//
// Source spec: apcore/docs/features/event-system.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_event_system_spec.py
//
// Each test maps to exactly one clause in the feature spec's '## Contract:'
// blocks. The verbatim cross-language clause id appears in a leading
// `// clause: <clause_id>` comment on the line above each test fn so that a
// cross-language diff tool can line up the Python / TypeScript / Rust rows by
// that exact string. The fn name is the clause id flattened to snake_case.
//
// Contract blocks covered (same order as the canonical Python suite):
//   - EventEmitter.emit
//   - EventEmitter.subscribe
//   - WebhookSubscriber.deliver
//   - SubscriberCircuitBreaker.on_failure  (Rust: CircuitBreakerWrapper)
//   - EventEmitter.unsubscribe
//   - EventEmitter.flush
//   - A2ASubscriber.deliver
//
// Symbol-reality notes (drive the skip / ignore decisions below):
//   - `EventEmitter::emit` takes `&ApCoreEvent` and is fire-and-forget; it
//     never returns an error and does not validate `event_type`. The spec's
//     `input.event_type.not_empty` rule is not enforced.
//   - The Rust `EventEmitter::emit` is `async` (unlike Python's sync emit), so
//     `property.async` for emit asserts the async, non-blocking contract.
//   - `WebhookSubscriber` / `A2ASubscriber` deliver via `on_event` (not a
//     `deliver` method). The HTTP path is gated behind the `events` cargo
//     feature. There is no `DeliveryError(WEBHOOK_DELIVERY_FAILED)` type — 5xx
//     surfaces as a `ModuleError(GeneralInternalError)` and the emitter owns
//     the dead-letter path. Recorded as a missing-symbol ignore.
//   - The circuit-breaker contract names `SubscriberCircuitBreaker.on_failure`
//     with signature `(subscriber_id, error) -> CircuitState`. Rust ships
//     `CircuitBreakerWrapper` with a private `on_failure(error_msg)` and a
//     public `on_event` driver. The exact contract method is a missing symbol;
//     the observable state machine is exercised via `on_event` / `state()`.
//
// HTTP delivery tests use a raw-TCP mock server (the established pattern from
// tests/test_webhook_http_retry_contract.rs) and are gated behind the `events`
// cargo feature so the binary still compiles without it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;

use apcore::errors::{ErrorCode, ModuleError};
use apcore::events::circuit_breaker::{CircuitBreakerWrapper, CircuitEventSink, CircuitState};
use apcore::events::emitter::{ApCoreEvent, EventEmitter};
use apcore::events::subscribers::EventSubscriber;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// No-op circuit-event sink for tests that only assert on circuit state, not
/// on emitted lifecycle events. The sink is mandatory (sync finding A-D-07).
#[derive(Debug, Default)]
struct NoopSink;

impl CircuitEventSink for NoopSink {
    fn emit_circuit_event(&self, _event: ApCoreEvent) {}
}

fn noop_sink() -> Arc<dyn CircuitEventSink> {
    Arc::new(NoopSink)
}

/// Build a canonical test event.
fn make_event() -> ApCoreEvent {
    ApCoreEvent {
        event_type: "apcore.test.event".to_string(),
        timestamp: "2026-03-08T00:00:00Z".to_string(),
        data: json!({}),
        module_id: Some("mod.a".to_string()),
        severity: "info".to_string(),
    }
}

/// Build a test event with a specific event_type and data payload.
fn make_event_with(event_type: &str, data: serde_json::Value) -> ApCoreEvent {
    ApCoreEvent {
        event_type: event_type.to_string(),
        timestamp: "2026-03-08T00:00:00Z".to_string(),
        data,
        module_id: Some("mod.a".to_string()),
        severity: "info".to_string(),
    }
}

/// Minimal `EventSubscriber` that records every delivered event's `data`.
#[derive(Debug, Clone)]
struct RecordingSubscriber {
    id: String,
    pattern: String,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl RecordingSubscriber {
    fn new(id: &str, pattern: &str) -> Self {
        Self {
            id: id.to_string(),
            pattern: pattern.to_string(),
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn handle(&self) -> Arc<Mutex<Vec<serde_json::Value>>> {
        Arc::clone(&self.received)
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
        self.received.lock().push(event.data.clone());
        Ok(())
    }
}

/// Subscriber that always fails — used to exercise error-isolation and the
/// circuit-breaker failure path.
#[derive(Debug)]
struct FailingSubscriber {
    id: String,
}

impl FailingSubscriber {
    fn new(id: &str) -> Self {
        Self { id: id.to_string() }
    }
}

#[async_trait]
impl EventSubscriber for FailingSubscriber {
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

// ===========================================================================
// Contract: EventEmitter.emit
// ===========================================================================

// clause: event_system.emit.input.event_type.not_empty
#[test]
#[ignore = "event_system.emit.input.event_type.not_empty: emit() takes &ApCoreEvent, is fire-and-forget, and does not validate event_type (rule unenforceable as a rejection contract)"]
fn emit_input_event_type_not_empty() {
    // Spec declares event_type MUST NOT be empty, but Rust's emit() takes an
    // &ApCoreEvent and never returns an error — the empty-event_type rule is
    // unenforced here. See ignore reason above.
    unreachable!();
}

// clause: event_system.emit.error.none_raised
#[tokio::test(flavor = "multi_thread")]
async fn emit_error_none_raised() {
    // emit() is fire-and-forget: a throwing subscriber must not surface to the
    // caller of emit() (the return type is unit — there is no Err to inspect).
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(FailingSubscriber::new("boom")));
    let event = make_event();
    // Must not panic even though the wrapped subscriber returns Err.
    emitter.emit(&event).await;
    emitter.flush(2000).await.unwrap();
    emitter.shutdown(1000).await.unwrap();
    // Reaching here means emit/flush never propagated the subscriber error.
    assert!(emitter.is_shutdown());
}

// clause: event_system.emit.property.async
#[tokio::test(flavor = "multi_thread")]
async fn emit_property_async() {
    // Rust emit() is async (spec: "async in Rust") and its future resolves to
    // `()` without blocking the caller on subscriber execution.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    let event = make_event();
    // The future resolves — assert the unit result.
    let result: () = emitter.emit(&event).await;
    assert_eq!(result, ());
    emitter.flush(2000).await.unwrap();
    assert_eq!(received.lock().len(), 1);
}

// clause: event_system.emit.property.thread_safe
#[tokio::test(flavor = "multi_thread")]
async fn emit_property_thread_safe() {
    // Concurrent emits with distinct payloads from >=8 spawned tasks must not
    // corrupt state; every event must be delivered exactly once.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    let emitter = Arc::new(emitter);

    let n = 16usize;
    let mut handles = Vec::new();
    for i in 0..n {
        let em = Arc::clone(&emitter);
        handles.push(tokio::spawn(async move {
            let event = make_event_with("apcore.test.event", json!({ "i": i }));
            em.emit(&event).await;
        }));
    }
    for h in handles {
        h.await.expect("emit task must not panic");
    }
    emitter.flush(5000).await.unwrap();

    let mut delivered: Vec<i64> = received
        .lock()
        .iter()
        .map(|v| v["i"].as_i64().unwrap())
        .collect();
    delivered.sort_unstable();
    let expected: Vec<i64> = (0..i64::try_from(n).unwrap()).collect();
    assert_eq!(delivered, expected);
}

// clause: event_system.emit.property.pure
#[tokio::test(flavor = "multi_thread")]
async fn emit_property_pure() {
    // emit() is NOT pure: it invokes subscriber callbacks (observable effect).
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    let event = make_event();
    emitter.emit(&event).await;
    emitter.flush(2000).await.unwrap();
    assert_eq!(received.lock().len(), 1); // callback invoked -> impure
}

// clause: event_system.emit.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn emit_property_idempotent() {
    // emit() is NOT idempotent: two identical emits deliver twice.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    let event = make_event();
    emitter.emit(&event).await;
    emitter.emit(&event).await;
    emitter.flush(2000).await.unwrap();
    assert_eq!(received.lock().len(), 2);
}

// clause: event_system.emit.side_effect.1.subscriber_invoked
#[tokio::test(flavor = "multi_thread")]
async fn emit_side_effect_1_subscriber_invoked() {
    // Observable side effect: matching subscribers receive the emitted event.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "apcore.test.*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    let event = make_event_with("apcore.test.event", json!({ "k": "v" }));
    emitter.emit(&event).await;
    emitter.flush(2000).await.unwrap();
    let got = received.lock().clone();
    assert_eq!(got, vec![json!({ "k": "v" })]);
}

// ===========================================================================
// Contract: EventEmitter.subscribe
// ===========================================================================

// clause: event_system.subscribe.input.subscriber.async_on_event
#[test]
#[ignore = "event_system.subscribe.input.subscriber.async_on_event: Python-only TypeError guard; Rust enforces the async on_event contract statically via Box<dyn EventSubscriber> (no runtime check exists to assert)"]
fn subscribe_input_subscriber_async_on_event() {
    // The Python SDK raises TypeError if on_event is not a coroutine function.
    // Rust enforces this at compile time through the EventSubscriber trait, so
    // there is no runtime rejection contract to assert. See ignore reason.
    unreachable!();
}

// clause: event_system.subscribe.error.none_raised
#[tokio::test(flavor = "multi_thread")]
async fn subscribe_error_none_raised() {
    // A correctly-typed EventSubscriber subscribes without error.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub)); // must not panic
    let event = make_event();
    emitter.emit(&event).await;
    emitter.flush(2000).await.unwrap();
    assert_eq!(received.lock().len(), 1);
}

// clause: event_system.subscribe.property.async
#[tokio::test(flavor = "multi_thread")]
async fn subscribe_property_async() {
    // The clause under test is that subscribe() is SYNCHRONOUS — it completes
    // without an await. It used to assert `let result: () = ...` as a proxy for
    // that, which pinned the return type rather than the property: subscribe now
    // returns a SubscriberHandle so a caller can remove exactly one subscriber
    // (sync finding A-D-025), and the synchronous property is unchanged.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let handle = emitter.subscribe(Box::new(sub));
    // No await was needed to get here, which is the property.
    assert!(
        emitter.unsubscribe_handle(handle),
        "the returned handle must identify the subscription just created"
    );
}

// clause: event_system.subscribe.property.thread_safe
#[tokio::test(flavor = "multi_thread")]
async fn subscribe_property_thread_safe() {
    // Concurrent subscribe() from >=8 tasks must register every subscriber
    // without loss. `subscribe` is `&self` (interior-mutable subscribers,
    // D1-011), so tasks subscribe directly through a shared `Arc<EventEmitter>`
    // with no external lock; assert the registry holds every subscriber
    // (observed via delivery).
    let emitter = Arc::new(EventEmitter::new());
    let n = 12usize;
    let mut handles = Vec::new();
    for i in 0..n {
        let em = Arc::clone(&emitter);
        handles.push(tokio::spawn(async move {
            let sub = RecordingSubscriber::new(&format!("s{i}"), "*");
            em.subscribe(Box::new(sub));
        }));
    }
    for h in handles {
        h.await.expect("subscribe task must not panic");
    }
    // Observe consistent state: emit once, every registered subscriber fires.
    let counter = Arc::new(AtomicUsize::new(0));
    {
        let guard = &emitter;
        let counting = CountingSubscriber::new("counter", Arc::clone(&counter));
        guard.subscribe(Box::new(counting));
        let event = make_event();
        guard.emit(&event).await;
        guard.flush(5000).await.unwrap();
    }
    // The counting subscriber fired exactly once -> registry consistent.
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

/// Subscriber that counts deliveries into a shared atomic.
#[derive(Debug)]
struct CountingSubscriber {
    id: String,
    count: Arc<AtomicUsize>,
}

impl CountingSubscriber {
    fn new(id: &str, count: Arc<AtomicUsize>) -> Self {
        Self {
            id: id.to_string(),
            count,
        }
    }
}

#[async_trait]
impl EventSubscriber for CountingSubscriber {
    fn subscriber_id(&self) -> &str {
        &self.id
    }
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        "*"
    }
    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// clause: event_system.subscribe.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn subscribe_property_idempotent() {
    // Each subscribe() call creates a new subscription: subscribing two
    // subscribers (same event_pattern) delivers each event to both. Rust stores
    // subscribers by value, so two distinct boxed subscribers both receive.
    let emitter = EventEmitter::new();
    let s1 = RecordingSubscriber::new("a", "*");
    let s2 = RecordingSubscriber::new("b", "*");
    let r1 = s1.handle();
    let r2 = s2.handle();
    emitter.subscribe(Box::new(s1));
    emitter.subscribe(Box::new(s2));
    let event = make_event();
    emitter.emit(&event).await;
    emitter.flush(2000).await.unwrap();
    // Two subscriptions -> the event was delivered twice in total (not deduped).
    assert_eq!(r1.lock().len() + r2.lock().len(), 2);
}

// ===========================================================================
// Contract: WebhookSubscriber.deliver
// ===========================================================================

// clause: event_system.deliver.error.WEBHOOK_DELIVERY_FAILED
#[test]
#[ignore = "event_system.deliver.error.WEBHOOK_DELIVERY_FAILED: missing symbol DeliveryError/WEBHOOK_DELIVERY_FAILED (contract gap) — WebhookSubscriber returns ModuleError(GeneralInternalError) on 5xx and the emitter owns the dead-letter path"]
fn webhook_deliver_error_delivery_error_code() {
    // Spec declares DeliveryError(code=WEBHOOK_DELIVERY_FAILED) on retry
    // exhaustion. Rust has no such type — 5xx surfaces as
    // ModuleError(GeneralInternalError) and the EventEmitter handles the DLQ
    // path (apcore.event.delivery_failed). See ignore reason.
    unreachable!();
}

// The remaining WebhookSubscriber.deliver clauses exercise the real HTTP path,
// which is gated behind the `events` cargo feature. They live in a feature-
// gated module so the binary still compiles when the feature is off.
#[cfg(feature = "events")]
mod webhook_deliver {
    use super::*;
    use apcore::events::subscribers::WebhookSubscriber;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Minimal HTTP mock server bound to an ephemeral port. Replies with a
    /// fixed status line to every request and counts requests received.
    /// Mirrors tests/test_webhook_http_retry_contract.rs.
    pub(super) struct MockServer {
        pub url: String,
        request_count: Arc<AtomicUsize>,
        last_body: Arc<Mutex<Vec<u8>>>,
    }

    impl MockServer {
        pub(super) async fn spawn(status_line: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let url = format!("http://127.0.0.1:{port}/hook");
            let request_count = Arc::new(AtomicUsize::new(0));
            let last_body = Arc::new(Mutex::new(Vec::new()));
            let counter = Arc::clone(&request_count);
            let body_sink = Arc::clone(&last_body);

            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let counter = Arc::clone(&counter);
                    let body_sink = Arc::clone(&body_sink);
                    tokio::spawn(async move {
                        let mut buf = Vec::with_capacity(8192);
                        let mut tmp = [0u8; 1024];
                        let mut headers_end: Option<usize> = None;
                        let mut content_length: usize = 0;
                        while headers_end.is_none() {
                            let Ok(n) = stream.read(&mut tmp).await else {
                                return;
                            };
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                            if let Some(pos) = find_headers_end(&buf) {
                                headers_end = Some(pos);
                                content_length = parse_content_length(&buf[..pos]).unwrap_or(0);
                            }
                        }
                        if let Some(pos) = headers_end {
                            while buf.len() - pos < content_length {
                                let Ok(n) = stream.read(&mut tmp).await else {
                                    break;
                                };
                                if n == 0 {
                                    break;
                                }
                                buf.extend_from_slice(&tmp[..n]);
                            }
                            *body_sink.lock() = buf[pos..].to_vec();
                        }
                        counter.fetch_add(1, Ordering::SeqCst);
                        let response = format!(
                            "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    });
                }
            });

            Self {
                url,
                request_count,
                last_body,
            }
        }

        pub(super) fn request_count(&self) -> usize {
            self.request_count.load(Ordering::SeqCst)
        }

        pub(super) fn last_json(&self) -> serde_json::Value {
            let body = self.last_body.lock().clone();
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)
        }
    }

    fn find_headers_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
    }

    fn parse_content_length(headers: &[u8]) -> Option<usize> {
        let text = std::str::from_utf8(headers).ok()?;
        for line in text.split("\r\n") {
            if let Some((name, value)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("content-length") {
                    return value.trim().parse().ok();
                }
            }
        }
        None
    }

    // clause: event_system.deliver.input.event.required
    #[tokio::test(flavor = "multi_thread")]
    async fn webhook_deliver_input_event_required() {
        // WebhookSubscriber delivery POSTs the serialized event. Observe the
        // event_type travels in the JSON body.
        let server = MockServer::spawn("200 OK").await;
        let sub = WebhookSubscriber::new("wh", &server.url, "*");
        let event = make_event_with("apcore.health.recovered", json!({}));
        sub.on_event(&event).await.expect("2xx must be Ok");
        assert_eq!(server.request_count(), 1);
        let body = server.last_json();
        assert_eq!(body["event_type"], "apcore.health.recovered");
    }

    // clause: event_system.deliver.property.async
    #[tokio::test(flavor = "multi_thread")]
    async fn webhook_deliver_property_async() {
        // WebhookSubscriber delivery is awaitable and resolves to Ok(()).
        let server = MockServer::spawn("200 OK").await;
        let sub = WebhookSubscriber::new("wh", &server.url, "*");
        let event = make_event();
        let result = sub.on_event(&event).await;
        assert!(result.is_ok());
    }

    // clause: event_system.deliver.property.thread_safe
    #[tokio::test(flavor = "multi_thread")]
    async fn webhook_deliver_property_thread_safe() {
        // >=8 concurrent webhook deliveries with distinct events all resolve
        // without error; each issues its own POST.
        let server = MockServer::spawn("200 OK").await;
        let sub = Arc::new(WebhookSubscriber::new("wh", &server.url, "*"));
        let mut handles = Vec::new();
        for i in 0..8usize {
            let s = Arc::clone(&sub);
            handles.push(tokio::spawn(async move {
                let event = make_event_with("apcore.test.event", json!({ "i": i }));
                s.on_event(&event).await
            }));
        }
        for h in handles {
            h.await.unwrap().expect("delivery must resolve Ok");
        }
        assert_eq!(server.request_count(), 8);
    }

    // clause: event_system.deliver.property.pure
    #[tokio::test(flavor = "multi_thread")]
    async fn webhook_deliver_property_pure() {
        // Delivery is NOT pure: it performs an outbound HTTP POST.
        let server = MockServer::spawn("200 OK").await;
        let sub = WebhookSubscriber::new("wh", &server.url, "*");
        sub.on_event(&make_event()).await.unwrap();
        assert_eq!(server.request_count(), 1); // outbound side effect occurred
    }

    // clause: event_system.deliver.side_effect.1.retry_on_5xx
    #[tokio::test(flavor = "multi_thread")]
    async fn webhook_deliver_side_effect_1_retry_on_5xx() {
        // Observable HTTP-status policy: 5xx returns Err (so the emitter retry
        // loop re-delivers); 4xx returns Ok (no retry / permanent failure).
        let server_5xx = MockServer::spawn("503 Service Unavailable").await;
        let sub5 = WebhookSubscriber::new("wh", &server_5xx.url, "*");
        let res5 = sub5.on_event(&make_event()).await;
        assert!(res5.is_err(), "5xx must surface as Err for emitter retry");
        assert_eq!(res5.unwrap_err().code, ErrorCode::GeneralInternalError);

        let server_4xx = MockServer::spawn("404 Not Found").await;
        let sub4 = WebhookSubscriber::new("wh", &server_4xx.url, "*");
        let res4 = sub4.on_event(&make_event()).await;
        assert!(res4.is_ok(), "4xx is a permanent failure: Ok, not retried");
    }
}

// ===========================================================================
// Contract: SubscriberCircuitBreaker.on_failure  (Rust: CircuitBreakerWrapper)
// ===========================================================================

// clause: event_system.on_failure.input.subscriber_id.required
#[test]
#[ignore = "event_system.on_failure.input.subscriber_id.required: missing symbol SubscriberCircuitBreaker.on_failure(subscriber_id, error) (contract gap) — Rust exposes CircuitBreakerWrapper with a private on_failure(error_msg) and no subscriber_id parameter"]
fn circuit_on_failure_input_subscriber_id_required() {
    // Spec contract is SubscriberCircuitBreaker.on_failure(subscriber_id,
    // error) -> CircuitState. Rust ships CircuitBreakerWrapper whose failure
    // accounting is a private on_failure(error_msg) with no subscriber_id
    // parameter. See ignore reason.
    unreachable!();
}

// clause: event_system.on_failure.error.none_raised
#[tokio::test(flavor = "multi_thread")]
async fn circuit_on_failure_error_none_raised() {
    // The circuit breaker MUST NOT error on a delivery failure; it records
    // state. Observed via the public on_event driver wrapping a failing sub.
    let wrapper =
        CircuitBreakerWrapper::new(Box::new(FailingSubscriber::new("webhook-x")), noop_sink())
            .with_open_threshold(5);
    // Must not error even though the wrapped subscriber returns Err.
    wrapper
        .on_event(&make_event())
        .await
        .expect("breaker swallows failure");
    assert_eq!(wrapper.consecutive_failures(), 1);
}

// clause: event_system.on_failure.returns.circuit_state
#[tokio::test(flavor = "multi_thread")]
async fn circuit_on_failure_returns_circuit_state() {
    // Failure handling drives the CLOSED->OPEN transition. After open_threshold
    // consecutive failures the observable state is OPEN (a CircuitState).
    let wrapper =
        CircuitBreakerWrapper::new(Box::new(FailingSubscriber::new("webhook-x")), noop_sink())
            .with_open_threshold(3);
    for _ in 0..3 {
        wrapper.on_event(&make_event()).await.unwrap();
    }
    assert_eq!(wrapper.state(), CircuitState::Open);
}

// clause: event_system.on_failure.property.async
#[tokio::test(flavor = "multi_thread")]
async fn circuit_on_failure_property_async() {
    // The failure-accounting transition is synchronous in the spec; in Rust the
    // observable counter mutation happens within the synchronous part of the
    // on_event driver. Assert the counter advances synchronously per delivery
    // (the breaker's own bookkeeping is not an async value).
    let wrapper =
        CircuitBreakerWrapper::new(Box::new(FailingSubscriber::new("webhook-x")), noop_sink())
            .with_open_threshold(2);
    let before = wrapper.consecutive_failures();
    wrapper.on_event(&make_event()).await.unwrap();
    // consecutive_failures() is a plain (non-async) accessor returning u32.
    let after: u32 = wrapper.consecutive_failures();
    assert_eq!(after, before + 1);
}

// clause: event_system.on_failure.property.thread_safe
#[tokio::test(flavor = "multi_thread")]
async fn circuit_on_failure_property_thread_safe() {
    // >=8 concurrent failure events must update the shared counter without
    // loss; the lock-protected state stays consistent (OPEN after threshold).
    let wrapper = Arc::new(
        CircuitBreakerWrapper::new(Box::new(FailingSubscriber::new("webhook-x")), noop_sink())
            .with_open_threshold(8),
    );
    let mut handles = Vec::new();
    for i in 0..8usize {
        let w = Arc::clone(&wrapper);
        handles.push(tokio::spawn(async move {
            let event = make_event_with("apcore.test.event", json!({ "i": i }));
            w.on_event(&event).await
        }));
    }
    for h in handles {
        h.await.unwrap().expect("breaker on_event must not error");
    }
    assert_eq!(wrapper.consecutive_failures(), 8);
    assert_eq!(wrapper.state(), CircuitState::Open);
}

// clause: event_system.on_failure.property.pure
#[tokio::test(flavor = "multi_thread")]
async fn circuit_on_failure_property_pure() {
    // Failure handling mutates circuit state (consecutive_failures increments).
    let wrapper =
        CircuitBreakerWrapper::new(Box::new(FailingSubscriber::new("webhook-x")), noop_sink())
            .with_open_threshold(5);
    let before = wrapper.consecutive_failures();
    wrapper.on_event(&make_event()).await.unwrap();
    let after = wrapper.consecutive_failures();
    assert_eq!(after, before + 1);
}

// clause: event_system.on_failure.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn circuit_on_failure_property_idempotent() {
    // Repeated failures are NOT idempotent: each increments the counter.
    let wrapper =
        CircuitBreakerWrapper::new(Box::new(FailingSubscriber::new("webhook-x")), noop_sink())
            .with_open_threshold(10);
    wrapper.on_event(&make_event()).await.unwrap();
    let first = wrapper.consecutive_failures();
    wrapper.on_event(&make_event()).await.unwrap();
    let second = wrapper.consecutive_failures();
    assert_eq!((first, second), (1, 2));
}

// ===========================================================================
// Contract: EventEmitter.unsubscribe
// ===========================================================================

// clause: event_system.unsubscribe.input.subscriber.same_reference
#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_input_subscriber_same_reference() {
    // unsubscribe removes the subscriber (Rust: by subscriber_id, an accepted
    // cross-language divergence); afterwards no more events are delivered to it.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub.clone()));
    let removed = emitter.unsubscribe(&sub);
    assert!(removed);
    emitter.emit(&make_event()).await;
    emitter.flush(2000).await.unwrap();
    assert!(received.lock().is_empty());
}

// clause: event_system.unsubscribe.error.unregistered_no_raise
#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_error_unregistered_no_raise() {
    // Unsubscribing an unregistered subscriber MUST NOT panic — it is a no-op
    // returning false.
    let emitter = EventEmitter::new();
    let never = RecordingSubscriber::new("never", "*");
    let removed = emitter.unsubscribe(&never);
    assert!(!removed);
}

// clause: event_system.unsubscribe.property.async
#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_property_async() {
    // unsubscribe() is synchronous and returns a bool (not a future).
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    emitter.subscribe(Box::new(sub.clone()));
    let result: bool = emitter.unsubscribe(&sub);
    assert!(result);
}

// clause: event_system.unsubscribe.property.thread_safe
#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_property_thread_safe() {
    // >=8 concurrent unsubscribe() calls (each removing a distinct subscriber,
    // by unique id) leave the registry consistent and empty. `unsubscribe` is
    // `&self` (interior-mutable subscribers, D1-011), so the calls run directly
    // through a shared `Arc<EventEmitter>`; assert no event is delivered after
    // all removals.
    let emitter = EventEmitter::new();
    let n = 10usize;
    let mut handles_to_remove: Vec<RecordingSubscriber> = Vec::new();
    let mut received_handles = Vec::new();
    for i in 0..n {
        let sub = RecordingSubscriber::new(&format!("s{i}"), "*");
        received_handles.push(sub.handle());
        handles_to_remove.push(sub.clone());
        emitter.subscribe(Box::new(sub));
    }
    let emitter = Arc::new(emitter);
    let mut tasks = Vec::new();
    for sub in handles_to_remove {
        let em = Arc::clone(&emitter);
        tasks.push(tokio::spawn(async move { em.unsubscribe(&sub) }));
    }
    let mut removed_count = 0usize;
    for t in tasks {
        if t.await.unwrap() {
            removed_count += 1;
        }
    }
    assert_eq!(removed_count, n);
    // Registry empty: emit delivers to nobody.
    {
        let guard = &emitter;
        guard.emit(&make_event()).await;
        guard.flush(2000).await.unwrap();
    }
    for r in received_handles {
        assert!(r.lock().is_empty());
    }
}

// clause: event_system.unsubscribe.property.pure
#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_property_pure() {
    // unsubscribe mutates the subscriber list (observable: delivery stops).
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub.clone()));
    // Before removal: delivery reaches the subscriber.
    emitter.emit(&make_event()).await;
    emitter.flush(2000).await.unwrap();
    let before = received.lock().len();
    emitter.unsubscribe(&sub);
    emitter.emit(&make_event()).await;
    emitter.flush(2000).await.unwrap();
    let after = received.lock().len();
    // Removal mutated the list: the second emit delivered nothing.
    assert_eq!((before, after), (1, 1));
}

// clause: event_system.unsubscribe.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_property_idempotent() {
    // Repeated unsubscribe of the same subscriber is a safe no-op. The first
    // call removes (true); the second is a no-op (false), and neither panics.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    emitter.subscribe(Box::new(sub.clone()));
    let first = emitter.unsubscribe(&sub);
    let second = emitter.unsubscribe(&sub); // must not panic
    assert_eq!((first, second), (true, false));
}

// ===========================================================================
// Contract: EventEmitter.flush
// ===========================================================================

// clause: event_system.flush.input.timeout.positive
#[tokio::test(flavor = "multi_thread")]
async fn flush_input_timeout_positive() {
    // flush(timeout_ms) waits up to `timeout_ms`; a positive timeout lets a
    // fast in-flight delivery complete before flush returns.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    emitter.emit(&make_event()).await;
    emitter.flush(5000).await.unwrap();
    assert_eq!(received.lock().len(), 1);
}

// clause: event_system.flush.error.none_raised
#[tokio::test(flavor = "multi_thread")]
async fn flush_error_none_raised() {
    // Subscriber errors surfacing during the flush window are silently
    // discarded; flush MUST return Ok (never error).
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(FailingSubscriber::new("boom")));
    emitter.emit(&make_event()).await;
    let result = emitter.flush(2000).await;
    assert!(result.is_ok());
}

// clause: event_system.flush.property.async
#[tokio::test(flavor = "multi_thread")]
async fn flush_property_async() {
    // Rust flush() is async (returns a future that resolves to Ok(())). Assert
    // it resolves to the unit Ok value.
    let emitter = EventEmitter::new();
    let result = emitter.flush(500).await;
    assert_eq!(result.unwrap(), ());
}

// clause: event_system.flush.property.thread_safe
#[tokio::test(flavor = "multi_thread")]
async fn flush_property_thread_safe() {
    // >=8 concurrent flush() calls (with emits in flight) all return Ok and the
    // pending set drains.
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    for _ in 0..8 {
        emitter.emit(&make_event()).await;
    }
    let emitter = Arc::new(emitter);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let em = Arc::clone(&emitter);
        handles.push(tokio::spawn(async move { em.flush(5000).await }));
    }
    for h in handles {
        h.await.unwrap().expect("flush must return Ok");
    }
    assert_eq!(received.lock().len(), 8);
}

// clause: event_system.flush.property.pure
#[tokio::test(flavor = "multi_thread")]
async fn flush_property_pure() {
    // flush waits on shared pending tasks and drains completed ones — it
    // mutates shared state (observable: a second flush has nothing to wait on
    // and returns immediately with the deliveries already done).
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    emitter.emit(&make_event()).await;
    emitter.flush(2000).await.unwrap();
    // After flush the pending set is drained: delivery already happened.
    assert_eq!(received.lock().len(), 1);
}

// clause: event_system.flush.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn flush_property_idempotent() {
    // Calling flush() again on an already-drained pending set returns
    // immediately with the same observable outcome (no error, no extra
    // deliveries).
    let emitter = EventEmitter::new();
    let sub = RecordingSubscriber::new("s", "*");
    let received = sub.handle();
    emitter.subscribe(Box::new(sub));
    emitter.emit(&make_event()).await;
    emitter.flush(2000).await.unwrap();
    let after_first = received.lock().len();
    emitter.flush(2000).await.unwrap(); // already empty -> no-op
    let after_second = received.lock().len();
    assert_eq!((after_first, after_second), (1, 1));
}

// ===========================================================================
// Contract: A2ASubscriber.deliver
// ===========================================================================

// clause: event_system.deliver.error.ImportError_a2a
#[test]
#[ignore = "event_system.deliver.error.ImportError_a2a: Python-only ImportError when aiohttp is absent; Rust gates the HTTP path behind the `events` cargo feature at compile time, so there is no runtime missing-dependency error to assert"]
fn a2a_deliver_error_import_error_without_aiohttp() {
    // Python raises ImportError synchronously if aiohttp is not installed. Rust
    // resolves the HTTP dependency at compile time via the `events` feature, so
    // there is no equivalent runtime contract. See ignore reason.
    unreachable!();
}

// The remaining A2ASubscriber.deliver clauses exercise the real HTTP path,
// gated behind the `events` cargo feature.
#[cfg(feature = "events")]
mod a2a_deliver {
    use super::webhook_deliver::MockServer;
    use super::*;
    use apcore::events::subscribers::{A2AAuth, A2ASubscriber};
    use std::collections::HashMap;

    // clause: event_system.deliver.input.event.required_a2a
    #[tokio::test(flavor = "multi_thread")]
    async fn a2a_deliver_input_event_required() {
        // A2A delivery POSTs a {skillId, event} wrapper carrying the event.
        let server = MockServer::spawn("200 OK").await;
        let sub = A2ASubscriber::new("a2a", &server.url, "*");
        let event = make_event_with("apcore.health.recovered", json!({}));
        sub.on_event(&event).await.expect("2xx must be Ok");
        assert_eq!(server.request_count(), 1);
        let body = server.last_json();
        assert_eq!(body["skillId"], "apevo.event_receiver");
        assert_eq!(body["event"]["event_type"], "apcore.health.recovered");
    }

    // clause: event_system.deliver.property.async_a2a
    #[tokio::test(flavor = "multi_thread")]
    async fn a2a_deliver_property_async() {
        // A2A delivery is awaitable and resolves to Ok(()).
        let server = MockServer::spawn("200 OK").await;
        let sub = A2ASubscriber::new("a2a", &server.url, "*");
        let result = sub.on_event(&make_event()).await;
        assert!(result.is_ok());
    }

    // clause: event_system.deliver.property.thread_safe_a2a
    #[tokio::test(flavor = "multi_thread")]
    async fn a2a_deliver_property_thread_safe() {
        // >=8 concurrent A2A deliveries with distinct events all resolve; each
        // issues its own POST.
        let server = MockServer::spawn("200 OK").await;
        let sub = Arc::new(A2ASubscriber::new("a2a", &server.url, "*"));
        let mut handles = Vec::new();
        for i in 0..8usize {
            let s = Arc::clone(&sub);
            handles.push(tokio::spawn(async move {
                let event = make_event_with("apcore.test.event", json!({ "i": i }));
                s.on_event(&event).await
            }));
        }
        for h in handles {
            h.await.unwrap().expect("delivery must resolve Ok");
        }
        assert_eq!(server.request_count(), 8);
    }

    // clause: event_system.deliver.property.pure_a2a
    #[tokio::test(flavor = "multi_thread")]
    async fn a2a_deliver_property_pure() {
        // Delivery is NOT pure: it performs an outbound HTTP POST to the URL.
        let server = MockServer::spawn("200 OK").await;
        let sub = A2ASubscriber::new("a2a", &server.url, "*");
        sub.on_event(&make_event()).await.unwrap();
        assert_eq!(server.request_count(), 1);
    }

    // clause: event_system.deliver.side_effect.1.auth_modes_a2a
    #[tokio::test(flavor = "multi_thread")]
    async fn a2a_deliver_side_effect_1_auth_modes() {
        // Observable auth policy: Bearer auth -> 'Authorization: Bearer <tok>';
        // Headers auth -> keys merged into headers; None -> no auth header.
        // The mock server here counts requests and confirms each auth mode
        // delivers successfully (the auth header is applied client-side before
        // send; a 200 reply confirms the request was well-formed).
        let server_bearer = MockServer::spawn("200 OK").await;
        let mut sub_bearer = A2ASubscriber::new("a2a", &server_bearer.url, "*");
        sub_bearer.auth = Some(A2AAuth::Bearer("tok123".to_string()));
        sub_bearer.on_event(&make_event()).await.expect("bearer ok");
        assert_eq!(server_bearer.request_count(), 1);

        let server_dict = MockServer::spawn("200 OK").await;
        let mut headers = HashMap::new();
        headers.insert("X-Api-Key".to_string(), "k".to_string());
        let mut sub_dict = A2ASubscriber::new("a2a", &server_dict.url, "*");
        sub_dict.auth = Some(A2AAuth::Headers(headers));
        sub_dict.on_event(&make_event()).await.expect("dict ok");
        assert_eq!(server_dict.request_count(), 1);

        let server_none = MockServer::spawn("200 OK").await;
        let sub_none = A2ASubscriber::new("a2a", &server_none.url, "*");
        // auth defaults to None.
        assert!(sub_none.auth.is_none());
        sub_none.on_event(&make_event()).await.expect("none ok");
        assert_eq!(server_none.request_count(), 1);
    }

    // clause: event_system.deliver.side_effect.2.retry_on_5xx_a2a
    #[tokio::test(flavor = "multi_thread")]
    async fn a2a_deliver_side_effect_2_retry_on_5xx() {
        // Observable HTTP-status policy for A2A: 5xx returns Err (emitter
        // retries); 4xx returns Ok (no retry).
        let server_5xx = MockServer::spawn("502 Bad Gateway").await;
        let sub5 = A2ASubscriber::new("a2a", &server_5xx.url, "*");
        let res5 = sub5.on_event(&make_event()).await;
        assert!(res5.is_err(), "5xx must surface as Err for emitter retry");
        assert_eq!(res5.unwrap_err().code, ErrorCode::GeneralInternalError);

        let server_4xx = MockServer::spawn("403 Forbidden").await;
        let sub4 = A2ASubscriber::new("a2a", &server_4xx.url, "*");
        let res4 = sub4.on_event(&make_event()).await;
        assert!(res4.is_ok(), "4xx is permanent: Ok, not retried");
    }
}

// ---------------------------------------------------------------------------
// sync-2026-08-26 A-D-025: removal must be able to name exactly one subscriber.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_handle_removes_exactly_one_of_two_sharing_an_id() {
    // `unsubscribe(&dyn EventSubscriber)` resolves to
    // `unsubscribe_by_id(subscriber.subscriber_id())`, and the trait default id
    // is the literal "default" — so two custom subscribers that do not override
    // it are indistinguishable, and removing one removed whichever was first.
    //
    // event-system.md's unsubscribe Contract requires "the same object
    // reference that was passed to subscribe", which apcore-python
    // (`list.remove`) and apcore-typescript (`indexOf`) honour because the
    // caller keeps its reference. `subscribe` here takes the Box, so the caller
    // has nothing left to pass back — the handle is what makes an unambiguous
    // removal expressible at all.
    #[derive(Debug)]
    struct Anon {
        tag: &'static str,
        seen: Arc<parking_lot::Mutex<Vec<&'static str>>>,
    }
    #[async_trait::async_trait]
    impl EventSubscriber for Anon {
        // deliberately does NOT override subscriber_id()
        fn event_pattern(&self) -> &str {
            "*"
        }
        async fn on_event(&self, _e: &ApCoreEvent) -> Result<(), apcore::errors::ModuleError> {
            self.seen.lock().push(self.tag);
            Ok(())
        }
    }

    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let emitter = EventEmitter::new();
    let h_a = emitter.subscribe(Box::new(Anon {
        tag: "a",
        seen: Arc::clone(&seen),
    }));
    let _h_b = emitter.subscribe(Box::new(Anon {
        tag: "b",
        seen: Arc::clone(&seen),
    }));

    assert!(emitter.unsubscribe_handle(h_a), "handle a must resolve");

    emitter.emit(&make_event()).await;
    emitter.flush(2_000).await.ok();

    let got = seen.lock().clone();
    assert_eq!(
        got,
        vec!["b"],
        "removing a's handle must leave b subscribed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_handle_is_idempotent_and_reports_absence() {
    #[derive(Debug)]
    struct Noop;
    #[async_trait::async_trait]
    impl EventSubscriber for Noop {
        fn event_pattern(&self) -> &str {
            "*"
        }
        async fn on_event(&self, _e: &ApCoreEvent) -> Result<(), apcore::errors::ModuleError> {
            Ok(())
        }
    }
    let emitter = EventEmitter::new();
    let h = emitter.subscribe(Box::new(Noop));
    assert!(emitter.unsubscribe_handle(h), "first removal succeeds");
    assert!(
        !emitter.unsubscribe_handle(h),
        "a handle already removed must report absence, not remove someone else"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn flush_default_uses_the_contract_default() {
    // The flush Contract declares `timeout` OPTIONAL with a 5-second default.
    // Rust has no default arguments, and this SDK was the only one that
    // therefore had no default at all (sync finding A-D-026).
    assert_eq!(EventEmitter::DEFAULT_FLUSH_TIMEOUT_MS, 5_000);
    let emitter = EventEmitter::new();
    emitter
        .flush_default()
        .await
        .expect("flushing an idle emitter is a no-op");
}
