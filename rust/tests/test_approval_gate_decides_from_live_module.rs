// Regression test: `BuiltinApprovalGate` fires on the UNION of its two
// governance sources — the live module instance's `annotations()` and the
// registry descriptor's — and NOT on either one alone (D-96).
//
// PROTOCOL_SPEC §7.4 Step 5 binds `annotations = module.annotations` (step 2)
// and skips on `annotations is null OR annotations.requires_approval is false`
// (step 3) — step 3 tests the binding step 2 established, which is the MODULE.
// apcore-python (`builtin_steps.py`, `_module_requires_approval(module)`) and
// apcore-typescript (`builtin-steps.ts`, `needsApproval(mod)`) both read it.
//
// The defect this pins: the gate-firing decision was read from
// `registry.get_definition(module_id).annotations` while the `ApprovalRequest`
// handed to the handler was read from the live module. A module declaring
// `requires_approval: true` in `annotations()` but registered with a
// descriptor that omits it therefore EXECUTED UNGATED on Rust while both peer
// SDKs gated it.
//
// Why the fix is a union and not a swap, which the second test pins: reading
// only the module inverts the bypass instead of closing it. The Rust-only
// three-argument `Registry::register(module_id, module, descriptor)` documents
// descriptors as "loaded from a config file or discovered from an external
// source", so a requirement an operator declares THERE would stop gating.
// Both single-source readings are fail-OPEN, and on an approval gate that
// direction is the whole argument — prompting for an approval that was not
// needed costs a prompt; skipping one that was needed is a bypass.
//
// `tests/conformance_test.rs` (the `approval_gate` fixture driver) could not
// see any of this until the fixture split `module_requires_approval` into
// `governance_sources: {module, descriptor}`: it set BOTH from one boolean, so
// no case could tell the two sources apart. These tests make them disagree
// deliberately, in both directions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::executor::Executor;
use apcore::module::{Module, ModuleAnnotations};
use apcore::pipeline::{PipelineContext, Step};
use apcore::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use apcore::BuiltinApprovalGate;

/// A module whose LIVE `annotations()` declares `requires_approval: true`.
#[derive(Debug)]
struct DeclaresApprovalModule;

#[async_trait]
impl Module for DeclaresApprovalModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Module that declares requires_approval on the instance"
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: true,
            destructive: true,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(inputs)
    }
}

/// Counts handler invocations and auto-approves.
#[derive(Debug)]
struct CountingHandler {
    invocations: Arc<AtomicUsize>,
    captured_destructive: Arc<AtomicUsize>,
}

#[async_trait]
impl ApprovalHandler for CountingHandler {
    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        if request.annotations.destructive {
            self.captured_destructive.fetch_add(1, Ordering::SeqCst);
        }
        let mut result = ApprovalResult::default();
        result.status = "approved".to_string();
        result.approved_by = Some("counter".to_string());
        Ok(result)
    }

    async fn check_approval(&self, _approval_id: &str) -> Result<ApprovalResult, ModuleError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let mut result = ApprovalResult::default();
        result.status = "approved".to_string();
        Ok(result)
    }
}

/// A descriptor that carries NO governance annotations at all — the shape
/// produced by any registration path that does not copy the module's
/// declaration into the descriptor.
fn descriptor_without_annotations(module_id: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: "Module that declares requires_approval on the instance".to_string(),
        documentation: None,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        version: DEFAULT_MODULE_VERSION.to_string(),
        tags: vec![],
        // The whole point: the descriptor says nothing about approval.
        annotations: None,
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

/// The mirror image of `DeclaresApprovalModule`: declares no governance at all,
/// so the only source of a requirement is the descriptor.
struct DeclaresNothingModule;

#[async_trait]
impl Module for DeclaresNothingModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Module that declares no governance annotations"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(inputs)
    }
}

/// A descriptor that DOES declare the requirement, paired with a module that
/// does not — the configuration-supplied governance case.
fn descriptor_with_approval(module_id: &str) -> ModuleDescriptor {
    let mut d = descriptor_without_annotations(module_id);
    d.description = "Module whose approval requirement lives only in the descriptor".to_string();
    d.annotations = Some(ModuleAnnotations {
        requires_approval: true,
        ..Default::default()
    });
    d
}

