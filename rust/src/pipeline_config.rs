// APCore Protocol — Pipeline YAML configuration: step type registry and strategy builder.
// Spec reference: docs/features/execution-pipeline.md (Configuring the pipeline from apcore.yaml)

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::builtin_steps::build_standard_strategy;
use crate::errors::{ErrorCode, ModuleError};
use crate::pipeline::{ExecutionStrategy, Step};

// ---------------------------------------------------------------------------
// Step factory type
// ---------------------------------------------------------------------------

/// Factory function that creates a `Step` from a config dict.
pub(crate) type StepFactory =
    Box<dyn Fn(&Value) -> Result<Box<dyn Step>, ModuleError> + Send + Sync>;

// ---------------------------------------------------------------------------
// Global step type registry (OnceLock + RwLock pattern)
// ---------------------------------------------------------------------------

fn global_step_factories() -> &'static RwLock<HashMap<String, StepFactory>> {
    static FACTORIES: OnceLock<RwLock<HashMap<String, StepFactory>>> = OnceLock::new();
    FACTORIES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register a step type for YAML pipeline configuration.
///
/// The `name` is the type name referenced in YAML `type` fields.
/// The `factory` is a callable that receives `&Value` config and returns a `Box<dyn Step>`.
///
/// Returns an error if the name is empty, contains whitespace, or is already registered.
pub fn register_step_type(name: &str, factory: StepFactory) -> Result<(), ModuleError> {
    if name.is_empty() || name.contains(' ') {
        return Err(ModuleError::new(
            ErrorCode::GeneralInvalidInput,
            format!("Invalid step type name: '{name}'"),
        ));
    }
    let mut map = global_step_factories().write();
    if map.contains_key(name) {
        return Err(ModuleError::new(
            ErrorCode::GeneralInvalidInput,
            format!("Step type '{name}' is already registered"),
        ));
    }
    map.insert(name.to_string(), factory);
    Ok(())
}

/// Remove a registered step type. Returns `true` if found and removed.
#[must_use]
pub fn unregister_step_type(name: &str) -> bool {
    let mut map = global_step_factories().write();
    map.remove(name).is_some()
}

/// Return a list of all registered step type names.
#[must_use]
pub fn registered_step_types() -> Vec<String> {
    let map = global_step_factories().read();
    map.keys().cloned().collect()
}

/// Clear the step type registry (for testing only).
#[cfg(test)]
pub(crate) fn reset_step_registry() {
    let mut map = global_step_factories().write();
    map.clear();
}

// ---------------------------------------------------------------------------
// Step resolution from config dict (protocol-spec §5.16.1)
// ---------------------------------------------------------------------------

/// Wrapper that overlays YAML metadata fields onto a factory-created step.
struct ConfiguredStep {
    inner: Box<dyn Step>,
    name_override: Option<String>,
    match_modules: Option<Vec<String>>,
    ignore_errors: bool,
    pure: bool,
    timeout_ms: u64,
}

#[async_trait::async_trait]
impl Step for ConfiguredStep {
    fn name(&self) -> &str {
        self.name_override
            .as_deref()
            .unwrap_or_else(|| self.inner.name())
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn removable(&self) -> bool {
        self.inner.removable()
    }
    fn replaceable(&self) -> bool {
        self.inner.replaceable()
    }
    fn match_modules(&self) -> Option<&[String]> {
        self.match_modules
            .as_deref()
            .or_else(|| self.inner.match_modules())
    }
    fn ignore_errors(&self) -> bool {
        self.ignore_errors
    }
    fn pure(&self) -> bool {
        self.pure
    }
    fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }
    // The overlay changes the four configurable fields and nothing else. The
    // gate identity and the capability contract belong to the implementation
    // (PROTOCOL_SPEC 6.6.5.2, §5.16.1): dropping `builtin_gate` made a gate
    // configured with nothing but `timeout_ms` read as unwired in
    // `governance_state()`, and dropping `requires` / `provides` exempted a
    // configured step from the `PIPELINE_DEPENDENCY_ERROR` check.
    fn builtin_gate(&self) -> Option<crate::pipeline::BuiltinGate> {
        self.inner.builtin_gate()
    }
    fn requires(&self) -> &[&str] {
        self.inner.requires()
    }
    fn provides(&self) -> &[&str] {
        self.inner.provides()
    }
    async fn execute(
        &self,
        ctx: &mut crate::pipeline::PipelineContext,
    ) -> Result<crate::pipeline::StepResult, ModuleError> {
        self.inner.execute(ctx).await
    }
}

