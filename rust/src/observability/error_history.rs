// APCore Protocol — Error history tracking with fingerprinting & min-heap eviction
// Spec reference: observability.md §1.3, §1.4

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;

use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};
use crate::middleware::base::Middleware;

use super::storage::StorageBackend;
use super::store::{InMemoryObservabilityStore, ObservabilityStore};

/// Canonical string representation of an `ErrorCode` (the SCREAMING_SNAKE_CASE
/// serde name, e.g. `MODULE_EXECUTE_ERROR`). This MUST match the wire format
/// emitted by `ModuleError` serialization and the corresponding Python/TS
/// representation, so that cross-language fingerprints (observability.md §1.4)
/// are stable. Falls back to Debug formatting only if the enum lacks a serde
/// representation, which should never occur in practice.
fn error_code_string(code: ErrorCode) -> String {
    // Delegates to the shared canonical wire-string method on ErrorCode so the
    // metrics middleware and error history use one source of truth (sync
    // finding A-D-14).
    code.wire_str()
}

/// A recorded error entry with deduplication support.
///
/// `fingerprint` is the SHA-256 of `error_code:module_id:normalized_message`.
/// Two errors that differ only by ephemeral values (UUIDs, large integers,
/// ISO 8601 timestamps) share the same fingerprint and are deduplicated.
/// Format an instant as `2026-09-16T10:30:00.123Z` (D-120).
///
/// Exactly three fractional digits and a `Z` suffix. chrono's `to_rfc3339()`
/// gives `+00:00` and whatever precision the instant happens to carry, and
/// serde's default for `DateTime<Utc>` gives `Z` with microseconds — two forms
/// inside one SDK, neither of them the one the spec's example uses.
///
/// Specifying the suffix alone would have left the precisions diverging behind
/// one `Z`: the same divergence, harder to see. That is why the decision fixes
/// both, and why this is applied at the PRODUCER — the record's own
/// serialization and every reader that formats it — rather than at one summary
/// on the way out, which would leave the stored record and every other
/// consumer on the old form.
#[must_use]
pub fn format_millis_z(ts: DateTime<Utc>) -> String {
    ts.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn serialize_millis_z<S>(ts: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&format_millis_z(*ts))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEntry {
    pub module_id: String,
    /// Canonical error code string. Serializes as `code` to match the
    /// cross-language wire format (Python `ErrorEntry.code`, observability.md §1.4).
    #[serde(rename = "code", alias = "error_code")]
    pub error_code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<String>,
    #[serde(serialize_with = "serialize_millis_z")]
    pub timestamp: DateTime<Utc>,
    pub count: u64,
    #[serde(serialize_with = "serialize_millis_z")]
    pub first_occurred: DateTime<Utc>,
    /// Most recent occurrence; semantically equivalent to spec's `last_seen_at`.
    #[serde(serialize_with = "serialize_millis_z")]
    pub last_occurred: DateTime<Utc>,
    /// SHA-256(error_code:module_id:normalize(message)) as 64-char lowercase hex.
    #[serde(default)]
    pub fingerprint: String,
}

// ---------------------------------------------------------------------------
// Normalization & fingerprinting (observability.md §1.4)
// ---------------------------------------------------------------------------

fn uuid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
            .expect("UUID regex is valid")
    })
}

fn iso8601_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?")
            .expect("ISO8601 regex is valid")
    })
}

fn integer_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b\d{4,}\b").expect("integer regex is valid"))
}

/// Normalize a message by replacing ephemeral values with placeholders.
///
/// Order is significant:
/// 1. UUIDs → `<uuid>`
/// 2. ISO 8601 timestamps → `<timestamp>` (must precede integers — years are 4 digits)
/// 3. Integers ≥ 4 digits → `<id>`
///
/// Final result is lowercased and trimmed.
#[must_use]
pub fn normalize_message(msg: &str) -> String {
    let s = uuid_re().replace_all(msg, "<UUID>");
    let s = iso8601_re().replace_all(&s, "<TIMESTAMP>");
    let s = integer_re().replace_all(&s, "<ID>");
    s.trim().to_lowercase()
}