/// D-96: the gate fires on the UNION of its governance sources.
///
/// The fix for the descriptor-only-read defect must not become a module-only
/// read, which would simply invert the bypass. This SDK — unlike its peers,
/// whose descriptors are DERIVED from the module — accepts a caller-supplied
/// `ModuleDescriptor`, and `Registry::register` documents it as "loaded from a
/// config file or discovered from an external source". An operator who declares
/// the requirement THERE must still get a gate.
#[tokio::test]
async fn gate_fires_on_descriptor_annotations_when_the_module_declares_none() {
    let module_id = "executor.test.approval_from_descriptor";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(DeclaresNothingModule),
            descriptor_with_approval(module_id),
        )
        .expect("register module");

    let invocations = Arc::new(AtomicUsize::new(0));
    let captured_destructive = Arc::new(AtomicUsize::new(0));
    let handler = Arc::new(CountingHandler {
        invocations: Arc::clone(&invocations),
        captured_destructive: Arc::clone(&captured_destructive),
    });

    let context = Context::<Value>::anonymous();
    let mut ctx = PipelineContext::new(module_id, json!({}), context, "standard");
    ctx.registry = Some(Arc::clone(&registry));
    ctx.approval_handler = Some(handler);

    let result = BuiltinApprovalGate.execute(&mut ctx).await;
    assert!(result.is_ok(), "approved gate should continue: {result:?}");

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "the gate MUST fire on a requirement declared only by the descriptor (D-96 union). \
         Reading the module alone inverts the original bypass instead of closing it: both \
         single-source readings are fail-OPEN, and skipping an approval that was needed is \
         strictly worse than prompting for one that was not."
    );
}

/// Step-level: drive `BuiltinApprovalGate` directly.
#[tokio::test]
async fn gate_fires_on_live_module_annotations_when_descriptor_omits_them() {
    let module_id = "executor.test.declares_approval";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(DeclaresApprovalModule),
            descriptor_without_annotations(module_id),
        )
        .expect("register module");

    let invocations = Arc::new(AtomicUsize::new(0));
    let captured_destructive = Arc::new(AtomicUsize::new(0));
    let handler = Arc::new(CountingHandler {
        invocations: Arc::clone(&invocations),
        captured_destructive: Arc::clone(&captured_destructive),
    });

    let context = Context::<Value>::anonymous();
    let mut ctx = PipelineContext::new(module_id, json!({}), context, "standard");
    ctx.registry = Some(Arc::clone(&registry));
    ctx.approval_handler = Some(handler);

    let result = BuiltinApprovalGate.execute(&mut ctx).await;
    assert!(result.is_ok(), "approved gate should continue: {result:?}");

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "the gate MUST fire on the live module's annotations.requires_approval \
         even when the registry descriptor carries none (PROTOCOL_SPEC §7.4 Step 5)"
    );
    assert_eq!(
        captured_destructive.load(Ordering::SeqCst),
        1,
        "the live module's annotations.destructive MUST reach the ApprovalRequest"
    );
}

/// End-to-end: the same divergence through the real pipeline, where
/// `ctx.module` is populated by Step 3 (`BuiltinModuleLookup`). This is the
/// actual approval bypass — the module executed and returned its output
/// without any handler ever being consulted.
#[tokio::test]
async fn executor_gates_on_live_module_annotations_when_descriptor_omits_them() {
    let module_id = "executor.test.declares_approval_e2e";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(DeclaresApprovalModule),
            descriptor_without_annotations(module_id),
        )
        .expect("register module");

    let invocations = Arc::new(AtomicUsize::new(0));
    let captured_destructive = Arc::new(AtomicUsize::new(0));

    let mut executor = Executor::new(Arc::clone(&registry), Arc::new(Config::default()));
    executor.set_approval_handler(Box::new(CountingHandler {
        invocations: Arc::clone(&invocations),
        captured_destructive: Arc::clone(&captured_destructive),
    }));

    let outcome = executor.call(module_id, json!({"v": 1}), None, None).await;
    assert!(
        outcome.is_ok(),
        "the handler approves, so the call proceeds: {outcome:?}"
    );

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "a module declaring requires_approval on its instance MUST NOT execute \
         ungated just because its registry descriptor omits the annotation"
    );
}

