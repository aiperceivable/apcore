//! Demonstrate the 11-step ExecutionStrategy pipeline.
//!
//! Three sections in one run:
//!   1. Introspection      — print the 11 default step names.
//!   2. Middleware tracing — register a StepMiddleware that logs entry,
//!      exit, and per-step duration.
//!   3. Orchestration      — insert_after() adds a custom AuditLogStep,
//!      then replace() swaps it for a quieter one.
//!
//! Run: cargo run --example pipeline_demo

use std::sync::{Arc, Mutex};
use std::time::Instant;

use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::Module;
use apcore::pipeline::{
    ExecutionStrategy, PipelineContext, PipelineState, Step, StepMiddleware, StepResult,
};
use apcore::registry::registry::Registry;
use apcore::{build_standard_strategy, Executor};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

// ── Module under test ───────────────────────────────────────────────────
struct AddModule;

#[async_trait]
impl Module for AddModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "a": {"type": "integer"}, "b": {"type": "integer"} },
            "required": ["a", "b"],
        })
    }
    fn output_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "result": {"type": "integer"} },
        })
    }
    fn description(&self) -> &'static str {
        "Add two integers"
    }
    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let a = input["a"].as_i64().unwrap_or(0);
        let b = input["b"].as_i64().unwrap_or(0);
        Ok(json!({ "result": a + b }))
    }
}

// The canonical 11-step pipeline defined by the apcore protocol spec.
// Anything not in this set is a user-inserted custom step.
const CANONICAL_STEPS: &[&str] = &[
    "context_creation",
    "call_chain_guard",
    "module_lookup",
    "acl_check",
    "approval_gate",
    "middleware_before",
    "input_validation",
    "execute",
    "output_validation",
    "middleware_after",
    "return_result",
];

fn is_canonical(name: &str) -> bool {
    CANONICAL_STEPS.contains(&name)
}

// ── Section 2: a StepMiddleware that traces each step ──────────────────
fn step_role(name: &str) -> &'static str {
    match name {
        "context_creation" => "create execution context, set global deadline",
        "call_chain_guard" => "check call depth & repeat limits",
        "module_lookup" => "resolve module from registry",
        "acl_check" => "enforce access control (default-deny)",
        "approval_gate" => "human approval gate (if required)",
        "middleware_before" => "run before-middleware chain (in order)",
        "input_validation" => "validate inputs against schema",
        "execute" => "invoke the module",
        "output_validation" => "validate output against schema",
        "middleware_after" => "run after-middleware chain (reverse order)",
        "return_result" => "finalize and return output",
        _ => "custom step",
    }
}

fn summarize(step_name: &str, ctx: &PipelineContext) -> String {
    match step_name {
        "context_creation" => {
            let cid = ctx.context.caller_id.as_deref().unwrap_or("anonymous");
            let tid: String = ctx.context.trace_id.chars().take(8).collect();
            format!("caller={cid} trace_id={tid}…")
        }
        "module_lookup" if ctx.module.is_some() => {
            format!("resolved module '{}'", ctx.module_id)
        }
        "middleware_before" => format!("inputs={}", ctx.inputs),
        "input_validation" => format!(
            "validated_inputs={}",
            ctx.validated_inputs
                .as_ref()
                .map_or(Value::Null, Value::clone)
        ),
        "execute" => format!(
            "output={}",
            ctx.output.as_ref().map_or(Value::Null, Value::clone),
        ),
        "output_validation" => format!(
            "validated_output={}",
            ctx.validated_output
                .as_ref()
                .map_or(Value::Null, Value::clone)
        ),
        "return_result" => format!(
            "returning {}",
            ctx.validated_output
                .as_ref()
                .or(ctx.output.as_ref())
                .map_or(Value::Null, Value::clone)
        ),
        _ => "continue".to_string(),
    }
}

struct TracingMiddleware {
    starts: Mutex<HashMap<String, Instant>>,
    core_idx: Mutex<usize>,
}

impl TracingMiddleware {
    fn new() -> Self {
        Self {
            starts: Mutex::new(HashMap::new()),
            core_idx: Mutex::new(0),
        }
    }
}

#[async_trait]
impl StepMiddleware for TracingMiddleware {
    async fn before_step(
        &self,
        step_name: &str,
        _state: &PipelineState<'_>,
    ) -> Result<(), ModuleError> {
        let canonical = is_canonical(step_name);
        let label = {
            let mut idx = self.core_idx.lock().unwrap();
            if step_name == "context_creation" {
                *idx = 0;
            }
            if canonical {
                *idx += 1;
                format!("[{:>2}/11]", *idx)
            } else {
                "[  +  ]".to_string()
            }
        };
        let role = if canonical {
            step_role(step_name).to_string()
        } else {
            "CUSTOM step inserted via insert_after / replace".to_string()
        };
        self.starts
            .lock()
            .unwrap()
            .insert(step_name.to_string(), Instant::now());
        println!("  {label} {step_name:<19} — {role}");
        Ok(())
    }

    async fn after_step(
        &self,
        step_name: &str,
        state: &PipelineState<'_>,
        _result: &Value,
    ) -> Result<(), ModuleError> {
        let start = self
            .starts
            .lock()
            .unwrap()
            .remove(step_name)
            .unwrap_or_else(Instant::now);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        println!(
            "          ✓ {:>6.2} ms · {}",
            elapsed_ms,
            summarize(step_name, state.context)
        );
        Ok(())
    }
}

