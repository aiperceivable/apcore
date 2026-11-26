// APCore Protocol — System control modules
// Spec reference: system.control.update_config (F11), system.control.reload_module (F10),
//                 system.control.toggle_feature (F19)
// Behaviour (system-modules.md §system.control.*):
//   - overrides_path persistence for update_config + toggle_feature
//   - contextual AuditEntry recorded for every state-changing call
//   - path_filter glob (mutually exclusive with module_id) and
//     dependency-topological reload order

use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::{canonical_config_key, Config};
use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};
use crate::events::emitter::EventEmitter;
use crate::module::Module;
use crate::observability::redaction::{RedactionConfig, DEFAULT_REPLACEMENT};
use crate::registry::dependencies::resolve_dependencies;
use crate::registry::registry::{Registry, DEFAULT_MODULE_VERSION};
use crate::registry::types::DepInfo;
use crate::utils::helpers::match_glob;

use super::audit::{build_audit_entry, record_audit, AuditAction, AuditChange, AuditStore};
use super::overrides::{
    normalize_config_overrides, persist_one, FileOverridesStore, OverridesStore,
    TOGGLE_OVERRIDE_PREFIX,
};
use super::{
    augment_with_context_identity, emit_event, is_sensitive_key, missing_field_error,
    require_string, ToggleState, RESTRICTED_KEYS,
};

fn is_restricted_config_key(key: &str) -> bool {
    key == "apcore"
        || key == "toggle"
        || key.starts_with(TOGGLE_OVERRIDE_PREFIX)
        || RESTRICTED_KEYS.iter().any(|&restricted| {
            key == restricted
                || restricted
                    .strip_prefix(key)
                    .is_some_and(|tail| tail.starts_with('.'))
                || key
                    .strip_prefix(restricted)
                    .is_some_and(|tail| tail.starts_with('.'))
        })
}

async fn persist_control_override(
    store: Option<&dyn OverridesStore>,
    path: Option<&Path>,
    key: &str,
    value: &serde_json::Value,
) {
    let result = if let Some(store) = store {
        persist_one(store, key, value).await
    } else if let Some(path) = path {
        FileOverridesStore::new(path).update(key, value).await
    } else {
        return;
    };
    if let Err(error) = result {
        tracing::warn!(%error, key, "OverridesStore persist failed");
    }
}

