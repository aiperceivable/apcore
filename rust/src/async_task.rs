// APCore Protocol — Async task manager for background module execution
// Spec reference: docs/features/async-tasks.md
// Issue #34: Pluggable TaskStore, retry with exponential backoff, Reaper TTL cleanup.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::context::Context;
use crate::errors::{ErrorCode, ModuleError};
use crate::executor::Executor;

/// Status of an async task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
}

/// Metadata and result tracking for a submitted async task.
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1. `task_id` and `module_id`
/// default to empty strings and SHOULD be set explicitly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TaskInfo {
    pub task_id: String,
    pub module_id: String,
    pub status: TaskStatus,
    pub submitted_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub max_retries: u32,
}

/// Returns the current time as seconds since the UNIX epoch.
fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ---------------------------------------------------------------------------
// TaskStore
// ---------------------------------------------------------------------------

/// Pluggable backing store for [`TaskInfo`] records.
///
/// Implementations decouple task state from in-process memory and enable
/// distributed deployments. The default backend is [`InMemoryTaskStore`].
///
/// Concrete implementations include `RedisTaskStore` and `SqlTaskStore`
/// (provided as optional add-ons in downstream crates).
#[async_trait]
pub trait TaskStore: Send + Sync {
    /// Persist a task record (create or overwrite).
    async fn save(&self, task: &TaskInfo) -> Result<(), ModuleError>;

    /// Look up a task by id; returns `None` if no record exists.
    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ModuleError>;

    /// List all tasks, optionally filtered by exact status match.
    async fn list(&self, status: Option<TaskStatus>) -> Result<Vec<TaskInfo>, ModuleError>;

    /// Remove a task record. No-op if `id` is absent.
    async fn delete(&self, id: &str) -> Result<(), ModuleError>;

    /// Return all terminal-state tasks whose `completed_at` is strictly less
    /// than `before_timestamp` (Unix seconds). Pending and Running tasks MUST
    /// NOT be returned by this method.
    async fn list_expired(&self, before_timestamp: f64) -> Result<Vec<TaskInfo>, ModuleError>;

    /// Identifier of the concrete store type, used by tooling to expose the
    /// active backend (matches the type name; e.g. `"InMemoryTaskStore"`).
    ///
    /// Defaults to the implementing type's own name, without its module path.
    fn store_type_name(&self) -> &'static str {
        let full = std::any::type_name::<Self>();
        let base = full.split('<').next().unwrap_or(full);
        base.rsplit("::").next().unwrap_or(base)
    }
}

/// Default in-memory [`TaskStore`] backed by [`DashMap`] for lock-free
/// concurrent access.
///
/// # Insertion order (D-82)
///
/// `list` and `list_expired` return records in **submission order**. `DashMap`
/// has no insertion order of its own, so each record is stamped with a
/// monotonic sequence number on first save and the listings sort on that.
/// Sorting on `task_id` — which this store used to do — is not a substitute:
/// the id is a UUID v4, so it is random with respect to submission, and
/// `list_tasks()[0]` returned the lexicographically smallest UUID rather than
/// the first-submitted task.
#[derive(Default)]
pub struct InMemoryTaskStore {
    /// `task_id -> (insertion sequence, record)`.
    tasks: DashMap<String, (u64, TaskInfo)>,
    /// Monotonic counter stamped onto each record the first time it is saved.
    next_seq: std::sync::atomic::AtomicU64,
}

impl InMemoryTaskStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl TaskStore for InMemoryTaskStore {
    async fn save(&self, task: &TaskInfo) -> Result<(), ModuleError> {
        use dashmap::mapref::entry::Entry;
        match self.tasks.entry(task.task_id.clone()) {
            // An overwrite keeps the record's original position: a status
            // transition is not a re-submission.
            Entry::Occupied(mut occupied) => {
                occupied.get_mut().1 = task.clone();
            }
            Entry::Vacant(vacant) => {
                let seq = self
                    .next_seq
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                vacant.insert((seq, task.clone()));
            }
        }
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ModuleError> {
        Ok(self.tasks.get(id).map(|entry| entry.value().1.clone()))
    }

    async fn list(&self, status: Option<TaskStatus>) -> Result<Vec<TaskInfo>, ModuleError> {
        let mut out: Vec<(u64, TaskInfo)> = self
            .tasks
            .iter()
            .filter(|entry| match status {
                Some(s) => entry.value().1.status == s,
                None => true,
            })
            .map(|entry| entry.value().clone())
            .collect();
        // D-82: insertion (submission) order is normative.
        out.sort_by_key(|(seq, _)| *seq);
        Ok(out.into_iter().map(|(_, info)| info).collect())
    }

    async fn delete(&self, id: &str) -> Result<(), ModuleError> {
        self.tasks.remove(id);
        Ok(())
    }

    async fn list_expired(&self, before_timestamp: f64) -> Result<Vec<TaskInfo>, ModuleError> {
        let mut out: Vec<(u64, TaskInfo)> = self
            .tasks
            .iter()
            .filter(|entry| {
                let info = &entry.value().1;
                if !info.status.is_terminal() {
                    return false;
                }
                match info.completed_at {
                    Some(ts) => ts < before_timestamp,
                    None => false,
                }
            })
            .map(|entry| entry.value().clone())
            .collect();
        out.sort_by_key(|(seq, _)| *seq);
        Ok(out.into_iter().map(|(_, info)| info).collect())
    }

    fn store_type_name(&self) -> &'static str {
        "InMemoryTaskStore"
    }
}

// ---------------------------------------------------------------------------
// RetryConfig
// ---------------------------------------------------------------------------

/// Retry policy applied per task on failure.
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RetryConfig {
    /// Maximum number of retry attempts after the initial execution. `0` disables retry.
    pub max_retries: u32,
    /// Base delay between retries in milliseconds.
    pub retry_delay_ms: u64,
    /// Multiplicative factor applied at each attempt (`1.0` = constant, `2.0` = exponential doubling).
    pub backoff_multiplier: f64,
    /// Upper bound on the computed retry delay, in milliseconds.
    pub max_retry_delay_ms: u64,
}

impl Default for RetryConfig {
    /// Spec-aligned default (D-14): `max_retries = 0` matches both
    /// `apcore-python` and `apcore-typescript`. Retries are explicitly opt-in;
    /// this prevents the SDK from silently re-running failed tasks behind the
    /// caller's back. The 1s base delay / 2.0× backoff / 60s ceiling stay
    /// useful when callers DO opt into retries, so they're preserved.
    fn default() -> Self {
        Self {
            max_retries: 0,
            retry_delay_ms: 1000,
            backoff_multiplier: 2.0,
            max_retry_delay_ms: 60_000,
        }
    }
}

