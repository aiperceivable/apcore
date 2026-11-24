//! Conformance driver for `acl_argument_scoped_approval.json`
//! (PROTOCOL_SPEC §6.1.1 / §6.1.6 / §6.1.7 / §6.1.8 / §6.8.1 / §6.9,
//! spec v1.28.0 apcore#108, extended v1.29.0 apcore#109).
//!
//! An ACL rule answers two independent questions — may this caller reach this
//! target at all, and must *this particular call* be put to a human first. The
//! orthogonal `approval` field carries the second, and the built-in
//! structure-only `arguments` condition decides whether a rule matches this
//! call.
//!
//! # Every case runs twice
//!
//! The `arguments` condition can only be answered when a governance projection
//! is available, and §6.1.8 case 1 makes `check()` a public entry point that
//! may be called without one — so the same rules and the same call have two
//! well-defined answers, and both are contracts. Run 1 supplies a projection
//! derived from `arguments`; run 2 supplies NO PROJECTION AT ALL. Keys ending
//! `_no_projection` belong to run 2, the unsuffixed ones to run 1.
//! `arguments: null` means the case has no projection to supply in either run,
//! so the two runs coincide.
//!
//! **What this SDK can and cannot assert.** `ACL::check(caller_id, target_id,
//! ctx)` has no parameter for a projection and passes `None` to `check_access`
//! internally. That is conforming — §6.1.8 rule 4 fixes only that the
//! *condition* sees the projection, and §6.1.8 case 1 contemplates `check()`
//! being called without one — so the legacy boolean here answers run 2's
//! question, and `expected_legacy_check_no_projection` is the column it is
//! asserted against, on every case. `expected_legacy_check` (run 1's boolean,
//! for an SDK whose `check()` does take a projection) is the ONE key this
//! driver cannot assert; asserting it would be asserting the answer to a
//! different question. Both STRUCTURED columns remain fully assertable via
//! `check_access(.., Some(&projection))` and `check_access(.., None)`, which
//! is what makes the unassertable key a single boolean rather than a hole.
//!
//! This two-column contract replaces a driver-side skip. The skip was honest
//! and reconciled against the fixture, but it left 17 of 20 cases unverified
//! here, and apcore#109 — an unevaluable `allow` rule discarding the
//! `approval: required` it carried, so a broader rule granted the call
//! unapproved — was sitting in exactly those cases.
//!
//! The two cases worth reading before the rest are
//! `no_projection_must_not_grant_via_an_empty_stand_in` and
//! `no_projection_makes_a_deny_rule_take_effect`: they bracket the same
//! fail-open bug from both directions. Substituting an empty key set for an
//! absent projection makes `has_none_of` vacuously satisfied, so an `allow`
//! rule grants for a call whose arguments were never seen — and leaves
//! `has_key` unsatisfied, so a `deny` rule fails to take effect. Only the
//! UNEVALUABLE reading of §6.1.8 rule 1 refuses in both directions.

use apcore::acl::{
    ACLRule, AccessDecision, ApprovalRequirement, AuditEntry, GovernanceProjection, ACL,
};
use apcore::context::{Context, Identity};
use serde_json::Value;
use std::sync::{Arc, Mutex};

use crate::conformance_env::find_fixtures_root;

const FIXTURE: &str = "acl_argument_scoped_approval.json";

/// Case ids this SDK is permitted to skip, because the fixture marks them
/// `skip_if_unrepresentable` and apcore-rust's `Vec<String>` makes the value
/// impossible to construct (§6.1.4.1). Same convention as the
/// `acl_handler_error.json` driver, and reconciled against the fixture after
/// the loop so a case that stops being unrepresentable starts being asserted
/// instead of silently staying skipped.
const SKIPPED_UNREPRESENTABLE: &[&str] = &[
    "malformed_pattern_field_raises_the_pending_requirement",
    "malformed_targets_field_raises_the_pending_requirement",
];

/// The fixture lands in the spec repo one push after this driver, so that
/// `check_driver_coverage.py --strict` has a driver to find for it. Until then
/// the driver reports "not verified" rather than a pass it did not earn.
fn load_fixture() -> Option<Value> {
    let path = find_fixtures_root().join(FIXTURE);
    if !path.is_file() {
        return None;
    }
    Some(
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
            .expect("parse fixture"),
    )
}

/// Whether a fixture rule uses a `callers` / `targets` shape this SDK cannot
/// represent (§6.1.4.1).
///
/// `callers_raw` / `targets_raw` supply a non-list value in place of the normal
/// field. [`ACLRule::callers`] and [`ACLRule::targets`] are `Vec<String>`, so
/// there is no value to construct and no runtime state to test — §6.1.1 rule
/// 5's "unknowable scope counts as scope" clause is satisfied by the type
/// system rather than by code, and there is nothing here to exercise.
fn is_unrepresentable(rule: &Value) -> bool {
    rule.get("callers_raw").is_some() || rule.get("targets_raw").is_some()
}

