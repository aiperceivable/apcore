//! PROTOCOL_SPEC §6.2.1 — a `callers` / `targets` pattern array is FLAT and its
//! shape is CLOSED (spec v1.31.0, apcore#112).
//!
//! # What the rule is
//!
//! The operators do not nest, there is no precedence, an operand is always a
//! plain pattern string, and there is exactly one operator position — index 0.
//! **Tier 1** is structural and is rejected with `ACLRuleError` at every entry
//! point that accepts a rule: the array is non-empty, every element is a
//! non-empty string, `$or` at index 0 takes at least one operand, `$not` at
//! index 0 takes exactly one, and a reserved token appears nowhere but index 0.
//! **Tier 2** is semantic — an array that is well-formed under every tier-1
//! clause and still matches no legal module ID — and is a `validate_rules()`
//! finding only: it MUST NOT be rejected and MUST NOT change any decision.
//!
//! # Why rejection rather than a non-match
//!
//! Through spec v1.30.0 all three SDKs returned `false` from the matcher for
//! `[]`, `["$or"]` and `["$not"]`, so the rule was inert: with one rule in the
//! ACL the decision tracked `default_effect` exactly and `validate_rules()`
//! reported nothing. On an `allow` rule that is merely useless. On a `deny`
//! rule under `default_effect: allow` it is a **fail-open** — the call the
//! operator wrote the rule to block is permitted, by a rule that loaded without
//! error. `schemas/acl-config.schema.json` has declared `minItems: 1` and
//! `minLength: 1` on both fields since the file existed and no door enforced
//! either, which is the shape #107 and #111 already had.
//!
//! # Scope of this file
//!
//! These are **SDK-local** tests mirroring `acl_pattern_arity.json` by fixture
//! ID. The fixture itself is driven from
//! `tests/test_acl_pattern_arity_conformance.rs`; these are kept because they
//! run without a spec-repo checkout and because they say in Rust what the
//! driver says in JSON, which is what a reader of this crate looks for first.
//!
//! Two things live here that the fixture cannot express, both of them §6.2.1's
//! "two points of order": that the three checks run in the order `effect` ->
//! `approval` -> `callers` / `targets`, and that `add_rule` re-validates the
//! rule it is handed whatever its history.
//!
//! The fixture's nine `kind: "backstop"` cases that mutate a field on an
//! already-constructed rule are **not** here: [`ACL::rules`] hands back
//! `&[ACLRule]` and this SDK exposes no `rules_mut`, no public field and no
//! `Deserialize` on `ACL`, so that route cannot be reached from outside the
//! crate at all. They live in `src/acl.rs`'s `pattern_arity_backstop_tests`,
//! which can reach it — see the module note there for why the backstop is
//! implemented regardless.

#![allow(clippy::too_many_lines)]

use std::sync::{Arc, Mutex};

use apcore::acl::{ACLRule, AuditEntry, ACL};

/// Every door this SDK exposes that accepts a rule (§6.1.6 rule 3).
///
/// The fallible and infallible halves of each pair are both exercised. A test
/// asserting only on the `Result` forms would prove nothing about `ACL::new`
/// and `ACL::add_rule`, which are the two a caller reaches for first — and a
/// door that is exempt because its signature is infallible is exactly the hole
/// #111 was opened about.
const DOOR_COUNT: usize = 5;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

fn rule_of(callers: &[&str], targets: &[&str], effect: &str) -> ACLRule {
    ACLRule::new(strings(callers), strings(targets), effect)
}

/// Emit the case as a one-rule ACL document.
///
/// Pattern arrays are written as JSON, which YAML is a superset of, so `""`
/// stays the empty string rather than becoming YAML's null — the distinction
/// the `empty_pattern_string_*` cases exist to test.
fn yaml_of(callers: &[&str], targets: &[&str], effect: &str, default_effect: &str) -> String {
    let json = |items: &[&str]| serde_json::to_string(items).expect("serialize patterns");
    format!(
        "default_effect: {default_effect}\nrules:\n  - callers: {}\n    targets: {}\n    effect: \"{effect}\"\n",
        json(callers),
        json(targets),
    )
}

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

/// Run every entry point against one rule and report what each did.
fn run_all_doors(
    id: &str,
    callers: &[&str],
    targets: &[&str],
    effect: &str,
    default_effect: &str,
) -> Vec<(&'static str, Verdict)> {
    let dir = std::env::temp_dir().join("apcore_acl_pattern_arity");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let file = dir.join(format!("{id}.yaml"));
    std::fs::write(&file, yaml_of(callers, targets, effect, default_effect)).expect("write yaml");

    vec![
        (
            "ACL::load",
            ACL::load(file.to_str().expect("utf8"))
                .map(|_| ())
                .map_err(|e| e.message),
        ),
        (
            "ACL::try_new",
            ACL::try_new(
                vec![rule_of(callers, targets, effect)],
                default_effect,
                None,
            )
            .map(|_| ())
            .map_err(|e| e.message),
        ),
        (
            "ACL::new",
            verdict_of_panicking(|| {
                let _ = ACL::new(
                    vec![rule_of(callers, targets, effect)],
                    default_effect,
                    None,
                );
            }),
        ),
        (
            "ACL::try_add_rule",
            ACL::try_new(vec![], default_effect, None)
                .expect("host ACL is valid")
                .try_add_rule(rule_of(callers, targets, effect))
                .map_err(|e| e.message),
        ),
        (
            "ACL::add_rule",
            verdict_of_panicking(|| {
                let mut acl =
                    ACL::try_new(vec![], default_effect, None).expect("host ACL is valid");
                acl.add_rule(rule_of(callers, targets, effect));
            }),
        ),
    ]
}

