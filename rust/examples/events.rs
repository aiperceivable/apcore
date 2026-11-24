//! Lifecycle event subscription (event bus) — end-to-end demo.
//!
//! apcore ships a global event bus for framework lifecycle events: module
//! registration, feature toggles, config updates, and health thresholds. This
//! example wires the bus into an `APCore` client and observes canonical
//! lifecycle events through the client's subscription API — see the feature
//! spec at `apcore/docs/features/event-system.md` ("Event Naming Convention"
//! and the "Event Types" table for the canonical `apcore.<subsystem>.<event>`
//! names).
//!
//! It
//!
//! 1. builds an `APCore` with `sys_modules` + `events` ENABLED in `Config`,
//! 2. subscribes via `client.on(event_type, callback)` to two canonical
//!    lifecycle events — `apcore.registry.module_registered` and
//!    `apcore.module.toggled` — collecting each into a shared list,
//! 3. performs real lifecycle actions: registers a tool, calls it, and
//!    disables + re-enables it through the executor pipeline, and
//! 4. observes the canonical lifecycle events arriving at the subscribers,
//!    prints the collected stream, and unsubscribes via `client.off()`.
//!
//! `client.on()` subscribes to the SAME bus the sys modules and registry
//! callbacks publish to (D1-011 resolved), so the real `register()` /
//! `disable()` / `enable()` actions deliver straight to the subscribers —
//! matching the Python and TypeScript SDKs. Delivery is fire-and-forget on
//! spawned tasks, so the demo flushes the bus before asserting.
//!
//! Each expected event carries its count, so this example doubles as a smoke
//! test: it exits non-zero if the observed lifecycle stream drifts.
//!
//! Run (from the apcore-rust repo root):
//!     cargo run --example events

use apcore::events::emitter::ApCoreEvent;
use apcore::{APCore, Config, Context, Module, ModuleError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A trivial tool module returning a canned result — stands in for a real
/// `executor.*` capability so the demo exercises actual registration + calls.
#[derive(Debug)]
struct CrmRead;

#[async_trait]
impl Module for CrmRead {
    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": { "record_id": { "type": "string" } } })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "Read a single CRM record"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({ "record_id": "C-7", "name": "Acme Corp", "tier": "gold" }))
    }
}

/// One canonical lifecycle event the demo subscribes to and expects.
struct Expectation {
    event_type: &'static str,
    expected: usize,
}

/// Build a client with the event bus enabled (off by default).
fn build_client() -> APCore {
    let mut config = Config::default();
    config.set("sys_modules.enabled", json!(true));
    config.set("sys_modules.events.enabled", json!(true));
    APCore::with_config(config)
}

/// Drive the real lifecycle actions; the unified bus delivers the resulting
/// canonical events to the `on()` subscribers (no manual emit, D1-011 resolved).
async fn run_lifecycle(client: &APCore) {
    let module_id = "executor.crm.read";

    // Real registration fires `apcore.registry.module_registered`; the toggle
    // calls each fire `apcore.module.toggled`.
    client
        .register(module_id, Box::new(CrmRead))
        .expect("register tool");
    client
        .call(module_id, json!({ "record_id": "C-7" }), None, None)
        .await
        .expect("call tool");
    client
        .disable(module_id, Some("maintenance window"))
        .await
        .expect("disable");
    client
        .enable(module_id, Some("maintenance complete"))
        .await
        .expect("enable");

    // The bus delivers on spawned tasks (fire-and-forget). Let the registry
    // callback + toggle emits dispatch, then flush pending deliveries so the
    // assertions observe a complete stream.
    tokio::time::sleep(Duration::from_millis(200)).await;
    if let Some(bus) = client.events() {
        bus.flush(2000).await.expect("flush deliveries");
    }
}

/// Print the collected stream and return the failure count against expectations.
fn report(collected: &[ApCoreEvent], expectations: &[Expectation]) -> u32 {
    println!(
        "Collected {} lifecycle events (in delivery order):",
        collected.len()
    );
    println!("{}", "-".repeat(92));
    for e in collected {
        let detail = if e.event_type == "apcore.module.toggled" {
            let module = e.data["module_id"].as_str().unwrap_or("-");
            let enabled = e.data["enabled"].as_bool().unwrap_or(false);
            format!("module_id={module}, enabled={enabled}")
        } else {
            format!("module_id={}", e.module_id.as_deref().unwrap_or("-"))
        };
        println!("  {:<6}{:<40}{detail}", e.severity, e.event_type);
    }

    println!("{}", "=".repeat(92));
    let mut failures: u32 = 0;
    for exp in expectations {
        let got = collected
            .iter()
            .filter(|e| e.event_type == exp.event_type)
            .count();
        let ok = got == exp.expected;
        if !ok {
            failures += 1;
        }
        let mark = if ok { "PASS" } else { "FAIL" };
        println!(
            "  {mark}  {:<40} received {got}, expected {}",
            exp.event_type, exp.expected
        );
    }
    failures
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    println!("Lifecycle event subscription (event bus) — end-to-end demo");
    println!("APCore with sys_modules.events enabled; canonical apcore.<subsystem>.<event> names");
    println!("{}", "=".repeat(92));

    let mut client = build_client();

    // Subscribe to canonical lifecycle events; every delivery is collected.
    let collected: Arc<Mutex<Vec<ApCoreEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let registered_sink = Arc::clone(&collected);
    let toggled_sink = Arc::clone(&collected);
    // `on()` errors with SYS_MODULES_DISABLED when the client has no event bus;
    // `build_client()` enables sys_modules, so these succeed.
    let sub_registered = client
        .on("apcore.registry.module_registered", move |e| {
            registered_sink.lock().unwrap().push(e.clone());
        })
        .expect("events are enabled on this client");
    let sub_toggled = client
        .on("apcore.module.toggled", move |e| {
            toggled_sink.lock().unwrap().push(e.clone());
        })
        .expect("events are enabled on this client");

    run_lifecycle(&client).await;

    let expectations = [
        Expectation {
            event_type: "apcore.registry.module_registered",
            expected: 1,
        },
        Expectation {
            event_type: "apcore.module.toggled",
            expected: 2,
        },
    ];
    let failures = {
        let events = collected.lock().unwrap();
        report(&events, &expectations)
    };

    // Unsubscribe; subsequent events would no longer be collected.
    client
        .off(&sub_registered)
        .expect("events are enabled on this client");
    client
        .off(&sub_toggled)
        .expect("events are enabled on this client");

    println!("{}", "=".repeat(92));
    let total = expectations.len();
    println!(
        "{}/{total} lifecycle event expectations met.",
        total - failures as usize
    );
    if failures > 0 {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
