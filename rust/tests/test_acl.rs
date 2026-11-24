//! Tests for ACL types, construction, and check() behavior.

use apcore::acl::{ACLRule, ApprovalRequirement, ACL};
use apcore::context::{Context, Identity};
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Rule builders
//
// `ACLRule` is `#[non_exhaustive]` (apcore#38), so a test crate builds one
// through `ACLRule::new` and assigns the optional fields afterwards. These two
// helpers keep that one line long at the call sites below, which care about a
// rule's three required fields and, occasionally, its description.
// ---------------------------------------------------------------------------

fn rule(callers: &[&str], targets: &[&str], effect: &str) -> ACLRule {
    ACLRule::new(
        callers.iter().map(|s| (*s).to_string()).collect(),
        targets.iter().map(|s| (*s).to_string()).collect(),
        effect,
    )
}

fn described(callers: &[&str], targets: &[&str], effect: &str, description: &str) -> ACLRule {
    let mut r = rule(callers, targets, effect);
    r.description = Some(description.to_string());
    r
}

// ---------------------------------------------------------------------------
// ACL construction
// ---------------------------------------------------------------------------

#[test]
fn test_acl_new_is_empty() {
    let acl = ACL::new(vec![], "deny", None);
    assert!(acl.rules().is_empty());
}

#[test]
fn test_acl_default_is_empty() {
    let acl = ACL::default();
    assert!(acl.rules().is_empty());
}

// ---------------------------------------------------------------------------
// ACLRule construction
// ---------------------------------------------------------------------------

#[test]
fn test_acl_rule_fields() {
    // The construction form `#[non_exhaustive]` leaves available to every crate
    // (api-surface-conventions.md §9.3): `new()` for the required fields, then
    // assignment for the optional ones. A struct literal does not compile from
    // outside apcore and is pinned as such in tests/compile_fail/.
    let mut rule = ACLRule::new(
        vec!["admin".to_string()],
        vec!["admin.*".to_string()],
        "allow",
    );
    rule.description = Some("Admins may administer".to_string());
    rule.approval = Some(ApprovalRequirement::Required);

    assert_eq!(rule.callers, vec!["admin"]);
    assert_eq!(rule.targets, vec!["admin.*"]);
    assert_eq!(rule.effect, "allow");
    assert_eq!(rule.description.as_deref(), Some("Admins may administer"));
    assert!(rule.approval_required());
}

#[test]
fn test_acl_rule_new_sets_required_fields_and_leaves_optional_fields_unset() {
    let rule = ACLRule::new(
        vec!["admin".to_string()],
        vec!["admin.*".to_string()],
        "allow",
    );

    assert_eq!(rule.callers, vec!["admin"]);
    assert_eq!(rule.targets, vec!["admin.*"]);
    assert_eq!(rule.effect, "allow");
    assert_eq!(rule.approval, None);
    assert_eq!(rule.description, None);
    assert_eq!(rule.conditions, None);
}

#[test]
fn test_acl_rule_deny() {
    let mut rule = ACLRule::new(vec!["guest".to_string()], vec!["*".to_string()], "deny");
    rule.description = Some("Deny all guests".to_string());
    assert_eq!(rule.effect, "deny");
    assert_eq!(rule.description.as_deref(), Some("Deny all guests"));
}

#[test]
fn test_acl_rule_with_conditions() {
    let mut rule = ACLRule::new(
        vec!["user".to_string()],
        vec!["data.*".to_string()],
        "allow",
    );
    rule.conditions = Some(serde_json::json!({"ip_range": "10.0.0.0/8"}));
    assert!(rule.conditions.is_some());
}

#[test]
fn test_acl_rule_serialization_round_trip() {
    let rule = ACLRule::new(
        vec!["user".to_string(), "admin".to_string()],
        vec!["user.*".to_string()],
        "allow",
    );
    let json = serde_json::to_string(&rule).unwrap();
    let restored: ACLRule = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.callers, rule.callers);
    assert_eq!(restored.targets, rule.targets);
    assert_eq!(restored.effect, rule.effect);
}

#[test]
fn test_acl_new_with_rules() {
    let rules = vec![rule(&["admin"], &["*"], "allow")];
    let acl = ACL::new(rules, "deny", None);
    assert_eq!(acl.rules().len(), 1);
}

// ---------------------------------------------------------------------------
// ACL.check() — allow rule matches
// ---------------------------------------------------------------------------

fn make_ctx(id: &str, id_type: &str, roles: Vec<String>) -> Context<Value> {
    Context::<Value>::new(Identity::new(
        id.to_string(),
        id_type.to_string(),
        roles,
        HashMap::default(),
    ))
}

