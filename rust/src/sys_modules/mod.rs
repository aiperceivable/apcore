// APCore Protocol — System modules registration
// Spec reference: Built-in system modules (F10, F11, F19) +
//                 system-modules.md §1.1–§1.5 hardening (Issue #45).

pub mod audit;
pub mod control;
pub mod health;
pub mod manifest;
pub mod overrides;
pub mod usage;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;

use serde_json::json;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::errors::{ErrorCode, ModuleError};
use crate::events::circuit_breaker::{CircuitBreakerWrapper, CircuitEventSink};
use crate::events::emitter::{ApCoreEvent, EventEmitter};
use crate::events::subscribers::create_subscriber;
use crate::executor::Executor;
use crate::middleware::PlatformNotifyMiddleware;
use crate::module::Module;
use crate::observability::error_history::{ErrorHistory, ErrorHistoryMiddleware};
use crate::observability::metrics::MetricsCollector;
use crate::observability::usage::{UsageCollector, UsageMiddleware};
use crate::registry::registry::{is_ephemeral_module_id, ModuleDescriptor, Registry};

pub use audit::{AuditAction, AuditChange, AuditEntry, AuditStore, InMemoryAuditStore};
pub use control::{ReloadModule, ToggleFeatureModule, UpdateConfigModule};
pub use health::{HealthModule, HealthSummaryModule};
pub use manifest::{ManifestFullModule, ManifestModule};
pub use usage::{UsageModule, UsageSummaryModule};

#[derive(Debug)]
struct EmitterCircuitSink(std::sync::Weak<EventEmitter>);

impl CircuitEventSink for EmitterCircuitSink {
    fn emit_circuit_event(&self, event: ApCoreEvent) {
        if let Some(emitter) = self.0.upgrade() {
            emitter.emit_delivery_semantics(event);
        }
    }
}

// ---------------------------------------------------------------------------
// ToggleState — thread-safe enable/disable tracking
// ---------------------------------------------------------------------------

/// Thread-safe set of disabled module IDs.
#[derive(Debug)]
pub struct ToggleState {
    disabled: RwLock<HashSet<String>>,
}

impl ToggleState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            disabled: RwLock::new(HashSet::new()),
        }
    }

    pub fn is_disabled(&self, module_id: &str) -> bool {
        self.disabled.read().contains(module_id)
    }

    pub fn disable(&self, module_id: &str) {
        self.disabled.write().insert(module_id.to_string());
    }

    pub fn enable(&self, module_id: &str) {
        self.disabled.write().remove(module_id);
    }

    pub fn clear(&self) {
        self.disabled.write().clear();
    }
}

impl Default for ToggleState {
    fn default() -> Self {
        Self::new()
    }
}

// Global default instance, shared via Arc so the execution pipeline
// (`BuiltinModuleLookup`) and `ToggleFeatureModule` observe the SAME toggle
// store. This is the authoritative disabled-set that survives registry
// reloads (the registry descriptor's `enabled` flag is reset on
// re-registration, so the pipeline must consult ToggleState — sync finding
// A-D-12).
static GLOBAL_TOGGLE_STATE: OnceLock<Arc<ToggleState>> = OnceLock::new();

fn global_toggle_state() -> &'static Arc<ToggleState> {
    GLOBAL_TOGGLE_STATE.get_or_init(|| Arc::new(ToggleState::new()))
}

/// Shared handle to the process-global [`ToggleState`].
///
/// The execution pipeline's module-lookup step reads this store to reject
/// disabled modules, and [`register_sys_modules`] injects it into
/// [`control::ToggleFeatureModule`] so toggles are immediately visible to the
/// pipeline and survive registry reloads.
#[must_use]
pub fn global_toggle_state_arc() -> Arc<ToggleState> {
    Arc::clone(global_toggle_state())
}

/// Check if a module is disabled using the default global toggle state.
#[must_use]
pub fn is_module_disabled(module_id: &str) -> bool {
    global_toggle_state().is_disabled(module_id)
}

