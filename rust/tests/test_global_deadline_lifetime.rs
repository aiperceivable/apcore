//! D-100 / D-101 / D-102 — where `global_deadline` is stored, and how long it lives.
//!
//! apcore-rust is the **authority** the deep-chain adjudication named for all
//! three of these rules, which is exactly why they were pinned nowhere here:
//! the reference implementation is the one nobody writes a regression test
//! against. D-99 (the clock is epoch seconds) is already pinned by
//! `test_executor.rs::test_global_deadline_set_by_context_creation`, which
//! asserts the computed deadline lands ~60s past an epoch-seconds `now`.
//!
//! What makes each test below RED:
//!
//! * **D-100** — dropping the `global_deadline.is_none()` guard at the set
//!   site (`builtin_steps.rs`), so a caller-supplied deadline is overwritten
//!   by the config default. The caller's 50 ms budget becomes 10 s and the
//!   slow module completes.
//! * **D-101** — persisting the computed deadline back onto the caller's
//!   Context (via interior mutability, or by taking `&mut Context`). The
//!   second top-level call on a reused Context then inherits the first call's
//!   spent budget.
//! * **D-102** — re-adding the `call_chain.is_empty()` conjunct to the set
//!   site. A Context arriving from another process carries a non-empty chain
//!   by definition, so the sub-tree would run with no budget at all.

use apcore::context::Context;
use apcore::errors::ModuleError;
use apcore::module::Module;
use apcore::APCore;
use async_trait::async_trait;
use serde_json::{json, Value};

/// Sleeps 150 ms, then reports the deadline its context carried.
struct SlowModule;

#[async_trait]
impl Module for SlowModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Sleeps 150ms"
    }
    async fn execute(&self, _inputs: Value, ctx: &Context<Value>) -> Result<Value, ModuleError> {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        Ok(json!({"global_deadline": ctx.global_deadline}))
    }
}

fn client_with_timeout(global_timeout_ms: u64) -> APCore {
    let mut config = apcore::config::Config::default();
    config.set("sys_modules.enabled", json!(false));
    config.set("executor.global_timeout", json!(global_timeout_ms));
    let client = APCore::with_config(config);
    client
        .register("probe.slow", Box::new(SlowModule))
        .expect("register");
    client
}

fn epoch_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ---------------------------------------------------------------------------
// D-100 — the first-class `Context.global_deadline` field IS the storage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn caller_supplied_deadline_wins_over_the_config_default() {
    // A 10 s config budget would let the 150 ms module finish easily. The
    // caller's own 50 ms budget must be the one that decides.
    let client = client_with_timeout(10_000);

    let mut ctx = Context::<Value>::anonymous();
    ctx.global_deadline = Some(epoch_now() + 0.05);

    let result = client.call("probe.slow", json!({}), Some(&ctx), None).await;
    assert!(
        result.is_err(),
        "a caller-supplied 50ms deadline must be honoured over the 10s config \
         default, but the call succeeded: {result:?}"
    );
}

#[tokio::test]
async fn control_the_same_module_completes_under_the_config_default() {
    // Control for the test above: without a caller-supplied deadline the very
    // same module and the very same 10 s config budget must succeed. Without
    // this, a blanket "everything times out" regression would pass the D-100
    // assertion for the wrong reason.
    let client = client_with_timeout(10_000);

    let result = client.call("probe.slow", json!({}), None, None).await;
    assert!(
        result.is_ok(),
        "the 150ms module must complete under a 10s budget: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// D-101 — the deadline belongs to the CALL TREE, not to the Context
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_reused_context_gets_a_fresh_budget_on_every_top_level_call() {
    // 250 ms budget, 150 ms module. If the first call's deadline were
    // persisted onto the caller's Context, the second call would start with
    // ~100 ms left and be refused.
    let client = client_with_timeout(250);
    let ctx = Context::<Value>::anonymous();

    for attempt in 1..=3 {
        let result = client.call("probe.slow", json!({}), Some(&ctx), None).await;
        assert!(
            result.is_ok(),
            "call #{attempt} on a reused Context must get its own 250ms budget: {result:?}"
        );
    }

    assert!(
        ctx.global_deadline.is_none(),
        "the caller's Context must not carry a deadline the executor computed"
    );
}

#[tokio::test]
async fn control_the_budget_is_actually_enforced_at_this_size() {
    // Control for the test above: "all three calls passed" must not be
    // explicable by the budget never being enforced. At 100 ms the same
    // 150 ms module must be refused.
    let client = client_with_timeout(100);

    let result = client.call("probe.slow", json!({}), None, None).await;
    assert!(
        result.is_err(),
        "a 150ms module must be refused under a 100ms budget: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// D-102 — a deserialized Context recomputes UNCONDITIONALLY
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_context_off_the_wire_recomputes_despite_a_non_empty_call_chain() {
    let client = client_with_timeout(50);

    // A Context arriving from another process: non-empty `call_chain` by
    // definition, and no `global_deadline` (it does not serialize).
    let wire = Context::<Value>::anonymous()
        .child("upstream.caller")
        .serialize();
    let ctx: Context<Value> = Context::deserialize(wire).expect("deserialize");
    assert!(
        !ctx.call_chain.is_empty(),
        "precondition: a cross-process Context is not a root call"
    );
    assert!(
        ctx.global_deadline.is_none(),
        "precondition: `global_deadline` does not cross the wire"
    );

    let result = client.call("probe.slow", json!({}), Some(&ctx), None).await;
    assert!(
        result.is_err(),
        "the receiving executor must recompute the budget for a cross-process \
         sub-tree; a non-empty call_chain must not gate it: {result:?}"
    );
}
