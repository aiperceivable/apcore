// APCore Protocol — Event emitter
// Spec reference: Event types and emission

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use super::subscribers::EventSubscriber;
use crate::errors::{ErrorCode, ModuleError};

/// Event type for dead-letter-queue notifications emitted on delivery exhaustion.
const DLQ_EVENT_TYPE: &str = "apcore.event.delivery_failed";

/// An event emitted by the `APCore` system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApCoreEvent {
    pub event_type: String,
    /// ISO 8601 timestamp string.
    pub timestamp: String,
    pub data: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_id: Option<String>,
    pub severity: String,
}

impl ApCoreEvent {
    /// Create a new event with "info" severity.
    pub fn new(event_type: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            event_type: event_type.into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            data,
            module_id: None,
            severity: "info".to_string(),
        }
    }

    /// Create a new event with explicit `module_id` and severity.
    pub fn with_module(
        event_type: impl Into<String>,
        data: serde_json::Value,
        module_id: impl Into<String>,
        severity: impl Into<String>,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            data,
            module_id: Some(module_id.into()),
            severity: severity.into(),
        }
    }
}

/// Manages event subscribers and dispatches events.
///
/// Subscribers are stored as `Arc<dyn EventSubscriber>` so they can be
/// cheaply cloned into spawned tasks for [`Self::emit_spawn`]'s
/// fire-and-forget dispatch model (sync finding A-D-501).
#[derive(Debug)]
pub struct EventEmitter {
    // Interior-mutable so a shared `Arc<EventEmitter>` supports BOTH subscribe
    // (`&self`, write-lock) and emit (`&self`, read-lock) — letting the client,
    // registry, and sys-modules share one bus (D1-011). A sync `parking_lot`
    // lock is used (not `tokio::Mutex`) because emit runs from both sync
    // (`Registry::register` -> `emit_spawn`) and async contexts; emit snapshots
    // the subscriber list under a short read-lock and never holds it across `.await`.
    subscribers: RwLock<Vec<Arc<dyn EventSubscriber>>>,
    pub max_workers: usize,
    /// Set to `true` by [`Self::shutdown`]. Once set, all `emit*` methods
    /// drop incoming events as no-ops (sync finding A-D-502).
    is_shutdown: Arc<AtomicBool>,
    /// In-flight delivery tasks spawned by [`Self::emit`]. [`Self::flush`]
    /// awaits these (up to a timeout) so callers can wait for pending
    /// deliveries to drain (sync findings A-D-024 / A-D-027). Completed
    /// handles are pruned lazily on each `emit`/`flush`.
    pending: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// Parallel to `subscribers`: the handle issued for each registration, so
    /// `unsubscribe_handle` can remove exactly one even when several share a
    /// `subscriber_id()` (sync finding A-D-025).
    handles: RwLock<Vec<SubscriberHandle>>,
    next_handle: AtomicU64,
}

/// Opaque token identifying one subscription, returned by
/// [`EventEmitter::subscribe`] and consumed by
/// [`EventEmitter::unsubscribe_handle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriberHandle(u64);

/// The `EventSubscriber::subscriber_id` trait default.
const DEFAULT_SUBSCRIBER_ID: &str = "default";

