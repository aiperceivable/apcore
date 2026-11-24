// Spec-traced contract tests for the apcore-rust async-tasks feature.
//
// Source spec: apcore/docs/features/async-tasks.md
// Canonical clause list mirrored from:
//   apcore-python/tests/test_async_tasks_spec.py
//
// Each test maps to exactly one clause in the feature spec's '## Contract:'
// blocks. The verbatim cross-language clause id appears in a leading
// `// clause: <clause_id>` comment on the line above each test fn so a
// cross-language diff tool can line up the Python / TypeScript / Rust rows by
// that exact string. The fn name is the clause id flattened to snake_case.
//
// These tests exercise OBSERVABLE behaviour only (public API). Where the
// implementation surfaces a contract error asynchronously (submit spawns the
// executor call into a background tokio task), the test drains the task to its
// terminal state and asserts the observable outcome.
//
// Framework: cargo test + #[tokio::test] (multi-thread).

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::float_cmp)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use apcore::async_task::{
    AsyncTaskManager, InMemoryTaskStore, ReaperConfig, TaskInfo, TaskStatus, TaskStore,
};
use apcore::config::Config;
use apcore::context::Context;
use apcore::errors::{ErrorCode, ModuleError};
use apcore::executor::Executor;
use apcore::module::Module;
use apcore::registry::registry::Registry;

// ---------------------------------------------------------------------------
// Helper modules
// ---------------------------------------------------------------------------

/// Module that echoes a value from inputs — completes immediately.
struct EchoModule;

#[async_trait]
impl Module for EchoModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "echo module"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let x = inputs.get("x").cloned().unwrap_or(json!(0));
        Ok(json!({ "value": x }))
    }
}

/// Async module that sleeps for a configurable duration then returns.
struct SlowModule;

#[async_trait]
impl Module for SlowModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "slow module"
    }
    async fn execute(&self, inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        let delay = inputs.get("delay").and_then(Value::as_f64).unwrap_or(0.5);
        tokio::time::sleep(Duration::from_secs_f64(delay)).await;
        Ok(json!({ "done": true }))
    }
}

/// Module that always raises a deterministic error.
struct FailingModule;

#[async_trait]
impl Module for FailingModule {
    fn input_schema(&self) -> Value {
        Value::Null
    }
    fn output_schema(&self) -> Value {
        Value::Null
    }
    fn description(&self) -> &'static str {
        "failing module"
    }
    async fn execute(&self, _inputs: Value, _ctx: &Context<Value>) -> Result<Value, ModuleError> {
        Err(ModuleError::new(
            ErrorCode::ModuleExecuteError,
            "intentional failure",
        ))
    }
}

// ---------------------------------------------------------------------------
// Fixtures / helpers
// ---------------------------------------------------------------------------

fn make_executor() -> Arc<Executor> {
    let reg = Registry::new();
    reg.register_module("test.echo", Box::new(EchoModule))
        .expect("register echo");
    reg.register_module("test.slow", Box::new(SlowModule))
        .expect("register slow");
    reg.register_module("test.failing", Box::new(FailingModule))
        .expect("register failing");
    Arc::new(Executor::new(reg, Config::default()))
}

fn make_manager() -> AsyncTaskManager {
    AsyncTaskManager::new(make_executor(), 10, 1000)
}

/// Build a `TaskInfo` via `Default` + field assignment. `TaskInfo` is
/// `#[non_exhaustive]`, so struct-literal construction from this external test
/// crate is forbidden (E0639); mutating a `Default::default()` value is the
/// idiomatic cross-crate construction path.
fn make_task_info(
    task_id: &str,
    module_id: &str,
    status: TaskStatus,
    submitted_at: f64,
    completed_at: Option<f64>,
    started_at: Option<f64>,
) -> TaskInfo {
    let mut info = TaskInfo::default();
    info.task_id = task_id.to_string();
    info.module_id = module_id.to_string();
    info.status = status;
    info.submitted_at = submitted_at;
    info.completed_at = completed_at;
    info.started_at = started_at;
    info
}

/// Build a `ReaperConfig` via `Default` + field assignment (`#[non_exhaustive]`
/// blocks cross-crate struct-literal construction, same as `TaskInfo`).
fn make_reaper_config(ttl_seconds: f64, sweep_interval_ms: u64) -> ReaperConfig {
    let mut cfg = ReaperConfig::default();
    cfg.ttl_seconds = ttl_seconds;
    cfg.sweep_interval_ms = sweep_interval_ms;
    cfg
}

/// Wire string for a `ModuleError`'s code (e.g. `"MODULE_NOT_FOUND"`).
fn code_str(err: &ModuleError) -> String {
    match serde_json::to_value(err.code) {
        Ok(Value::String(s)) => s,
        other => panic!("error code did not serialize to a string: {other:?}"),
    }
}