/// Return `Err(ModuleError)` with `ErrorCode::ModuleDisabled` if the module is disabled.
pub fn check_module_disabled(module_id: &str) -> Result<(), ModuleError> {
    if is_module_disabled(module_id) {
        return Err(ModuleError::new(
            ErrorCode::ModuleDisabled,
            format!("Module '{module_id}' is disabled"),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sensitive key detection
// ---------------------------------------------------------------------------

pub(crate) const SENSITIVE_SEGMENTS: &[&str] =
    &["token", "secret", "key", "password", "auth", "credential"];

pub(crate) fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    // W-6: Match exact segments ("key") or underscore-compound segments ("api_key",
    // "auth_token") without false-positives on "keyboard" or "authentication".
    lower.split('.').any(|seg| {
        SENSITIVE_SEGMENTS.iter().any(|&s| {
            seg == s || seg.ends_with(&format!("_{s}")) || seg.starts_with(&format!("{s}_"))
        })
    })
}

// ---------------------------------------------------------------------------
// Restricted config keys
// ---------------------------------------------------------------------------

// W-7: Lists keys that must not be changed at runtime via update_config.
// Scope: runtime-safety critical keys only. Schema-level immutability is
// enforced at load time; this list protects against inadvertent runtime mutations.
pub(crate) const RESTRICTED_KEYS: &[&str] = &["sys_modules.enabled"];

// ---------------------------------------------------------------------------
// Shared helpers (used by control.rs)
// ---------------------------------------------------------------------------

pub(crate) fn require_string(
    inputs: &serde_json::Value,
    field: &str,
) -> Result<String, ModuleError> {
    inputs
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(std::string::ToString::to_string)
        .ok_or_else(|| {
            ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!("'{field}' is required and must be a non-empty string"),
            )
        })
}

pub(crate) fn missing_field_error(field: &str) -> ModuleError {
    ModuleError::new(
        ErrorCode::GeneralInvalidInput,
        format!("'{field}' is required"),
    )
}

/// Emit an event; errors are logged and not propagated (error isolation).
pub(crate) async fn emit_event(
    emitter: &Arc<EventEmitter>,
    event_type: &str,
    module_id: &str,
    timestamp: &str,
    data: serde_json::Value,
) {
    let event = ApCoreEvent {
        event_type: event_type.to_string(),
        timestamp: timestamp.to_string(),
        data,
        module_id: Some(module_id.to_string()),
        severity: "info".to_string(),
    };
    // EventEmitter is interior-mutable (D1-011) — emit without an external lock.
    emitter.emit(&event).await;
}

/// Default `caller_id` when the `Context` has none (Issue #45.2 — contextual
/// auditing). Cross-language parity: `apcore-python` and `apcore-typescript`
/// both fall back to the same literal.
///
/// Aliases [`crate::acl::EXTERNAL_CALLER`] rather than restating it: the value
/// an audit record shows for a context-less call and the value the ACL matched
/// to produce that record are the same identity, and two independent literals
/// are how they drift apart.
pub(crate) const DEFAULT_EXTERNAL_CALLER: &str = crate::acl::EXTERNAL_CALLER;

/// Identity attribute names whose values are replaced with `<redacted>` in
/// audit-event payloads. Mirrors `apcore-python._IDENTITY_SENSITIVE_SUBSTRINGS`
/// (Issue #45.2). The list is intentionally a superset of the canonical
/// `obs.redaction.sensitive_keys` so bearer tokens, signed cookies, and
/// credentials can never leak through the contextual-audit channel even when
/// global redaction is disabled.
const IDENTITY_SENSITIVE_SUBSTRINGS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "key",
    "auth",
    "credential",
    "cookie",
    "session",
    "bearer",
];

const IDENTITY_REDACTED_TOKEN: &str = "<redacted>";

fn redact_identity_attr(name: &str, value: &serde_json::Value) -> serde_json::Value {
    let lower = name.to_lowercase();
    if IDENTITY_SENSITIVE_SUBSTRINGS
        .iter()
        .any(|sub| lower.contains(sub))
    {
        return serde_json::Value::String(IDENTITY_REDACTED_TOKEN.to_string());
    }
    value.clone()
}

/// Augment an audit-event payload with caller identity extracted from the
/// `Context` (Issue #45.2). Adds:
///   * `caller_id` — taken from `ctx.caller_id`, defaulted to `"@external"`
///     when the context is unauthenticated or the field is empty.
///   * `identity` — a redaction-safe snapshot (`id`, `type`, optional `roles`,
///     plus any non-sensitive `attrs` entries verbatim; sensitive attrs are
///     replaced with `"<redacted>"`). Omitted entirely when `ctx.identity`
///     is `None`.
///   * `actor_id` / `actor_type` — flat aliases preserved for backward
///     compatibility with pre-D-31 consumers.
///
/// The `data` argument is mutated in place and returned for ergonomic chaining.
pub(crate) fn augment_with_context_identity(
    mut data: serde_json::Value,
    ctx: &crate::context::Context<serde_json::Value>,
) -> serde_json::Value {
    if let Some(obj) = data.as_object_mut() {
        let caller_id = ctx
            .caller_id
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| DEFAULT_EXTERNAL_CALLER.to_string());
        obj.insert("caller_id".to_string(), serde_json::json!(caller_id));
        if let Some(identity) = ctx.identity.as_ref() {
            // Flat aliases (legacy).
            obj.insert("actor_id".to_string(), serde_json::json!(identity.id()));
            obj.insert(
                "actor_type".to_string(),
                serde_json::json!(identity.identity_type()),
            );

            // Canonical nested snapshot.
            let mut snapshot = serde_json::Map::new();
            snapshot.insert("id".to_string(), serde_json::json!(identity.id()));
            snapshot.insert(
                "type".to_string(),
                serde_json::json!(identity.identity_type()),
            );
            let roles = identity.roles();
            if !roles.is_empty() {
                snapshot.insert("roles".to_string(), serde_json::json!(roles));
            }
            for (key, value) in identity.attrs() {
                if matches!(key.as_str(), "id" | "type" | "roles") {
                    continue;
                }
                snapshot.insert(key.clone(), redact_identity_attr(key, value));
            }
            obj.insert("identity".to_string(), serde_json::Value::Object(snapshot));
        }
    }
    data
}