/// Assert that every door refuses the rule, and that each refusal names the
/// offending field so an operator can find it in their file (§6.2.1).
///
/// `field` is `callers` or `targets`; a message naming `targets[1]` satisfies
/// it, because the index narrows rather than replaces the field name.
fn assert_rejected_at_every_door(
    id: &str,
    callers: &[&str],
    targets: &[&str],
    effect: &str,
    default_effect: &str,
    field: &str,
) {
    let verdicts = run_all_doors(id, callers, targets, effect, default_effect);
    assert_eq!(verdicts.len(), DOOR_COUNT);
    for (api, verdict) in verdicts {
        let message = verdict.expect_err(&format!(
            "[{id}] {api} ACCEPTED a rule whose pattern shape is outside §6.2.1's table. \
             The shape is closed at EVERY entry point — a shape legal through one door and \
             illegal through another IS the defect (§6.1.6 rule 3)"
        ));
        assert!(
            message.contains(field),
            "[{id}] {api} refused without naming '{field}': {message}"
        );
    }
}

/// Assert that every door accepts the rule.
///
/// The controls matter as much as the rejections: a precheck that over-rejects
/// refuses configurations the specification permits, and `$orders.*` is the one
/// that separates reserved-token detection by **equality** from detection by a
/// `$` prefix.
fn assert_accepted_at_every_door(
    id: &str,
    callers: &[&str],
    targets: &[&str],
    effect: &str,
    default_effect: &str,
) {
    let verdicts = run_all_doors(id, callers, targets, effect, default_effect);
    assert_eq!(verdicts.len(), DOOR_COUNT);
    for (api, verdict) in verdicts {
        assert!(
            verdict.is_ok(),
            "[{id}] {api} REFUSED a rule §6.2.1 permits: {verdict:?}"
        );
    }
}

/// The `validate_rules()` finding paths for a one-rule ACL.
fn finding_paths(
    callers: &[&str],
    targets: &[&str],
    effect: &str,
    default_effect: &str,
) -> Vec<String> {
    let acl = ACL::try_new(
        vec![rule_of(callers, targets, effect)],
        default_effect,
        None,
    )
    .expect("a tier-2 array is well-formed and MUST load");
    acl.validate_rules()
        .into_iter()
        .map(|f| f.condition_path)
        .collect()
}

// ---------------------------------------------------------------------------
// Controls — the forms §6.2.1 permits must still load, at every door
// ---------------------------------------------------------------------------

#[test]
fn flat_single_pattern_loads() {
    assert_accepted_at_every_door(
        "flat_single_pattern_loads",
        &["api.*"],
        &["executor.*"],
        "allow",
        "deny",
    );
}

#[test]
fn flat_multi_pattern_loads() {
    assert_accepted_at_every_door(
        "flat_multi_pattern_loads",
        &["api.*", "worker.*"],
        &["executor.*"],
        "allow",
        "deny",
    );
}

#[test]
fn or_with_two_operands_loads() {
    assert_accepted_at_every_door(
        "or_with_two_operands_loads",
        &["$or", "admin", "moderator"],
        &["*"],
        "allow",
        "deny",
    );
}

#[test]
fn or_with_one_operand_loads() {
    // The boundary: `$or` requires AT LEAST one operand, not at least two. An
    // implementation that rejects this has read the rule as `minItems: 2` on
    // the array rather than as an arity rule on the operator.
    assert_accepted_at_every_door(
        "or_with_one_operand_loads",
        &["$or", "admin"],
        &["*"],
        "allow",
        "deny",
    );
}

#[test]
fn not_with_one_operand_loads() {
    assert_accepted_at_every_door(
        "not_with_one_operand_loads",
        &["$not", "banned.*"],
        &["*"],
        "allow",
        "deny",
    );
}

#[test]
fn token_lookalike_pattern_loads() {
    // Reserved-token detection is EQUALITY, never a `$` prefix or a substring.
    // `$orders.*` is an ordinary pattern that merely begins with the same
    // character. An implementation testing `starts_with('$')` passes every
    // rejection case in this file and fails here.
    assert_accepted_at_every_door(
        "token_lookalike_pattern_loads",
        &["api.*", "$orders.*"],
        &["*"],
        "allow",
        "deny",
    );
}

