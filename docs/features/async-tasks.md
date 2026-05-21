# Async Task Management

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.8 Async Module Specification.


## Overview

The Async Task Management system provides background module execution with concurrency limiting, task lifecycle tracking, and result retrieval. It wraps the Executor to submit module calls as background tasks, each progressing through a defined status lifecycle. This enables fire-and-forget execution patterns, long-running operations, and concurrent workload management without blocking the caller.

## Requirements

- Provide an `AsyncTaskManager` class that accepts an Executor and manages background task execution.
- Tasks **MUST** progress through a defined status lifecycle: `pending` → `running` → `completed` | `failed` | `cancelled`.
- Concurrency **MUST** be bounded by a configurable semaphore (`max_concurrent`, default 10).
- Total task count **MUST** be bounded (`max_tasks`, default 1000) to prevent unbounded memory growth.
- Task submission **MUST** return immediately with a `task_id` (UUID v4).
- Support cancellation of pending and running tasks via cooperative `CancelToken`.
- Provide cleanup of terminal-state tasks older than a configurable age threshold.
- Support graceful shutdown that cancels all pending and running tasks.

## Technical Design

### TaskStatus

The lifecycle is exactly **5 states**, identical across all three SDKs:

| Status | Terminal | Description |
|--------|----------|-------------|
| `PENDING` | No | Submitted, waiting for a concurrency slot, or waiting in retry backoff |
| `RUNNING` | No | Concurrency slot acquired, module executing |
| `COMPLETED` | Yes | Module returned successfully |
| `FAILED` | Yes | Module raised an error and retries are exhausted (or `max_retries=0`) |
| `CANCELLED` | Yes | Task was cancelled before or during execution |

!!! note "Retry backoff is `PENDING`, not a separate state"
    Tasks awaiting their next retry attempt remain in `PENDING`. There is no `RETRYING` state. Earlier Python SDK builds (≤ v0.20) exposed a `TaskStatus.RETRYING` value; this has been removed in alignment with TypeScript and Rust. Observers that previously matched on `RETRYING` SHOULD treat `PENDING` with `retry_count > 0` as the equivalent signal.

### TaskInfo

The retry-attempt count field is canonically named `retry_count` (Python, Rust) / `retryCount` (TypeScript) — value is 0-indexed and reflects the number of retries already taken. Earlier Python builds named the field `attempt_number`; that name is retained as a deprecated read-only alias that returns `retry_count`.

=== "Python"
    ```python
    from dataclasses import dataclass
    from typing import Any

    @dataclass
    class TaskInfo:
        task_id: str                     # UUID v4
        module_id: str                   # Module being executed
        status: TaskStatus               # Current lifecycle status
        submitted_at: float              # Unix timestamp (seconds)
        started_at: float | None         # Set when status → running
        completed_at: float | None       # Set when status → terminal
        result: Any = None               # Output (completed only, type depends on module)
        error: str | None = None         # Error message (failed only)
        retry_count: int = 0             # Number of retries taken so far (0-indexed)
        max_retries: int = 0             # Configured retry budget for this task

        @property
        def attempt_number(self) -> int:
            # DEPRECATED — alias for retry_count, retained for one minor version
            return self.retry_count
    ```
=== "TypeScript"
    ```typescript
    interface TaskInfo {
        readonly taskId: string;
        readonly moduleId: string;
        readonly status: TaskStatus;
        readonly submittedAt: number;
        readonly startedAt: number | null;
        readonly completedAt: number | null;
        readonly result: Record<string, unknown> | null;
        readonly error: string | null;
        readonly retryCount: number;     // 0-indexed
        readonly maxRetries: number;
    }
    ```
=== "Rust"
    ```rust
    pub struct TaskInfo {
        pub task_id: String,
        pub module_id: String,
        pub status: TaskStatus,
        pub submitted_at: f64,
        pub started_at: Option<f64>,
        pub completed_at: Option<f64>,
        pub result: Option<serde_json::Value>,
        pub error: Option<String>,
        pub retry_count: u32,            // 0-indexed
        pub max_retries: u32,
    }
    ```

