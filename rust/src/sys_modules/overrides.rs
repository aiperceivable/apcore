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
use std::sync::{Arc, OnceLock, Weak};

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
/// supplied map. Use [`Self::update`] to persist one key while retaining
/// other entries.
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

    /// Set an override key. The default serializes updates on this instance
    /// and uses `load` and `save`. Implementations that share storage across
    /// instances can coordinate at the backing store instead.
    async fn update(&self, key: &str, value: &serde_json::Value) -> Result<(), OverridesError> {
        let lock = store_update_lock(self);
        let _guard = lock.lock().await;
        let mut current = self.load().await?;
        current.insert(key.to_string(), value.clone());
        self.save(&current).await
    }
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

    async fn update(&self, key: &str, value: &serde_json::Value) -> Result<(), OverridesError> {
        self.inner.write().insert(key.to_string(), value.clone());
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
        read_yaml_overrides_sync(&self.path)
    }

    async fn save(
        &self,
        overrides: &HashMap<String, serde_json::Value>,
    ) -> Result<(), OverridesError> {
        let lock = write_lock_for(&self.path)?;
        let _guard = lock.lock();
        write_yaml_overrides_sync(&self.path, overrides)
    }

    async fn update(&self, key: &str, value: &serde_json::Value) -> Result<(), OverridesError> {
        write_override_sync(&self.path, key, value)
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
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashMap::new());
        }
        Err(source) => {
            return Err(OverridesError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
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
        let key = k
            .as_str()
            .ok_or_else(|| OverridesError::Serialize("non-string YAML mapping key".to_string()))?;
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
/// Used by control modules to delegate a single-key update to the backing store.
pub async fn persist_one(
    store: &dyn OverridesStore,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), OverridesError> {
    store.update(key, value).await
}

fn store_update_lock(store: &(impl OverridesStore + ?Sized)) -> Arc<tokio::sync::Mutex<()>> {
    type StoreLocks = Mutex<HashMap<usize, Weak<tokio::sync::Mutex<()>>>>;
    static LOCKS: OnceLock<StoreLocks> = OnceLock::new();

    let store_address = std::ptr::from_ref(store).cast::<()>() as usize;
    let mut locks = LOCKS.get_or_init(Mutex::default).lock();
    locks.retain(|_, lock| lock.strong_count() != 0);
    if let Some(lock) = locks.get(&store_address).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(store_address, Arc::downgrade(&lock));
    lock
}

// ---------------------------------------------------------------------------
// write_override — legacy free function used by control modules
// ---------------------------------------------------------------------------

/// Per-path lock registry — prevents concurrent writers from corrupting the
/// overrides file when multiple modules persist simultaneously.
fn write_lock_for(path: &Path) -> Result<Arc<Mutex<()>>, OverridesError> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

    // The physical destination directory gives relative paths and directory
    // aliases the same lock, including the first write to a new directory.
    let dir = override_directory(path);
    let canonical_dir = std::fs::create_dir_all(dir)
        .and_then(|()| dir.canonicalize())
        .map_err(|source| OverridesError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    let key = path
        .file_name()
        .map_or_else(|| path.to_path_buf(), |name| canonical_dir.join(name));
    let registry = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = registry.lock();
    Ok(Arc::clone(
        guard.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))),
    ))
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
    if let Err(error) = write_override_sync(overrides_path, key, value) {
        tracing::warn!(%error, key, "Failed to persist override");
    }
}

fn write_override_sync(
    path: &Path,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), OverridesError> {
    let lock = write_lock_for(path)?;
    let _guard = lock.lock();
    let mut existing = read_yaml_overrides_sync(path)?;
    existing.insert(key.to_string(), value.clone());
    write_yaml_overrides_sync(path, &existing)
}

fn override_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn write_yaml_overrides_sync(
    path: &Path,
    overrides: &HashMap<String, serde_json::Value>,
) -> Result<(), OverridesError> {
    let dir = override_directory(path);
    let sorted: BTreeMap<_, _> = overrides.iter().collect();
    let yaml = serde_yaml_ng::to_string(&sorted).map_err(|source| OverridesError::Yaml {
        path: path.to_path_buf(),
        source,
    })?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("overrides");
    let pid = std::process::id();
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let tmp = dir.join(format!(".{filename}.{pid}.{nanos}.tmp"));

    if let Err(source) = std::fs::write(&tmp, yaml).and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(OverridesError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
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
