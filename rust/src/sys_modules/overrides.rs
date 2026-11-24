// APCore Protocol — Runtime config & toggle override persistence
// Spec reference: system-modules.md §1.1 Config and Feature Toggle Persistence (Issue #45)
//
// Cross-language alignment (sync finding CRITICAL #1): the overrides layer is
// a pluggable [`OverridesStore`] trait so callers can swap in custom backends
// (e.g. Redis, S3, an in-memory test fake). Two reference implementations are
// shipped: [`InMemoryOverridesStore`] (volatile, thread-safe via `RwLock`) and
// [`FileOverridesStore`] (YAML on disk, atomic temp-file rename).
//
// The legacy free function [`load_overrides`] is preserved as a thin wrapper
// around [`FileOverridesStore::load`] so existing callers compile unchanged.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use thiserror::Error;

use crate::config::Config;
use crate::sys_modules::ToggleState;

// ---------------------------------------------------------------------------
// OverridesError
// ---------------------------------------------------------------------------

/// Errors raised by [`OverridesStore`] implementations.
#[derive(Debug, Error)]
pub enum OverridesError {
    #[error("overrides I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("overrides YAML at {path} is invalid: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml_ng::Error,
    },
    #[error("overrides root at {path} is not a YAML mapping")]
    NotMapping { path: PathBuf },
    #[error("overrides value is not JSON-serializable: {0}")]
    Serialize(String),
}

// ---------------------------------------------------------------------------
// OverridesStore trait
// ---------------------------------------------------------------------------

/// Pluggable backend for runtime config / feature-toggle overrides.
///
/// `load` returns the current persisted overrides as a `HashMap` keyed by
/// dot-path config key. `save` overwrites the entire backing store with the
/// supplied map (callers are expected to merge first, then save).
///
/// Cross-language: matches the `OverridesStore` interface in apcore-python and
/// apcore-typescript.
#[async_trait]
pub trait OverridesStore: Send + Sync {
    async fn load(&self) -> Result<HashMap<String, serde_json::Value>, OverridesError>;
    async fn save(
        &self,
        overrides: &HashMap<String, serde_json::Value>,
    ) -> Result<(), OverridesError>;
}

// ---------------------------------------------------------------------------
// InMemoryOverridesStore
// ---------------------------------------------------------------------------

/// Volatile, thread-safe overrides store. Useful in tests and ephemeral
/// process modes where persistence is not desired.
pub struct InMemoryOverridesStore {
    inner: RwLock<HashMap<String, serde_json::Value>>,
}

impl InMemoryOverridesStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryOverridesStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OverridesStore for InMemoryOverridesStore {
    async fn load(&self) -> Result<HashMap<String, serde_json::Value>, OverridesError> {
        Ok(self.inner.read().clone())
    }

    async fn save(
        &self,
        overrides: &HashMap<String, serde_json::Value>,
    ) -> Result<(), OverridesError> {
        let mut guard = self.inner.write();
        guard.clone_from(overrides);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FileOverridesStore
// ---------------------------------------------------------------------------

/// YAML-backed overrides store. Reads/writes a single file via atomic
/// temp-file rename and a per-path mutex to guard against concurrent writers.
pub struct FileOverridesStore {
    path: PathBuf,
}

impl FileOverridesStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait]
impl OverridesStore for FileOverridesStore {
    async fn load(&self) -> Result<HashMap<String, serde_json::Value>, OverridesError> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        read_yaml_overrides_sync(&self.path)
    }

    async fn save(
        &self,
        overrides: &HashMap<String, serde_json::Value>,
    ) -> Result<(), OverridesError> {
        // Sort keys for deterministic on-disk layout (cross-language parity).
        let sorted: BTreeMap<String, serde_json::Value> = overrides
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let lock = write_lock_for(&self.path);
        let _g = lock.lock();

        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| OverridesError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
        }

        let yaml = serde_yaml_ng::to_string(&sorted).map_err(|e| OverridesError::Yaml {
            path: self.path.clone(),
            source: e,
        })?;