impl RetryConfig {
    /// Compute the retry delay for the given attempt index (`0`-based).
    ///
    /// Formula: `min(retry_delay_ms * (backoff_multiplier ^ attempt), max_retry_delay_ms)`.
    ///
    /// Returns whole milliseconds: the fractional part is truncated, where
    /// apcore-python and apcore-typescript return a float (D-61). The two
    /// differ by under 1 ms, and only for a non-integer product.
    ///
    /// Cross-language: this is the canonical name across `apcore-python` and
    /// `apcore-typescript` (sync alignment D-08). The legacy
    /// [`Self::delay_for_attempt`] alias delegates to this method and is
    /// `#[deprecated]`; it will be removed in the next minor version.
    #[must_use]
    pub fn compute_delay_ms(&self, attempt: u32) -> u64 {
        // Use f64 arithmetic to apply the backoff factor, then clamp to the cap.
        // The `as f64` casts are intentional: retry delays are bounded small
        // integers in practice (sub-second to minutes), so precision loss is a
        // non-issue.
        #[allow(clippy::cast_precision_loss)]
        let base = self.retry_delay_ms as f64;
        // `powf` accepts an f64 exponent and avoids the i32 cast lint while
        // producing the same result for non-negative integer attempt values.
        let raw = base * self.backoff_multiplier.powf(f64::from(attempt));
        #[allow(clippy::cast_precision_loss)]
        let cap = self.max_retry_delay_ms as f64;
        let capped = raw.min(cap);
        // Saturate negative or NaN values to zero, then truncate.
        if !capped.is_finite() || capped <= 0.0 {
            return 0;
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let out = capped as u64;
        out
    }

    /// Deprecated alias for [`Self::compute_delay_ms`] (sync alignment D-08).
    ///
    /// Kept for one minor version to allow callers a graceful migration.
    #[must_use]
    #[deprecated(since = "0.21.0", note = "use compute_delay_ms")]
    pub fn delay_for_attempt(&self, attempt: u32) -> u64 {
        self.compute_delay_ms(attempt)
    }
}

// ---------------------------------------------------------------------------
// Reaper
// ---------------------------------------------------------------------------

/// Configuration for the [`AsyncTaskManager`] background reaper.
///
/// Marked `#[non_exhaustive]` (issue #24) so a future spec revision can add a
/// field without a major version bump. That works by **removing struct-literal
/// construction from every crate but this one** — `..Default::default()`
/// included, since it is itself a struct expression (`error[E0639]`). From a
/// downstream crate, start from `Default::default()` and assign the fields you
/// need; there is no builder for this type. See
/// `api-surface-conventions.md` §9.1.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ReaperConfig {
    /// Age threshold (seconds) before a terminal task becomes eligible for deletion.
    pub ttl_seconds: f64,
    /// Sweep interval (milliseconds) between reaper runs.
    pub sweep_interval_ms: u64,
}

impl Default for ReaperConfig {
    fn default() -> Self {
        Self {
            ttl_seconds: 3600.0,
            sweep_interval_ms: 300_000,
        }
    }
}

/// Handle returned by [`AsyncTaskManager::start_reaper`] to control the
/// background reaper task.
///
/// Call [`ReaperHandle::stop`] to signal cancellation and await termination.
/// Dropping the handle DETACHES the reaper: the sweep loop keeps running, so
/// the manager's single-reaper guard deliberately stays set — dropping a handle
/// is not a way to stop a reaper, and must not let a second sweep loop start
/// alongside the first. [`AsyncTaskManager::stop_reaper`] (which
/// [`AsyncTaskManager::shutdown`] calls) stops a detached reaper.
#[derive(Debug)]
pub struct ReaperHandle {
    handle: Option<JoinHandle<()>>,
    stop_tx: watch::Sender<bool>,
    /// Shared `running` flag owned by the [`AsyncTaskManager`] that spawned
    /// this reaper. Cleared on `stop()` so a subsequent `start_reaper` call
    /// succeeds. NOT cleared on drop — see the type docs.
    running_flag: Arc<std::sync::atomic::AtomicBool>,
    /// The manager's slot holding this reaper's stop sender, cleared by
    /// `stop()` so the manager does not keep signalling a dead reaper.
    stop_slot: Arc<Mutex<Option<watch::Sender<bool>>>>,
}

