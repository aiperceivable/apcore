//! Runtime feature-toggle + per-instance isolation — end-to-end demo (issue #71).
//!
//! apcore is a standalone product: this example governs real tool calls through
//! the execution pipeline, with NO framework integration required. It
//! demonstrates the `system.control.toggle_feature` capability via the `APCore`
//! convenience methods (`client.disable` / `client.enable`):
//!
//! 1. registers real `executor.*` tools on an `APCore` instance,
//! 2. calls a tool and gets a real result,
//! 3. disables it — the next call fails with `ErrorCode::ModuleDisabled` while a
//!    *different* tool keeps working,
//! 4. re-enables it — the call works again, and
//! 5. proves per-instance isolation (#71): two independent `APCore` instances
//!    each register the same tool; disabling it on instance A blocks A's call
//!    while instance B's identical call still succeeds. Each instance owns its
//!    own `ToggleState` (locked cross-language by
//!    `conformance/fixtures/toggle_state_isolation.json`).
//!
//! Toggle control requires `sys_modules` (and `sys_modules.events`) enabled in
//! config, so the example constructs that `Config` itself.
//!
//! Each step carries its expected outcome, so this example doubles as a smoke
//! test: it exits non-zero if any step drifts from the cross-language contract.
//!
//! Run (from the apcore-rust repo root):
//!     cargo run --example feature_toggle

use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};

/// A trivial tool module that returns a canned result — stands in for a real
/// `executor.*` capability so the demo exercises actual execution.
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

/// Config with sys_modules + events enabled — required for toggle control.
fn sys_config() -> Config {
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    config
}

/// Build a client with two real CRM tools registered.
fn build_client() -> APCore {
    let client = APCore::with_config(sys_config());
    client
        .register(
            "executor.crm.read",
            Box::new(Tool {
                description: "Read a single CRM record",
                output: json!({ "record_id": "C-7", "name": "Acme Corp", "tier": "gold" }),
            }),
        )
        .expect("register executor.crm.read");
    client
        .register(
            "executor.crm.query",
            Box::new(Tool {
                description: "Query CRM records by filter",
                output: json!({ "filter": "tier=gold", "matched": 3 }),
            }),
        )
        .expect("register executor.crm.query");
    client
}

/// Call `target`; return `Ok(value)` when allowed or `Err` when blocked. A
/// disabled module surfaces as `ErrorCode::ModuleDisabled`.
async fn call_tool(client: &APCore, target: &str) -> Result<Value, ModuleError> {
    let inputs = match target {
        "executor.crm.query" => json!({ "filter": "tier=gold" }),
        _ => json!({ "record_id": "C-7" }),
    };
    client.call(target, inputs, None, None).await
}

/// Records one PASS/FAIL line per step and tracks the failure count.
struct Checker {
    failures: u32,
    total: u32,
}

impl Checker {
    fn new() -> Self {
        Self {
            failures: 0,
            total: 0,
        }
    }

    /// `result` is the call outcome; `expect_allow` is whether it should succeed.
    fn check(&mut self, label: &str, result: &Result<Value, ModuleError>, expect_allow: bool) {
        self.total += 1;
        let allowed = result.is_ok();
        let ok = allowed == expect_allow;
        if !ok {
            self.failures += 1;
        }
        let outcome = if allowed { "ALLOW" } else { "DISABLED" };
        let detail = match result {
            Ok(v) => v.to_string(),
            Err(e) => format!("{:?}", e.code),
        };
        let mark = if ok { "PASS" } else { "FAIL" };
        println!("  {mark:<7}{label:<46}{outcome:<10}{detail}");
        if !ok {
            let want = if expect_allow { "ALLOW" } else { "DISABLED" };
            println!("         ^ expected {want}");
        }
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    println!("Runtime feature-toggle + per-instance isolation — end-to-end demo (issue #71)");
    println!("Real tool calls through APCore; disable/enable via system.control.toggle_feature");
    println!("{}", "=".repeat(92));
    println!("  {:<7}{:<46}{:<10}DETAIL", "RESULT", "STEP", "OUTCOME");
    println!("{}", "-".repeat(92));

    let mut chk = Checker::new();

    // --- Single-instance toggle lifecycle ---------------------------------
    let client = build_client();

    let r = call_tool(&client, "executor.crm.read").await;
    chk.check("1. read works before toggle", &r, true);

    client
        .disable("executor.crm.read", Some("incident-1234"))
        .await
        .expect("disable executor.crm.read");
    let r = call_tool(&client, "executor.crm.read").await;
    chk.check("2. read disabled -> blocked", &r, false);
    assert_blocked_code(&r);

    let r = call_tool(&client, "executor.crm.query").await;
    chk.check("3. different tool still works", &r, true);

    client
        .enable("executor.crm.read", Some("incident resolved"))
        .await
        .expect("enable executor.crm.read");
    let r = call_tool(&client, "executor.crm.read").await;
    chk.check("4. read re-enabled -> works", &r, true);

    // --- Per-instance isolation (#71) -------------------------------------
    let client_a = build_client();
    let client_b = build_client();
    client_a
        .disable("executor.crm.read", Some("A-only maintenance"))
        .await
        .expect("disable on instance A");

    let r = call_tool(&client_a, "executor.crm.read").await;
    chk.check("5. instance A disabled -> blocked", &r, false);
    assert_blocked_code(&r);

    let r = call_tool(&client_b, "executor.crm.read").await;
    chk.check("6. instance B unaffected -> works", &r, true);

    println!("{}", "=".repeat(92));
    println!(
        "{}/{} steps match the cross-language contract.",
        chk.total - chk.failures,
        chk.total
    );
    if chk.failures > 0 {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

/// A blocked call MUST surface as `ErrorCode::ModuleDisabled` (cross-language
/// contract); this guards against the call failing for some other reason.
fn assert_blocked_code(result: &Result<Value, ModuleError>) {
    if let Err(e) = result {
        assert_eq!(
            e.code,
            ErrorCode::ModuleDisabled,
            "expected ModuleDisabled, got {:?}",
            e.code
        );
    }
}
