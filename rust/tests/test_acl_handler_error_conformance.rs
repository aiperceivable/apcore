//! Conformance driver for `acl_handler_error.json` (PROTOCOL_SPEC §6.1.1 /
//! §6.1.4 / §6.3.1, sync findings A-D-011 / A-D-012, SECURITY).
//!
//! A condition that CANNOT BE EVALUATED is not a condition that is FALSE, and
//! the rule's `effect` decides what the difference means. An unevaluable
//! condition MUST resolve the rule toward refusing access: a `deny` rule takes
//! effect and the call is DENIED; an `allow` rule does not match and MUST NOT
//! grant. The emitted `AuditEntry` MUST carry a non-null `handler_error` in
//! both directions, naming the offending condition **path**.
//!
//! "Unevaluable" is a PRINCIPLE, not a closed list (§6.1.1): no resolvable
//! handler, a handler that panics, an async handler unresolvable on the sync
//! path, a value malformed for its key, or a `conditions` that is not a
//! mapping. §6.1.4's precheck is context-independent and runs BEFORE §6.5's
//! no-context check, so a malformed rule is unevaluable even for a caller that
//! supplied no context.
//!
//! Driver contract (from the fixture `description`): register a test condition
//! handler under `throwing_condition_key` whose evaluate panics, register
//! NOTHING for `unknown_condition_key`, build an ACL from `rules` +
//! `default_effect` with an audit sink, call `check(caller_id, target_id)` —
//! passing a context only when `with_context` is true. Assert the decision
//! equals `expected`, that `AuditEntry.handler_error` is non-null when
//! `expected_audit_handler_error_present` is true, and — where
//! `expected_handler_error_paths` is present — that `handler_error` names
//! exactly those paths, in that order (§6.1.1 rule 2 orders by path).
//!
//! # Cases this SDK skips, and why
//!
//! Two cases carry `callers_raw` / `targets_raw` with `skip_if_unrepresentable`
//! to exercise §6.1.4.1: a `callers` written as a bare string or a scalar. A
//! bare string is iterable in several host languages, so `callers: "admin.*"`
//! iterates character by character and its `*` matches everything — measured in
//! apcore-python, that typo granted an unrelated caller under
//! `default_effect: deny`.
//!
//! apcore-rust cannot represent the value: [`ACLRule::callers`] is
//! `Vec<String>`, so serde rejects a bare string at deserialization and the
//! compiler rejects one in a struct literal. §6.1.4.1 says such an
//! implementation "satisfies this clause by construction and needs no runtime
//! check", and the fixture says a driver in that position MUST skip those cases
//! and say so rather than report them as passing. [`SKIPPED_UNREPRESENTABLE`]
//! is the assertion that keeps the skip honest: exactly the cases the fixture
//! marks may be skipped, and every one of them must actually be unrepresentable.
//!
//! # Staged-fixture rollout (apcore#100)
//!
//! The corrected fixture lives at
//! `apcore/planning/acl-unevaluable-conditions/staged-fixtures/` rather than in
//! `conformance/fixtures/`, so CI does not go red across every SDK repository
//! while the three drivers land one at a time. The driver prefers the staged
//! copy and falls back to the canonical one, which means it needs no change
//! when the fixture is promoted.
//!
//! Against the pre-v1.22.0 fixture, [`SUPERSEDED_BY_V1_22_0`] overrode one case
//! by ID. The corrected fixture drops that id, so the table is inert; the
//! guard in [`acl_handler_error_conformance`] asserts that it is, and the table
//! can be deleted once no SDK reads the old fixture.

#![allow(clippy::pedantic)] // fixture-driven test file: casts/layout follow the fixture schema

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use apcore::acl::{ACLRule, AuditEntry, ACL};
use apcore::acl_handlers::ACLConditionHandler;
use apcore::context::{Context, Identity};
use async_trait::async_trait;
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

use crate::conformance_env::{find_fixtures_root, find_staged_fixture};

/// Prefer the staged (corrected) fixture; fall back to the canonical one.
///
/// The fallback is what makes this driver survive the promotion: when
/// `planning/.../staged-fixtures/` goes away, `conformance/fixtures/` holds the
/// same content and nothing here changes.
fn load_fixture() -> Value {
    let path = find_staged_fixture("acl-unevaluable-conditions", "acl_handler_error.json")
        .unwrap_or_else(|| find_fixtures_root().join("acl_handler_error.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON: {e}"))
}

// ---------------------------------------------------------------------------
// Test condition handler that panics during evaluation.
// ---------------------------------------------------------------------------

struct ThrowingHandler;

