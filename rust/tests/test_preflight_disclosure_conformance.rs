//! Cross-language conformance test for the preflight disclosure gate
//! (PROTOCOL_SPEC §12.8.5.1).
//!
//! Consumes the canonical `preflight_disclosure.json` fixture shipped by the
//! `apcore` spec repo (sibling directory or `CONFORMANCE_SPEC_REPO`).
//!
//! `Executor::validate` MUST NOT disclose module-level introspection to a caller
//! the ACL denied. `preflight()` and `preview()` are module-authored code whose
//! output names what the call would do — the resolved binary and argv of a
//! command-wrapping module, the target of a write. Module lookup is Step 3 and
//! the ACL check is Step 4, so gating those hooks on "lookup succeeded" alone
//! runs them for a denied caller and returns what they said.
//!
//! Per the fixture's `driver_contract` this drives the real `Executor::validate`
//! against a real `Registry` and a real `ACL`: the defect lives in `validate`'s
//! own gating, so a driver that assembles a `PreflightResult` itself asserts
//! nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use apcore::context::{Context, Identity};
use apcore::executor::Executor;
use apcore::module::{Change, Module, PreviewResult};
use apcore::registry::{ModuleDescriptor, Registry};
use apcore::{ACLRule, ACL};

use crate::conformance_env::find_fixtures_root;

fn load_fixture(name: &str) -> Value {
    let path = find_fixtures_root().join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"))
}

/// The fixture's `module_contract`, with an invocation recorder attached.
///
/// `hooks` is observed inside the hook bodies rather than inferred from the
/// absent check entries: an implementation that calls the hooks and then drops
/// their results still ran module code for a denied caller, which is the
/// side-effect half of the requirement.
struct DestructiveModule {
    input_schema: Value,
    output_schema: Value,
    preflight_returns: Vec<String>,
    preview_change: Change,
    hooks: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl Module for DestructiveModule {
    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }
    fn output_schema(&self) -> Value {
        self.output_schema.clone()
    }
    fn description(&self) -> &str {
        "Conformance module for the preflight disclosure gate"
    }
    async fn execute(
        &self,
        _inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Value, apcore::errors::ModuleError> {
        panic!("validate() must never execute the module body")
    }
    fn preflight(&self, _inputs: &Value, _ctx: Option<&Context<Value>>) -> Vec<String> {
        self.hooks.lock().unwrap().push("preflight".to_string());
        self.preflight_returns.clone()
    }
    fn preview(&self, _inputs: &Value, _ctx: Option<&Context<Value>>) -> Option<PreviewResult> {
        self.hooks.lock().unwrap().push("preview".to_string());
        let mut result = PreviewResult::default();
        result.changes = vec![self.preview_change.clone()];
        Some(result)
    }
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("expected an array")
        .iter()
        .map(|v| v.as_str().expect("expected a string").to_string())
        .collect()
}

#[tokio::test]
async fn conformance_preflight_disclosure() {
    let fixture = load_fixture("preflight_disclosure");
    let contract = &fixture["module_contract"];
    let module_id = contract["module_id"].as_str().expect("module_id");
    let sentinel = contract["sentinel"].as_str().expect("sentinel");
    let preview_change: Change = serde_json::from_value(contract["preview_change"].clone())
        .expect("preview_change must deserialize into a Change");

    let cases = fixture["test_cases"]
        .as_array()
        .expect("fixture must carry a test_cases array");
    assert!(!cases.is_empty(), "fixture must carry at least one case");

    let mut failures: Vec<String> = Vec::new();

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");
        let spec = &tc["input"];
        let expected = &tc["expected"];

        let hooks = Arc::new(Mutex::new(Vec::<String>::new()));
        let registry = Arc::new(Registry::default());
        let descriptor = ModuleDescriptor {
            module_id: module_id.to_string(),
            name: None,
            description: "Conformance module for the preflight disclosure gate".to_string(),
            documentation: None,
            input_schema: contract["input_schema"].clone(),
            output_schema: contract["output_schema"].clone(),
            version: "1.0.0".to_string(),
            tags: vec![],
            annotations: None,
            examples: vec![],
            metadata: HashMap::new(),
            display: None,
            sunset_date: None,
            dependencies: vec![],
            enabled: true,
        };
        registry
            .register(
                module_id,
                Box::new(DestructiveModule {
                    input_schema: contract["input_schema"].clone(),
                    output_schema: contract["output_schema"].clone(),
                    preflight_returns: string_list(&contract["preflight_returns"]),
                    preview_change: preview_change.clone(),
                    hooks: Arc::clone(&hooks),
                }),
                descriptor,
            )
            .expect("register the conformance module");

        let rules: Vec<ACLRule> = spec["acl_rules"]
            .as_array()
            .expect("acl_rules")
            .iter()
            .map(|r| {
                let mut rule = ACLRule::new(
                    string_list(&r["callers"]),
                    string_list(&r["targets"]),
                    r["effect"].as_str().expect("effect").to_string(),
                );
                rule.description = Some("conformance rule".to_string());
                rule
            })
            .collect();
        let acl = ACL::new(
            rules,
            spec["default_effect"].as_str().expect("default_effect"),
            None,
        );

        let mut executor = Executor::new(registry, Arc::new(apcore::config::Config::default()));
        executor.set_acl(acl);

        let ctx = Context::<Value>::new(Identity::new(
            spec["caller_id"].as_str().expect("caller_id").to_string(),
            "module".to_string(),
            vec![],
            HashMap::new(),
        ));

        let result = executor
            .validate(module_id, &spec["inputs"], Some(&ctx))
            .await
            .expect("validate must return a structured result");

        let names: Vec<&str> = result.checks.iter().map(|c| c.check.as_str()).collect();
        let mut record = |msg: String| {
            failures.push(format!(
                "[{id}] {msg}\n  checks: {:?}\n  hooks: {:?}",
                result
                    .checks
                    .iter()
                    .map(|c| (c.check.as_str(), c.passed))
                    .collect::<Vec<_>>(),
                hooks.lock().unwrap()
            ));
        };

        let want_valid = expected["valid"].as_bool().expect("expected.valid");
        if result.valid != want_valid {
            record(format!(
                "valid mismatch: got {}, expected {want_valid}",
                result.valid
            ));
        }

        for name in string_list(&expected["checks_present"]) {
            if !names.contains(&name.as_str()) {
                record(format!("check '{name}' MUST be present"));
            }
        }

        // Absence is asserted on the check list itself: a present-but-empty
        // `module_preflight` entry is already the disclosure that the module
        // implements the hook.
        for name in string_list(&expected["checks_absent"]) {
            if names.contains(&name.as_str()) {
                record(format!("check '{name}' MUST NOT be present"));
            }
        }

        let mut got_failed: Vec<&str> = result
            .checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.check.as_str())
            .collect();
        got_failed.sort_unstable();
        let mut want_failed = string_list(&expected["failed_checks"]);
        want_failed.sort();
        if got_failed != want_failed {
            record(format!(
                "failed-check set mismatch: got {got_failed:?}, expected {want_failed:?}"
            ));
        }