// ---------------------------------------------------------------------------
// Tier 1 — rejected at every door
// ---------------------------------------------------------------------------

#[test]
fn empty_callers_is_rejected() {
    // `ACL::load` rejects an OMITTED `callers` / `targets` and used to permit
    // an empty one, so a plain YAML file reaches this — not only direct
    // construction.
    assert_rejected_at_every_door(
        "empty_callers_is_rejected",
        &[],
        &["*"],
        "allow",
        "deny",
        "callers",
    );
}

#[test]
fn empty_targets_is_rejected() {
    assert_rejected_at_every_door(
        "empty_targets_is_rejected",
        &["*"],
        &[],
        "allow",
        "deny",
        "targets",
    );
}

#[test]
fn both_fields_empty_are_rejected() {
    // One rejection is enough — the error names at least one field — but the
    // rule MUST NOT be accepted.
    assert_rejected_at_every_door(
        "both_fields_empty_are_rejected",
        &[],
        &[],
        "allow",
        "deny",
        "callers",
    );
}

#[test]
fn empty_targets_on_deny_rule_under_default_allow_is_rejected() {
    // THE DRIVING CASE of #112. Written as YAML this loaded clean,
    // `validate_rules()` returned zero findings, and `check(_, "cli.rm", _)`
    // returned ALLOW: the operator has a rule that says "block everything
    // dangerous" and a deployment that blocks nothing.
    assert_rejected_at_every_door(
        "empty_targets_on_deny_rule_under_default_allow_is_rejected",
        &["*"],
        &[],
        "deny",
        "allow",
        "targets",
    );
}

#[test]
fn or_with_no_operands_is_rejected() {
    // A one-element array that passes `minItems: 1` and is still an OR over
    // the empty set — which is why the arity is stated per operator.
    assert_rejected_at_every_door(
        "or_with_no_operands_is_rejected",
        &["*"],
        &["$or"],
        "deny",
        "allow",
        "targets",
    );
}

#[test]
fn not_with_no_operands_is_rejected() {
    // §6.2.1 through v1.30.0 required this form to "evaluate to false
    // (fail-closed)". The parenthetical was wrong: a `deny` rule that never
    // matches refuses nothing, and under `default_effect: allow` the blocked
    // call is permitted.
    assert_rejected_at_every_door(
        "not_with_no_operands_is_rejected",
        &["*"],
        &["$not"],
        "deny",
        "allow",
        "targets",
    );
}

#[test]
fn not_with_two_operands_is_rejected() {
    // THE SECOND DEFECT, and the one that is not inert. §6.2.1 called this
    // implementation-defined — consult the first operand, ignore the rest —
    // and all three SDKs did exactly that, so the form was uniform across
    // implementations and uniformly wider than written: the operator excluded
    // two targets from an `allow` rule and the second one was granted.
    assert_rejected_at_every_door(
        "not_with_two_operands_is_rejected",
        &["*"],
        &["$not", "secrets.a", "secrets.b"],
        "allow",
        "deny",
        "targets",
    );
}

#[test]
fn not_with_two_operands_on_deny_rule_is_rejected() {
    // The same arity fault where the old reading is over-broad rather than
    // escalating. It fails for the same reason rather than surviving because
    // this effect happens to land on the safe side.
    assert_rejected_at_every_door(
        "not_with_two_operands_on_deny_rule_is_rejected",
        &["*"],
        &["$not", "secrets.a", "secrets.b"],
        "deny",
        "deny",
        "targets",
    );
}

#[test]
fn empty_pattern_string_is_rejected() {
    // The empty pattern matches only the empty module ID, which is not a legal
    // module ID, so the array is structurally never-matching rather than
    // semantically so and belongs at the door rather than in the validator.
    assert_rejected_at_every_door(
        "empty_pattern_string_is_rejected",
        &["*"],
        &[""],
        "deny",
        "allow",
        "targets",
    );
}

#[test]
fn empty_pattern_string_under_or_is_rejected() {
    // The same fault as an operand rather than as the whole array. An
    // implementation that checks only index 0 passes the case above.
    assert_rejected_at_every_door(
        "empty_pattern_string_under_or_is_rejected",
        &["*"],
        &["$or", ""],
        "deny",
        "allow",
        "targets",
    );
}

#[test]
fn reserved_token_after_operator_is_rejected() {
    // THE NESTING CASE. A pattern array is FLAT, but nothing said so before
    // v1.31.0 while `$or` / `$not` DO nest arbitrarily in `conditions`. An
    // operator who learned the condition grammar writes `["$or", "$not", "a"]`
    // expecting or-of-not and got an OR of two literals — matching `a`, and
    // also matching a module literally named `$not`, which §6.2.1's own
    // reserved-token clause says MUST NOT happen.
    assert_rejected_at_every_door(
        "reserved_token_after_operator_is_rejected",
        &["*"],
        &["$or", "$not", "a"],
        "allow",
        "deny",
        "targets",
    );
}

