//! AI Agent Tool-Governance ACL — end-to-end demo (issue #72).
//!
//! apcore is a standalone product: this example governs real tool calls through
//! the execution pipeline, with NO framework integration required. It
//!
//! 1. registers real `executor.*` / `data.*` tools,
//! 2. wires the canonical default-deny agent-governance ACL into an `Executor`
//!    (the same policy as the spec repo's `examples/acl/agent-tool-governance.yaml`,
//!    locked cross-language by `conformance/fixtures/acl_agent_scoping.json`),
//! 3. has agents of different roles actually CALL the tools, and
//! 4. prints the resulting audit trail.
//!
//! Allowed calls return real results; denied calls fail with `ErrorCode::ACLDenied`.
//! The two conditions that make ACL valuable for per-agent tool governance:
//!
//! - `roles` — set-intersection against the caller's Identity.
//! - `max_call_depth` — fuses runaway tool chains (the pipeline measures the
//!   caller's live call-chain length, so a deeper chain trips the fuse even for
//!   the same role).
//!
//! Each scenario carries its expected outcome, so this example doubles as a smoke
//! test: it exits non-zero if any decision drifts from the cross-language contract.
//!
//! Run (from the apcore-rust repo root):
//!     cargo run --example acl_agent_governance

use apcore::acl::{ACLRule, AuditEntry, ACL};
use apcore::errors::ModuleError;
use apcore::{Config, Context, Executor, Identity, Module, Registry};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

/// A trivial tool module that returns a canned result — stands in for a real
/// `executor.*` / `data.*` capability so the demo exercises actual execution.
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

/// One tool-access scenario and its expected outcome.
struct Scenario {
    label: &'static str,
    caller_id: Option<&'static str>,
    roles: Option<&'static [&'static str]>,
    upstream_hops: usize,
    target: &'static str,
    expected: bool,
}

/// Register the real tools on a fresh registry.
fn build_registry() -> Registry {
    let registry = Registry::new();
    #[rustfmt::skip]
    let tools = [
        ("executor.crm.read",   Tool { description: "Read a single CRM record",        output: json!({"record_id": "C-7", "name": "Acme Corp", "tier": "gold"}) }),
        ("executor.crm.query",  Tool { description: "Query CRM records by filter",      output: json!({"filter": "tier=gold", "matched": 3}) }),
        ("executor.crm.delete", Tool { description: "Delete a CRM record",              output: json!({"record_id": "C-7", "deleted": true}) }),
        ("data.export",         Tool { description: "Export a dataset to cold storage", output: json!({"dataset": "crm", "rows": 1280}) }),
    ];
    for (id, tool) in tools {
        registry
            .register_module(id, Box::new(tool))
            .expect("register tool");
    }
    registry
}

/// The canonical agent-tool-governance policy (default-deny).
/// Mirrors examples/acl/agent-tool-governance.yaml in the apcore spec repo.
fn build_policy() -> Vec<ACLRule> {
    // `ACLRule` is `#[non_exhaustive]`, so a rule is built through
    // `ACLRule::new` and its optional fields assigned afterwards — the form
    // that compiles from outside the crate (api-surface-conventions.md §9.3).

    // External / unauthenticated callers (no caller_id) — read-only.
    let mut external = ACLRule::new(
        vec!["@external".to_string()],
        vec!["executor.*.read".to_string()],
        "allow",
    );
    external.description = Some("External callers may only read.".to_string());

    // Reader-role agents — read + query, depth-capped to fuse runaway chains.
    let mut reader = ACLRule::new(
        vec!["agent.*".to_string()],
        vec![
            "executor.*.read".to_string(),
            "executor.*.query".to_string(),
        ],
        "allow",
    );
    reader.description = Some("Reader agents may read/query, depth-capped.".to_string());
    reader.conditions = Some(json!({"roles": ["reader"], "max_call_depth": 3}));

    // Data-admin agents — exports and sensitive deletes (no depth cap).
    let mut data_admin = ACLRule::new(
        vec!["agent.*".to_string()],
        vec!["data.export".to_string(), "executor.*.delete".to_string()],
        "allow",
    );
    data_admin.description = Some("Data-admin agents may export and delete.".to_string());
    data_admin.conditions = Some(json!({"roles": ["data_admin"]}));

    vec![external, reader, data_admin]
}

/// Fixed inputs per tool (kept out of the scenario table for readability).
fn inputs_for(target: &str) -> Value {
    match target {
        "executor.crm.query" => json!({ "filter": "tier=gold" }),
        "data.export" => json!({ "dataset": "crm" }),
        _ => json!({ "record_id": "C-7" }), // read / delete
    }
}

/// Call `target` as `caller_id`. The pipeline derives the ACL caller from the
/// context's call chain (`caller_id == call_chain.last()`) and propagates the
/// agent Identity to the child context. `upstream_hops` synthetic prior entries
/// lengthen the chain so the `max_call_depth` fuse can be exercised.
async fn attempt(executor: &Executor, s: &Scenario) -> Result<Value, ModuleError> {
    let identity = s.roles.map(|rs| {
        Identity::new(
            s.caller_id.unwrap_or("unknown").to_string(),
            "ai".to_string(),
            rs.iter().map(std::string::ToString::to_string).collect(),
            HashMap::new(),
        )
    });
    let mut ctx: Context<Value> = Context::create(identity, None, None, None, Value::Null, None);
    let mut chain: Vec<String> = (0..s.upstream_hops)
        .map(|i| format!("upstream.hop{i}"))
        .collect();
    if let Some(caller) = s.caller_id {
        chain.push(caller.to_string());
    }
    ctx.call_chain = chain;
    executor
        .call(s.target, inputs_for(s.target), Some(&ctx), None)
        .await
}

