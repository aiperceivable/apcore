// APCore Protocol — Built-in execution pipeline steps
// Spec reference: docs/features/execution-pipeline.md (Built-in steps)

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::{ErrorCode, ModuleError};
use crate::events::emitter::ApCoreEvent;
use crate::executor::{redact_sensitive_with, validate_against_schema};
use crate::pipeline::{BuiltinGate, ExecutionStrategy, PipelineContext, Step, StepResult};
use crate::policy::PolicyDecision;

// Macro for step metadata — execute is implemented manually per step.
macro_rules! step_meta {
    ($name:ident, $step_name:expr, $desc:expr, removable=$rm:expr, replaceable=$rp:expr, pure=$pure:expr) => {
        pub struct $name;

        impl $name {
            fn _name(&self) -> &str {
                $step_name
            }
            fn _description(&self) -> &str {
                $desc
            }
            fn _removable(&self) -> bool {
                $rm
            }
            fn _replaceable(&self) -> bool {
                $rp
            }
            fn _pure(&self) -> bool {
                $pure
            }
        }
    };
}

// Shared helper macro to implement the non-execute Step trait methods.
macro_rules! impl_step_meta {
    ($name:ident) => {
        fn name(&self) -> &str {
            self._name()
        }
        fn description(&self) -> &str {
            self._description()
        }
        fn removable(&self) -> bool {
            self._removable()
        }
        fn replaceable(&self) -> bool {
            self._replaceable()
        }
        fn pure(&self) -> bool {
            self._pure()
        }
    };
}

step_meta!(
    BuiltinContextCreation,
    "context_creation",
    "Create or inherit execution context",
    removable = false,
    replaceable = false,
    pure = true
);
step_meta!(
    BuiltinCallChainGuard,
    "call_chain_guard",
    "Validate call depth and module repeat limits",
    removable = true,
    replaceable = true,
    pure = true
);
/// Resolve a module from the registry by ID and reject disabled modules.
///
/// Unlike the other unit-struct steps, this step carries a per-instance
/// `Arc<ToggleState>` (issue #71). The strategy factory injects the owning
/// `APCore` instance's toggle store so that toggling a module on one instance
/// does not affect another instance in the same process. When no instance
/// handle is available (e.g. `build_standard_strategy()` called directly, or a
/// custom strategy built without an `APCore`), it falls back to the
/// process-global toggle store via [`Default`].
pub struct BuiltinModuleLookup {
    toggle_state: std::sync::Arc<crate::sys_modules::ToggleState>,
}

impl BuiltinModuleLookup {
    /// Build a lookup step bound to a specific toggle store (read path).
    #[must_use]
    pub fn new(toggle_state: std::sync::Arc<crate::sys_modules::ToggleState>) -> Self {
        Self { toggle_state }
    }
}

impl Default for BuiltinModuleLookup {
    /// Fall back to the process-global toggle store, preserving back-compat for
    /// callers that construct the standard strategy without an `APCore`.
    fn default() -> Self {
        Self::new(crate::sys_modules::global_toggle_state_arc())
    }
}
step_meta!(
    BuiltinACLCheck,
    "acl_check",
    "Enforce access control rules",
    removable = true,
    replaceable = true,
    pure = true
);
step_meta!(
    BuiltinApprovalGate,
    "approval_gate",
    "Handle human or AI approval flow",
    removable = true,
    replaceable = true,
    pure = false
);
step_meta!(
    BuiltinInputValidation,
    "input_validation",
    "Validate inputs against schema",
    removable = true,
    replaceable = true,
    pure = true
);
step_meta!(
    BuiltinMiddlewareBefore,
    "middleware_before",
    "Execute before-middleware chain",
    removable = true,
    replaceable = false,
    pure = false
);
step_meta!(
    BuiltinExecute,
    "execute",
    "Invoke the module with timeout",
    removable = false,
    replaceable = true,
    pure = false
);
step_meta!(
    BuiltinOutputValidation,
    "output_validation",
    "Validate outputs against schema",
    removable = true,
    replaceable = true,
    pure = true
);
step_meta!(
    BuiltinMiddlewareAfter,
    "middleware_after",
    "Execute after-middleware chain",
    removable = true,
    replaceable = false,
    pure = false
);
step_meta!(
    BuiltinReturnResult,
    "return_result",
    "Finalize and return output",
    removable = false,
    replaceable = false,
    pure = true
);

// ---------------------------------------------------------------------------
// Step implementations
// ---------------------------------------------------------------------------

fn missing_pipeline_input(name: &str) -> ModuleError {
    ModuleError::new(
        ErrorCode::PipelineDependencyError,
        format!("Required pipeline input '{name}' is missing"),
    )
}

