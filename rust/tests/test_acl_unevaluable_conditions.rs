//! Unevaluable ACL conditions — PROTOCOL_SPEC §6.1.1 / §6.1.2 / §6.1.3 / §6.8
//! (spec v1.22.0 & v1.23.0, apcore#100 and #101).
//!
//! A condition that **is false** and a condition that **cannot be evaluated**
//! are different outcomes, and the rule's `effect` decides what the difference
//! means. Before v1.22.0 both reached the rule loop as `false`, so a `deny`
//! rule carrying a misspelled key blocked nothing at all.
//!
//! Covered here:
//!
//! * the cases that produce UNEVALUABLE (§6.1.1 — a principle, not a closed
//!   list), on BOTH the sync
//!   `check()` and the async `async_check()` paths — `matches_rule` and
//!   `matches_rule_async` are separate code paths;
//! * the three-valued composition table for AND / `$or` / `$not`, including
//!   `$not` of UNEVALUABLE staying UNEVALUABLE;
//! * `handler_error` aggregation: every unevaluable key, ordered
//!   lexicographically by key, joined with `"; "` (§6.1.1 rule 2);
//! * `ACL::validate_rules()` and the two separate registry flags (§6.1.3);
//! * the §6.8 read-only accessors.
//!
//! Every condition key registered here is prefixed `__uc_` and unique to this
//! file, because the handler registry is process-wide.

#![allow(clippy::pedantic)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use apcore::acl::{ACLRule, AuditEntry, ACL};
use apcore::acl_handlers::{ACLConditionHandler, ConditionOutcome};
use apcore::context::{Context, Identity};
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_context(roles: Vec<&str>) -> Context<Value> {
    let identity = Identity::new(
        "test-user".to_string(),
        "user".to_string(),
        roles.into_iter().map(String::from).collect(),
        HashMap::new(),
    );
    Context::new(identity)
}

fn rule(effect: &str, conditions: Value) -> ACLRule {
    let mut rule = ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], effect);
    rule.description = Some(format!("{effect} rule under test"));
    rule.conditions = Some(conditions);
    rule
}

/// Build an ACL over one rule with an audit sink attached.
fn acl_with_sink(r: ACLRule, default_effect: &str) -> (ACL, Arc<Mutex<Vec<AuditEntry>>>) {
    let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let mut acl = ACL::new(vec![r], default_effect, None);
    acl.set_audit_logger(move |entry: &AuditEntry| {
        sink.lock().unwrap().push(entry.clone());
    });
    (acl, captured)
}

fn last_handler_error(captured: &Arc<Mutex<Vec<AuditEntry>>>) -> Option<String> {
    captured
        .lock()
        .unwrap()
        .last()
        .expect("an audit entry must be emitted for every check()")
        .handler_error
        .clone()
}

/// Suppress panic-hook stderr noise for the deliberately panicking handlers.
fn silence_panic_hook<T>(f: impl FnOnce() -> T) -> T {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = f();
    std::panic::set_hook(prev);
    out
}

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

/// Panics on evaluation — §6.1.1 case 2.
struct PanickingHandler;

#[async_trait]
impl ACLConditionHandler for PanickingHandler {
    async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
        panic!("simulated handler panic");
    }
}

/// Genuinely suspends before answering — §6.1.1 case 3 on the sync path,
/// but resolvable under `async_check()`.
struct SuspendingHandler {
    answer: bool,
}

#[async_trait]
impl ACLConditionHandler for SuspendingHandler {
    async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
        tokio::task::yield_now().await;
        self.answer
    }
}

/// Answers from the value, and counts how many times it ran — used to prove
/// which children the evaluator actually reached.
struct CountingHandler {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ACLConditionHandler for CountingHandler {
    async fn evaluate(&self, value: &Value, _ctx: &Context<Value>) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        value.as_bool().unwrap_or(false)
    }
}

fn init() {
    ACL::init_builtin_handlers();
}

// ---------------------------------------------------------------------------
// §6.1.1 — the enumerated cases, on the deny and allow sides, sync path
// ---------------------------------------------------------------------------

// [acl-unevaluable-unknown-key-deny] The misspelled-key case #100 was opened
// for: a deny rule referencing a key nobody registered must DENY, not fall
// through to a permissive default_effect.
#[test]
fn unknown_key_makes_a_deny_rule_deny() {
    init();
    let (acl, captured) = acl_with_sink(
        rule("deny", json!({ "__uc_never_registered__": true })),
        "allow",
    );

    assert!(
        !acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))),
        "a deny rule whose condition is unevaluable MUST take effect (§6.1.1)"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(
        err.contains("__uc_never_registered__") && err.contains("unknown ACL condition"),
        "handler_error must name the key and the reason: {err}"
    );
}

// [acl-unevaluable-unknown-key-allow] The allow side is unchanged from
// v1.21.0: an unevaluable condition still does not grant.
#[test]
fn unknown_key_does_not_let_an_allow_rule_grant() {
    init();
    let (acl, captured) = acl_with_sink(
        rule("allow", json!({ "__uc_never_registered__": true })),
        "deny",
    );

    assert!(
        !acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))),
        "an allow rule whose condition is unevaluable MUST NOT grant (§6.1.1)"
    );
    assert!(last_handler_error(&captured).is_some());
}

// [acl-unevaluable-panic-deny] §6.1.1 case 2. The panic is caught, never
// unwinds out of check(), and the deny rule takes effect.
#[test]
fn panicking_handler_makes_a_deny_rule_deny() {
    init();
    ACL::register_condition("__uc_panic__", Arc::new(PanickingHandler));
    let (acl, captured) = acl_with_sink(rule("deny", json!({ "__uc_panic__": true })), "allow");

    let ctx = make_context(vec![]);
    let decision = silence_panic_hook(|| acl.check(Some("api.x"), "executor.y", Some(&ctx)));
    assert!(!decision, "a panicking handler is unevaluable, not false");
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(
        err.contains("__uc_panic__") && err.contains("panicked"),
        "handler_error must carry the panic: {err}"
    );
}

// [acl-unevaluable-async-on-sync-deny] §6.1.1 case 3. Through spec
// v1.21.0 this was specified as "unsatisfied", which left a deny rule guarded
// by an async-only handler inert on the sync path.
#[test]
fn suspending_handler_on_the_sync_path_makes_a_deny_rule_deny() {
    init();
    ACL::register_condition(
        "__uc_suspends__",
        Arc::new(SuspendingHandler { answer: true }),
    );
    let (acl, captured) = acl_with_sink(rule("deny", json!({ "__uc_suspends__": true })), "allow");

    assert!(
        !acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))),
        "a handler that is not ready synchronously is UNEVALUABLE (§6.1.1), not unsatisfied"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(
        err.contains("__uc_suspends__") && err.contains("not ready synchronously"),
        "handler_error must explain the sync-path failure: {err}"
    );
}

