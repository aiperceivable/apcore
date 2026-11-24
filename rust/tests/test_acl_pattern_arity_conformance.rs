//! Cross-language driver for `acl_pattern_arity.json`.
//!
//! PROTOCOL_SPEC §6.2.1 (spec v1.31.0, apcore#112): a `callers` / `targets`
//! pattern array is FLAT and its shape is CLOSED. **Tier 1** is structural —
//! at least one operand, every element a non-empty string, `$or` at index 0
//! with at least one operand, `$not` at index 0 with exactly one, and a
//! reserved token nowhere but index 0 — and is rejected with `ACLRuleError` at
//! every entry point that accepts a rule (§6.1.6 rule 3). **Tier 2** is
//! semantic — an array well-formed under every tier-1 clause that still
//! matches no legal module ID — and is a `validate_rules()` finding only: it
//! MUST NOT be rejected and MUST NOT change any decision.
//!
//! # The two case shapes
//!
//! `kind: "closure"` offers the rule at each door in `entry_points` and
//! asserts `expected_load`. There is deliberately no per-door expectation: a
//! shape legal through one door and illegal through another IS the defect, so
//! the fixture cannot express it. A closure case that also carries
//! `expected_validation_finding_paths` is a tier-2 case — it MUST load, and
//! `validate_rules()` must then report exactly those paths.
//!
//! A case carries either `rule` (one) or `rules` (an ordered list); the list
//! form is offered at `load` and `construct` only, since `add_rule` takes one
//! rule at a time. `expected_refused_axis` and `expected_refused_rule_index`
//! pin §6.2.1 point 2 — `default_effect` first, having no index for the rule
//! ordering to reach; then the lowest-indexed bad rule; then the axis order
//! inside it — none of which `expected_load` can see, since every one of
//! those cases reads `reject` whichever fault is named. "Axis" there covers
//! EVERY per-rule check a door performs, not only `effect` / `approval` /
//! patterns: `ACL::load` has three the other doors cannot have (#107's rule
//! key set, the missing-field check, the value types), and sweeping one of
//! those across the whole file ahead of the next axis is what
//! `lowest_indexed_bad_rule_wins_over_a_loader_only_axis` catches.
//!
//! `kind: "backstop"` is the one route no door covers: assigning a field on an
//! already-constructed rule, which the matcher still reads. Two of the nine
//! carry no `mutate` and are executed here in full. The other seven need a
//! rule mutated **after it is installed in an ACL**, and this SDK has no such
//! route — see below.
//!
//! # The seven mutating backstop cases are satisfied by construction, not
//! skipped
//!
//! `ACL::rules` returns `&[ACLRule]`; there is no `rules_mut`, no public field
//! and no `Deserialize` on `ACL`, and `ACL::new_unchecked` is private. A rule
//! cannot be installed and then mutated from outside this crate at all. The
//! fixture's own `description` states that an SDK in that position satisfies
//! those cases by construction and **MUST assert the closure rather than
//! skip** — "N skipped" and "N satisfied by construction" are different
//! claims, and only the second is evidence. So for each of the seven this
//! driver asserts, from the case's own `mutate` payload:
//!
//! 1. the accessor hands back an immutable view with no mutable counterpart —
//!    the signature pinned as a coerced `fn` here, the refusal to write
//!    through it pinned as a compile-fail in
//!    `tests/compile_fail/acl_rules_immutable.rs`, and the absence of a
//!    `rules_mut` read from `src/acl.rs`;
//! 2. the already-mutated rule is **rejected at the door** by tier 1, at both
//!    halves of `add_rule` and at `try_new` — the route's dead end;
//! 3. the case's decision keys are internally consistent with §6.1.1's effect
//!    table and §6.8.1's fail-closed boolean, so a fixture edit that changed
//!    an expectation cannot pass unnoticed here;
//! 4. `src/acl.rs` carries an in-crate backstop test **of the same name**,
//!    which is where the behaviour itself is pinned. Asserted by reading the
//!    source rather than asserted in prose: a rename or deletion there fails
//!    this driver instead of quietly hollowing out the claim.
//!
//! The backstop is implemented regardless of being unreachable, because a
//! future `rules_mut` or in-place editing API would open the route and
//! §6.1.4.1's classification has to already be correct when it does.