// ── Section 3: a custom step inserted after output_validation ──────────
struct AuditLogStep;

#[async_trait]
// trait `Step` defines `fn name/description(&self) -> &str`; the impl signature
// must match, so we cannot widen literals to `&'static str` even though clippy suggests it.
#[allow(clippy::unnecessary_literal_bound)]
impl Step for AuditLogStep {
    fn name(&self) -> &str {
        "audit_log"
    }
    fn description(&self) -> &str {
        "Emit an audit record after output validation."
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        let caller = ctx.context.caller_id.as_deref().unwrap_or("anonymous");
        println!(
            "    [audit] caller={caller} target={} ok=true",
            ctx.module_id
        );
        Ok(StepResult::continue_step())
    }
}

struct QuietAuditLogStep;

#[async_trait]
// Same trait-signature constraint as `AuditLogStep` above.
#[allow(clippy::unnecessary_literal_bound)]
impl Step for QuietAuditLogStep {
    fn name(&self) -> &str {
        "audit_log"
    }
    fn description(&self) -> &str {
        "Quiet audit step (replacement demo)."
    }
    fn removable(&self) -> bool {
        true
    }
    fn replaceable(&self) -> bool {
        true
    }
    async fn execute(&self, _ctx: &mut PipelineContext) -> Result<StepResult, ModuleError> {
        Ok(StepResult {
            action: "continue".into(),
            explanation: Some("quiet audit recorded".into()),
            ..Default::default()
        })
    }
}

// ── Helpers ────────────────────────────────────────────────────────────
fn banner(title: &str) {
    println!("\n=== {title} ===");
}

fn print_steps(strategy: &ExecutionStrategy, show_custom: bool) {
    for (i, name) in strategy.step_names().iter().enumerate() {
        let tag = if show_custom && !is_canonical(name) {
            "  ← CUSTOM (inserted)"
        } else {
            ""
        };
        println!("  {:>2}. {name}{tag}", i + 1);
    }
}

fn build_executor(
    registry: Arc<Registry>,
    config: Arc<Config>,
    strategy: ExecutionStrategy,
) -> Executor {
    Executor::with_strategy(registry, config, strategy)
}

#[tokio::main]
// `ModuleError` is large by design; boxing it in an example `main` adds noise without value.
#[allow(clippy::result_large_err)]
async fn main() -> Result<(), ModuleError> {
    let registry = Arc::new(Registry::new());
    registry.register_module("math.add", Box::new(AddModule))?;
    let config = Arc::new(Config::default());
    let identity = Identity::new(
        "demo-user".into(),
        "user".into(),
        vec!["user".into()],
        HashMap::new(),
    );
    let ctx: Context<Value> = Context::new(identity);

    // ── Section 1: Introspection ───────────────────────────────────────
    banner("Section 1: Introspection — the default 11-step pipeline");
    let strategy0 = build_standard_strategy();
    let info = strategy0.info();
    println!("strategy: {}  (steps: {})", info.name, info.step_count);
    print_steps(&strategy0, false);

    // ── Section 2: Middleware tracing ──────────────────────────────────
    banner("Section 2: Middleware tracing — one call through 11 steps");
    let mut strategy = build_standard_strategy();
    strategy.add_step_middleware(Arc::new(TracingMiddleware::new()));
    let executor = build_executor(registry.clone(), config.clone(), strategy);
    let result = executor
        .call("math.add", json!({"a": 10, "b": 5}), Some(&ctx), None)
        .await?;
    println!("result: {result}");

    // ── Section 3a: insert_after ───────────────────────────────────────
    banner("Section 3: Orchestration — insert_after + replace");
    let mut strategy = build_standard_strategy();
    strategy.insert_after("output_validation", Box::new(AuditLogStep))?;
    strategy.add_step_middleware(Arc::new(TracingMiddleware::new()));
    let custom_count = strategy
        .step_names()
        .iter()
        .filter(|n| !is_canonical(n))
        .count();
    println!(
        "after insert_after: 11 standard + {custom_count} custom = {} steps",
        strategy.steps().len()
    );
    print_steps(&strategy, true);
    println!("\ncalling with the inserted audit step:");
    let executor = build_executor(registry.clone(), config.clone(), strategy);
    executor
        .call("math.add", json!({"a": 2, "b": 3}), Some(&ctx), None)
        .await?;

    // ── Section 3b: replace ────────────────────────────────────────────
    let mut strategy = build_standard_strategy();
    strategy.insert_after("output_validation", Box::new(AuditLogStep))?;
    strategy.replace("audit_log", Box::new(QuietAuditLogStep))?;
    strategy.add_step_middleware(Arc::new(TracingMiddleware::new()));
    let idx = strategy
        .step_names()
        .iter()
        .position(|n| n == "audit_log")
        .expect("audit_log should still exist after replace");
    println!(
        "\nafter replace: {} steps (audit_log still at index {idx})",
        strategy.steps().len()
    );
    println!("\ncalling with the quiet replacement:");
    let executor = build_executor(registry, config, strategy);
    executor
        .call("math.add", json!({"a": 7, "b": 9}), Some(&ctx), None)
        .await?;

    Ok(())
}
