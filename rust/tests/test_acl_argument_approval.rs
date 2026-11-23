//! Argument-scoped approval — PROTOCOL_SPEC §6.1.6–§6.1.8, §6.3.1, §6.8.1,
//! §6.9, §7.4 and §7.9.5 (spec v1.28.0, apcore#108).
//!
//! Before v1.28.0 every decision point that could read a call's arguments was
//! unable to escalate it to a human: the ACL could **refuse** on arguments and
//! `ApprovalHandler` could **wave through** on arguments, but nothing could
//! *ask*, and a refusal is not a question. An operator who needed
//! `git push --force` reviewed had to gate every `git push`.
//!
//! What is pinned here:
//!
//! * `approval: required` on a `deny` rule is rejected at load (§6.1.6 rule 2).
//! * The built-in `arguments` condition and its three predicates, including a
//!   malformed operand, which is UNEVALUABLE and not UNSATISFIED (§6.1.7,
//!   §6.1.1) — on **both** `matches_rule` and `matches_rule_async`, which are
//!   separate code paths.
//! * The legacy boolean `check()` / `async_check()` failing closed (§6.8.1).
//! * The governance projection: computed at Step 3, carrying no value (§6.1.8).
//! * §6.9 row 4 — an `ExecutionPolicy` override may ADD an approval
//!   requirement and MUST NOT remove one the ACL set.
//! * `AuditEntry.approval_required` beside `decision` (§6.3.1).
//! * `Executor::validate()` reporting the governance-effective union (§7.9.5).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};