#[test]
fn reserved_token_in_flat_list_is_rejected() {
    // `["api.*", "$not", "cli.*"]` is not "api.* but not cli.*" — no such form
    // exists. Rejecting the token outside index 0 makes §6.2.1's reserved-token
    // MUST NOT hold by construction instead of by a check nothing performs.
    assert_rejected_at_every_door(
        "reserved_token_in_flat_list_is_rejected",
        &["*"],
        &["api.*", "$not", "cli.*"],
        "allow",
        "deny",
        "targets",
    );
}

#[test]
fn reserved_token_at_index_one_in_callers_is_rejected() {
    assert_rejected_at_every_door(
        "reserved_token_at_index_one_in_callers_is_rejected",
        &["api.*", "$or"],
        &["*"],
        "allow",
        "deny",
        "callers",
    );
}

// --- field parity: §6.2.1 constrains `callers` and `targets` identically -----
//
// An implementation that validates only `targets` passes almost every
// rejection above. Every structural shape rejected on `targets` is mirrored
// onto `callers` here so that asymmetry cannot pass.

#[test]
fn or_with_no_operands_in_callers_is_rejected() {
    assert_rejected_at_every_door(
        "or_with_no_operands_in_callers_is_rejected",
        &["$or"],
        &["*"],
        "deny",
        "allow",
        "callers",
    );
}

#[test]
fn not_with_no_operands_in_callers_is_rejected() {
    assert_rejected_at_every_door(
        "not_with_no_operands_in_callers_is_rejected",
        &["$not"],
        &["*"],
        "deny",
        "allow",
        "callers",
    );
}

#[test]
fn not_with_two_operands_in_callers_is_rejected() {
    assert_rejected_at_every_door(
        "not_with_two_operands_in_callers_is_rejected",
        &["$not", "admin.*", "ops.*"],
        &["*"],
        "allow",
        "deny",
        "callers",
    );
}

#[test]
fn empty_pattern_string_in_callers_is_rejected() {
    assert_rejected_at_every_door(
        "empty_pattern_string_in_callers_is_rejected",
        &[""],
        &["*"],
        "deny",
        "allow",
        "callers",
    );
}

#[test]
fn empty_pattern_string_under_or_in_callers_is_rejected() {
    // Not a duplicate of the case above: this one fails only if the
    // implementation scans the OPERANDS of a `$or` on the `callers` side.
    assert_rejected_at_every_door(
        "empty_pattern_string_under_or_in_callers_is_rejected",
        &["$or", ""],
        &["*"],
        "deny",
        "allow",
        "callers",
    );
}

#[test]
fn reserved_token_after_operator_in_callers_is_rejected() {
    // Distinct from `reserved_token_at_index_one_in_callers_is_rejected`, where
    // index 0 is an ordinary pattern: here index 0 is an OPERATOR, so an
    // implementation that stops checking positions once it has consumed a
    // leading `$or` passes that case and fails this one.
    assert_rejected_at_every_door(
        "reserved_token_after_operator_in_callers_is_rejected",
        &["$or", "$not", "admin"],
        &["*"],
        "allow",
        "deny",
        "callers",
    );
}

// ---------------------------------------------------------------------------
// Tier 2 — reported by validate_rules(), never rejected, never decisive
// ---------------------------------------------------------------------------

#[test]
fn not_of_wildcard_loads_and_is_reported() {
    // The case that proves the two tiers are distinct. `["$not", "*"]` has
    // perfectly legal arity — exactly one operand — and matches NOTHING,
    // producing the identical fail-open the arity shapes produce. It is
    // well-formed, so it MUST load; it protects nothing, so it MUST be
    // reported. Collapsing the tiers here turns a well-formed rule into one
    // that denies every call.
    assert_accepted_at_every_door(
        "not_of_wildcard_loads_and_is_reported",
        &["*"],
        &["$not", "*"],
        "deny",
        "allow",
    );
    assert_eq!(
        finding_paths(&["*"], &["$not", "*"], "deny", "allow"),
        vec!["targets".to_string()]
    );
}

#[test]
fn not_of_double_wildcard_loads_and_is_reported() {
    // `**` is universal too. The MUST-detect minimum is "any pattern
    // consisting only of wildcards", not the single literal `*` — an
    // implementation that string-compares against "*" passes the case above
    // and fails this one.
    assert_accepted_at_every_door(
        "not_of_double_wildcard_loads_and_is_reported",
        &["*"],
        &["$not", "**"],
        "deny",
        "allow",
    );
    assert_eq!(
        finding_paths(&["*"], &["$not", "**"], "deny", "allow"),
        vec!["targets".to_string()]
    );
}

