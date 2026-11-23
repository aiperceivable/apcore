//! Drive `acl_audit_delivery.json` — §6.3.2 (#118 D-66).
//!
//! Every case loads a real ACL file and drives real `check()` calls. Reading the
//! parsed block back off a config object would prove the parser works, which was
//! never the question: what had no contract at all was **delivery**.

use std::sync::{Arc, Mutex, PoisonError};

use apcore::acl::{AuditEntry, ACL, AUDIT_EVENT_NAME};
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;
use crate::test_acl_audit_delivery::capture;

fn fixture() -> Value {
    let path = find_fixtures_root().join("acl_audit_delivery.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("acl_audit_delivery.json parses")
}

fn audit_lines(logs: &str) -> Vec<&str> {
    logs.lines()
        .filter(|l| l.contains(AUDIT_EVENT_NAME))
        .collect()
}

fn other_lines(logs: &str) -> Vec<&str> {
    logs.lines()
        .filter(|l| !l.contains(AUDIT_EVENT_NAME))
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)] // one arm per fixture expectation; splitting hides the mapping
fn conformance_acl_audit_delivery() {
    let fx = fixture();
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert!(!cases.is_empty(), "the fixture drove nothing");

    for case in cases {
        let id = case["id"].as_str().expect("case id");
        let input = &case["input"];
        let expected = &case["expected"];

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("acl.yaml");
        std::fs::write(
            &path,
            serde_yaml_ng::to_string(&input["acl_file"]).expect("yaml"),
        )
        .expect("write");
        let path_str = path.to_string_lossy().into_owned();

        let collected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let kind = input["callback"].as_str().expect("callback");

        if expected["loads"].as_bool() == Some(false) {
            let err = ACL::load(&path_str).expect_err(&format!("case {id} must be rejected"));
            assert_eq!(
                err.code.wire_str(),
                expected["error_code"].as_str().expect("error_code"),
                "case {id}"
            );
            let needle = expected["error_message_contains"].as_str().expect("needle");
            assert!(err.message.contains(needle), "case {id}: {}", err.message);
            continue;
        }

        let sink = Arc::clone(&collected);
        let (decisions, logs) = capture(|| {
            let mut acl = ACL::load(&path_str).expect("loads");
            match kind {
                "collecting" => acl.set_audit_logger(move |e: &AuditEntry| {
                    sink.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(e.decision.clone());
                }),
                "failing" => {
                    acl.set_audit_logger(|_e: &AuditEntry| panic!("the audit sink is down"));
                }
                _ => {}
            }
            input["checks"]
                .as_array()
                .expect("checks")
                .iter()
                .map(|c| {
                    let allowed = acl.check(
                        c["caller_id"].as_str(),
                        c["target_id"].as_str().expect("target_id"),
                        None,
                    );
                    if allowed { "allow" } else { "deny" }.to_string()
                })
                .collect::<Vec<_>>()
        });

        if let Some(want) = expected.get("decisions").and_then(Value::as_array) {
            let want: Vec<String> = want
                .iter()
                .map(|v| v.as_str().expect("decision").to_string())
                .collect();
            assert_eq!(decisions, want, "case {id}");
        }
        if let Some(want) = expected.get("callback_decisions").and_then(Value::as_array) {
            let want: Vec<String> = want
                .iter()
                .map(|v| v.as_str().expect("decision").to_string())
                .collect();
            assert_eq!(
                *collected.lock().unwrap_or_else(PoisonError::into_inner),
                want,
                "case {id}"
            );
        }

        let audit = audit_lines(&logs);
        if let Some(want) = expected.get("default_sink_records").and_then(Value::as_u64) {
            assert_eq!(audit.len() as u64, want, "case {id}, logs:\n{logs}");
        }
        if let Some(names) = expected
            .get("default_sink_field_names")
            .and_then(Value::as_array)
        {
            for name in names {
                let name = name.as_str().expect("field name");
                assert!(
                    audit[0].contains(&format!("{name}=")),
                    "case {id}: {name} missing from\n{}",
                    audit[0]
                );
            }
        }
        if let Some(level) = expected.get("default_sink_level").and_then(Value::as_str) {
            let marker = level.to_uppercase();
            assert!(audit[0].contains(&marker), "case {id}: {}", audit[0]);
        }
        if let Some(want) = expected
            .get("default_sink_decisions")
            .and_then(Value::as_array)
        {
            let got: Vec<&str> = audit
                .iter()
                .map(|l| {
                    if l.contains("decision=allow") {
                        "allow"
                    } else {
                        "deny"
                    }
                })
                .collect();
            let want: Vec<&str> = want.iter().map(|v| v.as_str().expect("d")).collect();
            assert_eq!(got, want, "case {id}");
        }

        let others = other_lines(&logs).join("\n");
        if let Some(needle) = expected
            .get("load_warning_contains")
            .and_then(Value::as_str)
        {
            assert!(others.contains(needle), "case {id}, logs:\n{others}");
        }
        if let Some(needle) = expected.get("load_warning_absent").and_then(Value::as_str) {
            assert!(!others.contains(needle), "case {id}, logs:\n{others}");
        }
        if let Some(fields) = expected
            .get("override_warning_names")
            .and_then(Value::as_array)
        {
            assert_eq!(
                others.matches("does not apply").count(),
                1,
                "case {id}, logs:\n{others}"
            );
            for field in fields {
                let field = field.as_str().expect("field");
                assert!(others.contains(field), "case {id}: {field} missing");
            }
        }
        if let Some(want) = expected
            .get("delivery_failure_reports")
            .and_then(Value::as_u64)
        {
            assert_eq!(
                others.matches("audit delivery failed").count() as u64,
                want,
                "case {id}, logs:\n{others}"
            );
        }
    }
}