fn capture_redacted(
    value: &Value,
    schema: &Value,
    ctx: &PipelineContext,
) -> std::collections::HashMap<String, Value> {
    redact_sensitive_with(value, schema, ctx.redaction.as_deref())
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[async_trait]
impl Step for BuiltinContextCreation {
    impl_step_meta!(BuiltinContextCreation);
    fn provides(&self) -> &[&str] {
        &["context"]
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        // If the context has no caller_id, default to @external. Preserve
        // every other field on the incoming context (cancel_token,
        // global_deadline, data, identity, services, redacted_inputs/output)
        // so per-call resources like cancellation tokens flow through the
        // pipeline unmodified (otherwise D-21 cancel-token checks observe a
        // fresh, never-cancelled token).
        //
        // D-103: `@external` is the caller-side ACL sentinel substituted for a
        // null `caller_id` — it is NOT a principal, and an implementation MUST
        // NOT synthesize an `Identity` for a call that supplied none. A module
        // written to the spec's own example (`if not context.identity: raise
        // "Authentication required"`) rejects an unauthenticated call on
        // apcore-python and apcore-typescript, and used to be ADMITTED here
        // because this step manufactured a principal and `child()` then cloned
        // it down to the module.
        if ctx.context.caller_id.is_none() {
            ctx.context.caller_id = Some(crate::acl::EXTERNAL_CALLER.to_string());
        }

        // Spec: BuiltinContextCreation MUST set context.global_deadline if
        // the context does not already carry one and the executor config
        // declares a non-zero global_timeout (sync finding A-D-201).
        // Cross-language parity with apcore-python (sets in step 1) and
        // apcore-typescript (sets via Context constructor).
        //
        // Units: BuiltinExecute reads global_deadline as fractional seconds
        // since UNIX_EPOCH (see builtin_steps.rs:500-507). We store the same
        // unit here so set+read are consistent.
        if ctx.context.global_deadline.is_none() {
            if let Some(config) = ctx.config.as_ref() {
                let global_timeout_ms = config.executor.global_timeout;
                if global_timeout_ms > 0 {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64();
                    #[allow(clippy::cast_precision_loss)]
                    let deadline = now + (global_timeout_ms as f64) / 1000.0;
                    ctx.context.global_deadline = Some(deadline);
                }
            }
        }

        // Derive the child context for THIS call. `Context::child` appends
        // `module_id` to `call_chain` and promotes the previous tail to
        // `caller_id`.
        //
        // This MUST live in `context_creation`, which is non-removable, not in
        // `call_chain_guard`, which `build_testing_strategy` and
        // `build_minimal_strategy` remove. With the derivation in the guard,
        // those two presets never grew `call_chain`, so depth limits,
        // circular-call detection and frequency throttling all reset and a
        // nested call reported the wrong `caller_id`. apcore-python
        // (`builtin_steps.py` context_creation) and apcore-typescript
        // (`builtin-steps.ts` contextCreation) both derive it here;
        // docs/features/execution-pipeline.md lists `context_creation` among
        // the non-removable steps for exactly this reason.
        ctx.context = ctx.context.child(&ctx.module_id);
        ctx.context.cancel_token = Some(ctx.context.cancel_token.as_ref().map_or_else(
            crate::cancel::CancelToken::new,
            crate::cancel::CancelToken::child,
        ));

        Ok(StepResult::continue_step())
    }
}

#[async_trait]
impl Step for BuiltinCallChainGuard {
    impl_step_meta!(BuiltinCallChainGuard);
    fn requires(&self) -> &[&str] {
        &["context"]
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        let config = ctx
            .config
            .as_ref()
            .ok_or_else(|| missing_pipeline_input("config"))?;
        // Cancellation check before any expensive validation/middleware work
        // (D-21 — `Cancel Token Mid-Pipeline Check`, point 1). The pipeline
        // MUST observe `cancel_token` at the call-chain guard so a caller
        // who cancelled before submission does not pay the cost of ACL,
        // middleware, and validation steps. Spec:
        // docs/features/core-executor.md §Cancel Token Mid-Pipeline Check.
        if let Some(token) = ctx.context.cancel_token.as_ref() {
            token.check_for(&ctx.module_id)?;
        }
        // The child context was already derived by the non-removable
        // `context_creation` step, so `call_chain` here already ends with
        // `module_id`. The guard counts the full chain — including the
        // trailing self-entry — for frequency, and strips it for circular
        // detection (sync findings A-D-039 / A-D-040). Deriving it here
        // instead would tie chain growth to a removable step; see the note in
        // `BuiltinContextCreation::execute`.
        crate::utils::guard_call_chain_with_repeat(
            &ctx.context,
            &ctx.module_id,
            config.executor.max_call_depth as usize,
            config.executor.max_module_repeat as usize,
        )?;
        Ok(StepResult::continue_step())
    }
}

#[async_trait]
impl Step for BuiltinModuleLookup {
    fn name(&self) -> &'static str {
        "module_lookup"
    }
    fn description(&self) -> &'static str {
        "Resolve module from registry by ID"
    }
    fn removable(&self) -> bool {
        false
    }
    fn replaceable(&self) -> bool {
        false
    }
    fn pure(&self) -> bool {
        true
    }
    fn provides(&self) -> &[&str] {
        &["module"]
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        let registry = ctx
            .registry
            .as_ref()
            .ok_or_else(|| missing_pipeline_input("registry"))?;
        // ERR-004: the guidance-bearing builder — the peers attach a default
        // `ai_guidance` to MODULE_NOT_FOUND on the error class, so every raise
        // site carries it.
        let (module, lease) = registry.acquire_execution(
            &ctx.module_id,
            ctx.version_hint.as_deref(),
            ctx.context.cancel_token.as_ref(),
        )?;
        // Check if the module is disabled before proceeding. Two sources:
        //   1. The registry descriptor's `enabled` flag (direct registry-level
        //      disable).
        //   2. The per-instance ToggleState owned by the `APCore` that built
        //      this strategy (issue #71). It is the authoritative store written
        //      by that instance's `ToggleFeatureModule` and survives registry
        //      reloads (sync finding A-D-12). The pipeline MUST consult it
        //      because the toggle no longer mutates the descriptor flag, and it
        //      MUST read the instance store (not the process-global) so toggles
        //      stay isolated per `APCore`. Mirrors apcore-python
        //      BuiltinModuleLookup (builtin_steps.py:252).
        if !lease.enabled || self.toggle_state.is_disabled(&ctx.module_id) {
            return Err(ModuleError::module_disabled(&ctx.module_id));
        }
        ctx.module = Some(module.clone());
        ctx.module_lease = Some(lease);

        // PROTOCOL_SPEC §6.1.8 rule 1: compute the governance projection of the
        // call's arguments HERE, at Step 3, so it is available to the ACL check
        // at Step 4. The ordering is normative, not incidental — the `arguments`
        // condition (§6.1.7) has nothing to read otherwise, and a rule carrying
        // it would load, match, and go unevaluable on every call.
        //
        // Deliberately NOT `context.redacted_inputs`, which is computed just
        // below for a different purpose (§6.1.8 rule 3): that field's contract
        // is safe logging regardless of whether the module declares an input
        // schema. The projection carries the key set and each key's JSON
        // type and has no field that can hold a value.
        ctx.governance_projection = Some(crate::acl::GovernanceProjection::from_arguments(
            &ctx.inputs,
        ));

        // Early input redaction: set context.redacted_inputs BEFORE
        // middleware runs (step 6), so logging sees redacted data.
        let mut captured = capture_redacted(&ctx.inputs, &module.input_schema(), ctx);
        captured.remove("_approval_token");
        ctx.context.redacted_inputs = Some(captured);

        Ok(StepResult::continue_step())
    }
}

#[async_trait]
impl Step for BuiltinACLCheck {
    impl_step_meta!(BuiltinACLCheck);

    /// PROTOCOL_SPEC 6.6.5.2 — the capability marker that identifies this as
    /// the framework's ACL gate, so `governance_state()` never has to match on
    /// the step's name.
    fn builtin_gate(&self) -> Option<BuiltinGate> {
        Some(BuiltinGate::Acl)
    }

