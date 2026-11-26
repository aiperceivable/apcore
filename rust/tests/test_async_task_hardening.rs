//! Regression tests for v0.22 async-task hardening — A-D-AT-01, A-D-AT-05.
//!
//! Covers:
//! - A-D-AT-01: `max_tasks` capacity counts only active tasks (`Pending` +
//!   `Running`), not terminal-state records still pending TTL cleanup.
//! - A-D-AT-05: `start_reaper` is single-instance — a second call without
//!   `stop()` returns `ErrorCode::ReaperAlreadyRunning`. Dropping the handle
//!   detaches rather than stops, so it does NOT release the guard;
//!   `AsyncTaskManager::stop_reaper` (and `shutdown`) do.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use apcore::async_task::{
    AsyncTaskManager, InMemoryTaskStore, ReaperConfig, TaskInfo, TaskStatus, TaskStore,
};
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::module::Module;
use apcore::registry::registry::Registry;
use apcore::Executor;
use async_trait::async_trait;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct NoopModule;

#[async_trait]
impl Module for NoopModule {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "noop"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Ok(json!({"ok": true}))
    }
}

fn make_manager_with_store(max_tasks: usize, store: Arc<dyn TaskStore>) -> AsyncTaskManager {
    let registry = Arc::new(Registry::new());
    registry
        .register_module("noop.module", Box::new(NoopModule))
        .unwrap();
    let executor = Arc::new(Executor::new(registry, Arc::new(Config::default())));
    AsyncTaskManager::with_store(executor, /*max_concurrent=*/ 16, max_tasks, store)
}

fn make_manager(max_tasks: usize) -> AsyncTaskManager {
    make_manager_with_store(max_tasks, Arc::new(InMemoryTaskStore::new()))
}

struct CountedTask(Arc<AtomicUsize>);

#[async_trait]
impl Module for CountedTask {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn description(&self) -> &'static str {
        "Count task invocations"
    }

    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let attempt = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(json!({"attempt": attempt}))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalFault {
    ReadAtStart,
    CommitRunningThenError,
    ReadOnce,
    WriteOnce,
    CommitThenError,
    RejectResult,
    DeleteBeforeRetry,
    AlwaysRejectResult,
}

struct TerminalFaultStore {
    inner: InMemoryTaskStore,
    calls: Arc<AtomicUsize>,
    failures: AtomicUsize,
    fault: TerminalFault,
}

#[async_trait]
impl TaskStore for TerminalFaultStore {
    async fn save(&self, task: &TaskInfo) -> Result<(), ModuleError> {
        if self.fault == TerminalFault::CommitRunningThenError
            && task.status == TaskStatus::Running
            && self.failures.fetch_add(1, Ordering::SeqCst) == 0
        {
            self.inner.save(task).await?;
            return Err(ModuleError::task_store_unavailable(
                "save",
                "connection interrupted",
            ));
        }
        if task.status == TaskStatus::Completed
            && matches!(
                self.fault,
                TerminalFault::WriteOnce
                    | TerminalFault::CommitThenError
                    | TerminalFault::RejectResult
                    | TerminalFault::DeleteBeforeRetry
                    | TerminalFault::AlwaysRejectResult
            )
        {
            let failure = self.failures.fetch_add(1, Ordering::SeqCst);
            if self.fault == TerminalFault::RejectResult {
                return Err(
                    ModuleError::task_store_unavailable("save", "result too large")
                        .with_retryable(false),
                );
            }
            if failure == 0 || self.fault == TerminalFault::AlwaysRejectResult {
                if self.fault == TerminalFault::CommitThenError {
                    self.inner.save(task).await?;
                }
                if self.fault == TerminalFault::DeleteBeforeRetry {
                    self.inner.delete(&task.task_id).await?;
                }
                return Err(ModuleError::task_store_unavailable(
                    "save",
                    "connection interrupted",
                ));
            }
        }
        self.inner.save(task).await
    }

    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ModuleError> {
        if (self.fault == TerminalFault::ReadAtStart
            || (self.fault == TerminalFault::ReadOnce && self.calls.load(Ordering::SeqCst) > 0))
            && self.failures.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Err(ModuleError::task_store_unavailable(
                "get",
                "connection interrupted",
            ));
        }
        self.inner.get(id).await
    }

    async fn list(&self, status: Option<TaskStatus>) -> Result<Vec<TaskInfo>, ModuleError> {
        self.inner.list(status).await
    }

    async fn delete(&self, id: &str) -> Result<(), ModuleError> {
        self.inner.delete(id).await
    }

    async fn list_expired(&self, before: f64) -> Result<Vec<TaskInfo>, ModuleError> {
        self.inner.list_expired(before).await
    }
}