use apcore::acl::{ACLRule, AccessDecision, ApprovalRequirement, AuditEntry, ACL};
use apcore::approval::{ApprovalHandler, ApprovalRequest, ApprovalResult};
use apcore::builtin_steps::BuiltinModuleLookup;
use apcore::context::{Context, Identity};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::executor::Executor;
use apcore::module::{Module, ModuleAnnotations};
use apcore::pipeline::{PipelineContext, Step};
use apcore::registry::registry::{ModuleDescriptor, Registry, DEFAULT_MODULE_VERSION};
use apcore::{
    build_standard_strategy, ExecutionPolicy, GovernanceProjection, JsonType, PolicyRule,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rule(callers: &[&str], targets: &[&str], effect: &str) -> ACLRule {
    ACLRule::new(
        callers.iter().map(|s| (*s).to_string()).collect(),
        targets.iter().map(|s| (*s).to_string()).collect(),
        effect,
    )
}

fn with_conditions(mut r: ACLRule, conditions: Value) -> ACLRule {
    r.conditions = Some(conditions);
    r
}

fn with_approval(mut r: ACLRule, approval: ApprovalRequirement) -> ACLRule {
    r.approval = Some(approval);
    r
}

fn ctx() -> Context<Value> {
    Context::new(Identity::new(
        "agent-1".to_string(),
        "agent".to_string(),
        vec!["ops".to_string()],
        HashMap::new(),
    ))
}

/// Write an ACL YAML document to a temp file and return its path.
fn write_acl(name: &str, yaml: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("apcore_acl_argument_approval");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join(format!("{name}.yaml"));
    std::fs::write(&path, yaml).expect("write yaml");
    path
}

fn load_acl(name: &str, yaml: &str) -> Result<ACL, ModuleError> {
    ACL::load(write_acl(name, yaml).to_str().expect("utf8"))
}

/// The driving example of §6.1.6: `cli.git_push` is allowed, and a call
/// carrying `force` is allowed **but** put to a human first.
///
/// Two rules and `default_effect: deny`, because that is how the rule would
/// really be written — gating `--force` is worth nothing if the un-gated push
/// falls through to a default-allow.
fn force_needs_approval() -> ACL {
    ACL::try_new(
        vec![
            with_approval(
                with_conditions(
                    rule(&["*"], &["cli.git_push"], "allow"),
                    json!({"arguments": {"has_key": ["force"]}}),
                ),
                ApprovalRequirement::Required,
            ),
            rule(&["*"], &["cli.git_push"], "allow"),
        ],
        "deny",
        None,
    )
    .expect("allow + approval is a valid combination")
}

/// An unconditional `allow` + `approval: required` rule.
///
/// §6.8.1's fail-closed requirement is about the *decision*, not about the
/// `arguments` condition: any rule resolving to allow-with-approval must make
/// the legacy boolean return `false`.
fn always_needs_approval() -> ACL {
    ACL::try_new(
        vec![with_approval(
            rule(&["*"], &["cli.git_push"], "allow"),
            ApprovalRequirement::Required,
        )],
        "deny",
        None,
    )
    .expect("construct")
}

fn projection(arguments: &Value) -> GovernanceProjection {
    GovernanceProjection::from_arguments(arguments)
}

// ---------------------------------------------------------------------------
// §6.1.6 — `approval: required` on a `deny` rule is rejected at load
// ---------------------------------------------------------------------------

#[test]
fn approval_required_on_a_deny_rule_is_rejected_by_load() {
    let err = load_acl(
        "deny_with_approval",
        r#"
default_effect: deny
rules:
  - callers: ["*"]
    targets: ["cli.git_push"]
    effect: deny
    approval: required
"#,
    )
    .expect_err(
        "`approval: required` on a `deny` rule means nothing — a refusal is not a question \
         (§6.1.6 rule 2)",
    );
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert!(
        err.message.contains("approval") && err.message.contains("deny"),
        "the refusal must name what is wrong with the rule: {}",
        err.message
    );
}

#[test]
fn approval_required_on_a_deny_rule_is_rejected_by_try_new() {
    // Direct construction is an entry point too — a governance rule silently
    // half-applied is the failure mode §6.1.5 was written to end.
    let err = ACL::try_new(
        vec![with_approval(
            rule(&["*"], &["cli.git_push"], "deny"),
            ApprovalRequirement::Required,
        )],
        "deny",
        None,
    )
    .expect_err("try_new must reject the meaningless combination too");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
}

#[test]
fn approval_required_on_a_deny_rule_is_rejected_by_add_rule() {
    // The third entry point. A rule built in code is as meaningless as one
    // parsed from YAML, so all three refuse it rather than two refusing and
    // one warning. `try_add_rule` is the fallible form, mirroring the
    // `new` / `try_new` pairing the crate already uses.
    let mut acl = ACL::try_new(vec![], "deny", None).expect("construct");
    let err = acl
        .try_add_rule(with_approval(
            rule(&["*"], &["cli.git_push"], "deny"),
            ApprovalRequirement::Required,
        ))
        .expect_err("runtime insertion must refuse the meaningless combination too");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert!(acl.rules().is_empty(), "the rejected rule is not inserted");

    // A valid rule still goes in, at position 0 as before.
    acl.try_add_rule(with_approval(
        rule(&["*"], &["cli.git_push"], "allow"),
        ApprovalRequirement::Required,
    ))
    .expect("allow + approval is valid");
    assert_eq!(acl.rules().len(), 1);
    assert!(acl.rules()[0].approval_required());
}

#[test]
#[should_panic(expected = "use ACL::try_add_rule")]
fn the_infallible_add_rule_panics_rather_than_accepting_it() {
    let mut acl = ACL::try_new(vec![], "deny", None).expect("construct");
    acl.add_rule(with_approval(
        rule(&["*"], &["cli.git_push"], "deny"),
        ApprovalRequirement::Required,
    ));
}

#[test]
fn approval_not_required_on_a_deny_rule_still_loads() {
    // Only `required` is meaningless on a `deny` rule. Writing the default out
    // explicitly is redundant, not wrong.
    let acl = load_acl(
        "deny_with_approval_not_required",
        r#"
default_effect: deny
rules:
  - callers: ["*"]
    targets: ["cli.git_push"]
    effect: deny
    approval: not_required
"#,
    )
    .expect("`approval: not_required` on a deny rule is redundant, not invalid");
    assert_eq!(acl.rules().len(), 1);
    assert!(!acl.rules()[0].approval_required());
}

#[test]
fn approval_is_a_recognised_rule_key() {
    // §6.1.5 closed the rule key set in v1.27.0; `approval` had to join it, or
    // every rule carrying it would be refused as an unknown key.
    let acl = load_acl(
        "allow_with_approval",
        r#"
default_effect: deny
rules:
  - callers: ["*"]
    targets: ["cli.git_push"]
    effect: allow
    approval: required
"#,
    )
    .expect("`approval` is part of the closed rule key set as of v1.28.0");
    assert!(acl.rules()[0].approval_required());
}

#[test]
fn an_absent_approval_key_means_not_required() {
    // §6.1.6 rule 1: every rule written before v1.28.0 keeps its meaning.
    let acl = ACL::try_new(vec![rule(&["*"], &["cli.git_push"], "allow")], "deny", None)
        .expect("construct");
    assert!(acl.rules()[0].approval.is_none());
    assert!(!acl.rules()[0].approval_required());
    let decision = acl.check_access(Some("agent.a"), "cli.git_push", None, None);
    assert!(decision.is_allowed());
    assert!(!decision.approval_required);
}

#[test]
fn an_unknown_approval_value_is_refused() {
    let err = load_acl(
        "bad_approval_value",
        r#"
default_effect: deny
rules:
  - callers: ["*"]
    targets: ["cli.git_push"]
    effect: allow
    approval: maybe
"#,
    )
    .expect_err("`approval` is a two-valued enum");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
}

// ---------------------------------------------------------------------------
// §6.8.1 — the structured result, and the legacy boolean failing closed
// ---------------------------------------------------------------------------

#[test]
fn check_access_reports_both_axes() {
    let acl = force_needs_approval();
    let decision: AccessDecision = acl.check_access(
        Some("agent.a"),
        "cli.git_push",
        Some(&ctx()),
        Some(&projection(&json!({"force": true, "remote": "origin"}))),
    );
    assert!(decision.is_allowed(), "authorization is unchanged");
    assert!(decision.approval_required, "the rule asked for a human");
    assert_eq!(decision.matched_rule_index, Some(0));
    assert_eq!(decision.reason, "rule_match");
}

#[test]
fn legacy_check_fails_closed_on_an_approval_requirement() {
    let acl = always_needs_approval();

    // The structured accessor says allow-but-ask.
    let decision = acl.check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), None);
    assert!(decision.is_allowed() && decision.approval_required);

    // The boolean says NO. A non-Executor caller can only read `true` as "let
    // it through", and that would run a call the ACL said needed a human
    // (§6.8.1). `false` is wrong in the benign direction.
    assert!(
        !acl.check(Some("agent.a"), "cli.git_push", Some(&ctx())),
        "check() MUST return false when the decision is allow-with-approval-required"
    );
}

#[tokio::test]
async fn legacy_async_check_fails_closed_on_an_approval_requirement() {
    let acl = always_needs_approval();
    let decision = acl
        .async_check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), None)
        .await;
    assert!(decision.is_allowed() && decision.approval_required);
    assert!(
        !acl.async_check(Some("agent.a"), "cli.git_push", Some(&ctx()))
            .await,
        "async_check() MUST fail closed on an approval requirement too"
    );
}

#[test]
fn a_plain_allow_still_returns_true_from_check() {
    // The fail-closed rule must not leak into ordinary rules: a legacy caller
    // only meets it once an operator has authored a rule carrying `approval`.
    let acl = ACL::try_new(vec![rule(&["*"], &["cli.git_push"], "allow")], "deny", None)
        .expect("construct");
    assert!(acl.check(Some("agent.a"), "cli.git_push", Some(&ctx())));
}

// ---------------------------------------------------------------------------
// §6.1.7 — the `arguments` condition
// ---------------------------------------------------------------------------

