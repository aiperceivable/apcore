//! Call-site inputs to policy resolution — PROTOCOL_SPEC §7.9.6
//! (spec v1.24.0, apcore#102).
//!
//! Governance used to key on *which* module was being called and never on
//! *what it was being called with*, so an operator who needed to gate some
//! calls to a module had to gate all of them. §7.9.6 opens the input while
//! keeping two guarantees:
//!
//! * rule 2 — the built-in pattern rules **MUST NOT** consult the call site,
//!   so a rule set's verdict stays a function of module ID and annotations
//!   alone and remains reproducible from the policy document;
//! * rule 5 — adding the call site **MUST NOT** change the verdict any
//!   existing policy produces.
//!
//! Both are tested here by differential comparison: for a matrix of policies
//! and call sites, `resolve_with_call_site` must return a value equal to
//! `resolve`'s, bit for bit.

#![allow(clippy::pedantic)]

use std::collections::HashMap;

use apcore::context::{Context, Identity};
use apcore::module::ModuleAnnotations;
use apcore::policy::{ExecutionPolicy, PolicyRule};
use serde_json::{json, Value};

fn annotations(requires_approval: bool, destructive: bool) -> ModuleAnnotations {
    let mut a = ModuleAnnotations::default();
    a.requires_approval = requires_approval;
    a.destructive = destructive;
    a
}

fn context() -> Context<Value> {
    let identity = Identity::new(
        "operator-1".to_string(),
        "user".to_string(),
        vec!["admin".to_string()],
        HashMap::new(),
    );
    let mut ctx = Context::new(identity);
    ctx.call_chain = vec!["api.entry".to_string(), "orchestrator.flow".to_string()];
    ctx
}

fn policies() -> Vec<(&'static str, ExecutionPolicy)> {
    vec![
        ("empty", ExecutionPolicy::new(vec![])),
        (
            "forces approval",
            ExecutionPolicy::new(vec![PolicyRule::new("orders.*")
                .unwrap()
                .with_requires_approval(true)]),
        ),
        (
            "clears approval",
            ExecutionPolicy::new(vec![PolicyRule::new("orders.delete_order")
                .unwrap()
                .with_requires_approval(false)]),
        ),
        (
            "gate_destructive",
            ExecutionPolicy::new(vec![PolicyRule::new("orders.*")
                .unwrap()
                .with_destructive(true)])
            .with_gate_destructive(true),
        ),
        (
            "specificity tie",
            ExecutionPolicy::new(vec![
                PolicyRule::new("orders.*")
                    .unwrap()
                    .with_requires_approval(false),
                PolicyRule::new("orders.*")
                    .unwrap()
                    .with_requires_approval(true),
            ]),
        ),
    ]
}

/// Call sites chosen to be as different from each other as possible: absent,
/// empty, populated, and a shape that has NOT been schema-validated (§7.9.6
/// rule 4 — the gate is Step 5, input validation is Step 7).
fn call_sites() -> Vec<(&'static str, Option<Value>)> {
    vec![
        ("absent", None),
        ("empty object", Some(json!({}))),
        (
            "populated",
            Some(json!({ "order_id": "O-1", "force": true, "amount": 9_999 })),
        ),
        ("not an object", Some(json!("not schema-valid"))),
        ("null", Some(Value::Null)),
    ]
}

// [policy-call-site-verdict-unchanged] §7.9.6 rules 2 and 5. Every
// (policy, module, annotations, call site) combination must produce a decision
// identical to the one `resolve` produces without the call site.
#[test]
fn the_call_site_never_changes_a_verdict() {
    let modules = [
        "orders.delete_order",
        "orders.list_orders",
        "unrelated.module",
    ];
    let annotation_matrix = [(false, false), (true, false), (false, true), (true, true)];
    let ctx = context();

    for (policy_name, policy) in policies() {
        for module_id in modules {
            for (requires_approval, destructive) in annotation_matrix {
                let anns = annotations(requires_approval, destructive);
                let baseline = policy.resolve(module_id, Some(&anns));

                for (site_name, arguments) in call_sites() {
                    for with_ctx in [None, Some(&ctx)] {
                        let with_call_site = policy.resolve_with_call_site(
                            module_id,
                            Some(&anns),
                            arguments.as_ref(),
                            with_ctx,
                        );
                        assert_eq!(
                            with_call_site, baseline,
                            "policy '{policy_name}', module '{module_id}', annotations \
                             ({requires_approval}, {destructive}), call site '{site_name}': \
                             the built-in pattern rules MUST NOT consult the call site \
                             (§7.9.6 rule 2) and adding it MUST NOT change any verdict \
                             (rule 5)"
                        );
                    }
                }
            }
        }
    }
}

