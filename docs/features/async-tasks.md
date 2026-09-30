---
description: "AsyncTaskManager for background module execution: task lifecycle, concurrency and active-task limits, cancellation, pluggable TaskStore, retry with backoff, TTL reaper, graceful shutdown."
---

# Async Task Management

<!-- preamble-tier-doc -->
> **Type:** Implementation guide. **Normative spec:** [PROTOCOL_SPEC](../spec/protocol-spec.md) §5.8 Async Module Specification.


## Overview

The Async Task Management system runs module calls in the background. `AsyncTaskManager` wraps an Executor: `submit()` records the call in a `TaskStore`, returns a `task_id` immediately, and executes the module under a concurrency limit. Callers poll the task's status, fetch its result, or cancel it. Terminal task records are removed by `cleanup()` or by an optional background reaper.

## Requirements

- Provide an `AsyncTaskManager` that accepts an Executor and manages background task execution.
- Tasks **MUST** progress through a defined status lifecycle: `pending` → `running` → `completed` | `failed` | `cancelled`.
- Concurrency **MUST** be bounded by `max_concurrent` (default 10).
- The number of **active** (`pending` or `running`) tasks **MUST** be bounded by `max_tasks` (default 1000); a submission beyond it raises `TaskLimitExceededError` (`TASK_LIMIT_EXCEEDED`). Terminal records do not count.
- Task submission **MUST** return a `task_id` (UUID v4) without waiting for the module to run.
- Support cancellation of pending and running tasks that interrupts the running module (D-18).
- Persist task records through a pluggable, fully asynchronous `TaskStore` (D-17), with `InMemoryTaskStore` as the default.
- Support per-task retry with exponential backoff.
- Provide `cleanup()` of terminal tasks older than a threshold, and an opt-in reaper that does it periodically.
- Support graceful shutdown that cancels all active tasks.
- Propagate `TaskStore` failures to the caller from every manager method (D-81).

## Technical Design

### TaskStatus

The lifecycle has exactly **5 states**, identical across all three SDKs:

| Status | Terminal | Description |
|--------|----------|-------------|
| `pending` | No | Submitted and waiting for a concurrency slot, or waiting in retry backoff |
| `running` | No | Concurrency slot acquired, module executing |
| `completed` | Yes | Module returned successfully |
| `failed` | Yes | Module raised an error and retries are exhausted (or none were configured) |
| `cancelled` | Yes | Task was cancelled before or during execution |

A task waiting for its next retry attempt is `pending` with `retry_count > 0`; there is no separate retrying state. `TaskStatus.PENDING` in Python and TypeScript, `TaskStatus::Pending` in Rust; the serialized values are the lowercase names above.

### TaskInfo

=== "Python"
    ```python
    from dataclasses import dataclass
    from typing import Any

    from apcore import TaskStatus


    @dataclass
    class TaskInfo:
        task_id: str                     # UUID v4
        module_id: str                   # Module being executed
        status: TaskStatus               # Current lifecycle status
        submitted_at: float              # Unix timestamp (seconds)
        started_at: float | None = None  # Set when status -> running
        completed_at: float | None = None  # Set when status -> terminal
        result: Any = None               # Output (completed only)
        error: str | None = None         # Error message (failed only)
        retry_count: int = 0             # Retries taken so far
        max_retries: int = 0             # Configured retry budget for this task
    ```
=== "TypeScript"
    ```typescript
    import type { TaskStatus } from "apcore-js";

    interface TaskInfo {
        readonly taskId: string;
        readonly moduleId: string;
        readonly status: TaskStatus;
        readonly submittedAt: number;        // Unix seconds
        readonly startedAt: number | null;
        readonly completedAt: number | null;
        readonly result: Record<string, unknown> | null;
        readonly error: string | null;
        readonly retryCount: number;
        readonly maxRetries: number;
    }
    ```
=== "Rust"
    ```rust
    use apcore::TaskStatus;

    // #[non_exhaustive]; Clone + Serialize + Deserialize
    pub struct TaskInfo {
        pub task_id: String,
        pub module_id: String,
        pub status: TaskStatus,
        pub submitted_at: f64,               // Unix seconds
        pub started_at: Option<f64>,
        pub completed_at: Option<f64>,
        pub result: Option<serde_json::Value>,
        pub error: Option<String>,
        pub retry_count: u32,
        pub max_retries: u32,
    }
    ```