    fn requires(&self) -> &[&str] {
        &["context", "module"]
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        if let Some(acl) = ctx.acl.clone() {
            let caller_id = ctx.context.caller_id.clone();
            // The structured accessor (PROTOCOL_SPEC §6.8.1), never the boolean:
            // Step 4 now produces TWO results (§6.1.6) and `check()` folds them
            // together by failing closed, which would turn "allowed, ask a
            // human" into a denial here. The Executor is the one caller that
            // must see them apart.
            // The ASYNC accessor, never the synchronous one (PROTOCOL_SPEC
            // §6.1.3, D-105). This step is already `async`, and the two entry
            // points resolve conditions from different registries: on the sync
            // path a key registered through `ACL::register_async_condition` is
            // "async only" and resolves to UNEVALUABLE, so §6.1.1 makes an
            // `allow` rule carrying it stop granting and a `deny` rule carrying
            // it deny unconditionally. Calling `check_access` here left the
            // entire async condition registry dead in the only path that
            // enforces. Mirrors apcore-python (builtin_steps.py) and
            // apcore-typescript (builtin-steps.ts), which both prefer the async
            // accessor when the ACL exposes one.
            let decision = acl
                .async_check_access(
                    caller_id.as_deref(),
                    &ctx.module_id,
                    Some(&ctx.context),
                    ctx.governance_projection.as_ref(),
                )
                .await;
            // Carried to Step 5 and to `Executor::validate`'s preflight, which
            // union it with the module annotation and the policy (§6.9 rows
            // 3-6). Recorded even on a denial, where it is always false.
            ctx.acl_approval_required = decision.approval_required;
            let allowed = decision.is_allowed();
            ctx.acl_decision = Some(decision);
            if !allowed {
                // Publish a governance event on denial (canonical name proposed
                // in apcore#77). Guarded by dry_run so a validate() preflight
                // probe never emits a spurious denial event; fires only on deny
                // (allows are high-volume and already covered by tracing spans).
                if !ctx.dry_run {
                    if let Some(emitter) = ctx.event_emitter.clone() {
                        let event = ApCoreEvent::with_module(
                            "apcore.acl.denied",
                            serde_json::json!({
                                "module_id": ctx.module_id,
                                "caller_id": caller_id,
                                "reason": "ACL denied",
                                "trace_id": ctx.context.trace_id,
                            }),
                            ctx.module_id.clone(),
                            "warn",
                        );
                        emitter.emit(&event).await;
                    }
                }
                return Err(ModuleError::acl_denied(
                    caller_id.as_deref(),
                    &ctx.module_id,
                ));
            }
        }
        Ok(StepResult::continue_step())
    }
}

impl BuiltinApprovalGate {
    /// Emit an `approval_decision` audit/observability event for a resolved
    /// approval. Mirrors apcore-python (`_emit_audit`) and apcore-typescript:
    /// emits a structured `tracing` log and attaches an `approval_decision`
    /// event carrying `{module_id, status, approved_by, reason, approval_id}`
    /// to the active span of this invocation or its enclosing call.
    fn emit_approval_decision(
        ctx: &PipelineContext,
        result: &crate::approval::ApprovalResult,
        audit_id: Option<&str>,
    ) {
        let approved_by = result.approved_by.clone().unwrap_or_default();
        let reason = result.reason.clone().unwrap_or_default();
        let approval_id = audit_id.unwrap_or_default();

        tracing::info!(
            module_id = %ctx.module_id,
            status = %result.status,
            approved_by = %approved_by,
            reason = %reason,
            approval_id = %approval_id,
            "approval_decision"
        );

        crate::observability::tracing_middleware::add_context_span_event(
            &ctx.context,
            "approval_decision",
            serde_json::json!({
                "module_id": ctx.module_id,
                "status": result.status,
                "approved_by": approved_by,
                "reason": reason,
                "approval_id": approval_id,
            }),
        );
    }

    /// Publish `apcore.approval.decision` on the event bus (apcore#77) when an
    /// event emitter is configured. Emitted for every adjudication (handler
    /// decisions AND strict fail-closed rejections). Severity mirrors the
    /// outcome: `approved`/`pending` are `info`; `rejected`/`timeout`
    /// (governance interventions) are `warn`. Emitted ALONGSIDE the log/span.
    async fn emit_approval_decision_event(
        ctx: &PipelineContext,
        result: &crate::approval::ApprovalResult,
        audit_id: Option<&str>,
    ) {
        let Some(emitter) = ctx.event_emitter.clone() else {
            return;
        };
        let severity = if matches!(result.status.as_str(), "approved" | "pending") {
            "info"
        } else {
            "warn"
        };
        let event = ApCoreEvent::with_module(
            "apcore.approval.decision",
            serde_json::json!({
                "module_id": ctx.module_id,
                "status": result.status,
                "approved_by": result.approved_by,
                "reason": result.reason,
                "approval_id": audit_id,
                "trace_id": ctx.context.trace_id,
            }),
            ctx.module_id.clone(),
            severity,
        );
        emitter.emit(&event).await;
    }

    /// Audit a policy-driven override: structured log + active-span event.
    /// Publishes to the bus separately via [`Self::emit_policy_override_event`].
    fn emit_policy_override(ctx: &PipelineContext, decision: &PolicyDecision) {
        let pattern = decision.rule.as_ref().map_or("", |r| r.pattern());
        let reason = decision
            .rule
            .as_ref()
            .and_then(|r| r.reason())
            .unwrap_or_default();

        tracing::info!(
            module_id = %decision.module_id,
            pattern = %pattern,
            requires_approval = decision.requires_approval,
            destructive = decision.destructive,
            needs_approval = decision.needs_approval,
            reason = %reason,
            "policy_override"
        );

        crate::observability::tracing_middleware::add_context_span_event(
            &ctx.context,
            "policy_override",
            serde_json::json!({
                "module_id": decision.module_id,
                "pattern": pattern,
                "requires_approval": decision.requires_approval,
                "destructive": decision.destructive,
                "needs_approval": decision.needs_approval,
                "reason": reason,
            }),
        );
    }

    /// Publish `apcore.policy.override` on the event bus (apcore#77) when an
    /// event emitter is configured. Emitted whenever a policy rule/gate changed
    /// a base governance value (`PolicyDecision::overridden`).
    async fn emit_policy_override_event(ctx: &PipelineContext, decision: &PolicyDecision) {
        let Some(emitter) = ctx.event_emitter.clone() else {
            return;
        };
        let pattern = decision.rule.as_ref().map_or("", |r| r.pattern());
        let reason = decision
            .rule
            .as_ref()
            .and_then(|r| r.reason())
            .unwrap_or_default();
        let event = ApCoreEvent::with_module(
            "apcore.policy.override",
            serde_json::json!({
                "module_id": decision.module_id,
                "pattern": pattern,
                "requires_approval": decision.requires_approval,
                "destructive": decision.destructive,
                "needs_approval": decision.needs_approval,
                "reason": reason,
                "trace_id": ctx.context.trace_id,
            }),
            decision.module_id.clone(),
            "info",
        );
        emitter.emit(&event).await;
    }