impl ReaperHandle {
    /// Signal the reaper to stop and await its clean shutdown.
    pub async fn stop(mut self) {
        // Receivers may have already been dropped — ignore the send error.
        let _ = self.stop_tx.send(true);
        // Take the JoinHandle so we can `.await` it without moving out of
        // `self` (the Drop impl below releases the running flag).
        if let Some(handle) = self.handle.take() {
            if let Err(err) = handle.await {
                if !err.is_cancelled() {
                    warn!("reaper task join failed: {err}");
                }
            }
        }
        self.stop_slot.lock().take();
        self.running_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

// No `Drop` impl on purpose. Clearing `running_flag` here without signalling
// `stop_tx` was the defect: the sweep loop kept running while the guard it was
// holding was released, so the next `start_reaper` won its compare_exchange and
// a SECOND sweep loop ran alongside the first, both deleting from the same
// store. Dropping a handle detaches — and a detached reaper is still running,
// so the flag stays set. Use `ReaperHandle::stop` or
// `AsyncTaskManager::stop_reaper` to actually stop one.

// ---------------------------------------------------------------------------
// AsyncTaskManager
// ---------------------------------------------------------------------------

struct TaskHandle {
    join: JoinHandle<()>,
    // Shared with the worker's normal deregistration path, so cancellation
    // cannot reinsert a handle after that worker has already removed itself.
    finished: Arc<AtomicBool>,
}

#[cfg(test)]
impl From<JoinHandle<()>> for TaskHandle {
    fn from(join: JoinHandle<()>) -> Self {
        Self {
            join,
            finished: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Manages background execution of modules via tokio tasks.
///
/// Bounds concurrency with a semaphore, persists task state through a
/// pluggable [`TaskStore`], and supports retry with exponential backoff and
/// an opt-in [`ReaperHandle`] for TTL-based cleanup.
pub struct AsyncTaskManager {
    executor: Arc<Executor>,
    max_tasks: usize,
    store: Arc<dyn TaskStore>,
    handles: Arc<Mutex<HashMap<String, TaskHandle>>>,
    execution_tokens: Arc<Mutex<HashMap<String, crate::cancel::CancelToken>>>,
    cancellation_lock: tokio::sync::Mutex<()>,
    semaphore: Arc<Semaphore>,
    // Serialises the capacity-check + save sequence in `submit_with_retry`.
    // Without this guard, two concurrent submits can both observe
    // `store.list().len() < max_tasks` and both `save()` — exceeding the cap.
    // The lock is held only for the synchronous `block_on_local` poll; we never
    // hold it across a real .await (the in-memory store resolves immediately,
    // and a yielding store would already panic in `block_on_local`).
    /// Async-aware admission lock — held across `check_capacity_and_save.await`
    /// so concurrent submitters serialize through the capacity check + save.
    /// `tokio::sync::Mutex` (rather than `parking_lot::Mutex`) because the
    /// guard is alive across an `.await` point in `submit_with_retry`; a
    /// sync mutex's `!Send` guard would block `tokio::spawn` of any code
    /// path that touches `submit` (D10-003 alignment).
    admission_lock: Arc<tokio::sync::Mutex<()>>,
    /// Tracks whether a reaper is currently running. Closes A-D-AT-05 —
    /// `start_reaper` returns `Err(ReaperAlreadyRunning)` when this flag is
    /// already set. Cleared by `ReaperHandle::stop` (and on drop) so a
    /// caller can `stop()` then `start_reaper()` again.
    reaper_running: Arc<std::sync::atomic::AtomicBool>,
    /// Stop sender for the reaper this manager started, retained so
    /// [`Self::shutdown`] can stop it even when the [`ReaperHandle`] was
    /// dropped (detached). `shutdown` used to leave the reaper sweeping
    /// forever, while apcore-python (`stop_reaper`) and apcore-typescript both
    /// stop theirs.
    reaper_stop: Arc<Mutex<Option<watch::Sender<bool>>>>,
}

impl AsyncTaskManager {
    /// Create a new manager backed by the default [`InMemoryTaskStore`].
    ///
    /// # Arguments
    ///
    /// * `executor` — Executor used to invoke modules.
    /// * `max_concurrent` — Maximum simultaneously running tasks (semaphore size).
    /// * `max_tasks` — Maximum tracked task records (rejects further submits).
    pub fn new(executor: Arc<Executor>, max_concurrent: usize, max_tasks: usize) -> Self {
        Self::with_store(
            executor,
            max_concurrent,
            max_tasks,
            Arc::new(InMemoryTaskStore::new()),
        )
    }

    /// Create a manager with a caller-provided [`TaskStore`] implementation.
    pub fn with_store(
        executor: Arc<Executor>,
        max_concurrent: usize,
        max_tasks: usize,
        store: Arc<dyn TaskStore>,
    ) -> Self {
        // A task's module can make nested calls like any other.
        executor.enable_nested_calls();
        Self {
            executor,
            max_tasks,
            store,
            handles: Arc::new(Mutex::new(HashMap::new())),
            execution_tokens: Arc::new(Mutex::new(HashMap::new())),
            cancellation_lock: tokio::sync::Mutex::new(()),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            admission_lock: Arc::new(tokio::sync::Mutex::new(())),
            reaper_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            reaper_stop: Arc::new(Mutex::new(None)),
        }
    }

    /// Identifier of the underlying store backend.
    pub fn store_type_name(&self) -> &'static str {
        self.store.store_type_name()
    }

    /// Borrow the underlying [`TaskStore`] handle (for direct interaction or
    /// custom maintenance routines).
    pub fn store(&self) -> Arc<dyn TaskStore> {
        Arc::clone(&self.store)
    }

    /// Submit a module call for background execution. See [`Self::submit_with_retry`].
    ///
    /// Async (D10-003): the spec async-tasks.md contract declares submit
    /// async, matching apcore-python `async def submit` and apcore-typescript
    /// `async submit`. Awaiting submit is required so the capacity check
    /// + store.save are observed atomically before the task ID is returned.
    pub async fn submit(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        context: Option<Context<serde_json::Value>>,
    ) -> Result<String, ModuleError> {
        self.submit_with_retry(module_id, inputs, context, None)
            .await
    }

    /// Submit a module call with optional retry policy.
    ///
    /// Returns the generated task ID (UUID v4). Spawns a background tokio task
    /// that will acquire a concurrency permit before invoking the executor.
    /// On failure, when `retry` is supplied, the task is rescheduled after
    /// the policy-derived backoff delay until `max_retries` is exhausted.
    pub async fn submit_with_retry(
        &self,
        module_id: &str,
        inputs: serde_json::Value,
        context: Option<Context<serde_json::Value>>,
        retry: Option<RetryConfig>,
    ) -> Result<String, ModuleError> {
        // Construct the initial TaskInfo and reserve a slot in the store.
        // Concurrent submits are serialised through `admission_lock` so the
        // capacity check and subsequent save below are atomic.
        let task_id = Uuid::new_v4().to_string();
        let max_retries = retry.as_ref().map_or(0, |r| r.max_retries);
        let info = TaskInfo {
            task_id: task_id.clone(),
            module_id: module_id.to_string(),
            status: TaskStatus::Pending,
            submitted_at: now_secs(),
            started_at: None,
            completed_at: None,
            result: None,
            error: None,
            retry_count: 0,
            max_retries,
        };

        // Capacity check + persist must be atomic with respect to other
        // concurrent submits. tokio::sync::Mutex's guard is `Send` so
        // submit_with_retry is itself Send-safe and can be spawned via
        // tokio::spawn — required for the async `pub async fn submit`
        // entry point to compose with the broader async runtime.
        {
            let _admit = self.admission_lock.lock().await;
            check_capacity_and_save(&*self.store, self.max_tasks, &info).await?;
        }

        let handles = Arc::clone(&self.handles);
        let execution_tokens = Arc::clone(&self.execution_tokens);
        let mut context = context.unwrap_or_else(Context::anonymous);
        let execution_token = context.cancel_token.as_ref().map_or_else(
            crate::cancel::CancelToken::new,
            crate::cancel::CancelToken::child,
        );
        context.cancel_token = Some(execution_token.clone());
        execution_tokens
            .lock()
            .insert(task_id.clone(), execution_token.clone());
        let semaphore = Arc::clone(&self.semaphore);
        let executor = Arc::clone(&self.executor);
        let store_for_run = Arc::clone(&self.store);
        let mid = module_id.to_string();
        let tid = task_id.clone();

        // The spawned task can finish BEFORE the parent registers its
        // `JoinHandle` — a fast failure such as MODULE_NOT_FOUND, with a free
        // semaphore permit and an immediately-ready `InMemoryTaskStore`, reaches
        // the `remove` below while this thread is still between `spawn` and
        // `insert` on a multi-threaded runtime. The removal then found nothing
        // and a stale handle was inserted for an already-terminal task, so
        // `cancel()` would later `abort()` a finished task and `handles` grew
        // without bound.
        //
        // `finished` closes the window: the task sets it BEFORE taking the
        // `handles` lock, and the parent reads it while HOLDING that lock. So
        // either the parent observes the flag and never inserts, or its insert
        // is ordered before the task's remove. Neither interleaving leaves a
        // stale entry.
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished_in_task = Arc::clone(&finished);

        let handle = tokio::spawn(async move {
            run_task(
                tid.clone(),
                mid,
                inputs,
                Some(context),
                retry,
                executor,
                semaphore,
                store_for_run,
            )
            .await;
            // The store may already expose a timeout while the invocation is
            // still cleaning up cooperatively. Keep bookkeeping until actual
            // completion, including after cleanup() has deleted its record.
            execution_token.drain().await;
            finished_in_task.store(true, std::sync::atomic::Ordering::SeqCst);
            handles.lock().remove(&tid);
            execution_tokens.lock().remove(&tid);
        });

        {
            let mut registered = self.handles.lock();
            if finished.load(std::sync::atomic::Ordering::SeqCst) {
                // Already terminal: registering now would strand the entry.
                drop(handle);
            } else {
                registered.insert(
                    task_id.clone(),
                    TaskHandle {
                        join: handle,
                        finished,
                    },
                );
            }
        }

        Ok(task_id)
    }

    /// Return the current snapshot of a task, or `None` if unknown.
    ///
    /// This is the synchronous wrapper used by the existing public API. For
    /// network-backed stores, prefer [`Self::get_status_async`].
    ///
    /// # Errors
    ///
    /// Propagates the store's error (D-81). `Ok(None)` means the store
    /// answered and has no such task; a store outage is an `Err`, never a
    /// "not found".
    pub fn get_status(&self, task_id: &str) -> Result<Option<TaskInfo>, ModuleError> {
        block_on_local(self.store.get(task_id))
    }

    /// Async variant of [`Self::get_status`] for network-backed stores.
    ///
    /// # Errors
    ///
    /// Propagates the store's error (D-81).
    pub async fn get_status_async(&self, task_id: &str) -> Result<Option<TaskInfo>, ModuleError> {
        self.store.get(task_id).await
    }

    /// Return the result of a completed task or an error if not found / not
    /// completed.
    ///
    /// # Errors
    ///
    /// The store's own error is propagated unchanged (D-81); a task that is
    /// absent or not yet complete is reported as
    /// [`ErrorCode::GeneralInternalError`].
    pub fn get_result(&self, task_id: &str) -> Result<serde_json::Value, ModuleError> {
        block_on_local(self.get_result_async(task_id))
    }

    /// Async variant of [`Self::get_result`] for network-backed stores.
    ///
    /// # Errors
    ///
    /// As [`Self::get_result`].
    pub async fn get_result_async(&self, task_id: &str) -> Result<serde_json::Value, ModuleError> {
        let info = self.store.get(task_id).await?.ok_or_else(|| {
            ModuleError::new(
                ErrorCode::GeneralInternalError,
                format!("Task not found: {task_id}"),
            )
        })?;
        if info.status != TaskStatus::Completed {
            return Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                format!("Task {task_id} is not completed (status={:?})", info.status),
            ));
        }
        Ok(info.result.unwrap_or(serde_json::Value::Null))
    }

    /// Cancel a pending or running task. Returns `true` if cancellation was applied.
    ///
    /// Async (D10-004): spec async-tasks.md cancel contract declares the
    /// method async. Python (`async def cancel`) and TypeScript
    /// (`async cancel`) already comply; Rust now matches via
    /// `pub async fn cancel`. Drain semantics typically require await.
    ///
    /// # Errors
    ///
    /// Propagates the store's error (D-81). This is the case the decision
    /// calls out as worst: this method used to swallow a failed `save` and
    /// still return `true`, telling the caller the task was terminated while
    /// it kept running. The cancellation OUTCOME is still the boolean —
    /// `Ok(false)` for an unknown or already-terminal task.
    pub async fn cancel(&self, task_id: &str) -> Result<bool, ModuleError> {
        let _cancellation = self.cancellation_lock.lock().await;
        let Some(info) = self.store.get(task_id).await? else {
            return Ok(false);
        };
        if !info.status.is_active() {
            return Ok(false);
        }
        self.stop_local_task(task_id).await;

        // State may have changed while the physical execution was draining.
        let Some(mut updated) = self.store.get(task_id).await? else {
            return Ok(false);
        };
        if !updated.status.is_active() {
            return Ok(false);
        }
        updated.status = TaskStatus::Cancelled;
        updated.completed_at = Some(now_secs());
        self.store.save(&updated).await?;
        Ok(true)
    }

    async fn stop_local_task(&self, task_id: &str) {
        // A caller can drop cancel()/shutdown() at either await. Return any
        // unfinished join to the manager rather than detaching its teardown.
        struct PendingJoin<'a> {
            task_id: &'a str,
            handles: &'a Mutex<HashMap<String, TaskHandle>>,
            handle: Option<TaskHandle>,
        }
        impl Drop for PendingJoin<'_> {
            fn drop(&mut self) {
                if let Some(handle) = self.handle.take() {
                    let mut handles = self.handles.lock();
                    if !handle.finished.load(Ordering::SeqCst) && !handle.join.is_finished() {
                        handles.insert(self.task_id.to_owned(), handle);
                    }
                }
            }
        }
        let mut pending = PendingJoin {
            task_id,
            handles: &self.handles,
            handle: self.handles.lock().remove(task_id),
        };
        if let Some(handle) = pending.handle.as_mut() {
            handle.join.abort();
            let _ = (&mut handle.join).await;
        }
        pending.handle.take();
        // Keep the token registered until the physical invocation tree has
        // drained, even when this particular cancellation waiter is dropped.
        let execution_token = self.execution_tokens.lock().get(task_id).cloned();
        if let Some(token) = execution_token {
            token.cancel();
            token.abort_owned_executions().await;
            self.execution_tokens.lock().remove(task_id);
        }
    }