// [acl-unevaluable-false-is-not-unevaluable] The control: a handler that ran
// and answered "no" is an ORDINARY non-match. The deny rule does not fire and
// handler_error stays null — that is what makes the two distinguishable after
// the fact (§6.3.1).
#[test]
fn a_handler_that_answers_false_leaves_handler_error_null() {
    init();
    let (acl, captured) = acl_with_sink(rule("deny", json!({ "roles": ["admin"] })), "allow");

    assert!(
        acl.check(
            Some("api.x"),
            "executor.y",
            Some(&make_context(vec!["user"]))
        ),
        "an UNSATISFIED condition is a plain non-match: the deny rule does not apply"
    );
    assert_eq!(
        last_handler_error(&captured),
        None,
        "handler_error MUST be null for a merely-false condition (§6.3.1)"
    );
}

// ---------------------------------------------------------------------------
// §6.1.1 — the same, on the ASYNC path (matches_rule_async is separate code)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn async_check_unknown_key_makes_a_deny_rule_deny() {
    init();
    let (acl, captured) = acl_with_sink(
        rule("deny", json!({ "__uc_never_registered_async__": true })),
        "allow",
    );

    assert!(
        !acl.async_check(Some("api.x"), "executor.y", Some(&make_context(vec![])))
            .await,
        "async_check must apply §6.1.1 exactly as check does"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(err.contains("__uc_never_registered_async__"), "{err}");
}

#[tokio::test]
async fn async_check_unknown_key_does_not_let_an_allow_rule_grant() {
    init();
    let (acl, _captured) = acl_with_sink(
        rule("allow", json!({ "__uc_never_registered_async__": true })),
        "deny",
    );

    assert!(
        !acl.async_check(Some("api.x"), "executor.y", Some(&make_context(vec![])))
            .await
    );
}

#[tokio::test]
async fn async_check_panicking_handler_makes_a_deny_rule_deny() {
    init();
    ACL::register_condition("__uc_panic_async__", Arc::new(PanickingHandler));
    let (acl, captured) =
        acl_with_sink(rule("deny", json!({ "__uc_panic_async__": true })), "allow");

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let decision = acl
        .async_check(Some("api.x"), "executor.y", Some(&make_context(vec![])))
        .await;
    std::panic::set_hook(prev);

    assert!(!decision);
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(
        err.contains("__uc_panic_async__") && err.contains("panicked"),
        "{err}"
    );
}

// [acl-unevaluable-async-resolves] The mirror of case 3: the SAME handler
// that is unevaluable on the sync path resolves normally under async_check, so
// the deny rule fires on its real answer rather than on a failure.
#[tokio::test]
async fn a_suspending_handler_resolves_under_async_check() {
    init();
    ACL::register_condition(
        "__uc_suspends_resolves__",
        Arc::new(SuspendingHandler { answer: false }),
    );
    let (acl, captured) = acl_with_sink(
        rule("deny", json!({ "__uc_suspends_resolves__": true })),
        "allow",
    );

    assert!(
        acl.async_check(Some("api.x"), "executor.y", Some(&make_context(vec![])))
            .await,
        "the handler answered false, so the deny rule does not match and the default allows"
    );
    assert_eq!(
        last_handler_error(&captured),
        None,
        "a resolved async handler is not an evaluation failure"
    );
}

// ---------------------------------------------------------------------------
// §6.1.1 — three-valued composition (AND, $or, $not)
// ---------------------------------------------------------------------------

// [acl-outcome-and] An outright "no" wins an AND even against an unevaluable
// sibling.
//
// The unevaluable sibling here is a REGISTERED key whose handler panics, so it
// is execution-origin. That distinction is load-bearing after spec v1.25.0: a
// key the §6.1.4 precheck rejects makes the whole rule unevaluable before any
// composition happens (see the precheck section below), so the Kleene table
// can only be exercised with a fault the precheck passes. §6.1.1's own worked
// example is chosen the same way, for the same reason.
#[test]
fn and_of_unsatisfied_and_unevaluable_is_unsatisfied() {
    init();
    ACL::register_condition("__uc_and_panic__", Arc::new(PanickingHandler));
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({ "roles": ["admin"], "__uc_and_panic__": true }),
        ),
        "allow",
    );

    let ctx = make_context(vec!["user"]);
    let decision = silence_panic_hook(|| acl.check(Some("api.x"), "executor.y", Some(&ctx)));
    assert!(
        decision,
        "roles answered NO outright, which wins the conjunction, so the deny rule does not \
         match and the default allows"
    );
    assert!(
        last_handler_error(&captured).is_some(),
        "a rule can end UNSATISFIED while a condition inside it was unevaluable, and the \
         diagnostic is still recorded (PROTOCOL_SPEC 6.3.1)"
    );
}

// [acl-outcome-and-unevaluable] No child said no, one could not answer.
#[test]
fn and_of_satisfied_and_unevaluable_is_unevaluable() {
    init();
    ACL::register_condition("__uc_and_panic2__", Arc::new(PanickingHandler));
    let (acl, _c) = acl_with_sink(
        rule(
            "deny",
            json!({ "roles": ["admin"], "__uc_and_panic2__": true }),
        ),
        "allow",
    );

    let ctx = make_context(vec!["admin"]);
    let decision = silence_panic_hook(|| acl.check(Some("api.x"), "executor.y", Some(&ctx)));
    assert!(
        !decision,
        "roles is SATISFIED and the other child is UNEVALUABLE, so the conjunction is \
         UNEVALUABLE and the deny rule takes effect"
    );
}

// [acl-outcome-or] An outright "yes" wins an $or even against an unevaluable
// sibling. Execution-origin again, for the reason noted on the AND case above.
#[test]
fn or_with_a_satisfied_child_is_satisfied_despite_an_unevaluable_sibling() {
    init();
    ACL::register_condition("__uc_or_panic__", Arc::new(PanickingHandler));
    let (acl, _c) = acl_with_sink(
        rule(
            "allow",
            json!({ "$or": [ { "roles": ["admin"] }, { "__uc_or_panic__": true } ] }),
        ),
        "deny",
    );

    let ctx = make_context(vec!["admin"]);
    let decision = silence_panic_hook(|| acl.check(Some("api.x"), "executor.y", Some(&ctx)));
    assert!(
        decision,
        "an outright SATISFIED child wins the disjunction (PROTOCOL_SPEC 6.1.1)"
    );
}

// [acl-outcome-or-unevaluable] No child said yes, one could not answer.
#[test]
fn or_with_no_satisfied_child_and_an_unevaluable_one_is_unevaluable() {
    init();
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({ "$or": [ { "roles": ["admin"] }, { "__uc_never_registered__": true } ] }),
        ),
        "allow",
    );

    assert!(
        !acl.check(
            Some("api.x"),
            "executor.y",
            Some(&make_context(vec!["user"]))
        ),
        "no SATISFIED child and one UNEVALUABLE child makes the $or UNEVALUABLE, so the deny \
         rule takes effect"
    );
    let err = last_handler_error(&captured).expect("the nested key must reach handler_error");
    assert!(err.contains("__uc_never_registered__"), "{err}");
}

