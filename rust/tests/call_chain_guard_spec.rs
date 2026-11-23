// Spec-traced contract tests for the apcore-rust call-chain-guard feature.
//
// Source spec: apcore/docs/features/call-chain-guard.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_call_chain_guard_spec.py
//
// Each test maps to exactly one clause in the feature spec's '## Contract:
// guard_call_chain' block. The verbatim cross-language clause id appears in a
// leading `// clause: <clause_id>` comment on the line above each test fn so a
// cross-language diff tool can line up the Python / TypeScript / Rust rows by
// that exact string. The fn name is the clause id flattened to snake_case.
// Tests only — production source is never modified here.
//
// SIGNATURE (resolved by D-83, spec v1.49.0):
//   The contract's ### Inputs block lists `module_id`, `call_chain`,
//   `max_call_depth` and `max_module_repeat` — there is NO `context` input; the
//   guard takes the chain directly. apcore-rust now publishes exactly that:
//     guard_call_chain(module_id: &str, call_chain: &[String],
//                      max_call_depth: usize, max_module_repeat: usize)
//   The crate previously shipped only
//     guard_call_chain(ctx: &Context<Value>, module_name: &str, max_depth: u32)
//   which the spec's own Rust tab did not compile against, and which — because
//   `Context<T>` is generic — no host whose services type differed from
//   `serde_json::Value` could call at all. Two Context-taking wrappers remain,
//   now generic over that type parameter: `guard_call_chain_for_context`
//   (applies DEFAULT_MAX_CALL_DEPTH / DEFAULT_MAX_MODULE_REPEAT) and
//   `guard_call_chain_with_repeat` (explicit limits — what the executor's
//   pipeline step uses). Limit floors (max_call_depth >= 1, max_module_repeat
//   >= 1) are validated with GENERAL_INVALID_INPUT, matching Python/TS (D-84).

use apcore::errors::{ErrorCode, ModuleError};
use apcore::utils::{
    guard_call_chain, guard_call_chain_for_context, guard_call_chain_with_repeat,
    DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT,
};
use apcore::Context;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an anonymous context whose `call_chain` is the supplied sequence.
/// In the canonical contract the chain ALREADY includes the current module at
/// the end (appended by `Context::child()` before the guard runs).
fn ctx_with_chain(chain: &[&str]) -> Context<serde_json::Value> {
    let mut ctx = Context::<serde_json::Value>::anonymous();
    ctx.call_chain = chain.iter().map(|s| String::from(*s)).collect();
    ctx
}

