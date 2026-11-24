// Spec-traced contract tests for the apcore-rust acl-system feature.
//
// Source spec: apcore/docs/features/acl-system.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_acl_system_spec.py
//
// Each test maps to exactly one clause in the feature spec's '## Contract:'
// blocks. The verbatim cross-language clause id appears in a leading
// `// clause: <clause_id>` comment on the line above each test fn so a
// cross-language diff tool can line up the Python / TypeScript / Rust rows by
// that exact string. The fn name is the clause id flattened to snake_case.
//
// Contract blocks covered:
//   - ACL.check
//   - ACL.load
//   - ACL.add_rule
//   - ACL.remove_rule
//   - ACL.reload
//
// TESTS ONLY — never modifies production source.

use std::sync::{Arc, Mutex};

use apcore::acl::{ACLRule, AuditEntry, ACL};
use apcore::errors::ErrorCode;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an ACLRule with no description / conditions.
fn rule(callers: &[&str], targets: &[&str], effect: &str) -> ACLRule {
    ACLRule::new(
        callers.iter().map(|s| (*s).to_string()).collect(),
        targets.iter().map(|s| (*s).to_string()).collect(),
        effect,
    )
}

const VALID_YAML: &str = r#"version: "1.0"
default_effect: deny
rules:
  - callers: ["api.*"]
    targets: ["db.*"]
    effect: allow
    description: "API to DB"
"#;