// [acl-outcome-not-unevaluable] THE nesting bypass §6.1.1 closes: `$not` of an
// unevaluable condition MUST NOT yield SATISFIED. If it did, a misspelled key
// inside a `$not` would satisfy the very rule it was meant to gate.
#[test]
fn not_of_an_unevaluable_condition_is_unevaluable_never_satisfied() {
    init();
    let (acl, captured) = acl_with_sink(
        rule(
            "allow",
            json!({ "$not": { "__uc_never_registered__": true } }),
        ),
        "deny",
    );

    assert!(
        !acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))),
        "$not of UNEVALUABLE is UNEVALUABLE, so the allow rule MUST NOT grant"
    );
    assert!(last_handler_error(&captured).is_some());
}

#[test]
fn not_of_an_unevaluable_condition_makes_a_deny_rule_deny() {
    init();
    let (acl, _c) = acl_with_sink(
        rule(
            "deny",
            json!({ "$not": { "__uc_never_registered__": true } }),
        ),
        "allow",
    );

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
}

// [acl-outcome-not-plain] The ordinary negations are unchanged.
#[test]
fn not_still_negates_a_satisfied_and_an_unsatisfied_child() {
    init();
    let (satisfied, _a) = acl_with_sink(
        rule("allow", json!({ "$not": { "roles": ["admin"] } })),
        "deny",
    );
    assert!(
        satisfied.check(
            Some("api.x"),
            "executor.y",
            Some(&make_context(vec!["user"]))
        ),
        "$not of UNSATISFIED is SATISFIED"
    );

    let (unsatisfied, _b) = acl_with_sink(
        rule("allow", json!({ "$not": { "roles": ["admin"] } })),
        "deny",
    );
    assert!(
        !unsatisfied.check(
            Some("api.x"),
            "executor.y",
            Some(&make_context(vec!["admin"]))
        ),
        "$not of SATISFIED is UNSATISFIED"
    );
}

// [acl-outcome-enum] The composition table, exercised directly on the enum so
// a future refactor of the evaluator cannot quietly change the algebra.
#[test]
fn condition_outcome_algebra_matches_the_spec_table() {
    use ConditionOutcome::{Satisfied, Unevaluable, Unsatisfied};

    // AND
    assert_eq!(Unsatisfied.and(Unevaluable), Unsatisfied);
    assert_eq!(Unevaluable.and(Unsatisfied), Unsatisfied);
    assert_eq!(Satisfied.and(Unevaluable), Unevaluable);
    assert_eq!(Unevaluable.and(Satisfied), Unevaluable);
    assert_eq!(Satisfied.and(Satisfied), Satisfied);
    assert_eq!(Satisfied.and(Unsatisfied), Unsatisfied);

    // OR
    assert_eq!(Satisfied.or(Unevaluable), Satisfied);
    assert_eq!(Unevaluable.or(Satisfied), Satisfied);
    assert_eq!(Unsatisfied.or(Unevaluable), Unevaluable);
    assert_eq!(Unevaluable.or(Unsatisfied), Unevaluable);
    assert_eq!(Unsatisfied.or(Unsatisfied), Unsatisfied);

    // NOT — the load-bearing row.
    assert_eq!(Satisfied.negate(), Unsatisfied);
    assert_eq!(Unsatisfied.negate(), Satisfied);
    assert_eq!(
        Unevaluable.negate(),
        Unevaluable,
        "$not of UNEVALUABLE MUST NOT be SATISFIED (§6.1.1)"
    );
}

// ---------------------------------------------------------------------------
// §6.1.1 rule 2 — handler_error aggregation and ordering
// ---------------------------------------------------------------------------

// [acl-handler-error-order] Several unevaluable conditions in one check() are
// ALL reported, ordered lexicographically BY CONDITION KEY and joined with
// "; ". Evaluation order is not portable across SDKs, so "the first one
// encountered" would put a different key in the audit log per language.
#[test]
fn handler_error_lists_every_unevaluable_key_in_lexicographic_order() {
    init();
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({
                "__uc_zeta_missing__": true,
                "__uc_alpha_missing__": true,
                "__uc_mid_missing__": true,
            }),
        ),
        "allow",
    );

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");

    let parts: Vec<&str> = err.split("; ").collect();
    assert_eq!(
        parts.len(),
        3,
        "every unevaluable condition MUST be reported (§6.1.1 rule 2): {err}"
    );
    assert!(parts[0].starts_with("__uc_alpha_missing__"), "{err}");
    assert!(parts[1].starts_with("__uc_mid_missing__"), "{err}");
    assert!(parts[2].starts_with("__uc_zeta_missing__"), "{err}");
}

// [acl-handler-error-nested] A key nested inside $or / $not reaches
// handler_error too — it was reached by the evaluator, so it is unevaluable.
#[test]
fn handler_error_reports_keys_nested_in_compound_operators() {
    init();
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({
                "$or": [ { "__uc_nested_b__": true } ],
                "$not": { "__uc_nested_a__": true },
            }),
        ),
        "allow",
    );

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(
        err.contains("__uc_nested_a__") && err.contains("__uc_nested_b__"),
        "both nested keys must be reported: {err}"
    );
    assert!(
        err.find("__uc_nested_a__") < err.find("__uc_nested_b__"),
        "still ordered lexicographically by key: {err}"
    );
}

// [acl-no-shortcircuit-on-unevaluable] An implementation MUST NOT
// short-circuit on UNEVALUABLE — the remaining children may still produce the
// decisive outcome. The unevaluable child sorts first, and the decisive
// counted child must still be reached.
//
// Both keys are REGISTERED so the precheck passes and execution actually runs;
// a precheck fault would decide the rule before any handler.
#[test]
fn evaluation_does_not_stop_at_an_unevaluable_child() {
    init();
    let calls = Arc::new(AtomicUsize::new(0));
    ACL::register_condition("__uc_aaa_panic__", Arc::new(PanickingHandler));
    ACL::register_condition(
        "__uc_zzz_counted__",
        Arc::new(CountingHandler {
            calls: Arc::clone(&calls),
        }),
    );

    let (acl, _c) = acl_with_sink(
        rule(
            "deny",
            json!({ "__uc_aaa_panic__": true, "__uc_zzz_counted__": false }),
        ),
        "allow",
    );

    let ctx = make_context(vec![]);
    let decision = silence_panic_hook(|| acl.check(Some("api.x"), "executor.y", Some(&ctx)));
    assert!(
        decision,
        "the counted child answered NO outright, which wins the AND even though a sibling was \
         unevaluable, so the deny rule does not match"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the child after the unevaluable one MUST still be evaluated"
    );
}

// [acl-precheck-is-rule-level] The other half of the split, and the reason the
// composition tests above use panicking handlers. PROTOCOL_SPEC 6.1.4 rule 1
// states it flatly: "a rule that fails the precheck is unevaluable". A
// misspelled key is misconfiguration, not an answer, so the rule does not get
// to apply to anybody — an UNSATISFIED sibling does NOT rescue it the way it
// rescues an execution-origin failure.
#[test]
fn a_precheck_fault_makes_the_whole_rule_unevaluable_despite_an_unsatisfied_sibling() {
    init();
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({ "roles": ["admin"], "__uc_never_registered__": true }),
        ),
        "allow",
    );

    assert!(
        !acl.check(
            Some("api.x"),
            "executor.y",
            Some(&make_context(vec!["user"]))
        ),
        "the caller has no admin role, but the rule is MISCONFIGURED, so it is unevaluable \
         outright and the deny rule takes effect"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(err.contains("__uc_never_registered__"), "{err}");
}