#[async_trait]
impl ACLConditionHandler for ThrowingHandler {
    async fn evaluate(&self, _value: &Value, _ctx: &Context<Value>) -> bool {
        panic!("conformance: __test_throwing_condition__ simulated handler panic");
    }
}

/// Build the context for a case that sets `with_context: true`, carrying
/// exactly the identity its `context_identity` declares.
///
/// The identity is fixture data, not a driver choice: several cases turn on
/// whether a `roles` condition is SATISFIED, and a bare identity-less context
/// makes `execution_fault_does_not_gate_when_an_or_sibling_is_satisfied` return
/// `false` where `true` is expected — the `roles` branch goes UNSATISFIED, the
/// `$or` UNEVALUABLE, and the `allow` rule correctly does not grant. That
/// failure looks exactly like over-gating in the precheck and is not, which is
/// why the fixture now states the identity rather than leaving it to the
/// driver.
fn make_context(context_identity: Option<&Value>) -> Context<Value> {
    let declared =
        context_identity.expect("a case with with_context: true must declare context_identity");
    let id = declared["id"]
        .as_str()
        .expect("context_identity.id")
        .to_string();
    let identity_type = declared["type"]
        .as_str()
        .expect("context_identity.type")
        .to_string();
    let roles = declared["roles"]
        .as_array()
        .expect("context_identity.roles")
        .iter()
        .map(|v| v.as_str().expect("role is a string").to_string())
        .collect();
    Context::new(Identity::new(id, identity_type, roles, HashMap::new()))
}

/// Whether a fixture rule uses a `callers` / `targets` shape this SDK cannot
/// represent (§6.1.4.1).
///
/// `callers_raw` / `targets_raw` supply a non-list value in place of the normal
/// field. [`ACLRule::callers`] and [`ACLRule::targets`] are `Vec<String>`, so
/// there is no value to construct and no runtime state to test — the clause is
/// satisfied by the type system.
fn is_unrepresentable(rule: &Value) -> bool {
    rule.get("callers_raw").is_some() || rule.get("targets_raw").is_some()
}

/// Build an `ACLRule` from a fixture rule object.
fn build_rule(rule: &Value) -> ACLRule {
    let callers = rule["callers"]
        .as_array()
        .expect("rule.callers")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let targets = rule["targets"]
        .as_array()
        .expect("rule.targets")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let effect = rule["effect"].as_str().expect("rule.effect").to_string();
    // Carried through VERBATIM, including a `conditions` that is not a mapping:
    // §6.1.1 case 5 is exactly that shape, and coercing it to `None` here would
    // quietly turn the case under test into a rule with no conditions at all.
    let conditions = rule.get("conditions").cloned();
    let mut built = ACLRule::new(callers, targets, effect);
    built.conditions = conditions;
    built
}

/// The condition paths named by a `handler_error` string.
///
/// `handler_error` is `"{path}: {reason}"` entries joined with `"; "`
/// (§6.1.1 rule 2), so the paths are the prefixes before the first `": "` of
/// each entry, already ordered lexicographically by path.
fn handler_error_paths(handler_error: &str) -> Vec<String> {
    handler_error
        .split("; ")
        .map(|entry| {
            entry
                .split_once(": ")
                .map_or_else(|| entry.to_string(), |(path, _)| path.to_string())
        })
        .collect()
}

/// Suppress the default panic-hook stderr noise so caught panics in these
/// tests don't spam the log.
fn silence_panic_hook<T>(f: impl FnOnce() -> T) -> T {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = f();
    std::panic::set_hook(prev);
    out
}

/// Cases whose expectation spec v1.22.0 (#100) changed, mapped to the value
/// the corrected fixture asserts. Consulted only while the fixture on disk is
/// still the pre-v1.22.0 generation — see the module docs. Inert against the
/// corrected fixture, which drops the id.
const SUPERSEDED_BY_V1_22_0: &[(&str, bool)] = &[(
    // A `deny` rule + a panicking handler + `default_effect: allow`. Was
    // `true`: the deny rule did not match and the call fell through to the
    // permissive default, so a handler crash silently disabled the block.
    // §6.1.1 now makes the deny rule take effect.
    "throwing_handler_does_not_flip_default_allow_to_deny_unsafely",
    false,
)];

/// Case ids this SDK is permitted to skip, because the fixture marks them
/// `skip_if_unrepresentable` and apcore-rust's `Vec<String>` makes the value
/// impossible to construct (§6.1.4.1).
const SKIPPED_UNREPRESENTABLE: &[&str] = &[
    "string_callers_on_allow_rule_must_not_grant",
    "scalar_callers_on_deny_rule_denies_without_raising",
];

