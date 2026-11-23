// APCore Protocol — Execution pipeline types
// Spec reference: docs/features/execution-pipeline.md

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::acl::ACL;
use crate::approval::ApprovalHandler;
use crate::config::Config;
use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};
use crate::events::emitter::EventEmitter;
use crate::middleware::manager::MiddlewareManager;
use crate::module::Module;
use crate::observability::redaction::RedactionConfig;
use crate::policy::ExecutionPolicy;
use crate::registry::registry::Registry;
use crate::utils::helpers::match_pattern;

/// Predicate evaluated by [`PipelineEngine`] after each step completes.
/// Returning `true` halts the pipeline; subsequent steps are not executed.
/// See `execution-pipeline.md` §run_until.
pub type RunUntilPredicate = Box<dyn Fn(&PipelineState) -> bool + Send + Sync>;

// ---------------------------------------------------------------------------
// StepMiddleware
// ---------------------------------------------------------------------------

/// Step-scoped interceptor invoked around every step in an [`ExecutionStrategy`].
///
/// `StepMiddleware` complements the global [`crate::middleware::Middleware`] trait,
/// which wraps the entire module call. `StepMiddleware` instead wraps each
/// pipeline step individually and is the Rust counterpart of Python/TS
/// step-level middleware (Issue #33 §2.2).
///
/// Hook semantics:
/// - [`Self::before_step`] runs before every step. It is an **observation**
///   hook: a [`Step`] is `execute(ctx)`, so there is no inputs parameter for a
///   middleware to replace and the hook has no meaningful return value.
///   Returning `Err` **terminates the step and is not recoverable**: the step
///   body never runs, the error surfaces wrapped in
///   [`ErrorCode::MiddlewareChainError`] (matching the module-level middleware
///   contract), the middlewares that had already entered `before_step` receive
///   `on_step_error` for observation and cleanup only — any value they return
///   is **discarded** — `after_step` is NOT invoked because no body ran, and
///   the step's `ignore_errors` does NOT apply.
/// - [`Self::after_step`] runs after a step body completes, whether it
///   succeeded naturally or was **recovered** by `on_step_error`. A recovered
///   step produced an output and the pipeline continued, so the onion MUST
///   close: a middleware that acquired something in `before_step` gets its
///   `after_step` on both paths, or the recovery path leaks. Errors propagate
///   wrapped in `PipelineStepError`.
/// - [`Self::on_step_error`] runs when a step's `execute()` returns `Err`, and
///   is consulted *before* the step's `ignore_errors` setting is applied.
///   Returning `Ok(Some(value))` recovers — `ctx.output` is replaced with
///   `value`, the failure is treated as a successful continue, and
///   `after_step` still fires for the recovered step. Returning `Ok(None)`
///   lets the original error propagate, subject to `ignore_errors`.
///
/// Invocation order (`features/middleware-system.md` "Pipeline Step Middleware"
/// → "Normative Rules", fixture `pipeline_step_middleware.json`):
/// - `before_step` runs in **registration** order.
/// - `after_step` runs in **reverse** registration order (onion model,
///   identical to the module-level middleware chain).
/// - `on_step_error` runs in **reverse** registration order over only the
///   middlewares whose `before_step` had already executed, and the first
///   `Ok(Some(_))` short-circuits the remaining handlers (first-recovery-wins).
#[async_trait]
pub trait StepMiddleware: Send + Sync {
    /// Called before each step's `execute()` method.
    async fn before_step(
        &self,
        _step_name: &str,
        _state: &PipelineState<'_>,
    ) -> Result<(), ModuleError> {
        Ok(())
    }

    /// Called after each step's `execute()` method completes successfully.
    /// `result` is a snapshot of `ctx.output` taken right after the step ran;
    /// when the step did not set an output it is `Value::Null`.
    async fn after_step(
        &self,
        _step_name: &str,
        _state: &PipelineState<'_>,
        _result: &serde_json::Value,
    ) -> Result<(), ModuleError> {
        Ok(())
    }

    /// Called when a step's `execute()` returns `Err`. Returning `Ok(Some(value))`
    /// recovers the failure: `ctx.output` is set to `value` and the pipeline
    /// continues. `Ok(None)` lets the original error propagate.
    async fn on_step_error(
        &self,
        _step_name: &str,
        _state: &PipelineState<'_>,
        _error: &ModuleError,
    ) -> Result<Option<serde_json::Value>, ModuleError> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Step trait
// ---------------------------------------------------------------------------

/// Identifies a step as one of the framework's built-in governance gates.
///
/// See [`Step::builtin_gate`] — this exists so gate detection is by capability
/// rather than by the step's name (PROTOCOL_SPEC 6.6.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinGate {
    /// The built-in ACL check (`acl_check`).
    Acl,
    /// The built-in approval gate (`approval_gate`).
    Approval,
}

/// A single unit of work in the execution pipeline.
///
/// Step implementations receive their configuration via constructor — the trait
/// only defines the `execute()` contract and metadata accessors.
#[async_trait]
pub trait Step: Send + Sync {
    /// Unique identifier within a strategy (e.g. "`acl_check`").
    fn name(&self) -> &str;

    /// AI-readable purpose description.
    fn description(&self) -> &str;

    /// Whether this step can be removed from the pipeline.
    /// Safety-critical steps return `false`.
    fn removable(&self) -> bool;

    /// Whether this step's implementation can be swapped.
    fn replaceable(&self) -> bool;

    /// Glob patterns for module IDs this step applies to. `None` = all.
    fn match_modules(&self) -> Option<&[String]> {
        None
    }

    /// `true` = step failure logs warning and continues. `false` = step failure aborts pipeline.
    fn ignore_errors(&self) -> bool {
        false
    }

    /// `true` = no side effects. Safe to run during `validate()` (`dry_run` mode).
    fn pure(&self) -> bool {
        false
    }

    /// Per-step timeout in milliseconds. `0` = no per-step timeout.
    fn timeout_ms(&self) -> u64 {
        0
    }

    /// `PipelineContext` fields this step reads (e.g. `["module", "context"]`). Advisory only.
    fn requires(&self) -> &[&str] {
        &[]
    }

    /// Which built-in governance gate this step *is*, if any.
    ///
    /// The capability marker PROTOCOL_SPEC 6.6.5.2 requires: gate detection
    /// **MUST NOT** be by step name. A custom step is free to call itself
    /// `acl_check`, and it gets the `None` default here, so
    /// `Executor::governance_state` cannot mistake it for the gate. Reporting a
    /// gate that is not there is the one direction that accessor must never
    /// fail in.
    ///
    /// Only the framework's own gates override this.
    fn builtin_gate(&self) -> Option<BuiltinGate> {
        None
    }

    /// `PipelineContext` fields this step writes (e.g. `["output"]`). Advisory only.
    fn provides(&self) -> &[&str] {
        &[]
    }

    /// Execute the step, reading/writing shared [`PipelineContext`] state.
    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError>;
}

// ---------------------------------------------------------------------------
// StepResult
// ---------------------------------------------------------------------------

/// The outcome of a step execution, with AI-readable metadata.
///
/// `StepResult` only controls flow (continue / skip / abort) and provides
/// explanatory metadata — it does NOT carry data between steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Flow control action: `"continue"`, `"skip_to"`, or `"abort"`.
    pub action: String,
    /// Target step name when `action` is `"skip_to"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_to: Option<String>,
    /// AI/human-readable explanation of the decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    /// AI decision confidence (0.0–1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Suggested alternatives when the action is `"abort"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternatives: Option<Vec<String>>,
}

impl Default for StepResult {
    fn default() -> Self {
        Self {
            action: "continue".into(),
            skip_to: None,
            explanation: None,
            confidence: None,
            alternatives: None,
        }
    }
}

impl StepResult {
    /// Create a result that continues to the next step.
    #[must_use]
    pub fn continue_step() -> Self {
        Self::default()
    }

    /// Create a result that aborts the pipeline with an explanation.
    #[must_use]
    pub fn abort(explanation: &str) -> Self {
        Self {
            action: "abort".into(),
            explanation: Some(explanation.to_string()),
            ..Default::default()
        }
    }

