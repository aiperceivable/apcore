//! Regression tests for v0.22 async-task hardening — A-D-AT-01, A-D-AT-05.
//!
//! Covers:
//! - A-D-AT-01: `max_tasks` capacity counts only active tasks (`Pending` +
//!   `Running`), not terminal-state records still pending TTL cleanup.
//! - A-D-AT-05: `start_reaper` is single-instance — a second call without
//!   `stop()` returns `ErrorCode::ReaperAlreadyRunning`. Dropping the handle
//!   detaches rather than stops, so it does NOT release the guard;
//!   `AsyncTaskManager::stop_reaper` (and `shutdown`) do.

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