fn redact_config_value(path: &str, value: &mut serde_json::Value, redaction: &RedactionConfig) {
    if is_sensitive_key(path)
        || redaction.field_matches(path)
        || path.strip_prefix("apcore.").map_or_else(
            || redaction.field_matches(&format!("apcore.{path}")),
            |bare| redaction.field_matches(bare),
        )
        || path
            .split('.')
            .any(|segment| redaction.field_matches(segment))
    {
        *value = json!(DEFAULT_REPLACEMENT);
        return;
    }
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                redact_config_value(&format!("{path}.{key}"), value, redaction);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_config_value(path, item, redaction);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// UpdateConfigModule (F11) — runtime config mutation with optional persistence
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct ControlConfig {
    current: Arc<Mutex<Config>>,
    executor: Option<Arc<Mutex<Config>>>,
    // Serialize an update's persistence with reload's override read.
    operation: Arc<Mutex<()>>,
}

impl ControlConfig {
    pub(super) fn new(current: Arc<Mutex<Config>>, executor: Option<Arc<Mutex<Config>>>) -> Self {
        Self {
            current,
            executor,
            operation: Arc::new(Mutex::new(())),
        }
    }

    async fn lock(
        &self,
    ) -> (
        tokio::sync::MutexGuard<'_, Config>,
        Option<tokio::sync::MutexGuard<'_, Config>>,
    ) {
        let current = self.current.lock().await;
        let executor = match &self.executor {
            Some(config) => Some(config.lock().await),
            None => None,
        };
        (current, executor)
    }
}

// These sections are read through the Executor's live configuration snapshot.
fn is_executor_config_key(key: &str) -> bool {
    matches!(key.split('.').next(), Some("executor" | "obs" | "stream"))
}

fn invalid_config_value(
    key: &str,
    value: &serde_json::Value,
    reason: &str,
    redaction: &RedactionConfig,
) -> ModuleError {
    let mut safe_value = value.clone();
    redact_config_value(key, &mut safe_value, redaction);
    let message = format!("Invalid value for '{key}': {reason} (got {safe_value})");
    ModuleError::new(ErrorCode::ConfigInvalid, message).with_details(
        [
            ("key".to_owned(), json!(key)),
            ("value".to_owned(), safe_value),
        ]
        .into_iter()
        .collect(),
    )
}

fn value_applied(
    previous: Option<&serde_json::Value>,
    read_back: Option<&serde_json::Value>,
    requested: &serde_json::Value,
) -> bool {
    read_back.is_some_and(|current| {
        Some(current) != previous
            || current == requested
            || (current.is_f64() && current.as_f64() == requested.as_f64())
    })
}

fn refresh_snapshot(
    mut config: Config,
    require_source: bool,
    source_less_baseline: Option<&Config>,
    overrides: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<Config, ModuleError> {
    let from_disk = require_source || config.yaml_path.is_some();
    if from_disk {
        // Startup-owned system modules stay wired according to the active configuration.
        let enabled = config.get("sys_modules.enabled").unwrap_or(json!(false));
        config.reload_from_disk()?;
        config.set("sys_modules.enabled", enabled);
    } else if let Some(baseline) = source_less_baseline {
        // Replay persisted overrides from the supplied Executor's original
        // configuration so a removed override exposes its baseline again.
        let generation = config.generation;
        config = baseline.clone();
        config.generation = generation + 1;
    }
    for (key, value) in overrides {
        let config_key = key.strip_prefix("apcore.").unwrap_or(key);
        if !config_key.starts_with('_') && !is_restricted_config_key(config_key) {
            config.set(key, value.clone());
        }
    }
    if from_disk {
        config.validate()?;
    }
    Ok(config)
}

/// Update a runtime configuration value by dot-path key (F11).
pub struct UpdateConfigModule {
    config: ControlConfig,
    emitter: Arc<EventEmitter>,
    overrides_path: Option<PathBuf>,
    overrides_store: Option<Arc<dyn OverridesStore>>,
    audit_store: Option<Arc<dyn AuditStore>>,
}

impl UpdateConfigModule {
    pub fn new(config: Arc<Mutex<Config>>, emitter: Arc<EventEmitter>) -> Self {
        Self {
            config: ControlConfig::new(config, None),
            emitter,
            overrides_path: None,
            overrides_store: None,
            audit_store: None,
        }
    }

    pub(super) fn with_control_config(mut self, config: ControlConfig) -> Self {
        self.config = config;
        self
    }

    #[must_use]
    pub fn with_overrides_path(mut self, overrides_path: Option<PathBuf>) -> Self {
        self.overrides_path = overrides_path;
        self
    }

    /// Bind a pluggable [`OverridesStore`] for persistence.
    ///
    /// When set, takes precedence over `overrides_path`. The store is used to
    /// perform a read-modify-write of the supplied `key` after the in-memory
    /// `Config` has been mutated. Cross-language: matches the `OverridesStore`
    /// parameter accepted by `apcore-python` and `apcore-typescript`.
    #[must_use]
    pub fn with_overrides_store(
        mut self,
        overrides_store: Option<Arc<dyn OverridesStore>>,
    ) -> Self {
        self.overrides_store = overrides_store;
        self
    }

    #[must_use]
    pub fn with_audit_store(mut self, audit_store: Option<Arc<dyn AuditStore>>) -> Self {
        self.audit_store = audit_store;
        self
    }

    async fn update_config(
        &self,
        key: &str,
        effective_key: &str,
        value: &serde_json::Value,
    ) -> Result<(serde_json::Value, serde_json::Value), ModuleError> {
        let (mut current, mut executor) = self.config.lock().await;
        let redaction = RedactionConfig::from_config(&current);
        let invalid = |reason: &str| invalid_config_value(key, value, reason, &redaction);
        if Config::is_path_typed_key(effective_key) && value.as_str() == Some("") {
            return Err(invalid("path-typed values cannot be empty"));
        }
        if let Some(Err(reason)) = Config::validate_key_constraint(effective_key, value) {
            return Err(invalid(&reason));
        }

        let runtime_key = is_executor_config_key(effective_key);
        let stage_write = |active: &Config| {
            let before = active.get(effective_key);
            let mut staged = active.clone();
            staged.set(effective_key, value.clone());
            let after = staged.get(effective_key);
            if !value_applied(before.as_ref(), after.as_ref(), value) {
                return Err(invalid(
                    "the effective value would not change to the requested value",
                ));
            }
            Ok((staged, before, after))
        };
        let (staged, mut before, mut after) = stage_write(&current)?;
        if let Some(executor) = executor.as_mut() {
            let (staged_executor, executor_before, executor_after) = stage_write(executor)?;
            **executor = staged_executor;
            if runtime_key {
                before = executor_before;
                after = executor_after;
            }
        }
        *current = staged;

        let mut before = before.unwrap_or(serde_json::Value::Null);
        let mut after = after.unwrap_or(serde_json::Value::Null);
        redact_config_value(key, &mut before, &redaction);
        redact_config_value(key, &mut after, &redaction);
        Ok((before, after))
    }
}

#[async_trait]
impl Module for UpdateConfigModule {
    fn description(&self) -> &'static str {
        "Update a runtime configuration value by dot-path key"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["key", "value", "reason"],
            "properties": {
                "key":    {"type": "string"},
                "value":  {"x-sensitive": true},
                "reason": {"type": "string"}
            }
        })
    }

    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["success", "key", "old_value", "new_value"],
            "additionalProperties": false,
            "properties": {
                "success":   {"type": "boolean"},
                "key":       {"type": "string"},
                "old_value": {"description": "Previous value, null if absent, or redacted for sensitive keys"},
                "new_value": {}
            }
        })
    }

    async fn execute(
        &self,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        let key = require_string(&inputs, "key")?;
        let reason = require_string(&inputs, "reason")?;
        let value = inputs
            .get("value")
            .cloned()
            .ok_or_else(|| missing_field_error("value"))?;

        let config_key = key.strip_prefix("apcore.").unwrap_or(&key);
        if is_restricted_config_key(config_key) {
            // D-25: emit a distinct CONFIG_KEY_RESTRICTED code so callers can
            // match the policy-deny case independently of value-shape errors
            // (Python/TS already do this).
            return Err(ModuleError::new(
                ErrorCode::ConfigKeyRestricted,
                format!("Configuration key '{key}' cannot be changed at runtime"),
            )
            .with_details([("key".to_string(), json!(key))].into_iter().collect()));
        }

        let _operation = self.config.operation.lock().await;
        let effective_key = canonical_config_key(&key);
        let (redacted_old, redacted_new) = self.update_config(&key, effective_key, &value).await?;

        // Persist *after* the in-memory mutation succeeded so a write failure
        // cannot poison the runtime state. Errors are logged and not
        // propagated — overrides persistence is best-effort.
        // Pluggable store takes precedence over the legacy file path.
        persist_control_override(
            self.overrides_store.as_deref(),
            self.overrides_path.as_deref(),
            effective_key,
            &value,
        )
        .await;

        let timestamp = chrono::Utc::now().to_rfc3339();
        // Issue #45.2 — contextual auditing: augment payload with caller_id
        // (defaulted to "@external") and identity from the Context.
        let event_data = augment_with_context_identity(
            json!({
                "key": key,
                "old_value": redacted_old,
                "new_value": redacted_new,
                "reason": reason,
            }),
            ctx,
        );

        emit_event(
            &self.emitter,
            "apcore.config.updated",
            "system.control.update_config",
            &timestamp,
            event_data,
        )
        .await;

        tracing::info!(
            key = %key,
            old_value = ?redacted_old,
            new_value = ?redacted_new,
            reason = %reason,
            "Config updated"
        );

        let entry = build_audit_entry(
            AuditAction::UpdateConfig,
            "system.control.update_config",
            ctx,
            AuditChange {
                before: redacted_old.clone(),
                after: redacted_new.clone(),
            },
        );
        record_audit(self.audit_store.as_ref(), entry).await;

        Ok(json!({
            "success": true,
            "key": key,
            "old_value": redacted_old,
            "new_value": redacted_new,
        }))
    }
}