    /// Create a result that skips forward to the named step.
    #[must_use]
    pub fn skip_to(target: &str) -> Self {
        Self {
            action: "skip_to".into(),
            skip_to: Some(target.to_string()),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// PipelineContext
// ---------------------------------------------------------------------------

/// Shared mutable state flowing through all pipeline steps.
///
/// Fields follow a two-tier model:
/// - **Tier 1** (direct fields): pipeline-essential data written by built-in steps.
/// - **Tier 2** (`context.data`): extension state written by middleware/custom steps
///   via `ContextKey`.
pub struct PipelineContext {
    /// Module being invoked.
    pub module_id: String,
    /// Original inputs (may be mutated by `middleware_before`).
    pub inputs: serde_json::Value,
    /// `APCore` execution context.
    pub context: Context<serde_json::Value>,

    // -- Resolved during pipeline (None until the responsible step runs) --
    /// Set by `module_lookup` step.
    pub module: Option<Arc<dyn Module>>,
    /// The §6.1.8 governance projection of `inputs` — the argument key set and
    /// each key's JSON type, and **never** a value.
    ///
    /// Computed by `module_lookup` (Step 3) and read by `acl_check` (Step 4);
    /// PROTOCOL_SPEC §6.1.8 rule 1 makes that ordering normative rather than an
    /// implementation detail that happens to hold. It is deliberately NOT
    /// `context.redacted_inputs`: that field's contract is safe *logging*, and
    /// it is a raw copy of the arguments when the module declares no input
    /// schema (§6.1.8 rule 3).
    pub governance_projection: Option<crate::acl::GovernanceProjection>,
    /// Whether the Step-4 ACL decision required this call to be put to a human
    /// (PROTOCOL_SPEC §6.1.6).
    ///
    /// Set by `acl_check`; read by `approval_gate` (Step 5) and by
    /// `Executor::validate`'s preflight, both of which take the **union** of
    /// this with the module annotation and the policy (§6.9 rows 3-6). Stays
    /// `false` when no ACL is configured or the `acl_check` step was removed
    /// from the strategy — absence of the gate is not a requirement.
    pub acl_approval_required: bool,
    /// Set by `input_validation` step.
    pub validated_inputs: Option<serde_json::Value>,
    /// Set by `execute` step (non-streaming).
    pub output: Option<serde_json::Value>,
    /// Set by `output_validation` step.
    pub validated_output: Option<serde_json::Value>,

    // -- Pipeline v2 --
    /// `true` during `validate()`. `PipelineEngine` skips steps with `pure=false`.
    pub dry_run: bool,
    /// Pure-step errors collected by a non-fail-fast preflight run.
    pub preflight_errors: Vec<(String, ModuleError)>,
    /// Passed through to `module_lookup` for version negotiation.
    pub version_hint: Option<String>,
    /// Tracks which middleware ran, enabling `on_error` recovery chain.
    pub executed_middlewares: Vec<usize>,

    // -- Executor resources (injected by Executor::call) --
    /// Module registry for lookups.
    pub registry: Option<Arc<Registry>>,
    /// Executor configuration (timeouts, call depth limits, etc.).
    pub config: Option<Arc<Config>>,
    /// The `obs.redaction.*` rules the input/output capture point applies.
    ///
    /// PROTOCOL_SPEC §10.6.1 "Where the rules apply": the union of §10.6's
    /// `x-sensitive` rule and the two configured rules MUST hold at BOTH log
    /// emission and this capture point, with the same rules at each. Until
    /// this field existed `redact_sensitive(data, schema)` had no parameter for
    /// them at all, so an operator's `regex_patterns` entry redacted a bearer
    /// token in the log line they were watching and stored it in the audit
    /// record they were not (aiperceivable/apcore#120).
    ///
    /// Resolved ONCE by the executor and injected, rather than built per
    /// execution from `config` above: §10.6.1 requirement 5 puts compilation at
    /// the configuration read. `None` means the spec defaults, which is what
    /// the capture point already applied.
    pub redaction: Option<Arc<RedactionConfig>>,
    /// Access control list, if configured.
    pub acl: Option<Arc<ACL>>,
    /// Approval handler, if configured.
    pub approval_handler: Option<Arc<dyn ApprovalHandler>>,
    /// Execution-time governance policy for the approval gate (apcore#76 RFC pilot).
    pub policy: Option<Arc<ExecutionPolicy>>,
    /// Event emitter for governance events (apcore#77). When present, the ACL
    /// and approval-gate steps publish `apcore.acl.denied` /
    /// `apcore.approval.decision` / `apcore.policy.override` events.
    pub event_emitter: Option<Arc<EventEmitter>>,
    /// Executor-owned dedup set backing the approval gate's fail-loud
    /// warn-once-per-module governance warnings (apcore#76). Threaded from the
    /// Executor so dedup persists across calls; `None` for standalone steps.
    pub governance_warned: Option<Arc<parking_lot::Mutex<std::collections::HashSet<String>>>>,
    /// Middleware manager for before/after chains.
    pub middleware_manager: Option<Arc<MiddlewareManager>>,

    // -- Metadata --
    /// Name of the strategy driving this execution.
    pub strategy_name: String,
    /// Accumulates step-level trace records.
    pub trace: PipelineTrace,
}

impl PipelineContext {
    /// Create a new `PipelineContext` for a module invocation.
    pub fn new(
        module_id: impl Into<String>,
        inputs: serde_json::Value,
        context: Context<serde_json::Value>,
        strategy_name: impl Into<String>,
    ) -> Self {
        let module_id = module_id.into();
        let strategy_name = strategy_name.into();
        Self {
            trace: PipelineTrace::new(module_id.clone(), strategy_name.clone()),
            module_id,
            inputs,
            context,
            module: None,
            governance_projection: None,
            acl_approval_required: false,
            validated_inputs: None,
            output: None,
            validated_output: None,
            dry_run: false,
            preflight_errors: vec![],
            version_hint: None,
            executed_middlewares: vec![],
            registry: None,
            config: None,
            redaction: None,
            acl: None,
            approval_handler: None,
            policy: None,
            event_emitter: None,
            governance_warned: None,
            middleware_manager: None,
            strategy_name,
        }
    }
}

// ---------------------------------------------------------------------------
// Trace types
// ---------------------------------------------------------------------------

/// A single step's execution record within a pipeline trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepTrace {
    /// Step name.
    pub name: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: f64,
    /// The step's result (action, explanation, confidence).
    pub result: StepResult,
    /// Whether this step was skipped (e.g. via `skip_to`).
    pub skipped: bool,
    /// Whether this step is an AI decision point.
    pub decision_point: bool,
    /// Reason the step was skipped: "`no_match`", "`dry_run`", or "`error_ignored`".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

/// Complete execution record for a pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineTrace {
    /// Module that was invoked.
    pub module_id: String,
    /// Strategy that was used.
    pub strategy_name: String,
    /// Per-step records, in execution order.
    pub steps: Vec<StepTrace>,
    /// Total pipeline duration in milliseconds.
    pub total_duration_ms: f64,
    /// Whether the pipeline completed successfully.
    pub success: bool,
}

impl PipelineTrace {
    /// Create an empty trace for a new pipeline run.
    #[must_use]
    pub fn new(module_id: String, strategy_name: String) -> Self {
        Self {
            module_id,
            strategy_name,
            steps: Vec::new(),
            total_duration_ms: 0.0,
            success: false,
        }
    }
}

// ---------------------------------------------------------------------------
// PipelineState
// ---------------------------------------------------------------------------

/// Snapshot passed to a [`RunUntilPredicate`] after each pipeline step
/// completes. See `execution-pipeline.md` §run_until.
///
/// Mirrors `apcore.pipeline.PipelineState` in Python.
pub struct PipelineState<'a> {
    /// The step this view is about: the one that just completed for a
    /// [`RunUntilPredicate`], the one being observed for a [`StepMiddleware`]
    /// hook.
    pub step_name: &'a str,
    /// Output snapshots, keyed by step name — **exactly the steps that
    /// completed BEFORE [`Self::step_name`]**. The value is a copy of
    /// `ctx.output` taken after the step ran (or `None` if the step set no
    /// output). Snapshots are append-only across the run.
    ///
    /// `step_name` itself is **never** a key in any of the three
    /// [`StepMiddleware`] hooks (`features/middleware-system.md` → "Pipeline
    /// Step Middleware" → "What `state.outputs` contains"): in `before_step` it
    /// has not run; in `on_step_error` it ran and failed, so there is no
    /// output; in `after_step` it succeeded and its output is the `result`
    /// parameter. Implementations MUST NOT insert it before invoking
    /// `after_step` — the insert being ordered AFTER the hook is what enforces
    /// this, on the naturally successful path and the recovery path alike.
    pub outputs: &'a HashMap<String, Option<serde_json::Value>>,
    /// The live pipeline context. Held by reference — predicates must not
    /// mutate state through it.
    pub context: &'a PipelineContext,
}

// ---------------------------------------------------------------------------
// StrategyInfo
// ---------------------------------------------------------------------------

/// AI-introspectable summary of an [`ExecutionStrategy`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyInfo {
    /// Strategy name.
    pub name: String,
    /// Number of steps in the strategy.
    pub step_count: usize,
    /// Ordered list of step names.
    pub step_names: Vec<String>,
    /// Auto-generated description from step descriptions.
    pub description: String,
}