/// Exact wire-string code emitted by Rust for a `ModuleError`, mirroring the
/// Python `err.code == "CALL_DEPTH_EXCEEDED"` string assertion. Rust serializes
/// `ErrorCode` as SCREAMING_SNAKE_CASE via serde.
fn wire_code(err: &ModuleError) -> String {
    match serde_json::to_value(err.code).expect("ErrorCode serializes") {
        serde_json::Value::String(s) => s,
        other => panic!("ErrorCode did not serialize to a string: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// INPUT VALIDATION CLAUSES
// ---------------------------------------------------------------------------

// clause: call_chain_guard.guard_call_chain.input.max_depth.below_one
#[test]
fn input_max_depth_below_one_rejected() {
    // Cross-language contract: max_depth < 1 is rejected as invalid input
    // before any chain inspection (apcore-python `call_chain.py` raises
    // ValueError; apcore-typescript `call-chain.ts` throws). Rust returns
    // GENERAL_INVALID_INPUT. The rejection is unconditional — it fires even on
    // an empty chain where the depth check would otherwise pass.
    let empty = Context::<serde_json::Value>::anonymous();
    let err = guard_call_chain_with_repeat(&empty, "a", 0, DEFAULT_MAX_MODULE_REPEAT)
        .expect_err("max_depth=0 is invalid input");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);

    let ctx = ctx_with_chain(&["a"]);
    let err = guard_call_chain_with_repeat(&ctx, "a", 0, DEFAULT_MAX_MODULE_REPEAT)
        .expect_err("max_depth=0 is invalid input");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: call_chain_guard.guard_call_chain.input.max_repeat.below_one
#[test]
fn input_max_repeat_below_one_rejected() {
    // Cross-language contract: max_module_repeat < 1 is rejected as invalid
    // input before any chain inspection (apcore-python / apcore-typescript both
    // raise). Rust returns GENERAL_INVALID_INPUT — not the frequency guard.
    let ctx = ctx_with_chain(&["a"]);
    let err = guard_call_chain_with_repeat(&ctx, "a", 100, 0)
        .expect_err("max_module_repeat=0 is invalid input");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

// clause: call_chain_guard.guard_call_chain.input.module_id.required
#[test]
fn input_module_id_required() {
    // Python: `module_id` is a required positional param (omitting -> TypeError).
    // Rust: `module_name: &str` is a required positional argument enforced by
    // the type system at COMPILE time — a call cannot omit it. We assert the
    // required argument is honoured: a valid module_name produces a defined
    // outcome on a well-formed chain (no panic, Ok).
    let ctx = ctx_with_chain(&["a", "b", "c"]);
    let max_depth = DEFAULT_MAX_CALL_DEPTH;
    let result = guard_call_chain_with_repeat(&ctx, "c", max_depth, DEFAULT_MAX_MODULE_REPEAT);
    assert!(result.is_ok(), "valid required module_name -> Ok");
}

// clause: call_chain_guard.guard_call_chain.input.call_chain.required
#[test]
fn input_call_chain_required() {
    // D-83: the published signature takes the chain directly, with no
    // `Context` and no type parameter. Assert the chain is genuinely
    // consulted: two different chains yield different outcomes for the same
    // module_id.
    let ok_chain: Vec<String> = ["a", "b", "c"].iter().map(|s| (*s).to_string()).collect();
    assert!(guard_call_chain(
        "c",
        &ok_chain,
        DEFAULT_MAX_CALL_DEPTH,
        DEFAULT_MAX_MODULE_REPEAT
    )
    .is_ok());

    let cyclic: Vec<String> = ["c", "b", "c"].iter().map(|s| (*s).to_string()).collect();
    let err = guard_call_chain(
        "c",
        &cyclic,
        DEFAULT_MAX_CALL_DEPTH,
        DEFAULT_MAX_MODULE_REPEAT,
    )
    .expect_err("c -> b -> c is a cycle");
    assert_eq!(err.code, ErrorCode::CircularCall);
}

// clause: call_chain_guard.guard_call_chain.input.generic_free_entry_point
#[test]
fn input_generic_free_entry_point() {
    // D-83: the whole point of the chain-taking form is that a host whose
    // services type is NOT serde_json::Value can reach the guard. Drive it
    // from such a context to prove the reachability the old signature denied.
    #[derive(Clone, Default)]
    struct HostServices {
        _db: &'static str,
    }

    let mut ctx = Context::<HostServices>::create(
        None,
        None,
        None,
        None,
        HostServices { _db: "postgres" },
        None,
    );
    ctx.call_chain = ["a", "b", "a"].iter().map(|s| (*s).to_string()).collect();

    // Directly, via the published signature...
    let err = guard_call_chain(
        "a",
        &ctx.call_chain,
        DEFAULT_MAX_CALL_DEPTH,
        DEFAULT_MAX_MODULE_REPEAT,
    )
    .expect_err("a -> b -> a is a cycle");
    assert_eq!(err.code, ErrorCode::CircularCall);

    // ...and through the Context wrappers, which are generic over T.
    assert_eq!(
        guard_call_chain_for_context(&ctx, "a")
            .expect_err("wrapper sees the same cycle")
            .code,
        ErrorCode::CircularCall
    );
    assert_eq!(
        guard_call_chain_with_repeat(&ctx, "a", DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT)
            .expect_err("wrapper sees the same cycle")
            .code,
        ErrorCode::CircularCall
    );
}

// clause: call_chain_guard.guard_call_chain.input.defaults_applied
#[test]
fn input_defaults_applied() {
    // D-83: DEFAULT_MAX_CALL_DEPTH was a `pub const` the guard never
    // consulted. `guard_call_chain_for_context` is the form that applies it,
    // so the documented default is now the enforced one.
    assert_eq!(DEFAULT_MAX_CALL_DEPTH, 32);

    let mut ok = Context::<serde_json::Value>::anonymous();
    ok.call_chain = (0..32).map(|i| format!("mod.{i}")).collect();
    assert!(guard_call_chain_for_context(&ok, "mod.31").is_ok());

    let mut over = Context::<serde_json::Value>::anonymous();
    over.call_chain = (0..33).map(|i| format!("mod.{i}")).collect();
    assert_eq!(
        guard_call_chain_for_context(&over, "mod.32")
            .expect_err("33 > DEFAULT_MAX_CALL_DEPTH")
            .code,
        ErrorCode::CallDepthExceeded
    );

    // And DEFAULT_MAX_MODULE_REPEAT likewise.
    assert_eq!(DEFAULT_MAX_MODULE_REPEAT, 3);
    let mut repeated = Context::<serde_json::Value>::anonymous();
    repeated.call_chain = (0..4).map(|_| "mod.a".to_string()).collect();
    assert_eq!(
        guard_call_chain_for_context(&repeated, "mod.a")
            .expect_err("4 > DEFAULT_MAX_MODULE_REPEAT")
            .code,
        ErrorCode::CallFrequencyExceeded
    );
}

// clause: call_chain_guard.guard_call_chain.input.context.required
#[test]
fn input_context_required() {
    // The Context-taking wrappers remain, and read their decision from the
    // supplied context: two different contexts yield different outcomes for
    // the same module_id.
    let ok_ctx = ctx_with_chain(&["a", "b", "c"]);
    assert!(guard_call_chain_with_repeat(&ok_ctx, "c", 100, DEFAULT_MAX_MODULE_REPEAT).is_ok());

    // A circular context for the same module_name now fails -> proves the
    // guard reads its decision from the supplied context.
    let circular_ctx = ctx_with_chain(&["c", "b", "c"]);
    let err = guard_call_chain_with_repeat(&circular_ctx, "c", 100, DEFAULT_MAX_MODULE_REPEAT)
        .expect_err("cycle via context");
    assert_eq!(err.code, ErrorCode::CircularCall);
}

// ---------------------------------------------------------------------------
// ERROR CLAUSES
// ---------------------------------------------------------------------------

// clause: call_chain_guard.guard_call_chain.error.CALL_DEPTH_EXCEEDED
#[test]
fn error_call_depth_exceeded() {
    // Chain longer than max_depth -> CallDepthExceeded with exact wire code.
    let chain: Vec<String> = (0..6).map(|i| format!("mod.{i}")).collect();
    let mut ctx = Context::<serde_json::Value>::anonymous();
    ctx.call_chain = chain;
    let err = guard_call_chain_with_repeat(&ctx, "mod.5", 5, DEFAULT_MAX_MODULE_REPEAT)
        .expect_err("len 6 > max_depth 5");
    assert_eq!(err.code, ErrorCode::CallDepthExceeded);
    assert_eq!(wire_code(&err), "CALL_DEPTH_EXCEEDED");
    // Rust carries the depth/limit in the message rather than typed `details`
    // fields; assert the offending numbers are surfaced to the caller.
    assert!(err.message.contains('6'), "message reports current depth 6");
    assert!(err.message.contains('5'), "message reports max_depth 5");
}

// clause: call_chain_guard.guard_call_chain.error.CIRCULAR_CALL
#[test]
fn error_circular_call() {
    // A strict cycle of length >= 2 (A->B->A) -> CircularCall with exact code.
    let ctx = ctx_with_chain(&["a", "b", "a"]);
    let err = guard_call_chain_with_repeat(&ctx, "a", 100, DEFAULT_MAX_MODULE_REPEAT)
        .expect_err("A->B->A cycle");
    assert_eq!(err.code, ErrorCode::CircularCall);
    assert_eq!(wire_code(&err), "CIRCULAR_CALL");
    // The offending module_id is surfaced in the message.
    assert!(
        err.message.contains("'a'"),
        "message names the cycling module"
    );
}

// clause: call_chain_guard.guard_call_chain.error.CALL_FREQUENCY_EXCEEDED
#[test]
fn error_call_frequency_exceeded() {
    // A module appearing more than max_module_repeat times (self-calls, no
    // cycle) -> CallFrequencyExceeded with exact code.
    let ctx = ctx_with_chain(&["a", "a", "a", "a"]);
    let err =
        guard_call_chain_with_repeat(&ctx, "a", 100, 3).expect_err("count 4 > max_module_repeat 3");
    assert_eq!(err.code, ErrorCode::CallFrequencyExceeded);
    assert_eq!(wire_code(&err), "CALL_FREQUENCY_EXCEEDED");
    assert!(err.message.contains('4'), "message reports count 4");
    assert!(err.message.contains('3'), "message reports limit 3");
}

// ---------------------------------------------------------------------------
// ORDERING (Side-Effect-like ordered checks: depth -> circular -> frequency)
// ---------------------------------------------------------------------------

// clause: call_chain_guard.guard_call_chain.side_effect.1.depth_before_circular
#[test]
fn side_effect_1_depth_before_circular() {
    // ["a","b","a"] is circular AND exceeds max_depth=2 (length 3). Depth is
    // checked first, so CallDepthExceeded must win.
    let ctx = ctx_with_chain(&["a", "b", "a"]);
    let err = guard_call_chain_with_repeat(&ctx, "a", 2, DEFAULT_MAX_MODULE_REPEAT)
        .expect_err("depth+circular both violated");
    assert_eq!(err.code, ErrorCode::CallDepthExceeded);
    assert_eq!(wire_code(&err), "CALL_DEPTH_EXCEEDED");
}

// clause: call_chain_guard.guard_call_chain.side_effect.2.circular_before_frequency
#[test]
fn side_effect_2_circular_before_frequency() {
    // A->B->A->B->A: circular (B between repeats) AND "a" repeats 3x over
    // max_module_repeat=2. Circular is checked before frequency.
    let ctx = ctx_with_chain(&["a", "b", "a", "b", "a"]);
    let err = guard_call_chain_with_repeat(&ctx, "a", 100, 2)
        .expect_err("circular+frequency both violated");
    assert_eq!(err.code, ErrorCode::CircularCall);
    assert_eq!(wire_code(&err), "CIRCULAR_CALL");
}

// ---------------------------------------------------------------------------
// PROPERTY CLAUSES
// ---------------------------------------------------------------------------

// clause: call_chain_guard.guard_call_chain.property.async
#[test]
fn property_async() {
    // Contract declares async: false. The Rust guard is a plain synchronous
    // function returning Result<(), ModuleError> with no .await; calling it
    // directly (no executor/runtime) resolves to Ok(()) on a valid chain.
    let ctx = ctx_with_chain(&["a", "b", "c"]);
    let result: Result<(), ModuleError> =
        guard_call_chain_with_repeat(&ctx, "c", 100, DEFAULT_MAX_MODULE_REPEAT);
    assert!(result.is_ok(), "sync call returns Ok(()) with no awaiting");
}

// clause: call_chain_guard.guard_call_chain.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn property_thread_safe() {
    // Contract declares thread_safe: true. Launch >=8 concurrent guard calls
    // with distinct, valid, non-circular chains; none must error and all must
    // return Ok(()), proving no shared-state corruption.
    let mut handles = Vec::new();
    for idx in 0..16u32 {
        handles.push(tokio::spawn(async move {
            let chain: Vec<String> = (0..3).map(|j| format!("mod.{idx}.{j}")).collect();
            let mut ctx = Context::<serde_json::Value>::anonymous();
            ctx.call_chain = chain;
            guard_call_chain_with_repeat(
                &ctx,
                &format!("mod.{idx}.2"),
                100,
                DEFAULT_MAX_MODULE_REPEAT,
            )
        }));
    }

    let mut ok_count = 0usize;
    for handle in handles {
        let result = handle.await.expect("task must not panic");
        assert!(result.is_ok(), "each valid concurrent guard call -> Ok(())");
        ok_count += 1;
    }
    assert_eq!(
        ok_count, 16,
        "all 16 concurrent calls observed consistent state"
    );
}

// clause: call_chain_guard.guard_call_chain.property.pure
#[test]
fn property_pure() {
    // Contract declares pure: true (reads context, does not mutate). The guard
    // takes `&Context` (a shared borrow) so it CANNOT mutate the chain; assert
    // the chain is unchanged across calls and the outcome is stable.
    let ctx = ctx_with_chain(&["a", "b", "c"]);
    let snapshot = ctx.call_chain.clone();

    assert!(guard_call_chain_with_repeat(&ctx, "c", 100, DEFAULT_MAX_MODULE_REPEAT).is_ok());
    assert_eq!(
        ctx.call_chain, snapshot,
        "guard must not mutate the call chain"
    );

    // Second call on identical state -> identical observable outcome.
    assert!(guard_call_chain_with_repeat(&ctx, "c", 100, DEFAULT_MAX_MODULE_REPEAT).is_ok());
    assert_eq!(ctx.call_chain, snapshot);
}

// clause: call_chain_guard.guard_call_chain.property.idempotent
#[test]
fn property_idempotent() {
    // Calling the guard twice with identical inputs yields an identical outcome
    // (same error code) and leaves the input chain unchanged.
    let ctx = ctx_with_chain(&["a", "b", "a"]);
    let snapshot = ctx.call_chain.clone();

    let mut codes = Vec::new();
    for _ in 0..2 {
        let err = guard_call_chain_with_repeat(&ctx, "a", 100, DEFAULT_MAX_MODULE_REPEAT)
            .expect_err("A->B->A cycle");
        codes.push(wire_code(&err));
    }

    assert_eq!(codes, vec!["CIRCULAR_CALL", "CIRCULAR_CALL"]);
    assert_eq!(ctx.call_chain, snapshot);
}

// ---------------------------------------------------------------------------
// DEFAULTS (Configuration clause)
// ---------------------------------------------------------------------------

// clause: call_chain_guard.guard_call_chain.input.max_depth.default
#[test]
fn input_max_depth_default() {
    // DEFAULT_MAX_CALL_DEPTH is 32: a chain of exactly 32 passes, 33 fails.
    assert_eq!(DEFAULT_MAX_CALL_DEPTH, 32);

    let ok_chain: Vec<String> = (0..32).map(|i| format!("mod.{i}")).collect();
    let mut ok_ctx = Context::<serde_json::Value>::anonymous();
    ok_ctx.call_chain = ok_chain;
    let max_depth = DEFAULT_MAX_CALL_DEPTH;
    assert!(
        guard_call_chain_with_repeat(&ok_ctx, "mod.31", max_depth, DEFAULT_MAX_MODULE_REPEAT)
            .is_ok(),
        "chain of exactly 32 is at the limit -> Ok"
    );

    let over_chain: Vec<String> = (0..33).map(|i| format!("mod.{i}")).collect();
    let mut over_ctx = Context::<serde_json::Value>::anonymous();
    over_ctx.call_chain = over_chain;
    let err =
        guard_call_chain_with_repeat(&over_ctx, "mod.32", max_depth, DEFAULT_MAX_MODULE_REPEAT)
            .expect_err("chain of 33 exceeds default depth");
    assert_eq!(err.code, ErrorCode::CallDepthExceeded);
}

// clause: call_chain_guard.guard_call_chain.input.max_repeat.default
#[test]
fn input_max_repeat_default() {
    // DEFAULT_MAX_MODULE_REPEAT is 3: a module appearing exactly 3 times passes,
    // 4 times fails. `guard_call_chain` uses this default internally.
    assert_eq!(DEFAULT_MAX_MODULE_REPEAT, 3);

    // "a" appears 3 times via self-calls (no cycle) -> at limit, Ok.
    let at_limit = ctx_with_chain(&["a", "a", "a"]);
    assert!(
        guard_call_chain_with_repeat(&at_limit, "a", 100, DEFAULT_MAX_MODULE_REPEAT).is_ok(),
        "3 self-calls at the default repeat limit -> Ok"
    );

    let over = ctx_with_chain(&["a", "a", "a", "a"]);
    let err = guard_call_chain_with_repeat(&over, "a", 100, DEFAULT_MAX_MODULE_REPEAT)
        .expect_err("4 self-calls exceeds default");
    assert_eq!(err.code, ErrorCode::CallFrequencyExceeded);
}