/// Resolve a single step definition dict into a `Box<dyn Step>`.
///
/// The four fields `pipeline.configure` may override, per
/// `schemas/apcore-config.schema.json` `$defs/ConfigurableStepFields` and
/// protocol-spec §5.16.1 — the step-entry fields that still
/// mean something applied to a step that already exists.
///
/// `requires` / `provides` are deliberately absent: a step's capability
/// contract is declared by its implementation. Configuration able to rewrite it
/// disables the `PipelineDependencyError` rule that
/// `features/middleware-system.md` § Configuration safety states as a MUST —
/// measured on apcore-python and apcore-typescript, the spec's own former
/// example moved `input_validation` from `requires=["module"]` to
/// `requires=["context"]`, deleting the dependency `module_lookup` satisfies.
/// This SDK never applied them, so it was never affected; it dropped them with
/// a warning instead, which is the defect fixed below.
const CONFIGURABLE_STEP_FIELDS: &[&str] = &["match_modules", "ignore_errors", "pure", "timeout_ms"];

/// PROTOCOL_SPEC §5.16.1 "Governance gates cannot be weakened by `configure`"
/// (D-130), applied to the raw `pipeline.configure` entry for `step_name`.
///
/// `gate` is the target step's [`Step::builtin_gate`] — gates are located by
/// type, like `governance_state()` does, not by name. On either gate,
/// `ignore_errors: true` and any non-null `match_modules` (an empty list
/// included: it matches nothing, so it exempts everything) are refused; on the
/// approval gate `pure: true` is refused too. The ACL gate is pure by default
/// (`validate()` runs it), so both `pure` values are accepted there. Every
/// offending key is named. Writing a field's default (`false`, `null`) is
/// accepted, and `timeout_ms` stays configurable.
fn reject_gate_weakening_keys(
    step_name: &str,
    gate: Option<crate::pipeline::BuiltinGate>,
    fields: &serde_json::Map<String, Value>,
) -> Result<(), ModuleError> {
    let Some(gate) = gate else {
        return Ok(());
    };
    let mut offending: Vec<&str> = Vec::new();
    if fields.get("ignore_errors") == Some(&Value::Bool(true)) {
        offending.push("ignore_errors: true");
    }
    if fields.get("match_modules").is_some_and(|v| !v.is_null()) {
        offending.push("match_modules");
    }
    if gate == crate::pipeline::BuiltinGate::Approval
        && fields.get("pure") == Some(&Value::Bool(true))
    {
        offending.push("pure: true");
    }
    if offending.is_empty() {
        return Ok(());
    }
    let mut err = crate::pipeline::gate_weakening_error(step_name, &offending);
    err.message = format!("pipeline.configure: {}", err.message);
    Err(err)
}

/// The ten keys a `pipeline.steps` entry may carry, per `$defs/PipelineStep`
/// (`additionalProperties: false`) and §4.3.
///
/// That definition has been closed since it was written and nothing enforced
/// it. Measured on this SDK before the fix: an entry
/// `{"name": "x", "type": "probe_noop", "after": "execute", "tiemout_ms": 5000}`
/// built successfully with `timeout_ms == 0` — the operator's five-second
/// timeout silently absent. Same failure mode as the `configure:` warning, one
/// key over.
const STEP_ENTRY_FIELDS: &[&str] = &[
    "name",
    "type",
    "handler",
    "config",
    "match_modules",
    "ignore_errors",
    "pure",
    "timeout_ms",
    "after",
    "before",
];

