//! Integration tests for AsyncTaskManager.
//!
//! These tests exercise the public API exposed through `apcore::` and cover
//! the scenarios described in the conformance spec:
//!   - submit() returns a non-empty UUID task_id
//!   - get_status() returns TaskInfo with correct initial/terminal status
//!   - cancel() a pending task
//!   - cancel() a task that is already terminal (idempotent)
//!   - list_tasks() with and without status filter
//!   - cleanup() removes old completed/cancelled tasks and leaves recent ones
//!   - max_tasks limit enforcement (submit rejected when at capacity)
//!   - max_concurrent limit (tasks stay Pending when semaphore is exhausted)

use apcore::async_task::{AsyncTaskManager, TaskStatus};
use apcore::config::Config;
use apcore::context::{Context, Identity};
use apcore::errors::ModuleError;
use apcore::module::Module;
use apcore::registry::registry::Registry;
use apcore::Executor;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a bare executor backed by an empty registry.
fn make_executor() -> Arc<Executor> {
    let registry = Arc::new(Registry::default());
    let config = Arc::new(Config::default());
    Arc::new(Executor::new(registry, config))
}

/// A module that echoes its input immediately.
struct EchoModule;

#[async_trait]
impl Module for EchoModule {
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn output_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn description(&self) -> &'static str {
        "Echo input"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(inputs)
    }
}

/// Build an executor with an EchoModule registered at the given id.
fn make_executor_with_echo(module_id: &str) -> Arc<Executor> {
    let registry = Arc::new(Registry::default());
    registry
        .register_module(module_id, Box::new(EchoModule))
        .expect("register EchoModule");
    let config = Arc::new(Config::default());
    Arc::new(Executor::new(registry, config))
}

fn _make_ctx() -> Context<Value> {
    Context::new(Identity::new(
        "test".to_string(),
        "Test".to_string(),
        vec![],
        HashMap::new(),
    ))
}

// ---------------------------------------------------------------------------
// submit() — returns a task_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn submit_returns_non_empty_task_id() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    let task_id = mgr
        .submit("any.module", json!({}), None)
        .await
        .expect("submit should succeed");
    assert!(!task_id.is_empty(), "task_id must be a non-empty string");
}

#[tokio::test]
async fn submit_increments_task_count() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    assert_eq!(mgr.task_count().expect("store list"), 0);
    let _ = mgr.submit("m", json!({}), None).await.unwrap();
    assert_eq!(mgr.task_count().expect("store list"), 1);
    let _ = mgr.submit("m", json!({}), None).await.unwrap();
    assert_eq!(mgr.task_count().expect("store list"), 2);
}

#[tokio::test]
async fn submit_task_ids_are_unique() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    let id1 = mgr.submit("m", json!({}), None).await.unwrap();
    let id2 = mgr.submit("m", json!({}), None).await.unwrap();
    assert_ne!(id1, id2, "each submitted task must receive a unique id");
}

// ---------------------------------------------------------------------------
// get_status() — TaskInfo and status progression
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_status_returns_some_after_submit() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();
    // Task exists immediately after submit (may be Pending, Running, or Completed
    // depending on scheduling, but must be present).
    assert!(
        mgr.get_status(&task_id).expect("store read").is_some(),
        "get_status should return Some right after submit"
    );
}

#[tokio::test]
async fn get_status_returns_none_for_unknown_id() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    assert!(mgr
        .get_status("no-such-task")
        .expect("store read")
        .is_none());
}

#[tokio::test]
async fn task_info_contains_correct_module_id() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    let task_id = mgr.submit("echo.module", json!({}), None).await.unwrap();
    let info = mgr.get_status(&task_id).expect("store read").unwrap();
    assert_eq!(info.module_id, "echo.module");
}

#[tokio::test]
async fn task_info_submitted_at_is_set() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();
    let info = mgr.get_status(&task_id).expect("store read").unwrap();
    assert!(
        info.submitted_at > 0.0,
        "submitted_at must be a positive UNIX timestamp"
    );
}

