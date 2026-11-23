//! Cross-language driver for `acl_effect_value_closure.json`.
//!
//! PROTOCOL_SPEC §6.1.5 (spec v1.30.0, #111): a rule's `effect` accepts
//! `allow` and `deny` and nothing else, and the closure holds at **every**
//! entry point that accepts a rule — file loading, direct construction and
//! runtime insertion (§6.1.6 rule 3). `default_effect` is the same two values
//! one field up and is closed on the same terms at the same doors.
//!
//! # The entry point is the substance here, not a detail
//!
//! #107 closed the rule KEY set; this closes a legal key's VALUE, and it was
//! found because the check already existed and was reachable from only one of
//! three doors. So the fixture carries `entry_points` and no per-door
//! expectation: `expected_load: ok` means accepted at every listed door,
//! `reject` means refused at every listed door, and a value legal through one
//! door and illegal through another is precisely the defect being pinned.
//!
//! **This SDK was not the conforming one it was reported to be.** `ACL::load`
//! and `ACL::try_new` rejected an out-of-enum `effect`; `ACL::try_add_rule`
//! checked only the §6.1.6 `deny` + `approval: required` combination and let
//! everything else in, so `effect: "Allow"` inserted at runtime reached
//! `finalize_rule_match` and was copied verbatim into
//! `AccessDecision::access` — a decision string no consumer knows, from a rule
//! the operator wrote to permit. Fixed alongside this driver by routing all
//! three doors through one `ACL::validate_rule`.
//!
//! # Both halves of each infallible/fallible pair are exercised
//!
//! `ACL::new` and `ACL::add_rule` are infallible in signature and signal an
//! unconstructable value the way Rust does — by panicking — while `try_new`
//! and `try_add_rule` return `Result`. A driver that asserted only on the
//! `Result` forms would prove nothing about the two entry points a caller is
//! most likely to reach for, which is the shape of hole #111 is about. Each
//! door is therefore asserted twice, the panicking half through
//! `catch_unwind` (the panic-observing convention already used by
//! `test_system_modules_hardening_conformance.rs`; `#[should_panic]` cannot be
//! used from inside a fixture loop).
//!
//! The fixture lands in the spec repo one push after this driver, so that
//! `check_driver_coverage.py --strict` has a driver to find for it. Until then
//! the test skips and names the unexercised fixture — "not verified", never
//! "passed".

use apcore::acl::{ACLRule, ACL};
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

const FIXTURE: &str = "acl_effect_value_closure.json";

/// Every door this SDK exposes, in the fixture's vocabulary. A case naming an
/// entry point outside this set is a fixture the driver does not understand,
/// and is failed rather than skipped — silently ignoring an unknown door is
/// how a door goes unguarded in the first place.
const DOORS: &[&str] = &["load", "construct", "add_rule"];

/// Build the case's rule as this SDK's in-memory type.
///
/// `effect` is copied through as the raw string the fixture wrote, which is
/// the whole point: [`ACLRule::effect`] is a `String`, so an out-of-enum value
/// is representable and must be refused by a check rather than by the type
/// system (unlike `callers` / `targets`, which §6.1.4.1 lets `Vec<String>`
/// satisfy by construction).
fn rule_of(case: &Value) -> ACLRule {
    let rule = &case["rule"];
    let strings = |key: &str| -> Vec<String> {
        rule[key]
            .as_array()
            .unwrap_or_else(|| panic!("rule.{key} is an array"))
            .iter()
            .map(|v| v.as_str().expect("pattern is a string").to_string())
            .collect()
    };
    ACLRule::new(
        strings("callers"),
        strings("targets"),
        rule["effect"]
            .as_str()
            .expect("rule.effect is a string")
            .to_string(),
    )
}

/// Emit the case as a one-rule ACL document.
///
/// Values are written as JSON scalars, which YAML is a superset of, so
/// `"Allow"` stays a quoted string and `""` stays the empty string rather than
/// becoming YAML's null — a distinction the `empty_*` cases exist to test.
fn yaml_of(case: &Value) -> String {
    let rule = &case["rule"];
    format!(
        "default_effect: {}\nrules:\n  - callers: {}\n    targets: {}\n    effect: {}\n",
        case["default_effect"], rule["callers"], rule["targets"], rule["effect"],
    )
}

/// Assert that a refusal names the offending value, per §6.1.5.
///
/// The value is matched **quoted** rather than bare: the empty string is one
/// of the cases, and a bare `contains("")` is vacuously true — the case that
/// most needs the assertion would be the one case not making it.
fn names_value(message: &str, value: &str) -> bool {
    message.contains(&format!("'{value}'"))
}

/// One door's verdict, with the message a refusal carried.
///
/// `Ok(())` means the door accepted the rule; `Err(message)` means it refused.
/// A panic is folded in as a refusal here — for `ACL::new` and `ACL::add_rule`
/// that IS the refusal, which is what §6.1.6 rule 3 means by failing loudly in
/// whatever way the language signals an unconstructable value.
type Verdict = Result<(), String>;

/// Run `f`, mapping a panic to a refusal carrying the panic payload.
fn verdict_of_panicking(f: impl FnOnce()) -> Verdict {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => Ok(()),
        Err(payload) => Err(payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "<non-string panic payload>".to_string())),
    }
}