/// The expectation to assert for `case_id`, honouring the staged-fixture
/// override while the corrected fixture has not yet landed.
fn superseded_expectation(fixture_is_corrected: bool, case_id: &str, from_fixture: bool) -> bool {
    if fixture_is_corrected {
        return from_fixture;
    }
    SUPERSEDED_BY_V1_22_0
        .iter()
        .find(|(id, _)| *id == case_id)
        .map_or(from_fixture, |(_, corrected)| *corrected)
}

#[test]
fn acl_handler_error_conformance() {
    let fixture = load_fixture();
    let throwing_key = fixture["throwing_condition_key"]
        .as_str()
        .expect("fixture must declare throwing_condition_key");
    // The corrected (v1.22.0+) fixture adds `unknown_condition_key`; the old
    // one has no such field. That is the generation switch.
    let fixture_is_corrected = fixture.get("unknown_condition_key").is_some();

    // Register the panicking handler under the fixture-declared key (global,
    // process-wide registry). The key is fixture-specific and unique, so
    // cross-test interference is not a concern. Nothing is registered for
    // `unknown_condition_key` — that is the point of the case.
    ACL::init_builtin_handlers();
    ACL::register_condition(throwing_key, Arc::new(ThrowingHandler));

    let cases = fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array");
    let ids: Vec<&str> = cases.iter().filter_map(|tc| tc["id"].as_str()).collect();

    if fixture_is_corrected {
        // The override must be dead against the corrected fixture. If a
        // superseded id reappeared, the override would silently start
        // rewriting a live expectation again.
        for (superseded_id, _) in SUPERSEDED_BY_V1_22_0 {
            assert!(
                !ids.contains(superseded_id),
                "the corrected fixture still carries '{superseded_id}', so \
                 SUPERSEDED_BY_V1_22_0 is NOT inert — it would override a live expectation"
            );
        }
    } else {
        // Keep the override table honest: a superseded id that no longer
        // appears is dead weight that would hide a real regression.
        for (superseded_id, _) in SUPERSEDED_BY_V1_22_0 {
            assert!(
                ids.contains(superseded_id),
                "SUPERSEDED_BY_V1_22_0 names case '{superseded_id}', which the fixture no \
                 longer carries. Either the corrected fixture landed (drop the entry, and the \
                 whole override with it) or the case was renamed."
            );
        }
    }

    let mut executed: Vec<&str> = Vec::new();
    let mut skipped: Vec<&str> = Vec::new();

    for tc in cases {
        let id = tc["id"].as_str().expect("each case needs an id");

        let raw_rules = tc["rules"].as_array().expect("case.rules");

        // §6.1.4.1: a malformed `callers` / `targets` is unrepresentable here.
        // Skip loudly rather than silently pass — a skipped case that reported
        // as green would claim coverage this SDK does not have.
        if raw_rules.iter().any(is_unrepresentable) {
            assert!(
                tc.get("skip_if_unrepresentable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                "case {id} carries an unrepresentable callers/targets value but is NOT marked \
                 skip_if_unrepresentable; it cannot be skipped"
            );
            assert!(
                SKIPPED_UNREPRESENTABLE.contains(&id),
                "case {id} is being skipped but is not listed in SKIPPED_UNREPRESENTABLE; add \
                 it deliberately or make the case run"
            );
            skipped.push(id);
            continue;
        }

        let rules: Vec<ACLRule> = raw_rules.iter().map(build_rule).collect();
        let default_effect = tc["default_effect"].as_str().expect("case.default_effect");
        let caller_id = tc["caller_id"].as_str();
        let target_id = tc["target_id"].as_str().expect("case.target_id");
        let expected = superseded_expectation(
            fixture_is_corrected,
            id,
            tc["expected"].as_bool().expect("case.expected"),
        );
        let expect_handler_error = tc["expected_audit_handler_error_present"]
            .as_bool()
            .unwrap_or(false);
        // Absent on the pre-v1.22.0 fixture, whose cases all supplied a context.
        let with_context = tc
            .get("with_context")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_logger = Arc::clone(&captured);
        let mut acl = ACL::new(rules, default_effect, None);
        acl.set_audit_logger(move |entry: &AuditEntry| {
            captured_for_logger.lock().unwrap().push(entry.clone());
        });

        // Built only when the case asks for one, so a `with_context: false`
        // case cannot accidentally depend on an identity it never receives.
        let ctx = with_context.then(|| make_context(tc.get("context_identity")));
        let decision = silence_panic_hook(|| acl.check(caller_id, target_id, ctx.as_ref()));

        assert_eq!(
            decision, expected,
            "case {id}: an unevaluable condition must resolve the rule toward \
             refusing access (PROTOCOL_SPEC §6.1.1) — expected decision \
             {expected}, got {decision}"
        );

        let entries = captured.lock().unwrap();
        let entry = entries
            .last()
            .unwrap_or_else(|| panic!("case {id}: an audit entry must be emitted"));
        if expect_handler_error {
            assert!(
                entry.handler_error.is_some(),
                "case {id}: AuditEntry.handler_error must be non-null when a \
                 condition is unevaluable, got {:?}",
                entry.handler_error
            );
        } else {
            assert!(
                entry.handler_error.is_none(),
                "case {id}: AuditEntry.handler_error must be NULL — this case is the control \
                 that catches a precheck which over-reaches into well-formed rules. Got {:?}",
                entry.handler_error
            );
        }

        // §6.1.1 rule 2: the exact paths, in path order.
        if let Some(expected_paths) = tc
            .get("expected_handler_error_paths")
            .and_then(Value::as_array)
        {
            let want: Vec<String> = expected_paths
                .iter()
                .map(|v| v.as_str().expect("path is a string").to_string())
                .collect();
            let handler_error = entry
                .handler_error
                .as_deref()
                .unwrap_or_else(|| panic!("case {id}: handler_error must be present"));
            assert_eq!(
                handler_error_paths(handler_error),
                want,
                "case {id}: handler_error must name exactly these condition paths, in this \
                 order (§6.1.1 rule 2 orders lexicographically by path). Got {handler_error:?}"
            );
        }

        executed.push(id);
    }

    println!(
        "acl_handler_error: {} case(s) executed, {} skipped as unrepresentable ({})",
        executed.len(),
        skipped.len(),
        if skipped.is_empty() {
            "none".to_string()
        } else {
            skipped.join(", ")
        }
    );
    assert!(
        !executed.is_empty(),
        "the fixture produced no executable case"
    );
    // Every skip must be one the fixture sanctioned, and the corrected fixture
    // carries exactly two.
    for id in &skipped {
        assert!(SKIPPED_UNREPRESENTABLE.contains(id));
    }
    if fixture_is_corrected {
        assert_eq!(
            skipped.len(),
            SKIPPED_UNREPRESENTABLE.len(),
            "the corrected fixture is expected to carry exactly {} unrepresentable case(s); \
             skipped {skipped:?}",
            SKIPPED_UNREPRESENTABLE.len()
        );
    }
}

/// The unknown-key half of the corrected fixture, covered natively until that
/// fixture lands (see the module docs).
///
/// A condition key nobody registered is UNEVALUABLE, not false — this is the
/// misspelled-key case #100 was opened for (`role:` written for `roles:`). On
/// an `allow` rule the outcome is unchanged (no grant); on a `deny` rule the
/// rule now takes effect instead of falling through to a permissive
/// `default_effect`.
#[test]
fn unknown_condition_key_resolves_toward_refusing_access() {
    ACL::init_builtin_handlers();
    // Deliberately NOT registered anywhere.
    let unknown_key = "__test_unregistered_condition_rs__";

    // (effect, default_effect, expected decision)
    let cases = [
        ("allow", "deny", false),
        // The regression guard: pre-v1.22.0 this returned `true`.
        ("deny", "allow", false),
    ];

    for (effect, default_effect, expected) in cases {
        let mut conditions = Map::new();
        conditions.insert(unknown_key.to_string(), Value::Bool(true));
        let mut rule = ACLRule::new(vec!["*".to_string()], vec!["*".to_string()], effect);
        rule.conditions = Some(Value::Object(conditions));

        let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_logger = Arc::clone(&captured);
        let mut acl = ACL::new(vec![rule], default_effect, None);
        acl.set_audit_logger(move |entry: &AuditEntry| {
            captured_for_logger.lock().unwrap().push(entry.clone());
        });

        let ctx = make_context(Some(
            &json!({ "id": "user", "type": "user", "roles": ["dev"] }),
        ));
        let decision = acl.check(Some("user"), "service.operation", Some(&ctx));
        assert_eq!(
            decision, expected,
            "effect={effect} default_effect={default_effect}: an unregistered condition key is \
             unevaluable (PROTOCOL_SPEC §6.1.1), so a deny rule denies and an allow rule does \
             not grant"
        );

        let entries = captured.lock().unwrap();
        let entry = entries.last().expect("an audit entry must be emitted");
        let handler_error = entry
            .handler_error
            .as_deref()
            .expect("AuditEntry.handler_error must be non-null for an unevaluable condition");
        assert!(
            handler_error.contains(unknown_key),
            "handler_error must name the offending condition key: {handler_error}"
        );
    }
}