#[test]
fn test_check_allow_rule_matches() {
    let rules = vec![described(
        &["admin"],
        &["secrets.*"],
        "allow",
        "Admin can access secrets",
    )];
    let acl = ACL::new(rules, "deny", None);
    let ctx = make_ctx("admin", "user", vec![]);
    let result = acl.check(Some("admin"), "secrets.read", Some(&ctx));
    assert!(result, "Admin should be allowed to access secrets.*");
}

#[test]
fn test_check_allow_without_context() {
    let rules = vec![rule(&["bot"], &["public.*"], "allow")];
    let acl = ACL::new(rules, "deny", None);
    // check() with ctx=None should still match when there are no conditions
    let result = acl.check(Some("bot"), "public.info", None);
    assert!(result);
}

// ---------------------------------------------------------------------------
// ACL.check() — deny rule matches
// ---------------------------------------------------------------------------

#[test]
fn test_check_deny_rule_matches() {
    let rules = vec![described(
        &["guest"],
        &["admin.*"],
        "deny",
        "Guests cannot access admin",
    )];
    let acl = ACL::new(rules, "allow", None);
    let ctx = make_ctx("guest", "user", vec![]);
    let result = acl.check(Some("guest"), "admin.panel", Some(&ctx));
    assert!(!result, "Guest should be denied access to admin.*");
}

// ---------------------------------------------------------------------------
// ACL.check() — default effect when no rules match
// ---------------------------------------------------------------------------

#[test]
fn test_check_default_deny_when_no_rules_match() {
    let rules = vec![rule(&["admin"], &["admin.*"], "allow")];
    let acl = ACL::new(rules, "deny", None);
    // "user1" does not match the "admin" caller pattern
    let result = acl.check(Some("user1"), "admin.panel", None);
    assert!(!result, "Should fall through to default deny");
}

#[test]
fn test_check_default_allow_when_no_rules_match() {
    let rules = vec![rule(&["blocked"], &["*"], "deny")];
    let acl = ACL::new(rules, "allow", None);
    // "friendly" does not match "blocked"
    let result = acl.check(Some("friendly"), "anything", None);
    assert!(result, "Should fall through to default allow");
}

#[test]
fn test_check_default_effect_with_empty_rules() {
    let acl_deny = ACL::new(vec![], "deny", None);
    assert!(!acl_deny.check(Some("anyone"), "anything", None));

    let acl_allow = ACL::new(vec![], "allow", None);
    assert!(acl_allow.check(Some("anyone"), "anything", None));
}

// ---------------------------------------------------------------------------
// ACL.check() — wildcard pattern matching
// ---------------------------------------------------------------------------

#[test]
fn test_check_wildcard_target_matches_all() {
    let rules = vec![rule(&["superadmin"], &["*"], "allow")];
    let acl = ACL::new(rules, "deny", None);
    assert!(acl.check(Some("superadmin"), "any.module.here", None));
    assert!(acl.check(Some("superadmin"), "another", None));
}

#[test]
fn test_check_wildcard_caller_matches_all() {
    let rules = vec![rule(&["*"], &["public.health"], "allow")];
    let acl = ACL::new(rules, "deny", None);
    assert!(acl.check(Some("anyone"), "public.health", None));
    assert!(acl.check(Some("someone_else"), "public.health", None));
}

#[test]
fn test_check_glob_pattern_in_target() {
    let rules = vec![rule(&["svc"], &["data.*"], "allow")];
    let acl = ACL::new(rules, "deny", None);
    assert!(acl.check(Some("svc"), "data.read", None));
    assert!(acl.check(Some("svc"), "data.write", None));
    assert!(
        !acl.check(Some("svc"), "admin.read", None),
        "Should not match non-data targets"
    );
}

#[test]
fn test_check_none_caller_maps_to_external() {
    let rules = vec![rule(&["@external"], &["public.*"], "allow")];
    let acl = ACL::new(rules, "deny", None);
    // None caller should be treated as @external
    assert!(acl.check(None, "public.api", None));
    // Explicit non-@external caller should not match
    assert!(!acl.check(Some("user1"), "public.api", None));
}

// ---------------------------------------------------------------------------
// ACL.check() — first-match-wins ordering
// ---------------------------------------------------------------------------

#[test]
fn test_check_first_match_wins_allow_before_deny() {
    let rules = vec![
        described(&["user"], &["resource"], "allow", "Allow first"),
        described(&["user"], &["resource"], "deny", "Deny second"),
    ];
    let acl = ACL::new(rules, "deny", None);
    let result = acl.check(Some("user"), "resource", None);
    assert!(result, "First matching rule (allow) should win");
}