#![allow(clippy::too_many_lines)]

use std::sync::{Arc, Mutex};

use apcore::acl::{ACLRule, ApprovalRequirement, AuditEntry, ACL};
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

const FIXTURE: &str = "acl_pattern_arity.json";

/// Every door this SDK exposes, in the fixture's vocabulary. A case naming an
/// entry point outside this set is a fixture the driver does not understand,
/// and is failed rather than skipped — silently ignoring an unknown door is
/// how a door goes unguarded in the first place.
const DOORS: &[&str] = &["load", "construct", "add_rule"];

/// The only `mutation_route` this fixture defines. A case naming another route
/// is failed for the same reason.
const MUTATION_ROUTE: &str = "installed_rule";

// ---------------------------------------------------------------------------
// Fixture -> this SDK's types
// ---------------------------------------------------------------------------

fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} is an array"))
        .iter()
        .map(|v| v.as_str().expect("pattern is a string").to_string())
        .collect()
}

/// Build the case's rule as this SDK's in-memory type.
///
/// The pattern arrays are copied through verbatim, which is the whole point:
/// `Vec<String>` constrains the element **type** and places no constraint on
/// length or on what an element says, so every shape in the closure set is
/// constructible here — which is why the fixture states that no closure case
/// is unrepresentable in any SDK.
fn rule_of(rule: &Value) -> ACLRule {
    let mut built = ACLRule::new(
        strings(rule, "callers"),
        strings(rule, "targets"),
        rule["effect"].as_str().expect("rule.effect").to_string(),
    );
    built.description = rule
        .get("description")
        .and_then(Value::as_str)
        .map(String::from);
    if let Some(approval) = rule.get("approval").and_then(Value::as_str) {
        built.approval = Some(match approval {
            "required" => ApprovalRequirement::Required,
            other => panic!("fixture uses an approval value this driver does not know: '{other}'"),
        });
    }
    built
}

/// The case's rules, in order.
///
/// A case carries either `rule` (one) or `rules` (an ordered list). The list
/// form exists for the cross-rule half of §6.2.1 point 2 — "refused for the
/// lowest-indexed bad rule" cannot be stated about a set of one — and is
/// offered at the `load` and `construct` doors only, since `add_rule` takes
/// one rule at a time.
fn rules_of(case: &Value) -> Vec<&Value> {
    match (case.get("rule"), case.get("rules")) {
        (Some(one), None) => vec![one],
        (None, Some(Value::Array(many))) => many.iter().collect(),
        _ => panic!(
            "[{}] a case carries exactly one of `rule` or `rules`",
            case["id"].as_str().unwrap_or("?")
        ),
    }
}

/// The case's single rule, for the shapes that can only have one.
fn single_rule(case: &Value) -> &Value {
    let rules = rules_of(case);
    assert_eq!(
        rules.len(),
        1,
        "[{}] this case shape carries exactly one rule",
        case["id"].as_str().unwrap_or("?")
    );
    rules[0]
}

/// Emit the case as an ACL document.
///
/// Written as JSON, which YAML 1.2 is a superset of, so `""` stays the empty
/// string rather than becoming YAML's null — the distinction the
/// `empty_pattern_string_*` cases exist to test — and a `description` needs no
/// quoting rules of its own.
fn yaml_of(case: &Value) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "default_effect": case["default_effect"],
        "rules": rules_of(case),
    }))
    .expect("serialize ACL document")
}

// ---------------------------------------------------------------------------
// Doors
// ---------------------------------------------------------------------------