/// Drive one `arguments` predicate through the sync and the async rule loop.
///
/// `matches_rule` and `matches_rule_async` are separate code paths that resolve
/// conditions from different registries (§6.1.3); a predicate verified on one
/// says nothing about the other.
fn both_paths(conditions: Value, arguments: Value) -> (bool, bool) {
    let acl = ACL::try_new(
        vec![with_conditions(
            rule(&["*"], &["cli.git_push"], "allow"),
            conditions,
        )],
        "deny",
        None,
    )
    .expect("construct");
    let args = projection(&arguments);
    let sync = acl
        .check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), Some(&args))
        .is_allowed();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let acl_ref = &acl;
    let args_ref = &args;
    let asynchronous = rt.block_on(async move {
        acl_ref
            .async_check_access(
                Some("agent.a"),
                "cli.git_push",
                Some(&ctx()),
                Some(args_ref),
            )
            .await
            .is_allowed()
    });
    (sync, asynchronous)
}

#[test]
fn has_key_passes_when_any_named_key_is_present() {
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_key": ["force", "mirror"]}}),
            json!({"force": true, "remote": "origin"}),
        ),
        (true, true)
    );
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_key": ["force", "mirror"]}}),
            json!({"remote": "origin"}),
        ),
        (false, false)
    );
}

#[test]
fn has_all_keys_requires_every_named_key() {
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_all_keys": ["force", "remote"]}}),
            json!({"force": true, "remote": "origin"}),
        ),
        (true, true)
    );
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_all_keys": ["force", "remote"]}}),
            json!({"force": true}),
        ),
        (false, false)
    );
}

#[test]
fn has_none_of_passes_only_when_no_named_key_is_present() {
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_none_of": ["force"]}}),
            json!({"remote": "origin"}),
        ),
        (true, true)
    );
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_none_of": ["force"]}}),
            json!({"force": false}),
        ),
        (false, false),
        "presence is the question — `force: false` is still present"
    );
}

#[test]
fn several_predicates_in_one_object_are_anded() {
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_key": ["force"], "has_none_of": ["mirror"]}}),
            json!({"force": true}),
        ),
        (true, true)
    );
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_key": ["force"], "has_none_of": ["mirror"]}}),
            json!({"force": true, "mirror": true}),
        ),
        (false, false)
    );
}

/// A malformed predicate operand is UNEVALUABLE, not UNSATISFIED (§6.1.1).
///
/// The direction matters and only shows on a `deny` rule: an unsatisfied `deny`
/// rule goes inert and the call proceeds, which is exactly the fail-open
/// v1.22.0 was written to end. Reached here through the operand rather than
/// through the key.
#[test]
fn a_malformed_predicate_is_unevaluable_and_a_deny_rule_still_denies() {
    let audit: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&audit);
    let acl = ACL::try_new(
        vec![with_conditions(
            rule(&["*"], &["cli.git_push"], "deny"),
            // A bare string where a list of key names belongs.
            json!({"arguments": {"has_key": "force"}}),
        )],
        // default_effect: allow, so an inert deny rule would let the call
        // through and the assertion below would fail loudly.
        "allow",
        Some(Arc::new(move |entry: &AuditEntry| {
            sink.lock().push(entry.clone());
        })),
    )
    .expect("construct");

    let args = projection(&json!({"force": true}));
    let decision = acl.check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), Some(&args));
    assert!(
        !decision.is_allowed(),
        "an unevaluable condition on a deny rule makes the rule TAKE EFFECT (§6.1.1)"
    );
    let entries = audit.lock();
    let handler_error = entries[0]
        .handler_error
        .as_deref()
        .expect("handler_error names the unevaluable condition (§6.3.1)");
    assert!(
        handler_error.contains("arguments.has_key"),
        "handler_error must name the condition path: {handler_error}"
    );
}

#[test]
fn a_malformed_predicate_does_not_let_an_allow_rule_grant() {
    let acl = ACL::try_new(
        vec![with_conditions(
            rule(&["*"], &["cli.git_push"], "allow"),
            json!({"arguments": {"has_all_keys": [1, 2]}}),
        )],
        "deny",
        None,
    )
    .expect("construct");
    let args = projection(&json!({"force": true}));
    assert!(
        !acl.check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), Some(&args))
            .is_allowed(),
        "an allow rule whose condition cannot be evaluated MUST NOT grant"
    );
}

#[test]
fn a_non_object_arguments_value_is_unevaluable() {
    assert_eq!(
        both_paths(json!({"arguments": ["force"]}), json!({"force": true})),
        (false, false),
        "the operand of `arguments` is an object of predicates"
    );
}

#[test]
fn an_empty_predicate_object_is_unevaluable_not_vacuously_true() {
    // §6.1.7, and the reason §6.1 gives for `$not: {}` being fail-closed: an
    // operator who wrote `arguments: {}` asked nothing, and reading "asked
    // nothing" as "passes" turns a rule meant to restrict into an
    // unconditional one.
    assert_eq!(
        both_paths(json!({"arguments": {}}), json!({"force": true})),
        (false, false)
    );
}

#[test]
fn empty_predicate_arrays_are_well_formed_and_vacuous() {
    // An empty *array* is not a malformed operand — `any`/`all` over nothing
    // have their ordinary meanings.
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_key": []}}),
            json!({"force": true})
        ),
        (false, false),
        "has_key: [] — any of no keys is present: unsatisfied"
    );
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_all_keys": []}}),
            json!({"force": true}),
        ),
        (true, true),
        "has_all_keys: [] — every one of no keys is present: satisfied"
    );
    assert_eq!(
        both_paths(
            json!({"arguments": {"has_none_of": []}}),
            json!({"force": true}),
        ),
        (true, true),
        "has_none_of: [] — none of no keys is present: satisfied"
    );
}

#[test]
fn the_precheck_reports_a_broken_predicate() {
    // §6.1.4 covers the `arguments` operand, not only the condition key's
    // registry status: the predicate vocabulary is closed and there is no
    // registration point for it, so the shape of a well-formed operand is
    // knowable without a context and without running anything. Every fault is
    // reported — the precheck does not short-circuit.
    let acl = ACL::try_new(
        vec![
            with_conditions(
                rule(&["*"], &["cli.git_push"], "deny"),
                json!({"arguments": {"has_keys": ["force"], "has_none_of": 3}}),
            ),
            with_conditions(
                rule(&["*"], &["cli.git_push"], "deny"),
                json!({"arguments": {}}),
            ),
            with_conditions(
                rule(&["*"], &["cli.git_push"], "deny"),
                json!({"arguments": ["force"]}),
            ),
        ],
        "allow",
        None,
    )
    .expect("construct");

    let findings = acl.validate_rules();
    let paths: Vec<(usize, &str)> = findings
        .iter()
        .map(|f| (f.rule_index, f.condition_path.as_str()))
        .collect();
    assert_eq!(
        paths,
        vec![
            (0, "arguments.has_keys"),
            (0, "arguments.has_none_of"),
            (1, "arguments"),
            (2, "arguments"),
        ],
        "{findings:?}"
    );
    assert!(findings.iter().all(|f| !f.sync_resolvable));
}