/// Exercise one door and report what it did.
///
/// Each door is run twice where this SDK offers a fallible/infallible pair,
/// and the two verdicts are reconciled by the caller: a pair that disagrees is
/// the same defect as two doors that disagree, one level down.
fn run_door(door: &str, case: &Value, dir: &std::path::Path) -> Vec<(String, Verdict)> {
    let id = case["id"].as_str().expect("id");
    let default_effect = case["default_effect"].as_str().expect("default_effect");
    match door {
        "load" => {
            let file = dir.join(format!("{id}.yaml"));
            std::fs::write(&file, yaml_of(case)).expect("write yaml");
            let verdict = ACL::load(file.to_str().expect("utf8"))
                .map(|_| ())
                .map_err(|e| e.message);
            vec![("ACL::load".to_string(), verdict)]
        }
        "construct" => vec![
            (
                "ACL::try_new".to_string(),
                ACL::try_new(vec![rule_of(case)], default_effect, None)
                    .map(|_| ())
                    .map_err(|e| e.message),
            ),
            (
                "ACL::new".to_string(),
                verdict_of_panicking(|| {
                    let _ = ACL::new(vec![rule_of(case)], default_effect, None);
                }),
            ),
        ],
        "add_rule" => {
            // The ACL itself is built empty and valid, so the only thing under
            // test is the rule going in at runtime. A case whose
            // `default_effect` is invalid never lists this door — `add_rule`
            // takes no default_effect — and the fixture reflects that.
            let mut acl = ACL::try_new(vec![], default_effect, None).unwrap_or_else(|e| {
                panic!("[{id}] add_rule needs a valid host ACL: {}", e.message)
            });
            let try_verdict = acl.try_add_rule(rule_of(case)).map_err(|e| e.message);
            vec![
                ("ACL::try_add_rule".to_string(), try_verdict),
                (
                    "ACL::add_rule".to_string(),
                    verdict_of_panicking(|| {
                        let mut acl =
                            ACL::try_new(vec![], default_effect, None).expect("host ACL is valid");
                        acl.add_rule(rule_of(case));
                    }),
                ),
            ]
        }
        other => panic!("[{id}] fixture names an entry point this driver does not know: '{other}'"),
    }
}

#[test]
fn acl_effect_value_closure_conformance() {
    let path = find_fixtures_root().join(FIXTURE);
    if !path.is_file() {
        eprintln!("SKIP: {FIXTURE} not in the spec repo yet (spec v1.30.0, #111) — NOT VERIFIED");
        return;
    }
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
            .expect("parse fixture");

    let dir = std::env::temp_dir().join("apcore_acl_effect_value_closure");
    std::fs::create_dir_all(&dir).expect("tmp dir");

    let cases = fixture["test_cases"].as_array().expect("test_cases");
    assert!(!cases.is_empty(), "fixture carries no cases");
    let mut doors_exercised = 0usize;

    for case in cases {
        let id = case["id"].as_str().expect("id");
        let note = case["note"].as_str().unwrap_or("");
        let expected_reject = match case["expected_load"].as_str().expect("expected_load") {
            "ok" => false,
            "reject" => true,
            other => panic!("[{id}] unknown expected_load '{other}'"),
        };
        // §6.1.5 names both fields; whichever one this case makes invalid is
        // the value the refusal has to surface.
        let offending = if expected_reject {
            let effect = case["rule"]["effect"].as_str().expect("rule.effect");
            let default_effect = case["default_effect"].as_str().expect("default_effect");
            Some(if ["allow", "deny"].contains(&effect) {
                default_effect
            } else {
                effect
            })
        } else {
            None
        };

        let entry_points: Vec<&str> = case["entry_points"]
            .as_array()
            .unwrap_or_else(|| panic!("[{id}] entry_points is required — it is the fixture"))
            .iter()
            .map(|v| v.as_str().expect("entry point is a string"))
            .collect();
        assert!(
            !entry_points.is_empty(),
            "[{id}] a case with no entry point asserts nothing"
        );
        for door in &entry_points {
            assert!(
                DOORS.contains(door),
                "[{id}] unknown entry point '{door}'; this SDK knows {DOORS:?}"
            );
        }

        for door in &entry_points {
            for (api, verdict) in run_door(door, case, &dir) {
                doors_exercised += 1;
                assert_eq!(
                    verdict.is_err(),
                    expected_reject,
                    "[{id}] {api} disagrees with the fixture (expected_load = {}). \
                     The value set is closed at EVERY entry point (§6.1.5, §6.1.6 rule 3) — \
                     a value legal through one door and illegal through another IS the defect.\
                     \n  {note}\n  verdict: {verdict:?}",
                    if expected_reject { "reject" } else { "ok" },
                );
                // §6.1.5: the refusal names the offending value, so the
                // operator can find the rule in their file rather than being
                // told only that something in it is wrong.
                if let (Err(message), Some(value)) = (&verdict, offending) {
                    assert!(
                        names_value(message, value),
                        "[{id}] {api} refused without naming '{value}': {message}\n  {note}"
                    );
                }
            }
        }
    }

    // Reconciled against the fixture rather than trusted: a fixture that
    // stopped listing a door would otherwise quietly stop testing it, which is
    // the exact way the hole this fixture pins went unnoticed.
    for door in DOORS {
        assert!(
            cases.iter().any(|c| c["entry_points"]
                .as_array()
                .is_some_and(|e| e.iter().any(|v| v.as_str() == Some(door)))),
            "no case exercises the '{door}' entry point; this SDK exposes it, so it must be \
             covered (§6.1.6 rule 3)"
        );
    }
    println!(
        "acl_effect_value_closure: {} case(s), {doors_exercised} door invocation(s)",
        cases.len()
    );
}