#[tokio::test]
async fn completed_task_has_completed_status() {
    // Use an executor that actually has the module so the task completes.
    let exec = make_executor_with_echo("echo.v1");
    let mgr = AsyncTaskManager::new(exec, 4, 100);
    let task_id = mgr.submit("echo.v1", json!({"x": 1}), None).await.unwrap();

    // Poll for completion (up to 1 second).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let status = mgr
            .get_status(&task_id)
            .expect("store read")
            .unwrap()
            .status;
        if status == TaskStatus::Completed || status == TaskStatus::Failed {
            break;
        }
        assert!(
            std::time::Instant::now() <= deadline,
            "task did not reach a terminal state within 1 second"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let info = mgr.get_status(&task_id).expect("store read").unwrap();
    assert_eq!(
        info.status,
        TaskStatus::Completed,
        "task for a registered module should complete successfully"
    );
    assert!(
        info.completed_at.is_some(),
        "completed_at must be set after completion"
    );
    assert!(
        info.started_at.is_some(),
        "started_at must be set once execution began"
    );
}

#[tokio::test]
async fn failed_task_has_failed_status_and_error_message() {
    // Module is not registered — executor returns ModuleNotFound.
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    let task_id = mgr
        .submit("nonexistent.module", json!({}), None)
        .await
        .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let status = mgr
            .get_status(&task_id)
            .expect("store read")
            .unwrap()
            .status;
        if matches!(status, TaskStatus::Failed | TaskStatus::Completed) {
            break;
        }
        assert!(
            std::time::Instant::now() <= deadline,
            "task did not reach a terminal state within 1 second"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let info = mgr.get_status(&task_id).expect("store read").unwrap();
    assert_eq!(info.status, TaskStatus::Failed);
    assert!(
        info.error.is_some(),
        "failed task must have an error message"
    );
    assert!(
        !info.error.as_ref().unwrap().is_empty(),
        "error message must not be empty"
    );
}

// ---------------------------------------------------------------------------
// cancel() — pending task
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_pending_task_returns_true() {
    // max_concurrent = 0 keeps all tasks in Pending state.
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();

    let result = mgr.cancel(&task_id).await.expect("store write");
    assert!(result, "cancel should return true for a Pending task");
}

#[tokio::test]
async fn cancel_pending_task_sets_cancelled_status() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();
    mgr.cancel(&task_id).await.expect("store write");

    let info = mgr.get_status(&task_id).expect("store read").unwrap();
    assert_eq!(info.status, TaskStatus::Cancelled);
}

#[tokio::test]
async fn cancel_pending_task_sets_completed_at() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();
    mgr.cancel(&task_id).await.expect("store write");

    let info = mgr.get_status(&task_id).expect("store read").unwrap();
    assert!(
        info.completed_at.is_some(),
        "completed_at should be set when task is cancelled"
    );
}

// ---------------------------------------------------------------------------
// cancel() — already terminal task (idempotent)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_already_cancelled_task_returns_false() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();
    assert!(
        mgr.cancel(&task_id).await.expect("store write"),
        "first cancel should succeed"
    );
    assert!(
        !mgr.cancel(&task_id).await.expect("store write"),
        "second cancel on an already-cancelled task should return false"
    );
}

#[tokio::test]
async fn cancel_unknown_task_returns_false() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
    assert!(!mgr.cancel("ghost-task-id").await.expect("store write"));
}

// ---------------------------------------------------------------------------
// cancel() — running task
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_running_task_sets_cancelled_status() {
    // Use max_concurrent = 1 so the task starts running, but point it at a
    // nonexistent module so we have a brief window where it is Running before
    // it either fails or we cancel it. We cancel while it may be Running or
    // Pending; either way the final status must be Cancelled.
    let mgr = AsyncTaskManager::new(make_executor(), 1, 100);
    let task_id = mgr
        .submit("some.module.that.does.not.exist", json!({}), None)
        .await
        .unwrap();

    // Give the tokio runtime a tick to let the task acquire the semaphore and
    // mark itself Running before we cancel.
    tokio::task::yield_now().await;

    // Cancel regardless of whether it transitioned to Running yet.
    let cancelled = mgr.cancel(&task_id).await.expect("store write");

    // The task may already be Failed (module not found) or Cancelled.
    let info = mgr.get_status(&task_id).expect("store read").unwrap();
    if cancelled {
        // cancel() returned true — status must now be Cancelled.
        assert_eq!(info.status, TaskStatus::Cancelled);
    } else {
        // cancel() returned false — task already reached Failed on its own.
        assert_eq!(
            info.status,
            TaskStatus::Failed,
            "if cancel returns false the task should be in a terminal state"
        );
    }
}

// ---------------------------------------------------------------------------
// list_tasks() — with and without filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tasks_without_filter_returns_all() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    assert!(mgr.list_tasks(None).expect("store list").is_empty());

    let id1 = mgr.submit("m1", json!({}), None).await.unwrap();
    let id2 = mgr.submit("m2", json!({}), None).await.unwrap();

    let all = mgr.list_tasks(None).expect("store list");
    assert_eq!(all.len(), 2);
    let ids: Vec<&str> = all.iter().map(|t| t.task_id.as_str()).collect();
    assert!(ids.contains(&id1.as_str()));
    assert!(ids.contains(&id2.as_str()));
}

