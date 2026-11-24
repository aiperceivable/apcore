//! Cross-language driver for `acl_rule_key_closure.json`.
//!
//! PROTOCOL_SPEC §6.1 (spec v1.27.0, #107): ACL rule keys are a closed set, and
//! a rule carrying anything else fails to load. A key nothing evaluates was
//! dropped in silence before this, which widens an `allow` rule with no warning
//! — the §6.1.1 defect class on the pattern side rather than the condition side.
//!
//! The fixture lands in the spec repo one push after this driver, so that
//! `check_driver_coverage.py --strict` has a driver to find for it. Until then
//! the test skips and names the unexercised fixture — "not verified", never
//! "passed".

use apcore::acl::ACL;
use serde_json::Value;

use crate::conformance_env::find_fixtures_root;

const FIXTURE: &str = "acl_rule_key_closure.json";

/// Emit the case's rule as one entry under `rules:`. The cases are flat maps of
/// scalars, string arrays and one nested `conditions` map.
fn rule_to_yaml(rule: &serde_json::Map<String, Value>) -> String {
    let mut out = String::new();
    for (k, v) in rule {
        match v {
            Value::Object(inner) => {
                out.push_str(&format!("    {k}:\n"));
                for (ik, iv) in inner {
                    out.push_str(&format!("      {ik}: {iv}\n"));
                }
            }
            _ => out.push_str(&format!("    {k}: {v}\n")),
        }
    }
    out
}

#[test]
fn acl_rule_key_closure_conformance() {
    let path = find_fixtures_root().join(FIXTURE);
    if !path.is_file() {
        eprintln!("SKIP: {FIXTURE} not in the spec repo yet (spec v1.27.0, #107) — not verified");
        return;
    }
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
            .expect("parse fixture");

    let closed: Vec<&str> = fixture["closed_rule_keys"]
        .as_array()
        .expect("closed_rule_keys")
        .iter()
        .map(|v| v.as_str().expect("string"))
        .collect();

    let dir = std::env::temp_dir().join("apcore_acl_rule_key_closure");
    std::fs::create_dir_all(&dir).expect("tmp dir");

    let cases = fixture["test_cases"].as_array().expect("test_cases");
    for tc in cases {
        let id = tc["id"].as_str().expect("id");
        let note = tc["note"].as_str().unwrap_or("");
        let rule = tc["rule"].as_object().expect("rule");
        let yaml = format!(
            "default_effect: {}\nrules:\n  -\n{}",
            tc["default_effect"].as_str().expect("default_effect"),
            rule_to_yaml(rule)
        );
        let file = dir.join(format!("{id}.yaml"));
        std::fs::write(&file, yaml).expect("write yaml");

        let result = ACL::load(file.to_str().expect("utf8"));
        match tc["expected_load"].as_str().expect("expected_load") {
            "ok" => {
                let acl = result
                    .unwrap_or_else(|e| panic!("[{id}] expected load to succeed: {e:?}\n  {note}"));
                assert_eq!(acl.rules().len(), 1, "[{id}] {note}");
            }
            _ => {
                let err = result.err().unwrap_or_else(|| {
                    panic!("[{id}] expected load to be refused\n  {note}");
                });
                for key in rule.keys().filter(|k| !closed.contains(&k.as_str())) {
                    assert!(
                        err.message.contains(key),
                        "[{id}] refusal did not name '{key}': {}\n  {note}",
                        err.message
                    );
                }
            }
        }
    }
    println!("acl_rule_key_closure: {} case(s) executed", cases.len());
}