#[test]
fn test_check_first_match_wins_deny_before_allow() {
    let rules = vec![
        described(&["user"], &["resource"], "deny", "Deny first"),
        described(&["user"], &["resource"], "allow", "Allow second"),
    ];
    let acl = ACL::new(rules, "allow", None);
    let result = acl.check(Some("user"), "resource", None);
    assert!(!result, "First matching rule (deny) should win");
}

#[test]
fn test_check_first_match_skips_non_matching_rules() {
    let rules = vec![
        described(&["other"], &["resource"], "deny", "Does not match caller"),
        described(&["user"], &["resource"], "allow", "Matches"),
    ];
    let acl = ACL::new(rules, "deny", None);
    let result = acl.check(Some("user"), "resource", None);
    assert!(
        result,
        "Should skip non-matching first rule and match second"
    );
}

#[test]
fn test_check_add_rule_inserts_at_front() {
    let mut acl = ACL::new(
        vec![described(
            &["user"],
            &["resource"],
            "allow",
            "Original allow",
        )],
        "deny",
        None,
    );

    // add_rule inserts at position 0 — this deny rule should now be first
    acl.add_rule(described(&["user"], &["resource"], "deny", "Added deny"));

    let result = acl.check(Some("user"), "resource", None);
    assert!(!result, "Newly added deny rule at front should win");
}

// ---------------------------------------------------------------------------
// A-D-302: ACL::new validates default_effect
// ---------------------------------------------------------------------------

#[test]
// The panic now carries `try_new`'s own message, which names the offending
// value (§6.1.5): an `.expect()` string could not, and said "default_effect"
// for rule-level rejections too.
#[should_panic(expected = "Invalid default_effect 'wrong_value'")]
fn test_acl_new_panics_on_invalid_default_effect() {
    // ACL::new must validate default_effect — bogus value should panic,
    // matching apcore-python and apcore-typescript constructor-throws
    // behaviour (sync finding A-D-302).
    let _ = ACL::new(vec![], "wrong_value", None);
}

#[test]
fn test_acl_new_accepts_allow_and_deny() {
    // Both legal values must construct successfully without panic.
    let _ = ACL::new(vec![], "allow", None);
    let _ = ACL::new(vec![], "deny", None);
}

#[test]
fn test_acl_load_propagates_invalid_default_effect_as_result() {
    // load() must propagate validation failures via Result rather than
    // panicking — YAML errors must not crash the host.
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
    writeln!(tmp, "default_effect: not_a_real_effect\nrules: []\n").expect("write tempfile");
    let path = tmp.path().to_str().expect("utf8 path").to_string();
    let result = ACL::load(&path);
    assert!(
        result.is_err(),
        "load should error on invalid default_effect"
    );
}

// `schemas/acl-config.schema.json` declares `default_effect` as a plain
// `allow` / `deny` string enum, not nullable — an explicit `default_effect:
// null` is a type violation, not a synonym for an omitted key. Only omission
// may default to `deny`; a written `null` must be refused like any other
// non-string value.
#[test]
fn test_acl_load_rejects_explicit_null_default_effect() {
    use apcore::errors::ErrorCode;
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
    writeln!(tmp, "default_effect: null\nrules: []\n").expect("write tempfile");
    let path = tmp.path().to_str().expect("utf8 path").to_string();
    let err = ACL::load(&path).expect_err("load must reject an explicit null default_effect");
    assert_eq!(
        err.code,
        ErrorCode::ACLRuleError,
        "explicit null default_effect must be ACLRuleError, got {:?}",
        err.code
    );
    assert!(
        err.message.contains("default_effect"),
        "error must name default_effect: {}",
        err.message
    );
}

// Regression: sync finding A-D-022 — structural ACL parse/validation
// errors carry `ErrorCode::ACLRuleError` per spec contract (apcore-python
// and apcore-typescript both raise `ACLRuleError`). Previously Rust used
// `ErrorCode::ConfigInvalid`, which broke cross-language fixtures
// asserting on the error code.
#[test]
fn test_acl_load_uses_acl_rule_error_for_parse_failures() {
    use apcore::errors::ErrorCode;
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
    // Malformed YAML: stray colon in a scalar context
    writeln!(tmp, "rules: : :\n").expect("write tempfile");
    let path = tmp.path().to_str().expect("utf8 path").to_string();
    let err = ACL::load(&path).expect_err("load must error on malformed YAML");
    assert_eq!(
        err.code,
        ErrorCode::ACLRuleError,
        "structural ACL parse errors must surface as ACLRuleError, got {:?}",
        err.code
    );
}

#[test]
fn test_acl_load_uses_acl_rule_error_for_missing_rules_key() {
    use apcore::errors::ErrorCode;
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
    // Valid YAML but no `rules` key.
    writeln!(tmp, "default_effect: deny\n").expect("write tempfile");
    let path = tmp.path().to_str().expect("utf8 path").to_string();
    let err = ACL::load(&path).expect_err("load must error on missing rules key");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
}