#[tokio::test]
async fn list_tasks_with_pending_filter_returns_only_pending() {
    // max_concurrent = 0 → all tasks stay Pending.
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let _ = mgr.submit("m", json!({}), None).await.unwrap();
    let _ = mgr.submit("m", json!({}), None).await.unwrap();

    let pending = mgr
        .list_tasks(Some(TaskStatus::Pending))
        .expect("store list");
    assert_eq!(pending.len(), 2, "both tasks should be Pending");

    let completed = mgr
        .list_tasks(Some(TaskStatus::Completed))
        .expect("store list");
    assert!(completed.is_empty(), "no tasks should be Completed yet");
}

#[tokio::test]
async fn list_tasks_with_cancelled_filter_returns_only_cancelled() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let id1 = mgr.submit("m", json!({}), None).await.unwrap();
    let id2 = mgr.submit("m", json!({}), None).await.unwrap();

    mgr.cancel(&id1).await.expect("store write");

    let cancelled = mgr
        .list_tasks(Some(TaskStatus::Cancelled))
        .expect("store list");
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].task_id, id1);

    let pending = mgr
        .list_tasks(Some(TaskStatus::Pending))
        .expect("store list");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].task_id, id2);
}

#[tokio::test]
async fn list_tasks_empty_when_no_tasks_match_filter() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let _ = mgr.submit("m", json!({}), None).await.unwrap();

    // No tasks have been completed — filter should yield nothing.
    let completed = mgr
        .list_tasks(Some(TaskStatus::Completed))
        .expect("store list");
    assert!(completed.is_empty());
}

// ---------------------------------------------------------------------------
// cleanup() — removes old completed tasks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cleanup_removes_cancelled_tasks_past_max_age() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();
    mgr.cancel(&task_id).await.expect("store write");

    // A negative max_age means every task is "old enough."
    let removed = mgr.cleanup(-1.0).expect("store sweep");
    assert_eq!(removed, 1);
    assert!(
        mgr.get_status(&task_id).expect("store read").is_none(),
        "task should be gone"
    );
}

#[tokio::test]
async fn cleanup_keeps_tasks_within_max_age() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();
    mgr.cancel(&task_id).await.expect("store write");

    // Very large max_age — the task was just created, so it is not old enough.
    let removed = mgr.cleanup(9_999_999.0).expect("store sweep");
    assert_eq!(removed, 0);
    assert!(
        mgr.get_status(&task_id).expect("store read").is_some(),
        "task should still exist"
    );
}

#[tokio::test]
async fn cleanup_does_not_remove_active_tasks() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let task_id = mgr.submit("m", json!({}), None).await.unwrap();
    // Task is Pending, not terminal — cleanup with age=-1 must not remove it.
    let removed = mgr.cleanup(-1.0).expect("store sweep");
    assert_eq!(
        removed, 0,
        "active (Pending) tasks must never be cleaned up"
    );
    assert!(mgr.get_status(&task_id).expect("store read").is_some());
}

#[tokio::test]
async fn cleanup_removes_multiple_terminal_tasks() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let id1 = mgr.submit("m", json!({}), None).await.unwrap();
    let id2 = mgr.submit("m", json!({}), None).await.unwrap();
    let id3 = mgr.submit("m", json!({}), None).await.unwrap();

    mgr.cancel(&id1).await.expect("store write");
    mgr.cancel(&id2).await.expect("store write");
    // id3 stays Pending

    let removed = mgr.cleanup(-1.0).expect("store sweep");
    assert_eq!(removed, 2, "only the two cancelled tasks should be removed");
    assert!(
        mgr.get_status(&id3).expect("store read").is_some(),
        "pending task must remain"
    );
}

// ---------------------------------------------------------------------------
// max_tasks limit enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn submit_rejected_at_max_tasks_limit() {
    let mgr = AsyncTaskManager::new(make_executor(), 4, 2); // capacity = 2
    let _ = mgr.submit("m", json!({}), None).await.unwrap();
    let _ = mgr.submit("m", json!({}), None).await.unwrap();

    let err = mgr
        .submit("m", json!({}), None)
        .await
        .expect_err("third submit should be rejected");
    assert!(
        err.to_string().contains("Task limit"),
        "error message should mention task limit; got: {err}"
    );
}

#[tokio::test]
async fn submit_allowed_after_cleanup_frees_space() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 2); // capacity = 2
    let id1 = mgr.submit("m", json!({}), None).await.unwrap();
    let _ = mgr.submit("m", json!({}), None).await.unwrap();

    // At capacity — third submit should fail.
    assert!(mgr.submit("m", json!({}), None).await.is_err());

    // Cancel one task, cleanup it away.
    mgr.cancel(&id1).await.expect("store write");
    mgr.cleanup(-1.0).expect("store sweep");

    // Now there is room for one more.
    assert!(
        mgr.submit("m", json!({}), None).await.is_ok(),
        "submit should succeed once cleanup freed space"
    );
}