### AsyncTaskManager

=== "Python"
    ```python
    from apcore.async_task import AsyncTaskManager
    from apcore import Executor, Registry

    executor = Executor(registry=Registry())
    manager = AsyncTaskManager(executor, max_concurrent=10, max_tasks=1000)

    # Submit a background task
    task_id = await manager.submit("data.process_batch", {"items": large_list})

    # Check status
    info = manager.get_status(task_id)
    print(info.status)  # TaskStatus.PENDING or RUNNING

    # Wait and retrieve result (only when completed)
    result = manager.get_result(task_id)

    # Cancel a task
    cancelled = await manager.cancel(task_id)

    # List tasks (optionally filtered by status)
    all_tasks = manager.list_tasks()
    running = manager.list_tasks(status=TaskStatus.RUNNING)

    # Clean up old terminal tasks (default: older than 1 hour)
    removed = manager.cleanup(max_age_seconds=3600.0)

    # Graceful shutdown
    await manager.shutdown()
    ```
=== "TypeScript"
    ```typescript
    import { AsyncTaskManager, Executor, Registry } from "apcore-js";

    const executor = new Executor({ registry: new Registry() });
    const manager = new AsyncTaskManager({ executor, maxConcurrent: 10, maxTasks: 1000 });

    // Submit a background task
    const taskId = await manager.submit("data.process_batch", { items: largeList });

    // Check status
    const info = manager.getStatus(taskId);
    console.log(info?.status); // "pending" or "running"

    // Retrieve result (only when completed)
    const result = manager.getResult(taskId);

    // Cancel a task
    const cancelled = await manager.cancel(taskId);

    // List tasks (optionally filtered by status)
    const allTasks = manager.listTasks();
    const running = manager.listTasks("running");

    // Clean up old terminal tasks (default: older than 1 hour)
    const removed = manager.cleanup(3600);

    // Graceful shutdown
    await manager.shutdown();
    ```
=== "Rust"
    ```rust
    use apcore::async_task::AsyncTaskManager;
    use apcore::{Executor, Registry};

    let executor = Executor::new(registry);
    let mut manager = AsyncTaskManager::new(executor, 10, 1000);

    // Submit a background task
    let task_id = manager.submit(
        "data.process_batch",
        serde_json::json!({"items": large_list}),
        None,
    ).await?;

    // Check status
    if let Some(info) = manager.get_status(&task_id) {
        println!("Status: {:?}", info.status);
    }

    // Cancel a task
    let cancelled = manager.cancel(&task_id).await;

    // Clean up old terminal tasks
    let removed = manager.cleanup(3600.0);

    // Graceful shutdown
    manager.shutdown().await;
    ```

### API Reference

| Method | Returns | Description |
|--------|---------|-------------|
| `submit(module_id, inputs, context?)` | `task_id: str` | Submit a module call for background execution. Raises if `max_tasks` reached. |
| `get_status(task_id)` | `TaskInfo \| None` | Get current task info (returns copy). |
| `get_result(task_id)` | `Any` | Get result of a completed task. Raises if not found or not completed. |
| `cancel(task_id)` | `bool` | Cancel a pending or running task. Returns `true` if cancellation was applied. |
| `list_tasks(status?)` | `list[TaskInfo]` | List all tasks, optionally filtered by status. |
| `cleanup(max_age_seconds=3600)` | `int` | Remove terminal-state tasks older than the threshold. Returns count removed. |
| `shutdown()` | `None` | Cancel all pending/running tasks and wait for completion. |

### Concurrency Model

The manager uses a semaphore-based concurrency limiter:

1. On `submit()`, a task is created in `pending` state and enqueued for execution.
2. Before execution begins, the task acquires a concurrency slot (semaphore).
3. Once acquired, the task transitions to `running` and the module is invoked via the executor.
4. On completion (success or failure), the slot is released and the next queued task proceeds.
5. Cancellation is checked at two points: after slot acquisition and after execution, ensuring that tasks cancelled during the wait are not executed.