#[test]
fn a_structurally_broken_predicate_denies_even_without_a_context() {
    // §6.1.4 rule 1: the precheck runs BEFORE §6.5's no-context check, which is
    // what closes the bypass where a broken condition on a `deny` rule passed
    // traffic simply because the caller carried no identity.
    let acl = ACL::try_new(
        vec![with_conditions(
            rule(&["*"], &["cli.git_push"], "deny"),
            json!({"arguments": {"has_keys": ["force"]}}),
        )],
        "allow",
        None,
    )
    .expect("construct");
    assert!(!acl
        .check_access(Some("agent.a"), "cli.git_push", None, None)
        .is_allowed());
}

#[test]
fn an_unknown_predicate_is_unevaluable_not_ignored() {
    // The vocabulary is closed. A value-level predicate such as `equals` is
    // deliberately unspecified; silently ignoring it would leave a rule its
    // author believed was restrictive with no restriction at all.
    assert_eq!(
        both_paths(
            json!({"arguments": {"equals": ["force"]}}),
            json!({"force": true}),
        ),
        (false, false)
    );
}

#[test]
fn a_misspelled_condition_key_is_caught_by_the_precheck() {
    // §6.1.7: being built-in means §6.1.4's precheck covers it for free —
    // `argument:` written for `arguments:` is an unregistered condition key.
    let acl = ACL::try_new(
        vec![with_conditions(
            rule(&["*"], &["cli.git_push"], "deny"),
            json!({"argument": {"has_key": ["force"]}}),
        )],
        "allow",
        None,
    )
    .expect("construct");
    let findings = acl.validate_rules();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].condition_key.as_deref(), Some("argument"));
    assert!(!findings[0].sync_resolvable);
}

#[test]
fn without_a_projection_the_condition_is_unevaluable() {
    // A caller with no call site is asking "may X reach Y?", not "may X reach Y
    // with these arguments?". The question cannot be answered as written, so it
    // is unevaluable (§6.1.1's principle) rather than vacuously false — the
    // fail-closed direction on both rule effects.
    let deny = ACL::try_new(
        vec![with_conditions(
            rule(&["*"], &["cli.git_push"], "deny"),
            json!({"arguments": {"has_key": ["force"]}}),
        )],
        "allow",
        None,
    )
    .expect("construct");
    assert!(
        !deny
            .check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), None)
            .is_allowed(),
        "the deny rule takes effect"
    );

    let allow = ACL::try_new(
        vec![with_approval(
            with_conditions(
                rule(&["*"], &["cli.git_push"], "allow"),
                json!({"arguments": {"has_key": ["force"]}}),
            ),
            ApprovalRequirement::Required,
        )],
        "deny",
        None,
    )
    .expect("construct");
    let decision = allow.check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), None);
    assert!(
        !decision.is_allowed(),
        "the allow rule does not grant, so default_effect decides"
    );
}

// ---------------------------------------------------------------------------
// §6.1.8 — the governance projection
// ---------------------------------------------------------------------------

#[test]
fn the_projection_carries_keys_and_types_but_no_value() {
    let p = projection(&json!({
        "force": true,
        "remote": "origin",
        "depth": 1,
        "refs": ["main"],
        "opts": {"k": "v"},
        "note": null,
        "password": "hunter2",
    }));
    assert_eq!(
        p.keys().collect::<Vec<_>>(),
        vec!["depth", "force", "note", "opts", "password", "refs", "remote"]
    );
    assert_eq!(p.type_of("force"), Some(JsonType::Boolean));
    assert_eq!(p.type_of("remote"), Some(JsonType::String));
    assert_eq!(p.type_of("depth"), Some(JsonType::Number));
    assert_eq!(p.type_of("refs"), Some(JsonType::Array));
    assert_eq!(p.type_of("opts"), Some(JsonType::Object));
    assert_eq!(p.type_of("note"), Some(JsonType::Null));
    assert_eq!(p.type_of("missing"), None);

    // The point of the type: a projection that structurally cannot hold a value
    // cannot leak one, whatever a future predicate does with it. `Debug` is the
    // one surface that would print a value if the type held one, and it is what
    // a diagnostic or a panic message reaches for.
    let rendered = format!("{p:?}");
    assert!(
        !rendered.contains("hunter2") && !rendered.contains("origin"),
        "the projection must never carry an argument value: {rendered}"
    );
}

#[test]
fn the_projection_excludes_the_framework_approval_token() {
    // §7.9.6 rule 5 excludes `_approval_token` from policy resolution on the
    // grounds that it is a protocol-level key rather than caller input. The
    // same holds here, and it keeps Step 4's verdict identical across the
    // approval suspend/resume boundary, which §7.4 re-enters from Step 1 with
    // the token present in `arguments`.
    let p = projection(&json!({"force": true, "_approval_token": "apr-1"}));
    assert_eq!(p.keys().collect::<Vec<_>>(), vec!["force"]);
    assert!(!p.contains_key("_approval_token"));
}

#[test]
fn non_object_arguments_project_to_an_empty_key_set() {
    // The ACL check is Step 4 and input validation is Step 7, so `inputs` here
    // is whatever the caller passed.
    for value in [json!(null), json!(3), json!("x"), json!(["a"])] {
        let p = projection(&value);
        assert!(p.is_empty(), "{value} projected to {p:?}");
        assert_eq!(p.len(), 0);
    }
}

