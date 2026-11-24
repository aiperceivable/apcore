//! The nested `retry:` block in subscriber config is read by every factory (apcore#85).
//!
//! `docs/features/event-system.md` documents a per-subscriber `retry:` block and
//! shows it on multiple subscriber types. Before apcore#85 no factory parsed it:
//! only `webhook` read a policy field, and only from the legacy flat
//! `retry_count` shorthand — which in Rust was written to a struct field that
//! `EventSubscriber::retry()` never returned, so it was fully inert. An operator
//! copying the documented example got no retry policy at all, silently.
//!
//! Every assertion below deliberately uses values that differ from
//! `EventRetryConfig::default()` (`max_attempts=3`, `initial_backoff_ms=100`,
//! `max_backoff_ms=30000`, `backoff_multiplier=2.0`) — otherwise the test would
//! pass whether or not the config was ever read.
//!
//! The registry is only read here (no custom types are registered), so these
//! tests do not mutate process-global state.

use apcore::errors::ModuleError;
use apcore::events::emitter::ApCoreEvent;
use apcore::events::retry::EventRetryConfig;
use apcore::events::{create_subscriber, EventEmitter, EventSubscriber};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A policy in which every single field differs from the spec default.
fn non_default_retry_json() -> serde_json::Value {
    json!({
        "max_attempts": 7,
        "initial_backoff_ms": 250,
        "max_backoff_ms": 10_000,
        "backoff_multiplier": 3.0
    })
}

fn assert_non_default_policy(retry: EventRetryConfig) {
    let default = EventRetryConfig::default();
    assert_eq!(retry.max_attempts, 7);
    assert_ne!(retry.max_attempts, default.max_attempts);
    assert_eq!(retry.initial_backoff_ms, 250);
    assert_ne!(retry.initial_backoff_ms, default.initial_backoff_ms);
    assert_eq!(retry.max_backoff_ms, 10_000);
    assert_ne!(retry.max_backoff_ms, default.max_backoff_ms);
    assert!((retry.backoff_multiplier - 3.0).abs() < f64::EPSILON);
    assert!((retry.backoff_multiplier - default.backoff_multiplier).abs() > f64::EPSILON);
}

// ---------------------------------------------------------------------------
// One case per built-in subscriber type
// ---------------------------------------------------------------------------

#[test]
fn webhook_reads_nested_retry_block() {
    let sub = create_subscriber(&json!({
        "type": "webhook",
        "url": "https://example.com/hook",
        "retry": non_default_retry_json()
    }))
    .unwrap();
    assert_eq!(sub.subscriber_type(), "webhook");
    assert_non_default_policy(sub.retry());
}

#[test]
fn a2a_reads_nested_retry_block() {
    let sub = create_subscriber(&json!({
        "type": "a2a",
        "platform_url": "https://platform.example.com",
        "retry": non_default_retry_json()
    }))
    .unwrap();
    assert_eq!(sub.subscriber_type(), "a2a");
    assert_non_default_policy(sub.retry());
}

#[test]
fn file_reads_nested_retry_block() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let sub = create_subscriber(&json!({
        "type": "file",
        "path": path.to_string_lossy(),
        "retry": non_default_retry_json()
    }))
    .unwrap();
    assert_eq!(sub.subscriber_type(), "file");
    assert_non_default_policy(sub.retry());
}

#[test]
fn stdout_reads_nested_retry_block() {
    let sub = create_subscriber(&json!({
        "type": "stdout",
        "format": "json",
        "retry": non_default_retry_json()
    }))
    .unwrap();
    assert_eq!(sub.subscriber_type(), "stdout");
    assert_non_default_policy(sub.retry());
}

#[test]
fn filter_reads_nested_retry_block() {
    let sub = create_subscriber(&json!({
        "type": "filter",
        "delegate_type": "stdout",
        "delegate_config": {"format": "json"},
        "include_events": ["apcore.error.*"],
        "retry": non_default_retry_json()
    }))
    .unwrap();
    // FilterSubscriber reports the delegate's type (A-D-029); its own identity
    // comes from the subscriber_id prefix.
    assert!(sub.subscriber_id().starts_with("filter-"));
    assert_non_default_policy(sub.retry());
}

// ---------------------------------------------------------------------------
// Parsing semantics
// ---------------------------------------------------------------------------

#[test]
fn partial_block_merges_over_spec_defaults() {
    // The documented `file` example declares only two of the four keys.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let sub = create_subscriber(&json!({
        "type": "file",
        "path": path.to_string_lossy(),
        "retry": {"max_attempts": 2, "initial_backoff_ms": 50}
    }))
    .unwrap();
    let default = EventRetryConfig::default();
    let retry = sub.retry();
    assert_eq!(retry.max_attempts, 2);
    assert_ne!(retry.max_attempts, default.max_attempts);
    assert_eq!(retry.initial_backoff_ms, 50);
    assert_ne!(retry.initial_backoff_ms, default.initial_backoff_ms);
    // Unspecified keys keep the spec defaults.
    assert_eq!(retry.max_backoff_ms, default.max_backoff_ms);
    assert!((retry.backoff_multiplier - default.backoff_multiplier).abs() < f64::EPSILON);
}