### AsyncTaskManager

| SDK | Construction |
|-----|--------------|
| Python | `AsyncTaskManager(executor, max_concurrent=10, max_tasks=1000, store=None)` |
| TypeScript | `new AsyncTaskManager({ executor, maxConcurrent?, maxTasks?, store? })` |
| Rust | `AsyncTaskManager::new(executor: Arc<Executor>, max_concurrent, max_tasks)`, or `AsyncTaskManager::with_store(executor, max_concurrent, max_tasks, store: Arc<dyn TaskStore>)` |

=== "Python"
    ```python
    import asyncio

    from apcore import APCore, AsyncTaskManager, TaskStatus

    client = APCore()


    @client.module(id="data.process_batch", description="Process a batch of items")
    def process_batch(items: list[str]) -> dict:
        return {"processed": len(items)}


    async def main() -> None:
        manager = AsyncTaskManager(client.executor, max_concurrent=10, max_tasks=1000)

        # Submit a background task
        task_id = await manager.submit("data.process_batch", {"items": ["a", "b"]})

        # Poll until the task reaches a terminal state.
        # get_status() is synchronous for the in-memory store; use
        # get_status_async() with an I/O-backed store.
        info = manager.get_status(task_id)
        while info is not None and info.status in (TaskStatus.PENDING, TaskStatus.RUNNING):
            await asyncio.sleep(0.05)
            info = manager.get_status(task_id)

        # Retrieve the result (raises unless the task completed)
        if info is not None and info.status == TaskStatus.COMPLETED:
            print(manager.get_result(task_id))

        # Cancel a task (False if it is unknown or already terminal)
        cancelled = await manager.cancel(task_id)

        # List tasks, optionally filtered by status
        all_tasks = manager.list_tasks()
        running = manager.list_tasks(TaskStatus.RUNNING)

        # Remove terminal tasks older than one hour
        removed = await manager.cleanup(max_age_seconds=3600.0)

        # Cancel everything still active
        await manager.shutdown()


    asyncio.run(main())
    ```
=== "TypeScript"
    ```typescript
    import { Type } from "@sinclair/typebox";
    import { APCore, AsyncTaskManager, TaskStatus } from "apcore-js";

    const client = new APCore();
    client.module({
        id: "data.process_batch",
        description: "Process a batch of items",
        inputSchema: Type.Object({ items: Type.Array(Type.String()) }),
        outputSchema: Type.Object({ processed: Type.Number() }),
        execute: (inputs) => ({ processed: (inputs.items as string[]).length }),
    });

    const manager = new AsyncTaskManager({
        executor: client.executor,
        maxConcurrent: 10,
        maxTasks: 1000,
    });

    // Submit a background task
    const taskId = await manager.submit("data.process_batch", { items: ["a", "b"] });

    // Poll until the task reaches a terminal state (every accessor is async)
    let info = await manager.getStatus(taskId);
    while (info && (info.status === TaskStatus.PENDING || info.status === TaskStatus.RUNNING)) {
        await new Promise((resolve) => setTimeout(resolve, 50));
        info = await manager.getStatus(taskId);
    }

    // Retrieve the result (throws unless the task completed)
    if (info?.status === TaskStatus.COMPLETED) {
        console.log(await manager.getResult(taskId));
    }

    // Cancel a task (false if it is unknown or already terminal)
    const cancelled = await manager.cancel(taskId);

    // List tasks, optionally filtered by status
    const allTasks = await manager.listTasks();
    const running = await manager.listTasks(TaskStatus.RUNNING);

    // Remove terminal tasks older than one hour
    const removed = await manager.cleanup(3600);

    // Cancel everything still active
    await manager.shutdown();
    ```