impl EventEmitter {
    /// Create a new event emitter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(vec![]),
            max_workers: 4,
            is_shutdown: Arc::new(AtomicBool::new(false)),
            pending: Arc::new(Mutex::new(Vec::new())),
            handles: RwLock::new(vec![]),
            next_handle: AtomicU64::new(0),
        }
    }

    /// Add a subscriber, returning a handle that removes exactly this one.
    ///
    /// The handle exists because Rust's ownership model makes the peer SDKs'
    /// removal contract unreachable. `event-system.md`'s unsubscribe Contract
    /// requires "the same object reference that was passed to `subscribe`",
    /// which apcore-python (`list.remove`) and apcore-typescript (`indexOf`)
    /// can honour because the caller keeps its reference. This method takes the
    /// `Box`, so the caller has nothing left to pass back — and
    /// [`Self::unsubscribe`] therefore approximates identity by
    /// `subscriber_id()`, whose trait default is the literal `"default"`, so
    /// two custom subscribers that do not override it are indistinguishable
    /// (sync finding A-D-025).
    ///
    /// The return value may be discarded by callers that never remove.
    pub fn subscribe(&self, subscriber: Box<dyn EventSubscriber>) -> SubscriberHandle {
        // Convert Box -> Arc so subscribers can be cloned into spawned
        // tasks by `emit_spawn` (sync finding A-D-501).
        let mut subs = self.subscribers.write();
        let handle = SubscriberHandle(self.next_handle.fetch_add(1, Ordering::SeqCst));
        subs.push(Arc::from(subscriber));
        self.handles.write().push(handle);
        handle
    }

    /// Remove exactly the subscriber that [`Self::subscribe`] returned `handle`
    /// for. Unambiguous regardless of `subscriber_id()`.
    pub fn unsubscribe_handle(&self, handle: SubscriberHandle) -> bool {
        let mut handles = self.handles.write();
        let Some(pos) = handles.iter().position(|h| *h == handle) else {
            return false;
        };
        handles.remove(pos);
        self.subscribers.write().remove(pos);
        true
    }

    /// Remove the first subscriber whose `subscriber_id()` matches the given
    /// subscriber's ID.
    ///
    /// **Cross-language divergence (A-D-030, accepted Rust constraint):**
    /// apcore-python / apcore-typescript remove by exact *instance* identity.
    /// Rust stores subscribers as `Arc<dyn EventSubscriber>` trait objects, and
    /// pointer identity is not preserved across the `Box`→`Arc` conversion in
    /// [`Self::subscribe`], so instance matching is not feasible without a
    /// handle-returning API. Identity is therefore approximated by
    /// `subscriber_id()`; callers needing independent removal MUST assign each
    /// subscriber a unique id.
    pub fn unsubscribe(&self, subscriber: &dyn EventSubscriber) -> bool {
        let target_id = subscriber.subscriber_id();
        // Removal by the trait-default id is ambiguous: every subscriber that
        // did not override `subscriber_id()` reports the same literal, so this
        // removes the FIRST such registration, which may not be the one the
        // caller meant. Warn rather than silently removing the wrong one —
        // `unsubscribe_handle` is the unambiguous form.
        if target_id == DEFAULT_SUBSCRIBER_ID {
            let ambiguous = self
                .subscribers
                .read()
                .iter()
                .filter(|s| s.subscriber_id() == DEFAULT_SUBSCRIBER_ID)
                .count();
            if ambiguous > 1 {
                tracing::warn!(
                    count = ambiguous,
                    "unsubscribe() called with the default subscriber_id and {ambiguous} \
                     subscribers share it — removing the first. Use unsubscribe_handle(), \
                     or override subscriber_id(), to remove a specific one."
                );
            }
        }
        self.unsubscribe_by_id(target_id)
    }

    /// Remove the first subscriber whose `subscriber_id()` matches the given ID string.
    pub fn unsubscribe_by_id(&self, subscriber_id: &str) -> bool {
        let mut subs = self.subscribers.write();
        let pos = subs.iter().position(|s| s.subscriber_id() == subscriber_id);
        if let Some(i) = pos {
            subs.remove(i);
            true
        } else {
            false
        }
    }

    /// Remove all subscribers whose `event_type_filter()` equals `event_type`.
    ///
    /// Returns the number of subscribers removed. Matches Python/TypeScript
    /// `off(event_type)` semantics where passing an event-type string removes
    /// all handlers bound to that type.
    pub fn unsubscribe_by_event_type(&self, event_type: &str) -> usize {
        let mut subs = self.subscribers.write();
        let before = subs.len();
        subs.retain(|s| s.event_type_filter().is_none_or(|t| t != event_type));
        before - subs.len()
    }

    /// Emit an event to all subscribers whose pattern matches the event type.
    ///
    /// Canonical delivery path — applies the per-subscriber retry policy and
    /// emits an `apcore.event.delivery_failed` DLQ event on exhaustion, per
    /// docs/features/event-system.md §Delivery Semantics.
    ///
    /// **Non-blocking (sync finding A-D-024):** each matching subscriber's
    /// retry loop (including backoff sleeps) runs on its own spawned task, so
    /// this method returns immediately without waiting for subscriber
    /// execution — matching apcore-python (`ThreadPoolExecutor.submit`) and
    /// apcore-typescript (fire-and-forget). Use [`Self::flush`] to wait for
    /// pending deliveries to drain.
    ///
    /// **Post-shutdown behaviour:** if [`Self::shutdown`] has been called,
    /// this method returns immediately as a no-op (sync finding A-D-502).
    ///
    /// Spec event-system.md:448 declares `No errors raised`. Subscriber
    /// errors are caught, retried per the per-subscriber `retry` config, and
    /// surfaced via DLQ + `on_failure` only after exhaustion. The return
    /// type is unit (D10-008).
    ///
    /// `async fn` is retained for API parity and so callers may `.await` it;
    /// the body itself does not block on subscriber execution.
    #[allow(clippy::unused_async)]
    pub async fn emit(&self, event: &ApCoreEvent) {
        if self.is_shutdown.load(Ordering::SeqCst) {
            return;
        }
        let all_subscribers: Vec<Arc<dyn EventSubscriber>> = self.subscribers.read().clone();
        let mut handles: Vec<JoinHandle<()>> = Vec::new();
        for subscriber in &all_subscribers {
            if Self::subscriber_matches(subscriber.as_ref(), &event.event_type) {
                let sub = Arc::clone(subscriber);
                let evt = event.clone();
                let dlq_subs = all_subscribers.clone();
                handles.push(tokio::spawn(async move {
                    Self::deliver_with_dlq(sub, evt, dlq_subs).await;
                }));
            }
        }
        // Track in-flight tasks so flush() can await them; prune finished ones.
        let mut pending = self.pending.lock();
        pending.retain(|h| !h.is_finished());
        pending.extend(handles);
    }

    /// Single-attempt sequential emit — bypasses the per-subscriber retry
    /// policy and DLQ machinery.
    ///
    /// Use this in tests that need strict ordering and deterministic
    /// single-shot delivery semantics. Production code SHOULD call
    /// [`Self::emit`] (canonical retry + DLQ) instead.
    ///
    /// Errors from individual subscribers are logged but not propagated
    /// (error isolation), matching Python's behaviour.
    pub async fn emit_sequential(&self, event: &ApCoreEvent) {
        if self.is_shutdown.load(Ordering::SeqCst) {
            return;
        }
        // Snapshot under a short read-lock; never hold the lock across `.await`.
        let subscribers: Vec<Arc<dyn EventSubscriber>> = self.subscribers.read().clone();
        for subscriber in &subscribers {
            if Self::subscriber_matches(subscriber.as_ref(), &event.event_type) {
                if let Err(e) = subscriber.on_event(event).await {
                    tracing::warn!(
                        subscriber_id = %subscriber.subscriber_id(),
                        event_type = %event.event_type,
                        error = %e,
                        "event subscriber failed"
                    );
                }
            }
        }
    }

    /// Emit an event to subscribers matching both the caller's filter pattern
    /// AND the subscriber's own `event_pattern`.
    ///
    /// Applies the same retry + DLQ semantics as [`Self::emit`].
    ///
    /// **Post-shutdown behaviour:** drops the event as a no-op (sync
    /// finding A-D-502).
    pub async fn emit_filtered(
        &self,
        event: &ApCoreEvent,
        pattern: &str,
    ) -> Result<(), ModuleError> {
        if self.is_shutdown.load(Ordering::SeqCst) {
            return Ok(());
        }
        let all_subscribers: Vec<Arc<dyn EventSubscriber>> = self.subscribers.read().clone();
        for subscriber in &all_subscribers {
            if Self::matches_pattern(pattern, &event.event_type)
                && Self::subscriber_matches(subscriber.as_ref(), &event.event_type)
            {
                Self::deliver_with_dlq(
                    Arc::clone(subscriber),
                    event.clone(),
                    all_subscribers.clone(),
                )
                .await;
            }
        }
        Ok(())
    }

    /// Flush all pending event deliveries, waiting up to `timeout_ms`
    /// milliseconds for the tasks spawned by [`Self::emit`] to complete.
    ///
    /// Drains the tracked in-flight delivery tasks (sync findings A-D-024 /
    /// A-D-027), matching apcore-python's `flush()` which waits on each pending
    /// future. If the overall timeout elapses before all tasks finish, the
    /// remaining (still-running) tasks are left in place and `flush` returns
    /// `Ok(())` — delivery continues in the background, mirroring Python which
    /// swallows per-future timeouts. A `timeout_ms` of 0 waits indefinitely.
    /// Default flush timeout in milliseconds, matching the 5-second default the
    /// `flush` Contract declares and the one apcore-typescript uses.
    pub const DEFAULT_FLUSH_TIMEOUT_MS: u64 = 5_000;

    /// [`Self::flush`] with the Contract's default timeout.
    ///
    /// The Contract declares `timeout` OPTIONAL with a default; Rust has no
    /// default arguments, so this is where that default lives. Without it this
    /// SDK was the only one with no default at all (sync finding A-D-026).
    pub async fn flush_default(&self) -> Result<(), ModuleError> {
        self.flush(Self::DEFAULT_FLUSH_TIMEOUT_MS).await
    }

    pub async fn flush(&self, timeout_ms: u64) -> Result<(), ModuleError> {
        // Take ownership of the current in-flight handles.
        let handles: Vec<JoinHandle<()>> = {
            let mut pending = self.pending.lock();
            std::mem::take(&mut *pending)
        };
        if handles.is_empty() {
            return Ok(());
        }

        let deadline = if timeout_ms == 0 {
            None
        } else {
            Some(tokio::time::Instant::now() + Duration::from_millis(timeout_ms))
        };

        let mut unfinished: Vec<JoinHandle<()>> = Vec::new();
        for handle in handles {
            match deadline {
                None => {
                    let _ = handle.await;
                }
                Some(dl) => {
                    let now = tokio::time::Instant::now();
                    if now >= dl {
                        // Budget exhausted — keep remaining tasks running.
                        unfinished.push(handle);
                        continue;
                    }
                    // Await the task up to the deadline. On either outcome
                    // (completed or per-task timeout) the result is discarded:
                    // a timed-out delivery continues detached in the background
                    // (the JoinHandle is consumed by timeout_at).
                    let _ = tokio::time::timeout_at(dl, handle).await;
                }
            }
        }

        if !unfinished.is_empty() {
            let mut pending = self.pending.lock();
            // Re-track any tasks we did not get to await, plus any new ones.
            unfinished.extend(std::mem::take(&mut *pending));
            *pending = unfinished;
        }
        Ok(())
    }

    /// Fire-and-forget dispatch: spawns one `tokio::task` per matching
    /// subscriber and returns immediately.
    ///
    /// Use this when the caller cannot wait for subscriber completion (the
    /// canonical "fire-and-forget" path called out by the spec, sync finding
    /// A-D-501). Errors from subscribers are logged via `tracing::warn!` and
    /// never propagated. Subscribers run **concurrently** because each is
    /// driven on its own task.
    ///
    /// `emit_spawn` is the preferred dispatch path for runtime modules
    /// (system modules, circuit breaker, etc.); the sequential `emit` is
    /// retained for tests that need deterministic ordering.
    ///
    /// **Post-shutdown behaviour:** drops the event as a no-op once
    /// [`Self::shutdown`] has been called (sync finding A-D-502).
    // Takes `event: ApCoreEvent` by value because each spawned task needs an
    // owned `ApCoreEvent`; passing by reference would force the caller to
    // clone, hiding the cost. The spec's pseudocode signature also
    // matches by-value (sync finding A-D-501).
    #[allow(clippy::needless_pass_by_value)]
    pub fn emit_spawn(&self, event: ApCoreEvent) {
        if self.is_shutdown.load(Ordering::SeqCst) {
            return;
        }
        let subscribers: Vec<Arc<dyn EventSubscriber>> = self.subscribers.read().clone();
        for subscriber in &subscribers {
            if !Self::subscriber_matches(subscriber.as_ref(), &event.event_type) {
                continue;
            }
            let sub = Arc::clone(subscriber);
            let evt = event.clone();
            tokio::spawn(async move {
                if let Err(e) = sub.on_event(&evt).await {
                    tracing::warn!(
                        subscriber_id = %sub.subscriber_id(),
                        event_type = %evt.event_type,
                        error = %e,
                        "emit_spawn: subscriber on_event failed"
                    );
                }
            });
        }
    }

    /// Mark this emitter as shut down and flush any pending work.
    ///
    /// After `shutdown` returns:
    /// - Subsequent calls to `emit`, `emit_filtered`, and `emit_spawn` are
    ///   no-ops — the event is dropped.
    /// - Subsequent calls to `shutdown` return `Ok(())` immediately
    ///   (idempotent).
    ///
    /// `timeout_ms` bounds the wait for in-flight deliveries to drain via
    /// [`Self::flush`]. After the flush completes (or times out), no new
    /// events are accepted (sync finding A-D-502). Kept for API parity with
    /// apcore-python and apcore-typescript.
    pub async fn shutdown(&self, timeout_ms: u64) -> Result<(), ModuleError> {
        if self.is_shutdown.swap(true, Ordering::SeqCst) {
            // Already shut down — idempotent.
            return Ok(());
        }
        self.flush(timeout_ms).await
    }

    /// Returns `true` if this emitter has been shut down.
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.is_shutdown.load(Ordering::SeqCst)
    }

    /// Fire-and-forget dispatch with full delivery semantics (retry + DLQ).
    ///
    /// For each matching subscriber, spawns a `tokio::task` running the full
    /// per-subscriber retry loop. On exhaustion (regardless of `max_attempts`):
    /// - a `apcore.event.delivery_failed` DLQ event is delivered to any
    ///   subscriber whose pattern matches that event type, EXCEPT catch-all
    ///   `'*'` subscribers (sync findings A-D-025 / A-D-026).
    /// - `on_failure` is called.
    ///
    /// Spawned tasks are tracked so [`Self::flush`] can await them.
    ///
    /// **Post-shutdown:** drops the event as a no-op.
    #[allow(clippy::needless_pass_by_value)]
    pub fn emit_delivery_semantics(&self, event: ApCoreEvent) {
        if self.is_shutdown.load(Ordering::SeqCst) {
            return;
        }
        // Capture a snapshot for both the delivery loop and DLQ delivery.
        let all_subscribers: Vec<Arc<dyn EventSubscriber>> = self.subscribers.read().clone();

        let mut handles: Vec<JoinHandle<()>> = Vec::new();
        for subscriber in &all_subscribers {
            if !Self::subscriber_matches(subscriber.as_ref(), &event.event_type) {
                continue;
            }
            let sub = Arc::clone(subscriber);
            let evt = event.clone();
            let dlq_subs = all_subscribers.clone();
            handles.push(tokio::spawn(async move {
                Self::deliver_with_dlq(sub, evt, dlq_subs).await;
            }));
        }
        let mut pending = self.pending.lock();
        pending.retain(|h| !h.is_finished());
        pending.extend(handles);
    }

    /// Deliver an event to one subscriber with retry + optional DLQ emission.
    async fn deliver_with_dlq(
        subscriber: Arc<dyn EventSubscriber>,
        event: ApCoreEvent,
        all_subscribers: Vec<Arc<dyn EventSubscriber>>,
    ) {
        let retry = subscriber.retry();
        let mut last_error: Option<ModuleError> = None;

        for attempt in 0..retry.max_attempts {
            match subscriber.on_event(&event).await {
                Ok(()) => return,
                Err(e) => {
                    last_error = Some(e);
                    if attempt + 1 < retry.max_attempts {
                        tokio::time::sleep(Duration::from_millis(retry.compute_delay_ms(attempt)))
                            .await;
                    }
                }
            }
        }

        let err = last_error.unwrap_or_else(|| {
            ModuleError::new(ErrorCode::GeneralInternalError, "unknown delivery failure")
        });

        // A-D-025: always emit the DLQ event on exhaustion, regardless of
        // max_attempts (including single-attempt subscribers). Matches
        // apcore-python / apcore-typescript, which emit the DLQ whenever a
        // subscriber's delivery attempts are exhausted.
        let sub_id = subscriber.subscriber_id().to_string();
        // A-D-029: use the subscriber's DECLARED type rather than parsing the id.
        let subscriber_type = subscriber.subscriber_type().to_string();
        let dlq_event = ApCoreEvent::new(
            DLQ_EVENT_TYPE,
            serde_json::json!({
                "subscriber_type": subscriber_type,
                "subscriber_id": sub_id,
                // A-D-009: original_event MUST use the spec wire keys
                // name/payload/metadata (event-system.md, conformance fixture
                // event_delivery_semantics.json). The original event's
                // module_id/timestamp are preserved under metadata so no
                // information is lost. Mirrors Python emitter.py original_event.
                "original_event": {
                    "name": event.event_type,
                    "payload": event.data,
                    "metadata": {
                        "module_id": event.module_id,
                        "timestamp": event.timestamp,
                    },
                },
                "error": {
                    "type": format!("{:?}", err.code),
                    "message": err.message,
                },
                "attempt_count": retry.max_attempts,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            }),
        );
        for dlq_sub in &all_subscribers {
            // A-D-026: exclude catch-all '*' subscribers from DLQ delivery to
            // prevent cascading failures where every wildcard subscriber would
            // recursively receive a DLQ about itself. Matches apcore-python.
            if dlq_sub.event_pattern() == "*" {
                continue;
            }
            if Self::subscriber_matches(dlq_sub.as_ref(), DLQ_EVENT_TYPE) {
                // DLQ delivery is single-attempt (no retry, no second-order DLQ).
                if let Err(e) = dlq_sub.on_event(&dlq_event).await {
                    tracing::error!(
                        subscriber_id = %dlq_sub.subscriber_id(),
                        error = %e,
                        "DLQ subscriber on_event failed (discarded, not retried)"
                    );
                }
            }
        }

        subscriber
            .on_failure(&event, &err, retry.max_attempts)
            .await;
    }

    /// Whether `subscriber` should receive an event of type `event_type`.
    ///
    /// This is the single decision point every dispatch loop in this file
    /// funnels through, so `on()`'s exact-match contract and every other
    /// subscriber's glob semantics cannot drift apart the way they had.
    ///
    /// A subscriber created through `APCore::on()` / `on_subscriber()` is
    /// wrapped in `EventTypeSubscriber`, which is the only implementor that
    /// overrides `event_type_filter()` to `Some(event_type)`. Per the spec
    /// Contract (`docs/features/apcore-client.md` "## Contract: APCore.on"),
    /// `event_type` there is "filtered by exact equality match inside the
    /// subscriber" — apcore-python's `on()`-created subscriber does
    /// `if event.event_type != self.event_type:` and apcore-typescript's does
    /// `if (event.eventType === eventType)`, neither a glob. So
    /// `client.on("apcore.acl.*", handler)` subscribes to the literal event
    /// type `"apcore.acl.*"`, not a prefix — routing it through
    /// `matches_pattern` here would have it also fire on
    /// `apcore.acl.rule_matched`.
    ///
    /// Every other subscriber (`WebhookSubscriber`, `A2ASubscriber`, a bare
    /// `EventSubscriber` impl, the DLQ's own `"*"` catch-all) reports `None`
    /// from `event_type_filter()` and keeps this crate's `*`-wildcard glob
    /// semantics via `event_pattern()` + [`Self::matches_pattern`] — those
    /// subscribers legitimately rely on glob matching (A-D-026) and are
    /// unaffected.
    fn subscriber_matches(subscriber: &dyn EventSubscriber, event_type: &str) -> bool {
        match subscriber.event_type_filter() {
            Some(exact) => exact == event_type,
            None => Self::matches_pattern(subscriber.event_pattern(), event_type),
        }
    }

    /// Event-type pattern matching (Algorithm A25, PROTOCOL_SPEC §9.16.3).
    ///
    /// - `"*"` matches everything.
    /// - `"foo.*"` matches `"foo.bar"`, `"foo.baz"`, etc.
    /// - `?` matches exactly one character.
    /// - Every other character is a literal, `[`, `]`, `{`, `}` and `\`
    ///   included — a subscriber pattern is never rejected.
    ///
    /// This was a local `*`-only matcher, one of three different metacharacter
    /// sets across the three SDKs while no section of the specification said
    /// which was right (#117).
    fn matches_pattern(pattern: &str, event_type: &str) -> bool {
        crate::utils::helpers::match_glob(pattern, event_type)
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use serde_json::json;
    use std::sync::Arc;

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
            self.received.lock().push(event.event_type.clone());
            Ok(())
        }
    }

    #[test]
    fn test_event_new_defaults() {
        let event = ApCoreEvent::new("test.event", json!({"key": "val"}));
        assert_eq!(event.event_type, "test.event");
        assert_eq!(event.severity, "info");
        assert!(event.module_id.is_none());
        assert_ne!(event.timestamp, "");
    }

    #[test]
    fn test_event_with_module() {
        let event = ApCoreEvent::with_module("err.event", json!({}), "mod.a", "error");
        assert_eq!(event.event_type, "err.event");
        assert_eq!(event.severity, "error");
        assert_eq!(event.module_id.as_deref(), Some("mod.a"));
    }

    #[test]
    fn test_event_serialization_skips_none_module_id() {
        let event = ApCoreEvent::new("test", json!(null));
        let serialized = serde_json::to_value(&event).unwrap();
        assert!(serialized.get("module_id").is_none());
    }

    #[test]
    fn test_emitter_default_max_workers() {
        let emitter = EventEmitter::new();
        assert_eq!(emitter.max_workers, 4);
    }

    #[tokio::test]
    async fn test_emit_to_matching_subscriber() {
        let emitter = EventEmitter::new();
        let sub = RecordingSubscriber::new("sub1", "test.*");
        let received = sub.received.clone();
        emitter.subscribe(Box::new(sub));

        let event = ApCoreEvent::new("test.hello", json!({}));
        emitter.emit(&event).await;
        emitter.flush(5000).await.unwrap();
        assert_eq!(received.lock().len(), 1);
        assert_eq!(received.lock()[0], "test.hello");
    }

    #[tokio::test]
    async fn test_emit_skips_non_matching_subscriber() {
        let emitter = EventEmitter::new();
        let sub = RecordingSubscriber::new("sub1", "other.*");
        let received = sub.received.clone();
        emitter.subscribe(Box::new(sub));

        let event = ApCoreEvent::new("test.hello", json!({}));
        emitter.emit(&event).await;
        assert!(received.lock().is_empty());
    }

    #[tokio::test]
    async fn test_emit_wildcard_matches_all() {
        let emitter = EventEmitter::new();
        let sub = RecordingSubscriber::new("sub1", "*");
        let received = sub.received.clone();
        emitter.subscribe(Box::new(sub));

        let event = ApCoreEvent::new("anything.at.all", json!({}));
        emitter.emit(&event).await;
        emitter.flush(5000).await.unwrap();
        assert_eq!(received.lock().len(), 1);
    }

    #[tokio::test]
    async fn test_unsubscribe_by_id() {
        let emitter = EventEmitter::new();
        let sub = RecordingSubscriber::new("sub1", "*");
        emitter.subscribe(Box::new(sub));
        assert!(emitter.unsubscribe_by_id("sub1"));
        assert!(!emitter.unsubscribe_by_id("sub1"));
    }

    #[tokio::test]
    async fn test_unsubscribe_removes_subscriber() {
        let emitter = EventEmitter::new();
        let sub = RecordingSubscriber::new("sub1", "*");
        let received = sub.received.clone();
        emitter.subscribe(Box::new(sub.clone()));
        emitter.unsubscribe(&sub);

        let event = ApCoreEvent::new("test", json!({}));
        emitter.emit(&event).await;
        assert!(received.lock().is_empty());
    }

    #[tokio::test]
    async fn test_emit_filtered() {
        let emitter = EventEmitter::new();
        let sub = RecordingSubscriber::new("sub1", "test.*");
        let received = sub.received.clone();
        emitter.subscribe(Box::new(sub));

        let event = ApCoreEvent::new("test.hello", json!({}));
        emitter.emit_filtered(&event, "test.*").await.unwrap();
        assert_eq!(received.lock().len(), 1);

        emitter.emit_filtered(&event, "other.*").await.unwrap();
        assert_eq!(received.lock().len(), 1);
    }

    #[tokio::test]
    async fn test_flush_succeeds() {
        let emitter = EventEmitter::new();
        emitter.flush(1000).await.unwrap();
    }

    #[test]
    fn test_matches_pattern_wildcard() {
        assert!(EventEmitter::matches_pattern("*", "anything"));
    }

    #[test]
    fn test_matches_pattern_exact() {
        assert!(EventEmitter::matches_pattern("test.event", "test.event"));
        assert!(!EventEmitter::matches_pattern("test.event", "test.other"));
    }

    #[test]
    fn test_matches_pattern_prefix_wildcard() {
        assert!(EventEmitter::matches_pattern("test.*", "test.hello"));
        assert!(EventEmitter::matches_pattern("test.*", "test."));
        assert!(!EventEmitter::matches_pattern("test.*", "other.hello"));
    }

    #[test]
    fn test_matches_pattern_suffix_wildcard() {
        assert!(EventEmitter::matches_pattern("*.event", "test.event"));
        assert!(!EventEmitter::matches_pattern("*.event", "test.other"));
    }

    #[test]
    fn test_matches_pattern_middle_wildcard() {
        assert!(EventEmitter::matches_pattern("a.*.z", "a.b.z"));
        assert!(EventEmitter::matches_pattern("a.*.z", "a.anything.z"));
        assert!(!EventEmitter::matches_pattern("a.*.z", "a.b.c"));
    }

    #[tokio::test]
    async fn test_emit_error_isolation() {
        #[derive(Debug)]
        struct FailingSub;

        #[async_trait]
        impl EventSubscriber for FailingSub {
            fn subscriber_id(&self) -> &'static str {
                "fail"
            }
            fn event_pattern(&self) -> &'static str {
                "*"
            }
            async fn on_event(&self, _event: &ApCoreEvent) -> Result<(), ModuleError> {
                Err(ModuleError::new(
                    crate::errors::ErrorCode::GeneralInternalError,
                    "boom",
                ))
            }
        }

        let emitter = EventEmitter::new();
        emitter.subscribe(Box::new(FailingSub));
        let good_sub = RecordingSubscriber::new("good", "*");
        let received = good_sub.received.clone();
        emitter.subscribe(Box::new(good_sub));

        let event = ApCoreEvent::new("test", json!({}));
        // Use the sequential single-attempt path so this test focuses on
        // error isolation without invoking the retry+DLQ machinery (which
        // is exercised by tests/test_v022_event_delivery_semantics.rs).
        emitter.emit_sequential(&event).await;
        assert_eq!(received.lock().len(), 1);
    }
}