// ---------------------------------------------------------------------------
// max_concurrent limit — tasks are queued when all permits are held
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tasks_are_queued_when_max_concurrent_reached() {
    // max_concurrent = 0 means no task can ever acquire the semaphore and
    // start running — all tasks stay Pending indefinitely.
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);

    let id1 = mgr.submit("m", json!({}), None).await.unwrap();
    let id2 = mgr.submit("m", json!({}), None).await.unwrap();

    // Yield so any tokio tasks get a chance to run.
    tokio::task::yield_now().await;

    let s1 = mgr.get_status(&id1).expect("store read").unwrap().status;
    let s2 = mgr.get_status(&id2).expect("store read").unwrap().status;

    assert_eq!(s1, TaskStatus::Pending, "task 1 should be stuck Pending");
    assert_eq!(s2, TaskStatus::Pending, "task 2 should be stuck Pending");
}

#[tokio::test]
async fn max_concurrent_one_limits_parallelism() {
    // With max_concurrent = 1, submit two tasks. One may start running; the
    // other must remain Pending until the first completes or is cancelled.
    let mgr = Arc::new(AsyncTaskManager::new(make_executor(), 1, 100));

    let _id1 = mgr.submit("m1", json!({}), None).await.unwrap();
    let _id2 = mgr.submit("m2", json!({}), None).await.unwrap();

    // The combined count must be exactly 2.
    assert_eq!(mgr.task_count().expect("store list"), 2);

    // At most 1 task should be Running at any point.
    let running = mgr
        .list_tasks(Some(TaskStatus::Running))
        .expect("store list")
        .len();
    assert!(
        running <= 1,
        "at most 1 task should be Running with max_concurrent=1; got {running}"
    );
}

// ---------------------------------------------------------------------------
// shutdown() — cancels all active tasks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shutdown_cancels_all_pending_tasks() {
    let mgr = AsyncTaskManager::new(make_executor(), 0, 100);
    let id1 = mgr.submit("m1", json!({}), None).await.unwrap();
    let id2 = mgr.submit("m2", json!({}), None).await.unwrap();
    mgr.shutdown().await.expect("store shutdown");

    assert_eq!(
        mgr.get_status(&id1).expect("store read").unwrap().status,
        TaskStatus::Cancelled
    );
    assert_eq!(
        mgr.get_status(&id2).expect("store read").unwrap().status,
        TaskStatus::Cancelled
    );
}

// ---------------------------------------------------------------------------
// Regression: concurrent submits must not exceed max_tasks (TOCTOU guard).
// Without the admission_lock around the capacity-check + save in
// `submit_with_retry`, two racing submits can both observe `len < max` and
// both insert, exceeding the cap. With max_concurrent=0 the spawned tasks
// stay Pending so they never finish or get cleaned up — any over-cap insert
// is observable in the final task count.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn submit_max_tasks_holds_under_concurrent_load() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const CAP: usize = 8;
    const SUBMITTERS: usize = 32;

    // max_concurrent = 0 keeps every accepted task Pending — they do not
    // complete and are not eligible for cleanup, so the post-race
    // task_count() reads exactly the number of accepted submits.
    let mgr = Arc::new(AsyncTaskManager::new(make_executor(), 0, CAP));
    let accepted = Arc::new(AtomicUsize::new(0));
    let rejected = Arc::new(AtomicUsize::new(0));

    let mut joins = Vec::with_capacity(SUBMITTERS);
    for _ in 0..SUBMITTERS {
        let mgr = Arc::clone(&mgr);
        let accepted = Arc::clone(&accepted);
        let rejected = Arc::clone(&rejected);
        joins.push(tokio::spawn(async move {
            match mgr.submit("m", json!({}), None).await {
                Ok(_) => accepted.fetch_add(1, Ordering::SeqCst),
                Err(_) => rejected.fetch_add(1, Ordering::SeqCst),
            };
        }));
    }
    for j in joins {
        j.await.unwrap();
    }

    let accepted = accepted.load(Ordering::SeqCst);
    let rejected = rejected.load(Ordering::SeqCst);
    assert_eq!(
        accepted + rejected,
        SUBMITTERS,
        "every submitter must observe a definite outcome"
    );
    assert!(
        accepted <= CAP,
        "accepted submits must never exceed max_tasks; got accepted={accepted}, cap={CAP}"
    );
    assert_eq!(
        mgr.task_count().expect("store list"),
        accepted,
        "task_count must equal the number of accepted submits"
    );
}