    /// Log a fail-loud governance warning once per dedup `key`
    /// (module/kind pair), using the executor-owned dedup set threaded via
    /// `ctx.governance_warned`. Without a set (standalone step) it always warns.
    fn warn_once(ctx: &PipelineContext, key: &str, message: &str) {
        if let Some(warned) = ctx.governance_warned.as_ref() {
            if !warned.lock().insert(key.to_string()) {
                return;
            }
        }
        tracing::warn!("{}", message);
    }

    /// Preserve the full decision for the caller, including the pending
    /// credential at its existing details key for older clients.
    fn approval_error(module_id: &str, result: &crate::approval::ApprovalResult) -> ModuleError {
        let reason = result.reason.as_deref().unwrap_or("no reason given");
        let (code, message) = match result.status.as_str() {
            "rejected" => (
                ErrorCode::ApprovalDenied,
                format!("Approval denied for module '{module_id}': {reason}"),
            ),
            "timeout" => (
                ErrorCode::ApprovalTimeout,
                format!("Approval timed out for module '{module_id}': {reason}"),
            ),
            "pending" => (
                ErrorCode::ApprovalPending,
                format!("Approval pending for module '{module_id}': {reason}"),
            ),
            status => {
                tracing::warn!(%module_id, %status, "Unknown approval status, treating as denied");
                (
                    ErrorCode::ApprovalDenied,
                    format!("Approval denied for module '{module_id}': unknown status '{status}'"),
                )
            }
        };
        let mut error = ModuleError::new(code, message);
        error
            .details
            .insert("result".to_owned(), serde_json::json!(result));
        if code == ErrorCode::ApprovalPending {
            error.details.insert(
                "approval_id".to_owned(),
                serde_json::json!(result.approval_id),
            );
        }
        error
    }
}

#[async_trait]
impl Step for BuiltinApprovalGate {
    impl_step_meta!(BuiltinApprovalGate);

    /// PROTOCOL_SPEC 6.6.5.2 — see [`BuiltinACLCheck::builtin_gate`].
    fn builtin_gate(&self) -> Option<BuiltinGate> {
        Some(BuiltinGate::Approval)
    }
    fn requires(&self) -> &[&str] {
        &["context", "module"]
    }

    #[allow(clippy::too_many_lines)] // approval gate logic is inherently multi-step; splitting would obscure the protocol flow
    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        let approval_token = ctx.take_approval_credential()?;

        let registry = ctx
            .registry
            .as_ref()
            .ok_or_else(|| missing_pipeline_input("registry"))?;

        // Resolve the LIVE module instance. Its annotations are one of the two
        // governance sources the gate unions below (D-96).
        //
        // PROTOCOL_SPEC §7.4 Step 5 binds `annotations = module.annotations`
        // (step 2) and decides the skip from THAT binding (step 3: "IF
        // annotations is null OR annotations.requires_approval is false →
        // SKIP"). This step used to decide from
        // `registry.get_definition(...).annotations` INSTEAD, while the
        // `ApprovalRequest` it builds below read the live module — so a module
        // whose `annotations()` declares `requires_approval: true` executed
        // UNGATED whenever its descriptor omitted the annotation, and a single
        // call could be gated by one source and described by the other.
        // apcore-python (`_module_requires_approval(module)`) and
        // apcore-typescript (`needsApproval(mod)`) both read the live module.
        //
        // `ctx.module` is set by Step 3 (`BuiltinModuleLookup`) — this step
        // declares `module` in `requires()`. The registry fallback keeps the
        // step usable when it is driven directly, outside a full pipeline.
        let module = ctx
            .module
            .clone()
            .or_else(|| registry.get(&ctx.module_id).ok().flatten());

        // D-96: the gate fires on the UNION of its governance sources, not on
        // the module alone. Reading only the descriptor ignored what the module
        // declared (the defect above); reading only the module would ignore what
        // an operator declared in configuration, because this SDK — unlike its
        // peers, whose descriptors are DERIVED from the module — accepts a
        // caller-supplied `ModuleDescriptor`, and `Registry::register` documents
        // it as "loaded from a config file or discovered from an external
        // source". Both single-source readings are fail-OPEN, and on an approval
        // gate the failure direction is the whole point: requiring approval that
        // was not strictly needed costs a prompt, skipping approval that WAS
        // needed is a bypass. Mirrors §6.9, where the requirement is already the
        // union of the annotation, the ACL rule and `gate_destructive`.
        // The union itself lives on `ModuleAnnotations` so the gate, the
        // `ApprovalRequest` it builds below and `Executor::validate`'s preflight
        // all read governance through ONE function — three sites that drifted
        // apart the first time this was fixed inline here.
        let module_annotations = crate::module::ModuleAnnotations::governance_union(
            module.as_ref().map(|m| m.annotations()).as_ref(),
            ctx.declared_module_annotations().as_ref(),
        );

        // Consult the ExecutionPolicy (apcore#76). A matched rule (or
        // gate_destructive) overrides the module's declared governance values.
        //
        // PROTOCOL_SPEC §7.9.6 (v1.24.0, apcore#102): resolution receives the
        // call site — the invocation `arguments` and the `Context` — which are
        // already in scope here because the gate is Step 5. The built-in
        // pattern rules MUST NOT consult them, so no existing verdict moves;
        // the inputs exist so the call site reaches the audit trail (rule 3).
        // These are the business arguments at the gate. Module middleware and
        // schema validation run later, and the resume credential is isolated.
        let decision: Option<PolicyDecision> = ctx.policy.clone().map(|policy| {
            policy.resolve_with_call_site(
                &ctx.module_id,
                module_annotations.as_ref(),
                Some(&ctx.inputs),
                Some(&ctx.context),
            )
        });

        let (policy_effective_approval, effective_destructive) = match &decision {
            Some(d) => (d.needs_approval, d.destructive),
            None => (
                module_annotations
                    .as_ref()
                    .is_some_and(|a| a.requires_approval),
                module_annotations.as_ref().is_some_and(|a| a.destructive),
            ),
        };

        // PROTOCOL_SPEC §7.4 / §6.9 rows 3-5: the gate fires on the UNION of
        // the module's `annotations.requires_approval`, the Step-4 ACL
        // decision, and `gate_destructive`. `policy_effective_approval` already
        // folds the annotation and `gate_destructive` together; the ACL's
        // requirement joins them here.
        //
        // The union is what makes §6.9 row 4 hold: an `ExecutionPolicy`
        // override may ADD a requirement and MUST NOT remove one the ACL set.
        // The ACL is a caller-scoped authorization layer and `ExecutionPolicy`
        // is a module-scoped platform override, so letting the module-scoped
        // one cancel the caller-scoped one is a privilege escalation — a policy
        // rule written for `orders.*` would silently strip a requirement an ACL
        // author attached to one untrusted caller. `policy_effective_approval`
        // is ORed, never assigned over, which is the whole guard: a policy
        // `requires_approval: false` overrides the module's *annotation* and
        // never the ACL's decision.
        let acl_requires_approval = ctx.acl_approval_required;
        let needs_approval = policy_effective_approval || acl_requires_approval;