/// Compute the deduplication fingerprint for an error.
#[must_use]
pub fn compute_fingerprint(error_code: &str, module_id: &str, message: &str) -> String {
    let normalized = normalize_message(message);
    let raw = format!("{error_code}:{module_id}:{normalized}");
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute the deduplication fingerprint directly from a `ModuleError`.
///
/// Issue #43 §4 — fingerprint = SHA-256 of
/// `<error_code>:<module_id>:<sanitized_message>`. The message is sanitized
/// via [`normalize_message`], which strips UUIDs, ISO 8601 timestamps, and
/// digit-runs of length ≥ 4 so two errors that differ only in ephemeral
/// identifiers share a fingerprint. `ModuleError` does not carry a module_id
/// (the call site does), so callers MUST supply it explicitly. Aligned with
/// the cross-language fingerprinting algorithm in
/// `apcore-python.compute_fingerprint(error, module_id)` and
/// `apcore-typescript.computeFingerprint(error, moduleId)`.
#[must_use]
pub fn compute_fingerprint_from_error(error: &ModuleError, module_id: &str) -> String {
    let error_code = error_code_string(error.code);
    compute_fingerprint(&error_code, module_id, &error.message)
}

// ---------------------------------------------------------------------------
// ErrorHistory — pluggable storage + min-heap eviction + fingerprint dedup
// ---------------------------------------------------------------------------

/// Internal heap entry: (last_occurred, monotonic seq, fingerprint).
/// Wrapped in `Reverse` so `BinaryHeap` becomes a min-heap on last_occurred.
type HeapEntry = Reverse<(DateTime<Utc>, u64, String)>;

#[derive(Debug, Default)]
struct ErrorHistoryState {
    /// fingerprint → entry (O(1) dedup lookup).
    fp_index: HashMap<String, ErrorEntry>,
    /// module_id → fingerprints in insertion order. `VecDeque` gives O(1)
    /// `pop_front` for per-module eviction (observability.md §1.3 mandates
    /// O(log N) — this is the per-module half; heap pop is O(log N) total).
    module_index: HashMap<String, VecDeque<String>>,
    /// Min-heap on last_occurred timestamp for O(log N) eviction.
    /// Lazy deletion: stale entries (where stored timestamp != entry.last_occurred,
    /// or fingerprint already evicted) are skipped on pop. Bound is roughly
    /// `max_total_entries × dedup_factor` — every dedup hit pushes a new heap
    /// entry that becomes stale immediately.
    heap: BinaryHeap<HeapEntry>,
    /// Tie-breaker for heap ordering when timestamps collide.
    seq: u64,
}

/// Total-entry limit when a constructor does not take one.
const DEFAULT_MAX_TOTAL_ENTRIES: usize = 1000;

/// Stores a history of errors with O(log N) eviction and SHA-256 deduplication.
///
/// Construction injects a `Arc<dyn ObservabilityStore>`; the default store is
/// `InMemoryObservabilityStore`. The store MUST NOT be replaced after
/// construction (observability.md §1.1).
#[derive(Debug, Clone)]
pub struct ErrorHistory {
    state: Arc<Mutex<ErrorHistoryState>>,
    max_entries_per_module: usize,
    max_total_entries: usize,
    store: Arc<dyn ObservabilityStore>,
    /// Issue #43 §1: optional `StorageBackend` for cross-process persistence.
    /// When set, every `record()` is also forwarded to the backend under
    /// namespace `"error_history"` keyed by fingerprint. The bundled
    /// `ObservabilityStore` is independent of this — both are kept so existing
    /// integrations don't have to change.
    storage_backend: Option<Arc<dyn StorageBackend>>,
}

impl ErrorHistory {
    /// Create a new error history with default in-memory store and the
    /// default total limit (1000), as apcore-python and apcore-typescript do
    /// when only the per-module limit is given.
    #[must_use]
    pub fn new(max_entries_per_module: usize) -> Self {
        Self::with_store_and_limits(
            max_entries_per_module,
            DEFAULT_MAX_TOTAL_ENTRIES,
            Arc::new(InMemoryObservabilityStore::new()),
        )
    }

    /// Create with explicit per-module and total limits, default in-memory store.
    #[must_use]
    pub fn with_limits(max_entries_per_module: usize, max_total_entries: usize) -> Self {
        Self::with_store_and_limits(
            max_entries_per_module,
            max_total_entries,
            Arc::new(InMemoryObservabilityStore::new()),
        )
    }

    /// Create with an explicit observability store and default limits (50 / 1000).
    #[must_use]
    pub fn with_store(store: Arc<dyn ObservabilityStore>) -> Self {
        Self::with_store_and_limits(50, DEFAULT_MAX_TOTAL_ENTRIES, store)
    }

    /// Create with explicit limits and an observability store.
    #[must_use]
    pub fn with_store_and_limits(
        max_entries_per_module: usize,
        max_total_entries: usize,
        store: Arc<dyn ObservabilityStore>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ErrorHistoryState::default())),
            max_entries_per_module,
            max_total_entries,
            store,
            // D-113: an omitted backend is the in-memory one, on every
            // constructor that does not take one.
            storage_backend: Some(crate::observability::storage::default_storage_backend()),
        }
    }

    /// Create with explicit limits and an optional `StorageBackend` (Issue
    /// #43 §1). The internal `ObservabilityStore` is the default in-memory
    /// one; the storage backend is purely additive — when supplied, every
    /// recorded `ErrorEntry` is also persisted under namespace
    /// `"error_history"` so external consumers can read it.
    #[must_use]
    pub fn with_storage_backend(
        max_entries_per_module: usize,
        max_total_entries: usize,
        storage_backend: Option<Arc<dyn StorageBackend>>,
    ) -> Self {
        // D-113: an OMITTED backend means the bundled in-memory one, not "no
        // storage". Only apcore-typescript honoured that, so the same omission
        // produced a working store there and a silent no-op here — and a caller
        // reading records back got an empty list rather than an error. The
        // parameter stays `Option` so existing callers compile unchanged;
        // `None` now resolves to the default rather than disabling persistence.
        let storage_backend = Some(
            storage_backend.unwrap_or_else(crate::observability::storage::default_storage_backend),
        );
        Self {
            state: Arc::new(Mutex::new(ErrorHistoryState::default())),
            max_entries_per_module,
            max_total_entries,
            store: Arc::new(InMemoryObservabilityStore::new()),
            storage_backend,
        }
    }

    /// Attach an optional `StorageBackend` after construction (builder-style).
    #[must_use]
    pub fn with_storage(mut self, storage_backend: Option<Arc<dyn StorageBackend>>) -> Self {
        self.storage_backend = storage_backend;
        self
    }

    /// Get a clone of the underlying store handle.
    #[must_use]
    pub fn store(&self) -> Arc<dyn ObservabilityStore> {
        self.store.clone()
    }

    /// Record an error, deduplicating by fingerprint. Uses the current time.
    pub fn record(&self, module_id: &str, error: &ModuleError) {
        self.record_at(module_id, error, Utc::now());
    }

    /// The resolved storage backend (D-113).
    ///
    /// apcore-typescript exposes `history.storage` and apcore-python
    /// `_storage`; this SDK exposed nothing, so "an omitted backend means the
    /// in-memory one" was not observable from outside at all — the decision was
    /// untestable here rather than merely unimplemented, the same shape D-91
    /// settled for `ExtensionManager::unregister`.
    ///
    /// Always `Some` since D-113: an omitted backend resolves to the default
    /// rather than disabling persistence.
    #[must_use]
    pub fn storage_backend(&self) -> Option<Arc<dyn StorageBackend>> {
        self.storage_backend.clone()
    }

    /// Record an error at an explicit timestamp. Used by conformance tests
    /// to verify time-based eviction behavior independently of wall-clock.
    pub fn record_at(&self, module_id: &str, error: &ModuleError, when: DateTime<Utc>) {
        let error_code = error_code_string(error.code);
        let fp = compute_fingerprint(&error_code, module_id, &error.message);

        // Capture the entry to forward to the store BEFORE eviction can run —
        // when `max_total_entries` is small or `when` is older than every
        // existing `last_occurred`, eviction may pop the entry we just inserted.
        let entry_to_notify: ErrorEntry;
        {
            let mut state = self.state.lock();

            if let Some(existing) = state.fp_index.get_mut(&fp) {
                existing.count += 1;
                existing.last_occurred = when;
                existing.timestamp = when;
                entry_to_notify = existing.clone();
                state.seq += 1;
                let seq = state.seq;
                state.heap.push(Reverse((when, seq, fp.clone())));
            } else {
                let entry = ErrorEntry {
                    module_id: module_id.to_string(),
                    error_code,
                    message: error.message.clone(),
                    ai_guidance: error.ai_guidance.clone(),
                    timestamp: when,
                    count: 1,
                    first_occurred: when,
                    last_occurred: when,
                    fingerprint: fp.clone(),
                };
                entry_to_notify = entry.clone();
                state.fp_index.insert(fp.clone(), entry);
                state
                    .module_index
                    .entry(module_id.to_string())
                    .or_default()
                    .push_back(fp.clone());
                state.seq += 1;
                let seq = state.seq;
                state.heap.push(Reverse((when, seq, fp)));

                // Eviction may now remove the entry we just inserted (if it has
                // the oldest last_occurred); that's fine — the store has already
                // been notified above with the entry's full state.
                evict_per_module(&mut state, module_id, self.max_entries_per_module);
                evict_total(&mut state, self.max_total_entries);
            }
        }

        // Notify the store outside the internal lock to avoid lock-ordering issues.
        // Use try_current so callers in non-async contexts (e.g. unit tests)
        // do not panic; if no runtime is active the notification is dropped and
        // a debug log is emitted so production callers see the silent path.
        let store = self.store.clone();
        let backend = self.storage_backend.clone();
        let entry_clone = entry_to_notify.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                store.record_error(entry_to_notify).await;
            });
            if let Some(backend) = backend {
                let fp = entry_clone.fingerprint.clone();
                handle.spawn(async move {
                    if let Ok(value) = serde_json::to_value(&entry_clone) {
                        let _ = backend.save("error_history", &fp, value).await;
                    }
                });
            }
        } else {
            tracing::debug!(
                "ErrorHistory::record called outside a tokio runtime; \
                 store notification skipped"
            );
        }
    }

    /// Get errors for a specific module, most recently CREATED first.
    ///
    /// A repeat of an existing error updates its `count` and `last_occurred`
    /// but keeps its place, as in apcore-python and apcore-typescript; use
    /// [`Self::get_all`] for recency of occurrence.
    #[must_use]
    pub fn get(&self, module_id: &str, limit: Option<usize>) -> Vec<ErrorEntry> {
        let state = self.state.lock();
        let Some(fps) = state.module_index.get(module_id) else {
            return Vec::new();
        };
        let mut entries: Vec<ErrorEntry> = fps
            .iter()
            .rev()
            .filter_map(|fp| state.fp_index.get(fp).cloned())
            .collect();
        if let Some(n) = limit {
            entries.truncate(n);
        }
        entries
    }

    /// Get all recorded errors across all modules, sorted by `last_occurred` desc.
    #[must_use]
    pub fn get_all(&self, limit: Option<usize>) -> Vec<ErrorEntry> {
        let state = self.state.lock();
        let mut all: Vec<ErrorEntry> = state.fp_index.values().cloned().collect();
        all.sort_by_key(|e| Reverse(e.last_occurred));
        if let Some(n) = limit {
            all.truncate(n);
        }
        all
    }

    /// Total number of distinct (deduplicated) entries currently retained.
    #[must_use]
    pub fn count(&self) -> usize {
        self.state.lock().fp_index.len()
    }

    /// Clear errors. If `module_id` is Some, clear only that module; otherwise clear all.
    pub fn clear(&self, module_id: Option<&str>) {
        let mut state = self.state.lock();
        if let Some(id) = module_id {
            if let Some(fps) = state.module_index.remove(id) {
                for fp in fps {
                    state.fp_index.remove(&fp);
                }
            }
        } else {
            state.fp_index.clear();
            state.module_index.clear();
            state.heap.clear();
            state.seq = 0;
        }
    }
}