### Cancellation Integration

When `cancel()` is called on a running task:

1. If the task's context has a `CancelToken`, `token.cancel()` is called for cooperative cancellation.
2. If no `CancelToken` is available, the underlying async task is cancelled directly (e.g., `asyncio.Task.cancel()` in Python, `tokio::task::JoinHandle::abort()` in Rust).
3. The task transitions to `cancelled` state and its result is discarded.

**Normative — real interrupt across SDKs (D-18):** `cancel()` MUST interrupt the in-flight executor invocation, not merely set a cooperative flag. In every SDK the running future/promise/coroutine MUST be interrupted at the next suspension point such that further module-level statements do not execute. In TypeScript specifically, `CancelToken` MUST be backed by an `AbortController` and MUST expose its `signal: AbortSignal` on `Context` so that modules performing standard Web-API I/O (`fetch`, `setTimeout`, streams) participate in real abort. SDKs MAY document residual cooperative behavior at non-Web-API await points, but the contract is "cancel means cancel" across all three languages — a flag-only implementation is non-conforming.

## Dependencies

- **Executor** — Used to invoke modules via `call_async()`.
- **Context** — Optional context passed to module execution; carries `CancelToken`.
- **Cancellation System** — `CancelToken` enables cooperative cancellation of running tasks.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    **Source files:**

    | File | Purpose |
    |------|---------|
    | `src/apcore/async_task.py` | `AsyncTaskManager`, `TaskStatus`, `TaskInfo` |

## Testing Strategy

- **Lifecycle tests** verify the full status progression: pending → running → completed/failed/cancelled.
- **Concurrency tests** verify that no more than `max_concurrent` tasks run simultaneously.
- **Capacity tests** verify that submission is rejected when `max_tasks` is reached.
- **Cancellation tests** verify that pending tasks are cancelled before execution and running tasks receive cooperative cancellation.
- **Cleanup tests** verify that only terminal-state tasks older than the threshold are removed.
- **Shutdown tests** verify that all pending/running tasks are cancelled and the manager enters a clean state.
- **Result retrieval tests** verify that `get_result()` raises for non-completed tasks and returns the correct output for completed tasks.

## Contract: AsyncTaskManager.submit

### Inputs
- `module_id` (str/string/&str, required) — module to execute asynchronously
- `inputs` (dict/object/Value, required) — module inputs
- `context` (Context, optional) — execution context

### Errors
- `InvalidInputError(code=INVALID_MODULE_ID)` — malformed module_id
- `ModuleNotFoundError(code=MODULE_NOT_FOUND)` — no such module

### Returns
- On success: `AsyncTask` — task handle with `task_id`, `status`, `result` (when complete)

### Properties
- async: true
- thread_safe: true
- pure: false (spawns background work, persists task state)
- idempotent: false

## Contract: AsyncTaskManager.cancel

### Inputs
- `task_id` (str/string/&str, required) — ID of the task to cancel

### Errors
- None raised under normal operation. Implementations report cancellation outcome via the boolean return value rather than raising.

### Returns
- On success: `bool` — `true` if cancellation was applied (the task was active and is now `Cancelled`), `false` if the task did not exist or had already reached a terminal state.

### Properties
- async: true (D10-004 alignment — drain/cancel semantics require awaiting persisted state mutation; matches apcore-python `async def cancel`, apcore-typescript `async cancel`, and apcore-rust `pub async fn cancel`)
- thread_safe: true
- idempotent: true (calling cancel on an already-cancelled task returns `false` rather than raising; subsequent calls are no-ops)

---

## AsyncTaskManager Evolution (Issue #34)

This section defines three capability extensions to `AsyncTaskManager`: pluggable storage backends, configurable retry with exponential backoff, and automatic TTL-based cleanup via a Reaper background task.

### 1.1 Pluggable Task Storage

