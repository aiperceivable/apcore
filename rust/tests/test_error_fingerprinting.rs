//! Issue #43 §4 — Error fingerprinting via `&ModuleError`.
//!
//! `compute_fingerprint_from_error` builds a fingerprint from an error code,
//! a module_id, and the sanitized message template (UUIDs / hex IDs /
//! digit-runs ≥4 are normalized away), so two errors that differ only in
//! ephemeral identifiers share a fingerprint and dedupe in ErrorHistory.

use apcore::errors::{ErrorCode, ModuleError};
use apcore::observability::error_history::{compute_fingerprint_from_error, ErrorHistory};

#[test]
fn fingerprint_dedupes_uuid_bearing_messages() {
    let module_id = "executor.auth";
    let err_a = ModuleError::new(
        ErrorCode::ModuleExecuteError,
        "token a1b2c3d4-e5f6-7890-abcd-ef1234567890 expired",
    );
    let err_b = ModuleError::new(
        ErrorCode::ModuleExecuteError,
        "token deadbeef-0000-1111-2222-cafebabe0000 expired",
    );

    let fp_a = compute_fingerprint_from_error(&err_a, module_id);
    let fp_b = compute_fingerprint_from_error(&err_b, module_id);
    assert_eq!(
        fp_a, fp_b,
        "UUID-bearing messages should share a fingerprint"
    );
    assert_eq!(fp_a.len(), 64);
    assert!(fp_a.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn fingerprint_distinguishes_different_codes() {
    let module_id = "executor.auth";
    let err_a = ModuleError::new(ErrorCode::ModuleExecuteError, "boom");
    let err_b = ModuleError::new(ErrorCode::ModuleTimeout, "boom");
    assert_ne!(
        compute_fingerprint_from_error(&err_a, module_id),
        compute_fingerprint_from_error(&err_b, module_id),
        "different error codes must produce different fingerprints"
    );
}

#[tokio::test]
async fn error_history_dedupes_uuid_bearing_messages() {
    let history = ErrorHistory::with_limits(50, 1000);
    let module_id = "executor.payments";

    history.record(
        module_id,
        &ModuleError::new(
            ErrorCode::ModuleExecuteError,
            "txn a1b2c3d4-e5f6-7890-abcd-ef1234567890 failed",
        ),
    );
    history.record(
        module_id,
        &ModuleError::new(
            ErrorCode::ModuleExecuteError,
            "txn 00000000-1111-2222-3333-444444444444 failed",
        ),
    );

    let entries = history.get(module_id, None);
    assert_eq!(
        entries.len(),
        1,
        "two UUID-only-different errors should dedupe to one entry, got {entries:#?}"
    );
    assert_eq!(entries[0].count, 2, "dedup should bump count to 2");
}

#[test]
fn fingerprint_normalizes_long_digit_runs() {
    let module_id = "executor.queue";
    let err_a = ModuleError::new(ErrorCode::ModuleExecuteError, "retry after 30000 ms");
    let err_b = ModuleError::new(ErrorCode::ModuleExecuteError, "retry after 99999 ms");
    assert_eq!(
        compute_fingerprint_from_error(&err_a, module_id),
        compute_fingerprint_from_error(&err_b, module_id),
        "digit runs ≥4 should normalize to identical fingerprints"
    );
}