fn evict_per_module(state: &mut ErrorHistoryState, module_id: &str, max_per_module: usize) {
    let Some(fps) = state.module_index.get_mut(module_id) else {
        return;
    };
    while fps.len() > max_per_module {
        if let Some(evicted_fp) = fps.pop_front() {
            state.fp_index.remove(&evicted_fp);
        } else {
            break;
        }
    }
    if state
        .module_index
        .get(module_id)
        .is_some_and(VecDeque::is_empty)
    {
        state.module_index.remove(module_id);
    }
}

fn evict_total(state: &mut ErrorHistoryState, max_total: usize) {
    while state.fp_index.len() > max_total {
        if !pop_oldest(state) {
            break;
        }
    }
}

/// Pop the oldest live entry from the heap; returns true if an entry was removed.
fn pop_oldest(state: &mut ErrorHistoryState) -> bool {
    while let Some(Reverse((heap_ts, _seq, fp))) = state.heap.pop() {
        if let Some(entry) = state.fp_index.get(&fp) {
            // Skip stale heap entries: dedup may have refreshed last_occurred,
            // leaving older heap entries that no longer reflect entry state.
            if entry.last_occurred != heap_ts {
                continue;
            }
            let module_id = entry.module_id.clone();
            state.fp_index.remove(&fp);
            if let Some(fps) = state.module_index.get_mut(&module_id) {
                fps.retain(|f| f != &fp);
                if fps.is_empty() {
                    state.module_index.remove(&module_id);
                }
            }
            return true;
        }
    }
    false
}

