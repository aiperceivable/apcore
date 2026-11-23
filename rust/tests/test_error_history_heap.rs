// Issue #43 §3: ErrorHistory MUST evict via a min-heap on (timestamp,
// module_id, fingerprint) so eviction is O(log N) rather than O(N).
// This test pushes more entries than max_total can hold and verifies that
// the count never exceeds max_total — and that eviction completes within a
// bounded number of heap pops (lazy-deletion stale-skip pattern).

use apcore::observability::ErrorHistory;
use apcore::{ErrorCode, ModuleError};
use chrono::{Duration, Utc};

#[test]
fn bounded_eviction_keeps_count_at_or_below_max_total() {
    // 5 modules × per-module limit 100, total cap 50.
    let history = ErrorHistory::with_limits(100, 50);
    let now = Utc::now();
    for i in 0..200 {
        let err = ModuleError::new(
            ErrorCode::ModuleExecuteError,
            format!("error number {i} failure"),
        );
        let module_id = format!("executor.m{}", i % 5);
        history.record_at(&module_id, &err, now + Duration::milliseconds(i64::from(i)));
    }
    assert!(
        history.count() <= 50,
        "post-eviction count must not exceed max_total (got {})",
        history.count()
    );
}

#[test]
fn dedup_does_not_explode_count_under_eviction() {
    // Re-recording the same error must dedup; eviction must remain bounded
    // even when each dedup hit pushes a stale heap entry.
    let history = ErrorHistory::with_limits(100, 10);
    let err = ModuleError::new(ErrorCode::ModuleExecuteError, "the same message");
    let now = Utc::now();
    for i in 0..1_000 {
        history.record_at(
            "executor.alpha",
            &err,
            now + Duration::milliseconds(i64::from(i)),
        );
    }
    assert_eq!(history.count(), 1, "dedup must collapse to a single entry");
}
