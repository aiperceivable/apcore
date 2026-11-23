//! User-facing before/after middleware — end-to-end demo.
//!
//! apcore is a standalone product: this example shows the middleware hooks an SDK
//! *user* writes — `client.use_before(...)` and `client.use_after(...)` — which
//! wrap the whole module call (distinct from the internal pipeline *step*
//! middleware shown in `pipeline_demo.rs`). It
//!
//! 1. registers a real `executor.math.add` tool,
//! 2. installs a before-hook that augments inputs (injects a default `b`) and an
//!    after-hook that transforms output (adds a `verified` flag), with both hooks
//!    appending to a shared ordered trace,
//! 3. calls the tool through `client.call(...)`, and
//! 4. prints the original vs. middleware-transformed result and the trace.
//!
//! The before-hook returns `Some(replacement_inputs)` (or `None` to pass through);
//! the after-hook returns `Some(replacement_output)` (same contract). See the
//! feature doc `docs/features/middleware-system.md` (Contract: Middleware.before/after).
//!
//! Each expectation is asserted, so this example doubles as a smoke test: it exits
//! non-zero if the hook order or the transformed output drifts.
//!
//! Run (from the apcore-rust repo root):
//!     cargo run --example middleware

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::middleware::adapters::{AfterMiddleware, BeforeMiddleware};
use apcore::module::Module;
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

const DEFAULT_B: i64 = 22;

/// Shared ordered trace the tool and both hooks append to. Closures captured by
/// middleware mutate shared state, so it MUST be `Arc<Mutex<...>>`.
type Trace = Arc<Mutex<Vec<String>>>;

/// The real tool: adds two integers and records "execute" in the shared trace.
#[derive(Debug)]
struct AddModule {
    trace: Trace,
}

#[async_trait]
// The `Module` trait defines `fn description(&self) -> &str`; the impl signature
// must match, so we cannot widen the literal to `&'static str` as clippy suggests.
#[allow(clippy::unnecessary_literal_bound)]
impl Module for AddModule {
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "a": {"type": "integer"}, "b": {"type": "integer"} },
            "required": ["a", "b"],
        })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "result": {"type": "integer"} } })
    }
    fn description(&self) -> &str {
        "Add two integers"
    }
    async fn execute(&self, input: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        self.trace.lock().unwrap().push("execute".to_string());
        let a = input["a"].as_i64().unwrap_or(0);
        let b = input["b"].as_i64().unwrap_or(0);
        Ok(json!({ "result": a + b }))
    }
}

/// Before-hook: observe + augment inputs (inject a default `b` when omitted).
#[derive(Debug)]
struct RecordBefore {
    trace: Trace,
}

#[async_trait]
// Same trait-signature constraint as `AddModule`: `name` returns `&str`.
#[allow(clippy::unnecessary_literal_bound)]
impl BeforeMiddleware for RecordBefore {
    fn name(&self) -> &str {
        "record-before"
    }
    async fn before(
        &self,
        _module_id: &str,
        inputs: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.trace.lock().unwrap().push("before".to_string());
        if inputs.get("b").is_none() {
            let mut augmented = inputs;
            augmented["b"] = json!(DEFAULT_B);
            return Ok(Some(augmented)); // replacement inputs
        }
        Ok(None) // pass through unchanged
    }
}

/// After-hook: observe + transform output (stamp it as verified by middleware).
#[derive(Debug)]
struct RecordAfter {
    trace: Trace,
}

#[async_trait]
// Same trait-signature constraint as `AddModule`: `name` returns `&str`.
#[allow(clippy::unnecessary_literal_bound)]
impl AfterMiddleware for RecordAfter {
    fn name(&self) -> &str {
        "record-after"
    }
    async fn after(
        &self,
        _module_id: &str,
        _inputs: Value,
        output: Value,
        _ctx: &Context<Value>,
    ) -> Result<Option<Value>, ModuleError> {
        self.trace.lock().unwrap().push("after".to_string());
        let mut transformed = output;
        transformed["verified"] = json!(true);
        Ok(Some(transformed)) // replacement output
    }
}

/// Wire the tool + hooks onto a fresh client sharing one trace.
fn build_client(trace: &Trace) -> Result<APCore, ModuleError> {
    let client = APCore::new();
    client.register(
        "executor.math.add",
        Box::new(AddModule {
            trace: Arc::clone(trace),
        }),
    )?;
    client.use_before(Box::new(RecordBefore {
        trace: Arc::clone(trace),
    }))?;
    client.use_after(Box::new(RecordAfter {
        trace: Arc::clone(trace),
    }))?;
    Ok(client)
}

/// One named expectation and whether it held — keeps the check table flat.
struct Check {
    label: &'static str,
    ok: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let trace: Trace = Arc::new(Mutex::new(Vec::new()));
    let client = match build_client(&trace) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("setup failed: {e:?}");
            return ExitCode::FAILURE;
        }
    };

    println!("User-facing before/after middleware — end-to-end demo");
    println!("Tool: executor.math.add — before injects default b, after stamps verified");
    println!("{}", "=".repeat(72));

    // Call with `b` omitted so the before-hook's injection is observable.
    let result = match client
        .call("executor.math.add", json!({ "a": 20 }), None, None)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("call failed: {e:?}");
            return ExitCode::FAILURE;
        }
    };

    let trace_snapshot = trace.lock().unwrap().clone();
    println!("  inputs (as called)   : {{\"a\": 20}}  (b omitted)");
    println!("  before injected      : b = {DEFAULT_B}");
    println!(
        "  raw add result       : {{\"result\": {}}}",
        20 + DEFAULT_B
    );
    println!("  after-transformed    : {result}");
    println!("  hook/execute trace   : {trace_snapshot:?}");
    println!("{}", "=".repeat(72));

    let expected_trace = ["before", "execute", "after"];
    let expected_output = json!({ "result": 42, "verified": true });
    let checks = [
        Check {
            label: "trace order is before -> execute -> after",
            ok: trace_snapshot == expected_trace,
        },
        Check {
            label: "before-hook injected default b",
            ok: result["result"] == json!(42),
        },
        Check {
            label: "after-hook stamped verified=true",
            ok: result["verified"] == json!(true),
        },
        Check {
            label: "transformed output matches",
            ok: result == expected_output,
        },
    ];

    let mut failures: u32 = 0;
    for c in &checks {
        if !c.ok {
            failures += 1;
        }
        println!("  {}  {}", if c.ok { "PASS" } else { "FAIL" }, c.label);
        if !c.ok {
            println!("        ^ trace={trace_snapshot:?} output={result}");
        }
    }

    println!("{}", "=".repeat(72));
    let total = checks.len();
    println!("{}/{total} checks passed.", total - failures as usize);
    if failures > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