// ---------------------------------------------------------------------------
// ReloadModule (F10) — single + bulk path_filter reload
// ---------------------------------------------------------------------------

/// Hot-reload a module via safe unregister (F10).
///
/// Full re-discovery is not supported in Rust (no dynamic loading); the
/// module is unregistered and callers must re-register manually. The reload
/// event is always emitted with `new_version` == `previous_version`.
///
/// When `path_filter` is supplied instead of `module_id`, every module ID
/// matching the glob pattern is reloaded in dependency-topological order
/// (leaves first). Supplying both inputs raises `MODULE_RELOAD_CONFLICT`.
///
/// When the input contains `reload_config: true`, the bound [`Config`] (if
/// supplied via [`Self::with_config`]) is refreshed via
/// [`Config::reload_from_disk`] and an `apcore.config.reloaded` event is
/// emitted. Issue #45.5 — Rust cannot dynamically swap compiled module code
/// (`.so`/`.rlib`), but static configuration MUST be reloadable without a
/// binary restart.
pub struct ReloadModule {
    registry: Arc<Registry>,
    emitter: Arc<EventEmitter>,
    audit_store: Option<Arc<dyn AuditStore>>,
    config: Option<ControlConfig>,
    executor_baseline: Option<Arc<Config>>,
    overrides_path: Option<PathBuf>,
    overrides_store: Option<Arc<dyn OverridesStore>>,
    /// Once per INSTANCE, not per process (D-121). `reload` is called by
    /// hot-reload loops and watchers, so an advisory whose volume is
    /// proportional to traffic is one operators learn to filter out — the
    /// cadence reasoning D-89 settled. Per instance rather than per process for
    /// the reason D-90's notice is per token: a process-wide one-shot tells the
    /// first caller and leaves every later one to discover it in production.
    reload_dependents_warned: std::sync::atomic::AtomicBool,
}