#[test]
fn absent_block_keeps_spec_defaults() {
    let sub = create_subscriber(&json!({"type": "stdout"})).unwrap();
    let default = EventRetryConfig::default();
    assert_eq!(sub.retry().max_attempts, default.max_attempts);
    assert_eq!(sub.retry().initial_backoff_ms, default.initial_backoff_ms);
}

#[test]
fn non_object_retry_value_is_ignored() {
    let sub = create_subscriber(&json!({"type": "stdout", "retry": "aggressive"})).unwrap();
    assert_eq!(
        sub.retry().max_attempts,
        EventRetryConfig::default().max_attempts
    );
}

#[test]
fn flat_retry_count_still_honoured_for_webhook() {
    // Deprecated alias: retry_count counted retries AFTER the first attempt.
    let sub = create_subscriber(&json!({
        "type": "webhook",
        "url": "https://example.com/hook",
        "retry_count": 5
    }))
    .unwrap();
    assert_eq!(sub.retry().max_attempts, 6);
}

#[test]
fn nested_block_wins_over_flat_retry_count() {
    let sub = create_subscriber(&json!({
        "type": "webhook",
        "url": "https://example.com/hook",
        "retry_count": 5,
        "retry": non_default_retry_json()
    }))
    .unwrap();
    // retry_count=5 would have produced max_attempts=6; the nested block wins.
    assert_non_default_policy(sub.retry());
}

#[test]
fn delegate_config_retry_does_not_leak_to_the_filter() {
    // A `retry:` inside delegate_config configures the delegate; the filter
    // itself keeps the spec default until it declares its own block.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let sub = create_subscriber(&json!({
        "type": "filter",
        "delegate_type": "file",
        "delegate_config": {
            "path": path.to_string_lossy(),
            "retry": non_default_retry_json()
        }
    }))
    .unwrap();
    assert_eq!(
        sub.retry().max_attempts,
        EventRetryConfig::default().max_attempts
    );
}

// ---------------------------------------------------------------------------
// End-to-end: the declared policy actually governs delivery
// ---------------------------------------------------------------------------

/// Captures `apcore.event.delivery_failed` payloads. A non-wildcard
/// `event_pattern` is required to receive DLQ events (A-D-026).
#[derive(Debug)]
struct DlqCollector {
    received: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait]
impl EventSubscriber for DlqCollector {
    fn subscriber_id(&self) -> &str {
        "dlq-collector"
    }
    fn event_pattern(&self) -> &str {
        "apcore.event.delivery_failed"
    }
    async fn on_event(&self, event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.received.lock().push(event.data.clone());
        Ok(())
    }
}

#[tokio::test]
async fn emitter_honours_config_declared_max_attempts() {
    // A `file` subscriber pointed at a non-existent directory fails every
    // write, so the emitter exhausts the declared policy and reports the real
    // attempt count in the DLQ payload. max_attempts=5 differs from the
    // default 3, so a factory that ignored the block would report 3.
    let sub = create_subscriber(&json!({
        "type": "file",
        "path": "/nonexistent-apcore-85-dir/events.jsonl",
        "retry": {"max_attempts": 5, "initial_backoff_ms": 0, "max_backoff_ms": 0}
    }))
    .unwrap();
    assert_eq!(sub.retry().max_attempts, 5);

    let received = Arc::new(Mutex::new(Vec::new()));
    let emitter = EventEmitter::new();
    emitter.subscribe(sub);
    emitter.subscribe(Box::new(DlqCollector {
        received: Arc::clone(&received),
    }));

    emitter.emit_delivery_semantics(ApCoreEvent::new("test.event", json!({})));
    tokio::time::sleep(Duration::from_millis(300)).await;

    let payloads = received.lock().clone();
    assert_eq!(payloads.len(), 1, "expected exactly one DLQ event");
    assert_eq!(payloads[0]["attempt_count"], json!(5));
    assert_eq!(payloads[0]["subscriber_type"], json!("file"));
}

/// The counting variant proves the emitter really re-invoked `on_event`, not
/// merely that it reported `max_attempts` in the DLQ payload.
#[derive(Debug)]
struct CountingSink {
    retry: EventRetryConfig,
    attempts: Arc<AtomicU32>,
}

#[async_trait]
impl EventSubscriber for CountingSink {
    fn subscriber_id(&self) -> &str {
        "counting-sink"
    }
    fn retry(&self) -> EventRetryConfig {
        self.retry
    }
    async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(ModuleError::new(
            apcore::errors::ErrorCode::GeneralInternalError,
            "transient sink failure",
        ))
    }
}

#[tokio::test]
async fn config_declared_policy_drives_real_attempt_count() {
    // Borrow ONLY the parsed policy from a config-built subscriber; the sink
    // counts how many times the emitter actually calls on_event.
    let configured = create_subscriber(&json!({
        "type": "stdout",
        "retry": {"max_attempts": 5, "initial_backoff_ms": 0, "max_backoff_ms": 0}
    }))
    .unwrap();

    let attempts = Arc::new(AtomicU32::new(0));
    let emitter = EventEmitter::new();
    emitter.subscribe(Box::new(CountingSink {
        retry: configured.retry(),
        attempts: Arc::clone(&attempts),
    }));

    emitter.emit_delivery_semantics(ApCoreEvent::new("test.event", json!({})));
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(attempts.load(Ordering::SeqCst), 5);
}