#[tokio::test]
async fn module_lookup_populates_the_projection_before_step_4() {
    // §6.1.8 rule 1 makes the ordering normative. Two halves: the step that
    // computes it, and the strategy order that puts that step ahead of the ACL.
    let registry = make_registry();
    let mut pipe = PipelineContext::new(
        "cli.git_push",
        json!({"force": true, "remote": "origin"}),
        Context::<Value>::anonymous(),
        "standard",
    );
    pipe.registry = Some(Arc::clone(&registry));
    pipe.config = Some(Arc::new(apcore::config::Config::default()));
    assert!(
        pipe.governance_projection.is_none(),
        "nothing before Step 3 computes it"
    );

    BuiltinModuleLookup::default()
        .execute(&mut pipe)
        .await
        .expect("module lookup");

    let p = pipe
        .governance_projection
        .as_ref()
        .expect("module_lookup (Step 3) MUST compute the projection");
    assert!(p.contains_key("force") && p.contains_key("remote"));

    let names = build_standard_strategy().step_names();
    let lookup = names
        .iter()
        .position(|n| n == "module_lookup")
        .expect("step");
    let acl = names.iter().position(|n| n == "acl_check").expect("step");
    assert!(
        lookup < acl,
        "the projection is computed at Step 3 and read at Step 4: {names:?}"
    );
}

#[tokio::test]
async fn the_projection_never_reaches_the_wire_context() {
    // §6.1.8 rule 3: the projection MUST be computed by the framework and MUST
    // NOT be accepted from caller-supplied input. A caller that could supply
    // its own would satisfy `has_none_of` for a call whose arguments say
    // otherwise, which turns the condition into a caller-controlled switch.
    //
    // apcore-rust carries it on `PipelineContext`, which is framework-internal
    // and never deserialized, so there is no field for a wire payload to name.
    // `src/acl.rs`'s `projection_forgery_tests` pin the structural half (no
    // serde derives, nothing on `Context`); this pins the observable half.
    let registry = make_registry();
    let mut pipe = PipelineContext::new(
        "cli.git_push",
        json!({"force": true}),
        Context::<Value>::anonymous(),
        "standard",
    );
    pipe.registry = Some(Arc::clone(&registry));
    pipe.config = Some(Arc::new(apcore::config::Config::default()));
    BuiltinModuleLookup::default()
        .execute(&mut pipe)
        .await
        .expect("module lookup");
    assert!(pipe.governance_projection.is_some());

    let wire = serde_json::to_value(&pipe.context).expect("Context serializes");
    let rendered = wire.to_string();
    assert!(
        !rendered.contains("projection"),
        "nothing a caller can send names the projection: {rendered}"
    );
}

#[tokio::test]
async fn the_projection_is_not_context_redacted_inputs() {
    // §6.1.8 rule 3 is a MUST NOT. `redacted_inputs` is a raw copy of the
    // arguments when the module declares no field-level `x-sensitive` marker,
    // so it carries values; the projection never does.
    let registry = make_registry();
    let mut pipe = PipelineContext::new(
        "cli.git_push",
        json!({"remote": "origin"}),
        Context::<Value>::anonymous(),
        "standard",
    );
    pipe.registry = Some(Arc::clone(&registry));
    pipe.config = Some(Arc::new(apcore::config::Config::default()));
    BuiltinModuleLookup::default()
        .execute(&mut pipe)
        .await
        .expect("module lookup");

    let redacted = pipe
        .context
        .redacted_inputs
        .as_ref()
        .expect("redacted_inputs is still populated for logging");
    assert_eq!(
        redacted.get("remote"),
        Some(&json!("origin")),
        "redacted_inputs holds values — which is exactly why the ACL must not read it"
    );
    let rendered = format!("{:?}", pipe.governance_projection);
    assert!(!rendered.contains("origin"), "{rendered}");
}

// ---------------------------------------------------------------------------
// §6.3.1 — AuditEntry.approval_required
// ---------------------------------------------------------------------------

#[test]
fn the_audit_entry_carries_the_requirement_beside_the_decision() {
    let audit: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&audit);
    let mut acl = force_needs_approval();
    acl.set_audit_logger(move |entry: &AuditEntry| sink.lock().push(entry.clone()));

    let args = projection(&json!({"force": true}));
    acl.check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), Some(&args));

    let entries = audit.lock();
    assert_eq!(entries.len(), 1, "exactly one entry per check");
    assert_eq!(
        entries[0].decision, "allow",
        "`decision` stays allow/deny — a third value would break every existing parser (§6.9 row 7)"
    );
    assert!(entries[0].approval_required);
    assert_eq!(entries[0].matched_rule_index, Some(0));

    // The wire shape carries the new field beside the old one.
    let wire = serde_json::to_value(&entries[0]).expect("serialize");
    assert_eq!(wire["decision"], json!("allow"));
    assert_eq!(wire["approval_required"], json!(true));
}

#[test]
fn approval_required_is_false_when_no_rule_matched() {
    // §6.9 row 2: `default_effect` is allow/deny only; there is no default
    // approval requirement.
    let audit: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&audit);
    let acl = ACL::try_new(
        vec![with_approval(
            rule(&["admin.*"], &["cli.git_push"], "allow"),
            ApprovalRequirement::Required,
        )],
        "allow",
        Some(Arc::new(move |entry: &AuditEntry| {
            sink.lock().push(entry.clone());
        })),
    )
    .expect("construct");

    let decision = acl.check_access(Some("agent.a"), "cli.git_push", Some(&ctx()), None);
    assert!(decision.is_allowed());
    assert!(!decision.approval_required);
    assert_eq!(decision.reason, "default_effect");
    assert!(!audit.lock()[0].approval_required);
}

#[test]
fn an_audit_entry_written_before_v1_28_still_deserializes() {
    let legacy = json!({
        "timestamp": "2026-08-28T00:00:00Z",
        "caller_id": "agent.a",
        "target_id": "cli.git_push",
        "decision": "allow",
        "reason": "rule_match",
        "roles": [],
    });
    let entry: AuditEntry = serde_json::from_value(legacy).expect("deserialize");
    assert!(!entry.approval_required);
}

// ---------------------------------------------------------------------------
// Executor integration — §7.4, §7.9.5 and §6.9 row 4
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PushModule;