/// Write `body` to a uniquely-named temp file and return its absolute path.
fn write_yaml(body: &str, tag: &str) -> String {
    let mut dir = std::env::temp_dir();
    let unique = format!(
        "apcore_acl_spec_{}_{}_{}.yaml",
        tag,
        std::process::id(),
        // nanoseconds give per-call uniqueness within a process
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    dir.push(unique);
    std::fs::write(&dir, body).expect("write temp yaml");
    dir.to_string_lossy().into_owned()
}

/// Serialized wire string for an ErrorCode (e.g. "ACL_RULE_ERROR").
fn code_str(code: ErrorCode) -> String {
    match serde_json::to_value(code) {
        Ok(serde_json::Value::String(s)) => s,
        other => panic!("error code did not serialize to string: {other:?}"),
    }
}

// ===========================================================================
// Contract: ACL.check
// ===========================================================================

// clause: acl_system.check.property.async
#[test]
fn test_acl_system_check_property_async_is_sync() {
    // check() is declared async: false (plain bool, not a Result/Future).
    let acl = ACL::new(vec![rule(&["*"], &["*"], "allow")], "deny", None);
    let result: bool = acl.check(Some("api.x"), "db.y", None);
    assert!(result, "wildcard allow rule must permit the call");
}

// clause: acl_system.check.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_acl_system_check_property_thread_safe_concurrent() {
    // N>=8 concurrent checks, no panic, consistent state.
    let acl = Arc::new(ACL::new(
        vec![rule(&["api.*"], &["db.*"], "allow")],
        "deny",
        None,
    ));

    let mut handles = Vec::new();
    for i in 0..16 {
        let acl = Arc::clone(&acl);
        handles.push(tokio::spawn(async move {
            acl.check(Some(&format!("api.{i}")), "db.read", None)
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.expect("task must not panic"));
    }

    assert_eq!(results.len(), 16);
    assert!(results.iter().all(|r| *r), "every concurrent check allows");
    // Final state unchanged: rule list still intact.
    assert!(acl.check(Some("api.gateway"), "db.read", None));
    assert!(!acl.check(Some("other"), "db.read", None));
}

// clause: acl_system.check.property.idempotent
#[test]
fn test_acl_system_check_property_idempotent() {
    // Identical inputs yield identical decisions.
    let acl = ACL::new(vec![rule(&["api.*"], &["db.*"], "allow")], "deny", None);
    let first = acl.check(Some("api.gateway"), "db.query", None);
    let second = acl.check(Some("api.gateway"), "db.query", None);
    assert!(first);
    assert_eq!(first, second);
    // Deny path is equally stable.
    assert_eq!(
        acl.check(Some("nope"), "db.query", None),
        acl.check(Some("nope"), "db.query", None)
    );
    assert!(!acl.check(Some("nope"), "db.query", None));
}

// clause: acl_system.check.property.pure
#[test]
fn test_acl_system_check_property_pure_no_self_mutation() {
    // pure:false BUT must not mutate rule list / default via public query.
    let acl = ACL::new(
        vec![
            rule(&["api.*"], &["db.*"], "allow"),
            rule(&["*"], &["*"], "deny"),
        ],
        "deny",
        None,
    );
    let before_allow = acl.check(Some("api.gateway"), "db.read", None);
    let before_deny = acl.check(Some("evil"), "secret", None);
    acl.check(Some("api.gateway"), "db.read", None);
    acl.check(Some("evil"), "secret", None);
    // Decisions stable across repeated calls — no observable self-mutation.
    assert_eq!(
        acl.check(Some("api.gateway"), "db.read", None),
        before_allow
    );
    assert_eq!(acl.check(Some("evil"), "secret", None), before_deny);
    assert!(before_allow);
    assert!(!before_deny);
}

// clause: acl_system.check.side_effect.4.evaluate_first_match_wins
#[test]
fn test_acl_system_check_side_effect_4_first_match_wins() {
    // First matching rule decides, despite a later contradicting rule.
    let acl = ACL::new(
        vec![rule(&["*"], &["*"], "allow"), rule(&["*"], &["*"], "deny")],
        "deny",
        None,
    );
    assert!(
        acl.check(Some("anyone"), "anything", None),
        "first allow rule wins over later deny"
    );
}

// clause: acl_system.check.side_effect.5.emit_audit_event
#[test]
fn test_acl_system_check_side_effect_5_emit_audit_event() {
    // Audit event emitted carrying the decision.
    let captured: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let mut acl = ACL::new(vec![rule(&["api.*"], &["db.*"], "allow")], "deny", None);
    acl.set_audit_logger(move |entry: &AuditEntry| {
        sink.lock().unwrap().push(entry.clone());
    });

    acl.check(Some("api.gateway"), "db.read", None);

    let entries = captured.lock().unwrap();
    assert_eq!(entries.len(), 1, "exactly one audit event per check");
    let entry = &entries[0];
    assert_eq!(entry.decision, "allow");
    assert_eq!(entry.caller_id, "api.gateway");
    assert_eq!(entry.target_id, "db.read");
}

// clause: acl_system.check.input.caller_id.none_maps_to_external
#[test]
fn test_acl_system_check_input_caller_id_none_maps_external() {
    // None caller => @external; a real caller_id must NOT match @external.
    let acl = ACL::new(
        vec![rule(&["@external"], &["public.*"], "allow")],
        "deny",
        None,
    );
    assert!(acl.check(None, "public.docs", None));
    assert!(!acl.check(Some("api.handler"), "public.docs", None));
}

// clause: acl_system.check.error.no_raise_returns_false
#[test]
fn test_acl_system_check_error_no_raise_returns_false() {
    // Deny is a `false` return, never an error — check() returns plain bool.
    let acl = ACL::new(vec![], "deny", None);
    let result: bool = acl.check(Some("api.x"), "db.y", None);
    assert!(!result, "no rules + default deny must return false");
}

// ===========================================================================
// Contract: ACL.load
// ===========================================================================

// clause: acl_system.load.input.yaml_path.file_must_exist
#[test]
fn test_acl_system_load_input_yaml_path_missing_file() {
    // Missing file rejects with ConfigNotFound + exact code string.
    let missing = format!(
        "{}/apcore_acl_spec_does_not_exist_{}.yaml",
        std::env::temp_dir().to_string_lossy(),
        std::process::id()
    );
    let err = ACL::load(&missing).expect_err("missing file must error");
    assert_eq!(err.code, ErrorCode::ConfigNotFound);
    assert_eq!(code_str(err.code), "CONFIG_NOT_FOUND");
}

// clause: acl_system.load.error.CONFIG_NOT_FOUND
#[test]
fn test_acl_system_load_error_config_not_found() {
    // Nonexistent path => ConfigNotFound + code.
    let missing = format!(
        "{}/apcore_acl_spec_nope_{}.yaml",
        std::env::temp_dir().to_string_lossy(),
        std::process::id()
    );
    let err = ACL::load(&missing).expect_err("nonexistent path must error");
    assert_eq!(err.code, ErrorCode::ConfigNotFound);
    assert_eq!(code_str(err.code), "CONFIG_NOT_FOUND");
}

// clause: acl_system.load.error.ACL_RULE_ERROR.not_a_mapping
#[test]
fn test_acl_system_load_error_acl_rule_error_not_mapping() {
    // Top-level non-mapping (a YAML list) => ACLRuleError.
    let path = write_yaml("- just\n- a\n- list\n", "not_mapping");
    let err = ACL::load(&path).expect_err("non-mapping top level must error");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert_eq!(code_str(err.code), "ACL_RULE_ERROR");
}

// clause: acl_system.load.error.ACL_RULE_ERROR.rules_key_absent
#[test]
fn test_acl_system_load_error_acl_rule_error_rules_absent() {
    // Missing 'rules' key => ACLRuleError.
    let path = write_yaml("version: \"1.0\"\ndefault_effect: deny\n", "rules_absent");
    let err = ACL::load(&path).expect_err("missing rules key must error");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert_eq!(code_str(err.code), "ACL_RULE_ERROR");
}

// clause: acl_system.load.error.ACL_RULE_ERROR.rules_not_list
#[test]
fn test_acl_system_load_error_acl_rule_error_rules_not_list() {
    // 'rules' value is a mapping not a list => ACLRuleError.
    let path = write_yaml("rules:\n  foo: bar\n", "rules_not_list");
    let err = ACL::load(&path).expect_err("non-list rules must error");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert_eq!(code_str(err.code), "ACL_RULE_ERROR");
}

// clause: acl_system.load.error.ACL_RULE_ERROR.rule_missing_required_key
#[test]
fn test_acl_system_load_error_acl_rule_error_missing_key() {
    // Rule missing required 'effect' key => ACLRuleError.
    let path = write_yaml(
        "rules:\n  - callers: [\"a.*\"]\n    targets: [\"b.*\"]\n",
        "missing_key",
    );
    let err = ACL::load(&path).expect_err("missing effect must error");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert_eq!(code_str(err.code), "ACL_RULE_ERROR");
}

// clause: acl_system.load.error.ACL_RULE_ERROR.invalid_effect
#[test]
fn test_acl_system_load_error_acl_rule_error_invalid_effect() {
    // A per-rule effect that is not allow/deny => ACLRuleError, matching
    // apcore-python `acl.py` and apcore-typescript `acl.ts`. The default_effect
    // is valid here, so the error can only originate from per-rule validation.
    let path = write_yaml(
        "default_effect: deny\nrules:\n  - callers: [\"a.*\"]\n    targets: [\"b.*\"]\n    effect: maybe\n",
        "invalid_effect",
    );
    let err = ACL::load(&path).expect_err("invalid per-rule effect must error");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert_eq!(code_str(err.code), "ACL_RULE_ERROR");
}

// clause: acl_system.load.error.ACL_RULE_ERROR.callers_not_list
#[test]
fn test_acl_system_load_error_acl_rule_error_callers_not_list() {
    // callers value is a scalar not a list => ACLRuleError.
    let path = write_yaml(
        "rules:\n  - callers: \"a.*\"\n    targets: [\"b.*\"]\n    effect: allow\n",
        "callers_not_list",
    );
    let err = ACL::load(&path).expect_err("non-list callers must error");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert_eq!(code_str(err.code), "ACL_RULE_ERROR");
}

// clause: acl_system.load.side_effect.4.set_yaml_path
#[test]
fn test_acl_system_load_side_effect_4_sets_yaml_path() {
    // Returned instance has its yaml_path set (enables reload). Rust has no public
    // _yaml_path accessor; observe via reload() succeeding (proves the path is wired).
    let path = write_yaml(VALID_YAML, "sets_path");
    let mut acl = ACL::load(&path).expect("load valid yaml");
    acl.reload()
        .expect("reload must succeed when path was stored by load");
}

// clause: acl_system.load.postcondition.default_effect_deny
#[test]
fn test_acl_system_load_postcondition_default_effect_deny() {
    // Absent default_effect => deny.
    let path = write_yaml(
        "rules:\n  - callers: [\"a.*\"]\n    targets: [\"b.*\"]\n    effect: allow\n",
        "default_deny",
    );
    let acl = ACL::load(&path).expect("load");
    // No rule matches "x" -> falls to default; must be deny.
    assert!(!acl.check(Some("x"), "y", None));
}

// clause: acl_system.load.postcondition.rules_order_preserved
#[test]
fn test_acl_system_load_postcondition_rules_order_preserved() {
    // Rules keep YAML order (first-match-wins): the first allow rule wins.
    let path = write_yaml(
        "default_effect: deny\nrules:\n  - callers: [\"*\"]\n    targets: [\"*\"]\n    effect: allow\n  - callers: [\"*\"]\n    targets: [\"*\"]\n    effect: deny\n",
        "order_preserved",
    );
    let acl = ACL::load(&path).expect("load");
    assert!(acl.check(Some("anyone"), "anything", None));
}

// clause: acl_system.load.property.async
#[test]
fn test_acl_system_load_property_async_is_sync() {
    // load() is declared async: false — returns a value, not a Future.
    let path = write_yaml(VALID_YAML, "load_sync");
    let acl: ACL = ACL::load(&path).expect("load");
    assert!(acl.check(Some("api.x"), "db.y", None));
}

// clause: acl_system.load.property.idempotent
#[test]
fn test_acl_system_load_property_idempotent() {
    // Same file content => equivalent instances (equivalent observable behavior).
    let path = write_yaml(VALID_YAML, "load_idem");
    let a = ACL::load(&path).expect("load a");
    let b = ACL::load(&path).expect("load b");
    assert_eq!(
        a.check(Some("api.x"), "db.y", None),
        b.check(Some("api.x"), "db.y", None)
    );
    assert!(a.check(Some("api.x"), "db.y", None));
    assert_eq!(a.check(Some("x"), "y", None), b.check(Some("x"), "y", None));
    assert!(!a.check(Some("x"), "y", None));
}

// clause: acl_system.load.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_acl_system_load_property_thread_safe() {
    // Concurrent loads create independent instances.
    let path = write_yaml(VALID_YAML, "load_threadsafe");

    let mut handles = Vec::new();
    for _ in 0..8 {
        let path = path.clone();
        handles.push(tokio::spawn(async move { ACL::load(&path) }));
    }

    let mut instances = Vec::new();
    for h in handles {
        instances.push(h.await.expect("task must not panic").expect("load ok"));
    }

    assert_eq!(instances.len(), 8);
    assert!(instances
        .iter()
        .all(|acl| acl.check(Some("api.x"), "db.y", None)));
}

// ===========================================================================
// Contract: ACL.add_rule
// ===========================================================================

// clause: acl_system.add_rule.side_effect.2.insert_at_index_0
#[test]
fn test_acl_system_add_rule_side_effect_2_insert_front() {
    // New rule evaluated before existing ones (inserted at index 0).
    let mut acl = ACL::new(vec![rule(&["*"], &["*"], "deny")], "deny", None);
    assert!(!acl.check(Some("admin.root"), "anything", None));
    acl.add_rule(rule(&["admin.*"], &["*"], "allow"));
    // New rule sits at index 0 => evaluated first => allow.
    assert!(acl.check(Some("admin.root"), "anything", None));
}

// clause: acl_system.add_rule.postcondition.shifts_prior_rules
#[test]
fn test_acl_system_add_rule_postcondition_shifts_prior() {
    // Prior rules shift up; new rule is first => latest add wins via first-match.
    let mut acl = ACL::new(vec![], "deny", None);
    acl.add_rule(rule(&["*"], &["*"], "deny"));
    acl.add_rule(rule(&["*"], &["*"], "allow"));
    assert!(acl.check(Some("x"), "y", None));
}

// clause: acl_system.add_rule.property.idempotent
#[test]
fn test_acl_system_add_rule_property_not_idempotent() {
    // Declared false: two identical calls add two rules.
    let mut acl = ACL::new(vec![], "deny", None);
    let r = rule(&["a.*"], &["b.*"], "allow");
    acl.add_rule(r.clone());
    acl.add_rule(r);
    let callers = vec!["a.*".to_string()];
    let targets = vec!["b.*".to_string()];
    // Removing once still leaves a matching rule (the second copy).
    assert!(acl.remove_rule(&callers, &targets));
    assert!(acl.check(Some("a.x"), "b.y", None));
    assert!(acl.remove_rule(&callers, &targets));
    assert!(!acl.check(Some("a.x"), "b.y", None));
}

// clause: acl_system.add_rule.property.async
#[test]
fn test_acl_system_add_rule_property_async_is_sync() {
    // add_rule() is declared async: false, returns unit (Rust analogue of None).
    let mut acl = ACL::new(vec![], "deny", None);
    let result: () = acl.add_rule(rule(&["a.*"], &["b.*"], "allow"));
    assert_eq!(result, ());
    assert!(acl.check(Some("a.x"), "b.y", None));
}

// clause: acl_system.add_rule.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_acl_system_add_rule_property_thread_safe() {
    // N>=8 concurrent inserts via an Arc<Mutex<ACL>> wrapper, list not corrupted.
    let acl = Arc::new(Mutex::new(ACL::new(vec![], "deny", None)));

    let mut handles = Vec::new();
    for i in 0..16 {
        let acl = Arc::clone(&acl);
        handles.push(tokio::spawn(async move {
            acl.lock()
                .unwrap()
                .add_rule(rule(&[&format!("svc.{i}")], &["t.*"], "allow"));
        }));
    }
    for h in handles {
        h.await.expect("task must not panic");
    }

    let guard = acl.lock().unwrap();
    for i in 0..16 {
        assert!(
            guard.check(Some(&format!("svc.{i}")), "t.x", None),
            "rule svc.{i} must be present and matchable"
        );
    }
    assert!(!guard.check(Some("svc.absent"), "t.x", None));
}

