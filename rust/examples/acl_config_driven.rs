//! Config-driven ACL discovery — `acl.root` end-to-end demo (D-64, issue #74).
//!
//! This is the CONFIG-DRIVEN path. Contrast it with `acl_agent_governance.rs`,
//! which wires a policy MANUALLY with `executor.set_acl(acl)`. Here NOTHING is
//! set by hand: we point a config file at an `acl/` directory and let
//! `APCore::from_path` auto-discover and enforce the policy at bootstrap.
//!
//! The recipe (D-64, Recommendation A):
//!
//! Step 1 — lay out a tiny project on disk:
//!
//! ```text
//! <project>/apcore.yaml          # acl: { root: ./acl, default_effect: deny }
//! <project>/acl/global_acl.yaml  # first-match-wins, default-deny policy
//! ```
//!
//! Co-locating them lets `acl.root` (`./acl`) resolve relative to the config
//! file via `Config::source_path`.
//!
//! Step 2 — `APCore::from_path(<project>/apcore.yaml)` loads the config, runs
//! `ACL::discover`, and attaches the loaded ACL to the executor automatically.
//!
//! Step 3 — calls are now governed with no manual `set_acl`. With `ctx: None`
//! the call arrives as the `@external` (unauthenticated) caller, so both
//! outcomes below are observable without constructing an Identity:
//!
//! - `@external` -> `executor.greeter.greet` is ALLOWED by an explicit rule.
//! - `@external` -> `executor.vault.read` is DENIED by default-deny.
//!
//! Run (from the apcore-rust repo root):
//!     cargo run --example acl_config_driven

use std::io::Write;
use std::process::ExitCode;

use apcore::errors::{ErrorCode, ModuleError};
use apcore::{APCore, Context, Module, Registry};
use async_trait::async_trait;
use serde_json::{json, Value};

/// A trivial tool module that echoes a canned result — stands in for a real
/// `executor.*` capability so the demo exercises actual execution under the ACL.
struct Tool {
    description: &'static str,
    output: Value,
}

#[async_trait]
impl Module for Tool {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &str {
        self.description
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(self.output.clone())
    }
}

/// Register the demo tools on the client's registry.
fn register_tools(registry: &Registry) {
    let tools = [
        (
            "executor.greeter.greet",
            Tool {
                description: "Return a friendly greeting",
                output: json!({ "message": "hello from apcore" }),
            },
        ),
        (
            "executor.vault.read",
            Tool {
                description: "Read a sensitive secret",
                output: json!({ "secret": "redacted" }),
            },
        ),
    ];
    for (id, tool) in tools {
        registry
            .register_module(id, Box::new(tool))
            .expect("register tool");
    }
}

/// Write a self-contained apcore project (config + co-located ACL) under `dir`
/// and return the path to the config file. The config mirrors the spec-mandated
/// required fields (version, project.name, extensions.root, schema.root,
/// executor.*) and points `acl.root` at the sibling `./acl` directory.
fn write_project(dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    // apcore.yaml — note `acl.root` is RELATIVE, so it resolves against this
    // file's own directory (Config::source_path), not the process CWD.
    let config_yaml = "\
version: \"0.24.0\"
project:
  name: acl-config-driven-demo
extensions:
  root: ./extensions
schema:
  root: ./schemas
acl:
  root: ./acl
  default_effect: deny
executor:
  default_timeout: 30000
  global_timeout: 60000
  max_call_depth: 32
  max_module_repeat: 3
";
    let config_path = dir.join("apcore.yaml");
    std::fs::File::create(&config_path)?.write_all(config_yaml.as_bytes())?;

    // acl/global_acl.yaml — first-match-wins, default-deny. The single allow
    // rule lets unauthenticated `@external` callers reach the greeter only;
    // everything else (e.g. the vault) falls through to `default_effect: deny`.
    let acl_dir = dir.join("acl");
    std::fs::create_dir_all(&acl_dir)?;
    let acl_yaml = "\
default_effect: deny
rules:
  - callers: [\"@external\"]
    targets: [\"executor.greeter.greet\"]
    effect: allow
    description: \"External callers may greet, nothing else.\"
";
    std::fs::File::create(acl_dir.join("global_acl.yaml"))?.write_all(acl_yaml.as_bytes())?;

    Ok(config_path)
}

#[tokio::main]
async fn main() -> ExitCode {
    // 1. Materialize a tiny project in a unique temp dir (cleaned up on exit).
    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("failed to create temp project dir: {e}");
            return ExitCode::FAILURE;
        }
    };
    let config_path = match write_project(tmp.path()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("failed to write project files: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("Config-driven ACL discovery — acl.root end-to-end demo (D-64, issue #74)");
    println!("Project laid out at: {}", tmp.path().display());
    println!("  apcore.yaml          -> acl: {{ root: ./acl, default_effect: deny }}");
    println!("  acl/global_acl.yaml  -> allow @external -> executor.greeter.greet");
    println!("{}", "=".repeat(78));

    // 2. Build the client straight from the config. No executor.set_acl here:
    //    APCore::from_path runs ACL::discover and wires the policy automatically.
    let client = match APCore::from_path(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to build APCore from config: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    register_tools(client.registry());

    let enforcing = client.executor().acl.is_some();
    println!(
        "ACL auto-wired by discovery (no manual set_acl): {}",
        if enforcing { "YES" } else { "NO" }
    );
    println!("{}", "-".repeat(78));

    // 3. Exercise the policy. ctx: None => the `@external` (unauthenticated)
    //    caller, so the allow rule and the default-deny are both observable.
    let mut failures = 0u32;

    println!("BEFORE: calling executor.greeter.greet as @external (expect ALLOW)");
    match client
        .call("executor.greeter.greet", json!({}), None, None)
        .await
    {
        Ok(v) => println!("AFTER:  ALLOWED -> {v}"),
        Err(e) => {
            failures += 1;
            println!("AFTER:  unexpected DENY -> {:?}: {}", e.code, e.message);
        }
    }
    println!("{}", "-".repeat(78));

    println!("BEFORE: calling executor.vault.read as @external (expect DENY)");
    match client
        .call("executor.vault.read", json!({}), None, None)
        .await
    {
        Ok(v) => {
            failures += 1;
            println!("AFTER:  unexpected ALLOW -> {v}");
        }
        Err(e) if e.code == ErrorCode::ACLDenied => {
            println!("AFTER:  DENIED -> {:?}: {}", e.code, e.message);
        }
        Err(e) => {
            failures += 1;
            println!("AFTER:  wrong error -> {:?}: {}", e.code, e.message);
        }
    }
    println!("{}", "=".repeat(78));

    if failures == 0 {
        println!("OK: enforcement active from config alone — allow + default-deny both held.");
        ExitCode::SUCCESS
    } else {
        eprintln!("FAIL: {failures} decision(s) drifted from the config-driven policy.");
        ExitCode::FAILURE
    }
}
