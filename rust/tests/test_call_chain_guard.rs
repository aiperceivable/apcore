//! Tests for call chain guard utilities.
//!
//! Covers `guard_call_chain` and `guard_call_chain_with_repeat` — depth limit,
//! circular call detection, frequency throttle, happy path, and edge cases.

use apcore::errors::ErrorCode;
use apcore::Context;
use apcore::{guard_call_chain_with_repeat, DEFAULT_MAX_MODULE_REPEAT};

fn anon_ctx() -> Context<serde_json::Value> {
    Context::<serde_json::Value>::anonymous()
}

fn ctx_with_chain(chain: Vec<&str>) -> Context<serde_json::Value> {
    let mut ctx = anon_ctx();
    ctx.call_chain = chain.into_iter().map(String::from).collect();
    ctx
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn guard_empty_chain_passes() {
    let ctx = anon_ctx();
    assert!(guard_call_chain_with_repeat(&ctx, "mod.a", 10, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}

#[test]
fn guard_single_module_in_chain_passes() {
    let ctx = ctx_with_chain(vec!["mod.a"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.b", 10, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}

#[test]
fn guard_short_diverse_chain_passes() {
    let ctx = ctx_with_chain(vec!["mod.a", "mod.b"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.c", 10, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}

#[test]
fn guard_module_repeated_below_limit_passes() {
    // mod.a appears twice; default repeat limit is 3, so one more is allowed.
    let ctx = ctx_with_chain(vec!["mod.a", "mod.b", "mod.a"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.c", 100, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}

// ---------------------------------------------------------------------------
// Depth limit exceeded
// ---------------------------------------------------------------------------

#[test]
fn guard_depth_exceeded_returns_error() {
    // chain of 4 entries exceeds max_depth=3
    let ctx = ctx_with_chain(vec!["a", "b", "c", "d"]);
    let result = guard_call_chain_with_repeat(&ctx, "e", 3, DEFAULT_MAX_MODULE_REPEAT);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CallDepthExceeded);
}

#[test]
fn guard_depth_exactly_at_limit_passes() {
    // chain length == max_depth is allowed (guard checks `>`, not `>=`)
    let ctx = ctx_with_chain(vec!["a", "b", "c"]);
    assert!(guard_call_chain_with_repeat(&ctx, "d", 3, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}

#[test]
fn guard_depth_one_above_limit_errors() {
    let ctx = ctx_with_chain(vec!["a", "b", "c", "d"]);
    let result = guard_call_chain_with_repeat(&ctx, "e", 3, DEFAULT_MAX_MODULE_REPEAT);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CallDepthExceeded);
}

#[test]
fn guard_max_depth_one_empty_chain_passes() {
    let ctx = anon_ctx();
    assert!(guard_call_chain_with_repeat(&ctx, "mod.a", 1, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}

#[test]
fn guard_max_depth_one_chain_of_one_passes() {
    // chain length == max_depth (1), no violation
    let ctx = ctx_with_chain(vec!["mod.a"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.b", 1, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}

#[test]
fn guard_max_depth_one_chain_of_two_errors() {
    let ctx = ctx_with_chain(vec!["a", "b"]);
    let result = guard_call_chain_with_repeat(&ctx, "c", 1, DEFAULT_MAX_MODULE_REPEAT);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CallDepthExceeded);
}

// ---------------------------------------------------------------------------
// Circular call detection
// ---------------------------------------------------------------------------

#[test]
fn guard_circular_call_returns_error() {
    // mod.a -> mod.b -> mod.a: circular
    let ctx = ctx_with_chain(vec!["mod.a", "mod.b", "mod.a"]);
    let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, DEFAULT_MAX_MODULE_REPEAT);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CircularCall);
}

#[test]
fn guard_circular_longer_chain_detected() {
    // a -> b -> c -> a is circular when calling a again
    let ctx = ctx_with_chain(vec!["a", "b", "c", "a"]);
    let result = guard_call_chain_with_repeat(&ctx, "a", 100, DEFAULT_MAX_MODULE_REPEAT);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CircularCall);
}

// ---------------------------------------------------------------------------
// Frequency throttle exceeded
// ---------------------------------------------------------------------------

#[test]
fn guard_frequency_at_default_limit_passes() {
    // A-D-040: canonical frequency uses `count > max_module_repeat`. The chain
    // includes the trailing self, so exactly 3 occurrences == default limit 3
    // is allowed (3 > 3 is false).
    let ctx = ctx_with_chain(vec!["mod.a", "mod.a", "mod.a"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.a", 100, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}

#[test]
fn guard_frequency_exceeded_returns_error() {
    // mod.a appears 4 times (consecutive, no cycle) — exceeds default
    // max_module_repeat=3 (count 4 > 3).
    let ctx = ctx_with_chain(vec!["mod.a", "mod.a", "mod.a", "mod.a"]);
    let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, DEFAULT_MAX_MODULE_REPEAT);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CallFrequencyExceeded);
}

#[test]
fn guard_frequency_two_occurrences_passes_with_default_limit() {
    // default limit is 3; two occurrences should not trigger it
    let ctx = ctx_with_chain(vec!["mod.a", "mod.b", "mod.a"]);
    // mod.a at chain end would create a cycle; check a different module
    let ctx2 = ctx_with_chain(vec!["mod.a", "mod.b", "mod.a", "mod.c"]);
    assert!(guard_call_chain_with_repeat(&ctx2, "mod.d", 100, DEFAULT_MAX_MODULE_REPEAT).is_ok());
    // also: mod.a appears twice; calling mod.a next would be the 3rd occurrence (allowed)
    let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, DEFAULT_MAX_MODULE_REPEAT);
    // This may trigger CircularCall (cycle a->b->a) before FrequencyExceeded
    assert!(result.is_err());
    let code = result.unwrap_err().code;
    assert!(
        code == ErrorCode::CircularCall || code == ErrorCode::CallFrequencyExceeded,
        "expected CircularCall or CallFrequencyExceeded, got {code:?}"
    );
}

// ---------------------------------------------------------------------------
// guard_call_chain_with_repeat — custom limits
// ---------------------------------------------------------------------------

#[test]
fn guard_with_repeat_max_repeat_one_single_occurrence_passes() {
    // With max_repeat=1, a single occurrence (count 1) is within the limit
    // (1 > 1 is false). The chain includes the trailing self.
    let ctx = ctx_with_chain(vec!["mod.a"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.a", 100, 1).is_ok());
}

#[test]
fn guard_with_repeat_max_repeat_one_second_occurrence_errors() {
    // With max_repeat=1, two occurrences (count 2 > 1) exceed the limit.
    let ctx = ctx_with_chain(vec!["mod.a", "mod.a"]);
    let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, 1);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CallFrequencyExceeded);
}

#[test]
fn guard_with_repeat_max_repeat_one_different_module_passes() {
    let ctx = ctx_with_chain(vec!["mod.a"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.b", 100, 1).is_ok());
}

#[test]
fn guard_with_repeat_custom_limit_two_passes_at_one_occurrence() {
    let ctx = ctx_with_chain(vec!["mod.a"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.a", 100, 2).is_ok());
}

#[test]
fn guard_with_repeat_custom_limit_two_passes_at_two_occurrences() {
    // count 2 == max_repeat 2 → allowed (2 > 2 is false).
    let ctx = ctx_with_chain(vec!["mod.a", "mod.a"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.a", 100, 2).is_ok());
}

#[test]
fn guard_with_repeat_custom_limit_two_errors_at_three_occurrences() {
    let ctx = ctx_with_chain(vec!["mod.a", "mod.a", "mod.a"]);
    let result = guard_call_chain_with_repeat(&ctx, "mod.a", 100, 2);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CallFrequencyExceeded);
}

#[test]
fn guard_with_repeat_depth_check_still_applies() {
    let ctx = ctx_with_chain(vec!["a", "b"]);
    let result = guard_call_chain_with_repeat(&ctx, "c", 1, 10);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, ErrorCode::CallDepthExceeded);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn guard_max_depth_zero_rejected_as_invalid_input() {
    // max_depth < 1 is rejected as invalid input before any chain inspection,
    // matching apcore-python and apcore-typescript. The rejection is
    // unconditional — it fires even on an empty chain.
    let ctx = anon_ctx();
    let err = guard_call_chain_with_repeat(&ctx, "mod.a", 0, DEFAULT_MAX_MODULE_REPEAT)
        .expect_err("max_depth=0 is invalid input");
    assert_eq!(err.code, ErrorCode::GeneralInvalidInput);
}

#[test]
fn guard_module_name_not_in_chain_passes() {
    let ctx = ctx_with_chain(vec!["x", "y", "z"]);
    assert!(guard_call_chain_with_repeat(&ctx, "mod.new", 100, DEFAULT_MAX_MODULE_REPEAT).is_ok());
}