/// Middleware that records errors into an `ErrorHistory`.
#[derive(Debug)]
pub struct ErrorHistoryMiddleware {
    history: ErrorHistory,
}

impl ErrorHistoryMiddleware {
    /// Create a new error history middleware.
    #[must_use]
    pub fn new(history: ErrorHistory) -> Self {
        Self { history }
    }

    /// Get a reference to the underlying error history.
    #[must_use]
    pub fn history(&self) -> &ErrorHistory {
        &self.history
    }
}

#[async_trait]
impl Middleware for ErrorHistoryMiddleware {
    fn name(&self) -> &'static str {
        "error_history"
    }

    async fn before(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }

    async fn after(
        &self,
        _module_id: &str,
        _inputs: serde_json::Value,
        _output: serde_json::Value,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }

    async fn on_error(
        &self,
        module_id: &str,
        _inputs: serde_json::Value,
        error: &ModuleError,
        _ctx: &Context<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        self.history.record(module_id, error);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_replaces_uuid() {
        let s = normalize_message("token a1b2c3d4-e5f6-7890-abcd-ef1234567890 is invalid");
        assert_eq!(s, "token <uuid> is invalid");
    }

    #[test]
    fn normalize_replaces_integers_over_3_digits() {
        // Word-boundary on both sides: matches numeric tokens flanked by non-word chars.
        let s = normalize_message("retry after 30000 ms");
        assert_eq!(s, "retry after <id> ms");
    }

    #[test]
    fn normalize_replaces_iso8601() {
        let s = normalize_message("at 2026-01-01T10:00:00Z something failed");
        assert_eq!(s, "at <timestamp> something failed");
    }

    #[test]
    fn fingerprint_is_64_char_hex() {
        let fp = compute_fingerprint("DB_TIMEOUT", "executor.db", "connection timed out");
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_dedup_normalized_messages() {
        let a = compute_fingerprint(
            "TOKEN_INVALID",
            "executor.auth",
            "token a1b2c3d4-e5f6-7890-abcd-ef1234567890 is invalid",
        );
        let b = compute_fingerprint(
            "TOKEN_INVALID",
            "executor.auth",
            "token 00000000-0000-0000-0000-000000000001 is invalid",
        );
        assert_eq!(a, b);
    }
}
