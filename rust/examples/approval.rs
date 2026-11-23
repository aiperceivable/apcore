//! Human-in-the-loop Approval gate — end-to-end demo.
//!
//! apcore is a standalone product: this example runs real tool calls through the
//! execution pipeline, with NO framework integration required. It is the natural
//! companion to `examples/acl_agent_governance.rs` — ACL governs *who* may call a
//! tool; Approval is the *human-in-the-loop gate* for the sensitive ones. The gate
//! fires at Executor Step 5, after ACL passes and before middleware runs. See the
//! spec repo's `docs/features/approval-system.md`.
//!
//! It
//!
//! 1. registers a sensitive tool that declares `requires_approval=true`
//!    (`executor.crm.delete`) and a normal tool that does not (`executor.crm.read`),
//! 2. wires a `CallbackApprovalHandler` whose decision depends on the request — it
//!    approves only when the caller passes `confirmed=true`, and rejects otherwise
//!    (the kind of policy a real reviewer UI would enforce), and
//! 3. runs scenarios that actually CALL the tools.
//!
//! A normal tool runs without ever reaching the gate. A sensitive tool with the
//! handler APPROVING returns its real result; with the handler REJECTING it fails
//! with `ErrorCode::ApprovalDenied`.
//!
//! Each scenario carries its expected outcome, so this example doubles as a smoke
//! test: it exits non-zero if any decision drifts from the cross-language contract.
//!
//! Cross-language: the `requires_approval` annotation rides with the module via
//! its `annotations()` method in all three SDKs. `register_module` derives the
//! descriptor's annotations from the module (matching Python's `@module(...)`
//! decorator and TS's `annotations` option), so the gate sees it.
//!
//! Run (from the apcore-rust repo root):
//!     cargo run --example approval

use apcore::approval::{ApprovalRequest, ApprovalResult, CallbackApprovalHandler};
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::ModuleAnnotations;
use apcore::{Config, Context, Executor, Identity, Module, Registry};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::ExitCode;

/// A trivial tool module that returns a canned result — stands in for a real
/// `executor.*` capability so the demo exercises actual execution. It declares
/// `requires_approval` via the `annotations()` trait method, just like a Python
/// `@module(annotations={"requires_approval": True})` or a TS `annotations` option.
struct Tool {
    description: &'static str,
    output: Value,
    requires_approval: bool,
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
    fn annotations(&self) -> ModuleAnnotations {
        ModuleAnnotations {
            requires_approval: self.requires_approval,
            ..ModuleAnnotations::default()
        }
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(self.output.clone())
    }
}

/// One approval scenario and its expected outcome.
struct Scenario {
    label: &'static str,
    caller_id: &'static str,
    target: &'static str,
    inputs: Value,
    expected: bool,
}

/// Register the demo tools: a normal reader and a sensitive, gated delete.
/// `register_module` derives each descriptor's annotations from the module, so
/// the sensitive tool's `requires_approval=true` reaches the gate.
fn build_registry() -> Registry {
    let registry = Registry::new();
    registry
        .register_module(
            "executor.crm.read",
            Box::new(Tool {
                description: "Read a single CRM record",
                output: json!({"record_id": "C-7", "name": "Acme Corp", "tier": "gold"}),
                requires_approval: false,
            }),
        )
        .expect("register read tool");
    registry
        .register_module(
            "executor.crm.delete",
            Box::new(Tool {
                description: "Delete a CRM record (sensitive — requires human approval)",
                output: json!({"record_id": "C-7", "deleted": true}),
                requires_approval: true,
            }),
        )
        .expect("register delete tool");
    registry
}