impl std::fmt::Display for StrategyInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-step pipeline: {}",
            self.step_count,
            self.step_names.join(" \u{2192} ")
        )
    }
}

// ---------------------------------------------------------------------------
// ExecutionStrategy
// ---------------------------------------------------------------------------

/// An ordered list of steps defining a complete execution pipeline.
///
/// Provides safe mutation methods that enforce removability / replaceability
/// constraints and step-name uniqueness.
///
/// Maintains an internal `HashMap<String, usize>` index from step name to
/// position so step lookups (used by `skip_to`, `configure_step`, and the
/// streaming `run_until_step` path) are O(1). The index is rebuilt after
/// every mutation.
pub struct ExecutionStrategy {
    name: String,
    steps: Vec<Box<dyn Step>>,
    name_to_idx: HashMap<String, usize>,
    step_middlewares: Vec<Arc<dyn StepMiddleware>>,
}

impl std::fmt::Debug for ExecutionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionStrategy")
            .field("name", &self.name)
            .field("step_names", &self.step_names())
            .field("step_count", &self.steps.len())
            .field("name_to_idx_len", &self.name_to_idx.len())
            .field("step_middleware_count", &self.step_middlewares.len())
            .finish()
    }
}

/// PROTOCOL_SPEC §5.16.1 "Governance gates cannot be weakened by `configure`"
/// (D-130), enforced on a step object.
///
/// Three fields turn a built-in gate into something that no longer gates:
/// `ignore_errors` turns a denial into a warning and runs the call,
/// `match_modules` exempts every module it does not match, and `pure: true` on
/// the approval gate makes `validate()` run it — and consult the
/// `ApprovalHandler` — during a dry run. A step that identifies itself as a
/// built-in gate ([`Step::builtin_gate`]) carrying any of them is refused with
/// `PIPELINE_CONFIGURATION_ERROR`, naming the step and every offending field.
///
/// `pure` is checked on the approval gate only: the built-in ACL gate is
/// `pure` by default (`validate()` runs it), so either value is accepted there.
/// The `pipeline.configure` path applies the same rule to the raw keys
/// (`pipeline_config::reject_gate_weakening_keys`).
///
/// # Errors
///
/// [`ErrorCode::PipelineConfigurationError`] when the step is a built-in gate
/// with a weakening field set.
pub(crate) fn reject_weakened_gate(step: &dyn Step) -> Result<(), ModuleError> {
    let Some(gate) = step.builtin_gate() else {
        return Ok(());
    };
    let mut offending: Vec<&str> = Vec::new();
    if step.ignore_errors() {
        offending.push("ignore_errors: true");
    }
    if step.match_modules().is_some() {
        offending.push("match_modules");
    }
    if gate == BuiltinGate::Approval && step.pure() {
        offending.push("pure: true");
    }
    if offending.is_empty() {
        return Ok(());
    }
    Err(gate_weakening_error(step.name(), &offending))
}

/// The one message both D-130 enforcement points use, so a configuration-file
/// rejection and a programmatic one read the same.
pub(crate) fn gate_weakening_error(step_name: &str, offending: &[&str]) -> ModuleError {
    ModuleError::new(
        ErrorCode::PipelineConfigurationError,
        format!(
            "Step '{step_name}' is a built-in governance gate and cannot be weakened: {} \
             not allowed (PROTOCOL_SPEC 5.16.1). Only timeout_ms is configurable on a gate \
             step; remove the step instead to take the gate out, which is reported by \
             governance_state().",
            offending.join(", ")
        ),
    )
}

/// A gate a rejected [`ExecutionStrategy::replace_with`] wrapper produced, with
/// its three weakening fields forced back to the built-in values.
///
/// `replace_with` hands the current step to the wrapper by value, so once the
/// wrapper returns there is no original left to put back. Re-installing the
/// wrapper's step as-is would leave the weakened gate running behind an error
/// the caller may ignore; this leaves a gate that behaves as the built-in does.
struct NeutralisedGate {
    inner: Box<dyn Step>,
    gate: BuiltinGate,
}

#[async_trait]
impl Step for NeutralisedGate {
    fn name(&self) -> &str {
        self.inner.name()
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
        None
    }
    fn ignore_errors(&self) -> bool {
        false
    }
    fn pure(&self) -> bool {
        // The built-in values: the ACL gate is pure, the approval gate is not.
        self.gate == BuiltinGate::Acl
    }
    fn timeout_ms(&self) -> u64 {
        self.inner.timeout_ms()
    }
    fn requires(&self) -> &[&str] {
        self.inner.requires()
    }
    fn provides(&self) -> &[&str] {
        self.inner.provides()
    }
    fn builtin_gate(&self) -> Option<BuiltinGate> {
        Some(self.gate)
    }
    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        self.inner.execute(ctx).await
    }
}

/// No-op step used as a temporary placeholder by [`ExecutionStrategy::replace_with`].
struct PlaceholderStep;

#[async_trait]
impl Step for PlaceholderStep {
    fn name(&self) -> &'static str {
        "__placeholder__"
    }
    fn description(&self) -> &'static str {
        ""
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult::continue_step())
    }
}

impl ExecutionStrategy {
    /// Create a new strategy with the given name and initial steps.
    ///
    /// Returns an error if any two steps share the same name.
    pub fn new(name: impl Into<String>, steps: Vec<Box<dyn Step>>) -> Result<Self, ModuleError> {
        let name = name.into();
        // Check for duplicate step names.
        let mut seen = std::collections::HashSet::new();
        for step in &steps {
            // D-130: a strategy cannot be assembled around a weakened gate.
            reject_weakened_gate(step.as_ref())?;
            if !seen.insert(step.name().to_string()) {
                // Mirrors apcore-python `pipeline.py:218` and
                // apcore-typescript `pipeline.ts:235`, which both raise
                // `StepNameDuplicateError` from the constructor's duplicate
                // detection.
                return Err(ModuleError::step_name_duplicate(format!(
                    "Duplicate step name '{}' in strategy '{}'",
                    step.name(),
                    name,
                )));
            }
        }
        let mut strategy = Self {
            name,
            steps,
            name_to_idx: HashMap::new(),
            step_middlewares: Vec::new(),
        };
        strategy.rebuild_index();
        // §2.1: fail-fast on unmet requires/provides at construction.
        strategy.validate_dependencies()?;
        Ok(strategy)
    }