impl ReloadModule {
    pub fn new(registry: Arc<Registry>, emitter: Arc<EventEmitter>) -> Self {
        Self {
            registry,
            emitter,
            audit_store: None,
            config: None,
            executor_baseline: None,
            overrides_path: None,
            overrides_store: None,
            reload_dependents_warned: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Warn once that `reload_dependents` is ignored and will be removed (D-121).
    ///
    /// The field was declared in all three SDKs' input schemas and read by
    /// none — a spec MUST that no implementation satisfied, which is the
    /// §9.1.3 "declared surface reaches no mechanism" shape applied to a module
    /// input field. Three independent implementations skipping it is the
    /// evidence the maintainer decision rests on.
    ///
    /// Deprecated rather than removed now, because the input schema sets
    /// `additionalProperties: false`: at 2.0 the same call stops being a silent
    /// no-op and becomes a validation error, so a caller passing it today needs
    /// a release in which they are told.
    fn warn_reload_dependents(&self, inputs: &serde_json::Value) {
        if inputs
            .get("reload_dependents")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            return;
        }
        if self
            .reload_dependents_warned
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        tracing::warn!(
            "system.control.reload: 'reload_dependents' is ignored and will be removed at 2.0 \
             (spec D-121). No SDK has ever implemented it. Use a 'path_filter' that also matches \
             the dependents; after removal this field becomes a validation error, not a no-op."
        );
    }

    /// Snapshot every matched module's instance, descriptor and version while
    /// they are still registered.
    ///
    /// D-112: the restore needs the previous INSTANCE, and `Registry::get`
    /// cannot answer once the module is unregistered. It hands back an `Arc`,
    /// which only `reinstate_internal` accepts. SYS-15's version capture has
    /// the same "while it is still there" constraint, so both happen here.
    fn capture_for_restore(
        &self,
        order: &[String],
        previous_versions: &mut std::collections::HashMap<String, String>,
    ) -> std::collections::HashMap<
        String,
        (Arc<dyn Module>, crate::registry::registry::ModuleDescriptor),
    > {
        let mut captured = std::collections::HashMap::new();
        for mid in order {
            if !self.registry.has(mid) {
                continue;
            }
            if let Ok(Some(descriptor)) = self.registry.get_definition(mid) {
                previous_versions.insert(mid.clone(), descriptor.version.clone());
                if let Ok(Some(module)) = self.registry.get(mid) {
                    captured.insert(mid.clone(), (module, descriptor));
                }
            }
        }
        captured
    }

    /// Restore each named module after a failed BULK reload (D-112 rule 4).
    ///
    /// Restoration is PER MODULE. The operation as a whole still fails (D-17),
    /// and modules that already reloaded successfully are NOT rolled back —
    /// cross-module transactionality is not a primitive the registry has, and
    /// claiming it would be the atomicity rule 1 forbids at batch scale.
    fn restore_each_after_failed_reload(
        &self,
        module_ids: &[String],
        previous: &std::collections::HashMap<
            String,
            (Arc<dyn Module>, crate::registry::registry::ModuleDescriptor),
        >,
    ) {
        for mid in module_ids {
            let Some((module, descriptor)) = previous.get(mid) else {
                continue;
            };
            self.restore_after_failed_reload(
                mid,
                Some(Arc::clone(module)),
                Some(descriptor.clone()),
            );
        }
    }

    /// Put the previous instance back after a reload failed (D-112).
    ///
    /// COMPENSATING, not transactional. The module is genuinely unregistered
    /// for a window and a concurrent call in that window sees
    /// `MODULE_NOT_FOUND`; this does not claim atomic replacement, and the spec
    /// forbids implementations from claiming it. What it does prevent is the
    /// worse outcome: a failed hot-fix making a WORKING module disappear.
    ///
    /// `reinstate_internal` re-runs the module's `on_load`, which rule 2
    /// requires — `on_unload` already ran during the unregister, so
    /// re-publishing without it yields a module that is visible but torn down,
    /// harder to diagnose than one that is absent.
    ///
    /// If the restoring load ALSO fails the module stays unavailable (rule 3).
    /// The original failure is the one the caller is told about, so this logs
    /// rather than returning an error.
    fn restore_after_failed_reload(
        &self,
        module_id: &str,
        previous_module: Option<Arc<dyn Module>>,
        previous_descriptor: Option<crate::registry::registry::ModuleDescriptor>,
    ) {
        let (Some(module), Some(descriptor)) = (previous_module, previous_descriptor) else {
            return;
        };
        if self.registry.has(module_id) {
            return;
        }
        if let Err(restore_err) = self
            .registry
            .reinstate_internal(module_id, module, descriptor)
        {
            tracing::error!(
                module_id = %module_id,
                error = %restore_err.message,
                "Restoring the module after a failed reload also failed; it is unavailable (D-112 rule 3)"
            );
        }
    }

    #[must_use]
    pub fn with_audit_store(mut self, audit_store: Option<Arc<dyn AuditStore>>) -> Self {
        self.audit_store = audit_store;
        self
    }

    /// Bind a runtime [`Config`] so `reload_config: true` invocations can
    /// refresh static configuration via [`Config::reload_from_disk`].
    /// Issue #45.5.
    #[must_use]
    pub fn with_config(mut self, config: Option<Arc<Mutex<Config>>>) -> Self {
        self.config = config.map(|config| ControlConfig::new(config, None));
        self
    }

    pub(super) fn with_control_config(
        mut self,
        config: ControlConfig,
        executor_baseline: Option<Arc<Config>>,
        overrides_path: Option<PathBuf>,
        overrides_store: Option<Arc<dyn OverridesStore>>,
    ) -> Self {
        self.config = Some(config);
        self.executor_baseline = executor_baseline;
        self.overrides_path = overrides_path;
        self.overrides_store = overrides_store;
        self
    }

    async fn refresh_config(&self, reason: &str) -> Result<(), ModuleError> {
        let binding = self.config.as_ref().ok_or_else(|| {
            ModuleError::new(
                ErrorCode::ReloadFailed,
                "No configuration is bound to reload_module",
            )
        })?;
        let _operation = binding.operation.lock().await;
        let overrides = if let Some(store) = &self.overrides_store {
            store.load().await
        } else if let Some(path) = &self.overrides_path {
            FileOverridesStore::new(path).load().await
        } else {
            Ok(std::collections::HashMap::new())
        }
        .map_err(|error| {
            ModuleError::new(
                ErrorCode::ReloadFailed,
                format!("Cannot reload configuration overrides: {error}"),
            )
        })?;
        let overrides = normalize_config_overrides(overrides);
        let executor_baseline = (self.overrides_path.is_some() || self.overrides_store.is_some())
            .then(|| self.executor_baseline.clone())
            .flatten();

        {
            let (mut config, mut executor) = binding.lock().await;
            let current = config.clone();
            let executor_current = executor.as_deref().cloned();
            let stage = move || {
                let config = refresh_snapshot(current, true, None, &overrides)?;
                let executor = executor_current
                    .map(|snapshot| {
                        refresh_snapshot(snapshot, false, executor_baseline.as_deref(), &overrides)
                    })
                    .transpose()?;
                Ok::<_, ModuleError>((config, executor))
            };
            let (staged, staged_executor) =
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn_blocking(stage).await.map_err(|error| {
                        ModuleError::new(
                            ErrorCode::ReloadFailed,
                            format!("Configuration reload task failed: {error}"),
                        )
                    })?
                } else {
                    stage()
                }?;
            *config = staged;
            if let (Some(active), Some(staged)) = (executor.as_mut(), staged_executor) {
                **active = staged;
            }
        }

        emit_event(
            &self.emitter,
            "apcore.config.reloaded",
            "system.control.reload_module",
            &chrono::Utc::now().to_rfc3339(),
            json!({"reason": reason}),
        )
        .await;
        tracing::info!(reason, "Config reloaded from disk");
        Ok(())
    }

