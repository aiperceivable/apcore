// APCore Protocol — Registry, Discoverer, ModuleValidator
// Spec reference: Module registration, discovery, validation, and descriptors

use async_trait::async_trait;
use parking_lot::{Mutex as ParkingLotMutex, RwLock};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};

use crate::errors::ModuleError;
use crate::events::emitter::{ApCoreEvent, EventEmitter};
use crate::module::{
    Module, ModuleAnnotations, ModuleExample, ValidationErrorDetail, ValidationResult,
};
use crate::registry::conflicts::{detect_id_conflicts, ConflictSeverity, ConflictType};
use crate::registry::metadata::parse_dependencies;
use crate::registry::types::DepInfo;

/// Join the `message` field of each validation error detail into a single
/// comma-separated string for human-readable error reporting.
fn join_validation_messages(errors: &[ValidationErrorDetail]) -> String {
    errors
        .iter()
        .map(|e| e.message.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Cross-language compatible module descriptor.
///
/// Aligned with `apcore-python.ModuleDescriptor` and
/// `apcore-typescript.ModuleDescriptor`.  All fields match `PROTOCOL_SPEC`
/// section 5.2.  The `enabled` field is a Rust-specific runtime addition
/// used by `Registry::disable()` / `Registry::enable()` for module toggling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    /// Canonical module identifier (e.g. "math.add").
    pub module_id: String,
    /// Human-readable display name (optional).
    #[serde(default)]
    pub name: Option<String>,
    /// One-line description of what the module does.
    #[serde(default)]
    pub description: String,
    /// Long-form documentation (Markdown).
    #[serde(default)]
    pub documentation: Option<String>,
    /// JSON Schema for the module's input.
    pub input_schema: serde_json::Value,
    /// JSON Schema for the module's output.
    pub output_schema: serde_json::Value,
    /// Semantic version string.
    #[serde(default = "default_version")]
    pub version: String,
    /// Categorisation tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Behavioural annotations (readonly, destructive, etc.).
    #[serde(default)]
    pub annotations: Option<ModuleAnnotations>,
    /// Example invocations.
    #[serde(default)]
    pub examples: Vec<ModuleExample>,
    /// Arbitrary metadata for display overlays, AI intent hints, and version hints.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// UI display metadata (optional). Mirrors `display` in Python/TypeScript SDKs.
    #[serde(default)]
    pub display: Option<serde_json::Value>,
    /// ISO 8601 date string (YYYY-MM-DD) after which this module is removed.
    #[serde(default)]
    pub sunset_date: Option<String>,
    /// Module dependencies.
    #[serde(default)]
    pub dependencies: Vec<DependencyInfo>,
    /// Runtime-only: whether the module is enabled (not in `PROTOCOL_SPEC`).
    #[serde(default = "default_enabled", skip_serializing)]
    pub enabled: bool,
}

fn default_version() -> String {
    DEFAULT_MODULE_VERSION.to_string()
}

fn default_enabled() -> bool {
    true
}

/// Dependency information for a module.
///
/// **Wire name (SYS-12):** the constraint is serialized as `version`, which is
/// how the metadata YAML input spells it ([`DepInfo::version`]) and how
/// apcore-python and apcore-typescript emit it from `system.manifest.module` /
/// `system.manifest.full`. The Rust field keeps the more explicit
/// `version_constraint` name; only the wire form is renamed, so a manifest
/// consumer reading `dependencies[].version` gets an answer on all three SDKs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyInfo {
    pub module_id: String,
    #[serde(rename = "version")]
    pub version_constraint: String,
    #[serde(default)]
    pub optional: bool,
}

impl From<DepInfo> for DependencyInfo {
    /// Lift a parsed metadata dependency into the descriptor's shape.
    ///
    /// The two types differ only in how "no version constraint" is spelled:
    /// [`DepInfo::version`] is `Option<String>` (the YAML/metadata form, where
    /// the key may simply be absent) while [`DependencyInfo::version_constraint`]
    /// is a plain `String`. The empty string is the descriptor's "unconstrained"
    /// value — `ReloadModule::topo_sort_modules` maps it straight back to
    /// `None`, so the round trip is lossless.
    fn from(dep: DepInfo) -> Self {
        Self {
            module_id: dep.module_id,
            version_constraint: dep.version.unwrap_or_default(),
            optional: dep.optional,
        }
    }
}

/// Read `metadata["dependencies"]` — the cross-language metadata shape, a list
/// of `{module_id, version?, optional?}` objects — into descriptor dependencies.
///
/// This is the canonical four-argument `register`'s only source of declared
/// dependencies: the [`Module`] trait exposes no `dependencies()` method, so
/// unlike `description()` / `tags()` / `annotations()` there is nothing to fall
/// back to in code. apcore-python (`merge_module_metadata`, code fallback via
/// `getattr(module, "dependencies", [])`) and apcore-typescript
/// (`mergeModuleMetadata`) accept exactly this shape through their own
/// `register(module_id, module, version?, metadata?)`.
///
/// A `dependencies` value that is not an array means "none declared" rather
/// than an error: nothing validates this key (no schema describes it), so a
/// scalar can arrive here from hand-written YAML or a hand-built metadata map.
pub(crate) fn dependencies_from_metadata(
    metadata: &HashMap<String, serde_json::Value>,
) -> Vec<DependencyInfo> {
    metadata
        .get("dependencies")
        .and_then(|v| v.as_array())
        .map(|arr| {
            parse_dependencies(arr)
                .into_iter()
                .map(DependencyInfo::from)
                .collect()
        })
        .unwrap_or_default()
}

/// A module found via discovery.
///
/// Every entry carries a live module instance. Discoverers for out-of-process
/// modules (subprocesses, RPC endpoints, network-hosted modules) wrap the
/// external resource in a [`Module`] impl — e.g., a
/// `SubprocessModule { executable: PathBuf, descriptor }` whose `execute`
/// spawns the process — so the registry can treat all modules uniformly.
///
/// Aligned with `apcore-python.Discoverer` (returns `{module_id, module}`)
/// and `apcore-typescript.Discoverer` (returns `{moduleId, module}`).
///
/// Not serializable: the `module` field is `Arc<dyn Module>` (a trait object)
/// and has no meaningful serde representation. This is an intentional exception
/// to the project-wide "all public data types implement Serialize/Deserialize"
/// convention in `CLAUDE.md` — do not add the derives back without also solving
/// how to round-trip a live module instance.
#[derive(Clone)]
pub struct DiscoveredModule {
    pub name: String,
    pub source: String,
    pub descriptor: ModuleDescriptor,
    pub module: Arc<dyn Module>,
}

impl std::fmt::Debug for DiscoveredModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveredModule")
            .field("name", &self.name)
            .field("source", &self.source)
            .field("descriptor", &self.descriptor)
            .field("module", &"<Module>")
            .finish()
    }
}

/// Trait for discovering modules from external sources.
#[async_trait]
pub trait Discoverer: Send + Sync {
    /// Discover available modules.
    ///
    /// `roots` is a list of filesystem paths (or logical namespaces) to search.
    /// Implementations that perform filesystem discovery SHOULD restrict their
    /// search to the provided roots. Passing an empty slice means "use the
    /// implementation's default search paths."
    ///
    /// Each returned [`DiscoveredModule`] carries a live `module` instance —
    /// discoverers for out-of-process modules wrap the external resource in a
    /// `Module` impl so the registry can treat every module uniformly.
    ///
    /// Aligned with `apcore-python.Discoverer.discover(roots)` and
    /// `apcore-typescript.Discoverer.discover(roots)`.
    async fn discover(&self, roots: &[String]) -> Result<Vec<DiscoveredModule>, ModuleError>;
}

/// Trait for validating module implementations.
pub trait ModuleValidator: Send + Sync {
    /// Validate a module against the protocol contract.
    ///
    /// `descriptor` is optional — pass `Some(&descriptor)` when a full
    /// `ModuleDescriptor` is available, or `None` for schema-free validation.
    /// Returning a non-empty `ValidationResult.errors` causes `register()` to
    /// reject the module.
    ///
    /// Aligned with `apcore-python.ModuleValidator.validate(module)` and
    /// `apcore-typescript.ModuleValidator.validate(module)`.
    fn validate(
        &self,
        module: &dyn Module,
        descriptor: Option<&ModuleDescriptor>,
    ) -> ValidationResult;
}

/// Type alias for the event callback closure.
pub type ModuleCallbackFn = dyn Fn(&str, &dyn Module) + Send + Sync;
type CallbackMap = HashMap<String, Vec<(u64, Arc<ModuleCallbackFn>)>>;

/// Type alias for the on_load_failed callback closure (Issue #65).
type LoadFailedCallbackFn = dyn Fn(&str, &ModuleError) + Send + Sync;

/// Reserved words that cannot be used as the first segment of a module ID.
///
/// Aligned with the Python and TypeScript SDKs to ensure cross-language consistency.
pub const RESERVED_WORDS: &[&str] = &[
    "system", "internal", "core", "apcore", "plugin", "schema", "acl",
];

/// Namespace reserved for programmatically-registered modules synthesized at
/// runtime.
///
/// Per protocol-spec §2.5.1. IDs in this namespace MUST be registered through
/// [`Registry::register`] / [`Registry::register_module`] only; the filesystem
/// discoverer rejects matching IDs because the namespace has no
/// directory-rooted source of truth, and [`Registry::register_internal`]
/// rejects them so sys/internal modules cannot squat on the prefix.
///
/// The trailing dot is required so module IDs whose first segment merely
/// *starts with* `ephemeral` (e.g. `ephemerals`) are not falsely classified.
///
/// Cross-language alignment: matches `EPHEMERAL_NAMESPACE_PREFIX` in
/// apcore-python.
pub const EPHEMERAL_NAMESPACE_PREFIX: &str = "ephemeral.";

/// Return `true` when `module_id` belongs to the reserved `ephemeral.*`
/// namespace (either the bare segment `ephemeral` or any descendant).
///
/// Aligned with apcore-python's `_is_ephemeral`.
#[must_use]
pub fn is_ephemeral_module_id(module_id: &str) -> bool {
    module_id == "ephemeral" || module_id.starts_with(EPHEMERAL_NAMESPACE_PREFIX)
}

/// Return `true` if the descriptor's annotations declare `discoverable=true`
/// (or if no annotations / no descriptor are present — default-discoverable).
///
/// Aligned with apcore-python's `Registry._is_discoverable`.
fn descriptor_is_discoverable(descriptor: Option<&ModuleDescriptor>) -> bool {
    match descriptor {
        Some(desc) => desc.annotations.as_ref().is_none_or(|ann| ann.discoverable),
        None => true,
    }
}

/// Maximum allowed length for a module ID.
///
/// Per `PROTOCOL_SPEC` §2.7 EBNF constraint #1. 192 is filesystem-safe
/// (`192 + ".binding.yaml".len() = 205 < 255`-byte filename limit on
/// ext4/xfs/NTFS/APFS/btrfs) and accommodates Java/.NET deep-namespace
/// FQN-derived IDs. Bumped from 128 in spec 1.6.0-draft (2026-04-08).
///
/// Aligned with `apcore-python` and `apcore-typescript` `MAX_MODULE_ID_LENGTH`.
pub const MAX_MODULE_ID_LENGTH: usize = 192;

/// Default module version when a caller does not supply one.
///
/// Used by [`ModuleDescriptor`]'s serde default, by
/// [`Registry::register_module`]'s auto-built descriptor, and by
/// [`crate::client::APCore::module`]. Aligned with `apcore-python`'s default.
pub const DEFAULT_MODULE_VERSION: &str = "1.0.0";

/// Standard registry event names per `PROTOCOL_SPEC` §12.2.
///
/// All SDKs **MUST** export these event names as named constants so that
/// consumers do not hardcode the underlying string literals. Aligned with
/// `apcore-python.REGISTRY_EVENTS` and `apcore-typescript.REGISTRY_EVENTS`.
///
/// Usage: `registry.on(RegistryEvents::REGISTER, callback);` — or
/// `registry.on(registry_events::REGISTER, callback);` for the free-standing
/// constants below. The dotted `REGISTRY_EVENTS.REGISTER` spelling this
/// comment used to show is Python/TypeScript syntax and does not compile in
/// Rust: `REGISTER` is an associated const on [`RegistryEvents`], reachable
/// only through the type, never through a value of it.
pub mod registry_events {
    /// Fired after a module is successfully registered.
    pub const REGISTER: &str = "register";

    /// Fired before a module is removed from the registry.
    pub const UNREGISTER: &str = "unregister";

    /// Every event name this implementation can emit, and therefore the exact
    /// set [`Registry::on`](crate::registry::Registry::on) accepts (D-80).
    ///
    /// The spec's closed set is `register`, `unregister`, and a **conditional**
    /// `file_changed`. `file_changed` exists only for implementations whose
    /// `watch()` is notify-only; apcore-rust's `watch()` re-runs discovery and
    /// already emits `unregister`/`register` for the change, so per requirement
    /// 3 it MUST NOT additionally emit `file_changed` — and, by requirement 2
    /// read the other way, must not advertise a name it never fires.
    pub const ALL: [&str; 2] = [REGISTER, UNREGISTER];
}

/// Container for the standard registry event names.
///
/// Carries the same names `apcore-python` (dict) and `apcore-typescript`
/// (frozen object) group under `REGISTRY_EVENTS`, spelled the way Rust
/// reaches associated constants: `RegistryEvents::REGISTER` /
/// `RegistryEvents::UNREGISTER`, through the type rather than through a value.
pub struct RegistryEvents;