fn counted_task_manager(fault: TerminalFault) -> (AsyncTaskManager, Arc<TerminalFaultStore>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(TerminalFaultStore {
        inner: InMemoryTaskStore::new(),
        calls: calls.clone(),
        failures: AtomicUsize::new(0),
        fault,
    });
    let registry = Registry::new();
    registry
        .register_module("tasks.counted", Box::new(CountedTask(calls)))
        .unwrap();
    let executor = Arc::new(Executor::new(registry, Config::default()));
    (
        AsyncTaskManager::with_store(executor, 1, 1, store.clone()),
        store,
    )
}

#[tokio::test]
async fn terminal_storage_retries_preserve_the_single_module_result() {
    for fault in [
        TerminalFault::ReadAtStart,
        TerminalFault::CommitRunningThenError,
        TerminalFault::ReadOnce,
        TerminalFault::WriteOnce,
        TerminalFault::CommitThenError,
        TerminalFault::RejectResult,
    ] {
        let (manager, store) = counted_task_manager(fault);
        let task_id = manager
            .submit("tasks.counted", json!({}), None)
            .await
            .unwrap();
        let terminal = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let info = store.inner.get(&task_id).await.unwrap().unwrap();
                if matches!(info.status, TaskStatus::Completed | TaskStatus::Failed) {
                    break info;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the original attempt should reach terminal state");
        assert_eq!(store.calls.load(Ordering::SeqCst), 1);
        if fault == TerminalFault::RejectResult {
            assert_eq!(terminal.status, TaskStatus::Failed);
            assert_eq!(terminal.result, None);
        } else {
            assert_eq!(
                manager.get_result_async(&task_id).await.unwrap(),
                json!({"attempt": 1})
            );
        }
        assert!(!manager.cancel(&task_id).await.unwrap());
        if fault == TerminalFault::CommitThenError {
            assert_eq!(store.failures.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn terminal_storage_retry_preserves_cancellation_and_deletion() {
    for fault in [
        TerminalFault::DeleteBeforeRetry,
        TerminalFault::AlwaysRejectResult,
    ] {
        let (manager, store) = counted_task_manager(fault);
        let task_id = manager
            .submit("tasks.counted", json!({}), None)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while store.failures.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        if fault == TerminalFault::AlwaysRejectResult {
            assert!(manager.cancel(&task_id).await.unwrap());
        }
        manager.shutdown().await.unwrap();
        let stored_status = manager
            .get_status_async(&task_id)
            .await
            .unwrap()
            .map(|task| task.status);
        assert_eq!(
            stored_status,
            (fault == TerminalFault::AlwaysRejectResult).then_some(TaskStatus::Cancelled)
        );
        assert_eq!(store.calls.load(Ordering::SeqCst), 1);
    }
}

struct HeldTask {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl Module for HeldTask {
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn output_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn description(&self) -> &'static str {
        "Task with a completion barrier"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(json!({}))
    }
}

#[tokio::test]
async fn timed_out_task_retains_execution_slot_until_cleanup() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let registry = Registry::new();
    registry
        .register_module(
            "tasks.held",
            Box::new(HeldTask {
                entered: entered.clone(),
                release: release.clone(),
            }),
        )
        .unwrap();
    registry
        .register_module("tasks.next", Box::new(NoopModule))
        .unwrap();
    let mut config = Config::default();
    config.executor.default_timeout = 30;
    let manager = AsyncTaskManager::new(Arc::new(Executor::new(registry, config)), 1, 10);
    let first = manager.submit("tasks.held", json!({}), None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    let second = manager.submit("tasks.next", json!({}), None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while manager
            .get_status_async(&first)
            .await
            .unwrap()
            .unwrap()
            .status
            != TaskStatus::Failed
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Give the queued worker a chance to acquire any incorrectly released slot.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        manager
            .get_status_async(&second)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Pending
    );
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        while manager
            .get_status_async(&second)
            .await
            .unwrap()
            .unwrap()
            .status
            != TaskStatus::Completed
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// A-D-AT-01: max_tasks counts active statuses only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_tasks_counts_only_active_statuses() {
    let store = Arc::new(InMemoryTaskStore::new());

    // Pre-populate the store with `max_tasks` terminal-state records — these
    // are exactly the records that pluggable storage and TTL-based cleanup
    // are designed to retain. They MUST NOT consume the active budget.
    for i in 0..3 {
        let mut info = TaskInfo::default();
        info.task_id = format!("done-{i}");
        info.module_id = "noop.module".to_string();
        info.status = TaskStatus::Completed;
        info.completed_at = Some(0.0);
        info.started_at = Some(0.0);
        info.result = Some(json!({}));
        store.save(&info).await.unwrap();
    }

    let mgr = make_manager_with_store(/*max_tasks=*/ 3, store.clone() as Arc<dyn TaskStore>);

    // Even with 3 terminal records present, a new submission MUST succeed
    // because 0 active tasks < max_tasks=3.
    let result = mgr.submit("noop.module", json!({}), None).await;
    assert!(
        result.is_ok(),
        "submit must not be rejected by terminal-state records (closes A-D-AT-01); got {:?}",
        result.err()
    );
}

#[tokio::test]
async fn max_tasks_still_rejects_when_active_budget_exhausted() {
    // Sanity check: the active-count fix MUST NOT regress the original
    // protection — if active >= max_tasks, submit still fails.
    let store = Arc::new(InMemoryTaskStore::new());
    for i in 0..2 {
        let mut info = TaskInfo::default();
        info.task_id = format!("running-{i}");
        info.module_id = "noop.module".to_string();
        info.status = TaskStatus::Running;
        info.started_at = Some(0.0);
        store.save(&info).await.unwrap();
    }

    let mgr = make_manager_with_store(/*max_tasks=*/ 2, store as Arc<dyn TaskStore>);
    let err = mgr
        .submit("noop.module", json!({}), None)
        .await
        .expect_err("submit must fail when active >= max_tasks");
    assert_eq!(err.code, ErrorCode::TaskLimitExceeded);
}

// ---------------------------------------------------------------------------
// A-D-AT-05: start_reaper is single-instance
// ---------------------------------------------------------------------------

#[tokio::test]
async fn start_reaper_rejects_concurrent_start() {
    let mgr = make_manager(/*max_tasks=*/ 100);
    let mut cfg = ReaperConfig::default();
    cfg.ttl_seconds = 60.0;
    cfg.sweep_interval_ms = 5_000;

    let first = mgr
        .start_reaper(cfg)
        .expect("first start_reaper must succeed");

    let err = mgr
        .start_reaper(cfg)
        .expect_err("second start_reaper must fail while first is live");
    assert_eq!(err.code, ErrorCode::ReaperAlreadyRunning);

    // After stop() releases the flag, a fresh reaper can be started again.
    first.stop().await;
    let third = mgr
        .start_reaper(cfg)
        .expect("start_reaper must succeed after stop()");
    third.stop().await;
}

#[tokio::test]
async fn dropped_reaper_handle_keeps_the_reaper_running() {
    // Dropping the handle DETACHES the reaper — the sweep loop keeps running.
    // Releasing the single-reaper flag on drop (without signalling stop) let a
    // SECOND sweep loop start alongside the first, both deleting from the same
    // store. A detached reaper is still running, so the guard stays set.
    let mgr = make_manager(/*max_tasks=*/ 100);
    let mut cfg = ReaperConfig::default();
    cfg.ttl_seconds = 60.0;
    cfg.sweep_interval_ms = 5_000;
    {
        let _detached = mgr.start_reaper(cfg).unwrap();
        // _detached drops here.
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    mgr.start_reaper(cfg)
        .expect_err("a detached reaper is still running; a second must not start");

    // stop_reaper() is how a detached reaper is stopped.
    assert!(mgr.stop_reaper(), "a reaper was running");
    let handle = mgr
        .start_reaper(cfg)
        .expect("stop_reaper must release the guard");
    handle.stop().await;
    assert!(!mgr.stop_reaper(), "stop_reaper is idempotent");
}

#[tokio::test]
async fn test_old_reaper_handle_cannot_clear_replacement() {
    let manager = make_manager(100);
    let config = ReaperConfig::default();
    let old = manager.start_reaper(config).unwrap();
    assert!(manager.stop_reaper());

    let replacement = manager.start_reaper(config).unwrap();
    old.stop().await;
    let error = manager.start_reaper(config).unwrap_err();
    assert_eq!(error.code, ErrorCode::ReaperAlreadyRunning);

    assert!(manager.stop_reaper());
    replacement.stop().await;
    manager.start_reaper(config).unwrap().stop().await;
}

#[tokio::test]
async fn shutdown_stops_the_reaper() {
    // `shutdown()` never touched the reaper, so a started sweep loop outlived
    // it — apcore-python's `shutdown` awaits `stop_reaper()` first and
    // apcore-typescript clears its sweep timer there.
    let mgr = make_manager(/*max_tasks=*/ 100);
    let mut cfg = ReaperConfig::default();
    cfg.ttl_seconds = 60.0;
    cfg.sweep_interval_ms = 5_000;
    let _detached = mgr.start_reaper(cfg).unwrap();

    mgr.shutdown().await.expect("store shutdown");

    mgr.start_reaper(cfg)
        .expect("shutdown must have stopped the reaper")
        .stop()
        .await;
}