        // Audit a policy-driven override: log, span, and bus event (apcore#77).
        if let Some(d) = &decision {
            if d.overridden {
                Self::emit_policy_override(ctx, d);
                Self::emit_policy_override_event(ctx, d).await;
            }
        }

        if !needs_approval {
            // Fail-loud governance (apcore#76): a module that is effectively
            // destructive but that no approval gate covers is warned about once.
            if effective_destructive {
                Self::warn_once(
                    ctx,
                    &format!("destructive_ungated:{}", ctx.module_id),
                    &format!(
                        "Module '{}' is annotated destructive=true but is not covered by the \
                         approval gate (requires_approval is false and no policy gates \
                         destructive modules). Consider requires_approval=true or \
                         ExecutionPolicy(gate_destructive=true). (apcore#76)",
                        ctx.module_id
                    ),
                );
            }
            return Ok(StepResult::continue_step());
        }

        // The module needs approval. Resolve the handler; when none is
        // configured, FLIP the historical silent-skip to fail-loud (apcore#76).
        let Some(handler) = ctx.approval_handler.clone() else {
            if ctx.policy.as_ref().is_some_and(|p| p.strict) {
                // strict=true fails CLOSED: build a rejected result, audit it,
                // and deny.
                let result = crate::approval::ApprovalResult {
                    status: "rejected".to_string(),
                    reason: Some(
                        "Approval required but no ApprovalHandler is configured; \
                         ExecutionPolicy(strict=true) fails closed"
                            .to_string(),
                    ),
                    ..Default::default()
                };
                Self::emit_approval_decision(ctx, &result, None);
                Self::emit_approval_decision_event(ctx, &result, None).await;
                return Err(Self::approval_error(&ctx.module_id, &result));
            }
            // Non-strict: keep the PROTOCOL_SPEC §7.4 skip behavior but warn
            // once per module (fail-loud, not silent). Emits no decision event
            // — parity with the "no audit log when gate skipped" contract.
            Self::warn_once(
                ctx,
                &format!("no_handler:{}", ctx.module_id),
                &format!(
                    "Module '{}' requires approval but no ApprovalHandler is configured; \
                     the approval gate is skipped per PROTOCOL_SPEC §7.4. Configure an \
                     approval handler, or ExecutionPolicy(strict=true) to fail closed. \
                     (apcore#76)",
                    ctx.module_id
                ),
            );
            return Ok(StepResult::continue_step());
        };

        let module = module.ok_or_else(|| ModuleError::module_not_found(&ctx.module_id))?;
        let mut annotations = module_annotations.unwrap_or_default();
        annotations.requires_approval = true;
        annotations.destructive = effective_destructive;
        let description = module.description();
        let request = crate::approval::ApprovalRequest {
            module_id: ctx.module_id.clone(),
            arguments: ctx.inputs.clone(),
            context: Some(ctx.context.clone()),
            annotations,
            caller_id: ctx.context.caller_id.clone(),
            action: ctx.module_id.clone(),
            description: (!description.is_empty()).then(|| description.to_owned()),
            tags: module.tags(),
        };
        let approval_result = if let Some(credential) = approval_token.as_deref() {
            handler
                .check_approval_for_call(credential, &request)
                .await?
        } else {
            handler.request_approval(&request).await?
        };
        let audit_id = approval_result.observable_id(approval_token.as_deref());
        Self::emit_approval_decision(ctx, &approval_result, audit_id);
        Self::emit_approval_decision_event(ctx, &approval_result, audit_id).await;

        if approval_result.status != "approved" {
            return Err(Self::approval_error(&ctx.module_id, &approval_result));
        }

        Ok(StepResult::continue_step())
    }
}

#[async_trait]
impl Step for BuiltinInputValidation {
    impl_step_meta!(BuiltinInputValidation);
    fn requires(&self) -> &[&str] {
        &["module"]
    }
    fn provides(&self) -> &[&str] {
        &["validated_inputs"]
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        let module = ctx
            .module
            .as_ref()
            .ok_or_else(|| missing_pipeline_input("module"))?;
        let input_schema = module.input_schema();
        validate_against_schema(&ctx.inputs, &input_schema, "Input")?;

        let mut captured = capture_redacted(&ctx.inputs, &input_schema, ctx);
        captured.remove("_approval_token");
        ctx.context.redacted_inputs = Some(captured);
        ctx.validated_inputs = Some(ctx.inputs.clone());
        Ok(StepResult::continue_step())
    }
}

#[async_trait]
impl Step for BuiltinMiddlewareBefore {
    impl_step_meta!(BuiltinMiddlewareBefore);

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        let middleware_manager = match ctx.middleware_manager {
            Some(ref mm) => mm.clone(),
            None => return Ok(StepResult::continue_step()),
        };

        let snapshot = ctx
            .middleware_snapshot
            .get_or_insert_with(|| middleware_manager.execution_snapshot());
        let (modified_inputs, executed) =
            match crate::middleware::manager::MiddlewareManager::execute_before_snapshot(
                snapshot,
                &ctx.module_id,
                ctx.inputs.clone(),
                &ctx.context,
            )
            .await
            {
                Ok((inputs, executed)) => (inputs, executed),
                Err(e) => {
                    // Recover the list of middlewares that ran during execute_before
                    // from the MiddlewareChainError details so on_error runs over
                    // exactly those (mirrors Python builtin_steps.py:483 setting
                    // ctx.executed_middlewares = exc.executed_middlewares and TS
                    // builtin-steps.ts:442) — sync finding A-D-01. Without this the
                    // recovery middleware's on_error is skipped (empty &[]) and the
                    // executor-level on_error/RetrySignal loop is bypassed.
                    let executed: Vec<usize> = e
                        .details
                        .get("executed_middlewares")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    ctx.executed_middlewares.clone_from(&executed);
                    // A-D-010 / A-D-012: do NOT run step-level on_error recovery
                    // here. Record which middlewares ran (above) and propagate the
                    // error so the executor-level loop is the SOLE on_error site —
                    // it runs `execute_on_error_outcome` over `executed` and honors
                    // both Recovery and Retry. Step-level `execute_on_error` mapped
                    // RetrySignal→None (dropping retries) and, combined with the
                    // executor pass, fired on_error twice. Mirrors apcore-python
                    // (chain errors store executed_middlewares + raise) and
                    // apcore-typescript.
                    return Err(e);
                }
            };
        ctx.inputs = modified_inputs;
        ctx.executed_middlewares = executed;
        Ok(StepResult::continue_step())
    }
}