// [acl-precheck-runs-without-handlers] The precheck MUST NOT invoke a handler.
// A rule carrying both a misspelled key and a registered counted handler
// decides on the precheck alone, so the counted handler never runs.
#[test]
fn the_precheck_never_invokes_a_handler() {
    init();
    let calls = Arc::new(AtomicUsize::new(0));
    ACL::register_condition(
        "__uc_precheck_counted__",
        Arc::new(CountingHandler {
            calls: Arc::clone(&calls),
        }),
    );

    let (acl, _c) = acl_with_sink(
        rule(
            "deny",
            json!({ "__uc_precheck_counted__": true, "__uc_never_registered__": true }),
        ),
        "allow",
    );

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the precheck decided the rule; no handler may have been invoked"
    );
}

// ---------------------------------------------------------------------------
// §6.5 — conditions with no context stay an ordinary non-match
// ---------------------------------------------------------------------------

// [acl-no-context-not-unevaluable] Deliberately NOT an §6.1.1 unevaluable
// condition: calling with no context is a legitimate shape for external entry
// points, not a misconfiguration. Treating it as a failure would flip the
// decision for every @external call meeting a conditional deny rule.
#[test]
fn a_conditional_rule_with_no_context_is_unsatisfied_not_unevaluable() {
    init();
    let (acl, captured) = acl_with_sink(rule("deny", json!({ "roles": ["admin"] })), "allow");

    assert!(
        acl.check(None, "executor.y", None),
        "no context means the rule does not match (§6.5), so the default applies"
    );
    assert_eq!(
        last_handler_error(&captured),
        None,
        "a context-less call is not an evaluation failure"
    );
}

// ---------------------------------------------------------------------------
// §6.1.2 / §6.1.3 — validate_rules()
// ---------------------------------------------------------------------------

// [acl-validate-rules-basic]
#[test]
fn validate_conditions_reports_unregistered_keys_with_rule_index_and_effect() {
    init();
    let acl = ACL::new(
        vec![
            rule("allow", json!({ "roles": ["admin"] })),
            rule("deny", json!({ "__uc_validate_missing__": true })),
        ],
        "deny",
        None,
    );

    let findings = acl.validate_rules();
    assert_eq!(findings.len(), 1, "only the unregistered key is a finding");
    let f = &findings[0];
    assert_eq!(f.rule_index, 1);
    assert_eq!(f.condition_path, "__uc_validate_missing__");
    assert_eq!(f.condition_key.as_deref(), Some("__uc_validate_missing__"));
    assert_eq!(f.effect, "deny");
    assert!(!f.sync_resolvable);
    assert!(!f.async_resolvable);
}

// [acl-validate-rules-builtins] The built-ins are never findings, and
// neither are the compound operators themselves.
#[test]
fn validate_conditions_is_empty_for_builtin_keys_only() {
    init();
    let acl = ACL::new(
        vec![rule(
            "allow",
            json!({
                "identity_types": ["user"],
                "max_call_depth": 5,
                "$or": [ { "roles": ["admin"] } ],
                "$not": { "roles": ["banned"] },
            }),
        )],
        "deny",
        None,
    );

    assert!(
        acl.validate_rules().is_empty(),
        "every referenced key resolves: {:?}",
        acl.validate_rules()
    );
}

// [acl-validate-rules-nested] Keys nested inside $or / $not are reported.
#[test]
fn validate_conditions_descends_into_compound_operators() {
    init();
    let acl = ACL::new(
        vec![rule(
            "deny",
            json!({
                "$or": [ { "__uc_nested_validate_b__": true }, { "roles": ["admin"] } ],
                "$not": { "__uc_nested_validate_a__": true },
            }),
        )],
        "deny",
        None,
    );

    let keys: Vec<String> = acl
        .validate_rules()
        .into_iter()
        .map(|f| f.condition_path)
        .collect();
    assert_eq!(
        keys,
        vec![
            "$not.__uc_nested_validate_a__".to_string(),
            "$or[0].__uc_nested_validate_b__".to_string()
        ],
        "nested faults are reported by PATH (§6.1.4), ascending"
    );
}

// [acl-validate-rules-async-only] §6.1.3 rule 2: a finding IS emitted
// when a key is registered on the async path only, because check() cannot
// evaluate it. The two flags MUST NOT be collapsed into one boolean.
#[test]
fn validate_conditions_reports_an_async_only_key_with_both_flags() {
    init();
    ACL::register_async_condition(
        "__uc_async_only__",
        Arc::new(SuspendingHandler { answer: true }),
    );

    let acl = ACL::new(
        vec![rule("deny", json!({ "__uc_async_only__": true }))],
        "deny",
        None,
    );

    let findings = acl.validate_rules();
    assert_eq!(findings.len(), 1);
    assert!(
        !findings[0].sync_resolvable,
        "async-only keys do not resolve for check()"
    );
    assert!(
        findings[0].async_resolvable,
        "they do resolve for async_check(), and that MUST be reported separately (§6.1.3)"
    );
}

// [acl-validate-rules-pure] Pure read: no mutation, no audit event.
#[test]
fn validate_conditions_is_a_pure_read() {
    init();
    let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let mut acl = ACL::new(
        vec![rule("deny", json!({ "__uc_validate_pure__": true }))],
        "deny",
        None,
    );
    acl.set_audit_logger(move |entry: &AuditEntry| {
        sink.lock().unwrap().push(entry.clone());
    });

    let first = acl.validate_rules();
    let second = acl.validate_rules();
    assert_eq!(first, second, "idempotent for a fixed rule list + registry");
    assert_eq!(acl.rules().len(), 1, "the rule list is untouched");
    assert!(
        captured.lock().unwrap().is_empty(),
        "validate_conditions MUST NOT emit an audit event"
    );
}

// [acl-validate-rules-add-rule] §6.1.2 rule 4: runtime insertion is
// covered too — a rule added later shows up in the findings.
#[test]
fn validate_conditions_covers_rules_inserted_at_runtime() {
    init();
    let mut acl = ACL::new(vec![], "deny", None);
    assert!(acl.validate_rules().is_empty());

    acl.add_rule(rule("deny", json!({ "__uc_added_at_runtime__": true })));

    let findings = acl.validate_rules();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule_index, 0, "add_rule inserts at index 0");
    assert_eq!(findings[0].condition_path, "__uc_added_at_runtime__");
    assert_eq!(findings[0].effect, "deny");
}

// [acl-validate-rules-never-fails] §6.1.2 rules 1-2: construction and
// loading WARN, they never fail, because handler registration is a runtime
// property and acl.root discovery commonly runs before application code.
#[test]
fn constructing_an_acl_with_an_unregistered_key_succeeds() {
    init();
    let acl = ACL::try_new(
        vec![rule("deny", json!({ "__uc_construct_missing__": true }))],
        "deny",
        None,
    );
    assert!(
        acl.is_ok(),
        "an unregistered condition key MUST NOT fail construction (§6.1.2 rule 1)"
    );
}

