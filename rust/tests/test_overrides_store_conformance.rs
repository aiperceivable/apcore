//! Drive `overrides_store.json` — pluggable persistence for
//! `system.control.update_config` / `system.control.toggle_feature`
//! (Issue #45.1, D-40).
//!
//! `tests/test_system_modules_hardening_conformance.rs` covers this ground
//! against a different fixture; the canonical `overrides_store.json` was never
//! loaded by this SDK, so a case added to it could not reach Rust.
//!
//! API mapping. The fixture speaks in per-key operations (`save {key,value}`,
//! `get {key}`, `delete {key}`, `get_all`); every SDK implements the same
//! behaviour over a whole-map surface — Rust `OverridesStore::load()/save(map)`,
//! exactly as apcore-python does. The per-key ops are therefore executed as
//! read-modify-write against that surface. The behavioural claims the fixture
//! makes (durability across reopen, instance isolation, missing-file
//! tolerance, idempotent delete) are asserted against the real API, not
//! simulated.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use apcore::sys_modules::overrides::{
    load_overrides, FileOverridesStore, InMemoryOverridesStore, OverridesStore,
};
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("overrides_store.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("overrides_store.json parses")
}

type Overrides = HashMap<String, Value>;

async fn put(store: &dyn OverridesStore, key: &str, value: Value) -> Result<(), String> {
    let mut map: Overrides = store.load().await.map_err(|e| e.to_string())?;
    map.insert(key.to_string(), value);
    store.save(&map).await.map_err(|e| e.to_string())
}

async fn remove(store: &dyn OverridesStore, key: &str) -> Result<(), String> {
    let mut map: Overrides = store.load().await.map_err(|e| e.to_string())?;
    map.remove(key);
    store.save(&map).await.map_err(|e| e.to_string())
}

/// Everything the fixture can assert about a replayed operation sequence.
#[derive(Default)]
struct Replay {
    raised_error: bool,
    /// `(instance_index, value)` per `get`, in order.
    gets: Vec<(usize, Option<Value>)>,
    /// Each `get_all` snapshot, in order.
    get_alls: Vec<Overrides>,
    /// Files present under the sandbox directory once the sequence finishes.
    files_on_disk: usize,
    /// Whether the file-backed store's path exists at the end.
    path_exists: bool,
}

fn count_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_file())
                .count()
        })
        .unwrap_or(0)
}

async fn replay(store_type: &str, ops: &[Value], dir: &Path) -> Replay {
    let path = dir.join("overrides.yaml");
    let mut instance = 0usize;
    let mut last_key: Option<String> = None;
    let mut store: Arc<dyn OverridesStore> = match store_type {
        "FileOverridesStore" => Arc::new(FileOverridesStore::new(path.clone())),
        "InMemoryOverridesStore" => Arc::new(InMemoryOverridesStore::new()),
        other => panic!("overrides_store.json names store type `{other}` this driver cannot build"),
    };
    let mut out = Replay::default();

    for op in ops {
        match op["op"].as_str().expect("every operation needs an `op`") {
            // Constructing is already done above; the case asserts it does not
            // raise, which a panic in the match arm above would have surfaced.
            "construct" => {}
            // D-47: the shipped surface is `load()` / `save(mapping)`, so a
            // single-key change is a read-modify-write over the whole map.
            // `put` / `remove` are that adapter; the fixture used to speak a
            // per-key surface (save/get/delete/get_all) no SDK implements.
            "load_modify_save" => {
                if let Some(sets) = op.get("set").and_then(Value::as_object) {
                    for (key, value) in sets {
                        if put(store.as_ref(), key, value.clone()).await.is_err() {
                            out.raised_error = true;
                        }
                        last_key = Some(key.clone());
                    }
                }
                if let Some(removals) = op.get("remove").and_then(Value::as_array) {
                    for key in removals {
                        let key = key.as_str().expect("remove entries are strings");
                        if remove(store.as_ref(), key).await.is_err() {
                            out.raised_error = true;
                        }
                        last_key = Some(key.to_string());
                    }
                }
            }
            // `load` yields the whole map. `gets` projects it onto the most
            // recently written key so the fixture's single-value expectations
            // (value_after_reopen, first_load_value, ...) stay readable.
            "load" => match store.load().await {
                Ok(map) => {
                    let projected = last_key.as_ref().and_then(|k| map.get(k).cloned());
                    out.gets.push((instance, projected));
                    out.get_alls.push(map);
                }
                Err(_) => {
                    out.raised_error = true;
                    out.gets.push((instance, None));
                    out.get_alls.push(HashMap::new());
                }
            },
            // A fresh handle over the SAME path: only durable state survives.
            "reopen_store" => {
                instance += 1;
                store = Arc::new(FileOverridesStore::new(path.clone()));
            }
            // A fresh instance with NO shared backing: nothing may survive.
            "new_store_instance" => {
                instance += 1;
                store = Arc::new(InMemoryOverridesStore::new());
            }
            other => panic!(
                "overrides_store.json grew operation `{other}` that this driver \
                 cannot execute — teach the driver, do not skip the case"
            ),
        }
    }

    out.files_on_disk = count_files(dir);
    out.path_exists = path.exists();
    out
}