The `AsyncTaskManager` MUST support pluggable storage backends via a `TaskStore` interface. This decouples task state from in-process memory, enabling distributed deployments and persistence across process restarts.

**Normative rules:**

- Implementations MUST define a `TaskStore` protocol/interface/trait with the following methods (canonical names — identical across all three SDKs):
    - `save(task_info)` — create or overwrite a task record
    - `get(task_id) → TaskInfo | None` — retrieve a task by ID
    - `list(status_filter?) → List[TaskInfo]` — list all tasks, optionally filtered by status
    - `delete(task_id)` — remove a task record
    - `list_expired(before_timestamp) → List[TaskInfo]` — return tasks whose `completed_at` is before the given timestamp
- All `TaskStore` methods MUST be asynchronous in every SDK (Python `async def`, TypeScript returning `Promise<T>`, Rust `async fn` on the trait via `#[async_trait]`). This is required so that Redis-, SQL-, and other I/O-backed stores can be plugged in without blocking the runtime's event loop. The `InMemoryTaskStore` MUST still expose async signatures even though its operations are CPU-only — uniform shape lets callers and middleware compose stores generically. (Decision **D-17**, supersedes the partially-sync contract that existed in Python+TS through v0.21.x.)
- Implementations MUST provide `InMemoryTaskStore` as the default backend.
- Implementations SHOULD provide `RedisTaskStore` and `SqlTaskStore` as optional backends.
- The store MUST be injected at construction time: `AsyncTaskManager(store=InMemoryTaskStore())`.

!!! note "Python `TaskStore.put` deprecation"
    Earlier Python builds (≤ v0.20) named the create/overwrite method `put`. The canonical name across all three SDKs is now `save`. Python retains `put` as a deprecated thin wrapper that calls `save` and emits a `DeprecationWarning`. The wrapper will be removed in v0.22.

**Using the default `InMemoryTaskStore` (no change from existing API):**

=== "Python"
    ```python
    from apcore.async_task import AsyncTaskManager, InMemoryTaskStore
    from apcore import Executor, Registry

    executor = Executor(registry=Registry())
    # Default: InMemoryTaskStore is used when no store is specified
    manager = AsyncTaskManager(executor, store=InMemoryTaskStore())
    ```
=== "TypeScript"
    ```typescript
    import { AsyncTaskManager, InMemoryTaskStore, Executor, Registry } from "apcore-js";

    const executor = new Executor({ registry: new Registry() });
    // Default: InMemoryTaskStore is used when no store is specified
    const manager = new AsyncTaskManager({ executor, store: new InMemoryTaskStore() });
    ```
=== "Rust"
    ```rust
    use apcore::async_task::{AsyncTaskManager, InMemoryTaskStore};
    use apcore::{Executor, Registry};

    let executor = Executor::new(Registry::new());
    // Default: InMemoryTaskStore is used when no store is specified
    let manager = AsyncTaskManager::new(executor, InMemoryTaskStore::new());
    ```

**Injecting a `RedisTaskStore`:**

=== "Python"
    ```python
    from apcore.async_task import AsyncTaskManager, RedisTaskStore
    from apcore import Executor, Registry

    executor = Executor(registry=Registry())
    store = RedisTaskStore(redis_url="redis://localhost:6379", key_prefix="apcore:tasks:")
    manager = AsyncTaskManager(executor, store=store)
    ```
=== "TypeScript"
    ```typescript
    import { AsyncTaskManager, RedisTaskStore, Executor, Registry } from "apcore-js/async-task";

    const executor = new Executor({ registry: new Registry() });
    const store = new RedisTaskStore({ url: "redis://localhost:6379" });
    const manager = new AsyncTaskManager({ executor, store });
    ```
=== "Rust"
    ```rust
    use apcore::async_task::{AsyncTaskManager, RedisTaskStore};
    use apcore::{Executor, Registry};

    let executor = Executor::new(Registry::new());
    let store = RedisTaskStore::new("redis://localhost:6379")?;
    let manager = AsyncTaskManager::new(executor, store);
    ```