// ---------------------------------------------------------------------------
// SysModuleError — strict registration failure (Issue #45 §1.5)
// ---------------------------------------------------------------------------

/// Error type returned by `register_sys_modules`. The `RegistrationFailed`
/// variant carries the offending `module_id` so callers can route the failure
/// to module-specific recovery logic.
#[derive(Debug, Error)]
pub enum SysModuleError {
    #[error("system module '{module_id}' failed to register: {source}")]
    RegistrationFailed {
        module_id: String,
        #[source]
        source: ModuleError,
    },
}

impl SysModuleError {
    #[must_use]
    pub fn module_id(&self) -> &str {
        match self {
            Self::RegistrationFailed { module_id, .. } => module_id,
        }
    }

    #[must_use]
    pub fn error_code(&self) -> ErrorCode {
        ErrorCode::SysModuleRegistrationFailed
    }
}

// ---------------------------------------------------------------------------
// SysModulesContext — typed return value for register_sys_modules
// ---------------------------------------------------------------------------

/// Holds references to components created during sys-module registration.
pub struct SysModulesContext {
    pub registered_modules: HashMap<String, serde_json::Value>,
    pub emitter: Arc<EventEmitter>,
    pub toggle_state: Arc<ToggleState>,
    pub error_history: ErrorHistory,
    pub usage_collector: UsageCollector,
    pub audit_store: Option<Arc<dyn AuditStore>>,
}

// ---------------------------------------------------------------------------
// SysModulesOptions — optional inputs for hardening features (§1.1, §1.2, §1.5)
// ---------------------------------------------------------------------------

