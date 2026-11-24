//! Tests for Module `on_suspend` / `on_resume` lifecycle hooks.
//!
//! These hooks are defined on the `Module` trait (src/module.rs) and support
//! hot-reload state preservation:
//!   - `on_suspend(&self) -> Option<serde_json::Value>` — capture state before
//!     the module is swapped out; returns `None` if there is no state to save.
//!   - `on_resume(&self, state: serde_json::Value)` — restore previously
//!     captured state after the new module instance is loaded.
//!
//! Because these are plain trait methods (not driven by the executor), the
//! tests call them directly on module instances and verify the behaviour
//! required by PROTOCOL_SPEC §"Module Lifecycle".

use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::Module;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn make_ctx() -> Context<Value> {
    Context::new(Identity::new(
        "test".to_string(),
        "Test".to_string(),
        vec![],
        HashMap::new(),
    ))
}

// ---------------------------------------------------------------------------
// Module fixtures
// ---------------------------------------------------------------------------

/// A module that records a counter as its suspend state.
///
/// `on_suspend` returns `Some({"counter": N})`.
/// `on_resume` restores the counter from the provided state.
struct CounterModule {
    counter: Arc<Mutex<u64>>,
}

impl CounterModule {
    fn new(initial: u64) -> Self {
        Self {
            counter: Arc::new(Mutex::new(initial)),
        }
    }

    fn count(&self) -> u64 {
        *self.counter.lock().unwrap()
    }
}

#[async_trait]
impl Module for CounterModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "Counter module that tracks call count"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let mut guard = self.counter.lock().unwrap();
        *guard += 1;
        Ok(json!({ "counter": *guard }))
    }

    fn on_suspend(&self) -> Option<Value> {
        let count = self.counter.lock().unwrap();
        Some(json!({ "counter": *count }))
    }

    fn on_resume(&self, state: Value) {
        if let Some(n) = state.get("counter").and_then(Value::as_u64) {
            *self.counter.lock().unwrap() = n;
        }
    }
}

// ---------------------------------------------------------------------------

/// A stateless module — `on_suspend` returns `None`.
struct StatelessModule;

#[async_trait]
impl Module for StatelessModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "A module with no persistent state"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"ok": true}))
    }
    // `on_suspend` and `on_resume` use the default (no-op) implementations.
}

// ---------------------------------------------------------------------------

/// A module that stores a key-value map as its state.
struct KVModule {
    store: Arc<Mutex<HashMap<String, Value>>>,
}

impl KVModule {
    fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn set(&self, key: &str, val: Value) {
        self.store.lock().unwrap().insert(key.to_string(), val);
    }

    fn get(&self, key: &str) -> Option<Value> {
        self.store.lock().unwrap().get(key).cloned()
    }
}