## Contract: TaskStore.save

### Inputs
- `task_info` (TaskInfo, required) — the task to persist (creates or overwrites)

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend is unreachable

### Returns
- On success: void/None/()

### Properties
- async: true (MAY be async for network-backed stores)
- thread_safe: true
- pure: false
- idempotent: true (calling save twice with same task_id overwrites)

---

### 1.2 Retry with Configurable Backoff

`AsyncTaskManager` MUST support per-task retry configuration to handle transient failures in module execution.

**Normative rules:**

- `AsyncTaskManager` MUST support per-task retry configuration with the following fields: `max_retries` (int, default 0), `retry_delay_ms` (int, default 1000), `backoff_multiplier` (float, default 2.0), `max_retry_delay_ms` (int, default 60000).
- When a task fails and `max_retries > 0`, the manager MUST reschedule the task with delay calculated as: `min(retry_delay_ms * (backoff_multiplier ^ attempt), max_retry_delay_ms)`.
- After exhausting all retries, the task status MUST be set to `FAILED` with the `error` field populated.
- Retry count and attempt number MUST be stored in `TaskInfo` as `retry_count` (current attempt number, 0-indexed) and `max_retries`.

!!! note "Rust `RetryConfig::default()` alignment"
    Earlier Rust builds (≤ v0.20) defaulted `max_retries` to `3`, surprising callers using `..Default::default()`. The Rust default is now `max_retries = 0`, matching Python and TypeScript and the spec — retries are strictly opt-in across all three SDKs.

**`RetryConfig` delay-computation method (canonical name — decision D-08 / D-49):**

| SDK | Canonical method | Returns | Deprecated alias |
|-----|------------------|---------|------------------|
| Python | `RetryConfig.compute_delay_ms(attempt: int) -> float` | float ms | — |
| TypeScript | `RetryConfig.computeDelayMs(attempt: number) -> number` | number ms | `computeDelay(attempt)` (one-shot deprecation warning; removal in v0.22.0) |
| Rust | `RetryConfig::compute_delay_ms(&self, attempt: u32) -> u64` | u64 ms (truncated) | `delay_for_attempt(&self, attempt)` (`#[deprecated]`; removal in v0.22.0) |

