# Async Task Management

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

| Status | Terminal | Description |
|--------|----------|-------------|
| `pending` | No | Submitted, waiting for a concurrency slot |
| `running` | No | Concurrency slot acquired, module executing |
| `completed` | Yes | Module returned successfully |
| `failed` | Yes | Module raised an error |
| `cancelled` | Yes | Task was cancelled before or during execution |

### TaskInfo

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
        error: str | None                # Error message (failed only)
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
    const taskId = manager.submit("data.process_batch", { items: largeList });

    // Check status
    const info = manager.getStatus(taskId);
    console.log(info?.status); // "pending" or "running"

    // Retrieve result (only when completed)
    const result = manager.getResult(taskId);

    // Cancel a task
    const cancelled = manager.cancel(taskId);

    // List tasks (optionally filtered by status)
    const allTasks = manager.listTasks();
    const running = manager.listTasks("running");

    // Clean up old terminal tasks (default: older than 1 hour)
    const removed = manager.cleanup(3600);

    // Graceful shutdown
    manager.shutdown();
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
2. If no `CancelToken` is available, the underlying async task is cancelled directly (e.g., `asyncio.Task.cancel()` in Python).
3. The task transitions to `cancelled` state and its result is discarded.

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