/// Resolve the effective per-call timeout in milliseconds.
///
/// Starts from the module's declared `annotations.extra["resources"]["timeout"]`
/// (spec D-11), falls back to `config.executor.default_timeout`, then clamps
/// against whatever remains of `global_deadline` — the dual-timeout model in
/// docs/features/core-executor.md §Step 8. `0` means "no timeout".
///
/// Shared by `BuiltinExecute` and `Executor::stream`'s non-streaming fallback.
/// The streaming path drives `run_until_step(.., "execute")`, so it never
/// enters `BuiltinExecute`; before this was extracted, that fallback awaited
/// `module.execute()` bare, with neither the per-module timeout nor the
/// deadline clamp, while both peer SDKs run the full pipeline and get both.
pub(crate) fn resolve_effective_timeout_ms(
    annotations: Option<&crate::module::ModuleAnnotations>,
    module_id: &str,
    global_deadline: Option<f64>,
    default_timeout: u64,
) -> Result<u64, ModuleError> {
    let declared_timeout = annotations
        .and_then(|ann| ann.extra.get("resources"))
        .and_then(|resources| resources.get("timeout"));
    // A negative declared timeout is invalid and MUST be rejected rather than
    // silently swallowed (`as_u64()` would return None and fall back to the
    // default). Spec Edge Cases table; peer Python builtin_steps.py:620 raises
    // InvalidInputError (sync finding A-D-W1).
    if let Some(v) = declared_timeout {
        let is_negative = v.as_i64().is_some_and(|n| n < 0) || v.as_f64().is_some_and(|f| f < 0.0);
        if is_negative {
            return Err(ModuleError::new(
                ErrorCode::GeneralInvalidInput,
                format!(
                    "Module '{module_id}' declares a negative timeout ({v}); timeout must be non-negative"
                ),
            ));
        }
    }
    let per_module_timeout_ms = declared_timeout.and_then(Value::as_u64);
    let mut timeout_ms = per_module_timeout_ms.unwrap_or(default_timeout);
    if let Some(deadline) = global_deadline {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // intentional: remaining_ms is non-negative and fits in u64 for any reasonable deadline
        let remaining_ms = ((deadline - now) * 1000.0) as u64;
        if remaining_ms == 0 {
            let retryable = annotations.is_some_and(|ann| ann.idempotent);
            return Err(ModuleError::new(
                ErrorCode::ModuleTimeout,
                format!("Module '{module_id}' execution aborted: global deadline already exceeded"),
            )
            .with_retryable(retryable));
        }
        if timeout_ms == 0 || remaining_ms < timeout_ms {
            timeout_ms = remaining_ms;
        }
    }
    Ok(timeout_ms)
}

#[async_trait]
impl Step for BuiltinExecute {
    impl_step_meta!(BuiltinExecute);
    fn requires(&self) -> &[&str] {
        &["module"]
    }
    fn provides(&self) -> &[&str] {
        &["output"]
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        let module = ctx
            .module
            .as_ref()
            .ok_or_else(|| missing_pipeline_input("module"))?
            .clone();
        if ctx.stream.is_some() {
            let producer = crate::executor::open_stream(ctx, &module).await?;
            if let (Some(stream), Some(producer)) = (&mut ctx.stream, producer) {
                stream.producer = Some(parking_lot::Mutex::new(producer));
                return Ok(StepResult::continue_step());
            }
        }
        let config = ctx
            .config
            .as_ref()
            .ok_or_else(|| missing_pipeline_input("config"))?;

        // Compute effective timeout — start from per-module override
        // (`annotations.extra["resources"]["timeout"]`, in milliseconds; spec
        // D-11), fall back to `config.executor.default_timeout`. Both are
        // then clamped against the remaining global deadline below.
        // Spec: docs/features/core-executor.md §Step 8 (dual-timeout model).
        let timeout_ms = match resolve_effective_timeout_ms(
            ctx.declared_module_annotations().as_ref(),
            &ctx.module_id,
            ctx.context.global_deadline,
            config.executor.default_timeout,
        ) {
            Ok(timeout) => timeout,
            Err(error) => {
                if matches!(error.code, ErrorCode::ModuleTimeout) {
                    if let Some(token) = &ctx.context.cancel_token {
                        token.cancel_after_timeout(&error);
                    }
                }
                return Err(error);
            }
        };

        // Cancellation check immediately before invoking the module
        // (D-21 — `Cancel Token Mid-Pipeline Check`, point 2). Acts as a
        // defensive backstop for tokens that became cancelled while earlier
        // steps were running, ensuring `cancel()` interrupts the in-flight
        // pipeline rather than handing control to a doomed module. Spec:
        // docs/features/core-executor.md §Cancel Token Mid-Pipeline Check.
        if let Some(token) = ctx.context.cancel_token.as_ref() {
            token.check_for(&ctx.module_id)?;
        }

        let execute_result = if timeout_ms > 0 {
            // Own the invocation: timeout must not drop module code before cleanup.
            let inputs = ctx.inputs.clone();
            let context = ctx.context.clone();
            let token = context.cancel_token.clone();
            // Read the retry-safety flag before `module` moves into the task.
            let retryable = module.annotations().idempotent;
            let lease = ctx.module_lease.clone();
            let execution = async move {
                let _lease = lease;
                let result = module.execute(inputs, &context).await;
                if let Err(error) = &result {
                    tracing::debug!(
                        code = %error.wire_code(),
                        "Module invocation finished with an error"
                    );
                }
                result
            };
            let invocation = if let Some(token) = token {
                token.spawn_tracked(execution)
            } else {
                tokio::spawn(execution)
            };
            match tokio::time::timeout(Duration::from_millis(timeout_ms), invocation).await {
                Ok(Ok(result)) => result,
                Ok(Err(error)) => Err(ModuleError::new(
                    ErrorCode::GeneralInternalError,
                    format!("Module execution task failed: {error}"),
                )),
                Err(_elapsed) => {
                    let error = ModuleError::module_timeout(&ctx.module_id, timeout_ms)
                        .with_retryable(retryable);
                    if let Some(token) = &ctx.context.cancel_token {
                        token.cancel_after_timeout(&error);
                    }
                    Err(error)
                }
            }
        } else {
            module.execute(ctx.inputs.clone(), &ctx.context).await
        };

        match execute_result {
            Ok(output) if output.is_object() => {
                ctx.output = Some(output);
                Ok(StepResult::continue_step())
            }
            Ok(Value::Null) => Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "Return value cannot be None",
            )),
            Ok(_) => Err(ModuleError::new(
                ErrorCode::ModuleExecuteError,
                "Return value must be Map",
            )),
            Err(e) => {
                // Cancellation short-circuit (D-20): an ExecutionCancelled
                // error MUST bypass the on_error middleware chain even at the
                // step level, so a recovering on_error middleware cannot
                // swallow it into a success before reaching the executor-level
                // guard in `Executor::call` / `call_with_trace`. Propagate it
                // untouched. Mirrors apcore-python `_handle_error`
                // (`_unwrap_cancellation` before any on_error recovery).
                // Spec: docs/features/core-executor.md §Cancellation Short-Circuit
                if matches!(e.code, ErrorCode::ExecutionCancelled) {
                    return Err(e);
                }
                // A-D-010 / A-D-012: do NOT run step-level on_error recovery
                // here. The executor-level loop is the sole recovery site (it
                // runs `execute_on_error_outcome` over `ctx.executed_middlewares`
                // and honors Recovery + Retry). Step-level recovery double-fired
                // on_error and silently dropped RetrySignals. Mirrors
                // apcore-python `BuiltinExecute` (raises; no step-level on_error)
                // and apcore-typescript.
                Err(e)
            }
        }
    }
}