/// Run every scenario, print the decision table, and return the failure count.
async fn run_scenarios(executor: &Executor, scenarios: &[Scenario]) -> u32 {
    println!("AI Agent Tool-Governance ACL — end-to-end demo (issue #72)");
    println!("Real tool calls through Executor, default_effect: deny, gradient @external < reader < data_admin");
    println!("{}", "=".repeat(100));
    println!(
        "  {:<8}{:<17}{:<13}{:<22}{:<8}DETAIL",
        "RESULT", "CALLER", "ROLES", "TARGET", "OUTCOME"
    );
    println!("{}", "-".repeat(100));

    let mut failures: u32 = 0;
    for s in scenarios {
        let result = attempt(executor, s).await;
        let allowed = result.is_ok();
        let ok = allowed == s.expected;
        if !ok {
            failures += 1;
        }

        let caller_disp = s.caller_id.unwrap_or("@external");
        let roles_disp = match s.roles {
            Some(rs) => rs.join(","),
            None => "-".to_string(),
        };
        let outcome = if allowed { "ALLOW" } else { "DENY" };
        let detail = match &result {
            Ok(v) => v.to_string(),
            Err(e) => format!("{:?}", e.code),
        };
        let mark = if ok { "PASS" } else { "FAIL" };
        println!(
            "  {mark:<8}{caller_disp:<17}{roles_disp:<13}{:<22}{outcome:<8}{detail}",
            s.target
        );
        if !ok {
            let want = if s.expected { "ALLOW" } else { "DENY" };
            println!("          ^ expected {want} — {}", s.label);
        }
    }
    failures
}

/// Print the ACL's recorded audit trail.
fn print_audit(entries: &[AuditEntry]) {
    println!("{}", "=".repeat(100));
    println!(
        "Audit trail ({} decisions recorded by the ACL):",
        entries.len()
    );
    for e in entries {
        let roles = if e.roles.is_empty() {
            "-".to_string()
        } else {
            e.roles.join(",")
        };
        let depth = e.call_depth.map_or("-".to_string(), |d| d.to_string());
        println!(
            "  {:<17} -> {:<22} {:<5} (reason={}, roles={roles}, depth={depth})",
            e.caller_id, e.target_id, e.decision, e.reason
        );
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    // Build the default-deny policy with an audit logger, wired into the executor.
    let audit_log: Arc<Mutex<Vec<AuditEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&audit_log);
    let mut acl = ACL::new(build_policy(), "deny", None);
    acl.set_audit_logger(move |entry| sink.lock().unwrap().push(entry.clone()));

    let mut executor = Executor::new(build_registry(), Config::default());
    executor.set_acl(acl);

    // Representative scenarios spanning the privilege gradient.
    #[rustfmt::skip]
    let scenarios = [
        Scenario { label: "External — read",                caller_id: None,                   roles: None,                    upstream_hops: 0, target: "executor.crm.read",   expected: true },
        Scenario { label: "External — query (blocked)",     caller_id: None,                   roles: None,                    upstream_hops: 0, target: "executor.crm.query",  expected: false },
        Scenario { label: "External — delete (blocked)",    caller_id: None,                   roles: None,                    upstream_hops: 0, target: "executor.crm.delete", expected: false },
        Scenario { label: "Reader — read",                  caller_id: Some("agent.research"), roles: Some(&["reader"]),       upstream_hops: 0, target: "executor.crm.read",   expected: true },
        Scenario { label: "Reader — query",                 caller_id: Some("agent.research"), roles: Some(&["reader"]),       upstream_hops: 0, target: "executor.crm.query",  expected: true },
        Scenario { label: "Reader — query at depth cap",    caller_id: Some("agent.research"), roles: Some(&["reader"]),       upstream_hops: 1, target: "executor.crm.query",  expected: true },
        Scenario { label: "Reader — query over depth cap",  caller_id: Some("agent.research"), roles: Some(&["reader"]),       upstream_hops: 2, target: "executor.crm.query",  expected: false },
        Scenario { label: "Reader — delete (blocked)",      caller_id: Some("agent.research"), roles: Some(&["reader"]),       upstream_hops: 0, target: "executor.crm.delete", expected: false },
        Scenario { label: "Reader — export (blocked)",      caller_id: Some("agent.research"), roles: Some(&["reader"]),       upstream_hops: 0, target: "data.export",         expected: false },
        Scenario { label: "Data-admin — export",            caller_id: Some("agent.etl"),      roles: Some(&["data_admin"]),   upstream_hops: 0, target: "data.export",         expected: true },
        Scenario { label: "Data-admin — delete",            caller_id: Some("agent.etl"),      roles: Some(&["data_admin"]),   upstream_hops: 0, target: "executor.crm.delete", expected: true },
        Scenario { label: "Data-admin — delete deep chain", caller_id: Some("agent.etl"),      roles: Some(&["data_admin"]),   upstream_hops: 3, target: "executor.crm.delete", expected: true },
        Scenario { label: "Data-admin — query (blocked)",   caller_id: Some("agent.etl"),      roles: Some(&["data_admin"]),   upstream_hops: 0, target: "executor.crm.query",  expected: false },
        Scenario { label: "Unknown-role agent (blocked)",   caller_id: Some("agent.guest"),    roles: Some(&["guest"]),        upstream_hops: 0, target: "executor.crm.read",   expected: false },
    ];

    let failures = run_scenarios(&executor, &scenarios).await;

    let entries = audit_log.lock().unwrap();
    print_audit(&entries);

    println!("{}", "=".repeat(100));
    let total = scenarios.len();
    println!(
        "{}/{total} decisions match the cross-language contract.",
        total - failures as usize
    );
    if failures > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