        let dir = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf);
        let pid = std::process::id();
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let tmp = dir.join(format!(
            ".{}.{pid}.{nanos}.tmp",
            self.path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("overrides")
        ));

        std::fs::write(&tmp, &yaml).map_err(|e| OverridesError::Io {
            path: tmp.clone(),
            source: e,
        })?;
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(OverridesError::Io {
                path: self.path.clone(),
                source: e,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Legacy free function: load_overrides (thin wrapper)
// ---------------------------------------------------------------------------

/// Load YAML overrides into `config` and `toggle_state`.
///
/// Each top-level YAML key is treated as a dot-path config key. Keys prefixed
/// with `toggle.` are routed to `toggle_state` (boolean payload required).
/// Missing files are silently ignored — overrides are an opt-in layer applied
/// after the base config so a manual restore of the base never erases them.
///
/// This wrapper keeps the existing synchronous signature (callable from sync
/// startup code outside a Tokio runtime) while delegating the read to the
/// same YAML parser used by [`FileOverridesStore::load`]. New code should
/// construct a [`FileOverridesStore`] (or any [`OverridesStore`]
/// implementation) directly.
pub fn load_overrides(
    overrides_path: &Path,
    config: &mut Config,
    toggle_state: Option<&ToggleState>,
) {
    if !overrides_path.exists() {
        return;
    }
    let loaded = match read_yaml_overrides_sync(overrides_path) {
        Ok(map) => map,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %overrides_path.display(),
                "Failed to load overrides; skipping"
            );
            return;
        }
    };
    apply_overrides(loaded, config, toggle_state, overrides_path);
}

/// Synchronous YAML reader shared by the legacy [`load_overrides`] entry
/// point. Mirrors the parsing logic in [`FileOverridesStore::load`] without
/// requiring a Tokio runtime.
fn read_yaml_overrides_sync(
    path: &Path,
) -> Result<HashMap<String, serde_json::Value>, OverridesError> {
    let raw = std::fs::read_to_string(path).map_err(|e| OverridesError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    if raw.is_empty() {
        return Ok(HashMap::new());
    }
    let parsed: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&raw).map_err(|e| OverridesError::Yaml {
            path: path.to_path_buf(),
            source: e,
        })?;
    let map = match parsed {
        serde_yaml_ng::Value::Mapping(map) => map,
        serde_yaml_ng::Value::Null => return Ok(HashMap::new()),
        _ => {
            return Err(OverridesError::NotMapping {
                path: path.to_path_buf(),
            })
        }
    };
    let mut out: HashMap<String, serde_json::Value> = HashMap::new();
    for (k, v) in map {
        let Some(key) = k.as_str() else { continue };
        let json_value = yaml_to_json(&v).map_err(OverridesError::Serialize)?;
        out.insert(key.to_string(), json_value);
    }
    Ok(out)
}

fn apply_overrides(
    loaded: HashMap<String, serde_json::Value>,
    config: &mut Config,
    toggle_state: Option<&ToggleState>,
    overrides_path: &Path,
) {
    let mut config_count = 0usize;
    let mut toggle_count = 0usize;
    for (key, json_value) in loaded {
        if let Some(module_id) = key.strip_prefix("toggle.") {
            if let (Some(ts), Some(enabled)) = (toggle_state, json_value.as_bool()) {
                if enabled {
                    ts.enable(module_id);
                } else {
                    ts.disable(module_id);
                }
                toggle_count += 1;
            }
        } else if !key.starts_with('_') {
            config.set(&key, json_value);
            config_count += 1;
        }
    }
    tracing::info!(
        config_count,
        toggle_count,
        path = %overrides_path.display(),
        "Loaded overrides"
    );
}

// ---------------------------------------------------------------------------
// persist_one — read-modify-write helper for a single override key via store
// ---------------------------------------------------------------------------

/// Persist a single key/value into an [`OverridesStore`].
///
/// Performs a read-modify-write cycle (load existing entries, insert/update
/// the supplied key, save). Used by the control modules so callers can swap
/// the backing store without rewriting the persistence flow.
pub async fn persist_one(
    store: &dyn OverridesStore,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), OverridesError> {
    let mut current = store.load().await.unwrap_or_default();
    current.insert(key.to_string(), value.clone());
    store.save(&current).await
}