// A-D-09: a rule that omits `callers` (or `targets`) MUST be rejected at load
// with ACL_RULE_ERROR, not silently loaded with an empty list (which would
// make a deny rule inert).
#[test]
fn test_acl_load_rejects_rule_missing_callers() {
    use apcore::errors::ErrorCode;
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
    // Rule omits the `callers` key entirely.
    writeln!(
        tmp,
        "default_effect: allow\nrules:\n  - targets: [\"secret.*\"]\n    effect: deny\n"
    )
    .expect("write tempfile");
    let path = tmp.path().to_str().expect("utf8 path").to_string();
    let err = ACL::load(&path).expect_err("load must reject rule missing 'callers'");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
}

// ---------------------------------------------------------------------------
// A-D-303: ACL::reload doesn't deadlock (borrow scope released before file IO)
// ---------------------------------------------------------------------------

#[test]
fn test_acl_reload_succeeds_from_yaml_path() {
    // Smoke test that reload() picks up changes to the file. The structural
    // requirement for A-D-303 is that the borrow of self.yaml_path ends
    // before Self::load is called; this test ensures the public behavior
    // works end-to-end.
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
    writeln!(
        tmp,
        "default_effect: deny\nrules:\n  - callers: [\"user\"]\n    targets: [\"r\"]\n    effect: allow\n"
    )
    .expect("write tempfile");
    let path = tmp.path().to_str().expect("utf8").to_string();

    let mut acl = ACL::load(&path).expect("initial load");
    assert!(acl.check(Some("user"), "r", None));

    // Replace file: now deny everything.
    std::fs::write(
        &path,
        "default_effect: deny\nrules:\n  - callers: [\"user\"]\n    targets: [\"r\"]\n    effect: deny\n",
    )
    .expect("rewrite tempfile");

    acl.reload().expect("reload");
    assert!(!acl.check(Some("user"), "r", None));
}

// ---------------------------------------------------------------------------
// D10-005: ACL::add_rule has unit return type (no Result wrapper)
// ---------------------------------------------------------------------------

#[test]
fn test_acl_add_rule_returns_unit_no_result_wrapper() {
    // The body of add_rule is infallible — vec.insert(0, _) cannot fail.
    // Spec contract acl-system.md:259 declares On success: None, so the
    // return type must be unit, not Result<(), ModuleError>. Callers
    // should not need `?`/`.unwrap()` to use it.
    let mut acl = ACL::new(vec![], "deny", None);
    let rule = rule(&["caller"], &["target"], "allow");
    // The next line would not compile if add_rule returned Result<(), _>
    // because that requires `?` or explicit handling — the bare statement
    // form proves the type is unit.
    let _: () = acl.add_rule(rule);
    assert_eq!(acl.rules().len(), 1);
}

// ---------------------------------------------------------------------------
// D10-001: ACL::reload without yaml_path raises ACLRuleError with spec message
// ---------------------------------------------------------------------------

#[test]
fn test_acl_reload_without_yaml_path_raises_acl_rule_error() {
    use apcore::errors::ErrorCode;
    let mut acl = ACL::new(vec![], "deny", None);
    let err = acl
        .reload()
        .expect_err("reload without yaml_path must error");
    assert_eq!(
        err.code,
        ErrorCode::ACLRuleError,
        "code must be ACLRuleError to match Python/TS spec contract"
    );
    assert_eq!(
        err.message, "Cannot reload: ACL was not loaded from a YAML file",
        "message must match spec acl-system.md:314 verbatim"
    );
}

// ---------------------------------------------------------------------------
// A-D-301: async_check snapshots rules + default_effect at entry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_async_check_uses_snapshot_consistent_with_sync() {
    // async_check must snapshot rules + default_effect at entry, mirroring
    // the sync check() snapshot. This test exercises the basic
    // first-match-wins behaviour through async_check to verify the
    // snapshot path produces the same decisions.
    let rules = vec![
        described(&["user"], &["resource"], "deny", "first deny"),
        described(&["user"], &["resource"], "allow", "second allow"),
    ];
    let acl = ACL::new(rules, "deny", None);
    let r = acl.async_check(Some("user"), "resource", None).await;
    assert!(!r, "First-match deny should win in async_check");
}

#[tokio::test]
async fn test_async_check_no_rules_path() {
    // No-rules path through async_check should also use the snapshotted
    // default_effect.
    let acl = ACL::new(vec![], "allow", None);
    let r = acl.async_check(Some("user"), "resource", None).await;
    assert!(r);

    let acl_deny = ACL::new(vec![], "deny", None);
    let r2 = acl_deny.async_check(Some("user"), "resource", None).await;
    assert!(!r2);
}