    /// Topologically sort the matched module IDs (leaves first). Falls back
    /// to alphabetical order if the dependency graph contains a cycle or
    /// references a missing module — the reload still happens, just without
    /// the optimal order.
    fn topo_sort_modules(&self, matched: &[String]) -> Vec<String> {
        let matched_set: std::collections::HashSet<String> = matched.iter().cloned().collect();
        let entries: Vec<(String, Vec<DepInfo>)> = matched
            .iter()
            .map(|mid| {
                let deps: Vec<DepInfo> = self
                    .registry
                    .get_definition(mid)
                    .ok()
                    .flatten()
                    .map(|d| {
                        d.dependencies
                            .into_iter()
                            .filter(|dep| matched_set.contains(&dep.module_id))
                            .map(|dep| DepInfo {
                                module_id: dep.module_id,
                                version: if dep.version_constraint.is_empty() {
                                    None
                                } else {
                                    Some(dep.version_constraint)
                                },
                                optional: dep.optional,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (mid.clone(), deps)
            })
            .collect();

        match resolve_dependencies(&entries, Some(&matched_set), None) {
            Ok(order) => order,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Topological sort failed for path_filter reload; falling back to alphabetical"
                );
                let mut sorted = matched.to_vec();
                sorted.sort();
                sorted
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        clippy::single_match_else,
        clippy::map_unwrap_or
    )]
    async fn execute_single(
        &self,
        module_id: String,
        reason: &str,
        ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        // Sync SM-006: implement the 8-step reload pipeline aligned with
        // apcore-python (sys_modules/control.py:412-458) and apcore-typescript
        // (sys-modules/control.ts:186-254).
        //   1. capture_previous_version
        //   2. on_suspend (best-effort)
        //   3. safe_unregister
        //   4. registry.discover_internal() (re-discover modules)
        //   5. register_internal (no-op when discoverer reinstates)
        //   6. on_resume (best-effort)
        //   7. emit_reloaded event with actual previous + new versions
        //   8. log
        let start = std::time::Instant::now();

        if !self.registry.has(&module_id) {
            return Err(ModuleError::new(
                ErrorCode::ModuleNotFound,
                format!("Module '{module_id}' not found"),
            ));
        }

        // (1) Capture previous version from the registry descriptor.
        let previous_version = self
            .registry
            .get_definition(&module_id)
            .ok()
            .flatten()
            .map(|d| d.version)
            .unwrap_or_else(|| "unknown".to_string());

        // (2) on_suspend (best-effort) — capture state for handoff to on_resume.
        // Panics inside the user-supplied trait method are caught so a faulty
        // hook cannot abort the reload.
        // D-112: hold the previous instance so a failed reload can put it back.
        // `Registry::get` hands back an `Arc`, which no other registration entry
        // point accepts — `reinstate_internal` exists for exactly this.
        let previous_module = self.registry.get(&module_id).ok().flatten();
        let previous_descriptor = self.registry.get_definition(&module_id).ok().flatten();

        let suspended_state = match self.registry.get(&module_id) {
            Ok(Some(module)) => {
                let module_for_panic = Arc::clone(&module);
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    module_for_panic.on_suspend()
                })) {
                    Ok(state) => state,
                    Err(_) => {
                        tracing::warn!(
                            module_id = %module_id,
                            "Module on_suspend panicked; continuing reload"
                        );
                        None
                    }
                }
            }
            _ => None,
        };

        // (3) Safe unregister — drains in-flight calls.
        self.registry.safe_unregister(&module_id, 5000).await?;

        // (4) Re-run the configured discoverer to repopulate the registry.
        //
        // NOT best-effort: step 3 already unregistered the module, so swallowing a
        // discovery failure here reports `success: true` for a module that is now
        // gone. The spec Contract lists ReloadFailedError for exactly this —
        // "registry.discover() raised or module_id was absent after discovery" —
        // and apcore-python / apcore-typescript both raise it.
        if let Err(e) = self.registry.discover_internal().await {
            tracing::error!(
                module_id = %module_id,
                error = %e.message,
                "Reload: discover_internal failed after unregistering"
            );
            self.restore_after_failed_reload(
                &module_id,
                previous_module.clone(),
                previous_descriptor.clone(),
            );
            return Err(ModuleError::new(
                ErrorCode::ReloadFailed,
                format!(
                    "Reload of '{module_id}' unregistered the module but re-discovery \
                     failed: {}",
                    e.message
                ),
            ));
        }

        // (5) register_internal: in Rust we don't carry stand-alone factory
        // closures, so re-registration is delegated to the discoverer in step
        // 4. This branch is intentionally a no-op for cross-language parity.
        //
        // The presence check the delegation makes necessary: if the discoverer
        // ran without error but did not bring this module back, the reload has
        // still failed and must not report success.
        if !self.registry.has(&module_id) {
            tracing::error!(
                module_id = %module_id,
                "Reload: module absent after re-discovery"
            );
            self.restore_after_failed_reload(&module_id, previous_module, previous_descriptor);
            return Err(ModuleError::new(
                ErrorCode::ReloadFailed,
                format!(
                    "Reload of '{module_id}': module was not restored by re-discovery \
                     and is no longer registered"
                ),
            ));
        }

        // (6) on_resume (best-effort) — handoff state to the freshly loaded module.
        let new_version = self
            .registry
            .get_definition(&module_id)
            .ok()
            .flatten()
            .map(|d| d.version)
            .unwrap_or_else(|| previous_version.clone());

        if let Some(state) = suspended_state {
            if let Ok(Some(module)) = self.registry.get(&module_id) {
                let module_for_panic = Arc::clone(&module);
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    module_for_panic.on_resume(state);
                }))
                .is_err()
                {
                    tracing::warn!(
                        module_id = %module_id,
                        "Module on_resume panicked; reload still considered successful"
                    );
                }
            }
        }

        // (7) Emit the reloaded event with actual versions.
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let timestamp = chrono::Utc::now().to_rfc3339();
        // Issue #45.2 — augment with caller_id / identity from the Context.
        // Include `module_id` in the data payload for cross-language parity
        // (Python/TS both inline it so subscribers reading only `data` see
        // the affected target).
        let event_data = augment_with_context_identity(
            json!({
                "module_id": module_id,
                "previous_version": previous_version,
                "new_version": new_version,
                "reason": reason,
            }),
            ctx,
        );
        emit_event(
            &self.emitter,
            "apcore.module.reloaded",
            &module_id,
            &timestamp,
            event_data,
        )
        .await;

        // (8) Structured log.
        tracing::info!(
            module_id = %module_id,
            previous_version = %previous_version,
            new_version = %new_version,
            reason = %reason,
            "Module reloaded"
        );

        let entry = build_audit_entry(
            AuditAction::ReloadModule,
            &module_id,
            ctx,
            AuditChange {
                before: json!(previous_version),
                after: json!(new_version),
            },
        );
        record_audit(self.audit_store.as_ref(), entry).await;

        Ok(json!({
            "success": true,
            "module_id": module_id,
            "previous_version": previous_version,
            "new_version": new_version,
            "reload_duration_ms": elapsed_ms,
        }))
    }

    /// Partition freshly re-discovered ids into those that came back (emitting
    /// `apcore.module.reloaded` for each) and those that did not.
    ///
    /// Split out of `execute_bulk` to keep it under the function-length lint.
    async fn verify_and_emit_reloaded(
        &self,
        unregistered: Vec<String>,
        previous_versions: &std::collections::HashMap<String, String>,
        reason: &str,
        ctx: &Context<serde_json::Value>,
    ) -> (Vec<String>, Vec<String>) {
        let mut reloaded: Vec<String> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        for mid in unregistered {
            if !self.registry.has(&mid) {
                missing.push(mid);
                continue;
            }
            let timestamp = chrono::Utc::now().to_rfc3339();
            // SYS-15: the real versions, as the single-module path already
            // emits. Both fields were the literal string "unknown" here, so a
            // subscriber watching `apcore.module.reloaded` could not tell a
            // version bump from a no-op reload on the bulk path — while the
            // same event carried real versions when one module was reloaded by
            // ID. `previous_versions` is captured BEFORE the unregister, which
            // is the only moment the old version is still readable.
            let previous_version = previous_versions
                .get(&mid)
                .cloned()
                .unwrap_or_else(|| DEFAULT_MODULE_VERSION.to_string());
            let new_version = self
                .registry
                .get_definition(&mid)
                .ok()
                .flatten()
                .map_or_else(|| previous_version.clone(), |d| d.version);
            let event_data = augment_with_context_identity(
                json!({
                    "previous_version": previous_version,
                    "new_version": new_version,
                    "reason": reason,
                }),
                ctx,
            );
            emit_event(
                &self.emitter,
                "apcore.module.reloaded",
                &mid,
                &timestamp,
                event_data,
            )
            .await;
            reloaded.push(mid);
        }
        (reloaded, missing)
    }

    async fn execute_bulk(
        &self,
        path_filter: String,
        reason: &str,
        ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        // PROTOCOL_SPEC 6.7 clause 4 / 9.2.3: `path_filter` is matched with
        // Algorithm A25 against each registered module ID. `glob::Pattern`
        // was the obvious local answer and the wrong one twice over: it
        // expands `[em]` as a character class, and it REJECTS patterns
        // (`a[b`, `a**b`), so a control-plane request the other two SDKs
        // served was refused here (#117). A25 never fails to parse.
        let mut matched: Vec<String> = self
            .registry
            .module_ids()
            .into_iter()
            .filter(|id| match_glob(&path_filter, id))
            .collect();
        matched.sort();

        let order = self.topo_sort_modules(&matched);
        let start = std::time::Instant::now();

        // Phase 1 — unregister every match, recording what we actually took out.
        // Nothing may be reported as reloaded on the strength of this phase alone:
        // an unregister that is never followed by a successful re-discovery is a
        // deletion, and this method previously returned `success: true` for exactly
        // that (it pushed each id here and never re-discovered at all).
        let mut unregistered: Vec<String> = Vec::new();
        // SYS-15: capture each module's version while it is still registered.
        let mut previous_versions: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let previous_modules = self.capture_for_restore(&order, &mut previous_versions);
        for mid in order {
            if !self.registry.has(&mid) {
                continue;
            }
            match self.registry.safe_unregister(&mid, 5000).await {
                Ok(_) => unregistered.push(mid),
                Err(e) => {
                    tracing::error!(error = %e, module_id = %mid, "Bulk reload: failed to unregister");
                }
            }
        }

        // Phase 2 — one re-discovery for the whole batch, mirroring `execute_single`
        // step 4 and apcore-typescript's single `registry.discover()`.
        //
        // Unlike `execute_single`, a discovery error here is FATAL rather than
        // best-effort: the batch is already unregistered, so continuing would
        // silently delete every matched module and report success.
        if let Err(e) = self.registry.discover_internal().await {
            tracing::error!(
                error = %e.message,
                path_filter = %path_filter,
                count = unregistered.len(),
                "Bulk reload: re-discovery failed after unregistering"
            );
            self.restore_each_after_failed_reload(&unregistered, &previous_modules);
            return Err(ModuleError::new(
                ErrorCode::ReloadFailed,
                format!(
                    "Bulk reload of '{path_filter}' unregistered {} module(s) but re-discovery \
                     failed: {}. Affected: {}",
                    unregistered.len(),
                    e.message,
                    unregistered.join(", ")
                ),
            ));
        }

        // Phase 3 — a module counts as reloaded only if it is actually back.
        let (reloaded, missing) = self
            .verify_and_emit_reloaded(unregistered, &previous_versions, reason, ctx)
            .await;

        if !missing.is_empty() {
            tracing::error!(
                path_filter = %path_filter,
                missing = %missing.join(", "),
                "Bulk reload: modules absent after re-discovery"
            );
            self.restore_each_after_failed_reload(&missing, &previous_modules);
            return Err(ModuleError::new(
                ErrorCode::ReloadFailed,
                format!(
                    "Bulk reload of '{path_filter}': {} module(s) were not restored by \
                     re-discovery and are no longer registered: {}",
                    missing.len(),
                    missing.join(", ")
                ),
            ));
        }

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        tracing::info!(
            count = reloaded.len(),
            path_filter = %path_filter,
            reason = %reason,
            "Bulk module reload"
        );

        // D-111: one entry PER MODULE, not one keyed on the glob.
        //
        // `AuditStore::query(module_id)` filters on a concrete id, so an entry
        // whose `target_module_id` was `executor.*` is unfindable by the
        // accessor the store exists for — the operation was audited and could
        // not be looked up.
        //
        // Per-module entries alone lose the fact that they were ONE deploy, so
        // they share a correlation id and "what did this deploy touch" stays a
        // single query. Generated once, here, rather than per entry.
        let correlation_id = uuid::Uuid::new_v4().to_string();
        for mid in &reloaded {
            let entry = crate::sys_modules::audit::build_correlated_audit_entry(
                AuditAction::ReloadModule,
                mid,
                ctx,
                AuditChange {
                    before: serde_json::Value::Null,
                    after: json!(mid),
                },
                correlation_id.clone(),
            );
            record_audit(self.audit_store.as_ref(), entry).await;
        }

        Ok(json!({
            "success": true,
            "module_id": serde_json::Value::Null,
            "reloaded_modules": reloaded,
            "reload_duration_ms": elapsed_ms,
        }))
    }
}

