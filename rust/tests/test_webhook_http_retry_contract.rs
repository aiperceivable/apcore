// apcore issue #69 — Regression lock for the WebhookSubscriber HTTP 4xx-vs-5xx
// retry contract.
//
// Contract (already implemented in src/events/subscribers.rs; this test LOCKS
// it, it does not change it):
//   - HTTP 4xx  -> permanent client error, NOT retried.
//   - HTTP 5xx  -> retried per the subscriber's retry config; on exhaustion the
//                  emitter emits `apcore.event.delivery_failed` (DLQ).
//
// The shared conformance driver fails attempts generically and bypasses the
// HTTP-status logic, so the 4xx/5xx distinction is unguarded in the conformance
// layer. This test exercises the real HTTP path end to end.
//
// HTTP is mocked the same way as tests/test_observability.rs: a `TcpListener`
// bound to an ephemeral 127.0.0.1 port that accepts real connections and
// replies with a fixed status line. No new HTTP-mock dependency is introduced.
// The core assertion is the number of HTTP requests the mock server receives.
//
// These tests require the `events` cargo feature (the WebhookSubscriber HTTP
// path is gated behind it); without it the whole module compiles to nothing.

#![cfg(feature = "events")]

use apcore::errors::ModuleError;
use apcore::events::emitter::{ApCoreEvent, EventEmitter};
use apcore::events::retry::EventRetryConfig;
use apcore::events::subscribers::{A2ASubscriber, EventSubscriber, WebhookSubscriber};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Mock HTTP server
// ---------------------------------------------------------------------------

/// A minimal HTTP server bound to an ephemeral port that replies to every
/// request with a fixed status line and counts how many requests it received.
///
/// Mirrors the raw-TCP mock approach in tests/test_observability.rs, extended
/// to accept multiple sequential connections so we can assert the retry count.
struct MockServer {
    url: String,
    request_count: Arc<AtomicUsize>,
}

impl MockServer {
    /// Spawn a server that replies with `status_line` (e.g. "400 Bad Request")
    /// to every incoming request. The returned handle is kept alive for the
    /// duration of the test; the accept loop runs until the listener is dropped.
    async fn spawn(status_line: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/hook");
        let request_count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&request_count);

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let counter = Arc::clone(&counter);
                tokio::spawn(async move {
                    // Drain the request (read headers + body) before replying so
                    // the client sees a clean, complete response.
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
                    }

                    // Count the request only after we have read a full request.
                    counter.fetch_add(1, Ordering::SeqCst);

                    let response = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });

        Self { url, request_count }
    }

    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
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

// ---------------------------------------------------------------------------
// DLQ-capturing subscriber (mirrors tests/test_event_delivery_semantics.rs)
// ---------------------------------------------------------------------------

/// Records every `apcore.event.delivery_failed` event it receives so tests can
/// assert DLQ emission on retry exhaustion.
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
    // Non-`*` pattern: catch-all subscribers are excluded from DLQ delivery
    // (A-D-026), so the DLQ sink must match the DLQ event type explicitly.
    #[allow(clippy::unnecessary_literal_bound)]
    fn event_pattern(&self) -> &str {
        "apcore.event.delivery_failed"
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.received.lock().push(event.clone());
        Ok(())
    }
}