// clause: acl_system.add_rule.error.value_error_kwargs_path
// MISSING SYMBOL: the kwargs/no-arg add_rule() ValueError path is Python-only
// (spec D10-006: "kwargs path is therefore not normative for cross-language
// conformance"). Rust callers always pass a prebuilt ACLRule, so there is no
// fallible add_rule surface to exercise.
#[test]
#[ignore = "acl_system.add_rule.error.value_error_kwargs_path: missing symbol kwargs-form add_rule() (Python-only per spec D10-006; Rust uses prebuilt ACLRule) (contract gap)"]
fn test_acl_system_add_rule_error_value_error_kwargs() {
    unreachable!("Python-only kwargs path; no Rust equivalent");
}

// ===========================================================================
// Contract: ACL.remove_rule
// ===========================================================================

// clause: acl_system.remove_rule.side_effect.2.find_first_match
#[test]
fn test_acl_system_remove_rule_side_effect_2_first_match() {
    // Removes by exact callers/targets equality; sibling rule intact.
    let mut acl = ACL::new(
        vec![
            rule(&["a.*"], &["b.*"], "allow"),
            rule(&["c.*"], &["d.*"], "allow"),
        ],
        "deny",
        None,
    );
    assert!(acl.remove_rule(&["a.*".to_string()], &["b.*".to_string()]));
    assert!(!acl.check(Some("a.x"), "b.y", None));
    assert!(acl.check(Some("c.x"), "d.y", None));
}