// ---------------------------------------------------------------------------
// §6.8 — ACL introspection
// ---------------------------------------------------------------------------

// [acl-introspection-default-effect]
#[test]
fn default_effect_is_readable_and_matches_the_enforced_value() {
    init();
    let deny = ACL::new(vec![], "deny", None);
    assert_eq!(deny.default_effect(), "deny");
    assert!(
        !deny.check(Some("api.x"), "executor.y", None),
        "the accessor must equal the effect check() applies when no rule matches"
    );

    let allow = ACL::new(vec![], "allow", None);
    assert_eq!(allow.default_effect(), "allow");
    assert!(allow.check(Some("api.x"), "executor.y", None));
}

// [acl-introspection-pure] Both accessors are pure reads: no audit event.
#[test]
fn the_introspection_accessors_emit_no_audit_event() {
    init();
    let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let mut acl = ACL::new(
        vec![rule("allow", json!({ "roles": ["admin"] }))],
        "deny",
        None,
    );
    acl.set_audit_logger(move |entry: &AuditEntry| {
        sink.lock().unwrap().push(entry.clone());
    });

    let _ = acl.default_effect();
    let _ = acl.rules();
    assert!(
        captured.lock().unwrap().is_empty(),
        "§6.8 rule 2: reading MUST NOT emit an audit event"
    );
}

// [acl-introspection-reload] §6.8 rule 4: both accessors read the live object.
#[test]
fn both_accessors_reflect_a_reload() {
    use std::io::Write;
    init();

    let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
    writeln!(
        tmp,
        "default_effect: deny\nrules:\n  - callers: ['*']\n    targets: ['*']\n    effect: deny\n"
    )
    .expect("write");
    tmp.flush().expect("flush");
    let path = tmp.path().to_str().expect("utf8").to_string();

    let mut acl = ACL::load(&path).expect("initial load");
    assert_eq!(acl.default_effect(), "deny");
    assert_eq!(acl.rules().len(), 1);

    std::fs::write(
        &path,
        "default_effect: allow\nrules:\n  - callers: ['a']\n    targets: ['b']\n    effect: allow\n  \
         - callers: ['c']\n    targets: ['d']\n    effect: deny\n",
    )
    .expect("rewrite");
    acl.reload().expect("reload");

    assert_eq!(
        acl.default_effect(),
        "allow",
        "default_effect MUST reflect the reloaded file"
    );
    assert_eq!(
        acl.rules().len(),
        2,
        "rules MUST reflect the reloaded file, in definition order"
    );
    assert_eq!(acl.rules()[0].callers, vec!["a".to_string()]);
}

// ---------------------------------------------------------------------------
// The two normative warning requirements (§6.1.1 rule 3, §6.1.2 rule 2)
// ---------------------------------------------------------------------------

/// A minimal `tracing::Subscriber` that records WARN/ERROR events as flat
/// strings. Implemented against the `tracing` facade directly rather than
/// through `tracing-subscriber`, whose `registry` feature this crate does not
/// enable.
mod warn_capture {
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Level, Metadata, Subscriber};

    #[derive(Clone, Default)]
    pub struct Captured(pub Arc<Mutex<Vec<String>>>);

    impl Captured {
        pub fn joined(&self) -> String {
            self.0.lock().unwrap().join("\n")
        }
    }

    struct FlatVisitor {
        out: String,
    }

    impl Visit for FlatVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write;
            let _ = write!(self.out, " {}={value:?}", field.name());
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            use std::fmt::Write;
            let _ = write!(self.out, " {}={value}", field.name());
        }
    }

    pub struct CapturingSubscriber(pub Captured);

    impl Subscriber for CapturingSubscriber {
        fn enabled(&self, meta: &Metadata<'_>) -> bool {
            *meta.level() <= Level::WARN
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            if *event.metadata().level() > Level::WARN {
                return;
            }
            let mut visitor = FlatVisitor { out: String::new() };
            event.record(&mut visitor);
            self.0 .0.lock().unwrap().push(visitor.out);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    /// Run `f` with warnings captured.
    pub fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
        let captured = Captured::default();
        let subscriber = CapturingSubscriber(captured.clone());
        let value = tracing::subscriber::with_default(subscriber, f);
        (value, captured.joined())
    }
}

// [acl-unevaluable-warning] §6.1.1 rule 3: the warning MUST name the condition
// key, the rule's index, and the rule's `effect`. The effect is required
// because a misconfigured deny rule is the consequential case.
#[test]
fn an_unevaluable_rule_warns_with_the_key_index_and_effect() {
    init();
    let ctx = make_context(vec![]);
    let (decision, warnings) = warn_capture::capture(|| {
        // Rule 0 never matches, so rule 1 is the one that goes unevaluable —
        // proving the index is the rule's real position, not a constant.
        let non_matching = ACLRule::new(
            vec!["nobody.*".to_string()],
            vec!["nothing.*".to_string()],
            "allow",
        );
        let acl = ACL::new(
            vec![
                non_matching,
                rule("deny", json!({ "__uc_warn_key__": true })),
            ],
            "allow",
            None,
        );
        acl.check(Some("api.x"), "executor.y", Some(&ctx))
    });

    assert!(!decision);
    assert!(
        warnings.contains("__uc_warn_key__"),
        "the warning must name the condition key: {warnings}"
    );
    assert!(
        warnings.contains("rule_index=1"),
        "the warning must name the rule's index: {warnings}"
    );
    assert!(
        warnings.contains("effect=deny"),
        "the warning must name the rule's effect: {warnings}"
    );
}

// [acl-load-warning] §6.1.2 rule 2: construction warns for every rule
// referencing a condition key with no registered handler, naming the rule
// index, the key and the rule's effect — and MUST NOT fail.
#[test]
fn constructing_an_acl_warns_for_each_unresolvable_condition_key() {
    init();
    let (acl, warnings) = warn_capture::capture(|| {
        ACL::new(
            vec![
                rule("allow", json!({ "roles": ["admin"] })),
                rule("deny", json!({ "__uc_load_warn__": true })),
            ],
            "deny",
            None,
        )
    });

    assert_eq!(acl.rules().len(), 2, "construction still succeeds");
    assert!(
        warnings.contains("__uc_load_warn__")
            && warnings.contains("rule_index=1")
            && warnings.contains("effect=deny"),
        "the load warning must name the rule index, the key and the effect: {warnings}"
    );
    assert!(
        !warnings.contains("roles"),
        "a key that resolves is not warned about: {warnings}"
    );
}

// [acl-add-rule-warning] §6.1.2 rule 4: runtime insertion is an entry point
// that MUST be covered. `add_rule` performed no validation at all before this.
#[test]
fn add_rule_warns_for_an_unresolvable_condition_key() {
    init();
    let mut acl = ACL::new(vec![], "deny", None);
    let ((), warnings) = warn_capture::capture(|| {
        acl.add_rule(rule("deny", json!({ "__uc_add_rule_warn__": true })));
    });

    assert_eq!(acl.rules().len(), 1, "insertion still succeeds");
    assert!(
        warnings.contains("__uc_add_rule_warn__")
            && warnings.contains("rule_index=0")
            && warnings.contains("effect=deny"),
        "add_rule must warn naming the index (0), the key and the effect: {warnings}"
    );
}

