//! Integration tests for config-driven ACL discovery (D-64, Recommendation A;
//! issue #74).
//!
//! `acl.root` is now an *activated* config key: at bootstrap the client resolves
//! it (default `"./acl"`), and when the resolved path holds an ACL the executor
//! enforces it. Crucially, a missing path is a NO-OP — it must NOT synthesize an
//! empty default-deny ACL, which would silently deny every inter-module call in
//! every project lacking an `acl` dir.
//!
//! Covered:
//! - present `acl` dir with `global_acl.yaml` -> `ACL::discover` returns `Some`
//!   AND a call denied by that ACL is actually blocked (`ACLDenied`).
//! - missing `acl` dir -> `ACL::discover` returns `None` AND calls are NOT
//!   blocked (no enforcement, identical to pre-D-64 behavior).
//! - a config omitting `acl.root` is VALID and `get("acl.root")` resolves to the
//!   default `"./acl"`.

use std::collections::HashMap;
use std::io::Write;

use apcore::acl::ACL;
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::registry::{ModuleDescriptor, Registry};
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A trivial module that echoes a canned result.
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
        .unwrap();
}

/// Write a complete, spec-valid legacy-mode JSON config that sets `acl.root` to
/// `acl_root`. Returns the config file path. The config lives at the root of
/// `dir`, so relative `acl.root` resolves against `dir` via `source_path`.
fn write_config(dir: &std::path::Path, acl_root: &str) -> std::path::PathBuf {
    let config = json!({
        "version": "0.24.0",
        "project": { "name": "acl-discovery-test" },
        "extensions": { "root": "./extensions" },
        "schema": { "root": "./schemas" },
        "acl": { "root": acl_root, "default_effect": "deny" },
        "executor": {
            "default_timeout": 30000,
            "global_timeout": 60000,
            "max_call_depth": 32,
            "max_module_repeat": 3
        }
    });
    let path = dir.join("apcore.config.json");
    let mut file = std::fs::File::create(&path).expect("create config file");
    file.write_all(serde_json::to_string_pretty(&config).unwrap().as_bytes())
        .expect("write config file");
    path
}

/// Write a default-deny ACL at `<dir>/global_acl.yaml` whose only rule denies
/// `@external` from reaching `secure.module`. With `default_effect: deny`,
/// `@external` is denied regardless, but the explicit rule documents intent and
/// proves the loaded file's rules are honored.
fn write_global_acl(dir: &std::path::Path) {
    let yaml = "default_effect: deny\n\
                rules:\n\
                \x20 - callers: [\"@external\"]\n\
                \x20   targets: [\"secure.module\"]\n\
                \x20   effect: deny\n\
                \x20   description: \"External callers cannot reach secure.module.\"\n";
    let mut file = std::fs::File::create(dir.join("global_acl.yaml")).expect("create acl file");
    file.write_all(yaml.as_bytes()).expect("write acl file");
}

// ---------------------------------------------------------------------------
// Present dir + ACL file -> discover Some AND enforcement active
// ---------------------------------------------------------------------------

#[tokio::test]
async fn present_acl_dir_discovers_and_enforces() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let acl_dir = tmp.path().join("acl");
    std::fs::create_dir(&acl_dir).expect("create acl dir");
    write_global_acl(&acl_dir);

    let config_path = write_config(tmp.path(), "./acl");
    let config = Config::load(&config_path).expect("config loads and validates");

    // Direct discover() returns Some.
    let discovered = ACL::discover(&config).expect("discover ok");
    assert!(
        discovered.is_some(),
        "present acl dir with global_acl.yaml must discover an ACL"
    );

    // End-to-end: the client wires the discovered ACL into the executor, so an
    // external call to secure.module is blocked.
    let client = APCore::from_path(&config_path).expect("client from config");
    register_echo(client.registry(), "secure.module");

    let result = client.call("secure.module", json!({}), None, None).await;
    assert!(
        result.is_err(),
        "discovered default-deny ACL must block the external call, got {result:?}"
    );
    assert_eq!(
        result.unwrap_err().code,
        ErrorCode::ACLDenied,
        "blocked call must surface ACLDenied"
    );
}

// ---------------------------------------------------------------------------
// Missing dir -> discover None AND no enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_acl_dir_discovers_none_and_does_not_enforce() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    // Note: we intentionally do NOT create the acl dir.
    let config_path = write_config(tmp.path(), "./acl");
    let config = Config::load(&config_path).expect("config loads and validates");

    // Direct discover() returns None — the missing-path no-op invariant.
    let discovered = ACL::discover(&config).expect("discover ok");
    assert!(
        discovered.is_none(),
        "missing acl dir must yield None (no synthesized default-deny ACL)"
    );

    // End-to-end: with no ACL wired, the external call succeeds.
    let client = APCore::from_path(&config_path).expect("client from config");
    register_echo(client.registry(), "open.module");

    let result = client.call("open.module", json!({}), None, None).await;
    assert!(
        result.is_ok(),
        "missing acl dir must mean no enforcement; call should succeed, got {result:?}"
    );
    assert_eq!(result.unwrap(), json!({"ok": true}));
}

// ---------------------------------------------------------------------------
// Present dir but no conventional global_acl.yaml -> None (no-op still holds)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn present_dir_without_global_acl_yaml_discovers_none() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    std::fs::create_dir(tmp.path().join("acl")).expect("create empty acl dir");
    let config_path = write_config(tmp.path(), "./acl");
    let config = Config::load(&config_path).expect("config loads and validates");

    let discovered = ACL::discover(&config).expect("discover ok");
    assert!(
        discovered.is_none(),
        "acl dir present but missing global_acl.yaml must yield None"
    );
}

// ---------------------------------------------------------------------------
// Config omitting acl.root is VALID and gets the default "./acl"
// ---------------------------------------------------------------------------

#[test]
fn config_omitting_acl_root_is_valid_with_default() {
    let config = json!({
        "version": "0.24.0",
        "project": { "name": "no-acl-root" },
        "extensions": { "root": "./extensions" },
        "schema": { "root": "./schemas" },
        "acl": { "default_effect": "deny" },
        "executor": {
            "default_timeout": 30000,
            "global_timeout": 60000,
            "max_call_depth": 32,
            "max_module_repeat": 3
        }
    });
    let cfg: Config = serde_json::from_value(config).expect("config without acl.root deserializes");
    assert!(
        cfg.validate().is_ok(),
        "a config omitting acl.root must be VALID: {:?}",
        cfg.validate()
    );
    assert_eq!(
        cfg.get("acl.root"),
        Some(json!("./acl")),
        "omitted acl.root must resolve to the default \"./acl\""
    );
}