=== "Rust"
    ```rust
    use std::sync::Arc;
    use std::time::Duration;

    use apcore::{AsyncTaskManager, Config, Executor, ModuleError, Registry, TaskStatus};
    use serde_json::json;

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        // `registry` is assumed to hold a module registered as "data.process_batch".
        let registry = Registry::default();
        let executor = Arc::new(Executor::new(registry, Config::default()));
        let manager = AsyncTaskManager::new(executor, 10, 1000);

        // Submit a background task
        let task_id = manager
            .submit("data.process_batch", json!({"items": ["a", "b"]}), None)
            .await?;

        // Poll until the task reaches a terminal state.
        // get_status() is synchronous for the in-memory store; use
        // get_status_async() with an I/O-backed store.
        loop {
            match manager.get_status(&task_id)? {
                Some(info) if matches!(info.status, TaskStatus::Pending | TaskStatus::Running) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                _ => break,
            }
        }

        // Retrieve the result (Err unless the task completed)
        if let Ok(result) = manager.get_result(&task_id) {
            println!("{result}");
        }

        // Cancel a task (Ok(false) if it is unknown or already terminal)
        let _cancelled = manager.cancel(&task_id).await?;

        // List tasks, optionally filtered by status
        let _all = manager.list_tasks(None)?;
        let _running = manager.list_tasks(Some(TaskStatus::Running))?;

        // Remove terminal tasks older than one hour
        let _removed = manager.cleanup(3600.0)?;

        // Cancel everything still active
        manager.shutdown().await?;
        Ok(())
    }
    ```

### API Reference

| Operation | Python | TypeScript | Rust |
|-----------|--------|------------|------|
| Submit | `await submit(module_id, inputs, context=None, retry_policy=None) -> str` | `await submit(moduleId, inputs, { context?, retry? }): Promise<string>` | `submit(module_id, inputs, context).await -> Result<String>`; `submit_with_retry(module_id, inputs, context, retry)` |
| Status | `get_status(task_id) -> TaskInfo \| None`; `await get_status_async(task_id)` | `await getStatus(taskId): Promise<TaskInfo \| null>` | `get_status(task_id) -> Result<Option<TaskInfo>>`; `get_status_async(task_id).await` |
| Result | `get_result(task_id) -> Any` | `await getResult(taskId)` | `get_result(task_id) -> Result<Value>`; `get_result_async(task_id).await` |
| Cancel | `await cancel(task_id) -> bool` | `await cancel(taskId): Promise<boolean>` | `cancel(task_id).await -> Result<bool>` |
| List | `list_tasks(status=None) -> list[TaskInfo]`; `await list_tasks_async(status=None)` | `await listTasks(status?): Promise<TaskInfo[]>` | `list_tasks(status: Option<TaskStatus>) -> Result<Vec<TaskInfo>>` |
| Cleanup | `await cleanup(max_age_seconds=3600.0) -> int` | `await cleanup(maxAgeSeconds = 3600): Promise<number>` | `cleanup(max_age_seconds) -> Result<usize>` |
| Reaper | `start_reaper(*, ttl_seconds=3600.0, sweep_interval_ms=300_000) -> ReaperHandle`; `await stop_reaper()` | `startReaper({ ttlSeconds?, sweepIntervalMs? }): Promise<ReaperHandle>` | `start_reaper(ReaperConfig) -> Result<ReaperHandle>`; `stop_reaper() -> bool` |
| Shutdown | `await shutdown()` | `await shutdown()` | `shutdown().await -> Result<()>` |

In TypeScript every accessor is asynchronous, because the store is. In Python and Rust the synchronous accessors (`get_status`, `get_result`, `list_tasks`, and in Rust `cleanup`) drive the store call without an event loop; that works for `InMemoryTaskStore` and any store that completes without suspending. With an I/O-backed store use the async variants: a suspending store makes the synchronous accessor raise `RuntimeError` in Python and panic in Rust. Python has no async `get_result`, and Rust no async `list_tasks` / `cleanup`; with an I/O-backed store read the record through `get_status_async()` or the store itself.

### Concurrency Model