/// Await until `task_id` reaches a terminal state and return its snapshot.
async fn drain(manager: &AsyncTaskManager, task_id: &str, timeout: Duration) -> TaskInfo {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(info) = manager.get_status(task_id).expect("store read") {
            if matches!(
                info.status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
            ) {
                return info;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("task {task_id} did not reach a terminal state within {timeout:?}");
}

const DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.submit
// ---------------------------------------------------------------------------

// clause: async_tasks.submit.input.module_id.malformed
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_submit_input_module_id_malformed() {
    // A malformed module_id must surface as a failed task. Rust validates
    // module_id inside the spawned executor call, so the failure is observable
    // as the task's terminal FAILED state carrying the "Invalid module ID"
    // message. Rust uses INVALID_MODULE_ID (validate_module_id is private),
    // matching Python/TS per error-system.md §560.
    let manager = make_manager();
    let task_id = manager
        .submit("Bad-ID!", json!({"x": 1}), None)
        .await
        .expect("submit returns a task id even for a bad module id");
    let info = drain(&manager, &task_id, DRAIN_TIMEOUT).await;
    assert_eq!(info.status, TaskStatus::Failed);
    let err = info.error.expect("failed task carries an error message");
    assert!(
        err.contains("Invalid module ID"),
        "error should mention invalid module ID, got: {err}"
    );
}

// clause: async_tasks.submit.error.MODULE_NOT_FOUND
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_submit_error_module_not_found() {
    // Submitting a well-formed but unregistered module_id drives the task to
    // FAILED, with the underlying executor error mentioning the module id.
    let manager = make_manager();
    let task_id = manager
        .submit("no.such.module", json!({"x": 1}), None)
        .await
        .expect("submit returns a task id");
    let info = drain(&manager, &task_id, DRAIN_TIMEOUT).await;
    assert_eq!(info.status, TaskStatus::Failed);
    let err = info.error.expect("failed task carries an error message");
    assert!(
        err.contains("no.such.module"),
        "error should mention the missing module id, got: {err}"
    );
}

// clause: async_tasks.submit.error.TASK_LIMIT_EXCEEDED
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_submit_error_task_limit_exceeded() {
    // With max_tasks=1, a second concurrent active submission must return the
    // typed TaskLimitExceeded error whose wire code is TASK_LIMIT_EXCEEDED.
    let limited = AsyncTaskManager::new(make_executor(), 10, 1);
    limited
        .submit("test.slow", json!({"delay": 0.5}), None)
        .await
        .expect("first submit succeeds");
    let err = limited
        .submit("test.slow", json!({"delay": 0.5}), None)
        .await
        .expect_err("second submit at the cap must fail");
    assert_eq!(err.code, ErrorCode::TaskLimitExceeded);
    assert_eq!(code_str(&err), "TASK_LIMIT_EXCEEDED");
    limited.shutdown().await.expect("store shutdown");
}

// clause: async_tasks.submit.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_submit_property_async() {
    // submit is awaitable and resolves to a non-empty String task id.
    let manager = make_manager();
    let task_id = manager
        .submit("test.echo", json!({"x": 7}), None)
        .await
        .expect("submit resolves to Ok(task_id)");
    assert!(!task_id.is_empty());
    let info = drain(&manager, &task_id, DRAIN_TIMEOUT).await;
    assert_eq!(info.status, TaskStatus::Completed);
}

// clause: async_tasks.submit.property.thread_safe
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_submit_property_thread_safe() {
    // N>=8 concurrent submissions with distinct inputs complete without error
    // and produce N distinct task ids with consistent final state.
    let manager = Arc::new(make_manager());
    let n = 12u32;
    let mut handles = Vec::new();
    for i in 0..n {
        let mgr = Arc::clone(&manager);
        handles.push(tokio::spawn(async move {
            mgr.submit("test.echo", json!({"x": i}), None)
                .await
                .expect("concurrent submit succeeds")
        }));
    }
    let mut task_ids = Vec::new();
    for h in handles {
        task_ids.push(h.await.expect("spawned submit task did not panic"));
    }
    let unique: std::collections::HashSet<&String> = task_ids.iter().collect();
    assert_eq!(unique.len(), n as usize, "all task ids must be distinct");

    let mut values = std::collections::HashSet::new();
    for tid in &task_ids {
        let info = drain(&manager, tid, DRAIN_TIMEOUT).await;
        assert_eq!(info.status, TaskStatus::Completed);
        let v = info
            .result
            .and_then(|r| r.get("value").and_then(Value::as_u64))
            .expect("completed echo task has a numeric value");
        values.insert(u32::try_from(v).expect("echo task value fits u32"));
    }
    let expected: std::collections::HashSet<u32> = (0..n).collect();
    assert_eq!(values, expected, "every input value appears exactly once");
}

// clause: async_tasks.submit.property.idempotent_false
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_submit_property_idempotent_false() {
    // submit is NOT idempotent: two identical calls create two distinct tasks.
    let manager = make_manager();
    let first = manager
        .submit("test.echo", json!({"x": 1}), None)
        .await
        .expect("first submit");
    let second = manager
        .submit("test.echo", json!({"x": 1}), None)
        .await
        .expect("second submit");
    assert_ne!(first, second);
    let ids: std::collections::HashSet<String> = manager
        .list_tasks(None)
        .expect("store list")
        .into_iter()
        .map(|t| t.task_id)
        .collect();
    assert!(ids.contains(&first) && ids.contains(&second));
}

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.cancel
// ---------------------------------------------------------------------------

// clause: async_tasks.cancel.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_cancel_property_async() {
    // cancel is awaitable and resolves to a bool; cancelling an active task
    // returns true and transitions it to CANCELLED.
    let manager = make_manager();
    let task_id = manager
        .submit("test.slow", json!({"delay": 1.0}), None)
        .await
        .expect("submit");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let result = manager.cancel(&task_id).await.expect("store write");
    assert!(result, "cancel of an active task returns true");
    let info = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("status present");
    assert_eq!(info.status, TaskStatus::Cancelled);
}

// clause: async_tasks.cancel.return.unknown_task_false
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_cancel_return_unknown_task_false() {
    // Cancelling a non-existent task id returns false (no error raised).
    let manager = make_manager();
    let result = manager.cancel("does-not-exist").await.expect("store write");
    assert!(!result);
}