/// Reject any key of `entry` outside `allowed`, naming the offender and the
/// legal set. Runs before anything is constructed, so a typo is a start-up
/// error rather than a field that quietly never took effect.
fn reject_unknown_keys(
    entry: &serde_json::Map<String, Value>,
    allowed: &[&str],
    context: &str,
) -> Result<(), ModuleError> {
    // EVERY offending key, not the first. Two reasons, and the second is why
    // this is not merely nicer:
    //
    //   * One restart shows the whole problem instead of one restart per typo.
    //     apcore-python set this precedent for the `_config.strict` framework-key
    //     check.
    //   * Reporting only the first makes the message depend on map iteration
    //     order, which differs by language — `serde_json::Map` is a BTreeMap here
    //     (sorted) while Python dicts and JS objects preserve insertion order. A
    //     conformance assertion naming one key would then pass on two SDKs and
    //     fail on this one for a reason that is not a behaviour difference.
    let unknown: Vec<&str> = entry
        .keys()
        .map(String::as_str)
        .filter(|k| !allowed.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let plural = if unknown.len() == 1 {
        "field"
    } else {
        "fields"
    };
    Err(ModuleError::new(
        ErrorCode::PipelineConfigurationError,
        format!(
            "{context}: {} {plural} not valid here: {}. Accepted: {}.",
            unknown.len(),
            unknown
                .iter()
                .map(|k| format!("'{k}'"))
                .collect::<Vec<_>>()
                .join(", "),
            allowed.join(", ")
        ),
    ))
}

/// Per protocol-spec §5.16.1:
///   - `type:` → registry lookup (only supported mode in Rust)
///   - `handler:` → parse-time error (Rust cannot dynamically load modules)
///   - Metadata: `match_modules`, `ignore_errors`, `pure`, `timeout_ms` applied via wrapper.
fn resolve_step(step_def: &Value) -> Result<Box<dyn Step>, ModuleError> {
    let step_name = step_def.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(entry) = step_def.as_object() {
        let label = if step_name.is_empty() {
            "pipeline.steps entry".to_string()
        } else {
            format!("pipeline.steps entry '{step_name}'")
        };
        reject_unknown_keys(entry, STEP_ENTRY_FIELDS, &label)?;
    }
    let type_name = step_def.get("type").and_then(|v| v.as_str());
    let handler_path = step_def.get("handler").and_then(|v| v.as_str());
    let config = step_def
        .get("config")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    // handler: is not supported in Rust (compiled language, no runtime import).
    if let Some(hp) = handler_path {
        return Err(ModuleError::new(
            ErrorCode::PipelineHandlerNotSupported,
            format!(
                "pipeline step '{step_name}' uses 'handler: {hp}' which is not supported in apcore-rust. \
                 Use 'type:' with register_step_type(). \
                 See protocol-spec §5.16.1",
            ),
        ));
    }

    let type_name = type_name.ok_or_else(|| {
        ModuleError::new(
            ErrorCode::GeneralInvalidInput,
            format!("Step '{step_name}' has neither 'type' nor 'handler'"),
        )
    })?;

    let map = global_step_factories().read();
    let factory = map.get(type_name).ok_or_else(|| {
        ModuleError::new(
            ErrorCode::GeneralInvalidInput,
            format!(
                "Step type '{type_name}' not registered. \
                 Register with: register_step_type(\"{type_name}\", factory)"
            ),
        )
    })?;

    let inner = factory(&config)?;

    // Parse metadata from YAML
    let match_modules = step_def
        .get("match_modules")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });
    let ignore_errors = step_def
        .get("ignore_errors")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let pure = step_def
        .get("pure")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let timeout_ms = step_def
        .get("timeout_ms")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    let name_override = step_def
        .get("name")
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(Box::new(ConfiguredStep {
        inner,
        name_override,
        match_modules,
        ignore_errors,
        pure,
        timeout_ms,
    }))
}

// ---------------------------------------------------------------------------
// Build strategy from YAML config
// ---------------------------------------------------------------------------