/// A request-dependent approval policy: approve only when the caller passed
/// `confirmed=true`, otherwise reject with an explanatory reason. A real handler
/// would block on a reviewer UI / Slack / e-mail; this one is deterministic.
/// A synchronous, in-process decision — no I/O, so `new_sync` is the right
/// constructor (apcore#104). This reviewer is a pure function of the request
/// and cannot fail; the `Ok(...)` is added at the call site below, where the
/// comment explains what the fallible shape buys a real handler.
fn review(request: &ApprovalRequest) -> ApprovalResult {
    let approver = request
        .context
        .as_ref()
        .and_then(|c| c.identity.as_ref())
        .map_or_else(|| "anonymous".to_string(), |id| id.id().to_string());
    // ApprovalResult is #[non_exhaustive]: no cross-crate struct literal, and
    // no `..Default::default()` either. The semantic constructors set `status`
    // to a canonical value; `approved_by` is assigned afterwards.
    let mut result = if request.arguments.get("confirmed") == Some(&Value::Bool(true)) {
        ApprovalResult::approved(approver.clone())
    } else {
        ApprovalResult::rejected("caller did not confirm the destructive operation")
    };
    result.approved_by = Some(approver);
    result
}

/// Call `target` as `caller_id`, propagating the operator identity to the gate.
async fn attempt(executor: &Executor, s: &Scenario) -> Result<Value, ModuleError> {
    let identity = Identity::new(
        s.caller_id.to_string(),
        "user".to_string(),
        vec!["operator".to_string()],
        HashMap::new(),
    );
    let ctx: Context<Value> = Context::create(Some(identity), None, None, None, Value::Null, None);
    executor
        .call(s.target, s.inputs.clone(), Some(&ctx), None)
        .await
}

/// Run every scenario, print the decision table, and return the failure count.
async fn run_scenarios(executor: &Executor, scenarios: &[Scenario]) -> u32 {
    println!("Human-in-the-loop Approval gate — end-to-end demo");
    println!(
        "Real tool calls through Executor; gate fires at Step 5 for requires_approval modules"
    );
    println!("{}", "=".repeat(100));
    println!(
        "  {:<8}{:<10}{:<22}{:<10}DETAIL",
        "RESULT", "CALLER", "TARGET", "OUTCOME"
    );
    println!("{}", "-".repeat(100));

    let mut failures: u32 = 0;
    for s in scenarios {
        let result = attempt(executor, s).await;
        let executed = result.is_ok();
        let ok = executed == s.expected;
        if !ok {
            failures += 1;
        }

        let outcome = if executed { "EXECUTED" } else { "BLOCKED" };
        let detail = match &result {
            Ok(v) => v.to_string(),
            Err(e) if e.code == ErrorCode::ApprovalDenied => {
                format!("ApprovalDeniedError: {}", e.message)
            }
            Err(e) => format!("{:?}: {}", e.code, e.message),
        };
        let mark = if ok { "PASS" } else { "FAIL" };
        println!(
            "  {mark:<8}{:<10}{:<22}{outcome:<10}{detail}",
            s.caller_id, s.target
        );
        if !ok {
            let want = if s.expected { "EXECUTED" } else { "BLOCKED" };
            println!("          ^ expected {want} — {}", s.label);
        }
    }
    failures
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut executor = Executor::new(build_registry(), Config::default());
    // apcore#104: `CallbackApprovalHandler::new` is async and fallible (parity
    // with apcore-python and apcore-typescript); `new_sync` is the in-process,
    // no-I/O form. Both return a `Result`, so a handler that could not reach
    // its approval service reports a FAILURE rather than fabricating a
    // "rejected" result that reads in the audit log like a human saying no.
    // This reviewer is pure, so it always succeeds.
    executor.set_approval_handler(Box::new(CallbackApprovalHandler::new_sync(|req| {
        Ok(review(req))
    })));

    #[rustfmt::skip]
    let scenarios = [
        Scenario { label: "Normal tool — no gate",                  caller_id: "alice", target: "executor.crm.read",   inputs: json!({"record_id": "C-7"}),                     expected: true },
        Scenario { label: "Sensitive — confirmed (approved)",       caller_id: "alice", target: "executor.crm.delete", inputs: json!({"record_id": "C-7", "confirmed": true}),  expected: true },
        Scenario { label: "Sensitive — unconfirmed (rejected)",     caller_id: "alice", target: "executor.crm.delete", inputs: json!({"record_id": "C-7"}),                     expected: false },
        Scenario { label: "Sensitive — confirmed=false (rejected)", caller_id: "bob",   target: "executor.crm.delete", inputs: json!({"record_id": "C-9", "confirmed": false}), expected: false },
    ];

    let failures = run_scenarios(&executor, &scenarios).await;

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