/// Optional knobs accepted by [`register_sys_modules_with_options`].
///
/// Defaults preserve pre-hardening behavior: no overrides file, no audit store,
/// `fail_on_error: false`. Cross-language: maps to the `audit_store`,
/// `overrides_path`, and `fail_on_error` parameters in the Python and
/// TypeScript SDKs.
#[derive(Default, Clone)]
pub struct SysModulesOptions {
    /// When set, runtime overrides are loaded from this YAML path on startup
    /// and persisted on every `update_config` / `toggle_feature` call. Use
    /// [`Self::overrides_store`] for non-file-backed persistence.
    pub overrides_path: Option<PathBuf>,
    /// Pluggable [`OverridesStore`] backend. When set, takes precedence over
    /// `overrides_path` and is wired into `UpdateConfigModule` /
    /// `ToggleFeatureModule` for read-modify-write persistence. Cross-language:
    /// matches the `overrides_store` parameter in `apcore-python` and
    /// `apcore-typescript`.
    ///
    /// [`OverridesStore`]: overrides::OverridesStore
    pub overrides_store: Option<Arc<dyn overrides::OverridesStore>>,
    /// When set, every state-changing control call appends an `AuditEntry`.
    /// When `None`, audit entries are logged at INFO level and discarded.
    pub audit_store: Option<Arc<dyn AuditStore>>,
    /// When `true`, any module-registration failure halts startup with an
    /// `Err(SysModuleError::RegistrationFailed)`. Default is `false`, which
    /// matches the lenient behavior of the Python/TypeScript SDKs.
    pub fail_on_error: bool,
    /// Per-instance toggle store shared with the execution pipeline's
    /// `module_lookup` step (issue #71). When `None` (the default), the
    /// process-global store is used, preserving pre-#71 behavior. `APCore`
    /// passes its own `Arc<ToggleState>` here so the write path
    /// (`ToggleFeatureModule`) and the read path (pipeline lookup) share one
    /// instance-scoped store.
    pub toggle_state: Option<Arc<ToggleState>>,
}

// ---------------------------------------------------------------------------
// register_sys_modules — main entry, breaking change in 0.20.0 (§1.5)
// ---------------------------------------------------------------------------

/// Register built-in system modules into the registry.
///
/// **Breaking change in 0.20.0** (system-modules.md §1.5): the return type is
/// now `Result<SysModulesContext, SysModuleError>`. When `sys_modules.enabled`
/// is `false`, the function returns `Ok(SysModulesContext { … empty … })`
/// rather than `Option::None` so callers can always destructure the value.
///
/// For overrides persistence and audit trails, use
/// [`register_sys_modules_with_options`].
///
/// Workflow (per spec §9.15):
/// 1. Check `sys_modules.enabled` — return an empty `SysModulesContext` if `false`.
/// 2. Load `overrides_path` (if any) into the live `Config` after the base load.
/// 3. Create `ErrorHistory` + `ErrorHistoryMiddleware`.
/// 4. Create `UsageCollector` + `UsageMiddleware`.
/// 5. Register health, manifest, and usage modules.
/// 6. If `sys_modules.events.enabled`: register control modules + `EventEmitter`.
pub fn register_sys_modules(
    registry: Arc<Registry>,
    executor: &Executor,
    config: &Config,
    metrics_collector: Option<MetricsCollector>,
) -> Result<SysModulesContext, SysModuleError> {
    register_sys_modules_with_options(
        registry,
        executor,
        config,
        metrics_collector,
        SysModulesOptions::default(),
    )
}