    /// Signal this manager's reaper to stop and release the single-reaper
    /// guard, whether or not its [`ReaperHandle`] is still held.
    ///
    /// Returns `true` when a reaper was running. Idempotent. Mirrors
    /// apcore-python `AsyncTaskManager.stop_reaper`; apcore-typescript stops
    /// its sweep timer in the same place.
    ///
    /// Does not await the sweep loop's final iteration — hold the
    /// [`ReaperHandle`] and call [`ReaperHandle::stop`] when that matters.
    pub fn stop_reaper(&self) -> bool {
        let sender = self.reaper_stop.lock().take();
        let was_running = sender.is_some();
        if let Some(stop_tx) = sender {
            // Receivers may already be gone — ignore the send error.
            let _ = stop_tx.send(true);
        }
        self.reaper_running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        was_running
    }

    /// Cancel all pending and running tasks, and stop the reaper.
    ///
    /// The reaper stop is what apcore-python's `shutdown` does first
    /// (`await self.stop_reaper()`) and what apcore-typescript's clears; this
    /// SDK left a started reaper sweeping after `shutdown()` returned.
    ///
    /// Every active task is attempted even after one cancellation fails, and the
    /// first failure is returned once they all have been (D-122). The two
    /// failure modes are not symmetric: if the store is unreachable every
    /// cancellation fails and stopping early reaches the same outcome sooner,
    /// but if ONE task fails, stopping leaves every remaining task uncancelled
    /// when they could have been cancelled. An uncancelled task in a shared
    /// store is a lasting cost — it holds a `max_tasks` slot for every manager
    /// sharing that store, and outlives the process that could have cancelled
    /// it — while a slower shutdown is transient. This method is already an
    /// unbounded wait by contract and takes no timeout, so a caller needing a
    /// bound already imposes one.
    ///
    /// # Errors
    ///
    /// Propagates the store's error (D-81): the `list` failure immediately, or
    /// the first `cancel` failure after every active task has been attempted.
    /// The reaper is stopped before the store is touched, so a store outage
    /// still leaves the reaper stopped.
    pub async fn shutdown(&self) -> Result<(), ModuleError> {
        self.stop_reaper();

        let mut first_error: Option<ModuleError> = None;
        let mut task_ids: Vec<String> = self.handles.lock().keys().cloned().collect();
        task_ids.extend(self.execution_tokens.lock().keys().cloned());
        match self.store.list(None).await {
            Ok(tasks) => task_ids.extend(
                tasks
                    .into_iter()
                    .filter_map(|info| info.status.is_active().then_some(info.task_id)),
            ),
            Err(error) => first_error = Some(error),
        }
        task_ids.sort();
        task_ids.dedup();
        for task_id in task_ids {
            if let Err(err) = self.cancel(&task_id).await {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
            // Explicit shutdown owns physical teardown even for terminal
            // records with late cleanup, or when the store is unavailable.
            let _cancellation = self.cancellation_lock.lock().await;
            self.stop_local_task(&task_id).await;
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Return all tasks, optionally filtered by status. Synchronous wrapper.
    ///
    /// The list is in submission order (D-82). An empty `Ok` list means the
    /// store answered and nothing matched.
    ///
    /// # Errors
    ///
    /// Propagates the store's error (D-81) rather than reporting "no tasks"
    /// for a store that is merely unreachable.
    pub fn list_tasks(&self, status: Option<TaskStatus>) -> Result<Vec<TaskInfo>, ModuleError> {
        block_on_local(self.list_tasks_async(status))
    }

    /// Async variant of [`Self::list_tasks`] for network-backed stores.
    ///
    /// # Errors
    ///
    /// Propagates the store's error (D-81).
    pub async fn list_tasks_async(
        &self,
        status: Option<TaskStatus>,
    ) -> Result<Vec<TaskInfo>, ModuleError> {
        self.store.list(status).await
    }

    /// Remove terminal-state tasks older than `max_age_seconds`. Returns the
    /// count of removed tasks. Synchronous wrapper.
    ///
    /// # Errors
    ///
    /// Propagates the store's error (D-81). A failed `delete` aborts the sweep
    /// rather than being counted as a removal that never happened.
    pub fn cleanup(&self, max_age_seconds: f64) -> Result<usize, ModuleError> {
        block_on_local(self.cleanup_async(max_age_seconds))
    }

    /// Async variant of [`Self::cleanup`] for network-backed stores.
    ///
    /// # Errors
    ///
    /// As [`Self::cleanup`].
    pub async fn cleanup_async(&self, max_age_seconds: f64) -> Result<usize, ModuleError> {
        let now = now_secs();
        let to_remove: Vec<String> = self
            .store
            .list(None)
            .await?
            .into_iter()
            .filter(|info| info.status.is_terminal())
            .filter(|info| {
                let ref_time = info.completed_at.unwrap_or(info.submitted_at);
                (now - ref_time) >= max_age_seconds
            })
            .map(|info| info.task_id)
            .collect();

        let count = to_remove.len();
        for id in &to_remove {
            self.store.delete(id).await?;
            self.handles.lock().remove(id);
            // A reported timeout may still be cleaning up cooperatively.
            // Keep its scope until shutdown/cancel has actually drained it.
            self.execution_tokens
                .lock()
                .retain(|task_id, token| task_id != id || token.has_active_executions());
        }
        Ok(count)
    }

    /// Total tracked task count across all states.
    ///
    /// # Errors
    ///
    /// Propagates the store's error (D-81) rather than reporting `0` for a
    /// store that is merely unreachable.
    pub fn task_count(&self) -> Result<usize, ModuleError> {
        Ok(block_on_local(self.store.list(None))?.len())
    }

    /// Start the opt-in background reaper. Returns a [`ReaperHandle`] used to
    /// stop the reaper gracefully.
    ///
    /// At most one reaper may be running per manager. Calling `start_reaper`
    /// while another reaper is still live returns
    /// `Err(ModuleError { code: ReaperAlreadyRunning, .. })`. Call
    /// [`ReaperHandle::stop`] (or drop the handle) before starting a new one.
    ///
    /// **Cross-language note (A-C-004/A-C-005):** apcore-python and
    /// apcore-typescript now also raise `ModuleError`/`ModuleError` with this
    /// same code for a double `start_reaper`, matching Rust. They previously
    /// raised an untyped `RuntimeError` / plain `Error`, which a caller
    /// writing `except ModuleError` could not catch.
    ///
    /// # Errors
    /// Returns `ModuleError` with [`ErrorCode::ReaperAlreadyRunning`] when a
    /// reaper handle from a prior `start_reaper` call has not yet been
    /// stopped or dropped.
    pub fn start_reaper(&self, config: ReaperConfig) -> Result<ReaperHandle, ModuleError> {
        // Compare-and-swap from `false` → `true` so concurrent calls do not
        // race past the check. `Err` indicates the flag was already `true`.
        if self
            .reaper_running
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err(ModuleError::new(
                ErrorCode::ReaperAlreadyRunning,
                "AsyncTaskManager reaper is already running; call stop() first",
            ));
        }

        let store = Arc::clone(&self.store);
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let interval = Duration::from_millis(config.sweep_interval_ms);
        let ttl = config.ttl_seconds;

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    changed = stop_rx.changed() => {
                        if changed.is_ok() && *stop_rx.borrow() {
                            debug!("reaper received stop signal");
                            return;
                        }
                    }
                    () = tokio::time::sleep(interval) => {
                        let before = now_secs() - ttl;
                        match store.list_expired(before).await {
                            Ok(expired) => {
                                let count = expired.len();
                                for info in &expired {
                                    if let Err(err) = store.delete(&info.task_id).await {
                                        warn!(task_id = %info.task_id, "reaper delete failed: {err}");
                                    }
                                }
                                if count > 0 {
                                    debug!("reaper deleted {count} expired tasks");
                                }
                            }
                            Err(err) => {
                                warn!("reaper list_expired failed: {err}");
                            }
                        }
                    }
                }
            }
        });

        // Retain the sender so `shutdown` / `stop_reaper` can stop this reaper
        // even after the handle below is dropped (detached).
        *self.reaper_stop.lock() = Some(stop_tx.clone());

        Ok(ReaperHandle {
            handle: Some(handle),
            stop_tx,
            running_flag: Arc::clone(&self.reaper_running),
            stop_slot: Arc::clone(&self.reaper_stop),
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Capacity-check then persist a new task record.
///
/// Counts only **active** tasks (`Pending` + `Running`) so terminal-state
/// records — completed/failed/cancelled tasks still living in the store
/// pending TTL-based cleanup — do not consume the `max_tasks` budget.
/// Mirrors apcore-python `_ACTIVE_STATUSES` and apcore-typescript
/// `_isActiveStatus` (closes A-D-AT-01).
async fn check_capacity_and_save(
    store: &dyn TaskStore,
    max_tasks: usize,
    info: &TaskInfo,
) -> Result<(), ModuleError> {
    let current = store.list(None).await?;
    let active = current.iter().filter(|t| t.status.is_active()).count();
    if active >= max_tasks {
        return Err(ModuleError::new(
            ErrorCode::TaskLimitExceeded,
            format!("Task limit reached ({max_tasks})"),
        ));
    }
    store.save(info).await?;
    Ok(())
}

/// Run a single task with optional retry/backoff. Persists state transitions
/// via the [`TaskStore`].
#[allow(clippy::too_many_arguments)]
async fn run_task(
    task_id: String,
    module_id: String,
    inputs: serde_json::Value,
    context: Option<Context<serde_json::Value>>,
    retry: Option<RetryConfig>,
    executor: Arc<Executor>,
    semaphore: Arc<Semaphore>,
    store: Arc<dyn TaskStore>,
) {
    let max_retries = retry.as_ref().map_or(0, |r| r.max_retries);

    loop {
        // Acquire concurrency permit. A closed semaphore is treated as
        // cancellation (matches the legacy `_run` behaviour).
        let Ok(permit) = semaphore.acquire().await else {
            mark_cancelled(&store, &task_id).await;
            return;
        };

        // Re-fetch and short-circuit if cancellation happened during the wait.
        let Ok(Some(mut info)) = store.get(&task_id).await else {
            return;
        };
        if info.status == TaskStatus::Cancelled {
            return;
        }
        info.status = TaskStatus::Running;
        if info.started_at.is_none() {
            info.started_at = Some(now_secs());
        }
        if let Err(err) = store.save(&info).await {
            error!(task_id = %task_id, "store.save(running) failed: {err}");
            return;
        }

        // Execute the module.
        let result = executor
            .call(&module_id, inputs.clone(), context.as_ref(), None)
            .await;
        drop(permit);

        // Re-fetch in case cancel() raced with the executor call.
        let Ok(Some(mut info)) = store.get(&task_id).await else {
            return;
        };
        if info.status == TaskStatus::Cancelled {
            return;
        }

        match result {
            Ok(output) => {
                info.status = TaskStatus::Completed;
                info.completed_at = Some(now_secs());
                info.result = Some(output);
                save_terminal_if_not_cancelled(&store, &task_id, &info).await;
                return;
            }
            Err(err) => {
                if let Some(cfg) = retry.as_ref() {
                    if info.retry_count < max_retries {
                        let delay_ms = cfg.compute_delay_ms(info.retry_count);
                        info.retry_count += 1;
                        info.status = TaskStatus::Pending;
                        // `started_at` is intentionally NOT reset across retries:
                        // it captures the wall-clock of the first execution and
                        // matches the Python reference behaviour so cross-language
                        // TaskInfo snapshots remain comparable mid-retry.
                        if let Err(e) = store.save(&info).await {
                            warn!(task_id = %task_id, "retry state save failed: {e}");
                        }
                        debug!(
                            task_id = %task_id,
                            attempt = info.retry_count,
                            delay_ms,
                            "scheduling retry"
                        );
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        continue;
                    }
                }
                info.status = TaskStatus::Failed;
                info.completed_at = Some(now_secs());
                info.error = Some(err.to_string());
                save_terminal_if_not_cancelled(&store, &task_id, &info).await;
                error!(task_id = %task_id, "task failed: {err}");
                return;
            }
        }
    }
}

/// Re-fetches the task immediately before writing a terminal status. If a
/// concurrent `cancel()` flipped the task to Cancelled while the executor was
/// finishing, leave that state intact — do not overwrite it with
/// Completed/Failed. The intermediate `re-fetch + check + save` is not
/// atomic at the store level, so a perfectly-timed cancel can still slip in
/// between the `get` and the `save`; closing that final window would require
/// CAS support in the `TaskStore` trait. The window left here is small and
/// the cost of slipping (one extra terminal write that the next cancel will
/// no-op against) is non-corrupting.
pub(crate) async fn save_terminal_if_not_cancelled(
    store: &Arc<dyn TaskStore>,
    task_id: &str,
    info: &TaskInfo,
) {
    if let Ok(Some(current)) = store.get(task_id).await {
        if current.status == TaskStatus::Cancelled {
            return;
        }
    }
    // Background task: there is no caller to propagate to (D-81 governs the
    // manager surface), so a store failure is logged rather than dropped.
    if let Err(e) = store.save(info).await {
        warn!(task_id = %task_id, "terminal state save failed: {e}");
    }
}

async fn mark_cancelled(store: &Arc<dyn TaskStore>, task_id: &str) {
    if let Ok(Some(mut info)) = store.get(task_id).await {
        if info.status.is_active() {
            info.status = TaskStatus::Cancelled;
            info.completed_at = Some(now_secs());
            // Background task: no caller to propagate to; log instead.
            if let Err(e) = store.save(&info).await {
                warn!(task_id = %task_id, "cancelled state save failed: {e}");
            }
        }
    }
}

/// Drive a future to completion synchronously by polling it once with a
/// no-op waker. This is intentional: the manager exposes a synchronous
/// facade on top of an async [`TaskStore`], and the supported in-process
/// stores ([`InMemoryTaskStore`]) resolve without yielding so a single poll
/// is sufficient. For network-backed stores callers MUST use the `_async`
/// variants instead.
///
/// If a custom store actually yields, this panics — surfacing the misuse
/// rather than silently deadlocking the calling thread.
fn block_on_local<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    use std::pin::pin;
    use std::ptr;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    // No-op waker: synchronous stores never wake themselves; if they did, we
    // would not be in this code path.
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );
    // SAFETY: the vtable above performs no operations on the data pointer,
    // so a null pointer is safe for every callback.
    let waker = unsafe { Waker::from_raw(RawWaker::new(ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!(
            "block_on_local: TaskStore future yielded — use the _async variants for non-blocking stores"
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Executor;
    use crate::registry::registry::Registry;

    struct UnavailableStore;
    #[async_trait]
    impl TaskStore for UnavailableStore {
        async fn save(&self, _task: &TaskInfo) -> Result<(), ModuleError> {
            Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "store save unavailable",
            ))
        }
        async fn get(&self, _id: &str) -> Result<Option<TaskInfo>, ModuleError> {
            Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "store get unavailable",
            ))
        }
        async fn list(&self, _status: Option<TaskStatus>) -> Result<Vec<TaskInfo>, ModuleError> {
            Err(ModuleError::new(
                ErrorCode::GeneralInternalError,
                "store list unavailable",
            ))
        }
        async fn delete(&self, _id: &str) -> Result<(), ModuleError> {
            Ok(())
        }
        async fn list_expired(&self, _before: f64) -> Result<Vec<TaskInfo>, ModuleError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_shutdown_drains_local_work_when_store_is_unavailable() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Cleanup(Arc<AtomicBool>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let manager =
            AsyncTaskManager::with_store(make_executor(), 1, 100, Arc::new(UnavailableStore));
        let cleaned = Arc::new(AtomicBool::new(false));
        let cleanup = Cleanup(cleaned.clone());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _cleanup = cleanup;
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        manager.handles.lock().insert("local".into(), handle.into());
        started_rx.await.unwrap();
        let error = manager.shutdown().await.unwrap_err();
        assert_eq!(error.message, "store list unavailable");
        assert!(cleaned.load(Ordering::Acquire));
        assert!(manager.handles.lock().is_empty());
    }

    #[tokio::test]
    async fn test_cancel_preserves_terminal_state_written_during_join() {
        struct CompleteOnDrop(Arc<InMemoryTaskStore>);
        impl Drop for CompleteOnDrop {
            fn drop(&mut self) {
                let mut stored = self.0.tasks.get_mut("race").unwrap();
                stored.1.status = TaskStatus::Completed;
                stored.1.result = Some(serde_json::json!({"winner": "completion"}));
            }
        }
        let store = Arc::new(InMemoryTaskStore::new());
        let manager = AsyncTaskManager::with_store(make_executor(), 1, 100, store.clone());
        store
            .save(&TaskInfo {
                task_id: "race".into(),
                module_id: "module".into(),
                status: TaskStatus::Running,
                submitted_at: 0.0,
                started_at: Some(0.0),
                completed_at: None,
                result: None,
                error: None,
                retry_count: 0,
                max_retries: 0,
            })
            .await
            .unwrap();
        let cleanup = CompleteOnDrop(store);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _cleanup = cleanup;
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        manager.handles.lock().insert("race".into(), handle.into());
        started_rx.await.unwrap();
        assert!(!manager.cancel("race").await.unwrap());
        let result = manager.get_status("race").unwrap().unwrap();
        assert_eq!(result.status, TaskStatus::Completed);
        assert_eq!(result.result.unwrap()["winner"], "completion");
    }

    fn make_executor() -> Arc<Executor> {
        let registry = Arc::new(Registry::default());
        let config = Arc::new(crate::config::Config::default());
        Arc::new(Executor::new(registry, config))
    }

    struct LateCleanupModule(Arc<tokio::sync::Notify>);
    #[async_trait]
    impl crate::module::Module for LateCleanupModule {
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn output_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn description(&self) -> &'static str {
            "Wait for late cooperative cleanup"
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _context: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, ModuleError> {
            self.0.notified().await;
            Ok(serde_json::json!({}))
        }
    }

    #[tokio::test]
    async fn test_reclaimed_timed_out_record_releases_scope_after_late_cleanup() {
        let finish = Arc::new(tokio::sync::Notify::new());
        let registry = Arc::new(Registry::new());
        registry
            .register_module("late", Box::new(LateCleanupModule(finish.clone())))
            .unwrap();
        let mut config = crate::config::Config::default();
        config.set("executor.default_timeout", serde_json::json!(10));
        let manager =
            AsyncTaskManager::new(Arc::new(Executor::new(registry, Arc::new(config))), 1, 100);
        let task_id = manager
            .submit("late", serde_json::json!({}), None)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.get_status(&task_id).unwrap().unwrap().status != TaskStatus::Failed {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(manager.cleanup(0.0).unwrap(), 1);
        assert!(!manager.execution_tokens.lock().is_empty());
        finish.notify_one();
        tokio::time::timeout(Duration::from_millis(100), async {
            while !manager.execution_tokens.lock().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("scope remained after deleted record's late execution finished");
    }

    struct PendingCleanupModule {
        started: Arc<tokio::sync::Notify>,
        cleaned: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl crate::module::Module for PendingCleanupModule {
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn output_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn description(&self) -> &'static str {
            "Keep execution alive until cancelled"
        }
        async fn execute(
            &self,
            _inputs: serde_json::Value,
            _context: &Context<serde_json::Value>,
        ) -> Result<serde_json::Value, ModuleError> {
            struct Cleanup(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    self.0.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            let _cleanup = Cleanup(self.cleaned.clone());
            self.started.notify_one();
            std::future::pending().await
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_dropped_cancel_keeps_execution_tracked_for_shutdown() {
        use std::future::Future;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::task::{Context as TaskContext, Waker};

        for drain_started in [false, true] {
            let started = Arc::new(tokio::sync::Notify::new());
            let cleaned = Arc::new(AtomicBool::new(false));
            let registry = Arc::new(Registry::new());
            registry
                .register_module(
                    "pending",
                    Box::new(PendingCleanupModule {
                        started: started.clone(),
                        cleaned: cleaned.clone(),
                    }),
                )
                .unwrap();
            let manager = AsyncTaskManager::new(
                Arc::new(Executor::new(
                    registry,
                    Arc::new(crate::config::Config::default()),
                )),
                1,
                100,
            );
            let id = manager
                .submit("pending", serde_json::json!({}), None)
                .await
                .unwrap();
            started.notified().await;
            let mut cancel = Box::pin(manager.cancel(&id));
            let mut context = TaskContext::from_waker(Waker::noop());
            assert!(cancel.as_mut().poll(&mut context).is_pending());
            if drain_started {
                tokio::task::yield_now().await;
                // Reach the execution-drain await after the manager worker is aborted.
                assert!(cancel.as_mut().poll(&mut context).is_pending());
            }
            drop(cancel);
            assert!(!cleaned.load(Ordering::Acquire));
            manager.shutdown().await.unwrap();
            assert!(
                cleaned.load(Ordering::Acquire),
                "shutdown lost the cancelled caller's execution"
            );
            assert!(manager.execution_tokens.lock().is_empty());
            assert!(manager.handles.lock().is_empty());
        }
    }

    #[tokio::test]
    async fn test_shutdown_drains_execution_after_timeout_record_is_reclaimed() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let started = Arc::new(tokio::sync::Notify::new());
        let cleaned = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(Registry::new());
        registry
            .register_module(
                "pending",
                Box::new(PendingCleanupModule {
                    started: started.clone(),
                    cleaned: cleaned.clone(),
                }),
            )
            .unwrap();
        let mut config = crate::config::Config::default();
        config.set("executor.default_timeout", serde_json::json!(10));
        let manager =
            AsyncTaskManager::new(Arc::new(Executor::new(registry, Arc::new(config))), 1, 100);
        let id = manager
            .submit("pending", serde_json::json!({}), None)
            .await
            .unwrap();
        started.notified().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.get_status(&id).unwrap().unwrap().status != TaskStatus::Failed {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(manager
            .get_status(&id)
            .unwrap()
            .unwrap()
            .error
            .unwrap()
            .contains("timed out"));
        assert_eq!(manager.cleanup(0.0).unwrap(), 1);
        assert!(manager.get_status(&id).unwrap().is_none());
        assert!(!cleaned.load(Ordering::Acquire));
        manager.shutdown().await.unwrap();
        assert!(cleaned.load(Ordering::Acquire));
        assert!(manager.execution_tokens.lock().is_empty());
    }

    #[tokio::test]
    async fn test_cancel_and_shutdown_wait_for_running_future_cleanup() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Cleanup(Arc<AtomicBool>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        for shutdown in [false, true] {
            let manager = Arc::new(AsyncTaskManager::new(make_executor(), 1, 100));
            let cleaned = Arc::new(AtomicBool::new(false));
            let cleanup = Cleanup(cleaned.clone());
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let handle = tokio::spawn(async move {
                let _cleanup = cleanup;
                let _ = started_tx.send(());
                std::future::pending::<()>().await;
            });
            manager
                .store
                .save(&TaskInfo {
                    task_id: "cleanup".into(),
                    module_id: "waiting".into(),
                    status: if shutdown {
                        TaskStatus::Completed
                    } else {
                        TaskStatus::Running
                    },
                    submitted_at: 0.0,
                    started_at: Some(0.0),
                    completed_at: None,
                    result: None,
                    error: None,
                    retry_count: 0,
                    max_retries: 0,
                })
                .await
                .unwrap();
            manager
                .handles
                .lock()
                .insert("cleanup".into(), handle.into());
            started_rx.await.unwrap();
            if shutdown {
                assert!(!manager.cancel("cleanup").await.unwrap());
                assert!(
                    !cleaned.load(Ordering::Acquire),
                    "terminal cancel must not interrupt late cleanup"
                );
                manager.shutdown().await.unwrap();
            } else {
                assert!(manager.cancel("cleanup").await.unwrap());
            }
            assert!(
                cleaned.load(Ordering::Acquire),
                "shutdown={shutdown}: cancellation returned before future Drop"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn test_concurrent_cancel_waits_for_the_first_callers_cleanup() {
        struct HeldCleanup {
            started: Option<tokio::sync::oneshot::Sender<()>>,
            release: Arc<std::sync::Barrier>,
        }
        impl Drop for HeldCleanup {
            fn drop(&mut self) {
                let _ = self.started.take().unwrap().send(());
                // Deliberately hold synchronous Drop on one worker so a second
                // caller can deterministically probe cancellation completion.
                self.release.wait();
            }
        }
        let manager = Arc::new(AsyncTaskManager::new(make_executor(), 1, 100));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropping_tx, dropping_rx) = tokio::sync::oneshot::channel();
        let release = Arc::new(std::sync::Barrier::new(2));
        let cleanup = HeldCleanup {
            started: Some(dropping_tx),
            release: release.clone(),
        };
        let handle = tokio::spawn(async move {
            let _cleanup = cleanup;
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        manager
            .store
            .save(&TaskInfo {
                task_id: "concurrent".into(),
                module_id: "waiting".into(),
                status: TaskStatus::Running,
                submitted_at: 0.0,
                started_at: Some(0.0),
                completed_at: None,
                result: None,
                error: None,
                retry_count: 0,
                max_retries: 0,
            })
            .await
            .unwrap();
        manager
            .handles
            .lock()
            .insert("concurrent".into(), handle.into());
        started_rx.await.unwrap();
        let first_manager = manager.clone();
        let first = tokio::spawn(async move { first_manager.cancel("concurrent").await });
        dropping_rx.await.unwrap();
        let mut second = Box::pin(manager.cancel("concurrent"));
        let second_poll = futures_util::poll!(second.as_mut());
        let returned_early = second_poll.is_ready();
        // Always release the worker, even when the regression is present.
        tokio::task::spawn_blocking(move || release.wait())
            .await
            .unwrap();
        assert!(first.await.unwrap().unwrap());
        assert!(
            !returned_early,
            "second cancel returned before future cleanup completed"
        );
        assert!(!second.await.unwrap());
    }

    #[test]
    fn retry_delay_grows_exponentially_and_caps() {
        let cfg = RetryConfig {
            max_retries: 5,
            retry_delay_ms: 1000,
            backoff_multiplier: 2.0,
            max_retry_delay_ms: 30_000,
        };
        assert_eq!(cfg.compute_delay_ms(0), 1000);
        assert_eq!(cfg.compute_delay_ms(1), 2000);
        assert_eq!(cfg.compute_delay_ms(2), 4000);
        assert_eq!(cfg.compute_delay_ms(3), 8000);
        assert_eq!(cfg.compute_delay_ms(4), 16_000);
        assert_eq!(cfg.compute_delay_ms(5), 30_000);
    }

    #[tokio::test]
    async fn default_store_is_in_memory() {
        let mgr = AsyncTaskManager::new(make_executor(), 4, 100);
        assert_eq!(mgr.store_type_name(), "InMemoryTaskStore");
    }

    #[tokio::test]
    async fn in_memory_store_save_and_get_round_trip() {
        let store = InMemoryTaskStore::new();
        let info = TaskInfo {
            task_id: "abc".into(),
            module_id: "data.process".into(),
            status: TaskStatus::Completed,
            submitted_at: 1.0,
            started_at: Some(2.0),
            completed_at: Some(3.0),
            result: Some(serde_json::json!({"ok": true})),
            error: None,
            retry_count: 0,
            max_retries: 0,
        };
        store.save(&info).await.unwrap();
        let got = store.get("abc").await.unwrap().unwrap();
        assert_eq!(got.task_id, "abc");
        assert_eq!(got.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn in_memory_store_list_filters_by_status() {
        let store = InMemoryTaskStore::new();
        for (id, status) in [
            ("c1", TaskStatus::Completed),
            ("c2", TaskStatus::Running),
            ("c3", TaskStatus::Failed),
        ] {
            store
                .save(&TaskInfo {
                    task_id: id.into(),
                    module_id: "m".into(),
                    status,
                    submitted_at: 0.0,
                    started_at: None,
                    completed_at: None,
                    result: None,
                    error: None,
                    retry_count: 0,
                    max_retries: 0,
                })
                .await
                .unwrap();
        }
        let completed = store.list(Some(TaskStatus::Completed)).await.unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].task_id, "c1");
    }

    #[tokio::test]
    async fn in_memory_store_list_expired_skips_active_tasks() {
        let store = InMemoryTaskStore::new();
        store
            .save(&TaskInfo {
                task_id: "old-completed".into(),
                module_id: "m".into(),
                status: TaskStatus::Completed,
                submitted_at: 0.0,
                started_at: Some(0.0),
                completed_at: Some(100.0),
                result: None,
                error: None,
                retry_count: 0,
                max_retries: 0,
            })
            .await
            .unwrap();
        store
            .save(&TaskInfo {
                task_id: "old-running".into(),
                module_id: "m".into(),
                status: TaskStatus::Running,
                submitted_at: 0.0,
                started_at: Some(0.0),
                completed_at: None,
                result: None,
                error: None,
                retry_count: 0,
                max_retries: 0,
            })
            .await
            .unwrap();
        let expired = store.list_expired(1000.0).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].task_id, "old-completed");
    }

    /// Regression: when a `cancel()` flips the task to Cancelled in the
    /// store between the run-task post-execution re-fetch and its terminal
    /// `save()`, the helper MUST refrain from overwriting the Cancelled
    /// status with Completed/Failed.
    #[tokio::test]
    async fn save_terminal_does_not_overwrite_cancelled() {
        let store: Arc<dyn TaskStore> = Arc::new(InMemoryTaskStore::new());
        let task_id = "race-task";

        // Seed the store with the cancelled state (as `cancel()` would have
        // written between re-fetch and the terminal save).
        store
            .save(&TaskInfo {
                task_id: task_id.into(),
                module_id: "m".into(),
                status: TaskStatus::Cancelled,
                submitted_at: 0.0,
                started_at: Some(0.0),
                completed_at: Some(1.0),
                result: None,
                error: None,
                retry_count: 0,
                max_retries: 0,
            })
            .await
            .unwrap();

        // The terminal info `run_task` would otherwise write — Completed
        // with a payload. The helper must observe the Cancelled state in
        // the store and skip the save.
        let terminal = TaskInfo {
            task_id: task_id.into(),
            module_id: "m".into(),
            status: TaskStatus::Completed,
            submitted_at: 0.0,
            started_at: Some(0.0),
            completed_at: Some(2.0),
            result: Some(serde_json::json!({"value": 42})),
            error: None,
            retry_count: 0,
            max_retries: 0,
        };
        save_terminal_if_not_cancelled(&store, task_id, &terminal).await;

        let after = store.get(task_id).await.unwrap().expect("task present");
        assert_eq!(
            after.status,
            TaskStatus::Cancelled,
            "terminal save MUST NOT overwrite a concurrent cancellation"
        );
        assert!(
            after.result.is_none(),
            "the cancel-time TaskInfo had no result; the overwriting Completed payload must not leak through"
        );
    }
}
