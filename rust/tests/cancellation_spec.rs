//! Spec-traced contract tests for the cancellation feature (Rust SDK).
//!
//! Mirrors the canonical Python suite
//! `apcore-python/tests/test_cancellation_spec.py` and the feature spec
//! `apcore/docs/features/cancellation.md`.
//!
//! The spec declares 2 `## Contract:` blocks:
//!   - CancelToken.cancel
//!   - CancelToken.raise_if_cancelled
//!
//! Each test fn carries a verbatim clause id of the form
//! `cancellation.<method>.<kind>.<detail>` in a leading `// clause:` comment so
//! cross-language diffs line up.
//!
//! `CancelToken::raise_if_cancelled` now exists (`src/cancel.rs`) as the
//! spec's canonical name for the same behavior `check()` already had —
//! `check()` and `check_for()` are unchanged and still used internally
//! throughout this crate.

use apcore::cancel::{CancelToken, ExecutionCancelledError};
use apcore::errors::{ErrorCode, ModuleError};

// ---------------------------------------------------------------------------
// Contract: CancelToken.cancel
// ---------------------------------------------------------------------------

// clause: cancellation.cancel.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_cancel_property_thread_safe() {
    // Launch >=8 concurrent cancel() calls on distinct tokens; assert no panic
    // and every token ends in a consistent cancelled state.
    let tokens: Vec<CancelToken> = (0..16).map(|_| CancelToken::new()).collect();

    let mut handles = Vec::new();
    for tok in &tokens {
        let tok = tok.clone();
        handles.push(tokio::spawn(async move {
            // Yield so calls genuinely interleave across worker threads.
            tokio::task::yield_now().await;
            tok.cancel();
        }));
    }

    for h in handles {
        h.await.expect("cancel task must not panic");
    }

    // Final state must be consistent: all tokens cancelled.
    assert!(tokens.iter().all(CancelToken::is_cancelled));
}

// clause: cancellation.cancel.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_cancel_property_thread_safe_shared_token() {
    // Concurrent cancel() of a single shared token from many tasks must
    // converge to exactly one consistent cancelled state with no panic.
    let shared = CancelToken::new();

    let mut handles = Vec::new();
    for _ in 0..16 {
        let tok = shared.clone();
        handles.push(tokio::spawn(async move {
            tokio::task::yield_now().await;
            tok.cancel();
        }));
    }

    for h in handles {
        h.await.expect("shared cancel task must not panic");
    }

    assert!(shared.is_cancelled());
}

// clause: cancellation.cancel.property.idempotent
#[test]
fn cancellation_cancel_property_idempotent() {
    // Call cancel() twice; assert identical observable outcome and state
    // (is_cancelled stays true, no panic). check() behaves identically after.
    let token = CancelToken::new();

    token.cancel();
    let first_state = token.is_cancelled();
    token.cancel(); // Second call must be a safe no-op.
    let second_state = token.is_cancelled();

    assert!(first_state);
    assert!(second_state);
    assert_eq!(first_state, second_state);

    // check() must still report cancellation after the repeated cancel.
    assert!(token.check().is_err());
}

// ---------------------------------------------------------------------------
// Contract: CancelToken.raise_if_cancelled
// ---------------------------------------------------------------------------

// clause: cancellation.raise_if_cancelled.error.EXECUTION_CANCELLED
#[test]
fn cancellation_raise_if_cancelled_error_execution_cancelled() {
    // Not cancelled: Ok(()).
    let token = CancelToken::new();
    assert!(token.raise_if_cancelled().is_ok());

    // Cancelled: Err(ExecutionCancelledError(code=EXECUTION_CANCELLED)).
    token.cancel();
    let err: ExecutionCancelledError = token
        .raise_if_cancelled()
        .expect_err("expected cancel error");
    assert!(!err.message.is_empty());

    let module_err: ModuleError = err.into();
    assert_eq!(module_err.code, ErrorCode::ExecutionCancelled);
    let code_str = serde_json::to_value(module_err.code).expect("serialize code");
    assert_eq!(
        code_str,
        serde_json::Value::String("EXECUTION_CANCELLED".to_string())
    );
}

// clause: cancellation.raise_if_cancelled.property.thread_safe
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_raise_if_cancelled_property_thread_safe() {
    // Concurrent raise_if_cancelled() calls on a shared token must not panic
    // and must all observe the same cancelled state once cancel() has been
    // observed, mirroring cancellation_cancel_property_thread_safe_shared_token.
    let shared = CancelToken::new();
    shared.cancel();

    let mut handles = Vec::new();
    for _ in 0..16 {
        let tok = shared.clone();
        handles.push(tokio::spawn(async move {
            tokio::task::yield_now().await;
            tok.raise_if_cancelled()
        }));
    }

    for h in handles {
        let result = h.await.expect("raise_if_cancelled task must not panic");
        assert!(
            result.is_err(),
            "every call must observe the cancelled state"
        );
    }
}

// clause: cancellation.raise_if_cancelled.property.pure
#[test]
fn cancellation_raise_if_cancelled_property_pure() {
    // Calling raise_if_cancelled() repeatedly must not itself mutate state —
    // only checks internal cancelled state (spec: "no side effects").
    let token = CancelToken::new();
    assert!(token.raise_if_cancelled().is_ok());
    assert!(
        token.raise_if_cancelled().is_ok(),
        "no side effects: repeated calls are stable"
    );
    assert!(!token.is_cancelled());

    token.cancel();
    assert!(token.raise_if_cancelled().is_err());
    assert!(
        token.raise_if_cancelled().is_err(),
        "no side effects: repeated calls after cancel are stable"
    );
    assert!(token.is_cancelled());
}

// ---------------------------------------------------------------------------
// Regression: check() — raise_if_cancelled's original name in this crate —
// must keep emitting the spec'd error type/code too, since it is unchanged
// and still used internally throughout this crate.
// ---------------------------------------------------------------------------

// clause: cancellation.raise_if_cancelled.error.EXECUTION_CANCELLED
#[test]
fn cancellation_execution_cancelled_error_code_matches_spec() {
    // The contract requires ExecutionCancelledError(code=EXECUTION_CANCELLED).
    // Verify the error TYPE and CODE field match exactly via the check()
    // path, which raise_if_cancelled delegates to.
    let token = CancelToken::new();
    token.cancel();

    // Typed error variant proves the declared error type exists.
    let err: ExecutionCancelledError = token.check().expect_err("expected cancel error");
    assert!(!err.message.is_empty());

    // Widen to ModuleError and assert the code matches the spec exactly.
    let module_err: ModuleError = err.into();
    assert_eq!(module_err.code, ErrorCode::ExecutionCancelled);

    // The wire-string code MUST be exactly "EXECUTION_CANCELLED"
    // (SCREAMING_SNAKE_CASE serialization of the ErrorCode variant).
    let code_str = serde_json::to_value(module_err.code).expect("serialize code");
    assert_eq!(
        code_str,
        serde_json::Value::String("EXECUTION_CANCELLED".to_string())
    );
}