// clause: async_tasks.cancel.property.thread_safe
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_cancel_property_thread_safe() {
    // Concurrently cancelling N>=8 distinct running tasks raises no panic and
    // leaves every task in CANCELLED state.
    let manager = Arc::new(AsyncTaskManager::new(make_executor(), 20, 1000));
    let n = 8;
    let mut task_ids = Vec::new();
    for _ in 0..n {
        task_ids.push(
            manager
                .submit("test.slow", json!({"delay": 1.0}), None)
                .await
                .expect("submit"),
        );
    }
    tokio::time::sleep(Duration::from_millis(80)).await; // let them reach RUNNING

    let mut handles = Vec::new();
    for tid in &task_ids {
        let mgr = Arc::clone(&manager);
        let tid = tid.clone();
        handles.push(tokio::spawn(async move {
            mgr.cancel(&tid).await.expect("store write")
        }));
    }
    for h in handles {
        assert!(h.await.expect("cancel task did not panic"));
    }
    for tid in &task_ids {
        let info = manager
            .get_status(tid)
            .expect("store read")
            .expect("status present");
        assert_eq!(info.status, TaskStatus::Cancelled);
    }
    manager.shutdown().await.expect("store shutdown");
}

// clause: async_tasks.cancel.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_cancel_property_idempotent() {
    // First cancel applies (true); the second is a no-op (false). Observable
    // state stays CANCELLED across both calls.
    let manager = make_manager();
    let task_id = manager
        .submit("test.slow", json!({"delay": 1.0}), None)
        .await
        .expect("submit");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let first = manager.cancel(&task_id).await.expect("store write");
    let second = manager.cancel(&task_id).await.expect("store write");
    assert!(first);
    assert!(!second);
    let info = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("status present");
    assert_eq!(info.status, TaskStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.get_status
// ---------------------------------------------------------------------------

// clause: async_tasks.get_status.property.async_false
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_status_property_async_false() {
    // In Rust get_status is synchronous (async:false): it returns Option<TaskInfo>
    // directly, not a future.
    let manager = make_manager();
    let task_id = manager
        .submit("test.echo", json!({"x": 1}), None)
        .await
        .expect("submit");
    let info: Option<TaskInfo> = manager.get_status(&task_id).expect("store read");
    let info = info.expect("known task returns Some");
    assert_eq!(info.task_id, task_id);
}

// clause: async_tasks.get_status.return.shallow_copy
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_status_return_shallow_copy() {
    // The returned object is a clone (D-23): mutating it must not propagate
    // back to the store.
    let manager = make_manager();
    let task_id = manager
        .submit("test.echo", json!({"x": 1}), None)
        .await
        .expect("submit");
    let mut info = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("status present");
    info.module_id = "tampered".to_string();
    let again = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("status present");
    assert_eq!(again.module_id, "test.echo");
}

// clause: async_tasks.get_status.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_status_property_idempotent() {
    // Repeated reads of the same terminal task return equal snapshots without
    // altering observable state.
    let manager = make_manager();
    let task_id = manager
        .submit("test.echo", json!({"x": 9}), None)
        .await
        .expect("submit");
    drain(&manager, &task_id, DRAIN_TIMEOUT).await;
    let a = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("a");
    let b = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("b");
    assert_eq!(a.task_id, b.task_id);
    assert_eq!(a.status, b.status);
    assert_eq!(a.result, b.result);
}

// clause: async_tasks.get_status.return.unknown_none
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_status_return_unknown_none() {
    // An unknown task id returns None rather than panicking.
    let manager = make_manager();
    assert!(manager.get_status("nope").expect("store read").is_none());
}

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.get_result
// ---------------------------------------------------------------------------

// clause: async_tasks.get_result.error.task_not_found
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_result_error_task_not_found() {
    // get_result errors when no task with the id exists. Rust returns
    // Err(ModuleError) with message "Task not found: <id>".
    let manager = make_manager();
    let err = manager
        .get_result("missing")
        .expect_err("missing task errors");
    assert!(
        err.message.contains("Task not found"),
        "got: {}",
        err.message
    );
}

// clause: async_tasks.get_result.error.not_completed
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_result_error_not_completed() {
    // get_result errors when the task exists but is not COMPLETED (here:
    // RUNNING). Rust message: "Task <id> is not completed (status=...)".
    let manager = make_manager();
    let task_id = manager
        .submit("test.slow", json!({"delay": 1.0}), None)
        .await
        .expect("submit");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let err = manager
        .get_result(&task_id)
        .expect_err("running task is not completed");
    assert!(
        err.message.contains("is not completed"),
        "got: {}",
        err.message
    );
    manager.cancel(&task_id).await.expect("store write");
}

// clause: async_tasks.get_result.return.completed_result
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_result_return_completed_result() {
    // get_result returns the module output once the task is COMPLETED.
    let manager = make_manager();
    let task_id = manager
        .submit("test.echo", json!({"x": 42}), None)
        .await
        .expect("submit");
    drain(&manager, &task_id, DRAIN_TIMEOUT).await;
    let result = manager.get_result(&task_id).expect("completed result");
    assert_eq!(result, json!({"value": 42}));
}

// clause: async_tasks.get_result.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_result_property_idempotent() {
    // Two get_result calls on the same completed task return identical output.
    let manager = make_manager();
    let task_id = manager
        .submit("test.echo", json!({"x": 5}), None)
        .await
        .expect("submit");
    drain(&manager, &task_id, DRAIN_TIMEOUT).await;
    let a = manager.get_result(&task_id).expect("a");
    let b = manager.get_result(&task_id).expect("b");
    assert_eq!(a, b);
    assert_eq!(a, json!({"value": 5}));
}

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.list_tasks
// ---------------------------------------------------------------------------

