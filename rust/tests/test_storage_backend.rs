// Issue #43 §1: StorageBackend trait + InMemoryStorageBackend default.
//
// Verifies:
//   - InMemoryStorageBackend implements save/get/list/delete correctly.
//   - Namespaces are isolated (writes to one namespace are invisible to another).
//   - delete is idempotent (deleting an absent key is a no-op).
//   - list filters by prefix when provided.
//   - ErrorHistory accepts an optional StorageBackend at construction.

use apcore::observability::storage::{InMemoryStorageBackend, StorageBackend};
use apcore::observability::ErrorHistory;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn save_and_get_roundtrips_value() {
    let backend = InMemoryStorageBackend::new();
    backend
        .save("ns", "k1", json!({"a": 1}))
        .await
        .expect("save ok");
    let v = backend.get("ns", "k1").await.expect("get ok");
    assert_eq!(v, Some(json!({"a": 1})));
}

#[tokio::test]
async fn get_missing_returns_none() {
    let backend = InMemoryStorageBackend::new();
    let v = backend.get("ns", "missing").await.expect("get ok");
    assert_eq!(v, None);
}

#[tokio::test]
async fn namespaces_are_isolated() {
    let backend = InMemoryStorageBackend::new();
    backend.save("ns_a", "k", json!(1)).await.unwrap();
    backend.save("ns_b", "k", json!(2)).await.unwrap();
    assert_eq!(backend.get("ns_a", "k").await.unwrap(), Some(json!(1)));
    assert_eq!(backend.get("ns_b", "k").await.unwrap(), Some(json!(2)));
}

#[tokio::test]
async fn list_filters_by_prefix() {
    let backend = InMemoryStorageBackend::new();
    backend.save("ns", "user/1", json!("a")).await.unwrap();
    backend.save("ns", "user/2", json!("b")).await.unwrap();
    backend.save("ns", "post/1", json!("c")).await.unwrap();
    let mut entries = backend.list("ns", "user/").await.unwrap();
    entries.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, "user/1");
    assert_eq!(entries[1].0, "user/2");
}

#[tokio::test]
async fn list_with_empty_prefix_returns_all() {
    let backend = InMemoryStorageBackend::new();
    backend.save("ns", "a", json!(1)).await.unwrap();
    backend.save("ns", "b", json!(2)).await.unwrap();
    let entries = backend.list("ns", "").await.unwrap();
    assert_eq!(entries.len(), 2);
}

#[tokio::test]
async fn delete_is_idempotent() {
    let backend = InMemoryStorageBackend::new();
    // delete absent key — should not error
    backend.delete("ns", "missing").await.expect("idempotent");
    backend.save("ns", "k", json!(1)).await.unwrap();
    backend.delete("ns", "k").await.unwrap();
    assert_eq!(backend.get("ns", "k").await.unwrap(), None);
    // delete again — still ok
    backend.delete("ns", "k").await.unwrap();
}

#[tokio::test]
async fn error_history_accepts_optional_storage_backend() {
    // Constructor MUST allow passing an Option<Arc<dyn StorageBackend>>.
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryStorageBackend::new());
    let _history = ErrorHistory::with_storage_backend(50, 1000, Some(backend));
}

#[tokio::test]
async fn error_history_accepts_none_storage_backend() {
    let _history = ErrorHistory::with_storage_backend(50, 1000, None);
}