1. `submit()` checks the active-task count against `max_tasks` and saves a `pending` record, atomically with respect to other submissions, then schedules the task and returns its ID.
2. The task waits for one of `max_concurrent` slots.
3. Before running, the runner re-reads the stored record; a task that is already terminal (for example, cancelled while waiting) is not run. Otherwise it becomes `running` and the module is invoked through the Executor (`call_async` / `call`) with the submitted context.
4. On success the record becomes `completed` with the result; on failure it is retried (see [Retry with Backoff](#retry-with-backoff)) or becomes `failed` with `error` set. Every status write is preceded by a fresh read, so a concurrent `cancel()` — including one from another process sharing the store — is never overwritten.
5. The slot is released and the next waiting task proceeds.

A module that is not registered does not fail `submit()`: the task is accepted and ends `failed`.

### Cancellation

`cancel(task_id)` returns `false` if the task does not exist or is already terminal. Otherwise it interrupts the in-flight execution and writes the record as `cancelled` (D-18):

| SDK | How the running task is interrupted |
|-----|-------------------------------------|
| Python | The task's `asyncio.Task` is cancelled: the module sees `asyncio.CancelledError` at its next `await`, and `cancel()` waits for the task to settle. A `CancelToken` on the context you submitted is not touched. |
| TypeScript | Every task owns a `CancelToken`, bound into the task's context (replacing any token on the context you submitted). `cancel()` cancels it: the executor rejects with `ExecutionCancelledError`, and Web-API I/O that uses `context.signal` is aborted. A task in retry backoff wakes and ends. |
| Rust | The task's `JoinHandle` is aborted: the module future is dropped at its next `.await`. |

A task that is active in the store but has no in-process handle (written by another process, or by a previous run of this one) is still written as `cancelled`; only the interrupt step is skipped. See [Cancellation](./cancellation.md) for `CancelToken` itself.

### Task Storage

Task records live in a `TaskStore`. Every method is asynchronous in every SDK so that network-backed stores can be plugged in without blocking (D-17):

=== "Python"
    ```python
    from typing import Protocol

    from apcore import TaskInfo, TaskStatus


    class TaskStore(Protocol):
        async def save(self, info: TaskInfo) -> None: ...
        async def get(self, task_id: str) -> TaskInfo | None: ...
        async def list(self, status: TaskStatus | None = None) -> list[TaskInfo]: ...
        async def delete(self, task_id: str) -> None: ...
        async def list_expired(self, before_timestamp: float) -> list[TaskInfo]: ...
    ```
=== "TypeScript"
    ```typescript
    import type { TaskInfo, TaskStatus } from "apcore-js";

    interface TaskStore {
        save(task: TaskInfo): Promise<void>;
        get(taskId: string): Promise<TaskInfo | null>;
        list(status?: TaskStatus): Promise<TaskInfo[]>;
        delete(taskId: string): Promise<void>;
        listExpired(beforeTimestamp: number): Promise<TaskInfo[]>;
    }
    ```
=== "Rust"
    ```rust
    use apcore::{ModuleError, TaskInfo, TaskStatus};
    use async_trait::async_trait;

    #[async_trait]
    pub trait TaskStore: Send + Sync {
        async fn save(&self, task: &TaskInfo) -> Result<(), ModuleError>;
        async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ModuleError>;
        async fn list(&self, status: Option<TaskStatus>) -> Result<Vec<TaskInfo>, ModuleError>;
        async fn delete(&self, id: &str) -> Result<(), ModuleError>;
        async fn list_expired(&self, before_timestamp: f64) -> Result<Vec<TaskInfo>, ModuleError>;
        /// Name of the concrete store type, e.g. "InMemoryTaskStore".
        fn store_type_name(&self) -> &'static str;
    }
    ```

- `InMemoryTaskStore` is the default and the only store the SDKs ship. A durable backend (Redis, SQL, …) is implemented by the application against this interface and injected at construction; apcore takes no dependency on a storage client.
- `list()` returns records in submission order. A store whose backing map has no insertion order keeps its own insertion counter; ordering by `task_id` (a random UUID) is not acceptable (D-82).
- A store that cannot reach its backend raises `TaskStoreError` (code `TASK_STORE_UNAVAILABLE`; Rust `ModuleError::task_store_unavailable(operation, reason)`). `InMemoryTaskStore` never does. Every manager method propagates it — `submit`, `cancel`, `get_status`, `get_result`, `list_tasks`, `cleanup`, `shutdown` — rather than turning an outage into `false`, `None` or an empty list (D-81).

Injecting a store:

=== "Python"
    ```python
    from apcore import APCore, AsyncTaskManager, InMemoryTaskStore

    client = APCore()

    # Replace InMemoryTaskStore with your own TaskStore implementation for durability.
    store = InMemoryTaskStore()
    manager = AsyncTaskManager(client.executor, store=store)
    ```
=== "TypeScript"
    ```typescript
    import { APCore, AsyncTaskManager, InMemoryTaskStore } from "apcore-js";

    const client = new APCore();

    // Replace InMemoryTaskStore with your own TaskStore implementation for durability.
    const store = new InMemoryTaskStore();
    const manager = new AsyncTaskManager({ executor: client.executor, store });
    ```
=== "Rust"
    ```rust
    use std::sync::Arc;

    use apcore::{AsyncTaskManager, Config, Executor, InMemoryTaskStore, Registry};

    fn main() {
        let executor = Arc::new(Executor::new(Registry::default(), Config::default()));

        // Replace InMemoryTaskStore with your own `impl TaskStore` for durability.
        let store = Arc::new(InMemoryTaskStore::new());
        let _manager = AsyncTaskManager::with_store(executor, 10, 1000, store);
    }
    ```

### Retry with Backoff

A task can be submitted with a retry configuration. Its fields and defaults are identical in all SDKs: `max_retries` (0 — no retries), `retry_delay_ms` (1000), `backoff_multiplier` (2.0), `max_retry_delay_ms` (60000).

- When an attempt fails and fewer than `max_retries` retries have been taken, the task returns to `pending`, `retry_count` is incremented, and the next attempt starts after a delay. Otherwise the task becomes `failed` with `error` set.
- The delay before retry *n* (1-based) is `min(retry_delay_ms × backoff_multiplier^(n−1), max_retry_delay_ms)`. `compute_delay_ms(attempt)` / `computeDelayMs(attempt)` computes it for a 0-based `attempt`; Rust truncates the result to whole milliseconds (`u64`).
- A task cancelled during backoff ends `cancelled` without another attempt.

The class is exported at the package root as `AsyncRetryConfig` in all three SDKs (the root name `RetryConfig` belongs to the retry middleware). There are no configuration-file keys for async tasks; retry is set per submission.

=== "Python"
    ```python
    from apcore import APCore, AsyncRetryConfig, AsyncTaskManager

    client = APCore()
    manager = AsyncTaskManager(client.executor)


    async def submit_with_retry() -> str:
        return await manager.submit(
            "data.process_batch",
            {"items": ["a", "b"]},
            retry_policy=AsyncRetryConfig(
                max_retries=3,
                retry_delay_ms=500,
                backoff_multiplier=2.0,
                max_retry_delay_ms=30000,
            ),
        )
    ```
=== "TypeScript"
    ```typescript
    import { APCore, AsyncRetryConfig, AsyncTaskManager } from "apcore-js";

    const client = new APCore();
    const manager = new AsyncTaskManager({ executor: client.executor });

    const taskId = await manager.submit(
        "data.process_batch",
        { items: ["a", "b"] },
        {
            retry: new AsyncRetryConfig({
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
    use std::sync::Arc;

    use apcore::{AsyncRetryConfig, AsyncTaskManager, Config, Executor, ModuleError, Registry};
    use serde_json::json;

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let executor = Arc::new(Executor::new(Registry::default(), Config::default()));
        let manager = AsyncTaskManager::new(executor, 10, 1000);

        // `#[non_exhaustive]`: start from Default and assign fields.
        let mut retry = AsyncRetryConfig::default();
        retry.max_retries = 3;
        retry.retry_delay_ms = 500;
        retry.backoff_multiplier = 2.0;
        retry.max_retry_delay_ms = 30_000;

        let _task_id = manager
            .submit_with_retry("data.process_batch", json!({"items": ["a", "b"]}), None, Some(retry))
            .await?;
        Ok(())
    }
    ```

### Reaper (TTL-Based Cleanup)

The reaper is an opt-in background task that periodically deletes terminal tasks whose `completed_at` is older than `ttl_seconds`, using `store.list_expired(now - ttl_seconds)`. It runs only after `start_reaper()` is called; there is no configuration key that starts it.

- Defaults in all three SDKs: `ttl_seconds` 3600 (1 hour), `sweep_interval_ms` 300000 (5 minutes) (D-48).
- It never deletes `pending` or `running` tasks.
- A failed sweep is logged as a warning and retried at the next interval; it is not surfaced to the caller.
- Starting a second reaper while one is running raises `ModuleError` with code `REAPER_ALREADY_RUNNING` in every SDK.
- `start_reaper` returns a `ReaperHandle` without waiting; `handle.stop()` is async and waits for an in-flight sweep to finish. `shutdown()` stops the reaper first.

=== "Python"
    ```python
    from apcore import APCore, AsyncTaskManager

    client = APCore()


    async def run_with_reaper() -> None:
        manager = AsyncTaskManager(client.executor)

        # Synchronous; must be called while an event loop is running.
        handle = manager.start_reaper(ttl_seconds=7200, sweep_interval_ms=600_000)

        # ... application runs ...

        await handle.stop()  # or: await manager.stop_reaper()
    ```
=== "TypeScript"
    ```typescript
    import { APCore, AsyncTaskManager } from "apcore-js";

    const client = new APCore();
    const manager = new AsyncTaskManager({ executor: client.executor });

    // Returns an already-resolved Promise; a second call while running throws synchronously.
    const handle = await manager.startReaper({ ttlSeconds: 7200, sweepIntervalMs: 600000 });

    // ... application runs ...

    await handle.stop();
    ```
=== "Rust"
    ```rust
    use std::sync::Arc;

    use apcore::{AsyncTaskManager, Config, Executor, ModuleError, ReaperConfig, Registry};

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let executor = Arc::new(Executor::new(Registry::default(), Config::default()));
        let manager = AsyncTaskManager::new(executor, 10, 1000);

        // `#[non_exhaustive]`: start from Default and assign fields.
        let mut config = ReaperConfig::default();
        config.ttl_seconds = 7200.0;
        config.sweep_interval_ms = 600_000;

        // Synchronous; must be called inside a Tokio runtime.
        let handle = manager.start_reaper(config)?;

        // ... application runs ...

        handle.stop().await;
        Ok(())
    }
    ```

## Dependencies

- **Executor** — Invokes modules (`call_async` / `call`).
- **Context** — Optional context passed with each submission and on to the module.
- **Cancellation System** — `CancelToken` (TypeScript tasks) and `ExecutionCancelledError`.
- **Error System** — `TaskLimitExceededError`, `TaskStoreError`, `REAPER_ALREADY_RUNNING`.

??? info "Python SDK reference"
    The following table is **not a protocol requirement** — it documents the Python SDK's source layout for implementers/users of `apcore-python`.

    | File | Purpose |
    |------|---------|
    | `src/apcore/async_task.py` | `AsyncTaskManager`, `TaskStatus`, `TaskInfo`, `TaskStore`, `InMemoryTaskStore`, `RetryConfig`, `ReaperHandle` |

## Testing Strategy

- **Lifecycle tests** verify the full status progression: pending → running → completed/failed/cancelled, and pending during retry backoff.
- **Concurrency tests** verify that no more than `max_concurrent` tasks run simultaneously.
- **Capacity tests** verify that submission is rejected with `TASK_LIMIT_EXCEEDED` when `max_tasks` active tasks exist, that terminal tasks do not count, and that concurrent submissions cannot overshoot the limit.
- **Cancellation tests** verify that pending tasks never run, running tasks are interrupted, a store-resident task with no local handle is still cancelled, and a runner never overwrites a `cancelled` record.
- **Store error tests** verify that a failing `TaskStore` surfaces from every manager method.
- **Retry tests** verify the backoff delays and the `retry_count` / `failed` transitions.
- **Cleanup and reaper tests** verify that only terminal tasks older than the threshold are removed and that a second `start_reaper` raises `REAPER_ALREADY_RUNNING`.
- **Shutdown tests** verify that all active tasks are cancelled and that a store failure is raised only after every task was attempted.
- Cross-language cases live in `conformance/fixtures/async_task_cancellation.json` and `async_task_evolution.json`.

## Contract: AsyncTaskManager.submit

### Inputs
- `module_id` (str/string/&str, required) — module to execute
- `inputs` (dict/object/Value, required) — module inputs
- `context` (Context, optional) — execution context passed to the module
- retry configuration (optional) — Python `retry_policy=`, TypeScript `{ retry }`, Rust `submit_with_retry(..., Some(retry))`

### Errors
- `TaskLimitExceededError(code=TASK_LIMIT_EXCEEDED)` — `max_tasks` tasks are already `pending` or `running`
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — the store failed
- An unknown or malformed `module_id` is **not** a submit error; the task ends `failed`

### Returns
- On success: `task_id` — a UUID v4 string

### Properties
- async: true
- thread_safe: true
- pure: false (spawns background work, persists task state)
- idempotent: false

## Contract: AsyncTaskManager.get_status

### Inputs
- `task_id` (str/string/&str, required) — UUID v4 identifying the task

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — the store failed
- An unknown `task_id` is not an error: it returns `None` / `null` / `Ok(None)`

### Returns
- On success: `TaskInfo | None` — a snapshot of the task record. The snapshot is a copy in every SDK: mutating it never affects the store, and later store changes are not visible through it (D-23).

### Properties
- async: TypeScript `getStatus` is async. Python and Rust `get_status` are synchronous (in-memory store); `get_status_async` is the async form for I/O-backed stores.
- thread_safe: true
- pure: false (reads mutable task state)
- idempotent: true

## Contract: AsyncTaskManager.get_result

### Inputs
- `task_id` (str/string/&str, required) — UUID v4 identifying the task

### Errors
- Task not found — Python `KeyError("Task not found: <id>")`, TypeScript `Error("Task not found: <id>")`, Rust `ModuleError(GENERAL_INTERNAL_ERROR)`
- Task not `completed` (including `failed` and `cancelled`) — Python `RuntimeError`, TypeScript `Error`, Rust `ModuleError(GENERAL_INTERNAL_ERROR)`, each with message `Task <id> is not completed (status=<value>)`
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — the store failed

### Returns
- On success: the task's `result` (module output)

### Properties
- async: TypeScript `getResult` is async. Python `get_result` is synchronous; Rust has `get_result` and `get_result_async`.
- thread_safe: true
- pure: false (reads mutable task state)
- idempotent: true

## Contract: AsyncTaskManager.cancel

### Inputs
- `task_id` (str/string/&str, required) — ID of the task to cancel

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — the store failed; a cancel whose `cancelled` write failed never reports `true`
- Otherwise none: the outcome is reported through the return value

### Returns
- On success: `bool` — `true` if the task was active and is now `cancelled`; `false` if it did not exist or was already terminal

### Properties
- async: true
- thread_safe: true
- idempotent: true (cancelling an already-cancelled task returns `false`)

## Contract: AsyncTaskManager.list_tasks

### Inputs
- `status` (TaskStatus, optional) — only tasks with this status; all tasks when omitted

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — the store failed

### Returns
- On success: a list of `TaskInfo` snapshots (copies, as for `get_status`), in submission order (D-82); empty if nothing matches

### Properties
- async: TypeScript `listTasks` is async. Python `list_tasks` is synchronous with `list_tasks_async` for I/O-backed stores; Rust `list_tasks` is synchronous.
- thread_safe: true
- pure: false (reads mutable task state)
- idempotent: true

## Contract: AsyncTaskManager.cleanup

### Inputs
- `max_age_seconds` (float, optional, default=3600.0) — tasks whose reference timestamp is **at least** this many seconds old are removed

### Reference timestamp
- `completed_at` when set, otherwise `submitted_at`

### Eligible states
Only terminal tasks (`completed`, `failed`, `cancelled`) are removed; `pending` and `running` tasks never are.

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — the store failed

### Returns
- On success: the number of tasks removed (`0` if none were eligible)

### Properties
- async: true in Python and TypeScript; synchronous in Rust (`Result<usize>`)
- thread_safe: true
- pure: false (mutates the task store)
- idempotent: false

## Contract: AsyncTaskManager.shutdown

### Inputs
- None

### Behavior
1. Stops the reaper, if one is running.
2. Lists the store and cancels every `pending` or `running` task through `cancel()`, including store-resident tasks with no in-process handle.
3. Attempts every cancellation even after one fails (D-122), then propagates the first failure.

Exceptions raised by task bodies while they are being cancelled are logged and not re-raised.

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` (or any other error a `cancel()` raised) — the first failure, raised after every active task was attempted