// [acl-load-warning-nested] Keys nested in $or / $not are reported too.
#[test]
fn the_load_warning_covers_keys_nested_in_compound_operators() {
    init();
    let (_acl, warnings) = warn_capture::capture(|| {
        ACL::new(
            vec![rule(
                "deny",
                json!({ "$or": [ { "__uc_load_nested__": true } ] }),
            )],
            "deny",
            None,
        )
    });

    assert!(
        warnings.contains("__uc_load_nested__"),
        "a key nested inside $or is just as capable of being misspelled: {warnings}"
    );
}

// ---------------------------------------------------------------------------
// §6.1.1 cases 4 and 5 — a value malformed for its key (spec v1.25.0)
// ---------------------------------------------------------------------------
//
// "Unevaluable" is a PRINCIPLE, not a closed list. Through v1.24.0 the list
// enumerated exactly three situations, and all three SDKs read it as
// exhaustive: a `$or` whose value was not a list simply returned false, which
// is indistinguishable from a handler answering "no" and puts a `deny` rule
// carrying `$or: "typo"` right back into the inert state §6.1.1 exists to end.

// [acl-malformed-or-deny]
#[test]
fn a_malformed_or_operand_makes_a_deny_rule_deny() {
    init();
    let (acl, captured) = acl_with_sink(rule("deny", json!({ "$or": "not-a-list" })), "allow");

    assert!(
        !acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))),
        "a `$or` that is not a list is malformed for its key, so there is no question to \
         answer: UNEVALUABLE, and the deny rule takes effect"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(err.starts_with("$or: "), "path must be `$or`: {err}");
}

// [acl-malformed-or-allow] The allow side: still no grant.
#[test]
fn a_malformed_or_operand_does_not_let_an_allow_rule_grant() {
    init();
    let (acl, _c) = acl_with_sink(rule("allow", json!({ "$or": "not-a-list" })), "deny");

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
}

// [acl-malformed-not-deny]
#[test]
fn a_malformed_not_operand_makes_a_deny_rule_deny() {
    init();
    let (acl, captured) = acl_with_sink(rule("deny", json!({ "$not": 5 })), "allow");

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(err.starts_with("$not: "), "path must be `$not`: {err}");
}

// [acl-malformed-or-branch] A `$or` branch that is not an object is malformed
// for the same reason its container would be.
#[test]
fn a_malformed_or_branch_makes_a_deny_rule_deny() {
    init();
    let (acl, captured) = acl_with_sink(
        rule("deny", json!({ "$or": [ { "roles": ["admin"] }, 7 ] })),
        "allow",
    );

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(err.starts_with("$or[1]: "), "path must be `$or[1]`: {err}");
}

// [acl-non-mapping-conditions] §6.1.1 case 5. `ACL::load` rejects most
// malformed YAML, but direct construction and `add_rule` do not — the same
// door case 4 exists for. This produced a three-way divergence before v1.25.0:
// Python raised out of check(), TypeScript denied, Rust went inert.
#[test]
fn a_non_mapping_conditions_value_makes_a_deny_rule_deny() {
    init();
    let (acl, captured) = acl_with_sink(rule("deny", json!("oops")), "allow");

    assert!(
        !acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))),
        "`conditions` that is not a mapping is UNEVALUABLE, not a non-match"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(
        err.starts_with("$: "),
        "the whole conditions object is path `$`: {err}"
    );
}

// [acl-malformed-async] The async path is separate code; §6.1.1 case 4 applies
// there identically.
#[tokio::test]
async fn async_check_treats_a_malformed_operand_as_unevaluable() {
    init();
    let (acl, captured) = acl_with_sink(rule("deny", json!({ "$or": "not-a-list" })), "allow");

    assert!(
        !acl.async_check(Some("api.x"), "executor.y", Some(&make_context(vec![])))
            .await
    );
    assert!(last_handler_error(&captured).is_some());
}

// ---------------------------------------------------------------------------
// §6.1.4 — the precheck runs before §6.5's no-context check
// ---------------------------------------------------------------------------

// [acl-precheck-before-no-context] THE bypass v1.25.0 closes. All three SDKs
// returned true here: §6.5 short-circuited on the missing context before
// §6.1.1 ever ran, so a misspelled key on a deny rule passed traffic simply
// because the caller carried no identity.
#[test]
fn a_misspelled_key_denies_even_when_the_call_carries_no_context() {
    init();
    let (acl, captured) = acl_with_sink(
        rule("deny", json!({ "__uc_never_registered__": true })),
        "allow",
    );

    assert!(
        !acl.check(Some("api.x"), "executor.y", None),
        "the precheck is context-independent and runs FIRST: a misspelled key is \
         misconfigured regardless of who is calling"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(err.contains("__uc_never_registered__"), "{err}");
}

// [acl-no-context-control] The control that keeps the precheck from
// over-reaching, and the distinction §6.1.4 rule 2 draws: `roles` is
// registered and well-formed, so the rule PASSES the precheck and then takes
// §6.5's no-context path. A registered, context-dependent condition is NOT
// unevaluable merely because this caller sent no context — it is answerable in
// principle, and this caller just did not supply the input.
#[test]
fn a_well_formed_conditional_rule_still_takes_the_no_context_path() {
    init();
    let (acl, captured) = acl_with_sink(rule("deny", json!({ "roles": ["admin"] })), "allow");

    assert!(
        acl.check(Some("api.x"), "executor.y", None),
        "the rule passes the precheck, finds no context, does not match, and the default allows"
    );
    assert_eq!(
        last_handler_error(&captured),
        None,
        "a question this caller did not answer is not a question nobody can answer"
    );
}

// [acl-precheck-before-no-context-async]
#[tokio::test]
async fn async_check_also_prechecks_before_the_no_context_path() {
    init();
    let (acl, _c) = acl_with_sink(
        rule("deny", json!({ "__uc_never_registered__": true })),
        "allow",
    );
    assert!(!acl.async_check(Some("api.x"), "executor.y", None).await);

    let (control, captured) = acl_with_sink(rule("deny", json!({ "roles": ["admin"] })), "allow");
    assert!(control.async_check(Some("api.x"), "executor.y", None).await);
    assert_eq!(last_handler_error(&captured), None);
}

// ---------------------------------------------------------------------------
// §6.1.4 — condition paths
// ---------------------------------------------------------------------------

// [acl-path-nesting] Paths are positional and they nest: a key inside `$not`
// inside the second `$or` branch is `$or[1].$not.k`. Ordering by path rather
// than by key is what makes the diagnostic well-defined when one key occurs at
// two positions.
#[test]
fn handler_error_names_nested_condition_paths() {
    init();
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({
                "$or": [
                    { "roles": ["admin"] },
                    { "$not": { "__uc_deep_missing__": true } }
                ]
            }),
        ),
        "allow",
    );

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(
        err.starts_with("$or[1].$not.__uc_deep_missing__: "),
        "paths nest: {err}"
    );
}