#[test]
fn external_sentinel_as_target_loads_and_is_reported() {
    // `@external` is the caller-side sentinel §6.5 substitutes for a null
    // `caller_id`; no module ID is `@external`, so as a TARGET pattern it
    // matches nothing.
    assert_accepted_at_every_door(
        "external_sentinel_as_target_loads_and_is_reported",
        &["*"],
        &["@external"],
        "deny",
        "allow",
    );
    assert_eq!(
        finding_paths(&["*"], &["@external"], "deny", "allow"),
        vec!["targets".to_string()]
    );
}

#[test]
fn external_sentinel_as_caller_loads_clean() {
    // The control for the case above. `@external` in `callers` is the
    // documented way to write a rule about top-level entry points. A finding
    // that fires on both fields has read the rule as being about the token
    // rather than about the field.
    assert_accepted_at_every_door(
        "external_sentinel_as_caller_loads_clean",
        &["@external"],
        &["*"],
        "deny",
        "allow",
    );
    assert!(finding_paths(&["@external"], &["*"], "deny", "allow").is_empty());
}

#[test]
fn external_sentinel_beside_a_real_pattern_loads_clean() {
    // Tier 2 judges the array AS A WHOLE — the criterion is "matches no legal
    // module ID for any input", not "contains an unmatchable operand". The
    // array is OR-ed, so `api.*` still matches and the rule still protects
    // something. An implementation that reports any occurrence of `@external`
    // in `targets` passes the case above and fails here, and would be flagging
    // a rule that is doing its job.
    assert_accepted_at_every_door(
        "external_sentinel_beside_a_real_pattern_loads_clean",
        &["*"],
        &["api.*", "@external"],
        "deny",
        "allow",
    );
    assert!(finding_paths(&["*"], &["api.*", "@external"], "deny", "allow").is_empty());
    // The same array under an explicit `$or`, which is observably the same OR.
    assert!(finding_paths(&["*"], &["$or", "@external", "api.*"], "deny", "allow").is_empty());
}

#[test]
fn not_of_narrow_pattern_loads_clean() {
    // The control against tier 2 over-firing. `["$not", "cli.*"]` matches every
    // module outside `cli.`, which is the form's whole purpose. An
    // implementation that reports every `$not` has confused "negation" with
    // "matches nothing".
    assert_accepted_at_every_door(
        "not_of_narrow_pattern_loads_clean",
        &["*"],
        &["$not", "cli.*"],
        "deny",
        "allow",
    );
    assert!(finding_paths(&["*"], &["$not", "cli.*"], "deny", "allow").is_empty());
}

// ---------------------------------------------------------------------------
// Tier 2 changes no decision, and neither tier fires on a well-formed rule
// ---------------------------------------------------------------------------

fn acl_with_sink(rule: ACLRule, default_effect: &str) -> (ACL, Arc<Mutex<Vec<AuditEntry>>>) {
    ACL::init_builtin_handlers();
    let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let mut acl = ACL::try_new(vec![rule], default_effect, None).expect("rule is well-formed");
    acl.set_audit_logger(move |entry: &AuditEntry| {
        sink.lock().expect("audit sink").push(entry.clone());
    });
    (acl, captured)
}

fn only_handler_error(captured: &Arc<Mutex<Vec<AuditEntry>>>) -> Option<String> {
    let entries = captured.lock().expect("audit sink");
    assert_eq!(entries.len(), 1, "exactly one audit entry per check()");
    entries[0].handler_error.clone()
}

#[test]
fn not_of_wildcard_does_not_change_the_decision() {
    // The other half of tier 2: reported, and inert BY DESIGN. The rule matches
    // nothing, so `cli.rm` falls through to `default_effect: allow` exactly as
    // it did before v1.31.0. A denial here means tier 2 was implemented as a
    // rejection or as an UNEVALUABLE fault, which would make a well-formed rule
    // deny every call.
    let (acl, captured) = acl_with_sink(rule_of(&["*"], &["$not", "*"], "deny"), "allow");

    let decision = acl.check_access(Some("api.gateway"), "cli.rm", None, None);
    assert_eq!(
        decision.access, "allow",
        "a tier-2 finding MUST NOT change any access decision (§6.2.1)"
    );
    assert!(
        !decision.approval_required,
        "and it raises no question either — the rule simply does not match"
    );
    assert_eq!(
        only_handler_error(&captured),
        None,
        "tier 2 is a validator finding — it MUST NOT reach the per-call handler-error scope"
    );
    assert_eq!(
        acl.validate_rules()
            .into_iter()
            .map(|f| f.condition_path)
            .collect::<Vec<_>>(),
        vec!["targets".to_string()],
        "and it MUST still be reported"
    );
}

#[test]
fn well_formed_rule_raises_no_finding() {
    // The control without which an implementation that flags every rule passes
    // everything else in this file.
    let (acl, captured) = acl_with_sink(rule_of(&["*"], &["cli.*"], "deny"), "allow");

    assert!(!acl.check(Some("api.gateway"), "cli.rm", None));
    assert_eq!(only_handler_error(&captured), None);
    assert!(acl.validate_rules().is_empty());
}

// ---------------------------------------------------------------------------
// §6.2.1's two points of order (spec v1.31.0, #112)
// ---------------------------------------------------------------------------