#[async_trait]
impl Step for BuiltinOutputValidation {
    impl_step_meta!(BuiltinOutputValidation);
    fn requires(&self) -> &[&str] {
        &["module", "output"]
    }
    fn provides(&self) -> &[&str] {
        &["validated_output"]
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        // In dry_run mode, execute step is skipped so output may be absent.
        let Some(output) = ctx.output.as_ref() else {
            return Ok(StepResult::continue_step());
        };
        let module = ctx
            .module
            .as_ref()
            .ok_or_else(|| missing_pipeline_input("module"))?;
        let output_schema = module.output_schema();
        validate_against_schema(output, &output_schema, "Output")?;

        ctx.context.redacted_output = Some(capture_redacted(output, &output_schema, ctx));
        ctx.validated_output.clone_from(&ctx.output);
        Ok(StepResult::continue_step())
    }
}

#[async_trait]
impl Step for BuiltinMiddlewareAfter {
    impl_step_meta!(BuiltinMiddlewareAfter);

    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        let middleware_manager = match ctx.middleware_manager {
            Some(ref mm) => mm.clone(),
            None => return Ok(StepResult::continue_step()),
        };

        // BuiltinExecute normally sets ctx.output before this step, but a custom
        // strategy may reach here with no output (e.g. execute step omitted or
        // skipped). Match Python/TS: pass through instead of panicking.
        let Some(output) = ctx.output.take() else {
            return Ok(StepResult::continue_step());
        };
        if ctx.context.redacted_output.is_none() {
            if let Some(module) = &ctx.module {
                ctx.context.redacted_output =
                    Some(capture_redacted(&output, &module.output_schema(), ctx));
            }
        }

        let snapshot = ctx
            .middleware_snapshot
            .get_or_insert_with(|| middleware_manager.execution_snapshot());
        match crate::middleware::manager::MiddlewareManager::execute_after_snapshot(
            snapshot,
            &ctx.module_id,
            ctx.inputs.clone(),
            output,
            &ctx.context,
        )
        .await
        {
            Ok(modified_output) => {
                ctx.output = Some(modified_output);
                Ok(StepResult::continue_step())
            }
            Err(e) => {
                // A-D-010 / A-D-012: do NOT run step-level on_error here. The
                // sole on_error recovery site is the executor-level loop
                // (`Executor::call`), which runs `execute_on_error_outcome` over
                // `ctx.executed_middlewares` and honors both Recovery and Retry
                // outcomes. Running it at the step level too made on_error fire
                // twice on an after()-failure (step + executor) and dropped any
                // RetrySignal (step-level `execute_on_error` maps Retry→None).
                // Mirrors apcore-python `BuiltinMiddlewareAfter` (no step-level
                // recovery — it raises) and apcore-typescript. The before-step
                // already populated `ctx.executed_middlewares`, so the executor
                // recovers over exactly the middlewares that ran.
                Err(e)
            }
        }
    }
}

#[async_trait]
impl Step for BuiltinReturnResult {
    impl_step_meta!(BuiltinReturnResult);
    fn requires(&self) -> &[&str] {
        &["output"]
    }

    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        // Output is already stored in ctx.output; PipelineEngine returns it.
        Ok(StepResult::continue_step())
    }
}

// ---------------------------------------------------------------------------
// Preset strategies
// ---------------------------------------------------------------------------

/// Build the standard 11-step execution strategy.
///
/// The `module_lookup` step defaults to the process-global toggle store. To
/// bind a per-`APCore` toggle store (issue #71) use
/// [`build_standard_strategy_with_toggle`].
#[must_use]
pub fn build_standard_strategy() -> ExecutionStrategy {
    build_standard_strategy_with_toggle(crate::sys_modules::global_toggle_state_arc())
}

/// Build the standard 11-step execution strategy bound to a specific toggle
/// store for the `module_lookup` read path (issue #71).
///
/// `APCore::with_options` passes its own `Arc<ToggleState>` here so the
/// pipeline observes per-instance toggles rather than the process-global store.
#[must_use]
pub fn build_standard_strategy_with_toggle(
    toggle_state: std::sync::Arc<crate::sys_modules::ToggleState>,
) -> ExecutionStrategy {
    // INVARIANT: all step names are unique literals, so `new()` cannot fail.
    ExecutionStrategy::new(
        "standard",
        vec![
            Box::new(BuiltinContextCreation),
            Box::new(BuiltinCallChainGuard),
            Box::new(BuiltinModuleLookup::new(toggle_state)),
            Box::new(BuiltinACLCheck),
            Box::new(BuiltinApprovalGate),
            Box::new(BuiltinMiddlewareBefore),
            Box::new(BuiltinInputValidation),
            Box::new(BuiltinExecute),
            Box::new(BuiltinOutputValidation),
            Box::new(BuiltinMiddlewareAfter),
            Box::new(BuiltinReturnResult),
        ],
    )
    // INVARIANT: the builtin step list above uses distinct types — duplicates would be a compile-time error.
    .expect("standard strategy should have unique step names")
}

/// Build the internal strategy (standard minus `acl_check` and `approval_gate`).
#[must_use]
pub fn build_internal_strategy() -> ExecutionStrategy {
    let mut s = build_standard_strategy();
    s.set_name("internal");
    s.remove("acl_check").ok();
    s.remove("approval_gate").ok();
    s
}