/// Apply the `validation.pipeline.*` limits (PROTOCOL_SPEC §9.1.2).
///
/// `remove:` is deliberately not name-checked: those names identify steps that
/// already exist rather than naming new ones, so a limit there would reject a
/// request to remove a step the operator did not author.
fn validate_pipeline_limits(
    pipeline_config: &Value,
    config: &crate::config::Config,
) -> Result<(), ModuleError> {
    let limit = |key: &str| -> Option<u64> { config.get(key).and_then(|v| v.as_u64()) };
    let max_name = limit("validation.pipeline.step_name_max_length");
    let max_timeout = limit("validation.pipeline.timeout_ms_max");
    if max_name.is_none() && max_timeout.is_none() {
        return Ok(());
    }

    let check_timeout = |where_: &str, value: Option<&Value>| -> Result<(), ModuleError> {
        if let (Some(max), Some(v)) = (max_timeout, value.and_then(serde_json::Value::as_u64)) {
            if v > max {
                return Err(ModuleError::new(
                    ErrorCode::ConfigInvalid,
                    format!(
                        "{where_}: timeout_ms {v} is over the {max} configured by \
                         validation.pipeline.timeout_ms_max"
                    ),
                ));
            }
        }
        Ok(())
    };

    if let Some(Value::Object(configure)) = pipeline_config.get("configure") {
        for (step_name, overrides) in configure {
            check_timeout(&format!("Step '{step_name}'"), overrides.get("timeout_ms"))?;
        }
    }

    if let Some(steps) = pipeline_config.get("steps").and_then(|v| v.as_array()) {
        for step in steps {
            let name = step.get("name").and_then(serde_json::Value::as_str);
            if let (Some(max), Some(name)) = (max_name, name) {
                // §9.1.2 requirement 4: characters, not bytes.
                let len = name.chars().count() as u64;
                if len > max {
                    return Err(ModuleError::new(
                        ErrorCode::ConfigInvalid,
                        format!(
                            "Step '{name}': name is {len} characters, over the {max} configured \
                             by validation.pipeline.step_name_max_length"
                        ),
                    ));
                }
            }
            check_timeout(
                &format!("Step '{}'", name.unwrap_or("<unnamed>")),
                step.get("timeout_ms"),
            )?;
        }
    }
    Ok(())
}

/// Build an `ExecutionStrategy` from a YAML pipeline configuration section.
///
/// Starts with `build_standard_strategy()`, then applies:
///   1. `remove` -- remove named steps
///   2. `steps` -- resolve and insert custom steps (via `after` or `before` anchors)
///
/// # Example config shape (as JSON)
///
/// ```json
/// {
///   "remove": ["acl_check", "approval_gate"],
///   "steps": [
///     {
///       "name": "rate_limit",
///       "type": "rate_limiter",
///       "after": "call_chain_guard",
///       "config": { "max_rps": 100 }
///     }
///   ]
/// }
/// ```
pub fn build_strategy_from_config(
    pipeline_config: &Value,
) -> Result<ExecutionStrategy, ModuleError> {
    build_strategy_from_config_with_limits(pipeline_config, None)
}

/// [`build_strategy_from_config`] with the PROTOCOL_SPEC §9.1.2
/// `validation.pipeline.*` limits applied.
///
/// **Both limits are unconstrained by default**, so passing `None` — which is
/// what the entry point above does, and what every caller before v1.38.0 did —
/// checks nothing and a pipeline that parsed before still parses. apcore does
/// not impose limits on the content its users author; it offers them.
pub fn build_strategy_from_config_with_limits(
    pipeline_config: &Value,
    config: Option<&crate::config::Config>,
) -> Result<ExecutionStrategy, ModuleError> {
    build_strategy_from_seed(build_standard_strategy(), pipeline_config, config)
}

/// [`build_strategy_from_config_with_limits`] seeded with a standard strategy
/// bound to a specific [`ToggleState`](crate::sys_modules::ToggleState).
///
/// `Executor` uses this one. Seeding from the plain
/// [`build_standard_strategy`] would bind the *process-global* toggle store to
/// the `module_lookup` step, so `apcore.disable(module)` on one instance would
/// leak into another — the regression issue #71 fixed for the five presets, and
/// which applying a `pipeline:` section would otherwise reintroduce through a
/// different door.
pub fn build_strategy_from_config_with_toggle(
    pipeline_config: &Value,
    config: Option<&crate::config::Config>,
    toggle_state: std::sync::Arc<crate::sys_modules::ToggleState>,
) -> Result<ExecutionStrategy, ModuleError> {
    build_strategy_from_seed(
        crate::builtin_steps::build_standard_strategy_with_toggle(toggle_state),
        pipeline_config,
        config,
    )
}