#[async_trait]
impl Module for PushModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "push module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"status": "pushed"}))
    }
}

fn descriptor(module_id: &str, requires_approval: bool) -> ModuleDescriptor {
    ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: "push module".to_string(),
        documentation: None,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        version: DEFAULT_MODULE_VERSION.to_string(),
        tags: vec![],
        annotations: Some(ModuleAnnotations {
            requires_approval,
            ..ModuleAnnotations::default()
        }),
        examples: vec![],
        metadata: HashMap::new(),
        display: None,
        sunset_date: None,
        dependencies: vec![],
        enabled: true,
    }
}

fn make_registry() -> Arc<Registry> {
    let reg = Arc::new(Registry::new());
    reg.register(
        "cli.git_push",
        Box::new(PushModule),
        descriptor("cli.git_push", false),
    )
    .expect("register");
    reg
}

/// Approval handler that records every request it is handed.
#[derive(Debug)]
struct RecordingHandler {
    requests: Arc<Mutex<Vec<ApprovalRequest>>>,
}

impl RecordingHandler {
    fn new() -> (Self, Arc<Mutex<Vec<ApprovalRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                requests: Arc::clone(&requests),
            },
            requests,
        )
    }
}

#[async_trait]
impl ApprovalHandler for RecordingHandler {
    async fn request_approval(
        &self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalResult, ModuleError> {
        self.requests.lock().push(request.clone());
        let mut result = ApprovalResult::default();
        result.status = "approved".to_string();
        Ok(result)
    }
    async fn check_approval(&self, _id: &str) -> Result<ApprovalResult, ModuleError> {
        let mut result = ApprovalResult::default();
        result.status = "rejected".to_string();
        Ok(result)
    }
}

fn executor_with_acl() -> (Executor, Arc<Mutex<Vec<ApprovalRequest>>>) {
    let (handler, requests) = RecordingHandler::new();
    let mut exec = Executor::new(make_registry(), apcore::config::Config::default());
    exec.set_acl(force_needs_approval());
    exec.set_approval_handler(Box::new(handler));
    (exec, requests)
}

#[tokio::test]
async fn the_acl_requirement_reaches_the_step_5_gate() {
    // §7.4: an implementation that reads only the annotation silently ignores
    // every rule carrying `approval` — the rule loads, matches, and does
    // nothing. The module here declares `requires_approval: false`.
    let (exec, requests) = executor_with_acl();

    let out = exec
        .call("cli.git_push", json!({"force": true}), None, None)
        .await
        .expect("approved");
    assert_eq!(out["status"], "pushed");
    assert_eq!(
        requests.lock().len(),
        1,
        "the gate fired on the ACL's say-so"
    );
    assert!(
        requests.lock()[0].annotations.requires_approval,
        "§7.4 rule 3: an ACL-sourced requirement makes requires_approval effectively true"
    );
}

#[tokio::test]
async fn a_call_without_the_gated_argument_is_not_gated() {
    let (exec, requests) = executor_with_acl();
    let out = exec
        .call("cli.git_push", json!({"remote": "origin"}), None, None)
        .await
        .expect("allowed");
    assert_eq!(out["status"], "pushed");
    assert!(
        requests.lock().is_empty(),
        "the whole point: gate the calls that carry --force, not every push"
    );
}

/// §6.9 row 4 — the privilege-escalation guard.
///
/// A module-scoped `ExecutionPolicy` may ADD an approval requirement and MUST
/// NOT remove one the caller-scoped ACL set. Letting a policy rule written for
/// `cli.*` strip a requirement an ACL author attached to one untrusted caller
/// is the escalation the union exists to prevent.
#[tokio::test]
async fn a_policy_requires_approval_false_cannot_clear_the_acl_requirement() {
    let (exec_base, requests) = executor_with_acl();
    let mut exec = exec_base;
    exec.set_policy(Some(ExecutionPolicy::new(vec![PolicyRule::new("cli.*")
        .expect("pattern")
        .with_requires_approval(false)
        .with_reason("platform default: cli modules are unattended")])));

    let out = exec
        .call("cli.git_push", json!({"force": true}), None, None)
        .await
        .expect("approved");
    assert_eq!(out["status"], "pushed");
    assert_eq!(
        requests.lock().len(),
        1,
        "the policy override MUST NOT clear the ACL's requirement (§6.9 row 4)"
    );
}

#[tokio::test]
async fn a_policy_may_still_add_a_requirement() {
    // The other half of row 4: union, not precedence. A policy that gates a
    // module gates it whether or not the ACL asked for anything.
    let (handler, requests) = RecordingHandler::new();
    let mut exec = Executor::new(make_registry(), apcore::config::Config::default());
    exec.set_acl(force_needs_approval());
    exec.set_approval_handler(Box::new(handler));
    exec.set_policy(Some(ExecutionPolicy::new(vec![PolicyRule::new("cli.*")
        .expect("pattern")
        .with_requires_approval(true)
        .with_reason("sign-off")])));

    // No `force` argument, so the ACL asks for nothing; the policy still does.
    exec.call("cli.git_push", json!({"remote": "origin"}), None, None)
        .await
        .expect("approved");
    assert_eq!(requests.lock().len(), 1);
}

#[tokio::test]
async fn preflight_reports_the_governance_effective_requirement() {
    // §7.9.5: `validate()` reports the union of §6.9 rows 3-5 for the given
    // call site. Reporting only the policy-effective value would tell a caller
    // no approval is needed for a call the Step-5 gate will stop.
    let (exec, _requests) = executor_with_acl();

    let gated = exec
        .validate("cli.git_push", &json!({"force": true}), None)
        .await
        .expect("validate");
    assert!(gated.valid, "{:?}", gated.checks);
    assert!(
        gated.requires_approval,
        "the ACL rule requires a human for this call site"
    );

    let ungated = exec
        .validate("cli.git_push", &json!({"remote": "origin"}), None)
        .await
        .expect("validate");
    assert!(ungated.valid);
    assert!(
        !ungated.requires_approval,
        "the same module without the gated argument needs no approval"
    );
}

#[tokio::test]
async fn preflight_keeps_reporting_the_requirement_under_a_clearing_policy() {
    let (exec_base, _requests) = executor_with_acl();
    let mut exec = exec_base;
    exec.set_policy(Some(ExecutionPolicy::new(vec![PolicyRule::new("cli.*")
        .expect("pattern")
        .with_requires_approval(false)])));

    let result = exec
        .validate("cli.git_push", &json!({"force": true}), None)
        .await
        .expect("validate");
    assert!(
        result.requires_approval,
        "preflight reports the union, so it agrees with the gate (§7.9.5, §6.9 row 6)"
    );
}

// ---------------------------------------------------------------------------
// §6.1.1 rule 5 — an unevaluable `allow` rule's approval requirement is
// PENDING, not discarded (spec v1.29.0, apcore#109)
// ---------------------------------------------------------------------------
//
// v1.22.0 wrote "an `allow` rule MUST NOT grant" when a rule carried one axis,
// and stepping aside was then harmless because whatever granted next also said
// `allow`. v1.28.0 gave rules a second axis and "does not grant" began
// discarding it: on the very shape §6.1.7 exists for — `force_needs_approval()`
// — the gate stepped aside, the broad rule granted, and the result was `allow`
// with `approval_required: false` on exactly the call the operator gated.
//
// The fixture driver exercises this on the sync path over 24 cases. What is
// pinned here is the async twin (a separate scan loop), the audit entry, and
// the containment clause, none of which the driver reaches.

/// Both scan loops, for one call against one ACL: `(sync, async)` decisions.
///
/// `check_inner` and `async_check_inner` are separate loops carrying separate
/// pending-approval accumulators; a rule-5 behaviour verified on one says
/// nothing about the other.
fn both_decisions(
    acl: &ACL,
    target: &str,
    projection: Option<&GovernanceProjection>,
) -> (AccessDecision, AccessDecision) {
    let sync = acl.check_access(Some("agent.planner"), target, Some(&ctx()), projection);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let asynchronous = rt.block_on(async {
        acl.async_check_access(Some("agent.planner"), target, Some(&ctx()), projection)
            .await
    });
    (sync, asynchronous)
}

#[test]
fn a_stepped_aside_approval_rule_still_gates_the_rule_that_grants() {
    // THE DEFECT. No projection, so the `arguments` gate is unevaluable
    // (§6.1.8 rule 1) and rule 0 must not grant — but rule 1 does, and it
    // inherits the requirement rule 0 carried.
    let acl = force_needs_approval();
    let (sync, asynchronous) = both_decisions(&acl, "cli.git_push", None);

    for (label, decision) in [("sync", &sync), ("async", &asynchronous)] {
        assert!(decision.is_allowed(), "[{label}] rule 1 grants");
        assert!(
            decision.approval_required,
            "[{label}] the requirement rule 0 carried is PENDING, not discarded (§6.1.1 rule 5)"
        );
        assert_eq!(
            decision.matched_rule_index,
            Some(1),
            "[{label}] `matched_rule_index` still names the rule that decided ACCESS"
        );
    }
}

#[test]
fn the_legacy_boolean_fails_closed_on_a_pending_requirement() {
    // §6.8.1 as amended: fail-closed is a property of the DECISION, so it
    // holds for a requirement that originated in a rule which did not match.
    // Without rule 5 this returns `true` and a legacy caller runs the gated
    // call.
    let acl = force_needs_approval();
    assert!(!acl.check(Some("agent.planner"), "cli.git_push", Some(&ctx())));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    assert!(!rt.block_on(async {
        acl.async_check(Some("agent.planner"), "cli.git_push", Some(&ctx()))
            .await
    }));
}

#[test]
fn a_pending_requirement_rides_through_default_effect_allow() {
    // The boundary a "a later rule grants" reading misses: there IS no later
    // rule. `approval_required: true` with `matched_rule_index: None` is a
    // legal combination since v1.29.0 (§6.9 row 2).
    let acl = ACL::try_new(
        vec![with_approval(
            with_conditions(
                rule(&["*"], &["cli.git_push"], "allow"),
                json!({"arguments": {"has_key": ["force"]}}),
            ),
            ApprovalRequirement::Required,
        )],
        // Against the house rule deliberately: the fall-through is not
        // observable against a `deny` default, because not-granting and
        // denying produce the same answer.
        "allow",
        None,
    )
    .expect("construct");

    let (sync, asynchronous) = both_decisions(&acl, "cli.git_push", None);
    for (label, decision) in [("sync", &sync), ("async", &asynchronous)] {
        assert!(decision.is_allowed(), "[{label}] the default grants");
        assert!(
            decision.approval_required,
            "[{label}] the default MUST carry the pending requirement (§6.1.1 rule 5)"
        );
        assert_eq!(
            decision.matched_rule_index, None,
            "[{label}] no rule matched"
        );
        assert_eq!(decision.reason, "default_effect", "[{label}]");
    }
}

#[test]
fn a_denial_clears_the_pending_requirement() {
    // "Denied and needs approval" is not a state that means anything (§6.1.6):
    // the call is not going to run, so there is nothing to put to a human.
    // Rule 0 raises a pending requirement, rule 1 denies.
    let acl = ACL::try_new(
        vec![
            with_approval(
                with_conditions(
                    rule(&["*"], &["cli.git_push"], "allow"),
                    json!({"arguments": {"has_key": ["force"]}}),
                ),
                ApprovalRequirement::Required,
            ),
            rule(&["*"], &["cli.git_push"], "deny"),
        ],
        "allow",
        None,
    )
    .expect("construct");

    let (sync, asynchronous) = both_decisions(&acl, "cli.git_push", None);
    for (label, decision) in [("sync", &sync), ("async", &asynchronous)] {
        assert!(!decision.is_allowed(), "[{label}] rule 1 denies");
        assert!(
            !decision.approval_required,
            "[{label}] a denial clears the pending requirement (§6.1.1 rule 5)"
        );
        assert_eq!(
            decision.matched_rule_index,
            Some(1),
            "[{label}] `matched_rule_index` names the rule that actually decided"
        );
    }
}

#[test]
fn an_out_of_scope_approval_rule_raises_nothing() {
    // The containment clause, and what keeps the fix from over-reaching: rule
    // 0 is written about `cli.deploy`, so its conditions are never consulted
    // (§6.1.4 rule 4) and it must not attach a human to a call it was never
    // written about. An implementation that sets `pending` before matching
    // patterns passes every other test in this section and fails this one.
    let acl = ACL::try_new(
        vec![
            with_approval(
                with_conditions(
                    rule(&["*"], &["cli.deploy"], "allow"),
                    json!({"arguments": {"has_key": ["force"]}}),
                ),
                ApprovalRequirement::Required,
            ),
            rule(&["*"], &["cli.git_push"], "allow"),
        ],
        "deny",
        None,
    )
    .expect("construct");

    let (sync, asynchronous) = both_decisions(&acl, "cli.git_push", None);
    for (label, decision) in [("sync", &sync), ("async", &asynchronous)] {
        assert!(decision.is_allowed(), "[{label}]");
        assert!(
            !decision.approval_required,
            "[{label}] a rule whose patterns do not match raises no pending requirement"
        );
        assert_eq!(decision.matched_rule_index, Some(1), "[{label}]");
    }
    assert!(
        acl.check(Some("agent.planner"), "cli.git_push", Some(&ctx())),
        "and the legacy boolean is not dragged closed by an unrelated rule"
    );
}

#[test]
fn the_audit_entry_carries_the_final_approval_value_and_still_reports_the_fault() {
    // §6.1.1 rule 5's closing sentence: a pending requirement neither
    // suppresses nor substitutes for `handler_error`. The entry must show BOTH
    // — the gate could not be evaluated, and a human is required anyway — or
    // the audit trail says the call ran unapproved.
    let audit: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&audit);
    let acl = ACL::try_new(
        vec![
            with_approval(
                with_conditions(
                    rule(&["*"], &["cli.git_push"], "allow"),
                    json!({"arguments": {"has_key": ["force"]}}),
                ),
                ApprovalRequirement::Required,
            ),
            rule(&["*"], &["cli.git_push"], "allow"),
        ],
        "deny",
        Some(Arc::new(move |entry: &AuditEntry| {
            sink.lock().push(entry.clone());
        })),
    )
    .expect("construct");