/// Variant of [`register_sys_modules`] accepting hardening options. See
/// [`SysModulesOptions`] for details.
#[allow(clippy::too_many_lines)] // complex orchestration; extraction would obscure the registration flow
#[allow(clippy::needless_pass_by_value)] // public API: Arc<Registry> and Option<MetricsCollector> consumed by sub-modules
pub fn register_sys_modules_with_options(
    registry: Arc<Registry>,
    executor: &Executor,
    config: &Config,
    metrics_collector: Option<MetricsCollector>,
    options: SysModulesOptions,
) -> Result<SysModulesContext, SysModuleError> {
    let SysModulesOptions {
        overrides_path,
        overrides_store,
        audit_store,
        fail_on_error,
        toggle_state,
    } = options;

    // Use the caller-supplied per-instance toggle store (issue #71) so toggles
    // made via ToggleFeatureModule are observed by THIS instance's execution
    // pipeline (BuiltinModuleLookup) and survive registry reloads (sync finding
    // A-D-12). Fall back to the process-global store when no instance store is
    // provided, preserving pre-#71 behavior.
    let toggle_state = toggle_state.unwrap_or_else(global_toggle_state_arc);

    // Spec default is FALSE: system modules are opt-in. Mirrors apcore-python
    // registration.py:335 and apcore-typescript registration.ts:250 (sync
    // finding A-D-11). Previously defaulted to true, registering system
    // modules even when the config omitted `sys_modules.enabled`.
    let enabled = config
        .get("sys_modules.enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return Ok(SysModulesContext {
            registered_modules: HashMap::new(),
            emitter: Arc::new(EventEmitter::new()),
            toggle_state,
            error_history: ErrorHistory::with_limits(50, 1000),
            usage_collector: UsageCollector::new(),
            audit_store,
        });
    }

    // --- §1.1: load overrides into a mutable Config clone, then share it ---
    let mut effective_config = config.clone();
    // Explicit option wins; otherwise fall back to the
    // `sys_modules.control.overrides_path` CONFIG key, as apcore-python does
    // (registration.py:382). Reading `SysModulesOptions` alone meant a project
    // that declared the key in apcore.yaml got neither persistence nor restore
    // (sync finding A-D-015).
    let resolved_overrides_path: Option<PathBuf> = overrides_path.clone().or_else(|| {
        config
            .get("sys_modules.control.overrides_path")
            .and_then(|v| v.as_str().map(PathBuf::from))
    });
    if let Some(path) = resolved_overrides_path.as_deref() {
        overrides::load_overrides(path, &mut effective_config, Some(&toggle_state));
    } else if overrides_store.is_some() {
        // An injected OverridesStore is written to by the control modules but
        // cannot be READ here: `OverridesStore::load` is async and
        // `register_sys_modules` is sync, so awaiting it would change this
        // function's signature and every caller with it. apcore-typescript hits
        // the same wall and logs the same way. Callers needing restore from a
        // programmatic store should pre-load it and pass the values through
        // `Config`, or use `overrides_path`.
        tracing::warn!(
            "An OverridesStore was supplied but cannot be loaded during synchronous \
             register_sys_modules; its persisted values will not be restored. Pre-load \
             into Config, or use sys_modules.control.overrides_path."
        );
    }

    // --- Step 2: ErrorHistory + middleware ---
    #[allow(clippy::cast_possible_truncation)] // config value won't exceed platform usize limits
    let max_per_module = effective_config
        .get("sys_modules.error_history.max_entries_per_module")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;
    #[allow(clippy::cast_possible_truncation)] // config value won't exceed platform usize limits
    let max_total = effective_config
        .get("sys_modules.error_history.max_total_entries")
        .and_then(|v| v.as_u64())
        .unwrap_or(1000) as usize;
    let error_history = ErrorHistory::with_limits(max_per_module, max_total);
    let eh_middleware = ErrorHistoryMiddleware::new(error_history.clone());
    if let Err(e) = executor.use_middleware(Box::new(eh_middleware)) {
        tracing::error!(error = %e, middleware = "ErrorHistoryMiddleware", "sys middleware registration failed");
    }

    // --- Step 3: UsageCollector + middleware ---
    let usage_collector = UsageCollector::new();
    let usage_middleware = UsageMiddleware::new(usage_collector.clone());
    if let Err(e) = executor.use_middleware(Box::new(usage_middleware)) {
        tracing::error!(error = %e, middleware = "UsageMiddleware", "sys middleware registration failed");
    }

    let config_arc = Arc::new(Mutex::new(effective_config.clone()));

    // Build the EventEmitter up-front as an owned value so we can populate
    // its subscribers from config synchronously, then wrap it in the Arc<Mutex<_>>
    // shared with sys modules.
    let emitter_arc = Arc::new(EventEmitter::new());

    let events_enabled = effective_config
        .get("sys_modules.events.enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // §1.1 + §1.2 wire `overrides_path` / `overrides_store` / `audit_store`
    // through the control modules — but control modules are only registered
    // when events are enabled. Surface a warning instead of silently dropping
    // the options so misconfiguration shows up at startup, not as missing
    // audit entries.
    if !events_enabled
        && (overrides_path.is_some() || overrides_store.is_some() || audit_store.is_some())
    {
        tracing::warn!(
            overrides_path_set = overrides_path.is_some(),
            overrides_store_set = overrides_store.is_some(),
            audit_store_set = audit_store.is_some(),
            "SysModulesOptions overrides/audit options set but \
             sys_modules.events.enabled=false — control modules are not \
             registered, so these options have no effect. Enable events to \
             activate runtime overrides and audit trails."
        );
    }

    if events_enabled {
        // Configured subscribers share the bootstrap emitter through weak circuit sinks.
        if let Some(subs) = effective_config.get("sys_modules.events.subscribers") {
            if let Some(arr) = subs.as_array() {
                for sub_config in arr {
                    match create_subscriber(sub_config) {
                        Ok(subscriber) => {
                            let circuit = sub_config.get("circuit_breaker");
                            let value = |key: &str, default: u64| {
                                circuit
                                    .and_then(|c| c.get(key))
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(default)
                            };
                            let threshold =
                                u32::try_from(value("open_threshold", 5)).unwrap_or(u32::MAX);
                            let wrapped = CircuitBreakerWrapper::new(
                                subscriber,
                                Arc::new(EmitterCircuitSink(Arc::downgrade(&emitter_arc))),
                            )
                            .with_timeout_ms(value("timeout_ms", 5000))
                            .with_open_threshold(threshold)
                            .with_recovery_window_ms(value("recovery_window_ms", 60000));
                            emitter_arc.subscribe(Box::new(wrapped));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to create subscriber from config");
                        }
                    }
                }
            }
        }
    }

    // --- Step 4: Build the module list the per-group flags select ---
    //
    // `sys_modules.enabled` is the master switch (checked above). The three
    // flags here select WHICH modules register once activation has happened —
    // the split PROTOCOL_SPEC 9.15.3 states (v1.17.0) and
    // `schemas/sys-modules.schema.json` declares key by key: "Whether
    // system.health.summary and system.health.module are registered", and so
    // on. All three default to true, so a flag narrows and never widens.
    //
    // They were read by nothing until now (apcore#118): every flag false
    // registered the same six modules as every flag true, while the master
    // switch worked — a half-alive section, so a smoke test of it passes.
    let group_enabled = |group: &str| -> bool {
        effective_config
            .get(&format!("sys_modules.{group}.enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    };
    let health_enabled = group_enabled("health");
    let manifest_enabled = group_enabled("manifest");
    let usage_enabled = group_enabled("usage");
    let control_enabled = group_enabled("control");

    let mut modules: Vec<(&str, Box<dyn Module>, Vec<String>)> = Vec::new();
    if health_enabled {
        modules.extend::<Vec<(&str, Box<dyn Module>, Vec<String>)>>(vec![
            (
                "system.health.summary",
                Box::new(health::HealthSummaryModule::new(
                    Arc::clone(&registry),
                    metrics_collector.clone(),
                    error_history.clone(),
                    Arc::clone(&config_arc),
                )),
                vec!["system".into(), "health".into()],
            ),
            (
                "system.health.module",
                Box::new(health::HealthModule::new(
                    Arc::clone(&registry),
                    metrics_collector.clone(),
                    error_history.clone(),
                )),
                vec!["system".into(), "health".into()],
            ),
        ]);
    }
    if manifest_enabled {
        modules.extend::<Vec<(&str, Box<dyn Module>, Vec<String>)>>(vec![
            (
                "system.manifest.module",
                Box::new(manifest::ManifestModule::new(
                    Arc::clone(&registry),
                    Arc::clone(&config_arc),
                )),
                vec!["system".into(), "manifest".into()],
            ),
            (
                "system.manifest.full",
                Box::new(manifest::ManifestFullModule::new(
                    Arc::clone(&registry),
                    Arc::clone(&config_arc),
                )),
                vec!["system".into(), "manifest".into()],
            ),
        ]);
    }
    if usage_enabled {
        modules.extend::<Vec<(&str, Box<dyn Module>, Vec<String>)>>(vec![
            (
                "system.usage.summary",
                Box::new(usage::UsageSummaryModule::new(usage_collector.clone())),
                vec!["system".into(), "usage".into()],
            ),
            (
                "system.usage.module",
                Box::new(usage::UsageModule::new(
                    Arc::clone(&registry),
                    usage_collector.clone(),
                )),
                vec!["system".into(), "usage".into()],
            ),
        ]);
    }

    // --- Step 5: Control modules only if events.enabled AND control.enabled ---
    //
    // `control.enabled` selects whether the Level 2 WRITE plane registers once
    // `events.enabled` has activated it — the schema reads "Whether the
    // system.control.* modules are registered". Default true, so it narrows and
    // never widens. Read by nothing until now (apcore#118), which made it the
    // most consequential of the four inert sub-flags: an operator writing
    // `control.enabled: false` to keep the approval-gated write surface off a
    // deployment got all three modules anyway.
    if events_enabled {
        let error_rate_threshold = effective_config
            .get("sys_modules.events.thresholds.error_rate")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.1);
        let latency_p99_threshold = effective_config
            .get("sys_modules.events.thresholds.latency_p99_ms")
            .and_then(|v| v.as_f64())
            .unwrap_or(5000.0);
        let pn_middleware = PlatformNotifyMiddleware::new(
            // Share the bus the configured subscribers are attached to. This was
            // `EventEmitter::new()` — a private, subscriber-less bus, so
            // apcore.health.* threshold alerts were emitted and dropped.
            Arc::clone(&emitter_arc),
            metrics_collector.clone(),
            error_rate_threshold,
            latency_p99_threshold,
        );
        if let Err(e) = executor.use_middleware(Box::new(pn_middleware)) {
            tracing::error!(error = %e, middleware = "PlatformNotifyMiddleware", "sys middleware registration failed");
        }

        if control_enabled {
            modules.push((
                "system.control.update_config",
                Box::new(
                    UpdateConfigModule::new(Arc::clone(&config_arc), Arc::clone(&emitter_arc))
                        // The RESOLVED path: an overrides file named only by the
                        // config key is restored above and must be written back
                        // here, or a restart loses every runtime change.
                        .with_overrides_path(resolved_overrides_path.clone())
                        .with_overrides_store(overrides_store.clone())
                        .with_audit_store(audit_store.clone()),
                ),
                vec!["system".into(), "control".into()],
            ));
            modules.push((
                "system.control.reload_module",
                Box::new(
                    ReloadModule::new(Arc::clone(&registry), Arc::clone(&emitter_arc))
                        .with_config(Some(Arc::clone(&config_arc)))
                        .with_audit_store(audit_store.clone()),
                ),
                vec!["system".into(), "control".into()],
            ));
            modules.push((
                "system.control.toggle_feature",
                Box::new(
                    ToggleFeatureModule::new(
                        Arc::clone(&registry),
                        Arc::clone(&emitter_arc),
                        Arc::clone(&toggle_state),
                    )
                    .with_overrides_path(resolved_overrides_path.clone())
                    .with_overrides_store(overrides_store.clone())
                    .with_audit_store(audit_store.clone()),
                ),
                vec!["system".into(), "control".into()],
            ));
        }
    }

    // --- Register all modules ---
    let mut registered: HashMap<String, serde_json::Value> = HashMap::new();

    for (id, module, tags) in modules {
        let is_control = tags.contains(&"control".to_string());
        let descriptor = ModuleDescriptor {
            module_id: id.to_string(),
            name: None,
            description: module.description().to_string(),
            documentation: None,
            input_schema: module.input_schema(),
            output_schema: module.output_schema(),
            version: "1.0.0".to_string(),
            tags,
            annotations: Some(crate::module::ModuleAnnotations {
                requires_approval: is_control,
                readonly: !is_control,
                // Setting a module to the state it is already in changes
                // nothing, so toggle_feature is idempotent where the other two
                // control modules are not (system-modules.md; SYS-19).
                idempotent: !is_control || id == "system.control.toggle_feature",
                // D-119: written out rather than inherited. `Default` gives
                // `open_world: true`, which means the OPPOSITE of the intended
                // value — no system module reaches an external system — and
                // relying on that default is how the divergence arose.
                open_world: false,
                ..Default::default()
            }),
            examples: vec![],
            metadata: std::collections::HashMap::new(),
            display: None,
            sunset_date: None,
            dependencies: vec![],
            enabled: true,
        };
        let info = json!({
            "name": id,
            "description": module.description(),
        });
        match registry.register_internal(id, module, descriptor) {
            Ok(()) => {
                registered.insert(id.to_string(), info);
            }
            Err(e) => {
                if fail_on_error {
                    return Err(SysModuleError::RegistrationFailed {
                        module_id: id.to_string(),
                        source: e,
                    });
                }
                tracing::error!(module_id = %id, error = %e, "System module failed to register; continuing");
            }
        }
    }

    // Step 5d: Bridge registry events to ApCoreEvents.
    //
    // Each registry hook emits only the canonical `apcore.registry.<event>`
    // name per the v0.22.0 spec MUST; the legacy bare-name aliases
    // (`module_registered`, `module_unregistered`) were removed (apcore#78).
    //
    // Registry callbacks are synchronous, so each hook spawns a task to
    // dispatch the async emit — fire-and-forget, error-isolated.
    if events_enabled {
        let emitter_for_register = Arc::clone(&emitter_arc);
        registry.set_event_emitter(Arc::clone(&emitter_arc));
        // `on` is fallible since D-80; both names are in the closed set, so the
        // `?` can only fire if that set ever changes under us.
        registry
            .on(
                crate::registry::registry_events::REGISTER,
                Box::new(move |module_id: &str, _module: &dyn Module| {
                    if is_ephemeral_module_id(module_id) {
                        return;
                    }
                    tracing::info!(module_id = %module_id, "module_registered");
                    let emitter = Arc::clone(&emitter_for_register);
                    let module_id_owned = module_id.to_string();
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        handle.spawn(async move {
                            let canonical = ApCoreEvent::with_module(
                                "apcore.registry.module_registered",
                                json!({}),
                                &module_id_owned,
                                "info",
                            );
                            emitter.emit(&canonical).await;
                        });
                    }
                }),
            )
            .unwrap_or_else(|e| {
                // Unreachable: REGISTER is in the closed event set. Log rather
                // than widen SysModuleError for a branch that cannot be taken.
                tracing::error!(error = %e.message, "registry 'register' subscription refused");
                0
            });
        let emitter_for_unregister = Arc::clone(&emitter_arc);
        registry
            .on(
                crate::registry::registry_events::UNREGISTER,
                Box::new(move |module_id: &str, _module: &dyn Module| {
                    if is_ephemeral_module_id(module_id) {
                        return;
                    }
                    tracing::info!(module_id = %module_id, "module_unregistered");
                    let emitter = Arc::clone(&emitter_for_unregister);
                    let module_id_owned = module_id.to_string();
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        handle.spawn(async move {
                            let canonical = ApCoreEvent::with_module(
                                "apcore.registry.module_unregistered",
                                json!({}),
                                &module_id_owned,
                                "info",
                            );
                            emitter.emit(&canonical).await;
                        });
                    }
                }),
            )
            .unwrap_or_else(|e| {
                // Unreachable: UNREGISTER is in the closed event set.
                tracing::error!(error = %e.message, "registry 'unregister' subscription refused");
                0
            });
    }

    Ok(SysModulesContext {
        registered_modules: registered,
        emitter: emitter_arc,
        toggle_state,
        error_history,
        usage_collector,
        audit_store,
    })
}
