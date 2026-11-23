// APCore Protocol — Pluggable key/value storage backend for observability.
// Spec reference: observability.md §Pluggable Observability Storage (Issue #43 §1).
//
// `StorageBackend` is a *generic* namespace/key/value KV surface that
// `ErrorHistory`, `UsageCollector`, and `MetricsCollector` can adopt as their
// persistence layer. Production deployments may swap in Redis-, SQL-, or
// S3-backed implementations; the bundled `InMemoryStorageBackend` covers
// in-process tests and single-process daemons.
//
// The trait is intentionally minimal — `save`/`get`/`list`/`delete` are the
// four operations every collector needs. Anything richer (range scans,
// transactions, TTL) belongs in adapter crates that extend this trait, not in
// the protocol surface.

use async_trait::async_trait;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

/// Errors a `StorageBackend` may surface. Network-backed implementations
/// typically wrap their underlying client error in `Backend(...)`; the
/// in-memory default never errors but exposes the variant for symmetry.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The backing store rejected an operation (network failure, serialization
    /// error, permission denied, etc.). The wrapped string carries the
    /// implementation-specific reason.
    #[error("storage backend error: {0}")]
    Backend(String),
}

/// Pluggable namespace/key/value storage abstraction (observability.md §1).
///
/// Implementations MUST be `Send + Sync` (collectors are shared across
/// async tasks). All operations are async to accommodate network-backed
/// stores, even though the in-memory default completes synchronously.
///
/// Semantics:
/// * `save(ns, k, v)` — create or overwrite. The previous value, if any, is
///   discarded silently.
/// * `get(ns, k)` — `Ok(None)` for absent keys; `Err` only on backend errors.
/// * `list(ns, prefix)` — return all `(key, value)` pairs whose key starts
///   with `prefix`. An empty prefix returns every entry in the namespace.
///   Implementations MAY return entries in arbitrary order; callers that
///   need a stable order MUST sort.
/// * `delete(ns, k)` — idempotent: deleting an absent key is a no-op and
///   MUST NOT return an error.
///
/// Namespaces MUST be isolated: writes to namespace `A` MUST NOT be visible
/// to reads against namespace `B`.
#[async_trait]
pub trait StorageBackend: Send + Sync + std::fmt::Debug {
    /// Create or overwrite the value at `(namespace, key)`.
    async fn save(
        &self,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StorageError>;

    /// Retrieve the value at `(namespace, key)`. Returns `Ok(None)` when the
    /// key is absent — only true backend errors propagate as `Err`.
    async fn get(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StorageError>;

    /// List `(key, value)` pairs in `namespace` whose key starts with
    /// `prefix`. An empty prefix returns every entry in the namespace.
    async fn list(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<Vec<(String, serde_json::Value)>, StorageError>;

    /// Remove `(namespace, key)`. Idempotent — deleting an absent key is
    /// a no-op (MUST NOT return an error).
    async fn delete(&self, namespace: &str, key: &str) -> Result<(), StorageError>;
}

/// Default in-memory `StorageBackend`. Thread-safe (RwLock), namespace-isolated.
///
/// Designed for tests and single-process daemons. Production deployments that
/// need durability across restarts implement `StorageBackend` themselves
/// against Redis/Postgres/etc.
#[derive(Debug, Default)]
pub struct InMemoryStorageBackend {
    // namespace -> key -> value
    inner: RwLock<HashMap<String, HashMap<String, serde_json::Value>>>,
}

impl InMemoryStorageBackend {
    /// Create an empty backend.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl StorageBackend for InMemoryStorageBackend {
    async fn save(
        &self,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StorageError> {
        let mut guard = self.inner.write();
        guard
            .entry(namespace.to_string())
            .or_default()
            .insert(key.to_string(), value);
        Ok(())
    }

    async fn get(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        let guard = self.inner.read();
        Ok(guard.get(namespace).and_then(|ns| ns.get(key)).cloned())
    }

    async fn list(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<Vec<(String, serde_json::Value)>, StorageError> {
        let guard = self.inner.read();
        let Some(ns) = guard.get(namespace) else {
            return Ok(Vec::new());
        };
        Ok(ns
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    async fn delete(&self, namespace: &str, key: &str) -> Result<(), StorageError> {
        // Idempotent: missing namespace OR missing key both succeed silently.
        let mut guard = self.inner.write();
        if let Some(ns) = guard.get_mut(namespace) {
            ns.remove(key);
        }
        Ok(())
    }
}

/// Convenience constructor for the most common wiring pattern: an
/// `Arc<dyn StorageBackend>` pointing at a fresh `InMemoryStorageBackend`.
#[must_use]
pub fn default_storage_backend() -> Arc<dyn StorageBackend> {
    Arc::new(InMemoryStorageBackend::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn save_get_roundtrip() {
        let b = InMemoryStorageBackend::new();
        b.save("ns", "k", json!({"x": 1})).await.unwrap();
        assert_eq!(b.get("ns", "k").await.unwrap(), Some(json!({"x": 1})));
    }

    #[tokio::test]
    async fn delete_is_idempotent_for_missing_keys() {
        let b = InMemoryStorageBackend::new();
        // delete from empty backend — no error
        b.delete("ns", "absent").await.unwrap();
        // delete from existing namespace, missing key — no error
        b.save("ns", "k", json!(1)).await.unwrap();
        b.delete("ns", "absent").await.unwrap();
        assert_eq!(b.get("ns", "k").await.unwrap(), Some(json!(1)));
    }

    #[tokio::test]
    async fn list_returns_only_prefix_matches() {
        let b = InMemoryStorageBackend::new();
        b.save("ns", "user/1", json!(1)).await.unwrap();
        b.save("ns", "post/1", json!(2)).await.unwrap();
        let got = b.list("ns", "user/").await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "user/1");
    }

    #[tokio::test]
    async fn namespaces_are_isolated() {
        let b = InMemoryStorageBackend::new();
        b.save("a", "k", json!(1)).await.unwrap();
        b.save("b", "k", json!(2)).await.unwrap();
        assert_eq!(b.get("a", "k").await.unwrap(), Some(json!(1)));
        assert_eq!(b.get("b", "k").await.unwrap(), Some(json!(2)));
    }
}