#[async_trait]
impl Module for ReloadModule {
    fn description(&self) -> &'static str {
        "Hot-reload a module by safe unregister (re-registration must be done explicitly in Rust)"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["reason"],
            "properties": {
                "module_id":         {"type": "string"},
                "path_filter":       {"type": "string"},
                "reload_dependents": {
                    "type": "boolean",
                    "default": false,
                    "deprecated": true,
                    "description": "DEPRECATED (spec v1.51.0, D-121) — ignored, and removed at 2.0. \
        Declared in all three SDKs and implemented by none, so no caller could ever rely on it. Use an \
        explicit path_filter that also matches the dependents. At 2.0 this field is REMOVED and, because \
        this schema sets additionalProperties: false, passing it becomes a validation error rather than a \
        silent no-op."
                },
                "reload_config":     {"type": "boolean", "default": false},
                "reason":            {"type": "string"}
            }
        })
    }

    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["success", "module_id"],
            "additionalProperties": false,
            "properties": {
                "success":            {"type": "boolean"},
                "module_id":          {"type": ["string", "null"]},
                "previous_version":   {"type": ["string", "null"]},
                "new_version":        {"type": ["string", "null"]},
                "reload_duration_ms": {"type": "number"},
                "reloaded_modules":   {"type": "array", "items": {"type": "string"}},
                "config_reloaded":    {"type": "boolean"}
            },
            "oneOf": [
                {"properties": {"module_id": {"type": "string"}, "reloaded_modules": false}},
                {
                    "required": ["reloaded_modules"],
                    "properties": {
                        "module_id": {"type": "null"},
                        "reloaded_modules": {"type": "array"},
                        "previous_version": false,
                        "new_version": false
                    }
                },
                {
                    "required": ["config_reloaded"],
                    "properties": {
                        "module_id": {"type": "null"},
                        "config_reloaded": {"type": "boolean"},
                        "reloaded_modules": false,
                        "previous_version": false,
                        "new_version": false,
                        "reload_duration_ms": false
                    }
                }
            ]
        })
    }

    async fn execute(
        &self,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        self.warn_reload_dependents(&inputs);
        let reason = require_string(&inputs, "reason")?;

        let module_id_input = inputs
            .get("module_id")
            .filter(|v| !v.is_null())
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let path_filter_input = inputs
            .get("path_filter")
            .filter(|v| !v.is_null())
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        if module_id_input.is_some() && path_filter_input.is_some() {
            return Err(ModuleError::new(
                ErrorCode::ModuleReloadConflict,
                "'module_id' and 'path_filter' are mutually exclusive",
            ));
        }

        let reload_config_flag = inputs
            .get("reload_config")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        // Issue #45.5: when reload_config is set, refresh static configuration
        // from disk. This runs BEFORE module reload so any module that re-reads
        // config during its on_resume sees the fresh values.
        let config_reloaded = if reload_config_flag {
            match self.refresh_config(&reason).await {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "Configuration reload failed");
                    false
                }
            }
        } else {
            false
        };

        if let Some(filter) = path_filter_input {
            let mut result = self.execute_bulk(filter.to_string(), &reason, ctx).await?;
            if let Some(obj) = result.as_object_mut() {
                obj.insert("config_reloaded".to_string(), json!(config_reloaded));
            }
            return Ok(result);
        }

        // A configuration-only request reports just the outcome of that reload.
        if module_id_input.is_none() && reload_config_flag {
            return Ok(json!({
                "success": config_reloaded,
                "module_id": serde_json::Value::Null,
                "config_reloaded": config_reloaded,
            }));
        }

        let module_id = module_id_input.ok_or_else(|| {
            ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                "'module_id', 'path_filter', or 'reload_config' is required",
            )
        })?;

        let mut result = self
            .execute_single(module_id.to_string(), &reason, ctx)
            .await?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("config_reloaded".to_string(), json!(config_reloaded));
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// ToggleFeatureModule (F19) — runtime enable/disable with optional persistence
// ---------------------------------------------------------------------------