// ===========================================================================
// Review follow-up — the union has to reach every READER, not just the gate
// ===========================================================================
//
// The D-96 fix above unioned the two governance sources at the point where the
// gate DECIDES. Three other sites kept reading one source directly, so the gate
// fired on the union and the call was then described from something narrower:
//
//   * the `ApprovalRequest` handed to the handler (`builtin_steps.rs`),
//   * `Executor::validate`'s preflight verdict (`executor.rs`),
//   * `Executor::governance_state`'s control-module posture (`executor.rs`).
//
// All three now go through `ModuleAnnotations::governance_union`, which exists
// as one function for exactly this reason: an inline union is a union at one
// site, and the readers are where it drifts. The tests below are what makes
// that structural claim enforceable — each fails if its site goes back to
// reading a single source.

/// Declares the requirement but NOT the risk: `requires_approval` without
/// `destructive`. Pairs with `descriptor_with_destructive` so the gate fires
/// from the module while the risk flag exists only on the descriptor.
#[derive(Debug)]
struct DeclaresApprovalNotDestructiveModule;

#[async_trait]
impl Module for DeclaresApprovalNotDestructiveModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Module that declares requires_approval but not destructive"
    }
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: true,
            destructive: false,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(inputs)
    }
}

/// A descriptor whose only governance claim is `destructive: true` — the
/// operator-supplied risk marking for a module that does not self-declare it.
fn descriptor_with_destructive(module_id: &str) -> ModuleDescriptor {
    let mut d = descriptor_without_annotations(module_id);
    d.description = "Module whose destructive marking lives only in the descriptor".to_string();
    d.annotations = Some(ModuleAnnotations {
        destructive: true,
        ..Default::default()
    });
    d
}

/// The handler must be told what the gate fired on.
///
/// The gate fires from the module's `requires_approval`; the descriptor is the
/// only source of `destructive: true`. Rebuilding the request from
/// `module.annotations()` handed the handler `destructive: false` for a call
/// the operator marked high-risk — and a handler that routes by risk
/// (auto-approve the safe ones, escalate the rest) then takes the low-risk
/// path. Gating on one source and describing from another is the shape D-96
/// exists to close; it survived the original fix because the union landed on
/// the decision and the request was left reading the instance.
#[tokio::test]
async fn approval_request_carries_the_descriptor_half_of_the_union() {
    let module_id = "executor.test.destructive_from_descriptor";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(DeclaresApprovalNotDestructiveModule),
            descriptor_with_destructive(module_id),
        )
        .expect("register module");

    let invocations = Arc::new(AtomicUsize::new(0));
    let captured_destructive = Arc::new(AtomicUsize::new(0));
    let handler = Arc::new(CountingHandler {
        invocations: Arc::clone(&invocations),
        captured_destructive: Arc::clone(&captured_destructive),
    });

    let context = Context::<Value>::anonymous();
    let mut ctx = PipelineContext::new(module_id, json!({}), context, "standard");
    ctx.registry = Some(Arc::clone(&registry));
    ctx.approval_handler = Some(handler);

    let result = BuiltinApprovalGate.execute(&mut ctx).await;
    assert!(result.is_ok(), "approved gate should continue: {result:?}");

    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "the module's own requires_approval must still fire the gate"
    );
    assert_eq!(
        captured_destructive.load(Ordering::SeqCst),
        1,
        "the ApprovalRequest MUST carry the UNION's destructive flag. The gate \
         fired on the union; handing the handler the live module's narrower \
         reading tells it this call is low-risk when the operator marked it \
         high-risk, which is the same gate/description split D-96 closed."
    );
}

/// §7.9.5: the preflight reports the verdict the Step-5 gate will enforce.
///
/// `validate()` took the descriptor only when no live module resolved — an
/// `or_else`, not a union — so a descriptor-only `requires_approval: true` was
/// reported as "no approval needed" for every module that DID resolve, and the
/// caller discovered otherwise at the gate. That is precisely the disagreement
/// the preflight exists to prevent.
#[tokio::test]
async fn preflight_reports_a_descriptor_only_approval_requirement() {
    let module_id = "executor.test.preflight_descriptor_approval";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(DeclaresNothingModule),
            descriptor_with_approval(module_id),
        )
        .expect("register module");

    let executor = Executor::new(registry, Arc::new(Config::default()));
    let report = executor
        .validate(module_id, &json!({}), None)
        .await
        .expect("validate is non-throwing");

    assert!(
        report.requires_approval,
        "preflight MUST report the same governance source the gate reads \
         (§7.9.5); reporting false here sends the caller into a gate it was \
         told would not fire"
    );
}