// clause: async_tasks.list_tasks.input.status.filter
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_tasks_input_status_filter() {
    // When a status is supplied, only tasks with that exact status are returned.
    let manager = make_manager();
    let completed_id = manager
        .submit("test.echo", json!({"x": 1}), None)
        .await
        .expect("submit echo");
    drain(&manager, &completed_id, DRAIN_TIMEOUT).await;
    let running_id = manager
        .submit("test.slow", json!({"delay": 1.0}), None)
        .await
        .expect("submit slow");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let completed: Vec<String> = manager
        .list_tasks(Some(TaskStatus::Completed))
        .expect("store list")
        .into_iter()
        .map(|t| t.task_id)
        .collect();
    assert_eq!(completed, vec![completed_id]);

    let running: std::collections::HashSet<String> = manager
        .list_tasks(Some(TaskStatus::Running))
        .expect("store list")
        .into_iter()
        .map(|t| t.task_id)
        .collect();
    assert!(running.contains(&running_id));
    manager.cancel(&running_id).await.expect("store write");
}

// clause: async_tasks.list_tasks.return.shallow_copy
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_tasks_return_shallow_copy() {
    // Each entry is a clone (D-23): mutating a listed entry must not affect the
    // stored task.
    let manager = make_manager();
    let task_id = manager
        .submit("test.echo", json!({"x": 1}), None)
        .await
        .expect("submit");
    drain(&manager, &task_id, DRAIN_TIMEOUT).await;
    let mut listed = manager.list_tasks(None).expect("store list");
    assert!(!listed.is_empty());
    listed[0].module_id = "tampered".to_string();
    let again = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("status present");
    assert_eq!(again.module_id, "test.echo");
}

// clause: async_tasks.list_tasks.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_tasks_property_idempotent() {
    // Repeated list_tasks calls over an unchanged store return the same set of
    // task ids without mutating state.
    let manager = make_manager();
    let mut ids = std::collections::HashSet::new();
    for i in 0..3 {
        let id = manager
            .submit("test.echo", json!({"x": i}), None)
            .await
            .expect("submit");
        ids.insert(id);
    }
    for id in &ids {
        drain(&manager, id, DRAIN_TIMEOUT).await;
    }
    let first: std::collections::HashSet<String> = manager
        .list_tasks(None)
        .expect("store list")
        .into_iter()
        .map(|t| t.task_id)
        .collect();
    let second: std::collections::HashSet<String> = manager
        .list_tasks(None)
        .expect("store list")
        .into_iter()
        .map(|t| t.task_id)
        .collect();
    assert_eq!(first, second);
    assert_eq!(first, ids);
}

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.cleanup
// ---------------------------------------------------------------------------

// clause: async_tasks.cleanup.eligible.terminal_only
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_cleanup_eligible_terminal_only() {
    // cleanup removes terminal-state tasks past the age threshold and never
    // removes PENDING/RUNNING tasks.
    let manager = make_manager();
    let done_id = manager
        .submit("test.echo", json!({"x": 1}), None)
        .await
        .expect("submit echo");
    drain(&manager, &done_id, DRAIN_TIMEOUT).await;
    let running_id = manager
        .submit("test.slow", json!({"delay": 1.0}), None)
        .await
        .expect("submit slow");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // max_age_seconds=0 makes every terminal task eligible immediately.
    let removed = manager.cleanup(0.0).expect("store sweep");
    assert_eq!(removed, 1);
    assert!(manager.get_status(&done_id).expect("store read").is_none());
    assert!(manager
        .get_status(&running_id)
        .expect("store read")
        .is_some());
    manager.cancel(&running_id).await.expect("store write");
}

// clause: async_tasks.cleanup.property.idempotent_false
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_cleanup_property_idempotent_false() {
    // cleanup is non-idempotent: the first call removes the eligible task
    // (count 1); a second identical call removes nothing (count 0).
    let manager = make_manager();
    let done_id = manager
        .submit("test.echo", json!({"x": 1}), None)
        .await
        .expect("submit");
    drain(&manager, &done_id, DRAIN_TIMEOUT).await;
    let first = manager.cleanup(0.0).expect("store sweep");
    let second = manager.cleanup(0.0).expect("store sweep");
    assert_eq!(first, 1);
    assert_eq!(second, 0);
}

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.shutdown
// ---------------------------------------------------------------------------

// clause: async_tasks.shutdown.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_shutdown_property_async() {
    // shutdown is awaitable and resolves to () (None equivalent) — wrapped in
    // a Result since D-81 so a store outage reaches the caller.
    let manager = make_manager();
    let out: () = manager.shutdown().await.expect("store shutdown");
    assert_eq!(out, ());
}

// clause: async_tasks.shutdown.side_effect.1.cancel_active
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_shutdown_side_effect_1_cancel_active() {
    // After shutdown, every task that was PENDING/RUNNING is CANCELLED —
    // observed via post-state queries on each task.
    let manager = AsyncTaskManager::new(make_executor(), 20, 1000);
    let mut ids = Vec::new();
    for _ in 0..8 {
        ids.push(
            manager
                .submit("test.slow", json!({"delay": 1.0}), None)
                .await
                .expect("submit"),
        );
    }
    tokio::time::sleep(Duration::from_millis(80)).await;
    manager.shutdown().await.expect("store shutdown");
    for tid in &ids {
        let info = manager
            .get_status(tid)
            .expect("store read")
            .expect("status present");
        assert_eq!(info.status, TaskStatus::Cancelled);
    }
}