/// Disable or enable a module without unloading it from the Registry (F19).
pub struct ToggleFeatureModule {
    registry: Arc<Registry>,
    emitter: Arc<EventEmitter>,
    toggle_state: Arc<ToggleState>,
    overrides_path: Option<PathBuf>,
    overrides_store: Option<Arc<dyn OverridesStore>>,
    audit_store: Option<Arc<dyn AuditStore>>,
}

impl ToggleFeatureModule {
    pub fn new(
        registry: Arc<Registry>,
        emitter: Arc<EventEmitter>,
        toggle_state: Arc<ToggleState>,
    ) -> Self {
        Self {
            registry,
            emitter,
            toggle_state,
            overrides_path: None,
            overrides_store: None,
            audit_store: None,
        }
    }

    #[must_use]
    pub fn with_overrides_path(mut self, overrides_path: Option<PathBuf>) -> Self {
        self.overrides_path = overrides_path;
        self
    }

    /// Bind a pluggable [`OverridesStore`] for persistence. Takes precedence
    /// over `overrides_path` when both are set.
    #[must_use]
    pub fn with_overrides_store(
        mut self,
        overrides_store: Option<Arc<dyn OverridesStore>>,
    ) -> Self {
        self.overrides_store = overrides_store;
        self
    }

    #[must_use]
    pub fn with_audit_store(mut self, audit_store: Option<Arc<dyn AuditStore>>) -> Self {
        self.audit_store = audit_store;
        self
    }
}