/// Build the testing strategy (standard minus `acl_check`, `approval_gate`, and `call_chain_guard`).
#[must_use]
pub fn build_testing_strategy() -> ExecutionStrategy {
    let mut s = build_standard_strategy();
    s.set_name("testing");
    s.remove("acl_check").ok();
    s.remove("approval_gate").ok();
    s.remove("call_chain_guard").ok();
    s
}

/// Build the performance strategy (standard minus `middleware_before` and `middleware_after`).
#[must_use]
pub fn build_performance_strategy() -> ExecutionStrategy {
    let mut s = build_standard_strategy();
    s.set_name("performance");
    s.remove("middleware_before").ok();
    s.remove("middleware_after").ok();
    s
}

/// Build a minimal strategy: context → lookup → execute → return only.
///
/// No safety checks, no ACL, no approval, no validation, no middleware.
/// Suitable for pre-validated internal hot paths. Use with caution.
#[must_use]
pub fn build_minimal_strategy() -> ExecutionStrategy {
    let mut s = build_standard_strategy();
    s.set_name("minimal");
    s.remove("call_chain_guard").ok();
    s.remove("acl_check").ok();
    s.remove("approval_gate").ok();
    s.remove("middleware_before").ok();
    s.remove("input_validation").ok();
    s.remove("output_validation").ok();
    s.remove("middleware_after").ok();
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::context::Identity;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_input_validation_requires_module_context() {
        let mut ctx = PipelineContext::new(
            "sample.module",
            serde_json::json!({}),
            Context::anonymous(),
            "test",
        );
        let error = BuiltinInputValidation.execute(&mut ctx).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::PipelineDependencyError);
    }

    struct PendingModule {
        started: Arc<tokio::sync::Notify>,
        cleaned: Arc<std::sync::atomic::AtomicBool>,
        finish: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl crate::module::Module for PendingModule {
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn output_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn description(&self) -> &'static str {
            "Wait until cancelled"
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _context: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, ModuleError> {
            struct Cleanup(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    self.0.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            let _cleanup = Cleanup(self.cleaned.clone());
            self.started.notify_one();
            self.finish.notified().await;
            Ok(serde_json::json!({}))
        }
    }

    #[tokio::test]
    async fn test_executor_timeout_tracks_cleanup_and_manager_cancel_drains() {
        use std::sync::atomic::{AtomicBool, Ordering};
        for timeout in [false, true] {
            let started = Arc::new(tokio::sync::Notify::new());
            let cleaned = Arc::new(AtomicBool::new(false));
            let finish = Arc::new(tokio::sync::Notify::new());
            let registry = Arc::new(crate::registry::registry::Registry::new());
            registry
                .register_module(
                    "pending",
                    Box::new(PendingModule {
                        started: started.clone(),
                        cleaned: cleaned.clone(),
                        finish: finish.clone(),
                    }),
                )
                .unwrap();
            let mut config = crate::config::Config::default();
            config.set(
                "executor.default_timeout",
                serde_json::json!(if timeout { 20 } else { 60_000 }),
            );
            let executor = Arc::new(crate::executor::Executor::new(registry, Arc::new(config)));
            if timeout {
                let token = crate::cancel::CancelToken::new();
                let mut context = Context::anonymous();
                context.cancel_token = Some(token.clone());
                let invocation = tokio::spawn(async move {
                    executor
                        .call("pending", serde_json::json!({}), Some(&context), None)
                        .await
                });
                started.notified().await;
                assert_eq!(
                    invocation.await.unwrap().unwrap_err().code,
                    ErrorCode::ModuleTimeout
                );
                assert!(
                    !cleaned.load(Ordering::Acquire),
                    "deadline must return before cooperative cleanup"
                );
                assert!(token.has_active_executions());
                finish.notify_one();
                token.drain().await;
            } else {
                let manager = crate::async_task::AsyncTaskManager::new(executor, 1, 100);
                let task_id = manager
                    .submit("pending", serde_json::json!({}), None)
                    .await
                    .unwrap();
                started.notified().await;
                assert!(manager.cancel(&task_id).await.unwrap());
            }
            assert!(
                cleaned.load(Ordering::Acquire),
                "timeout={timeout}: module future outlived its owner"
            );
        }
    }

    #[tokio::test]
    async fn safe_unregister_waits_for_timed_out_module_to_finish() {
        let started = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        let registry = Arc::new(crate::registry::registry::Registry::new());
        registry
            .register_module(
                "pending",
                Box::new(PendingModule {
                    started: started.clone(),
                    finish: finish.clone(),
                    cleaned: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                }),
            )
            .unwrap();
        let mut config = crate::config::Config::default();
        config.set("executor.default_timeout", serde_json::json!(20));
        let executor = crate::executor::Executor::new(registry.clone(), Arc::new(config));
        let invocation = tokio::spawn(async move {
            executor
                .call("pending", serde_json::json!({}), None, None)
                .await
        });
        started.notified().await;
        assert_eq!(
            invocation.await.unwrap().unwrap_err().code,
            ErrorCode::ModuleTimeout
        );

        let drain = registry.safe_unregister("pending", 5000);
        tokio::pin!(drain);
        assert!(tokio::time::timeout(Duration::from_millis(5), &mut drain)
            .await
            .is_err());
        assert!(registry.is_draining("pending"));
        finish.notify_one();
        assert!(drain.await.unwrap());
        assert!(registry.get("pending").unwrap().is_none());
    }

    fn empty_context() -> PipelineContext {
        let identity = Identity::new(
            "user-1".to_string(),
            "user".to_string(),
            vec![],
            HashMap::new(),
        );
        let context: Context<serde_json::Value> = Context::new(identity);
        PipelineContext::new("mod-x", serde_json::json!({}), context, "custom")
    }

    // [exec-none-output-panic] A custom strategy may reach the after-steps with
    // ctx.output == None. Python/TS pass through; Rust must not panic.
    #[tokio::test]
    async fn output_validation_passes_through_when_output_none() {
        let step = BuiltinOutputValidation;
        let mut ctx = empty_context();
        assert!(ctx.output.is_none());
        let result = step.execute(&mut ctx).await;
        assert!(result.is_ok(), "must not panic/err when output is None");
    }

    #[tokio::test]
    async fn middleware_after_passes_through_when_output_none() {
        use crate::middleware::manager::MiddlewareManager;
        use std::sync::Arc;

        let step = BuiltinMiddlewareAfter;
        let mut ctx = empty_context();
        // Attach a manager so the step reaches the output `take()` path; with
        // output None it must pass through (continue) rather than panic.
        ctx.middleware_manager = Some(Arc::new(MiddlewareManager::new()));
        assert!(ctx.output.is_none());
        let result = step.execute(&mut ctx).await;
        assert!(result.is_ok(), "must not panic/err when output is None");
    }
}