    /// Rebuild the O(1) name→index map. Called after any structural mutation.
    fn rebuild_index(&mut self) {
        self.name_to_idx = self
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name().to_string(), i))
            .collect();
    }

    /// Read-only access to the step name→index map. Useful for tests asserting
    /// O(1) lookup compliance per §1.5.
    #[must_use]
    pub fn name_to_idx(&self) -> &HashMap<String, usize> {
        &self.name_to_idx
    }

    /// Validate `requires`/`provides` declarations across the strategy.
    ///
    /// Issue #33 §2.1: rather than logging a `tracing::warn!` and proceeding
    /// (the previous behaviour), an unmet `requires` reference now causes
    /// strategy construction to fail with [`ErrorCode::PipelineDependencyError`].
    /// This makes pipeline misconfiguration a startup-time error rather than
    /// a latent runtime hazard.
    fn validate_dependencies(&self) -> Result<(), ModuleError> {
        let mut provided = std::collections::HashSet::new();
        for step in &self.steps {
            for req in step.requires() {
                if !provided.contains(*req) {
                    return Err(ModuleError::new(
                        ErrorCode::PipelineDependencyError,
                        format!(
                            "Step '{}' in strategy '{}' requires '{}', but no preceding step \
                             provides it",
                            step.name(),
                            self.name,
                            req,
                        ),
                    ));
                }
            }
            for p in step.provides() {
                provided.insert(*p);
            }
        }
        Ok(())
    }

    /// Register a [`StepMiddleware`] for this strategy. Middlewares execute in
    /// registration order during the before phase and reverse order during the
    /// after phase. See [`StepMiddleware`] (Issue #33 §2.2).
    pub fn add_step_middleware(&mut self, mw: Arc<dyn StepMiddleware>) {
        self.step_middlewares.push(mw);
    }

    /// Read-only access to the registered step middlewares.
    #[must_use]
    pub fn step_middlewares(&self) -> &[Arc<dyn StepMiddleware>] {
        &self.step_middlewares
    }

    /// Strategy name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Rename this strategy.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Ordered list of step names.
    #[must_use]
    pub fn step_names(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.name().to_string()).collect()
    }

    /// Read-only access to the step list.
    #[must_use]
    pub fn steps(&self) -> &[Box<dyn Step>] {
        &self.steps
    }

    /// Insert a step immediately after the named anchor.
    pub fn insert_after(&mut self, anchor: &str, step: Box<dyn Step>) -> Result<(), ModuleError> {
        reject_weakened_gate(step.as_ref())?;
        self.validate_no_duplicate(step.name())?;
        let idx = self.find_step_index(anchor)?;
        self.steps.insert(idx + 1, step);
        self.rebuild_index();
        self.validate_dependencies()?;
        Ok(())
    }

    /// Insert a step immediately before the named anchor.
    pub fn insert_before(&mut self, anchor: &str, step: Box<dyn Step>) -> Result<(), ModuleError> {
        reject_weakened_gate(step.as_ref())?;
        self.validate_no_duplicate(step.name())?;
        let idx = self.find_step_index(anchor)?;
        self.steps.insert(idx, step);
        self.rebuild_index();
        self.validate_dependencies()?;
        Ok(())
    }

    /// Remove a step by name. Fails if the step is not removable.
    ///
    /// Errors: [`ErrorCode::StepNotFound`] when the step does not exist,
    /// [`ErrorCode::StepNotRemovable`] when it exists but is pinned. Mirrors
    /// apcore-python `StepNotFoundError`/`StepNotRemovableError` and
    /// apcore-typescript `StepNotFoundError`/`StepNotRemovableError`.
    pub fn remove(&mut self, step_name: &str) -> Result<(), ModuleError> {
        let idx = self.find_step_index(step_name)?;
        if !self.steps[idx].removable() {
            return Err(ModuleError::new(
                ErrorCode::StepNotRemovable,
                format!("Step '{step_name}' is not removable"),
            ));
        }
        self.steps.remove(idx);
        self.rebuild_index();
        Ok(())
    }

    /// Replace a step's implementation. Fails if the step is not replaceable.
    ///
    /// Errors: [`ErrorCode::StepNotFound`] when the step does not exist,
    /// [`ErrorCode::StepNotReplaceable`] when it exists but is pinned.
    ///
    /// Note the deliberate cross-SDK split on the missing-step code: `replace`
    /// reports `STEP_NOT_FOUND` while [`configure_step`](Self::configure_step)
    /// reports `PIPELINE_STEP_NOT_FOUND`. All three SDKs share that split.
    pub fn replace(&mut self, step_name: &str, new_step: Box<dyn Step>) -> Result<(), ModuleError> {
        let idx = self.find_step_index(step_name)?;
        if !self.steps[idx].replaceable() {
            return Err(ModuleError::new(
                ErrorCode::StepNotReplaceable,
                format!("Step '{step_name}' is not replaceable"),
            ));
        }
        self.validate_replacement_name(step_name, new_step.name())?;
        reject_weakened_gate(new_step.as_ref())?;
        // Step name may differ from original; rebuild the index either way.
        self.steps[idx] = new_step;
        self.rebuild_index();
        Ok(())
    }

    /// Reject a replacement whose name collides with a *different* existing
    /// step.
    ///
    /// Without this, the strategy can end up holding two identically-named
    /// steps while `rebuild_index` keeps only the last: a `skip_to` then
    /// resolves past the first occurrence and `remove` targets the wrong
    /// position. apcore-typescript guards the same case in
    /// `ExecutionStrategy.configureStep` (`pipeline.ts:355`) and raises
    /// `StepNameDuplicateError` — it is the same duplicate-name condition as
    /// [`validate_no_duplicate`](Self::validate_no_duplicate), so it carries
    /// the same [`ErrorCode::StepNameDuplicate`].
    fn validate_replacement_name(
        &self,
        step_name: &str,
        new_name: &str,
    ) -> Result<(), ModuleError> {
        if new_name != step_name && self.name_to_idx.contains_key(new_name) {
            return Err(ModuleError::step_name_duplicate(format!(
                "Step name '{new_name}' already exists at a different position in strategy '{}'",
                self.name
            )));
        }
        Ok(())
    }

    /// Configure a step by replacing it in place — `execution-pipeline.md`
    /// §Modifying a strategy.
    ///
    /// Calling this twice with the same `step_name` is idempotent: there is
    /// always exactly one step at the original position. Fails with
    /// [`ErrorCode::PipelineStepNotFound`] when the target does not exist.
    pub fn configure_step(
        &mut self,
        step_name: &str,
        new_step: Box<dyn Step>,
    ) -> Result<(), ModuleError> {
        let idx = self.name_to_idx.get(step_name).copied().ok_or_else(|| {
            ModuleError::new(
                ErrorCode::PipelineStepNotFound,
                format!("Pipeline step not found: '{step_name}'"),
            )
        })?;
        // Respect the replaceable flag, matching `replace()` and the
        // apcore-python / apcore-typescript `configure_step` guards, which
        // raise `StepNotReplaceableError` here (only the missing-step code
        // differs between `configure_step` and `replace`).
        if !self.steps[idx].replaceable() {
            return Err(ModuleError::new(
                ErrorCode::StepNotReplaceable,
                format!("Step '{step_name}' is not replaceable"),
            ));
        }
        self.validate_replacement_name(step_name, new_step.name())?;
        // D-130: this is the programmatic step-configuration API, so the rule
        // `pipeline.configure` enforces applies to the step it installs.
        reject_weakened_gate(new_step.as_ref())?;
        self.steps[idx] = new_step;
        self.rebuild_index();
        Ok(())
    }

    /// Replace a step by applying a wrapper function over its current value.
    ///
    /// Used by `build_strategy_from_config` to overlay YAML metadata
    /// (`match_modules`, `ignore_errors`, etc.) on built-in steps without
    /// losing the original step logic.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::StepNotFound`] when the step does not exist, and
    /// [`ErrorCode::PipelineConfigurationError`] when the wrapper returns a
    /// weakened built-in gate (D-130). The original step was moved into the
    /// wrapper, so that rejection cannot be undone; the wrapper's gate stays
    /// installed with its weakening fields forced back to the built-in values
    /// rather than running weakened behind an error the caller may ignore.
    pub fn replace_with<F>(&mut self, step_name: &str, wrapper: F) -> Result<(), ModuleError>
    where
        F: FnOnce(Box<dyn Step>) -> Box<dyn Step>,
    {
        let idx = self.find_step_index(step_name)?;
        let old = std::mem::replace(&mut self.steps[idx], Box::new(PlaceholderStep));
        let installed = wrapper(old);
        let rejection = reject_weakened_gate(installed.as_ref()).err();
        self.steps[idx] = match (&rejection, installed.builtin_gate()) {
            (Some(_), Some(gate)) => Box::new(NeutralisedGate {
                inner: installed,
                gate,
            }),
            _ => installed,
        };
        self.rebuild_index();
        rejection.map_or(Ok(()), Err)
    }

    /// Build a [`StrategyInfo`] summary for AI introspection.
    #[must_use]
    pub fn info(&self) -> StrategyInfo {
        let step_names = self.step_names();
        let description = self
            .steps
            .iter()
            .map(|s| format!("{}: {}", s.name(), s.description()))
            .collect::<Vec<_>>()
            .join("; ");
        StrategyInfo {
            name: self.name.clone(),
            step_count: self.steps.len(),
            step_names,
            description,
        }
    }

    // -- helpers --

    /// Locate a step by name.
    ///
    /// Raises [`ErrorCode::StepNotFound`] when absent — the code shared by
    /// `insert_after`/`insert_before` (missing anchor), `remove` and `replace`
    /// in apcore-python and apcore-typescript. `configure_step` deliberately
    /// does NOT route through here: it reports `PIPELINE_STEP_NOT_FOUND`.
    fn find_step_index(&self, step_name: &str) -> Result<usize, ModuleError> {
        // O(1) via name_to_idx (§1.5).
        self.name_to_idx.get(step_name).copied().ok_or_else(|| {
            ModuleError::new(
                ErrorCode::StepNotFound,
                format!("Step '{}' not found in strategy '{}'", step_name, self.name),
            )
        })
    }

    /// Reject inserting a step whose name is already taken.
    ///
    /// Raises [`ErrorCode::StepNameDuplicate`] — the code apcore-python
    /// (`pipeline.py:259,271`) and apcore-typescript (`pipeline.ts:286,300`)
    /// raise from `insert_after` / `insert_before`.
    fn validate_no_duplicate(&self, name: &str) -> Result<(), ModuleError> {
        if self.name_to_idx.contains_key(name) {
            return Err(ModuleError::step_name_duplicate(format!(
                "Step name '{}' already exists in strategy '{}'",
                name, self.name,
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PipelineEngine
// ---------------------------------------------------------------------------

/// Executes an [`ExecutionStrategy`] against a [`PipelineContext`], returning
/// the final output and a complete execution trace.
pub struct PipelineEngine;

/// Optional knobs for [`PipelineEngine::run_with_options`]. All fields are
/// independent — set only what you need.
#[derive(Default)]
pub struct RunOptions {
    /// Halt execution **before** the step with this name (the step is not run
    /// and not recorded in the trace). Used by the streaming path to drive the
    /// shared engine up to the `execute` step. `None` runs to the end.
    pub stop_before_step: Option<String>,
    /// Run ONLY the steps named here; every other step is passed over without
    /// running and without a trace record. `None` runs every step.
    ///
    /// The streaming path uses this to run a SEGMENT of an already-started
    /// strategy — the tail after `execute` on the non-streaming fallback
    /// (STR-1), and the post-stream steps in Phase 3 (STR-2) — against the same
    /// `PipelineContext` the pre-execute steps ran against. It is the Rust
    /// equivalent of apcore-python's `post_strategy` slice, which is built with
    /// `validate_dependencies=False` for the same reason: the segment's
    /// `requires` keys were seeded by the earlier steps, not by this run.
    pub only_steps: Option<Vec<String>>,
    /// Predicate evaluated **after** every successful step completes. Returning
    /// `true` halts the pipeline; the current step's output is preserved.
    /// See `execution-pipeline.md` §run_until.
    pub until: Option<RunUntilPredicate>,
}

impl RunOptions {
    /// Build options that stop before the step with the given name.
    #[must_use]
    pub fn stop_before(step_name: impl Into<String>) -> Self {
        Self {
            stop_before_step: Some(step_name.into()),
            only_steps: None,
            until: None,
        }
    }

    /// Build options that run only the named steps, in strategy order.
    #[must_use]
    pub fn only_steps(step_names: Vec<String>) -> Self {
        Self {
            stop_before_step: None,
            only_steps: Some(step_names),
            until: None,
        }
    }

    /// Build options with a predicate-based termination condition.
    #[must_use]
    pub fn run_until<F>(predicate: F) -> Self
    where
        F: Fn(&PipelineState) -> bool + Send + Sync + 'static,
    {
        Self {
            stop_before_step: None,
            only_steps: None,
            until: Some(Box::new(predicate)),
        }
    }
}

/// Run every `after_step` hook in REVERSE registration order (onion model — the
/// middleware registered first opens the outermost layer and closes last).
///
/// Shared by the two paths that MUST close the onion: a step body that
/// succeeded naturally, and a step body that failed and was **recovered** by
/// `on_step_error`. `features/middleware-system.md` ("Pipeline Step
/// Middleware" → Normative Rules) requires the recovered case too — the step
/// produced an output and the pipeline continued, so a middleware that
/// acquired something in `before_step` would otherwise never release it.
///
/// The `before_step` unwind path deliberately does NOT call this: no step body
/// ran there, so there is nothing to close over, and the two paths MUST NOT be
/// unified (same section, "A `before_step` failure terminates the step").
async fn run_after_step_hooks(
    middlewares: &[Arc<dyn StepMiddleware>],
    step_name: &str,
    step_outputs: &HashMap<String, Option<serde_json::Value>>,
    ctx: &PipelineContext,
    result: &serde_json::Value,
) -> Result<(), ModuleError> {
    for mw in middlewares.iter().rev() {
        let state = PipelineState {
            step_name,
            outputs: step_outputs,
            context: ctx,
        };
        mw.after_step(step_name, &state, result).await?;
    }
    Ok(())
}

impl PipelineEngine {
    /// Run every step in `strategy` against `ctx`, respecting flow-control
    /// actions (`continue`, `skip_to`, `abort`).
    pub async fn run(
        strategy: &ExecutionStrategy,
        ctx: &mut PipelineContext,
    ) -> Result<(Option<serde_json::Value>, PipelineTrace), ModuleError> {
        Self::run_with_options(strategy, ctx, RunOptions::default()).await
    }

    /// Run with a predicate-based termination condition — `execution-pipeline.md`
    /// §run_until.
    ///
    /// The predicate is evaluated **after** each step completes successfully.
    /// Returning `true` halts the pipeline and returns the accumulated output;
    /// returning `false` lets the pipeline proceed to the next step.
    pub async fn run_until<F>(
        strategy: &ExecutionStrategy,
        ctx: &mut PipelineContext,
        predicate: F,
    ) -> Result<(Option<serde_json::Value>, PipelineTrace), ModuleError>
    where
        F: Fn(&PipelineState) -> bool + Send + Sync + 'static,
    {
        Self::run_with_options(strategy, ctx, RunOptions::run_until(predicate)).await
    }

    /// Run every step in `strategy` against `ctx` UP TO (but not including)
    /// the named step. All step metadata — `match_modules`, `ignore_errors`,
    /// `timeout_ms`, `dry_run` purity filtering, `skip_to` flow control — is
    /// honored identically to [`Self::run`], so streaming and non-streaming
    /// paths never diverge on per-step semantics.
    ///
    /// Used by `Executor::stream` (with `stop_before_step = "execute"`) to
    /// prepare the pipeline without running the module itself, so the caller
    /// can then drive true chunk-by-chunk streaming.
    pub async fn run_until_step(
        strategy: &ExecutionStrategy,
        ctx: &mut PipelineContext,
        stop_before_step: &str,
    ) -> Result<(Option<serde_json::Value>, PipelineTrace), ModuleError> {
        Self::run_with_options(strategy, ctx, RunOptions::stop_before(stop_before_step)).await
    }

    /// Run with full control over termination. See [`RunOptions`].
    #[allow(clippy::too_many_lines)] // pipeline control loop is inherently stateful; splitting would reduce clarity
    pub async fn run_with_options(
        strategy: &ExecutionStrategy,
        ctx: &mut PipelineContext,
        options: RunOptions,
    ) -> Result<(Option<serde_json::Value>, PipelineTrace), ModuleError> {
        let pipeline_start = std::time::Instant::now();
        let steps = strategy.steps();
        let mut idx: usize = 0;
        // Snapshot of ctx.output keyed by step name, accumulated across the
        // run. Passed to run_until predicates as PipelineState::outputs.
        let mut step_outputs: HashMap<String, Option<serde_json::Value>> = HashMap::new();
        let mut failed_capabilities: HashSet<String> = HashSet::new();

        while idx < steps.len() {
            let step = &steps[idx];

            // Early exit for streaming / partial-pipeline callers.
            if let Some(stop_name) = options.stop_before_step.as_deref() {
                if step.name() == stop_name {
                    break;
                }
            }

            // Segment filter: a step outside the requested set is passed over
            // entirely — not run, not traced, and not recorded as skipped,
            // because it is not part of this run at all.
            if let Some(only) = options.only_steps.as_ref() {
                if !only.iter().any(|name| name == step.name()) {
                    idx += 1;
                    continue;
                }
            }

            // Read declarations (trait defaults for backward compat)
            let step_match_modules = step.match_modules();
            let step_ignore_errors = step.ignore_errors();
            let step_pure = step.pure();
            let step_timeout_ms = step.timeout_ms();
            if ctx.dry_run
                && ((ctx.module.is_none() && step.requires().contains(&"module"))
                    || step
                        .requires()
                        .iter()
                        .any(|capability| failed_capabilities.contains(*capability)))
            {
                failed_capabilities.extend(
                    step.provides()
                        .iter()
                        .map(|capability| (*capability).to_owned()),
                );
                ctx.trace.steps.push(StepTrace {
                    name: step.name().to_string(),
                    duration_ms: 0.0,
                    result: StepResult::continue_step(),
                    skipped: true,
                    decision_point: false,
                    skip_reason: Some("missing_dependency".to_string()),
                });
                idx += 1;
                continue;
            }

            // (1) match_modules filter
            if let Some(patterns) = step_match_modules {
                let matched = patterns
                    .iter()
                    .any(|pattern| match_pattern(pattern, &ctx.module_id));
                if !matched {
                    ctx.trace.steps.push(StepTrace {
                        name: step.name().to_string(),
                        duration_ms: 0.0,
                        result: StepResult::continue_step(),
                        skipped: true,
                        decision_point: false,
                        skip_reason: Some("no_match".to_string()),
                    });
                    idx += 1;
                    continue;
                }
            }

            // (2) dry_run filter: skip steps with side effects
            if ctx.dry_run && !step_pure {
                ctx.trace.steps.push(StepTrace {
                    name: step.name().to_string(),
                    duration_ms: 0.0,
                    result: StepResult::continue_step(),
                    skipped: true,
                    decision_point: false,
                    skip_reason: Some("dry_run".to_string()),
                });
                idx += 1;
                continue;
            }

            // (3a) StepMiddleware: before_step hooks run in REGISTRATION order
            //      (middleware-system.md "Normative Rules"). The hook only
            //      observes — a Step is `execute(ctx)`, so there is no inputs
            //      parameter it could rewrite and its return value is unused.
            let step_name_for_hooks = step.name().to_string();
            let middlewares = strategy.step_middlewares();
            let mut before_err: Option<ModuleError> = None;
            // Count of middlewares whose `before_step` actually ran, including
            // the one that failed. Middlewares past this point never entered
            // the step and MUST NOT be told it failed.
            let mut before_executed = 0usize;
            for mw in middlewares {
                let state = PipelineState {
                    step_name: &step_name_for_hooks,
                    outputs: &step_outputs,
                    context: ctx,
                };
                before_executed += 1;
                if let Err(e) = mw.before_step(&step_name_for_hooks, &state).await {
                    before_err = Some(e);
                    break;
                }
            }
            if let Some(err) = before_err {
                // Spec: "Step middleware errors raised from `before_step`
                // itself (not from the step body) MUST be wrapped in
                // `MiddlewareChainError`, identical to the module-level
                // contract." The original error stays recoverable through
                // `ModuleError::unwrap_middleware_chain_error()`, mirroring
                // `MiddlewareManager::execute_before`.
                let mut details = std::collections::HashMap::new();
                if let Ok(inner_json) = serde_json::to_value(&err) {
                    details.insert("inner_error".to_string(), inner_json);
                }
                details.insert(
                    "step_name".to_string(),
                    serde_json::json!(step_name_for_hooks),
                );
                let chain_err =
                    ModuleError::new(ErrorCode::MiddlewareChainError, err.message.clone())
                        .with_details(details)
                        .with_cause(format!(
                    "StepMiddleware before_step failed for step '{step_name_for_hooks}': {err}"
                ));

                // The step body never ran, so unwind by notifying exactly the
                // middlewares that did run, in REVERSE order. A recovery value
                // returned here is ignored: there is no step output to replace
                // because the step never executed, and the fixture requires the
                // MiddlewareChainError to surface.
                //
                // First-recovery-wins MUST NOT apply to this pass: short-
                // circuiting exists to stop shopping for a recovery once one is
                // found, and no recovery is being sought here. The loop
                // therefore has NO early exit — neither `Ok(Some(_))` (dropped
                // by the `if let Err` pattern) nor `Err(_)` (logged) may cut it
                // short, or the cleanup of every middleware registered behind
                // that one is stranded.
                for mw in middlewares[..before_executed].iter().rev() {
                    let state = PipelineState {
                        step_name: &step_name_for_hooks,
                        outputs: &step_outputs,
                        context: ctx,
                    };
                    if let Err(hook_err) = mw
                        .on_step_error(&step_name_for_hooks, &state, &chain_err)
                        .await
                    {
                        tracing::warn!(
                            step = %step_name_for_hooks,
                            error = %hook_err,
                            "StepMiddleware on_step_error failed while unwinding before_step"
                        );
                    }
                }

                ctx.trace.steps.push(StepTrace {
                    name: step_name_for_hooks.clone(),
                    duration_ms: 0.0,
                    result: StepResult::abort(&chain_err.to_string()),
                    skipped: false,
                    decision_point: false,
                    skip_reason: None,
                });
                ctx.trace.total_duration_ms = pipeline_start.elapsed().as_secs_f64() * 1000.0;
                return Err(chain_err);
            }

            // (3) Execute with per-step timeout
            let step_start = std::time::Instant::now();
            let exec_result = if step_timeout_ms > 0 {
                match tokio::time::timeout(
                    std::time::Duration::from_millis(step_timeout_ms),
                    step.execute(ctx),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_elapsed) => Err(ModuleError::new(
                        ErrorCode::ModuleTimeout,
                        format!(
                            "Step '{}' timed out after {}ms",
                            step.name(),
                            step_timeout_ms
                        ),
                    )),
                }
            } else {
                step.execute(ctx).await
            };

            let duration_ms = step_start.elapsed().as_secs_f64() * 1000.0;

            let result = match exec_result {
                Ok(r) => r,
                Err(err) => {
                    if ctx.dry_run {
                        failed_capabilities.extend(
                            step.provides()
                                .iter()
                                .map(|capability| (*capability).to_owned()),
                        );
                        ctx.preflight_errors
                            .push((step.name().to_string(), err.clone()));
                        ctx.trace.steps.push(StepTrace {
                            name: step.name().to_string(),
                            duration_ms,
                            result: StepResult::abort(&err.to_string()),
                            skipped: false,
                            decision_point: false,
                            skip_reason: None,
                        });
                        idx += 1;
                        continue;
                    }
                    // (3b) StepMiddleware: on_step_error hooks run in REVERSE
                    // registration order (onion unwinding) and may recover by
                    // returning Some(value). The FIRST middleware to return a
                    // recovery value wins; remaining middlewares are skipped.
                    let mut recovery: Option<serde_json::Value> = None;
                    let mut hook_err: Option<ModuleError> = None;
                    for mw in middlewares.iter().rev() {
                        let state = PipelineState {
                            step_name: &step_name_for_hooks,
                            outputs: &step_outputs,
                            context: ctx,
                        };
                        match mw.on_step_error(&step_name_for_hooks, &state, &err).await {
                            Ok(Some(v)) => {
                                recovery = Some(v);
                                break;
                            }
                            Ok(None) => {}
                            Err(e) => {
                                hook_err = Some(e);
                                break;
                            }
                        }
                    }
                    if let Some(e) = hook_err {
                        ctx.trace.steps.push(StepTrace {
                            name: step_name_for_hooks.clone(),
                            duration_ms,
                            result: StepResult::abort(&e.to_string()),
                            skipped: false,
                            decision_point: false,
                            skip_reason: None,
                        });
                        ctx.trace.total_duration_ms =
                            pipeline_start.elapsed().as_secs_f64() * 1000.0;
                        return Err(ModuleError::pipeline_step_error(&step_name_for_hooks, &e));
                    }
                    if let Some(value) = recovery {
                        ctx.output = Some(value);
                        // The onion MUST close after a RECOVERED step body as
                        // well as after a naturally successful one: the step
                        // produced an output and the pipeline continues, so a
                        // middleware that acquired something in `before_step`
                        // gets its `after_step` here or the recovery path
                        // leaks (`features/middleware-system.md` → "Pipeline
                        // Step Middleware" → Normative Rules; fixture case
                        // `after_step_fires_after_a_recovered_step`). Every
                        // registered middleware ran `before_step` — a step body
                        // only executes once the whole chain succeeded — so the
                        // full chain unwinds, in reverse registration order.
                        let after_result_value =
                            ctx.output.clone().unwrap_or(serde_json::Value::Null);
                        if let Err(after_err) = run_after_step_hooks(
                            middlewares,
                            &step_name_for_hooks,
                            &step_outputs,
                            ctx,
                            &after_result_value,
                        )
                        .await
                        {
                            ctx.trace.steps.push(StepTrace {
                                name: step_name_for_hooks.clone(),
                                duration_ms,
                                result: StepResult::abort(&after_err.to_string()),
                                skipped: false,
                                decision_point: false,
                                skip_reason: None,
                            });
                            ctx.trace.total_duration_ms =
                                pipeline_start.elapsed().as_secs_f64() * 1000.0;
                            return Err(ModuleError::pipeline_step_error(
                                &step_name_for_hooks,
                                &after_err,
                            ));
                        }
                        ctx.trace.steps.push(StepTrace {
                            name: step_name_for_hooks.clone(),
                            duration_ms,
                            result: StepResult {
                                action: "continue".into(),
                                explanation: Some(format!("recovered from: {err}")),
                                ..Default::default()
                            },
                            skipped: false,
                            decision_point: false,
                            skip_reason: Some("error_recovered".to_string()),
                        });
                        // Below `run_after_step_hooks` for the same reason as
                        // (6): the recovered step is still the CURRENT step
                        // while its `after_step` runs, so it MUST NOT yet be a
                        // key in `state.outputs`.
                        step_outputs.insert(step_name_for_hooks.clone(), ctx.output.clone());
                        idx += 1;
                        continue;
                    }

                    // (4) ignore_errors: log and continue (§1.1)
                    if step_ignore_errors {
                        tracing::warn!(
                            step = step.name(),
                            error = %err,
                            "Step failed (ignored)"
                        );
                        ctx.trace.steps.push(StepTrace {
                            name: step.name().to_string(),
                            duration_ms,
                            result: StepResult {
                                action: "continue".into(),
                                explanation: Some(err.to_string()),
                                ..Default::default()
                            },
                            skipped: false,
                            decision_point: false,
                            skip_reason: Some("error_ignored".to_string()),
                        });
                        idx += 1;
                        continue;
                    }
                    // Fail-fast (§1.1): record the abort in the trace and
                    // surface a `PipelineStepError` carrying the step name and
                    // original cause. Executor consumers (call/validate)
                    // unwrap this back to the original error before returning
                    // to user code.
                    ctx.trace.steps.push(StepTrace {
                        name: step.name().to_string(),
                        duration_ms,
                        result: StepResult::abort(&err.to_string()),
                        skipped: false,
                        decision_point: false,
                        skip_reason: None,
                    });
                    ctx.trace.total_duration_ms = pipeline_start.elapsed().as_secs_f64() * 1000.0;
                    return Err(ModuleError::pipeline_step_error(step.name(), &err));
                }
            };

            // (3c) StepMiddleware: after_step hooks run after the step body,
            //      in REVERSE registration order (onion model — a middleware
            //      registered first opens the outermost layer and closes last).
            //      Snapshot of ctx.output is passed; null when step did not set output.
            //      The recovered-step path at (3b) closes the onion the same way.
            let after_result_value = ctx.output.clone().unwrap_or(serde_json::Value::Null);
            let after_err = run_after_step_hooks(
                middlewares,
                &step_name_for_hooks,
                &step_outputs,
                ctx,
                &after_result_value,
            )
            .await
            .err();
            if let Some(err) = after_err {
                ctx.trace.steps.push(StepTrace {
                    name: step_name_for_hooks.clone(),
                    duration_ms,
                    result: StepResult::abort(&err.to_string()),
                    skipped: false,
                    decision_point: false,
                    skip_reason: None,
                });
                ctx.trace.total_duration_ms = pipeline_start.elapsed().as_secs_f64() * 1000.0;
                return Err(ModuleError::pipeline_step_error(&step_name_for_hooks, &err));
            }

            let action = result.action.clone();
            let skip_target = result.skip_to.clone();

            // (5) Record trace
            ctx.trace.steps.push(StepTrace {
                name: step.name().to_string(),
                duration_ms,
                result,
                skipped: false,
                decision_point: false,
                skip_reason: None,
            });

            // (6) Snapshot output for run_until predicates. `Value::clone` walks
            // the full JSON tree, so later in-place mutations of `ctx.output`
            // cannot alter the historical record. (Python's `dict(ctx.output)`
            // does only a one-level copy; the Rust snapshot is deeper but the
            // intent — a frozen copy — is the same.)
            //
            // ORDERING IS NORMATIVE: this MUST stay below (3c). `state.outputs`
            // holds exactly the steps that completed BEFORE the current one, so
            // moving the insert above the `after_step` hooks would start
            // showing them the current step — the drift apcore-python was
            // corrected for. Fixture case
            // `state_outputs_excludes_the_current_step_in_every_hook`.
            let step_name_owned = step.name().to_string();
            step_outputs.insert(step_name_owned.clone(), ctx.output.clone());

            // (7) Handle abort / skip_to
            match action.as_str() {
                "continue" => {
                    // (8) run_until predicate (§1.4): evaluated after a clean
                    // continue. Stops further steps when it returns true; the
                    // pipeline reports success and returns the current output.
                    if let Some(ref predicate) = options.until {
                        let state = PipelineState {
                            step_name: &step_name_owned,
                            outputs: &step_outputs,
                            context: ctx,
                        };
                        if predicate(&state) {
                            ctx.trace.total_duration_ms =
                                pipeline_start.elapsed().as_secs_f64() * 1000.0;
                            ctx.trace.success = true;
                            return Ok((ctx.output.clone(), ctx.trace.clone()));
                        }
                    }
                    idx += 1;
                }
                "abort" => {
                    ctx.trace.total_duration_ms = pipeline_start.elapsed().as_secs_f64() * 1000.0;
                    ctx.trace.success = false;
                    return Ok((ctx.output.clone(), ctx.trace.clone()));
                }
                "skip_to" => {
                    let target = skip_target.as_deref().unwrap_or("");
                    // O(1) lookup via the strategy's name_to_idx map (§1.5).
                    // The lookup must reject same-position and backward
                    // targets explicitly to prevent infinite loops; the prior
                    // linear scan implicitly enforced this.
                    let target_idx = strategy
                        .name_to_idx()
                        .get(target)
                        .copied()
                        .filter(|t| *t > idx);
                    match target_idx {
                        Some(target_idx) => {
                            // Mark skipped steps in trace.
                            for step in steps.iter().take(target_idx).skip(idx + 1) {
                                ctx.trace.steps.push(StepTrace {
                                    name: step.name().to_string(),
                                    duration_ms: 0.0,
                                    result: StepResult::continue_step(),
                                    skipped: true,
                                    decision_point: false,
                                    skip_reason: None,
                                });
                            }
                            idx = target_idx;
                        }
                        None => {
                            return Err(ModuleError::new(
                                ErrorCode::StepNotFound,
                                format!(
                                    "skip_to target '{}' not found after step '{}'",
                                    target,
                                    step.name(),
                                ),
                            ));
                        }
                    }
                }
                other => {
                    return Err(ModuleError::new(
                        ErrorCode::GeneralInvalidInput,
                        format!("Unknown step action: '{other}'"),
                    ));
                }
            }
        }

        ctx.trace.total_duration_ms = pipeline_start.elapsed().as_secs_f64() * 1000.0;
        ctx.trace.success = true;
        Ok((ctx.output.clone(), ctx.trace.clone()))
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal step implementation for testing.
    struct FakeStep {
        name: String,
        description: String,
        removable: bool,
        replaceable: bool,
    }

    impl FakeStep {
        fn new(name: &str, removable: bool, replaceable: bool) -> Self {
            Self {
                name: name.to_string(),
                description: format!("Fake step: {name}"),
                removable,
                replaceable,
            }
        }

        fn boxed(name: &str, removable: bool, replaceable: bool) -> Box<dyn Step> {
            Box::new(Self::new(name, removable, replaceable))
        }
    }

    #[async_trait]
    impl Step for FakeStep {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            &self.description
        }
        fn removable(&self) -> bool {
            self.removable
        }
        fn replaceable(&self) -> bool {
            self.replaceable
        }
        async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            Ok(StepResult::continue_step())
        }
    }

    #[test]
    fn test_step_result_continue() {
        let r = StepResult::continue_step();
        assert_eq!(r.action, "continue");
        assert!(r.skip_to.is_none());
        assert!(r.explanation.is_none());
    }

    #[test]
    fn test_step_result_abort() {
        let r = StepResult::abort("bad input");
        assert_eq!(r.action, "abort");
        assert_eq!(r.explanation.as_deref(), Some("bad input"));
    }

    #[test]
    fn test_step_result_skip_to() {
        let r = StepResult::skip_to("execute");
        assert_eq!(r.action, "skip_to");
        assert_eq!(r.skip_to.as_deref(), Some("execute"));
    }

    #[test]
    fn test_step_result_serde_round_trip() {
        let r = StepResult {
            action: "abort".into(),
            explanation: Some("denied".into()),
            confidence: Some(0.95),
            alternatives: Some(vec!["retry".into()]),
            ..Default::default()
        };
        let json = serde_json::to_string(&r).expect("serialize");
        let r2: StepResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r2.action, "abort");
        assert_eq!(r2.confidence, Some(0.95));
    }

    #[test]
    fn test_strategy_new_rejects_duplicate_names() {
        let steps: Vec<Box<dyn Step>> = vec![
            FakeStep::boxed("a", true, true),
            FakeStep::boxed("a", true, true),
        ];
        let result = ExecutionStrategy::new("test", steps);
        assert!(result.is_err());
    }

    #[test]
    fn test_strategy_step_names() {
        let strategy = ExecutionStrategy::new(
            "default",
            vec![
                FakeStep::boxed("one", true, true),
                FakeStep::boxed("two", true, true),
                FakeStep::boxed("three", true, true),
            ],
        )
        .expect("create strategy");

        assert_eq!(strategy.step_names(), vec!["one", "two", "three"]);
    }

    #[test]
    fn test_strategy_insert_after() {
        let mut strategy = ExecutionStrategy::new(
            "s",
            vec![
                FakeStep::boxed("a", true, true),
                FakeStep::boxed("c", true, true),
            ],
        )
        .unwrap();

        strategy
            .insert_after("a", FakeStep::boxed("b", true, true))
            .unwrap();

        assert_eq!(strategy.step_names(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_strategy_insert_before() {
        let mut strategy = ExecutionStrategy::new(
            "s",
            vec![
                FakeStep::boxed("a", true, true),
                FakeStep::boxed("c", true, true),
            ],
        )
        .unwrap();

        strategy
            .insert_before("c", FakeStep::boxed("b", true, true))
            .unwrap();

        assert_eq!(strategy.step_names(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_strategy_insert_rejects_duplicate() {
        let mut strategy =
            ExecutionStrategy::new("s", vec![FakeStep::boxed("a", true, true)]).unwrap();

        let result = strategy.insert_after("a", FakeStep::boxed("a", true, true));
        assert!(result.is_err());
    }

    #[test]
    fn test_strategy_insert_rejects_unknown_anchor() {
        let mut strategy =
            ExecutionStrategy::new("s", vec![FakeStep::boxed("a", true, true)]).unwrap();

        let result = strategy.insert_after("missing", FakeStep::boxed("b", true, true));
        assert!(result.is_err());
    }

    #[test]
    fn test_strategy_remove() {
        let mut strategy = ExecutionStrategy::new(
            "s",
            vec![
                FakeStep::boxed("a", true, true),
                FakeStep::boxed("b", true, true),
            ],
        )
        .unwrap();

        strategy.remove("a").unwrap();
        assert_eq!(strategy.step_names(), vec!["b"]);
    }

    #[test]
    fn test_strategy_remove_non_removable() {
        let mut strategy =
            ExecutionStrategy::new("s", vec![FakeStep::boxed("core", false, false)]).unwrap();

        let result = strategy.remove("core");
        assert!(result.is_err());
    }

    #[test]
    fn test_strategy_replace() {
        let mut strategy =
            ExecutionStrategy::new("s", vec![FakeStep::boxed("a", true, true)]).unwrap();

        strategy
            .replace("a", FakeStep::boxed("a", true, true))
            .unwrap();

        assert_eq!(strategy.step_names(), vec!["a"]);
    }

    #[test]
    fn test_strategy_replace_non_replaceable() {
        let mut strategy =
            ExecutionStrategy::new("s", vec![FakeStep::boxed("a", true, false)]).unwrap();

        let result = strategy.replace("a", FakeStep::boxed("a", true, true));
        assert!(result.is_err());
    }

    #[test]
    fn test_strategy_info() {
        let strategy = ExecutionStrategy::new(
            "default",
            vec![
                FakeStep::boxed("one", true, true),
                FakeStep::boxed("two", false, true),
            ],
        )
        .unwrap();

        let info = strategy.info();
        assert_eq!(info.name, "default");
        assert_eq!(info.step_count, 2);
        assert_eq!(info.step_names, vec!["one", "two"]);
        assert!(info.description.contains("one"));
        assert!(info.description.contains("two"));
    }

    #[test]
    fn test_pipeline_trace_new() {
        let trace = PipelineTrace::new("my_module".into(), "default".into());
        assert_eq!(trace.module_id, "my_module");
        assert_eq!(trace.strategy_name, "default");
        assert!(trace.steps.is_empty());
        assert!(!trace.success);
    }

    #[test]
    fn test_pipeline_context_new() {
        let ctx_inner = Context::<serde_json::Value>::anonymous();
        let pctx = PipelineContext::new(
            "test_module",
            serde_json::json!({"key": "value"}),
            ctx_inner,
            "default",
        );
        assert_eq!(pctx.module_id, "test_module");
        assert_eq!(pctx.strategy_name, "default");
        assert!(pctx.module.is_none());
        assert!(pctx.validated_inputs.is_none());
        assert!(pctx.output.is_none());
        assert!(pctx.validated_output.is_none());
        assert_eq!(pctx.trace.module_id, "test_module");
    }

    // --- run_until / prepare_stream unification tests --------------------

    /// Step that records how many times it was invoked. Used to prove that
    /// `run_until` stops BEFORE the named step and that `match_modules`
    /// filtering runs on the streaming path exactly as on the non-streaming
    /// path.
    struct CountingStep {
        name: String,
        invocations: Arc<std::sync::atomic::AtomicUsize>,
        match_modules: Option<Vec<String>>,
    }

    #[async_trait]
    impl Step for CountingStep {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            &self.name
        }
        fn removable(&self) -> bool {
            true
        }
        fn replaceable(&self) -> bool {
            true
        }
        fn match_modules(&self) -> Option<&[String]> {
            self.match_modules.as_deref()
        }
        async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
            self.invocations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(StepResult::continue_step())
        }
    }

    #[tokio::test]
    async fn run_until_stops_before_named_step() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let pre_count = Arc::new(AtomicUsize::new(0));
        let execute_count = Arc::new(AtomicUsize::new(0));
        let post_count = Arc::new(AtomicUsize::new(0));

        let steps: Vec<Box<dyn Step>> = vec![
            Box::new(CountingStep {
                name: "pre".into(),
                invocations: Arc::clone(&pre_count),
                match_modules: None,
            }),
            Box::new(CountingStep {
                name: "execute".into(),
                invocations: Arc::clone(&execute_count),
                match_modules: None,
            }),
            Box::new(CountingStep {
                name: "post".into(),
                invocations: Arc::clone(&post_count),
                match_modules: None,
            }),
        ];
        let strategy = ExecutionStrategy::new("test", steps).unwrap();
        let mut pctx = PipelineContext::new(
            "mod.x",
            serde_json::json!({}),
            Context::<serde_json::Value>::anonymous(),
            "test",
        );

        let (_, trace) = PipelineEngine::run_until_step(&strategy, &mut pctx, "execute")
            .await
            .unwrap();

        assert_eq!(pre_count.load(Ordering::SeqCst), 1, "'pre' runs once");
        assert_eq!(
            execute_count.load(Ordering::SeqCst),
            0,
            "'execute' must NOT run — run_until stops before it"
        );
        assert_eq!(post_count.load(Ordering::SeqCst), 0, "'post' must not run");
        assert!(trace.success);
    }

    #[tokio::test]
    async fn run_until_applies_match_modules_filtering() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Regression: previously `prepare_stream` had a bespoke loop that
        // skipped match_modules filtering, so a step declaring
        // `match_modules: ["api.*"]` would run even for `mod.other`. This
        // test confirms `run_until` inherits filtering from `run`.
        let filtered_count = Arc::new(AtomicUsize::new(0));

        let steps: Vec<Box<dyn Step>> = vec![
            Box::new(CountingStep {
                name: "filtered_step".into(),
                invocations: Arc::clone(&filtered_count),
                match_modules: Some(vec!["api.*".into()]),
            }),
            Box::new(CountingStep {
                name: "execute".into(),
                invocations: Arc::new(AtomicUsize::new(0)),
                match_modules: None,
            }),
        ];
        let strategy = ExecutionStrategy::new("test", steps).unwrap();
        let mut pctx = PipelineContext::new(
            "other.mod",
            serde_json::json!({}),
            Context::<serde_json::Value>::anonymous(),
            "test",
        );

        PipelineEngine::run_until_step(&strategy, &mut pctx, "execute")
            .await
            .unwrap();

        assert_eq!(
            filtered_count.load(Ordering::SeqCst),
            0,
            "step with match_modules=['api.*'] must be skipped when module_id='other.mod' \
             — confirms streaming and non-streaming paths share dispatch semantics"
        );
    }
}