/// Cases whose `input` is an operation sequence.
async fn run_operation_case(tc: &Value, id: &str) {
    let store_type = tc["input"]["store_type"]
        .as_str()
        .unwrap_or_else(|| panic!("[{id}] case has no input.store_type"));
    let ops = tc["input"]["operations"]
        .as_array()
        .unwrap_or_else(|| panic!("[{id}] case has no input.operations"));

    let dir = tempfile::tempdir().expect("tempdir");
    let got = replay(store_type, ops, dir.path()).await;

    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

    for (field, want) in expected {
        match field.as_str() {
            "value_after_reopen" | "second_instance_load_value" => {
                let (_, actual) = got
                    .gets
                    .iter()
                    .rev()
                    .find(|(inst, _)| *inst > 0)
                    .unwrap_or_else(|| {
                        panic!("[{id}] no get() ran after the store was re-created")
                    });
                let actual = actual.clone().unwrap_or(Value::Null);
                assert_eq!(&actual, want, "[{id}] {field}");
            }
            "first_load_value" => {
                let (_, actual) = got
                    .gets
                    .first()
                    .unwrap_or_else(|| panic!("[{id}] expects first_get_value but no get() ran"));
                let actual = actual.clone().unwrap_or(Value::Null);
                assert_eq!(&actual, want, "[{id}] first_get_value");
            }
            "get_all_before_save" => {
                let snapshot = got
                    .get_alls
                    .first()
                    .unwrap_or_else(|| panic!("[{id}] expects get_all() but none ran"));
                let want_map = want.as_object().expect("get_all_before_save is an object");
                assert_eq!(
                    snapshot.len(),
                    want_map.len(),
                    "[{id}] get_all_before_save: got {snapshot:?}"
                );
                for (k, v) in want_map {
                    assert_eq!(snapshot.get(k), Some(v), "[{id}] get_all_before_save[{k}]");
                }
            }
            "get_all_keys" => {
                let snapshot = got
                    .get_alls
                    .last()
                    .unwrap_or_else(|| panic!("[{id}] expects get_all() but none ran"));
                let mut actual: Vec<&String> = snapshot.keys().collect();
                actual.sort();
                let want_keys: Vec<String> = want
                    .as_array()
                    .expect("get_all_keys is an array")
                    .iter()
                    .map(|v| v.as_str().expect("key is a string").to_string())
                    .collect();
                let actual_owned: Vec<String> = actual.into_iter().cloned().collect();
                assert_eq!(actual_owned, want_keys, "[{id}] get_all_keys");
            }
            "raised_error" | "construction_raised_error" => {
                assert_eq!(
                    got.raised_error,
                    want.as_bool().expect("bool expectation"),
                    "[{id}] {field}"
                );
            }
            "path_exists_after_save" => {
                assert_eq!(
                    got.path_exists,
                    want.as_bool().expect("path_exists_after_save is a bool"),
                    "[{id}] first save() must create the backing file"
                );
            }
            "disk_writes" => {
                let want_writes = want.as_u64().expect("disk_writes is a number") as usize;
                assert_eq!(
                    got.files_on_disk, want_writes,
                    "[{id}] InMemoryOverridesStore must not touch the filesystem"
                );
            }
            other => panic!(
                "[{id}] overrides_store.json grew expectation `{other}` that this \
                 driver does not check — teach the driver, do not skip it"
            ),
        }
    }
}

