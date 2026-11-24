//! Drive `acl_root_discovery.json` — config-driven ACL discovery
//! (D-64 Recommendation A, issue #74).
//!
//! `tests/test_acl_root_discovery.rs` covers four of these ten cases by hand.
//! The invariant that matters most is the one a hand copy is likeliest to drop:
//! a missing `acl.root` attaches NOTHING and MUST NOT synthesize an empty
//! default-deny ACL, which would silently deny every inter-module call in every
//! project without an `acl/` directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use apcore::acl::ACL;
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::executor::Executor;
use apcore::module::Module;
use apcore::registry::{ModuleDescriptor, Registry};
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::conformance_env::find_fixtures_root;

fn fixture() -> Value {
    let path = find_fixtures_root().join("acl_root_discovery.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("acl_root_discovery.json parses")
}

// ---------------------------------------------------------------------------
// Fixtures on disk
// ---------------------------------------------------------------------------

struct EchoModule;

#[async_trait]
impl Module for EchoModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Echo a canned result"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"ok": true}))
    }
}

fn register_echo(registry: &Registry, module_id: &str) {
    let module = EchoModule;
    let descriptor = ModuleDescriptor {
        module_id: module_id.to_string(),
        name: None,
        description: module.description().to_string(),
        documentation: None,
        input_schema: module.input_schema(),
        output_schema: module.output_schema(),
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
        .register(module_id, Box::new(module), descriptor)
        .expect("register module");
}

/// A complete, spec-valid legacy-mode config at `<dir>/apcore.config.json`.
/// `acl.root` is omitted entirely when the case sets `acl_root_unset`, which is
/// the point of the first two cases.
fn write_config(dir: &Path, acl_root: Option<&str>, default_effect: &str) -> PathBuf {
    let mut acl = serde_json::Map::new();
    if let Some(root) = acl_root {
        acl.insert("root".to_string(), json!(root));
    }
    acl.insert("default_effect".to_string(), json!(default_effect));
    let config = json!({
        "version": "0.26.0",
        "project": { "name": "acl-root-discovery-conformance" },
        "extensions": { "root": "./extensions" },
        "schema": { "root": "./schemas" },
        "acl": Value::Object(acl),
        "executor": {
            "default_timeout": 30000,
            "global_timeout": 60000,
            "max_call_depth": 32,
            "max_module_repeat": 3
        }
    });
    let path = dir.join("apcore.config.json");
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap())
        .expect("write config file");
    path
}

/// Materialise the case's `fs` block under `dir`. Values are either the literal
/// string `"acl_policy"` (write the fixture's shared policy as YAML there) or
/// `"directory"` (create the directory and nothing else).
fn materialise_fs(dir: &Path, fs: &Value, acl_policy: &Value, id: &str) {
    for (rel, kind) in fs.as_object().unwrap_or(&serde_json::Map::new()) {
        let target = dir.join(rel.trim_end_matches('/'));
        match kind.as_str().expect("fs value is a string") {
            "acl_policy" => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).expect("create parent dir");
                }
                let yaml = serde_yaml_ng::to_string(acl_policy).expect("policy serializes to YAML");
                std::fs::write(&target, yaml).expect("write acl policy");
            }
            "directory" => std::fs::create_dir_all(&target).expect("create dir"),
            other => panic!(
                "[{id}] acl_root_discovery.json grew fs kind `{other}` that this driver \
                 cannot materialise — teach the driver, do not skip it"
            ),
        }
    }
}

/// Register both a policy-allowed and a policy-denied module, then report
/// whether enforcement is actually active on this client.
async fn observe_enforcement(client: &APCore) -> bool {
    register_echo(client.registry(), "greet");
    register_echo(client.registry(), "db.write");

    // `greet` is allowed to @external by the fixture policy; `db.write` is not.
    let allowed = client.call("greet", json!({}), None, None).await;
    let denied = client.call("db.write", json!({}), None, None).await;

    match (&allowed, &denied) {
        // Enforcement active: the allowed call goes through, the other is blocked.
        (Ok(_), Err(e)) if e.code == ErrorCode::ACLDenied => true,
        // No ACL attached: both calls run.
        (Ok(_), Ok(_)) => false,
        _ => panic!(
            "unexpected enforcement shape: greet={allowed:?} db.write={denied:?}\n\
             Neither 'ACL enforced' nor 'no ACL' matches this outcome."
        ),
    }
}