#[async_trait]
impl Module for KVModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "KV store module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({}))
    }

    fn on_suspend(&self) -> Option<Value> {
        let map = self.store.lock().unwrap();
        if map.is_empty() {
            None
        } else {
            let obj: serde_json::Map<_, _> =
                map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            Some(Value::Object(obj))
        }
    }

    fn on_resume(&self, state: Value) {
        if let Value::Object(map) = state {
            let mut store = self.store.lock().unwrap();
            for (k, v) in map {
                store.insert(k, v);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// on_suspend() — returns Some(state)
// ---------------------------------------------------------------------------

#[test]
fn on_suspend_returns_some_when_module_has_state() {
    let module = CounterModule::new(42);
    let state = module.on_suspend();
    assert!(
        state.is_some(),
        "CounterModule.on_suspend() should return Some"
    );
}

#[test]
fn on_suspend_state_contains_counter_value() {
    let module = CounterModule::new(7);
    let state = module.on_suspend().unwrap();
    assert_eq!(
        state.get("counter").and_then(Value::as_u64),
        Some(7),
        "suspended state must encode the current counter value"
    );
}

#[test]
fn on_suspend_reflects_updated_counter() {
    let module = CounterModule::new(0);
    // Simulate some executions by directly bumping the counter.
    *module.counter.lock().unwrap() = 5;
    let state = module.on_suspend().unwrap();
    assert_eq!(state["counter"], json!(5));
}

// ---------------------------------------------------------------------------
// on_resume() — receives previous state
// ---------------------------------------------------------------------------

#[test]
fn on_resume_restores_counter_from_state() {
    let module = CounterModule::new(0);
    module.on_resume(json!({ "counter": 99 }));
    assert_eq!(module.count(), 99);
}

#[test]
fn on_resume_is_no_op_for_stateless_module() {
    // Default impl must not panic — calling with arbitrary state is safe.
    let module = StatelessModule;
    module.on_resume(json!({ "anything": true }));
    // No assertion needed beyond "it didn't panic".
}

#[test]
fn on_resume_ignores_unrecognised_state_keys() {
    // Counter module only looks at "counter"; extra keys are silently ignored.
    let module = CounterModule::new(0);
    module.on_resume(json!({ "counter": 3, "garbage": "data" }));
    assert_eq!(module.count(), 3);
}

// ---------------------------------------------------------------------------
// Round-trip: suspend → get state → resume with state → verify restoration
// ---------------------------------------------------------------------------

#[test]
fn round_trip_counter_state_survives_suspend_resume_cycle() {
    // --- "Old" module instance accumulates state ---
    let old_module = CounterModule::new(0);
    *old_module.counter.lock().unwrap() = 17;

    // Suspend the old instance to capture state.
    let saved_state = old_module
        .on_suspend()
        .expect("CounterModule must return Some state");

    // --- "New" module instance starts fresh ---
    let new_module = CounterModule::new(0);
    assert_eq!(new_module.count(), 0, "new instance starts at zero");

    // Resume the new instance with the saved state.
    new_module.on_resume(saved_state);

    // Verify the new instance has the same counter as the old one.
    assert_eq!(
        new_module.count(),
        17,
        "counter must be restored to the value captured at suspend time"
    );
}

#[test]
fn round_trip_kv_module_all_entries_restored() {
    let old_module = KVModule::new();
    old_module.set("alpha", json!(1));
    old_module.set("beta", json!("hello"));
    old_module.set("gamma", json!([1, 2, 3]));

    let state = old_module
        .on_suspend()
        .expect("KVModule with data must return Some");

    let new_module = KVModule::new();
    new_module.on_resume(state);

    assert_eq!(new_module.get("alpha"), Some(json!(1)));
    assert_eq!(new_module.get("beta"), Some(json!("hello")));
    assert_eq!(new_module.get("gamma"), Some(json!([1, 2, 3])));
}

#[test]
fn round_trip_multiple_cycles_accumulate_correctly() {
    // Simulates two successive hot-reloads.
    let m1 = CounterModule::new(10);
    let state1 = m1.on_suspend().unwrap();

    let m2 = CounterModule::new(0);
    m2.on_resume(state1);
    assert_eq!(m2.count(), 10);

    // More executions on m2.
    *m2.counter.lock().unwrap() = 25;
    let state2 = m2.on_suspend().unwrap();

    let m3 = CounterModule::new(0);
    m3.on_resume(state2);
    assert_eq!(m3.count(), 25);
}

// ---------------------------------------------------------------------------
// on_suspend() returning None — no state to preserve
// ---------------------------------------------------------------------------

#[test]
fn on_suspend_returns_none_for_stateless_module() {
    let module = StatelessModule;
    let state = module.on_suspend();
    assert!(
        state.is_none(),
        "stateless module's default on_suspend() must return None"
    );
}

#[test]
fn kv_module_returns_none_when_empty() {
    let module = KVModule::new();
    // Nothing stored yet — on_suspend should return None.
    assert!(
        module.on_suspend().is_none(),
        "KVModule with no entries must return None from on_suspend()"
    );
}

#[test]
fn kv_module_returns_some_after_entry_added() {
    let module = KVModule::new();
    module.set("key", json!("value"));
    assert!(module.on_suspend().is_some());
}

// ---------------------------------------------------------------------------
// suspend during / after active calls
// ---------------------------------------------------------------------------

#[tokio::test]
async fn suspend_state_captured_after_execute_reflects_execution() {
    let module = CounterModule::new(0);
    let ctx = make_ctx();

    // Execute the module once — counter increments to 1.
    let _ = module.execute(json!({}), &ctx).await.unwrap();

    // Now suspend — the captured counter must reflect the post-execute state.
    let state = module.on_suspend().unwrap();
    assert_eq!(
        state["counter"],
        json!(1),
        "on_suspend should capture the state as it exists after execute()"
    );
}

#[tokio::test]
async fn suspend_resume_preserves_state_across_simulated_reload() {
    // Simulate the full hot-reload flow:
    //   1. Run module for a while.
    //   2. Suspend to snapshot state.
    //   3. Build a fresh instance.
    //   4. Resume with the snapshot.
    //   5. Continue execution — counter must carry over.

    let original = CounterModule::new(0);
    let ctx = make_ctx();

    // Three executions on the original instance.
    for _ in 0..3 {
        let _ = original.execute(json!({}), &ctx).await.unwrap();
    }
    assert_eq!(original.count(), 3);

    // Suspend.
    let snapshot = original.on_suspend().unwrap();

    // New instance starts from zero.
    let replacement = CounterModule::new(0);
    replacement.on_resume(snapshot);
    assert_eq!(replacement.count(), 3, "count must be restored");

    // Continue executing on the replacement.
    let result = replacement.execute(json!({}), &ctx).await.unwrap();
    assert_eq!(
        result["counter"],
        json!(4),
        "execute on the resumed instance must continue from the restored count"
    );
}

// ---------------------------------------------------------------------------
// on_load / on_unload defaults (spec §Module Lifecycle, related coverage)
// ---------------------------------------------------------------------------

#[test]
fn on_load_default_does_not_panic() {
    let module = StatelessModule;
    module
        .on_load()
        .expect("on_load default must return Ok(())"); // must be a no-op
}

#[test]
fn on_unload_default_does_not_panic() {
    let module = StatelessModule;
    module.on_unload(); // must be a no-op and not panic
}
