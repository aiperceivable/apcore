//! Tests for CancelToken — cooperative cancellation primitives.

use apcore::cancel::CancelToken;

#[test]
fn test_new_token_is_not_cancelled() {
    let token = CancelToken::new();
    assert!(!token.is_cancelled());
}

#[test]
fn test_cancel_sets_flag() {
    let token = CancelToken::new();
    token.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn test_cancel_is_idempotent() {
    let token = CancelToken::new();
    token.cancel();
    token.cancel(); // second call must not panic
    assert!(token.is_cancelled());
}

#[test]
fn test_clone_shares_state() {
    let token = CancelToken::new();
    let clone = token.clone();

    assert!(!clone.is_cancelled());
    token.cancel();
    // clone sees the same cancellation
    assert!(clone.is_cancelled());
}

#[test]
fn test_clone_cancels_original() {
    let token = CancelToken::new();
    let clone = token.clone();

    clone.cancel();
    assert!(token.is_cancelled());
}

#[test]
fn test_default_is_not_cancelled() {
    let token = CancelToken::default();
    assert!(!token.is_cancelled());
}

#[test]
fn test_multiple_clones_share_state() {
    let t1 = CancelToken::new();
    let t2 = t1.clone();
    let t3 = t2.clone();

    assert!(!t3.is_cancelled());
    t1.cancel();
    assert!(t2.is_cancelled());
    assert!(t3.is_cancelled());
}

// ---------------------------------------------------------------------------
// Sync CANCEL-001 — typed Result from check()
// ---------------------------------------------------------------------------

#[test]
fn test_check_returns_typed_execution_cancelled_error() {
    use apcore::cancel::ExecutionCancelledError;
    use apcore::errors::{ErrorCode, ModuleError};

    let token = CancelToken::new();
    // Before cancellation: Ok.
    assert!(token.check().is_ok());

    token.cancel();
    // After cancellation: a typed ExecutionCancelledError that we can match on
    // — pattern proves the return type is `Result<(), ExecutionCancelledError>`.
    match token.check() {
        Err(ExecutionCancelledError {
            ref message,
            ref module_id,
        }) => {
            assert!(!message.is_empty());
            // A bare `check()` knows no module, so it reports none rather than
            // fabricating the "@unknown" sentinel. Parity with apcore-python
            // and apcore-typescript, whose `check()` produces `details == {}`.
            assert!(module_id.is_none());
        }
        Ok(()) => panic!("expected typed cancel error after cancel()"),
    }

    // Widening back to ModuleError must work via the From impl.
    let cancelled = token.check().unwrap_err();
    let me: ModuleError = cancelled.into();
    assert_eq!(me.code, ErrorCode::ExecutionCancelled);
}

#[test]
fn test_check_for_carries_module_id() {
    use apcore::cancel::ExecutionCancelledError;

    let token = CancelToken::new();
    token.cancel();
    let err: ExecutionCancelledError = token.check_for("ns.target").unwrap_err();
    assert_eq!(err.module_id.as_deref(), Some("ns.target"));
}

#[test]
fn test_check_produces_no_module_id_detail() {
    // PROTOCOL_SPEC names no "@unknown" module. `check()` used to invent it and
    // write it into `details`, so an external caller following the spec's own
    // Rust example emitted a wire payload no other SDK emits — apcore-python and
    // apcore-typescript both produce `details == {}` here.
    use apcore::errors::ModuleError;

    let token = CancelToken::new();
    token.cancel();
    let err: ModuleError = token.check().unwrap_err().into();
    assert!(
        err.details.is_empty(),
        "check() must not fabricate a module_id: {:?}",
        err.details
    );

    // check_for() still carries it.
    let err: ModuleError = token.check_for("ns.target").unwrap_err().into();
    assert_eq!(
        err.details.get("module_id").and_then(|v| v.as_str()),
        Some("ns.target")
    );
}

// ---------------------------------------------------------------------------
// D-90 (spec v1.49.0) — `reset()` must not substitute the cancellation handle
// ---------------------------------------------------------------------------
//
// This SDK is one of the decision's two AUTHORITIES and had no test for it.
// The defect the decision is about is apcore-typescript's: `reset()` installed
// a fresh `AbortController`, so a consumer holding the pre-reset handle was
// permanently detached and a later `cancel()` could not reach it — invisible to
// cooperative checkers, which read the current handle and report what the
// caller expects.
//
// Here the handle is the `Arc<AtomicBool>` a `CancelToken` clone shares.
//
// The substitution itself is NOT expressible today: `reset(&self)` takes a
// shared reference, and moving the `Arc` behind an interior-mutable cell does
// not reproduce the defect either — every clone reads through the same cell, so
// swapping what it holds is visible to all of them rather than detaching any.
// Reproducing it needs per-clone state, i.e. `reset(&mut self)` replacing the
// field, which is a signature change. These tests therefore stand against that
// future refactor and pin the OBSERVABLE contract apcore-typescript had to be
// changed to match.
//
// They are verified red by the one detachment that IS expressible: a `Clone`
// impl returning `Self::new()`. All three go red, which is what establishes
// that they rest on genuine handle sharing rather than on two tokens that
// happen to agree.

#[test]
fn a_clone_taken_before_reset_still_observes_a_later_cancel() {
    let token = CancelToken::new();
    let held_by_a_module = token.clone();

    token.reset();
    token.cancel();

    assert!(
        held_by_a_module.is_cancelled(),
        "a clone taken before reset must not be detached from the token"
    );
    assert!(held_by_a_module.check().is_err());
}

#[test]
fn reset_clears_the_flag_for_every_holder() {
    let token = CancelToken::new();
    let held_by_a_module = token.clone();

    token.cancel();
    assert!(held_by_a_module.is_cancelled());

    token.reset();
    assert!(
        !held_by_a_module.is_cancelled(),
        "the cooperative flag is shared, so a reset is visible to every holder"
    );
    assert!(held_by_a_module.check().is_ok());
}

#[test]
fn control_a_clone_is_not_an_independent_token() {
    // Without this, both tests above would also pass for a `clone()` that
    // produced a fresh, unrelated token: it would be un-cancelled after the
    // reset and un-cancelled after the cancel, satisfying neither assertion by
    // sharing anything. Cancel through the CLONE and observe it on the
    // original, which only a shared handle can do.
    let token = CancelToken::new();
    let clone = token.clone();

    clone.cancel();
    assert!(
        token.is_cancelled(),
        "cancelling through a clone must reach the original"
    );
}