// clause: acl_system.remove_rule.return.true_when_found
#[test]
fn test_acl_system_remove_rule_return_true_when_found() {
    // Returns true when a matching rule is removed.
    let mut acl = ACL::new(vec![rule(&["a.*"], &["b.*"], "allow")], "deny", None);
    assert!(acl.remove_rule(&["a.*".to_string()], &["b.*".to_string()]));
}

// clause: acl_system.remove_rule.return.false_when_absent
#[test]
fn test_acl_system_remove_rule_return_false_when_absent() {
    // Returns false when no rule matches.
    let mut acl = ACL::new(vec![rule(&["a.*"], &["b.*"], "allow")], "deny", None);
    assert!(!acl.remove_rule(&["nope.*".to_string()], &["nope.*".to_string()]));
}

// clause: acl_system.remove_rule.postcondition.at_most_one_removed
#[test]
fn test_acl_system_remove_rule_postcondition_at_most_one() {
    // Only first match removed per call.
    let mut acl = ACL::new(
        vec![
            rule(&["a.*"], &["b.*"], "allow"),
            rule(&["a.*"], &["b.*"], "allow"),
        ],
        "deny",
        None,
    );
    assert!(acl.remove_rule(&["a.*".to_string()], &["b.*".to_string()]));
    // One duplicate remains.
    assert!(acl.check(Some("a.x"), "b.y", None));
    assert!(acl.remove_rule(&["a.*".to_string()], &["b.*".to_string()]));
    assert!(!acl.check(Some("a.x"), "b.y", None));
}