/// Point 2 — validation order within a rule is `effect` -> `approval` ->
/// `callers` / `targets`.
///
/// A rule bad on more than one axis is refused for the FIRST of these it
/// fails, so the same rule produces the same error in every implementation.
/// This SDK checked the patterns first, so the fixture case below was refused
/// here for its patterns and in apcore-python for its `effect` — both
/// conformant while nothing said otherwise, which is what the ordering ends.
/// §6.2.1 states the order for the first time; §6.1.6 rule 2 only *implies*
/// `effect` before `approval` and is not the citation for it.
///
/// Asserted at every door, not only at `try_new`: an ordering that held in one
/// entry point and not another would be the same class of defect #111 was
/// opened about, one level down.
#[test]
fn validation_order_is_effect_then_approval_then_patterns() {
    // Bad on two axes: an out-of-enum `effect` AND two empty pattern arrays.
    // §6.2.1 point 2 says the `effect` wins.
    for (api, verdict) in run_all_doors(
        "validation_order_effect_before_patterns",
        &[],
        &[],
        "Allow",
        "deny",
    ) {
        let message = verdict.expect_err(&format!(
            "[validation_order] {api} accepted a rule bad on two axes"
        ));
        assert!(
            message.contains("'Allow'"),
            "[validation_order] {api} must refuse for the EFFECT first (§6.2.1 point 2), \
             naming the offending value: {message}"
        );
        assert!(
            !message.contains("callers") && !message.contains("targets"),
            "[validation_order] {api} refused for the patterns; the `effect` is checked \
             first and only one refusal is raised: {message}"
        );
    }

    // Bad on the other two axes: `approval: required` on a `deny` rule AND an
    // empty `targets`. The `approval` check is second, so it wins over the
    // patterns. Built by hand because `run_all_doors` takes no `approval`, and
    // `ACL::load` is exercised through the same YAML the loader would see.
    let mut rule = rule_of(&["*"], &[], "deny");
    rule.approval = Some(apcore::acl::ApprovalRequirement::Required);
    let message = ACL::try_new(vec![rule.clone()], "allow", None)
        .expect_err("a rule bad on two axes is still refused")
        .message;
    assert!(
        message.contains("approval"),
        "the `approval` check precedes the pattern check (§6.2.1 point 2): {message}"
    );
    assert!(
        !message.contains("targets"),
        "and only one refusal is raised: {message}"
    );

    let dir = std::env::temp_dir().join("apcore_acl_pattern_arity");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let file = dir.join("validation_order_approval_before_patterns.yaml");
    std::fs::write(
        &file,
        "default_effect: allow\nrules:\n  - callers: [\"*\"]\n    targets: []\n    \
         effect: \"deny\"\n    approval: required\n",
    )
    .expect("write yaml");
    let loaded = ACL::load(file.to_str().expect("utf8"))
        .expect_err("the loader shares `validate_rule`, so it shares the order")
        .message;
    assert!(
        loaded.contains("approval") && !loaded.contains("targets"),
        "ACL::load must apply the same order as the constructors: {loaded}"
    );

    // The control: with the `effect` and `approval` axes clean, the pattern
    // fault is what is left to report. Without this, an implementation that
    // never reports a pattern fault at all passes both assertions above.
    let only_patterns = ACL::try_new(vec![rule_of(&[], &[], "allow")], "deny", None)
        .expect_err("empty pattern arrays are still refused")
        .message;
    assert!(
        only_patterns.contains("callers"),
        "an ordering is not an exemption — the pattern check still runs: {only_patterns}"
    );
}

/// Point 1 — `add_rule` re-validates the rule it is handed, whatever its
/// history.
///
/// Including a rule that was well-formed when constructed and has since had
/// `callers` or `targets` assigned: [`ACLRule`]'s fields are public, so
/// `ACLRule::new` cannot be the only check. This SDK routes all three doors
/// through one `ACL::validate_rule` and so satisfies it by construction —
/// which is a reason to pin it, not to assume it, because the funnel is one
/// refactor away from being bypassed and apcore-python is being changed to
/// match this behaviour.
///
/// This is also the assertion that closes the fixture's `backstop` route from
/// outside the crate: the mutated rule exists, and the door refuses it.
#[test]
fn add_rule_revalidates_a_rule_mutated_after_construction() {
    // Every mutation the fixture's backstop cases apply, plus the field each
    // one lands on.
    let mutations: &[(&str, &[&str])] = &[
        ("targets", &[]),
        ("targets", &["$or"]),
        ("callers", &["$not"]),
        ("targets", &["$not", "secrets.a", "secrets.b"]),
        ("callers", &[""]),
        ("targets", &["api.*", "$not", "cli.*"]),
    ];

    for (field, value) in mutations {
        // Well-formed at construction — `ACLRule::new` has nothing to object
        // to and `try_add_rule` would accept it as it stands.
        let mut rule = rule_of(&["*"], &["*"], "deny");
        assert!(
            ACL::try_new(vec![], "allow", None)
                .expect("host ACL is valid")
                .try_add_rule(rule.clone())
                .is_ok(),
            "the unmutated rule is accepted, so the refusal below is the mutation's"
        );

        // ...and mutated afterwards, which no constructor can intercept.
        match *field {
            "callers" => rule.callers = strings(value),
            "targets" => rule.targets = strings(value),
            other => panic!("unknown field {other}"),
        }

        let message = ACL::try_new(vec![], "allow", None)
            .expect("host ACL is valid")
            .try_add_rule(rule.clone())
            .expect_err(&format!(
                "try_add_rule MUST re-validate the rule it is handed, whatever its history \
                 (§6.2.1 point 1) — {field} = {value:?}"
            ))
            .message;
        assert!(
            message.contains(field),
            "the refusal names the offending field: {message}"
        );

        // The infallible half signals the same refusal the way Rust does.
        let panicked = verdict_of_panicking(|| {
            let mut acl = ACL::try_new(vec![], "allow", None).expect("host ACL is valid");
            acl.add_rule(rule.clone());
        });
        assert!(
            panicked.is_err(),
            "ACL::add_rule inherits the refusal as a panic — an infallible signature is \
             not an exemption from the door ({field} = {value:?})"
        );
    }
}