All three implementations MUST produce numerically equivalent values for the same inputs (subject to Rust's `u64` truncation of fractional milliseconds — see decision D-20).

**Default retry configuration (YAML):**

```yaml
async_task:
  default_retry:
    max_retries: 3
    retry_delay_ms: 1000
    backoff_multiplier: 2.0
    max_retry_delay_ms: 30000
```

**Submitting a task with custom retry configuration:**

=== "Python"
    ```python
    from apcore.async_task import AsyncTaskManager, RetryConfig

    manager = AsyncTaskManager(executor)

    task_id = await manager.submit(
        "data.process_batch",
        {"items": large_list},
        retry=RetryConfig(
            max_retries=3,
            retry_delay_ms=500,
            backoff_multiplier=2.0,
            max_retry_delay_ms=30000,
        ),
    )
    ```
=== "TypeScript"
    ```typescript
    import { AsyncTaskManager, RetryConfig } from "apcore-js/async-task";

    const manager = new AsyncTaskManager({ executor });

    const taskId = await manager.submit(
        "data.process_batch",
        { items: largeList },
        {
            retry: new RetryConfig({
                maxRetries: 3,
                retryDelayMs: 500,
                backoffMultiplier: 2.0,
                maxRetryDelayMs: 30000,
            }),
        },
    );
    ```
=== "Rust"
    ```rust
    use apcore::async_task::{AsyncTaskManager, RetryConfig};

    let manager = AsyncTaskManager::new(executor, store);

    let task_id = manager.submit(
        "data.process_batch",
        serde_json::json!({"items": large_list}),
        Some(RetryConfig {
            max_retries: 3,
            retry_delay_ms: 500,
            backoff_multiplier: 2.0,
            max_retry_delay_ms: 30000,
        }),
    ).await?;
    ```

---

### 1.3 Automatic TTL-Based Cleanup (Reaper)

The Reaper is an opt-in background task that automatically removes terminal-state tasks older than a configurable TTL, preventing unbounded storage growth in long-running deployments.

**Normative rules:**

- Implementations SHOULD run a Reaper background task that periodically calls `store.list_expired(before=now - ttl_seconds)` and deletes all returned tasks.
- The Reaper MUST be opt-in: it MUST NOT run unless `reaper_enabled: true` is configured.
- Default `ttl_seconds`: 3600 (1 hour). Default `sweep_interval_ms`: 300000 (5 minutes). These defaults are normative across **all three SDKs** (decision **D-48**); earlier per-SDK divergence (e.g. Rust's 600_000 builder, historical Python 60_000 helpers) has been retired.
- The Reaper MUST NOT delete tasks in `PENDING` or `RUNNING` status.
- When the Reaper deletes a task batch, it SHOULD log at DEBUG level with count.

**Reaper configuration (YAML):**

```yaml
async_task:
  reaper:
    enabled: true
    ttl_seconds: 7200
    sweep_interval_ms: 600000
```

**Enabling the Reaper at runtime:**

=== "Python"
    ```python
    from apcore.async_task import AsyncTaskManager

    manager = AsyncTaskManager(executor)

    # Start the background reaper; returns a handle to stop it later
    reaper_handle = await manager.start_reaper(
        ttl_seconds=7200,
        sweep_interval_ms=600000,
    )

    # ... application runs ...

    # Graceful shutdown
    await reaper_handle.stop()
    ```
=== "TypeScript"
    ```typescript
    import { AsyncTaskManager } from "apcore-js/async-task";

    const manager = new AsyncTaskManager({ executor });

    // Start the background reaper; returns a handle to stop it later
    const reaperHandle = await manager.startReaper({
        ttlSeconds: 7200,
        sweepIntervalMs: 600000,
    });

    // ... application runs ...

    // Graceful shutdown
    await reaperHandle.stop();
    ```
=== "Rust"
    ```rust
    use apcore::async_task::AsyncTaskManager;

    let manager = AsyncTaskManager::new(executor, store);

    // Start the background reaper; returns a handle to stop it later
    let reaper_handle = manager.start_reaper(7200.0, 600_000).await;

    // ... application runs ...

    // Graceful shutdown
    reaper_handle.stop().await;
    ```

## Contract: AsyncTaskManager.start_reaper

### Canonical signature

`start_reaper(ttl_seconds, sweep_interval_ms) -> ReaperHandle` is the canonical signature across **all three SDKs** (decision **D-11**). The two named arguments and the `ReaperHandle` return type are normative.

| SDK | Signature | Notes |
|-----|-----------|-------|
| Python | `await manager.start_reaper(ttl_seconds=3600.0, sweep_interval_ms=300_000) -> ReaperHandle` | Returns awaitable; `ReaperHandle.stop()` is async |
| TypeScript | `await manager.startReaper({ ttlSeconds, sweepIntervalMs }) -> Promise<ReaperHandle>` | Object-style kwargs; `reaperHandle.stop()` is async |
| Rust | `manager.start_reaper(ttl_seconds: f64, sweep_interval_ms: u64).await -> ReaperHandle` | `ReaperHandle::stop` is async |

### Python deprecation note

Pre-D-11 Python releases used `start_reaper(interval_seconds=..., max_age_seconds=...)` (sync, returned `None`, sweep unit was **seconds**). These keyword arguments are now **deprecated aliases**:

- `interval_seconds=N` — accepted with `DeprecationWarning("interval_seconds is deprecated; use sweep_interval_ms (note unit change to milliseconds)")`. Internally multiplied by 1000 to convert to milliseconds.
- `max_age_seconds=N` — accepted with `DeprecationWarning("max_age_seconds is deprecated; use ttl_seconds")`. Same unit (seconds), only the name changed.
- The deprecation aliases are scheduled for removal in the next MAJOR release.

```python
# Deprecated form (still works, emits DeprecationWarning)
handle = await manager.start_reaper(interval_seconds=600.0, max_age_seconds=7200.0)

# Canonical form (D-11 alignment)
handle = await manager.start_reaper(ttl_seconds=7200.0, sweep_interval_ms=600_000)
```

### Inputs
- `ttl_seconds` (float, optional, default=3600) — task age threshold in seconds; tasks with `completed_at` older than `now - ttl_seconds` are eligible for deletion
- `sweep_interval_ms` (int, optional, default=300000) — how often (in milliseconds) the Reaper sweeps for expired tasks

### Errors
- None — if the underlying store is unavailable during a sweep, log WARN and retry next interval

### Returns
- On success: `ReaperHandle` — a handle to stop the background task (call `.stop()` to cancel the Reaper)

### Properties
- async: true (spawns background coroutine/task/thread)
- thread_safe: true
- pure: false (starts background process)
- idempotent: false (calling twice starts two Reapers; implementations SHOULD guard against this)

---

## Contract: AsyncTaskManager.get_status

### Inputs
- `task_id` (str/string, required) — UUID v4 identifying the task

### Errors
- None — unknown `task_id` returns `None`/`null` rather than raising

### Returns
- On success: `TaskInfo | None` — the current snapshot of the task record, or `None`/`null` if no task with that ID exists
- The returned object MUST be a shallow copy in every SDK — Python returns `dataclasses.replace(info)`, TypeScript returns `{ ...info }`, Rust returns a clone. Callers MUST NOT rely on mutation of the returned value to propagate back to the store; conversely, store-side mutations MUST NOT be observable through a previously-returned snapshot. (Decision **D-23**, supersedes the pre-v0.22 Python behavior of returning a live reference.)

### Properties
- async: false
- thread_safe: true
- pure: false (reads mutable task state)
- idempotent: true

---

## Contract: AsyncTaskManager.get_result

### Inputs
- `task_id` (str/string, required) — UUID v4 identifying the task

### Errors
- `KeyError` / `Error("Task not found: <id>")` — no task with the given `task_id` exists; raised unconditionally regardless of task state
- `RuntimeError` / `Error("Task <id> is not completed (status=<value>)")` — task exists but status is not `COMPLETED`; this includes `PENDING`, `RUNNING`, `FAILED`, and `CANCELLED`

### Returns
- On success: the `result` field of the `TaskInfo` record — Python type is `Any` (module-defined); TypeScript type is `Record<string, unknown>`
- Result is only populated when `status == COMPLETED`; in all other terminal states (`FAILED`, `CANCELLED`) the result field is `None`/`null`

### Properties
- async: false
- thread_safe: true
- pure: false (reads mutable task state)
- idempotent: true

---

## Contract: AsyncTaskManager.list_tasks

### Inputs
- `status` (TaskStatus, optional) — when provided, only tasks with this exact status are returned; when omitted, all tasks are returned regardless of status

### Errors
- None

### Returns
- On success: `list[TaskInfo]` / `TaskInfo[]` — a snapshot list of matching task records. Each entry MUST be a shallow copy in every SDK (Python `dataclasses.replace(info)`, TypeScript `{ ...info }`, Rust `clone()`). See `get_status` and Decision **D-23** for the mutation-safety contract.
- The list order is insertion order (Python dict / JavaScript Map)
- An empty list is returned if no tasks match the filter

### Properties
- async: false
- thread_safe: true
- pure: false (reads mutable task state)
- idempotent: true

---

## Contract: AsyncTaskManager.cleanup

### Inputs
- `max_age_seconds` (float, optional, default=3600.0) — age threshold in seconds; only tasks whose reference timestamp is **at least** this many seconds in the past are removed

### Reference timestamp selection
The reference time used to compute age differs by task completion state:

- If `completed_at` is set (task reached a terminal state normally): `completed_at` is used
- If `completed_at` is `None` (e.g. task was never started): `submitted_at` is used as the fallback

### Eligible states
Only tasks in terminal states are considered: `COMPLETED`, `FAILED`, `CANCELLED`. Tasks in `PENDING` or `RUNNING` are never removed by `cleanup`.

### Errors
- None

### Returns
- On success: `int` — count of tasks removed from the store during this call; returns `0` if nothing was eligible

### Properties
- async: false
- thread_safe: true
- pure: false (mutates task store)
- idempotent: false (a second call with the same threshold removes nothing if all eligible tasks were already removed, but the side-effect on state differs from no-op)

---

## Contract: AsyncTaskManager.shutdown

### Inputs
- None

### Behavior
Iterates all tasks currently in `PENDING` or `RUNNING` state and cancels each one. In Python, each cancellation awaits the underlying `asyncio.Task` to finish (cooperative cancellation). In TypeScript, `cancel()` is called for each such task and then `Promise.allSettled` awaits all task promises to settle.

After `shutdown` returns, every task that was `PENDING` or `RUNNING` at the time of the call will be in `CANCELLED` state.

### Errors
- None raised to caller; unexpected exceptions from individual task bodies during cancellation are logged at `WARNING` level (Python) or printed to `console.warn` (TypeScript) and not re-raised

### Returns
- On success: `None` / `void`

### Properties
- async: true
- thread_safe: true
- pure: false (mutates task state)
- idempotent: true (calling shutdown on an already-shut-down manager with no active tasks is a no-op)

---

## Contract: TaskStore.get

!!! note "Planned interface — not yet implemented in any SDK"
    `TaskStore` is specified as part of the pluggable-storage evolution (Issue #34). The contracts below describe the normative interface that all SDK implementations MUST satisfy. The in-process `AsyncTaskManager` behavior documented above is equivalent to what an `InMemoryTaskStore` will provide.

### Inputs
- `task_id` (str/string/&str, required) — UUID v4 identifying the task

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend unreachable (network-backed stores only); in-memory implementations MUST NOT raise this

### Returns
- On success: `TaskInfo | None` — the stored task record, or `None`/`null` if no task with that ID exists

### Properties
- async: true (MAY be async for network-backed stores)
- thread_safe: true
- pure: false (reads external state)
- idempotent: true

---

## Contract: TaskStore.list

### Inputs
- `status` (TaskStatus, optional) — when provided, only tasks with this exact status are returned; when omitted, all stored tasks are returned

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend unreachable (network-backed stores only)

### Returns
- On success: `List[TaskInfo]` / `TaskInfo[]` — all matching task records; empty list if none match

### Properties
- async: true (MAY be async for network-backed stores)
- thread_safe: true
- pure: false (reads external state)
- idempotent: true

---

## Contract: TaskStore.delete

### Inputs
- `task_id` (str/string/&str, required) — UUID v4 identifying the task to remove

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend unreachable (network-backed stores only)
- Deleting a non-existent `task_id` MUST be a no-op (no error raised)

### Returns
- On success: `None` / `void` / `()`

### Properties
- async: true (MAY be async for network-backed stores)
- thread_safe: true
- pure: false (mutates store)
- idempotent: true (deleting an already-absent task_id succeeds silently)

---

## Contract: TaskStore.list_expired

### Inputs
- `before_timestamp` (float, required) — Unix timestamp (seconds); tasks whose `completed_at` is strictly less than this value are considered expired

### Eligible states
Only terminal-state tasks (`COMPLETED`, `FAILED`, `CANCELLED`) are eligible for expiry. Tasks without a `completed_at` (i.e. still `PENDING` or `RUNNING`) MUST NOT be returned by this method.

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend unreachable (network-backed stores only)

### Returns
- On success: `List[TaskInfo]` / `TaskInfo[]` — all terminal-state tasks whose `completed_at < before_timestamp`; empty list if none qualify

### Properties
- async: true (MAY be async for network-backed stores)
- thread_safe: true
- pure: false (reads external state)
- idempotent: true