/// Case `startup_loads_overrides_after_base_config`: a real base YAML on disk
/// plus a real overrides YAML, applied in the documented order.
fn run_startup_precedence_case(tc: &Value, id: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let base_path = dir.path().join("apcore.yaml");
    let overrides_path = dir.path().join("overrides.yaml");

    // Base config: dot-path keys become a nested YAML document, the shape a
    // real `apcore.yaml` has. `version` / `project.name` are the two fields
    // `Config::validate` requires of any legacy-mode document (§9.1); the
    // fixture says nothing about them, so they are scaffolding, not expectation.
    let mut base_doc = serde_json::Map::new();
    base_doc.insert("version".to_string(), Value::String("1.0".to_string()));
    base_doc.insert(
        "project".to_string(),
        serde_json::json!({"name": "overrides-store-conformance"}),
    );
    for (dotted, value) in tc["input"]["base_config"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no input.base_config"))
    {
        let (ns, leaf) = dotted
            .split_once('.')
            .unwrap_or_else(|| panic!("[{id}] base_config key `{dotted}` is not dotted"));
        base_doc
            .entry(ns.to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .expect("namespace object")
            .insert(leaf.to_string(), value.clone());
    }
    std::fs::write(
        &base_path,
        serde_yaml_ng::to_string(&Value::Object(base_doc)).expect("base yaml"),
    )
    .expect("write base config");

    // Overrides file: flat dot-path keys, the shape `load_overrides` documents.
    let overrides_doc: serde_json::Map<String, Value> = tc["input"]["overrides_file"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no input.overrides_file"))
        .clone();
    std::fs::write(
        &overrides_path,
        serde_yaml_ng::to_string(&Value::Object(overrides_doc)).expect("overrides yaml"),
    )
    .expect("write overrides");

    let base_bytes_before = std::fs::read(&base_path).expect("read base");

    let mut config = apcore::config::Config::load(&base_path).expect("base config loads");
    load_overrides(&overrides_path, &mut config, None);

    let expected = tc["expected"]
        .as_object()
        .unwrap_or_else(|| panic!("[{id}] case has no expected object"));
    for (field, want) in expected {
        match field.as_str() {
            "effective_config" => {
                for (key, want_value) in want.as_object().expect("effective_config is an object") {
                    let got = config
                        .get(key)
                        .unwrap_or_else(|| panic!("[{id}] config key `{key}` resolved to None"));
                    let equal = if got.is_number() && want_value.is_number() {
                        got.as_f64() == want_value.as_f64()
                    } else {
                        &got == want_value
                    };
                    assert!(
                        equal,
                        "[{id}] effective_config[{key}]: sdk={got} canonical={want_value}"
                    );
                }
            }
            "base_file_modified" => {
                let modified =
                    std::fs::read(&base_path).expect("re-read base") != base_bytes_before;
                assert_eq!(
                    modified,
                    want.as_bool().expect("base_file_modified is a bool"),
                    "[{id}] applying overrides must not rewrite the base config file"
                );
            }
            other => panic!(
                "[{id}] overrides_store.json grew expectation `{other}` that this \
                 driver does not check — teach the driver, do not skip it"
            ),
        }
    }
}

#[tokio::test]
async fn conformance_overrides_store() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert_eq!(cases.len(), 5, "driver is written against all 5 cases");

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        match id {
            "save_persists_override"
            | "inmemory_store_for_tests"
            | "missing_path_first_run_ok"
            | "delete_removes_override" => run_operation_case(tc, id).await,
            "startup_loads_overrides_after_base_config" => run_startup_precedence_case(tc, id),
            other => panic!(
                "overrides_store.json grew case `{other}` that this driver does not \
                 run — teach the driver, do not skip it"
            ),
        }
    }
}