// [policy-call-site-no-annotations] The `None` annotations path is separate
// code in `resolve`; cover it too.
#[test]
fn the_call_site_never_changes_a_verdict_without_annotations() {
    let policy = ExecutionPolicy::new(vec![PolicyRule::new("*")
        .unwrap()
        .with_requires_approval(true)]);
    let ctx = context();

    assert_eq!(
        policy.resolve_with_call_site("a.b", None, Some(&json!({ "x": 1 })), Some(&ctx)),
        policy.resolve("a.b", None)
    );
}

// [policy-call-site-reaches-resolution] The point of the change: resolution
// actually receives the call site (§7.9.6 rule 1). A compile-time assertion —
// if the parameters were dropped from the signature this would not build.
#[test]
fn resolution_accepts_the_arguments_and_the_context() {
    let policy = ExecutionPolicy::new(vec![]);
    let ctx = context();
    let arguments = json!({ "record_id": "C-7" });

    let decision =
        policy.resolve_with_call_site("executor.crm.delete", None, Some(&arguments), Some(&ctx));

    assert_eq!(decision.module_id, "executor.crm.delete");
    assert!(!decision.needs_approval);
}

// ---------------------------------------------------------------------------
// §7.9.6 rule 5 — the framework-owned approval token
// ---------------------------------------------------------------------------

// [policy-call-site-strips-approval-token] `_approval_token` MUST be stripped
// from `arguments` BEFORE policy resolution. §7.4 already requires it to be
// removed "before passing to subsequent steps", which does not reach this
// case: resolution happens INSIDE Step 5, ahead of any subsequent step, so an
// implementation can satisfy §7.4 literally and still hand the token to the
// policy. Confirmed against the gate: `builtin_steps.rs` resolves the policy
// near the top of `execute()` and strips the token ~120 lines later, so the
// strip has to live at the policy API boundary to be soon enough.
//
// The token is protocol-level, not caller input; leaving it in place puts a
// token into the audit trail and the `apcore.policy.override` payload.
#[test]
fn resolution_strips_the_framework_approval_token_from_the_call_site() {
    let policy = ExecutionPolicy::new(vec![PolicyRule::new("orders.*")
        .unwrap()
        .with_requires_approval(true)]);
    let ctx = context();
    let anns = annotations(false, false);

    let with_token = json!({ "_approval_token": "tok-secret", "order_id": "O-1" });
    let without_token = json!({ "order_id": "O-1" });

    // Rule 6 first: the verdict is identical either way, as it must be for any
    // call site.
    let a =
        policy.resolve_with_call_site("orders.delete", Some(&anns), Some(&with_token), Some(&ctx));
    let b = policy.resolve_with_call_site(
        "orders.delete",
        Some(&anns),
        Some(&without_token),
        Some(&ctx),
    );
    assert_eq!(a, b);
    assert_eq!(a, policy.resolve("orders.delete", Some(&anns)));

    // And the caller's own value is not mutated — the strip is a copy, so a
    // Phase B resume still finds its token in `ctx.inputs` further down Step 5.
    assert_eq!(
        with_token.get("_approval_token").and_then(Value::as_str),
        Some("tok-secret"),
        "resolution must not mutate the caller's arguments"
    );
}

// [policy-call-site-non-object-arguments] Rule 4: the arguments have not been
// schema-validated, so the strip must cope with a non-object.
#[test]
fn stripping_the_token_tolerates_arguments_that_are_not_an_object() {
    let policy = ExecutionPolicy::new(vec![]);
    for arguments in [json!("a bare string"), json!(7), json!([1, 2]), Value::Null] {
        let decision = policy.resolve_with_call_site("a.b", None, Some(&arguments), None);
        assert_eq!(decision, policy.resolve("a.b", None));
    }
}