    let decision = acl.check_access(Some("agent.planner"), "cli.git_push", Some(&ctx()), None);
    let entries = audit.lock();
    assert_eq!(entries.len(), 1, "one decision, one entry");
    assert_eq!(
        entries[0].approval_required, decision.approval_required,
        "the entry reports the FINAL value, not the matched rule's own"
    );
    assert!(entries[0].approval_required);
    assert_eq!(entries[0].matched_rule_index, Some(1));
    let handler_error = entries[0]
        .handler_error
        .as_deref()
        .expect("the unevaluable gate is still reported (§6.3.1)");
    assert!(
        handler_error.contains("arguments"),
        "handler_error must name the condition path: {handler_error}"
    );
}

#[test]
fn a_misspelled_predicate_does_not_drop_the_requirement_on_the_executor_path() {
    // Not confined to the projection-less legacy boolean. `has_keys` written
    // for `has_all_keys` is a precheck fault (§6.1.8 case 3), so the rule is
    // unevaluable WITH a projection present, on the ordinary Executor path.
    // One typo turned "ask a human" into "do not ask".
    let acl = ACL::try_new(
        vec![
            with_approval(
                with_conditions(
                    rule(&["*"], &["cli.git_push"], "allow"),
                    json!({"arguments": {"has_keys": ["force"]}}),
                ),
                ApprovalRequirement::Required,
            ),
            rule(&["*"], &["cli.git_push"], "allow"),
        ],
        "deny",
        None,
    )
    .expect("construct");

    let args = projection(&json!({"remote": "origin", "force": true}));
    let (sync, asynchronous) = both_decisions(&acl, "cli.git_push", Some(&args));
    for (label, decision) in [("sync", &sync), ("async", &asynchronous)] {
        assert!(decision.is_allowed(), "[{label}]");
        assert!(
            decision.approval_required,
            "[{label}] a typo MUST NOT silently disarm the gate (§6.1.1 rule 5)"
        );
        assert_eq!(decision.matched_rule_index, Some(1), "[{label}]");
    }
    // §6.1.2 makes an unregistered key a warning rather than a load failure, so
    // deploy-time validation is not a mitigation that can be assumed — but it
    // does name the fault.
    assert!(
        acl.validate_rules()
            .iter()
            .any(|f| f.condition_path == "arguments.has_keys"),
        "validate_rules() surfaces the misspelling at deploy time"
    );
}

