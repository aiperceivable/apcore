---
description: "Cookbook: cancel a long-running module mid-flight with a CancelToken passed through Context, in Python, TypeScript, and Rust."
---

# Cookbook — Cooperative Cancellation

> **Type:** User cookbook. **Normative spec:** [PROTOCOL_SPEC §8](../spec/protocol-spec.md#8-error-handling-specification) (`EXECUTION_CANCELLED`). Feature reference: [features/cancellation.md](../features/cancellation.md).

End-to-end recipe: a module that checks a `CancelToken` between units of work, and a caller that cancels it from outside. The same `demo.slow_task` module is used in all three languages; the SDKs ship the same scenario as `examples/cancel_token.py`, `examples/cancel-token.ts` and `examples/cancel_token.rs`.

## When to use this pattern

- A module works in a loop or in chunks and should stop when something external happens — a user action, a shutdown signal, a failed sibling request.
- You want the caller to get a distinct `EXECUTION_CANCELLED` error rather than a generic failure.

## When NOT to use this pattern

- Wall-clock limits on every call: set `executor.default_timeout` (milliseconds, per module call) or `executor.global_timeout` (whole call tree) in `apcore.yaml`. The executor enforces them without module code; see [features/cancellation.md § Integration with Executor Timeout](../features/cancellation.md#integration-with-executor-timeout).
- Work blocked in synchronous I/O that cannot poll a token: cancellation is **cooperative**, so a module that never checks the token runs to completion.

The executor also checks the token itself — at the start of the pipeline and again right before invoking the module — so a call whose token is already cancelled never reaches your code.

---

## 1. The module

=== "Python"
    ```python
    import time

    from apcore import APCore
    from apcore.context import Context

    client = APCore()


    @client.module(id="demo.slow_task", description="Simulates a long-running task")
    def slow_task(steps: int, context: Context) -> dict:
        completed = 0
        for _ in range(steps):
            if context.cancel_token:
                context.cancel_token.check()  # raises ExecutionCancelledError once cancelled
            time.sleep(0.05)  # simulate work
            completed += 1
        return {"completed": completed}
    ```

=== "TypeScript"
    ```typescript
    import { Type } from '@sinclair/typebox';
    import { APCore } from 'apcore-js';

    const client = new APCore();

    client.module({
      id: 'demo.slow_task',
      description: 'Simulates a long-running task',
      inputSchema: Type.Object({ steps: Type.Integer() }),
      outputSchema: Type.Object({ completed: Type.Integer() }),
      execute: async (inputs, context) => {
        const steps = inputs.steps as number;
        let completed = 0;
        for (let i = 0; i < steps; i++) {
          context.cancelToken?.check(); // throws ExecutionCancelledError once cancelled
          await new Promise((resolve) => setTimeout(resolve, 50)); // simulate work
          completed++;
        }
        return { completed };
      },
    });
    ```

=== "Rust"
    ```rust
    use apcore::{APCore, Context, ModuleError};
    use serde_json::{json, Value};
    use std::time::Duration;

    fn build_client() -> Result<APCore, ModuleError> {
        let mut client = APCore::new();
        client.module(
            "demo.slow_task",
            "Simulates a long-running task",
            json!({"type": "object", "properties": {"steps": {"type": "integer"}}, "required": ["steps"]}),
            json!({"type": "object", "properties": {"completed": {"type": "integer"}}}),
            None,   // documentation
            vec![], // tags
            None,   // version
            None,   // metadata
            vec![], // examples
            None,   // display
            |inputs: Value, ctx: &Context<Value>| {
                // `cancel_token` is a public field: Option<CancelToken>.
                let cancel_token = ctx.cancel_token.clone();
                Box::pin(async move {
                    let steps = inputs["steps"].as_u64().unwrap_or(0);
                    let mut completed = 0;
                    for _ in 0..steps {
                        if let Some(token) = &cancel_token {
                            token.check()?; // Err(ExecutionCancelledError) once cancelled
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await; // simulate work
                        completed += 1;
                    }
                    Ok(json!({"completed": completed}))
                })
            },
        )?;
        Ok(client)
    }
    ```

## 2. The caller

Create a token, pass it into the `Context`, and cancel it from elsewhere. Here a timer fires after 80 ms, while the 10-step task needs about 500 ms.

=== "Python"
    ```python
    import threading

    from apcore.cancel import CancelToken, ExecutionCancelledError

    token = CancelToken()
    ctx = Context.create(cancel_token=token)

    timer = threading.Timer(0.08, token.cancel)  # cancel from another thread after 80 ms
    timer.start()
    try:
        client.call("demo.slow_task", {"steps": 10}, ctx)
    except ExecutionCancelledError as e:
        print(f"Cancelled: {e}")
    finally:
        timer.cancel()  # no-op if the timer already fired
    ```

=== "TypeScript"
    ```typescript
    import { CancelToken, Context, ExecutionCancelledError } from 'apcore-js';

    const token = new CancelToken();
    // Context.create(identity, traceParent, cancelToken, data, services, globalDeadline)
    const ctx = Context.create(null, null, token);

    const timer = setTimeout(() => token.cancel(), 80);
    try {
      await client.call('demo.slow_task', { steps: 10 }, ctx);
    } catch (e) {
      if (!(e instanceof ExecutionCancelledError)) throw e;
      console.log(`Cancelled: ${e.message}`);
    } finally {
      clearTimeout(timer);
    }
    ```

=== "Rust"
    ```rust
    use apcore::{CancelToken, ErrorCode};

    #[tokio::main]
    async fn main() -> Result<(), ModuleError> {
        let client = build_client()?;

        let token = CancelToken::new(); // Clone shares the same cancellation state
        // Context::create(identity, trace_parent, cancel_token, data, services, global_deadline)
        let ctx: Context<Value> = Context::create(None, None, Some(token.clone()), None, Value::Null, None);

        let timer_token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            timer_token.cancel();
        });

        match client.call("demo.slow_task", json!({"steps": 10}), Some(&ctx), None).await {
            Err(e) if e.code == ErrorCode::ExecutionCancelled => println!("Cancelled: {e}"),
            Err(e) => return Err(e),
            Ok(result) => println!("Completed: {result}"),
        }
        Ok(())
    }
    ```

All three print a `Cancelled` line after roughly 100 ms: the token is set at 80 ms and the module notices at its next check.

## 3. Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| The module never checks the token | `cancel()` has no effect; the call runs to completion | Check the token at every loop iteration or between chunks of work |
| Reusing a cancelled token | Every later call with it fails immediately with `EXECUTION_CANCELLED` | Create a fresh `CancelToken` per call, or call `token.reset()` before reuse |
| Catching every error | Cancellation looks like an ordinary failure to upstream callers | Catch `ExecutionCancelledError` (Rust: `ErrorCode::ExecutionCancelled`) separately |
| Retrying a cancelled call with the same token | The retry is cancelled again at once | `EXECUTION_CANCELLED` is retryable only with a new token; decide in your retry logic whether the cancellation should end the request |
| Expecting a timeout to cancel the token | Module keeps running after `MODULE_TIMEOUT` | Timeouts do not call `cancel()`; see [features/cancellation.md § Integration with Executor Timeout](../features/cancellation.md#integration-with-executor-timeout) for what each SDK does to the running module |

---

## See also

- [features/cancellation.md](../features/cancellation.md) — `CancelToken` reference
- [features/middleware-system.md](../features/middleware-system.md) — how `on_error` middleware sees `ExecutionCancelledError`
- [Cookbook — Streaming Modules](./cookbook-streaming.md) — cancelling a stream producer