fn build(case: &Value) -> (ACL, Arc<Mutex<Vec<AuditEntry>>>) {
    let entries: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&entries);
    let rules: Vec<ACLRule> = case["rules"]
        .as_array()
        .expect("rules array")
        .iter()
        .map(|r| {
            let mut rule = ACLRule::new(
                r["callers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c.as_str().unwrap().to_string())
                    .collect(),
                r["targets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t.as_str().unwrap().to_string())
                    .collect(),
                r["effect"].as_str().unwrap().to_string(),
            );
            rule.approval = r.get("approval").and_then(Value::as_str).map(|a| match a {
                "required" => ApprovalRequirement::Required,
                _ => ApprovalRequirement::NotRequired,
            });
            rule.conditions = r.get("conditions").cloned();
            rule
        })
        .collect();
    let acl = ACL::new(
        rules,
        case["default_effect"].as_str().unwrap(),
        Some(Arc::new(move |entry: &AuditEntry| {
            sink.lock().unwrap().push(entry.clone())
        })),
    );
    (acl, entries)
}

fn context() -> Context<Value> {
    Context::create(
        Some(Identity::new(
            "u".to_string(),
            "user".to_string(),
            vec!["dev".to_string()],
            std::collections::HashMap::new(),
        )),
        None,
        None,
        None,
        Value::Null,
        None,
    )
}

/// `arguments: null` means no projection at all (§6.1.8 rule 1) — NOT a
/// projection of `{}`, which is a materially different input.
fn projection(case: &Value) -> Option<GovernanceProjection> {
    match &case["arguments"] {
        Value::Null => None,
        args => Some(GovernanceProjection::from_arguments(args)),
    }
}

/// One of the two runs: the expectation keys it reads and the projection it
/// supplies.
struct Run {
    /// `""` for run 1, `"_no_projection"` for run 2 — the fixture's key suffix.
    suffix: &'static str,
    /// Whether this run supplies the projection derived from `arguments`.
    with_projection: bool,
}

const RUNS: [Run; 2] = [
    Run {
        suffix: "",
        with_projection: true,
    },
    Run {
        suffix: "_no_projection",
        with_projection: false,
    },
];

/// Read one expectation key for a run, panicking rather than defaulting: a
/// missing key is a fixture the driver does not understand, not a `false`.
fn expect_bool(case: &Value, key: &str, suffix: &str) -> bool {
    let full = format!("{key}{suffix}");
    case[&full]
        .as_bool()
        .unwrap_or_else(|| panic!("case {} is missing '{full}'", case["id"]))
}

#[test]
fn acl_argument_scoped_approval_conformance() {
    let Some(fixture) = load_fixture() else {
        eprintln!(
            "SKIP: {FIXTURE} not in the spec repo yet (spec v1.29.0, apcore#109) — NOT VERIFIED"
        );
        return;
    };
    let cases = fixture["test_cases"].as_array().expect("test_cases array");
    assert!(!cases.is_empty(), "fixture carries no cases");
    let mut skipped: Vec<&str> = Vec::new();
    let mut executed = 0usize;

    for case in cases {
        let id = case["id"].as_str().unwrap();
        let note = case["note"].as_str().unwrap();

        // §6.1.4.1: a malformed `callers` / `targets` is unrepresentable here.
        // Skip loudly rather than silently pass — a skipped case reported as
        // green would claim coverage this SDK does not have.
        if case["rules"]
            .as_array()
            .expect("rules array")
            .iter()
            .any(is_unrepresentable)
        {
            assert!(
                case.get("skip_if_unrepresentable")
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

        for run in &RUNS {
            let label = if run.with_projection {
                "with projection"
            } else {
                "no projection"
            };
            let (acl, entries) = build(case);
            let ctx = context();
            let caller = case["caller_id"].as_str().unwrap();
            let target = case["target_id"].as_str().unwrap();
            let proj = projection(case).filter(|_| run.with_projection);

            let decision: AccessDecision =
                acl.check_access(Some(caller), target, Some(&ctx), proj.as_ref());
            let want_access = case[&format!("expected_access{}", run.suffix)]
                .as_str()
                .expect("expected_access column");
            assert_eq!(decision.access, want_access, "[{id}] ({label}) {note}");
            let want_approval = expect_bool(case, "expected_approval_required", run.suffix);
            assert_eq!(
                decision.approval_required, want_approval,
                "[{id}] ({label}) {note}"
            );
            assert_eq!(
                decision.matched_rule_index,
                case[&format!("expected_matched_rule_index{}", run.suffix)]
                    .as_u64()
                    .map(|i| i as usize),
                "[{id}] ({label}) {note}"
            );

            // §6.3.1: handler_error is non-null IF AND ONLY IF a condition was
            // unevaluable. Read before the legacy call below, which emits its
            // own entry.
            let logged = entries.lock().unwrap().clone();
            assert_eq!(
                logged.len(),
                1,
                "[{id}] ({label}) check_access must emit exactly one audit entry"
            );
            let entry = &logged[0];
            assert_eq!(
                entry.handler_error.is_some(),
                expect_bool(case, "expected_audit_handler_error_present", run.suffix),
                "[{id}] ({label}) {note}\n  handler_error was {:?}",
                entry.handler_error
            );
            // §6.1.1 rule 5: a pending requirement neither suppresses nor
            // substitutes for `handler_error`, and the audit entry carries the
            // FINAL approval value rather than the matched rule's own.
            assert_eq!(
                entry.approval_required, want_approval,
                "[{id}] ({label}) {note}"
            );

            // §6.8.1: the legacy boolean fails closed on an approval
            // requirement, so allow + required reads as false — and since
            // v1.29.0 that is a property of the DECISION, so it fails closed on
            // a pending requirement carried through a later rule or through
            // `default_effect: allow` identically.
            //
            // `ACL::check` takes no projection in this SDK, so it always
            // answers run 2's question. It is asserted once, on run 2's column;
            // `expected_legacy_check` (run 1's column) is the one key this
            // driver leaves unasserted — see the module docs.
            if !run.with_projection {
                assert_eq!(
                    acl.check(Some(caller), target, Some(&ctx)),
                    expect_bool(case, "expected_legacy_check", run.suffix),
                    "[{id}] ({label}) {note}"
                );
            }

            executed += 1;
        }

        // §6.1.8 closing paragraph: the well-formedness cases are decidable
        // with no context and no handler, so validate_rules() must surface
        // them at deploy time rather than at the first call that trips them.
        // Validation is context-free, so it has one column, not two.
        let (acl, _) = build(case);
        let findings = acl.validate_rules();
        // §6.1.8 rule 3: every faulty predicate is reported, so a case may pin
        // the exact finding set rather than the presence of one.
        if let Some(paths) = case["expected_validation_finding_paths"].as_array() {
            let want: Vec<&str> = paths.iter().map(|p| p.as_str().unwrap()).collect();
            let got: Vec<&str> = findings.iter().map(|f| f.condition_path.as_str()).collect();
            assert_eq!(got, want, "[{id}] {note}");
            for finding in &findings {
                assert!(!finding.sync_resolvable, "[{id}] {note}");
                assert!(!finding.async_resolvable, "[{id}] {note}");
            }
            continue;
        }
        match case["expected_validation_finding_path"].as_str() {
            Some(path) => {
                let at: Vec<_> = findings
                    .iter()
                    .filter(|f| f.condition_path == path)
                    .collect();
                assert!(
                    !at.is_empty(),
                    "[{id}] {note}\n  no finding at '{path}': {findings:?}"
                );
                assert!(!at[0].sync_resolvable, "[{id}] {note}");
                assert!(!at[0].async_resolvable, "[{id}] {note}");
            }
            None => assert!(
                findings.is_empty(),
                "[{id}] {note}\n  unexpected findings: {findings:?}"
            ),
        }
    }

    // Reconciled against the fixture rather than trusted: an id that stopped
    // being skipped, or one that never was, is a silent loss of coverage.
    let expected_skips: Vec<&str> = cases
        .iter()
        .filter(|c| {
            c["rules"]
                .as_array()
                .is_some_and(|rules| rules.iter().any(is_unrepresentable))
        })
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        skipped, expected_skips,
        "the unrepresentable-case skip list drifted from the fixture"
    );
    assert_eq!(
        skipped, SKIPPED_UNREPRESENTABLE,
        "SKIPPED_UNREPRESENTABLE names cases the fixture no longer skips"
    );
    eprintln!(
        "{executed} run(s) over {} case(s); {} skipped as unrepresentable ({:?})",
        cases.len(),
        skipped.len(),
        skipped
    );
}

/// §6.1.6 rule 3 — the meaningless combination cannot get in by any door.
#[test]
fn deny_plus_approval_is_rejected_at_every_entry_point() {
    let mut rule = ACLRule::new(vec!["*".to_string()], vec!["x.y".to_string()], "deny");
    rule.approval = Some(ApprovalRequirement::Required);

    assert!(
        ACL::try_new(vec![rule.clone()], "deny", None).is_err(),
        "try_new must reject"
    );
    let mut acl = ACL::new(vec![], "deny", None);
    assert!(acl.try_add_rule(rule).is_err(), "try_add_rule must reject");
}