// clause: acl_system.remove_rule.property.idempotent
#[test]
fn test_acl_system_remove_rule_property_not_idempotent() {
    // Declared false: first True, second False.
    let mut acl = ACL::new(vec![rule(&["a.*"], &["b.*"], "allow")], "deny", None);
    let first = acl.remove_rule(&["a.*".to_string()], &["b.*".to_string()]);
    let second = acl.remove_rule(&["a.*".to_string()], &["b.*".to_string()]);
    assert!(first);
    assert!(!second);
}

// clause: acl_system.remove_rule.property.async
#[test]
fn test_acl_system_remove_rule_property_async_is_sync() {
    // remove_rule() is declared async: false — returns plain bool.
    let mut acl = ACL::new(vec![rule(&["a.*"], &["b.*"], "allow")], "deny", None);
    let result: bool = acl.remove_rule(&["a.*".to_string()], &["b.*".to_string()]);
    assert!(result);
}

// clause: acl_system.remove_rule.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_acl_system_remove_rule_property_thread_safe() {
    // N>=8 concurrent removals via Arc<Mutex<ACL>>, no corruption.
    let rules: Vec<ACLRule> = (0..16)
        .map(|i| rule(&[&format!("svc.{i}")], &["t.*"], "allow"))
        .collect();
    let acl = Arc::new(Mutex::new(ACL::new(rules, "deny", None)));

    let mut handles = Vec::new();
    for i in 0..16 {
        let acl = Arc::clone(&acl);
        handles.push(tokio::spawn(async move {
            acl.lock()
                .unwrap()
                .remove_rule(&[format!("svc.{i}")], &["t.*".to_string()])
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        results.push(h.await.expect("task must not panic"));
    }
    assert!(results.iter().all(|r| *r), "every removal found its rule");

    let guard = acl.lock().unwrap();
    for i in 0..16 {
        assert!(!guard.check(Some(&format!("svc.{i}")), "t.x", None));
    }
}

// ===========================================================================
// Contract: ACL.reload
// ===========================================================================

// clause: acl_system.reload.precondition.requires_yaml_path
#[test]
fn test_acl_system_reload_precondition_no_yaml_path() {
    // reload on a non-loaded ACL => ACLRuleError.
    let mut acl = ACL::new(vec![], "deny", None);
    let err = acl
        .reload()
        .expect_err("reload without stored path must error");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert_eq!(code_str(err.code), "ACL_RULE_ERROR");
}

// clause: acl_system.reload.error.ACL_RULE_ERROR.not_loaded_from_yaml
#[test]
fn test_acl_system_reload_error_acl_rule_error_not_loaded() {
    // No stored path => ACLRuleError + code.
    let mut acl = ACL::new(vec![rule(&["*"], &["*"], "allow")], "deny", None);
    let err = acl.reload().expect_err("reload on non-yaml ACL must error");
    assert_eq!(err.code, ErrorCode::ACLRuleError);
    assert_eq!(code_str(err.code), "ACL_RULE_ERROR");
}

// clause: acl_system.reload.error.CONFIG_NOT_FOUND.file_removed
#[test]
fn test_acl_system_reload_error_config_not_found() {
    // File deleted after load => ConfigNotFound on reload.
    let path = write_yaml(VALID_YAML, "reload_removed");
    let mut acl = ACL::load(&path).expect("load");
    std::fs::remove_file(&path).expect("remove file");
    let err = acl.reload().expect_err("reload of removed file must error");
    assert_eq!(err.code, ErrorCode::ConfigNotFound);
    assert_eq!(code_str(err.code), "CONFIG_NOT_FOUND");
}

// clause: acl_system.reload.postcondition.rules_reflect_file
#[test]
fn test_acl_system_reload_postcondition_rules_reflect_file() {
    // reload re-reads YAML and updates rules.
    let path = write_yaml(
        "default_effect: deny\nrules:\n  - callers: [\"a.*\"]\n    targets: [\"b.*\"]\n    effect: allow\n",
        "reload_reflect",
    );
    let mut acl = ACL::load(&path).expect("load");
    assert!(acl.check(Some("a.x"), "b.y", None));
    // Rewrite the same file with a different ruleset.
    std::fs::write(
        &path,
        "default_effect: deny\nrules:\n  - callers: [\"c.*\"]\n    targets: [\"d.*\"]\n    effect: allow\n",
    )
    .expect("rewrite");
    acl.reload().expect("reload");
    assert!(!acl.check(Some("a.x"), "b.y", None), "old rule gone");
    assert!(acl.check(Some("c.x"), "d.y", None), "new rule active");
}

// clause: acl_system.reload.postcondition.discards_runtime_mutations
#[test]
fn test_acl_system_reload_postcondition_discards_mutations() {
    // add_rule before reload is discarded (file content wins).
    let path = write_yaml(VALID_YAML, "reload_discard");
    let mut acl = ACL::load(&path).expect("load");
    acl.add_rule(rule(&["runtime.*"], &["*"], "allow"));
    assert!(acl.check(Some("runtime.x"), "anything", None));
    acl.reload().expect("reload");
    assert!(!acl.check(Some("runtime.x"), "anything", None));
}

// clause: acl_system.reload.property.async
#[test]
fn test_acl_system_reload_property_async_is_sync() {
    // reload() is declared async: false, returns Ok(()) (Rust analogue of None).
    let path = write_yaml(VALID_YAML, "reload_sync");
    let mut acl = ACL::load(&path).expect("load");
    let result: () = acl.reload().expect("reload");
    assert_eq!(result, ());
}

// clause: acl_system.reload.property.idempotent
#[test]
fn test_acl_system_reload_property_idempotent() {
    // Same file content => same rule list across reloads.
    let path = write_yaml(VALID_YAML, "reload_idem");
    let mut acl = ACL::load(&path).expect("load");
    acl.reload().expect("reload 1");
    let first_allow = acl.check(Some("api.x"), "db.y", None);
    let first_deny = acl.check(Some("x"), "y", None);
    acl.reload().expect("reload 2");
    assert_eq!(acl.check(Some("api.x"), "db.y", None), first_allow);
    assert!(first_allow);
    assert_eq!(acl.check(Some("x"), "y", None), first_deny);
    assert!(!first_deny);
}

// clause: acl_system.reload.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_acl_system_reload_property_thread_safe() {
    // Concurrent reload + check through an Arc<RwLock<ACL>> wrapper, no corruption.
    use std::sync::RwLock;
    let path = write_yaml(VALID_YAML, "reload_threadsafe");
    let acl = Arc::new(RwLock::new(ACL::load(&path).expect("load")));

    let mut handles = Vec::new();
    for i in 0..8 {
        let acl_reload = Arc::clone(&acl);
        handles.push(tokio::spawn(async move {
            acl_reload.write().unwrap().reload().expect("reload");
            let _ = i;
        }));
        let acl_check = Arc::clone(&acl);
        handles.push(tokio::spawn(async move {
            let _ = acl_check.read().unwrap().check(Some("api.x"), "db.y", None);
        }));
    }
    for h in handles {
        h.await.expect("task must not panic");
    }

    // Final state consistent: the file's allow rule still applies.
    assert!(acl.read().unwrap().check(Some("api.x"), "db.y", None));
}

// ---------------------------------------------------------------------------
// §6.5 warning discipline (D-88)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct WarnCapture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for WarnCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for WarnCapture {
    type Writer = Self;
    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

/// Run `f` with a capturing subscriber installed and return the log text.
fn capture_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
    let buf = WarnCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let bytes = buf
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    (out, String::from_utf8_lossy(&bytes).into_owned())
}

