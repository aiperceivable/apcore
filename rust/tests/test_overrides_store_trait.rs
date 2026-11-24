//! Tests for the `OverridesStore` trait abstraction (sync alignment finding).
//!
//! Spec: cross-language `OverridesStore` trait with `InMemoryOverridesStore`
//! and `FileOverridesStore` implementations. Both must be `Send + Sync` and
//! load/save a `HashMap<String, serde_json::Value>` of override entries.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::sys_modules::overrides::{FileOverridesStore, InMemoryOverridesStore, OverridesStore};
use serde_json::json;

#[tokio::test]
async fn in_memory_overrides_store_load_returns_empty_when_unsaved() {
    let store: Arc<dyn OverridesStore> = Arc::new(InMemoryOverridesStore::new());
    let loaded = store.load().await.expect("load must not error");
    assert!(loaded.is_empty(), "fresh in-memory store must be empty");
}

#[tokio::test]
async fn in_memory_overrides_store_save_then_load_roundtrips() {
    let store: Arc<dyn OverridesStore> = Arc::new(InMemoryOverridesStore::new());
    let mut data: HashMap<String, serde_json::Value> = HashMap::new();
    data.insert("foo.bar".to_string(), json!(42));
    data.insert("toggle.system.health.module".to_string(), json!(false));

    store.save(&data).await.expect("save must not error");
    let loaded = store.load().await.expect("load must not error");

    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded.get("foo.bar"), Some(&json!(42)));
    assert_eq!(
        loaded.get("toggle.system.health.module"),
        Some(&json!(false))
    );
}

#[tokio::test]
async fn file_overrides_store_load_returns_empty_when_path_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.yaml");
    let store: Arc<dyn OverridesStore> = Arc::new(FileOverridesStore::new(path));
    let loaded = store.load().await.expect("missing file must not error");
    assert!(loaded.is_empty(), "missing file should yield empty map");
}

#[tokio::test]
async fn file_overrides_store_save_persists_to_disk_and_load_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("overrides.yaml");
    let store: Arc<dyn OverridesStore> = Arc::new(FileOverridesStore::new(path.clone()));

    let mut data: HashMap<String, serde_json::Value> = HashMap::new();
    data.insert("retry.timeout_ms".to_string(), json!(5000));
    data.insert("toggle.foo".to_string(), json!(true));
    store.save(&data).await.expect("save must succeed");

    assert!(path.exists(), "save must create the file");

    // Re-open the same path and confirm the data round-trips.
    let store2: Arc<dyn OverridesStore> = Arc::new(FileOverridesStore::new(path));
    let loaded = store2.load().await.expect("load must succeed");
    assert_eq!(loaded.get("retry.timeout_ms"), Some(&json!(5000)));
    assert_eq!(loaded.get("toggle.foo"), Some(&json!(true)));
}

#[tokio::test]
async fn legacy_load_overrides_function_still_available() {
    // The free function should remain as a thin wrapper for backward compatibility.
    use apcore::config::Config;
    use apcore::sys_modules::overrides::load_overrides;
    use apcore::sys_modules::ToggleState;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.yaml");
    std::fs::write(&path, "foo.bar: 7\n").unwrap();

    let mut cfg = Config::default();
    let toggle = ToggleState::new();
    load_overrides(&path, &mut cfg, Some(&toggle));

    assert_eq!(cfg.get("foo.bar"), Some(json!(7)));
}