/// One door's verdict: `Ok(())` accepted, `Err(message)` refused.
///
/// A panic is folded in as a refusal — for `ACL::new` and `ACL::add_rule` that
/// IS the refusal, which is what §6.1.6 rule 3 means by failing loudly in
/// whatever way the language signals an unconstructable value.
type Verdict = Result<(), String>;

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
/// Each door is run twice where this SDK offers a fallible/infallible pair.
/// A driver asserting only on the `Result` forms would prove nothing about
/// `ACL::new` and `ACL::add_rule`, which are the two a caller reaches for
/// first, and a door exempt because its signature is infallible is the shape
/// of the hole #111 was opened about.
fn run_door(door: &str, case: &Value, dir: &std::path::Path) -> Vec<(&'static str, Verdict)> {
    let id = case["id"].as_str().expect("id");
    let default_effect = case["default_effect"].as_str().expect("default_effect");
    let built = || rules_of(case).into_iter().map(rule_of).collect::<Vec<_>>();
    match door {
        "load" => {
            let file = case_yaml_path(dir, id);
            std::fs::write(&file, yaml_of(case)).expect("write yaml");
            vec![(
                "ACL::load",
                ACL::load(file.to_str().expect("utf8"))
                    .map(|_| ())
                    .map_err(|e| e.message),
            )]
        }
        "construct" => vec![
            (
                "ACL::try_new",
                ACL::try_new(built(), default_effect, None)
                    .map(|_| ())
                    .map_err(|e| e.message),
            ),
            (
                "ACL::new",
                verdict_of_panicking(|| {
                    let _ = ACL::new(built(), default_effect, None);
                }),
            ),
        ],
        // `add_rule` takes one rule at a time, so a `rules` case never lists
        // it — asserted rather than assumed, because silently inserting the
        // head of a list would test the wrong thing and still pass.
        "add_rule" => {
            let rule = single_rule(case);
            vec![
                (
                    "ACL::try_add_rule",
                    ACL::try_new(vec![], default_effect, None)
                        .expect("host ACL is valid")
                        .try_add_rule(rule_of(rule))
                        .map_err(|e| e.message),
                ),
                (
                    "ACL::add_rule",
                    verdict_of_panicking(|| {
                        let mut acl =
                            ACL::try_new(vec![], default_effect, None).expect("host ACL is valid");
                        acl.add_rule(rule_of(rule));
                    }),
                ),
            ]
        }
        other => panic!("[{id}] fixture names an entry point this driver does not know: '{other}'"),
    }
}

/// Where the `load` door writes this case's one-rule document.
///
/// Named for the case so a failure can be reproduced by hand — and named
/// consistently so [`assert_names_axis`] can strip the path back out of a
/// refusal before reading it. `callers_is_reported_before_targets.yaml`
/// contains the word `targets`, and a message-content assertion that did not
/// strip it would be reading the file name rather than the diagnosis.
fn case_yaml_path(dir: &std::path::Path, id: &str) -> std::path::PathBuf {
    dir.join(format!("{id}.yaml"))
}

/// Assert that a refusal names the axis `expected_refused_axis` gives, and
/// **only** that axis.
///
/// §6.2.1 point 2 judges `default_effect` first — it is not a rule and has no
/// index, so the rule ordering cannot reach it — then each rule on `effect` ->
/// `approval` -> `callers` / `targets`. The key mixes levels deliberately:
/// `default_effect`, `effect` and `approval` name axes, while `callers` /
/// `targets` name a FIELD inside the single pattern axis, so `callers` asserts
/// both that the pattern axis fired and that it fired on `callers`. The
/// vocabulary is open — §6.2.1 says "axis" covers every per-rule check a door
/// performs, not only the names it happens to use — so an unrecognised value
/// is a fixture this driver does not understand, and is failed rather than
/// skipped.
/// `expected_load` cannot see which of a rule's faults was named — every one
/// of these cases is a `reject` either way — so a driver that read
/// `expected_load` alone would pass an implementation running any order at
/// all. The negative half carries the weight: naming the right axis is easy
/// for an implementation that names several.
fn assert_names_axis(id: &str, api: &str, axis: &str, case: &Value, rule: &Value, message: &str) {
    let (needle, forbidden): (String, &[&str]) = match axis {
        // Not a rule and carrying no index, so the rule ordering cannot reach
        // it — judged FIRST, ahead of every rule, at every door. A refusal
        // that names a rule here has judged the rules first and found this
        // second, or not at all.
        "default_effect" => (
            format!(
                "'{}'",
                case["default_effect"].as_str().expect("default_effect")
            ),
            &["callers", "targets", "approval", "Rule ", "rule "],
        ),
        // The offending value, quoted — §6.1.5 requires the refusal to carry
        // it so an operator can find the rule in their file.
        "effect" => (
            format!("'{}'", rule["effect"].as_str().expect("rule.effect")),
            &["callers", "targets", "approval"],
        ),
        "approval" => ("approval".to_string(), &["callers", "targets"]),
        "callers" => ("callers".to_string(), &["targets"]),
        "targets" => ("targets".to_string(), &["callers"]),
        other => panic!("[{id}] fixture names a refusal axis this driver does not know: '{other}'"),
    };
    assert!(
        message.contains(&needle),
        "[{id}] {api} refused, but not for the '{axis}' axis — §6.2.1 point 2 judges \
         default_effect first and then each rule on effect -> approval -> callers -> \
         targets, and this document is bad on more than one of them: {message}"
    );
    for other in forbidden {
        assert!(
            !message.contains(other),
            "[{id}] {api} refused for '{axis}' AND named '{other}'. One rule, one refusal: \
             a door that reports several faults leaves which one an operator sees up to \
             chance: {message}"
        );
    }
}