#[tokio::test]
async fn conformance_acl_root_discovery() {
    let fx = fixture();
    let acl_policy = fx["acl_policy"].clone();
    let default_acl_root = fx["default_acl_root"]
        .as_str()
        .expect("fixture states default_acl_root");
    let cases = fx["test_cases"].as_array().expect("test_cases is an array");
    assert_eq!(cases.len(), 10, "driver is written against all 10 cases");

    for tc in cases {
        let id = tc["id"].as_str().expect("every case needs an id");

        let tmp = tempfile::tempdir().expect("tempdir");
        let acl_root = tc["acl_root"].as_str();
        let unset = tc["acl_root_unset"].as_bool().unwrap_or(false);
        assert!(
            unset || acl_root.is_some(),
            "[{id}] case states neither acl_root nor acl_root_unset"
        );
        let default_effect = tc["default_effect"].as_str().unwrap_or("deny");
        let config_path = write_config(
            tmp.path(),
            if unset { None } else { acl_root },
            default_effect,
        );
        materialise_fs(tmp.path(), &tc["fs"], &acl_policy, id);

        let config = Config::load(&config_path).expect("config loads");
        let caller_supplied_executor = tc["caller_supplied_executor"].as_bool().unwrap_or(false);

        let expected = tc["expected"]
            .as_object()
            .unwrap_or_else(|| panic!("[{id}] case has no expected object"));

        for (field, want) in expected {
            match field.as_str() {
                "resolved_acl_root" => {
                    let resolved = config
                        .get("acl.root")
                        .unwrap_or_else(|| panic!("[{id}] acl.root resolved to None"));
                    assert_eq!(&resolved, want, "[{id}] resolved_acl_root");
                    assert_eq!(
                        want.as_str(),
                        Some(default_acl_root),
                        "[{id}] the fixture's default_acl_root and this case disagree"
                    );
                }
                "config_valid" => {
                    assert_eq!(
                        config.validate().is_ok(),
                        want.as_bool().expect("config_valid is a bool"),
                        "[{id}] config_valid: {:?}",
                        config.validate()
                    );
                }
                "acl_attached" => {
                    let want_attached = want.as_bool().expect("acl_attached is a bool");
                    if caller_supplied_executor {
                        // Discovery MUST be skipped, so the observable claim is
                        // that the client the caller built has no ACL wired.
                        // `ACL::discover` alone would still find the file — that
                        // is exactly what must NOT be attached.
                        let registry = std::sync::Arc::new(Registry::new());
                        let executor = Executor::new(registry, std::sync::Arc::new(config.clone()));
                        let client =
                            APCore::with_options(None, Some(executor), Some(config.clone()), None);
                        assert_eq!(
                            observe_enforcement(&client).await,
                            want_attached,
                            "[{id}] a caller-supplied Executor must not gain a discovered ACL"
                        );
                    } else {
                        let discovered =
                            ACL::discover(&config).expect("discover must not error here");
                        assert_eq!(
                            discovered.is_some(),
                            want_attached,
                            "[{id}] acl_attached (acl.root={:?})",
                            config.get("acl.root")
                        );
                    }
                }
                "enforcement" => {
                    let want_enforced = want.as_bool().expect("enforcement is a bool");
                    let client = if caller_supplied_executor {
                        let registry = std::sync::Arc::new(Registry::new());
                        let executor = Executor::new(registry, std::sync::Arc::new(config.clone()));
                        APCore::with_options(None, Some(executor), Some(config.clone()), None)
                    } else {
                        APCore::from_path(&config_path).expect("client from config")
                    };
                    assert_eq!(
                        observe_enforcement(&client).await,
                        want_enforced,
                        "[{id}] enforcement"
                    );
                }
                "decision" => {
                    let acl = ACL::discover(&config)
                        .expect("discover ok")
                        .unwrap_or_else(|| panic!("[{id}] expected an ACL to evaluate"));
                    let caller = tc["caller_id"].as_str();
                    let target = tc["target_id"]
                        .as_str()
                        .unwrap_or_else(|| panic!("[{id}] decision case needs a target_id"));
                    assert_eq!(
                        acl.check(caller, target, None),
                        want.as_bool().expect("decision is a bool"),
                        "[{id}] decision for caller={caller:?} target={target}"
                    );
                }
                other => panic!(
                    "[{id}] acl_root_discovery.json grew expectation `{other}` that this \
                     driver does not check — teach the driver, do not skip it"
                ),
            }
        }
    }
}
