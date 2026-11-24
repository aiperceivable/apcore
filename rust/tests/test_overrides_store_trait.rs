//! Tests for the `OverridesStore` trait abstraction (sync alignment finding).
//!
//! Spec: cross-language `OverridesStore` trait with `InMemoryOverridesStore`
//! and `FileOverridesStore` implementations. Both must be `Send + Sync` and
//! load/save a `HashMap<String, serde_json::Value>` of override entries.

use std::collections::HashMap;
use std::sync::Arc;

use apcore::sys_modules::overrides::{
    persist_one, write_override, FileOverridesStore, InMemoryOverridesStore, OverridesError,
    OverridesStore,
};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;

#[derive(Default)]
struct TestStore {
    data: Mutex<HashMap<String, serde_json::Value>>,
    fail_reads: bool,
}

#[async_trait]
impl OverridesStore for TestStore {
    async fn load(&self) -> Result<HashMap<String, serde_json::Value>, OverridesError> {
        if self.fail_reads {
            return Err(OverridesError::Serialize("test read failure".into()));
        }
        let snapshot = self.data.lock().clone();
        tokio::task::yield_now().await;
        Ok(snapshot)
    }

    async fn save(
        &self,
        overrides: &HashMap<String, serde_json::Value>,
    ) -> Result<(), OverridesError> {
        self.data.lock().clone_from(overrides);
        Ok(())
    }
}

#[tokio::test]
async fn test_persist_one_does_not_save_after_read_failure() {
    let store = TestStore {
        data: Mutex::new(HashMap::from([("toggle.demo.held".into(), json!(false))])),
        fail_reads: true,
    };
    let result = persist_one(&store, "executor.default_timeout", &json!(700)).await;
    assert!(result.is_err());
    assert_eq!(
        *store.data.lock(),
        HashMap::from([("toggle.demo.held".into(), json!(false))])
    );
}

#[tokio::test]
async fn test_persist_one_serializes_updates_to_the_same_store() {
    let store = TestStore::default();
    let first = json!(10);
    let second = json!(false);
    let (updated, toggled) = tokio::join!(
        persist_one(&store, "executor.max_call_depth", &first),
        persist_one(&store, "toggle.demo.held", &second),
    );
    updated.unwrap();
    toggled.unwrap();
    assert_eq!(
        *store.data.lock(),
        HashMap::from([
            ("executor.max_call_depth".into(), first),
            ("toggle.demo.held".into(), second),
        ]),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_file_updates_serialize_across_instances_and_legacy_writers() {
    const UPDATES: u32 = 24;
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let path = project.join("state/overrides.yaml");
    #[cfg(unix)]
    let alias = {
        let link = dir.path().join("project-link");
        std::os::unix::fs::symlink(&project, &link).unwrap();
        link.join("state/overrides.yaml")
    };
    #[cfg(not(unix))]
    let alias = path.clone();
    let start = Arc::new(tokio::sync::Barrier::new(UPDATES as usize));
    let mut writers = Vec::new();
    for index in 0..UPDATES {
        let path = if index % 4 < 2 { &path } else { &alias }.clone();
        let start = start.clone();
        writers.push(tokio::spawn(async move {
            let key = format!("example.value_{index}");
            let value = json!(index);
            start.wait().await;
            if index % 2 == 0 {
                let store = FileOverridesStore::new(path);
                persist_one(&store, &key, &value).await.unwrap();
            } else {
                write_override(&path, &key, &value);
            }
        }));
    }
    for writer in writers {
        writer.await.unwrap();
    }

    let stored = FileOverridesStore::new(path).load().await.unwrap();
    assert_eq!(stored.len(), UPDATES as usize);
    for index in 0..UPDATES {
        assert_eq!(stored[&format!("example.value_{index}")], json!(index));
    }
}

#[test]
fn test_write_override_preserves_unparseable_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("overrides.yaml");
    let existing = "toggle.demo.held: false\nunfinished: [\n";
    std::fs::write(&path, existing).unwrap();
    write_override(&path, "executor.max_call_depth", &json!(10));
    assert_eq!(std::fs::read_to_string(path).unwrap(), existing);
}

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