        let want_changes = expected["predicted_changes_count"]
            .as_u64()
            .expect("predicted_changes_count") as usize;
        if result.predicted_changes.len() != want_changes {
            record(format!(
                "predicted_changes count mismatch: got {}, expected {want_changes}",
                result.predicted_changes.len()
            ));
        }

        let got_hooks = hooks.lock().unwrap().clone();
        let want_hooks = string_list(&expected["hooks_invoked"]);
        if got_hooks != want_hooks {
            record(format!(
                "module hook invocation mismatch — the hooks themselves must not run: \
                 got {got_hooks:?}, expected {want_hooks:?}"
            ));
        }

        // The sentinel appears in no value the Executor computes on its own, so
        // finding it anywhere in the serialized result proves introspection
        // reached the caller.
        let serialized = serde_json::to_string(&json!({
            "checks": result.checks,
            "predicted_changes": result.predicted_changes,
        }))
        .expect("PreflightResult must serialize");
        let leaked = serialized.contains(sentinel);
        let want_absent = expected["sentinel_absent"]
            .as_bool()
            .expect("sentinel_absent");
        if want_absent && leaked {
            record(format!(
                "sentinel '{sentinel}' leaked to a denied caller: {serialized}"
            ));
        } else if !want_absent && !leaked {
            record(
                "control case: the sentinel MUST reach a permitted caller, otherwise the denial \
                 cases pass for an implementation that never introspects at all"
                    .to_string(),
            );
        }
    }

    assert!(
        failures.is_empty(),
        "preflight_disclosure conformance failures ({}/{}):\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