#[allow(clippy::too_many_lines)] // declarative-config dispatcher; splitting hurts readability more than length helps
fn build_strategy_from_seed(
    mut strategy: ExecutionStrategy,
    pipeline_config: &Value,
    config: Option<&crate::config::Config>,
) -> Result<ExecutionStrategy, ModuleError> {
    if let Some(cfg) = config {
        validate_pipeline_limits(pipeline_config, cfg)?;
    }

    // (1) Remove steps — Issue #33 §1.2: fail-fast when YAML refers to a
    // nonexistent step rather than emitting a tracing::warn! and proceeding.
    if let Some(remove_list) = pipeline_config.get("remove").and_then(|v| v.as_array()) {
        for entry in remove_list {
            if let Some(step_name) = entry.as_str() {
                strategy.remove(step_name).map_err(|e| {
                    // Sync alignment W-7: missing-step is a structural
                    // configuration error, distinct from a dependency-graph
                    // violation. `PIPELINE_CONFIGURATION_ERROR` mirrors PY/TS.
                    ModuleError::new(
                        ErrorCode::PipelineConfigurationError,
                        format!(
                            "pipeline.remove: cannot remove step '{step_name}': {}",
                            e.message
                        ),
                    )
                })?;
            }
        }
    }

    // (2) Configure existing step fields (protocol-spec §5.16.1)
    if let Some(Value::Object(configure)) = pipeline_config.get("configure") {
        for (step_name, overrides) in configure {
            let step_name_str = step_name.as_str();
            if let Value::Object(fields) = overrides {
                let match_modules =
                    fields
                        .get("match_modules")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        });
                let ignore_errors = fields
                    .get("ignore_errors")
                    .and_then(serde_json::Value::as_bool);
                let pure_val = fields.get("pure").and_then(serde_json::Value::as_bool);
                let timeout_ms = fields.get("timeout_ms").and_then(serde_json::Value::as_u64);

                // #89: an unknown key is a parse-time error, not a warning.
                // Warn-and-continue meant an operator's typo produced an
                // unconfigured pipeline while the process reported success —
                // and apcore-python and apcore-typescript both raised, so the
                // same apcore.yaml behaved differently on each SDK.
                reject_unknown_keys(
                    fields,
                    CONFIGURABLE_STEP_FIELDS,
                    &format!("pipeline.configure: cannot configure step '{step_name_str}'"),
                )?;
                // D-130: checked here, on the raw keys, before the overlay is
                // built — `ExecutionStrategy::replace_with` would also refuse
                // the weakened step, but only after consuming the original.
                let gate = strategy
                    .steps()
                    .iter()
                    .find(|s| s.name() == step_name_str)
                    .and_then(|s| s.builtin_gate());
                reject_gate_weakening_keys(step_name_str, gate, fields)?;

                // Wrap the existing step with a ConfiguredStep overlay.
                // Issue #33 §1.2: configuring a nonexistent step is a hard
                // configuration error, not a warning.
                strategy
                    .replace_with(step_name_str, |inner| {
                        Box::new(ConfiguredStep {
                            name_override: None,
                            match_modules: match_modules.or_else(|| {
                                inner.match_modules().map(<[std::string::String]>::to_vec)
                            }),
                            ignore_errors: ignore_errors.unwrap_or_else(|| inner.ignore_errors()),
                            pure: pure_val.unwrap_or_else(|| inner.pure()),
                            timeout_ms: timeout_ms.unwrap_or_else(|| inner.timeout_ms()),
                            inner,
                        })
                    })
                    .map_err(|e| {
                        // Sync alignment W-7: configuring a non-existent step
                        // is a structural configuration error, not a
                        // dependency violation.
                        ModuleError::new(
                            ErrorCode::PipelineConfigurationError,
                            format!(
                                "pipeline.configure: cannot configure step '{step_name_str}': {}",
                                e.message
                            ),
                        )
                    })?;
            }
        }
    }

    // (3) Resolve and insert custom steps
    if let Some(steps_list) = pipeline_config.get("steps").and_then(|v| v.as_array()) {
        for step_def in steps_list {
            let step = resolve_step(step_def)?;
            let after = step_def.get("after").and_then(|v| v.as_str());
            let before = step_def.get("before").and_then(|v| v.as_str());

            if let Some(anchor) = after {
                strategy.insert_after(anchor, step).map_err(|e| {
                    // Re-classify missing-anchor as a structural configuration
                    // error (sync alignment W-7). `find_step_index` reports
                    // `StepNotFound`, which is correct for direct strategy
                    // callers but too low-level for pipeline-config callers.
                    // Other failure modes (duplicate step name, dependency
                    // violation) propagate unchanged.
                    if matches!(e.code, ErrorCode::StepNotFound) {
                        ModuleError::new(
                            ErrorCode::PipelineConfigurationError,
                            format!(
                                "pipeline.steps: 'after' anchor '{anchor}' not found: {}",
                                e.message
                            ),
                        )
                    } else {
                        e
                    }
                })?;
            } else if let Some(anchor) = before {
                strategy.insert_before(anchor, step).map_err(|e| {
                    if matches!(e.code, ErrorCode::StepNotFound) {
                        ModuleError::new(
                            ErrorCode::PipelineConfigurationError,
                            format!(
                                "pipeline.steps: 'before' anchor '{anchor}' not found: {}",
                                e.message
                            ),
                        )
                    } else {
                        e
                    }
                })?;
            } else {
                // Issue #33 §1.2: fail-fast on YAML configuration errors.
                // Sync alignment W-7: structural configuration errors use
                // PipelineConfigurationError rather than PipelineConfigInvalid.
                let name = step_def
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                return Err(ModuleError::new(
                    ErrorCode::PipelineConfigurationError,
                    format!(
                        "pipeline.steps: step '{name}' has neither 'after' nor 'before' anchor"
                    ),
                ));
            }
        }
    }

    Ok(strategy)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{PipelineContext, StepResult};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Mutex;

    /// Minimal configurable step for testing.
    struct ConfigurableStep {
        name: String,
        description: String,
    }

    impl ConfigurableStep {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                description: format!("Configurable step: {name}"),
            }
        }
    }

    #[async_trait]
    impl Step for ConfigurableStep {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            &self.description
        }
        fn removable(&self) -> bool {
            true
        }
        fn replaceable(&self) -> bool {
            true
        }
        fn pure(&self) -> bool {
            true
        }
        async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            Ok(StepResult::continue_step())
        }
    }

    // Serialize tests that mutate the global registry to avoid races.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_register_step_type_success() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_step_registry();
        let result = register_step_type(
            "my_step",
            Box::new(|_config| Ok(Box::new(ConfigurableStep::new("my_step")))),
        );
        assert!(result.is_ok());
        assert!(registered_step_types().contains(&"my_step".to_string()));
    }

    #[test]
    fn test_register_step_type_empty_name_rejected() {
        let _guard = TEST_LOCK.lock().unwrap();
        let result = register_step_type("", Box::new(|_| Ok(Box::new(ConfigurableStep::new("x")))));
        assert!(result.is_err());
    }

    #[test]
    fn test_register_step_type_whitespace_name_rejected() {
        let _guard = TEST_LOCK.lock().unwrap();
        let result = register_step_type(
            "my step",
            Box::new(|_| Ok(Box::new(ConfigurableStep::new("x")))),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_register_step_type_duplicate_rejected() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_step_registry();
        register_step_type(
            "dup_step",
            Box::new(|_| Ok(Box::new(ConfigurableStep::new("dup_step")))),
        )
        .unwrap();
        let result = register_step_type(
            "dup_step",
            Box::new(|_| Ok(Box::new(ConfigurableStep::new("dup_step")))),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_unregister_step_type_returns_true_if_found() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_step_registry();
        register_step_type(
            "removable",
            Box::new(|_| Ok(Box::new(ConfigurableStep::new("removable")))),
        )
        .unwrap();
        assert!(unregister_step_type("removable"));
        assert!(!registered_step_types().contains(&"removable".to_string()));
    }

    #[test]
    fn test_unregister_step_type_returns_false_if_not_found() {
        let _guard = TEST_LOCK.lock().unwrap();
        assert!(!unregister_step_type("__nonexistent__"));
    }

    #[test]
    fn test_registered_step_types_after_reset() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_step_registry();
        assert_eq!(registered_step_types(), Vec::<String>::new());
    }

    #[test]
    fn test_build_strategy_from_config_remove_steps() {
        // Does not use the step type registry — no lock needed.
        let config = json!({
            "remove": ["acl_check", "approval_gate"]
        });
        let strategy = build_strategy_from_config(&config).unwrap();
        let names = strategy.step_names();
        assert!(!names.contains(&"acl_check".to_string()));
        assert!(!names.contains(&"approval_gate".to_string()));
        assert!(names.contains(&"module_lookup".to_string()));
        assert!(names.contains(&"execute".to_string()));
    }

    #[test]
    fn test_build_strategy_from_config_insert_custom_step() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_step_registry();
        register_step_type(
            "rate_limiter",
            Box::new(|_config| Ok(Box::new(ConfigurableStep::new("rate_limit")))),
        )
        .unwrap();

        let config = json!({
            "steps": [{
                "name": "rate_limit",
                "type": "rate_limiter",
                "after": "call_chain_guard",
                "config": {}
            }]
        });
        let strategy = build_strategy_from_config(&config).unwrap();
        let names = strategy.step_names();
        let guard_idx = names.iter().position(|n| n == "call_chain_guard").unwrap();
        let rate_idx = names.iter().position(|n| n == "rate_limit").unwrap();
        assert_eq!(rate_idx, guard_idx + 1);
    }

    #[test]
    fn test_build_strategy_from_config_insert_before() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_step_registry();
        register_step_type(
            "before_type",
            Box::new(|_config| Ok(Box::new(ConfigurableStep::new("custom_before")))),
        )
        .unwrap();

        let config = json!({
            "steps": [{
                "name": "custom_before",
                "type": "before_type",
                "before": "execute"
            }]
        });
        let strategy = build_strategy_from_config(&config).unwrap();
        let names = strategy.step_names();
        let custom_idx = names.iter().position(|n| n == "custom_before").unwrap();
        let exec_idx = names.iter().position(|n| n == "execute").unwrap();
        assert_eq!(custom_idx + 1, exec_idx);
    }

    #[test]
    fn test_build_strategy_from_config_unknown_type_returns_error() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_step_registry();
        let config = json!({
            "steps": [{
                "name": "bad",
                "type": "__nonexistent_type__",
                "after": "execute"
            }]
        });
        let result = build_strategy_from_config(&config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("__nonexistent_type__"));
    }

    #[test]
    fn test_build_strategy_from_config_empty_config() {
        let config = json!({});
        let strategy = build_strategy_from_config(&config).unwrap();
        assert_eq!(strategy.step_names().len(), 11);
    }

    #[test]
    fn test_resolve_step_missing_type_returns_error() {
        let _guard = TEST_LOCK.lock().unwrap();
        let step_def = json!({"name": "no_type"});
        let result = resolve_step(&step_def);
        assert!(result.is_err());
    }

    #[test]
    fn test_handler_rejected_with_pipeline_handler_not_supported() {
        let _guard = TEST_LOCK.lock().unwrap();
        let step_def = json!({
            "name": "dynamic",
            "handler": "my_app.steps:CustomStep",
            "after": "execute"
        });
        let result = resolve_step(&step_def);
        match result {
            Err(err) => {
                assert_eq!(err.code, ErrorCode::PipelineHandlerNotSupported);
                assert!(err.message.contains("handler"));
                assert!(err.message.contains("See protocol-spec §5.16.1"));
            }
            Ok(_) => panic!("expected PipelineHandlerNotSupportedError"),
        }
    }

    #[test]
    fn test_resolve_step_applies_metadata_overrides() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_step_registry();
        register_step_type(
            "meta_step",
            Box::new(|_config| Ok(Box::new(ConfigurableStep::new("meta")))),
        )
        .unwrap();

        let step_def = json!({
            "name": "configured",
            "type": "meta_step",
            "match_modules": ["api.*", "web.*"],
            "ignore_errors": true,
            "pure": true,
            "timeout_ms": 5000
        });
        let step = resolve_step(&step_def).unwrap();
        assert_eq!(step.name(), "configured");
        assert_eq!(step.match_modules().unwrap(), &["api.*", "web.*"]);
        assert!(step.ignore_errors());
        assert!(step.pure());
        assert_eq!(step.timeout_ms(), 5000);
    }

    #[test]
    fn test_configure_section_overrides_existing_step() {
        let config = json!({
            "configure": {
                "input_validation": {
                    "ignore_errors": true,
                    "timeout_ms": 3000
                }
            }
        });
        let strategy = build_strategy_from_config(&config).unwrap();
        // Find the configured step and verify overrides took effect.
        let step = strategy
            .steps()
            .iter()
            .find(|s| s.name() == "input_validation")
            .expect("input_validation step should exist");
        assert!(step.ignore_errors());
        assert_eq!(step.timeout_ms(), 3000);
    }
}