// [acl-path-same-key-twice] The case that makes ordering by KEY undefined: one
// key at two positions. Both are reported, ordered by path.
#[test]
fn the_same_key_at_two_positions_is_reported_at_both_paths() {
    init();
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({
                "$or": [
                    { "__uc_twice__": 1 },
                    { "__uc_twice__": 2 }
                ]
            }),
        ),
        "allow",
    );

    assert!(!acl.check(Some("api.x"), "executor.y", Some(&make_context(vec![]))));
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    let paths: Vec<&str> = err
        .split("; ")
        .map(|e| e.split_once(": ").expect("path: reason").0)
        .collect();
    assert_eq!(
        paths,
        vec!["$or[0].__uc_twice__", "$or[1].__uc_twice__"],
        "one key, two positions, both reported and ordered by path: {err}"
    );
}

// [acl-path-execution-origin] An execution-origin fault two levels down still
// reports its full path — the compound operators carry the path prefix across
// their recursion, which the trait's `evaluate_outcome(value, ctx)` cannot.
#[test]
fn an_execution_fault_inside_a_compound_operator_reports_its_full_path() {
    init();
    ACL::register_condition("__uc_deep_panic__", Arc::new(PanickingHandler));
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({ "$or": [ { "$not": { "__uc_deep_panic__": true } } ] }),
        ),
        "allow",
    );

    let ctx = make_context(vec![]);
    let decision = silence_panic_hook(|| acl.check(Some("api.x"), "executor.y", Some(&ctx)));
    assert!(!decision);
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert!(
        err.starts_with("$or[0].$not.__uc_deep_panic__: "),
        "an execution-origin fault carries the same positional path: {err}"
    );
}

// [acl-validate-rules-structural] §6.1.2 rule 3: the validator reports
// structural faults too, which is why it is `validate_rules` and not
// `validate_conditions`.
#[test]
fn validate_rules_reports_structural_faults_with_both_flags_false() {
    init();
    let acl = ACL::new(
        vec![
            rule("deny", json!({ "$or": "not-a-list" })),
            rule("deny", json!("oops")),
        ],
        "deny",
        None,
    );

    let findings = acl.validate_rules();
    assert_eq!(findings.len(), 2);

    assert_eq!(findings[0].rule_index, 0);
    assert_eq!(findings[0].condition_path, "$or");
    assert_eq!(findings[0].condition_key.as_deref(), Some("$or"));
    assert!(!findings[0].sync_resolvable && !findings[0].async_resolvable);

    assert_eq!(findings[1].rule_index, 1);
    assert_eq!(findings[1].condition_path, "$");
    assert_eq!(
        findings[1].condition_key, None,
        "a `conditions` that is not a mapping has no key to name"
    );
}

// [acl-validate-rules-order] Ordered by rule index, then by path.
#[test]
fn validate_rules_orders_by_rule_index_then_path() {
    init();
    let acl = ACL::new(
        vec![
            rule(
                "deny",
                json!({ "$or": [ { "__uc_ord_z__": true }, { "__uc_ord_a__": true } ] }),
            ),
            rule("allow", json!({ "__uc_ord_m__": true })),
        ],
        "deny",
        None,
    );

    let seen: Vec<(usize, String)> = acl
        .validate_rules()
        .into_iter()
        .map(|f| (f.rule_index, f.condition_path))
        .collect();
    assert_eq!(
        seen,
        vec![
            (0, "$or[0].__uc_ord_z__".to_string()),
            (0, "$or[1].__uc_ord_a__".to_string()),
            (1, "__uc_ord_m__".to_string()),
        ],
        "by rule index, then lexicographically by PATH — note $or[0] precedes $or[1] even \
         though its key sorts later, which is exactly why key ordering was undefined"
    );
}

// ---------------------------------------------------------------------------
// §6.1.4 rule 5 — a precheck fault GATES the rule; execution faults compose
// ---------------------------------------------------------------------------