// clause: async_tasks.shutdown.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_shutdown_property_idempotent() {
    // Calling shutdown twice is a no-op and does not panic; task state is
    // unchanged across the second call.
    let manager = make_manager();
    let task_id = manager
        .submit("test.slow", json!({"delay": 1.0}), None)
        .await
        .expect("submit");
    tokio::time::sleep(Duration::from_millis(80)).await;
    manager.shutdown().await.expect("store shutdown");
    let before = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("status present");
    manager.shutdown().await.expect("store shutdown");
    let after = manager
        .get_status(&task_id)
        .expect("store read")
        .expect("status present");
    assert_eq!(before.status, TaskStatus::Cancelled);
    assert_eq!(after.status, TaskStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Contract: AsyncTaskManager.start_reaper
// ---------------------------------------------------------------------------

// clause: async_tasks.start_reaper.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_start_reaper_property_async() {
    // start_reaper spawns a background sweep loop and returns a ReaperHandle
    // whose stop() is awaitable. (Rust start_reaper returns the handle
    // synchronously; the spawned loop is the async/background effect.)
    let manager = make_manager();
    let handle = manager
        .start_reaper(make_reaper_config(3600.0, 300_000))
        .expect("start_reaper succeeds");
    // stop() is async — awaiting it resolves and tears down the loop.
    handle.stop().await;
    // After stop, a fresh reaper may be started again (running flag cleared).
    let again = manager
        .start_reaper(make_reaper_config(3600.0, 300_000))
        .expect("reaper restartable after stop");
    again.stop().await;
}

// clause: async_tasks.start_reaper.property.idempotent_false
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_start_reaper_property_idempotent_false() {
    // start_reaper is NOT idempotent and guards against a second concurrent
    // start: starting again while a reaper runs returns
    // Err(ReaperAlreadyRunning).
    let manager = make_manager();
    let handle = manager
        .start_reaper(make_reaper_config(3600.0, 300_000))
        .expect("first start_reaper succeeds");
    let err = manager
        .start_reaper(make_reaper_config(3600.0, 300_000))
        .expect_err("second start_reaper while running must error");
    assert_eq!(err.code, ErrorCode::ReaperAlreadyRunning);
    handle.stop().await;
}

// ---------------------------------------------------------------------------
// Contract: TaskStore.save
// ---------------------------------------------------------------------------

// clause: async_tasks.save.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_save_property_async() {
    // TaskStore::save is async (D-17): InMemoryTaskStore::save resolves to
    // Ok(()) and persists the record.
    let store = InMemoryTaskStore::new();
    let info = make_task_info("t1", "test.echo", TaskStatus::Pending, 1.0, None, None);
    store.save(&info).await.expect("save resolves to Ok(())");
    assert!(store.get("t1").await.expect("get ok").is_some());
}

// clause: async_tasks.save.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_save_property_idempotent() {
    // Saving twice with the same task_id overwrites — exactly one record remains.
    let store = InMemoryTaskStore::new();
    let info = make_task_info("t1", "test.echo", TaskStatus::Pending, 1.0, None, None);
    store.save(&info).await.expect("save1");
    let info2 = make_task_info("t1", "test.echo", TaskStatus::Completed, 1.0, None, None);
    store.save(&info2).await.expect("save2");
    let all = store.list(None).await.expect("list");
    assert_eq!(all.len(), 1);
    let stored = store.get("t1").await.expect("get").expect("present");
    assert_eq!(stored.status, TaskStatus::Completed);
}

// clause: async_tasks.save.error.TASK_STORE_UNAVAILABLE
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_save_error_task_store_unavailable() {
    // This test was `#[ignore]`d with the reason "missing symbol
    // TaskStoreError/TASK_STORE_UNAVAILABLE (contract gap)" — true when it was
    // written, and made false by D-92, which required all three SDKs to define
    // and export the type. Nothing turned red when the gap closed, because a
    // disabled test explaining why something cannot be tested keeps explaining
    // it after it can.
    let store = InMemoryTaskStore::new();
    let info = make_task_info("t1", "test.echo", TaskStatus::Pending, 1.0, None, None);

    // The bundled in-memory store cannot fail, so it MUST NOT raise it.
    store
        .save(&info)
        .await
        .expect("the in-memory store cannot be unavailable");

    // A network-backed store has one canonical code to raise, and it survives
    // the manager's error channel unchanged.
    let err = ModuleError::task_store_unavailable("save", "connection refused");
    assert_eq!(err.code, ErrorCode::TaskStoreUnavailable);
    assert!(
        err.message.contains("save") && err.message.contains("connection refused"),
        "operation and reason must reach the caller: {}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// Contract: TaskStore.get
// ---------------------------------------------------------------------------

// clause: async_tasks.get.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_property_async() {
    // TaskStore::get is async (D-17) and returns the stored record or None.
    let store = InMemoryTaskStore::new();
    let info = make_task_info("g1", "test.echo", TaskStatus::Pending, 1.0, None, None);
    store.save(&info).await.expect("save");
    let fetched = store.get("g1").await.expect("get ok").expect("present");
    assert_eq!(fetched.task_id, "g1");
}

// clause: async_tasks.get.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_property_idempotent() {
    // Two gets of the same id return equal records and never mutate the store.
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "g1",
            "test.echo",
            TaskStatus::Pending,
            1.0,
            None,
            None,
        ))
        .await
        .expect("save");
    let a = store.get("g1").await.expect("a").expect("present");
    let b = store.get("g1").await.expect("b").expect("present");
    assert_eq!(a.task_id, b.task_id);
    assert_eq!(store.list(None).await.expect("list").len(), 1);
}