impl RegistryEvents {
    pub const REGISTER: &'static str = registry_events::REGISTER;
    pub const UNREGISTER: &'static str = registry_events::UNREGISTER;
    /// The closed set of names `Registry::on` accepts (D-80).
    pub const ALL: [&'static str; 2] = registry_events::ALL;
}

/// Singleton value of [`RegistryEvents`], kept so the name `REGISTRY_EVENTS`
/// resolves for readers coming from the Python and TypeScript SDKs. The event
/// names themselves are read off the type — `RegistryEvents::REGISTER` — not
/// off this value; `REGISTRY_EVENTS.REGISTER` is not valid Rust.
pub const REGISTRY_EVENTS: RegistryEvents = RegistryEvents;

/// Canonical regex source for the module ID pattern (`PROTOCOL_SPEC` §2.7).
///
/// Raw string form, matching `apcore-python` / `apcore-typescript`
/// `MODULE_ID_PATTERN`. Consumers needing a compiled pattern should call
/// [`module_id_pattern`] instead.
pub const MODULE_ID_PATTERN: &str = r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$";

/// Canonical EBNF pattern for module IDs per `PROTOCOL_SPEC` §2.7.
///
/// Equivalent regex of: `canonical_id = segment ("." segment)*` where
/// `segment = [a-z][a-z0-9_]*`. Aligned with `apcore-python` and
/// `apcore-typescript` `MODULE_ID_PATTERN`.
pub fn module_id_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(MODULE_ID_PATTERN).unwrap())
}

/// Validate a module ID against `PROTOCOL_SPEC` §2.7 in canonical order:
/// 1. non-empty
/// 2. matches EBNF pattern
/// 3. length ≤ `MAX_MODULE_ID_LENGTH`
/// 4. (if `allow_reserved == false`) first segment is not a reserved word
///
/// Duplicate detection is the caller's responsibility (it requires registry
/// state). `register_internal` calls this with `allow_reserved=true` so sys
/// modules can use the `system.*` prefix; all other validations still apply.
///
/// Aligned with `apcore-python._validate_module_id` and
/// `apcore-typescript._validateModuleId`.
fn validate_module_id(name: &str, allow_reserved: bool) -> Result<(), ModuleError> {
    // 1. empty check
    if name.is_empty() {
        return Err(ModuleError::new(
            crate::errors::ErrorCode::InvalidModuleId,
            "module_id must be a non-empty string".to_string(),
        ));
    }

    // 2. EBNF pattern check
    if !module_id_pattern().is_match(name) {
        return Err(ModuleError::new(
            crate::errors::ErrorCode::InvalidModuleId,
            format!(
                "Invalid module ID: '{name}'. Must match pattern: ^[a-z][a-z0-9_]*(\\.[a-z][a-z0-9_]*)*$ (lowercase, digits, underscores, dots only; no hyphens)"
            ),
        ));
    }

    // 3. length check
    if name.len() > MAX_MODULE_ID_LENGTH {
        return Err(ModuleError::new(
            crate::errors::ErrorCode::InvalidModuleId,
            format!(
                "Module ID exceeds maximum length of {}: {}",
                MAX_MODULE_ID_LENGTH,
                name.len()
            ),
        ));
    }

    // 4. reserved word first-segment check (skipped for register_internal)
    if !allow_reserved {
        // INVARIANT: pattern check (step 2) guarantees at least one segment.
        let first_segment = name.split('.').next().unwrap();
        if RESERVED_WORDS.contains(&first_segment) {
            return Err(ModuleError::new(
                crate::errors::ErrorCode::InvalidModuleId,
                format!("Module ID contains reserved word: '{first_segment}'"),
            ));
        }
    }

    Ok(())
}

/// Internal shared state for `Registry`, protected by a single `RwLock`.
///
/// All mutating methods acquire `core.write()` for the duration of the
/// mutation and release before invoking any user callbacks.
struct RegistryCore {
    modules: HashMap<String, Arc<dyn Module>>,
    descriptors: HashMap<String, ModuleDescriptor>,
    /// Reference counts for safe hot-reload — prevents unloading while in use.
    ref_counts: HashMap<String, usize>,
    /// Modules marked for unload (draining active requests before removal).
    draining: HashSet<String>,
    /// Case-insensitive lookup: lowercase name -> canonical name.
    lowercase_map: HashMap<String, String>,
    /// Cached JSON schemas for registered modules.
    schema_cache: HashMap<String, serde_json::Value>,
    /// Every version of a module registered more than once through
    /// [`Registry::register_versioned`]. `modules` / `descriptors` hold the
    /// highest of them, which is what `get` returns (it takes no version hint,
    /// D-126).
    versions: HashMap<String, Vec<VersionEntry>>,
    /// `DiscoveredModule::source` of every module registered by discovery, so
    /// the file watcher can tell which module a changed or removed file is.
    sources: HashMap<String, String>,
}

/// One registered version of a multi-version module.
struct VersionEntry {
    version: String,
    module: Arc<dyn Module>,
    descriptor: ModuleDescriptor,
}

impl RegistryCore {
    fn new() -> Self {
        Self {
            modules: HashMap::new(),
            descriptors: HashMap::new(),
            ref_counts: HashMap::new(),
            draining: HashSet::new(),
            lowercase_map: HashMap::new(),
            schema_cache: HashMap::new(),
            versions: HashMap::new(),
            sources: HashMap::new(),
        }
    }
}

/// Central registry of modules.
///
/// The `Registry` uses interior mutability via `parking_lot::RwLock` so that
/// a single `Arc<Registry>` may be cloned freely and shared across the
/// pipeline, sys modules, and user code. All methods take `&self`; callers
/// never need `Arc::get_mut` or an external `Mutex` wrapper.
///
/// Critical invariant: no Registry lock is ever held across an `.await`.
/// All methods are synchronous, callback invocations clone the callbacks
/// out of their lock before running them, and the drain-wait logic releases
/// the core lock before awaiting.
pub struct Registry {
    core: RwLock<RegistryCore>,
    /// Event callbacks keyed by event name (e.g. "register", "unregister").
    ///
    /// Callbacks are stored as `(id, Arc)` so they can be cloned out of the lock
    /// before invocation — holding this lock while calling a callback would
    /// deadlock if the callback tried to register or unregister a module.
    callbacks: RwLock<CallbackMap>,
    /// Monotonically increasing counter for callback handle IDs.
    callback_counter: AtomicU64,
    /// D-89: `(module_id, version)` pairs whose deprecation warning has already
    /// been emitted by THIS registry instance.
    deprecation_warned: ParkingLotMutex<HashSet<(String, String, String)>>,
    /// Drain completion notification — signaled when a draining module reaches zero refs.
    drain_events: RwLock<HashMap<String, Arc<tokio::sync::Notify>>>,
    /// Optional discoverer for module discovery.
    discoverer: RwLock<Option<Arc<dyn Discoverer>>>,
    /// Optional validator for module validation.
    ///
    /// Stored as `Arc` (not `Box`) so the validator can be cloned out of the
    /// read lock before invocation. Holding a `parking_lot::RwLock` read
    /// guard across user-supplied `validate()` would deadlock if the
    /// validator re-entered `set_validator` (parking_lot's guards are not
    /// reentrant).
    validator: RwLock<Option<Arc<dyn ModuleValidator>>>,
    /// Extension roots passed to custom Discoverers.
    ///
    /// Aligned with `apcore-python.Registry._extension_roots` and
    /// `apcore-typescript.Registry._extensionRoots`. Set via
    /// [`set_extension_roots`](Self::set_extension_roots); passed verbatim to
    /// `Discoverer::discover(roots)` in `discover_internal()`.
    extension_roots: RwLock<Vec<String>>,
    /// Live filesystem watcher (sync finding A-D-010). `None` when
    /// `watch()` has not been called or has been stopped via `unwatch()`.
    watcher: parking_lot::Mutex<Option<notify::RecommendedWatcher>>,
    /// Background task handle that consumes notify events and triggers
    /// debounced re-discovery. Cleared on `unwatch()`.
    watch_handle: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Issue #65: module IDs whose on_load is currently in progress.
    /// A module in in_flight is NOT visible via get()/list().
    /// Prevents concurrent duplicate registration of the same ID.
    in_flight: ParkingLotMutex<HashSet<String>>,
    /// Issue #65: callbacks invoked when any module's on_load fails.
    load_failed_callbacks: RwLock<Vec<Arc<LoadFailedCallbackFn>>>,
    /// Optional event emitter wired by the host (A-D-002). When set, the
    /// registry emits `apcore.registry.module_load_failed` on `on_load`
    /// failure, mirroring apcore-python `_event_emitter` /
    /// apcore-typescript `_eventEmitter`.
    event_emitter: RwLock<Option<Arc<EventEmitter>>>,
}

/// RAII guard that restores a taken-out `Discoverer` back into the registry's
/// slot when dropped — including during panic unwind from `discover().await`.
///
/// Without this, a panic inside a custom `Discoverer::discover` future would
/// permanently lose the discoverer because the manual "put it back" block
/// below the `.await` would be unreachable.
///
/// If a concurrent `set_discoverer` swapped in a new instance during the
/// `.await`, that new one wins — the guard only restores when the slot is
/// still `None`.
struct DiscovererRestoreGuard<'a> {
    slot: &'a RwLock<Option<Arc<dyn Discoverer>>>,
    discoverer: Option<Arc<dyn Discoverer>>,
}

/// A watched path in the form the platform watcher reports it.
///
/// Watchers report resolved paths (`/private/var/...` on macOS for a root under
/// `/var/...`), so both sides of a comparison are canonicalised. A removed file
/// can no longer be canonicalised, so its parent is instead.
fn normalize_watched_path(path: &std::path::Path) -> std::path::PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            std::fs::canonicalize(parent).map_or_else(|_| path.to_path_buf(), |dir| dir.join(name))
        }
        _ => path.to_path_buf(),
    }
}

impl Drop for DiscovererRestoreGuard<'_> {
    fn drop(&mut self) {
        if let Some(d) = self.discoverer.take() {
            let mut slot = self.slot.write();
            if slot.is_none() {
                *slot = Some(d);
            }
        }
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let core = self.core.read();
        // Use try_lock for in_flight so Debug never deadlocks when called while
        // in_flight is already held by a concurrent register() call.
        let in_flight_snapshot: Vec<String> = self
            .in_flight
            .try_lock()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("Registry")
            .field("modules", &core.modules.keys().collect::<Vec<_>>())
            .field("descriptors", &core.descriptors)
            .field("ref_counts", &core.ref_counts)
            .field("draining", &core.draining)
            .field("in_flight", &in_flight_snapshot)
            .field(
                "drain_events_keys",
                &self.drain_events.read().keys().cloned().collect::<Vec<_>>(),
            )
            .field(
                "callbacks_keys",
                &self.callbacks.read().keys().cloned().collect::<Vec<_>>(),
            )
            .field("lowercase_map", &core.lowercase_map)
            .field(
                "schema_cache_keys",
                &core.schema_cache.keys().collect::<Vec<_>>(),
            )
            .field(
                "discoverer",
                &self.discoverer.read().as_ref().map(|_| "<Discoverer>"),
            )
            .field(
                "validator",
                &self.validator.read().as_ref().map(|_| "<Validator>"),
            )
            .finish_non_exhaustive()
    }
}