#[tokio::test]
async fn a_typo_in_the_gate_still_reaches_the_step_5_approval_gate() {
    // End to end, on the ordinary Executor pipeline with a projection present:
    // the gate rule is unevaluable because `has_keys` is not a predicate name,
    // and the broad rule grants. Before §6.1.1 rule 5 this ran `git push
    // --force` with the handler never consulted — no denial, no error, no
    // audit record of an approval, just a call the operator gated going
    // through. Asserted at the gate rather than at the ACL because that is
    // where the consequence lands.
    let (handler, requests) = RecordingHandler::new();
    let mut exec = Executor::new(make_registry(), apcore::config::Config::default());
    exec.set_acl(
        ACL::try_new(
            vec![
                with_approval(
                    with_conditions(
                        rule(&["*"], &["cli.git_push"], "allow"),
                        json!({"arguments": {"has_keys": ["force"]}}),
                    ),
                    ApprovalRequirement::Required,
                ),
                rule(&["*"], &["cli.git_push"], "allow"),
            ],
            "deny",
            None,
        )
        .expect("construct"),
    );
    exec.set_approval_handler(Box::new(handler));

    let out = exec
        .call("cli.git_push", json!({"force": true}), None, None)
        .await
        .expect("approved");
    assert_eq!(out["status"], "pushed");
    assert_eq!(
        requests.lock().len(),
        1,
        "the pending requirement reaches the Step-5 gate (§6.1.1 rule 5, §7.4)"
    );
}