/// The complement, as a control: a module that genuinely needs no approval
/// must still preflight as `requires_approval: false`. Without this, a reader
/// cannot tell the union from a hardcoded `true`.
#[tokio::test]
async fn preflight_still_reports_no_approval_when_neither_source_declares_one() {
    let module_id = "executor.test.preflight_no_approval";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(DeclaresNothingModule),
            descriptor_without_annotations(module_id),
        )
        .expect("register module");

    let executor = Executor::new(registry, Arc::new(Config::default()));
    let report = executor
        .validate(module_id, &json!({}), None)
        .await
        .expect("validate is non-throwing");

    assert!(
        !report.requires_approval,
        "neither source declares a requirement, so the preflight must not invent one"
    );
}

/// `governance_state()` describes what is actually gating this executor
/// (§6.6.5). It read the descriptor alone while claiming in its own comment to
/// read "the same source the approval gate reads" — so a `system.control.*`
/// module declaring `requires_approval` on its INSTANCE, registered with a
/// descriptor that omits it, was gated by the pipeline and reported here as
/// ungated. A serve-time adapter reading this flag would refuse to start, or
/// warn, over a control surface that is in fact protected.
#[test]
fn governance_state_reads_the_union_for_control_modules() {
    let module_id = "system.control.union_probe";

    let registry = Arc::new(Registry::new());
    registry
        .register_internal(
            module_id,
            Box::new(DeclaresApprovalModule),
            descriptor_without_annotations(module_id),
        )
        .expect("register control module");

    let executor = Executor::new(registry, Arc::new(Config::default()));

    assert!(
        executor
            .governance_state()
            .all_control_modules_require_approval,
        "a control module declaring requires_approval on its instance is gated \
         by the pipeline, so the posture accessor must not report it as ungated"
    );
}

// ===========================================================================
// The manifest advertises what the gate enforces
// ===========================================================================
//
// `manifest.module` / `manifest.full` are what an AGENT reads to decide whether
// to call something. Projecting the two governance flags off the descriptor
// alone advertised `requires_approval: false` for a module whose instance
// declares it — and this SDK accepts a caller-supplied `ModuleDescriptor`, so
// the two genuinely disagree. Advertising a governance value the gate does not
// enforce is worse than advertising none: it is a specific, checkable claim,
// and it is false.
//
// The peers have the same defect through their own second source (a
// `*_meta.yaml` / `metadata=` declaration merged into the descriptor with
// YAML > code precedence), and are fixed in the same pass.

#[tokio::test]
async fn the_manifest_advertises_the_governance_the_gate_enforces() {
    use apcore::sys_modules::ManifestModule;

    let module_id = "executor.test.manifest_governance";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(DeclaresApprovalModule),
            descriptor_without_annotations(module_id),
        )
        .expect("register module");

    let manifest = ManifestModule::new(
        Arc::clone(&registry),
        Arc::new(tokio::sync::Mutex::new(Config::default())),
    );
    let out = manifest
        .execute(
            json!({ "module_id": module_id }),
            &Context::<Value>::anonymous(),
        )
        .await
        .expect("manifest.module");

    assert_eq!(
        out["annotations"]["requires_approval"].as_bool(),
        Some(true),
        "the gate fires on this module (its instance declares the requirement); \
         a manifest advertising requires_approval=false for it is a false claim \
         an agent will act on: {out}"
    );
    assert_eq!(
        out["annotations"]["destructive"].as_bool(),
        Some(true),
        "and the risk marking travels with it: {out}"
    );
}

/// The control: the manifest must not invent governance either.
#[tokio::test]
async fn the_manifest_reports_no_governance_when_neither_source_declares_any() {
    use apcore::sys_modules::ManifestModule;

    let module_id = "executor.test.manifest_no_governance";

    let registry = Arc::new(Registry::new());
    registry
        .register(
            module_id,
            Box::new(DeclaresNothingModule),
            descriptor_without_annotations(module_id),
        )
        .expect("register module");

    let out = ManifestModule::new(
        Arc::clone(&registry),
        Arc::new(tokio::sync::Mutex::new(Config::default())),
    )
    .execute(
        json!({ "module_id": module_id }),
        &Context::<Value>::anonymous(),
    )
    .await
    .expect("manifest.module");

    assert_eq!(
        out["annotations"]["requires_approval"].as_bool(),
        Some(false)
    );
    assert_eq!(out["annotations"]["destructive"].as_bool(), Some(false));
}
