//! Conformance driver for `async_task_cancellation.json` (sync findings
//! A-D-003 / A-D-004).
//!
//! - A-D-003: submitting beyond `max_tasks` MUST raise the typed
//!   `TASK_LIMIT_EXCEEDED` error (catchable by code, not a bare error).
//! - A-D-004: cancelling a task while it is in retry backoff MUST stop further
//!   retry attempts and end in CANCELLED — expressed as a deterministic
//!   invariant on attempt count + final status, NOT a latency assertion.
//!
//! Both cases are driven by the fixture's per-case `action`.
#![allow(clippy::pedantic)] // fixture-driven test file: casts/layout follow the fixture schema

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use apcore::async_task::{AsyncTaskManager, RetryConfig, TaskStatus};
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::registry::registry::Registry;
use apcore::Executor;
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

use crate::conformance_env::find_fixtures_root;

fn load_fixture() -> Value {
    let path = find_fixtures_root().join("async_task_cancellation.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read fixture: {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("Invalid JSON: {e}"))
}

fn fixture_case<'a>(fixture: &'a Value, id: &str) -> &'a Value {
    fixture["test_cases"]
        .as_array()
        .expect("test_cases must be an array")
        .iter()
        .find(|c| c["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("fixture missing test case {id}"))
}

// ---------------------------------------------------------------------------
// Test modules
// ---------------------------------------------------------------------------

/// A module that runs "forever" (until cancelled) so it occupies its slot,
/// letting a second submit hit the capacity limit.
struct LongRunningModule;

#[async_trait]
impl Module for LongRunningModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "long-running module that occupies a task slot"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        // Sleep long enough that the slot is still occupied when the second
        // submit is attempted.
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(json!({"done": true}))
    }
}

/// A module that always fails (to trigger retries), incrementing a shared
/// attempt counter on every invocation so the test can observe attempts.
struct AlwaysFailingModule {
    attempts: Arc<AtomicUsize>,
}

#[async_trait]
impl Module for AlwaysFailingModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "module that always errors, with an observable attempt counter"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(ModuleError::new(
            ErrorCode::GeneralInternalError,
            "conformance: always-failing module",
        ))
    }
}

// ---------------------------------------------------------------------------
// A-D-003: submit over capacity raises TASK_LIMIT_EXCEEDED
// ---------------------------------------------------------------------------

#[tokio::test]
async fn submit_over_capacity_raises_task_limit_exceeded() {
    let fixture = load_fixture();
    let tc = fixture_case(&fixture, "submit_over_capacity_raises_task_limit_exceeded");

    let max_tasks = tc["max_tasks"].as_u64().expect("max_tasks") as usize;
    let max_concurrent = tc["max_concurrent"].as_u64().expect("max_concurrent") as usize;
    let submit_count = tc["submit_count"].as_u64().expect("submit_count");
    let expected_error = tc["expected_error"].as_str().expect("expected_error");
    assert_eq!(
        expected_error, "TASK_LIMIT_EXCEEDED",
        "fixture contract drift: expected_error must be TASK_LIMIT_EXCEEDED"
    );

    let registry = Arc::new(Registry::new());
    registry
        .register_module("test.long_running", Box::new(LongRunningModule))
        .unwrap();
    let executor = Arc::new(Executor::new(registry, Arc::new(Config::default())));
    let mgr = AsyncTaskManager::new(executor, max_concurrent, max_tasks);

    // First submit occupies the single slot.
    let first = mgr.submit("test.long_running", json!({}), None).await;
    assert!(
        first.is_ok(),
        "first submit must succeed; got {:?}",
        first.err()
    );

    // Submit the remaining (second) task(s); the over-capacity submit MUST
    // raise the typed TASK_LIMIT_EXCEEDED error.
    let mut last_err: Option<ModuleError> = None;
    for _ in 1..submit_count {
        match mgr.submit("test.long_running", json!({}), None).await {
            Ok(_) => {
                panic!("submit beyond max_tasks={max_tasks} must be rejected, but it succeeded")
            }
            Err(e) => last_err = Some(e),
        }
    }

    let err = last_err.expect("over-capacity submit must produce an error");
    assert_eq!(
        err.code,
        ErrorCode::TaskLimitExceeded,
        "over-capacity submit must raise the typed TASK_LIMIT_EXCEEDED error, got {:?}",
        err.code
    );

    // Tidy up the long-running background task.
    mgr.shutdown().await.expect("store shutdown");
}

// ---------------------------------------------------------------------------
// A-D-004: cancel during backoff stops further retries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_during_backoff_stops_further_retries() {
    let fixture = load_fixture();
    let tc = fixture_case(&fixture, "cancel_during_backoff_stops_further_retries");

    let max_retries = tc["max_retries"].as_u64().expect("max_retries") as u32;
    let retry_delay_ms = tc["retry_delay_ms"].as_u64().expect("retry_delay_ms");
    let backoff_multiplier = tc["backoff_multiplier"]
        .as_f64()
        .expect("backoff_multiplier");
    let expected_final_status = tc["expected_final_status"]
        .as_str()
        .expect("expected_final_status");
    assert_eq!(
        expected_final_status, "cancelled",
        "fixture contract drift: expected_final_status must be cancelled"
    );

    let attempts = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(Registry::new());
    registry
        .register_module(
            "test.always_fail",
            Box::new(AlwaysFailingModule {
                attempts: Arc::clone(&attempts),
            }),
        )
        .unwrap();
    let executor = Arc::new(Executor::new(registry, Arc::new(Config::default())));
    let mgr = AsyncTaskManager::new(executor, /*max_concurrent=*/ 4, /*max_tasks=*/ 4);

    // `RetryConfig` is `#[non_exhaustive]`; build from default and set fields.
    let mut retry = RetryConfig::default();
    retry.max_retries = max_retries;
    retry.retry_delay_ms = retry_delay_ms;
    retry.backoff_multiplier = backoff_multiplier;

    let task_id = mgr
        .submit_with_retry("test.always_fail", json!({}), None, Some(retry))
        .await
        .expect("submit_with_retry must succeed");

    // Wait for the FIRST failure: the module body has run exactly once and the
    // task is now sleeping in its (>= retry_delay_ms) backoff window. Poll the
    // attempt counter rather than asserting on timing.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if attempts.load(Ordering::SeqCst) >= 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "module never executed its first attempt"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Record the attempt count at the moment of cancel, then cancel while the
    // task is in backoff (retry_delay_ms is large enough to guarantee the
    // window is still open).
    let attempts_at_cancel = attempts.load(Ordering::SeqCst);
    let cancelled = mgr.cancel(&task_id).await.expect("store write");
    assert!(cancelled, "cancel() must report success for an active task");

    // Give the runtime ample time to honor the cancel and (in a buggy SDK)
    // run any further retry attempt. The deterministic invariant: no further
    // attempt may start after the cancel.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let attempts_after = attempts.load(Ordering::SeqCst);
    assert_eq!(
        attempts_after, attempts_at_cancel,
        "cancelling during backoff MUST stop further retry attempts: \
         attempts went from {attempts_at_cancel} to {attempts_after}"
    );

    let info = mgr
        .get_status(&task_id)
        .expect("store read")
        .expect("task status must be retrievable");
    assert_eq!(
        info.status,
        TaskStatus::Cancelled,
        "cancelled task must end in CANCELLED, got {:?}",
        info.status
    );
}