// ---------------------------------------------------------------------------
// write_override — legacy free function used by control modules
// ---------------------------------------------------------------------------

/// Per-path lock registry — prevents concurrent writers from corrupting the
/// overrides file when multiple modules persist simultaneously.
fn write_lock_for(path: &Path) -> Arc<Mutex<()>> {
    use std::collections::HashMap as StdHashMap;
    use std::sync::OnceLock;

    static LOCKS: OnceLock<Mutex<StdHashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let registry = LOCKS.get_or_init(|| Mutex::new(StdHashMap::new()));
    let mut guard = registry.lock();
    Arc::clone(
        guard
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

/// Persist a single key/value into the overrides YAML file.
///
/// The write is serialized per path and uses a tempfile + rename so a crashed
/// or interrupted writer cannot leave a half-written file behind.
///
/// This is a convenience wrapper around the read-modify-write loop used by
/// the control modules. Errors are logged at WARN level — overrides
/// persistence is best-effort and must not abort the calling control module.
pub fn write_override(overrides_path: &Path, key: &str, value: &serde_json::Value) {
    let lock = write_lock_for(overrides_path);
    let _g = lock.lock();

    if let Some(parent) = overrides_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!(
                    error = %e,
                    path = %overrides_path.display(),
                    "Failed to create overrides parent directory"
                );
                return;
            }
        }
    }

    // Read existing overrides as an ordered mapping so the on-disk layout
    // remains stable across writes (BTreeMap → alphabetical key order).
    let mut existing: BTreeMap<String, serde_json::Value> =
        match std::fs::read_to_string(overrides_path) {
            Ok(s) if !s.is_empty() => match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&s) {
                Ok(serde_yaml_ng::Value::Mapping(map)) => map
                    .into_iter()
                    .filter_map(|(k, v)| {
                        let k = k.as_str()?.to_string();
                        let v = yaml_to_json(&v).ok()?;
                        Some((k, v))
                    })
                    .collect(),
                _ => BTreeMap::new(),
            },
            _ => BTreeMap::new(),
        };

    existing.insert(key.to_string(), value.clone());

    let yaml = match serde_yaml_ng::to_string(&existing) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, key = %key, "Failed to serialize overrides YAML");
            return;
        }
    };

    let dir = overrides_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf);
    let pid = std::process::id();
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let tmp = dir.join(format!(
        ".{}.{pid}.{nanos}.tmp",
        overrides_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("overrides")
    ));

    if let Err(e) = std::fs::write(&tmp, &yaml) {
        tracing::error!(error = %e, path = %tmp.display(), "Failed to write overrides tempfile");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, overrides_path) {
        tracing::error!(
            error = %e,
            path = %overrides_path.display(),
            "Failed to rename overrides tempfile into place"
        );
        // best-effort cleanup
        let _ = std::fs::remove_file(&tmp);
    }
}

// ---------------------------------------------------------------------------
// YAML <-> JSON conversion
// ---------------------------------------------------------------------------

/// Convert a `serde_yaml_ng::Value` to its `serde_json::Value` equivalent.
///
/// YAML supports a few non-JSON shapes (mappings with non-string keys, raw
/// tags) that we reject up-front rather than silently coerce.
fn yaml_to_json(v: &serde_yaml_ng::Value) -> Result<serde_json::Value, String> {
    match v {
        serde_yaml_ng::Value::Null => Ok(serde_json::Value::Null),
        serde_yaml_ng::Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        serde_yaml_ng::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(serde_json::Value::Number(i.into()))
            } else if let Some(u) = n.as_u64() {
                Ok(serde_json::Value::Number(u.into()))
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| "non-finite f64".to_string())
            } else {
                Err("unrepresentable YAML number".to_string())
            }
        }
        serde_yaml_ng::Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        serde_yaml_ng::Value::Sequence(seq) => seq
            .iter()
            .map(yaml_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_yaml_ng::Value::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                let key = k
                    .as_str()
                    .ok_or_else(|| "non-string YAML mapping key".to_string())?
                    .to_string();
                obj.insert(key, yaml_to_json(v)?);
            }
            Ok(serde_json::Value::Object(obj))
        }
        serde_yaml_ng::Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}