fn missing_context_warnings(logs: &str) -> usize {
    logs.lines()
        .filter(|l| l.contains("has conditions but the check supplied no context"))
        .count()
}

/// A conditional `deny` rule that can only be evaluated with a context.
fn conditional_deny(target: &str) -> ACLRule {
    let mut r = rule(&["*"], &[target], "deny");
    r.conditions = Some(serde_json::json!({ "roles": ["admin"] }));
    r
}

/// A conditional `deny` rule matching EVERY target, distinguished only by the
/// role it names.
///
/// The index-collision shape D-88 is about needs every rule evaluated on one
/// `check`, so that removing rule 0 leaves the survivor sitting on an index a
/// marker was already recorded for. Rules with distinct targets do not collide
/// — the survivor lands on an index nobody warned about and warns whether or
/// not the dedupe was cleared, which is a test that reads as coverage and
/// proves nothing.
fn conditional_deny_role(role: &str) -> ACLRule {
    let mut r = rule(&["*"], &["*"], "deny");
    r.conditions = Some(serde_json::json!({ "roles": [role] }));
    r
}

// clause: acl_system.check.warning.conditions_without_context_is_deduped
#[test]
fn check_warning_conditions_without_context_is_deduped() {
    // §6.5 + D-88: the warning fires, and fires ONCE per (rule index, effect)
    // — not once per check, which would be log spam proportional to traffic.
    let acl = ACL::new(vec![conditional_deny("db.users")], "allow", None);

    let (_, logs) = capture_logs(|| {
        for _ in 0..5 {
            assert!(acl.check(Some("api.x"), "db.users", None));
        }
    });
    assert_eq!(
        missing_context_warnings(&logs),
        1,
        "the §6.5 warning must be deduped per rule index: {logs}"
    );
}