#[async_trait]
impl Module for ToggleFeatureModule {
    fn description(&self) -> &'static str {
        "Disable or enable a module without unloading it"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["module_id", "enabled", "reason"],
            "properties": {
                "module_id": {"type": "string"},
                "enabled":   {"type": "boolean"},
                "reason":    {"type": "string"}
            }
        })
    }

    fn output_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "success":   {"type": "boolean"},
                "module_id": {"type": "string"},
                "enabled":   {"type": "boolean"}
            }
        })
    }

    async fn execute(
        &self,
        inputs: serde_json::Value,
        ctx: &Context<serde_json::Value>,
    ) -> Result<serde_json::Value, ModuleError> {
        let module_id = require_string(&inputs, "module_id")?;
        let reason = require_string(&inputs, "reason")?;
        let enabled = inputs
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                ModuleError::new(
                    ErrorCode::GeneralInvalidInput,
                    "'enabled' is required and must be a boolean",
                )
            })?;

        if !self.registry.has(&module_id) {
            return Err(ModuleError::new(
                ErrorCode::ModuleNotFound,
                format!("Module '{module_id}' not found"),
            ));
        }

        let before_enabled = !self.toggle_state.is_disabled(&module_id);

        // Mutate ONLY the ToggleState — never the registry descriptor's
        // `enabled` flag. A reload re-registers descriptors (resetting their
        // `enabled` flag), so toggling the descriptor would not survive a
        // reload, violating the spec's "toggle survives reload" guarantee.
        // The authoritative disabled-set lives in ToggleState, which is
        // independent of registry re-registration. Cross-language parity with
        // apcore-python control.py and apcore-typescript toggle.ts, which
        // mutate only the toggle store (sync finding A-D-12).
        if enabled {
            self.toggle_state.enable(&module_id);
        } else {
            self.toggle_state.disable(&module_id);
        }

        let toggle_key = format!("{TOGGLE_OVERRIDE_PREFIX}{module_id}");
        let toggle_value = serde_json::Value::Bool(enabled);
        persist_control_override(
            self.overrides_store.as_deref(),
            self.overrides_path.as_deref(),
            &toggle_key,
            &toggle_value,
        )
        .await;

        let timestamp = chrono::Utc::now().to_rfc3339();
        // Issue #45.2 — augment with caller_id / identity from the Context.
        // Cross-language parity: Python/TS include `module_id` directly in the
        // event data payload (alongside the outer ApCoreEvent.module_id field)
        // so subscribers that only look at `data` still see the affected target.
        let event_data = augment_with_context_identity(
            json!({
                "module_id": module_id,
                "enabled": enabled,
                "reason": reason,
            }),
            ctx,
        );
        emit_event(
            &self.emitter,
            "apcore.module.toggled",
            &module_id,
            &timestamp,
            event_data,
        )
        .await;

        tracing::info!(
            module_id = %module_id,
            enabled = %enabled,
            reason = %reason,
            "Module toggled"
        );

        let entry = build_audit_entry(
            AuditAction::ToggleFeature,
            &module_id,
            ctx,
            AuditChange {
                before: json!(before_enabled),
                after: json!(enabled),
            },
        );
        record_audit(self.audit_store.as_ref(), entry).await;

        Ok(json!({
            "success": true,
            "module_id": module_id,
            "enabled": enabled,
        }))
    }
}