// Fast retry config so the 5xx test does not spend real backoff time.
fn fast_retry(max_attempts: u32) -> EventRetryConfig {
    EventRetryConfig {
        max_attempts,
        initial_backoff_ms: 1,
        max_backoff_ms: 5,
        backoff_multiplier: 1.0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 4xx (400) with max_attempts=3: the endpoint must receive EXACTLY 1 request.
/// A 4xx is a permanent client error and must NOT be retried, even though the
/// retry config allows 3 attempts. No DLQ event is emitted (delivery is treated
/// as terminal-success by the subscriber to suppress the retry loop).
#[tokio::test]
async fn test_webhook_4xx_is_not_retried_single_request() {
    let server = MockServer::spawn("400 Bad Request").await;

    let webhook = WebhookSubscriber::new("wh-4xx", &server.url, "*").with_retry(fast_retry(3));

    let (dlq, dlq_events) = DlqSubscriber::new("dlq-4xx");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(webhook));
    emitter.subscribe(Box::new(dlq));

    let event = ApCoreEvent::new("test.event", json!({"hello": "world"}));
    emitter.emit_delivery_semantics(event);
    emitter.flush(5000).await.unwrap();

    assert_eq!(
        server.request_count(),
        1,
        "4xx must NOT be retried: endpoint should receive exactly 1 request"
    );
    assert!(
        dlq_events.lock().is_empty(),
        "4xx is a non-retryable client error reported as success: no DLQ event expected"
    );
}

/// 5xx (503) with max_attempts=3: the endpoint must receive EXACTLY 3 requests
/// (retried to exhaustion), and a single `apcore.event.delivery_failed` DLQ
/// event must be emitted.
#[tokio::test]
async fn test_webhook_5xx_is_retried_to_exhaustion_then_dlq() {
    let server = MockServer::spawn("503 Service Unavailable").await;

    let webhook = WebhookSubscriber::new("wh-5xx", &server.url, "*").with_retry(fast_retry(3));

    let (dlq, dlq_events) = DlqSubscriber::new("dlq-5xx");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(webhook));
    emitter.subscribe(Box::new(dlq));

    let event = ApCoreEvent::new("test.event", json!({"hello": "world"}));
    emitter.emit_delivery_semantics(event);
    emitter.flush(5000).await.unwrap();

    assert_eq!(
        server.request_count(),
        3,
        "5xx must be retried per max_attempts=3: endpoint should receive 3 requests"
    );

    let events = dlq_events.lock();
    assert_eq!(
        events.len(),
        1,
        "exactly one apcore.event.delivery_failed DLQ event expected on exhaustion"
    );
    let dlq_event = &events[0];
    assert_eq!(dlq_event.event_type, "apcore.event.delivery_failed");
    assert_eq!(
        dlq_event
            .data
            .get("subscriber_type")
            .and_then(serde_json::Value::as_str),
        Some("webhook"),
        "DLQ payload must identify the webhook subscriber type"
    );
    assert_eq!(
        dlq_event
            .data
            .get("attempt_count")
            .and_then(serde_json::Value::as_u64),
        Some(3),
        "DLQ payload must report 3 exhausted attempts"
    );
}

// ---------------------------------------------------------------------------
// A2ASubscriber — same 4xx/5xx contract (apcore issue #69)
// ---------------------------------------------------------------------------

/// A2A 4xx (400) with max_attempts=3: the endpoint must receive EXACTLY 1
/// request. A 4xx is a permanent client error and must NOT be retried, mirroring
/// WebhookSubscriber. No DLQ event is emitted.
#[tokio::test]
async fn test_a2a_4xx_is_not_retried_single_request() {
    let server = MockServer::spawn("400 Bad Request").await;

    let a2a = A2ASubscriber::new("a2a-4xx", &server.url, "*").with_retry(fast_retry(3));

    let (dlq, dlq_events) = DlqSubscriber::new("dlq-a2a-4xx");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(a2a));
    emitter.subscribe(Box::new(dlq));

    let event = ApCoreEvent::new("test.event", json!({"hello": "world"}));
    emitter.emit_delivery_semantics(event);
    emitter.flush(5000).await.unwrap();

    assert_eq!(
        server.request_count(),
        1,
        "A2A 4xx must NOT be retried: endpoint should receive exactly 1 request"
    );
    assert!(
        dlq_events.lock().is_empty(),
        "A2A 4xx is a non-retryable client error reported as success: no DLQ event expected"
    );
}

/// A2A 5xx (503) with max_attempts=3: the endpoint must receive EXACTLY 3
/// requests (retried to exhaustion), and a single `apcore.event.delivery_failed`
/// DLQ event must be emitted with `subscriber_type == "a2a"`.
#[tokio::test]
async fn test_a2a_5xx_is_retried_to_exhaustion_then_dlq() {
    let server = MockServer::spawn("503 Service Unavailable").await;

    let a2a = A2ASubscriber::new("a2a-5xx", &server.url, "*").with_retry(fast_retry(3));

    let (dlq, dlq_events) = DlqSubscriber::new("dlq-a2a-5xx");

    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(a2a));
    emitter.subscribe(Box::new(dlq));

    let event = ApCoreEvent::new("test.event", json!({"hello": "world"}));
    emitter.emit_delivery_semantics(event);
    emitter.flush(5000).await.unwrap();

    assert_eq!(
        server.request_count(),
        3,
        "A2A 5xx must be retried per max_attempts=3: endpoint should receive 3 requests"
    );

    let events = dlq_events.lock();
    assert_eq!(
        events.len(),
        1,
        "exactly one apcore.event.delivery_failed DLQ event expected on exhaustion"
    );
    let dlq_event = &events[0];
    assert_eq!(dlq_event.event_type, "apcore.event.delivery_failed");
    assert_eq!(
        dlq_event
            .data
            .get("subscriber_type")
            .and_then(serde_json::Value::as_str),
        Some("a2a"),
        "DLQ payload must identify the a2a subscriber type"
    );
    assert_eq!(
        dlq_event
            .data
            .get("attempt_count")
            .and_then(serde_json::Value::as_u64),
        Some(3),
        "DLQ payload must report 3 exhausted attempts"
    );
}