// clause: async_tasks.get.return.unknown_none
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_get_return_unknown_none() {
    // An unknown task id returns None; in-memory store never errors.
    let store = InMemoryTaskStore::new();
    assert!(store.get("absent").await.expect("get ok").is_none());
}

// ---------------------------------------------------------------------------
// Contract: TaskStore.list
// ---------------------------------------------------------------------------

// clause: async_tasks.list.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_property_async() {
    // TaskStore::list is async (D-17) and returns a Vec of records.
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "l1",
            "test.echo",
            TaskStatus::Pending,
            1.0,
            None,
            None,
        ))
        .await
        .expect("save");
    let items = store.list(None).await.expect("list");
    let ids: Vec<String> = items.into_iter().map(|i| i.task_id).collect();
    assert_eq!(ids, vec!["l1".to_string()]);
}

// clause: async_tasks.list.input.status.filter
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_input_status_filter() {
    // When a status is supplied, only matching records are returned.
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "l1",
            "m",
            TaskStatus::Pending,
            1.0,
            None,
            None,
        ))
        .await
        .expect("save l1");
    store
        .save(&make_task_info(
            "l2",
            "m",
            TaskStatus::Completed,
            1.0,
            None,
            None,
        ))
        .await
        .expect("save l2");
    let done = store.list(Some(TaskStatus::Completed)).await.expect("list");
    let ids: Vec<String> = done.into_iter().map(|i| i.task_id).collect();
    assert_eq!(ids, vec!["l2".to_string()]);
}

// clause: async_tasks.list.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_property_idempotent() {
    // Repeated list calls over an unchanged store return the same ids.
    let store = InMemoryTaskStore::new();
    for id in ["l1", "l2"] {
        store
            .save(&make_task_info(
                id,
                "m",
                TaskStatus::Pending,
                1.0,
                None,
                None,
            ))
            .await
            .expect("save");
    }
    let a: std::collections::HashSet<String> = store
        .list(None)
        .await
        .expect("a")
        .into_iter()
        .map(|i| i.task_id)
        .collect();
    let b: std::collections::HashSet<String> = store
        .list(None)
        .await
        .expect("b")
        .into_iter()
        .map(|i| i.task_id)
        .collect();
    let expected: std::collections::HashSet<String> =
        ["l1".to_string(), "l2".to_string()].into_iter().collect();
    assert_eq!(a, b);
    assert_eq!(a, expected);
}

// ---------------------------------------------------------------------------
// Contract: TaskStore.delete
// ---------------------------------------------------------------------------

// clause: async_tasks.delete.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_delete_property_async() {
    // TaskStore::delete is async (D-17) and resolves to Ok(()), removing the record.
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "d1",
            "m",
            TaskStatus::Completed,
            1.0,
            None,
            None,
        ))
        .await
        .expect("save");
    store.delete("d1").await.expect("delete resolves to Ok(())");
    assert!(store.get("d1").await.expect("get ok").is_none());
}

// clause: async_tasks.delete.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_delete_property_idempotent() {
    // Deleting an already-absent task id succeeds silently (no error, no change).
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "d1",
            "m",
            TaskStatus::Completed,
            1.0,
            None,
            None,
        ))
        .await
        .expect("save");
    store.delete("d1").await.expect("delete1");
    store.delete("d1").await.expect("delete2 is a silent no-op");
    assert!(store.get("d1").await.expect("get ok").is_none());
    assert!(store.list(None).await.expect("list").is_empty());
}

// ---------------------------------------------------------------------------
// Contract: TaskStore.list_expired
// ---------------------------------------------------------------------------

// clause: async_tasks.list_expired.property.async
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_expired_property_async() {
    // TaskStore::list_expired is async (D-17) and returns a Vec.
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "e1",
            "m",
            TaskStatus::Completed,
            1.0,
            Some(10.0),
            None,
        ))
        .await
        .expect("save");
    let expired = store.list_expired(100.0).await.expect("list_expired");
    let ids: Vec<String> = expired.into_iter().map(|i| i.task_id).collect();
    assert_eq!(ids, vec!["e1".to_string()]);
}

// clause: async_tasks.list_expired.eligible.terminal_only
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_expired_eligible_terminal_only() {
    // Only terminal tasks with completed_at < before_timestamp are returned;
    // PENDING/RUNNING tasks (no completed_at) are never returned.
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "done",
            "m",
            TaskStatus::Completed,
            1.0,
            Some(10.0),
            None,
        ))
        .await
        .expect("save done");
    store
        .save(&make_task_info(
            "pending",
            "m",
            TaskStatus::Pending,
            1.0,
            None,
            None,
        ))
        .await
        .expect("save pending");
    store
        .save(&make_task_info(
            "running",
            "m",
            TaskStatus::Running,
            1.0,
            None,
            Some(2.0),
        ))
        .await
        .expect("save running");
    let expired = store.list_expired(100.0).await.expect("list_expired");
    let ids: Vec<String> = expired.into_iter().map(|i| i.task_id).collect();
    assert_eq!(ids, vec!["done".to_string()]);
}