/// The field a refusal has to name, where the case makes exactly one of the
/// two arrays faulty.
///
/// The fixture fills the innocent field with `["*"]` throughout, so the
/// offending one is the other. `both_fields_empty_are_rejected` makes both
/// faulty and returns `None` — one rejection is enough there, and §6.2.1 only
/// requires the error to name a field, not both.
fn offending_field(rule: &Value) -> Option<&'static str> {
    let neutral = |key: &str| {
        rule[key]
            .as_array()
            .is_some_and(|a| a.len() == 1 && a[0].as_str() == Some("*"))
    };
    match (neutral("callers"), neutral("targets")) {
        (true, false) => Some("targets"),
        (false, true) => Some("callers"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Findings and audit
// ---------------------------------------------------------------------------

fn expected_paths(case: &Value, key: &str) -> Vec<String> {
    case[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} is an array"))
        .iter()
        .map(|v| v.as_str().expect("path is a string").to_string())
        .collect()
}

/// `validate_rules()`'s paths, asserting each finding's shape as it goes.
///
/// §6.2.1 gives a pattern fault — tier 1 reaching the precheck, and tier 2
/// alike — §6.1.3 rule 3's keyless structural shape: a **null** key and
/// **both** resolvability flags false. A driver that compared paths alone
/// would pass an implementation reporting the right path with a condition
/// key invented for it.
fn finding_paths(acl: &ACL, id: &str) -> Vec<String> {
    acl.validate_rules()
        .into_iter()
        .map(|f| {
            assert_eq!(
                f.condition_key, None,
                "[{id}] a pattern-shape finding is §6.1.3 rule 3's KEYLESS structural fault \
                 — there is no condition key to name"
            );
            assert!(
                !f.sync_resolvable && !f.async_resolvable,
                "[{id}] both resolvability flags are false on a structural fault: {f:?}"
            );
            f.condition_path
        })
        .collect()
}

fn acl_with_sink(rule: ACLRule, default_effect: &str) -> (ACL, Arc<Mutex<Vec<AuditEntry>>>) {
    ACL::init_builtin_handlers();
    let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let mut acl = ACL::try_new(vec![rule], default_effect, None)
        .expect("a backstop case's rule is well-formed as written");
    acl.set_audit_logger(move |entry: &AuditEntry| {
        sink.lock().expect("audit sink").push(entry.clone());
    });
    (acl, captured)
}

/// The `handler_error` paths on the single audit entry a `check()` emitted.
///
/// `handler_error` is the joined `"{path}: {reason}"` form (§6.1.1 rule 2),
/// already ordered lexicographically by path, so the paths are recovered by
/// splitting rather than re-sorted — the order under test is the one the entry
/// was written in.
fn handler_error_paths(captured: &Arc<Mutex<Vec<AuditEntry>>>, id: &str) -> (bool, Vec<String>) {
    let entries = captured.lock().expect("audit sink");
    assert_eq!(
        entries.len(),
        1,
        "[{id}] §6.3.1: exactly one audit entry per check()"
    );
    let Some(joined) = entries[0].handler_error.as_ref() else {
        return (false, Vec::new());
    };
    (
        true,
        joined
            .split("; ")
            .map(|part| part.split(':').next().unwrap_or_default().to_string())
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Case runners
// ---------------------------------------------------------------------------

fn run_closure_case(case: &Value, dir: &std::path::Path) -> usize {
    let id = case["id"].as_str().expect("id");
    let note = case["note"].as_str().unwrap_or("");
    let expected_reject = match case["expected_load"].as_str().expect("expected_load") {
        "ok" => false,
        "reject" => true,
        other => panic!("[{id}] unknown expected_load '{other}'"),
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

    let rules = rules_of(case);
    let field = (rules.len() == 1)
        .then(|| offending_field(rules[0]))
        .flatten();

    // §6.2.1 point 2, both halves: the rule INDEX chooses which rule is
    // refused, and the axis order then chooses the fault inside it. Neither is
    // visible to `expected_load`, which reads `reject` whichever fault is
    // named.
    let axis = case.get("expected_refused_axis").map(|v| {
        v.as_str()
            .unwrap_or_else(|| panic!("[{id}] expected_refused_axis is a string"))
    });
    let refused_index = case.get("expected_refused_rule_index").map(|v| {
        usize::try_from(
            v.as_u64()
                .unwrap_or_else(|| panic!("[{id}] expected_refused_rule_index is an integer")),
        )
        .expect("index")
    });
    assert!(
        (axis.is_none() && refused_index.is_none()) || expected_reject,
        "[{id}] a case that loads has no refusal to name a rule or an axis for"
    );
    assert!(
        rules.len() == 1 || refused_index.is_some() || !expected_reject,
        "[{id}] a rejected multi-rule case must say which rule is refused — that is what \
         the shape exists for"
    );
    let path_noise = case_yaml_path(dir, id).display().to_string();
    let mut invocations = 0usize;
    for door in &entry_points {
        for (api, verdict) in run_door(door, case, dir) {
            invocations += 1;
            assert_eq!(
                verdict.is_err(),
                expected_reject,
                "[{id}] {api} disagrees with the fixture (expected_load = {}). \
                 The shape is closed at EVERY entry point (§6.2.1, §6.1.6 rule 3) — \
                 a shape legal through one door and illegal through another IS the defect.\
                 \n  {note}\n  verdict: {verdict:?}",
                if expected_reject { "reject" } else { "ok" },
            );
            // §6.2.1: the refusal names the offending field, so an operator
            // can find the rule in their file rather than being told only that
            // something in it is wrong.
            if let Err(message) = &verdict {
                // The `load` door's refusal quotes the file path, and this
                // fixture has a case whose ID contains the word `targets`.
                // Strip it, or an axis assertion reads the file name.
                let message = &message.replace(&path_noise, "<file>");
                // `expected_refused_axis`, where the fixture gives one, is the
                // authority — it says which of several faults must be named,
                // and for the `effect` and `approval` axes that means NOT a
                // pattern field. The heuristic below is for the cases faulty
                // on the pattern axis alone.
                // The index is asserted first: it chooses the rule, and only
                // then does the axis order choose the fault inside it. Both
                // spellings are accepted because a loader-only axis words it
                // differently — `ACL rule 1 in '<file>' carries 'priority'
                // unrecognised` beside `Rule 0 has invalid effect 'Allow'` —
                // and §6.2.1's sweep prohibition binds those axes too, so an
                // assertion that only knew one spelling would read a refusal
                // on the other as naming no rule at all.
                if let Some(index) = refused_index {
                    let names = |i: usize| {
                        message.contains(&format!("Rule {i}"))
                            || message.contains(&format!("rule {i}"))
                    };
                    assert!(
                        names(index),
                        "[{id}] {api} refused, but not for rule {index} — §6.2.1 point 2 \
                         makes the LOWEST-INDEXED bad rule the one reported, and an axis \
                         MUST NOT be swept across every rule ahead of the next axis, \
                         including a per-rule axis only this door has: {message}"
                    );
                    for other in 0..rules.len() {
                        assert!(
                            other == index || !names(other),
                            "[{id}] {api} named rule {other} as well as rule {index}. \
                             One rule set, one refusal: {message}"
                        );
                    }
                }
                if let Some(axis) = axis {
                    assert_names_axis(
                        id,
                        api,
                        axis,
                        case,
                        rules[refused_index.unwrap_or(0)],
                        message,
                    );
                } else {
                    match field {
                        Some(field) => assert!(
                            message.contains(field),
                            "[{id}] {api} refused without naming '{field}': {message}\n  {note}"
                        ),
                        None => assert!(
                            message.contains("callers") || message.contains("targets"),
                            "[{id}] {api} refused without naming either pattern field: {message}"
                        ),
                    }
                }
            }
        }
    }

    // Tier 2: a closure case carrying finding paths is well-formed, so it
    // loaded above, and `validate_rules()` must now report exactly those paths
    // — including the empty list, which is what the tier-2 controls assert.
    if case.get("expected_validation_finding_paths").is_some() {
        assert!(
            !expected_reject,
            "[{id}] a rejected rule has no ACL to validate; tier 2 presumes a well-formed array"
        );
        let acl = ACL::try_new(
            rules.iter().map(|r| rule_of(r)).collect(),
            case["default_effect"].as_str().expect("default_effect"),
            None,
        )
        .unwrap_or_else(|e| panic!("[{id}] a tier-2 array is well-formed and MUST load: {e:?}"));
        assert_eq!(
            finding_paths(&acl, id),
            expected_paths(case, "expected_validation_finding_paths"),
            "[{id}] validate_rules() must report exactly the fixture's paths\n  {note}"
        );
    }
    invocations
}

/// Execute a backstop case that carries no `mutate` — nothing is mutated, so
/// the route this SDK lacks is not needed and the case runs in full.
fn run_executable_backstop_case(case: &Value) {
    let id = case["id"].as_str().expect("id");
    let note = case["note"].as_str().unwrap_or("");
    let default_effect = case["default_effect"].as_str().expect("default_effect");
    let (acl, captured) = acl_with_sink(rule_of(single_rule(case)), default_effect);

    let caller_id = case["caller_id"].as_str();
    let target_id = case["target_id"].as_str().expect("target_id");

    let decision = acl.check_access(caller_id, target_id, None, None);
    assert_eq!(
        decision.access,
        case["expected_access"].as_str().expect("expected_access"),
        "[{id}] the STRUCTURED accessor's `access` field\n  {note}"
    );
    if let Some(expected) = case
        .get("expected_approval_required")
        .and_then(Value::as_bool)
    {
        assert_eq!(decision.approval_required, expected, "[{id}] approval");
    }
    if let Some(expected) = case.get("expected_matched_rule_index") {
        assert_eq!(
            decision.matched_rule_index,
            expected
                .as_u64()
                .map(|v| usize::try_from(v).expect("index")),
            "[{id}] matched_rule_index"
        );
    }

    let (present, paths) = handler_error_paths(&captured, id);
    assert_eq!(
        present,
        case["expected_audit_handler_error_present"]
            .as_bool()
            .expect("expected_audit_handler_error_present"),
        "[{id}] audit handler_error presence\n  {note}"
    );
    assert_eq!(
        paths,
        expected_paths(case, "expected_handler_error_paths"),
        "[{id}] handler_error paths"
    );

    // The legacy boolean is a SEPARATE surface (§6.8.1) and is asserted as
    // one. Run after the audit assertions above so its own audit entry does
    // not disturb the single-entry count.
    assert_eq!(
        acl.check(caller_id, target_id, None),
        case["expected_legacy_check"]
            .as_bool()
            .expect("expected_legacy_check"),
        "[{id}] the LEGACY boolean check(), which is not the same question as `access`"
    );

    assert_eq!(
        finding_paths(&acl, id),
        expected_paths(case, "expected_validation_finding_paths"),
        "[{id}] validate_rules() paths"
    );
}

/// The fields a case's `mutate` payload assigns, in the order given.
fn mutations(case: &Value) -> Vec<(String, Vec<String>)> {
    let one = |m: &Value| {
        (
            m["field"].as_str().expect("mutate.field").to_string(),
            m["value"]
                .as_array()
                .expect("mutate.value is an array")
                .iter()
                .map(|v| v.as_str().expect("pattern is a string").to_string())
                .collect::<Vec<_>>(),
        )
    };
    match &case["mutate"] {
        Value::Array(items) => items.iter().map(one).collect(),
        m @ Value::Object(_) => vec![one(m)],
        other => panic!("mutate is an object or an array of them, got {other}"),
    }
}

/// Assert the closure that makes a mutating backstop case unreachable here,
/// and that the behaviour it describes is pinned in-crate.
///
/// Reported as PASSING, never as skipped: the fixture's `description` says so
/// explicitly, and it is right to — "no route exists" is a stronger statement
/// than "not checked", but only if something checks that no route exists.
fn assert_backstop_closed_by_construction(case: &Value, acl_source: &str) {
    let id = case["id"].as_str().expect("id");
    let note = case["note"].as_str().unwrap_or("");
    let route = case["mutation_route"].as_str().expect("mutation_route");
    assert_eq!(
        route, MUTATION_ROUTE,
        "[{id}] fixture names a mutation route this driver does not know"
    );

    // 1. The accessor hands back an immutable view, and there is no mutable
    //    counterpart. The signature is pinned as a coerced `fn` value — this
    //    stops compiling the day `rules` returns anything a caller could write
    //    through — and `tests/compile_fail/acl_rules_immutable.rs` pins that
    //    assigning through it does not compile. The absence of a `rules_mut`
    //    is read from the source, because a second, name-resolution error in
    //    that compile-fail file would abort compilation before borrow checking
    //    and suppress the assignment error it exists for.
    let accessor: for<'a> fn(&'a ACL) -> &'a [ACLRule] = ACL::rules;
    assert!(
        !acl_source.contains("fn rules_mut"),
        "[{id}] this SDK grew a mutable rule accessor, which OPENS the fixture's \
         installed-rule mutation route. These cases can no longer be reported as satisfied \
         by construction — run them for real."
    );

    // 2. The mutated rule exists, and every door refuses it. This is the
    //    route's dead end: `add_rule` re-validates whatever it is handed,
    //    §6.2.1 point 1, so a rule mutated after construction never reaches
    //    an ACL to be matched against.
    let default_effect = case["default_effect"].as_str().expect("default_effect");
    let mut rule = rule_of(single_rule(case));
    let host = ACL::try_new(vec![rule.clone()], default_effect, None).unwrap_or_else(|e| {
        panic!("[{id}] a backstop case's rule is well-formed BEFORE the mutation: {e:?}")
    });
    assert_eq!(
        accessor(&host).len(),
        1,
        "[{id}] the well-formed rule is installed, and this is the view a mutation would \
         have to write through"
    );

    let applied = mutations(case);
    assert!(!applied.is_empty(), "[{id}] mutate assigns nothing");
    for (field, value) in &applied {
        match field.as_str() {
            "callers" => rule.callers.clone_from(value),
            "targets" => rule.targets.clone_from(value),
            other => panic!("[{id}] mutate names a field this driver does not know: '{other}'"),
        }
    }

    for (api, verdict) in [
        (
            "ACL::try_add_rule",
            ACL::try_new(vec![], default_effect, None)
                .expect("host ACL is valid")
                .try_add_rule(rule.clone())
                .map_err(|e| e.message),
        ),
        (
            "ACL::add_rule",
            verdict_of_panicking(|| {
                let mut acl =
                    ACL::try_new(vec![], default_effect, None).expect("host ACL is valid");
                acl.add_rule(rule.clone());
            }),
        ),
        (
            "ACL::try_new",
            ACL::try_new(vec![rule.clone()], default_effect, None)
                .map(|_| ())
                .map_err(|e| e.message),
        ),
    ] {
        let message = verdict.expect_err(&format!(
            "[{id}] {api} accepted a rule already carrying the mutation. §6.2.1 point 1: \
             the rule offered to runtime insertion is re-validated AT THAT MOMENT, whatever \
             its history — the rule type's construction-time check does not cover this door.\
             \n  {note}"
        ));
        assert!(
            applied
                .iter()
                .any(|(field, _)| message.contains(field.as_str())),
            "[{id}] {api} refused without naming a mutated field: {message}"
        );
    }

    // 3. The case's own decision keys are read and cross-checked, so a fixture
    //    edit cannot change an expectation this driver silently stops seeing.
    //    §6.1.1's effect table and §6.8.1's fail-closed boolean fix the
    //    relationship between the two surfaces exactly.
    let access = case["expected_access"].as_str().expect("expected_access");
    let legacy = case["expected_legacy_check"]
        .as_bool()
        .expect("expected_legacy_check");
    let approval = case
        .get("expected_approval_required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    assert_eq!(
        legacy,
        access == "allow" && !approval,
        "[{id}] §6.8.1: the legacy boolean is `access == allow` MINUS any outstanding \
         approval requirement. It diverges from `access` exactly when approval is pending, \
         and a driver reading the boolean as `access` fails that case alone."
    );
    assert!(
        case["expected_audit_handler_error_present"]
            .as_bool()
            .expect("expected_audit_handler_error_present"),
        "[{id}] a mutated pattern field is a §6.1.4.1 precheck fault, so the audit entry \
         carries a handler_error"
    );
    let mut mutated_fields: Vec<String> = applied.into_iter().map(|(field, _)| field).collect();
    mutated_fields.sort();
    assert_eq!(
        expected_paths(case, "expected_handler_error_paths"),
        mutated_fields,
        "[{id}] the reported paths are the mutated fields, ordered lexicographically \
         (§6.1.1 rule 2), with no short-circuiting between them (§6.1.4 rule 3)"
    );
    assert_eq!(
        expected_paths(case, "expected_validation_finding_paths"),
        mutated_fields,
        "[{id}] and validate_rules() reports the same set"
    );

    // 4. The behaviour itself is pinned in-crate, where the route IS reachable.
    //    Asserted by reading the source: a rename or a deletion there must fail
    //    here rather than quietly hollow out the claim above.
    assert!(
        acl_source.contains(&format!("fn {id}()")),
        "[{id}] §6.2.1's backstop behaviour is unreachable from a test crate, so it is \
         covered by an in-crate test of the same name in src/acl.rs — and there is none. \
         Add it, or this case is skipped in all but name."
    );
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

#[test]
fn acl_pattern_arity_conformance() {
    let path = find_fixtures_root().join(FIXTURE);
    assert!(
        path.is_file(),
        "{FIXTURE} is missing from the spec repo (spec v1.31.0, apcore#112) at {}",
        path.display()
    );
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
            .expect("parse fixture");

    // The in-crate backstop tests, read once. `CARGO_MANIFEST_DIR` is this
    // crate's root at compile time, so this finds the source whatever the
    // working directory of the test run.
    let acl_source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/acl.rs"),
    )
    .expect("read src/acl.rs");

    let dir = std::env::temp_dir().join("apcore_acl_pattern_arity_conformance");
    std::fs::create_dir_all(&dir).expect("tmp dir");

    let cases = fixture["test_cases"].as_array().expect("test_cases");
    assert!(!cases.is_empty(), "fixture carries no cases");

    let (mut closure, mut executed_backstop, mut by_construction, mut invocations) = (0, 0, 0, 0);
    for case in cases {
        let id = case["id"].as_str().expect("id");
        match case["kind"].as_str().expect("kind") {
            "closure" => {
                closure += 1;
                invocations += run_closure_case(case, &dir);
            }
            "backstop" => {
                if case.get("mutate").is_some() {
                    by_construction += 1;
                    assert_backstop_closed_by_construction(case, &acl_source);
                } else {
                    executed_backstop += 1;
                    run_executable_backstop_case(case);
                }
            }
            other => panic!("[{id}] fixture uses a case kind this driver does not know: '{other}'"),
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

    // Every case is accounted for, and none of them as a skip.
    assert_eq!(
        closure + executed_backstop + by_construction,
        cases.len(),
        "every case is run or asserted closed — this driver has no skip branch"
    );
    println!(
        "acl_pattern_arity: {} case(s) — {closure} closure ({invocations} door invocations), \
         {executed_backstop} backstop executed, {by_construction} backstop satisfied by \
         construction (installed-rule mutation is unreachable in this SDK; asserted, not \
         skipped). 0 skipped.",
        cases.len()
    );
}