// [acl-gating-vs-composition-structural] The discriminating case. The caller
// HAS `dev`, so the second `$or` branch is SATISFIED and §6.1.1's table alone
// would make the whole `$or` SATISFIED. A precheck fault gates instead: the
// rule is not well-formed enough to evaluate, so it is UNEVALUABLE as a whole
// and no sibling rescues it.
//
// Gating is what the principle demands — otherwise a typo stays silent for
// exactly as long as some sibling keeps matching, which is the failure mode
// §6.1.1 exists to end, reappearing one nesting level down. It is also
// required for consistency with §6.1.4 rule 1: a precheck fault must resolve a
// context-less call, where no condition is ever evaluated and the table has
// nothing to operate on.
#[test]
fn a_precheck_fault_gates_even_when_an_or_sibling_is_satisfied() {
    init();
    let (acl, captured) = acl_with_sink(
        rule(
            "deny",
            json!({
                "$or": [
                    { "__uc_never_registered__": true },
                    { "roles": ["dev"] }
                ]
            }),
        ),
        "allow",
    );

    assert!(
        !acl.check(
            Some("api.x"),
            "executor.y",
            Some(&make_context(vec!["dev"]))
        ),
        "the satisfied `roles` branch does NOT rescue a rule the precheck rejected"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert_eq!(
        err.split(": ").next(),
        Some("$or[0].__uc_never_registered__")
    );
}

// [acl-gating-vs-composition-execution] The counterpart, and the reason the
// split is not "gate everything". The throwing handler is REGISTERED, so the
// precheck passes and the failure is execution-origin — §6.1.1's `$or` table
// applies, the satisfied `roles` branch wins, and the ALLOW rule GRANTS.
//
// An implementation that gated on execution-origin faults too would deny here.
#[test]
fn an_execution_fault_does_not_gate_when_an_or_sibling_is_satisfied() {
    init();
    ACL::register_condition("__uc_gate_split_panic__", Arc::new(PanickingHandler));
    let (acl, captured) = acl_with_sink(
        rule(
            "allow",
            json!({
                "$or": [
                    { "__uc_gate_split_panic__": true },
                    { "roles": ["dev"] }
                ]
            }),
        ),
        "deny",
    );

    let ctx = make_context(vec!["dev"]);
    let decision = silence_panic_hook(|| acl.check(Some("api.x"), "executor.y", Some(&ctx)));
    assert!(
        decision,
        "an outright SATISFIED branch wins the $or over an execution-origin failure, so the \
         allow rule matches and GRANTS"
    );
    assert!(
        last_handler_error(&captured).is_some(),
        "the diagnostic is still recorded even though the rule matched"
    );
}

// ---------------------------------------------------------------------------
// §6.1.4 rule 4 — the precheck must not widen a rule's reach
// ---------------------------------------------------------------------------

// [acl-precheck-does-not-widen-reach] A rule that a well-formed pattern field
// definitively excludes from this call does not participate in the decision,
// and its condition faults MUST NOT be consulted, reported, or allowed to
// change the outcome.
//
// Without this, one misspelled key in a narrowly scoped rule would decide
// calls the rule was never written about: `callers: ["api.*"]` with a typo'd
// condition and `effect: deny` would deny a `worker.*` caller, which breaks
// first-match-wins.
#[test]
fn a_condition_fault_in_a_non_matching_rule_does_not_affect_the_call() {
    init();
    let mut scoped = ACLRule::new(vec!["api.*".to_string()], vec!["*".to_string()], "deny");
    scoped.description = Some("narrowly scoped deny with a typo".to_string());
    scoped.conditions = Some(json!({ "__uc_never_registered__": true }));

    let (acl, captured) = acl_with_sink(scoped, "allow");

    assert!(
        acl.check(
            Some("worker.job"),
            "executor.y",
            Some(&make_context(vec![]))
        ),
        "the caller pattern excludes this call, so the rule does not apply and its typo is \
         irrelevant to the decision"
    );
    assert_eq!(
        last_handler_error(&captured),
        None,
        "a fault in a rule that does not apply MUST NOT be reported in handler_error"
    );
}

// [acl-precheck-target-mismatch] The same, via `targets`.
#[test]
fn a_condition_fault_in_a_target_mismatched_rule_does_not_affect_the_call() {
    init();
    let mut scoped = ACLRule::new(
        vec!["*".to_string()],
        vec!["executor.payment.*".to_string()],
        "deny",
    );
    scoped.conditions = Some(json!({ "__uc_never_registered__": true }));

    let (acl, captured) = acl_with_sink(scoped, "allow");

    assert!(acl.check(
        Some("api.x"),
        "executor.email.send",
        Some(&make_context(vec![]))
    ));
    assert_eq!(last_handler_error(&captured), None);
}

// [acl-validate-rules-reports-scoped-faults] The fault is still real, and
// `validate_rules()` is where a scoped rule's typo is meant to surface: it
// reports every fault in every rule regardless of any call.
#[test]
fn validate_rules_reports_a_fault_in_a_rule_no_call_would_match() {
    init();
    let mut scoped = ACLRule::new(
        vec!["never.matches.anything".to_string()],
        vec!["also.never".to_string()],
        "deny",
    );
    scoped.conditions = Some(json!({ "__uc_scoped_missing__": true }));

    let acl = ACL::new(vec![scoped], "deny", None);

    let findings = acl.validate_rules();
    assert_eq!(findings.len(), 1, "static analysis is call-independent");
    assert_eq!(findings[0].condition_path, "__uc_scoped_missing__");
}

// [acl-precheck-does-not-widen-reach-async] The async rule loop is separate
// code and must not widen reach either.
#[tokio::test]
async fn async_check_also_ignores_a_condition_fault_in_a_non_matching_rule() {
    init();
    let mut scoped = ACLRule::new(vec!["api.*".to_string()], vec!["*".to_string()], "deny");
    scoped.conditions = Some(json!({ "__uc_never_registered__": true }));

    let (acl, captured) = acl_with_sink(scoped, "allow");

    assert!(
        acl.async_check(
            Some("worker.job"),
            "executor.y",
            Some(&make_context(vec![]))
        )
        .await
    );
    assert_eq!(last_handler_error(&captured), None);
}

// [acl-or-element-fault-not-skipped] §6.1.1 case 4 extended by the principle:
// an `$or` ELEMENT that is not a condition object is a fault at `$or[i]`, not
// a branch to skip. Skipping is how `$or: ["typo"]` on a deny rule stays
// inert — the same defect one level down. The satisfied sibling does not
// rescue it either, because this is a precheck fault (rule 5).
#[test]
fn a_non_object_or_element_is_a_fault_rather_than_a_skipped_branch() {
    init();
    let (acl, captured) = acl_with_sink(
        rule("deny", json!({ "$or": [ "typo", { "roles": ["dev"] } ] })),
        "allow",
    );

    assert!(
        !acl.check(
            Some("api.x"),
            "executor.y",
            Some(&make_context(vec!["dev"]))
        ),
        "the malformed branch is a fault, so the rule is gated even though the sibling matches"
    );
    let err = last_handler_error(&captured).expect("handler_error MUST be non-null");
    assert_eq!(err.split(": ").next(), Some("$or[0]"));
}

// [acl-keyless-fault-flags] The keyed/keyless split (§6.1.3 rule 3).
//
// A malformed operator *value* keeps its key: the fault is attached to `$or` /
// `$not`, so the finding names it. A malformed *element* is keyless — there is
// no key at that position — as are a non-mapping `conditions` and a malformed
// pattern field.
//
// Both resolvability flags are `false` for every one of them. The flags
// describe the FAULT, not the key's registration: `$or` is always registered,
// so "is the key registered" would report a malformed `$or` value as resolvable
// on both paths — i.e. would say a malformed rule is fine.
#[test]
fn keyless_and_keyed_structural_faults_report_the_right_key_and_both_flags_false() {
    init();
    let acl = ACL::new(
        vec![
            rule("deny", json!({ "$or": [ "typo" ] })),
            rule("deny", json!("not-a-mapping")),
            rule("deny", json!({ "$or": "not-a-list" })),
            rule("deny", json!({ "$not": 3 })),
        ],
        "deny",
        None,
    );

    let findings = acl.validate_rules();
    let seen: Vec<(usize, &str, Option<&str>)> = findings
        .iter()
        .map(|f| {
            (
                f.rule_index,
                f.condition_path.as_str(),
                f.condition_key.as_deref(),
            )
        })
        .collect();

    assert_eq!(
        seen,
        vec![
            // A malformed ELEMENT is keyless.
            (0, "$or[0]", None),
            // A non-mapping `conditions` is keyless.
            (1, "$", None),
            // A malformed operator VALUE keeps its key.
            (2, "$or", Some("$or")),
            (3, "$not", Some("$not")),
        ]
    );

    for finding in &findings {
        assert!(
            !finding.sync_resolvable && !finding.async_resolvable,
            "a structural fault resolves on neither path, even when its key IS registered — \
             the flags describe the fault, not the registration: {finding:?}"
        );
    }
}

// [acl-registry-fault-flags] The contrast that gives the flags their meaning:
// a REGISTRY fault reports the key's real resolvability, and an async-only key
// is the case where the two flags differ.
#[test]
fn a_registry_fault_reports_the_keys_real_resolvability() {
    init();
    ACL::register_async_condition(
        "__uc_flags_async_only__",
        Arc::new(SuspendingHandler { answer: true }),
    );
    let acl = ACL::new(
        vec![
            rule("deny", json!({ "__uc_flags_async_only__": true })),
            rule("deny", json!({ "__uc_flags_nowhere__": true })),
        ],
        "deny",
        None,
    );

    let findings = acl.validate_rules();
    assert_eq!(findings.len(), 2);

    assert_eq!(
        findings[0].condition_key.as_deref(),
        Some("__uc_flags_async_only__")
    );
    assert!(
        !findings[0].sync_resolvable && findings[0].async_resolvable,
        "an async-only key is unevaluable under check() and fine under async_check()"
    );

    assert_eq!(
        findings[1].condition_key.as_deref(),
        Some("__uc_flags_nowhere__")
    );
    assert!(!findings[1].sync_resolvable && !findings[1].async_resolvable);
}