### Returns
- On success: `None` / `void` / `Ok(())`. Every task that was active at the call is then `cancelled`.

### Properties
- async: true
- thread_safe: true
- pure: false (mutates task state)
- idempotent: true (no active tasks → no-op)

## Contract: AsyncTaskManager.start_reaper

### Inputs
- `ttl_seconds` (float, optional, default=3600) — tasks with `completed_at` older than `now - ttl_seconds` are deleted
- `sweep_interval_ms` (int, optional, default=300000) — how often the reaper sweeps
- Rust takes both in a `ReaperConfig`; TypeScript as `{ ttlSeconds, sweepIntervalMs }`; Python as keyword-only arguments

### Errors
- `ModuleError(code=REAPER_ALREADY_RUNNING)` — a reaper is already running on this manager
- Sweep failures are logged as warnings and retried at the next interval; they are never surfaced

### Returns
- On success: `ReaperHandle` — `stop()` (async) cancels the loop and waits for an in-flight sweep

### Properties
- async: false — the call schedules the loop and returns the handle. TypeScript returns an already-resolved `Promise<ReaperHandle>`.
- thread_safe: true
- pure: false (starts a background task)
- idempotent: false (a second call raises `REAPER_ALREADY_RUNNING`)

## Contract: TaskStore.save