/// The rule accessor hands back an **immutable view**, and there is no mutable
/// counterpart — which is what makes the fixture's `mutation_route:
/// "installed_rule"` unreachable from outside this crate.
///
/// The signature is pinned as a value rather than described in prose: this
/// coerces `ACL::rules` to `fn(&ACL) -> &[ACLRule]` and stops compiling the
/// day it returns anything a caller could write through. The absence of a
/// mutable counterpart is pinned by `tests/compile_fail/acl_rules_immutable.rs`
/// — assigning through `rules()` does not compile — and by the conformance
/// driver, which reads `src/acl.rs` for a `fn rules_mut`.
#[test]
fn rules_accessor_hands_back_an_immutable_view() {
    let accessor: for<'a> fn(&'a ACL) -> &'a [ACLRule] = ACL::rules;
    let acl = ACL::try_new(vec![rule_of(&["*"], &["*"], "deny")], "allow", None)
        .expect("rule is well-formed");
    assert_eq!(accessor(&acl).len(), 1);
}

/// Point 2's other half — **rule index dominates every axis**.
///
/// A rule set with more than one bad rule is refused for the LOWEST-INDEXED
/// bad rule, and an implementation MUST NOT sweep one axis across every rule
/// before looking at the next. apcore-typescript's `ACL.load` validated rule
/// by rule while its constructor swept check by check across the list, so the
/// same file produced different errors through different doors — both
/// conformant under the pre-v1.31.0 wording.
///
/// `ACL::load` is the door with the most to get wrong here, because it carries
/// a fourth axis the others do not: #107's rule-KEY closure, which needs the
/// raw document and so cannot live in `validate_rule`. It used to run that
/// axis across the whole list before `try_new` looked at any rule's `effect`,
/// which is the forbidden sweep — a file whose rule 0 carried `effect:
/// "Allow"` and whose rule 1 carried an unknown key was refused for rule 1.
#[test]
fn the_lowest_indexed_bad_rule_is_the_one_reported() {
    let dir = std::env::temp_dir().join("apcore_acl_pattern_arity");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let load = |name: &str, doc: &str| -> String {
        let file = dir.join(format!("{name}.yaml"));
        std::fs::write(&file, doc).expect("write yaml");
        ACL::load(file.to_str().expect("utf8"))
            .expect_err("the document carries two bad rules")
            .message
    };

    // Rule 0 is bad on the PATTERN axis, rule 1 on the EFFECT axis. The axis
    // order would put rule 1's effect first; the rule index outranks it.
    let across_axes = load(
        "lowest_index_across_axes",
        r#"{"default_effect":"deny","rules":[
             {"callers":[],"targets":["*"],"effect":"allow"},
             {"callers":["*"],"targets":["*"],"effect":"Allow"}]}"#,
    );
    assert!(
        across_axes.contains("Rule 0") && !across_axes.contains("'Allow'"),
        "the lowest-indexed bad rule is the one reported (§6.2.1 point 2): {across_axes}"
    );

    // And the same file through the constructor, which is the comparison that
    // caught apcore-typescript: one door validating rule by rule while the
    // other sweeps axis by axis reports two different rules for one file.
    let constructed = ACL::try_new(
        vec![
            rule_of(&[], &["*"], "allow"),
            rule_of(&["*"], &["*"], "Allow"),
        ],
        "deny",
        None,
    )
    .expect_err("the rule set carries two bad rules")
    .message;
    assert!(
        constructed.contains("Rule 0") && !constructed.contains("'Allow'"),
        "ACL::try_new must name the same rule ACL::load does: {constructed}"
    );

    // `ACL::load`'s fourth axis, #107's rule-key closure, is subject to the
    // same rule: rule 0's bad `effect` outranks rule 1's unknown key, which
    // the pre-v1.31.0 two-pass shape got backwards.
    let against_key_axis = load(
        "lowest_index_against_key_axis",
        r#"{"default_effect":"deny","rules":[
             {"callers":["*"],"targets":["*"],"effect":"Allow"},
             {"callers":["*"],"targets":["*"],"effect":"deny","bogus":1}]}"#,
    );
    assert!(
        against_key_axis.contains("Rule 0") && !against_key_axis.contains("bogus"),
        "an axis MUST NOT be swept across every rule ahead of the next axis: {against_key_axis}"
    );

    // The control: with rule 0 clean, rule 1's fault is what is reported —
    // otherwise an implementation that always names rule 0 passes the above.
    let second_rule = load(
        "lowest_index_control",
        r#"{"default_effect":"deny","rules":[
             {"callers":["*"],"targets":["*"],"effect":"allow"},
             {"callers":["*"],"targets":[],"effect":"deny"}]}"#,
    );
    assert!(
        second_rule.contains("Rule 1") && second_rule.contains("targets"),
        "a fault in a later rule is still reported, named by its own index: {second_rule}"
    );
}