// clause: acl_system.add_rule.side_effect.clears_missing_context_dedupe
#[test]
fn add_rule_clears_missing_context_dedupe() {
    // D-88: `add_rule` inserts at index 0 and shifts every existing rule, so a
    // retained marker would suppress the warning for a DIFFERENT rule — and
    // specifically for the rule the operator just added.
    let mut acl = ACL::new(vec![conditional_deny("db.users")], "allow", None);

    let (_, first) = capture_logs(|| {
        let _ = acl.check(Some("api.x"), "db.users", None);
    });
    assert_eq!(missing_context_warnings(&first), 1);

    let (_, suppressed) = capture_logs(|| {
        let _ = acl.check(Some("api.x"), "db.users", None);
    });
    assert_eq!(
        missing_context_warnings(&suppressed),
        0,
        "still the same rule at the same index"
    );

    acl.add_rule(conditional_deny("db.orders"));
    let (_, after_insert) = capture_logs(|| {
        // The NEW rule is now index 0, and the original has shifted to 1.
        let _ = acl.check(Some("api.x"), "db.orders", None);
        let _ = acl.check(Some("api.x"), "db.users", None);
    });
    assert_eq!(
        missing_context_warnings(&after_insert),
        2,
        "add_rule must clear the index-keyed dedupe: {after_insert}"
    );
}