// clause: async_tasks.list_expired.input.before_timestamp.strict
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_expired_input_before_timestamp_strict() {
    // Expiry is strict (completed_at < before_timestamp): a task whose
    // completed_at equals before_timestamp is NOT expired.
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "eq",
            "m",
            TaskStatus::Completed,
            1.0,
            Some(50.0),
            None,
        ))
        .await
        .expect("save");
    assert!(store.list_expired(50.0).await.expect("eq").is_empty());
    let expired = store.list_expired(50.0001).await.expect("after");
    let ids: Vec<String> = expired.into_iter().map(|i| i.task_id).collect();
    assert_eq!(ids, vec!["eq".to_string()]);
}

// clause: async_tasks.list_expired.property.idempotent
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_expired_property_idempotent() {
    // Repeated list_expired calls over an unchanged store return the same ids
    // and never mutate state.
    let store = InMemoryTaskStore::new();
    store
        .save(&make_task_info(
            "e1",
            "m",
            TaskStatus::Completed,
            1.0,
            Some(10.0),
            None,
        ))
        .await
        .expect("save");
    let a: std::collections::HashSet<String> = store
        .list_expired(100.0)
        .await
        .expect("a")
        .into_iter()
        .map(|i| i.task_id)
        .collect();
    let b: std::collections::HashSet<String> = store
        .list_expired(100.0)
        .await
        .expect("b")
        .into_iter()
        .map(|i| i.task_id)
        .collect();
    let expected: std::collections::HashSet<String> = ["e1".to_string()].into_iter().collect();
    assert_eq!(a, b);
    assert_eq!(a, expected);
    assert_eq!(store.list(None).await.expect("list").len(), 1);
}

// ---------------------------------------------------------------------------
// "Store errors reach the caller" (D-81) and insertion order (D-82)
// ---------------------------------------------------------------------------

/// A store that answers every call with `TASK_STORE_UNAVAILABLE`, except the
/// `save` that admits a task (so a task can exist before the store goes down).
struct FlakyStore {
    inner: InMemoryTaskStore,
    down: std::sync::atomic::AtomicBool,
}