/// `default_effect` is judged **first**, and "first" reaches past the
/// individual rules to the file-level checks on the `rules` collection itself.
///
/// A document missing `rules`, or carrying a `rules` that is not a list, is
/// malformed at the file level rather than at any rule — there is no index to
/// name. §6.2.1 point 2 places `default_effect` ahead of those too, so a
/// document wrong in both is refused for the `default_effect`. No fixture case
/// covers the combination, deliberately: a doubly malformed document is
/// refused either way and only the message differs, which is exactly the class
/// of divergence that goes unnoticed until two SDKs are compared.
///
/// This SDK failed the missing-`rules` half. `ACL::load` looked the `rules`
/// key up before it had even read `default_effect`, so a file with both faults
/// was refused for the absent `rules` here while `ACL::try_new` — which cannot
/// see that fault at all — named the `default_effect`. The non-list half was
/// already correct, being caught by the deserialization further down.
#[test]
fn default_effect_is_judged_before_the_rules_collection_itself() {
    let dir = std::env::temp_dir().join("apcore_acl_pattern_arity");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    // The refusal quotes the file path, and these file names contain the word
    // `rules`. Strip it, or the negative assertions below read the file name
    // rather than the diagnosis.
    let load = |name: &str, doc: &str| -> String {
        let file = dir.join(format!("{name}.yaml"));
        std::fs::write(&file, doc).expect("write yaml");
        let path = file.to_str().expect("utf8").to_string();
        ACL::load(&path)
            .expect_err("the document is malformed twice over")
            .message
            .replace(&path, "<file>")
    };

    for (name, doc) in [
        // `rules` absent entirely.
        (
            "default_effect_before_missing_rules",
            r#"{"default_effect":"Allow"}"#,
        ),
        // `rules` present and not a list.
        (
            "default_effect_before_non_list_rules",
            r#"{"default_effect":"Allow","rules":{"callers":["*"]}}"#,
        ),
        // ...and not a list, in the shape an operator is likeliest to write:
        // a single rule mapping where a list of one belongs.
        (
            "default_effect_before_scalar_rules",
            r#"{"default_effect":"Allow","rules":"everything"}"#,
        ),
    ] {
        let message = load(name, doc);
        assert!(
            message.contains("'Allow'"),
            "[{name}] `default_effect` is judged ahead of the file-level checks on the \
             `rules` collection, not merely ahead of the individual rules (§6.2.1 \
             point 2): {message}"
        );
        assert!(
            !message.contains("rules"),
            "[{name}] and only one refusal is raised: {message}"
        );
    }

    // The controls. Each fault alone still produces its own refusal — without
    // these, an implementation that reports `default_effect` for every
    // malformed document passes the loop above.
    let missing_rules = load(
        "missing_rules_alone",
        r#"{"default_effect":"deny","not_rules":[]}"#,
    );
    assert!(
        missing_rules.contains("missing 'rules' key"),
        "a document missing `rules` and otherwise clean is still refused for `rules`: \
         {missing_rules}"
    );
    let non_list_rules = load(
        "non_list_rules_alone",
        r#"{"default_effect":"deny","rules":"everything"}"#,
    );
    assert!(
        non_list_rules.contains("rules"),
        "and so is a `rules` that is not a list: {non_list_rules}"
    );

    // The doors agree, which is the point of placing it at all: `try_new`
    // cannot see either `rules` fault, so if `load` named one of them the two
    // would answer differently for one document.
    let constructed = ACL::try_new(Vec::new(), "Allow", None)
        .expect_err("an out-of-enum default_effect is refused")
        .message;
    assert!(
        constructed.contains("'Allow'"),
        "ACL::try_new names the same axis ACL::load does: {constructed}"
    );
}