impl Registry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            core: RwLock::new(RegistryCore::new()),
            callbacks: RwLock::new(HashMap::new()),
            callback_counter: AtomicU64::new(1),
            deprecation_warned: ParkingLotMutex::new(HashSet::new()),
            drain_events: RwLock::new(HashMap::new()),
            discoverer: RwLock::new(None),
            validator: RwLock::new(None),
            extension_roots: RwLock::new(Vec::new()),
            watcher: parking_lot::Mutex::new(None),
            watch_handle: parking_lot::Mutex::new(None),
            in_flight: ParkingLotMutex::new(HashSet::new()),
            load_failed_callbacks: RwLock::new(Vec::new()),
            event_emitter: RwLock::new(None),
        }
    }

    /// Register a callback invoked when any module's on_load fails.
    ///
    /// The callback receives the module_id and the error from on_load.
    pub fn on_load_failed(&self, callback: Arc<LoadFailedCallbackFn>) {
        self.load_failed_callbacks.write().push(callback);
    }

    /// Wire an event emitter so the registry can emit normative registry
    /// events such as `apcore.registry.module_load_failed` (A-D-002).
    ///
    /// Mirrors apcore-python `Registry.set_event_emitter` and
    /// apcore-typescript `Registry.setEventEmitter`.
    pub fn set_event_emitter(&self, emitter: Arc<EventEmitter>) {
        *self.event_emitter.write() = Some(emitter);
    }

    /// Emit `apcore.registry.module_load_failed` when a module's `on_load`
    /// raises (A-D-002). Payload mirrors Python/TS:
    /// `{module_id, callback_name, error_type, error_message, timestamp}`.
    ///
    /// When no emitter is wired the failure is logged at `error` level so the
    /// signal is never silently lost (matching the Python fallback).
    fn emit_module_load_failed(&self, module_id: &str, error: &ModuleError) {
        let emitter = self.event_emitter.read().clone();
        let error_type = format!("{:?}", error.code);
        let error_message = error.message.clone();
        let timestamp = chrono::Utc::now().to_rfc3339();
        let Some(emitter) = emitter else {
            tracing::error!(
                module_id = %module_id,
                error_type = %error_type,
                error_message = %error_message,
                "apcore.registry.module_load_failed (no EventEmitter wired)"
            );
            return;
        };
        let event = ApCoreEvent::with_module(
            "apcore.registry.module_load_failed",
            serde_json::json!({
                "module_id": module_id,
                "callback_name": "on_load",
                "error_type": error_type,
                "error_message": error_message,
                "timestamp": timestamp,
            }),
            module_id,
            "error",
        );
        // A-D-013: dispatch via the DLQ-bearing path (retry + dead-letter)
        // rather than `emit_spawn` (single attempt, silent drop on failure).
        // Spec: events MUST deliver-or-dead-letter and MUST NOT silently drop.
        // Mirrors apcore-python (`emitter.emit(...)`) and apcore-typescript
        // (`emitter.emit(event)`), both of which route through the sole emit
        // that always retries and emits `apcore.event.delivery_failed`.
        emitter.emit_delivery_semantics(event);
    }

    /// Snapshot the callbacks for a given event name so they can be invoked
    /// without holding the callbacks lock.
    fn snapshot_callbacks(&self, event: &str) -> Vec<Arc<ModuleCallbackFn>> {
        self.callbacks
            .read()
            .get(event)
            .map(|v| v.iter().map(|(_, cb)| cb.clone()).collect())
            .unwrap_or_default()
    }

    /// Register a module with an explicit `ModuleDescriptor`.
    ///
    /// This is the **extended** registration form. Use it when you need to
    /// supply a pre-built descriptor (e.g. loaded from a config file or
    /// discovered from an external source). For the **spec-compliant** form
    /// (`register(module_id, module)` — two arguments, descriptor
    /// auto-generated from the module's schema methods), use
    /// [`register_module`](Self::register_module) instead.
    ///
    /// Validation order (`PROTOCOL_SPEC` §2.7, aligned with apcore-python /
    /// apcore-typescript): empty → pattern → length → reserved (per-segment)
    /// → duplicate.
    pub fn register(
        &self,
        name: &str,
        module: Box<dyn Module>,
        descriptor: ModuleDescriptor,
    ) -> Result<(), ModuleError> {
        self.register_with_context(name, module, descriptor, None)
    }

    /// Register a module with caller context for ephemeral audit events.
    pub fn register_with_context(
        &self,
        name: &str,
        module: Box<dyn Module>,
        descriptor: ModuleDescriptor,
        context: Option<&crate::Context<serde_json::Value>>,
    ) -> Result<(), ModuleError> {
        self.register_core_with_context(name, module.into(), descriptor, false, true, context)
    }

    /// Register a module — **spec-compliant two-argument form**.
    ///
    /// Equivalent to `register(module_id, module)` in the Python and
    /// TypeScript SDKs. The `ModuleDescriptor` is auto-generated from the
    /// module's `input_schema()` / `output_schema()` / annotations.
    ///
    /// Aligned with `apcore-python.Registry.register(module_id, module)` and
    /// `apcore-typescript.Registry.register(moduleId, module)`.
    ///
    /// When you need a custom descriptor, use
    /// [`register`](Self::register) (the three-argument extended form).
    pub fn register_module(&self, name: &str, module: Box<dyn Module>) -> Result<(), ModuleError> {
        self.register_module_with_context(name, module, None)
    }

    /// Register a module with inferred descriptor and contextual audit metadata.
    pub fn register_module_with_context(
        &self,
        name: &str,
        module: Box<dyn Module>,
        context: Option<&crate::Context<serde_json::Value>>,
    ) -> Result<(), ModuleError> {
        // Read the module's declarations ONCE: `metadata` also feeds
        // `dependencies` below, and `register_core` derives `sunset_date` from
        // `metadata["x-deprecation"]`.
        let metadata = module.metadata();
        let descriptor = ModuleDescriptor {
            module_id: name.to_string(),
            name: None,
            description: module.description().to_string(),
            // Sourced from the module rather than hardcoded to `None`: a
            // declaration the module carries (a `*.binding.yaml`
            // `documentation:` block reaches `FunctionModule.documentation`)
            // was otherwise discarded here. Matches apcore-python
            // (`registry/metadata.py`) and apcore-typescript
            // (`registry/metadata-pure.ts`).
            documentation: module.documentation().map(ToString::to_string),
            input_schema: module.input_schema(),
            output_schema: module.output_schema(),
            // Sourced from the module for the same reason as `documentation`
            // and `metadata`: D-97 gave `annotations` / `tags` /
            // `documentation` / `metadata` trait accessors and left `version`
            // and `examples` as `FunctionModule` fields no accessor reached, so
            // a `*.binding.yaml` `version:` was **validated** against
            // `validation.binding.version_require_semver` and then discarded —
            // the module registered as 1.0.0 and no version hint could find it.
            version: module
                .version()
                .filter(|v| !v.is_empty())
                .unwrap_or(DEFAULT_MODULE_VERSION)
                .to_string(),
            // A-D-017: populate tags from module.tags() like register_versioned,
            // instead of dropping them. Matches Python/TS register().
            tags: module.tags(),
            // Derive annotations from the module's `annotations()` trait method
            // instead of discarding them with `default()`. Matches Python
            // (`merge_module_metadata` reads `module.annotations`) and TS
            // (`register` merges the module's annotations), so declarations like
            // `requires_approval` survive the two-argument register path.
            annotations: Some(module.annotations()),
            // The second half of the same gap as `version` above: stored on the
            // module, reachable through no accessor, replaced with an empty Vec
            // here. Both peers merge the module's examples into the descriptor.
            examples: module.examples(),
            // Sourced from the module for the same reason as `documentation`
            // above: the binding loader computes a `display:` block into
            // `FunctionModule.metadata["apcore.display"]`, and an empty map
            // here overwrote it.
            metadata: metadata.clone(),
            display: None,
            // Derived from `metadata["x-deprecation"].sunset_date` by
            // `register_core`, which reads `descriptor.metadata` — now that the
            // module's metadata reaches the descriptor, that derivation has a
            // source on this path too.
            sunset_date: None,
            // Parsed from the module's own metadata, the same shape
            // `register_versioned` accepts (`{"dependencies": [{"module_id":
            // …}]}`). Empty when the module declares none.
            dependencies: dependencies_from_metadata(&metadata),
            enabled: true,
        };
        self.register_with_context(name, module, descriptor, context)
    }

    /// Register a module with explicit version and metadata — the
    /// **canonical four-argument form** matching the spec layout
    /// `register(module_id, module, version?, metadata?)` declared in
    /// [`registry-system.md` §Contract.Registry.register]
    /// and implemented natively by apcore-python and apcore-typescript.
    ///
    /// This is the cross-language-symmetric path: pass `version=None,
    /// metadata=None` for default (latest, no metadata) and the result
    /// matches `register_module`. Pass a non-`None` version to opt into
    /// multi-version coexistence (the version is stored in the
    /// descriptor and surfaces through `get_definition().version`);
    /// pass non-`None` metadata to seed the descriptor's metadata map.
    ///
    /// A `dependencies` key inside `metadata` — the cross-language shape
    /// `[{"module_id": "…", "version": "…", "optional": false}]` that
    /// apcore-python and apcore-typescript accept here — is parsed into
    /// `descriptor.dependencies` and readable through
    /// [`get_definition`](Self::get_definition). That is the accessor
    /// `ReloadModule::topo_sort_modules` consults, so declaring an edge this
    /// way orders a `path_filter` reload.
    ///
    /// All other descriptor fields are auto-derived from the module —
    /// schemas from `input_schema()` / `output_schema()`, description
    /// from `description()`, tags from the new `tags()` trait method
    /// (D11-003), and annotations from `annotations()`.
    ///
    /// Sync alignment: D10-010.
    pub fn register_versioned(
        &self,
        name: &str,
        module: Box<dyn Module>,
        version: Option<&str>,
        metadata: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), ModuleError> {
        let metadata = metadata.unwrap_or_default();
        // `metadata["dependencies"]` was discarded here: the descriptor hard-coded
        // an empty list, so the same call that builds a working dependency graph
        // in apcore-python and apcore-typescript built an empty one in Rust and
        // `get_definition().dependencies` came back empty. Only the three-argument
        // descriptor form could declare an edge, and `ReloadModule::topo_sort_modules`
        // reads that accessor — so a `path_filter` reload of modules registered this
        // way sorted an empty graph and degenerated to alphabetical order.
        // The `metadata` ARGUMENT first, then the module's own declaration.
        // Parsing only the argument left a module that declares dependencies on
        // ITSELF — which is how apcore-python (`getattr(module,
        // "dependencies", [])`) and apcore-typescript read them — registered
        // with an empty graph, and `ReloadModule::topo_sort_modules` reads that
        // accessor, so a `path_filter` reload of such modules degenerated to
        // alphabetical order.
        let mut dependencies = dependencies_from_metadata(&metadata);
        if dependencies.is_empty() {
            dependencies = module.dependencies();
        }
        let descriptor = ModuleDescriptor {
            module_id: name.to_string(),
            name: None,
            description: module.description().to_string(),
            // The canonical four-argument form dropped the module's own
            // `documentation` / `version` / `examples` exactly as
            // `register_module` did before D-97 — and this is the path
            // registry-system.md names as the cross-language-symmetric one, so
            // the peers' `register(id, module, version?, metadata?)` carried
            // all three while this carried none.
            documentation: module.documentation().map(ToString::to_string),
            input_schema: module.input_schema(),
            output_schema: module.output_schema(),
            // Explicit argument > module declaration > default, matching
            // apcore-python's `meta.get("version") or code_version or "1.0.0"`
            // and apcore-typescript's `mergeModuleMetadata`.
            version: version
                .filter(|v| !v.is_empty())
                .or_else(|| module.version().filter(|v| !v.is_empty()))
                .unwrap_or(DEFAULT_MODULE_VERSION)
                .to_string(),
            tags: module.tags(),
            // Derive annotations from the module's `annotations()` trait method
            // (matches Python/TS register), so `requires_approval` and other
            // declarations are not silently dropped.
            annotations: Some(module.annotations()),
            examples: module.examples(),
            metadata,
            display: None,
            // Derived from `metadata["x-deprecation"].sunset_date` by
            // `register_core`, which also logs the deprecation warning.
            sunset_date: None,
            dependencies,
            enabled: true,
        };
        // Multi-version coexistence (§5.4), as apcore-python registers it: an
        // explicit version of an ID that is already registered is another
        // version, not a duplicate.
        if version.is_some_and(|v| !v.is_empty()) && self.has(name) {
            return self.add_version(name, module.into(), descriptor);
        }
        self.register(name, module, descriptor)
    }

    /// Add another version of a registered module. The highest version is the
    /// one [`get`](Self::get) and [`get_definition`](Self::get_definition)
    /// return; registering the same version twice is a duplicate.
    fn add_version(
        &self,
        name: &str,
        module: Arc<dyn Module>,
        descriptor: ModuleDescriptor,
    ) -> Result<(), ModuleError> {
        let mut descriptor = descriptor;
        self.check_registration(name, &module, &mut descriptor, false, true)?;
        let version = descriptor.version.clone();
        let duplicate = || {
            ModuleError::new(
                crate::errors::ErrorCode::DuplicateModuleId,
                format!("Module '{name}' version '{version}' is already registered"),
            )
        };
        if self.version_registered(name, &version) {
            return Err(duplicate());
        }

        if let Err(e) = module.on_load() {
            self.emit_module_load_failed(name, &e);
            return Err(e);
        }

        {
            let mut core = self.core.write();
            let (Some(current), Some(current_descriptor)) = (
                core.modules.get(name).cloned(),
                core.descriptors.get(name).cloned(),
            ) else {
                // Unregistered while `on_load` ran: this is a first registration.
                drop(core);
                return self.register_core(name, module, descriptor, false, false);
            };
            let entries = core.versions.entry(name.to_string()).or_insert_with(|| {
                vec![VersionEntry {
                    version: current_descriptor.version.clone(),
                    module: current,
                    descriptor: current_descriptor,
                }]
            });
            if entries.iter().any(|e| e.version == version) {
                return Err(duplicate());
            }
            entries.push(VersionEntry {
                version,
                module: Arc::clone(&module),
                descriptor,
            });
            let latest = entries
                .iter()
                .max_by_key(|e| crate::registry::version::parse_semver(&e.version))
                .map(|e| (Arc::clone(&e.module), e.descriptor.clone()));
            if let Some((latest_module, latest_descriptor)) = latest {
                core.schema_cache.insert(
                    name.to_string(),
                    serde_json::json!({
                        "input": latest_descriptor.input_schema,
                        "output": latest_descriptor.output_schema,
                    }),
                );
                core.modules.insert(name.to_string(), latest_module);
                core.descriptors.insert(name.to_string(), latest_descriptor);
            }
        }

        for cb in self.snapshot_callbacks("register") {
            cb(name, module.as_ref());
        }
        Ok(())
    }

    /// Whether `version` of `name` is registered, as the primary entry or as
    /// one of several versions.
    fn version_registered(&self, name: &str, version: &str) -> bool {
        let core = self.core.read();
        match core.versions.get(name) {
            Some(entries) => entries.iter().any(|e| e.version == version),
            None => core
                .descriptors
                .get(name)
                .is_some_and(|d| d.version == version),
        }
    }

    /// Unregister a module by name.
    ///
    /// Returns `Ok(true)` if the module was found and removed, `Ok(false)` if
    /// the module was not registered (idempotent — matches apcore-python
    /// `return False` and apcore-typescript `return false` semantics,
    /// sync finding A-D-002).
    ///
    /// Ordering (aligned with `apcore-python.Registry.unregister`):
    ///
    /// 1. Acquire `core.write()`, remove from all maps atomically, drop lock.
    /// 2. Invoke `module.on_unload()` BEFORE firing the `"unregister"` callback
    ///    so subscribers observe the post-on_unload module state (sync A-D-003).
    /// 3. Callers holding `Arc<dyn Module>` references from earlier `get()`
    ///    calls keep the module alive; `on_unload` still runs exactly once.
    ///
    /// Note: the return type changes from `Result<(), ModuleError>` to
    /// `Result<bool, ModuleError>` in this version. Callers should check the
    /// bool rather than treating `Ok(())` as success.
    pub fn unregister(&self, name: &str) -> Result<bool, ModuleError> {
        self.unregister_with_context(name, None)
    }

    /// Unregister with caller context for ephemeral audit events.
    pub fn unregister_with_context(
        &self,
        name: &str,
        context: Option<&crate::Context<serde_json::Value>>,
    ) -> Result<bool, ModuleError> {
        let removed: Arc<dyn Module> = {
            let mut core = self.core.write();
            let Some(module) = core.modules.remove(name) else {
                return Ok(false);
            };
            core.descriptors.remove(name);
            core.lowercase_map.remove(&name.to_lowercase());
            core.schema_cache.remove(name);
            core.versions.remove(name);
            core.sources.remove(name);
            core.ref_counts.remove(name);
            core.draining.remove(name);
            module
        };
        self.drain_events.write().remove(name);

        // Fire on_unload BEFORE the "unregister" callback so subscribers observe
        // the post-on_unload module state, matching apcore-python and
        // apcore-typescript ordering (sync finding A-D-003).
        removed.on_unload();
        for cb in self.snapshot_callbacks("unregister") {
            cb(name, removed.as_ref());
        }
        self.emit_ephemeral_event("apcore.registry.module_unregistered", name, context);

        Ok(true)
    }

    /// Get a shared reference to a module by name.
    ///
    /// # Errors
    ///
    /// - `Err(ModuleError(code=ModuleNotFound))` if `name` is an empty string —
    ///   per `registry-system.md §Contract: Registry.get` Preconditions:
    ///   "module_id MUST NOT be an empty string. An empty module_id MUST be
    ///   rejected before any lock is acquired" (sync finding A-004).
    ///
    /// # Returns
    ///
    /// - `Ok(Some(module))` if the module is registered
    /// - `Ok(None)` if `name` is well-formed but the module is not registered
    pub fn get(&self, name: &str) -> Result<Option<Arc<dyn Module>>, ModuleError> {
        if name.is_empty() {
            return Err(ModuleError::new(
                crate::errors::ErrorCode::ModuleNotFound,
                "Module ID must not be empty",
            ));
        }
        Ok(self.core.read().modules.get(name).cloned())
    }

    /// Get the definition (descriptor) for a module by name.
    ///
    /// Returns a cloned `ModuleDescriptor` because the underlying storage
    /// is behind a lock — we cannot hand out borrowed references.
    ///
    /// # Errors
    ///
    /// Propagates [`Registry::get`]'s empty-id error: an empty `name` yields
    /// `Err(ModuleError(code=ModuleNotFound))`, matching apcore-python
    /// `Registry.get_definition` (which delegates to `get()` and lets the
    /// `ModuleNotFoundError` propagate) and apcore-typescript (sync finding
    /// A-D-001). A well-formed but unregistered `name` returns `Ok(None)`.
    pub fn get_definition(&self, name: &str) -> Result<Option<ModuleDescriptor>, ModuleError> {
        // Route through get() so the empty-id contract is surfaced uniformly.
        // Descriptors and modules are inserted/removed together (1:1), so a
        // present module always has a present descriptor.
        if self.get(name)?.is_none() {
            return Ok(None);
        }
        let descriptor = self.core.read().descriptors.get(name).cloned();
        // D-89 (spec v1.59.0): the READ is the emission point. Done outside the
        // core read lock — `log_deprecation_warning` calls into `tracing`, and a
        // subscriber is host code.
        if let Some(ref d) = descriptor {
            self.warn_if_deprecated(name, d);
        }
        Ok(descriptor)
    }

    /// List registered module names with optional filtering.
    ///
    /// - `tags`: if provided, only return modules whose descriptor annotations
    ///   contain ALL of the specified tags.
    /// - `prefix`: if provided, only return modules whose name starts with the prefix.
    /// - When both are `None`, returns all registered module names.
    ///
    /// Modules whose `ModuleAnnotations { discoverable: false, .. }` are
    /// excluded. Use [`Self::list_full`] with `include_hidden=true` when the
    /// full set is required (introspection, debug consoles).
    /// Return sorted list of unique registered module IDs, optionally filtered.
    ///
    /// # Arguments
    /// * `tags` - When supplied, only modules carrying *all* of the given tags are returned.
    /// * `prefix` - When supplied, only IDs starting with the prefix are returned.
    /// * `visibility` - Filter by module visibility. Supported: `["public", "hidden"]`.
    ///   Defaults to `["public"]` — protocol-spec §4.4 `discoverable`.
    ///
    /// Aligned with apcore-python `Registry.list(...)` (default
    /// `visibility=["public"]`).
    ///
    /// Returns owned `String` values because the storage is behind a lock.
    pub fn list(
        &self,
        tags: Option<&[&str]>,
        prefix: Option<&str>,
        visibility: Option<&[&str]>,
    ) -> Vec<String> {
        self.list_full(tags, prefix, visibility)
    }

    /// Same as [`Self::list`] but with explicit visibility control.
    ///
    /// Pass `visibility=Some(&["public", "hidden"])` to enumerate every registered
    /// module ID including those annotated `discoverable: false` (protocol-spec §2.5.1).
    /// Aligned with apcore-python `Registry.list(..., visibility=["public", "hidden"])`.
    pub fn list_full(
        &self,
        tags: Option<&[&str]>,
        prefix: Option<&str>,
        visibility: Option<&[&str]>,
    ) -> Vec<String> {
        let vis = visibility.unwrap_or(&["public"]);
        let show_public = vis.contains(&"public");
        let show_hidden = vis.contains(&"hidden");

        let core = self.core.read();
        let mut result: Vec<String> = core
            .modules
            .keys()
            .filter(|name| {
                let is_disc = descriptor_is_discoverable(core.descriptors.get(name.as_str()));
                if !((is_disc && show_public) || (!is_disc && show_hidden)) {
                    return false;
                }
                if let Some(pfx) = prefix {
                    if !name.starts_with(pfx) {
                        return false;
                    }
                }
                if let Some(required_tags) = tags {
                    // D11-003: union descriptor.tags with module.tags() so a
                    // module declaring `fn tags(&self) -> vec!["a"]` registered
                    // via register_module(name, module) (which builds an
                    // empty descriptor.tags) is filtered IN by tag-match
                    // queries — matches apcore-python (registry.py:1027) and
                    // apcore-typescript (registry.ts:689) which both union
                    // module-instance tags with merged-meta tags.
                    let mut module_tags: Vec<String> = core
                        .descriptors
                        .get(name.as_str())
                        .map(|desc| desc.tags.clone())
                        .unwrap_or_default();
                    if let Some(module) = core.modules.get(name.as_str()) {
                        for t in module.tags() {
                            if !module_tags.contains(&t) {
                                module_tags.push(t);
                            }
                        }
                    }
                    if !required_tags
                        .iter()
                        .all(|t| module_tags.contains(&t.to_string()))
                    {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        // Spec: list() MUST return sorted, unique IDs for cross-language parity
        // with apcore-python and apcore-typescript (sync finding A-D-103).
        result.sort();
        result
    }

    /// Check if a module is registered.
    pub fn has(&self, name: &str) -> bool {
        self.core.read().modules.contains_key(name)
    }

    /// Discover and register modules from a discoverer.
    ///
    /// Returns the count of entries that were actually registered —
    /// entries whose `name` fails `PROTOCOL_SPEC` §2.7 validation, whose
    /// `name` duplicates an already-discovered descriptor, or whose
    /// instance is rejected by the custom validator are skipped with a
    /// `tracing::warn!` and excluded from the count.
    ///
    /// Aligned with `apcore-python.Registry._discover_custom` and
    /// `apcore-typescript.Registry._discoverCustom` — same skip-and-warn
    /// semantics for malformed entries, and the same roots: the registry's
    /// [`extension_roots`](Self::extension_roots), which a filesystem
    /// discoverer such as [`DefaultDiscoverer`](crate::DefaultDiscoverer)
    /// scans and nothing else.
    #[allow(clippy::similar_names)] // `discoverer` (param) and `discovered` (result) are semantically distinct
    pub async fn discover(&self, discoverer: &dyn Discoverer) -> Result<usize, ModuleError> {
        let roots = self.extension_roots.read().clone();
        let discovered = discoverer.discover(&roots).await?;
        Ok(self.register_discovered(discovered))
    }

    /// Register a sys/internal module that bypasses **only** the reserved
    /// word check. All other `PROTOCOL_SPEC` §2.7 validations (empty, EBNF
    /// pattern, length, duplicate) still apply.
    ///
    /// The intended use case is registering modules under reserved prefixes
    /// like `system.health` or `system.control.toggle_feature` from
    /// `apcore::sys_modules`. Aligned with apcore-typescript
    /// `Registry.registerInternal`.
    ///
    /// Per protocol-spec §2.5.1, IDs in the
    /// reserved `ephemeral.*` namespace are rejected at this entry point.
    /// Agent-synthesized modules MUST go through [`Self::register`] /
    /// [`Self::register_module`] so the audit-emit / soft-warn pilot fires.
    pub fn register_internal(
        &self,
        name: &str,
        module: Box<dyn Module>,
        descriptor: ModuleDescriptor,
    ) -> Result<(), ModuleError> {
        if is_ephemeral_module_id(name) {
            return Err(ModuleError::new(
                crate::errors::ErrorCode::InvalidModuleId,
                format!(
                    "ephemeral.* module IDs must be registered via Registry::register(), \
                     not register_internal(). See protocol-spec §2.5.1 \
                     for rationale. (offending id: '{name}')"
                ),
            ));
        }
        self.register_core(name, module.into(), descriptor, true, false)
    }

    /// Soft-warn when an `ephemeral.*` module is registered without
    /// `requires_approval=true`.
    ///
    /// Per protocol-spec §2.5.1, agent-synthesized modules
    /// SHOULD declare `requires_approval: true` so a human gates execution.
    /// The registry only warns; it does not refuse the registration.
    ///
    /// The two sources are unioned (PROTOCOL_SPEC §7.4, D-96) because that is
    /// what the approval gate reads. Reading the descriptor alone — which this
    /// did, while apcore-python and apcore-typescript read the instance — made
    /// the same registration warn on one SDK and stay silent on another, and
    /// advised an author to set something they had already set on the module.
    fn warn_if_missing_approval(name: &str, module: &dyn Module, descriptor: &ModuleDescriptor) {
        let requires_approval = crate::module::ModuleAnnotations::governance_union(
            Some(&module.annotations()),
            descriptor.annotations.as_ref(),
        )
        .is_some_and(|a| a.requires_approval);
        if !requires_approval {
            tracing::warn!(
                module_id = %name,
                "ephemeral.* module registered without requires_approval=true. \
                 The apcore protocol-spec §2.5.1 recommends \
                 setting ModuleAnnotations {{ requires_approval: true, .. }} so \
                 agent-synthesized code does not run unattended."
            );
        }
    }

    /// Metadata key carrying a module's deprecation block.
    ///
    /// Cross-language shape (apcore-python `Registry.get_definition`,
    /// apcore-typescript `Registry.getDefinition`):
    /// `{"x-deprecation": {"deprecated_since": "…", "sunset_version": "…",
    /// "sunset_date": "…", "migration_guide": "…"}}`.
    const DEPRECATION_METADATA_KEY: &'static str = "x-deprecation";

    /// Log the deprecation warning for a module carrying `x-deprecation`.
    ///
    /// Wording mirrors apcore-python `Registry._log_deprecation_warning`,
    /// including the `"unknown"` fallbacks and the optional migration clause.
    fn log_deprecation_warning(
        name: &str,
        version: &str,
        deprecation: &serde_json::Map<String, serde_json::Value>,
    ) {
        let field = |key: &str| -> String {
            deprecation
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string()
        };
        let mut message = format!(
            "Module '{}' v{} is deprecated (since {}, sunset in {}).",
            name,
            version,
            field("deprecated_since"),
            field("sunset_version")
        );
        if let Some(guide) = deprecation
            .get("migration_guide")
            .and_then(serde_json::Value::as_str)
            .filter(|g| !g.is_empty())
        {
            use std::fmt::Write as _;
            let _ = write!(message, " Migration: {guide}");
        }
        tracing::warn!(module_id = %name, "{}", message);
    }

    /// Derive `descriptor.sunset_date` from `metadata["x-deprecation"]`.
    ///
    /// `sunset_date` is DERIVED, not hand-set: apcore-python reads it out of the
    /// `x-deprecation` block in `get_definition`, while every Rust descriptor
    /// builder hard-coded `None` and `x-deprecation` appeared nowhere in this
    /// crate — so a deprecated module registered here reported no sunset date
    /// and logged no warning. Doing it in the shared registration core covers
    /// `register`, `register_module` and `register_versioned` at once. An
    /// explicit descriptor value wins, so a caller may still set it directly.
    ///
    /// This registration step DERIVES the field only. The warning itself is
    /// emitted from [`Self::get_definition`] — see [`Self::warn_if_deprecated`].
    fn apply_deprecation_metadata(descriptor: &mut ModuleDescriptor) {
        let Some(deprecation) = descriptor
            .metadata
            .get(Self::DEPRECATION_METADATA_KEY)
            .and_then(serde_json::Value::as_object)
        else {
            return;
        };
        if descriptor.sunset_date.is_none() {
            descriptor.sunset_date = deprecation
                .get("sunset_date")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
        }
    }

    /// Emit the deprecation warning for `descriptor`, at most once per
    /// `(module_id, version, x-deprecation block)` per registry instance.
    ///
    /// # Where this fires (D-89, spec v1.59.0)
    ///
    /// The READ, not registration. This crate used to emit from the shared
    /// registration core and its reads never warned at all, which loses the
    /// notice outright for a host that registers before installing a
    /// `tracing` subscriber — `discover()` at startup is the ordinary case —
    /// because the dedupe entry is written whether or not anything was
    /// listening, so no later read can re-emit it. The accepted cost of the
    /// rule is stated rather than hidden: a module whose definition is never
    /// read is not warned about.
    ///
    /// # What a re-registration does (D-89, spec v1.59.0)
    ///
    /// The BLOCK is part of the key, and the key is never cleared on
    /// `unregister`. A re-registration carrying the same notice is silent; one
    /// carrying a changed or newly added notice warns. Clearing on unregister
    /// is the other wrong answer — `watch()` re-runs discovery as an
    /// unregister + re-register, so it re-warns for every deprecated module on
    /// every hot reload, which is the traffic-proportional spam D-89 exists to
    /// prevent.
    fn warn_if_deprecated(&self, name: &str, descriptor: &ModuleDescriptor) {
        let Some(deprecation) = descriptor
            .metadata
            .get(Self::DEPRECATION_METADATA_KEY)
            .and_then(serde_json::Value::as_object)
        else {
            return;
        };
        let notice = crate::schema::hardening::canonical_json(
            &descriptor.metadata[Self::DEPRECATION_METADATA_KEY],
        );
        let first_sighting = self.deprecation_warned.lock().insert((
            name.to_string(),
            descriptor.version.clone(),
            notice,
        ));
        if first_sighting {
            Self::log_deprecation_warning(name, &descriptor.version, deprecation);
        }
    }

    /// Shared registration core for `register`, `register_internal`, and the
    /// per-entry path of `register_discovered`.
    ///
    /// Ordering — deferred-publish pattern (Issue #65):
    /// 1. `validate_module_id` (syntactic, no lock).
    /// 2. Snapshot the validator `Arc` out of `self.validator` and invoke it
    ///    WITHOUT holding any lock (prevents parking_lot non-reentrant
    ///    deadlock if the validator calls back into the registry).
    /// 3. Acquire `core.read()`, run `detect_id_conflicts` (surfaces duplicate,
    ///    reserved-word, and case-collision conflicts), release the lock.
    /// 4. Acquire `in_flight.lock()`, atomically check + insert the ID so
    ///    concurrent same-ID registrations are rejected, release the lock.
    /// 5. Call `on_load` WITHOUT any lock held and WITHOUT the module in
    ///    `core.modules` (deferred-publish: module is NOT yet discoverable).
    /// 6. On success: acquire `core.write()`, insert into all visible maps,
    ///    release, then remove from `in_flight`. Fire `"register"` callbacks.
    /// 7. On failure: remove from `in_flight`, fire `load_failed` callbacks,
    ///    re-raise the error unchanged.
    ///
    /// # Lock ordering invariant
    ///
    /// Always acquire `core` before `in_flight`. Never hold `in_flight.lock()`
    /// while trying to acquire `core.read()` or `core.write()`. Violations
    /// create deadlock cycles.
    /// Re-publish a module instance the caller already holds (D-112).
    ///
    /// Every other registration entry point takes `Box<dyn Module>`, and
    /// `Registry::get` hands back an `Arc` — which cannot be turned back into a
    /// `Box`, because the Arc may be shared. Without this, restoring the
    /// PREVIOUS instance after a failed reload is not expressible here at all:
    /// the same "provided but uncallable" shape D-91 settled for
    /// `ExtensionManager::unregister`, reached from the other side.
    ///
    /// Runs the full registration path, so the restored module's `on_load`
    /// re-runs — which D-112 rule 2 requires, since its `on_unload` already ran
    /// during the unregister and re-publishing without `on_load` yields a
    /// module that is visible but torn down.
    ///
    /// # Errors
    ///
    /// Propagates whatever the registration path reports, including a failing
    /// `on_load`. D-112 rule 3: if the restoring load also fails the module
    /// stays unavailable, because publishing a module whose load hook failed is
    /// the defect deferred publication exists to prevent.
    pub fn reinstate_internal(
        &self,
        name: &str,
        module: Arc<dyn Module>,
        descriptor: ModuleDescriptor,
    ) -> Result<(), ModuleError> {
        self.register_core(name, module, descriptor, true, false)
    }

    /// The checks every registration path runs before `on_load`: the ID,
    /// deprecation metadata, the streaming marker and the custom validator.
    fn check_registration(
        &self,
        name: &str,
        module: &Arc<dyn Module>,
        descriptor: &mut ModuleDescriptor,
        allow_reserved: bool,
        run_validator: bool,
    ) -> Result<(), ModuleError> {
        validate_module_id(name, allow_reserved)?;

        Self::apply_deprecation_metadata(descriptor);

        // Ephemeral RFC pilot: emit a soft tracing::warn when an ephemeral.*
        // module lacks requires_approval=true. Does NOT fail the registration —
        // the audit-emit single-emit rule fires later via the sys_modules bridge.
        if is_ephemeral_module_id(name) {
            Self::warn_if_missing_approval(name, module.as_ref(), descriptor);
        }

        // Issue #62: if annotations declare streaming=true, the module MUST implement
        // StreamingModule (i.e. as_streaming() must return Some(_)).
        if let Some(ann) = descriptor.annotations.as_ref() {
            if ann.streaming && module.as_streaming().is_none() {
                return Err(ModuleError::streaming_interface_mismatch(
                    name,
                    "missing_marker: module declares streaming=true but does not implement StreamingModule",
                ));
            }
        }

        if run_validator {
            // Clone the validator Arc out of the lock so the user-supplied
            // `validate()` call happens without any Registry lock held.
            let validator_snapshot = self.validator.read().as_ref().map(Arc::clone);
            if let Some(validator) = validator_snapshot {
                let result = validator.validate(module.as_ref(), Some(descriptor));
                if !result.valid {
                    // A rejected module is invalid input to `register`, the
                    // code apcore-python and apcore-typescript raise; nothing
                    // failed to load.
                    return Err(ModuleError::new(
                        crate::errors::ErrorCode::GeneralInvalidInput,
                        format!(
                            "Module '{}' failed validation: {}",
                            name,
                            join_validation_messages(&result.errors)
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // one atomic register path: conflict detection, in_flight reservation, lock-free on_load, and the re-checked publish/rollback must stay together to preserve the lock-ordering and single-winner invariants
    fn register_core(
        &self,
        name: &str,
        module: Arc<dyn Module>,
        descriptor: ModuleDescriptor,
        allow_reserved: bool,
        run_validator: bool,
    ) -> Result<(), ModuleError> {
        self.register_core_with_context(
            name,
            module,
            descriptor,
            allow_reserved,
            run_validator,
            None,
        )
    }

    #[allow(clippy::too_many_lines)] // publication and lifecycle cleanup preserve one atomic registration operation
    fn register_core_with_context(
        &self,
        name: &str,
        module: Arc<dyn Module>,
        descriptor: ModuleDescriptor,
        allow_reserved: bool,
        run_validator: bool,
        context: Option<&crate::Context<serde_json::Value>>,
    ) -> Result<(), ModuleError> {
        let mut descriptor = descriptor;
        self.check_registration(
            name,
            &module,
            &mut descriptor,
            allow_reserved,
            run_validator,
        )?;

        // Issue #65: deferred-publish — run conflict detection first (under
        // core.read() to avoid holding write for long), then atomically reserve
        // the slot in in_flight so concurrent same-ID registrations are rejected.
        //
        // LOCK ORDER: acquire core before in_flight (see invariant in doc comment).
        {
            let core = self.core.read();
            // Full conflict detection (duplicate / reserved / case-collision).
            let reserved: &[&str] = if allow_reserved { &[] } else { RESERVED_WORDS };
            let existing_ids: HashSet<String> = core.modules.keys().cloned().collect();
            if let Some(conflict) =
                detect_id_conflicts(name, &existing_ids, reserved, Some(&core.lowercase_map))
            {
                match conflict.severity {
                    ConflictSeverity::Error => {
                        // Use DuplicateModuleId for exact-duplicate conflicts so callers
                        // can distinguish "already registered" from other error types.
                        let error_code = if conflict.conflict_type == ConflictType::DuplicateId {
                            crate::errors::ErrorCode::DuplicateModuleId
                        } else {
                            crate::errors::ErrorCode::GeneralInvalidInput
                        };
                        return Err(ModuleError::new(error_code, conflict.message));
                    }
                    ConflictSeverity::Warning => {
                        tracing::warn!(
                            module_id = %name,
                            conflict = %conflict.message,
                            "Module registration proceeded despite warning-level ID conflict"
                        );
                    }
                }
            }
        }

        // Reserve slot in in_flight (atomic check-and-insert).
        // If the ID is already in_flight (concurrent registration in progress),
        // return DuplicateModuleId immediately.
        {
            let mut in_flight = self.in_flight.lock();
            if in_flight.contains(name) {
                return Err(ModuleError::duplicate_module_id(name));
            }
            in_flight.insert(name.to_string());
        }

        let module_arc: Arc<dyn Module> = module;
        let module_clone = Arc::clone(&module_arc);

        // Issue #65: run on_load WITHOUT any lock held and WITHOUT the module
        // being visible in core.modules. Only after on_load succeeds do we
        // atomically publish the module (insert into core.modules) and remove
        // from in_flight.
        match module_clone.on_load() {
            Ok(()) => {
                // Atomic publish, guarded by a final duplicate re-check under the
                // write lock. The early conflict check and this publish are
                // separated by on_load() running lock-free; a concurrent same-ID
                // register that had already published AND cleared its in_flight
                // slot in that window would otherwise let a second winner through
                // (TOCTOU — the in_flight guard alone does not cover it because
                // the loser passed the read-side conflict check before the winner
                // published). Re-checking core.modules here under the write lock
                // closes it: exactly one registration wins.
                let published = {
                    let mut core = self.core.write();
                    if core.modules.contains_key(name) {
                        false
                    } else {
                        let schema = serde_json::json!({
                            "input": descriptor.input_schema,
                            "output": descriptor.output_schema,
                        });
                        core.schema_cache.insert(name.to_string(), schema);
                        core.lowercase_map
                            .insert(name.to_lowercase(), name.to_string());
                        core.modules
                            .insert(name.to_string(), Arc::clone(&module_arc));
                        core.descriptors.insert(name.to_string(), descriptor);
                        true
                    }
                };
                self.in_flight.lock().remove(name);

                if !published {
                    return Err(ModuleError::duplicate_module_id(name));
                }

                for cb in self.snapshot_callbacks("register") {
                    cb(name, module_clone.as_ref());
                }
                self.emit_ephemeral_event("apcore.registry.module_registered", name, context);
                Ok(())
            }
            Err(e) => {
                // Roll back: remove from in_flight; fire load_failed callbacks.
                self.in_flight.lock().remove(name);
                let cbs: Vec<Arc<LoadFailedCallbackFn>> = self
                    .load_failed_callbacks
                    .read()
                    .iter()
                    .map(Arc::clone)
                    .collect();
                for cb in cbs {
                    cb(name, &e);
                }
                // A-D-002: emit the normative registry event so subscribers
                // (not just bespoke callbacks) observe the load failure.
                self.emit_module_load_failed(name, &e);
                // Re-raise the original error unchanged.
                Err(e)
            }
        }
    }

    fn emit_ephemeral_event(
        &self,
        event_type: &str,
        name: &str,
        context: Option<&crate::Context<serde_json::Value>>,
    ) {
        if !is_ephemeral_module_id(name) {
            return;
        }
        let Some(emitter) = self.event_emitter.read().clone() else {
            return;
        };
        // Attribute bags may contain arbitrary credentials; omit them from the snapshot.
        let identity = context
            .and_then(|context| context.identity.as_ref())
            .map_or(serde_json::Value::Null, |identity| {
                serde_json::json!({
                    "id":identity.id(), "type":identity.identity_type(), "roles":identity.roles(),
                })
            });
        let payload = serde_json::json!({"namespace_class":"ephemeral", "identity":identity,
            "caller_id":context.and_then(|context| context.caller_id.as_deref())
                .unwrap_or(crate::acl::EXTERNAL_CALLER)});
        let event = ApCoreEvent::with_module(event_type, payload, name, "info");
        if tokio::runtime::Handle::try_current().is_ok() {
            emitter.emit_delivery_semantics(event);
        } else {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(emitter.emit(&event)),
                Err(error) => {
                    tracing::error!(error = %error, "Failed to initialize registry audit runtime");
                }
            }
        }
    }

    /// Apply a closure to every registered module.
    ///
    /// The closure is invoked while holding a read lock on the core, so it
    /// MUST NOT recursively acquire any Registry lock.
    pub fn for_each_module(&self, mut f: impl FnMut(&str, &dyn Module)) {
        let core = self.core.read();
        for (name, module) in &core.modules {
            f(name.as_str(), module.as_ref());
        }
    }

    /// Human-readable, Markdown-formatted module description for AI/LLM use.
    ///
    /// Produces the same document as `apcore-python Registry.describe` and
    /// `apcore-typescript Registry.describe`, so a polyglot consumer reading a
    /// module description gets identical structure from every SDK:
    ///
    /// ```text
    /// # {module_id}
    ///
    /// {description}
    ///
    /// **Tags:** a, b
    ///
    /// **Parameters:**
    /// - `param` (type) (required): description
    ///
    /// **Documentation:**
    /// {documentation}
    /// ```
    ///
    /// # Module-supplied override
    ///
    /// The peers check for an optional `describe()` member on the module
    /// object and return it verbatim when present. Rust has no `hasattr`, and
    /// [`Module::describe`](crate::module::Module::describe) is a *provided*
    /// trait method whose default returns the structured
    /// `{description, input_schema, output_schema, annotations}` object of
    /// PROTOCOL_SPEC §5.6 — so "did the module override it?" cannot be asked
    /// directly. The detectable analogue is the return *shape*: a module that
    /// overrides `describe()` to return a JSON **string** is supplying its own
    /// human-readable document, and that string is returned unchanged. Any
    /// other shape (including the default object) falls through to the
    /// generated envelope below.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::ModuleNotFound`](crate::errors::ErrorCode::ModuleNotFound)
    /// when `name` is not registered — never a `"Module not found"` sentinel
    /// string, matching the peers' `ModuleNotFoundError`.
    pub fn describe(&self, name: &str) -> Result<String, ModuleError> {
        let override_text = {
            let core = self.core.read();
            let module = core
                .modules
                .get(name)
                .ok_or_else(|| ModuleError::module_not_found(name))?;
            match module.describe() {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            }
        };
        if let Some(text) = override_text {
            return Ok(text);
        }

        // Auto-generate from the descriptor. A registered module with no
        // descriptor is not reachable through the normal registration paths,
        // but mirror the peers' graceful fallback rather than erroring.
        let Some(descriptor) = self.get_definition(name)? else {
            return Ok(format!("Module: {name}\n\nNo description available."));
        };

        let mut lines: Vec<String> = vec![format!("# {}", descriptor.module_id)];
        if !descriptor.description.is_empty() {
            lines.push(format!("\n{}", descriptor.description));
        }
        if !descriptor.tags.is_empty() {
            lines.push(format!("\n**Tags:** {}", descriptor.tags.join(", ")));
        }
        if let Some(props) = descriptor
            .input_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .filter(|p| !p.is_empty())
        {
            lines.push("\n**Parameters:**".to_string());
            let required: Vec<&str> = descriptor
                .input_schema
                .get("required")
                .and_then(serde_json::Value::as_array)
                .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
                .unwrap_or_default();
            for (param, schema) in props {
                let param_type = schema
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("any");
                let param_desc = schema
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let req_marker = if required.contains(&param.as_str()) {
                    " (required)"
                } else {
                    ""
                };
                lines.push(format!(
                    "- `{param}` ({param_type}){req_marker}: {param_desc}"
                ));
            }
        }
        if let Some(documentation) = descriptor.documentation.as_ref().filter(|d| !d.is_empty()) {
            lines.push(format!("\n**Documentation:**\n{documentation}"));
        }
        Ok(lines.join("\n"))
    }

    /// Draining-aware unregister.
    pub async fn safe_unregister(&self, name: &str, timeout_ms: u64) -> Result<bool, ModuleError> {
        // Mark draining, check ref_count. If zero we can proceed immediately.
        let need_wait_notify: Option<Arc<tokio::sync::Notify>> = {
            let mut core = self.core.write();
            if !core.modules.contains_key(name) {
                // Idempotent — match apcore-python `return False` and
                // apcore-typescript `return false` semantics (sync finding A-D-007).
                return Ok(false);
            }
            core.draining.insert(name.to_string());
            let current_refs = core.ref_counts.get(name).copied().unwrap_or(0);
            if current_refs == 0 {
                None
            } else {
                let notify = Arc::new(tokio::sync::Notify::new());
                self.drain_events
                    .write()
                    .insert(name.to_string(), Arc::clone(&notify));
                Some(notify)
            }
        };

        match need_wait_notify {
            None => {
                self.unregister(name)?;
                Ok(true)
            }
            Some(notify) => {
                let result = tokio::time::timeout(
                    std::time::Duration::from_millis(timeout_ms),
                    notify.notified(),
                )
                .await;

                let clean = result.is_ok();
                if !clean {
                    // Timeout — force-unload, matching Python and TypeScript behavior.
                    let in_flight = self.core.read().ref_counts.get(name).copied().unwrap_or(0);
                    tracing::warn!(
                        "Force-unloading module '{}' after {}ms timeout ({} in-flight executions)",
                        name,
                        timeout_ms,
                        in_flight,
                    );
                }
                {
                    let mut core = self.core.write();
                    core.draining.remove(name);
                }
                self.drain_events.write().remove(name);
                self.unregister(name)?;
                Ok(clean)
            }
        }
    }

    /// Ref-counted module access with explicit reference tracking.
    ///
    /// Acquire a reference to a module, incrementing its ref count.
    ///
    /// Cross-language parity: matches `apcore-python.Registry.acquire()` and
    /// `apcore-typescript.Registry.acquire()` — both bump ref_counts so
    /// `safe_unregister()` can wait for in-flight calls to drain (sync
    /// finding A-D-009).
    ///
    /// Callers MUST call [`release`](Self::release) when done with the module
    /// or `safe_unregister()` will hang until its timeout elapses.
    ///
    /// # Errors
    ///
    /// - `Err(ModuleError(code=ModuleNotFound))` if the module is currently
    ///   draining (a `safe_unregister` is in progress) or not registered.
    pub fn acquire(&self, name: &str) -> Result<Arc<dyn Module>, ModuleError> {
        let mut core = self.core.write();
        if core.draining.contains(name) {
            return Err(ModuleError::new(
                crate::errors::ErrorCode::ModuleNotFound,
                format!("Module '{name}' is draining"),
            ));
        }
        let module = core
            .modules
            .get(name)
            .cloned()
            .ok_or_else(|| ModuleError::module_not_found(name))?;
        *core.ref_counts.entry(name.to_string()).or_insert(0) += 1;
        Ok(module)
    }

    /// Release a previously acquired module reference.
    ///
    /// Decrements the ref count; when it reaches zero, notifies any drain
    /// waiter (i.e. `safe_unregister`) that the module is unused.
    ///
    /// Cross-language parity with `apcore-python.Registry.release()` and
    /// `apcore-typescript.Registry.release()` (sync finding A-D-009).
    ///
    /// Calling `release()` on a name that was never acquired is a no-op —
    /// matches Python/TS forgiving semantics.
    pub fn release(&self, name: &str) {
        let should_notify = {
            let mut core = self.core.write();
            if let Some(count) = core.ref_counts.get_mut(name) {
                if *count > 0 {
                    *count -= 1;
                }
                if *count == 0 {
                    core.ref_counts.remove(name);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        if should_notify {
            if let Some(notify) = self.drain_events.read().get(name) {
                notify.notify_one();
            }
        }
    }

    /// Deprecated alias for [`acquire`](Self::acquire) — kept for backward
    /// compatibility with apcore-rust 0.19.x which had separate
    /// `acquire`/`acquire_ref` semantics.
    #[deprecated(since = "0.20.0", note = "Use `acquire()` — it now ref-counts.")]
    pub fn acquire_ref(&self, name: &str) -> Result<Arc<dyn Module>, ModuleError> {
        self.acquire(name)
    }

    /// Deprecated alias for [`release`](Self::release) — kept for backward
    /// compatibility with apcore-rust 0.19.x.
    #[deprecated(since = "0.20.0", note = "Use `release()`.")]
    pub fn release_ref(&self, name: &str) {
        self.release(name);
    }

    /// Check if a module is draining.
    pub fn is_draining(&self, name: &str) -> bool {
        self.core.read().draining.contains(name)
    }

    /// Event subscription.
    /// Register an event callback and return a handle ID for later removal.
    ///
    /// Returns a `u64` handle that can be passed to [`off`](Self::off) to
    /// remove the callback.
    ///
    /// The event set is CLOSED (D-80): only the names in
    /// [`registry_events::ALL`] are accepted. Accepting anything else and
    /// handing back a valid-looking handle turns a typo into a permanently
    /// silent subscription — the callback never fires and nothing says so.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::GeneralInvalidInput`](crate::errors::ErrorCode::GeneralInvalidInput)
    /// when `event` is not one of [`registry_events::ALL`].
    pub fn on(&self, event: &str, callback: Box<ModuleCallbackFn>) -> Result<u64, ModuleError> {
        if !registry_events::ALL.contains(&event) {
            return Err(ModuleError::invalid_input(format!(
                "Unknown registry event: '{}'. Valid events: {}",
                event,
                registry_events::ALL.join(", ")
            ))
            .with_ai_guidance(format!(
                "'{}' is not a registry event. Subscribe to one of: {}.",
                event,
                registry_events::ALL.join(", ")
            )));
        }
        let id = self.callback_counter.fetch_add(1, Ordering::Relaxed);
        self.callbacks
            .write()
            .entry(event.to_string())
            .or_default()
            .push((id, Arc::from(callback)));
        Ok(id)
    }

    /// Remove a previously registered event callback by handle ID.
    ///
    /// Returns `true` if the callback was found and removed, `false` otherwise.
    ///
    /// D-80 requires `off` to reject an out-of-set event name too. Rust's `off`
    /// takes only the handle [`on`](Self::on) issued and no event name, so
    /// there is no name here to reject: an unknown name cannot reach this
    /// method because `on` refused to issue a handle for it. A miss stays a
    /// silent `false`, as before.
    pub fn off(&self, handle_id: u64) -> bool {
        let mut callbacks = self.callbacks.write();
        for entries in callbacks.values_mut() {
            if let Some(pos) = entries.iter().position(|(id, _)| *id == handle_id) {
                entries.remove(pos);
                return true;
            }
        }
        false
    }

    /// Re-run module discovery and update the registry.
    ///
    /// Equivalent to calling [`discover_internal`](Self::discover_internal).
    /// Returns the number of newly registered modules.
    pub async fn reload(&self) -> Result<usize, ModuleError> {
        self.discover_internal().await
    }

    /// Watch the extension roots and apply file changes to the registry.
    ///
    /// Watches every path in `extension_roots` recursively. File events are
    /// debounced (300ms) and then applied through the configured discoverer:
    /// a new module file is registered, a changed one REPLACES its registered
    /// module (`on_suspend` → unregister → register → `on_resume`), and a
    /// removed one is unregistered. Cross-language parity with apcore-python's
    /// `watchdog`-based watcher and apcore-typescript's `fs.watch` watcher
    /// (sync finding A-D-010).
    ///
    /// # Caller obligations
    ///
    /// `watch()` requires `&Arc<Self>` because the spawned background task
    /// holds a `Weak<Registry>` to drive re-discovery; the registry must
    /// already live behind `Arc`. Calling on a non-Arc Registry is a
    /// compile-time error.
    ///
    /// # Errors
    ///
    /// - `Err(ModuleError(code=ReloadFailed))` if the platform watcher cannot
    ///   be created (kernel resource exhaustion, missing permissions, etc.)
    ///
    /// # Idempotency
    ///
    /// Calling `watch()` while already watching is a no-op (returns `Ok(())`).
    /// Use [`Self::unwatch`] to stop and re-call `watch()` to pick up new
    /// `extension_roots`.
    // `async` is kept for cross-language API parity: Python and TypeScript both
    // expose `await registry.watch()`, and dropping it here would make the Rust
    // surface the odd one out for a function whose whole job is to run for a
    // long time. Rust 1.98 split the lint, so both names are needed; the
    // `unknown_lints` allow keeps older toolchains from rejecting the newer
    // name, which they have never heard of.
    #[allow(unknown_lints)]
    #[allow(clippy::unused_async)]
    #[allow(clippy::unused_async_trait_impl)]
    pub async fn watch(self: &Arc<Self>) -> Result<(), ModuleError> {
        use notify::{RecursiveMode, Watcher};

        {
            let watcher_slot = self.watcher.lock();
            if watcher_slot.is_some() {
                return Ok(()); // already watching
            }
        }

        let extension_roots: Vec<String> = self.extension_roots.read().clone();
        if extension_roots.is_empty() {
            tracing::warn!(
                "Registry::watch() called with no extension_roots — call set_extension_roots() first"
            );
            return Ok(());
        }

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<notify::Result<notify::Event>>();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            // Best-effort send; if the receiver is gone, the watcher will be
            // dropped shortly after.
            let _ = tx.send(res);
        })
        .map_err(|e| {
            ModuleError::new(
                crate::errors::ErrorCode::ReloadFailed,
                format!("Failed to create file watcher: {e}"),
            )
        })?;

        for root in &extension_roots {
            let path = std::path::Path::new(root);
            if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
                tracing::warn!(
                    root = %root,
                    error = %e,
                    "Registry::watch failed for root, skipping"
                );
            }
        }

        let weak = Arc::downgrade(self);
        let handle = tokio::spawn(Self::watch_loop(rx, weak));

        *self.watcher.lock() = Some(watcher);
        *self.watch_handle.lock() = Some(handle);

        Ok(())
    }

    /// Background task body that consumes notify events and applies each
    /// debounced batch of changed paths. Exits when the receiver channel
    /// closes or the `Weak<Registry>` can no longer be upgraded.
    async fn watch_loop(
        mut rx: tokio::sync::mpsc::UnboundedReceiver<notify::Result<notify::Event>>,
        weak: std::sync::Weak<Self>,
    ) {
        use std::time::Duration;

        const DEBOUNCE: Duration = Duration::from_millis(300);

        fn relevant(event: &notify::Event) -> bool {
            matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Modify(_)
                    | notify::EventKind::Remove(_)
            )
        }

        while let Some(res) = rx.recv().await {
            let Ok(event) = res else {
                continue;
            };
            if !relevant(&event) {
                continue;
            }
            // Collapse a burst (editors writing through temp file + rename)
            // into one pass, keeping every path the burst touched.
            let mut paths: Vec<std::path::PathBuf> = event.paths;
            while let Ok(Some(res)) = tokio::time::timeout(DEBOUNCE, rx.recv()).await {
                if let Ok(event) = res {
                    if relevant(&event) {
                        paths.extend(event.paths);
                    }
                }
            }

            let Some(reg) = weak.upgrade() else {
                break;
            };
            if let Err(e) = reg.apply_file_changes(&paths).await {
                tracing::warn!(
                    error = %e.message,
                    "Registry watch: applying file changes failed during hot-reload"
                );
            }
        }
    }

    /// Apply a batch of changed file paths: register new modules, replace the
    /// modules whose source (or `_meta.yaml` sidecar) changed, and unregister
    /// those whose source file is gone.
    async fn apply_file_changes(&self, paths: &[std::path::PathBuf]) -> Result<(), ModuleError> {
        let changed: HashSet<std::path::PathBuf> =
            paths.iter().map(|p| normalize_watched_path(p)).collect();
        let touches = |source: &str| {
            let source = normalize_watched_path(std::path::Path::new(source));
            if changed.contains(&source) {
                return true;
            }
            let (Some(dir), Some(stem)) = (source.parent(), source.file_stem()) else {
                return false;
            };
            changed.contains(&dir.join(format!("{}_meta.yaml", stem.to_string_lossy())))
        };

        let discovered = self.run_discoverer().await?;

        let removed: Vec<String> = {
            let core = self.core.read();
            core.sources
                .iter()
                .filter(|(id, source)| {
                    !std::path::Path::new(source).exists()
                        && touches(source)
                        && !discovered.iter().any(|dm| &dm.name == *id)
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in removed {
            self.safe_unregister(&id, 5000).await?;
        }

        let mut new_modules = Vec::new();
        for dm in discovered {
            if !self.has(&dm.name) {
                new_modules.push(dm);
            } else if touches(&dm.source) {
                self.replace_discovered(dm).await?;
            }
        }
        self.register_discovered(new_modules);
        Ok(())
    }

    /// Replace a registered module with a freshly discovered instance,
    /// handing its `on_suspend` state to the new instance's `on_resume`. The
    /// previous instance is put back if the new one fails to register.
    async fn replace_discovered(&self, dm: DiscoveredModule) -> Result<(), ModuleError> {
        let name = dm.name.clone();
        let previous = self.get(&name)?;
        let previous_descriptor = self.get_definition(&name)?;
        let state = previous.as_ref().and_then(|module| {
            let module = Arc::clone(module);
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || module.on_suspend()))
                .unwrap_or_else(|_| {
                    tracing::warn!(module_id = %name, "Module on_suspend panicked; continuing reload");
                    None
                })
        });

        self.safe_unregister(&name, 5000).await?;
        if self.register_discovered(vec![dm]) == 0 {
            tracing::warn!(
                module_id = %name,
                "Registry watch: the changed module failed to register; keeping the previous instance"
            );
            if let (Some(module), Some(descriptor)) = (previous, previous_descriptor) {
                self.reinstate_internal(&name, module, descriptor)?;
            }
            return Ok(());
        }

        if let (Some(state), Ok(Some(module))) = (state, self.get(&name)) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                module.on_resume(state);
            }))
            .is_err()
            {
                tracing::warn!(module_id = %name, "Module on_resume panicked; reload still applied");
            }
        }
        Ok(())
    }

    /// Stop filesystem watching. The background task is aborted and the
    /// platform watcher is dropped. Idempotent — calling on a non-watching
    /// Registry is a no-op.
    pub fn unwatch(&self) {
        // Drop the watcher first so notify stops sending; the spawned task
        // will end naturally when the channel closes. We also abort the task
        // explicitly in case the Drop implementation is delayed.
        self.watcher.lock().take();
        if let Some(handle) = self.watch_handle.lock().take() {
            handle.abort();
        }
    }

    /// Discover modules using the internally-set discoverer.
    ///
    /// Returns the number of newly registered modules.
    pub async fn discover_internal(&self) -> Result<usize, ModuleError> {
        let discovered = self.run_discoverer().await?;
        Ok(self.register_discovered(discovered))
    }

    /// Run the internally-set discoverer over the extension roots, without
    /// registering what it returns.
    async fn run_discoverer(&self) -> Result<Vec<DiscoveredModule>, ModuleError> {
        // Run discovery outside of any lock, but we need to briefly check
        // that a discoverer is set. We can't hold the discoverer lock across
        // `.await`, so we invoke it through a short-lived critical section
        // instead — we need the discoverer to survive across the await, so
        // the call itself happens while we still hold a read lock but that
        // is a `parking_lot::RwLockReadGuard` which is `Send`. However, we
        // must NOT hold it across `.await`. Workaround: require the
        // discoverer's `discover()` future to be pollable independently.
        //
        // We achieve this by extracting nothing from the discoverer guard —
        // instead we do discovery on a separate dedicated path: the
        // discoverer must provide its own internal sync. In practice we
        // simply call discoverer.discover().await inside the block, but we
        // cannot hold a parking_lot guard across await.
        //
        // Since Box<dyn Discoverer> can't be moved out of the Option under
        // a read lock, we accept a subtle limitation: discovery and
        // set_discoverer are mutually exclusive in time. We hold the
        // discoverer write lock briefly, replace it with None, perform the
        // discovery, then put it back via an RAII guard so the discoverer
        // is restored even if `discover().await` panics during unwind.
        let discoverer_opt = self.discoverer.write().take();
        let guard = DiscovererRestoreGuard {
            slot: &self.discoverer,
            discoverer: discoverer_opt,
        };
        let Some(active_discoverer) = guard.discoverer.as_ref() else {
            return Err(ModuleError::new(
                crate::errors::ErrorCode::NoDiscovererConfigured,
                "No discoverer configured".to_string(),
            ));
        };

        let roots = self.extension_roots.read().clone();
        let discover_result = active_discoverer.discover(&roots).await;
        // Explicit drop restores the discoverer via Drop impl (also runs on
        // panic unwind from the .await above). `active_discoverer` borrow
        // ends here, so the drop is reachable.
        drop(guard);

        discover_result
    }

    /// Return true if a descriptor's schema fields have an acceptable shape (object or null).
    ///
    /// Custom discoverers may return non-object JSON for `input_schema` / `output_schema`
    /// (e.g., a string or number). Calling this before insertion prevents invalid values from
    /// flowing into `schema_cache` and later breaking `export_schema()`.
    fn descriptor_schema_shape_is_valid(descriptor: &ModuleDescriptor) -> bool {
        let input_ok = descriptor.input_schema.is_object() || descriptor.input_schema.is_null();
        let output_ok = descriptor.output_schema.is_object() || descriptor.output_schema.is_null();
        input_ok && output_ok
    }

    /// Shared discovery post-processing for `discover()` and `discover_internal()`.
    ///
    /// For each entry: validate the name per `PROTOCOL_SPEC` §2.7, reject
    /// duplicates, run the custom validator, invoke `on_load`, then register
    /// the instance. Returns the count of entries that successfully registered.
    /// Failed entries are logged at `warn`/`error` and skipped; one bad entry
    /// never aborts the batch.
    ///
    /// Uses the same **deferred-publish** pattern as `register_core` (Issue #65):
    /// a module is NOT visible in `core.modules` until its `on_load` succeeds.
    #[allow(clippy::too_many_lines)] // sequential per-entry validation gates in a batch loop; splitting further requires passing state and reduces clarity
    fn register_discovered(&self, discovered: Vec<DiscoveredModule>) -> usize {
        let mut registered_count = 0usize;

        for dm in discovered {
            // 1. Validate module_id.
            if let Err(e) = validate_module_id(&dm.name, false) {
                tracing::warn!(
                    module_id = %dm.name,
                    error = %e.message,
                    "Discovered module rejected: invalid module_id"
                );
                continue;
            }

            // 1b. Reject the reserved `ephemeral.*` namespace.
            //
            // PROTOCOL_SPEC:424 names "Standard Registry.register() only" as
            // the registration mechanism for `ephemeral.*`. A Discoverer
            // reaching this sink would bypass `warn_if_missing_approval` and
            // the namespace's audit-provenance contract entirely. apcore-python
            // (`registry.py` `_register_discovered`) and apcore-typescript
            // (`registry.ts`) both reject here.
            if is_ephemeral_module_id(&dm.name) {
                tracing::warn!(
                    module_id = %dm.name,
                    "Discovered module rejected: the reserved ephemeral.* namespace \
                     may only be populated via Registry::register()"
                );
                continue;
            }

            // 2. Run custom validator (outside any lock).
            let validator_snapshot = self.validator.read().as_ref().map(Arc::clone);
            if let Some(validator) = validator_snapshot {
                let result = validator.validate(dm.module.as_ref(), Some(&dm.descriptor));
                if !result.valid {
                    tracing::warn!(
                        module_id = %dm.name,
                        errors = ?result.errors,
                        "Custom validator rejected discovered module"
                    );
                    continue;
                }
            }

            // 3. Validate descriptor schema shapes before insertion.
            if !Self::descriptor_schema_shape_is_valid(&dm.descriptor) {
                tracing::warn!(
                    module_id = %dm.name,
                    "Discovered module descriptor has non-object schema shape — skipping"
                );
                continue;
            }

            // 4. Conflict detection under a brief core.read() (LOCK ORDER: core before in_flight).
            {
                let core = self.core.read();
                let existing_ids: HashSet<String> = core.modules.keys().cloned().collect();
                match detect_id_conflicts(
                    &dm.name,
                    &existing_ids,
                    RESERVED_WORDS,
                    Some(&core.lowercase_map),
                ) {
                    Some(c) if c.severity == ConflictSeverity::Error => {
                        tracing::warn!(
                            module_id = %dm.name,
                            conflict = %c.message,
                            "Discovered module rejected: id conflict"
                        );
                        continue;
                    }
                    Some(c) => {
                        tracing::warn!(
                            module_id = %dm.name,
                            conflict = %c.message,
                            "Discovered module registered despite warning-level ID conflict"
                        );
                    }
                    None => {}
                }
            }

            // 5. Reserve slot in in_flight (deferred-publish: not yet visible).
            {
                let mut in_flight = self.in_flight.lock();
                if in_flight.contains(&dm.name) {
                    tracing::warn!(
                        module_id = %dm.name,
                        "Discovered module rejected: concurrent registration in progress"
                    );
                    continue;
                }
                in_flight.insert(dm.name.clone());
            }

            // 6. Run on_load WITHOUT any lock held (module NOT yet in core.modules).
            match dm.module.on_load() {
                Ok(()) => {
                    // Commit: insert into visible maps, remove from in_flight.
                    {
                        let mut core = self.core.write();
                        let schema = serde_json::json!({
                            "input": dm.descriptor.input_schema.clone(),
                            "output": dm.descriptor.output_schema.clone(),
                        });
                        core.schema_cache.insert(dm.name.clone(), schema);
                        core.lowercase_map
                            .insert(dm.name.to_lowercase(), dm.name.clone());
                        core.modules.insert(dm.name.clone(), Arc::clone(&dm.module));
                        core.descriptors
                            .insert(dm.name.clone(), dm.descriptor.clone());
                        core.sources.insert(dm.name.clone(), dm.source.clone());
                    }
                    self.in_flight.lock().remove(&dm.name);

                    for cb in self.snapshot_callbacks("register") {
                        cb(&dm.name, dm.module.as_ref());
                    }
                    registered_count += 1;
                }
                Err(e) => {
                    tracing::error!(
                        module_id = %dm.name,
                        error = %e.message,
                        "Discovered module on_load failed; skipping registration"
                    );
                    self.in_flight.lock().remove(&dm.name);
                    // The same two signals `register_core` fires on this branch.
                    // Only the tracing line and the in_flight release were here,
                    // so an on_load failure on the DISCOVERY path was invisible
                    // to both `on_load_failed` callbacks and to subscribers of
                    // `apcore.registry.module_load_failed` — while the identical
                    // failure through the manual `register()` path notified both.
                    // registry-system.md names "discovery-driven paths
                    // (discover(), register_discovered, hot-reload, etc.)"
                    // explicitly (sync finding A-D-005).
                    let cbs: Vec<Arc<LoadFailedCallbackFn>> = self
                        .load_failed_callbacks
                        .read()
                        .iter()
                        .map(Arc::clone)
                        .collect();
                    for cb in cbs {
                        cb(&dm.name, &e);
                    }
                    self.emit_module_load_failed(&dm.name, &e);
                }
            }
        }

        registered_count
    }

    /// Set the discoverer.
    pub fn set_discoverer(&self, discoverer: Box<dyn Discoverer>) {
        self.set_discoverer_shared(Arc::from(discoverer));
    }

    /// Install an already-shared discoverer handle.
    ///
    /// The counterpart to [`Self::set_discoverer`] for callers that keep their
    /// own reference — [`ExtensionManager::apply`](crate::extensions::ExtensionManager::apply)
    /// retains its registrations (D-78), so it wires a clone rather than the
    /// only copy.
    pub fn set_discoverer_shared(&self, discoverer: Arc<dyn Discoverer>) {
        *self.discoverer.write() = Some(discoverer);
    }

    /// Set the extension roots passed to `Discoverer::discover()`.
    ///
    /// Aligned with `apcore-python.Registry` (`_extension_roots`) and
    /// `apcore-typescript.Registry` (`_extensionRoots`). Each string is a
    /// filesystem path (or logical namespace) the discoverer should search.
    pub fn set_extension_roots(&self, roots: Vec<String>) {
        *self.extension_roots.write() = roots;
    }

    /// Return a snapshot of the configured extension roots.
    pub fn extension_roots(&self) -> Vec<String> {
        self.extension_roots.read().clone()
    }

    /// Populate the extension roots from a [`Config`], unless they are already set.
    ///
    /// This is the config tier of the precedence apcore-python and
    /// apcore-typescript implement in their `Registry` constructors —
    /// **explicit > config > default**:
    ///
    /// 1. Roots already set by [`Self::set_extension_roots`] win and are left
    ///    alone. That is the explicit tier; an embedder who wired roots by hand
    ///    must never have them overwritten by a config file.
    /// 2. Otherwise `extensions.roots` (multi-root mode) is read, accepting both
    ///    element forms — a bare path string and a `{ root, namespace }` object.
    /// 3. Otherwise `extensions.root` is read. `Config::get` already falls back
    ///    to the canonical `./extensions` default, so the default tier needs no
    ///    separate branch here.
    ///
    /// Before this existed, `extension_roots` initialised empty and was written
    /// only by `set_extension_roots`, so a client built from a `Config` that set
    /// `extensions.root` handed an empty slice to the discoverer and
    /// `APCore::discover()` returned `Ok(0)` — a silent no-op indistinguishable
    /// from an empty directory.
    pub fn set_extension_roots_from_config(&self, config: &crate::config::Config) {
        if !self.extension_roots.read().is_empty() {
            return;
        }

        if let Some(serde_json::Value::Array(items)) = config.get("extensions.roots") {
            let roots: Vec<String> = items
                .iter()
                .filter_map(|item| match item {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Object(o) => {
                        o.get("root").and_then(|r| r.as_str()).map(str::to_string)
                    }
                    _ => None,
                })
                .collect();
            if !roots.is_empty() {
                *self.extension_roots.write() = roots;
                return;
            }
        }

        if let Some(root) = config.get("extensions.root").and_then(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(std::string::ToString::to_string)
        }) {
            *self.extension_roots.write() = vec![root];
        }
    }

    /// Set the validator.
    pub fn set_validator(&self, validator: Box<dyn ModuleValidator>) {
        self.set_validator_shared(validator.into());
    }

    /// Install an already-shared validator handle.
    ///
    /// The counterpart to [`Self::set_validator`] for callers that keep their
    /// own reference (D-78).
    pub fn set_validator_shared(&self, validator: Arc<dyn ModuleValidator>) {
        *self.validator.write() = Some(validator);
    }

    /// Return count of registered modules.
    pub fn count(&self) -> usize {
        self.core.read().modules.len()
    }

    /// Return all module IDs, sorted alphabetically.
    ///
    /// Modules annotated `discoverable: false` are excluded by default per
    /// protocol-spec §2.5.1. Use [`Self::module_ids_full`]
    /// to include hidden modules.
    pub fn module_ids(&self) -> Vec<String> {
        self.module_ids_full(false)
    }

    /// Return all module IDs sorted alphabetically, with explicit control
    /// over whether hidden (`discoverable: false`) modules are included.
    pub fn module_ids_full(&self, include_hidden: bool) -> Vec<String> {
        let core = self.core.read();
        let mut ids: Vec<String> = core
            .modules
            .keys()
            .filter(|name| {
                include_hidden || descriptor_is_discoverable(core.descriptors.get(name.as_str()))
            })
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    /// Return a snapshot of all registered (`module_id`, module) pairs.
    ///
    /// Modules annotated `discoverable: false` are excluded by default per
    /// protocol-spec §2.5.1. Use [`Self::entries_full`] to
    /// include hidden modules.
    pub fn entries(&self) -> Vec<(String, Arc<dyn Module>)> {
        self.entries_full(false)
    }

    /// Return a snapshot of all registered (`module_id`, module) pairs with
    /// explicit control over whether hidden (`discoverable: false`) modules
    /// are included.
    pub fn entries_full(&self, include_hidden: bool) -> Vec<(String, Arc<dyn Module>)> {
        let core = self.core.read();
        core.modules
            .iter()
            .filter(|(k, _)| {
                include_hidden || descriptor_is_discoverable(core.descriptors.get(k.as_str()))
            })
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    }

    /// Export a module's schema in the cross-SDK envelope
    /// `{module_id, description, input_schema, output_schema}`.
    ///
    /// This is the canonical form, aligned with
    /// `apcore-python Registry.export_schema(module_id, strict=False)` and
    /// `apcore-typescript Registry.exportSchema(moduleId, strict=false)` — a
    /// polyglot consumer reading `result["input_schema"]` gets the same answer
    /// from every SDK. (Before 0.27 Rust returned the raw internal
    /// `{input, output}` cache entry, so that read silently yielded `null`.)
    ///
    /// Rust has no default arguments, so `strict` is an explicit parameter
    /// rather than a second method. When `strict=true`,
    /// [`to_strict_schema`](crate::schema::to_strict_schema) is applied to both
    /// schemas, producing a document that disallows `additionalProperties`,
    /// marks all properties required, and rewrites optional fields as nullable.
    ///
    /// Returns `None` if the module is not registered.
    pub fn export_schema(&self, name: &str, strict: bool) -> Option<serde_json::Value> {
        // An empty/unregistered name maps to None for this Option-returning API.
        let descriptor = self.get_definition(name).ok().flatten()?;
        let (input_schema, output_schema) = if strict {
            (
                crate::schema::to_strict_schema(&descriptor.input_schema),
                crate::schema::to_strict_schema(&descriptor.output_schema),
            )
        } else {
            (
                descriptor.input_schema.clone(),
                descriptor.output_schema.clone(),
            )
        };
        Some(serde_json::json!({
            "module_id": descriptor.module_id,
            "description": descriptor.description,
            "input_schema": input_schema,
            "output_schema": output_schema,
        }))
    }

    /// Mark a module as disabled in its descriptor.
    ///
    /// Disabled modules remain registered but callers should check `is_enabled()`
    /// before dispatching. Returns an error if the module is not found.
    pub fn disable(&self, name: &str) -> Result<(), ModuleError> {
        let mut core = self.core.write();
        let descriptor = core.descriptors.get_mut(name).ok_or_else(|| {
            ModuleError::new(
                crate::errors::ErrorCode::ModuleNotFound,
                format!("Module '{name}' not found"),
            )
        })?;
        descriptor.enabled = false;
        Ok(())
    }

    /// Mark a module as enabled in its descriptor.
    ///
    /// Returns an error if the module is not found.
    pub fn enable(&self, name: &str) -> Result<(), ModuleError> {
        let mut core = self.core.write();
        let descriptor = core.descriptors.get_mut(name).ok_or_else(|| {
            ModuleError::new(
                crate::errors::ErrorCode::ModuleNotFound,
                format!("Module '{name}' not found"),
            )
        })?;
        descriptor.enabled = true;
        Ok(())
    }

    /// Return whether a module is enabled (per its descriptor).
    ///
    /// Returns `None` if the module is not registered.
    pub fn is_enabled(&self, name: &str) -> Option<bool> {
        self.core.read().descriptors.get(name).map(|d| d.enabled)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod extension_roots_from_config_tests {
    use super::Registry;
    use crate::config::Config;

    fn config_from(json: serde_json::Value) -> Config {
        serde_json::from_value(json).expect("config fixture must deserialize")
    }

    #[test]
    fn config_extensions_root_reaches_the_registry() {
        // The #39 case. Before `set_extension_roots_from_config` existed this
        // registry kept an empty root list and `discover()` returned Ok(0).
        let registry = Registry::new();
        assert_eq!(registry.extension_roots(), Vec::<String>::new());

        let config = config_from(serde_json::json!({
            "version": "1.0.0",
            "project": { "name": "demo" },
            "extensions": { "root": "./my-modules" }
        }));
        registry.set_extension_roots_from_config(&config);

        assert_eq!(registry.extension_roots(), vec!["./my-modules".to_string()]);
    }

    #[test]
    fn default_applies_when_config_declares_nothing() {
        // Consequence 2 of #39: the canonical `./extensions` default never
        // reached Rust, because an empty vec has no fallback. `Config::get`
        // resolves the default tier, so reading through it is enough.
        let registry = Registry::new();
        let config = config_from(serde_json::json!({
            "version": "1.0.0",
            "project": { "name": "demo" }
        }));
        registry.set_extension_roots_from_config(&config);

        assert_eq!(registry.extension_roots(), vec!["./extensions".to_string()]);
    }

    #[test]
    fn explicit_roots_are_never_overwritten_by_config() {
        // The explicit tier. An embedder who wired roots by hand must keep them.
        let registry = Registry::new();
        registry.set_extension_roots(vec!["./hand-wired".to_string()]);

        let config = config_from(serde_json::json!({
            "version": "1.0.0",
            "project": { "name": "demo" },
            "extensions": { "root": "./from-config" }
        }));
        registry.set_extension_roots_from_config(&config);

        assert_eq!(registry.extension_roots(), vec!["./hand-wired".to_string()]);
    }

    #[test]
    fn multi_root_mode_accepts_both_element_forms() {
        // `extensions.roots` elements are a bare string OR a {root, namespace}
        // object; an implementation that models only one form drops roots.
        let registry = Registry::new();
        let config = config_from(serde_json::json!({
            "version": "1.0.0",
            "project": { "name": "demo" },
            "extensions": { "roots": [
                "./extensions",
                { "root": "./plugins", "namespace": "plugins" }
            ]}
        }));
        registry.set_extension_roots_from_config(&config);

        assert_eq!(
            registry.extension_roots(),
            vec!["./extensions".to_string(), "./plugins".to_string()]
        );
    }

    #[test]
    fn multi_root_takes_precedence_over_single_root() {
        let registry = Registry::new();
        let config = config_from(serde_json::json!({
            "version": "1.0.0",
            "project": { "name": "demo" },
            "extensions": { "roots": ["./a", "./b"] }
        }));
        registry.set_extension_roots_from_config(&config);

        assert_eq!(
            registry.extension_roots(),
            vec!["./a".to_string(), "./b".to_string()]
        );
    }
}

#[cfg(test)]
mod deprecation_metadata_tests {
    use super::Registry;
    use crate::context::Context;
    use crate::errors::ModuleError;
    use crate::module::Module;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::collections::HashMap;

    struct Noop;

    #[async_trait]
    impl Module for Noop {
        fn input_schema(&self) -> Value {
            json!({})
        }
        fn output_schema(&self) -> Value {
            json!({})
        }
        fn description(&self) -> &'static str {
            "noop"
        }
        async fn execute(
            &self,
            _inputs: Value,
            _ctx: &Context<Value>,
        ) -> Result<Value, ModuleError> {
            Ok(json!({}))
        }
    }

    fn deprecation_metadata() -> HashMap<String, Value> {
        let mut metadata = HashMap::new();
        metadata.insert(
            "x-deprecation".to_string(),
            json!({
                "deprecated_since": "0.9.0",
                "sunset_version": "1.0.0",
                "sunset_date": "2027-01-31",
                "migration_guide": "Use executor.email.send_email_v2",
            }),
        );
        metadata
    }

    #[test]
    fn sunset_date_is_derived_from_x_deprecation() {
        // apcore-python reads `sunset_date` out of the `x-deprecation` block;
        // every Rust descriptor builder used to hard-code `None`, so a
        // deprecated module reported no sunset date at all.
        let registry = Registry::new();
        registry
            .register_versioned(
                "executor.email.send_email",
                Box::new(Noop),
                Some("0.9.0"),
                Some(deprecation_metadata()),
            )
            .expect("register");

        let desc = registry
            .get_definition("executor.email.send_email")
            .expect("get_definition")
            .expect("descriptor");
        assert_eq!(desc.sunset_date.as_deref(), Some("2027-01-31"));
    }

    #[test]
    fn sunset_date_stays_none_without_x_deprecation() {
        let registry = Registry::new();
        registry
            .register_versioned("executor.email.send", Box::new(Noop), None, None)
            .expect("register");

        let desc = registry
            .get_definition("executor.email.send")
            .expect("get_definition")
            .expect("descriptor");
        assert!(desc.sunset_date.is_none());
    }

    #[test]
    fn an_explicit_sunset_date_is_not_overwritten() {
        let registry = Registry::new();
        let descriptor = crate::registry::registry::ModuleDescriptor {
            module_id: "executor.email.legacy".to_string(),
            name: None,
            description: "noop".to_string(),
            documentation: None,
            input_schema: json!({}),
            output_schema: json!({}),
            version: "0.9.0".to_string(),
            tags: vec![],
            annotations: None,
            examples: vec![],
            metadata: deprecation_metadata(),
            display: None,
            sunset_date: Some("2026-12-01".to_string()),
            dependencies: vec![],
            enabled: true,
        };
        registry
            .register("executor.email.legacy", Box::new(Noop), descriptor)
            .expect("register");

        let desc = registry
            .get_definition("executor.email.legacy")
            .expect("get_definition")
            .expect("descriptor");
        assert_eq!(desc.sunset_date.as_deref(), Some("2026-12-01"));
    }

    #[test]
    fn deprecation_warning_fires_on_the_read_not_on_registration() {
        // D-89 / spec v1.59.0. This crate used to record the marker (and emit)
        // from the shared REGISTRATION core, and its reads never warned at all
        // — which loses the notice outright for a host that registers before
        // installing a `tracing` subscriber, because the marker is written
        // whether or not anything was listening and no later read can re-emit.
        //
        // This test reads the private marker set, so it proves WHERE the entry
        // is recorded, not that a subscriber received anything. The emitted
        // warning is counted in tests/test_deprecation_warning_cadence.rs,
        // which is the assertion that survives deleting the `tracing::warn!`.
        let registry = Registry::new();

        registry
            .register_versioned(
                "executor.email.send_email",
                Box::new(Noop),
                Some("0.9.0"),
                Some(deprecation_metadata()),
            )
            .expect("register");
        assert!(
            registry.deprecation_warned.lock().is_empty(),
            "registration must not consume the once-per-notice budget"
        );

        // The first read records it; further reads do not.
        for _ in 0..5 {
            let _ = registry.get_definition("executor.email.send_email");
        }
        assert_eq!(registry.deprecation_warned.lock().len(), 1);
    }

    #[test]
    fn the_deprecation_dedupe_key_carries_the_notice_and_survives_unregister() {
        // D-89 / spec v1.59.0. A re-registration carrying the SAME notice is
        // silent — `watch()` re-runs discovery as an unregister + re-register,
        // so clearing the key there re-warns for every deprecated module on
        // every hot reload. A re-registration carrying a CHANGED notice warns,
        // which is what keying on the block buys and what this crate used to
        // swallow.
        let registry = Registry::new();

        let register = |version: &str, metadata: HashMap<String, Value>| {
            registry
                .register_versioned(
                    "executor.email.send_email",
                    Box::new(Noop),
                    Some(version),
                    Some(metadata),
                )
                .expect("register");
            let _ = registry.get_definition("executor.email.send_email");
        };

        register("0.9.0", deprecation_metadata());
        assert_eq!(registry.deprecation_warned.lock().len(), 1);

        // Same notice, same version: no new entry.
        registry
            .unregister("executor.email.send_email")
            .expect("unregister");
        register("0.9.0", deprecation_metadata());
        assert_eq!(
            registry.deprecation_warned.lock().len(),
            1,
            "an unchanged notice must not be recorded twice"
        );

        // Changed notice, same version: a new entry.
        registry
            .unregister("executor.email.send_email")
            .expect("unregister");
        let mut changed = HashMap::new();
        changed.insert(
            "x-deprecation".to_string(),
            json!({
                "deprecated_since": "0.9.0",
                "sunset_version": "0.10.0",
                "sunset_date": "2026-06-30",
                "migration_guide": "Use executor.email.send_email_v2",
            }),
        );
        register("0.9.0", changed);
        assert_eq!(
            registry.deprecation_warned.lock().len(),
            2,
            "a CHANGED notice on a re-registered module must warn"
        );

        // A different VERSION is still a different notice.
        registry
            .unregister("executor.email.send_email")
            .expect("unregister");
        register("0.10.0", deprecation_metadata());
        assert_eq!(registry.deprecation_warned.lock().len(), 3);
    }
}