impl FlakyStore {
    fn new() -> Self {
        Self {
            inner: InMemoryTaskStore::new(),
            down: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn go_down(&self) {
        self.down.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn outage(&self) -> Result<(), ModuleError> {
        if self.down.load(std::sync::atomic::Ordering::SeqCst) {
            // The CANONICAL code D-92 requires the SDK to define. This store
            // used to raise `GeneralInternalError` with the string in its
            // message, which is what a host would have had to do before D-92
            // landed — and which left the whole D-81 suite green against a
            // manager that swallowed `TaskStoreUnavailable` specifically.
            return Err(ModuleError::new(
                ErrorCode::TaskStoreUnavailable,
                "backing store is unreachable",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl TaskStore for FlakyStore {
    async fn save(&self, task: &TaskInfo) -> Result<(), ModuleError> {
        self.outage()?;
        self.inner.save(task).await
    }
    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ModuleError> {
        self.outage()?;
        self.inner.get(id).await
    }
    async fn list(&self, status: Option<TaskStatus>) -> Result<Vec<TaskInfo>, ModuleError> {
        self.outage()?;
        self.inner.list(status).await
    }
    async fn delete(&self, id: &str) -> Result<(), ModuleError> {
        self.outage()?;
        self.inner.delete(id).await
    }
    async fn list_expired(&self, before: f64) -> Result<Vec<TaskInfo>, ModuleError> {
        self.outage()?;
        self.inner.list_expired(before).await
    }
    fn store_type_name(&self) -> &'static str {
        "FlakyStore"
    }
}

// clause: async_tasks.store_error.reaches_the_caller
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_store_error_reaches_the_caller() {
    // D-81: a store outage must NOT be absorbed into `false` / `None` / an
    // empty list. Before this, a store outage reported "task not found" and
    // "no tasks" — and, worst of all, cancel() returned true after a save that
    // never landed.
    let store = Arc::new(FlakyStore::new());
    let manager = AsyncTaskManager::with_store(
        make_executor(),
        4,
        100,
        Arc::clone(&store) as Arc<dyn TaskStore>,
    );
    let task_id = manager
        .submit("test.slow", json!({"delay": 1.0}), None)
        .await
        .expect("submit while the store is healthy");
    tokio::time::sleep(Duration::from_millis(50)).await;

    store.go_down();

    // Assert the CODE, not merely `is_err()`. A manager that maps the store's
    // outage onto some other error still hides which failure the caller is
    // looking at, and — the case that actually escaped — `is_err()` stays true
    // against a manager that swallows `TaskStoreUnavailable` specifically
    // while re-raising everything else, because the old FlakyStore raised
    // `GeneralInternalError`.
    fn unavailable(what: &str, r: Result<impl std::fmt::Debug, ModuleError>) {
        match r {
            Err(e) => assert_eq!(
                e.code,
                ErrorCode::TaskStoreUnavailable,
                "{what} must propagate the store's own code"
            ),
            Ok(v) => panic!("{what} absorbed a store outage into {v:?}"),
        }
    }

    unavailable("get_status", manager.get_status(&task_id));
    unavailable("get_status_async", manager.get_status_async(&task_id).await);
    unavailable("get_result", manager.get_result(&task_id));
    unavailable("get_result_async", manager.get_result_async(&task_id).await);
    unavailable("list_tasks", manager.list_tasks(None));
    unavailable("task_count", manager.task_count());
    unavailable("cleanup", manager.cleanup(0.0));
    unavailable("cancel", manager.cancel(&task_id).await);
    unavailable(
        "submit",
        manager
            .submit("test.slow", json!({"delay": 1.0}), None)
            .await,
    );
    unavailable("shutdown", manager.shutdown().await);
}

/// Control for `async_tasks_store_error_reaches_the_caller`: with the store
/// healthy, every one of those methods succeeds. Without it, "they all
/// returned Err" would also be satisfied by a manager that fails everything.
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_store_error_control_healthy_store_succeeds() {
    let store = Arc::new(FlakyStore::new());
    let manager = AsyncTaskManager::with_store(
        make_executor(),
        4,
        100,
        Arc::clone(&store) as Arc<dyn TaskStore>,
    );
    let task_id = manager
        .submit("test.slow", json!({"delay": 1.0}), None)
        .await
        .expect("submit");

    manager.get_status(&task_id).expect("get_status");
    manager
        .get_status_async(&task_id)
        .await
        .expect("get_status_async");
    manager.list_tasks(None).expect("list_tasks");
    manager.task_count().expect("task_count");
    manager.cleanup(0.0).expect("cleanup");
    manager.cancel(&task_id).await.expect("cancel");
    manager.shutdown().await.expect("shutdown");
}

// clause: async_tasks.list_tasks.return.insertion_order
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_list_tasks_return_insertion_order() {
    // D-82: list order is submission order. The store used to sort on
    // `task_id` — a UUID v4, hence random with respect to submission — so
    // `list_tasks()[0]` returned the lexicographically smallest UUID.
    let manager = make_manager();
    let mut submitted = Vec::new();
    for i in 0..12 {
        submitted.push(
            manager
                .submit("test.slow", json!({"delay": 1.0, "i": i}), None)
                .await
                .expect("submit"),
        );
    }

    let listed: Vec<String> = manager
        .list_tasks(None)
        .expect("store list")
        .into_iter()
        .map(|t| t.task_id)
        .collect();
    assert_eq!(listed, submitted, "list_tasks must be in submission order");

    // A `task_id` sort does not satisfy it: assert the two orders actually
    // differ, so this test would fail against the old implementation.
    let mut by_id = submitted.clone();
    by_id.sort();
    assert_ne!(
        by_id, submitted,
        "12 UUIDs should not happen to be submitted in sorted order"
    );

    // A status transition is not a re-submission: the order survives one.
    manager
        .cancel(&submitted[0])
        .await
        .expect("cancel the first task");
    let after: Vec<String> = manager
        .list_tasks(None)
        .expect("store list")
        .into_iter()
        .map(|t| t.task_id)
        .collect();
    assert_eq!(after, submitted, "an overwrite must keep its position");

    manager.shutdown().await.expect("store shutdown");
}

// Silence unused-import lint if HashMap ends up unreferenced in some builds.
#[allow(dead_code)]
fn _unused_hashmap_marker() -> HashMap<String, Value> {
    HashMap::new()
}

// ---------------------------------------------------------------------------
// D-122 — shutdown() attempts every cancellation before it reports
// ---------------------------------------------------------------------------

/// Fails the CANCELLED write for exactly one task id; healthy for every other.
///
/// The interesting failure mode is NOT a total outage — there, stopping at the
/// first error and attempting all reach the same place, because every
/// cancellation fails. It is a PER-TASK failure (a conditional-write conflict,
/// a corrupted record), where stopping strands every task after it.
struct OneBadRecordStore {
    inner: InMemoryTaskStore,
    bad_id: String,
}

#[async_trait]
impl TaskStore for OneBadRecordStore {
    async fn save(&self, task: &TaskInfo) -> Result<(), ModuleError> {
        if task.task_id == self.bad_id && task.status == TaskStatus::Cancelled {
            return Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "TASK_STORE_UNAVAILABLE: conditional write conflict",
            ));
        }
        self.inner.save(task).await
    }
    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ModuleError> {
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
    fn store_type_name(&self) -> &'static str {
        "OneBadRecordStore"
    }
}

// clause: async_tasks.shutdown.attempts_every_cancellation
#[tokio::test(flavor = "multi_thread")]
async fn async_tasks_shutdown_attempts_every_cancellation() {
    // D-122: a per-task store failure must not strand the tasks after it. An
    // uncancelled task in a shared store holds a `max_tasks` slot for every
    // manager sharing it and outlives the process that could have cancelled it,
    // while a slower shutdown is transient.
    let store = Arc::new(OneBadRecordStore {
        inner: InMemoryTaskStore::new(),
        bad_id: "t2".to_string(),
    });
    for id in ["t1", "t2", "t3"] {
        let info = make_task_info(id, "m.probe", TaskStatus::Pending, 0.0, None, None);
        store.inner.save(&info).await.expect("seed");
    }

    let manager = AsyncTaskManager::with_store(
        make_executor(),
        8,
        100,
        Arc::clone(&store) as Arc<dyn TaskStore>,
    );

    let outcome = manager.shutdown().await;
    assert!(
        outcome.is_err(),
        "the failing cancellation MUST still be reported"
    );

    let by_id: std::collections::HashMap<String, TaskStatus> = store
        .inner
        .list(None)
        .await
        .expect("list")
        .into_iter()
        .map(|i| (i.task_id, i.status))
        .collect();
    assert_eq!(
        by_id.get("t1"),
        Some(&TaskStatus::Cancelled),
        "the task before the failure must be cancelled"
    );
    assert_eq!(
        by_id.get("t3"),
        Some(&TaskStatus::Cancelled),
        "the task AFTER the failure must be cancelled too — stopping at the \
         first failure strands it holding a max_tasks slot (D-122)"
    );
}