### Inputs
- `task_info` (TaskInfo, required) — the task to persist (creates or overwrites)

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend is unreachable

### Returns
- On success: void/None/()

### Properties
- async: true
- thread_safe: true
- pure: false
- idempotent: true (saving twice with the same `task_id` overwrites)

## Contract: TaskStore.get

### Inputs
- `task_id` (str/string/&str, required) — UUID v4 identifying the task

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend unreachable (network-backed stores only)

### Returns
- On success: `TaskInfo | None` — the stored task record, or `None`/`null` if no task with that ID exists

### Properties
- async: true
- thread_safe: true
- pure: false (reads external state)
- idempotent: true

## Contract: TaskStore.list

### Inputs
- `status` (TaskStatus, optional) — only tasks with this status; all stored tasks when omitted

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend unreachable (network-backed stores only)

### Returns
- On success: `List[TaskInfo]` / `TaskInfo[]` — matching task records in insertion order; empty list if none match

### Properties
- async: true
- thread_safe: true
- pure: false (reads external state)
- idempotent: true

## Contract: TaskStore.delete

### Inputs
- `task_id` (str/string/&str, required) — UUID v4 identifying the task to remove

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend unreachable (network-backed stores only)
- Deleting a non-existent `task_id` is a no-op

### Returns
- On success: `None` / `void` / `()`

### Properties
- async: true
- thread_safe: true
- pure: false (mutates store)
- idempotent: true

## Contract: TaskStore.list_expired

### Inputs
- `before_timestamp` (float, required) — Unix timestamp (seconds); tasks whose `completed_at` is strictly less than this value are expired

### Eligible states
Only terminal tasks are eligible. Tasks without a `completed_at` (still `pending` or `running`) are never returned.

### Errors
- `TaskStoreError(code=TASK_STORE_UNAVAILABLE)` — backend unreachable (network-backed stores only)

### Returns
- On success: `List[TaskInfo]` / `TaskInfo[]` — terminal tasks with `completed_at < before_timestamp`; empty list if none qualify

### Properties
- async: true
- thread_safe: true
- pure: false (reads external state)
- idempotent: true