// clause: acl_system.remove_rule.side_effect.clears_missing_context_dedupe
#[test]
fn remove_rule_clears_missing_context_dedupe() {
    // D-88: removing rule `i` shifts every rule after it up by one.
    //
    // This test used to use two rules with DIFFERENT targets, so one `check`
    // evaluated one of them and the survivor moved onto an index nobody had
    // warned about. It passed with the clear removed — the right name, the
    // right comment, and no discriminating power. Both rules match every
    // target now, so one `check` marks index 0 AND index 1, and the survivor
    // inherits a marked index.
    let mut acl = ACL::new(
        vec![
            conditional_deny_role("admin"),
            conditional_deny_role("operator"),
        ],
        "allow",
        None,
    );

    let (_, first) = capture_logs(|| {
        let _ = acl.check(Some("api.x"), "db.users", None);
    });
    assert_eq!(
        missing_context_warnings(&first),
        2,
        "both rules warn: {first}"
    );

    assert!(acl.remove_rule_with_conditions(
        &["*".to_string()],
        &["*".to_string()],
        Some(&serde_json::json!({ "roles": ["admin"] })),
    ));

    let (_, after_remove) = capture_logs(|| {
        let _ = acl.check(Some("api.x"), "db.users", None);
    });
    assert_eq!(
        missing_context_warnings(&after_remove),
        1,
        "the survivor moved from index 1 to index 0; without the clear the \
         REMOVED rule's marker silences it: {after_remove}"
    );
}

// clause: acl_system.remove_rule.side_effect.failed_remove_keeps_dedupe
#[test]
fn a_remove_that_removed_nothing_does_not_have_to_clear() {
    // Separates "clears correctly" from "clears unconditionally": the latter
    // re-warns on every failed lookup, which is the spam D-88 exists to bound.
    let mut acl = ACL::new(vec![conditional_deny_role("admin")], "allow", None);

    let (_, first) = capture_logs(|| {
        let _ = acl.check(Some("api.x"), "db.users", None);
    });
    assert_eq!(missing_context_warnings(&first), 1);

    assert!(!acl.remove_rule(&["nobody".to_string()], &["*".to_string()]));

    let (_, after) = capture_logs(|| {
        let _ = acl.check(Some("api.x"), "db.users", None);
    });
    assert_eq!(
        missing_context_warnings(&after),
        0,
        "no rule removed means no index shifted: {after}"
    );
}
